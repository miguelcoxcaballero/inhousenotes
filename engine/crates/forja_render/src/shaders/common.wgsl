@group(0) @binding(3) var env_tex: texture_2d<f32>;
@group(0) @binding(4) var env_samp: sampler;
@group(0) @binding(5) var<storage, read> local_lights: array<LocalLight>;
// Per cluster: a light count, then u16 light indices packed in pairs.
@group(0) @binding(6) var<storage, read> clusters: array<u32>;

// Eye adaptation state (exposure.wgsl). Lighting is written to the HDR
// targets pre-multiplied by `pre` (the exposure of the previous frame,
// Frostbite-style pre-exposure) so night scenes keep full f16 precision;
// `pre` is 1 with manual exposure, which leaves the daytime path unchanged.
struct ExposureState {
    pre: f32,         // pre-exposure of this frame's HDR image
    prev_pre: f32,    // pre-exposure of the history (last frame)
    exposure: f32,    // display exposure (tonemap multiplies by exposure / pre)
    avg_log: f32,     // log2 of the adapted scene luminance (units)
};
@group(0) @binding(7) var<storage, read> exposure_state: ExposureState;

// Split-sum DFG LUT (r: A, g: B) for image-based specular (brdf.rs).
@group(0) @binding(9) var dfg_tex: texture_2d<f32>;

const CLUSTER_STRIDE: u32 = 49u;

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

// Split-sum directional albedo (A, B) of the specular BRDF, from the
// precomputed LUT (`brdf.rs`): E(v) = F0 * A + B.
fn dfg_terms(rough: f32, nv: f32) -> vec2<f32> {
    let uv = vec2<f32>(clamp(nv, 0.0, 1.0), clamp(rough, 0.0, 1.0)) * (31.0 / 32.0) + vec2<f32>(0.5 / 32.0);
    return textureSampleLevel(dfg_tex, env_samp, uv, 0.0).rg;
}

// Image-based specular with multiple-scattering energy compensation
// (Fdez-Aguera 2019, "A Multiple-Scattering Microfacet Model for Real-Time
// Image-based Lighting", JCGT 8(1)). Returns the weights of the prefiltered
// radiance (x3, single scattering with roughness-dependent Fresnel) and of
// the cosine-weighted irradiance (multiple scattering), plus the diffuse
// weight 1 - (FssEss + FmsEms) that keeps the sum energy conserving.
// `k` scales the dielectric Fresnel (material specular factor).
struct IblWeights {
    spec: vec3<f32>,
    multi: vec3<f32>,
    diffuse: vec3<f32>,
};

fn legacy(bit: u32) -> bool {
    return (u32(frame.env_params.w + 0.5) & bit) != 0u;
}

fn ibl_weights(f0: vec3<f32>, k: f32, rough: f32, nv: f32) -> IblWeights {
    var w: IblWeights;
    if (legacy(2u)) {
        w.spec = env_brdf(f0, rough, nv) * k;
        w.multi = vec3<f32>(0.0);
        w.diffuse = vec3<f32>(1.0);
        return w;
    }
    let ab = dfg_terms(rough, nv);
    let fr = max(vec3<f32>(1.0 - rough), f0) - f0;
    let ks = f0 + fr * pow(1.0 - nv, 5.0);
    let fss_ess = ks * ab.x + vec3<f32>(ab.y);
    let ess = ab.x + ab.y;
    let ems = 1.0 - ess;
    let f_avg = f0 + (vec3<f32>(1.0) - f0) / 21.0;
    let fms_ems = ems * fss_ess * f_avg / (vec3<f32>(1.0) - ems * f_avg);
    w.spec = fss_ess * k;
    w.multi = fms_ems * k;
    w.diffuse = max(vec3<f32>(1.0) - (fss_ess + fms_ems) * k, vec3<f32>(0.0));
    return w;
}

// Energy lost by single-scattering GGX restored for analytic lights
// (Kulla & Conty 2017, as approximated in Filament): 1 + F0 (1 / Ess - 1).
fn spec_energy(f0: vec3<f32>, rough: f32, nv: f32) -> vec3<f32> {
    if (legacy(2u)) { return vec3<f32>(1.0); }
    let ab = dfg_terms(rough, nv);
    return vec3<f32>(1.0) + f0 * (1.0 / max(ab.x + ab.y, 0.05) - 1.0);
}

