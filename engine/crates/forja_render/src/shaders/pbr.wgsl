// Forward PBR with cascaded sun shadows, sky ambient, point lights and fog.
// `common.wgsl` is prepended at load time.

struct Material {
    base_color: vec4<f32>,
    emissive: vec4<f32>,   // w: normal scale
    params: vec4<f32>,     // x: metallic, y: roughness, z: alpha cutoff, w: unlit
    layers: vec4<f32>,     // x: clearcoat, y: clearcoat roughness, z: transmission, w: subsurface
    foliage: vec4<f32>,    // x: translucency, y: cull back faces above this v, z: foliage on
};

@group(0) @binding(1) var shadow_map: texture_depth_2d_array;
@group(0) @binding(2) var shadow_sampler: sampler_comparison;

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
// Impostor quads (foliage texels past `foliage.y` in v) show one view per
// face: back faces are dropped, and quads seen at grazing angles dissolve
// (dithered, resolved by TAA) so crossed planes never show as thin seams.
// `face` is the geometric normal from screen derivatives, taken by the
// caller in uniform control flow.
fn impostor_hidden(in: VertexOut, front: bool, face: vec3<f32>) -> bool {
    // Only foliage materials have impostor texels: other materials tile
    // their textures, so their v can take any value.
    if (material.foliage.z < 0.5 || in.uv.y <= material.foliage.y) { return false; }
    if (!front) { return true; }
    let c = abs(dot(normalize(face), normalize(frame.camera_pos.xyz - in.world_pos)));
    let p = vec2<u32>(in.clip.xy);
    let h = f32((p.x * 73856093u) ^ (p.y * 19349663u) ^ u32(frame.jitter.z * 997.0)) / 4294967295.0;
    return c < mix(0.42, 0.72, fract(h * 7.31));
}

@fragment
fn fs_prepass(in: VertexOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let n = normalize(in.normal);
    return vec4<f32>(select(-n, n, front), 1.0);
}

