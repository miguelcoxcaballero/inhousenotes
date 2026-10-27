//! Graphics quality: Low / Medium / High / Ultra presets, automatic choice
//! from the adapter, dynamic resolution and the `data/graphics.toml` file.
//!
//! A preset only sets *costs* (resolutions, sample counts, update rates,
//! budgets). Artistic parameters (exposure, tonemapping, light colors, fog)
//! are never touched, so every preset shows the same picture with more or
//! less detail. See `docs/performance.md` for the per-pass budget.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Preset {
    Low,
    Medium,
    #[default]
    High,
    Ultra,
    /// Values as written in the file (no preset applied).
    Custom,
}

impl Preset {
    pub const ALL: [Preset; 4] = [Preset::Low, Preset::Medium, Preset::High, Preset::Ultra];

    /// `low`, `medium` (`med`), `high` or `ultra`, any case.
    pub fn parse(s: &str) -> Option<Preset> {
        Some(match s.trim().to_lowercase().as_str() {
            "low" => Preset::Low,
            "medium" | "med" => Preset::Medium,
            "high" => Preset::High,
            "ultra" => Preset::Ultra,
            _ => return None,
        })
    }

    pub fn label(self) -> &'static str {
        match self {
            Preset::Low => "Low",
            Preset::Medium => "Medium",
            Preset::High => "High",
            Preset::Ultra => "Ultra",
            Preset::Custom => "Custom",
        }
    }
}

/// Every cost knob of the renderer. Serialized as `data/graphics.toml`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GraphicsConfig {
    /// `auto` picks a preset from the adapter at startup.
    pub auto: bool,
    pub preset: Preset,
    /// Present with vertical sync (FIFO); off uses Mailbox when available.
    pub vsync: bool,
    /// Frames the CPU may run ahead of the display (1-3).
    pub frame_latency: u32,
    /// Frame rate the dynamic resolution aims for.
    pub target_fps: f32,
    /// Scale internal resolution from GPU time to hold `target_fps`.
    pub dynamic_resolution: bool,
    /// Internal resolution scale (fixed, or the start/max when dynamic).
    pub render_scale: f32,
    /// Lowest scale the dynamic resolution may use.
    pub min_render_scale: f32,
    /// Most internal pixels (millions) whatever the window size: a 2880x1800
    /// laptop panel or a 4K monitor renders at this budget and the
    /// upscaler (FSR 1 EASU) fills the window, so a preset costs the same
    /// GPU time on any display. 0 = no cap.
    pub max_render_mpix: f32,
    /// Shadow map size per cascade (texels).
    pub shadow_resolution: u32,
    /// Cascades 0..pcss_cascades use contact-hardening PCSS; the rest a
    /// fixed small PCF (their texels are larger than any penumbra).
    pub pcss_cascades: u32,
    /// Cascade 2 and 3 are re-rendered every N frames (1 = every frame).
    /// Stable texel-snapped cascades make a few frames' staleness invisible.
    pub far_cascade_interval: u32,
    /// Fraction of the scene's shadow distance actually shadowed.
    pub shadow_distance_scale: f32,
    pub ssao: bool,
    /// GTAO at half resolution with a depth-aware upsample.
    pub ssao_half_res: bool,
    pub ssr: bool,
    /// Screen-space reflection ray steps (8-48).
    pub ssr_steps: u32,
    pub contact_shadows: bool,
    pub taa: bool,
    pub bloom: bool,
    /// Depth of field of cutscenes / photo mode (one 64-tap gather pass
    /// at full resolution when a shot asks for it).
    pub dof: bool,
    /// Local lights per frame after frustum culling, closest first (the
    /// light clusters bound the per-pixel cost; far lights also lose their
    /// sprite when cut).
    pub max_lights: u32,
    /// Spot lights with their own shadow map (0-4, 1024² each).
    pub spot_shadows: u32,
    /// Textures larger than this are uploaded from a smaller mip (restart
    /// or map change to apply).
    pub max_texture_size: u32,
    /// Multiplies the LOD triangle budget (lower = coarser LODs sooner).
    pub lod_detail: f32,
    /// Sun shadow casters with a bounding radius under this many cascade
    /// texels are skipped (small facade details in the far cascades).
    pub shadow_caster_texels: f32,
    /// First sun cascade (0-3) where facade detail is replaced by the
    /// city's shadow proxies (building masses); 4 never uses them.
    pub shadow_proxy_cascade: u32,
    /// Distance (m) beyond which city tiles draw their HLOD stand-in
    /// instead of their facade detail (0: never).
    pub hlod_distance: f32,
    /// Camera and object motion blur (McGuire reconstruction filter).
    pub motion_blur: bool,
    /// Gather samples per moving pixel (4-24).
    pub motion_blur_samples: u32,
    /// Froxel volumetric fog with shadowed sun and lamps: 0 off (analytic
    /// height fog only), 1 coarse (16 px x 48 slices, 120 m), 2 (12 px x
    /// 64, 160 m), 3 fine (8 px x 96, 200 m).
    pub volumetric_fog: u32,
    /// GTAO bent normals: cone-traced specular occlusion and bent diffuse
    /// ambient (off: the Lagarde specular occlusion fit).
    pub bent_normals: bool,
    /// Edge-adaptive upscaling (FSR 1 EASU) when the render scale is below
    /// 1; off: bilinear upsample with CAS.
    pub fsr: bool,
    /// Show the frame profiler overlay at start (F11 toggles it).
    pub profiler_overlay: bool,
    /// Write the per-frame profile to this CSV file (empty: off).
    pub profiler_csv: String,
}

