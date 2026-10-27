//! Cinematic look: the camera and film stages that sit after the tone curve.
//!
//! * [`Grade`]: colour grading baked into a 32^3 3D LUT (display-encoded
//!   in and out), applied after tonemapping so it never fights the tone
//!   curve or exposure; `strength` blends it with the identity.
//! * Film grain: luminance-dependent (strongest in the mid-tones, as in
//!   negative film scans), temporal, mostly monochrome.
//! * Optical vignette: the cos^4 falloff of a real lens.
//! * Lateral chromatic aberration: a radial red/blue shift at the edges.
//! * Depth of field ([`DepthOfField`]): thin-lens circle of confusion from
//!   a physical aperture, gathered in HDR before bloom (cutscenes and photo
//!   mode).
//!
//! GPU cost at 2560x1440 on an RTX 3070 class GPU (estimates from the tap
//! counts; measure with timestamps on the target): grade + grain +
//! vignette + aberration share one full-screen pass with 3 colour taps and
//! 1 LUT tap per pixel, ~0.10-0.15 ms; depth of field gathers up to 64
//! colour + depth taps per pixel, ~0.8-1.2 ms, and runs only when enabled.

use forja_math::Vec3;

/// Colour grade in display-encoded (sRGB) space. All terms are subtle
/// offsets around the identity ([`Grade::default`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grade {
    /// White balance shift: + warms (amber), - cools (blue).
    pub temperature: f32,
    /// + magenta, - green.
    pub tint: f32,
    /// Global saturation multiplier.
    pub saturation: f32,
    /// Contrast around mid grey (display-encoded 0.45); 1 = unchanged.
    pub contrast: f32,
    /// Colour added to the shadows (weighted by 1 - luma).
    pub shadows: Vec3,
    /// Colour added to the highlights (weighted by luma).
    pub highlights: Vec3,
    /// Black level (print film never reaches 0), display-encoded.
    pub black: f32,
    /// Saturation removed near white (film's highlight roll-off), 0..1.
    pub highlight_desaturation: f32,
}

impl Default for Grade {
    fn default() -> Self {
        Self { temperature: 0.0, tint: 0.0, saturation: 1.0, contrast: 1.0, shadows: Vec3::ZERO, highlights: Vec3::ZERO, black: 0.0, highlight_desaturation: 0.0 }
    }
}

impl Grade {
    /// A restrained feature-film grade: a touch of warmth, cool shadows
    /// and warm highlights (complementary split toning), a slightly lifted
    /// print black and soft highlight desaturation. Mean luminance stays
    /// within ~1% of the input.
    pub fn cinematic() -> Self {
        Self {
            temperature: 0.025,
            tint: 0.0,
            saturation: 0.96,
            contrast: 1.03,
            shadows: Vec3::new(-0.010, 0.002, 0.014),
            highlights: Vec3::new(0.012, 0.004, -0.010),
            black: 0.012,
            highlight_desaturation: 0.15,
        }
    }

    /// The grade of one display-encoded colour.
    pub fn apply(&self, c: Vec3) -> Vec3 {
        let luma = |c: Vec3| c.dot(Vec3::new(0.2126, 0.7152, 0.0722));
        // White balance as per-channel gains (normalised to keep luma).
        let wb = Vec3::new(1.0 + self.temperature + 0.5 * self.tint, 1.0 - self.tint, 1.0 - self.temperature + 0.5 * self.tint);
        let mut c = c * wb / luma(wb);
        // Contrast around mid grey.
        // (clamped: values pushed below black would otherwise be crushed
        // before the print-black lift).
        c = ((c - Vec3::splat(0.45)) * self.contrast + Vec3::splat(0.45)).max(Vec3::ZERO);
        // Split toning.
        let l = luma(c).clamp(0.0, 1.0);
        c = (c + self.shadows * (1.0 - l) * (1.0 - l) + self.highlights * l * l).max(Vec3::ZERO);
        // Saturation, less of it near white.
        let l = luma(c);
        let sat = self.saturation * (1.0 - self.highlight_desaturation * smoothstep(0.6, 1.0, l));
        c = Vec3::splat(l) + (c - Vec3::splat(l)) * sat;
        // Print black.
        c = Vec3::splat(self.black) + c * (1.0 - self.black);
        c.clamp(Vec3::ZERO, Vec3::ONE)
    }

