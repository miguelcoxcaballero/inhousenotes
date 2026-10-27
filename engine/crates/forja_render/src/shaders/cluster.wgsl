// Light clustering: bins the frame's local lights (sorted closest first on
// the CPU) into a froxel grid of `cluster.w`-pixel tiles x `cluster.z`
// exponential depth slices, plus one whole-column bucket per tile (slice
// index `cluster.z`) holding the lights whose fog glow can reach the tile.
// One invocation per cluster; lights are staged through workgroup memory in
// batches of 64 (transformed to view space once per batch).
// `frame.wgsl` is prepended at load time.
//
// Cost: clusters x lights sphere/cone tests. 1080p (30 x 17 x 33 clusters)
// with 2000 visible lights: ~34 M tests, ~0.15 ms on an RTX 3070 class GPU.

@group(0) @binding(1) var<storage, read> lights_in: array<LocalLight>;
@group(0) @binding(2) var<storage, read_write> clusters_out: array<u32>;

const STRIDE: u32 = 49u;
const CAP: u32 = 96u;

struct Staged {
    pos_r: vec4<f32>,    // view-space position, range
    dir_c: vec4<f32>,    // view-space direction, cos outer (-2: point)
    sphere: vec4<f32>,   // view-space bounding sphere of the lit volume
    dist: f32,           // distance to the camera
};

var<workgroup> staged: array<Staged, 64>;

// Inward normals of the four side planes of a tile's view pyramid.
struct Tile {
    l: vec3<f32>,
    r: vec3<f32>,
    b: vec3<f32>,
    t: vec3<f32>,
};

fn sphere_in_tile(t: Tile, c: vec3<f32>, r: f32) -> bool {
    return dot(t.l, c) > -r && dot(t.r, c) > -r && dot(t.b, c) > -r && dot(t.t, c) > -r;
}

// Cone (apex p, axis d, cos/sin half angle, length) against a sphere
// (Wronski 2017).
fn cone_hits_sphere(p: vec3<f32>, d: vec3<f32>, cos_a: f32, range: f32, c: vec3<f32>, r: f32) -> bool {
    let v = c - p;
    let vlen2 = dot(v, v);
    let v1 = dot(v, d);
    let sin_a = sqrt(max(1.0 - cos_a * cos_a, 0.0));
    let closest = cos_a * sqrt(max(vlen2 - v1 * v1, 0.0)) - v1 * sin_a;
    return !(closest > r || v1 > r + range || v1 < -r);
}

