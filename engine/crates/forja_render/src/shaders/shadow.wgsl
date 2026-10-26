// Depth-only pass into one shadow cascade.
struct Cascade { vp: mat4x4<f32> };
@group(0) @binding(0) var<uniform> cascade: Cascade;
@group(1) @binding(0) var<storage, read> joints: array<mat4x4<f32>>;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(4) m0: vec4<f32>,
    @location(5) m1: vec4<f32>,
    @location(6) m2: vec4<f32>,
    @location(7) m3: vec4<f32>,
    @location(9) extra: vec4<u32>,
};

struct SkinIn {
    @location(10) joint_ids: vec4<u32>,
    @location(11) weights: vec4<f32>,
};

@vertex
fn vs_main(v: VertexIn) -> @builtin(position) vec4<f32> {
    let model = mat4x4<f32>(v.m0, v.m1, v.m2, v.m3);
    return cascade.vp * model * vec4<f32>(v.position, 1.0);
}

@vertex
fn vs_skinned(v: VertexIn, s: SkinIn) -> @builtin(position) vec4<f32> {
    let base = v.extra.x;
    let skin = joints[base + s.joint_ids.x] * s.weights.x
        + joints[base + s.joint_ids.y] * s.weights.y
        + joints[base + s.joint_ids.z] * s.weights.z
        + joints[base + s.joint_ids.w] * s.weights.w;
    let model = mat4x4<f32>(v.m0, v.m1, v.m2, v.m3) * skin;
    return cascade.vp * model * vec4<f32>(v.position, 1.0);
}
