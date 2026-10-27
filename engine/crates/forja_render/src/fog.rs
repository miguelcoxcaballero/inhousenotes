//! Froxel volumetric fog (`shaders/fog.wgsl`) and the sky LUTs
//! (`shaders/skylut.wgsl`). See `docs/rendering_research.md` items 11 and
//! 12 for the references and costs.

const VOLUME_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const LUT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
pub const TRANSMITTANCE_SIZE: (u32, u32) = (256, 64);
pub const SKYVIEW_SIZE: (u32, u32) = (192, 108);

/// Froxel grid of a quality level for a viewport: (tile px, slices, range m).
pub fn volume_quality(level: u32) -> Option<(u32, u32, f32)> {
    match level {
        0 => None,
        1 => Some((16, 48, 120.0)),
        2 => Some((12, 64, 160.0)),
        _ => Some((8, 96, 200.0)),
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct FogUniforms {
    grid: [f32; 4],
    params: [f32; 4],
}

pub struct Volume {
    pub dims: (u32, u32, u32),
    pub range: f32,
    /// Integrated in-scattering (rgb, pre-exposed) and transmittance (a).
    pub integrated: wgpu::TextureView,
    inject_binds: [wgpu::BindGroup; 2],
    integrate_binds: [wgpu::BindGroup; 2],
    bytes: u64,
}

pub struct Fog {
    bgl: wgpu::BindGroupLayout,
    inject: wgpu::ComputePipeline,
    integrate: wgpu::ComputePipeline,
    uniform: wgpu::Buffer,
    pub sampler: wgpu::Sampler,
    pub volume: Option<Volume>,
    /// A 1x1x1 volume (no fog, transmittance 1) bound when the volume is off.
    pub empty: wgpu::TextureView,
    frame: u32,
    history_valid: bool,
    /// Sky LUTs.
    lut_bgl: wgpu::BindGroupLayout,
    transmittance_pipe: wgpu::RenderPipeline,
    skyview_pipe: wgpu::RenderPipeline,
    pub transmittance: wgpu::TextureView,
    pub skyview: wgpu::TextureView,
    lut_bind: wgpu::BindGroup,
    transmittance_done: bool,
}

impl Fog {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, frame_bgl: &wgpu::BindGroupLayout) -> Self {
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fog"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D3, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture { access: wgpu::StorageTextureAccess::WriteOnly, format: VOLUME_FORMAT, view_dimension: wgpu::TextureViewDimension::D3 },
                    count: None,
                },
            ],
        });
        let module = crate::shader(device, "fog", include_str!("shaders/fog.wgsl"), true);
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("fog"), bind_group_layouts: &[Some(frame_bgl), Some(&bgl)], immediate_size: 0 });
        let compute = |entry: &str| device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some(entry), layout: Some(&layout), module: &module, entry_point: Some(entry), compilation_options: Default::default(), cache: None });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor { label: Some("fog"), size: std::mem::size_of::<FogUniforms>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor { label: Some("fog"), mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, ..Default::default() });
        let empty_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("fog-empty"),
            size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: VOLUME_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        // (0, 0, 0, 1) in f16: no in-scattering, full transmittance.
        let one = 0x3c00u16;
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &empty_tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            bytemuck::cast_slice(&[0u16, 0, 0, one]),
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(8), rows_per_image: Some(1) },
            wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
        );
        let empty = empty_tex.create_view(&Default::default());

        // Sky LUTs.
        let lut_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sky-lut"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            ],
        });
        let lut_module = crate::shader(device, "skylut", include_str!("shaders/skylut.wgsl"), true);
        let lut_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("sky-lut"), bind_group_layouts: &[Some(frame_bgl), Some(&lut_bgl)], immediate_size: 0 });
        let lut_pipe = |entry: &str| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&lut_layout),
                vertex: wgpu::VertexState { module: &lut_module, entry_point: Some("vs_fullscreen"), compilation_options: Default::default(), buffers: &[] },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState { module: &lut_module, entry_point: Some(entry), compilation_options: Default::default(), targets: &[Some(LUT_FORMAT.into())] }),
                multiview_mask: None,
                cache: None,
            })
        };
        let lut_tex = |label: &str, (w, h): (u32, u32)| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: LUT_FORMAT,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        let transmittance = lut_tex("sky transmittance", TRANSMITTANCE_SIZE);
        let skyview = lut_tex("sky view", SKYVIEW_SIZE);
        let lut_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sky-lut"),
            layout: &lut_bgl,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&transmittance) }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) }],
        });
        Self {
            inject: compute("cs_inject"),
            integrate: compute("cs_integrate"),
            bgl,
            uniform,
            sampler,
            volume: None,
            empty,
            frame: 0,
            history_valid: false,
            transmittance_pipe: lut_pipe("fs_transmittance"),
            skyview_pipe: lut_pipe("fs_skyview"),
            lut_bgl,
            transmittance,
            skyview,
            lut_bind,
            transmittance_done: false,
        }
    }

    /// The sky-view LUT pass reads the transmittance LUT; a dummy for the
    /// transmittance pass itself (which must not read its own target).
    fn lut_dummy_bind(&self, device: &wgpu::Device) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sky-lut-dummy"),
            layout: &self.lut_bgl,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&self.skyview) }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) }],
        })
    }

    /// Sets the froxel grid for a viewport and quality level (0 = off).
    /// Returns true when the integrated volume view changed (rebind).
    pub fn configure(&mut self, device: &wgpu::Device, size: (u32, u32), level: u32) -> bool {
        let Some((tile, slices, range)) = volume_quality(level) else {
            let changed = self.volume.is_some();
            self.volume = None;
            return changed;
        };
        let dims = (size.0.div_ceil(tile).max(1), size.1.div_ceil(tile).max(1), slices);
        if self.volume.as_ref().is_some_and(|v| v.dims == dims && v.range == range) {
            return false;
        }
        let tex = |label: &str| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width: dims.0, height: dims.1, depth_or_array_layers: dims.2 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D3,
                format: VOLUME_FORMAT,
                usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
        };
        let scatter = [tex("fog scatter a").create_view(&Default::default()), tex("fog scatter b").create_view(&Default::default())];
        let integrated = tex("fog integrated").create_view(&Default::default());
        let bind = |src: &wgpu::TextureView, dst: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("fog"),
                layout: &self.bgl,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: self.uniform.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(src) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                    wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(dst) },
                ],
            })
        };
        // Frame i injects into scatter[i % 2] reading the other as history,
        // then integrates scatter[i % 2].
        let inject_binds = [bind(&scatter[1], &scatter[0]), bind(&scatter[0], &scatter[1])];
        let integrate_binds = [bind(&scatter[0], &integrated), bind(&scatter[1], &integrated)];
        let bytes = dims.0 as u64 * dims.1 as u64 * dims.2 as u64 * 8 * 3;
        self.volume = Some(Volume { dims, range, integrated, inject_binds, integrate_binds, bytes });
        self.history_valid = false;
        true
    }

    pub fn reset_history(&mut self) {
        self.history_valid = false;
    }

    pub fn target_bytes(&self) -> u64 {
        self.volume.as_ref().map_or(0, |v| v.bytes) + ((TRANSMITTANCE_SIZE.0 * TRANSMITTANCE_SIZE.1 + SKYVIEW_SIZE.0 * SKYVIEW_SIZE.1) * 8) as u64
    }

    /// `fog_volume` of the frame uniforms: haze (x, y), range, slices.
    pub fn uniform_params(&self, haze: (f32, f32)) -> [f32; 4] {
        match &self.volume {
            Some(v) => [haze.0, haze.1, v.range, v.dims.2 as f32],
            None => [haze.0, haze.1, 0.0, 1.0],
        }
    }

    /// The integrated volume (or the empty one) for the apply passes.
    pub fn apply_view(&self) -> &wgpu::TextureView {
        self.volume.as_ref().map_or(&self.empty, |v| &v.integrated)
    }

    /// Sky LUTs: the transmittance LUT once, the sky-view LUT every frame.
    pub fn encode_sky(&mut self, device: &wgpu::Device, enc: &mut wgpu::CommandEncoder, frame_bind: &wgpu::BindGroup, prof: &crate::profiler::GpuProfiler) {
        let pass = |enc: &mut wgpu::CommandEncoder, view: &wgpu::TextureView, pipe: &wgpu::RenderPipeline, bind: &wgpu::BindGroup| {
            let mut p = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("sky lut"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: None,
                timestamp_writes: prof.pass("sky lut"),
                occlusion_query_set: None,
                multiview_mask: None,
            });
            p.set_pipeline(pipe);
            p.set_bind_group(0, frame_bind, &[]);
            p.set_bind_group(1, bind, &[]);
            p.draw(0..3, 0..1);
        };
        if !self.transmittance_done {
            let dummy = self.lut_dummy_bind(device);
            pass(enc, &self.transmittance, &self.transmittance_pipe, &dummy);
            self.transmittance_done = true;
        }
        pass(enc, &self.skyview, &self.skyview_pipe, &self.lut_bind);
    }

    /// Injection and integration of the froxel volume (after the shadow
    /// maps and light clusters of this frame).
    pub fn encode_volume(&mut self, queue: &wgpu::Queue, enc: &mut wgpu::CommandEncoder, frame_bind: &wgpu::BindGroup, prof: &crate::profiler::GpuProfiler) {
        let Some(v) = &self.volume else { return };
        self.frame = self.frame.wrapping_add(1);
        let i = (self.frame & 1) as usize;
        // Golden-ratio depth jitter; 12% of each frame once history exists.
        let jitter = (self.frame as f32 * 0.618_034).fract();
        let u = FogUniforms {
            grid: [v.dims.0 as f32, v.dims.1 as f32, v.dims.2 as f32, v.range],
            params: [if self.history_valid { 0.12 } else { 1.0 }, jitter, self.history_valid as u32 as f32, 0.0],
        };
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&u));
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("fog inject"), timestamp_writes: prof.compute_pass("volumetric fog") });
            pass.set_pipeline(&self.inject);
            pass.set_bind_group(0, frame_bind, &[]);
            pass.set_bind_group(1, &v.inject_binds[i], &[]);
            pass.dispatch_workgroups(v.dims.0.div_ceil(8), v.dims.1.div_ceil(8), v.dims.2);
        }
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("fog integrate"), timestamp_writes: prof.compute_pass("volumetric fog") });
            pass.set_pipeline(&self.integrate);
            pass.set_bind_group(0, frame_bind, &[]);
            pass.set_bind_group(1, &v.integrate_binds[i], &[]);
            pass.dispatch_workgroups(v.dims.0.div_ceil(8), v.dims.1.div_ceil(8), 1);
        }
        self.history_valid = true;
    }
}
