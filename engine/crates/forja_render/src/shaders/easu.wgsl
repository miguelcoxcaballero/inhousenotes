// Edge-adaptive spatial upsampling: a WGSL port of EASU from AMD
// FidelityFX Super Resolution 1.0 (T. Lottes, ffx_fsr1.h, MIT licence).
// 12 taps around the output pixel's source position estimate a local edge
// direction and length from luma gradients of the four nearest 2x2 cells,
// then a Lanczos-2-like kernel is stretched along the edge and clamped to
// the 2x2 neighbourhood (no ringing). Appended to post.wgsl and
// cinematic.wgsl: the last pass that changes resolution uses it when the
// internal resolution is below the output (render scale < 1).
// Cost: 12 loads + ~150 ALU per output pixel (~0.25 ms at 1440p output on a
// mid-range GPU; ~0.5 ms at 2880x1800 on an Intel Arc iGPU).

fn easu_luma(c: vec3<f32>) -> f32 {
    return c.b * 0.5 + (c.r * 0.5 + c.g);
}

struct EasuAcc {
    dir: vec2<f32>,
    len: f32,
};

fn easu_set(acc: EasuAcc, w: f32, la: f32, lb: f32, lc: f32, ld: f32, le: f32) -> EasuAcc {
    var o = acc;
    let dc = ld - lc;
    let cb = lc - lb;
    let dir_x = ld - lb;
    var len_x = clamp(abs(dir_x) / max(max(abs(dc), abs(cb)), 1e-5), 0.0, 1.0);
    len_x *= len_x;
    let ec = le - lc;
    let ca = lc - la;
    let dir_y = le - la;
    var len_y = clamp(abs(dir_y) / max(max(abs(ec), abs(ca)), 1e-5), 0.0, 1.0);
    len_y *= len_y;
    o.dir += vec2<f32>(dir_x, dir_y) * w;
    o.len += (len_x + len_y) * w;
    return o;
}

fn easu_weight(off: vec2<f32>, dir: vec2<f32>, len: vec2<f32>, lob: f32, clp: f32) -> f32 {
    var v = vec2<f32>(off.x * dir.x + off.y * dir.y, off.x * -dir.y + off.y * dir.x);
    v *= len;
    let d2 = min(dot(v, v), clp);
    var wb = 0.4 * d2 - 1.0;
    var wa = lob * d2 - 1.0;
    wb *= wb;
    wa *= wa;
    wb = 1.5625 * wb - 0.5625;
    return wb * wa;
}

fn easu_load(t: texture_2d<f32>, p: vec2<i32>, m: vec2<i32>) -> vec3<f32> {
    return textureLoad(t, clamp(p, vec2<i32>(0), m), 0).rgb;
}

// Upsampled colour of `t` at `uv` (output pixel centre).
fn easu(t: texture_2d<f32>, uv: vec2<f32>) -> vec3<f32> {
    let size = vec2<f32>(textureDimensions(t));
    let m = vec2<i32>(size) - vec2<i32>(1);
    let pp0 = uv * size - vec2<f32>(0.5);
    let fp = floor(pp0);
    let pp = pp0 - fp;
    let f0 = vec2<i32>(fp);
    //    b c
    //  e f g h
    //  i j k l
    //    n o
    let b = easu_load(t, f0 + vec2<i32>(0, -1), m);
    let c = easu_load(t, f0 + vec2<i32>(1, -1), m);
    let e = easu_load(t, f0 + vec2<i32>(-1, 0), m);
    let f = easu_load(t, f0, m);
    let g = easu_load(t, f0 + vec2<i32>(1, 0), m);
    let h = easu_load(t, f0 + vec2<i32>(2, 0), m);
    let i = easu_load(t, f0 + vec2<i32>(-1, 1), m);
    let j = easu_load(t, f0 + vec2<i32>(0, 1), m);
    let k = easu_load(t, f0 + vec2<i32>(1, 1), m);
    let l = easu_load(t, f0 + vec2<i32>(2, 1), m);
    let n = easu_load(t, f0 + vec2<i32>(0, 2), m);
    let o = easu_load(t, f0 + vec2<i32>(1, 2), m);
    let bl = easu_luma(b);
    let cl = easu_luma(c);
    let el = easu_luma(e);
    let fl = easu_luma(f);
    let gl = easu_luma(g);
    let hl = easu_luma(h);
    let il = easu_luma(i);
    let jl = easu_luma(j);
    let kl = easu_luma(k);
    let ll = easu_luma(l);
    let nl = easu_luma(n);
    let ol = easu_luma(o);
    var acc = EasuAcc(vec2<f32>(0.0), 0.0);
    acc = easu_set(acc, (1.0 - pp.x) * (1.0 - pp.y), bl, el, fl, gl, jl);
    acc = easu_set(acc, pp.x * (1.0 - pp.y), cl, fl, gl, hl, kl);
    acc = easu_set(acc, (1.0 - pp.x) * pp.y, fl, il, jl, kl, nl);
    acc = easu_set(acc, pp.x * pp.y, gl, jl, kl, ll, ol);
    var dir = acc.dir;
    let dir_r2 = dot(dir, dir);
    if (dir_r2 < 1.0 / 32768.0) {
        dir = vec2<f32>(1.0, 0.0);
    } else {
        dir *= inverseSqrt(dir_r2);
    }
    var len = acc.len * 0.5;
    len *= len;
    let stretch = dot(dir, dir) / max(abs(dir.x), abs(dir.y));
    let len2 = vec2<f32>(1.0 + (stretch - 1.0) * len, 1.0 - 0.5 * len);
    let lob = 0.5 + (0.25 - 0.04 - 0.5) * len;
    let clp = 1.0 / lob;
    let mn = min(min(f, g), min(j, k));
    let mx = max(max(f, g), max(j, k));
    var sum = vec3<f32>(0.0);
    var wsum = 0.0;
    var offs = array<vec2<f32>, 12>(
        vec2<f32>(0.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(-1.0, 0.0), vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0), vec2<f32>(2.0, 0.0), vec2<f32>(-1.0, 1.0), vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 1.0), vec2<f32>(2.0, 1.0), vec2<f32>(0.0, 2.0), vec2<f32>(1.0, 2.0));
    var cols = array<vec3<f32>, 12>(b, c, e, f, g, h, i, j, k, l, n, o);
    for (var q = 0; q < 12; q++) {
        let w = easu_weight(offs[q] - pp, dir, len2, lob, clp);
        sum += cols[q] * w;
        wsum += w;
    }
    return clamp(sum / max(wsum, 1e-5), mn, mx);
}

// True when this full-screen pass writes more pixels than `t` has (the
// output pixel size comes from the uv derivatives, uniform in the pass).
fn easu_upscaling(t: texture_2d<f32>, uv: vec2<f32>) -> bool {
    let out_w = 1.0 / max(abs(dpdx(uv.x)), 1e-6);
    return out_w > f32(textureDimensions(t).x) * 1.01;
}
