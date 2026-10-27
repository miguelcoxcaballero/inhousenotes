// Forward PBR with cascaded sun shadows, sky ambient, clustered point and
// spot lights, wet surfaces and fog.
// `common.wgsl` is prepended at load time.

struct Material {
    base_color: vec4<f32>,
    emissive: vec4<f32>,   // w: normal scale
    params: vec4<f32>,     // x: metallic, y: roughness, z: alpha cutoff, w: unlit
    layers: vec4<f32>,     // x: clearcoat, y: clearcoat roughness, z: transmission
    extra: vec4<f32>,      // x: specular factor (KHR_materials_specular: scales dielectric Fresnel)
};

@group(0) @binding(1) var shadow_map: texture_depth_2d_array;
@group(0) @binding(2) var shadow_sampler: sampler_comparison;
@group(0) @binding(8) var spot_shadow_map: texture_depth_2d_array;

@group(1) @binding(0) var<uniform> material: Material;
@group(1) @binding(1) var base_tex: texture_2d<f32>;
@group(1) @binding(2) var normal_tex: texture_2d<f32>;
@group(1) @binding(3) var mr_tex: texture_2d<f32>;
@group(1) @binding(4) var emissive_tex: texture_2d<f32>;
@group(1) @binding(5) var occlusion_tex: texture_2d<f32>;
@group(1) @binding(6) var tex_sampler: sampler;

@group(2) @binding(0) var<storage, read> joints: array<mat4x4<f32>>;

// Screen-space inputs of the main pass (depth prepass, GTAO, last frame).
@group(3) @binding(0) var ssao_tex: texture_2d<f32>;
@group(3) @binding(1) var scene_depth: texture_depth_2d;
@group(3) @binding(2) var history_tex: texture_2d<f32>;
@group(3) @binding(3) var screen_samp: sampler;
@group(3) @binding(4) var scene_normal: texture_2d<f32>;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) tangent: vec4<f32>,
    @location(4) m0: vec4<f32>,
    @location(5) m1: vec4<f32>,
    @location(6) m2: vec4<f32>,
    @location(7) m3: vec4<f32>,
    @location(8) tint: vec4<f32>,
    @location(9) extra: vec4<u32>,
};

struct SkinIn {
    @location(10) joint_ids: vec4<u32>,
    @location(11) weights: vec4<f32>,
};

struct VertexOut {
    // Invariant so the depth prepass and the main pass produce identical depth.
    @builtin(position) @invariant clip: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) tangent: vec4<f32>,
    @location(4) tint: vec4<f32>,
    @location(5) view_depth: f32,
};

fn finish(world: vec4<f32>, n: vec3<f32>, t: vec3<f32>, tw: f32, uv: vec2<f32>, tint: vec4<f32>) -> VertexOut {
    var out: VertexOut;
    out.clip = frame.view_proj * world;
    out.world_pos = world.xyz;
    out.normal = normalize(n);
    out.tangent = vec4<f32>(normalize(t), tw);
    out.uv = uv;
    out.tint = tint;
    out.view_depth = -(frame.view * world).z;
    return out;
}

@vertex
fn vs_main(v: VertexIn) -> VertexOut {
    let model = mat4x4<f32>(v.m0, v.m1, v.m2, v.m3);
    let m3 = mat3x3<f32>(model[0].xyz, model[1].xyz, model[2].xyz);
    return finish(model * vec4<f32>(v.position, 1.0), m3 * v.normal, m3 * v.tangent.xyz, v.tangent.w, v.uv, v.tint);
}

@vertex
fn vs_skinned(v: VertexIn, s: SkinIn) -> VertexOut {
    let base = v.extra.x;
    let skin = joints[base + s.joint_ids.x] * s.weights.x
        + joints[base + s.joint_ids.y] * s.weights.y
        + joints[base + s.joint_ids.z] * s.weights.z
        + joints[base + s.joint_ids.w] * s.weights.w;
    let model = mat4x4<f32>(v.m0, v.m1, v.m2, v.m3) * skin;
    let m3 = mat3x3<f32>(model[0].xyz, model[1].xyz, model[2].xyz);
    return finish(model * vec4<f32>(v.position, 1.0), m3 * v.normal, m3 * v.tangent.xyz, v.tangent.w, v.uv, v.tint);
}

// ---- Lighting helpers (render-owned). --------------------------------------

// Depth prepass output: geometric normal for GTAO. Opaque materials skip the
// texture fetch so the pass keeps early depth testing; masked materials
// (leaves, hair cards) repeat the main pass's alpha test.
@fragment
fn fs_prepass(in: VertexOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let n = normalize(in.normal);
    return vec4<f32>(select(-n, n, front), 1.0);
}

@fragment
fn fs_prepass_masked(in: VertexOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let a = textureSample(base_tex, tex_sampler, in.uv).a * material.base_color.a * in.tint.a;
    if (a < material.params.z) { discard; }
    let n = normalize(in.normal);
    return vec4<f32>(select(-n, n, front), 1.0);
}

