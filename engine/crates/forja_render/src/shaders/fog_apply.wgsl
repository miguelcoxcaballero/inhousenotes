// Fog along a view ray (appended to pbr.wgsl and sky.wgsl, which declare
// `fog_volume` and `fog_samp`): the froxel volume (fog.wgsl) up to its
// range, then the analytic height fog and lamp glow beyond it. Without a
// volume (Low preset) it is the analytic fog and glow from the camera, as
// before.

fn camera_forward() -> vec3<f32> {
    return -vec3<f32>(frame.view[0].z, frame.view[1].z, frame.view[2].z);
}

// `color` at `world_pos` (view depth `view_depth`, pixel `pix`) seen
// through the fog toward the camera; `dir` points from the camera to it.
fn apply_fog(color: vec3<f32>, world_pos: vec3<f32>, dir: vec3<f32>, pix: vec2<f32>, view_depth: f32, dist: f32) -> vec3<f32> {
    let range = frame.fog_volume.z;
    if (range <= 0.0) {
        return height_fog(color, world_pos, dir) + local_inscatter(pix, dir, 0.0, dist);
    }
    let u = sqrt(clamp(view_depth / range, 0.0, 1.0));
    let w = clamp(u - 0.5 / frame.fog_volume.w, 0.0, 1.0);
    let v = textureSampleLevel(fog_volume, fog_samp, vec3<f32>(pix * frame.screen.zw, w), 0.0);
    var c = color;
    if (view_depth > range) {
        // Beyond the volume: the closed-form fog of the rest of the ray.
        let t0 = dist * range / view_depth;
        c = height_fog_from(c, frame.camera_pos.xyz + dir * t0, world_pos, dir) + local_inscatter(pix, dir, t0, dist);
    }
    return c * v.a + v.rgb / max(exposure_state.pre, 1e-8);
}

// The sky behind the fog: the volume's full depth and the lamp glow past it
// (the sky radiance already holds the atmosphere's own scattering).
fn apply_fog_sky(color: vec3<f32>, dir: vec3<f32>, pix: vec2<f32>) -> vec3<f32> {
    let range = frame.fog_volume.z;
    if (range <= 0.0) {
        return color + local_inscatter(pix, dir, 0.0, 1e9);
    }
    let v = textureSampleLevel(fog_volume, fog_samp, vec3<f32>(pix * frame.screen.zw, 1.0), 0.0);
    let t0 = range / max(dot(dir, camera_forward()), 0.05);
    return (color + local_inscatter(pix, dir, t0, 1e9)) * v.a + v.rgb / max(exposure_state.pre, 1e-8);
}
