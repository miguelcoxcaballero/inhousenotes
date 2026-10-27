// Ground-truth ambient occlusion (Jimenez et al. 2016, following Intel's
// XeGTAO formulation) from the depth/normal prepass, plus a 4x4 bilateral
// denoise. `common.wgsl` is prepended at load time.
//
// Cost (full resolution): 2 slices x 6 steps x 2 sides = 24 depth taps per
// pixel, ~0.5-0.8 ms at 1440p on an RTX 3070 class GPU; denoise 16 taps.

@group(1) @binding(0) var depth_tex: texture_depth_2d;
@group(1) @binding(1) var normal_tex: texture_2d<f32>;
@group(1) @binding(2) var aux_tex: texture_2d<f32>;

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

fn max_pixel() -> vec2<i32> {
    return vec2<i32>(frame.screen.xy) - vec2<i32>(1);
}

fn load_depth(p: vec2<i32>) -> f32 {
    return textureLoad(depth_tex, clamp(p, vec2<i32>(0), max_pixel()), 0);
}

// View-space position (right-handed, camera looks down -Z) of a pixel center.
fn view_pos(pix: vec2<f32>, d: f32) -> vec3<f32> {
    let ndc = vec2<f32>(pix.x * frame.screen.z * 2.0 - 1.0, 1.0 - pix.y * frame.screen.w * 2.0) - frame.jitter.xy;
    let dist = linear_depth(d);
    return vec3<f32>(ndc * frame.proj_params.xy * dist, -dist);
}

const BAYER4: array<f32, 16> = array<f32, 16>(0.0, 8.0, 2.0, 10.0, 12.0, 4.0, 14.0, 6.0, 3.0, 11.0, 1.0, 9.0, 15.0, 7.0, 13.0, 5.0);

@fragment
fn fs_gtao(in: FsIn) -> @location(0) vec4<f32> {
    let pix = in.pos.xy;
    let ip = vec2<i32>(pix);
    let d = load_depth(ip);
    if (d <= 0.0) { return vec4<f32>(1.0); }
    let p = view_pos(pix, d);
    let v = normalize(-p);
    let nw = textureLoad(normal_tex, ip, 0).xyz;
    let n = normalize((frame.view * vec4<f32>(nw, 0.0)).xyz);
    let radius = frame.ao_params.x;
    let screen_radius = radius * frame.proj_params.w / max(-p.z, 1e-3);
    if (screen_radius < 1.5) { return vec4<f32>(1.0); }
    // Clamp the footprint (cache friendliness up close); shrink the world
    // radius accordingly so the falloff stays consistent.
    let r_px = min(screen_radius, frame.screen.y * 0.18);
    let eff_radius = radius * r_px / screen_radius;
    let falloff_range = 0.615 * eff_radius;
    let falloff_mul = -1.0 / falloff_range;
    let falloff_add = eff_radius * (1.0 - 0.615) / falloff_range + 1.0;

    // 4x4 spatial pattern (removed exactly by the 4x4 denoise) rotated per
    // frame (integrated by TAA).
    var bayer = BAYER4;
    let bi = (ip.y & 3) * 4 + (ip.x & 3);
    let noise_slice = fract((bayer[bi] + 0.5) / 16.0 + frame.jitter.z * 0.618034);
    let noise_step = ign(pix);
    let min_s = 1.3 / r_px;

    const SLICES: i32 = 2;
    const STEPS: i32 = 6;
    var vis = 0.0;
    for (var s = 0; s < SLICES; s++) {
        let phi = (f32(s) + noise_slice) * PI / f32(SLICES);
        let dir = vec3<f32>(cos(phi), sin(phi), 0.0);
        let omega = vec2<f32>(dir.x, -dir.y) * r_px;
        let ortho = dir - dot(dir, v) * v;
        let axis = normalize(cross(ortho, v));
        let pn = n - axis * dot(n, axis);
        let pn_len = length(pn);
        let sign_n = select(-1.0, 1.0, dot(ortho, pn) >= 0.0);
        let cos_n = clamp(dot(pn, v) / max(pn_len, 1e-5), 0.0, 1.0);
        let nang = sign_n * acos(cos_n);
        let low0 = cos(nang + PI * 0.5);
        let low1 = cos(nang - PI * 0.5);
        var hc0 = low0;
        var hc1 = low1;
        for (var k = 0; k < STEPS; k++) {
            var t = (f32(k) + noise_step) / f32(STEPS);
            t = t * t + min_s;
            let o = omega * t;
            let p0 = floor(pix + o) + vec2<f32>(0.5);
            let p1 = floor(pix - o) + vec2<f32>(0.5);
            let s0 = view_pos(p0, load_depth(vec2<i32>(p0))) - p;
            let s1 = view_pos(p1, load_depth(vec2<i32>(p1))) - p;
            let l0 = length(s0);
            let l1 = length(s1);
            let c0 = mix(low0, dot(s0, v) / max(l0, 1e-5), clamp(l0 * falloff_mul + falloff_add, 0.0, 1.0));
            let c1 = mix(low1, dot(s1, v) / max(l1, 1e-5), clamp(l1 * falloff_mul + falloff_add, 0.0, 1.0));
            hc0 = max(hc0, c0);
            hc1 = max(hc1, c1);
        }
        let h0 = nang + clamp(-acos(clamp(hc1, -1.0, 1.0)) - nang, -PI * 0.5, PI * 0.5);
        let h1 = nang + clamp(acos(clamp(hc0, -1.0, 1.0)) - nang, -PI * 0.5, PI * 0.5);
        let sin_n = sin(nang);
        let a0 = (cos_n + 2.0 * h0 * sin_n - cos(2.0 * h0 - nang)) * 0.25;
        let a1 = (cos_n + 2.0 * h1 * sin_n - cos(2.0 * h1 - nang)) * 0.25;
        vis += pn_len * (a0 + a1);
    }
    vis = clamp(vis / f32(SLICES), 0.0, 1.0);
    return vec4<f32>(vis, 0.0, 0.0, 1.0);
}

// Edge-aware 4x4 average: depth (relative) and normal weights keep AO from
// bleeding across silhouettes.
@fragment
fn fs_ao_blur(in: FsIn) -> @location(0) vec4<f32> {
    let ip = vec2<i32>(in.pos.xy);
    let dc = load_depth(ip);
    if (dc <= 0.0) { return vec4<f32>(1.0); }
    let zc = linear_depth(dc);
    let nc = textureLoad(normal_tex, ip, 0).xyz;
    var sum = 0.0;
    var wsum = 0.0;
    for (var y = -2; y < 2; y++) {
        for (var x = -2; x < 2; x++) {
            let q = clamp(ip + vec2<i32>(x, y), vec2<i32>(0), max_pixel());
            let z = linear_depth(load_depth(q));
            let ns = textureLoad(normal_tex, q, 0).xyz;
            let wz = clamp(1.0 - abs(z - zc) / (zc * 0.03 + 0.02), 0.0, 1.0);
            let wn = pow(clamp(dot(ns, nc), 0.0, 1.0), 8.0);
            let w = wz * wn + 1e-4;
            sum += textureLoad(aux_tex, q, 0).r * w;
            wsum += w;
        }
    }
    return vec4<f32>(sum / wsum, 0.0, 0.0, 1.0);
}