fn screen_max() -> vec2<i32> {
    return vec2<i32>(frame.screen.xy) - vec2<i32>(1);
}

fn scene_depth_at(p: vec2<i32>) -> f32 {
    return textureLoad(scene_depth, clamp(p, vec2<i32>(0), screen_max()), 0);
}

// True when this fragment is the surface the depth prepass kept, so the
// screen-space buffers at its pixel describe it (false for glass and other
// blended surfaces drawn in front).
fn is_prepass_surface(clip: vec4<f32>) -> bool {
    let d = scene_depth_at(vec2<i32>(clip.xy));
    return abs(d - clip.z) <= d * 1e-5 + 1e-8;
}

const POISSON: array<vec2<f32>, 12> = array<vec2<f32>, 12>(
    vec2<f32>(-0.326, -0.406), vec2<f32>(-0.840, -0.074), vec2<f32>(-0.696, 0.457),
    vec2<f32>(-0.203, 0.621), vec2<f32>(0.962, -0.195), vec2<f32>(0.473, -0.480),
    vec2<f32>(0.519, 0.767), vec2<f32>(0.185, -0.893), vec2<f32>(0.507, 0.064),
    vec2<f32>(0.896, 0.412), vec2<f32>(-0.322, -0.933), vec2<f32>(-0.792, -0.598));

// Sun shadow with a physically sized penumbra (PCSS): an 8-tap blocker
// search estimates the occluder distance, the sun's angular diameter turns
// it into a world-space penumbra, and a 12-tap rotated Poisson PCF filters
// that width. Contact areas stay sharp; shadows of tall buildings soften the
// way they do in photos. Cascades are cross-faded with dithering (resolved
// by TAA) instead of switching on a visible seam.
// Cost: 8 depth loads + 12 hardware PCF taps per lit pixel.
fn shadow_factor(world_pos: vec3<f32>, n: vec3<f32>, view_depth: f32, pix: vec2<f32>) -> f32 {
    if (view_depth > frame.cascade_splits[3]) { return 1.0; }
    var cascade = 3;
    for (var i = 0; i < 4; i++) {
        if (view_depth < frame.cascade_splits[i]) { cascade = i; break; }
    }
    let noise = ign(pix);
    if (cascade < 3) {
        let split = frame.cascade_splits[cascade];
        let blend = clamp((view_depth - split * 0.85) / (split * 0.15), 0.0, 1.0);
        if (noise < blend) { cascade += 1; }
    }
    let wtexel = frame.cascade_texel[cascade];
    let range = frame.cascade_range[cascade];
    let l = frame.sun_dir.xyz;
    let nl = clamp(dot(n, l), 0.0, 1.0);
    // Normal offset of about a texel, larger at grazing light.
    let p = world_pos + n * wtexel * (0.7 + 1.6 * (1.0 - nl));
    let lc = frame.cascade_vp[cascade] * vec4<f32>(p, 1.0);
    let uv = vec2<f32>(lc.x * 0.5 + 0.5, 0.5 - lc.y * 0.5);
    if (any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0)) || lc.z > 1.0) { return 1.0; }
    let size = 1.0 / frame.params.y;
    // Receiver-plane depth bias: the light-space depth gradient of the
    // surface plane (per shadow texel), from two tangents mapped through the
    // cascade matrix, so wide kernels compare against the plane, not a
    // constant depth (no acne, no lost contact).
    let t1 = normalize(cross(n, select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 1.0, 0.0), abs(n.x) > 0.7)));
    let t2 = cross(n, t1);
    let d1 = (frame.cascade_vp[cascade] * vec4<f32>(t1, 0.0)).xyz;
    let d2 = (frame.cascade_vp[cascade] * vec4<f32>(t2, 0.0)).xyz;
    let det = d1.x * d2.y - d2.x * d1.y;
    var gz = vec2<f32>(0.0);
    if (abs(det) > 1e-12) {
        let g = vec2<f32>(d1.z * d2.y - d2.z * d1.y, d2.z * d1.x - d1.z * d2.x) / det;
        gz = vec2<f32>(g.x, -g.y) * (2.0 / size);
        // Limit to ~3 texels of depth per texel (near-grazing receivers).
        let lim = 3.0 * wtexel / range;
        let gl = length(gz);
        if (gl > lim) { gz *= lim / gl; }
    }
    let z = lc.z - wtexel * 0.5 / range;
    let a = noise * 6.2831853;
    let rot = mat2x2<f32>(cos(a), sin(a), -sin(a), cos(a));
    var poisson = POISSON;
    // Blocker search sized for occluders up to ~30 m away.
    let sun_d = frame.fx.w;
    let search = clamp(30.0 * sun_d / wtexel, 1.5, 14.0);
    var bsum = 0.0;
    var bn = 0.0;
    let maxp = vec2<i32>(i32(size) - 1);
    for (var i = 0; i < 8; i++) {
        let o = rot * poisson[i] * search;
        let tp = clamp(vec2<i32>(floor(uv * size + o)), vec2<i32>(0), maxp);
        let bd = textureLoad(shadow_map, tp, cascade, 0);
        let zt = z + dot(gz, o) - abs(dot(gz, vec2<f32>(0.5)));
        if (bd < zt) { bsum += bd; bn += 1.0; }
    }
    var s = 1.0;
    if (bn > 0.0) {
        let blocker = bsum / bn;
        let pen = max(z - blocker, 0.0) * range * sun_d;
        let radius = clamp(pen / wtexel * 0.5, 0.8, 14.0);
        var sum = 0.0;
        for (var i = 0; i < 12; i++) {
            let o = rot * poisson[i] * radius;
            sum += textureSampleCompareLevel(shadow_map, shadow_sampler, uv + o / size, cascade, z + dot(gz, o));
        }
        s = sum / 12.0;
    }
    // Fade out at the end of the last cascade.
    let fade = clamp((frame.cascade_splits[3] - view_depth) / (frame.cascade_splits[3] * 0.1), 0.0, 1.0);
    return mix(1.0, s, fade);
}

