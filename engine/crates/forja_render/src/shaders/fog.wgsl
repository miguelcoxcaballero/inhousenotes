// Froxel volumetric fog (Wronski 2014, "Volumetric Fog: Unified Compute
// Shader Based Solution to Atmospheric Scattering"; Hillaire 2015,
// "Physically Based and Unified Volumetric Rendering in Frostbite").
// `common.wgsl` is prepended at load time.
//
// The same medium as the analytic fog (height fog + dawn/dusk haze, the
// same phase functions and source terms) is voxelized in camera-aligned
// froxels out to `fog_volume.z` metres (slices spaced quadratically) so it
// can be lit with what the closed form cannot have:
// * the sun / moon through the shadow cascades (light shafts down streets,
//   shadowed haze under arcades),
// * every clustered point and spot light, with the spot shadow maps.
// Pass 1 (`cs_inject`) writes in-scattering and extinction per froxel at a
// jittered depth and blends it with last frame's volume reprojected
// (temporal integration, 1 shadow tap + the froxel's cluster lights).
// Pass 2 (`cs_integrate`) accumulates in-scattering and transmittance front
// to back along each froxel column (energy conserving, Hillaire 2015).
// The main and sky passes then read one trilinear tap and continue with the
// analytic fog past the volume.

@group(0) @binding(1) var shadow_map: texture_depth_2d_array;
@group(0) @binding(2) var shadow_sampler: sampler_comparison;
@group(0) @binding(8) var spot_shadow_map: texture_depth_2d_array;

struct FogParams {
    grid: vec4<f32>,    // xyz: froxels, w: range (m)
    params: vec4<f32>,  // x: weight of this frame (1 = no history), y: depth jitter 0..1, z: 1 = history valid, w: unused
};

@group(1) @binding(0) var<uniform> fog: FogParams;
@group(1) @binding(1) var src_volume: texture_3d<f32>;
@group(1) @binding(2) var vol_samp: sampler;
@group(1) @binding(3) var dst_volume: texture_storage_3d<rgba16float, write>;

fn slice_depth(u: f32) -> f32 {
    return max(fog.grid.w * u * u, frame.proj_params.z * 1.01);
}

// World position at continuous froxel coordinates (x, y in froxels, z in
// slices).
fn froxel_world(g: vec3<f32>) -> vec3<f32> {
    let uv = g.xy / fog.grid.xy;
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let zv = slice_depth(g.z / fog.grid.z);
    let h = frame.inv_view_proj * vec4<f32>(ndc, frame.proj_params.z / zv, 1.0);
    return h.xyz / h.w;
}

// Sun / moon visibility through the cascades (one hardware PCF tap).
fn sun_visibility(p: vec3<f32>, view_depth: f32) -> f32 {
    if (view_depth > frame.cascade_splits[3]) { return 1.0; }
    var cascade = 3;
    for (var i = 0; i < 4; i++) {
        if (view_depth < frame.cascade_splits[i]) { cascade = i; break; }
    }
    let lc = frame.cascade_vp[cascade] * vec4<f32>(p, 1.0);
    let uv = vec2<f32>(lc.x * 0.5 + 0.5, 0.5 - lc.y * 0.5);
    if (any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0)) || lc.z > 1.0) { return 1.0; }
    let z = lc.z - frame.cascade_texel[cascade] / frame.cascade_range[cascade];
    return textureSampleCompareLevel(shadow_map, shadow_sampler, uv, cascade, z);
}

fn spot_visibility(layer: i32, p: vec3<f32>) -> f32 {
    let c = frame.spot_vp[layer] * vec4<f32>(p, 1.0);
    if (c.w <= 1e-4) { return 1.0; }
    let s = c.xyz / c.w;
    let uv = vec2<f32>(s.x * 0.5 + 0.5, 0.5 - s.y * 0.5);
    if (any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0)) || s.z > 1.0) { return 1.0; }
    return textureSampleCompareLevel(spot_shadow_map, shadow_sampler, uv, layer, s.z - 0.0005);
}

// Radiance scattered toward the camera per unit extinction from the local
// lights of the cluster at `p` (1/d^2 with the source radius and the
// froxel's own size, so a lamp inside a froxel does not blow up).
fn local_lights_at(p: vec3<f32>, dir: vec3<f32>, pix: vec2<f32>, view_depth: f32, extent: f32) -> vec3<f32> {
    if (frame.lighting.x <= 0.0 || frame.lighting.z <= 0.0) { return vec3<f32>(0.0); }
    let base = cluster_of(pix, view_depth);
    let n = min(clusters[base], 96u);
    let g = frame.scatter.x;
    var acc = vec3<f32>(0.0);
    for (var i = 0u; i < n; i++) {
        let l = local_lights[cluster_light(base, i)];
        if (l.params.w == 0.0) { continue; }
        let lx = p - l.pos_range.xyz;
        let d2 = dot(lx, lx);
        let r = l.pos_range.w;
        if (d2 >= r * r) { continue; }
        let ld = lx * inverseSqrt(max(d2, 1e-8));
        var w = spot_cone(l, ld) * range_window(d2, r) * phase_hg(dot(ld, -dir), g) / (d2 + l.color.w * l.color.w + extent * extent);
        if (w <= 0.0) { continue; }
        if (l.params.z >= 0.0) { w *= spot_visibility(i32(l.params.z + 0.5), p); }
        acc += l.color.rgb * (w * abs(l.params.w));
    }
    return acc * frame.lighting.z;
}

