//! # forja_render
//!
//! Modern forward+ renderer on wgpu. On Windows it runs on Direct3D 12
//! (Vulkan as fallback). Frame graph:
//!
//! 1. **Shadows**: 4 stable cascades (texel-snapped bounding spheres) into a
//!    depth array, culled per cascade; alpha-masked casters are alpha tested.
//! 2. **Depth/normal prepass**: reverse-Z depth plus geometric normals for
//!    every opaque surface (the main pass then shades each pixel once).
//! 3. **GTAO**: ground-truth ambient occlusion from depth + normals with a
//!    4x4 bilateral denoise.
//! 4. **Main**: HDR forward pass against the prepass depth (read-only); PBR
//!    (GGX, height-correlated Smith, Schlick) with normal/metal-rough/
//!    occlusion/emissive maps, PCSS sun shadows with screen-space contact
//!    shadows, HDRI or analytic sky ambient with GTAO multi-bounce and
//!    specular occlusion, clustered point and spot lights, wet surfaces,
//!    aerial perspective and the fog glow of local lights. Static,
//!    double-sided, GPU-skinned and blended variants.
//! 5. **Sky**: HDRI or per-pixel Rayleigh + Mie single scattering, plus the
//!    night sky (light-pollution dome, stars); then the **light sprites**
//!    that keep distant lamps visible as points.
//! 6. **TAA**: Halton-jittered frames resolved with reprojected, variance
//!    clipped history; still cameras converge to a supersampled image.
//! 7. **Eye adaptation** (night camera, optional): GPU luminance histogram
//!    and night-aware exposure; lighting is pre-exposed so night scenes keep
//!    f16 precision.
//! 8. **Post**: bloom chain, then per [`CameraModel`]: by day energy-conserving
//!    lens glare, the physical camera (EV100, metered) and a hue-preserving
//!    camera tone curve; at night AgX with a scotopic shift and the
//!    cinematic night grade. Vignette, CAS sharpening (FXAA when TAA is
//!    off).
//!
//! Daylight lighting is photometric ([`exposure`]): the HDRI sun is ~105
//! klx and the sky keeps the photo's own ratio to it. Cost of the daylight
//! camera: the meter is one extra fragment over the 1/64-resolution bloom
//! mip (~220 texel reads at 720p) per frame, the tone curve is cheaper than
//! AgX (two `pow` instead of three `log2`, a polynomial and two matrices),
//! and the aerial perspective adds a few ALU ops (one HG phase, three
//! `exp`) to the forward pass. No per-frame CPU work beyond a handful of
//! scalars.
//!
//! Between the prepass and the main pass a compute pass bins the frame's
//! lights into a froxel grid ([`lights`]). See [`Environment::night`],
//! [`Environment::dusk`] and [`Environment::rainy_night`] for the night
//! presets and [`night`] for their units.
//!
//! The renderer knows nothing about worlds or gameplay: callers upload
//! [`SceneData`] once ([`Renderer::upload_scene`]) and submit a [`FrameDesc`]
//! with instances every frame.

pub mod atmosphere;
pub mod brdf;
pub mod gpu;
pub mod fog;
pub mod motion;
pub mod capture;
pub mod camera;
pub mod env;
pub mod profiler;
pub mod quality;
mod static_scene;
mod mesh_pool;
mod texcomp;
pub mod exposure;
pub mod tonemap;
pub mod look;
pub mod lights;
pub mod night;

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forja_assets::{AlphaMode, MaterialData, SceneData, ShadowRole, TextureData};
use forja_math::{Aabb, Frustum, Mat4, Vec3, Vec4};
use wgpu::util::DeviceExt;

pub use camera::Camera;
pub use quality::{DisplayMode, DynamicResolution, FrameBound, GraphicsConfig, Preset, RENDER_SCALES};
pub use look::{DepthOfField, Grade, Look};
pub use lights::{Light, LightColor, Spot, candela, kelvin};
pub use night::{EyeAdaptation, NightSky};
pub use wgpu;

use static_scene::{Batch, DrawKey, StaticItem, StaticScene};

const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const SHADOW_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const NORMAL_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// GTAO: bent normal (rgb, world space, 0.5 + 0.5 n) and visibility (a).
const AO_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const BLOOM_MIPS: usize = 6;
/// Most lights per frame (clustered; each cluster keeps its closest
/// [`lights::CLUSTER_CAP`]).
pub const MAX_LIGHTS: usize = lights::MAX_LOCAL_LIGHTS;
const SPOT_SHADOW_SIZE: u32 = 1024;
/// Eye adaptation histogram: log2 luminance (units) from 2^-26 (~1.5e-4
/// cd/m²) over 34 stops.
const EXPOSURE_LOG_MIN: f32 = -26.0;
const EXPOSURE_LOG_RANGE: f32 = 34.0;
/// Scales the Krawczyk key so a sunlit street (~5000 cd/m²) gets the
/// daytime manual exposure (0.55).
const EXPOSURE_KEY_SCALE: f32 = 0.4;

const FRAME: &str = include_str!("shaders/frame.wgsl");
const COMMON: &str = include_str!("shaders/common.wgsl");

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct FrameUniforms {
    view_proj: [[f32; 4]; 4],
    view: [[f32; 4]; 4],
    inv_view_proj: [[f32; 4]; 4],
    camera_pos: [f32; 4],
    sun_dir: [f32; 4],
    sun_color: [f32; 4],
    sky_zenith: [f32; 4],
    sky_horizon: [f32; 4],
    ground_color: [f32; 4],
    cascade_splits: [f32; 4],
    cascade_vp: [[[f32; 4]; 4]; 4],
    params: [f32; 4],
    screen: [f32; 4],
    sh: [[f32; 4]; 9],
    env_params: [f32; 4],
    prev_view_proj: [[f32; 4]; 4],
    proj_params: [f32; 4],
    jitter: [f32; 4],
    cascade_texel: [f32; 4],
    cascade_range: [f32; 4],
    fx: [f32; 4],
    ao_params: [f32; 4],
    lighting: [f32; 4],
    cluster: [f32; 4],
    cluster_z: [f32; 4],
    wet: [f32; 4],
    fill: [f32; 4],
    night_zenith: [f32; 4],
    sky_glow: [f32; 4],
    scatter: [f32; 4],
    features: [f32; 4],
    fog_volume: [f32; 4],
    spot_vp: [[[f32; 4]; 4]; lights::MAX_SPOT_SHADOWS],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ExposureUniforms {
    a: [f32; 4],
    b: [f32; 4],
    c: [f32; 4],
    d: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct MaterialUniforms {
    base_color: [f32; 4],
    emissive: [f32; 4],
    params: [f32; 4],
    layers: [f32; 4],
    /// x: translucency, y: back faces above this v are discarded, z: foliage on.
    foliage: [f32; 4],
    /// x: specular factor.
    extra: [f32; 4],
}

/// Cost knobs read by the shaders (`Quality` in pbr.wgsl / gtao.wgsl).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct QualityUniforms {
    /// x: PCSS cascades, y: SSR steps, z: contact-shadow steps, w: AO scale.
    params: [f32; 4],
    /// x: 1 = GTAO bent normals (cone specular occlusion), y: 1 = sky-view
    /// LUT (0: per-pixel raymarch), z: 1 = FSR 1 EASU upscale, w: unused.
    extra: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Default, Pod, Zeroable)]
struct PostUniforms {
    texel: [f32; 4],
    params: [f32; 4],
    curve: [f32; 4],
    look: [f32; 4],
    /// Camera white balance gains (linear RGB).
    white: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    pos: [f32; 3],
    normal: [f32; 3],
    uv: [f32; 2],
    tangent: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SkinVertex {
    joints: [u16; 4],
    weights: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct InstanceRaw {
    model: [[f32; 4]; 4],
    tint: [f32; 4],
    extra: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct LineVertex {
    pub pos: [f32; 3],
    pub color: [f32; 4],
}

/// Handle to an uploaded scene (a model with its nodes, meshes and materials).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ModelHandle(pub u32);

/// Legacy point light (arbitrary intensity, `1 / (d² + 1)` falloff). New
/// code should use [`Light`] in [`FrameDesc::local_lights`]; both go
/// through the same clustered path.
#[derive(Clone, Copy, Debug)]
pub struct PointLight {
    pub position: Vec3,
    pub radius: f32,
    pub color: Vec3,
    pub intensity: f32,
}

/// Far-distance group of a static instance (see
/// [`Renderer::set_static_instances_lod`]): group 0 is always drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StaticLod {
    pub group: u32,
    /// The group's stand-in (drawn far) rather than its detail (near).
    pub hlod: bool,
}

/// One model placed in the world.
#[derive(Clone, Debug)]
pub struct DrawInstance {
    pub model: ModelHandle,
    pub transform: Mat4,
    /// Per-node model-space transforms (rigid node animation). Indexed like
    /// the scene's nodes; `None` uses the rest pose.
    pub nodes: Option<Arc<Vec<Mat4>>>,
    /// Skinning matrices (model space * inverse bind) for skinned meshes.
    pub joints: Option<Arc<Vec<Mat4>>>,
    pub tint: Vec4,
    pub cast_shadows: bool,
    /// Stable id of a moving object (0: none). The renderer keeps each id's
    /// transform for one frame so the prepass can write object motion
    /// (motion blur, TAA reprojection); without one only the camera moves.
    pub motion_id: u64,
}

impl DrawInstance {
    pub fn new(model: ModelHandle, transform: Mat4) -> Self {
        Self { model, transform, nodes: None, joints: None, tint: Vec4::ONE, cast_shadows: true, motion_id: 0 }
    }
}

/// Camera model of a frame: exposure, glare and tone curve (the cinematic
/// [`Look`] stages run after it in either case).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CameraModel {
    /// Physical camera: EV100 (metered or manual), energy-conserving lens
    /// glare, a hue-preserving camera tone curve, white balance and aerial
    /// perspective.
    #[default]
    Physical,
    /// The night renderer's look: linear exposure or eye adaptation, AgX
    /// with scotopic shift and the cinematic night grade, the night bloom
    /// chain and height fog ([`Environment::night`], [`Environment::dusk`]).
    Night,
}

#[derive(Clone, Debug)]
pub struct Environment {
    /// Direction toward the sun.
    pub sun_dir: Vec3,
    /// Analytic sky: sun illuminance at normal incidence in renderer units
    /// ([`exposure::LUX_PER_UNIT`] lux each; 14 is ~105 klx).
    pub sun_illuminance: f32,
    /// Camera model: the daylight physical camera or the night camera the
    /// night presets were tuned with.
    pub camera: CameraModel,
    /// Night camera: linear exposure (manual, or the start of eye
    /// adaptation). The daylight camera uses `ev100` instead.
    pub exposure: f32,
    /// Physical camera: exposure value at ISO 100 ("sunny
    /// 16" is 15).
    /// `None` exposes automatically: an incident-light reading of the sun
    /// and sky (which follows the time of day), refined per view by the
    /// GPU meter within `auto_exposure_range`.
    pub ev100: Option<f32>,
    /// Stops added to the camera exposure (negative is darker). The default
    /// +1/3 stop is how photographers expose a sunny street (sunlit cream
    /// plaster near white, mid grey a little over sRGB 128) rather than the
    /// strict "sunny 16" reading.
    pub exposure_compensation: f32,
    /// Auto exposure (`ev100: None`): a center-weighted average meter on the
    /// GPU (like a camera's reflected-light meter) may open up by `[0]`
    /// stops or close down by `[1]` stops from the incident reading of the
    /// sun and sky, so shaded views read like a photographer exposed them
    /// without losing the sunlit highlights. `[0, 0]` meters only the
    /// incident light.
    pub auto_exposure_range: [f32; 2],
    /// Time constant (s) of the camera adapting to a new exposure.
    pub exposure_adaptation: f32,
    /// Aerosol extinction at ground level (1/m). Visibility is 3.912 / it:
    /// 1.3e-4 is the ~30 km of a clear, dry summer day.
    pub fog_density: f32,
    /// Inverse scale height of the aerosol (1/m).
    pub fog_falloff: f32,
    /// Low ground haze (extinction at ground level, 1/m; scale height
    /// 80 m) on top of `fog_density`: the morning and evening haze that
    /// shows light shafts in the volumetric fog. `None`: automatic, zero
    /// with the sun above ~20 degrees and at night, rising to 6e-4 (a 6%
    /// veil per 100 m) as the sun reaches the horizon.
    pub haze: Option<f32>,
    /// Share of the light spread by the lens' veiling glare (0.02-0.04 for
    /// a good lens); the blur is energy conserving.
    pub bloom: f32,
    pub vignette: f32,
    /// Camera response (display transform).
    pub tone: tonemap::ToneCurve,
    /// Auto white balance: how far the camera neutralises the metered sun
    /// and sky colour (0 renders for a D65 white, 1 fully neutral). Partial
    /// adaptation keeps the warmth of sunlight and the blue of open shade.
    pub white_balance: f32,
    pub shadow_distance: f32,
    /// Light with the loaded HDRI ([`Renderer::set_environment`]) instead of
    /// the analytic atmosphere. Its sun drives the shadows.
    pub use_ibl: bool,
    /// World radius of the screen-space ambient occlusion (m). Screen space
    /// misses what is hidden or off screen, so like XeGTAO (radius
    /// multiplier 1.457) this is larger than the true occlusion distance.
    pub ao_radius: f32,
    /// Exponent applied to the ambient occlusion (1 = raw GTAO visibility;
    /// XeGTAO's default, matched to a ray-traced reference, is 2.2).
    pub ao_strength: f32,
    /// 0..1: how much ambient occlusion also occludes reflections.
    pub specular_occlusion: f32,
    /// Angular diameter of the sun (radians): sets the shadow penumbra
    /// (0.0093 is the real sun; a little more mimics forward-scattering haze).
    pub sun_angular_diameter: f32,
    /// Night camera: eye adaptation; `None` uses the fixed `exposure`. Ignored
    /// by the daylight camera, which meters itself.
    pub auto_exposure: Option<night::EyeAdaptation>,
    /// Night sky added to the analytic atmosphere (zero by day).
    pub night_sky: night::NightSky,
    /// Ambient radiance added everywhere (light bounced around the street by
    /// the city's lamps), occluded by AO.
    pub ambient_fill: Vec3,
    /// Rain wetness 0..1 (darker porous materials, glossier everything).
    pub wetness: f32,
    /// Puddle coverage of flat ground 0..1 (needs `wetness`).
    pub puddles: f32,
    /// Size of the puddle noise (m).
    pub puddle_size: f32,
    /// Multiplier of every material's emission.
    pub emissive_scale: f32,
    /// Glow of local lights in the height fog (0 = off, 1 = physical).
    pub light_scatter: f32,
    /// Henyey-Greenstein anisotropy of that glow (haze scatters forward).
    pub scatter_anisotropy: f32,
    /// Lights farther than this (m) do not glow in the fog (their sprite
    /// and bloom remain).
    pub scatter_distance: f32,
    /// Distant light sprites (0 = off, 1 = physical intensity).
    pub light_sprites: f32,
    /// Sprites fade in from this distance (m) to twice it.
    pub sprite_fade_distance: f32,
    /// Smallest sprite (pixels).
    pub sprite_min_pixels: f32,
    /// Weight of each wider bloom mip (1 = the daytime chain; lower keeps
    /// halos tight around lamps instead of a veil).
    pub bloom_radius: f32,
    /// Largest luminance one pixel feeds the bloom (exposed units).
    pub bloom_limit: f32,
    /// Night-vision shift of dim areas toward desaturated blue, 0..1.
    pub scotopic: f32,
    /// Cinematic night grade after the tonemapper, 0..1: filmic toe with
    /// true blacks, S-curve contrast, teal shadows and warm highlights
    /// (0 = the daytime look, unchanged).
    pub night_grade: f32,
}

impl Default for Environment {
    fn default() -> Self {
        Self {
            sun_dir: Vec3::new(0.35, 0.6, 0.25).normalize(),
            sun_illuminance: 14.0,
            ev100: None,
            camera: CameraModel::Physical,
            exposure: 0.55,
            exposure_compensation: 0.33,
            auto_exposure_range: [1.5, 0.5],
            exposure_adaptation: 0.4,
            fog_density: 1.3e-4,
            fog_falloff: 1.0e-3,
            haze: None,
            bloom: 0.02,
            vignette: 0.3,
            tone: tonemap::ToneCurve::default(),
            white_balance: 0.6,
            shadow_distance: 220.0,
            use_ibl: true,
            ao_radius: 1.5,
            ao_strength: 2.0,
            specular_occlusion: 1.0,
            sun_angular_diameter: 0.012,
            auto_exposure: None,
            night_sky: night::NightSky::default(),
            ambient_fill: Vec3::ZERO,
            wetness: 0.0,
            puddles: 0.0,
            puddle_size: 3.0,
            emissive_scale: 1.0,
            light_scatter: 0.0,
            scatter_anisotropy: 0.6,
            scatter_distance: 150.0,
            light_sprites: 0.0,
            sprite_fade_distance: 50.0,
            sprite_min_pixels: 1.5,
            bloom_radius: 1.0,
            bloom_limit: 4.0,
            scotopic: 0.0,
            night_grade: 0.0,
        }
    }
}

/// Quality switches of the renderer (`Renderer::settings`). `FORJA_FX`
/// overrides them for comparisons, e.g. `FORJA_FX=-taa,-ssao`,
/// `FORJA_CAPTURE_FRAMES` the number of frames accumulated per screenshot and
/// `FORJA_DEBUG` shows one term instead of the lit image (`ao`, `shadow`,
/// `ssr`).
#[derive(Clone, Debug)]
pub struct RenderSettings {
    /// Temporal anti-aliasing (with Halton jitter). Off: FXAA.
    pub taa: bool,
    /// Ground-truth ambient occlusion.
    pub ssao: bool,
    /// Screen-space contact shadows for the sun.
    pub contact_shadows: bool,
    /// Screen-space reflections (needs TAA history).
    pub ssr: bool,
    /// Jittered frames accumulated by [`Renderer::render_offscreen`] and
    /// [`Renderer::render_to_image`] (TAA converges to a supersampled image).
    pub capture_frames: u32,
    /// Post-TAA sharpening, 0..1.
    pub sharpen: f32,
    /// 0: lit image; 1: ambient occlusion; 2: sun shadow (map x contact);
    /// 3: screen-space reflection hit confidence; 4: specular radiance
    /// only (sun + environment + SSR); 5: shading roughness after specular
    /// anti-aliasing.
    pub debug_view: u32,
    /// GTAO at half resolution with a depth-aware upsample.
    pub ao_half_res: bool,
    /// Cascades below this index use PCSS; the others a fixed 4-tap PCF.
    pub pcss_cascades: u32,
    /// Cascades 2 and 3 re-render every N frames (1: every frame).
    pub far_cascade_interval: u32,
    pub ssr_steps: u32,
    pub contact_steps: u32,
    /// Local lights per frame after frustum culling (closest kept; the
    /// clusters then bound the per-pixel cost).
    pub max_lights: u32,
    /// Spot lights with a shadow map (at most `lights::MAX_SPOT_SHADOWS`).
    pub spot_shadows: u32,
    pub bloom: bool,
    /// Multiplies the scene's shadow distance.
    pub shadow_distance_scale: f32,
    /// Multiplies the LOD triangle budget.
    pub lod_detail: f32,
    /// Sun shadow casters smaller than this many cascade texels (radius)
    /// are skipped.
    pub shadow_caster_texels: f32,
    /// First sun cascade drawn with shadow proxies instead of
    /// [`forja_assets::ShadowRole::Near`] detail (4: never).
    pub shadow_proxy_cascade: u32,
    /// Textures are uploaded from the first mip not larger than this.
    pub max_texture_size: u32,
    /// Camera stage of [`FrameDesc::look`] (`FORJA_FX=-look` skips it).
    pub look: bool,
    /// Upload colour and data maps block-compressed (BC1/BC3, cached on
    /// disk) when the adapter supports BC (`FORJA_FX=-bc` keeps RGBA8).
    pub texture_compression: bool,
    /// Temporally stable shadows: caster LOD from the cascade texel only,
    /// thin detail ([`ShadowRole::Fine`]) in the first cascade only
    /// (`FORJA_FX=-stableshadow` restores the old behaviour, for A/B).
    pub stable_shadows: bool,
    /// Distance (m) beyond which a city tile's detail is replaced by its
    /// HLOD stand-in (0: never).
    pub hlod_distance: f32,
    /// Depth of field of [`FrameDesc::look`] (`FORJA_FX=-dof`).
    pub dof: bool,
    /// Multiple-scattering energy compensation and the DFG LUT (`-ms`
    /// falls back to the single-scattering analytic fit, for comparisons).
    pub multiscatter: bool,
    /// Geometric and normal-map specular anti-aliasing (`-specaa`).
    pub specular_aa: bool,
    /// Micro-shadowing of direct sun light by the material AO (`-micro`).
    pub micro_shadow: bool,
    /// Material specular factor (`-porous` renders every dielectric with
    /// F0 0.04, for comparisons).
    pub specular_factor: bool,
    /// `FORJA_DOF=<focus m>,<f-stop>` forces a depth of field on every
    /// frame (photo-mode previews of the examples).
    pub dof_override: Option<DepthOfField>,
    /// Camera + object motion blur (`-motion`) and its gather samples.
    pub motion_blur: bool,
    pub motion_blur_samples: u32,
    /// Froxel volumetric fog quality 0-3 (`-fog`: 0, analytic fog).
    pub volumetric_fog: u32,
    /// GTAO bent normals for cone specular occlusion (`-bent`).
    pub bent_normals: bool,
    /// Sky-view LUT instead of the per-pixel sky raymarch (`-skylut`).
    pub sky_lut: bool,
    /// FSR 1 EASU upscaling when the target is larger than the internal
    /// resolution (`-fsr`: bilinear + CAS).
    pub fsr: bool,
    /// Merge consecutive draws with the same state into multi-draws
    /// (`FORJA_NO_MULTI_DRAW=1` turns it off, for comparisons).
    pub multi_draw: bool,
}

impl Default for RenderSettings {
    /// The High preset.
    fn default() -> Self {
        Self {
            taa: true,
            ssao: true,
            contact_shadows: true,
            ssr: true,
            capture_frames: 8,
            sharpen: 0.5,
            debug_view: 0,
            ao_half_res: true,
            pcss_cascades: 2,
            far_cascade_interval: 2,
            ssr_steps: 32,
            contact_steps: 10,
            max_lights: MAX_LIGHTS as u32,
            spot_shadows: lights::MAX_SPOT_SHADOWS as u32,
            bloom: true,
            shadow_distance_scale: 1.0,
            lod_detail: 1.0,
            shadow_caster_texels: 2.5,
            shadow_proxy_cascade: 3,
            max_texture_size: 2048,
            look: true,
            texture_compression: true,
            stable_shadows: true,
            hlod_distance: 250.0,
            dof: true,
            multiscatter: true,
            specular_aa: true,
            micro_shadow: true,
            specular_factor: true,
            dof_override: None,
            motion_blur: true,
            motion_blur_samples: 10,
            volumetric_fog: 2,
            bent_normals: true,
            sky_lut: true,
            fsr: true,
            multi_draw: true,
        }
    }
}

impl RenderSettings {
    /// Shader switches for before/after comparisons (`env_params.w`):
    /// 1 = legacy box-filtered env lod, 2 = single scattering, 4 = no
    /// specular anti-aliasing, 8 = no micro-shadowing, 16 = F0 0.04 for
    /// every dielectric.
    fn legacy_bits(&self, box_env: bool) -> f32 {
        (box_env as u32 | (!self.multiscatter as u32) << 1 | (!self.specular_aa as u32) << 2 | (!self.micro_shadow as u32) << 3 | (!self.specular_factor as u32) << 4) as f32
    }

    /// Defaults with the `FORJA_FX` / `FORJA_CAPTURE_FRAMES` overrides applied.
    pub fn from_env() -> Self {
        let mut s = Self::default();
        if let Ok(fx) = std::env::var("FORJA_FX") {
            s.apply_fx(&fx);
        }
        if std::env::var_os("FORJA_NO_MULTI_DRAW").is_some() {
            s.multi_draw = false;
        }
        if let Some(n) = std::env::var("FORJA_CAPTURE_FRAMES").ok().and_then(|v| v.parse().ok()) {
            s.capture_frames = n;
        }
        if let Some(n) = std::env::var("FORJA_MAX_TEXTURE").ok().and_then(|v| v.parse().ok()) {
            s.max_texture_size = n;
        }
        if let Ok(v) = std::env::var("FORJA_DOF") {
            let n: Vec<f32> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
            if let [focus, f_stop] = n[..] {
                s.dof_override = Some(DepthOfField::new(focus, f_stop));
            } else {
                log::warn!("FORJA_DOF: expected <focus m>,<f-stop>, got {v}");
            }
        }
        if let Ok(v) = std::env::var("FORJA_DEBUG") {
            s.debug_view = match v.trim() {
                "ao" => 1,
                "shadow" => 2,
                "ssr" => 3,
                "spec" => 4,
                "rough" => 5,
                other => {
                    log::warn!("FORJA_DEBUG: unknown view {other}");
                    0
                }
            };
        }
        s
    }