// Geometric specular anti-aliasing (Kaplanyan et al. 2016; Tokuyoshi &
// Kaplanyan 2019, "Improved Geometric Specular Antialiasing"): widen the
// GGX lobe by the variance of the shading normal across the pixel
// footprint so curved and bumpy surfaces do not alias into sparkles, plus
// the normal-map variance left in a filtered (shortened) mip normal
// (Toksvig 2005 / vMF fit of Han et al. 2007). Takes and returns alpha^2.
// `n` must be computed in uniform control flow (screen derivatives).
fn specular_aa(n: vec3<f32>, a2: f32, nm_len: f32, nm_scale: f32) -> f32 {
    let dndu = dpdx(n);
    let dndv = dpdy(n);
    let variance = 0.15915494 * (dot(dndu, dndu) + dot(dndv, dndv));
    let kernel = min(2.0 * variance, 0.18);
    if (legacy(4u)) { return a2; }
    // vMF: kappa = (3L - L^3) / (1 - L^2), alpha^2 ~ 2 / kappa; a small dead
    // zone ignores 8-bit quantization of unfiltered normal maps.
    let l = clamp(nm_len, 0.0, 1.0);
    let toksvig = 2.0 * max(1.0 - l * l - 0.004, 0.0) / max(3.0 * l - l * l * l, 1e-3) * nm_scale * nm_scale;
    return clamp(a2 + kernel + min(toksvig, 0.25), 0.0, 1.0);
}

fn ambient_sky(n: vec3<f32>) -> vec3<f32> {
    if (frame.env_params.x > 0.5) {
        return sh_irradiance(n) + ambient_fill();
    }
    let up = n.y * 0.5 + 0.5;
    let sky = mix(frame.sky_horizon.rgb, frame.sky_zenith.rgb, clamp(n.y, 0.0, 1.0));
    return mix(frame.ground_color.rgb, sky, smoothstep(0.0, 1.0, up)) + ambient_fill();
}

// Blurry environment reflection from the analytic sky colors.
fn env_reflection(r: vec3<f32>, rough: f32) -> vec3<f32> {
    if (frame.env_params.x > 0.5) {
        // Mip 0 holds the sun; reflections start at mip 1 (the sun is the
        // analytic highlight). Mips 1.. are GGX-prefiltered (split-sum) for
        // roughness r = 1 - sqrt(1 - x), x = (lod - 1) / (max - 1)
        // (`env::mip_roughness`).
        var lod = 1.0 + rough * (2.0 - rough) * (frame.env_params.y - 1.0);
        if (legacy(1u)) { lod = 1.0 + rough * (frame.env_params.y - 1.0); }
        return textureSampleLevel(env_tex, env_samp, equirect_uv(r), lod).rgb;
    }
    let sharp = mix(frame.sky_horizon.rgb, frame.sky_zenith.rgb, pow(clamp(r.y, 0.0, 1.0), 0.5));
    let below = frame.ground_color.rgb * 0.6;
    let env = select(below, sharp, r.y > -0.02);
    let sun_glint = frame.sun_color.rgb * pow(max(dot(r, frame.sun_dir.xyz), 0.0), mix(2000.0, 8.0, rough)) * (1.0 - rough) * 2.0;
    return mix(env, ambient_sky(r), rough) + sun_glint;
}


// Mean radiance of the environment over the sphere (what an isotropic
// scatterer sees).
fn ambient_mean() -> vec3<f32> {
    if (frame.env_params.x > 0.5) {
        return frame.sh[0].rgb * 0.282095;
    }
    return (frame.sky_zenith.rgb + frame.sky_horizon.rgb * 2.0 + frame.ground_color.rgb) * 0.25;
}

// Optical depth integral of an exponential layer (inverse scale height
// `falloff`) between two points, per unit ground-level density.
fn layer_integral(o: vec3<f32>, p: vec3<f32>, falloff: f32) -> f32 {
    let dist = length(p - o);
    let dy = p.y - o.y;
    var integral = dist * exp(-falloff * max(o.y, 0.0));
    if (abs(dy * falloff) > 1e-4) {
        integral *= (1.0 - exp(-falloff * dy)) / (falloff * dy);
    }
    return integral;
}

