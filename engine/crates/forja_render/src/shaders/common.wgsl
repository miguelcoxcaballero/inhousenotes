const PI: f32 = 3.14159265359;
// Shared frame data. Must match `FrameUniforms` in frame.rs.
struct Light {
    pos_radius: vec4<f32>,
    color: vec4<f32>,
};

struct Frame {
    view_proj: mat4x4<f32>,
    view: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,      // w: time (s)
    sun_dir: vec4<f32>,         // toward the sun; w: sun illuminance scale
    sun_color: vec4<f32>,       // transmitted sun radiance; w: exposure
    sky_zenith: vec4<f32>,      // ambient sky colors for lighting
    sky_horizon: vec4<f32>,     // w: fog density
    ground_color: vec4<f32>,    // w: fog height falloff
    cascade_splits: vec4<f32>,  // view-space far distance per cascade
    cascade_vp: array<mat4x4<f32>, 4>,
    params: vec4<f32>,          // x: light count, y: 1 / shadow size, z: bloom strength, w: debug view
    screen: vec4<f32>,          // xy: size, zw: 1 / size
    sh: array<vec4<f32>, 9>,    // irradiance / pi (HDRI mode)
    env_params: vec4<f32>,      // x: 1 = HDRI image lighting, y: max mip, z: ambient intensity, w: HDRI sun above the atmosphere
    prev_view_proj: mat4x4<f32>,// previous frame, unjittered (TAA / SSR reprojection)
    proj_params: vec4<f32>,     // x: 1/P00, y: 1/P11, z: near, w: pixels per world unit at distance 1
    jitter: vec4<f32>,          // xy: sub-pixel jitter (NDC), z: frame index, w: 1 = history valid
    cascade_texel: vec4<f32>,   // world size of one shadow texel per cascade
    cascade_range: vec4<f32>,   // world depth range of each cascade (light space)
    fx: vec4<f32>,              // x: SSAO strength, y: contact shadows, z: SSR, w: sun angular diameter (rad)
    ao_params: vec4<f32>,       // x: AO radius (m), y: specular occlusion strength, z: SSR max distance (m), w: unused
    lights: array<Light, 64>,
};

@group(0) @binding(0) var<uniform> frame: Frame;
@group(0) @binding(3) var env_tex: texture_2d<f32>;
@group(0) @binding(4) var env_samp: sampler;

// Interleaved gradient noise (Jimenez 2014), animated per frame so TAA
// integrates it.
fn ign(p: vec2<f32>) -> f32 {
    let q = p + vec2<f32>(5.588238) * (frame.jitter.z % 64.0);
    return fract(52.9829189 * fract(dot(q, vec2<f32>(0.06711056, 0.00583715))));
}

// Linear view distance from a reverse-Z infinite-projection depth value.
fn linear_depth(d: f32) -> f32 {
    return frame.proj_params.z / max(d, 1e-9);
}

fn equirect_uv(d: vec3<f32>) -> vec2<f32> {
    return vec2<f32>(atan2(d.x, -d.z) / (2.0 * PI) + 0.5, acos(clamp(d.y, -1.0, 1.0)) / PI);
}

fn sh_irradiance(n: vec3<f32>) -> vec3<f32> {
    var c = frame.sh[0].rgb * 0.282095;
    c += frame.sh[1].rgb * 0.488603 * n.y + frame.sh[2].rgb * 0.488603 * n.z + frame.sh[3].rgb * 0.488603 * n.x;
    c += frame.sh[4].rgb * 1.092548 * n.x * n.y + frame.sh[5].rgb * 1.092548 * n.y * n.z;
    c += frame.sh[6].rgb * 0.315392 * (3.0 * n.z * n.z - 1.0) + frame.sh[7].rgb * 1.092548 * n.x * n.z;
    c += frame.sh[8].rgb * 0.546274 * (n.x * n.x - n.y * n.y);
    return max(c, vec3<f32>(0.0));
}


// --- Atmosphere (Rayleigh + Mie single scattering, Hillaire-style units in km).
const EARTH_R: f32 = 6360.0;
const ATMO_R: f32 = 6460.0;
const RAYLEIGH: vec3<f32> = vec3<f32>(5.802e-3, 13.558e-3, 33.1e-3);
const MIE: f32 = 3.996e-3;
const MIE_EXT: f32 = 4.4e-3;
const OZONE: vec3<f32> = vec3<f32>(0.65e-3, 1.881e-3, 0.085e-3);

