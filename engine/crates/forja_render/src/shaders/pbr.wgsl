// Forward PBR with cascaded sun shadows, sky ambient, point lights and fog.
// `common.wgsl` is prepended at load time.

struct Material {
    base_color: vec4<f32>,
    emissive: vec4<f32>,   // w: normal scale
    params: vec4<f32>,     // x: metallic, y: roughness, z: alpha cutoff, w: unlit
    layers: vec4<f32>,     // x: clearcoat, y: clearcoat roughness, z: transmission
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
    @builtin(position) clip: vec4<f32>,
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

fn shadow_factor(world_pos: vec3<f32>, n: vec3<f32>, view_depth: f32) -> f32 {
    var cascade = 3;
    for (var i = 0; i < 4; i++) {
        if (view_depth < frame.cascade_splits[i]) { cascade = i; break; }
    }
    if (view_depth > frame.cascade_splits[3]) { return 1.0; }
    let texel = frame.params.y;
    // Normal offset grows with cascade footprint.
    let world_texel = (frame.cascade_splits[cascade] * 2.0) * texel;
    let p = world_pos + n * world_texel * 1.5;
    let lc = frame.cascade_vp[cascade] * vec4<f32>(p, 1.0);
    let uv = vec2<f32>(lc.x * 0.5 + 0.5, 0.5 - lc.y * 0.5);
    if (any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0)) || lc.z > 1.0) { return 1.0; }
    let depth = lc.z - 0.0008;
    // 12-tap Poisson PCF on top of the hardware 2x2 filter.
    var taps = array<vec2<f32>, 12>(
        vec2<f32>(-0.326, -0.406), vec2<f32>(-0.840, -0.074), vec2<f32>(-0.696, 0.457),
        vec2<f32>(-0.203, 0.621), vec2<f32>(0.962, -0.195), vec2<f32>(0.473, -0.480),
        vec2<f32>(0.519, 0.767), vec2<f32>(0.185, -0.893), vec2<f32>(0.507, 0.064),
        vec2<f32>(0.896, 0.412), vec2<f32>(-0.322, -0.933), vec2<f32>(-0.792, -0.598));
    var sum = 0.0;
    for (var i = 0; i < 12; i++) {
        sum += textureSampleCompareLevel(shadow_map, shadow_sampler, uv + taps[i] * texel * 1.8, cascade, depth);
    }
    let s = sum / 12.0;
    // Fade out at the end of the last cascade.
    let fade = clamp((frame.cascade_splits[3] - view_depth) / (frame.cascade_splits[3] * 0.1), 0.0, 1.0);
    return mix(1.0, s, fade);
}

@fragment
fn fs_main(in: VertexOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let base_s = textureSample(base_tex, tex_sampler, in.uv);
    let base = base_s * material.base_color * in.tint;
    if (base.a < material.params.z) { discard; }
    if (material.params.w > 0.5) {
        return vec4<f32>(height_fog(base.rgb * frame.sun_color.w * 3.0, in.world_pos, normalize(in.world_pos - frame.camera_pos.xyz)), base.a);
    }
    var ng = normalize(in.normal);
    if (!front) { ng = -ng; }
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
    if (nl > 0.0) {
        let shadow = shadow_factor(in.world_pos, ng, in.view_depth);
        glass_sun = d_ggx(nh, a) * v_smith(nv, nl, a) * frame.sun_color.rgb * nl * shadow;
        let f = f_schlick(f0, vh);
        let spec = f * d_ggx(nh, a) * v_smith(nv, nl, a);
        let diff = (vec3<f32>(1.0) - f) * diffuse_color / PI;
        var lit = diff + spec;
        if (cc > 0.0) {
            let fc = f_schlick(vec3<f32>(0.04), vh).x * cc;
            let ncl = max(dot(ngc, l), 0.0);
            let spec_c = fc * d_ggx(max(dot(ngc, h), 0.0), cc_a) * v_smith(max(dot(ngc, v), 1e-4), ncl, cc_a);
            lit = lit * (1.0 - fc) + vec3<f32>(spec_c);
        }
        color += lit * frame.sun_color.rgb * nl * shadow;
    }
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
    // Image-based ambient from the analytic sky.
    let env = env_brdf(f0, rough, nv);
    let spec_occ = clamp(pow(nv + ao, exp2(-16.0 * rough - 1.0)) - 1.0 + ao, 0.0, 1.0);
    color += diffuse_color * ambient_sky(n) * ao;
    color += env_reflection(reflect(-v, n), rough) * env * spec_occ;
    if (cc > 0.0) {
        let fcv = f_schlick(vec3<f32>(0.04), max(dot(ngc, v), 1e-4)).x * cc;
        color = color * (1.0 - fcv) + env_reflection(reflect(-v, ngc), material.layers.y) * fcv;
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
        let refl = env_reflection(reflect(-v, n), rough) + glass_sun * fr / max(alpha, 0.03);
        color = height_fog(refl * (fr / alpha) + albedo * ambient_sky(n) * (1.0 - fr / alpha) * 0.3, in.world_pos, -v);
    }
    return vec4<f32>(color, alpha);
}