// Aerosol optical depth between two points: the height fog (density
// `sky_horizon.w`, inverse scale height `ground_color.w`) plus the low haze
// layer of dawn and dusk (`fog_volume.xy`, zero by day and at night).
fn aerosol_depth(o: vec3<f32>, p: vec3<f32>) -> f32 {
    var tau = frame.sky_horizon.w * layer_integral(o, p, frame.ground_color.w);
    if (frame.fog_volume.x > 0.0) {
        tau += frame.fog_volume.x * layer_integral(o, p, frame.fog_volume.y);
    }
    return tau;
}

// Aerial perspective: single scattering in an exponential aerosol layer
// (density `sky_horizon.w` at sea level, inverse scale height
// `ground_color.w`, plus the low haze of `fog_volume.xy`) and clear-air
// Rayleigh. Light scattered in comes from the sun through a
// Henyey-Greenstein lobe and from the sky's mean radiance, so the haze is
// blue-grey away from the sun and brighter toward it, and it only shows
// over real distances (30 km visibility: 1.3% at 100 m, 5% at 400 m).
fn height_fog(color: vec3<f32>, world_pos: vec3<f32>, view_dir: vec3<f32>) -> vec3<f32> {
    return height_fog_from(color, frame.camera_pos.xyz, world_pos, view_dir);
}