    /// The grade as an RGBA8 3D LUT of `n`^3 texels (red fastest), inputs
    /// at texel centres `i / (n - 1)`.
    pub fn lut(&self, n: usize) -> Vec<u8> {
        let mut out = Vec::with_capacity(n * n * n * 4);
        let s = (n - 1) as f32;
        for b in 0..n {
            for g in 0..n {
                for r in 0..n {
                    let c = self.apply(Vec3::new(r as f32 / s, g as f32 / s, b as f32 / s));
                    out.extend([(c.x * 255.0 + 0.5) as u8, (c.y * 255.0 + 0.5) as u8, (c.z * 255.0 + 0.5) as u8, 255]);
                }
            }
        }
        out
    }
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Thin-lens depth of field.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DepthOfField {
    /// Distance in focus (m).
    pub focus_distance: f32,
    /// Aperture as an f-number (f/1.4 shallow .. f/16 deep).
    pub f_stop: f32,
    /// Focal length (mm, full-frame 36x24 sensor); `None` derives it from
    /// the camera's vertical field of view.
    pub focal_length_mm: Option<f32>,
    /// Largest blur radius in pixels at 1080p (scaled with resolution).
    pub max_radius_px: f32,
}

impl DepthOfField {
    pub fn new(focus_distance: f32, f_stop: f32) -> Self {
        Self { focus_distance, f_stop, focal_length_mm: None, max_radius_px: 12.0 }
    }

    /// Focal length (m) of a full-frame camera with vertical fov `fov_y`.
    pub fn focal_length(&self, fov_y: f32) -> f32 {
        self.focal_length_mm.map_or(0.012 / (fov_y * 0.5).tan(), |mm| mm * 0.001)
    }

    /// Circle of confusion radius in pixels for a point at `depth` (m), on
    /// a sensor rendered `height` pixels tall: c = A f |z - zf| / (z (zf - f)),
    /// A = f / N (thin lens).
    pub fn coc_radius_px(&self, depth: f32, fov_y: f32, height: f32) -> f32 {
        let (k, _) = self.coc_params(fov_y, height);
        (k * (1.0 - self.focus_distance / depth.max(1e-3))).abs().min(self.max_radius_px * height / 1080.0)
    }

    /// (k, zf) with coc_px(z) = |k (1 - zf / z)|.
    pub fn coc_params(&self, fov_y: f32, height: f32) -> (f32, f32) {
        let f = self.focal_length(fov_y);
        let zf = self.focus_distance.max(f * 1.01);
        let aperture = f / self.f_stop.max(0.5);
        let coc_sensor = aperture * f / (zf - f); // diameter for z -> infinity
        // Radius in pixels of a 24 mm tall sensor.
        (0.5 * coc_sensor / 0.024 * height, zf)
    }
}

/// Post-tonemap camera and film settings of a frame ([`crate::FrameDesc::look`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    pub grade: Grade,
    /// 0 = identity, 1 = full grade.
    pub grade_strength: f32,
    /// Film grain amplitude (display-encoded, at mid grey); 0 = off.
    pub grain: f32,
    /// Grain size in pixels at 1080p.
    pub grain_size: f32,
    /// Optical (cos^4) vignette strength, 0..1, on top of
    /// `Environment::vignette`.
    pub vignette: f32,
    /// Lateral chromatic aberration: red/blue shift in pixels at the image
    /// corners at 1080p; 0 = off.
    pub chromatic_aberration: f32,
    /// Depth of field (cutscenes, photo mode); `None` = everything sharp.
    pub dof: Option<DepthOfField>,
}

impl Default for Look {
    fn default() -> Self {
        Self::cinematic()
    }
}

impl Look {
    /// No grading, grain, aberration or depth of field (the camera pass is
    /// skipped entirely).
    pub fn neutral() -> Self {
        Self { grade: Grade::default(), grade_strength: 0.0, grain: 0.0, grain_size: 1.0, vignette: 0.0, chromatic_aberration: 0.0, dof: None }
    }