impl Default for GraphicsConfig {
    fn default() -> Self {
        let mut c = Self {
            auto: true,
            preset: Preset::High,
            vsync: true,
            frame_latency: 2,
            target_fps: 60.0,
            dynamic_resolution: true,
            render_scale: 1.0,
            min_render_scale: 0.75,
            max_render_mpix: 3.7,
            shadow_resolution: 2048,
            pcss_cascades: 2,
            far_cascade_interval: 2,
            shadow_distance_scale: 1.0,
            ssao: true,
            ssao_half_res: true,
            ssr: true,
            ssr_steps: 32,
            contact_shadows: true,
            taa: true,
            bloom: true,
            dof: true,
            max_lights: 4096,
            spot_shadows: 4,
            max_texture_size: 4096,
            lod_detail: 1.0,
            shadow_caster_texels: 2.5,
            shadow_proxy_cascade: 3,
            hlod_distance: 250.0,
            motion_blur: true,
            motion_blur_samples: 10,
            volumetric_fog: 2,
            bent_normals: true,
            fsr: true,
            profiler_overlay: false,
            profiler_csv: String::new(),
        };
        c.apply_preset(Preset::High);
        c
    }
}

impl GraphicsConfig {
    pub fn for_preset(p: Preset) -> Self {
        let mut c = Self::default();
        c.apply_preset(p);
        c
    }