@fragment
fn fs_prepass_masked(in: VertexOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let face = cross(dpdx(in.world_pos), dpdy(in.world_pos));
    let a = textureSample(base_tex, tex_sampler, in.uv).a * material.base_color.a * in.tint.a;
    if (a < material.params.z) { discard; }
    if (impostor_hidden(in, front, face)) { discard; }
    let n = normalize(in.normal);
    return vec4<f32>(select(-n, n, front || material.foliage.z > 0.5), 1.0);
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
    let col = textureSampleLevel(history_tex, screen_samp, puv, 0.0).rgb;
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

@fragment
fn fs_main(in: VertexOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let face = cross(dpdx(in.world_pos), dpdy(in.world_pos));
    let base_s = textureSample(base_tex, tex_sampler, in.uv);
    let base = base_s * material.base_color * in.tint;
    if (base.a < material.params.z) { discard; }
    // One-sided impostor quads inside a double-sided foliage material.
    if (impostor_hidden(in, front, face)) { discard; }
    if (material.params.w > 0.5) {
        return vec4<f32>(height_fog(base.rgb * frame.sun_color.w * 3.0, in.world_pos, normalize(in.world_pos - frame.camera_pos.xyz)), base.a);
    }
    var ng = normalize(in.normal);
    // Foliage cards carry bent crown normals on both faces.
    let fol = material.foliage.z > 0.5;
    if (!front && !fol) { ng = -ng; }
    let t = normalize(in.tangent.xyz - ng * dot(ng, in.tangent.xyz));
    let b = cross(ng, t) * in.tangent.w;
    var nm = textureSample(normal_tex, tex_sampler, in.uv).xyz * 2.0 - vec3<f32>(1.0);
    nm = vec3<f32>(nm.xy * material.emissive.w, nm.z);
    let n = normalize(mat3x3<f32>(t, b, ng) * nm);

    let mr = textureSample(mr_tex, tex_sampler, in.uv);
    let metallic = clamp(material.params.x * mr.b, 0.0, 1.0);
    let rough = clamp(material.params.y * mr.g, 0.045, 1.0);
    let a = rough * rough;
    let ao = textureSample(occlusion_tex, tex_sampler, in.uv).r;
    let albedo = base.rgb;
    let f0 = mix(vec3<f32>(0.04), albedo, metallic);
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
    var glass_sun = vec3<f32>(0.0);
    // Skin: light diffuses under the surface. Diffuse uses a softened
    // normal and per-channel wrap so red reaches further past the
    // terminator, and shadow edges bleed red (pre-integrated SSS, cheap).
    let sss = material.layers.w;
    let nd = normalize(mix(n, ng, 0.55 * sss));
    let ndl = dot(nd, l);
    let on_surface = is_prepass_surface(in.clip);
    var shadow = 1.0;
    if (nl > 0.0 || fol || (sss > 0.0 && ndl > -0.5)) {
        shadow = shadow_factor(in.world_pos, ng, in.view_depth, in.clip.xy);
        if (shadow > 0.0 && on_surface && frame.fx.y > 0.5) {
            shadow *= contact_shadow(in.world_pos, ng, in.view_depth, in.clip.xy);
        }
    }
    var dterm = vec3<f32>(nl * shadow);
    if (fol) {
        // Wrapped diffuse for the soft, many-layered crown, plus light
        // transmitted through the leaves: toward the viewer when looking
        // into the sun, and onto the far side of the crown. Transmitted
        // light is warmer (chlorophyll passes yellow-green).
        let wrap = 0.45;
        let wd = clamp((dot(n, l) + wrap) / (1.0 + wrap), 0.0, 1.0);
        let back = pow(clamp(dot(-v, l), 0.0, 1.0), 4.0);
        let through = material.foliage.x * (0.3 * clamp(-dot(ng, l) + 0.3, 0.0, 1.0) + 0.9 * back);
        dterm = (vec3<f32>(wd) + through * vec3<f32>(1.05, 1.12, 0.55)) * shadow;
    }
    if (sss > 0.0) {
        // Measured skin scatters red light ~3x further than blue; keep the
        // wrap small so lit skin stays neutral and only the terminator and
        // shadow edges warm up.
        let w = vec3<f32>(0.10, 0.035, 0.015) * sss;
        let wrapv = clamp((vec3<f32>(ndl) + w) / (vec3<f32>(1.0) + w), vec3<f32>(0.0), vec3<f32>(1.0));
        dterm = wrapv * shadow;
    }
    if (nl > 0.0) {
        let f = f_schlick(f0, vh);
        glass_sun = d_ggx(nh, a) * v_smith(nv, nl, a) * frame.sun_color.rgb * nl * shadow;
        var spec = f * d_ggx(nh, a) * v_smith(nv, nl, a);
        if (sss > 0.0) {
            // Two specular lobes: broad oily sheen plus a tight highlight.
            let a2 = max(a * 0.35, 0.02);
            spec = f * (0.8 * d_ggx(nh, a) * v_smith(nv, nl, a) + 0.2 * d_ggx(nh, a2) * v_smith(nv, nl, a2));
        }
        var lit = spec * nl * shadow;
        if (cc > 0.0) {
            let fc = f_schlick(vec3<f32>(0.04), vh).x * cc;
            let ncl = max(dot(ngc, l), 0.0);
            let spec_c = fc * d_ggx(max(dot(ngc, h), 0.0), cc_a) * v_smith(max(dot(ngc, v), 1e-4), ncl, cc_a);
            lit = lit * (1.0 - fc) + vec3<f32>(spec_c) * nl * shadow;
        }
        color += lit * frame.sun_color.rgb;
    }
    color += (vec3<f32>(1.0) - f_schlick(f0, max(dot(nd, normalize(v + l)), 0.0))) * diffuse_color / PI * dterm * frame.sun_color.rgb;
    // Point lights with smooth range falloff (Karis).
    let count = i32(frame.params.x);
    for (var i = 0; i < count; i++) {
        let light = frame.lights[i];
        let to_l = light.pos_radius.xyz - in.world_pos;
        let d2 = dot(to_l, to_l);
        let r = light.pos_radius.w;
        if (d2 > r * r) { continue; }
        let ll = to_l * inverseSqrt(d2);
        let pnl = max(dot(n, ll), 0.0);
        if (pnl <= 0.0) { continue; }
        let w = clamp(1.0 - pow(d2 / (r * r), 2.0), 0.0, 1.0);
        let atten = w * w / (d2 + 1.0);
        let ph = normalize(v + ll);
        let pf = f_schlick(f0, max(dot(v, ph), 0.0));
        let pspec = pf * d_ggx(max(dot(n, ph), 0.0), a) * v_smith(nv, pnl, a);
        let pdiff = (vec3<f32>(1.0) - pf) * diffuse_color / PI;
        color += (pdiff + pspec) * light.color.rgb * light.color.w * pnl * atten;
    }
    // Image-based ambient (HDRI or analytic sky). Diffuse is occluded by the
    // material AO and GTAO (with multi-bounce); reflections by specular
    // occlusion (Lagarde 2014) and horizon fading, so crevices, window
    // reveals and balcony undersides do not mirror the open sky.
    let env = env_brdf(f0, rough, nv);
    let occ = min(ao, screen_ao(in.clip, on_surface));
    let spec_occ = mix(1.0, clamp(pow(nv + occ, exp2(-16.0 * rough - 1.0)) - 1.0 + occ, 0.0, 1.0), frame.ao_params.y);
    let r = reflect(-v, n);
    let horizon = clamp(1.0 + 1.2 * dot(r, ng), 0.0, 1.0);
    color += diffuse_color * ambient_sky(nd) * ao_multibounce(occ, diffuse_color);
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
    color += mix(env_reflection(r, rough) * (spec_occ * horizon * horizon), ssr_base.rgb, ssr_base.a) * env;
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
        // SSR reads last frame, which is this debug image: show where it
        // hits (confidence) rather than what it reflects.
        return vec4<f32>(ssr.a, ssr.a, ssr.a * 0.3, 1.0);
    }
    color += textureSample(emissive_tex, tex_sampler, in.uv).rgb * material.emissive.rgb * 4.0;

    color = height_fog(color, in.world_pos, -v);
    var alpha = base.a;
    let tr = material.layers.z;
    if (tr > 0.0) {
        // Thin glass: blend weight is the Fresnel reflectance (plus the
        // opaque share 1 - transmission), color is what the glass
        // reflects. Over a dark interior this reads like real windows.
        let fr = f_schlick(vec3<f32>(0.04), nv).x;
        alpha = clamp(fr + (1.0 - tr) * (1.0 - fr), 0.03, 1.0);
        let refl = mix(env_reflection(r, rough), ssr.rgb, ssr.a) + glass_sun * fr / max(alpha, 0.03);
        color = height_fog(refl * (fr / alpha) + albedo * ambient_sky(n) * (1.0 - fr / alpha) * 0.3, in.world_pos, -v);
    }
    return vec4<f32>(color, alpha);
}