    /// Copies the cost knobs of a graphics configuration (keeps the debug
    /// view, capture frames and sharpening).
    pub fn apply_graphics(&mut self, g: &GraphicsConfig) {
        self.taa = g.taa;
        self.ssao = g.ssao;
        self.ao_half_res = g.ssao_half_res;
        self.contact_shadows = g.contact_shadows;
        self.contact_steps = if g.preset == Preset::Low { 6 } else if g.preset == Preset::Medium { 8 } else { 10 };
        self.ssr = g.ssr;
        self.ssr_steps = g.ssr_steps;
        self.pcss_cascades = g.pcss_cascades;
        self.far_cascade_interval = g.far_cascade_interval;
        self.max_lights = g.max_lights;
        self.spot_shadows = g.spot_shadows;
        self.bloom = g.bloom;
        self.shadow_distance_scale = g.shadow_distance_scale;
        self.lod_detail = g.lod_detail;
        self.shadow_caster_texels = g.shadow_caster_texels;
        self.shadow_proxy_cascade = g.shadow_proxy_cascade;
        self.hlod_distance = g.hlod_distance;
        self.max_texture_size = g.max_texture_size;
        self.dof = g.dof;
        self.motion_blur = g.motion_blur;
        self.motion_blur_samples = g.motion_blur_samples;
        self.volumetric_fog = g.volumetric_fog;
        self.bent_normals = g.bent_normals;
        self.fsr = g.fsr;
    }

    /// Applies a `FORJA_FX` list: `name` or `+name` enables, `-name`
    /// disables (`taa`, `ssao`/`ao`, `contact`, `ssr`, `look`, `dof`, `ms`,
    /// `specaa`, `micro`, `porous`, `ggx`).
    pub fn apply_fx(&mut self, fx: &str) {
        for item in fx.split(',').map(str::trim).filter(|i| !i.is_empty()) {
            let (on, name) = match item.strip_prefix('-') {
                Some(n) => (false, n),
                None => (true, item.trim_start_matches('+')),
            };
            match name {
                "taa" => self.taa = on,
                "ssao" | "ao" => self.ssao = on,
                "contact" => self.contact_shadows = on,
                "ssr" => self.ssr = on,
                "halfao" => self.ao_half_res = on,
                "pcss" => self.pcss_cascades = if on { 4 } else { 0 },
                "shadowcache" => self.far_cascade_interval = if on { 2 } else { 1 },
                "bloom" => self.bloom = on,
                "look" | "cinematic" => self.look = on,
                "bc" => self.texture_compression = on,
                "stableshadow" => self.stable_shadows = on,
                "hlod" => self.hlod_distance = if on { self.hlod_distance.max(250.0) } else { 0.0 },
                "dof" => self.dof = on,
                "ms" => self.multiscatter = on,
                "specaa" => self.specular_aa = on,
                "micro" => self.micro_shadow = on,
                "porous" => self.specular_factor = on,
                "motion" | "mb" => self.motion_blur = on,
                "fog" | "volfog" => self.volumetric_fog = if on { self.volumetric_fog.max(2) } else { 0 },
                "bent" => self.bent_normals = on,
                "skylut" => self.sky_lut = on,
                "fsr" => self.fsr = on,
                // Read by `env::EnvMap::from_hdr` (box-filtered mips).
                "ggx" => {}
                _ => log::warn!("FORJA_FX: unknown effect {name}"),
            }
        }
    }
}

impl Environment {
    /// Manual or incident-metered linear exposure, before the GPU meter's
    /// correction. `meter_illuminance` is what the incident meter's dome
    /// receives in renderer units ([`exposure::dome_illuminance`]).
    pub fn exposure(&self, meter_illuminance: f32) -> f32 {
        let ev = self.ev100.unwrap_or_else(|| exposure::incident_ev100(meter_illuminance));
        exposure::exposure_from_ev100(ev - self.exposure_compensation)
    }

    /// Manual camera from a plain linear exposure multiplier (the units of
    /// the old fixed camera, whose daylight value was 0.55). Night scenes
    /// tuned against such a value keep it: the incident meter would expose
    /// a moonlit street like daylight.
    pub fn set_linear_exposure(&mut self, exposure: f32) {
        self.exposure = exposure;
        self.ev100 = Some((exposure::exposure_from_ev100(0.0) / exposure.max(1e-6)).log2());
        self.exposure_compensation = 0.0;
    }
}

#[derive(Clone, Debug, Default)]
pub struct FrameDesc {
    pub camera: Camera,
    pub env: Environment,
    pub instances: Vec<DrawInstance>,
    /// Legacy point lights (see [`PointLight`]).
    pub lights: Vec<PointLight>,
    /// Point and spot lights, thousands per frame (clustered).
    pub local_lights: Vec<Light>,
    pub lines: Vec<LineVertex>,
    pub time: f32,
    /// Camera and film stage after tonemapping (grade, grain, lens
    /// effects, depth of field); defaults to [`Look::cinematic`].
    pub look: Look,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct RenderStats {
    pub draw_calls: u32,
    pub instances_drawn: u32,
    pub instances_culled: u32,
    pub triangles: u64,
    pub shadow_draws: u32,
    /// Draw calls actually recorded (shadows, prepass, main): consecutive
    /// draws with the same state are merged into one multi-draw.
    pub api_draws: u32,
    /// Local lights after frustum culling (uploaded to the clusters).
    pub lights: u32,
    pub spot_shadows: u32,
    pub cpu_ms: f32,
    /// Instances and triangles drawn into the shadow cascades this frame.
    pub shadow_instances: u32,
    pub shadow_triangles: u64,
    /// Cascades re-rendered this frame (bit mask).
    pub cascades_updated: u8,
    /// Static instances / grid cells uploaded with `set_static_instances`.
    pub static_instances: u32,
    pub static_cells: u32,
    /// City tiles drawn as their HLOD stand-in this frame.
    pub hlod_tiles: u32,
    /// Internal render resolution.
    pub width: u32,
    pub height: u32,
    /// Estimated GPU memory (textures, meshes, render targets), MB.
    pub vram_mb: f32,
    /// EV100 of the manual setting or the incident meter, with the
    /// compensation applied (the GPU meter may still shift it within
    /// `Environment::auto_exposure_range`).
    pub ev100: f32,
}

struct GpuMesh {
    /// Vertices in the shared vertex pool (`None`: released).
    vertex: Option<mesh_pool::Alloc>,
    skin: Option<wgpu::Buffer>,
    /// All index levels in the shared index pool, LOD 0 first.
    index: Option<mesh_pool::Alloc>,
    index_count: u32,
    /// Simplified levels (LOD 1..): `(first index in the page, count)`.
    lods: Vec<(u32, u32)>,
    material: u32,
    aabb: Aabb,
    /// `MeshData::lod_sync` groups: triangles per level of the whole group
    /// (LOD selection) and the group's bounds (LOD distance and culling).
    lod_tris: Option<Arc<[u32]>>,
    lod_bounds: Aabb,
    /// Scene name (profiler reports).
    name: Arc<str>,
    shadow: ShadowRole,
}

impl GpuMesh {
    /// First index (in its pool page) and index count of level `lod`.
    fn indices(&self, lod: u32) -> (u32, u32) {
        let first = self.index.map_or(0, |a| a.first);
        match lod {
            0 => (first, self.index_count),
            l => self.lods.get(l as usize - 1).copied().unwrap_or((first, self.index_count)),
        }
    }

    /// Draw range of level `lod`: index page, first index, index count,
    /// base vertex and vertex page (`None` when released or empty).
    fn draw_range(&self, lod: u32) -> Option<DrawRange> {
        let (v, i) = (self.vertex?, self.index?);
        let (first, count) = self.indices(lod);
        (count > 0).then_some(DrawRange { vpage: v.page, ipage: i.page, first, count, base_vertex: v.first as i32 })
    }
}

/// Batches per main-pass chunk recorded on one worker (see
/// [`Renderer::render`]).
const MAIN_CHUNK_BATCHES: usize = 256;

/// `FORJA_MAIN_CHUNKS=n`: record the main pass in exactly `n` chunks.
fn main_chunks_override() -> Option<usize> {
    static N: std::sync::OnceLock<Option<usize>> = std::sync::OnceLock::new();
    *N.get_or_init(|| std::env::var("FORJA_MAIN_CHUNKS").ok().and_then(|v| v.parse().ok()))
}

/// One command buffer of the frame recorded on a worker thread (see
/// [`Renderer::render`]): a shadow view, the prepass or the main pass, or
/// only the `finish` of an encoder recorded on the main thread.
enum PassJob<'a> {
    Shadow { c: usize, base: usize, tw: Option<wgpu::RenderPassTimestampWrites<'a>> },
    Prepass { base: usize, tw: Option<wgpu::RenderPassTimestampWrites<'a>> },
    /// Batches `lo..hi` of the main pass (`first` clears the target,
    /// `last` also draws the lines).
    Main { base: usize, tw: Option<wgpu::RenderPassTimestampWrites<'a>>, lo: usize, hi: usize, first: bool, last: bool },
    Finish(wgpu::CommandEncoder),
}

/// A finished [`PassJob`]: its commands, multi-draw arguments (written at
/// `base` of the indirect buffer), counters and CPU time
/// (`kind`: 0 shadows, 1 prepass, 2 main, 3 only `finish`).
struct PassOut {
    cmd: wgpu::CommandBuffer,
    args: Vec<IndirectArgs>,
    base: usize,
    stats: RenderStats,
    kind: usize,
    ms: f32,
}

/// What the parallel passes read (all shared, nothing mutated).
struct PassCtx<'a> {
    device: &'a wgpu::Device,
    pipelines: &'a Pipelines,
    materials: &'a [GpuMaterial],
    meshes: &'a [GpuMesh],
    vpool: &'a mesh_pool::MeshPool,
    ipool: &'a mesh_pool::MeshPool,
    indirect: Option<&'a wgpu::Buffer>,
    instances: &'a wgpu::Buffer,
    cascade_binds: &'a [wgpu::BindGroup],
    shadow_layers: &'a [wgpu::TextureView],
    spot_shadow_layers: &'a [wgpu::TextureView],
    joint_bind: &'a wgpu::BindGroup,
    frame_bind: &'a wgpu::BindGroup,
    sky_bind: &'a wgpu::BindGroup,
    line_buf: &'a wgpu::Buffer,
    targets: &'a Targets,
    main_batches: &'a [Batch],
    shadow_batches: &'a [Vec<Batch>],
    /// Line vertices to draw after the main batches.
    lines: u32,
    /// Light sprites to draw after the sky (0: none).
    sprites: u32,
}

impl PassCtx<'_> {
    fn run(&self, job: PassJob<'_>) -> PassOut {
        let t0 = std::time::Instant::now();
        let mut stats = RenderStats::default();
        let mut args: Vec<IndirectArgs> = Vec::new();
        let (enc, base, kind) = match job {
            PassJob::Finish(enc) => (enc, 0, 3),
            PassJob::Shadow { c, base, tw } => {
                let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("shadow") });
                self.shadow(&mut enc, c, base, tw, &mut args, &mut stats);
                (enc, base, 0)
            }
            PassJob::Prepass { base, tw } => {
                let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("prepass") });
                self.prepass(&mut enc, base, tw, &mut args, &mut stats);
                (enc, base, 1)
            }
            PassJob::Main { base, tw, lo, hi, first, last } => {
                let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("main") });
                self.main(&mut enc, base, tw, (lo, hi, first, last), &mut args, &mut stats);
                (enc, base, 2)
            }
        };
        let cmd = enc.finish();
        PassOut { cmd, args, base, stats, kind, ms: t0.elapsed().as_secs_f32() * 1000.0 }
    }

    /// Sun cascade `c` (< 4) or shadowed spot `c - 4`; alpha-masked
    /// casters are alpha tested.
    fn shadow(&self, enc: &mut wgpu::CommandEncoder, c: usize, base: usize, tw: Option<wgpu::RenderPassTimestampWrites<'_>>, args: &mut Vec<IndirectArgs>, stats: &mut RenderStats) {
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("shadow"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: if c < 4 { &self.shadow_layers[c] } else { &self.spot_shadow_layers[c - 4] },
                depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                stencil_ops: None,
            }),
            timestamp_writes: tw,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_vertex_buffer(1, self.instances.slice(..));
        let mut bound: Option<(bool, bool)> = None;
        let mut bound_mat = u32::MAX;
        let mut ds = DrawStream::new(self.vpool, self.ipool, self.indirect, args, base);
        for b in &self.shadow_batches[c] {
            let masked = self.materials[b.material as usize].masked;
            let mesh = &self.meshes[b.mesh as usize];
            let Some(r) = mesh.draw_range(b.lod) else { continue };
            if bound != Some((masked, b.skinned)) || (masked && b.material != bound_mat) {
                ds.flush(&mut pass);
            }
            if bound != Some((masked, b.skinned)) {
                pass.set_pipeline(match (masked, b.skinned) {
                    (false, false) => &self.pipelines.shadow,
                    (false, true) => &self.pipelines.shadow_skinned,
                    (true, false) => &self.pipelines.shadow_masked,
                    (true, true) => &self.pipelines.shadow_skinned_masked,
                });
                pass.set_bind_group(0, &self.cascade_binds[c], &[]);
                pass.set_bind_group(1, self.joint_bind, &[]);
                bound = Some((masked, b.skinned));
                bound_mat = u32::MAX;
            }
            if masked && b.material != bound_mat {
                pass.set_bind_group(2, &self.materials[b.material as usize].bind_group, &[]);
                bound_mat = b.material;
            }
            ds.draw(&mut pass, &r, if b.skinned { mesh.skin.as_ref().map(|sk| (b.mesh, sk)) } else { None }, b.first..b.first + b.count);
            stats.shadow_draws += 1;
            stats.shadow_instances += b.count;
            stats.shadow_triangles += (r.count / 3) as u64 * b.count as u64;
        }
        ds.flush(&mut pass);
        stats.api_draws += ds.calls;
    }

    /// Depth + normal prepass of the opaque surfaces (blended ones are
    /// sorted last and skipped). Opaque materials keep whatever material
    /// group is bound: their prepass shader reads no texture.
    fn prepass(&self, enc: &mut wgpu::CommandEncoder, base: usize, tw: Option<wgpu::RenderPassTimestampWrites<'_>>, args: &mut Vec<IndirectArgs>, stats: &mut RenderStats) {
        let t = self.targets;
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("prepass"),
            color_attachments: &[
                Some(wgpu::RenderPassColorAttachment {
                    view: &t.normal,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.0, g: 1.0, b: 0.0, a: 0.0 }), store: wgpu::StoreOp::Store },
                }),
                Some(wgpu::RenderPassColorAttachment { view: &t.velocity, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store } }),
            ],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &t.depth,
                depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(0.0), store: wgpu::StoreOp::Store }),
                stencil_ops: None,
            }),
            timestamp_writes: tw,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_vertex_buffer(1, self.instances.slice(..));
        let mut bound_pipe = usize::MAX;
        let mut bound_mat = u32::MAX;
        let mut ds = DrawStream::new(self.vpool, self.ipool, self.indirect, args, base);
        for b in self.main_batches.iter().take_while(|b| b.pipe < 3) {
            let Some(r) = self.meshes[b.mesh as usize].draw_range(b.lod) else { continue };
            let pipe = b.pipe as usize;
            let masked = self.materials[b.material as usize].masked;
            let pp = pipe * 2 + masked as usize;
            if pp != bound_pipe || ((masked || bound_mat == u32::MAX) && b.material != bound_mat) {
                ds.flush(&mut pass);
            }
            if pp != bound_pipe {
                pass.set_pipeline(&self.pipelines.prepass[pipe][masked as usize]);
                pass.set_bind_group(0, self.frame_bind, &[]);
                pass.set_bind_group(2, self.joint_bind, &[]);
                bound_pipe = pp;
            }
            if (masked || bound_mat == u32::MAX) && b.material != bound_mat {
                pass.set_bind_group(1, &self.materials[b.material as usize].bind_group, &[]);
                bound_mat = b.material;
            }
            let mesh = &self.meshes[b.mesh as usize];
            ds.draw(&mut pass, &r, if pipe == 2 { mesh.skin.as_ref().map(|sk| (b.mesh, sk)) } else { None }, b.first..b.first + b.count);
        }
        ds.flush(&mut pass);
        stats.api_draws += ds.calls;
    }

    /// Main HDR pass + sky + lines, depth-tested against the prepass
    /// (read-only, so the shaders can also sample it).
    fn main(&self, enc: &mut wgpu::CommandEncoder, base: usize, tw: Option<wgpu::RenderPassTimestampWrites<'_>>, (lo, hi, first, last): (usize, usize, bool, bool), args: &mut Vec<IndirectArgs>, stats: &mut RenderStats) {
        let t = self.targets;
        let main_batches = self.main_batches;
        // The sky goes after the opaque batches and before the blended ones.
        let sky_at = main_batches.iter().position(|b| b.pipe >= 3).unwrap_or(main_batches.len());
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("main"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &t.hdr,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: if first { wgpu::LoadOp::Clear(wgpu::Color::BLACK) } else { wgpu::LoadOp::Load }, store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: &t.depth, depth_ops: None, stencil_ops: None }),
            timestamp_writes: tw,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_vertex_buffer(1, self.instances.slice(..));
        let mut bound_pipe = u8::MAX;
        let mut bound_mat = u32::MAX;
        let mut ds = DrawStream::new(self.vpool, self.ipool, self.indirect, args, base);
        // Opaque first; the sky goes after opaque and before blended (in
        // the chunk that holds the first blended batch, or the last one).
        let end = if last { main_batches.len() + 1 } else { hi };
        for i in lo..end {
            if i == sky_at {
                ds.flush(&mut pass);
                pass.set_pipeline(&self.pipelines.sky);
                pass.set_bind_group(0, self.frame_bind, &[]);
                pass.set_bind_group(1, self.sky_bind, &[]);
                pass.draw(0..3, 0..1);
                if self.sprites > 0 {
                    pass.set_pipeline(&self.pipelines.sprites);
                    pass.draw(0..4, 0..self.sprites);
                }
                bound_pipe = u8::MAX;
            }
            let Some(b) = main_batches.get(i) else { break };
            let Some(r) = self.meshes[b.mesh as usize].draw_range(b.lod) else { continue };
            if b.pipe != bound_pipe || b.material != bound_mat {
                ds.flush(&mut pass);
            }
            if b.pipe != bound_pipe {
                pass.set_pipeline(match b.pipe {
                    0 => &self.pipelines.opaque,
                    1 => &self.pipelines.double_sided,
                    2 => &self.pipelines.skinned,
                    4 => &self.pipelines.skinned_blend,
                    _ => &self.pipelines.blend,
                });
                pass.set_bind_group(0, self.frame_bind, &[]);
                pass.set_bind_group(2, self.joint_bind, &[]);
                pass.set_bind_group(3, &t.screen_bind, &[]);
                bound_pipe = b.pipe;
                bound_mat = u32::MAX;
            }
            if b.material != bound_mat {
                pass.set_bind_group(1, &self.materials[b.material as usize].bind_group, &[]);
                bound_mat = b.material;
            }
            let mesh = &self.meshes[b.mesh as usize];
            ds.draw(&mut pass, &r, if b.skinned { mesh.skin.as_ref().map(|sk| (b.mesh, sk)) } else { None }, b.first..b.first + b.count);
            stats.draw_calls += 1;
            stats.instances_drawn += b.count;
            stats.triangles += (r.count / 3) as u64 * b.count as u64;
        }
        ds.flush(&mut pass);
        stats.api_draws += ds.calls;
        if last && self.lines > 0 {
            pass.set_pipeline(&self.pipelines.lines);
            pass.set_bind_group(0, self.frame_bind, &[]);
            pass.set_vertex_buffer(0, self.line_buf.slice(..));
            pass.draw(0..self.lines, 0..1);
        }
    }
}

/// One view's static instances and batches (see [`Renderer::render`]).
#[derive(Default)]
struct ViewList {
    out: Vec<InstanceRaw>,
    batches: Vec<Batch>,
    scratch: static_scene::GatherScratch,
    /// First instance of `out` in the frame's instance buffer.
    base: u32,
    /// Worker CPU ms of the last gather.
    ms: f32,
}

#[derive(Clone, Copy)]
struct DrawRange {
    vpage: u32,
    ipage: u32,
    first: u32,
    count: u32,
    base_vertex: i32,
}

/// The frame's small buffer writes (uniforms, lights, joints, lines, draw
/// arguments) packed into one staging copy: every `queue.write_buffer`
/// creates and later frees its own staging buffer in wgpu-core (a
/// placed resource per call on D3D12). [`FrameWrites::flush`] writes them
/// all at once into a persistent upload buffer and copies each to its
/// destination in a command buffer that runs first in the frame's submit,
/// so they land before any pass, exactly like `write_buffer`.
#[derive(Default)]
struct FrameWrites {
    data: Vec<u8>,
    /// Destination, offset there, offset in `data`, size.
    copies: Vec<(wgpu::Buffer, u64, u64, u64)>,
    upload: Option<wgpu::Buffer>,
    cap: u64,
    /// Writes of the last frame (frame trace).
    pub last: usize,
}

impl FrameWrites {
    fn stage(&mut self, dst: &wgpu::Buffer, offset: u64, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        let at = self.data.len() as u64;
        self.data.extend_from_slice(bytes);
        // Copies need 4-byte sizes and offsets.
        while self.data.len() % wgpu::COPY_BUFFER_ALIGNMENT as usize != 0 {
            self.data.push(0);
        }
        self.copies.push((dst.clone(), offset, at, self.data.len() as u64 - at));
    }

    /// The copy command buffer (to submit before the frame's passes).
    fn flush(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) -> Option<wgpu::CommandBuffer> {
        self.last = self.copies.len();
        if self.copies.is_empty() {
            return None;
        }
        let len = self.data.len() as u64;
        if len > self.cap || self.upload.is_none() {
            self.cap = len.next_power_of_two().max(64 * 1024);
            self.upload = Some(device.create_buffer(&wgpu::BufferDescriptor { label: Some("frame writes"), size: self.cap, usage: wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }));
        }
        let upload = self.upload.as_ref().expect("allocated");
        queue.write_buffer(upload, 0, &self.data);
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame writes") });
        for (dst, off, at, size) in self.copies.drain(..) {
            enc.copy_buffer_to_buffer(upload, at, &dst, off, size);
        }
        self.data.clear();
        Some(enc.finish())
    }
}

/// `DrawIndexedIndirectArgs` as the GPU reads them.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct IndirectArgs {
    index_count: u32,
    instance_count: u32,
    first_index: u32,
    base_vertex: i32,
    first_instance: u32,
}

/// Draw submission of one pass: binds the pool pages (and skin streams)
/// only when they change, and merges consecutive draws that share all
/// bound state into one `multi_draw_indexed_indirect` (shadow and depth
/// passes become a handful of calls instead of one per mesh). The
/// arguments are collected in a frame-wide list written to `indirect`
/// before the submit. The caller must [`DrawStream::flush`] before it
/// changes pipeline or bind groups, and at the end of the pass.
struct DrawStream<'a> {
    vpool: &'a mesh_pool::MeshPool,
    ipool: &'a mesh_pool::MeshPool,
    /// `None`: every draw is direct (no `INDIRECT_FIRST_INSTANCE`).
    indirect: Option<&'a wgpu::Buffer>,
    args: &'a mut Vec<IndirectArgs>,
    /// Index of `args[0]` in the indirect buffer.
    base: usize,
    start: usize,
    vpage: Option<u32>,
    ipage: Option<u32>,
    skin: Option<u32>,
    /// Draw calls issued (direct or multi).
    pub calls: u32,
}

impl<'a> DrawStream<'a> {
    fn new(vpool: &'a mesh_pool::MeshPool, ipool: &'a mesh_pool::MeshPool, indirect: Option<&'a wgpu::Buffer>, args: &'a mut Vec<IndirectArgs>, base: usize) -> Self {
        let start = args.len();
        Self { vpool, ipool, indirect, args, base, start, vpage: None, ipage: None, skin: None, calls: 0 }
    }

    /// Issues the merged draws collected since the last flush.
    fn flush(&mut self, pass: &mut wgpu::RenderPass<'_>) {
        let n = self.args.len() - self.start;
        if n == 1 {
            // A single draw goes direct (no indirect validation).
            let a = self.args.pop().expect("one draw");
            pass.draw_indexed(a.first_index..a.first_index + a.index_count, a.base_vertex, a.first_instance..a.first_instance + a.instance_count);
            self.calls += 1;
        } else if n > 1 {
            let buf = self.indirect.expect("indirect buffer");
            pass.multi_draw_indexed_indirect(buf, ((self.base + self.start) * std::mem::size_of::<IndirectArgs>()) as u64, n as u32);
            self.calls += 1;
        }
        self.start = self.args.len();
    }

    /// Draws `instances` of `r` (with the skin stream of mesh `skin.0`).
    fn draw(&mut self, pass: &mut wgpu::RenderPass<'_>, r: &DrawRange, skin: Option<(u32, &wgpu::Buffer)>, instances: std::ops::Range<u32>) {
        let skin_change = skin.is_some_and(|(id, _)| self.skin != Some(id));
        if self.vpage != Some(r.vpage) || self.ipage != Some(r.ipage) || skin_change {
            self.flush(pass);
            if self.vpage != Some(r.vpage) {
                pass.set_vertex_buffer(0, self.vpool.buffer(r.vpage).slice(..));
                self.vpage = Some(r.vpage);
            }
            if self.ipage != Some(r.ipage) {
                pass.set_index_buffer(self.ipool.buffer(r.ipage).slice(..), wgpu::IndexFormat::Uint32);
                self.ipage = Some(r.ipage);
            }
            if let Some((id, sk)) = skin
                && skin_change
            {
                pass.set_vertex_buffer(2, sk.slice(..));
                self.skin = Some(id);
            }
        }
        if self.indirect.is_none() {
            pass.draw_indexed(r.first..r.first + r.count, r.base_vertex, instances);
            self.calls += 1;
            return;
        }
        self.args.push(IndirectArgs { index_count: r.count, instance_count: instances.end - instances.start, first_index: r.first, base_vertex: r.base_vertex, first_instance: instances.start });
    }
}

