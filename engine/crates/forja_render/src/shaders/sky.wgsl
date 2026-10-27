// Full-screen atmosphere at the far plane (reverse-Z depth 0).
// The single-scattering sky comes from the sky-view LUT (skylut.wgsl,
// Hillaire 2020): one bilinear tap per pixel instead of a 16 x 6 step
// raymarch; `features.x = 0` (FORJA_FX=-skylut) restores the raymarch.

@group(1) @binding(0) var skyview_lut: texture_2d<f32>;
@group(1) @binding(1) var transmittance_lut: texture_2d<f32>;
@group(1) @binding(2) var fog_samp: sampler;
@group(1) @binding(3) var fog_volume: texture_3d<f32>;

fn sky_lut_radiance(d: vec3<f32>, l: vec3<f32>, alt_km: f32) -> vec3<f32> {
    if (frame.features.x < 0.5) { return sky_radiance(d, l, alt_km); }
    return textureSampleLevel(skyview_lut, fog_samp, skyview_uv(d, l, alt_km), 0.0).rgb;
}

fn sun_lut_transmittance(alt_km: f32, l: vec3<f32>) -> vec3<f32> {
    let p = vec3<f32>(0.0, EARTH_R + max(alt_km, 0.001), 0.0);
    if (frame.features.x < 0.5) { return sun_transmittance(p, l); }
    let r = length(p);
    let horizon = -sqrt(max(1.0 - (EARTH_R * EARTH_R) / (r * r), 0.0));
    if (l.y < horizon) { return vec3<f32>(0.0); }
    return textureSampleLevel(transmittance_lut, fog_samp, transmittance_uv(r, l.y), 0.0).rgb;
}
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
    if (frame.env_params.x > 0.5) {
        var c = textureSampleLevel(env_tex, env_samp, equirect_uv(d), 0.0).rgb;
        // The photo is the view from ~1.7 m in the street it was shot in:
        // its lower half is that street and its horizon is a band of trees
        // and cars. From high viewpoints switch to the analytic atmosphere
        // lit by the photo's own sun (same irradiance above the air) over
        // the synthetic ground the diffuse light uses, under the same
        // aerial perspective as the city.
        let h = frame.camera_pos.y;
        let w = smoothstep(8.0, 60.0, h);
        if (w > 0.0) {
            // Below the horizon blend the horizon sky into the fogged ground.
            let dh = normalize(vec3<f32>(d.x, max(d.y, 0.0), d.z));
            var a = sky_lut_radiance(dh, frame.sun_dir.xyz, h * 0.001) * frame.params.x;
            let dist = min(h / max(-d.y, 1e-4), 3e4);
            // Land beyond the city seen from above (towns, orchards, dry
            // fields) reflects ~10%, darker than the 17% paving of the
            // diffuse ground.
            let ground = height_fog(frame.ground_color.rgb * 0.6, frame.camera_pos.xyz + d * dist, d);
            // Distant land melts into the bright horizon haze over the
            // last few degrees instead of meeting the sky at a hard line.
            a = mix(a, ground, smoothstep(0.0, 0.1, -d.y));
            c = mix(c, a, w);
        }
        c = apply_fog_sky(c, d, in.clip.xy);
        return vec4<f32>(min(c * exposure_state.pre, vec3<f32>(60000.0)), 1.0);
    }
    let l = frame.sun_dir.xyz;
    let alt = frame.camera_pos.y * 0.001;
    var c = sky_lut_radiance(d, l, alt) * frame.sun_dir.w;
    // Sun disc with limb darkening.
    let cos_sun = dot(d, l);
    let disc = smoothstep(0.99996, 0.99999, cos_sun);
    if (disc > 0.0) {
        let ts = sun_lut_transmittance(alt, l);
        c += ts * frame.sun_dir.w * 40.0 * disc;
    }
    // Night: dark zenith, light-pollution dome, stars (zero by day).
    c += night_sky(d);
    // Below the horizon: darkened ground bounce so the bottom never goes black.
    if (d.y < 0.0) {
        c = mix(c, frame.ground_color.rgb * 0.5, clamp(-d.y * 4.0, 0.0, 1.0));
    }
    // The fog in front of the sky: the froxel volume (light shafts, lamp
    // glow) or the glow of the lamps in the haze.
    c = apply_fog_sky(c, d, in.clip.xy);
    return vec4<f32>(min(c * exposure_state.pre, vec3<f32>(60000.0)), 1.0);
}