    /// The default: subtle feature-film grade, fine grain, a lens vignette
    /// and a hint of aberration at the corners. Everything in focus.
    pub fn cinematic() -> Self {
        Self { grade: Grade::cinematic(), grade_strength: 1.0, grain: 0.016, grain_size: 1.4, vignette: 0.22, chromatic_aberration: 0.9, dof: None }
    }

    /// Photo mode / cutscene: the cinematic look with a physical depth of
    /// field focused at `focus_distance` (m) with aperture `f_stop`.
    pub fn photo(focus_distance: f32, f_stop: f32) -> Self {
        Self { dof: Some(DepthOfField::new(focus_distance, f_stop)), ..Self::cinematic() }
    }

    /// This look on a frame that the night grade (`Environment::night_grade`,
    /// applied right after the tone curve) already grades with strength
    /// `night_grade`: the colour grade fades out so night frames are not
    /// graded twice; grain, vignette, aberration and depth of field stay.
    pub fn under_night_grade(mut self, night_grade: f32) -> Self {
        self.grade_strength *= 1.0 - night_grade.clamp(0.0, 1.0);
        self
    }

    /// True when the LDR camera pass has anything to do.
    pub fn has_camera_pass(&self) -> bool {
        self.grade_strength > 0.0 || self.grain > 0.0 || self.vignette > 0.0 || self.chromatic_aberration > 0.0
    }
}

/// LUT edge length.
pub const LUT_SIZE: usize = 32;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_grade_is_identity_lut() {
        let lut = Grade::default().lut(LUT_SIZE);
        let s = (LUT_SIZE - 1) as f32;
        for (i, px) in lut.chunks_exact(4).enumerate() {
            let (r, g, b) = (i % LUT_SIZE, (i / LUT_SIZE) % LUT_SIZE, i / (LUT_SIZE * LUT_SIZE));
            for (c, v) in [(px[0], r), (px[1], g), (px[2], b)] {
                assert!((c as f32 - v as f32 / s * 255.0).abs() <= 1.0, "texel {i}");
            }
        }
    }

    #[test]
    fn cinematic_grade_is_subtle() {
        let g = Grade::cinematic();
        let luma = |c: Vec3| c.dot(Vec3::new(0.2126, 0.7152, 0.0722));
        // Mid grey keeps its luminance and turns only slightly warm.
        let m = g.apply(Vec3::splat(0.5));
        assert!((luma(m) - 0.5).abs() < 0.02, "{m}");
        // Shadows lean cool, highlights warm.
        let d = g.apply(Vec3::splat(0.1));
        let h = g.apply(Vec3::splat(0.85));
        assert!(d.z > d.x && h.x > h.z, "{d} {h}");
        // Black is lifted to print black, white stays near white.
        assert!(g.apply(Vec3::ZERO).max_element() < 0.04 && g.apply(Vec3::ZERO).min_element() > 0.0);
        assert!(luma(g.apply(Vec3::ONE)) > 0.95);
        // Greys move by less than 0.04 per channel; saturated colours by
        // less than 0.1 (the highlight roll-off deliberately pulls bright
        // saturated colours towards their luma, as print film does).
        for i in 0..=10 {
            let x = i as f32 / 10.0;
            let grey = Vec3::splat(x);
            assert!((g.apply(grey) - grey).abs().max_element() < 0.04, "{grey} -> {}", g.apply(grey));
            for c in [Vec3::new(x, 0.3, 0.2), Vec3::new(0.2, x, 0.6)] {
                assert!((g.apply(c) - c).abs().max_element() < 0.1, "{c} -> {}", g.apply(c));
            }
        }
    }

    #[test]
    fn night_grade_replaces_the_colour_grade() {
        let day = Look::cinematic().under_night_grade(0.0);
        assert_eq!(day, Look::cinematic());
        let night = Look::cinematic().under_night_grade(1.0);
        assert_eq!(night.grade_strength, 0.0);
        assert!(night.grain > 0.0 && night.has_camera_pass());
        assert!((Look::cinematic().under_night_grade(0.7).grade_strength - 0.3).abs() < 1e-6);
    }

    #[test]
    fn thin_lens_coc() {
        // 50 mm f/2 focused at 3 m, background at infinity, 1080 px tall:
        // c = (25 mm) * 50 / 2950 = 0.42 mm -> radius 0.21 / 24 * 1080 = 9.5 px.
        let d = DepthOfField { focal_length_mm: Some(50.0), max_radius_px: 100.0, ..DepthOfField::new(3.0, 2.0) };
        let (k, zf) = d.coc_params(0.0, 1080.0);
        assert!((k - 9.53).abs() < 0.1 && zf == 3.0, "{k}");
        assert!(d.coc_radius_px(3.0, 0.0, 1080.0) < 1e-4);
        assert!((d.coc_radius_px(1e6, 0.0, 1080.0) - k).abs() < 0.01);
        // Foreground at half the focus distance blurs as much as infinity.
        assert!((d.coc_radius_px(1.5, 0.0, 1080.0) - k).abs() < 0.01);
    }
}

