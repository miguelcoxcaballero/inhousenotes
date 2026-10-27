// Camera and object motion blur: McGuire, Hennessy, Bukowski & Osman,
// "A Reconstruction Filter for Plausible Motion Blur" (I3D 2012), with the
// tile-max / neighbour-max dilation, soft depth comparison and the
// per-pixel jitter of Guertin, McGuire & Nowrouzezahrai (HPG 2014).
// `common.wgsl` is prepended at load time.
//
// The prepass writes the screen motion of every opaque pixel (camera and
// object, in pixels per frame); sky pixels get the camera rotation here.
// Blur radius = motion x shutter (a fixed 1/120 s exposure, so the streak
// length does not grow at low frame rates), clamped to the tile size K.
// Cost: tile max K^2 loads per K^2 pixels (1 per pixel), neighbour max 9
// per tile, gather 2 x S samples (colour, depth, velocity) per moving
// pixel; still pixels exit after one tile read.

struct MotionParams {
    params: vec4<f32>, // x: shutter scale (exposure / frame time), y: samples, z: tile size K (px), w: max radius (px)
};

@group(1) @binding(0) var color_tex: texture_2d<f32>;
@group(1) @binding(1) var depth_tex: texture_depth_2d;
@group(1) @binding(2) var velocity_tex: texture_2d<f32>;
@group(1) @binding(3) var tile_tex: texture_2d<f32>;
@group(1) @binding(4) var lin_samp: sampler;
@group(1) @binding(5) var<uniform> mb: MotionParams;

struct FsIn {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_fullscreen(@builtin(vertex_index) i: u32) -> FsIn {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var o: FsIn;
    o.pos = vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);
    o.uv = uv;
    return o;
}

fn max_px() -> vec2<i32> {
    return vec2<i32>(frame.screen.xy) - vec2<i32>(1);
}

// Blur radius vector (px) at a pixel: half the motion over the exposure,
// clamped to the tile size.
fn blur_radius(p: vec2<i32>) -> vec2<f32> {
    let q = clamp(p, vec2<i32>(0), max_px());
    let d = textureLoad(depth_tex, q, 0);
    var v = textureLoad(velocity_tex, q, 0).xy;
    if (d <= 0.0) {
        // Sky: reproject the direction at infinity (camera rotation only).
        let ndc = vec2<f32>((f32(q.x) + 0.5) * frame.screen.z * 2.0 - 1.0, 1.0 - (f32(q.y) + 0.5) * frame.screen.w * 2.0);
        let far = frame.inv_view_proj * vec4<f32>(ndc, 1e-7, 1.0);
        let dir = far.xyz / far.w - frame.camera_pos.xyz;
        let prev = frame.prev_view_proj * vec4<f32>(dir, 0.0);
        if (prev.w <= 1e-6) { return vec2<f32>(0.0); }
        let pn = prev.xy / prev.w;
        v = (ndc - frame.jitter.xy - pn) * vec2<f32>(0.5, -0.5) * frame.screen.xy;
    }
    let r = v * (0.5 * mb.params.x);
    let len = length(r);
    let k = mb.params.w;
    if (len > k) { return r * (k / len); }
    return r;
}

// Tile max: the longest blur radius in each K x K tile.
@fragment
fn fs_tile_max(in: FsIn) -> @location(0) vec4<f32> {
    let k = i32(mb.params.z);
    let base = vec2<i32>(in.pos.xy) * k;
    var best = vec2<f32>(0.0);
    var best2 = 0.0;
    for (var y = 0; y < k; y++) {
        for (var x = 0; x < k; x++) {
            let r = blur_radius(base + vec2<i32>(x, y));
            let l2 = dot(r, r);
            if (l2 > best2) { best2 = l2; best = r; }
        }
    }
    return vec4<f32>(best, 0.0, 1.0);
}

// Neighbour max: the longest tile radius in the 3x3 tiles around.
@fragment
fn fs_neighbor_max(in: FsIn) -> @location(0) vec4<f32> {
    let p = vec2<i32>(in.pos.xy);
    let m = vec2<i32>(textureDimensions(tile_tex)) - vec2<i32>(1);
    var best = vec2<f32>(0.0);
    var best2 = 0.0;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let r = textureLoad(tile_tex, clamp(p + vec2<i32>(x, y), vec2<i32>(0), m), 0).xy;
            let l2 = dot(r, r);
            if (l2 > best2) { best2 = l2; best = r; }
        }
    }
    return vec4<f32>(best, 0.0, 1.0);
}

fn soft_depth(za: f32, zb: f32, extent: f32) -> f32 {
    return clamp(1.0 - (za - zb) / extent, 0.0, 1.0);
}

fn cone(dist: f32, r: f32) -> f32 {
    return clamp(1.0 - dist / max(r, 1e-3), 0.0, 1.0);
}

fn cylinder(dist: f32, r: f32) -> f32 {
    return 1.0 - smoothstep(0.95 * r, 1.05 * r + 1e-3, dist);
}

fn lin_z(p: vec2<i32>) -> f32 {
    let d = textureLoad(depth_tex, clamp(p, vec2<i32>(0), max_px()), 0);
    return select(linear_depth(d), 1e6, d <= 0.0);
}

@fragment
fn fs_motion_blur(in: FsIn) -> @location(0) vec4<f32> {
    let p = vec2<i32>(in.pos.xy);
    let k = i32(mb.params.z);
    let c = textureLoad(color_tex, p, 0);
    let tiles = vec2<i32>(textureDimensions(tile_tex)) - vec2<i32>(1);
    let vn = textureLoad(tile_tex, min(p / k, tiles), 0).xy;
    let ln = length(vn);
    // Still neighbourhood (or sub-pixel motion): untouched.
    if (ln < 0.5) { return c; }
    let vx = blur_radius(p);
    let lx = max(length(vx), 0.5);
    let zx = lin_z(p);
    let s = i32(mb.params.y);
    let jit = ign(in.pos.xy) - 0.5;
    var w_sum = 1.0 / lx;
    var sum = c.rgb * w_sum;
    let soft = max(zx * 0.02, 0.05);
    for (var i = 0; i < s; i++) {
        // Samples spread over [-1, 1] along the dominant direction; odd
        // ones along the pixel's own motion (Guertin 2014), which keeps thin
        // moving objects sharp against a blurred background.
        let t = mix(-1.0, 1.0, (f32(i) + jit + 1.0) / f32(s + 1));
        let dir = select(vn, vx, (i & 1) == 1 && lx > 1.0);
        let off = dir * t;
        let q = p + vec2<i32>(round(off));
        let dist = length(off);
        if (dist < 0.5) { continue; }
        let vy = blur_radius(q);
        let ly = max(length(vy), 0.5);
        let zy = lin_z(q);
        let f = soft_depth(zx, zy, soft);
        let b = soft_depth(zy, zx, soft);
        let a = f * cone(dist, ly) + b * cone(dist, lx) + cylinder(dist, ly) * cylinder(dist, lx) * 2.0;
        let cy = textureLoad(color_tex, clamp(q, vec2<i32>(0), max_px()), 0).rgb;
        w_sum += a;
        sum += cy * a;
    }
    return vec4<f32>(sum / w_sum, c.a);
}