@compute @workgroup_size(64)
fn cs_cluster(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(local_invocation_index) li: u32) {
    let tx = u32(frame.cluster.x);
    let ty = u32(frame.cluster.y);
    let slices = u32(frame.cluster.z);
    let total = tx * ty * (slices + 1u);
    let id = gid.x;
    let valid = id < total;
    let tile_x = id % tx;
    let tile_y = (id / tx) % ty;
    let slice = id / (tx * ty);
    let column = slice == slices;

    // Tile pyramid in view space (camera looks down -Z, y up).
    let px = frame.cluster.w;
    let x0 = f32(tile_x) * px * frame.screen.z * 2.0 - 1.0;
    let x1 = min(f32(tile_x + 1u) * px * frame.screen.z * 2.0 - 1.0, 1.0);
    let y1 = 1.0 - f32(tile_y) * px * frame.screen.w * 2.0;
    let y0 = max(1.0 - f32(tile_y + 1u) * px * frame.screen.w * 2.0, -1.0);
    let sx = frame.proj_params.x;
    let sy = frame.proj_params.y;
    var tile: Tile;
    tile.l = normalize(vec3<f32>(1.0, 0.0, x0 * sx));
    tile.r = normalize(vec3<f32>(-1.0, 0.0, -x1 * sx));
    tile.b = normalize(vec3<f32>(0.0, 1.0, y0 * sy));
    tile.t = normalize(vec3<f32>(0.0, -1.0, -y1 * sy));
    // Depth slab of the slice (positive distances).
    let zn = frame.cluster_z.x;
    let inv_k = 1.0 / frame.cluster_z.z;
    var z_lo = select(zn * exp(f32(slice) * inv_k), 0.0, slice == 0u);
    var z_hi = zn * exp(f32(slice + 1u) * inv_k);
    if (slice + 1u >= slices) { z_hi = 1e9; }
    if (column) { z_lo = 0.0; z_hi = 1e9; }
    // Cluster bounding sphere, for the cone test (finite slices only).
    let zc_hi = min(z_hi, frame.cluster_z.y * 4.0);
    let cxs = array<f32, 2>(x0 * sx, x1 * sx);
    let cys = array<f32, 2>(y0 * sy, y1 * sy);
    var bmin = vec3<f32>(1e30);
    var bmax = vec3<f32>(-1e30);
    for (var k = 0u; k < 8u; k++) {
        let z = select(z_lo, zc_hi, (k & 4u) != 0u);
        let p = vec3<f32>(cxs[k & 1u] * z, cys[(k >> 1u) & 1u] * z, -z);
        bmin = min(bmin, p);
        bmax = max(bmax, p);
    }
    let bc = (bmin + bmax) * 0.5;
    let br = length(bmax - bmin) * 0.5;

    let n_lights = u32(frame.lighting.x);
    let scatter_dist = frame.cluster_z.w;
    var count = 0u;
    var packed = 0u;
    let base = id * STRIDE;
    for (var start = 0u; start < n_lights; start += 64u) {
        let j = start + li;
        if (j < n_lights) {
            let l = lights_in[j];
            let pv = (frame.view * vec4<f32>(l.pos_range.xyz, 1.0)).xyz;
            let dv = (frame.view * vec4<f32>(l.dir.xyz, 0.0)).xyz;
            var s: Staged;
            s.pos_r = vec4<f32>(pv, l.pos_range.w);
            s.dir_c = vec4<f32>(dv, l.dir.w);
            s.sphere = vec4<f32>(pv, l.pos_range.w);
            let cos_a = l.dir.w;
            if (cos_a > -1.5) {
                let r = l.pos_range.w;
                if (cos_a < 0.70710678) {
                    s.sphere = vec4<f32>(pv + dv * (cos_a * r), sqrt(max(1.0 - cos_a * cos_a, 0.0)) * r);
                } else {
                    let rr = r / (2.0 * cos_a);
                    s.sphere = vec4<f32>(pv + dv * rr, rr);
                }
            }
            s.dist = length(pv);
            staged[li] = s;
        }
        workgroupBarrier();
        let batch = min(64u, n_lights - start);
        if (valid) {
            for (var k = 0u; k < batch; k++) {
                if (count >= CAP) { break; }
                let s = staged[k];
                let c = s.sphere.xyz;
                let r = s.sphere.w;
                // Depth slab, then the tile's side planes.
                if (-c.z + r < z_lo || -c.z - r > z_hi) { continue; }
                if (!sphere_in_tile(tile, c, r)) { continue; }
                if (column) {
                    // Only nearby lights glow in the fog of this tile.
                    if (s.dist - s.pos_r.w > scatter_dist) { continue; }
                } else if (s.dir_c.w > -1.5 && !cone_hits_sphere(s.pos_r.xyz, s.dir_c.xyz, s.dir_c.w, s.pos_r.w, bc, br)) {
                    continue;
                }
                let idx = start + k;
                if ((count & 1u) == 0u) {
                    packed = idx;
                } else {
                    clusters_out[base + 1u + (count >> 1u)] = packed | (idx << 16u);
                }
                count += 1u;
            }
        }
        workgroupBarrier();
    }
    if (valid) {
        if ((count & 1u) == 1u) {
            clusters_out[base + 1u + (count >> 1u)] = packed;
        }
        clusters_out[base] = count;
    }
}