/// Vertex data of a scene's primitives in the renderer's layout, ready to
/// be copied to the GPU ([`Renderer::reload_prepared`]). Building it is
/// the CPU-heavy part of a re-upload and needs no renderer, so it can run
/// on a worker thread.
pub struct MeshUpload {
    prims: Vec<Option<PrimUpload>>,
}

struct PrimUpload {
    vertices: Vec<Vertex>,
    indices: Vec<u32>,
    lods: Vec<Vec<u32>>,
}

impl MeshUpload {
    /// Converts every primitive of `scene` (in upload order). Skinned or
    /// malformed primitives are skipped (they are never streamed).
    pub fn prepare(scene: &SceneData) -> Self {
        let prims = scene
            .meshes
            .iter()
            .flatten()
            .map(|p| {
                if p.is_skinned() || p.positions.len() != p.normals.len() {
                    return None;
                }
                let vertices = (0..p.positions.len())
                    .map(|i| Vertex { pos: p.positions[i].to_array(), normal: p.normals[i].to_array(), uv: p.uvs.get(i).copied().unwrap_or_default().to_array(), tangent: p.tangents.get(i).copied().unwrap_or(Vec4::X).to_array() })
                    .collect();
                Some(PrimUpload { vertices, indices: p.indices.clone(), lods: p.lods.clone() })
            })
            .collect();
        Self { prims }
    }

    /// Bytes this upload writes.
    pub fn bytes(&self) -> u64 {
        self.prims.iter().flatten().map(|p| (p.vertices.len() * std::mem::size_of::<Vertex>() + (p.indices.len() + p.lods.iter().map(|l| l.len()).sum::<usize>()) * 4) as u64).sum()
    }
}

struct GpuMaterial {
    bind_group: wgpu::BindGroup,
    blend: bool,
    /// Alpha-tested (leaves, hair cards): needs the texture in depth passes.
    masked: bool,
    double_sided: bool,
}

struct ModelPart {
    mesh: u32,
    node: usize,
}

struct GpuModel {
    parts: Vec<ModelPart>,
    rest: Vec<Mat4>,
    bounds: Aabb,
    /// GPU mesh of every primitive, in `SceneData::meshes` order.
    prims: Vec<u32>,
    /// Bytes of the mesh buffers while resident.
    resident: Option<u64>,
}

struct Targets {
    width: u32,
    height: u32,
    hdr: wgpu::TextureView,
    depth: wgpu::TextureView,
    /// Geometric normals of the depth prepass (world space).
    normal: wgpu::TextureView,
    ao_raw: wgpu::TextureView,
    ao: wgpu::TextureView,
    /// Half-resolution GTAO (raw, denoised) and its bind groups.
    ao_half_raw: wgpu::TextureView,
    ao_half: wgpu::TextureView,
    gtao_half_bind: wgpu::BindGroup,
    blur_half_bind: wgpu::BindGroup,
    upsample_bind: wgpu::BindGroup,
    /// TAA output of this frame (bloom and tonemap read it).
    resolved_tex: wgpu::Texture,
    resolved: wgpu::TextureView,
    /// Last frame's resolved image (TAA history, SSR source).
    history_tex: wgpu::Texture,
    ldr: wgpu::TextureView,
    bloom: Vec<(wgpu::TextureView, u32, u32)>,
    /// 1x1: the camera's adapted EV100 (R), written by the meter pass.
    exposure: wgpu::TextureView,
    post_bind: PostBinds,
    gtao_bind: wgpu::BindGroup,
    blur_bind: wgpu::BindGroup,
    taa_bind: wgpu::BindGroup,
    screen_bind: wgpu::BindGroup,
    /// Camera stage (look.rs): LDR input and depth-of-field bindings.
    camera: look::CameraFxTargets,
    /// Bloom's first downsample and the tonemap reading the depth of
    /// field output (written into `hdr`, free after TAA) instead of
    /// `resolved`.
    dof_down: wgpu::BindGroup,
    dof_tonemap: wgpu::BindGroup,
    /// Eye adaptation compute inputs (reads `resolved`).
    exposure_bind: wgpu::BindGroup,
    /// Prepass screen motion (px per frame).
    velocity: wgpu::TextureView,
    /// TAA history view (the motion blur reads it: a copy of `resolved`).
    history: wgpu::TextureView,
}

struct PostBinds {
    down: Vec<wgpu::BindGroup>,
    up: Vec<wgpu::BindGroup>,
    tonemap: wgpu::BindGroup,
    fxaa: wgpu::BindGroup,
    meter: wgpu::BindGroup,
    meter_uniform: wgpu::Buffer,
    uniforms: Vec<wgpu::Buffer>,
}

struct Pipelines {
    opaque: wgpu::RenderPipeline,
    double_sided: wgpu::RenderPipeline,
    skinned: wgpu::RenderPipeline,
    blend: wgpu::RenderPipeline,
    skinned_blend: wgpu::RenderPipeline,
    /// Depth/normal prepass: [opaque, double-sided, skinned] x [plain, masked].
    prepass: [[wgpu::RenderPipeline; 2]; 3],
    shadow: wgpu::RenderPipeline,
    shadow_skinned: wgpu::RenderPipeline,
    shadow_masked: wgpu::RenderPipeline,
    shadow_skinned_masked: wgpu::RenderPipeline,
    gtao: wgpu::RenderPipeline,
    ao_blur: wgpu::RenderPipeline,
    ao_upsample: wgpu::RenderPipeline,
    taa: wgpu::RenderPipeline,
    sky: wgpu::RenderPipeline,
    lines: wgpu::RenderPipeline,
    down: wgpu::RenderPipeline,
    up: wgpu::RenderPipeline,
    meter: wgpu::RenderPipeline,
    tonemap: wgpu::RenderPipeline,
    fxaa: wgpu::RenderPipeline,
    sprites: wgpu::RenderPipeline,
    cluster: wgpu::ComputePipeline,
    exposure_begin: wgpu::ComputePipeline,
    exposure_hist: wgpu::ComputePipeline,
    exposure_adapt: wgpu::ComputePipeline,
}

struct EnvInfo {
    sun_dir: Vec3,
    sun_color: Vec3,
    sh: [Vec3; 9],
    max_mip: f32,
    prefiltered: bool,
    zenith: Vec3,
    horizon: Vec3,
    ground: Vec3,
    /// Sun irradiance above the atmosphere that matches the photo's sun, for
    /// the analytic sky shown from high viewpoints.
    sun_top: f32,
}

pub struct Renderer {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub adapter_info: wgpu::AdapterInfo,
    /// The adapter the device was opened on ([`Renderer::new`]; `None`
    /// with [`Renderer::with_device`]).
    pub adapter: Option<wgpu::Adapter>,
    pub output_format: wgpu::TextureFormat,
    pipelines: Pipelines,
    frame_bgl: wgpu::BindGroupLayout,
    material_bgl: wgpu::BindGroupLayout,
    joints_bgl: wgpu::BindGroupLayout,
    post_bgl: wgpu::BindGroupLayout,
    screen_bgl: wgpu::BindGroupLayout,
    gtao_bgl: wgpu::BindGroupLayout,
    taa_bgl: wgpu::BindGroupLayout,
    cluster_bgl: wgpu::BindGroupLayout,
    exposure_bgl: wgpu::BindGroupLayout,
    taa_buf: wgpu::Buffer,
    frame_buf: wgpu::Buffer,
    frame_bind: wgpu::BindGroup,
    shadow_view: wgpu::TextureView,
    shadow_layers: Vec<wgpu::TextureView>,
    cascade_bufs: Vec<wgpu::Buffer>,
    cascade_binds: Vec<wgpu::BindGroup>,
    shadow_size: u32,
    linear_sampler: wgpu::Sampler,
    clamp_sampler: wgpu::Sampler,
    shadow_sampler: wgpu::Sampler,
    env_view: wgpu::TextureView,
    dfg_view: wgpu::TextureView,
    env: Option<EnvInfo>,
    meshes: Vec<GpuMesh>,
    /// Shared vertex and index buffers of every mesh (see [`mesh_pool`]).
    vertex_pool: mesh_pool::MeshPool,
    index_pool: mesh_pool::MeshPool,
    texture_cache: std::collections::HashMap<(usize, bool), (std::sync::Arc<[u8]>, wgpu::TextureView)>,
    default_textures: Option<[wgpu::TextureView; 3]>,
    materials: Vec<GpuMaterial>,
    models: Vec<GpuModel>,
    instance_buf: wgpu::Buffer,
    instance_cap: u64,
    joint_buf: wgpu::Buffer,
    joint_cap: u64,
    joint_bind: wgpu::BindGroup,
    line_buf: wgpu::Buffer,
    line_cap: u64,
    /// Local lights of the frame (`GpuLight`), grown on demand.
    lights_buf: wgpu::Buffer,
    lights_cap: u64,
    /// Light lists of the froxel grid (sized at resize).
    cluster_buf: wgpu::Buffer,
    cluster_grid: (u32, u32),
    cluster_bind: wgpu::BindGroup,
    /// `ExposureState` (pre-exposure, exposure), its histogram and params.
    exposure_state: wgpu::Buffer,
    exposure_hist: wgpu::Buffer,
    exposure_uniform: wgpu::Buffer,
    spot_shadow_view: wgpu::TextureView,
    spot_shadow_layers: Vec<wgpu::TextureView>,
    /// Adapt instantly (still captures, first frame of auto exposure).
    exposure_instant: bool,
    auto_exposure_prev: bool,
    prev_time: f32,
    targets: Option<Targets>,
    /// Frame time of the last metered exposure (`None`: snap next frame).
    exposure_time: Option<f32>,
    camera_fx: look::CameraFx,
    pub stats: RenderStats,
    pub settings: RenderSettings,
    temporal: TemporalState,
    quality_buf: wgpu::Buffer,
    /// GPU pass timings (timestamp queries) of a recent frame.
    pub profiler: profiler::GpuProfiler,
    /// CPU scopes of the last [`Renderer::render`].
    pub cpu: profiler::CpuScopes,
    statics: StaticScene,
    cascade_cache: [CascadeCache; 4],
    /// Sun direction the shadow maps are fitted to: follows the sun in
    /// steps of 0.1° (stable shadows), so a moving sun does not make
    /// every frame's shadow maps rasterise thin casters differently.
    shadow_sun: Vec3,
    /// Bytes of uploaded textures and meshes (VRAM estimate).
    texture_bytes: u64,
    mesh_bytes: u64,
    target_bytes: u64,
    /// Bytes written since the last flush of the upload queue (see
    /// [`Renderer::flush_uploads`]).
    pending_upload: u64,
    /// Bytes written since the last frame or blocking flush.
    unflushed_upload: u64,
    /// Blocking upload flushes so far: `(count, total ms)` waited in
    /// [`Renderer::flush_uploads`] (the sandbox's `--frame-trace` reads it).
    pub upload_waits: (u32, f64),
    /// Non-blocking upload submits ([`Renderer::submit_uploads`]).
    pub upload_submits: u32,
    scratch: Vec<InstanceRaw>,
    /// Per-view instance lists of the static set (gathered in parallel).
    view_lists: Vec<ViewList>,
    /// Arguments of the merged draws ([`DrawStream`]) and their buffer.
    indirect_args: Vec<IndirectArgs>,
    /// The frame's small writes, staged together ([`FrameWrites`]).
    writes: FrameWrites,
    indirect_buf: Option<wgpu::Buffer>,
    indirect_cap: u64,
    /// Batches of the last frame: camera, then every shadow view.
    last_batches: (Vec<Batch>, Vec<Batch>),
    sky_bgl: wgpu::BindGroupLayout,
    /// Sky LUTs and the froxel fog volume.
    fog: fog::Fog,
    motion: motion::MotionBlur,
    /// Last frame's transform of every `DrawInstance::motion_id`.
    prev_motion: std::collections::HashMap<u64, Mat4>,
    /// Sky pass inputs (LUTs, fog volume); rebuilt when the volume changes.
    sky_bind: Option<wgpu::BindGroup>,
}

/// Last rendered state of a shadow cascade (far cascades are refreshed
/// every few frames and reuse it in between).
#[derive(Clone, Copy, Default)]
struct CascadeCache {
    valid: bool,
    vp: Mat4,
    texel: f32,
    range: f32,
    split: f32,
    sun: Vec3,
}

/// Frame-to-frame state of the temporal passes.
#[derive(Default)]
struct TemporalState {
    frame_index: u32,
    history_valid: bool,
    prev_view_proj: Mat4,
    /// Camera, time and size of the last frame; equal means a still image.
    prev_key: Option<[f32; 11]>,
    still_frames: u32,
    /// Keep the history for reprojection (SSR) but restart the
    /// accumulation: the next frame replaces it.
    restart: bool,
}

/// Per-frame output of [`Renderer::temporal_frame`].
struct TemporalFrame {
    view_proj: Mat4,
    prev_view_proj: Mat4,
    jitter: forja_math::Vec2,
    frame_index: u32,
    history_valid: bool,
    taa_params: [f32; 4],
}

/// Radical inverse (Halton sequence) used for the TAA sub-pixel jitter.
fn halton(mut i: u32, base: u32) -> f32 {
    let (mut f, mut r) = (1.0f32, 0.0f32);
    while i > 0 {
        f /= base as f32;
        r += f * (i % base) as f32;
        i /= base;
    }
    r
}

/// DX12 first on Windows (Vulkan fallback); the platform's primary APIs
/// elsewhere. `WGPU_BACKEND` overrides it.
pub fn default_backends() -> wgpu::Backends {
    if cfg!(windows) { wgpu::Backends::DX12 | wgpu::Backends::VULKAN } else { wgpu::Backends::PRIMARY }
}

/// wgpu's release defaults minus `VALIDATION_INDIRECT_CALL`: with it, every
/// `multi_draw_indexed_indirect` is split per argument on the CPU and
/// rewritten by a validation compute pass (on DX12 the draws then read a
/// copy with another stride). The renderer writes its own arguments from
/// validated mesh ranges. `WGPU_VALIDATION_INDIRECT_CALL=1` turns it back on
/// (`with_env` runs after this).
fn instance_flags(flags: wgpu::InstanceFlags) -> wgpu::InstanceFlags {
    flags - wgpu::InstanceFlags::VALIDATION_INDIRECT_CALL
}

/// Instance bound to a window system display (needed by Wayland/GL; harmless elsewhere).
pub fn create_instance_with_display(display: Box<dyn wgpu::wgt::WgpuHasDisplayHandle>) -> wgpu::Instance {
    let mut desc = wgpu::InstanceDescriptor::new_with_display_handle(display);
    desc.backends = default_backends();
    desc.flags = instance_flags(desc.flags);
    wgpu::Instance::new(desc.with_env())
}

/// Instance for offscreen rendering or platforms that need no display handle
/// (Windows, macOS, X11 through Vulkan).
pub fn create_instance() -> wgpu::Instance {
    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
    desc.backends = default_backends();
    desc.flags = instance_flags(desc.flags);
    wgpu::Instance::new(desc.with_env())
}

/// WGSL source of `shaders/<name>.wgsl`: the embedded copy, or the file in
/// `FORJA_SHADER_DIR` when that is set (shader iteration without a rebuild).
fn wgsl(name: &str, embedded: &str) -> String {
    if let Ok(dir) = std::env::var("FORJA_SHADER_DIR")
        && let Ok(s) = std::fs::read_to_string(std::path::Path::new(&dir).join(format!("{name}.wgsl")))
    {
        return s;
    }
    embedded.to_string()
}

fn shader(device: &wgpu::Device, label: &str, src: &str, with_common: bool) -> wgpu::ShaderModule {
    let src = wgsl(label, src);
    let code = if with_common { format!("{}\n{}\n{src}", wgsl("frame", FRAME), wgsl("common", COMMON)) } else { src };
    device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some(label), source: wgpu::ShaderSource::Wgsl(code.into()) })
}

fn vertex_layouts(skinned: bool) -> Vec<Option<wgpu::VertexBufferLayout<'static>>> {
    const VERT: [wgpu::VertexAttribute; 4] = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2, 3 => Float32x4];
    const INST: [wgpu::VertexAttribute; 6] = wgpu::vertex_attr_array![4 => Float32x4, 5 => Float32x4, 6 => Float32x4, 7 => Float32x4, 8 => Float32x4, 9 => Uint32x4];
    const SKIN: [wgpu::VertexAttribute; 2] = wgpu::vertex_attr_array![10 => Uint16x4, 11 => Float32x4];
    let mut v = vec![
        Some(wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<Vertex>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &VERT }),
        Some(wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<InstanceRaw>() as u64, step_mode: wgpu::VertexStepMode::Instance, attributes: &INST }),
    ];
    if skinned {
        v.push(Some(wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<SkinVertex>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &SKIN }));
    }
    v
}

fn texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
        count: None,
    }
}

fn storage_entry(binding: u32, visibility: wgpu::ShaderStages, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry { binding, visibility, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only }, has_dynamic_offset: false, min_binding_size: None }, count: None }
}

/// Light lists of the froxel grid for a viewport.
fn cluster_buffer(device: &wgpu::Device, width: u32, height: u32) -> wgpu::Buffer {
    let (tx, ty) = lights::cluster_grid(width, height);
    let words = tx * ty * (lights::CLUSTER_SLICES + 1) * lights::CLUSTER_STRIDE;
    device.create_buffer(&wgpu::BufferDescriptor { label: Some("clusters"), size: words as u64 * 4, usage: wgpu::BufferUsages::STORAGE, mapped_at_creation: false })
}

fn volume_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D3, multisampled: false },
        count: None,
    }
}

fn uniform_entry(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry { binding, visibility, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None }
}

fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None }
}

impl Renderer {
    /// Creates a renderer. Pass the window surface when there is one; the
    /// output format follows the surface (or sRGB RGBA8 offscreen).
    pub fn new(instance: &wgpu::Instance, surface: Option<&wgpu::Surface<'_>>) -> Result<Self, String> {
        Self::new_with(instance, surface, &gpu::AdapterChoice::from_env())
    }

    /// [`Renderer::new`] on a chosen adapter (see [`gpu`]: best GPU first,
    /// falling back adapter by adapter, DX12 -> Vulkan).
    pub fn new_with(instance: &wgpu::Instance, surface: Option<&wgpu::Surface<'_>>, choice: &gpu::AdapterChoice) -> Result<Self, String> {
        let (adapter, device, queue) = gpu::open(instance, surface, choice)?;
        let adapter_info = adapter.get_info();
        let output_format = surface
            .map(|s| {
                let caps = s.get_capabilities(&adapter);
                caps.formats.iter().copied().find(|f| f.is_srgb()).unwrap_or(caps.formats[0])
            })
            .unwrap_or(wgpu::TextureFormat::Rgba8UnormSrgb);
        let mut r = Self::with_device(device, queue, adapter_info, output_format);
        r.adapter = Some(adapter);
        Ok(r)
    }