// The same along the segment from `o` to `world_pos` (the part of a view ray
// beyond the froxel fog volume).
fn height_fog_from(color: vec3<f32>, o: vec3<f32>, world_pos: vec3<f32>, view_dir: vec3<f32>) -> vec3<f32> {
    let dist = length(world_pos - o);
    let tau_m = aerosol_depth(o, world_pos);
    if (frame.params.w < 0.5) {
        // Night camera (Environment::camera = CameraModel::Night): the grey height fog
        // the night presets were tuned with; their lamps glow in it through
        // `local_inscatter`.
        let fog = 1.0 - exp(-tau_m);
        return mix(color, night_fog_color(dot(view_dir, frame.sun_dir.xyz), 1.0), clamp(fog, 0.0, 1.0));
    }
    // Optical depths: aerosol (grey) and Rayleigh (per channel, 1/m).
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

// Source radiance of the night fog toward the camera (`c`: cosine between
// the view ray and the sun/moon), with the moon's share scaled by its
// shadow visibility.
fn night_fog_color(c: f32, sun_vis: f32) -> vec3<f32> {
    let sun_amount = pow(max(c, 0.0), 8.0);
    return frame.sky_horizon.rgb * (1.0 - 0.5 * sun_amount) + frame.sun_color.rgb * (0.25 * sun_amount * sun_vis);
}

// ---- Clustered local lights -------------------------------------------------

fn cluster_base(pix: vec2<f32>, slice: u32) -> u32 {
    let tile = vec2<u32>(clamp(pix / frame.cluster.w, vec2<f32>(0.0), frame.cluster.xy - vec2<f32>(1.0)));
    let tx = u32(frame.cluster.x);
    let ty = u32(frame.cluster.y);
    return ((slice * ty + tile.y) * tx + tile.x) * CLUSTER_STRIDE;
}

// Cluster of a pixel at linear view depth `z`.
fn cluster_of(pix: vec2<f32>, z: f32) -> u32 {
    let s = clamp(log(max(z, 1e-3) / frame.cluster_z.x) * frame.cluster_z.z, 0.0, frame.cluster.z - 1.0);
    return cluster_base(pix, u32(s));
}

// The whole-column bucket of a pixel's tile (fog in-scattering).
fn column_of(pix: vec2<f32>) -> u32 {
    return cluster_base(pix, u32(frame.cluster.z));
}

fn cluster_light(base: u32, i: u32) -> u32 {
    let w = clusters[base + 1u + (i >> 1u)];
    return select(w & 0xffffu, w >> 16u, (i & 1u) == 1u);
}

fn spot_cone(l: LocalLight, from_light: vec3<f32>) -> f32 {
    let c = clamp(dot(from_light, l.dir.xyz) * l.params.x + l.params.y, 0.0, 1.0);
    return c * c;
}

// Smooth range window (Karis 2013).
fn range_window(d2: f32, r: f32) -> f32 {
    let x = d2 / (r * r);
    let w = clamp(1.0 - x * x, 0.0, 1.0);
    return w * w;
}

// Henyey-Greenstein phase function (1/sr).
fn phase_hg(cos_t: f32, g: f32) -> f32 {
    let g2 = g * g;
    return (1.0 - g2) / (4.0 * PI * pow(max(1.0 + g2 - 2.0 * g * cos_t, 1e-4), 1.5));
}

// Aerosol extinction coefficient at height y (m^-1): height fog plus haze.
fn fog_sigma(y: f32) -> f32 {
    return frame.sky_horizon.w * exp(-frame.ground_color.w * max(y, 0.0)) + frame.fog_volume.x * exp(-frame.fog_volume.y * max(y, 0.0));
}

// Light scattered toward the camera by the height fog from the local lights
// of this pixel's tile column, between `t_min` and `t_max` along `dir`
// (`t_min` > 0 when the froxel fog volume covers the first stretch).
// Each light is integrated in closed form over the angle it subtends
// (equiangular parameterization, Kulla & Fajardo 2012: dt / d² = dθ / h),
// so a point light needs only a few deterministic nodes for the
// Henyey-Greenstein phase and the fog density; spot cones get more.
// Gives lamps their halo in haze and spot beams their visible cone.
// Cost: (3 or 6) x lights within `cluster_z.w` metres in the tile.
fn local_inscatter(pix: vec2<f32>, dir: vec3<f32>, t_min: f32, t_max: f32) -> vec3<f32> {
    if (frame.lighting.x <= 0.0 || frame.lighting.z <= 0.0 || frame.sky_horizon.w <= 0.0) { return vec3<f32>(0.0); }
    let cam = frame.camera_pos.xyz;
    let base = column_of(pix);
    let n = min(clusters[base], 96u);
    let g = frame.scatter.x;
    var acc = vec3<f32>(0.0);
    for (var i = 0u; i < n; i++) {
        let l = local_lights[cluster_light(base, i)];
        let r = l.pos_range.w;
        let to = l.pos_range.xyz - cam;
        let tc = dot(to, dir);
        let h2 = max(dot(to, to) - tc * tc, 0.0);
        if (h2 >= r * r) { continue; }
        let half_chord = sqrt(r * r - h2);
        let ta = max(tc - half_chord, t_min);
        let tb = min(tc + half_chord, t_max);
        if (tb <= ta) { continue; }
        let h = sqrt(h2 + l.color.w * l.color.w + 1e-4);
        let th_a = atan((ta - tc) / h);
        let th_b = atan((tb - tc) / h);
        let is_spot = l.dir.w > -1.5;
        let nodes = select(3u, 6u, is_spot);
        var s = 0.0;
        for (var k = 0u; k < nodes; k++) {
            let th = mix(th_a, th_b, (f32(k) + 0.5) / f32(nodes));
            let t = tc + h * tan(th);
            let x = cam + dir * t;
            let lx = x - l.pos_range.xyz;
            let d2 = dot(lx, lx);
            let ld = lx * inverseSqrt(max(d2, 1e-8));
            s += spot_cone(l, ld) * phase_hg(dot(ld, -dir), g) * fog_sigma(x.y) * range_window(d2, r);
        }
        acc += l.color.rgb * (s * (th_b - th_a) / (f32(nodes) * h) * abs(l.params.w));
    }
    return acc * frame.lighting.z;
}

// ---- Night sky ---------------------------------------------------------------

fn hash22(p: vec2<f32>) -> vec2<f32> {
    var q = fract(p * vec2<f32>(123.34, 456.21));
    q += dot(q, q + 45.32);
    return fract(vec2<f32>(q.x * q.y, q.x + q.y));
}

// Night sky radiance: a dark zenith, the orange-grey dome of city light
// pollution toward the horizon and optional stars (one per cell of a
// direction grid, ~1.5 px wide so TAA keeps them stable).
fn night_sky(d: vec3<f32>) -> vec3<f32> {
    let up = max(d.y, 0.0);
    var c = frame.night_zenith.rgb + frame.sky_glow.rgb * exp(-up / max(frame.sky_glow.w, 0.005));
    if (frame.fill.w > 0.0 && d.y > 0.02) {
        let cells = 220.0;
        let uv = vec2<f32>(atan2(d.x, d.z) / (2.0 * PI) + 0.5, asin(clamp(d.y, -1.0, 1.0)) / PI + 0.5);
        let cell = floor(uv * vec2<f32>(cells * 2.0, cells));
        let h = hash22(cell);
        let h2 = hash22(cell + vec2<f32>(17.0, 3.0));
        let center = (cell + 0.2 + 0.6 * h) / vec2<f32>(cells * 2.0, cells);
        let phi = (center.x - 0.5) * 2.0 * PI;
        let el = (center.y - 0.5) * PI;
        let sd = vec3<f32>(cos(el) * sin(phi), sin(el), cos(el) * cos(phi));
        let ang = acos(clamp(dot(sd, d), -1.0, 1.0)) * frame.proj_params.w;
        let mag = pow(h2.x, 40.0);
        // Stars fade into the light-pollution glow near the horizon.
        let vis = smoothstep(0.05, 0.35, d.y);
        c += vec3<f32>(0.9, 0.95, 1.1) * (mag * exp(-ang * ang * 1.2) * frame.fill.w * vis);
    }
    return c;
}

// Ambient fill: light bounced around the street by the city's own lamps
// (no GI here), added to the sky ambient.
fn ambient_fill() -> vec3<f32> {
    return frame.fill.rgb;
}

// Height-fog transmittance from the camera to `p`.
fn fog_transmittance(p: vec3<f32>) -> f32 {
    return exp(-aerosol_depth(frame.camera_pos.xyz, p));
}

// ---- Sky LUTs (Hillaire 2020, "A Scalable and Production Ready Sky and
// Atmosphere Rendering Technique") --------------------------------------------

// Transmittance LUT (256 x 64): x maps the distance to the top of the
// atmosphere along the ray, y the height (Bruneton & Neyret 2008 mapping).
fn transmittance_uv(r: f32, mu: f32) -> vec2<f32> {
    let hh = sqrt(ATMO_R * ATMO_R - EARTH_R * EARTH_R);
    let rho = sqrt(max(r * r - EARTH_R * EARTH_R, 0.0));
    let disc = r * r * (mu * mu - 1.0) + ATMO_R * ATMO_R;
    let d = max(-r * mu + sqrt(max(disc, 0.0)), 0.0);
    let d_min = ATMO_R - r;
    let d_max = rho + hh;
    return vec2<f32>((d - d_min) / max(d_max - d_min, 1e-6), rho / hh);
}

// Inverse of `transmittance_uv`: (r, mu).
fn transmittance_params(uv: vec2<f32>) -> vec2<f32> {
    let hh = sqrt(ATMO_R * ATMO_R - EARTH_R * EARTH_R);
    let rho = hh * uv.y;
    let r = sqrt(rho * rho + EARTH_R * EARTH_R);
    let d_min = ATMO_R - r;
    let d_max = rho + hh;
    let d = d_min + uv.x * (d_max - d_min);
    var mu = 1.0;
    if (d > 0.0) { mu = (hh * hh - rho * rho - d * d) / (2.0 * r * d); }
    return vec2<f32>(r, clamp(mu, -1.0, 1.0));
}

// Sky-view LUT (192 x 108) mapping: x is the azimuth from the sun (finer
// toward it), y the zenith angle with half the texels on each side of the
// horizon, compressed toward it (where the sky changes fastest).
fn skyview_uv(d: vec3<f32>, sun: vec3<f32>, alt_km: f32) -> vec2<f32> {
    let h = EARTH_R + max(alt_km, 0.001);
    let cos_beta = sqrt(max(h * h - EARTH_R * EARTH_R, 0.0)) / h;
    let beta = acos(clamp(cos_beta, -1.0, 1.0));
    let zenith_horizon = PI - beta;
    let zenith = acos(clamp(d.y, -1.0, 1.0));
    var v: f32;
    if (zenith < zenith_horizon) {
        v = 0.5 * (1.0 - sqrt(max(1.0 - zenith / zenith_horizon, 0.0)));
    } else {
        v = 0.5 + 0.5 * sqrt(clamp((zenith - zenith_horizon) / max(beta, 1e-5), 0.0, 1.0));
    }
    let dxz = vec2<f32>(d.x, d.z);
    let sxz = vec2<f32>(sun.x, sun.z);
    var cos_az = 1.0;
    if (dot(dxz, dxz) > 1e-8 && dot(sxz, sxz) > 1e-8) { cos_az = dot(normalize(dxz), normalize(sxz)); }
    let u = sqrt(clamp(-cos_az * 0.5 + 0.5, 0.0, 1.0));
    return vec2<f32>(u, v);
}
