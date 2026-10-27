// Depth-only pass into one shadow cascade. Alpha-masked materials (leaves,
// hair cards, fences) use the `_masked` entry points so they cast the shadow
// of their texture, not of the whole card.
struct Cascade { vp: mat4x4<f32> };
@group(0) @binding(0) var<uniform> cascade: Cascade;
@group(1) @binding(0) var<storage, read> joints: array<mat4x4<f32>>;

struct Material {
    base_color: vec4<f32>,
    emissive: vec4<f32>,
    params: vec4<f32>,     // z: alpha cutoff
    layers: vec4<f32>,
    foliage: vec4<f32>,
    extra: vec4<f32>,
};
@group(2) @binding(0) var<uniform> material: Material;
@group(2) @binding(1) var base_tex: texture_2d<f32>;
@group(2) @binding(6) var tex_sampler: sampler;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(2) uv: vec2<f32>,
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

struct MaskedOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) alpha: f32,
};

fn skin_matrix(v: VertexIn, s: SkinIn) -> mat4x4<f32> {
    let base = v.extra.x;
    return joints[base + s.joint_ids.x] * s.weights.x
        + joints[base + s.joint_ids.y] * s.weights.y
        + joints[base + s.joint_ids.z] * s.weights.z
        + joints[base + s.joint_ids.w] * s.weights.w;
}

@vertex
fn vs_main(v: VertexIn) -> @builtin(position) vec4<f32> {
    let model = mat4x4<f32>(v.m0, v.m1, v.m2, v.m3);
    return cascade.vp * model * vec4<f32>(v.position, 1.0);
}

@vertex
fn vs_skinned(v: VertexIn, s: SkinIn) -> @builtin(position) vec4<f32> {
    let model = mat4x4<f32>(v.m0, v.m1, v.m2, v.m3) * skin_matrix(v, s);
    return cascade.vp * model * vec4<f32>(v.position, 1.0);
}

@vertex
fn vs_masked(v: VertexIn) -> MaskedOut {
    let model = mat4x4<f32>(v.m0, v.m1, v.m2, v.m3);
    var o: MaskedOut;
    o.clip = cascade.vp * model * vec4<f32>(v.position, 1.0);
    o.uv = v.uv;
    o.alpha = v.tint.a;
    return o;
}

@vertex
fn vs_skinned_masked(v: VertexIn, s: SkinIn) -> MaskedOut {
    let model = mat4x4<f32>(v.m0, v.m1, v.m2, v.m3) * skin_matrix(v, s);
    var o: MaskedOut;
    o.clip = cascade.vp * model * vec4<f32>(v.position, 1.0);
    o.uv = v.uv;
    o.alpha = v.tint.a;
    return o;
}

@fragment
fn fs_masked(in: MaskedOut) {
    let a = textureSample(base_tex, tex_sampler, in.uv).a * material.base_color.a * in.alpha;
    if (a < material.params.z) { discard; }
}
