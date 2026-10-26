//! # forja_render
//!
//! Modern forward+ renderer on wgpu. On Windows it runs on Direct3D 12
//! (Vulkan as fallback). Frame graph:
//!
//! 1. **Shadows**: 4 stable cascades (texel-snapped bounding spheres) into a
//!    depth array, culled per cascade.
//! 2. **Main**: reverse-Z HDR pass; PBR (GGX, height-correlated Smith,
//!    Schlick) with normal/metal-rough/occlusion/emissive maps, cascaded PCF,
//!    analytic sky ambient and reflections, point lights, height fog. Static,
//!    double-sided, GPU-skinned and blended variants.
//! 3. **Sky**: per-pixel Rayleigh + Mie single scattering at the far plane.
//! 4. **Post**: bloom chain, AgX tonemapping, vignette, FXAA.
//!
//! The renderer knows nothing about worlds or gameplay: callers upload
//! [`SceneData`] once ([`Renderer::upload_scene`]) and submit a [`FrameDesc`]
//! with instances every frame.

pub mod atmosphere;
pub mod camera;
pub mod env;

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forja_assets::{AlphaMode, MaterialData, SceneData, TextureData};
use forja_math::{Aabb, Frustum, Mat4, Vec3, Vec4};
use wgpu::util::DeviceExt;

pub use camera::Camera;
pub use wgpu;

const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const SHADOW_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const BLOOM_MIPS: usize = 6;
pub const MAX_LIGHTS: usize = 64;