    /// Sets every cost knob of `p` (keeps vsync, auto, fps target and the
    /// profiler options).
    pub fn apply_preset(&mut self, p: Preset) {
        self.preset = p;
        let (scale, min_scale, dynamic, shadow, pcss, far, sdist, ao_half, ssr, ssr_steps, contact, lights, tex, lod) = match p {
            Preset::Low => (0.75, 0.5, true, 1024, 0, 4, 0.6, true, false, 16, false, 512, 1024, 0.5),
            Preset::Medium => (0.9, 0.6, true, 1536, 1, 3, 0.8, true, true, 16, true, 2048, 2048, 0.75),
            Preset::High => (1.0, 0.75, true, 2048, 2, 2, 1.0, true, true, 32, true, 4096, 2048, 1.0),
            Preset::Ultra => (1.0, 1.0, false, 4096, 4, 1, 1.0, false, true, 48, true, 65535, 4096, 1.5),
            Preset::Custom => return,
        };
        self.render_scale = scale;
        self.min_render_scale = min_scale;
        // Internal pixel budgets: ~1280x800, 1080p, 1440p, 4K.
        self.max_render_mpix = match p {
            Preset::Low => 1.0,
            Preset::Medium => 2.1,
            Preset::High => 3.7,
            _ => 8.3,
        };
        self.dynamic_resolution = dynamic;
        self.shadow_resolution = shadow;
        self.pcss_cascades = pcss;
        self.far_cascade_interval = far;
        self.shadow_distance_scale = sdist;
        self.ssao = true;
        self.ssao_half_res = ao_half;
        self.ssr = ssr;
        self.ssr_steps = ssr_steps;
        self.contact_shadows = contact;
        self.taa = true;
        self.bloom = true;
        // The camera stage (grade LUT, grain, aberration) is one cheap
        // full-screen pass and stays on everywhere; depth of field is a
        // 64-tap gather and is kept for High and Ultra.
        self.dof = matches!(p, Preset::High | Preset::Ultra);
        self.max_lights = lights;
        self.spot_shadows = match p {
            Preset::Low => 0,
            Preset::Medium => 2,
            _ => 4,
        };
        self.max_texture_size = tex;
        self.lod_detail = lod;
        self.shadow_caster_texels = match p {
            Preset::Low => 6.0,
            Preset::Medium => 4.0,
            Preset::High => 2.5,
            _ => 1.5,
        };
        // Facade detail (window modules, walls with openings) in the far
        // cascades costs most of the shadow triangles; the building masses
        // cast the same shadow there.
        self.shadow_proxy_cascade = match p {
            Preset::Low | Preset::Medium => 2,
            _ => 3,
        };
        self.hlod_distance = match p {
            Preset::Low => 150.0,
            Preset::Medium => 200.0,
            Preset::High => 250.0,
            _ => 400.0,
        };
        // Research pass 2 (docs/rendering_research.md 9-12): motion blur off
        // on Low, volumetric fog off on Low (analytic fog), bent normals from
        // Medium, FSR 1 upscaling wherever the render scale drops below 1.
        (self.motion_blur, self.motion_blur_samples, self.volumetric_fog, self.bent_normals) = match p {
            Preset::Low => (false, 6, 0, false),
            Preset::Medium => (true, 6, 1, true),
            Preset::High => (true, 10, 2, true),
            _ => (true, 16, 3, true),
        };
        self.fsr = true;
    }

    /// Clamps every value to what the renderer supports.
    pub fn sanitized(mut self) -> Self {
        self.frame_latency = self.frame_latency.clamp(1, 3);
        self.target_fps = self.target_fps.clamp(20.0, 360.0);
        self.render_scale = self.render_scale.clamp(0.25, 2.0);
        self.min_render_scale = self.min_render_scale.clamp(0.25, self.render_scale);
        self.max_render_mpix = if self.max_render_mpix <= 0.0 { 0.0 } else { self.max_render_mpix.clamp(0.3, 40.0) };
        // Multiples of 512 (1536 is the Medium size).
        self.shadow_resolution = (self.shadow_resolution.clamp(512, 8192).div_ceil(512) * 512).min(8192);
        self.pcss_cascades = self.pcss_cascades.min(4);
        self.far_cascade_interval = self.far_cascade_interval.clamp(1, 8);
        self.shadow_distance_scale = self.shadow_distance_scale.clamp(0.25, 2.0);
        self.ssr_steps = self.ssr_steps.clamp(8, 64);
        self.max_lights = self.max_lights.clamp(16, crate::MAX_LIGHTS as u32);
        self.spot_shadows = self.spot_shadows.min(crate::lights::MAX_SPOT_SHADOWS as u32);
        self.max_texture_size = self.max_texture_size.clamp(256, 16384);
        self.lod_detail = self.lod_detail.clamp(0.1, 4.0);
        self.shadow_caster_texels = self.shadow_caster_texels.clamp(0.5, 16.0);
        self.shadow_proxy_cascade = self.shadow_proxy_cascade.min(4);
        self.hlod_distance = if self.hlod_distance <= 0.0 { 0.0 } else { self.hlod_distance.clamp(60.0, 2000.0) };
        self.motion_blur_samples = self.motion_blur_samples.clamp(4, 24);
        self.volumetric_fog = self.volumetric_fog.min(3);
        self
    }