// ------------------------------------------------------------------ GPU side

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    a: [f32; 4],
    b: [f32; 4],
}

/// Pipelines and resources of the camera stage (owned by the renderer).
pub(crate) struct CameraFx {
    look_bgl: wgpu::BindGroupLayout,
    camera_pipe: wgpu::RenderPipeline,
    dof_pipe: wgpu::RenderPipeline,
    lut_tex: wgpu::Texture,
    lut_view: wgpu::TextureView,
    lut_grade: Option<Grade>,
    look_buf: wgpu::Buffer,
    dof_buf: wgpu::Buffer,
    sampler: wgpu::Sampler,
}

/// Size-dependent bindings of the camera stage.
pub(crate) struct CameraFxTargets {
    /// Anti-aliased LDR image (input of the camera pass).
    pub ldr2: wgpu::TextureView,
    look_bind: wgpu::BindGroup,
    dof_bind: wgpu::BindGroup,
}

impl CameraFx {
    pub fn new(device: &wgpu::Device, output_format: wgpu::TextureFormat, frame_bgl: &wgpu::BindGroupLayout, taa_bgl: &wgpu::BindGroupLayout) -> Self {
        let tex2d = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
            count: None,
        };
        let look_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("look"),
            entries: &[
                tex2d(0),
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D3, multisampled: false },
                    count: None,
                },
            ],
        });
        let layout = |bgls: &[&wgpu::BindGroupLayout]| {
            let v: Vec<Option<&wgpu::BindGroupLayout>> = bgls.iter().map(|b| Some(*b)).collect();
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &v, immediate_size: 0 })
        };
        let pipe = |module: &wgpu::ShaderModule, layout: &wgpu::PipelineLayout, entry: &str, format: wgpu::TextureFormat, constants: &[(&str, f64)]| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(layout),
                vertex: wgpu::VertexState { module, entry_point: Some("vs_fullscreen"), compilation_options: Default::default(), buffers: &[] },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState { module, entry_point: Some(entry), compilation_options: wgpu::PipelineCompilationOptions { constants, ..Default::default() }, targets: &[Some(format.into())] }),
                multiview_mask: None,
                cache: None,
            })
        };
        let camera = crate::shader(device, "cinematic", include_str!("shaders/cinematic.wgsl"), false);
        let dof = crate::shader(device, "dof", include_str!("shaders/dof.wgsl"), true);
        // A plain UNORM surface gets the sRGB encoding in this last pass.
        let camera_pipe = pipe(&camera, &layout(&[&look_bgl]), "fs_camera", output_format, &[("encode_srgb", (!output_format.is_srgb()) as u32 as f64)]);
        let dof_pipe = pipe(&dof, &layout(&[frame_bgl, taa_bgl]), "fs_dof", crate::HDR_FORMAT, &[]);
        let n = LUT_SIZE as u32;
        let lut_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("grade_lut"),
            size: wgpu::Extent3d { width: n, height: n, depth_or_array_layers: n },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let lut_view = lut_tex.create_view(&Default::default());
        let buf = |label| device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: 32, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("look"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self { look_bgl, camera_pipe, dof_pipe, lut_tex, lut_view, lut_grade: None, look_buf: buf("look"), dof_buf: buf("dof"), sampler }
    }

    pub fn targets(
        &self,
        device: &wgpu::Device,
        output_format: wgpu::TextureFormat,
        (width, height): (u32, u32),
        taa_bgl: &wgpu::BindGroupLayout,
        resolved: &wgpu::TextureView,
        depth: &wgpu::TextureView,
    ) -> CameraFxTargets {
        let ldr2 = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("ldr2"),
                size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: output_format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&Default::default());
        let tv = |binding, view| wgpu::BindGroupEntry { binding, resource: wgpu::BindingResource::TextureView(view) };
        let look_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("look"),
            layout: &self.look_bgl,
            entries: &[
                tv(0, &ldr2),
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: self.look_buf.as_entire_binding() },
                tv(3, &self.lut_view),
            ],
        });
        let dof_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("dof"),
            layout: taa_bgl,
            entries: &[
                tv(0, resolved),
                tv(1, resolved),
                tv(2, depth),
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                wgpu::BindGroupEntry { binding: 4, resource: self.dof_buf.as_entire_binding() },
            ],
        });
        CameraFxTargets { ldr2, look_bind, dof_bind }
    }

    /// Uploads this frame's parameters (and the LUT when the grade
    /// changed). Returns (dof active, camera pass active).
    pub fn prepare(&mut self, queue: &wgpu::Queue, look: &Look, fov_y: f32, (width, height): (u32, u32), frame_index: u32, enabled: bool) -> (bool, bool) {
        if !enabled {
            return (false, false);
        }
        let camera = look.has_camera_pass();
        if camera {
            if self.lut_grade != Some(look.grade) {
                let n = LUT_SIZE as u32;
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo { texture: &self.lut_tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                    &look.grade.lut(LUT_SIZE),
                    wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(n * 4), rows_per_image: Some(n) },
                    wgpu::Extent3d { width: n, height: n, depth_or_array_layers: n },
                );
                self.lut_grade = Some(look.grade);
            }
            let p = Params {
                a: [look.grade_strength, look.grain, look.grain_size, look.vignette],
                b: [look.chromatic_aberration, (frame_index % 4096) as f32, 1.0 / width as f32, 1.0 / height as f32],
            };
            queue.write_buffer(&self.look_buf, 0, bytemuck::bytes_of(&p));
        }
        let mut dof_on = false;
        if let Some(d) = look.dof {
            let (k, zf) = d.coc_params(fov_y, height as f32);
            // Foreground can blur more than the background (k is the CoC
            // at infinity), so the spiral always spans the maximum radius.
            let max_r = d.max_radius_px * height as f32 / 1080.0;
            if max_r >= 0.75 && k >= 0.1 {
                // ~64 taps: the spiral's r^2 grows by 2 * step per tap.
                let step = (max_r * max_r / 128.0).max(0.5);
                let p = Params { a: [k, zf, max_r, step], b: [0.0; 4] };
                queue.write_buffer(&self.dof_buf, 0, bytemuck::bytes_of(&p));
                dof_on = true;
            }
        }
        (dof_on, camera)
    }

    /// Depth of field: `resolved` (HDR) -> `out`.
    pub fn dof_pass(&self, enc: &mut wgpu::CommandEncoder, out: &wgpu::TextureView, frame_bind: &wgpu::BindGroup, t: &CameraFxTargets, timestamps: Option<wgpu::RenderPassTimestampWrites<'_>>) {
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("dof"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: out, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
            depth_stencil_attachment: None,
            timestamp_writes: timestamps,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.dof_pipe);
        pass.set_bind_group(0, frame_bind, &[]);
        pass.set_bind_group(1, &t.dof_bind, &[]);
        pass.draw(0..3, 0..1);
    }

    /// Grade, grain, vignette and aberration: `ldr2` -> `target`.
    pub fn camera_pass(&self, enc: &mut wgpu::CommandEncoder, target: &wgpu::TextureView, t: &CameraFxTargets, timestamps: Option<wgpu::RenderPassTimestampWrites<'_>>) {
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("camera"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: target, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
            depth_stencil_attachment: None,
            timestamp_writes: timestamps,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.camera_pipe);
        pass.set_bind_group(0, &t.look_bind, &[]);
        pass.draw(0..3, 0..1);
    }
}
