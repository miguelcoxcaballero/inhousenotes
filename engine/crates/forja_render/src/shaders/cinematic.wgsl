// Camera and film stage after tonemapping and anti-aliasing (see look.rs):
// lateral chromatic aberration, cos^4 lens vignette, 3D LUT colour grade
// and luminance-dependent temporal film grain, in one full-screen pass.
// Cost: 3 colour taps + 1 trilinear 3D LUT tap + ~30 ALU per pixel
// (~0.1-0.15 ms at 1440p).

struct LookParams {
    a: vec4<f32>,  // x: grade strength, y: grain amount, z: grain size (px), w: vignette
    b: vec4<f32>,  // x: aberration (px at the corners), y: frame index, zw: 1 / size
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var<uniform> look: LookParams;
@group(0) @binding(3) var lut: texture_3d<f32>;

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

fn to_srgb(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3<f32>(0.0031308));
}

fn from_srgb(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((max(c, vec3<f32>(0.0)) + 0.055) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

// Integer hash (PCG-style) to [0, 1).
fn hash3(p: vec3<u32>) -> f32 {
    var v = p * 1664525u + 1013904223u;
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    v = v ^ (v >> vec3<u32>(16u));
    v.x += v.y * v.z;
    return f32(v.x >> 8u) / 16777216.0;
}

// Smooth value noise of unit-variance-ish grain at `p` (grain cells).
fn grain_noise(p: vec2<f32>, frame: u32) -> f32 {
    let i = floor(p);
    let f = p - i;
    let u = f * f * (3.0 - 2.0 * f);
    let c = vec2<u32>(vec2<i32>(i) + vec2<i32>(65536));
    let a = hash3(vec3<u32>(c, frame));
    let b = hash3(vec3<u32>(c + vec2<u32>(1u, 0u), frame));
    let d = hash3(vec3<u32>(c + vec2<u32>(0u, 1u), frame));
    let e = hash3(vec3<u32>(c + vec2<u32>(1u, 1u), frame));
    // Bilinear blend of uniform noise has variance ~0.44 of the input:
    // rescale to keep the amplitude independent of the grain size.
    return (mix(mix(a, b, u.x), mix(d, e, u.x), u.y) - 0.5) * 2.6;
}

@fragment
fn fs_camera(in: FsIn) -> @location(0) vec4<f32> {
    let size = 1.0 / look.b.zw;
    let res = size.y / 1080.0;
    let uv = in.uv;
    // Centre-relative coordinates, aspect correct, 1 at the corners.
    let d = (uv - vec2<f32>(0.5)) * size;
    let half_diag = 0.5 * length(size);
    let r = length(d) / half_diag;
    // Lateral chromatic aberration grows with the square of the image
    // height (a short lens's dominant transverse colour error).
    var c: vec3<f32>;
    let ca = look.b.x * res;
    if (ca > 0.01) {
        let shift = d / max(length(d), 1e-4) * ca * r * r * look.b.zw;
        c = vec3<f32>(
            textureSampleLevel(src, samp, uv - shift, 0.0).r,
            textureSampleLevel(src, samp, uv, 0.0).g,
            textureSampleLevel(src, samp, uv + shift, 0.0).b);
    } else {
        c = textureSampleLevel(src, samp, uv, 0.0).rgb;
    }
    // Natural vignetting: cos^4 of the field angle (a ~40 degree half
    // diagonal at full strength).
    let t = r * 0.84 * look.a.w;
    let cos2 = 1.0 / (1.0 + t * t);
    c *= cos2 * cos2;
    // Grade in display space through the LUT (texel-centre addressing).
    var e = clamp(to_srgb(c), vec3<f32>(0.0), vec3<f32>(1.0));
    if (look.a.x > 0.0) {
        let n = f32(textureDimensions(lut).x);
        let g = textureSampleLevel(lut, samp, e * ((n - 1.0) / n) + vec3<f32>(0.5 / n), 0.0).rgb;
        e = mix(e, g, look.a.x);
    }
    // Film grain: mostly luminance noise (a little chroma), strongest in
    // the mid-tones and fading in deep shadows and near white, re-seeded
    // every frame.
    if (look.a.y > 0.0) {
        let frame = u32(look.b.y);
        let p = in.pos.xy / max(look.a.z * res, 0.5);
        let gl = grain_noise(p, frame);
        let gc = vec3<f32>(grain_noise(p + vec2<f32>(17.3, 5.1), frame + 7u), 0.0, grain_noise(p + vec2<f32>(3.7, 29.9), frame + 13u));
        let l = dot(e, vec3<f32>(0.2126, 0.7152, 0.0722));
        let w = look.a.y * (0.25 + 3.0 * l * (1.0 - l));
        e = clamp(e + (vec3<f32>(gl) + gc * 0.15) * w, vec3<f32>(0.0), vec3<f32>(1.0));
    }
    return vec4<f32>(from_srgb(e), 1.0);
}
