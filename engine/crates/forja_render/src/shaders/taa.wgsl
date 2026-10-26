// Temporal anti-aliasing resolve (Karis 2014 / Salvi variance clipping).
// The scene is rendered with a Halton(2,3) sub-pixel jitter; history is
// reprojected with camera motion from the closest depth of the 3x3
// neighborhood, sampled with a 5-tap Catmull-Rom filter and clipped to the
// current neighborhood's color distribution in YCoCg. Blending happens on
// luma-weighted colors so fireflies cannot dominate.
// A still camera (`params.w` = 1) accumulates a running average instead, so
// static screenshots converge to a supersampled image.
// `common.wgsl` is prepended at load time.
//
// Cost: 9 + 9 color/depth loads + 5 bilinear history taps per pixel,
// ~0.25 ms at 1440p.

struct TaaParams {
    params: vec4<f32>,  // x: weight of the current frame, y: clip gamma, z: 1 = history valid, w: 1 = still camera
};

@group(1) @binding(0) var cur_tex: texture_2d<f32>;
@group(1) @binding(1) var hist_tex: texture_2d<f32>;
@group(1) @binding(2) var depth_tex: texture_depth_2d;
@group(1) @binding(3) var lin_samp: sampler;
@group(1) @binding(4) var<uniform> taa: TaaParams;

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

fn luma(c: vec3<f32>) -> f32 { return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722)); }
fn tm(c: vec3<f32>) -> vec3<f32> { return c / (1.0 + luma(c)); }
fn itm(c: vec3<f32>) -> vec3<f32> { return c / max(1.0 - luma(c), 1e-4); }
fn to_ycocg(c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(0.25 * c.r + 0.5 * c.g + 0.25 * c.b, 0.5 * c.r - 0.5 * c.b, -0.25 * c.r + 0.5 * c.g - 0.25 * c.b);
}
fn from_ycocg(c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(c.x + c.y - c.z, c.x + c.z, c.x - c.y - c.z);
}

fn load_color(p: vec2<i32>) -> vec3<f32> {
    let q = clamp(p, vec2<i32>(0), vec2<i32>(frame.screen.xy) - vec2<i32>(1));
    return max(textureLoad(cur_tex, q, 0).rgb, vec3<f32>(0.0));
}

fn history_catmull_rom(uv: vec2<f32>) -> vec3<f32> {
    let size = frame.screen.xy;
    let sp = uv * size;
    let t1 = floor(sp - 0.5) + 0.5;
    let f = sp - t1;
    let w0 = f * (-0.5 + f * (1.0 - 0.5 * f));
    let w1 = 1.0 + f * f * (-2.5 + 1.5 * f);
    let w2 = f * (0.5 + f * (2.0 - 1.5 * f));
    let w3 = f * f * (-0.5 + 0.5 * f);
    let w12 = w1 + w2;
    let t0 = (t1 - 1.0) / size;
    let t3 = (t1 + 2.0) / size;
    let t12 = (t1 + w2 / w12) / size;
    var r = textureSampleLevel(hist_tex, lin_samp, vec2<f32>(t12.x, t0.y), 0.0).rgb * (w12.x * w0.y);
    r += textureSampleLevel(hist_tex, lin_samp, vec2<f32>(t0.x, t12.y), 0.0).rgb * (w0.x * w12.y);
    r += textureSampleLevel(hist_tex, lin_samp, t12, 0.0).rgb * (w12.x * w12.y);
    r += textureSampleLevel(hist_tex, lin_samp, vec2<f32>(t3.x, t12.y), 0.0).rgb * (w3.x * w12.y);
    r += textureSampleLevel(hist_tex, lin_samp, vec2<f32>(t12.x, t3.y), 0.0).rgb * (w12.x * w3.y);
    let wsum = w12.x * w0.y + w0.x * w12.y + w12.x * w12.y + w3.x * w12.y + w12.x * w3.y;
    return max(r / wsum, vec3<f32>(0.0));
}

@fragment
fn fs_taa(in: FsIn) -> @location(0) vec4<f32> {
    let ip = vec2<i32>(in.pos.xy);
    let cur = load_color(ip);
    if (taa.params.z < 0.5) { return vec4<f32>(cur, 1.0); }
    var m1 = vec3<f32>(0.0);
    var m2 = vec3<f32>(0.0);
    var best_d = -1.0;
    var best = vec2<i32>(0);
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let o = vec2<i32>(x, y);
            let c = to_ycocg(tm(load_color(ip + o)));
            m1 += c;
            m2 += c * c;
            let q = clamp(ip + o, vec2<i32>(0), vec2<i32>(frame.screen.xy) - vec2<i32>(1));
            let d = textureLoad(depth_tex, q, 0);
            if (d > best_d) { best_d = d; best = o; }
        }
    }
    let mean = m1 / 9.0;
    let sigma = sqrt(max(m2 / 9.0 - mean * mean, vec3<f32>(0.0)));
    // Reproject the closest surface of the neighborhood (camera motion).
    let pc = in.pos.xy + vec2<f32>(best);
    let ndc = vec2<f32>(pc.x * frame.screen.z * 2.0 - 1.0, 1.0 - pc.y * frame.screen.w * 2.0);
    let world = frame.inv_view_proj * vec4<f32>(ndc, best_d, 1.0);
    let prev = frame.prev_view_proj * world;
    if (prev.w <= 1e-6) { return vec4<f32>(cur, 1.0); }
    let pn = prev.xy / prev.w;
    let prev_pix = vec2<f32>((pn.x * 0.5 + 0.5) * frame.screen.x, (0.5 - pn.y * 0.5) * frame.screen.y);
    // Motion between the unjittered projections: this frame's jitter must
    // not move the history, or a still camera would resample (and drift)
    // it every frame.
    let jitter_pix = frame.jitter.xy * vec2<f32>(0.5, -0.5) * frame.screen.xy;
    let hist_pix = in.pos.xy + (prev_pix - (pc - jitter_pix));
    if (any(hist_pix < vec2<f32>(0.0)) || any(hist_pix > frame.screen.xy)) { return vec4<f32>(cur, 1.0); }
    let hist = to_ycocg(tm(history_catmull_rom(hist_pix * frame.screen.zw)));
    // Variance clipping toward the neighborhood mean.
    let gamma = taa.params.y;
    let lo = mean - sigma * gamma;
    let hi = mean + sigma * gamma;
    let center = (lo + hi) * 0.5;
    let ext = max((hi - lo) * 0.5, vec3<f32>(1e-5));
    let off = hist - center;
    let ts = abs(off / ext);
    let m = max(ts.x, max(ts.y, ts.z));
    let clipped = select(hist, center + off / m, m > 1.0);
    let c = to_ycocg(tm(cur));
    // A moving camera resamples the history every frame, which blurs it:
    // trust the current frame more the further the history moved (0.1 at
    // rest up to 0.25 from 3 px), trading a little aliasing for detail.
    let motion = length(hist_pix - in.pos.xy);
    let w_cur = select(taa.params.x, clamp(taa.params.x * (1.0 + 0.5 * motion), taa.params.x, 0.25), taa.params.w < 0.5);
    let outc = mix(clipped, c, w_cur);
    return vec4<f32>(itm(from_ycocg(outc)), 1.0);
}