    pub fn with_device(device: wgpu::Device, queue: wgpu::Queue, adapter_info: wgpu::AdapterInfo, output_format: wgpu::TextureFormat) -> Self {
        // The frame group is also read by the compute passes (froxel fog).
        let fc = wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE;
        let with_vis = |mut e: wgpu::BindGroupLayoutEntry, v: wgpu::ShaderStages| {
            e.visibility = v;
            e
        };
        let frame_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("frame"),
            entries: &[
                uniform_entry(0, wgpu::ShaderStages::VERTEX | fc),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: fc,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2Array, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: fc, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison), count: None },
                with_vis(texture_entry(3), fc),
                with_vis(sampler_entry(4), fc),
                storage_entry(5, wgpu::ShaderStages::VERTEX | fc, true),
                storage_entry(6, fc, true),
                storage_entry(7, fc, true),
                wgpu::BindGroupLayoutEntry {
                    binding: 8,
                    visibility: fc,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2Array, multisampled: false },
                    count: None,
                },
                with_vis(texture_entry(9), fc),
            ],
        });
        let material_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("material"),
            entries: &[uniform_entry(0, wgpu::ShaderStages::FRAGMENT), texture_entry(1), texture_entry(2), texture_entry(3), texture_entry(4), texture_entry(5), sampler_entry(6)],
        });
        let joints_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("joints"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }],
        });
        let cascade_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("cascade"), entries: &[uniform_entry(0, wgpu::ShaderStages::VERTEX)] });
        let post_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("post"),
            entries: &[texture_entry(0), sampler_entry(1), uniform_entry(2, wgpu::ShaderStages::FRAGMENT), texture_entry(3), storage_entry(4, wgpu::ShaderStages::FRAGMENT, true), texture_entry(5)],
        });
        let cluster_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("cluster"),
            entries: &[uniform_entry(0, wgpu::ShaderStages::COMPUTE), storage_entry(1, wgpu::ShaderStages::COMPUTE, true), storage_entry(2, wgpu::ShaderStages::COMPUTE, false)],
        });
        let exposure_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("exposure"),
            entries: &[
                uniform_entry(0, wgpu::ShaderStages::COMPUTE),
                storage_entry(1, wgpu::ShaderStages::COMPUTE, false),
                storage_entry(2, wgpu::ShaderStages::COMPUTE, false),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
            ],
        });
        let depth_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
            count: None,
        };
        // Main pass screen inputs: GTAO, prepass depth (read-only attachment
        // at the same time), last frame's image and the prepass normals
        // (screen-space reflections).
        let screen_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("screen"),
            entries: &[
                texture_entry(0),
                depth_entry(1),
                texture_entry(2),
                sampler_entry(3),
                texture_entry(4),
                uniform_entry(5, wgpu::ShaderStages::FRAGMENT),
                volume_entry(6),
                sampler_entry(7),
            ],
        });
        // Sky pass: sky-view and transmittance LUTs, the froxel fog volume.
        let sky_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("sky"), entries: &[texture_entry(0), texture_entry(1), sampler_entry(2), volume_entry(3)] });
        let gtao_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("gtao"), entries: &[depth_entry(0), texture_entry(1), texture_entry(2), uniform_entry(3, wgpu::ShaderStages::FRAGMENT)] });
        let taa_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("taa"),
            entries: &[texture_entry(0), texture_entry(1), depth_entry(2), sampler_entry(3), uniform_entry(4, wgpu::ShaderStages::FRAGMENT), texture_entry(5)],
        });

        let fog_apply = wgsl("fog_apply", include_str!("shaders/fog_apply.wgsl"));
        let pbr = shader(&device, "pbr (fog)", &format!("{}\n{fog_apply}", wgsl("pbr", include_str!("shaders/pbr.wgsl"))), true);
        let sky = shader(&device, "sky (fog)", &format!("{}\n{fog_apply}", wgsl("sky", include_str!("shaders/sky.wgsl"))), true);
        let lines = shader(&device, "lines", include_str!("shaders/lines.wgsl"), true);
        let shadow = shader(&device, "shadow", include_str!("shaders/shadow.wgsl"), false);
        let easu_src = wgsl("easu", include_str!("shaders/easu.wgsl"));
        let post = shader(&device, "post (easu)", &format!("{}\n{easu_src}", wgsl("post", include_str!("shaders/post.wgsl"))), false);
        let gtao = shader(&device, "gtao", include_str!("shaders/gtao.wgsl"), true);
        let taa = shader(&device, "taa", include_str!("shaders/taa.wgsl"), true);
        let sprites = shader(&device, "sprites", include_str!("shaders/sprites.wgsl"), true);
        let cluster = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cluster"),
            source: wgpu::ShaderSource::Wgsl(format!("{}\n{}", wgsl("frame", FRAME), wgsl("cluster", include_str!("shaders/cluster.wgsl"))).into()),
        });
        let exposure = shader(&device, "exposure", include_str!("shaders/exposure.wgsl"), false);

        let layout = |bgls: &[&wgpu::BindGroupLayout]| {
            let v: Vec<Option<&wgpu::BindGroupLayout>> = bgls.iter().map(|b| Some(*b)).collect();
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &v, immediate_size: 0 })
        };
        let scene_layout = layout(&[&frame_bgl, &material_bgl, &joints_bgl, &screen_bgl]);
        let prepass_layout = layout(&[&frame_bgl, &material_bgl, &joints_bgl]);
        let frame_layout = layout(&[&frame_bgl]);
        let sky_layout = layout(&[&frame_bgl, &sky_bgl]);
        let shadow_layout = layout(&[&cascade_bgl, &joints_bgl]);
        let shadow_masked_layout = layout(&[&cascade_bgl, &joints_bgl, &material_bgl]);
        let post_layout = layout(&[&post_bgl]);
        let gtao_layout = layout(&[&frame_bgl, &gtao_bgl]);
        let taa_layout = layout(&[&frame_bgl, &taa_bgl]);
        let cluster_layout = layout(&[&cluster_bgl]);
        let exposure_layout = layout(&[&exposure_bgl]);
        let compute = |module: &wgpu::ShaderModule, layout: &wgpu::PipelineLayout, entry: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some(entry), layout: Some(layout), module, entry_point: Some(entry), compilation_options: Default::default(), cache: None })
        };

        let depth_state = |write: bool| wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(write),
            depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
            stencil: Default::default(),
            bias: Default::default(),
        };
        // The main pass tests against the prepass depth without writing it.
        let scene_pipeline = |label: &str, vs: &str, skinned: bool, cull: Option<wgpu::Face>, blend: bool| {
            let buffers = vertex_layouts(skinned);
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&scene_layout),
                vertex: wgpu::VertexState { module: &pbr, entry_point: Some(vs), compilation_options: Default::default(), buffers: &buffers },
                primitive: wgpu::PrimitiveState { cull_mode: cull, ..Default::default() },
                depth_stencil: Some(depth_state(false)),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &pbr,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: HDR_FORMAT,
                        blend: blend.then_some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let opaque = scene_pipeline("opaque", "vs_main", false, Some(wgpu::Face::Back), false);
        let double_sided = scene_pipeline("double", "vs_main", false, None, false);
        let skinned = scene_pipeline("skinned", "vs_skinned", true, None, false);
        let blend = scene_pipeline("blend", "vs_main", false, None, true);
        let skinned_blend = scene_pipeline("skinned_blend", "vs_skinned", true, None, true);

        let prepass_pipeline = |vs: &str, skinned: bool, cull: Option<wgpu::Face>, masked: bool| {
            let buffers = vertex_layouts(skinned);
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("prepass"),
                layout: Some(&prepass_layout),
                vertex: wgpu::VertexState { module: &pbr, entry_point: Some(vs), compilation_options: Default::default(), buffers: &buffers },
                primitive: wgpu::PrimitiveState { cull_mode: cull, ..Default::default() },
                depth_stencil: Some(depth_state(true)),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &pbr,
                    entry_point: Some(if masked { "fs_prepass_masked" } else { "fs_prepass" }),
                    compilation_options: Default::default(),
                    targets: &[Some(NORMAL_FORMAT.into()), Some(motion::VELOCITY_FORMAT.into())],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let prepass = [
            [prepass_pipeline("vs_main", false, Some(wgpu::Face::Back), false), prepass_pipeline("vs_main", false, Some(wgpu::Face::Back), true)],
            [prepass_pipeline("vs_main", false, None, false), prepass_pipeline("vs_main", false, None, true)],
            [prepass_pipeline("vs_skinned", true, None, false), prepass_pipeline("vs_skinned", true, None, true)],
        ];

        let shadow_pipeline = |vs: &str, skinned: bool, masked: bool| {
            let buffers = vertex_layouts(skinned);
            let fragment = masked.then(|| wgpu::FragmentState { module: &shadow, entry_point: Some("fs_masked"), compilation_options: Default::default(), targets: &[] });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("shadow"),
                layout: Some(if masked { &shadow_masked_layout } else { &shadow_layout }),
                vertex: wgpu::VertexState { module: &shadow, entry_point: Some(vs), compilation_options: Default::default(), buffers: &buffers },
                primitive: wgpu::PrimitiveState { cull_mode: None, unclipped_depth: false, ..Default::default() },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: SHADOW_FORMAT,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::LessEqual),
                    stencil: Default::default(),
                    // Small slope bias only: receivers apply an exact
                    // receiver-plane bias in the shader.
                    bias: wgpu::DepthBiasState { constant: 2, slope_scale: 1.0, clamp: 0.0 },
                }),
                multisample: Default::default(),
                fragment,
                multiview_mask: None,
                cache: None,
            })
        };
        let shadow_static = shadow_pipeline("vs_main", false, false);
        let shadow_skinned = shadow_pipeline("vs_skinned", true, false);
        let shadow_masked = shadow_pipeline("vs_masked", false, true);
        let shadow_skinned_masked = shadow_pipeline("vs_skinned_masked", true, true);

        let screen_pipeline = |module: &wgpu::ShaderModule, layout: &wgpu::PipelineLayout, entry: &str, format: wgpu::TextureFormat| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(layout),
                vertex: wgpu::VertexState { module, entry_point: Some("vs_fullscreen"), compilation_options: Default::default(), buffers: &[] },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState { module, entry_point: Some(entry), compilation_options: Default::default(), targets: &[Some(format.into())] }),
                multiview_mask: None,
                cache: None,
            })
        };
        let gtao_pipeline = screen_pipeline(&gtao, &gtao_layout, "fs_gtao", AO_FORMAT);
        let ao_blur_pipeline = screen_pipeline(&gtao, &gtao_layout, "fs_ao_blur", AO_FORMAT);
        let ao_upsample_pipeline = screen_pipeline(&gtao, &gtao_layout, "fs_ao_upsample", AO_FORMAT);
        let taa_pipeline = screen_pipeline(&taa, &taa_layout, "fs_taa", HDR_FORMAT);

        let sky_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sky"),
            layout: Some(&sky_layout),
            vertex: wgpu::VertexState { module: &sky, entry_point: Some("vs_main"), compilation_options: Default::default(), buffers: &[] },
            primitive: Default::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &sky,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(HDR_FORMAT.into())],
            }),
            multiview_mask: None,
            cache: None,
        });
        const LINE_ATTR: [wgpu::VertexAttribute; 2] = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4];
        let lines_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("lines"),
            layout: Some(&frame_layout),
            vertex: wgpu::VertexState {
                module: &lines,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<LineVertex>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &LINE_ATTR })],
            },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::LineList, ..Default::default() },
            depth_stencil: Some(depth_state(false)),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &lines,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format: HDR_FORMAT, blend: Some(wgpu::BlendState::ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL })],
            }),
            multiview_mask: None,
            cache: None,
        });
        // Light sprites: additive, depth tested against the prepass.
        let sprites_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sprites"),
            layout: Some(&frame_layout),
            vertex: wgpu::VertexState { module: &sprites, entry_point: Some("vs_sprite"), compilation_options: Default::default(), buffers: &[] },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleStrip, ..Default::default() },
            depth_stencil: Some(depth_state(false)),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &sprites,
                entry_point: Some("fs_sprite"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: HDR_FORMAT,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
                        alpha: wgpu::BlendComponent::OVER,
                    }),
                    write_mask: wgpu::ColorWrites::COLOR,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let additive = wgpu::BlendState {
            color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
            alpha: wgpu::BlendComponent::OVER,
        };
        // Exposure adaptation: new = metered * k + previous * (1 - k), with k
        // the blend constant set per frame.
        let adapt = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Constant, dst_factor: wgpu::BlendFactor::OneMinusConstant, operation: wgpu::BlendOperation::Add };
        let adapt = wgpu::BlendState { color: adapt, alpha: adapt };
        let post_pipeline = |entry: &str, format: wgpu::TextureFormat, blend: Option<wgpu::BlendState>| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&post_layout),
                vertex: wgpu::VertexState { module: &post, entry_point: Some("vs_fullscreen"), compilation_options: Default::default(), buffers: &[] },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &post,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format, blend, write_mask: wgpu::ColorWrites::ALL })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let pipelines = Pipelines {
            opaque,
            double_sided,
            skinned,
            blend,
            skinned_blend,
            prepass,
            shadow: shadow_static,
            shadow_skinned,
            shadow_masked,
            shadow_skinned_masked,
            gtao: gtao_pipeline,
            ao_blur: ao_blur_pipeline,
            ao_upsample: ao_upsample_pipeline,
            taa: taa_pipeline,
            sky: sky_pipeline,
            lines: lines_pipeline,
            down: post_pipeline("fs_downsample", HDR_FORMAT, None),
            up: post_pipeline("fs_upsample", HDR_FORMAT, Some(additive)),
            meter: post_pipeline("fs_meter", HDR_FORMAT, Some(adapt)),
            tonemap: post_pipeline("fs_tonemap", wgpu::TextureFormat::Rgba8UnormSrgb, None),
            fxaa: post_pipeline("fs_fxaa", output_format, None),
            sprites: sprites_pipeline,
            cluster: compute(&cluster, &cluster_layout, "cs_cluster"),
            exposure_begin: compute(&exposure, &exposure_layout, "cs_begin"),
            exposure_hist: compute(&exposure, &exposure_layout, "cs_histogram"),
            exposure_adapt: compute(&exposure, &exposure_layout, "cs_adapt"),
        };

        let shadow_size = 2048;
        let shadow_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("shadow"),
            size: wgpu::Extent3d { width: shadow_size, height: shadow_size, depth_or_array_layers: 4 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: SHADOW_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let shadow_view = shadow_tex.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
        let shadow_layers = (0..4)
            .map(|i| {
                shadow_tex.create_view(&wgpu::TextureViewDescriptor {
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    base_array_layer: i,
                    array_layer_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();
        let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("shadow"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        let linear_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("linear"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 16,
            ..Default::default()
        });
        let clamp_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("clamp"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let frame_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("frame"),
            size: std::mem::size_of::<FrameUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let env_view = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("env-none"),
                size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: HDR_FORMAT,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&Default::default());
        // Split-sum DFG LUT (A, B) for image-based specular and its
        // multiple-scattering compensation (see `brdf`).
        let dfg_view = {
            let n = brdf::DFG_SIZE as u32;
            let tex = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("dfg"),
                size: wgpu::Extent3d { width: n, height: n, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: HDR_FORMAT,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                bytemuck::cast_slice(&brdf::dfg_lut()),
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(n * 8), rows_per_image: Some(n) },
                wgpu::Extent3d { width: n, height: n, depth_or_array_layers: 1 },
            );
            tex.create_view(&Default::default())
        };
        // Shadow maps of up to four spot lights.
        let spot_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("spot-shadow"),
            size: wgpu::Extent3d { width: SPOT_SHADOW_SIZE, height: SPOT_SHADOW_SIZE, depth_or_array_layers: lights::MAX_SPOT_SHADOWS as u32 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: SHADOW_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let spot_shadow_view = spot_tex.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
        let spot_shadow_layers = (0..lights::MAX_SPOT_SHADOWS as u32)
            .map(|i| spot_tex.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2), base_array_layer: i, array_layer_count: Some(1), ..Default::default() }))
            .collect();
        let lights_cap = 1024 * std::mem::size_of::<lights::GpuLight>() as u64;
        let lights_buf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("lights"), size: lights_cap, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let cluster_buf = cluster_buffer(&device, 1280, 720);
        // Pre-exposure 1 and the default manual exposure until the first frame.
        let exposure_state = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("exposure-state"),
            contents: bytemuck::cast_slice(&[1.0f32, 1.0, 0.55, 0.0]),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });
        let exposure_hist = device.create_buffer(&wgpu::BufferDescriptor { label: Some("exposure-hist"), size: 256, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let exposure_uniform = device.create_buffer(&wgpu::BufferDescriptor { label: Some("exposure"), size: std::mem::size_of::<ExposureUniforms>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let frame_bind = make_frame_bind(&device, &frame_bgl, &frame_buf, &shadow_view, &shadow_sampler, &env_view, &linear_sampler, &lights_buf, &cluster_buf, &exposure_state, &spot_shadow_view, &dfg_view);
        let cluster_bind = make_cluster_bind(&device, &cluster_bgl, &frame_buf, &lights_buf, &cluster_buf);
        let cascade_bufs: Vec<wgpu::Buffer> = (0..4 + lights::MAX_SPOT_SHADOWS)
            .map(|_| device.create_buffer(&wgpu::BufferDescriptor { label: Some("cascade"), size: 64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }))
            .collect();
        let cascade_binds = cascade_bufs
            .iter()
            .map(|b| device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &cascade_bgl, entries: &[wgpu::BindGroupEntry { binding: 0, resource: b.as_entire_binding() }] }))
            .collect();
        let instance_cap = 4096 * std::mem::size_of::<InstanceRaw>() as u64;
        let instance_buf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("instances"), size: instance_cap, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let joint_cap = 1024 * 64;
        let joint_buf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("joints"), size: joint_cap, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let joint_bind = device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("joints"), layout: &joints_bgl, entries: &[wgpu::BindGroupEntry { binding: 0, resource: joint_buf.as_entire_binding() }] });
        let line_cap = 65536 * std::mem::size_of::<LineVertex>() as u64;
        let line_buf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("lines"), size: line_cap, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let taa_buf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("taa"), size: 16, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let quality_buf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("quality"), size: std::mem::size_of::<QualityUniforms>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let gpu_profiler = profiler::GpuProfiler::new(&device, &queue);
        let fog = fog::Fog::new(&device, &queue, &frame_bgl);
        let motion = motion::MotionBlur::new(&device, &frame_bgl, HDR_FORMAT);
        let camera_fx = look::CameraFx::new(&device, output_format, &frame_bgl, &taa_bgl);
        let mut r = Self {
            device,
            queue,
            adapter_info,
            adapter: None,
            output_format,
            pipelines,
            frame_bgl,
            material_bgl,
            joints_bgl,
            post_bgl,
            screen_bgl,
            gtao_bgl,
            taa_bgl,
            cluster_bgl,
            exposure_bgl,
            taa_buf,
            frame_buf,
            frame_bind,
            shadow_view,
            shadow_layers,
            cascade_bufs,
            cascade_binds,
            shadow_size,
            linear_sampler,
            clamp_sampler,
            shadow_sampler,
            env_view,
            dfg_view,
            env: None,
            meshes: Vec::new(),
            vertex_pool: mesh_pool::MeshPool::new("vertex pool", wgpu::BufferUsages::VERTEX, std::mem::size_of::<Vertex>() as u32, 64 << 20),
            index_pool: mesh_pool::MeshPool::new("index pool", wgpu::BufferUsages::INDEX, 4, 32 << 20),
            texture_cache: Default::default(),
            default_textures: None,
            materials: Vec::new(),
            models: Vec::new(),
            instance_buf,
            instance_cap,
            joint_buf,
            joint_cap,
            joint_bind,
            line_buf,
            line_cap,
            lights_buf,
            lights_cap,
            cluster_buf,
            cluster_grid: lights::cluster_grid(1280, 720),
            cluster_bind,
            exposure_state,
            exposure_hist,
            exposure_uniform,
            spot_shadow_view,
            spot_shadow_layers,
            exposure_instant: false,
            auto_exposure_prev: false,
            prev_time: 0.0,
            targets: None,
            exposure_time: None,
            camera_fx,
            stats: RenderStats::default(),
            settings: RenderSettings::from_env(),
            temporal: TemporalState::default(),
            quality_buf,
            profiler: gpu_profiler,
            cpu: Default::default(),
            statics: StaticScene::default(),
            cascade_cache: Default::default(),
            shadow_sun: Vec3::Y,
            texture_bytes: 0,
            mesh_bytes: 0,
            target_bytes: 0,
            pending_upload: 0,
            unflushed_upload: 0,
            upload_waits: (0, 0.0),
            upload_submits: 0,
            scratch: Vec::new(),
            view_lists: Vec::new(),
            indirect_args: Vec::new(),
            writes: FrameWrites::default(),
            indirect_buf: None,
            indirect_cap: 0,
            last_batches: Default::default(),
            sky_bgl,
            fog,
            motion,
            prev_motion: Default::default(),
            sky_bind: None,
        };
        r.resize(1280, 720);
        // `FORJA_PRESET=low|medium|high|ultra`: a quality preset for tools
        // and examples (the game reads data/graphics.toml instead). Explicit
        // `FORJA_FX` toggles still win.
        if let Some(p) = std::env::var("FORJA_PRESET").ok().and_then(|v| Preset::parse(&v)) {
            r.apply_graphics(&GraphicsConfig::for_preset(p));
            if let Ok(fx) = std::env::var("FORJA_FX") {
                r.settings.apply_fx(&fx);
            }
        }
        r
    }

    /// Drops every uploaded scene (meshes, materials, textures, the static
    /// set) and keeps the device objects: pipelines, render targets,
    /// shadow maps and settings. Replacing a world this way costs only the
    /// new upload, not a recompile of every shader (seconds with FXC).
    /// Every [`ModelHandle`] becomes invalid.
    pub fn clear_scene(&mut self) {
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        self.meshes = Vec::new();
        self.materials = Vec::new();
        self.models = Vec::new();
        self.texture_cache = Default::default();
        self.vertex_pool = mesh_pool::MeshPool::new("vertex pool", wgpu::BufferUsages::VERTEX, std::mem::size_of::<Vertex>() as u32, 64 << 20);
        self.index_pool = mesh_pool::MeshPool::new("index pool", wgpu::BufferUsages::INDEX, 4, 32 << 20);
        self.statics = StaticScene::default();
        self.scratch = Vec::new();
        self.last_batches = Default::default();
        self.mesh_bytes = 0;
        self.texture_bytes = 0;
        self.pending_upload = 0;
        self.unflushed_upload = 0;
        self.env = None;
        self.reset_history();
    }

    /// Forgets the temporal history (camera cut, new scene): the next frame
    /// starts TAA accumulation from scratch.
    pub fn reset_history(&mut self) {
        self.cascade_cache = Default::default();
        self.fog.reset_history();
        self.prev_motion.clear();
        self.temporal.history_valid = false;
        self.temporal.still_frames = 0;
        self.temporal.prev_key = None;
    }

    /// Jitter, reprojection matrices and TAA weights for this frame; also
    /// advances the temporal state.
    fn temporal_frame(&mut self, cam: &Camera, time: f32, width: u32, height: u32) -> TemporalFrame {
        let t = &mut self.temporal;
        let taa = self.settings.taa;
        let unjittered = cam.view_proj();
        let q = cam.rotation;
        let key = [cam.position.x, cam.position.y, cam.position.z, q.x, q.y, q.z, q.w, cam.fov_y, time, width as f32, height as f32];
        let still = t.history_valid && t.prev_key == Some(key);
        let restart = std::mem::take(&mut t.restart);
        t.still_frames = if still && !restart { t.still_frames + 1 } else { 0 };
        t.prev_key = Some(key);
        t.frame_index = t.frame_index.wrapping_add(1);
        // Halton(2,3), 16-frame cycle, in NDC units.
        let i = t.frame_index % 16 + 1;
        let jitter = if taa {
            forja_math::Vec2::new((halton(i, 2) - 0.5) * 2.0 / width as f32, (halton(i, 3) - 0.5) * 2.0 / height as f32)
        } else {
            forja_math::Vec2::ZERO
        };
        let view_proj = Mat4::from_translation(Vec3::new(jitter.x, jitter.y, 0.0)) * unjittered;
        let history_valid = t.history_valid;
        let prev_view_proj = if history_valid { t.prev_view_proj } else { unjittered };
        // Still camera: running average (converges); moving: exponential
        // 10% with tight variance clipping.
        let (alpha, gamma) = if !taa || !history_valid || restart {
            (1.0, 1.0)
        } else if still {
            ((1.0 / (t.still_frames + 1) as f32).max(1.0 / 24.0), 2.5)
        } else {
            (0.1, 1.0)
        };
        t.prev_view_proj = unjittered;
        t.history_valid = true;
        TemporalFrame {
            view_proj,
            prev_view_proj,
            jitter,
            frame_index: t.frame_index,
            history_valid,
            taa_params: [alpha, gamma, (taa && history_valid) as u32 as f32, still as u32 as f32],
        }
    }

    /// Uses an HDR photo for sky, reflections and ambient light (`None`
    /// returns to the analytic atmosphere).
    pub fn set_environment(&mut self, env: Option<&env::EnvMap>) {
        let Some(e) = env else {
            self.env = None;
            return;
        };
        let (w, h, _) = e.mips[0];
        let tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("env"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: e.mips.len() as u32,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: HDR_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        for (level, (mw, mh, data)) in e.mips.iter().enumerate() {
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: level as u32, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                bytemuck::cast_slice(data),
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(mw * 8), rows_per_image: Some(*mh) },
                wgpu::Extent3d { width: *mw, height: *mh, depth_or_array_layers: 1 },
            );
        }
        self.env_view = tex.create_view(&Default::default());
        self.rebuild_frame_bind();
        let horizon = (0..8).map(|i| {
            let a = i as f32 / 8.0 * std::f32::consts::TAU;
            e.irradiance(Vec3::new(a.cos(), 0.0, a.sin()))
        }).sum::<Vec3>() / 8.0;
        self.env = Some(EnvInfo {
            sun_dir: e.sun_dir,
            sun_color: e.sun_color,
            sh: e.sh,
            max_mip: (e.mips.len() - 1) as f32,
            prefiltered: e.prefiltered,
            zenith: e.irradiance(Vec3::Y),
            horizon,
            ground: e.irradiance(Vec3::NEG_Y),
            sun_top: {
                let t = atmosphere::sun_transmittance(Vec3::new(0.0, 6360.001, 0.0), e.sun_dir).dot(Vec3::new(0.2126, 0.7152, 0.0722));
                if t > 1e-3 { e.sun_color.dot(Vec3::new(0.2126, 0.7152, 0.0722)) / t } else { 0.0 }
            },
        });
    }

    /// Sun direction of the loaded HDRI, if any.
    pub fn environment_sun(&self) -> Option<Vec3> {
        self.env.as_ref().map(|e| e.sun_dir)
    }

    pub fn size(&self) -> (u32, u32) {
        self.targets.as_ref().map(|t| (t.width, t.height)).unwrap_or((0, 0))
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        let (width, height) = (width.max(1), height.max(1));
        if self.targets.as_ref().is_some_and(|t| t.width == width && t.height == height) {
            return;
        }
        let raw_tex = |label: &str, w: u32, h: u32, format: wgpu::TextureFormat, extra: wgpu::TextureUsages| {
            self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | extra,
                view_formats: &[],
            })
        };
        let tex = |label: &str, w: u32, h: u32, format: wgpu::TextureFormat| raw_tex(label, w, h, format, wgpu::TextureUsages::empty()).create_view(&Default::default());
        let hdr = tex("hdr", width, height, HDR_FORMAT);
        let depth = tex("depth", width, height, DEPTH_FORMAT);
        let normal = tex("normal", width, height, NORMAL_FORMAT);
        let velocity = tex("velocity", width, height, motion::VELOCITY_FORMAT);
        let ao_raw = tex("ao_raw", width, height, AO_FORMAT);
        let ao = tex("ao", width, height, AO_FORMAT);
        let (hw, hh) = (width.div_ceil(2), height.div_ceil(2));
        let ao_half_raw = tex("ao_half_raw", hw, hh, AO_FORMAT);
        let ao_half = tex("ao_half", hw, hh, AO_FORMAT);
        let resolved_tex = raw_tex("resolved", width, height, HDR_FORMAT, wgpu::TextureUsages::COPY_SRC);
        let resolved = resolved_tex.create_view(&Default::default());
        let history_tex = raw_tex("history", width, height, HDR_FORMAT, wgpu::TextureUsages::COPY_DST);
        let history = history_tex.create_view(&Default::default());
        let ldr = tex("ldr", width, height, wgpu::TextureFormat::Rgba8UnormSrgb);
        let entry = view_entry;
        let quality = || wgpu::BindGroupEntry { binding: 3, resource: self.quality_buf.as_entire_binding() };
        let gtao_group = |label: &str, aux: &wgpu::TextureView| self.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some(label), layout: &self.gtao_bgl, entries: &[entry(0, &depth), entry(1, &normal), entry(2, aux), quality()] });
        // `aux` is the texture read: the GTAO pass reads none (any view that
        // is not its target), the denoise the raw AO, the upsample the
        // denoised half-res AO.
        let gtao_bind = gtao_group("gtao", &ao);
        let blur_bind = gtao_group("ao_blur", &ao_raw);
        let gtao_half_bind = gtao_group("gtao_half", &ao_half);
        let blur_half_bind = gtao_group("ao_blur_half", &ao_half_raw);
        let upsample_bind = gtao_group("ao_upsample", &ao_half);
        let taa_bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("taa"),
            layout: &self.taa_bgl,
            entries: &[
                entry(0, &hdr),
                entry(1, &history),
                entry(2, &depth),
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&self.clamp_sampler) },
                wgpu::BindGroupEntry { binding: 4, resource: self.taa_buf.as_entire_binding() },
                entry(5, &velocity),
            ],
        });
        let screen_bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("screen"),
            layout: &self.screen_bgl,
            entries: &[
                entry(0, &ao),
                entry(1, &depth),
                entry(2, &history),
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&self.clamp_sampler) },
                entry(4, &normal),
                wgpu::BindGroupEntry { binding: 5, resource: self.quality_buf.as_entire_binding() },
                entry(6, self.fog.apply_view()),
                wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::Sampler(&self.fog.sampler) },
            ],
        });
        self.temporal.history_valid = false;
        // Render target memory: hdr, normal, resolved, history (8 B/px),
        // depth (4), ldr (4), ao x2 (1), half ao x2, bloom chain (~1/3 hdr).
        let px = width as u64 * height as u64;
        // Plus velocity (4), RGBA8 bent-normal AO (x4) and the motion tiles.
        self.target_bytes = px * (8 * 4 + 4 + 4 + 2) + px / 2 + px * 8 / 3 + px * 4 + px * 6 + self.motion.target_bytes();
        let mut bloom = Vec::new();
        let (mut w, mut h) = (width, height);
        for _ in 0..BLOOM_MIPS {
            w = (w / 2).max(1);
            h = (h / 2).max(1);
            bloom.push((tex("bloom", w, h, HDR_FORMAT), w, h));
        }
        let mk_uniform = |texel: [f32; 2], params: [f32; 4]| {
            self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("post"),
                contents: bytemuck::bytes_of(&PostUniforms { texel: [texel[0], texel[1], 0.0, 0.0], params, ..Default::default() }),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            })
        };
        let bind = |src: &wgpu::TextureView, u: &wgpu::Buffer, extra: &wgpu::TextureView, exposure: &wgpu::TextureView| {
            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("post"),
                layout: &self.post_bgl,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(src) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.clamp_sampler) },
                    wgpu::BindGroupEntry { binding: 2, resource: u.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(extra) },
                    wgpu::BindGroupEntry { binding: 4, resource: self.exposure_state.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(exposure) },
                ],
            })
        };
        // Passes that render into bloom mips must not also bind mip 0.
        let dummy = tex("dummy", 1, 1, HDR_FORMAT);
        let exposure = tex("exposure", 1, 1, HDR_FORMAT);
        // A new exposure texture has no history: the next frame snaps.
        self.exposure_time = None;
        let mut uniforms = Vec::new();
        let mut down = Vec::new();
        for i in 0..BLOOM_MIPS {
            let (src, sw, sh) = if i == 0 { (&resolved, width, height) } else { (&bloom[i - 1].0, bloom[i - 1].1, bloom[i - 1].2) };
            let u = mk_uniform([1.0 / sw as f32, 1.0 / sh as f32], [0.0, 0.0, 0.0, if i == 0 { 1.0 } else { 0.0 }]);
            down.push(bind(src, &u, &dummy, &dummy));
            uniforms.push(u);
        }
        let mut up = Vec::new();
        for i in (1..BLOOM_MIPS).rev() {
            let (src, sw, sh) = (&bloom[i].0, bloom[i].1, bloom[i].2);
            let u = mk_uniform([1.0 / sw as f32, 1.0 / sh as f32], [0.0; 4]);
            up.push(bind(src, &u, &dummy, &dummy));
            uniforms.push(u);
        }
        // Tonemap uniform is rewritten each frame (exposure, bloom, vignette).
        let tu = mk_uniform([1.0 / width as f32, 1.0 / height as f32], [1.0, 0.04, 0.3, 0.0]);
        let tonemap = bind(&resolved, &tu, &bloom[0].0, &exposure);
        let dof_tonemap = bind(&hdr, &tu, &bloom[0].0, &exposure);
        let dof_down = bind(&hdr, &uniforms[0], &dummy, &dummy);
        let camera = self.camera_fx.targets(&self.device, self.output_format, (width, height), &self.taa_bgl, &resolved, &depth, &velocity);
        uniforms.push(tu);
        // Final pass uniform is rewritten each frame (FXAA or sharpening).
        let fu = mk_uniform([1.0 / width as f32, 1.0 / height as f32], [1.0, 0.0, 0.0, 0.0]);
        let fxaa = bind(&ldr, &fu, &dummy, &dummy);
        uniforms.push(fu);
        // The meter reads the smallest bloom mip (written by the down chain
        // only) and the per-frame metering uniform.
        let meter_uniform = mk_uniform([0.0; 2], [0.0; 4]);
        let meter = bind(&bloom[BLOOM_MIPS - 1].0, &meter_uniform, &dummy, &dummy);
        let exposure_bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("exposure"),
            layout: &self.exposure_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.exposure_uniform.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: self.exposure_state.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: self.exposure_hist.as_entire_binding() },
                entry(3, &resolved),
            ],
        });
        self.motion.resize(&self.device, (width, height), &history, &depth, &velocity);
        self.targets = Some(Targets {
            width,
            height,
            hdr,
            depth,
            normal,
            ao_raw,
            ao,
            ao_half_raw,
            ao_half,
            gtao_half_bind,
            blur_half_bind,
            upsample_bind,
            resolved_tex,
            resolved,
            history_tex,
            ldr,
            bloom,
            exposure,
            post_bind: PostBinds { down, up, tonemap, fxaa, meter, meter_uniform, uniforms },
            gtao_bind,
            blur_bind,
            taa_bind,
            screen_bind,
            camera,
            dof_down,
            dof_tonemap,
            exposure_bind,
            velocity,
            history,
        });
        let grid = lights::cluster_grid(width, height);
        if grid != self.cluster_grid {
            self.cluster_grid = grid;
            self.cluster_buf = cluster_buffer(&self.device, width, height);
            self.rebuild_frame_bind();
        }
    }

    /// Main pass screen inputs (rebuilt when the fog volume changes).
    fn make_screen_bind(&self, ao: &wgpu::TextureView, depth: &wgpu::TextureView, history: &wgpu::TextureView, normal: &wgpu::TextureView) -> wgpu::BindGroup {
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("screen"),
            layout: &self.screen_bgl,
            entries: &[
                view_entry(0, ao),
                view_entry(1, depth),
                view_entry(2, history),
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&self.clamp_sampler) },
                view_entry(4, normal),
                wgpu::BindGroupEntry { binding: 5, resource: self.quality_buf.as_entire_binding() },
                view_entry(6, self.fog.apply_view()),
                wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::Sampler(&self.fog.sampler) },
            ],
        })
    }

    fn make_sky_bind(&self) -> wgpu::BindGroup {
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sky"),
            layout: &self.sky_bgl,
            entries: &[
                view_entry(0, &self.fog.skyview),
                view_entry(1, &self.fog.transmittance),
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.fog.sampler) },
                view_entry(3, self.fog.apply_view()),
            ],
        })
    }

    /// Recreates the bind groups that reference the frame's buffers (after
    /// a new environment map, cluster grid or a larger light buffer).
    fn rebuild_frame_bind(&mut self) {
        self.frame_bind = make_frame_bind(
            &self.device,
            &self.frame_bgl,
            &self.frame_buf,
            &self.shadow_view,
            &self.shadow_sampler,
            &self.env_view,
            &self.linear_sampler,
            &self.lights_buf,
            &self.cluster_buf,
            &self.exposure_state,
            &self.spot_shadow_view,
            &self.dfg_view,
        );
        self.cluster_bind = make_cluster_bind(&self.device, &self.cluster_bgl, &self.frame_buf, &self.lights_buf, &self.cluster_buf);
    }

    // ---------------------------------------------------------------- upload

    /// Uploads a texture once per shared pixel buffer.
    fn cached_texture(&mut self, t: &TextureData, alpha_cutoff: Option<f32>) -> wgpu::TextureView {
        let key = (t.rgba8.as_ptr() as usize, t.srgb);
        if let Some((_, view)) = self.texture_cache.get(&key) {
            return view.clone();
        }
        let (view, bytes) = Self::upload_texture_with(&self.device, &self.queue, t, alpha_cutoff, self.settings.max_texture_size);
        self.texture_bytes += bytes;
        self.note_upload(bytes);
        self.texture_cache.insert(key, (t.rgba8.clone(), view.clone()));
        view
    }

    /// Whether textures are uploaded block-compressed.
    fn use_bc(&self) -> bool {
        self.settings.texture_compression && self.device.features().contains(wgpu::Features::TEXTURE_COMPRESSION_BC)
    }

    /// The CPU side of a texture upload (mip chain, BC encoding or the
    /// cached chain): pure, so a scene's textures are prepared in parallel.
    fn prepare_texture(t: &TextureData, alpha_cutoff: Option<f32>, max_size: u32, bc: bool, normal_map: bool) -> (wgpu::TextureFormat, Option<texcomp::Kind>, texcomp::Mips) {
        let cap = texture_cap(max_size, t.srgb).max(1);
        let rgba = if t.srgb { wgpu::TextureFormat::Rgba8UnormSrgb } else { wgpu::TextureFormat::Rgba8Unorm };
        let mut first = (t.width.max(1), t.height.max(1));
        while first.0.max(first.1) > cap && (first.0 > 1 || first.1 > 1) {
            first = ((first.0 / 2).max(1), (first.1 / 2).max(1));
        }
        let kind = if bc { texcomp::kind_for(t, normal_map, first) } else { None };
        if let Some(k) = kind
            && let Some(m) = texcomp::load(t, k, alpha_cutoff, cap)
        {
            return (k.format(t.srgb), Some(k), m);
        }
        let mut mips = generate_mips(t, alpha_cutoff);
        while mips.len() > 1 && mips[0].0.max(mips[0].1) > cap {
            mips.remove(0);
        }
        match kind {
            Some(k) => {
                let m = texcomp::encode(t, k, alpha_cutoff, cap, &mips);
                (k.format(t.srgb), Some(k), m)
            }
            None => (rgba, None, mips),
        }
    }

    /// Creates the GPU texture of a prepared chain; returns the view and
    /// its bytes.
    fn create_prepared(device: &wgpu::Device, queue: &wgpu::Queue, name: &str, format: wgpu::TextureFormat, kind: Option<texcomp::Kind>, mips: &texcomp::Mips) -> (wgpu::TextureView, u64) {
        let bytes = mips.iter().map(|m| m.2.len() as u64).sum();
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(name),
            size: wgpu::Extent3d { width: mips[0].0, height: mips[0].1, depth_or_array_layers: 1 },
            mip_level_count: mips.len() as u32,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        for (level, (w, h, data)) in mips.iter().enumerate() {
            let (row, rows, extent) = match kind {
                // Compressed levels are copied at their physical size
                // (whole 4x4 blocks, also below 4 texels).
                Some(k) => (w.div_ceil(4) * k.block_bytes(), h.div_ceil(4), wgpu::Extent3d { width: w.div_ceil(4) * 4, height: h.div_ceil(4) * 4, depth_or_array_layers: 1 }),
                None => (4 * w, *h, wgpu::Extent3d { width: *w, height: *h, depth_or_array_layers: 1 }),
            };
            queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: level as u32, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                data,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(rows) },
                extent,
            );
        }
        (tex.create_view(&Default::default()), bytes)
    }

    /// Uploads the textures of `scene` that are not on the GPU yet,
    /// preparing them (mips, BC encoding or cached chains) in parallel.
    fn upload_scene_textures(&mut self, scene: &SceneData, cutoffs: &[Option<f32>]) {
        use rayon::prelude::*;
        let mut normal = vec![false; scene.textures.len()];
        for m in &scene.materials {
            if let Some(i) = m.normal_tex
                && i < normal.len()
            {
                normal[i] = true;
            }
        }
        let mut todo: Vec<usize> = Vec::new();
        for (i, t) in scene.textures.iter().enumerate() {
            let key = (t.rgba8.as_ptr() as usize, t.srgb);
            if !self.texture_cache.contains_key(&key) && !todo.iter().any(|&j| (scene.textures[j].rgba8.as_ptr() as usize, scene.textures[j].srgb) == key) {
                todo.push(i);
            }
        }
        if todo.is_empty() {
            return;
        }
        let (bc, max) = (self.use_bc(), self.settings.max_texture_size);
        // A few at a time: a full RGBA mip chain of a 2048² map is 21 MB.
        for chunk in todo.chunks(rayon::current_num_threads().clamp(2, 8)) {
            let prepared: Vec<_> = forja_assets::threads::on_frame_pool(|| chunk.par_iter().map(|&i| Self::prepare_texture(&scene.textures[i], cutoffs[i], max, bc, normal[i])).collect());
            for (&i, (format, kind, mips)) in chunk.iter().zip(prepared) {
                let t = &scene.textures[i];
                let (view, bytes) = Self::create_prepared(&self.device, &self.queue, &t.name, format, kind, &mips);
                self.texture_bytes += bytes;
                self.note_upload(bytes);
                self.texture_cache.insert((t.rgba8.as_ptr() as usize, t.srgb), (t.rgba8.clone(), view));
            }
        }
    }

    /// Uploads `t` with its mip chain (alpha coverage preserved when
    /// `alpha_cutoff` is set), starting at the first level not larger than
    /// `max_size` (the quality preset's texture cap). Returns the view and
    /// the bytes of GPU memory used.
    fn upload_texture_with(device: &wgpu::Device, queue: &wgpu::Queue, t: &TextureData, alpha_cutoff: Option<f32>, max_size: u32) -> (wgpu::TextureView, u64) {
        let format = if t.srgb { wgpu::TextureFormat::Rgba8UnormSrgb } else { wgpu::TextureFormat::Rgba8Unorm };
        let max_size = texture_cap(max_size, t.srgb);
        let mut mips = generate_mips(t, alpha_cutoff);
        while mips.len() > 1 && mips[0].0.max(mips[0].1) > max_size.max(1) {
            mips.remove(0);
        }
        let bytes = mips.iter().map(|m| m.2.len() as u64).sum();
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(&t.name),
            size: wgpu::Extent3d { width: mips[0].0, height: mips[0].1, depth_or_array_layers: 1 },
            mip_level_count: mips.len() as u32,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        for (level, (w, h, data)) in mips.iter().enumerate() {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: level as u32, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                data,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * w), rows_per_image: Some(*h) },
                wgpu::Extent3d { width: *w, height: *h, depth_or_array_layers: 1 },
            );
        }
        (tex.create_view(&Default::default()), bytes)
    }

    fn solid_view(device: &wgpu::Device, queue: &wgpu::Queue, rgba: [u8; 4], srgb: bool) -> wgpu::TextureView {
        Self::upload_texture_with(device, queue, &TextureData { name: "solid".into(), width: 1, height: 1, rgba8: rgba.to_vec().into(), srgb }, None, 1).0
    }

    fn upload_material(&self, m: &MaterialData, textures: &[wgpu::TextureView], defaults: &[wgpu::TextureView; 3]) -> GpuMaterial {
        let [white_srgb, white_linear, flat_normal] = defaults;
        let pick = |i: Option<usize>, fallback: &'_ wgpu::TextureView| -> wgpu::TextureView {
            i.and_then(|i| textures.get(i)).cloned().unwrap_or_else(|| fallback.clone())
        };
        let uniforms = MaterialUniforms {
            base_color: m.base_color.to_array(),
            emissive: [m.emissive.x, m.emissive.y, m.emissive.z, m.normal_scale],
            params: [
                m.metallic,
                m.roughness,
                match m.alpha {
                    AlphaMode::Mask(c) => c as f32 / 255.0,
                    _ => 0.0,
                },
                m.unlit as u32 as f32,
            ],
            layers: [m.clearcoat, m.clearcoat_roughness, m.transmission, m.subsurface],
            foliage: match m.foliage {
                Some(f) => [f.translucency, f.backface_cull_v, 1.0, 0.0],
                None => [0.0, 2.0, 0.0, 0.0],
            },
            extra: [m.specular, 0.0, 0.0, 0.0],
        };
        let buf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some(&m.name), contents: bytemuck::bytes_of(&uniforms), usage: wgpu::BufferUsages::UNIFORM });
        let base = pick(m.base_color_tex, white_srgb);
        let normal = pick(m.normal_tex, flat_normal);
        let mr = pick(m.metallic_roughness_tex, white_linear);
        let emissive = pick(m.emissive_tex, white_srgb);
        let occlusion = pick(m.occlusion_tex, white_linear);
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(&m.name),
            layout: &self.material_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&base) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&normal) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&mr) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&emissive) },
                wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(&occlusion) },
                wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::Sampler(&self.linear_sampler) },
            ],
        });
        let blend = m.alpha == AlphaMode::Blend || m.transmission > 0.0;
        GpuMaterial { bind_group, blend, masked: !blend && matches!(m.alpha, AlphaMode::Mask(_)), double_sided: m.double_sided }
    }

    /// Uploads a scene once; draw it with [`DrawInstance`]s.
    pub fn upload_scene(&mut self, scene: &SceneData) -> ModelHandle {
        // Base colour maps of alpha-tested materials get coverage-preserving
        // mips so foliage and hair cards do not thin out with distance.
        let mut cutoffs: Vec<Option<f32>> = vec![None; scene.textures.len()];
        for m in &scene.materials {
            if let (AlphaMode::Mask(c), Some(i)) = (m.alpha, m.base_color_tex)
                && i < cutoffs.len()
            {
                cutoffs[i] = Some(c as f32 / 255.0);
            }
        }
        self.upload_scene_textures(scene, &cutoffs);
        let textures: Vec<wgpu::TextureView> = scene.textures.iter().zip(cutoffs).map(|(t, c)| self.cached_texture(t, c)).collect();
        let defaults = self.default_textures.get_or_insert_with(|| [Self::solid_view(&self.device, &self.queue, [255; 4], true), Self::solid_view(&self.device, &self.queue, [255; 4], false), Self::solid_view(&self.device, &self.queue, [128, 128, 255, 255], false)]).clone();
        let mat_base = self.materials.len() as u32;
        for m in &scene.materials {
            let gm = self.upload_material(m, &textures, &defaults);
            self.materials.push(gm);
        }
        if scene.materials.is_empty() {
            let gm = self.upload_material(&MaterialData::default(), &textures, &defaults);
            self.materials.push(gm);
        }
        let name: Arc<str> = scene.name.as_str().into();
        let mesh_base = self.meshes.len() as u32;
        let mut mesh_ids: Vec<Vec<u32>> = Vec::new();
        for prims in &scene.meshes {
            let mut ids = Vec::new();
            for p in prims {
                let verts: Vec<Vertex> = (0..p.positions.len())
                    .map(|i| Vertex { pos: p.positions[i].to_array(), normal: p.normals[i].to_array(), uv: p.uvs[i].to_array(), tangent: p.tangents[i].to_array() })
                    .collect();
                let (vertex, index, lods) = self.pool_upload(&verts, &p.indices, &p.lods, p.is_skinned());
                let bytes = (verts.len() * std::mem::size_of::<Vertex>() + p.indices.len() * 4 + p.lods.iter().map(|l| l.len() * 4).sum::<usize>()) as u64;
                self.mesh_bytes += bytes;
                self.note_upload(bytes);
                let skin = p.is_skinned().then(|| {
                    let sv: Vec<SkinVertex> = (0..p.positions.len())
                        .map(|i| {
                            let w = p.weights.get(i).copied().unwrap_or([1.0, 0.0, 0.0, 0.0]);
                            let sum = (w[0] + w[1] + w[2] + w[3]).max(1e-6);
                            SkinVertex { joints: p.joints[i], weights: [w[0] / sum, w[1] / sum, w[2] / sum, w[3] / sum] }
                        })
                        .collect();
                    self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("skin"), contents: bytemuck::cast_slice(&sv), usage: wgpu::BufferUsages::VERTEX })
                });
                let material = mat_base + if scene.materials.is_empty() { 0 } else { p.material as u32 };
                ids.push(self.meshes.len() as u32);
                self.meshes.push(GpuMesh { vertex, skin, index, index_count: p.indices.len() as u32, lods, material, aabb: p.aabb, lod_tris: None, lod_bounds: p.aabb, name: name.clone(), shadow: p.shadow });
            }
            // LOD groups: primitives with `lod_sync` and the same number of
            // levels share one triangle table and one bounding box.
            let synced: Vec<(usize, u32)> = prims.iter().zip(&ids).filter(|(p, _)| p.lod_sync && !p.lods.is_empty()).map(|(p, id)| (p.lods.len(), *id)).collect();
            if let Some(&(levels, _)) = synced.first() {
                let group: Vec<u32> = synced.iter().filter(|(l, _)| *l == levels).map(|(_, id)| *id).collect();
                if group.len() > 1 {
                    let tris: Arc<[u32]> = (0..=levels as u32).map(|l| group.iter().map(|id| self.meshes[*id as usize].indices(l).1 / 3).sum()).collect();
                    let bounds = group.iter().fold(Aabb::EMPTY, |b, id| b.union(&self.meshes[*id as usize].aabb));
                    for id in &group {
                        let m = &mut self.meshes[*id as usize];
                        m.lod_tris = Some(tris.clone());
                        m.lod_bounds = bounds;
                    }
                }
            }
            mesh_ids.push(ids);
        }
        let _ = mesh_base;
        let rest = scene.world_matrices();
        let mut parts = Vec::new();
        for (ni, n) in scene.nodes.iter().enumerate() {
            if let Some(m) = n.mesh {
                for id in &mesh_ids[m] {
                    parts.push(ModelPart { mesh: *id, node: ni });
                }
            }
        }
        let handle = ModelHandle(self.models.len() as u32);
        let prims: Vec<u32> = mesh_ids.iter().flatten().copied().collect();
        let bytes = prims.iter().map(|&m| self.mesh_buffer_bytes(m)).sum();
        self.models.push(GpuModel { parts, rest, bounds: scene.bounds(), prims, resident: Some(bytes) });
        handle
    }

    fn mesh_buffer_bytes(&self, m: u32) -> u64 {
        let g = &self.meshes[m as usize];
        g.vertex.map_or(0, |a| a.units as u64 * self.vertex_pool.unit() as u64) + g.index.map_or(0, |a| a.units as u64 * 4) + g.skin.as_ref().map_or(0, |b| b.size())
    }

    /// Places a primitive's vertices and all its index levels (LOD 0
    /// first) in the shared pools: `(vertices, indices, LOD ranges)`.
    fn pool_upload(&mut self, verts: &[Vertex], indices: &[u32], lods: &[Vec<u32>], dedicated: bool) -> (Option<mesh_pool::Alloc>, Option<mesh_pool::Alloc>, Vec<(u32, u32)>) {
        let vertex = self.vertex_pool.alloc(&self.device, verts.len() as u32, dedicated);
        self.vertex_pool.write(&self.queue, vertex, bytemuck::cast_slice(verts));
        let total = indices.len() + lods.iter().map(|l| l.len()).sum::<usize>();
        let index = self.index_pool.alloc(&self.device, total as u32, false);
        let mut all: Vec<u32> = Vec::with_capacity(total);
        all.extend_from_slice(indices);
        let mut ranges = Vec::with_capacity(lods.len());
        for l in lods {
            ranges.push((index.first + all.len() as u32, l.len() as u32));
            all.extend_from_slice(l);
        }
        self.index_pool.write(&self.queue, index, bytemuck::cast_slice(&all));
        (Some(vertex), Some(index), ranges)
    }

    /// Frees the vertex and index buffers of a model (streaming: a city
    /// tile far away draws its HLOD instead). Materials and textures stay;
    /// the model must not be drawn until [`Renderer::reload_model`].
    /// Returns the bytes released.
    pub fn release_model(&mut self, h: ModelHandle) -> u64 {
        let Some(model) = self.models.get_mut(h.0 as usize) else { return 0 };
        let Some(bytes) = model.resident.take() else { return 0 };
        let prims = model.prims.clone();
        for m in prims {
            let g = &mut self.meshes[m as usize];
            if let Some(a) = g.vertex.take() {
                self.vertex_pool.free(a);
            }
            if let Some(a) = g.index.take() {
                self.index_pool.free(a);
            }
            g.index_count = 0;
            g.skin = None;
            g.lods.clear();
        }
        self.mesh_bytes = self.mesh_bytes.saturating_sub(bytes);
        bytes
    }

    /// Uploads the meshes of a released model again from `scene` (the
    /// scene it was uploaded from). Returns the bytes uploaded.
    pub fn reload_model(&mut self, h: ModelHandle, scene: &SceneData) -> u64 {
        self.reload_prepared(h, &MeshUpload::prepare(scene))
    }

    /// [`Renderer::reload_model`] from vertex data converted beforehand
    /// ([`MeshUpload::prepare`], e.g. on a loader thread): only copies
    /// into the shared pools (no GPU resource is created, nothing waits).
    pub fn reload_prepared(&mut self, h: ModelHandle, up: &MeshUpload) -> u64 {
        let Some(model) = self.models.get(h.0 as usize) else { return 0 };
        if model.resident.is_some() {
            return 0;
        }
        let prims = model.prims.clone();
        let mut bytes = 0;
        for (&m, p) in prims.iter().zip(&up.prims) {
            let Some(p) = p else { continue };
            let (vertex, index, lods) = self.pool_upload(&p.vertices, &p.indices, &p.lods, false);
            let g = &mut self.meshes[m as usize];
            g.vertex = vertex;
            g.index = index;
            g.index_count = p.indices.len() as u32;
            g.lods = lods;
            bytes += self.mesh_buffer_bytes(m);
        }
        self.mesh_bytes += bytes;
        self.note_upload(bytes);
        self.models[h.0 as usize].resident = Some(bytes);
        bytes
    }

    /// Whether a far-distance group's detail can be drawn (its streamed
    /// meshes are resident); a group that is not ready keeps its HLOD.
    pub fn set_static_group_ready(&mut self, group: u32, ready: bool) {
        self.statics.set_ready(group, ready);
    }

    /// Distance from `eye` to a far-distance group's bounds.
    pub fn static_group_distance(&self, group: u32, eye: Vec3) -> Option<f32> {
        self.statics.group_distance(group, eye)
    }

    /// Far-distance groups currently showing their detail.
    pub fn static_groups_near(&self) -> Vec<u32> {
        self.statics.near_groups()
    }

    /// Whether shadow view `c` (sun cascades 0-3, then spots) draws the
    /// far-cascade proxies instead of the near-only detail.
    fn proxy_view(c: usize, first: u32) -> bool {
        c < 4 && c as u32 >= first
    }

    /// Roles shadow view `c` does not draw ([`static_scene::role_bit`]).
    fn shadow_skip(c: usize, s: &RenderSettings) -> u8 {
        use static_scene::role_bit;
        let mut skip = role_bit(if Self::proxy_view(c, s.shadow_proxy_cascade) { ShadowRole::Near } else { ShadowRole::Proxy });
        if c >= 1 && c < 4 && (s.stable_shadows || Self::proxy_view(c, s.shadow_proxy_cascade)) {
            skip |= role_bit(ShadowRole::Fine);
        }
        skip
    }

    pub fn model_bounds(&self, m: ModelHandle) -> Aabb {
        self.models[m.0 as usize].bounds
    }

    // ---------------------------------------------------------------- static set

    /// Uploads the instances that never move (buildings, modules, street
    /// furniture, trees). They are bucketed into a culling grid once and
    /// drawn every frame without being resubmitted; [`FrameDesc::instances`]
    /// then only holds what moves. Replaces the previous static set
    /// (an empty slice clears it). Skinned instances are not allowed here.
    pub fn set_static_instances(&mut self, instances: &[DrawInstance]) {
        self.set_static_instances_lod(instances, &[]);
    }

    /// [`Self::set_static_instances`] with a far-distance group per
    /// instance (parallel to `instances`; missing entries: no group). A
    /// group draws its detail instances near the camera and its HLOD
    /// instances beyond [`RenderSettings::hlod_distance`].
    pub fn set_static_instances_lod(&mut self, instances: &[DrawInstance], lod: &[StaticLod]) {
        let t = std::time::Instant::now();
        let mut items = Vec::new();
        for (ii, inst) in instances.iter().enumerate() {
            let Some(model) = self.models.get(inst.model.0 as usize) else { continue };
            let tag = lod.get(ii).copied().unwrap_or_default();
            for part in &model.parts {
                let mesh = &self.meshes[part.mesh as usize];
                let node_m = inst.nodes.as_ref().and_then(|n| n.get(part.node).copied()).unwrap_or(model.rest[part.node]);
                let world = inst.transform * node_m;
                let mat = &self.materials[mesh.material as usize];
                let pipe = if mat.blend { 3 } else if mat.double_sided { 1 } else { 0 };
                // Synced LOD groups share their bounds, so every part of a
                // tree gets the same distance and the same level.
                let bounds = transform_aabb(&mesh.lod_bounds, &world);
                if bounds.is_empty() {
                    continue;
                }
                items.push(StaticItem {
                    key: DrawKey { pipe, material: mesh.material, mesh: part.mesh },
                    raw: InstanceRaw { model: world.to_cols_array_2d(), tint: inst.tint.to_array(), extra: [0, 0, 0, 0] },
                    bounds,
                    no_shadow: mat.blend || !inst.cast_shadows,
                    role: mesh.shadow,
                    group: tag.group,
                    hlod: tag.hlod,
                });
            }
        }
        self.statics = StaticScene::build(items);
        log::info!("static set: {} instances, {} parts in {} cells ({:.1} ms)", instances.len(), self.statics.items, self.statics.cell_count(), t.elapsed().as_secs_f32() * 1000.0);
    }

    /// Applies a graphics configuration: cost knobs immediately, shadow map
    /// size by reallocating it; the texture cap affects later uploads.
    pub fn apply_graphics(&mut self, g: &GraphicsConfig) {
        self.settings.apply_graphics(g);
        if g.shadow_resolution != self.shadow_size {
            self.set_shadow_resolution(g.shadow_resolution);
        }
    }

    /// Reallocates the cascade shadow maps at `size` texels per side.
    pub fn set_shadow_resolution(&mut self, size: u32) {
        let size = size.clamp(256, self.device.limits().max_texture_dimension_2d);
        let shadow_tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("shadow"),
            size: wgpu::Extent3d { width: size, height: size, depth_or_array_layers: 4 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: SHADOW_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        self.shadow_view = shadow_tex.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
        self.shadow_layers = (0..4)
            .map(|i| shadow_tex.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2), base_array_layer: i, array_layer_count: Some(1), ..Default::default() }))
            .collect();
        self.shadow_size = size;
        self.cascade_cache = Default::default();
        self.rebuild_frame_bind();
    }

    pub fn shadow_resolution(&self) -> u32 {
        self.shadow_size
    }

    /// Counts `bytes` written to the GPU and submits the upload queue every
    /// ~64 MB. `write_buffer` and `write_texture` stage their data in host
    /// memory until the next submit; a whole city uploaded in one go would
    /// otherwise hold a second copy of every mesh and texture (GBs of RAM)
    /// until the first frame. The submit does not wait: the staging memory
    /// is released as the GPU gets through the copies (the loading screen
    /// bounds it further with [`Renderer::flush_uploads`] between slices).
    /// A rendered frame's own submit also counts as a flush.
    /// Only a bulk upload of more than 512 MB without a frame in between
    /// (a headless load) waits, to bound the staging memory.
    fn note_upload(&mut self, bytes: u64) {
        self.pending_upload += bytes;
        self.unflushed_upload += bytes;
        if self.unflushed_upload > 512 << 20 {
            self.flush_uploads();
        } else if self.pending_upload > 64 << 20 {
            self.submit_uploads();
        }
    }

    /// Submits the staged uploads without waiting for them.
    pub fn submit_uploads(&mut self) {
        if self.pending_upload == 0 {
            return;
        }
        self.pending_upload = 0;
        self.queue.submit([]);
        let _ = self.device.poll(wgpu::PollType::Poll);
        self.upload_submits += 1;
    }

    /// Submits the staged uploads and waits for them, releasing the staging
    /// memory. Blocks until the GPU is idle: for loading screens only (the
    /// frame loop uses [`Renderer::submit_uploads`]).
    pub fn flush_uploads(&mut self) {
        if self.unflushed_upload == 0 {
            return;
        }
        self.pending_upload = 0;
        self.unflushed_upload = 0;
        let t = std::time::Instant::now();
        self.queue.submit([]);
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        self.upload_waits.0 += 1;
        self.upload_waits.1 += t.elapsed().as_secs_f64() * 1000.0;
    }

    /// Forgets cached textures whose pixel data nobody else holds any
    /// more (the caller dropped its scenes after uploading them), so the
    /// CPU copy is freed. The GPU textures stay bound to their materials.
    /// Safe: a dropped buffer's address can only be reused after its entry
    /// is gone.
    pub fn prune_texture_cache(&mut self) {
        self.texture_cache.retain(|_, (data, _)| std::sync::Arc::strong_count(data) > 1);
    }

    /// Bytes of uploaded meshes (vertex, index and LOD buffers) and
    /// textures (with mips).
    pub fn uploaded_bytes(&self) -> (u64, u64) {
        (self.mesh_bytes, self.texture_bytes)
    }

    /// Estimated GPU memory in bytes: textures (with mips), meshes, render
    /// targets and shadow maps.
    pub fn vram_estimate(&self) -> u64 {
        self.texture_bytes + self.mesh_bytes + self.target_bytes + self.shadow_size as u64 * self.shadow_size as u64 * 4 * 4 + self.instance_cap + self.joint_cap
    }

    /// The `n` scenes with most triangles drawn last frame, in the camera
    /// (`shadow` false) or the shadow maps: `(scene, instances, triangles)`.
    pub fn heaviest_meshes(&self, n: usize, shadow: bool) -> Vec<(Arc<str>, u32, u64)> {
        let list = if shadow { &self.last_batches.1 } else { &self.last_batches.0 };
        let mut by: std::collections::HashMap<&str, (Arc<str>, u32, u64)> = Default::default();
        for b in list {
            let m = &self.meshes[b.mesh as usize];
            let e = by.entry(&*m.name).or_insert_with(|| (m.name.clone(), 0, 0));
            e.1 += b.count;
            e.2 += (m.indices(b.lod).1 / 3) as u64 * b.count as u64;
        }
        let mut v: Vec<_> = by.into_values().collect();
        v.sort_by(|a, b| b.2.cmp(&a.2));
        v.truncate(n);
        v
    }

    /// Finest LOD of `mesh` with at most `budget` triangles, plus `bias`
    /// levels, clamped to the levels it has.
    fn lod_for(meshes: &[GpuMesh], mesh: u32, budget: f32, bias: u32) -> u32 {
        let m = &meshes[mesh as usize];
        let n = m.lods.len() as u32;
        let mut lod = 0;
        let tris = |l: u32| m.lod_tris.as_ref().map_or(m.indices(l).1 / 3, |t| t[l as usize]);
        while lod < n && tris(lod) as f32 > budget {
            lod += 1;
        }
        (lod + bias).min(n)
    }

    // ---------------------------------------------------------------- render

    /// Renders a frame into `target` (a surface texture view or offscreen
    /// texture in [`Renderer::output_format`]). The internal resolution is
    /// the last [`Renderer::resize`]; when it is smaller than `target` (a
    /// render scale below 1) the final pass upsamples with sharpening.
    pub fn render(&mut self, frame: &FrameDesc, target: &wgpu::TextureView) {
        let t0 = std::time::Instant::now();
        self.cpu.clear();
        self.profiler.begin_frame(&self.device);
        let (width, height) = self.size();
        let cam = Camera { aspect: width as f32 / height as f32, ..frame.camera };
        let tf = self.temporal_frame(&cam, frame.time, width, height);
        let env = &frame.env;
        // Local lights: frustum culled, closest first; the first few
        // shadowed spots get a shadow map.
        let light_frustum = Frustum::from_view_proj(&cam.view_proj());
        let mut visible = lights::visible_lights(frame.lights.iter().map(|p| Light::from(*p)).chain(frame.local_lights.iter().copied()), &light_frustum, cam.position);
        // Quality caps: lights per frame (closest kept) and shadowed spots.
        visible.truncate(self.settings.max_lights.max(1) as usize);
        let max_spot = (self.settings.spot_shadows as usize).min(lights::MAX_SPOT_SHADOWS);
        let mut spot_vp = [Mat4::IDENTITY; lights::MAX_SPOT_SHADOWS];
        let mut n_spot = 0usize;
        let gpu_lights: Vec<lights::GpuLight> = visible
            .iter()
            .map(|l| {
                let mut layer = -1;
                if l.cast_shadows && n_spot < max_spot && let Some(m) = lights::spot_view_proj(l) {
                    spot_vp[n_spot] = m;
                    layer = n_spot as i32;
                    n_spot += 1;
                }
                lights::GpuLight::pack(l, layer)
            })
            .collect();
        let light_bytes = std::mem::size_of_val(gpu_lights.as_slice()) as u64;
        if light_bytes > self.lights_cap {
            self.lights_cap = light_bytes.next_power_of_two();
            self.lights_buf = self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("lights"), size: self.lights_cap, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
            self.rebuild_frame_bind();
        }
        if !gpu_lights.is_empty() {
            self.writes.stage(&self.lights_buf, 0, bytemuck::cast_slice(&gpu_lights));
        }
        let n_lights = gpu_lights.len() as u32;
        // Eye adaptation state: manual exposure keeps pre-exposure 1 (the
        // daytime path is unchanged); auto exposure starts from `exposure`.
        // Eye adaptation belongs to the night camera; the daylight camera
        // meters itself (and needs pre-exposure 1).
        let auto = if env.camera == CameraModel::Night { env.auto_exposure } else { None };
        if let Some(a) = auto {
            let dt = frame.time - self.prev_time;
            let dt = if dt > 0.0 && dt < 0.5 { dt } else { 1.0 / 60.0 };
            let instant = self.exposure_instant || !self.auto_exposure_prev;
            let u = ExposureUniforms {
                a: [dt, a.brighten_rate, a.darken_rate, instant as u32 as f32],
                b: [a.min_exposure, a.max_exposure, a.compensation_ev.exp2(), a.low_percentile],
                c: [a.high_percentile, EXPOSURE_LOG_MIN, EXPOSURE_LOG_RANGE, EXPOSURE_KEY_SCALE],
                d: [lights::NITS_PER_UNIT, 0.0, 0.0, 0.0],
            };
            self.writes.stage(&self.exposure_uniform, 0, bytemuck::bytes_of(&u));
            if !self.auto_exposure_prev {
                self.writes.stage(&self.exposure_state, 0, bytemuck::cast_slice(&[1.0f32, 1.0, env.exposure, 0.0]));
            }
        } else {
            self.writes.stage(&self.exposure_state, 0, bytemuck::cast_slice(&[1.0f32, 1.0, env.exposure, 0.0]));
        }
        self.auto_exposure_prev = auto.is_some();
        // Real frame time (motion blur shutter); 1/60 for stills and jumps.
        let frame_dt = {
            let dt = frame.time - self.prev_time;
            if dt > 1e-4 && dt < 0.5 { dt } else { 1.0 / 60.0 }
        };
        self.prev_time = frame.time;
        let ibl = self.env.as_ref().filter(|_| env.use_ibl);
        let (sun, mut sky) = match ibl {
            Some(e) => (e.sun_dir, atmosphere::SkyLighting { sun_color: e.sun_color, zenith: e.zenith, horizon: e.horizon, ground: e.ground }),
            None => {
                let sun = env.sun_dir.normalize();
                (sun, atmosphere::sky_lighting(sun, env.sun_illuminance, Vec3::new(0.25, 0.24, 0.22)))
            }
        };
        // The night sky lights the scene too (analytic sky only).
        let night_sky = if ibl.is_none() { env.night_sky } else { night::NightSky { zenith: Vec3::ZERO, glow: Vec3::ZERO, stars: 0.0, ..env.night_sky } };
        {
            let ns = &night_sky;
            let g = |s: f32| ns.glow * (-s / ns.glow_height.max(0.005)).exp();
            sky.zenith += ns.zenith + g(0.94);
            sky.horizon += ns.zenith + g(0.08);
            sky.ground += (ns.zenith + g(0.3)) * 0.12;
        }
        // Physical camera: meter the sun and sky on open ground.
        let lum = |c: Vec3| c.dot(Vec3::new(0.2126, 0.7152, 0.0722));
        let meter_light = exposure::dome_illuminance(sky.sun_color, sun.y, sky.zenith * std::f32::consts::PI);
        let meter_illuminance = lum(meter_light);
        // Physical camera (manual EV100 or metered). Night camera:
        // the linear exposure (eye adaptation starts from it).
        let daylight = env.camera == CameraModel::Physical;
        let exposure = if daylight { env.exposure(meter_illuminance) } else { env.exposure };
        let mut sh = [[0.0f32; 4]; 9];
        if let Some(e) = ibl {
            for k in 0..9 {
                sh[k] = e.sh[k].extend(0.0).to_array();
            }
        }
        let view = cam.view();
        // Rendering uses the jittered matrix; culling is unaffected by a
        // sub-pixel shift.
        let view_proj = tf.view_proj;
        let frustum = Frustum::from_view_proj(&view_proj);
        // Fit the cascades to the visible depth range from a lower bound of
        // the distance to the closest visible geometry: bounding spheres for
        // ordinary dynamic instances, the box for large ones (a ground
        // plane's sphere would contain an aerial camera), and the cell
        // bounds of the static grid. Stops as soon as something is near the
        // camera, as it always is at street level.
        let near = cam.near.max(0.1);
        // `shadow_distance <= 0` turns the sun (or moon) shadows off.
        let sun_shadows = env.shadow_distance > 0.0;
        let mut nearest = if sun_shadows { self.statics.nearest_visible(&frustum.planes, cam.position, near * 1.2) } else { f32::INFINITY };
        if sun_shadows && nearest > near * 1.2 {
            for inst in &frame.instances {
                let Some(model) = self.models.get(inst.model.0 as usize) else { continue };
                if model.bounds.is_empty() {
                    continue;
                }
                let m = &inst.transform;
                let scale = m.x_axis.truncate().length().max(m.y_axis.truncate().length()).max(m.z_axis.truncate().length());
                let animated = inst.joints.is_some() || inst.nodes.is_some();
                let r = model.bounds.half_extents().length() * scale + if animated { 0.5 } else { 0.0 };
                let c = m.transform_point3(model.bounds.center());
                if frustum.intersects_sphere(c, r) {
                    let d = if r > 50.0 {
                        let b = transform_aabb(&model.bounds, m);
                        (b.min - cam.position).max(cam.position - b.max).max(Vec3::ZERO).length()
                    } else {
                        (c - cam.position).length() - r
                    };
                    nearest = nearest.min(d);
                    if nearest <= near * 1.2 {
                        break;
                    }
                }
            }
        }
        let s = self.settings.clone();
        let shadow_distance = env.shadow_distance * s.shadow_distance_scale;
        let (shadow_near, shadow_far) = if sun_shadows { camera::fitted_shadow_range(near, nearest, cam.fov_y, cam.aspect, shadow_distance) } else { (near, near * 2.0) };
        let splits = if sun_shadows { camera::cascade_splits(shadow_near, shadow_far, 0.8) } else { [0.0; 4] };
        let mut cascade_vp = [Mat4::IDENTITY; 4];
        let mut cascade_texel = [0.0f32; 4];
        let mut cascade_range = [0.0f32; 4];
        let mut prev = shadow_near.max(cam.near);
        let caster_reach = 400.0;
        // Far cascades are refreshed every `far_cascade_interval` frames
        // (cascade 2 and 3 on alternate frames) and otherwise reuse the
        // matrix they were rendered with. Their fit is padded by 4% so the
        // stale map still covers the moving slice.
        let interval = s.far_cascade_interval.max(1);
        if !s.stable_shadows || self.shadow_sun.dot(sun) < 0.1f32.to_radians().cos() {
            self.shadow_sun = sun;
        }
        let sun_exact = sun;
        let sun = self.shadow_sun;
        let mut refresh = [true; 4];
        for (i, split) in splits.iter().enumerate() {
            let cached = self.cascade_cache[i];
            let far_c = i >= 2 && interval > 1;
            let due = (tf.frame_index + i as u32) % interval == 0;
            let reuse = far_c && !due && cached.valid && (cached.split - split).abs() <= split * 0.01 && cached.sun.dot(sun) > 0.99999;
            if reuse {
                cascade_vp[i] = cached.vp;
                cascade_texel[i] = cached.texel;
                cascade_range[i] = cached.range;
                refresh[i] = false;
            } else {
                let pad = if far_c { 1.04 } else { 1.0 };
                let (m, radius) = camera::cascade_fit_padded(&cam, prev, *split, sun, self.shadow_size, caster_reach, pad);
                cascade_vp[i] = m;
                cascade_texel[i] = 2.0 * radius / self.shadow_size as f32;
                cascade_range[i] = 2.0 * radius + caster_reach;
                self.cascade_cache[i] = CascadeCache { valid: true, vp: m, texel: cascade_texel[i], range: cascade_range[i], split: *split, sun };
            }
            prev = *split;
        }
        let proj = cam.proj();
        let fx_on = |b: bool| b as u32 as f32;
        // Froxel fog grid for this viewport and quality (rebinds the main
        // and sky passes when the volume changes).
        if self.fog.configure(&self.device, (width, height), s.volumetric_fog) || self.sky_bind.is_none() {
            self.sky_bind = Some(self.make_sky_bind());
            if let Some(t) = &self.targets {
                let b = self.make_screen_bind(&t.ao, &t.depth, &t.history, &t.normal);
                self.targets.as_mut().expect("resized").screen_bind = b;
            }
        }
        // Lighting uses the exact sun (only the shadow maps are stepped).
        let sun = sun_exact;
        // Low dawn/dusk haze: zero above ~20 degrees of sun and with the
        // night camera, 6e-4 1/m at the horizon (80 m scale height).
        let haze = env.haze.unwrap_or_else(|| if daylight { 6.0e-4 * (1.0 - ((sun.y - 0.05) / 0.3).clamp(0.0, 1.0)).powi(2) } else { 0.0 });
        let fog_volume = self.fog.uniform_params((haze.max(0.0), 1.0 / 80.0));
        let (tiles_x, tiles_y) = self.cluster_grid;
        let slice_k = lights::CLUSTER_SLICES as f32 / (lights::CLUSTER_FAR / lights::CLUSTER_NEAR).ln();
        let ns = &night_sky;
        let uniforms = FrameUniforms {
            view_proj: view_proj.to_cols_array_2d(),
            view: view.to_cols_array_2d(),
            inv_view_proj: view_proj.inverse().to_cols_array_2d(),
            camera_pos: [cam.position.x, cam.position.y, cam.position.z, frame.time],
            sun_dir: [sun.x, sun.y, sun.z, env.sun_illuminance],
            sun_color: [sky.sun_color.x, sky.sun_color.y, sky.sun_color.z, exposure],
            sky_zenith: sky.zenith.extend(0.0).to_array(),
            sky_horizon: sky.horizon.extend(env.fog_density).to_array(),
            ground_color: sky.ground.extend(env.fog_falloff).to_array(),
            cascade_splits: splits,
            cascade_vp: cascade_vp.map(|m| m.to_cols_array_2d()),
            params: [ibl.map_or(0.0, |e| e.sun_top), 1.0 / self.shadow_size as f32, env.bloom, fx_on(daylight)],
            screen: [width as f32, height as f32, 1.0 / width as f32, 1.0 / height as f32],
            sh,
            env_params: [ibl.is_some() as u32 as f32, ibl.map_or(0.0, |e| e.max_mip), 1.0, self.settings.legacy_bits(ibl.is_some_and(|e| !e.prefiltered))],
            prev_view_proj: tf.prev_view_proj.to_cols_array_2d(),
            proj_params: [1.0 / proj.x_axis.x, 1.0 / proj.y_axis.y, cam.near, proj.y_axis.y * height as f32 * 0.5],
            jitter: [tf.jitter.x, tf.jitter.y, tf.frame_index as f32, fx_on(tf.history_valid)],
            cascade_texel,
            cascade_range,
            fx: [if s.ssao { env.ao_strength } else { 0.0 }, fx_on(s.contact_shadows && sun_shadows), fx_on(s.ssr && tf.history_valid), env.sun_angular_diameter],
            ao_params: [env.ao_radius, env.specular_occlusion, 60.0, s.debug_view as f32],
            lighting: [n_lights as f32, fx_on(auto.is_some()), env.light_scatter, env.sprite_fade_distance],
            cluster: [tiles_x as f32, tiles_y as f32, lights::CLUSTER_SLICES as f32, lights::CLUSTER_TILE as f32],
            cluster_z: [lights::CLUSTER_NEAR, lights::CLUSTER_FAR, slice_k, env.scatter_distance],
            wet: [env.wetness, env.puddles, 1.0 / env.puddle_size.max(0.1), env.emissive_scale],
            fill: env.ambient_fill.extend(ns.stars).to_array(),
            night_zenith: ns.zenith.extend(env.sprite_min_pixels).to_array(),
            sky_glow: ns.glow.extend(ns.glow_height.max(0.005)).to_array(),
            scatter: [env.scatter_anisotropy.clamp(-0.95, 0.95), env.light_sprites, n_spot as f32, fx_on(env.wetness > 0.0)],
            features: [fx_on(s.sky_lut), 0.0, 0.0, 0.0],
            fog_volume,
            spot_vp: spot_vp.map(|m| m.to_cols_array_2d()),
        };
        let half_ao = s.ssao && s.ao_half_res;
        let quality = QualityUniforms {
            params: [s.pcss_cascades as f32, s.ssr_steps.max(4) as f32, s.contact_steps.max(1) as f32, if half_ao { 2.0 } else { 1.0 }],
            extra: [fx_on(s.bent_normals && s.ssao), fx_on(s.sky_lut), fx_on(s.fsr), 0.0],
        };
        self.writes.stage(&self.frame_buf, 0, bytemuck::bytes_of(&uniforms));
        self.writes.stage(&self.quality_buf, 0, bytemuck::bytes_of(&quality));
        self.writes.stage(&self.taa_buf, 0, bytemuck::bytes_of(&tf.taa_params));
        for (i, m) in cascade_vp.iter().chain(&spot_vp[..n_spot]).enumerate() {
            if i >= 4 || refresh[i] {
                self.writes.stage(&self.cascade_bufs[i], 0, bytemuck::bytes_of(&m.to_cols_array_2d()));
            }
        }
        self.cpu.add("setup", t0.elapsed().as_secs_f32() * 1000.0);

        // ---- Visibility: static grid + dynamic instances -> instanced batches.
        let t_cull = std::time::Instant::now();
        let tan_half_fov = (cam.fov_y * 0.5).tan();
        // Shadow casters: the four sun cascades, then the shadowed spots
        // (with the smallest caster radius each accepts). A view that is
        // not re-rendered this frame (cached far cascade, sun shadows off)
        // gathers nothing.
        const VIEWS: usize = 4 + lights::MAX_SPOT_SHADOWS;
        let mut cascade_planes: Vec<([Vec4; 6], f32)> = cascade_vp.iter().zip(cascade_texel).map(|(m, t)| (camera::frustum_planes_standard(m), if sun_shadows { s.shadow_caster_texels * t } else { f32::INFINITY })).collect();
        cascade_planes.extend(spot_vp[..n_spot].iter().map(|m| (camera::frustum_planes_standard(m), 0.05)));
        let drawn = |c: usize| if c < 4 { sun_shadows && refresh[c] } else { c < 4 + n_spot };
        let mut all = std::mem::take(&mut self.scratch);
        all.clear();
        let mut main_batches: Vec<Batch> = Vec::new();
        let mut shadow_batches: [Vec<Batch>; VIEWS] = Default::default();
        // LOD: the finest level with at most one triangle per two covered
        // pixels; covered pixels = PI * (radius / distance * focal)^2.
        let focal = height as f32 * 0.5 / tan_half_fov;
        let lod_scale = std::f32::consts::PI * focal * focal * 0.5 * s.lod_detail;
        self.statics.update_groups(cam.position, s.hlod_distance);
        // Dynamic instances are culled on a worker while the static views
        // are gathered (they only read the scene).
        let dynamic_cull = || {
            let t = std::time::Instant::now();
            let mut joints: Vec<[[f32; 4]; 4]> = Vec::new();
            let mut main: Vec<(u32, InstanceRaw)> = Vec::new();
            let mut shadow: [Vec<(u32, InstanceRaw)>; 4 + lights::MAX_SPOT_SHADOWS] = Default::default();
            let mut culled = 0u32;
            // Object motion: last frame's transform of each `motion_id`.
            let mut motion_now: std::collections::HashMap<u64, Mat4> = std::collections::HashMap::with_capacity(self.prev_motion.len());
            let mut object_motion = false;
            for inst in &frame.instances {
                let Some(model) = self.models.get(inst.model.0 as usize) else { continue };
                // Whole-instance tests first: frustum, sub-pixel detail and
                // distant small shadow casters are rejected without touching
                // their parts.
                let ib = if inst.joints.is_some() || inst.nodes.is_some() { model.bounds.expanded(0.5) } else { model.bounds };
                let wbi = transform_aabb(&ib, &inst.transform);
                let radius = wbi.half_extents().length();
                let dist = (wbi.center() - cam.position).length().max(0.1);
                let px_radius = radius / (dist * tan_half_fov) * height as f32 * 0.5;
                let in_main = px_radius > 0.6 && frustum.intersects_aabb(&wbi);
                let mut cmask = 0u8;
                if inst.cast_shadows {
                    for (c, (planes, min_r)) in cascade_planes.iter().enumerate() {
                        if drawn(c) && radius >= *min_r && aabb_in_planes(&wbi, planes) {
                            cmask |= 1 << c;
                        }
                    }
                }
                if !in_main && cmask == 0 {
                    culled += model.parts.len() as u32;
                    continue;
                }
                // Large multi-part models (buildings, whole levels) still get
                // per-part culling; small modules take the instance result.
                let per_part = radius > 12.0 && model.parts.len() > 1;
                let joint_offset = if let Some(j) = &inst.joints {
                    let off = joints.len() as u32;
                    joints.extend(j.iter().map(|m| m.to_cols_array_2d()));
                    off
                } else {
                    0
                };
                let inst_pixels = std::f32::consts::PI * px_radius * px_radius;
                let prev_inst = if inst.motion_id != 0 {
                    motion_now.insert(inst.motion_id, inst.transform);
                    self.prev_motion.get(&inst.motion_id).copied().filter(|p| *p != inst.transform)
                } else {
                    None
                };
                for part in &model.parts {
                    let mesh = &self.meshes[part.mesh as usize];
                    let skinned = mesh.skin.is_some() && inst.joints.is_some();
                    let node_m = inst.nodes.as_ref().and_then(|n| n.get(part.node).copied()).unwrap_or(model.rest[part.node]);
                    let world = if skinned { inst.transform } else { inst.transform * node_m };
                    let (visible, pixels, wb) = if per_part {
                        let wb = transform_aabb(&mesh.lod_bounds, &world);
                        let cov = wb.half_extents().length() / ((wb.center() - cam.position).length().max(0.1) * tan_half_fov);
                        (in_main && frustum.intersects_aabb(&wb), std::f32::consts::PI * (cov * height as f32 * 0.5).powi(2), Some(wb))
                    } else {
                        (in_main, inst_pixels, None)
                    };
                    let lod = Self::lod_for(&self.meshes, part.mesh, pixels * 0.5 * s.lod_detail, 0);
                    // extra.w: 1 + index of last frame's model matrix in the
                    // joint buffer (prepass velocity), 0 when it did not move.
                    let prev_slot = match prev_inst {
                        Some(p) if visible => {
                            object_motion = true;
                            joints.push((if skinned { p } else { p * node_m }).to_cols_array_2d());
                            joints.len() as u32
                        }
                        _ => 0,
                    };
                    let raw = InstanceRaw { model: world.to_cols_array_2d(), tint: inst.tint.to_array(), extra: [joint_offset, skinned as u32, lod, prev_slot] };
                    let role = mesh.shadow;
                    if visible && role != ShadowRole::Proxy {
                        main.push((part.mesh, raw));
                    } else {
                        culled += 1;
                    }
                    if cmask != 0 && !self.materials[mesh.material as usize].blend {
                        let mut sraw = raw;
                        // One level coarser than the camera, and no finer than one
                        // triangle per two texels of the finest sun cascade it
                        // falls in (spot views: the camera rule only).
                        let c0 = cmask.trailing_zeros() as usize;
                        let texel_lod = if c0 < 4 && cascade_texel[c0] > 0.0 {
                            let r = wb.as_ref().map_or(radius, |b| b.half_extents().length()) / cascade_texel[c0];
                            Self::lod_for(&self.meshes, part.mesh, std::f32::consts::PI * 0.5 * s.lod_detail * r * r, 0)
                        } else {
                            0
                        };
                        sraw.extra[2] = if s.stable_shadows && c0 < 4 { texel_lod } else { (lod + 1).max(texel_lod) }.min(mesh.lods.len() as u32);
                        for (c, (planes, _)) in cascade_planes.iter().enumerate() {
                            let wanted = Self::shadow_skip(c, &s) & static_scene::role_bit(role) == 0;
                            if wanted && cmask & (1 << c) != 0 && wb.as_ref().is_none_or(|wb| aabb_in_planes(wb, planes)) {
                                shadow[c].push((part.mesh, sraw));
                            }
                        }
                    }
                }
            }
            (joints, main, shadow, culled, motion_now, object_motion, t.elapsed().as_secs_f32() * 1000.0)
        };
        let dyn_out;
        // The views are gathered in parallel, each into its own list; the
        // lists are written to the instance buffer one after the other
        // (static views first, the dynamic instances after them).
        let mut static_lists = std::mem::take(&mut self.view_lists);
        static_lists.resize_with(VIEWS + 1, Default::default);
        let static_total;
        {
            use rayon::prelude::*;
            let meshes = &self.meshes;
            let statics = &self.statics;
            let main_view = static_scene::View { planes: &frustum.planes, eye: cam.position, min_radius_per_m: 0.6 / focal, min_radius: 0.0, lod_scale, texel_lod_scale: 0.0, shadow: false, skip: static_scene::role_bit(ShadowRole::Proxy), stable_lod: false };
            // (view, LOD bias) per list: the camera, then each shadow view.
            let mut views: Vec<Option<(static_scene::View, u32)>> = vec![Some((main_view, 0))];
            for (c, (planes, min_r)) in cascade_planes.iter().enumerate() {
                if !drawn(c) {
                    views.push(None);
                    continue;
                }
                // Casters are culled by their size in shadow texels, not by
                // camera distance: facades are assembled from many small
                // modules that together cast the large shadows.
                // LOD: never finer than the camera's (plus the bias below),
                // nor than one triangle per two shadow texels covered.
                let texel = if c < 4 { cascade_texel[c] } else { 0.0 };
                let texel_lod_scale = if texel > 0.0 { std::f32::consts::PI * 0.5 * s.lod_detail / (texel * texel) } else { 0.0 };
                let skip = Self::shadow_skip(c, &s);
                let v = static_scene::View { planes, eye: cam.position, min_radius_per_m: 0.0, min_radius: *min_r, lod_scale, texel_lod_scale, shadow: true, skip, stable_lod: s.stable_shadows };
                // Shadow maps hide silhouette detail: one LOD coarser than
                // the camera in the near cascades and spots, two in the far
                // cascades.
                let bias = if (2..4).contains(&c) { 2 } else { 1 };
                views.push(Some((v, bias)));
            }
            views.resize_with(VIEWS + 1, || None);
            let gather = || static_lists.par_iter_mut().zip(views.par_iter()).for_each(|(list, view)| {
                let t = std::time::Instant::now();
                list.out.clear();
                list.batches.clear();
                list.ms = 0.0;
                if let Some((v, bias)) = view {
                    let bias = *bias;
                    statics.gather_into(&mut list.scratch, v, &|m, b| Self::lod_for(meshes, m, b, bias), &mut list.out, &mut list.batches);
                    list.ms = t.elapsed().as_secs_f32() * 1000.0;
                }
            });
            // On the frame's own pool: never queued behind a dusk/dawn
            // build's jobs in rayon's global pool.
            dyn_out = forja_assets::threads::on_frame_pool(|| rayon::join(gather, dynamic_cull)).1;
            // Instance offsets: each list after the previous ones.
            let mut base = 0u32;
            for (k, list) in static_lists.iter_mut().enumerate() {
                list.base = base;
                let dst = if k == 0 { &mut main_batches } else { &mut shadow_batches[k - 1] };
                dst.extend(list.batches.iter().map(|b| Batch { first: b.first + base, ..*b }));
                base += list.out.len() as u32;
            }
            static_total = base;
        }
        let static_ms = t_cull.elapsed().as_secs_f32() * 1000.0;
        self.cpu.add("cull static", static_ms);
        // Worker time of the camera view and the slowest shadow view.
        self.cpu.add("cull cam view", static_lists[0].ms);
        self.cpu.add("cull max shadow view", static_lists[1..].iter().map(|l| l.ms).fold(0.0, f32::max));
        let t_dyn = std::time::Instant::now();
        let (mut joints, mut main, mut shadow, culled, motion_now, object_motion, dyn_ms) = dyn_out;
        self.cpu.add("cull dynamic", dyn_ms);
        self.prev_motion = motion_now;
        // Dynamic lists -> batches (skinned meshes without joints fall back
        // to the static pipeline, i.e. the bind pose).
        let main_key = |r: &Renderer, mesh: u32, raw: &InstanceRaw| {
            let m = &r.meshes[mesh as usize];
            let mat = &r.materials[m.material as usize];
            let pipe: u8 = if mat.blend { if raw.extra[1] == 1 { 4 } else { 3 } } else if raw.extra[1] == 1 { 2 } else if mat.double_sided { 1 } else { 0 };
            (pipe, m.material, mesh, raw.extra[2])
        };
        main.sort_by_key(|(m, raw)| main_key(self, *m, raw));
        let mut i = 0;
        while i < main.len() {
            let k = main_key(self, main[i].0, &main[i].1);
            let first = static_total + all.len() as u32;
            while i < main.len() && main_key(self, main[i].0, &main[i].1) == k {
                all.push(main[i].1);
                i += 1;
            }
            main_batches.push(Batch { pipe: k.0, material: k.1, mesh: k.2, lod: k.3, skinned: k.0 == 2 || k.0 == 4, first, count: static_total + all.len() as u32 - first });
        }
        for (c, list) in shadow.iter_mut().enumerate() {
            list.sort_by_key(|(m, raw)| (raw.extra[1], *m, raw.extra[2]));
            let mut i = 0;
            while i < list.len() {
                let k = (list[i].1.extra[1], list[i].0, list[i].1.extra[2]);
                let first = static_total + all.len() as u32;
                while i < list.len() && (list[i].1.extra[1], list[i].0, list[i].1.extra[2]) == k {
                    all.push(list[i].1);
                    i += 1;
                }
                let material = self.meshes[k.1 as usize].material;
                shadow_batches[c].push(Batch { pipe: 0, material, mesh: k.1, lod: k.2, skinned: k.0 == 1, first, count: static_total + all.len() as u32 - first });
            }
        }
        // Submission order: pipeline, then material, then mesh.
        main_batches.sort_by_key(Batch::sort_key);
        for list in &mut shadow_batches {
            list.sort_by_key(|b| {
                let masked = self.materials[b.material as usize].masked;
                (masked, b.skinned, if masked { b.material } else { 0 }, b.mesh, b.lod)
            });
        }
        self.cpu.add("batch dynamic", t_dyn.elapsed().as_secs_f32() * 1000.0);

        let t_up = std::time::Instant::now();
        let raw_size = std::mem::size_of::<InstanceRaw>() as u64;
        let bytes = (static_total as u64 + all.len() as u64) * raw_size;
        if bytes > self.instance_cap {
            self.instance_cap = bytes.next_power_of_two();
            self.instance_buf = self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("instances"), size: self.instance_cap, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        }
        // All views' instances in one staging copy (each write_buffer
        // allocates and frees its own staging buffer in wgpu).
        if let Some(size) = wgpu::BufferSize::new(bytes)
            && let Some(mut view) = self.queue.write_buffer_with(&self.instance_buf, 0, size)
        {
            for list in &static_lists {
                let at = list.base as usize * raw_size as usize;
                let src: &[u8] = bytemuck::cast_slice(&list.out);
                view.slice(at..at + src.len()).copy_from_slice(src);
            }
            let at = static_total as usize * raw_size as usize;
            let src: &[u8] = bytemuck::cast_slice(&all);
            view.slice(at..at + src.len()).copy_from_slice(src);
        }
        self.view_lists = static_lists;
        if joints.is_empty() {
            joints.push(Mat4::IDENTITY.to_cols_array_2d());
        }
        let jbytes = (joints.len() * 64) as u64;
        if jbytes > self.joint_cap {
            self.joint_cap = jbytes.next_power_of_two();
            self.joint_buf = self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("joints"), size: self.joint_cap, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
            self.joint_bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("joints"), layout: &self.joints_bgl, entries: &[wgpu::BindGroupEntry { binding: 0, resource: self.joint_buf.as_entire_binding() }] });
        }
        self.writes.stage(&self.joint_buf, 0, bytemuck::cast_slice(&joints));
        let line_bytes = (frame.lines.len() * std::mem::size_of::<LineVertex>()) as u64;
        if line_bytes > self.line_cap {
            self.line_cap = line_bytes.next_power_of_two();
            self.line_buf = self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("lines"), size: self.line_cap, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        }
        if !frame.lines.is_empty() {
            self.writes.stage(&self.line_buf, 0, bytemuck::cast_slice(&frame.lines));
        }
        let instance_bytes = bytes;
        self.scratch = all;
        let t = self.targets.as_ref().expect("resized");
        let pu = &t.post_bind.uniforms;
        // Camera meter: EV of the center-weighted average luminance, clamped
        // around the incident reading (or the manual EV), minus compensation.
        let ev_incident = exposure::incident_ev100(meter_illuminance);
        let lux_ev = exposure::LUX_PER_UNIT * 100.0 / exposure::METER_K;
        let (open, close) = (env.auto_exposure_range[0].max(0.0), env.auto_exposure_range[1].max(0.0));
        self.writes.stage(&t.post_bind.meter_uniform,
            0,
            bytemuck::bytes_of(&PostUniforms {
                texel: [env.ev100.is_some() as u32 as f32, env.ev100.unwrap_or(0.0), env.exposure_compensation, 0.0],
                params: [lux_ev, ev_incident, open, close],
                ..Default::default()
            }),
        );
        // Stills, the first frame and hitches snap; a running clock adapts.
        let adapt = match self.exposure_time.map(|t0| frame.time - t0) {
            Some(dt) if dt > 0.0 && dt < 0.25 => 1.0 - (-dt / env.exposure_adaptation.max(1e-3)).exp(),
            _ => 1.0,
        } as f64;
        self.exposure_time = Some(frame.time);
        let tonemap_u = &pu[pu.len() - 2];
        let bloom_amount = if s.bloom { env.bloom } else { 0.0 };
        self.writes.stage(tonemap_u,
            0,
            bytemuck::bytes_of(&if daylight {
                PostUniforms {
                    texel: [1.0 / width as f32, 1.0 / height as f32, 0.0, 0.0],
                    params: [exposure::exposure_from_ev100(0.0), bloom_amount, env.vignette, 0.0],
                    curve: env.tone.coefficients(),
                    look: [env.tone.saturation, env.tone.highlight_desaturation, 1.0 / BLOOM_MIPS as f32, 1.0],
                    white: exposure::white_balance(meter_light, env.white_balance).extend(0.0).to_array(),
                }
            } else {
                PostUniforms { texel: [1.0 / width as f32, 1.0 / height as f32, env.scotopic, env.night_grade], params: [env.exposure, bloom_amount, env.vignette, fx_on(auto.is_some())], ..Default::default() }
            }),
        );
        // Bloom: first-mip compression (1 / limit) and the weight of the
        // wider mips (1 / 4 and 1 by day, as before).
        self.writes.stage(&pu[0],
            0,
            // By day the Karis average weights each 2x2 group by its exposed luma.
            bytemuck::bytes_of(&if daylight {
                PostUniforms { texel: [1.0 / width as f32, 1.0 / height as f32, 0.0, 1.0], params: [exposure, 0.0, 0.0, 1.0], ..Default::default() }
            } else {
                PostUniforms { texel: [1.0 / width as f32, 1.0 / height as f32, 1.0 / env.bloom_limit.max(0.01), 0.0], params: [0.0, 0.0, 0.0, 1.0], ..Default::default() }
            }),
        );
        for (k, i) in (1..BLOOM_MIPS).rev().enumerate() {
            let (_, bw, bh) = t.bloom[i];
            self.writes.stage(&pu[BLOOM_MIPS + k], 0, bytemuck::bytes_of(&PostUniforms { texel: [1.0 / bw as f32, 1.0 / bh as f32, 0.0, 0.0], params: [env.bloom_radius.max(1e-3), 0.0, 0.0, 0.0], ..Default::default() }));
        }
        // Final pass: FXAA without TAA, CAS sharpening with it (stronger
        // while the camera moves, when the temporal filter is softer).
        let still = tf.taa_params[3] > 0.5;
        let sharpen = if still { s.sharpen } else { (s.sharpen * 1.6).min(1.0) };
        self.writes.stage(&pu[pu.len() - 1],
            0,
            bytemuck::bytes_of(&PostUniforms { texel: [1.0 / width as f32, 1.0 / height as f32, 0.0, 0.0], params: [(!s.taa) as u32 as f32, sharpen, (!self.output_format.is_srgb()) as u32 as f32, fx_on(s.fsr)], ..Default::default() }),
        );
        self.cpu.add("upload", t_up.elapsed().as_secs_f32() * 1000.0);

        let mut look = frame.look;
        if self.settings.dof_override.is_some() {
            look.dof = self.settings.dof_override;
        }
        if !self.settings.dof {
            look.dof = None;
        }
        // The night grade (post.wgsl) already grades night frames: fade
        // the look's colour grade out so they are not graded twice.
        let look = look.under_night_grade(env.night_grade);
        self.camera_fx.fsr = s.fsr;
        let (dof_on, camera_on) = self.camera_fx.prepare(&self.queue, &look, frame.camera.fov_y, (width, height), tf.frame_index, self.settings.look);
        let t = self.targets.as_ref().expect("resized");
        if camera_on && !self.output_format.is_srgb() {
            // The camera stage is then the last pass and does the sRGB
            // encoding for a plain UNORM surface; FXAA must stay linear.
            self.writes.stage(&t.post_bind.uniforms[t.post_bind.uniforms.len() - 1], 0, bytemuck::bytes_of(&PostUniforms { texel: [1.0 / width as f32, 1.0 / height as f32, 0.0, 0.0], params: [(!self.settings.taa) as u32 as f32, sharpen, 0.0, 0.0], ..Default::default() }));
        }
        // Multi-draw arguments: at most one per batch of each pass.
        let mdi = self.settings.multi_draw && self.device.features().contains(wgpu::Features::INDIRECT_FIRST_INSTANCE);
        let max_args = (main_batches.len() * 2 + shadow_batches.iter().map(Vec::len).sum::<usize>()) as u64;
        let args_size = std::mem::size_of::<IndirectArgs>() as u64;
        if mdi && max_args * args_size > self.indirect_cap {
            self.indirect_cap = (max_args * args_size).next_power_of_two().max(4096);
            self.indirect_buf = Some(self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("indirect draws"), size: self.indirect_cap, usage: wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }));
        }
        let t_enc = std::time::Instant::now();
        let ev100 = env.ev100.unwrap_or_else(|| exposure::incident_ev100(meter_illuminance)) - env.exposure_compensation;
        let mut stats = RenderStats {
            instances_culled: culled,
            lights: n_lights,
            spot_shadows: n_spot as u32,
            ev100,
            width,
            height,
            static_instances: self.statics.items as u32,
            static_cells: self.statics.cell_count() as u32,
            hlod_tiles: self.statics.far_groups() as u32,
            ..Default::default()
        };
        for c in 0..4 {
            if drawn(c) {
                stats.cascades_updated |= 1 << c;
            }
        }
        // The frame is recorded into several command buffers, submitted in
        // this order: the shadow views, the prepass, AO + fog (`mid`), the
        // main pass, then post. wgpu validates and translates a pass into
        // backend commands in `CommandEncoder::finish`, on the calling
        // thread, so the heavy passes (thousands of draws) are recorded and
        // finished in parallel; the cheap full-screen passes stay here.
        // Each parallel pass writes its multi-draw arguments into its own
        // region of the indirect buffer (at most one per batch).
        let mut arg_base = 0usize;
        let mut region = |n: usize| {
            let b = arg_base;
            arg_base += n;
            b
        };
        // Timestamp slots are taken here, in submission order (the profiler
        // is not shared with the workers).
        const SHADOW_NAMES: [&str; 5] = ["shadow c0", "shadow c1", "shadow c2", "shadow c3", "shadow spots"];
        let mut jobs: Vec<PassJob<'_>> = Vec::new();
        for c in 0..4 + n_spot {
            if drawn(c) {
                jobs.push(PassJob::Shadow { c, base: region(shadow_batches[c].len()), tw: self.profiler.pass(SHADOW_NAMES[c.min(4)]) });
            }
        }
        jobs.push(PassJob::Prepass { base: region(main_batches.len()), tw: self.profiler.pass("prepass") });
        let t = self.targets.as_ref().expect("resized");
        let prof = &self.profiler;
        let compute = |enc: &mut wgpu::CommandEncoder, pipe: &wgpu::ComputePipeline, bind: &wgpu::BindGroup, groups: (u32, u32), name: &'static str| {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some(name), timestamp_writes: prof.compute_pass(name) });
            pass.set_pipeline(pipe);
            pass.set_bind_group(0, bind, &[]);
            pass.dispatch_workgroups(groups.0, groups.1, 1);
        };
        let screen_pass = |enc: &mut wgpu::CommandEncoder, view: &wgpu::TextureView, pipe: &wgpu::RenderPipeline, bind: &wgpu::BindGroup, name: &'static str| {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(name),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::WHITE), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: prof.pass(name),
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(pipe);
            pass.set_bind_group(0, &self.frame_bind, &[]);
            pass.set_bind_group(1, bind, &[]);
            pass.draw(0..3, 0..1);
        };
        let mut t_pass = std::time::Instant::now();
        let mut mid = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("ao+fog") });
        // Eye adaptation: this frame is pre-exposed by the last exposure
        // (read first by the main pass).
        if auto.is_some() {
            compute(&mut mid, &self.pipelines.exposure_begin, &t.exposure_bind, (1, 1), "exposure");
        }
        // Ambient occlusion: GTAO + bilateral denoise, at half resolution
        // with a depth-aware upsample, or at full resolution.
        if s.ssao {
            if half_ao {
                screen_pass(&mut mid, &t.ao_half_raw, &self.pipelines.gtao, &t.gtao_half_bind, "gtao");
                screen_pass(&mut mid, &t.ao_half, &self.pipelines.ao_blur, &t.blur_half_bind, "gtao");
                screen_pass(&mut mid, &t.ao, &self.pipelines.ao_upsample, &t.upsample_bind, "gtao");
            } else {
                screen_pass(&mut mid, &t.ao_raw, &self.pipelines.gtao, &t.gtao_bind, "gtao");
                screen_pass(&mut mid, &t.ao, &self.pipelines.ao_blur, &t.blur_bind, "gtao");
            }
        }
        // Light clusters (froxel light lists).
        if n_lights > 0 {
            let clusters = tiles_x * tiles_y * (lights::CLUSTER_SLICES + 1);
            compute(&mut mid, &self.pipelines.cluster, &self.cluster_bind, (clusters.div_ceil(64), 1), "light clusters");
        }
        // Sky-view LUT (and the transmittance LUT once), then the froxel
        // fog: lit by this frame's shadow maps and light clusters.
        self.fog.encode_sky(&self.device, &mut mid, &self.frame_bind, prof);
        self.fog.encode_volume(&self.queue, &mut mid, &self.frame_bind, prof);
        let ao_fog_ms = t_pass.elapsed().as_secs_f32() * 1000.0;
        jobs.push(PassJob::Finish(mid));
        // The main pass (one bind group change per material: the costliest
        // to record) is split into consecutive render passes recorded in
        // parallel; the first clears, the others load.
        {
            let n = main_batches.len();
            let chunks = match main_chunks_override() {
                Some(c) => c.clamp(1, n.max(1)),
                None => (n / MAIN_CHUNK_BATCHES).clamp(1, rayon::current_num_threads().max(1)),
            };
            let tw = prof.pass("main+sky");
            for k in 0..chunks {
                let (lo, hi) = (n * k / chunks, n * (k + 1) / chunks);
                // The first chunk writes the begin timestamp, the last the end.
                let tw = tw.as_ref().filter(|_| k == 0 || k + 1 == chunks).map(|w| wgpu::RenderPassTimestampWrites {
                    query_set: w.query_set,
                    beginning_of_pass_write_index: if k == 0 { w.beginning_of_pass_write_index } else { None },
                    end_of_pass_write_index: if k + 1 == chunks { w.end_of_pass_write_index } else { None },
                });
                jobs.push(PassJob::Main { base: region(hi - lo), tw, lo, hi, first: k == 0, last: k + 1 == chunks });
            }
        }
        // ---- Post (encoded here, finished in parallel): TAA, motion blur, DOF,
        // exposure, bloom, tonemap; the timestamps are resolved last.
        t_pass = std::time::Instant::now();
        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("post") });
        let screen_pass = |enc: &mut wgpu::CommandEncoder, view: &wgpu::TextureView, pipe: &wgpu::RenderPipeline, bind: &wgpu::BindGroup, name: &'static str| {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(name),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::WHITE), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: prof.pass(name),
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(pipe);
            pass.set_bind_group(0, &self.frame_bind, &[]);
            pass.set_bind_group(1, bind, &[]);
            pass.draw(0..3, 0..1);
        };
        let compute = |enc: &mut wgpu::CommandEncoder, pipe: &wgpu::ComputePipeline, bind: &wgpu::BindGroup, groups: (u32, u32), name: &'static str| {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some(name), timestamp_writes: prof.compute_pass(name) });
            pass.set_pipeline(pipe);
            pass.set_bind_group(0, bind, &[]);
            pass.dispatch_workgroups(groups.0, groups.1, 1);
        };
        // 5. TAA resolve into `resolved`, which becomes next frame's history.
        screen_pass(&mut enc, &t.resolved, &self.pipelines.taa, &t.taa_bind, "taa");
        enc.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo { texture: &t.resolved_tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyTextureInfo { texture: &t.history_tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
        // 5a. Motion blur: reads the history copy (= this frame's resolved
        // image), writes `resolved`; the TAA history stays sharp.
        if s.motion_blur && (!still || object_motion) {
            self.motion.encode(&self.queue, &mut enc, &self.frame_bind, &t.resolved, s.motion_blur_samples, frame_dt, prof);
        }
        // 5b. Depth of field (look.rs) into `hdr`, which is free after TAA.
        if dof_on {
            self.camera_fx.dof_pass(&mut enc, &t.hdr, &self.frame_bind, &t.camera, prof.pass("dof"));
        }
        // 5b. Eye adaptation: histogram of the resolved frame, new exposure
        // (read by the tonemapper below and next frame's pre-exposure).
        if auto.is_some() {
            enc.clear_buffer(&self.exposure_hist, 0, None);
            compute(&mut enc, &self.pipelines.exposure_hist, &t.exposure_bind, (width.div_ceil(32), height.div_ceil(32)), "exposure");
            compute(&mut enc, &self.pipelines.exposure_adapt, &t.exposure_bind, (1, 1), "exposure");
        }
        // 6. Bloom down/up chain.
        let fullscreen = |enc: &mut wgpu::CommandEncoder, view: &wgpu::TextureView, pipe: &wgpu::RenderPipeline, bind: &wgpu::BindGroup, load: wgpu::LoadOp<wgpu::Color>, name: &'static str| {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(name),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load, store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: None,
                timestamp_writes: prof.pass(name),
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(pipe);
            pass.set_bind_group(0, bind, &[]);
            pass.draw(0..3, 0..1);
        };
        for i in 0..BLOOM_MIPS {
            let bind = if i == 0 && dof_on { &t.dof_down } else { &t.post_bind.down[i] };
            fullscreen(&mut enc, &t.bloom[i].0, &self.pipelines.down, bind, wgpu::LoadOp::Clear(wgpu::Color::BLACK), "bloom");
        }
        // Meter the frame (the smallest mip is final once the down chain ran)
        // and adapt the camera's EV toward it.
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("meter"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &t.exposure, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: None,
                timestamp_writes: prof.pass("meter"),
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipelines.meter);
            pass.set_bind_group(0, &t.post_bind.meter, &[]);
            pass.set_blend_constant(wgpu::Color { r: adapt, g: adapt, b: adapt, a: adapt });
            pass.draw(0..3, 0..1);
        }
        // The down chain always runs (the meter reads it); the up chain only
        // with bloom on.
        if s.bloom {
            for (k, i) in (1..BLOOM_MIPS).rev().enumerate() {
                fullscreen(&mut enc, &t.bloom[i - 1].0, &self.pipelines.up, &t.post_bind.up[k], wgpu::LoadOp::Load, "bloom");
            }
        }
        // 7. Tonemap, then FXAA or sharpening, then the camera stage
        // (grade, grain, lens) when the look has one.
        let tonemap_bind = if dof_on { &t.dof_tonemap } else { &t.post_bind.tonemap };
        fullscreen(&mut enc, &t.ldr, &self.pipelines.tonemap, tonemap_bind, wgpu::LoadOp::Clear(wgpu::Color::BLACK), "tonemap");
        let aa_target = if camera_on { &t.camera.ldr2 } else { target };
        fullscreen(&mut enc, aa_target, &self.pipelines.fxaa, &t.post_bind.fxaa, wgpu::LoadOp::Clear(wgpu::Color::BLACK), "final");
        if camera_on {
            self.camera_fx.camera_pass(&mut enc, target, &t.camera, prof.pass("camera fx"));
        }
        self.profiler.resolve(&mut enc);
        let post_ms = t_pass.elapsed().as_secs_f32() * 1000.0;
        jobs.push(PassJob::Finish(enc));
        // ---- Parallel: record + finish the shadow, prepass and main passes.
        t_pass = std::time::Instant::now();
        let ctx = PassCtx {
            device: &self.device,
            pipelines: &self.pipelines,
            materials: &self.materials,
            meshes: &self.meshes,
            vpool: &self.vertex_pool,
            ipool: &self.index_pool,
            indirect: if mdi { self.indirect_buf.as_ref() } else { None },
            instances: &self.instance_buf,
            cascade_binds: &self.cascade_binds,
            shadow_layers: &self.shadow_layers,
            spot_shadow_layers: &self.spot_shadow_layers,
            joint_bind: &self.joint_bind,
            frame_bind: &self.frame_bind,
            sky_bind: self.sky_bind.as_ref().expect("sky bind"),
            line_buf: &self.line_buf,
            targets: t,
            main_batches: &main_batches,
            shadow_batches: &shadow_batches,
            lines: frame.lines.len() as u32,
            sprites: if env.light_sprites > 0.0 { n_lights } else { 0 },
        };
        let outs: Vec<PassOut> = {
            use rayon::prelude::*;
            forja_assets::threads::on_frame_pool(|| jobs.into_par_iter().with_max_len(1).map(|job| ctx.run(job)).collect())
        };
        let parallel_ms = t_pass.elapsed().as_secs_f32() * 1000.0;
        let mut indirect_args = std::mem::take(&mut self.indirect_args);
        indirect_args.clear();
        let mut cmds: Vec<wgpu::CommandBuffer> = Vec::with_capacity(outs.len());
        let mut enc_ms = [0.0f32; 4];
        for o in outs {
            cmds.push(o.cmd);
            enc_ms[o.kind] += o.ms;
            stats.api_draws += o.stats.api_draws;
            stats.shadow_draws += o.stats.shadow_draws;
            stats.shadow_instances += o.stats.shadow_instances;
            stats.shadow_triangles += o.stats.shadow_triangles;
            stats.draw_calls += o.stats.draw_calls;
            stats.instances_drawn += o.stats.instances_drawn;
            stats.triangles += o.stats.triangles;
            if !o.args.is_empty() {
                if indirect_args.len() < o.base + o.args.len() {
                    indirect_args.resize(o.base + o.args.len(), IndirectArgs::zeroed());
                }
                indirect_args[o.base..o.base + o.args.len()].copy_from_slice(&o.args);
            }
        }
        // The merged draws read their arguments from here (written before
        // the submit, so before the passes run).
        if let Some(buf) = ctx.indirect
            && !indirect_args.is_empty()
        {
            self.writes.stage(buf, 0, bytemuck::cast_slice(&indirect_args));
        }
        self.indirect_args = indirect_args;
        self.cpu.add("encode", ao_fog_ms + post_ms);
        self.cpu.add("enc ao+fog", ao_fog_ms);
        self.cpu.add("enc post", post_ms);
        // Worker CPU time of each parallel pass (encode + finish), and the
        // main thread's wait for all of them.
        self.cpu.add("enc shadows", enc_ms[0]);
        self.cpu.add("enc prepass", enc_ms[1]);
        self.cpu.add("enc main", enc_ms[2]);
        self.cpu.add("parallel passes", parallel_ms);
        self.cpu.add("finish", enc_ms[3]);
        let _ = t_enc;
        let t_sub = std::time::Instant::now();
        let writes = self.writes.flush(&self.device, &self.queue);
        cmds.splice(0..0, writes);
        self.queue.submit(cmds);
        // The frame's submit carries every staged upload with it: their
        // staging memory is released once the GPU is past this frame.
        self.pending_upload = 0;
        self.unflushed_upload = 0;
        self.profiler.after_submit();
        self.cpu.add("submit", t_sub.elapsed().as_secs_f32() * 1000.0);
        stats.cpu_ms = t0.elapsed().as_secs_f32() * 1000.0;
        stats.vram_mb = self.vram_estimate() as f32 / (1024.0 * 1024.0);
        let _ = instance_bytes;
        self.stats = stats;
        self.last_batches = (main_batches, shadow_batches.into_iter().flatten().collect());
    }

    /// Renders offscreen and reads back RGBA8 pixels (screenshots, tests).
    pub fn render_to_image(&mut self, frame: &FrameDesc, width: u32, height: u32) -> image::RgbaImage {
        let tex = self.render_offscreen(frame, width, height);
        self.read_image(&tex)
    }

    /// One real-time frame read back to RGBA8: unlike
    /// [`Renderer::render_to_image`] it keeps the temporal history, so a
    /// sequence of calls shows TAA, SSR and the noise of the screen-space
    /// effects as a moving game camera sees them.
    pub fn render_live_to_image(&mut self, frame: &FrameDesc, width: u32, height: u32) -> image::RgbaImage {
        self.resize(width, height);
        let tex = self.offscreen_texture(width, height);
        self.render(frame, &tex.create_view(&Default::default()));
        self.read_image(&tex)
    }

    fn offscreen_texture(&self, width: u32, height: u32) -> wgpu::Texture {
        self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("offscreen"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.output_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    }

    /// Renders into a new texture (so callers can draw UI on top before reading it back).
    pub fn render_offscreen(&mut self, frame: &FrameDesc, width: u32, height: u32) -> wgpu::Texture {
        self.resize(width, height);
        let tex = self.offscreen_texture(width, height);
        // A still image: start a fresh history and accumulate jittered
        // frames so TAA (and the per-frame noise of GTAO, PCSS and contact
        // shadows) converges to a supersampled result.
        self.reset_history();
        let view = tex.create_view(&Default::default());
        let frames = if self.settings.taa { self.settings.capture_frames.max(1) } else { 1 };
        // Eye adaptation converges instantly on a still image: two warm-up
        // frames (the first may be pre-exposed for another scene), which
        // also serve as the SSR history.
        let auto = frame.env.auto_exposure.is_some();
        self.exposure_instant = auto;
        let warmup = if auto { 2 } else { self.settings.ssr as u32 };
        // Screen-space reflections read the previous frame: render one
        // warm-up frame, then accumulate from scratch with it as history.
        if warmup > 0 {
            for _ in 0..warmup {
                self.render(frame, &view);
            }
            self.temporal.restart = true;
        }
        let mut cpu_ms = 0.0;
        for _ in 0..frames {
            self.render(frame, &view);
            cpu_ms += self.stats.cpu_ms;
        }
        self.exposure_instant = false;
        self.stats.cpu_ms = cpu_ms / frames as f32;
        tex
    }

    /// Copies an `output_format` texture back to the CPU.
    pub fn read_image(&self, tex: &wgpu::Texture) -> image::RgbaImage {
        let (width, height) = (tex.width(), tex.height());
        let row = (width * 4).div_ceil(256) * 256;
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("readback"), size: (row * height) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
        let mut enc = self.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(height) } },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
        self.queue.submit([enc.finish()]);
        let slice = buf.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        let data = slice.get_mapped_range().expect("mapped after poll");
        let bgra = matches!(self.output_format, wgpu::TextureFormat::Bgra8UnormSrgb | wgpu::TextureFormat::Bgra8Unorm);
        let mut img = image::RgbaImage::new(width, height);
        for y in 0..height {
            let src = &data[(y * row) as usize..(y * row + width * 4) as usize];
            for x in 0..width as usize {
                let p = &src[x * 4..x * 4 + 4];
                let px = if bgra { [p[2], p[1], p[0], 255] } else { [p[0], p[1], p[2], 255] };
                img.put_pixel(x as u32, y, image::Rgba(px));
            }
        }
        img
    }
}

