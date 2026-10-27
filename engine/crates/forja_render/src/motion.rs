//! Camera + object motion blur (McGuire et al. 2012, Guertin et al. 2014):
//! tile max and neighbour max of the prepass velocities, then a
//! reconstruction gather of the TAA output (`shaders/motion.wgsl`).
//!
//! Runs after TAA on a copy of the resolved image (the TAA history copy,
//! which is identical) and writes back into `resolved`, so depth of field,
//! bloom and tonemapping see the blurred frame while the TAA history stays
//! sharp. Skipped on still frames.

/// Velocity target of the depth prepass: screen motion in pixels per frame.
pub const VELOCITY_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg16Float;
const TILE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg16Float;

/// Exposure of the virtual shutter (s): 1/120 is a 180 degree shutter at
/// 60 FPS, kept constant at other frame rates.
pub const SHUTTER_SECONDS: f32 = 1.0 / 120.0;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct MotionUniforms {
    params: [f32; 4],
}

pub struct MotionBlur {
    bgl: wgpu::BindGroupLayout,
    tile_max: wgpu::RenderPipeline,
    neighbor_max: wgpu::RenderPipeline,
    gather: wgpu::RenderPipeline,
    uniform: wgpu::Buffer,
    sampler: wgpu::Sampler,
    targets: Option<Targets>,
}

struct Targets {
    size: (u32, u32),
    tile: u32,
    tile_view: wgpu::TextureView,
    neighbor_view: wgpu::TextureView,
    tile_bind: wgpu::BindGroup,
    neighbor_bind: wgpu::BindGroup,
    gather_bind: wgpu::BindGroup,
}

/// Tile size K (px) for a viewport height: ~1/60 of the height, 8..32.
pub fn tile_size(height: u32) -> u32 {
    (height / 60).clamp(8, 32)
}

impl MotionBlur {
    pub fn new(device: &wgpu::Device, frame_bgl: &wgpu::BindGroupLayout, hdr_format: wgpu::TextureFormat) -> Self {
        let tex = |binding: u32, sample_type: wgpu::TextureSampleType| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture { sample_type, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
            count: None,
        };
        let float = wgpu::TextureSampleType::Float { filterable: true };
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("motion"),
            entries: &[
                tex(0, float),
                tex(1, wgpu::TextureSampleType::Depth),
                tex(2, float),
                tex(3, float),
                wgpu::BindGroupLayoutEntry { binding: 4, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
            ],
        });
        let module = crate::shader(device, "motion", include_str!("shaders/motion.wgsl"), true);
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("motion"), bind_group_layouts: &[Some(frame_bgl), Some(&bgl)], immediate_size: 0 });
        let pipe = |entry: &str, format: wgpu::TextureFormat| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&layout),
                vertex: wgpu::VertexState { module: &module, entry_point: Some("vs_fullscreen"), compilation_options: Default::default(), buffers: &[] },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState { module: &module, entry_point: Some(entry), compilation_options: Default::default(), targets: &[Some(format.into())] }),
                multiview_mask: None,
                cache: None,
            })
        };
        let uniform = device.create_buffer(&wgpu::BufferDescriptor { label: Some("motion"), size: 16, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor { label: Some("motion"), mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, ..Default::default() });
        Self { tile_max: pipe("fs_tile_max", TILE_FORMAT), neighbor_max: pipe("fs_neighbor_max", TILE_FORMAT), gather: pipe("fs_motion_blur", hdr_format), bgl, uniform, sampler, targets: None }
    }

    /// (Re)creates the tile targets and bind groups for a viewport.
    /// `color` is the sharp TAA output copy that the gather reads.
    pub fn resize(&mut self, device: &wgpu::Device, size: (u32, u32), color: &wgpu::TextureView, depth: &wgpu::TextureView, velocity: &wgpu::TextureView) {
        let k = tile_size(size.1);
        let (tw, th) = (size.0.div_ceil(k), size.1.div_ceil(k));
        let tex = |label: &str| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d { width: tw, height: th, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: TILE_FORMAT,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        let tile_view = tex("motion tiles");
        let neighbor_view = tex("motion neighbours");
        let bind = |aux: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("motion"),
                layout: &self.bgl,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(color) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(depth) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(velocity) },
                    wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(aux) },
                    wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                    wgpu::BindGroupEntry { binding: 5, resource: self.uniform.as_entire_binding() },
                ],
            })
        };
        // The tile pass reads no tile texture (any view that is not its
        // target), the neighbour pass the tiles, the gather the neighbours.
        let tile_bind = bind(&neighbor_view);
        let neighbor_bind = bind(&tile_view);
        let gather_bind = bind(&neighbor_view);
        self.targets = Some(Targets { size, tile: k, tile_view, neighbor_view, tile_bind, neighbor_bind, gather_bind });
    }

    /// Bytes of the tile targets.
    pub fn target_bytes(&self) -> u64 {
        self.targets.as_ref().map_or(0, |t| (t.size.0.div_ceil(t.tile) * t.size.1.div_ceil(t.tile)) as u64 * 8)
    }

    /// Records the three passes; `out` is the resolved image (the gather
    /// reads its copy). `frame_dt` is the real frame time (s).
    #[allow(clippy::too_many_arguments)]
    pub fn encode(
        &self,
        queue: &wgpu::Queue,
        enc: &mut wgpu::CommandEncoder,
        frame_bind: &wgpu::BindGroup,
        out: &wgpu::TextureView,
        samples: u32,
        frame_dt: f32,
        prof: &crate::profiler::GpuProfiler,
    ) {
        let Some(t) = &self.targets else { return };
        let scale = SHUTTER_SECONDS / frame_dt.clamp(1.0 / 240.0, 0.1);
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&MotionUniforms { params: [scale, samples.max(2) as f32, t.tile as f32, t.tile as f32] }));
        let mut pass = |view: &wgpu::TextureView, pipe: &wgpu::RenderPipeline, bind: &wgpu::BindGroup| {
            let mut p = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("motion blur"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: None,
                timestamp_writes: prof.pass("motion blur"),
                occlusion_query_set: None,
                multiview_mask: None,
            });
            p.set_pipeline(pipe);
            p.set_bind_group(0, frame_bind, &[]);
            p.set_bind_group(1, bind, &[]);
            p.draw(0..3, 0..1);
        };
        pass(&t.tile_view, &self.tile_max, &t.tile_bind);
        pass(&t.neighbor_view, &self.neighbor_max, &t.neighbor_bind);
        pass(out, &self.gather, &t.gather_bind);
    }
}
