// Full-screen atmosphere at the far plane (reverse-Z depth 0).
struct SkyOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> SkyOut {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u)) * 2.0 - vec2<f32>(1.0);
    var o: SkyOut;
    o.clip = vec4<f32>(p, 0.0, 1.0);
    o.ndc = p;
    return o;
}

@fragment
fn fs_main(in: SkyOut) -> @location(0) vec4<f32> {
    let far = frame.inv_view_proj * vec4<f32>(in.ndc, 1e-6, 1.0);
    let d = normalize(far.xyz / far.w - frame.camera_pos.xyz);
    let l = frame.sun_dir.xyz;
    let alt = frame.camera_pos.y * 0.001;
    var c = sky_radiance(d, l, alt) * frame.sun_dir.w;
    // Sun disc with limb darkening.
    let cos_sun = dot(d, l);
    let disc = smoothstep(0.99996, 0.99999, cos_sun);
    if (disc > 0.0) {
        let ts = sun_transmittance(vec3<f32>(0.0, EARTH_R + max(alt, 0.001), 0.0), l);
        c += ts * frame.sun_dir.w * 40.0 * disc;
    }
    // Below the horizon: darkened ground bounce so the bottom never goes black.
    if (d.y < 0.0) {
        c = mix(c, frame.ground_color.rgb * 0.5, clamp(-d.y * 4.0, 0.0, 1.0));
    }
    return vec4<f32>(c, 1.0);
}