#[allow(clippy::too_many_arguments)]
fn make_frame_bind(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    frame_buf: &wgpu::Buffer,
    shadow_view: &wgpu::TextureView,
    shadow_sampler: &wgpu::Sampler,
    env_view: &wgpu::TextureView,
    env_sampler: &wgpu::Sampler,
    lights_buf: &wgpu::Buffer,
    cluster_buf: &wgpu::Buffer,
    exposure_state: &wgpu::Buffer,
    spot_shadow_view: &wgpu::TextureView,
    dfg_view: &wgpu::TextureView,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("frame"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: frame_buf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(shadow_view) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(shadow_sampler) },
            wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(env_view) },
            wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::Sampler(env_sampler) },
            wgpu::BindGroupEntry { binding: 5, resource: lights_buf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 6, resource: cluster_buf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 7, resource: exposure_state.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::TextureView(spot_shadow_view) },
            wgpu::BindGroupEntry { binding: 9, resource: wgpu::BindingResource::TextureView(dfg_view) },
        ],
    })
}

fn make_cluster_bind(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, frame_buf: &wgpu::Buffer, lights_buf: &wgpu::Buffer, cluster_buf: &wgpu::Buffer) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("cluster"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: frame_buf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: lights_buf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 2, resource: cluster_buf.as_entire_binding() },
        ],
    })
}