const COMMON: &str = include_str!("shaders/common.wgsl");

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuLight {
    pos_radius: [f32; 4],
    color: [f32; 4],
}

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
    lights: [GpuLight; MAX_LIGHTS],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct MaterialUniforms {
    base_color: [f32; 4],
    emissive: [f32; 4],
    params: [f32; 4],
    layers: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PostUniforms {
    texel: [f32; 4],
    params: [f32; 4],
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

#[derive(Clone, Copy, Debug)]
pub struct PointLight {
    pub position: Vec3,
    pub radius: f32,
    pub color: Vec3,
    pub intensity: f32,
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
}

impl DrawInstance {
    pub fn new(model: ModelHandle, transform: Mat4) -> Self {
        Self { model, transform, nodes: None, joints: None, tint: Vec4::ONE, cast_shadows: true }
    }
}

#[derive(Clone, Debug)]
pub struct Environment {
    /// Direction toward the sun.
    pub sun_dir: Vec3,
    pub sun_illuminance: f32,
    pub exposure: f32,
    pub fog_density: f32,
    pub fog_falloff: f32,
    pub bloom: f32,
    pub vignette: f32,
    pub shadow_distance: f32,
    /// Light with the loaded HDRI ([`Renderer::set_environment`]) instead of
    /// the analytic atmosphere. Its sun drives the shadows.
    pub use_ibl: bool,
}

impl Default for Environment {
    fn default() -> Self {
        Self {
            sun_dir: Vec3::new(0.35, 0.6, 0.25).normalize(),
            sun_illuminance: 14.0,
            exposure: 0.55,
            fog_density: 0.0012,
            fog_falloff: 0.012,
            bloom: 0.04,
            vignette: 0.35,
            shadow_distance: 220.0,
            use_ibl: true,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct FrameDesc {
    pub camera: Camera,
    pub env: Environment,
    pub instances: Vec<DrawInstance>,
    pub lights: Vec<PointLight>,
    pub lines: Vec<LineVertex>,
    pub time: f32,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct RenderStats {
    pub draw_calls: u32,
    pub instances_drawn: u32,
    pub instances_culled: u32,
    pub triangles: u64,
    pub shadow_draws: u32,
    pub cpu_ms: f32,
}

struct GpuMesh {
    vertex: wgpu::Buffer,
    skin: Option<wgpu::Buffer>,
    index: wgpu::Buffer,
    index_count: u32,
    /// Simplified index buffers (LOD 1..).
    lods: Vec<(wgpu::Buffer, u32)>,
    material: u32,
    aabb: Aabb,
}

impl GpuMesh {
    fn indices(&self, lod: u32) -> (&wgpu::Buffer, u32) {
        match lod {
            0 => (&self.index, self.index_count),
            l => self.lods.get(l as usize - 1).map(|(b, c)| (b, *c)).unwrap_or((&self.index, self.index_count)),
        }
    }
}

struct GpuMaterial {
    bind_group: wgpu::BindGroup,
    blend: bool,
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
}

struct Targets {
    width: u32,
    height: u32,
    hdr: wgpu::TextureView,
    depth: wgpu::TextureView,
    ldr: wgpu::TextureView,
    bloom: Vec<(wgpu::TextureView, u32, u32)>,
    post_bind: PostBinds,
}

struct PostBinds {
    down: Vec<wgpu::BindGroup>,
    up: Vec<wgpu::BindGroup>,
    tonemap: wgpu::BindGroup,
    fxaa: wgpu::BindGroup,
    uniforms: Vec<wgpu::Buffer>,
}

struct Pipelines {
    opaque: wgpu::RenderPipeline,
    double_sided: wgpu::RenderPipeline,
    skinned: wgpu::RenderPipeline,
    blend: wgpu::RenderPipeline,
    shadow: wgpu::RenderPipeline,
    shadow_skinned: wgpu::RenderPipeline,
    sky: wgpu::RenderPipeline,
    lines: wgpu::RenderPipeline,
    down: wgpu::RenderPipeline,
    up: wgpu::RenderPipeline,
    tonemap: wgpu::RenderPipeline,
    fxaa: wgpu::RenderPipeline,
}

struct EnvInfo {
    sun_dir: Vec3,
    sun_color: Vec3,
    sh: [Vec3; 9],
    max_mip: f32,
    zenith: Vec3,
    horizon: Vec3,
    ground: Vec3,
}

pub struct Renderer {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub adapter_info: wgpu::AdapterInfo,
    pub output_format: wgpu::TextureFormat,
    pipelines: Pipelines,
    frame_bgl: wgpu::BindGroupLayout,
    material_bgl: wgpu::BindGroupLayout,
    joints_bgl: wgpu::BindGroupLayout,
    post_bgl: wgpu::BindGroupLayout,
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
    env: Option<EnvInfo>,
    meshes: Vec<GpuMesh>,
    materials: Vec<GpuMaterial>,
    models: Vec<GpuModel>,
    instance_buf: wgpu::Buffer,
    instance_cap: u64,
    joint_buf: wgpu::Buffer,
    joint_cap: u64,
    joint_bind: wgpu::BindGroup,
    line_buf: wgpu::Buffer,
    line_cap: u64,
    targets: Option<Targets>,
    pub stats: RenderStats,
}

/// DX12 first on Windows (Vulkan fallback); the platform's primary APIs
/// elsewhere. `WGPU_BACKEND` overrides it.
pub fn default_backends() -> wgpu::Backends {
    if cfg!(windows) { wgpu::Backends::DX12 | wgpu::Backends::VULKAN } else { wgpu::Backends::PRIMARY }
}

/// Instance bound to a window system display (needed by Wayland/GL; harmless elsewhere).
pub fn create_instance_with_display(display: Box<dyn wgpu::wgt::WgpuHasDisplayHandle>) -> wgpu::Instance {
    let mut desc = wgpu::InstanceDescriptor::new_with_display_handle(display);
    desc.backends = default_backends();
    wgpu::Instance::new(desc.with_env())
}

/// Instance for offscreen rendering or platforms that need no display handle
/// (Windows, macOS, X11 through Vulkan).
pub fn create_instance() -> wgpu::Instance {
    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
    desc.backends = default_backends();
    wgpu::Instance::new(desc.with_env())
}

fn shader(device: &wgpu::Device, label: &str, src: &str, with_common: bool) -> wgpu::ShaderModule {
    let code = if with_common { format!("{COMMON}\n{src}") } else { src.to_string() };
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
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: surface,
            ..Default::default()
        }))
        .map_err(|e| format!("no GPU adapter: {e}"))?;
        let adapter_info = adapter.get_info();
        log::info!("GPU: {} ({:?})", adapter_info.name, adapter_info.backend);
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("forja"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default().using_resolution(adapter.limits()),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
        }))
        .map_err(|e| format!("device: {e}"))?;
        let output_format = surface
            .map(|s| {
                let caps = s.get_capabilities(&adapter);
                caps.formats.iter().copied().find(|f| f.is_srgb()).unwrap_or(caps.formats[0])
            })
            .unwrap_or(wgpu::TextureFormat::Rgba8UnormSrgb);
        Ok(Self::with_device(device, queue, adapter_info, output_format))
    }

    pub fn with_device(device: wgpu::Device, queue: wgpu::Queue, adapter_info: wgpu::AdapterInfo, output_format: wgpu::TextureFormat) -> Self {
        let frame_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("frame"),
            entries: &[
                uniform_entry(0, wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2Array, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison), count: None },
                texture_entry(3),
                sampler_entry(4),
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
            entries: &[texture_entry(0), sampler_entry(1), uniform_entry(2, wgpu::ShaderStages::FRAGMENT), texture_entry(3)],
        });

        let pbr = shader(&device, "pbr", include_str!("shaders/pbr.wgsl"), true);
        let sky = shader(&device, "sky", include_str!("shaders/sky.wgsl"), true);
        let lines = shader(&device, "lines", include_str!("shaders/lines.wgsl"), true);
        let shadow = shader(&device, "shadow", include_str!("shaders/shadow.wgsl"), false);
        let post = shader(&device, "post", include_str!("shaders/post.wgsl"), false);

        let layout = |bgls: &[&wgpu::BindGroupLayout]| {
            let v: Vec<Option<&wgpu::BindGroupLayout>> = bgls.iter().map(|b| Some(*b)).collect();
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &v, immediate_size: 0 })
        };
        let scene_layout = layout(&[&frame_bgl, &material_bgl, &joints_bgl]);
        let frame_layout = layout(&[&frame_bgl]);
        let shadow_layout = layout(&[&cascade_bgl, &joints_bgl]);
        let post_layout = layout(&[&post_bgl]);

        let depth_state = |write: bool| wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(write),
            depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
            stencil: Default::default(),
            bias: Default::default(),
        };
        let scene_pipeline = |label: &str, vs: &str, skinned: bool, cull: Option<wgpu::Face>, blend: bool| {
            let buffers = vertex_layouts(skinned);
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&scene_layout),
                vertex: wgpu::VertexState { module: &pbr, entry_point: Some(vs), compilation_options: Default::default(), buffers: &buffers },
                primitive: wgpu::PrimitiveState { cull_mode: cull, ..Default::default() },
                depth_stencil: Some(depth_state(!blend)),
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

        let shadow_pipeline = |vs: &str, skinned: bool| {
            let buffers = vertex_layouts(skinned);
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("shadow"),
                layout: Some(&shadow_layout),
                vertex: wgpu::VertexState { module: &shadow, entry_point: Some(vs), compilation_options: Default::default(), buffers: &buffers },
                primitive: wgpu::PrimitiveState { cull_mode: None, unclipped_depth: false, ..Default::default() },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: SHADOW_FORMAT,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::LessEqual),
                    stencil: Default::default(),
                    bias: wgpu::DepthBiasState { constant: 2, slope_scale: 2.0, clamp: 0.0 },
                }),
                multisample: Default::default(),
                fragment: None,
                multiview_mask: None,
                cache: None,
            })
        };
        let shadow_static = shadow_pipeline("vs_main", false);
        let shadow_skinned = shadow_pipeline("vs_skinned", true);

        let sky_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sky"),
            layout: Some(&frame_layout),
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
        let post_pipeline = |entry: &str, format: wgpu::TextureFormat, additive: bool| {
            let blend = additive.then_some(wgpu::BlendState {
                color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
                alpha: wgpu::BlendComponent::OVER,
            });
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
            shadow: shadow_static,
            shadow_skinned,
            sky: sky_pipeline,
            lines: lines_pipeline,
            down: post_pipeline("fs_downsample", HDR_FORMAT, false),
            up: post_pipeline("fs_upsample", HDR_FORMAT, true),
            tonemap: post_pipeline("fs_tonemap", wgpu::TextureFormat::Rgba8UnormSrgb, false),
            fxaa: post_pipeline("fs_fxaa", output_format, false),
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
        let frame_bind = make_frame_bind(&device, &frame_bgl, &frame_buf, &shadow_view, &shadow_sampler, &env_view, &linear_sampler);
        let cascade_bufs: Vec<wgpu::Buffer> = (0..4)
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
        let mut r = Self {
            device,
            queue,
            adapter_info,
            output_format,
            pipelines,
            frame_bgl,
            material_bgl,
            joints_bgl,
            post_bgl,
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
            env: None,
            meshes: Vec::new(),
            materials: Vec::new(),
            models: Vec::new(),
            instance_buf,
            instance_cap,
            joint_buf,
            joint_cap,
            joint_bind,
            line_buf,
            line_cap,
            targets: None,
            stats: RenderStats::default(),
        };
        r.resize(1280, 720);
        r
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
        self.frame_bind = make_frame_bind(&self.device, &self.frame_bgl, &self.frame_buf, &self.shadow_view, &self.shadow_sampler, &self.env_view, &self.linear_sampler);
        let horizon = (0..8).map(|i| {
            let a = i as f32 / 8.0 * std::f32::consts::TAU;
            e.irradiance(Vec3::new(a.cos(), 0.0, a.sin()))
        }).sum::<Vec3>() / 8.0;
        self.env = Some(EnvInfo {
            sun_dir: e.sun_dir,
            sun_color: e.sun_color,
            sh: e.sh,
            max_mip: (e.mips.len() - 1) as f32,
            zenith: e.irradiance(Vec3::Y),
            horizon,
            ground: e.irradiance(Vec3::NEG_Y),
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
        let tex = |label: &str, w: u32, h: u32, format: wgpu::TextureFormat| {
            self.device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        let hdr = tex("hdr", width, height, HDR_FORMAT);
        let depth = tex("depth", width, height, DEPTH_FORMAT);
        let ldr = tex("ldr", width, height, wgpu::TextureFormat::Rgba8UnormSrgb);
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
                contents: bytemuck::bytes_of(&PostUniforms { texel: [texel[0], texel[1], 0.0, 0.0], params }),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            })
        };
        let bind = |src: &wgpu::TextureView, u: &wgpu::Buffer, extra: &wgpu::TextureView| {
            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("post"),
                layout: &self.post_bgl,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(src) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.clamp_sampler) },
                    wgpu::BindGroupEntry { binding: 2, resource: u.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(extra) },
                ],
            })
        };
        // Passes that render into bloom mips must not also bind mip 0.
        let dummy = tex("dummy", 1, 1, HDR_FORMAT);
        let mut uniforms = Vec::new();
        let mut down = Vec::new();
        for i in 0..BLOOM_MIPS {
            let (src, sw, sh) = if i == 0 { (&hdr, width, height) } else { (&bloom[i - 1].0, bloom[i - 1].1, bloom[i - 1].2) };
            let u = mk_uniform([1.0 / sw as f32, 1.0 / sh as f32], [0.0, 0.0, 0.0, if i == 0 { 1.0 } else { 0.0 }]);
            down.push(bind(src, &u, &dummy));
            uniforms.push(u);
        }
        let mut up = Vec::new();
        for i in (1..BLOOM_MIPS).rev() {
            let (src, sw, sh) = (&bloom[i].0, bloom[i].1, bloom[i].2);
            let u = mk_uniform([1.0 / sw as f32, 1.0 / sh as f32], [0.0; 4]);
            up.push(bind(src, &u, &dummy));
            uniforms.push(u);
        }
        // Tonemap uniform is rewritten each frame (exposure, bloom, vignette).
        let tu = mk_uniform([1.0 / width as f32, 1.0 / height as f32], [1.0, 0.04, 0.3, 0.0]);
        let tonemap = bind(&hdr, &tu, &bloom[0].0);
        uniforms.push(tu);
        let fu = mk_uniform([1.0 / width as f32, 1.0 / height as f32], [0.0; 4]);
        let fxaa = bind(&ldr, &fu, &dummy);
        uniforms.push(fu);
        self.targets = Some(Targets { width, height, hdr, depth, ldr, bloom, post_bind: PostBinds { down, up, tonemap, fxaa, uniforms } });
    }

    // ---------------------------------------------------------------- upload

    fn upload_texture(&self, t: &TextureData) -> wgpu::TextureView {
        let format = if t.srgb { wgpu::TextureFormat::Rgba8UnormSrgb } else { wgpu::TextureFormat::Rgba8Unorm };
        let mips = generate_mips(t);
        let tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(&t.name),
            size: wgpu::Extent3d { width: t.width, height: t.height, depth_or_array_layers: 1 },
            mip_level_count: mips.len() as u32,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        for (level, (w, h, data)) in mips.iter().enumerate() {
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: level as u32, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                data,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * w), rows_per_image: Some(*h) },
                wgpu::Extent3d { width: *w, height: *h, depth_or_array_layers: 1 },
            );
        }
        tex.create_view(&Default::default())
    }

    fn solid(&self, rgba: [u8; 4], srgb: bool) -> wgpu::TextureView {
        self.upload_texture(&TextureData { name: "solid".into(), width: 1, height: 1, rgba8: rgba.to_vec(), srgb })
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
            layers: [m.clearcoat, m.clearcoat_roughness, m.transmission, 0.0],
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
        GpuMaterial { bind_group, blend: m.alpha == AlphaMode::Blend || m.transmission > 0.0, double_sided: m.double_sided }
    }

    /// Uploads a scene once; draw it with [`DrawInstance`]s.
    pub fn upload_scene(&mut self, scene: &SceneData) -> ModelHandle {
        let textures: Vec<wgpu::TextureView> = scene.textures.iter().map(|t| self.upload_texture(t)).collect();
        let defaults = [self.solid([255; 4], true), self.solid([255; 4], false), self.solid([128, 128, 255, 255], false)];
        let mat_base = self.materials.len() as u32;
        for m in &scene.materials {
            let gm = self.upload_material(m, &textures, &defaults);
            self.materials.push(gm);
        }
        if scene.materials.is_empty() {
            let gm = self.upload_material(&MaterialData::default(), &textures, &defaults);
            self.materials.push(gm);
        }
        let mesh_base = self.meshes.len() as u32;
        let mut mesh_ids: Vec<Vec<u32>> = Vec::new();
        for prims in &scene.meshes {
            let mut ids = Vec::new();
            for p in prims {
                let verts: Vec<Vertex> = (0..p.positions.len())
                    .map(|i| Vertex { pos: p.positions[i].to_array(), normal: p.normals[i].to_array(), uv: p.uvs[i].to_array(), tangent: p.tangents[i].to_array() })
                    .collect();
                let vertex = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("vb"), contents: bytemuck::cast_slice(&verts), usage: wgpu::BufferUsages::VERTEX });
                let index = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("ib"), contents: bytemuck::cast_slice(&p.indices), usage: wgpu::BufferUsages::INDEX });
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
                let lods = p
                    .lods
                    .iter()
                    .map(|l| (self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("lod"), contents: bytemuck::cast_slice(l), usage: wgpu::BufferUsages::INDEX }), l.len() as u32))
                    .collect();
                ids.push(self.meshes.len() as u32);
                self.meshes.push(GpuMesh { vertex, skin, index, index_count: p.indices.len() as u32, lods, material, aabb: p.aabb });
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
        self.models.push(GpuModel { parts, rest, bounds: scene.bounds() });
        handle
    }

    pub fn model_bounds(&self, m: ModelHandle) -> Aabb {
        self.models[m.0 as usize].bounds
    }

    // ---------------------------------------------------------------- render

    /// Renders a frame into `target` (a surface texture view or offscreen
    /// texture in [`Renderer::output_format`]).
    pub fn render(&mut self, frame: &FrameDesc, target: &wgpu::TextureView) {
        let t0 = std::time::Instant::now();
        let (width, height) = self.size();
        let cam = Camera { aspect: width as f32 / height as f32, ..frame.camera };
        let env = &frame.env;
        let ibl = self.env.as_ref().filter(|_| env.use_ibl);
        let (sun, sky) = match ibl {
            Some(e) => (e.sun_dir, atmosphere::SkyLighting { sun_color: e.sun_color, zenith: e.zenith, horizon: e.horizon, ground: e.ground }),
            None => {
                let sun = env.sun_dir.normalize();
                (sun, atmosphere::sky_lighting(sun, env.sun_illuminance, Vec3::new(0.25, 0.24, 0.22)))
            }
        };
        let mut sh = [[0.0f32; 4]; 9];
        if let Some(e) = ibl {
            for k in 0..9 {
                sh[k] = e.sh[k].extend(0.0).to_array();
            }
        }
        let view = cam.view();
        let view_proj = cam.view_proj();
        let splits = camera::cascade_splits(cam.near.max(0.1), env.shadow_distance, 0.8);
        let mut cascade_vp = [Mat4::IDENTITY; 4];
        let mut prev = cam.near;
        let caster_reach = 400.0;
        for (i, s) in splits.iter().enumerate() {
            cascade_vp[i] = camera::cascade_matrix(&cam, prev, *s, sun, self.shadow_size, caster_reach);
            prev = *s;
        }
        let mut lights = [GpuLight::zeroed(); MAX_LIGHTS];
        let mut sorted: Vec<&PointLight> = frame.lights.iter().collect();
        sorted.sort_by(|a, b| (a.position - cam.position).length_squared().total_cmp(&(b.position - cam.position).length_squared()));
        for (i, l) in sorted.iter().take(MAX_LIGHTS).enumerate() {
            lights[i] = GpuLight { pos_radius: [l.position.x, l.position.y, l.position.z, l.radius], color: [l.color.x, l.color.y, l.color.z, l.intensity] };
        }
        let uniforms = FrameUniforms {
            view_proj: view_proj.to_cols_array_2d(),
            view: view.to_cols_array_2d(),
            inv_view_proj: view_proj.inverse().to_cols_array_2d(),
            camera_pos: [cam.position.x, cam.position.y, cam.position.z, frame.time],
            sun_dir: [sun.x, sun.y, sun.z, env.sun_illuminance],
            sun_color: [sky.sun_color.x, sky.sun_color.y, sky.sun_color.z, env.exposure],
            sky_zenith: sky.zenith.extend(0.0).to_array(),
            sky_horizon: sky.horizon.extend(env.fog_density).to_array(),
            ground_color: sky.ground.extend(env.fog_falloff).to_array(),
            cascade_splits: splits,
            cascade_vp: cascade_vp.map(|m| m.to_cols_array_2d()),
            params: [sorted.len().min(MAX_LIGHTS) as f32, 1.0 / self.shadow_size as f32, env.bloom, 0.0],
            screen: [width as f32, height as f32, 1.0 / width as f32, 1.0 / height as f32],
            sh,
            env_params: [ibl.is_some() as u32 as f32, ibl.map_or(0.0, |e| e.max_mip), 1.0, 0.0],
            lights,
        };
        self.queue.write_buffer(&self.frame_buf, 0, bytemuck::bytes_of(&uniforms));
        for (i, m) in cascade_vp.iter().enumerate() {
            self.queue.write_buffer(&self.cascade_bufs[i], 0, bytemuck::bytes_of(&m.to_cols_array_2d()));
        }

        // Build instance batches: main view + one list per cascade.
        let frustum = Frustum::from_view_proj(&view_proj);
        let tan_half_fov = (cam.fov_y * 0.5).tan();
        let cascade_planes: Vec<[Vec4; 6]> = cascade_vp.iter().map(camera::frustum_planes_standard).collect();
        let mut joints: Vec<[[f32; 4]; 4]> = Vec::new();
        // (mesh, skinned, joint offset, instance raw)
        let mut main: Vec<(u32, InstanceRaw)> = Vec::new();
        let mut shadow: [Vec<(u32, InstanceRaw)>; 4] = Default::default();
        let mut culled = 0u32;
        for inst in &frame.instances {
            let Some(model) = self.models.get(inst.model.0 as usize) else { continue };
            let joint_offset = if let Some(j) = &inst.joints {
                let off = joints.len() as u32;
                joints.extend(j.iter().map(|m| m.to_cols_array_2d()));
                off
            } else {
                0
            };
            for part in &model.parts {
                let mesh = &self.meshes[part.mesh as usize];
                let skinned = mesh.skin.is_some() && inst.joints.is_some();
                let node_m = inst.nodes.as_ref().and_then(|n| n.get(part.node).copied()).unwrap_or(model.rest[part.node]);
                let world = if skinned { inst.transform } else { inst.transform * node_m };
                let bounds = if skinned { model.bounds.expanded(0.5) } else { mesh.aabb };
                let wb = transform_aabb(&bounds, &world);
                // LOD from screen-space triangle density: the finest level
                // with at most one triangle per two covered pixels.
                let coverage = wb.half_extents().length() / ((wb.center() - cam.position).length().max(0.1) * tan_half_fov);
                let pixels = std::f32::consts::PI * (coverage * height as f32 * 0.5).powi(2);
                let budget = (pixels * 0.5) as u32;
                let mut lod = 0;
                while lod < mesh.lods.len() as u32 && mesh.indices(lod).1 / 3 > budget {
                    lod += 1;
                }
                let raw = InstanceRaw { model: world.to_cols_array_2d(), tint: inst.tint.to_array(), extra: [joint_offset, skinned as u32, lod, 0] };
                if frustum.intersects_aabb(&wb) {
                    main.push((part.mesh, raw));
                } else {
                    culled += 1;
                }
                if inst.cast_shadows && !self.materials[mesh.material as usize].blend {
                    let mut sraw = raw;
                    sraw.extra[2] = (lod + 1).min(mesh.lods.len() as u32);
                    for (c, planes) in cascade_planes.iter().enumerate() {
                        if aabb_in_planes(&wb, planes) {
                            shadow[c].push((part.mesh, sraw));
                        }
                    }
                }
            }
        }
        // Sort for batching: by pipeline then mesh.
        // Skinned meshes without joints fall back to the static pipeline (bind pose).
        let key = |r: &Renderer, mesh: u32, raw: &InstanceRaw| {
            let m = &r.meshes[mesh as usize];
            let mat = &r.materials[m.material as usize];
            let pipe = if mat.blend { 3 } else if raw.extra[1] == 1 { 2 } else if mat.double_sided { 1 } else { 0 };
            (pipe, m.material, mesh, raw.extra[2])
        };
        main.sort_by_key(|(m, raw)| key(self, *m, raw));
        for s in &mut shadow {
            s.sort_by_key(|(m, raw)| (raw.extra[1], *m, raw.extra[2]));
        }
        let mut all: Vec<InstanceRaw> = Vec::with_capacity(main.len() + shadow.iter().map(Vec::len).sum::<usize>());
        all.extend(main.iter().map(|(_, r)| *r));
        let mut shadow_ranges = [0u32; 4];
        for (c, s) in shadow.iter().enumerate() {
            shadow_ranges[c] = all.len() as u32;
            all.extend(s.iter().map(|(_, r)| *r));
        }
        let bytes = (all.len() * std::mem::size_of::<InstanceRaw>()) as u64;
        if bytes > self.instance_cap {
            self.instance_cap = bytes.next_power_of_two();
            self.instance_buf = self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("instances"), size: self.instance_cap, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        }
        if !all.is_empty() {
            self.queue.write_buffer(&self.instance_buf, 0, bytemuck::cast_slice(&all));
        }
        if joints.is_empty() {
            joints.push(Mat4::IDENTITY.to_cols_array_2d());
        }
        let jbytes = (joints.len() * 64) as u64;
        if jbytes > self.joint_cap {
            self.joint_cap = jbytes.next_power_of_two();
            self.joint_buf = self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("joints"), size: self.joint_cap, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
            self.joint_bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("joints"), layout: &self.joints_bgl, entries: &[wgpu::BindGroupEntry { binding: 0, resource: self.joint_buf.as_entire_binding() }] });
        }
        self.queue.write_buffer(&self.joint_buf, 0, bytemuck::cast_slice(&joints));
        let line_bytes = (frame.lines.len() * std::mem::size_of::<LineVertex>()) as u64;
        if line_bytes > self.line_cap {
            self.line_cap = line_bytes.next_power_of_two();
            self.line_buf = self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("lines"), size: self.line_cap, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        }
        if !frame.lines.is_empty() {
            self.queue.write_buffer(&self.line_buf, 0, bytemuck::cast_slice(&frame.lines));
        }
        let t = self.targets.as_ref().expect("resized");
        let pu = &t.post_bind.uniforms;
        let tonemap_u = &pu[pu.len() - 2];
        self.queue.write_buffer(
            tonemap_u,
            0,
            bytemuck::bytes_of(&PostUniforms { texel: [1.0 / width as f32, 1.0 / height as f32, 0.0, 0.0], params: [env.exposure, env.bloom, env.vignette, 0.0] }),
        );

        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame") });
        let mut stats = RenderStats { instances_culled: culled, ..Default::default() };
        // 1. Shadows.
        for c in 0..4 {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shadow"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.shadow_layers[c],
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.cascade_binds[c], &[]);
            pass.set_bind_group(1, &self.joint_bind, &[]);
            pass.set_vertex_buffer(1, self.instance_buf.slice(..));
            let base = shadow_ranges[c];
            let list = &shadow[c];
            let mut i = 0;
            let mut skinned_bound: Option<bool> = None;
            while i < list.len() {
                let mesh_id = list[i].0;
                let sk = list[i].1.extra[1] == 1;
                let lod = list[i].1.extra[2];
                let mut j = i + 1;
                while j < list.len() && list[j].0 == mesh_id && (list[j].1.extra[1] == 1) == sk && list[j].1.extra[2] == lod {
                    j += 1;
                }
                let mesh = &self.meshes[mesh_id as usize];
                if skinned_bound != Some(sk) {
                    pass.set_pipeline(if sk { &self.pipelines.shadow_skinned } else { &self.pipelines.shadow });
                    skinned_bound = Some(sk);
                }
                pass.set_vertex_buffer(0, mesh.vertex.slice(..));
                if sk && let Some(s) = &mesh.skin {
                    pass.set_vertex_buffer(2, s.slice(..));
                }
                let (ib, count) = mesh.indices(lod);
                pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..count, 0, base + i as u32..base + j as u32);
                stats.shadow_draws += 1;
                i = j;
            }
        }
        // 2. Main HDR pass + sky + lines.
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("main"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &t.hdr,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &t.depth,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(0.0), store: wgpu::StoreOp::Store }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.frame_bind, &[]);
            pass.set_bind_group(2, &self.joint_bind, &[]);
            pass.set_vertex_buffer(1, self.instance_buf.slice(..));
            let mut i = 0;
            let mut bound_pipe = usize::MAX;
            let mut bound_mat = u32::MAX;
            // Opaque first; the sky goes after opaque and before blended.
            let mut sky_drawn = false;
            while i <= main.len() {
                let blend_start = i == main.len() || key(self, main[i].0, &main[i].1).0 == 3;
                if blend_start && !sky_drawn {
                    pass.set_pipeline(&self.pipelines.sky);
                    pass.draw(0..3, 0..1);
                    bound_pipe = usize::MAX;
                    sky_drawn = true;
                }
                if i == main.len() {
                    break;
                }
                let mesh_id = main[i].0;
                let k = key(self, mesh_id, &main[i].1);
                let mut j = i + 1;
                while j < main.len() && key(self, main[j].0, &main[j].1) == k {
                    j += 1;
                }
                let (pipe, mat, _, lod) = k;
                if pipe != bound_pipe {
                    pass.set_pipeline(match pipe {
                        0 => &self.pipelines.opaque,
                        1 => &self.pipelines.double_sided,
                        2 => &self.pipelines.skinned,
                        _ => &self.pipelines.blend,
                    });
                    pass.set_bind_group(0, &self.frame_bind, &[]);
                    pass.set_bind_group(2, &self.joint_bind, &[]);
                    bound_pipe = pipe;
                    bound_mat = u32::MAX;
                }
                if mat != bound_mat {
                    pass.set_bind_group(1, &self.materials[mat as usize].bind_group, &[]);
                    bound_mat = mat;
                }
                let mesh = &self.meshes[mesh_id as usize];
                pass.set_vertex_buffer(0, mesh.vertex.slice(..));
                if pipe == 2 && let Some(s) = &mesh.skin {
                    pass.set_vertex_buffer(2, s.slice(..));
                }
                let (ib, count) = mesh.indices(lod);
                pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..count, 0, i as u32..j as u32);
                stats.draw_calls += 1;
                stats.instances_drawn += (j - i) as u32;
                stats.triangles += (count / 3) as u64 * (j - i) as u64;
                i = j;
            }
            if !frame.lines.is_empty() {
                pass.set_pipeline(&self.pipelines.lines);
                pass.set_bind_group(0, &self.frame_bind, &[]);
                pass.set_vertex_buffer(0, self.line_buf.slice(..));
                pass.draw(0..frame.lines.len() as u32, 0..1);
            }
        }
        // 3. Bloom down/up chain.
        let fullscreen = |enc: &mut wgpu::CommandEncoder, view: &wgpu::TextureView, pipe: &wgpu::RenderPipeline, bind: &wgpu::BindGroup, load: wgpu::LoadOp<wgpu::Color>| {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("post"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load, store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(pipe);
            pass.set_bind_group(0, bind, &[]);
            pass.draw(0..3, 0..1);
        };
        for i in 0..BLOOM_MIPS {
            fullscreen(&mut enc, &t.bloom[i].0, &self.pipelines.down, &t.post_bind.down[i], wgpu::LoadOp::Clear(wgpu::Color::BLACK));
        }
        for (k, i) in (1..BLOOM_MIPS).rev().enumerate() {
            fullscreen(&mut enc, &t.bloom[i - 1].0, &self.pipelines.up, &t.post_bind.up[k], wgpu::LoadOp::Load);
        }
        // 4. Tonemap and FXAA.
        fullscreen(&mut enc, &t.ldr, &self.pipelines.tonemap, &t.post_bind.tonemap, wgpu::LoadOp::Clear(wgpu::Color::BLACK));
        fullscreen(&mut enc, target, &self.pipelines.fxaa, &t.post_bind.fxaa, wgpu::LoadOp::Clear(wgpu::Color::BLACK));
        self.queue.submit([enc.finish()]);
        stats.cpu_ms = t0.elapsed().as_secs_f32() * 1000.0;
        self.stats = stats;
    }

    /// Renders offscreen and reads back RGBA8 pixels (screenshots, tests).
    pub fn render_to_image(&mut self, frame: &FrameDesc, width: u32, height: u32) -> image::RgbaImage {
        let tex = self.render_offscreen(frame, width, height);
        self.read_image(&tex)
    }

    /// Renders into a new texture (so callers can draw UI on top before reading it back).
    pub fn render_offscreen(&mut self, frame: &FrameDesc, width: u32, height: u32) -> wgpu::Texture {
        self.resize(width, height);
        let tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("offscreen"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.output_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        self.render(frame, &tex.create_view(&Default::default()));
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

fn make_frame_bind(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    frame_buf: &wgpu::Buffer,
    shadow_view: &wgpu::TextureView,
    shadow_sampler: &wgpu::Sampler,
    env_view: &wgpu::TextureView,
    env_sampler: &wgpu::Sampler,
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
        ],
    })
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
fn generate_mips(t: &TextureData) -> Vec<(u32, u32, Vec<u8>)> {
    let to_lin: Vec<f32> = (0..256).map(|i| {
        let c = i as f32 / 255.0;
        if !t.srgb { c } else if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    }).collect();
    let to_enc = |v: f32| -> u8 {
        let v = v.clamp(0.0, 1.0);
        let e = if !t.srgb { v } else if v <= 0.0031308 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
        (e * 255.0 + 0.5) as u8
    };
    let mut out = vec![(t.width, t.height, t.rgba8.clone())];
    let (mut w, mut h) = (t.width, t.height);
    while w > 1 || h > 1 {
        let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
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
        out.push((nw, nh, next));
        w = nw;
        h = nh;
    }
    out
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