fn ray_sphere(o: vec3<f32>, d: vec3<f32>, r: f32) -> f32 {
    let b = dot(o, d);
    let c = dot(o, o) - r * r;
    let disc = b * b - c;
    if (disc < 0.0) { return -1.0; }
    let s = sqrt(disc);
    if (-b - s > 0.0) { return -b - s; }
    return -b + s;
}

fn density(h: f32) -> vec3<f32> {
    // x: rayleigh, y: mie, z: ozone
    let oz = max(0.0, 1.0 - abs(h - 25.0) / 15.0);
    return vec3<f32>(exp(-h / 8.0), exp(-h / 1.2), oz);
}

fn extinction(d: vec3<f32>) -> vec3<f32> {
    return RAYLEIGH * d.x + vec3<f32>(MIE_EXT * d.y) + OZONE * d.z;
}

fn sun_transmittance(p: vec3<f32>, l: vec3<f32>) -> vec3<f32> {
    if (ray_sphere(p, l, EARTH_R) > 0.0 && dot(p, l) < 0.0) {
        // Behind the planet (cheap test: only when looking down).
        let b = dot(p, l);
        if (dot(p, p) - b * b < EARTH_R * EARTH_R) { return vec3<f32>(0.0); }
    }
    let t = ray_sphere(p, l, ATMO_R);
    let n = 6;
    let dt = t / f32(n);
    var od = vec3<f32>(0.0);
    for (var i = 0; i < n; i++) {
        let q = p + l * (f32(i) + 0.5) * dt;
        od += extinction(density(length(q) - EARTH_R)) * dt;
    }
    return exp(-od);
}

fn phase_rayleigh(c: f32) -> f32 { return 3.0 / (16.0 * PI) * (1.0 + c * c); }
fn phase_mie(c: f32) -> f32 {
    let g = 0.8;
    let k = 3.0 / (8.0 * PI) * (1.0 - g * g) / (2.0 + g * g);
    return k * (1.0 + c * c) / pow(1.0 + g * g - 2.0 * g * c, 1.5);
}

// Sky radiance along `d` from a viewer `alt_km` above the ground.
fn sky_radiance(d: vec3<f32>, l: vec3<f32>, alt_km: f32) -> vec3<f32> {
    let o = vec3<f32>(0.0, EARTH_R + max(alt_km, 0.001), 0.0);
    var t_max = ray_sphere(o, d, ATMO_R);
    let t_ground = ray_sphere(o, d, EARTH_R);
    if (t_ground > 0.0) { t_max = min(t_max, t_ground); }
    let n = 16;
    let dt = t_max / f32(n);
    let c = dot(d, l);
    let pr = phase_rayleigh(c);
    let pm = phase_mie(c);
    var lum = vec3<f32>(0.0);
    var trans = vec3<f32>(1.0);
    for (var i = 0; i < n; i++) {
        let p = o + d * (f32(i) + 0.5) * dt;
        let dens = density(length(p) - EARTH_R);
        let ext = extinction(dens);
        let ts = sun_transmittance(p, l);
        let scatter = (RAYLEIGH * dens.x * pr + vec3<f32>(MIE * dens.y * pm)) * ts;
        let step_t = exp(-ext * dt);
        // Energy-conserving integration (Hillaire 2015).
        lum += trans * (scatter - scatter * step_t) / max(ext, vec3<f32>(1e-6));
        trans *= step_t;
    }
    return lum;
}

// --- PBR helpers.
fn d_ggx(nh: f32, a: f32) -> f32 {
    let a2 = a * a;
    let f = nh * nh * (a2 - 1.0) + 1.0;
    return a2 / (PI * f * f);
}

fn v_smith(nv: f32, nl: f32, a: f32) -> f32 {
    let a2 = a * a;
    let gv = nl * sqrt(nv * nv * (1.0 - a2) + a2);
    let gl = nv * sqrt(nl * nl * (1.0 - a2) + a2);
    return 0.5 / max(gv + gl, 1e-5);
}

fn f_schlick(f0: vec3<f32>, vh: f32) -> vec3<f32> {
    return f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - vh, 5.0);
}