// Screen-space contact shadow: a short march toward the sun through the
// depth prepass catches the small-scale occlusion the shadow map's texels
// and biases miss (bollard feet, window reveals, facial features).
// Cost: 10 depth loads per lit opaque pixel.
fn contact_shadow(p: vec3<f32>, ng: vec3<f32>, view_depth: f32, pix: vec2<f32>) -> f32 {
    let l = frame.sun_dir.xyz;
    let len = clamp(view_depth * 0.02, 0.03, 0.8);
    let jit = ign(pix + vec2<f32>(37.0, 11.0));
    let start = p + ng * (0.002 + view_depth * 0.0006);
    const STEPS: i32 = 10;
    for (var i = 0; i < STEPS; i++) {
        let t = (f32(i) + jit) / f32(STEPS);
        let q = start + l * (len * t * t);
        let c = frame.view_proj * vec4<f32>(q, 1.0);
        if (c.w <= frame.proj_params.z) { break; }
        let ndc = c.xy / c.w;
        if (any(abs(ndc) >= vec2<f32>(1.0))) { break; }
        let sp = vec2<i32>((ndc * vec2<f32>(0.5, -0.5) + 0.5) * frame.screen.xy);
        let delta = c.w - linear_depth(scene_depth_at(sp));
        if (delta > 0.002 * c.w + 0.004 && delta < 0.12 + c.w * 0.01) {
            return 0.0;
        }
    }
    return 1.0;
}

