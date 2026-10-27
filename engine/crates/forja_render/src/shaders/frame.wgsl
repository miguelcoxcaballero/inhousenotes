const PI: f32 = 3.14159265359;
// One local light (`GpuLight` in lights.rs).
struct LocalLight {
    pos_range: vec4<f32>,   // xyz world position, w range (m)
    color: vec4<f32>,       // rgb colour x intensity, w source radius (m)
    dir: vec4<f32>,         // xyz spot direction, w cos outer angle (-2: point)
    params: vec4<f32>,      // x, y: cone = sat(cos * x + y)^2, z: shadow layer (-1), w: fog scatter (< 0: no sprite)
};

// Shared frame data. Must match `FrameUniforms` in lib.rs.

struct Frame {
    view_proj: mat4x4<f32>,
    view: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,      // w: time (s)
    sun_dir: vec4<f32>,         // toward the sun; w: sun illuminance scale
    sun_color: vec4<f32>,       // transmitted sun radiance; w: exposure
    sky_zenith: vec4<f32>,      // ambient sky colors for lighting
    sky_horizon: vec4<f32>,     // w: fog density
    ground_color: vec4<f32>,    // w: fog height falloff
    cascade_splits: vec4<f32>,  // view-space far distance per cascade
    cascade_vp: array<mat4x4<f32>, 4>,
    params: vec4<f32>,          // x: HDRI sun irradiance above the atmosphere (daylight sky from high viewpoints), y: 1 / shadow size, z: bloom strength, w: 1 = physical (daylight) camera, 0 = night camera
    screen: vec4<f32>,          // xy: size, zw: 1 / size
    sh: array<vec4<f32>, 9>,    // irradiance / pi (HDRI mode)
    env_params: vec4<f32>,      // x: 1 = HDRI image lighting, y: max mip, z: intensity, w: legacy bits (1 box env, 2 single scattering, 4 no specular AA, 8 no micro-shadowing, 16 ignore material specular factor)
    prev_view_proj: mat4x4<f32>,// previous frame, unjittered (TAA / SSR reprojection)
    proj_params: vec4<f32>,     // x: 1/P00, y: 1/P11, z: near, w: pixels per world unit at distance 1
    jitter: vec4<f32>,          // xy: sub-pixel jitter (NDC), z: frame index, w: 1 = history valid
    cascade_texel: vec4<f32>,   // world size of one shadow texel per cascade
    cascade_range: vec4<f32>,   // world depth range of each cascade (light space)
    fx: vec4<f32>,              // x: SSAO strength, y: contact shadows, z: SSR, w: sun angular diameter (rad)
    ao_params: vec4<f32>,       // x: AO radius (m), y: specular occlusion strength, z: SSR max distance (m), w: unused
    lighting: vec4<f32>,        // x: local light count, y: 1 = auto exposure, z: fog scatter strength, w: sprite fade distance (m)
    cluster: vec4<f32>,         // x, y: tiles, z: depth slices, w: tile size (px)
    cluster_z: vec4<f32>,       // x: first slice depth, y: last, z: slices / ln(far / near), w: fog scatter distance (m)
    wet: vec4<f32>,             // x: wetness, y: puddles, z: 1 / puddle size (m), w: emissive scale
    fill: vec4<f32>,            // rgb: ambient fill radiance (city bounce light), w: stars
    night_zenith: vec4<f32>,    // rgb: night sky radiance, w: light sprite minimum size (px)
    sky_glow: vec4<f32>,        // rgb: light-pollution glow at the horizon, w: its height (sin elevation)
    scatter: vec4<f32>,         // x: fog phase anisotropy g, y: light sprite strength, z: shadowed spot count, w: 1 = wetness on
    spot_vp: array<mat4x4<f32>, 4>,
};

@group(0) @binding(0) var<uniform> frame: Frame;
