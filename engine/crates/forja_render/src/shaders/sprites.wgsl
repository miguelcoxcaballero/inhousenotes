// Distant light sprites: every frame light is drawn as a camera-facing
// Gaussian dot carrying the light's luminous intensity toward the camera
// (energy-conserving: peak radiance 4 I / (pi r²) over its footprint), no
// smaller than `night_zenith.w` pixels. Far street lamps whose geometry is
// sub-pixel (and culled) therefore still read as points of the right
// brightness and colour, and bloom gives them their glow: the "carpet of
// lights" of an aerial night view. Sprites fade in beyond
// `lighting.w` metres (closer lamps show their own emissive geometry),
// are depth tested against the prepass and dimmed by the height fog.
// `common.wgsl` is prepended at load time.
// Cost: 4 vertices per visible light, a few pixels each (~0.02 ms for
// 3000 lights).

struct SpriteOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec3<f32>,
};

@vertex
fn vs_sprite(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> SpriteOut {
    var o: SpriteOut;
    o.clip = vec4<f32>(2.0, 2.0, 0.0, 1.0);
    o.uv = vec2<f32>(0.0);
    o.color = vec3<f32>(0.0);
    let l = local_lights[ii];
    if (l.params.w < 0.0 || frame.scatter.y <= 0.0) { return o; }
    let cam = frame.camera_pos.xyz;
    let to = l.pos_range.xyz - cam;
    let d = length(to);
    let f = frame.lighting.w;
    let fade = smoothstep(f, f * 2.0, d);
    if (fade <= 0.0) { return o; }
    let vdir = to / d;
    // Intensity toward the camera. Luminaires stay visible from outside a
    // spot's beam (their housing and lens glow).
    let is_spot = l.dir.w > -1.5;
    let cone = max(spot_cone(l, -vdir), select(1.0, 0.2, is_spot));
    let px_per_m = frame.proj_params.w / d;
    let r = max(l.color.w, frame.night_zenith.w / px_per_m);
    let peak = l.color.rgb * (cone * 4.0 / (PI * r * r)) * fade * frame.scatter.y * fog_transmittance(l.pos_range.xyz);
    let corner = vec2<f32>(f32(vi & 1u), f32((vi >> 1u) & 1u)) * 2.0 - vec2<f32>(1.0);
    let right = vec3<f32>(frame.view[0].x, frame.view[1].x, frame.view[2].x);
    let up = vec3<f32>(frame.view[0].y, frame.view[1].y, frame.view[2].y);
    // Pulled toward the camera so the lamp's own housing does not hide it.
    let center = l.pos_range.xyz - vdir * min(1.0 + 2.0 * r, d * 0.5);
    let p = center + (right * corner.x + up * corner.y) * (1.5 * r);
    o.clip = frame.view_proj * vec4<f32>(p, 1.0);
    o.uv = corner * 1.5;
    o.color = peak;
    return o;
}

@fragment
fn fs_sprite(in: SpriteOut) -> @location(0) vec4<f32> {
    let g = exp(-4.0 * dot(in.uv, in.uv));
    return vec4<f32>(min(in.color * (g * exposure_state.pre), vec3<f32>(30000.0)), 1.0);
}