fn view_entry(binding: u32, view: &wgpu::TextureView) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry { binding, resource: wgpu::BindingResource::TextureView(view) }
}

fn transform_aabb(b: &Aabb, m: &Mat4) -> Aabb {
    if b.is_empty() {
        return *b;
    }
    let c = m.transform_point3(b.center());
    let h = b.half_extents();
    let e = m.x_axis.truncate().abs() * h.x + m.y_axis.truncate().abs() * h.y + m.z_axis.truncate().abs() * h.z;
    Aabb::from_center_half(c, e)
}

fn aabb_in_planes(b: &Aabb, planes: &[Vec4; 6]) -> bool {
    let c = b.center();
    let h = b.half_extents();
    planes.iter().all(|p| {
        let n = p.truncate();
        n.dot(c) + n.abs().dot(h) + p.w >= 0.0
    })
}

/// Box-filtered mip chain; sRGB data is averaged in linear space.
/// Full mip chain, filtered in linear space. With `alpha_cutoff` (alpha
/// tested materials) each level's alpha is rescaled so the share of texels
/// passing the test matches level 0 (Castaño 2010), region by region so an
/// atlas of sparse leaf cards next to solid impostor views keeps both: leaf
/// cards keep their density at distance instead of dissolving.
/// Largest first mip uploaded for a texture under the preset's cap
/// `max_size`: colour maps (sRGB) use the cap, data maps (normal, ORM) half
/// of it below 4096 (Ultra). Their detail is tiled at a few millimetres a
/// texel either way, and on the default map this saves ~0.5 GB of VRAM on
/// High.
pub fn texture_cap(max_size: u32, srgb: bool) -> u32 {
    if srgb || max_size >= 4096 { max_size } else { (max_size / 2).max(512) }
}