    pub fn from_toml(text: &str) -> Result<Self, String> {
        let mut c: Self = toml::from_str(text).map_err(|e| e.to_string())?;
        if c.preset != Preset::Custom && !c.auto {
            // Presets win over stale individual values unless Custom.
            let keep = c.clone();
            c.apply_preset(keep.preset);
            c.vsync = keep.vsync;
            c.frame_latency = keep.frame_latency;
            c.target_fps = keep.target_fps;
            c.profiler_overlay = keep.profiler_overlay;
            c.profiler_csv = keep.profiler_csv;
            c.dynamic_resolution = keep.dynamic_resolution;
        }
        Ok(c.sanitized())
    }

    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(self).unwrap_or_default()
    }

    /// Largest render scale for a `width` x `height` output: `render_scale`
    /// within the internal pixel budget (`max_render_mpix`).
    pub fn max_scale_for(&self, width: u32, height: u32) -> f32 {
        let px = width as f32 * height as f32;
        if self.max_render_mpix <= 0.0 || px <= 0.0 {
            return self.render_scale;
        }
        self.render_scale.min((self.max_render_mpix * 1e6 / px).sqrt()).max(0.25)
    }

    /// The automatic configuration for an adapter: [`Self::auto_preset`],
    /// then the tuning of its GPU class. Intel integrated graphics
    /// ([`is_intel_integrated`]) share LPDDR memory bandwidth with the CPU,
    /// so they keep the Medium look (same lighting, AO, reflections, bloom,
    /// camera stage) with the costs that scale with bandwidth cut: a 1.2 MP
    /// internal budget upscaled by FSR 1 (2880x1800 -> ~1390x870), dynamic
    /// resolution down to 35%, 1536 shadow maps with only the first cascade
    /// PCSS and the far cascades every 4th frame, 12-step SSR, 2048 lights,
    /// one shadowed spot, no motion blur or volumetric froxels (the analytic
    /// fog and lamp glow stay), bent normals on (they cost ALU, not memory).
    pub fn for_adapter(info: &wgpu::AdapterInfo) -> (Preset, GraphicsConfig) {
        let preset = Self::auto_preset(info);
        let mut c = Self::for_preset(preset);
        if is_intel_integrated(info) {
            c.apply_preset(Preset::Medium);
            c.max_render_mpix = 1.2;
            c.render_scale = 1.0;
            c.min_render_scale = 0.35;
            c.dynamic_resolution = true;
            c.shadow_resolution = 1536;
            c.pcss_cascades = 1;
            c.far_cascade_interval = 4;
            c.shadow_distance_scale = 0.8;
            c.shadow_caster_texels = 4.0;
            c.ssr_steps = 12;
            c.max_lights = 2048;
            c.spot_shadows = 1;
            c.lod_detail = 0.75;
            c.motion_blur = false;
            c.volumetric_fog = 0;
            c.bent_normals = true;
            c.dof = false;
            c.fsr = true;
        }
        (preset, c)
    }

    /// Preset for an adapter: discrete GPUs get High (dynamic resolution
    /// holds the frame rate on mid-range cards) except old or entry-level
    /// ones and laptop parts with little VRAM (Medium); integrated GPUs
    /// Low, or Medium for the strong ones (Radeon 780M class, Arc, Apple
    /// M); software rasterizers (WARP, lavapipe) and unknown devices Low.
    pub fn auto_preset(info: &wgpu::AdapterInfo) -> Preset {
        let name = info.name.to_lowercase();
        if crate::gpu::is_software(info) {
            return Preset::Low;
        }
        match info.device_type {
            wgpu::DeviceType::DiscreteGpu => {
                // Old, entry-level or 2-4 GB discrete cards (GTX 1060 3/6 GB,
                // RX 470-590, GTX 1650, MX laptop chips).
                let entry = [
                    "gtx 9", "gtx 10", "gt 1030", "gtx 1650", "gtx 1630", "rx 4", "rx 5", "r9 ", "r7 ", "mx1", "mx2", "mx3", "mx4", "mx5", "rx 6300", "rx 6400", "rx 6500", "rx 550", "gtx 750", "gtx 7", "quadro p", "quadro k", "quadro m", "rtx 3050 laptop", "rtx 2050", "arc a3",
                ]
                .iter()
                .any(|k| name.contains(k));
                if entry { Preset::Medium } else { Preset::High }
            }
            wgpu::DeviceType::IntegratedGpu => {
                let strong = ["radeon 780m", "radeon 760m", "radeon 890m", "radeon 880m", "radeon 8060s", "radeon 8050s", "arc", "apple m"].iter().any(|k| name.contains(k));
                if strong { Preset::Medium } else { Preset::Low }
            }
            wgpu::DeviceType::Cpu | wgpu::DeviceType::VirtualGpu | wgpu::DeviceType::Other => Preset::Low,
        }
    }
}