// Karis' analytic approximation of the split-sum environment BRDF.
fn env_brdf(f0: vec3<f32>, rough: f32, nv: f32) -> vec3<f32> {
    let c0 = vec4<f32>(-1.0, -0.0275, -0.572, 0.022);
    let c1 = vec4<f32>(1.0, 0.0425, 1.04, -0.04);
    let r = rough * c0 + c1;
    let a004 = min(r.x * r.x, exp2(-9.28 * nv)) * r.x + r.y;
    let ab = vec2<f32>(-1.04, 1.04) * a004 + r.zw;
    return f0 * ab.x + vec3<f32>(ab.y);
}

fn ambient_sky(n: vec3<f32>) -> vec3<f32> {
    if (frame.env_params.x > 0.5) {
        return sh_irradiance(n);
    }
    let up = n.y * 0.5 + 0.5;
    let sky = mix(frame.sky_horizon.rgb, frame.sky_zenith.rgb, clamp(n.y, 0.0, 1.0));
    return mix(frame.ground_color.rgb, sky, smoothstep(0.0, 1.0, up));
}

// Blurry environment reflection from the analytic sky colors.
fn env_reflection(r: vec3<f32>, rough: f32) -> vec3<f32> {
    if (frame.env_params.x > 0.5) {
        // Mip 0 holds the sun; start reflections at mip 1 to avoid double
        // counting the analytic sun highlight.
        let lod = 1.0 + rough * (frame.env_params.y - 1.0);
        return textureSampleLevel(env_tex, env_samp, equirect_uv(r), lod).rgb;
    }
    let sharp = mix(frame.sky_horizon.rgb, frame.sky_zenith.rgb, pow(clamp(r.y, 0.0, 1.0), 0.5));
    let below = frame.ground_color.rgb * 0.6;
    let env = select(below, sharp, r.y > -0.02);
    let sun_glint = frame.sun_color.rgb * pow(max(dot(r, frame.sun_dir.xyz), 0.0), mix(2000.0, 8.0, rough)) * (1.0 - rough) * 2.0;
    return mix(env, ambient_sky(r), rough) + sun_glint;
}

// Henyey-Greenstein phase function (1/sr).
fn phase_hg(c: f32, g: f32) -> f32 {
    let d = 1.0 + g * g - 2.0 * g * c;
    return (1.0 - g * g) / (4.0 * PI * d * sqrt(d));
}

// Mean radiance of the environment over the sphere (what an isotropic
// scatterer sees).
fn ambient_mean() -> vec3<f32> {
    if (frame.env_params.x > 0.5) {
        return frame.sh[0].rgb * 0.282095;
    }
    return (frame.sky_zenith.rgb + frame.sky_horizon.rgb * 2.0 + frame.ground_color.rgb) * 0.25;
}

// Aerial perspective: single scattering in an exponential aerosol layer
// (density `sky_horizon.w` at sea level, inverse scale height
// `ground_color.w`) plus clear-air Rayleigh. Light scattered in comes from
// the sun through a Henyey-Greenstein lobe and from the sky's mean
// radiance, so the haze is blue-grey away from the sun and brighter toward
// it, and it only shows over real distances (30 km visibility: 1.3% at
// 100 m, 5% at 400 m).
fn height_fog(color: vec3<f32>, world_pos: vec3<f32>, view_dir: vec3<f32>) -> vec3<f32> {
    let cam = frame.camera_pos.xyz;
    let dist = length(world_pos - cam);
    let density = frame.sky_horizon.w;
    let falloff = frame.ground_color.w;
    let dy = world_pos.y - cam.y;
    var integral = dist * exp(-falloff * max(cam.y, 0.0));
    if (abs(dy * falloff) > 1e-4) {
        integral *= (1.0 - exp(-falloff * dy)) / (falloff * dy);
    }
    // Optical depths: aerosol (grey) and Rayleigh (per channel, 1/m).
    let tau_m = density * integral;
    let tau_r = vec3<f32>(5.8e-6, 13.6e-6, 33.1e-6) * dist;
    let tau = vec3<f32>(tau_m) + tau_r;
    let trans = exp(-tau);
    let c = dot(view_dir, frame.sun_dir.xyz);
    let sun = frame.sun_color.rgb;
    let amb = ambient_mean();
    // Source functions: aerosol single-scattering albedo 0.9, g = 0.7.
    let j_m = (sun * phase_hg(c, 0.7) + amb) * 0.9;
    let j_r = sun * phase_rayleigh(c) + amb;
    let inscatter = (j_m * tau_m + j_r * tau_r) / max(tau, vec3<f32>(1e-9));
    return color * trans + inscatter * (vec3<f32>(1.0) - trans);
}