// Screen-space reflection: the reflected ray is marched through the depth
// prepass in screen space (perspective-correct depth, McGuire & Mara 2014),
// refined by bisection, and the hit is shaded with last frame's resolved
// HDR image (reprojected). Glass, car paint and glossy floors then mirror
// the actual street instead of the HDRI's unrelated surroundings; misses
// (off-screen, behind objects, sky) fall back to the environment.
// Rough surfaces trace one GGX-distributed direction per frame (stochastic
// SSR); TAA integrates them into a glossy reflection of the real street.
// Returns rgb and a confidence in a.
// Cost: up to 32 + 4 depth loads, 1 normal and 1 history tap per glossy
// pixel (roughness < 0.62), ~0.6-1.2 ms at 1440p on an RTX 3070 class GPU
// in a street full of windows and shutters.
fn ssr_trace(p: vec3<f32>, n: vec3<f32>, v: vec3<f32>, rough: f32, pix: vec2<f32>) -> vec4<f32> {
    let fade_rough = 1.0 - smoothstep(0.45, 0.62, rough);
    if (frame.fx.z < 0.5 || fade_rough <= 0.0) { return vec4<f32>(0.0); }
    let r = ssr_direction(n, v, rough, pix);
    let max_dist = frame.ao_params.z;
    // Keep the ray in front of the near plane (rays toward the camera).
    let pv = (frame.view * vec4<f32>(p, 1.0)).xyz;
    let rv = (frame.view * vec4<f32>(r, 0.0)).xyz;
    var len = max_dist;
    let near = frame.proj_params.z * 1.5;
    if (rv.z > 1e-4) { len = min(len, (-near - pv.z) / rv.z); }
    if (len <= 0.05) { return vec4<f32>(0.0); }
    let h0 = frame.view_proj * vec4<f32>(p, 1.0);
    let h1 = frame.view_proj * vec4<f32>(p + r * len, 1.0);
    let k0 = 1.0 / h0.w;
    var k1 = 1.0 / h1.w;
    let s0 = (h0.xy * k0 * vec2<f32>(0.5, -0.5) + 0.5) * frame.screen.xy;
    var s1 = (h1.xy * k1 * vec2<f32>(0.5, -0.5) + 0.5) * frame.screen.xy;
    // Clip the screen segment to the viewport.
    var t_end = 1.0;
    let d = s1 - s0;
    if (d.x > 1e-3) { t_end = min(t_end, (frame.screen.x - 0.5 - s0.x) / d.x); }
    if (d.x < -1e-3) { t_end = min(t_end, (0.5 - s0.x) / d.x); }
    if (d.y > 1e-3) { t_end = min(t_end, (frame.screen.y - 0.5 - s0.y) / d.y); }
    if (d.y < -1e-3) { t_end = min(t_end, (0.5 - s0.y) / d.y); }
    t_end = max(t_end, 0.0);
    let span = length(d) * t_end;
    if (span < 2.0) { return vec4<f32>(0.0); }
    const STEPS: i32 = 32;
    // Steps grow quadratically: fine near the surface (contact), coarse far.
    let jit = ign(pix + vec2<f32>(91.0, 23.0));
    var t_prev = 0.0;
    var hit = false;
    var t_hit = 0.0;
    for (var i = 1; i <= STEPS; i++) {
        let u = (f32(i) - 1.0 + jit) / f32(STEPS);
        let t = max(u * u * t_end, 1.5 / length(d));
        let sp = s0 + d * t;
        let ray_w = 1.0 / mix(k0, k1, t);
        let scene_w = linear_depth(scene_depth_at(vec2<i32>(sp)));
        let prev_w = 1.0 / mix(k0, k1, t_prev);
        // Hit when the ray passes behind the depth buffer by less than the
        // assumed thickness of what it hit (the step's own depth span plus a
        // margin that grows with distance).
        let thick = abs(ray_w - prev_w) + 0.1 + ray_w * 0.015;
        let dz = ray_w - scene_w;
        if (dz > 0.0 && dz < thick) {
            hit = true;
            t_hit = t;
            break;
        }
        t_prev = t;
    }
    if (!hit) { return vec4<f32>(0.0); }
    // Bisection between the last miss and the hit.
    var lo = t_prev;
    var hi = t_hit;
    for (var i = 0; i < 4; i++) {
        let mid = 0.5 * (lo + hi);
        let sp = s0 + d * mid;
        if (1.0 / mix(k0, k1, mid) > linear_depth(scene_depth_at(vec2<i32>(sp)))) { hi = mid; } else { lo = mid; }
    }
    let hp = s0 + d * hi;
    let hip = vec2<i32>(hp);
    let hd = scene_depth_at(hip);
    // Back faces and surfaces seen edge-on by the ray carry no valid color.
    let hn = textureLoad(scene_normal, clamp(hip, vec2<i32>(0), screen_max()), 0).xyz;
    let facing = clamp(-dot(hn, r) * 4.0, 0.0, 1.0);
    // Reproject the hit into last frame's image.
    let ndc = vec2<f32>(hp.x * frame.screen.z * 2.0 - 1.0, 1.0 - hp.y * frame.screen.w * 2.0);
    let wh = frame.inv_view_proj * vec4<f32>(ndc, hd, 1.0);
    let hit_pos = wh.xyz / wh.w;
    let prev = frame.prev_view_proj * vec4<f32>(hit_pos, 1.0);
    if (prev.w <= 1e-5) { return vec4<f32>(0.0); }
    let puv = vec2<f32>(prev.x / prev.w * 0.5 + 0.5, 0.5 - prev.y / prev.w * 0.5);
    if (any(puv <= vec2<f32>(0.0)) || any(puv >= vec2<f32>(1.0))) { return vec4<f32>(0.0); }
    // The history is pre-exposed by last frame's exposure.
    let col = textureSampleLevel(history_tex, screen_samp, puv, 0.0).rgb / max(exposure_state.prev_pre, 1e-8);
    // Fade toward the screen border, the end of the ray and grazing
    // directions toward the camera (likely to be hidden by the foreground).
    let edge = min(min(puv.x, 1.0 - puv.x), min(puv.y, 1.0 - puv.y));
    let fade_edge = smoothstep(0.0, 0.08, edge);
    let fade_dist = 1.0 - smoothstep(0.7, 1.0, length(hit_pos - p) / max_dist);
    let fade_cam = 1.0 - smoothstep(0.6, 0.95, normalize(rv).z);
    return vec4<f32>(col, fade_rough * facing * fade_edge * fade_dist * fade_cam);
}

// Reflection direction for SSR: the mirror direction for smooth surfaces,
// otherwise reflected about a GGX microfacet normal sampled from the
// visible-normal distribution (Heitz 2018). The lobe is truncated (u.x
// scaled) to trade a little blur for much less noise.
fn ssr_direction(n: vec3<f32>, v: vec3<f32>, rough: f32, pix: vec2<f32>) -> vec3<f32> {
    let mirror = reflect(-v, n);
    if (rough < 0.08) { return mirror; }
    let a = rough * rough;
    let helper = select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 1.0, 0.0), abs(n.x) > 0.7);
    let t = normalize(helper - n * dot(n, helper));
    let b = cross(n, t);
    let vl = vec3<f32>(dot(v, t), dot(v, b), dot(v, n));
    let u = vec2<f32>(ign(pix + vec2<f32>(13.0, 71.0)) * 0.7, ign(pix + vec2<f32>(53.0, 7.0)));
    // VNDF sample in the local frame (z = normal).
    let vh = normalize(vec3<f32>(a * vl.x, a * vl.y, max(vl.z, 1e-4)));
    let lensq = vh.x * vh.x + vh.y * vh.y;
    let t1 = select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(-vh.y, vh.x, 0.0) * inverseSqrt(max(lensq, 1e-12)), lensq > 1e-12);
    let t2 = cross(vh, t1);
    let rr = sqrt(u.x);
    let phi = 6.2831853 * u.y;
    let p1 = rr * cos(phi);
    let s = 0.5 * (1.0 + vh.z);
    let p2 = (1.0 - s) * sqrt(max(1.0 - p1 * p1, 0.0)) + s * rr * sin(phi);
    let nh = p1 * t1 + p2 * t2 + sqrt(max(1.0 - p1 * p1 - p2 * p2, 0.0)) * vh;
    let hl = normalize(vec3<f32>(a * nh.x, a * nh.y, max(nh.z, 1e-4)));
    let h = t * hl.x + b * hl.y + n * hl.z;
    let r = reflect(-v, h);
    return select(mirror, r, dot(r, n) > 0.02);
}

