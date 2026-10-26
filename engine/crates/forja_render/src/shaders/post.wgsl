// Bloom (Jimenez 2014 down/up chain), camera tone curve and FXAA.
struct PostParams {
    texel: vec4<f32>,   // xy: source texel size, z: 1 / bloom levels
    params: vec4<f32>,  // x: exposure (tonemap: exposure at EV100 0), y: bloom strength, z: vignette, w: first-mip Karis flag
    curve: vec4<f32>,   // tone curve x^a / (x^(a d) b + c): a, a d, b, c
    look: vec4<f32>,    // x: saturation, y: highlight desaturation
    white: vec4<f32>,   // camera white balance gains (rgb)
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var<uniform> post: PostParams;
@group(0) @binding(3) var bloom_tex: texture_2d<f32>;
@group(0) @binding(4) var exposure_tex: texture_2d<f32>;

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

@fragment
fn fs_downsample(in: FsIn) -> @location(0) vec4<f32> {
    let t = post.texel.xy;
    let uv = in.uv;
    let a = textureSample(src, samp, uv + t * vec2<f32>(-2.0, 2.0)).rgb;
    let b = textureSample(src, samp, uv + t * vec2<f32>(0.0, 2.0)).rgb;
    let c = textureSample(src, samp, uv + t * vec2<f32>(2.0, 2.0)).rgb;
    let d = textureSample(src, samp, uv + t * vec2<f32>(-2.0, 0.0)).rgb;
    let e = textureSample(src, samp, uv).rgb;
    let f = textureSample(src, samp, uv + t * vec2<f32>(2.0, 0.0)).rgb;
    let g = textureSample(src, samp, uv + t * vec2<f32>(-2.0, -2.0)).rgb;
    let h = textureSample(src, samp, uv + t * vec2<f32>(0.0, -2.0)).rgb;
    let i = textureSample(src, samp, uv + t * vec2<f32>(2.0, -2.0)).rgb;
    let j = textureSample(src, samp, uv + t * vec2<f32>(-1.0, 1.0)).rgb;
    let k = textureSample(src, samp, uv + t * vec2<f32>(1.0, 1.0)).rgb;
    let l = textureSample(src, samp, uv + t * vec2<f32>(-1.0, -1.0)).rgb;
    let m = textureSample(src, samp, uv + t * vec2<f32>(1.0, -1.0)).rgb;
    var c0 = e * 0.125 + (a + c + g + i) * 0.03125 + (b + d + f + h) * 0.0625 + (j + k + l + m) * 0.125;
    if (post.params.w > 0.5) {
        // Karis average on the first mip: weight each 2x2 group by
        // 1 / (1 + exposed luma) and renormalise, so sun glints and other
        // single-pixel fireflies cannot flood the glare, while the energy of
        // smooth areas is kept.
        let g0 = min((j + k + l + m) * 0.25, vec3<f32>(6e4));
        let g1 = min((a + b + d + e) * 0.25, vec3<f32>(6e4));
        let g2 = min((b + c + e + f) * 0.25, vec3<f32>(6e4));
        let g3 = min((d + e + g + h) * 0.25, vec3<f32>(6e4));
        let g4 = min((e + f + h + i) * 0.25, vec3<f32>(6e4));
        let x = post.params.x;
        let w0 = 0.5 / (1.0 + luma(g0) * x);
        let w1 = 0.125 / (1.0 + luma(g1) * x);
        let w2 = 0.125 / (1.0 + luma(g2) * x);
        let w3 = 0.125 / (1.0 + luma(g3) * x);
        let w4 = 0.125 / (1.0 + luma(g4) * x);
        c0 = (g0 * w0 + g1 * w1 + g2 * w2 + g3 * w3 + g4 * w4) / (w0 + w1 + w2 + w3 + w4);
    }
    // Half floats overflow on mirror-like sun glints; keep the chain finite.
    return vec4<f32>(clamp(c0, vec3<f32>(0.0), vec3<f32>(6e4)), 1.0);
}

@fragment
fn fs_upsample(in: FsIn) -> @location(0) vec4<f32> {
    let t = post.texel.xy;
    let uv = in.uv;
    var c = textureSample(src, samp, uv).rgb * 4.0;
    c += (textureSample(src, samp, uv + vec2<f32>(-t.x, 0.0)).rgb + textureSample(src, samp, uv + vec2<f32>(t.x, 0.0)).rgb
        + textureSample(src, samp, uv + vec2<f32>(0.0, -t.y)).rgb + textureSample(src, samp, uv + vec2<f32>(0.0, t.y)).rgb) * 2.0;
    c += textureSample(src, samp, uv + vec2<f32>(-t.x, -t.y)).rgb + textureSample(src, samp, uv + vec2<f32>(t.x, -t.y)).rgb
        + textureSample(src, samp, uv + vec2<f32>(-t.x, t.y)).rgb + textureSample(src, samp, uv + vec2<f32>(t.x, t.y)).rgb;
    return vec4<f32>(c / 16.0, 1.0);
}

// Camera meter: center-weighted average luminance of the frame, read from
// the smallest bloom mip (a 1/64-resolution blur). It outputs the EV100 a
// reflected-light meter reads (K = 12.5), kept within the allowed range
// around the incident reading; the pipeline blends it with the previous
// frame's EV, which is the camera adapting.
// Meter uniform: texel = (manual flag, manual EV, compensation, -),
// params = (lux per unit * 100 / K, incident EV, max opening, max closing).
@fragment
fn fs_meter(in: FsIn) -> @location(0) vec4<f32> {
    let dim = textureDimensions(src, 0);
    var sum = 0.0;
    var wsum = 0.0;
    for (var y = 0u; y < dim.y; y++) {
        for (var x = 0u; x < dim.x; x++) {
            let p = (vec2<f32>(f32(x), f32(y)) + 0.5) / vec2<f32>(dim) - 0.5;
            let w = exp(-dot(p, p) * 6.25);
            sum += luma(textureLoad(src, vec2<u32>(x, y), 0).rgb) * w;
            wsum += w;
        }
    }
    var ev = log2(max(sum / wsum, 1e-6) * post.params.x);
    ev = clamp(ev, post.params.y - post.params.z, post.params.y + post.params.w);
    if (post.texel.x > 0.5) { ev = post.texel.y; }
    ev -= post.texel.z;
    return vec4<f32>(ev, ev, ev, 1.0);
}

// Physical camera display transform (see tonemap.rs, which mirrors it):
// a camera JPEG-like curve on max(r, g, b) that keeps the RGB ratios, so
// hue and saturation survive; colours whiten only as the curve compresses
// them, like a saturating sensor.
fn camera_curve(x: f32) -> f32 {
    return clamp(pow(x, post.curve.x) / (pow(x, post.curve.y) * post.curve.z + post.curve.w), 0.0, 1.0);
}

fn tonemap(c_in: vec3<f32>) -> vec3<f32> {
    let c = max(c_in, vec3<f32>(0.0));
    let peak = max(max(c.r, c.g), c.b);
    if (peak <= 1e-8) { return vec3<f32>(0.0); }
    let t = camera_curve(peak);
    var ratio = c / peak;
    let l = luma(ratio);
    ratio = max(vec3<f32>(l) + (ratio - vec3<f32>(l)) * post.look.x, vec3<f32>(0.0));
    ratio = ratio / max(max(ratio.r, ratio.g), max(ratio.b, 1e-6));
    let g = 1.0 - 1.0 / (post.look.y * max(peak - t, 0.0) + 1.0);
    return mix(ratio, vec3<f32>(1.0), g) * t;
}

@fragment
fn fs_tonemap(in: FsIn) -> @location(0) vec4<f32> {
    let hdr = textureSample(src, samp, in.uv).rgb;
    // The up chain adds every level, so mip 0 holds the sum of
    // BLOOM_MIPS mean-preserving blurs of growing radius; dividing by the
    // count makes it one energy-conserving glare kernel (equal energy per
    // octave, a ~1/r^2 point spread like a real lens' veiling glare).
    let bloom = textureSample(bloom_tex, samp, in.uv).rgb * post.texel.z;
    // Exposure from the camera's EV100: params.x = 0.18 * lux per unit * 100 / K.
    let ev = textureLoad(exposure_tex, vec2<u32>(0u, 0u), 0).r;
    var c = mix(hdr, bloom, post.params.y) * (post.params.x * exp2(-ev)) * post.white.rgb;
    // Display-linear out; the sRGB target encodes it.
    c = tonemap(c);
    let d = in.uv - vec2<f32>(0.5);
    c *= 1.0 - dot(d, d) * post.params.z;
    // Store perceptual luma in alpha for FXAA.
    return vec4<f32>(c, sqrt(luma(c)));
}

// With TAA (params.x = 0): contrast-adaptive sharpening (AMD CAS style,
// 5 taps) to restore the detail the temporal filter softens, limited to the
// local min/max so it never rings. Strength in params.y.
fn sharpen(uv: vec2<f32>, t: vec2<f32>) -> vec4<f32> {
    let c = textureSampleLevel(src, samp, uv, 0.0).rgb;
    let n = textureSampleLevel(src, samp, uv + vec2<f32>(0.0, -t.y), 0.0).rgb;
    let s = textureSampleLevel(src, samp, uv + vec2<f32>(0.0, t.y), 0.0).rgb;
    let e = textureSampleLevel(src, samp, uv + vec2<f32>(t.x, 0.0), 0.0).rgb;
    let w = textureSampleLevel(src, samp, uv + vec2<f32>(-t.x, 0.0), 0.0).rgb;
    let mn = min(c, min(min(n, s), min(e, w)));
    let mx = max(c, max(max(n, s), max(e, w)));
    // Less sharpening where local contrast is already high.
    let amp = sqrt(clamp(min(mn, vec3<f32>(1.0) - mx) / max(mx, vec3<f32>(1e-4)), vec3<f32>(0.0), vec3<f32>(1.0)));
    let k = -amp * post.params.y * 0.2;
    let r = (c + (n + s + e + w) * k) / (vec3<f32>(1.0) + 4.0 * k);
    return encode_output(clamp(r, mn, mx));
}

// The output is display-linear for an sRGB target; a plain UNORM surface
// (params.z = 1) gets the sRGB encoding here instead of washed-dark values.
fn encode_output(c: vec3<f32>) -> vec4<f32> {
    if (post.params.z > 0.5) {
        let x = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
        return vec4<f32>(select(1.055 * pow(x, vec3<f32>(1.0 / 2.4)) - 0.055, x * 12.92, x <= vec3<f32>(0.0031308)), 1.0);
    }
    return vec4<f32>(c, 1.0);
}

// FXAA 3.11 quality preset (simplified), reads the tonemapped linear image.
@fragment
fn fs_fxaa(in: FsIn) -> @location(0) vec4<f32> {
    let t = post.texel.xy;
    let uv = in.uv;
    if (post.params.x < 0.5) { return sharpen(uv, t); }
    let cm = textureSampleLevel(src, samp, uv, 0.0);
    let lm = cm.a;
    let ln = textureSampleLevel(src, samp, uv + vec2<f32>(0.0, -t.y), 0.0).a;
    let ls = textureSampleLevel(src, samp, uv + vec2<f32>(0.0, t.y), 0.0).a;
    let le = textureSampleLevel(src, samp, uv + vec2<f32>(t.x, 0.0), 0.0).a;
    let lw = textureSampleLevel(src, samp, uv + vec2<f32>(-t.x, 0.0), 0.0).a;
    let lmax = max(lm, max(max(ln, ls), max(le, lw)));
    let lmin = min(lm, min(min(ln, ls), min(le, lw)));
    let range = lmax - lmin;
    if (range < max(0.0312, lmax * 0.125)) { return encode_output(cm.rgb); }
    let lne = textureSampleLevel(src, samp, uv + vec2<f32>(t.x, -t.y), 0.0).a;
    let lnw = textureSampleLevel(src, samp, uv + vec2<f32>(-t.x, -t.y), 0.0).a;
    let lse = textureSampleLevel(src, samp, uv + vec2<f32>(t.x, t.y), 0.0).a;
    let lsw = textureSampleLevel(src, samp, uv + vec2<f32>(-t.x, t.y), 0.0).a;
    let horizontal = abs(ln + ls - 2.0 * lm) * 2.0 + abs(lne + lse - 2.0 * le) + abs(lnw + lsw - 2.0 * lw)
        >= abs(le + lw - 2.0 * lm) * 2.0 + abs(lne + lnw - 2.0 * ln) + abs(lse + lsw - 2.0 * ls);
    let p = select(lw, ln, horizontal);
    let q = select(le, ls, horizontal);
    let gp = abs(p - lm);
    let gq = abs(q - lm);
    var step_len = select(t.x, t.y, horizontal);
    var l_edge = q;
    if (gp >= gq) { step_len = -step_len; l_edge = p; }
    let grad = max(gp, gq) * 0.25;
    var e_uv = uv;
    if (horizontal) { e_uv.y += step_len * 0.5; } else { e_uv.x += step_len * 0.5; }
    let along = select(vec2<f32>(0.0, t.y), vec2<f32>(t.x, 0.0), horizontal);
    let l_avg = 0.5 * (lm + l_edge);
    var uv_p = e_uv + along;
    var uv_n = e_uv - along;
    var dp = textureSampleLevel(src, samp, uv_p, 0.0).a - l_avg;
    var dn = textureSampleLevel(src, samp, uv_n, 0.0).a - l_avg;
    var done_p = abs(dp) >= grad;
    var done_n = abs(dn) >= grad;
    let steps = array<f32, 8>(1.0, 1.5, 2.0, 2.0, 2.0, 4.0, 8.0, 8.0);
    for (var i = 0; i < 8; i++) {
        if (done_p && done_n) { break; }
        if (!done_p) { uv_p += along * steps[i]; dp = textureSampleLevel(src, samp, uv_p, 0.0).a - l_avg; done_p = abs(dp) >= grad; }
        if (!done_n) { uv_n -= along * steps[i]; dn = textureSampleLevel(src, samp, uv_n, 0.0).a - l_avg; done_n = abs(dn) >= grad; }
    }
    let dist_p = select(uv_p.y - uv.y, uv_p.x - uv.x, horizontal);
    let dist_n = select(uv.y - uv_n.y, uv.x - uv_n.x, horizontal);
    let closer_p = dist_p < dist_n;
    let dist = min(dist_p, dist_n);
    let span = dist_p + dist_n;
    let good = (select(dn, dp, closer_p) < 0.0) != (lm - l_avg < 0.0);
    var offset = select(0.0, 0.5 - dist / span, good);
    // Sub-pixel blending.
    let l_full = (2.0 * (ln + ls + le + lw) + lne + lnw + lse + lsw) / 12.0;
    let sub = clamp(abs(l_full - lm) / range, 0.0, 1.0);
    let sub2 = (-2.0 * sub + 3.0) * sub * sub;
    offset = max(offset, sub2 * sub2 * 0.75);
    var final_uv = uv;
    if (horizontal) { final_uv.y += offset * step_len; } else { final_uv.x += offset * step_len; }
    return encode_output(textureSampleLevel(src, samp, final_uv, 0.0).rgb);
}
