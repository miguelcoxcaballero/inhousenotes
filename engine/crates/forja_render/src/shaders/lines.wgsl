// Debug lines in world space (physics shapes, rays, wheel contacts).
struct LineIn {
    @location(0) position: vec3<f32>,
    @location(1) color: vec4<f32>,
};
struct LineOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_main(v: LineIn) -> LineOut {
    var o: LineOut;
    o.clip = frame.view_proj * vec4<f32>(v.position, 1.0);
    o.color = v.color;
    return o;
}

@fragment
fn fs_main(in: LineOut) -> @location(0) vec4<f32> {
    return vec4<f32>(in.color.rgb * exposure_state.pre, in.color.a);
}