// GTAO multi-bounce approximation (Jimenez 2016): bright albedo gets light
// back from inter-reflections, so occlusion darkens it less.
fn ao_multibounce(ao: f32, albedo: vec3<f32>) -> vec3<f32> {
    let a = 2.0404 * albedo - vec3<f32>(0.3324);
    let b = -4.7951 * albedo + vec3<f32>(0.6417);
    let c = 2.7552 * albedo + vec3<f32>(0.6903);
    return max(vec3<f32>(ao), ((ao * a + b) * ao + c) * ao);
}

// Ambient occlusion from GTAO at this pixel (1 for blended surfaces).
fn screen_ao(clip: vec4<f32>, on_surface: bool) -> f32 {
    if (!on_surface || frame.fx.x <= 0.0) { return 1.0; }
    let ao = textureLoad(ssao_tex, vec2<i32>(clip.xy), 0).r;
    return pow(ao, frame.fx.x);
}


// ---- Wet surfaces -------------------------------------------------------------

fn hash21(p: vec2<f32>) -> f32 {
    let q = fract(p * vec2<f32>(0.1031, 0.1030));
    let r = q + dot(q, q.yx + 33.33);
    return fract((r.x + r.y) * r.x);
}

fn value_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    return mix(mix(hash21(i), hash21(i + vec2<f32>(1.0, 0.0)), u.x), mix(hash21(i + vec2<f32>(0.0, 1.0)), hash21(i + vec2<f32>(1.0, 1.0)), u.x), u.y);
}

// Puddle coverage noise by world position (0..1, smooth at 1 m scale).
fn puddle_noise(xz: vec2<f32>) -> f32 {
    let p = xz * frame.wet.z;
    return value_noise(p) * 0.55 + value_noise(p * 2.07 + vec2<f32>(5.2, 1.3)) * 0.3 + value_noise(p * 4.3 + vec2<f32>(9.1, 7.7)) * 0.15;
}

struct WetSurface {
    albedo: vec3<f32>,
    rough: f32,
    n: vec3<f32>,
};

// Rain-wet surface (Lagarde 2013): water fills porous materials, which
// darken and turn glossy; non-porous ones (metal, paint, glass) just get a
// smoother film. Upward faces are fully wet, walls a little (rain hits them
// slantwise), undersides stay dry. Flat ground also collects puddles: a
// world-space noise mask where the surface becomes a near-mirror with a
// flat normal, which is what stretches lamps and neon into the long
// reflections of a wet street (direct specular of the clustered lights plus
// SSR of the emissive signs).
fn wet_surface(albedo: vec3<f32>, rough: f32, metallic: f32, n: vec3<f32>, ng: vec3<f32>, p: vec3<f32>) -> WetSurface {
    var o = WetSurface(albedo, rough, n);
    let up = ng.y;
    let wet = frame.wet.x * mix(0.2, 1.0, smoothstep(0.25, 0.85, up)) * smoothstep(-0.6, -0.2, up);
    if (wet <= 0.0) { return o; }
    let porosity = clamp((rough - 0.25) / 0.5, 0.0, 1.0) * (1.0 - metallic);
    o.albedo = albedo * mix(1.0, 0.42, wet * porosity);
    o.rough = mix(rough, min(rough, mix(0.1, 0.24, porosity)), wet);
    // Water fills the small relief of the normal map.
    o.n = normalize(mix(n, ng, wet * (0.6 + 0.35 * porosity)));
    if (frame.wet.y > 0.0 && up > 0.9) {
        let th = 1.0 - 0.5 * frame.wet.y;
        let m = smoothstep(th - 0.05, th + 0.05, puddle_noise(p.xz)) * frame.wet.x * smoothstep(0.9, 0.98, up);
        o.rough = mix(o.rough, 0.025, m);
        o.n = normalize(mix(o.n, ng, m));
        o.albedo = o.albedo * mix(1.0, 0.7, m);
    }
    return o;
}

// ---- Clustered local lights -----------------------------------------------------

