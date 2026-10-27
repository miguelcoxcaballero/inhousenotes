// Sky LUTs (Hillaire 2020, "A Scalable and Production Ready Sky and
// Atmosphere Rendering Technique", EGSR / CGF 39(4)).
// `common.wgsl` is prepended at load time.
//
// * Transmittance LUT, 256 x 64, rendered once: optical depth to the top of
//   the atmosphere (40 steps) for every height and zenith angle.
// * Sky-view LUT, 192 x 108, every frame: the single-scattering sky of the
//   camera's altitude and the current sun, marched 30 steps per texel with
//   the transmittance LUT instead of a nested sun march. The sky pass then
//   reads one bilinear tap per pixel instead of 16 x 6 density evaluations
//   (the per-pixel raymarch of `sky_radiance`). Same model, same units.

@group(1) @binding(0) var trans_lut: texture_2d<f32>;
@group(1) @binding(1) var lut_samp: sampler;

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

@fragment
fn fs_transmittance(in: FsIn) -> @location(0) vec4<f32> {
    let rm = transmittance_params(in.uv);
    let r = rm.x;
    let mu = rm.y;
    let p = vec3<f32>(0.0, r, 0.0);
    let l = vec3<f32>(sqrt(max(1.0 - mu * mu, 0.0)), mu, 0.0);
    let t = ray_sphere(p, l, ATMO_R);
    let n = 40;
    let dt = max(t, 0.0) / f32(n);
    var od = vec3<f32>(0.0);
    for (var i = 0; i < n; i++) {
        let q = p + l * (f32(i) + 0.5) * dt;
        od += extinction(density(length(q) - EARTH_R)) * dt;
    }
    return vec4<f32>(exp(-od), 1.0);
}

// Sun transmittance from `p` (km, planet centred) toward `l`, 0 behind the
// planet.
fn lut_transmittance(p: vec3<f32>, l: vec3<f32>) -> vec3<f32> {
    let r = length(p);
    let mu = dot(p, l) / r;
    // Below the geometric horizon of this height: the planet blocks it.
    let horizon = -sqrt(max(1.0 - (EARTH_R * EARTH_R) / (r * r), 0.0));
    if (mu < horizon) { return vec3<f32>(0.0); }
    return textureSampleLevel(trans_lut, lut_samp, transmittance_uv(r, mu), 0.0).rgb;
}

@fragment
fn fs_skyview(in: FsIn) -> @location(0) vec4<f32> {
    let alt = frame.camera_pos.y * 0.001;
    let h = EARTH_R + max(alt, 0.001);
    let cos_beta = sqrt(max(h * h - EARTH_R * EARTH_R, 0.0)) / h;
    let beta = acos(clamp(cos_beta, -1.0, 1.0));
    let zenith_horizon = PI - beta;
    // Inverse of `skyview_uv` (texel centres).
    let uv = in.uv;
    var zenith: f32;
    if (uv.y < 0.5) {
        let c = 1.0 - 2.0 * uv.y;
        zenith = zenith_horizon * (1.0 - c * c);
    } else {
        let c = uv.y * 2.0 - 1.0;
        zenith = zenith_horizon + beta * c * c;
    }
    let cos_az = -(uv.x * uv.x * 2.0 - 1.0);
    let sin_az = sqrt(max(1.0 - cos_az * cos_az, 0.0));
    let sz = sin(zenith);
    // Local frame with the sun in the x-y plane.
    let s = frame.sun_dir.xyz;
    let s_h = sqrt(max(1.0 - s.y * s.y, 0.0));
    let l = vec3<f32>(s_h, s.y, 0.0);
    let d = vec3<f32>(sz * cos_az, cos(zenith), sz * sin_az);

    let o = vec3<f32>(0.0, h, 0.0);
    var t_max = ray_sphere(o, d, ATMO_R);
    let t_ground = ray_sphere(o, d, EARTH_R);
    if (t_ground > 0.0) { t_max = min(t_max, t_ground); }
    let n = 30;
    let c = dot(d, l);
    let pr = phase_rayleigh(c);
    let pm = phase_mie(c);
    var lum = vec3<f32>(0.0);
    var trans = vec3<f32>(1.0);
    // Non-uniform steps (denser near the camera, where the air is thick).
    var t_prev = 0.0;
    for (var i = 0; i < n; i++) {
        let u = (f32(i) + 1.0) / f32(n);
        let t1 = t_max * u * u;
        let dt = t1 - t_prev;
        let p = o + d * (t_prev + 0.5 * dt);
        t_prev = t1;
        let dens = density(length(p) - EARTH_R);
        let ext = extinction(dens);
        let ts = lut_transmittance(p, l);
        let scatter = (RAYLEIGH * dens.x * pr + vec3<f32>(MIE * dens.y * pm)) * ts;
        let step_t = exp(-ext * dt);
        lum += trans * (scatter - scatter * step_t) / max(ext, vec3<f32>(1e-6));
        trans *= step_t;
    }
    return vec4<f32>(lum, 1.0);
}