fn generate_mips(t: &TextureData, alpha_cutoff: Option<f32>) -> Vec<(u32, u32, Vec<u8>)> {
    let to_lin: Vec<f32> = (0..256).map(|i| {
        let c = i as f32 / 255.0;
        if !t.srgb { c } else if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    }).collect();
    let to_enc = |v: f32| -> u8 {
        let v = v.clamp(0.0, 1.0);
        let e = if !t.srgb { v } else if v <= 0.0031308 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
        (e * 255.0 + 0.5) as u8
    };
    // Level-0 coverage on a grid of CELL-pixel cells.
    const CELL: u32 = 64;
    let (w0, h0) = (t.width, t.height);
    let (gx, gy) = (w0.div_ceil(CELL), h0.div_ceil(CELL));
    let cov0: Option<(f32, Vec<f32>)> = alpha_cutoff.map(|cut| {
        let mut pass = vec![0u32; (gx * gy) as usize];
        let mut cnt = vec![0u32; (gx * gy) as usize];
        for y in 0..h0 {
            for x in 0..w0 {
                let c = ((y / CELL) * gx + x / CELL) as usize;
                cnt[c] += 1;
                if t.rgba8[((y * w0 + x) * 4 + 3) as usize] as f32 / 255.0 >= cut {
                    pass[c] += 1;
                }
            }
        }
        (cut, pass.iter().zip(&cnt).map(|(p, c)| *p as f32 / (*c).max(1) as f32).collect())
    });
    let mut out = vec![(t.width, t.height, t.rgba8.to_vec())];
    let (mut w, mut h) = (t.width, t.height);
    let mut level = 0u32;
    while w > 1 || h > 1 {
        let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
        level += 1;
        let prev = &out.last().expect("non-empty").2;
        let mut next = vec![0u8; (nw * nh * 4) as usize];
        for y in 0..nh {
            for x in 0..nw {
                for c in 0..4 {
                    let mut acc = 0.0;
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let sx = (x * 2 + dx).min(w - 1);
                        let sy = (y * 2 + dy).min(h - 1);
                        let v = prev[((sy * w + sx) * 4 + c) as usize];
                        acc += if c == 3 { v as f32 / 255.0 } else { to_lin[v as usize] };
                    }
                    acc *= 0.25;
                    next[((y * nw + x) * 4 + c) as usize] = if c == 3 { (acc * 255.0 + 0.5) as u8 } else { to_enc(acc) };
                }
            }
        }
        if let Some((cut, cov)) = &cov0
            && nw * nh >= 16
        {
            // Blocks of `bs` texels at this level (at least 4), each matched
            // to the mean level-0 coverage of the cells it covers.
            let bs = (CELL >> level.min(31)).max(4);
            let (bx, by) = (nw.div_ceil(bs), nh.div_ceil(bs));
            for j in 0..by {
                for i in 0..bx {
                    let (x0, y0) = (i * bs, j * bs);
                    let (x1, y1) = ((x0 + bs).min(nw), (y0 + bs).min(nh));
                    // Matching level-0 cells.
                    let cx0 = (x0 as u64 * w0 as u64 / nw as u64 / CELL as u64) as u32;
                    let cy0 = (y0 as u64 * h0 as u64 / nh as u64 / CELL as u64) as u32;
                    let cx1 = ((x1 as u64 * w0 as u64).div_ceil(nw as u64).div_ceil(CELL as u64) as u32).clamp(cx0 + 1, gx);
                    let cy1 = ((y1 as u64 * h0 as u64).div_ceil(nh as u64).div_ceil(CELL as u64) as u32).clamp(cy0 + 1, gy);
                    let mut target = 0.0;
                    for cy in cy0..cy1 {
                        for cx in cx0..cx1 {
                            target += cov[(cy * gx + cx) as usize];
                        }
                    }
                    target /= ((cx1 - cx0) * (cy1 - cy0)).max(1) as f32;
                    if target <= 0.0 {
                        continue;
                    }
                    let block_cov = |next: &[u8], k: f32| {
                        let mut p = 0u32;
                        for y in y0..y1 {
                            for x in x0..x1 {
                                if (next[((y * nw + x) * 4 + 3) as usize] as f32 / 255.0 * k).min(1.0) >= *cut {
                                    p += 1;
                                }
                            }
                        }
                        p as f32 / ((x1 - x0) * (y1 - y0)) as f32
                    };
                    let (mut lo, mut hi) = (0.25f32, 8.0f32);
                    for _ in 0..12 {
                        let mid = (lo + hi) * 0.5;
                        if block_cov(&next, mid) < target { lo = mid } else { hi = mid }
                    }
                    let k = (lo + hi) * 0.5;
                    for y in y0..y1 {
                        for x in x0..x1 {
                            let a = &mut next[((y * nw + x) * 4 + 3) as usize];
                            *a = ((*a as f32 * k).min(255.0) + 0.5) as u8;
                        }
                    }
                }
            }
        }
        out.push((nw, nh, next));
        w = nw;
        h = nh;
    }
    out
}