// Shadow of a shadow-mapped spot light (5-tap hardware PCF).
fn spot_shadow(layer: i32, p: vec3<f32>, ng: vec3<f32>, dist: f32) -> f32 {
    let texel = dist * 2.5 / 1024.0;
    let q = p + ng * (texel * 1.5);
    let c = frame.spot_vp[layer] * vec4<f32>(q, 1.0);
    if (c.w <= 1e-4) { return 1.0; }
    let s = c.xyz / c.w;
    let uv = vec2<f32>(s.x * 0.5 + 0.5, 0.5 - s.y * 0.5);
    if (any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0)) || s.z > 1.0) { return 1.0; }
    let t = 1.0 / 1024.0;
    var sum = textureSampleCompareLevel(spot_shadow_map, shadow_sampler, uv, layer, s.z);
    sum += textureSampleCompareLevel(spot_shadow_map, shadow_sampler, uv + vec2<f32>(t, t), layer, s.z);
    sum += textureSampleCompareLevel(spot_shadow_map, shadow_sampler, uv + vec2<f32>(-t, t), layer, s.z);
    sum += textureSampleCompareLevel(spot_shadow_map, shadow_sampler, uv + vec2<f32>(t, -t), layer, s.z);
    sum += textureSampleCompareLevel(spot_shadow_map, shadow_sampler, uv + vec2<f32>(-t, -t), layer, s.z);
    return sum / 5.0;
}

// Direct light of the cluster's point and spot lights: inverse square with
// the source radius softening (1 / (d² + r²)), a smooth range window, spot
// cones, and GGX highlights widened by the source size (sphere-light
// roughness, Karis 2013).
// Cost: one light read + ~60 ALU per light in the cluster.
fn shade_local(p: vec3<f32>, n: vec3<f32>, ng: vec3<f32>, v: vec3<f32>, nv: f32, a: f32, f0: vec3<f32>, spec_k: f32, diffuse_color: vec3<f32>, pix: vec2<f32>, view_depth: f32) -> vec3<f32> {
    let base = cluster_of(pix, view_depth);
    let count = min(clusters[base], 96u);
    var sum = vec3<f32>(0.0);
    for (var i = 0u; i < count; i++) {
        let l = local_lights[cluster_light(base, i)];
        let to_l = l.pos_range.xyz - p;
        let d2 = dot(to_l, to_l);
        let r = l.pos_range.w;
        if (d2 >= r * r) { continue; }
        let inv_d = inverseSqrt(max(d2, 1e-8));
        let ll = to_l * inv_d;
        let pnl = dot(n, ll);
        if (pnl <= 0.0) { continue; }
        let cone = spot_cone(l, -ll);
        if (cone <= 0.0) { continue; }
        let rs = l.color.w;
        var atten = range_window(d2, r) * cone / (d2 + rs * rs);
        if (l.params.z >= 0.0) {
            atten *= spot_shadow(i32(l.params.z + 0.5), p, ng, d2 * inv_d);
        }
        let a2 = clamp(a + 0.5 * rs * inv_d, 0.0, 1.0);
        let ph = normalize(v + ll);
        // spec_k: porous-material Fresnel scale, as for the sun.
        let pf = f_schlick(f0, max(dot(v, ph), 0.0)) * spec_k;
        let pspec = pf * d_ggx(max(dot(n, ph), 0.0), a2) * v_smith(nv, pnl, a);
        let pdiff = (vec3<f32>(1.0) - pf) * diffuse_color / PI;
        sum += (pdiff + pspec) * l.color.rgb * (pnl * atten);
    }
    return sum;
}