/// Intel integrated graphics (PCI vendor 0x8086): UHD, Iris Xe and the Arc
/// (Xe-LPG) iGPUs of Meteor / Lunar / Arrow Lake laptops.
pub fn is_intel_integrated(info: &wgpu::AdapterInfo) -> bool {
    info.vendor == 0x8086 && info.device_type == wgpu::DeviceType::IntegratedGpu
}

/// Internal resolution controller: keeps the GPU frame time under the
/// target by moving the render scale in steps of 5%, with hysteresis
/// (lowers after ~0.25 s over budget, raises after ~1 s well under it) so
/// render targets are not reallocated every frame.
#[derive(Clone, Debug)]
pub struct DynamicResolution {
    pub scale: f32,
    over: u32,
    under: u32,
    avg_ms: f32,
}

impl DynamicResolution {
    pub fn new(scale: f32) -> Self {
        Self { scale, over: 0, under: 0, avg_ms: 0.0 }
    }

    /// Feeds one frame's GPU time (ms); returns the scale to use.
    pub fn update(&mut self, gpu_ms: f32, cfg: &GraphicsConfig) -> f32 {
        self.update_capped(gpu_ms, cfg, cfg.render_scale)
    }

    /// [`Self::update`] with the largest scale for the output size
    /// ([`GraphicsConfig::max_scale_for`]).
    pub fn update_capped(&mut self, gpu_ms: f32, cfg: &GraphicsConfig, max: f32) -> f32 {
        let max = max.min(cfg.render_scale);
        let min = cfg.min_render_scale.min(max);
        if !cfg.dynamic_resolution {
            self.scale = max;
            return max;
        }
        if gpu_ms <= 0.0 {
            self.scale = self.scale.clamp(min, max);
            return self.scale;
        }
        self.avg_ms = if self.avg_ms == 0.0 { gpu_ms } else { self.avg_ms * 0.85 + gpu_ms * 0.15 };
        // Leave ~10% headroom for the CPU-GPU overlap and the UI.
        let budget = 1000.0 / cfg.target_fps * 0.9;
        if self.avg_ms > budget {
            self.over += 1;
            self.under = 0;
        } else if self.avg_ms < budget * 0.75 {
            self.under += 1;
            self.over = 0;
        } else {
            self.over = 0;
            self.under = 0;
        }
        let step = 0.05;
        if self.over > 15 && self.scale > min {
            // GPU time scales roughly with pixel count (scale^2).
            let want = self.scale * (budget / self.avg_ms).sqrt();
            self.scale = (want.max(self.scale - 0.15)).max(min);
            self.scale = (self.scale / step).floor() * step;
            self.scale = self.scale.max(min);
            self.over = 0;
            self.avg_ms = 0.0;
        } else if self.under > 60 && self.scale < max {
            self.scale = (self.scale + step).min(max);
            self.under = 0;
            self.avg_ms = 0.0;
        }
        self.scale = self.scale.clamp(min, max);
        self.scale
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_order_costs() {
        let l = GraphicsConfig::for_preset(Preset::Low);
        let m = GraphicsConfig::for_preset(Preset::Medium);
        let h = GraphicsConfig::for_preset(Preset::High);
        let u = GraphicsConfig::for_preset(Preset::Ultra);
        assert!(l.shadow_resolution < m.shadow_resolution && m.shadow_resolution < h.shadow_resolution && h.shadow_resolution < u.shadow_resolution);
        assert!(l.max_lights <= m.max_lights && m.max_lights <= h.max_lights);
        assert!(!l.ssr && h.ssr && h.taa && h.ssao);
        assert_eq!(h.render_scale, 1.0);
        assert_eq!(GraphicsConfig::default(), h);
    }

    #[test]
    fn auto_preset_by_adapter() {
        let info = |name: &str, t: wgpu::DeviceType| wgpu::AdapterInfo { name: name.into(), device_type: t, ..wgpu::AdapterInfo::new(wgpu::DeviceType::Other, wgpu::Backend::Dx12) };
        assert_eq!(GraphicsConfig::auto_preset(&info("NVIDIA GeForce GTX 1060 6GB", wgpu::DeviceType::DiscreteGpu)), Preset::Medium);
        assert_eq!(GraphicsConfig::auto_preset(&info("Radeon RX 570 Series", wgpu::DeviceType::DiscreteGpu)), Preset::Medium);
        assert_eq!(GraphicsConfig::auto_preset(&info("NVIDIA GeForce RTX 3070", wgpu::DeviceType::DiscreteGpu)), Preset::High);
        assert_eq!(GraphicsConfig::auto_preset(&info("Intel(R) Iris(R) Xe Graphics", wgpu::DeviceType::IntegratedGpu)), Preset::Low);
        assert_eq!(GraphicsConfig::auto_preset(&info("Microsoft Basic Render Driver", wgpu::DeviceType::Cpu)), Preset::Low);
        // Intel Arc (Meteor Lake) iGPU: the Medium look on a 1.2 MP budget.
        let arc = wgpu::AdapterInfo { vendor: 0x8086, ..info("Intel(R) Arc(TM) Graphics", wgpu::DeviceType::IntegratedGpu) };
        let (_, c) = GraphicsConfig::for_adapter(&arc);
        assert!(!c.motion_blur && c.volumetric_fog == 0 && c.ssao && c.ssr && c.bloom && c.fsr);
        let s = c.max_scale_for(2880, 1800);
        assert!((2880.0 * s * 1800.0 * s) <= 1.25e6 && s > 0.45, "{s}");
        assert_eq!(GraphicsConfig::for_preset(Preset::High).max_scale_for(1920, 1080), 1.0);
    }

    #[test]
    fn toml_roundtrip_and_preset_override() {
        let c = GraphicsConfig::for_preset(Preset::Medium);
        let back = GraphicsConfig::from_toml(&c.to_toml()).unwrap();
        assert_eq!(back.shadow_resolution, c.shadow_resolution);
        // A named preset resets stale values; Custom keeps them.
        let t = "auto = false\npreset = \"low\"\nshadow_resolution = 4096\nvsync = false\n";
        let c = GraphicsConfig::from_toml(t).unwrap();
        assert_eq!(c.shadow_resolution, 1024);
        assert!(!c.vsync);
        let t = "auto = false\npreset = \"custom\"\nshadow_resolution = 3000\n";
        assert_eq!(GraphicsConfig::from_toml(t).unwrap().shadow_resolution, 3072);
    }

    #[test]
    fn dynamic_resolution_tracks_budget() {
        let cfg = GraphicsConfig::for_preset(Preset::Medium);
        let mut d = DynamicResolution::new(cfg.render_scale);
        for _ in 0..40 {
            d.update(30.0, &cfg);
        }
        assert!(d.scale < cfg.render_scale);
        assert!(d.scale >= cfg.min_render_scale);
        let low = d.scale;
        for _ in 0..400 {
            d.update(5.0, &cfg);
        }
        assert!(d.scale > low);
        assert!(d.scale <= cfg.render_scale);
    }
}
