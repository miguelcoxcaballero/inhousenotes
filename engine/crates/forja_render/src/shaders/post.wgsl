// Bloom (Jimenez 2014 down/up chain), AgX tonemapping and FXAA.
struct PostParams {
    texel: vec4<f32>,   // xy: source texel size
    params: vec4<f32>,  // x: exposure, y: bloom strength, z: vignette, w: first-mip threshold flag
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var<uniform> post: PostParams;
@group(0) @binding(3) var bloom_tex: texture_2d<f32>;

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
        // Karis average on the first mip kills fireflies.
        c0 = c0 / (1.0 + luma(c0) * 0.25);
    }
    return vec4<f32>(max(c0, vec3<f32>(0.0)), 1.0);
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

// AgX (Troy Sobotka), polynomial fit by Benjamin Wrensch.
fn agx_contrast(x: vec3<f32>) -> vec3<f32> {
    let x2 = x * x;
    let x4 = x2 * x2;
    return 15.5 * x4 * x2 - 40.14 * x4 * x + 31.96 * x4 - 6.868 * x2 * x + 0.4298 * x2 + 0.1191 * x - vec3<f32>(0.00232);
}

fn agx(c: vec3<f32>) -> vec3<f32> {
    let m_in = mat3x3<f32>(
        vec3<f32>(0.842479062253094, 0.0423282422610123, 0.0423756549057051),
        vec3<f32>(0.0784335999999992, 0.878468636469772, 0.0784336),
        vec3<f32>(0.0792237451477643, 0.0791661274605434, 0.879142973793104));
    let m_out = mat3x3<f32>(
        vec3<f32>(1.19687900512017, -0.0528968517574562, -0.0529716355144438),
        vec3<f32>(-0.0980208811401368, 1.15190312990417, -0.0980434501171241),
        vec3<f32>(-0.0990297440797205, -0.0989611768448433, 1.15107367264116));
    let min_ev = -12.47393;
    let max_ev = 4.026069;
    var v = m_in * max(c, vec3<f32>(1e-10));
    v = clamp((log2(v) - vec3<f32>(min_ev)) / (max_ev - min_ev), vec3<f32>(0.0), vec3<f32>(1.0));
    v = agx_contrast(v);
    // "Punchy" look: a touch more saturation.
    let l = luma(v);
    v = vec3<f32>(l) + (v - vec3<f32>(l)) * 1.12;
    v = m_out * v;
    // The fit outputs display-encoded values; linearize for the sRGB target.
    return pow(clamp(v, vec3<f32>(0.0), vec3<f32>(1.0)), vec3<f32>(2.2));
}

@fragment
fn fs_tonemap(in: FsIn) -> @location(0) vec4<f32> {
    let hdr = textureSample(src, samp, in.uv).rgb;
    let bloom = textureSample(bloom_tex, samp, in.uv).rgb;
    var c = mix(hdr, bloom, post.params.y) * post.params.x;
    c = agx(c);
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
    return vec4<f32>(clamp(r, mn, mx), 1.0);
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
    if (range < max(0.0312, lmax * 0.125)) { return vec4<f32>(cm.rgb, 1.0); }
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
    return vec4<f32>(textureSampleLevel(src, samp, final_uv, 0.0).rgb, 1.0);
}