@fragment
fn fs_main(in: VertexOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let base_s = textureSample(base_tex, tex_sampler, in.uv);
    let base = base_s * material.base_color * in.tint;
    if (base.a < material.params.z) { discard; }
    if (material.params.w > 0.5) {
        if (frame.params.w > 0.5) {
            // Daylight camera: unlit (KHR_materials_unlit) colours are
            // display referred, so divide by the exposure the post pass
            // applies: they show as authored at any EV.
            return vec4<f32>(height_fog(base.rgb / max(frame.sun_color.w, 1e-6), in.world_pos, normalize(in.world_pos - frame.camera_pos.xyz)), base.a);
        }
        // Night camera: constant on screen (auto exposure: exposure / pre ~ 1).
        let k = select(frame.sun_color.w, 1.0, frame.lighting.y > 0.5);
        return vec4<f32>(height_fog(base.rgb * k * 3.0, in.world_pos, normalize(in.world_pos - frame.camera_pos.xyz)), base.a);
    }
    var ng = normalize(in.normal);
    if (!front) { ng = -ng; }
    let t = normalize(in.tangent.xyz - ng * dot(ng, in.tangent.xyz));
    let b = cross(ng, t) * in.tangent.w;
    var nm = textureSample(normal_tex, tex_sampler, in.uv).xyz * 2.0 - vec3<f32>(1.0);
    let nm_len = length(nm);
    nm = vec3<f32>(nm.xy * material.emissive.w, nm.z);
    var n = normalize(mat3x3<f32>(t, b, ng) * nm);

    let mr = textureSample(mr_tex, tex_sampler, in.uv);
    let metallic = clamp(material.params.x * mr.b, 0.0, 1.0);
    var rough_in = clamp(material.params.y * mr.g, 0.045, 1.0);
    var albedo = base.rgb;
    if (frame.scatter.w > 0.5 && material.layers.z <= 0.0 && material.layers.w <= 0.0) {
        let w = wet_surface(albedo, rough_in, metallic, n, ng, in.world_pos);
        albedo = w.albedo;
        rough_in = w.rough;
        n = w.n;
    }
    // Authored (or wet) roughness widened by the normal variance in the
    // pixel footprint (specular anti-aliasing); every specular term uses it.
    let a = sqrt(specular_aa(n, rough_in * rough_in * rough_in * rough_in, nm_len, material.emissive.w));
    let rough = sqrt(a);
    let ao = textureSample(occlusion_tex, tex_sampler, in.uv).r;
    let f0 = mix(vec3<f32>(0.04), albedo, metallic);
    // Dielectric Fresnel scale (porous masonry reflects less than its
    // solid mineral: pores trap the specular, KHR_materials_specular).
    let spec_k = select(mix(material.extra.x, 1.0, metallic), 1.0, legacy(16u));
    let diffuse_color = albedo * (1.0 - metallic);

    let v = normalize(frame.camera_pos.xyz - in.world_pos);
    let nv = max(dot(n, v), 1e-4);
    let l = frame.sun_dir.xyz;
    let h = normalize(v + l);
    let nl = max(dot(n, l), 0.0);
    let nh = max(dot(n, h), 0.0);
    let vh = max(dot(v, h), 0.0);

    // Clearcoat: a smooth dielectric layer over the base (car paint).
    let cc = material.layers.x;
    let cc_a = max(material.layers.y, 0.02) * max(material.layers.y, 0.02);
    let ngc = ng;
    var color = vec3<f32>(0.0);
    // Specular-only radiance for the `FORJA_DEBUG=spec` view.
    var spec_dbg = vec3<f32>(0.0);
    var glass_sun = vec3<f32>(0.0);
    // Skin: light diffuses under the surface. Diffuse uses a softened
    // normal and per-channel wrap so red reaches further past the
    // terminator, and shadow edges bleed red (pre-integrated SSS, cheap).
    let sss = material.layers.w;
    let nd = normalize(mix(n, ng, 0.55 * sss));
    let ndl = dot(nd, l);
    let on_surface = is_prepass_surface(in.clip);
    var shadow = 1.0;
    if (nl > 0.0 || (sss > 0.0 && ndl > -0.5)) {
        shadow = shadow_factor(in.world_pos, ng, in.view_depth, in.clip.xy);
        if (shadow > 0.0 && on_surface && frame.fx.y > 0.5) {
            shadow *= contact_shadow(in.world_pos, ng, in.view_depth, in.clip.xy);
        }
    }
    // Micro-shadowing (Chan 2018, "Material Advances in Call of Duty:
    // WWII"): the material AO acts as a visibility cone, so grazing sun is
    // shadowed inside brick joints, render pores and stone tooling. Rough
    // walls then show a broken, textured sheen instead of a smooth plastic
    // gradient. Skin keeps its own soft terminator.
    if (sss <= 0.0 && !legacy(8u)) {
        shadow *= clamp(abs(nl) + 2.0 * ao * ao - 1.0, 0.0, 1.0);
    }
    var dterm = vec3<f32>(nl * shadow);
    if (sss > 0.0) {
        // Measured skin scatters red light ~3x further than blue; keep the
        // wrap small so lit skin stays neutral and only the terminator and
        // shadow edges warm up.
        let w = vec3<f32>(0.10, 0.035, 0.015) * sss;
        let wrapv = clamp((vec3<f32>(ndl) + w) / (vec3<f32>(1.0) + w), vec3<f32>(0.0), vec3<f32>(1.0));
        dterm = wrapv * shadow;
    }
    if (nl > 0.0) {
        let f = f_schlick(f0, vh) * spec_k;
        glass_sun = d_ggx(nh, a) * v_smith(nv, nl, a) * frame.sun_color.rgb * nl * shadow;
        var spec = f * d_ggx(nh, a) * v_smith(nv, nl, a);
        if (sss > 0.0) {
            // Two specular lobes: broad oily sheen plus a tight highlight.
            let a2 = max(a * 0.35, 0.02);
            spec = f * (0.8 * d_ggx(nh, a) * v_smith(nv, nl, a) + 0.2 * d_ggx(nh, a2) * v_smith(nv, nl, a2));
        }
        var lit = spec * spec_energy(f0, rough, nv) * nl * shadow;
        if (cc > 0.0) {
            let fc = f_schlick(vec3<f32>(0.04), vh).x * cc;
            let ncl = max(dot(ngc, l), 0.0);
            let spec_c = fc * d_ggx(max(dot(ngc, h), 0.0), cc_a) * v_smith(max(dot(ngc, v), 1e-4), ncl, cc_a);
            lit = lit * (1.0 - fc) + vec3<f32>(spec_c) * nl * shadow;
        }
        color += lit * frame.sun_color.rgb;
        spec_dbg += lit * frame.sun_color.rgb;
    }
    color += (vec3<f32>(1.0) - f_schlick(f0, max(dot(nd, normalize(v + l)), 0.0)) * spec_k) * diffuse_color / PI * dterm * frame.sun_color.rgb;
    // Clustered point and spot lights.
    if (frame.lighting.x > 0.0) {
        color += shade_local(in.world_pos, n, ng, v, nv, a, f0, spec_k, diffuse_color, in.clip.xy, in.view_depth);
    }
    // Image-based ambient (HDRI or analytic sky). Diffuse is occluded by the
    // material AO and GTAO (with multi-bounce); reflections by specular
    // occlusion (Lagarde 2014) and horizon fading, so crevices, window
    // reveals and balcony undersides do not mirror the open sky.
    // Split-sum weights with multiple-scattering compensation: rough
    // dielectrics keep their energy (no dark rims) and diffuse loses what
    // the specular layer reflects (energy conservation).
    let ibl = ibl_weights(f0, spec_k, rough, nv);
    let occ = min(ao, screen_ao(in.clip, on_surface));
    let spec_occ = mix(1.0, clamp(pow(nv + occ, exp2(-16.0 * rough - 1.0)) - 1.0 + occ, 0.0, 1.0), frame.ao_params.y);
    let r = reflect(-v, n);
    let horizon = clamp(1.0 + 1.2 * dot(r, ng), 0.0, 1.0);
    color += diffuse_color * ibl.diffuse * ambient_sky(nd) * ao_multibounce(occ, diffuse_color);
    color += ibl.multi * ambient_sky(n) * (occ * spec_occ);
    spec_dbg += ibl.multi * ambient_sky(n) * (occ * spec_occ);
    // One screen-space reflection ray per pixel, along the glossiest lobe
    // (clearcoat when there is one). Its hits replace the environment, which
    // only keeps its occlusion terms where the ray found nothing.
    let rc = reflect(-v, ngc);
    var ssr: vec4<f32>;
    if (cc > 0.0) {
        ssr = ssr_trace(in.world_pos, ngc, v, material.layers.y, in.clip.xy);
    } else {
        ssr = ssr_trace(in.world_pos, n, v, rough, in.clip.xy);
    }
    let ssr_base = select(ssr, vec4<f32>(0.0), cc > 0.0);
    let ibl_spec = mix(env_reflection(r, rough) * (spec_occ * horizon * horizon), ssr_base.rgb, ssr_base.a) * ibl.spec;
    color += ibl_spec;
    spec_dbg += ibl_spec;
    if (cc > 0.0) {
        let fcv = f_schlick(vec3<f32>(0.04), max(dot(ngc, v), 1e-4)).x * cc;
        let horizon_c = clamp(1.0 + 1.2 * dot(rc, ng), 0.0, 1.0);
        color = color * (1.0 - fcv) + mix(env_reflection(rc, material.layers.y) * horizon_c, ssr.rgb, ssr.a) * fcv;
    }
    if (frame.ao_params.w > 0.5) {
        // Debug views (FORJA_DEBUG).
        let dv = i32(frame.ao_params.w + 0.5);
        if (dv == 1) { return vec4<f32>(vec3<f32>(occ), 1.0); }
        if (dv == 2) { return vec4<f32>(vec3<f32>(shadow), 1.0); }
        if (dv == 4) { return vec4<f32>(spec_dbg, 1.0); }
        if (dv == 5) { return vec4<f32>(vec3<f32>(rough), 1.0); }
        // SSR reads last frame, which is this debug image: show where it
        // hits (confidence) rather than what it reflects.
        return vec4<f32>(ssr.a, ssr.a, ssr.a * 0.3, 1.0);
    }
    color += textureSample(emissive_tex, tex_sampler, in.uv).rgb * material.emissive.rgb * (4.0 * frame.wet.w);

    let dist = length(in.world_pos - frame.camera_pos.xyz);
    let glow = local_inscatter(in.clip.xy, -v, dist);
    color = height_fog(color, in.world_pos, -v) + glow;
    var alpha = base.a;
    let tr = material.layers.z;
    if (tr > 0.0) {
        // Thin glass: blend weight is the Fresnel reflectance (plus the
        // opaque share 1 - transmission), color is what the glass
        // reflects. Over a dark interior this reads like real windows.
        let fr = f_schlick(vec3<f32>(0.04), nv).x;
        alpha = clamp(fr + (1.0 - tr) * (1.0 - fr), 0.03, 1.0);
        let refl = mix(env_reflection(r, rough), ssr.rgb, ssr.a) + glass_sun * fr / max(alpha, 0.03);
        color = height_fog(refl * (fr / alpha) + albedo * ambient_sky(n) * (1.0 - fr / alpha) * 0.3, in.world_pos, -v) + glow;
    }
    return vec4<f32>(min(color * exposure_state.pre, vec3<f32>(60000.0)), alpha);
}