@compute @workgroup_size(8, 8, 1)
fn cs_inject(@builtin(global_invocation_id) id: vec3<u32>) {
    let dims = vec3<u32>(fog.grid.xyz);
    if (any(id >= dims)) { return; }
    // Per-froxel jitter in depth, rotated every frame (integrated by the
    // temporal blend); the xy jitter comes from the TAA projection.
    let pix2 = vec2<f32>(id.xy);
    let jit = fract(fog.params.y + 52.9829189 * fract(dot(pix2 + vec2<f32>(f32(id.z) * 7.0), vec2<f32>(0.06711056, 0.00583715))));
    let g = vec3<f32>(f32(id.x) + 0.5, f32(id.y) + 0.5, f32(id.z) + jit);
    let p = froxel_world(g);
    let cam = frame.camera_pos.xyz;
    let to_p = p - cam;
    let dist = length(to_p);
    let dir = to_p / max(dist, 1e-4);
    let view_depth = slice_depth(g.z / fog.grid.z);
    let sigma = fog_sigma(p.y);
    let c = dot(dir, frame.sun_dir.xyz);
    let vis = sun_visibility(p, view_depth);
    // Source function as in `height_fog_from`, with the sun shadowed.
    var j: vec3<f32>;
    if (frame.params.w > 0.5) {
        j = (frame.sun_color.rgb * (phase_hg(c, 0.7) * vis) + ambient_mean()) * 0.9;
    } else {
        j = night_fog_color(c, vis);
    }
    // Pixel of the froxel centre (cluster lookup) and its rough size.
    let pix = g.xy / fog.grid.xy * frame.screen.xy;
    let extent = 0.5 * view_depth * (frame.screen.x / fog.grid.x) / max(frame.proj_params.w, 1e-4);
    j += local_lights_at(p, dir, pix, view_depth, max(extent, 0.05));
    var cur = vec4<f32>(min(j * sigma * exposure_state.pre, vec3<f32>(60000.0)), sigma);
    // Temporal reprojection of last frame's (unjittered) volume.
    if (fog.params.z > 0.5) {
        let pc = frame.prev_view_proj * vec4<f32>(froxel_world(vec3<f32>(g.xy, f32(id.z) + 0.5)), 1.0);
        if (pc.w > 1e-4) {
            let puv = vec2<f32>(pc.x / pc.w * 0.5 + 0.5, 0.5 - pc.y / pc.w * 0.5);
            // Slice z covers u = sqrt(depth / range) in [z, z + 1] / slices.
            let pw = sqrt(clamp(pc.w / fog.grid.w, 0.0, 1.0));
            if (all(puv > vec2<f32>(0.0)) && all(puv < vec2<f32>(1.0)) && pc.w < fog.grid.w) {
                let h = textureSampleLevel(src_volume, vol_samp, vec3<f32>(puv, pw), 0.0);
                let k = exposure_state.pre / max(exposure_state.prev_pre, 1e-8);
                cur = mix(vec4<f32>(h.rgb * k, h.a), cur, fog.params.x);
            }
        }
    }
    textureStore(dst_volume, id, cur);
}

@compute @workgroup_size(8, 8, 1)
fn cs_integrate(@builtin(global_invocation_id) id: vec3<u32>) {
    let dims = vec3<u32>(fog.grid.xyz);
    if (id.x >= dims.x || id.y >= dims.y) { return; }
    var acc = vec3<f32>(0.0);
    var trans = 1.0;
    var prev = frame.camera_pos.xyz;
    let xy = vec2<f32>(f32(id.x) + 0.5, f32(id.y) + 0.5);
    for (var z = 0u; z < dims.z; z++) {
        // Integrated up to the far side of slice z.
        let p = froxel_world(vec3<f32>(xy, f32(z) + 1.0));
        let ds = length(p - prev);
        prev = p;
        let s = textureLoad(src_volume, vec3<i32>(vec3<u32>(id.xy, z)), 0);
        let sigma = max(s.a, 0.0);
        let tr = exp(-sigma * ds);
        let integ = select((s.rgb - s.rgb * tr) / max(sigma, 1e-9), s.rgb * ds, sigma < 1e-8);
        acc += trans * integ;
        trans *= tr;
        textureStore(dst_volume, vec3<u32>(id.xy, z), vec4<f32>(acc, trans));
    }
}