#[cfg(test)]
mod mip_tests {
    use super::*;

    #[test]
    fn alpha_tested_mips_keep_coverage() {
        // Small leaf-like blobs: box filtering averages them below the
        // cutoff and the crown thins out; coverage-preserving mips keep them.
        let (w, h) = (128u32, 128u32);
        let mut px = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                let v = ((x as f32 * 0.9).sin() * (y as f32 * 0.7).sin() * 1.6).clamp(0.0, 1.0);
                px[i] = 40;
                px[i + 1] = 120;
                px[i + 2] = 30;
                px[i + 3] = (v * 255.0) as u8;
            }
        }
        let t = TextureData { name: "t".into(), width: w, height: h, rgba8: px.into(), srgb: true };
        let cov = |d: &[u8]| d.chunks_exact(4).filter(|p| p[3] >= 128).count() as f32 / (d.len() / 4) as f32;
        let plain = generate_mips(&t, None);
        let kept = generate_mips(&t, Some(0.5));
        let c0 = cov(&plain[0].2);
        // The left half of an atlas is solid: its coverage must not mask
        // the thinning of the right half.
        let mut px2 = t.rgba8.to_vec();
        for y in 0..h {
            for x in 0..w / 2 {
                px2[((y * w + x) * 4 + 3) as usize] = 255;
            }
        }
        let atlas = TextureData { name: "a".into(), width: w, height: h, rgba8: px2.into(), srgb: true };
        let am = generate_mips(&atlas, Some(0.5));
        let half_cov = |d: &[u8], lw: u32| d.chunks_exact(4).enumerate().filter(|(i, p)| (*i as u32 % lw) >= lw / 2 && p[3] >= 128).count() as f32 / (d.len() / 8) as f32;
        let right0 = half_cov(&am[0].2, w);
        assert!((half_cov(&am[3].2, w >> 3) - right0).abs() < 0.08, "atlas right half {} vs {right0}", half_cov(&am[3].2, w >> 3));
        assert!(cov(&plain[3].2) < c0 * 0.6, "box mips should thin out: {} vs {c0}", cov(&plain[3].2));
        for l in 1..5 {
            assert!((cov(&kept[l].2) - c0).abs() < 0.05, "level {l}: {} vs {c0}", cov(&kept[l].2));
        }
    }
}

/// Wireframe helpers for debug drawing.
pub mod debug {
    use super::LineVertex;
    use forja_math::{Aabb, Mat4, Vec3, Vec4};

    pub fn line(out: &mut Vec<LineVertex>, a: Vec3, b: Vec3, color: Vec4) {
        out.push(LineVertex { pos: a.to_array(), color: color.to_array() });
        out.push(LineVertex { pos: b.to_array(), color: color.to_array() });
    }

    pub fn aabb(out: &mut Vec<LineVertex>, b: &Aabb, m: &Mat4, color: Vec4) {
        let c = |i: u32| m.transform_point3(Vec3::new(if i & 1 == 0 { b.min.x } else { b.max.x }, if i & 2 == 0 { b.min.y } else { b.max.y }, if i & 4 == 0 { b.min.z } else { b.max.z }));
        for (a, d) in [(0, 1), (2, 3), (4, 5), (6, 7), (0, 2), (1, 3), (4, 6), (5, 7), (0, 4), (1, 5), (2, 6), (3, 7)] {
            line(out, c(a), c(d), color);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn halton_jitter_is_stratified() {
        // The first 16 Halton(2,3) points cover the pixel evenly: every 4x4
        // cell of base 2 and every third of base 3 receives samples.
        let xs: Vec<f32> = (1..=16).map(|i| halton(i, 2)).collect();
        let ys: Vec<f32> = (1..=16).map(|i| halton(i, 3)).collect();
        assert!(xs.iter().chain(&ys).all(|v| (0.0..1.0).contains(v)));
        assert_eq!(halton(1, 2), 0.5);
        assert!((halton(2, 3) - 2.0 / 3.0).abs() < 1e-6);
        let mut cells = [0; 4];
        for x in &xs {
            cells[(x * 4.0) as usize] += 1;
        }
        assert_eq!(cells, [4, 4, 4, 4]);
        let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len() as f32;
        assert!((mean(&xs) - 0.5).abs() < 0.05 && (mean(&ys) - 0.5).abs() < 0.05);
    }

    #[test]
    fn fx_list_toggles_effects() {
        let mut s = RenderSettings::default();
        s.apply_fx("-taa, -ao,+ssr, contact, bogus");
        assert!(!s.taa && !s.ssao && s.ssr && s.contact_shadows);
        s.apply_fx("taa,-contact,-ssr");
        assert!(s.taa && !s.contact_shadows && !s.ssr);
    }

    #[test]
    fn frame_uniforms_match_wgsl_layout() {
        // Every field of `Frame` in frame.wgsl is a vec4, a mat4 or an array
        // of them (no padding), so its size is the sum of the field sizes.
        let src = FRAME.split("struct Frame {").nth(1).and_then(|s| s.split("};").next()).expect("struct Frame");
        let mut size = 0usize;
        for line in src.lines().map(|l| l.split("//").next().unwrap_or("").trim()).filter(|l| !l.is_empty()) {
            let ty = line.split(':').nth(1).expect("field").trim().trim_end_matches(',');
            size += match ty {
                "vec4<f32>" => 16,
                "mat4x4<f32>" => 64,
                t if t.starts_with("array<mat4x4<f32>,") => 64 * t.trim_end_matches('>').rsplit(',').next().unwrap().trim().parse::<usize>().unwrap(),
                t if t.starts_with("array<vec4<f32>,") => 16 * t.trim_end_matches('>').rsplit(',').next().unwrap().trim().parse::<usize>().unwrap(),
                t => panic!("unexpected WGSL field type {t}"),
            };
        }
        assert_eq!(std::mem::size_of::<FrameUniforms>(), size);
        assert_eq!(std::mem::size_of::<lights::GpuLight>(), 64);
        assert_eq!(lights::CLUSTER_STRIDE, 49, "CLUSTER_STRIDE in common.wgsl and cluster.wgsl");
        assert!(COMMON.contains("const CLUSTER_STRIDE: u32 = 49u;"));
    }
}
