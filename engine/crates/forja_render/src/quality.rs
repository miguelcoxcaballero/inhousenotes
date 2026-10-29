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

/// How the game window covers the screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum DisplayMode {
    /// A normal window (75% of the monitor).
    Windowed,
    /// Borderless window covering the monitor at its native resolution.
    #[default]
    Borderless,
    /// Exclusive fullscreen at the monitor's native video mode.
    Fullscreen,
}

impl DisplayMode {
    pub const ALL: [DisplayMode; 3] = [DisplayMode::Windowed, DisplayMode::Borderless, DisplayMode::Fullscreen];

    pub fn label(self) -> &'static str {
        match self {
            DisplayMode::Windowed => "Ventana",
            DisplayMode::Borderless => "Ventana sin bordes",
            DisplayMode::Fullscreen => "Pantalla completa",
        }
    }
}

/// Render scales offered in the options (FSR 1 upscales below 100%).
pub const RENDER_SCALES: [f32; 5] = [1.0, 0.85, 0.75, 0.67, 0.5];

/// Version of the `graphics.toml` layout. Files from before version 2
/// (v0.1.3 and older) had a per-preset internal pixel budget and dynamic
/// resolution on by default: their resolution settings are reset to native.
pub const CONFIG_VERSION: u32 = 2;

/// Every cost knob of the renderer. Serialized as `data/graphics.toml`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GraphicsConfig {
    /// Layout version of the file ([`CONFIG_VERSION`]; 0 when missing).
    #[serde(default)]
    pub version: u32,
    /// `auto` picks a preset from the adapter at startup.
    pub auto: bool,
    pub preset: Preset,
    /// Present with vertical sync (FIFO); off uses Mailbox when available.
    pub vsync: bool,
    /// Frames the CPU may run ahead of the display (1-3).
    pub frame_latency: u32,
    /// Frame rate the dynamic resolution aims for.
    pub target_fps: f32,
    /// Frame rate cap (0: none; v-sync still applies).
    pub fps_cap: u32,
    /// Window, borderless or exclusive fullscreen.
    pub display_mode: DisplayMode,
    /// Lower the internal resolution while the GPU (timestamps) is over
    /// the `target_fps` budget and the CPU is not. Off by default; without
    /// GPU timestamps it stays at `render_scale`.
    pub dynamic_resolution: bool,
    /// Internal resolution as a fraction of the window's physical pixels
    /// (1.0: native, no upscaling; below 1 FSR 1 EASU upscales), and the
    /// ceiling of the dynamic resolution.
    pub render_scale: f32,
    /// Lowest scale the dynamic resolution may use.
    pub min_render_scale: f32,
    /// Most internal pixels (millions) whatever the window size; 0 = no
    /// cap (the default: the game renders at the window's native
    /// resolution unless `render_scale` says otherwise).
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
            version: CONFIG_VERSION,
            auto: true,
            preset: Preset::High,
            vsync: true,
            frame_latency: 2,
            target_fps: 60.0,
            fps_cap: 0,
            display_mode: DisplayMode::Borderless,
            dynamic_resolution: false,
            render_scale: 1.0,
            min_render_scale: 0.75,
            max_render_mpix: 0.0,
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

    /// Sets every cost knob of `p`. Keeps the player's display choices:
    /// vsync, frame cap, window mode, render scale, dynamic resolution,
    /// auto, fps target and the profiler options. Every preset renders at
    /// the window's native resolution (no pixel budget): v0.1.3 capped the
    /// internal image per preset (1.2 MP on Intel iGPUs, ~0.48 of a
    /// 2880x1800 panel) and looked blurry without gaining frame rate.
    pub fn apply_preset(&mut self, p: Preset) {
        self.preset = p;
        let (min_scale, shadow, pcss, far, sdist, ao_half, ssr, ssr_steps, contact, lights, tex, lod) = match p {
            Preset::Low => (0.5f32, 1024, 0, 4, 0.6, true, false, 16, false, 512, 1024, 0.5),
            Preset::Medium => (0.6, 1536, 1, 3, 0.8, true, true, 16, true, 2048, 2048, 0.75),
            Preset::High => (0.75, 2048, 2, 2, 1.0, true, true, 32, true, 4096, 2048, 1.0),
            Preset::Ultra => (0.85, 4096, 4, 1, 1.0, false, true, 48, true, 65535, 4096, 1.5),
            Preset::Custom => return,
        };
        // Floor of the (optional) dynamic resolution.
        self.min_render_scale = min_scale.min(self.render_scale);
        self.max_render_mpix = 0.0;
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
        self.fps_cap = if self.fps_cap == 0 { 0 } else { self.fps_cap.clamp(20, 500) };
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
        if c.version < 2 {
            // v0.1.3 and older: per-preset pixel budgets and dynamic
            // resolution by default made the image blurry; start native.
            c.render_scale = 1.0;
            c.max_render_mpix = 0.0;
            c.dynamic_resolution = false;
            c.version = CONFIG_VERSION;
        }
        if c.preset != Preset::Custom && !c.auto {
            // Presets win over stale individual values unless Custom.
            let keep = c.clone();
            c.apply_preset(keep.preset);
            c.keep_player_choices(&keep);
        }
        Ok(c.sanitized())
    }

    /// Copies the settings that are the player's, not the preset's
    /// (display, frame rate, resolution, profiler) from `from`.
    pub fn keep_player_choices(&mut self, from: &GraphicsConfig) {
        self.vsync = from.vsync;
        self.frame_latency = from.frame_latency;
        self.target_fps = from.target_fps;
        self.fps_cap = from.fps_cap;
        self.display_mode = from.display_mode;
        self.render_scale = from.render_scale;
        self.dynamic_resolution = from.dynamic_resolution;
        self.min_render_scale = self.min_render_scale.min(from.render_scale);
        self.max_render_mpix = from.max_render_mpix;
        self.profiler_overlay = from.profiler_overlay;
        self.profiler_csv = from.profiler_csv.clone();
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
    /// camera stage) at the panel's native resolution (no pixel budget,
    /// no dynamic resolution unless the player turns it on) with the
    /// other bandwidth costs cut: 1536 shadow maps with only the first
    /// cascade PCSS and the far cascades every 4th frame, 12-step SSR,
    /// 2048 lights, one shadowed spot, no motion blur or volumetric froxels
    /// (the analytic fog and lamp glow stay), bent normals on (they cost
    /// ALU, not memory).
    pub fn for_adapter(info: &wgpu::AdapterInfo) -> (Preset, GraphicsConfig) {
        let preset = Self::auto_preset(info);
        let mut c = Self::for_preset(preset);
        if is_intel_integrated(info) {
            c.apply_preset(Preset::Medium);
            c.max_render_mpix = 0.0;
            c.render_scale = 1.0;
            c.min_render_scale = 0.5;
            c.dynamic_resolution = false;
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

/// What limits the frame rate, from the CPU work of a frame and the GPU
/// time of its passes (timestamps).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FrameBound {
    /// No GPU timestamps: cannot tell.
    #[default]
    Unknown,
    Cpu,
    Gpu,
}

impl FrameBound {
    pub fn of(cpu_ms: f32, gpu_ms: Option<f32>) -> FrameBound {
        match gpu_ms {
            Some(g) if g > 0.0 => {
                if g > cpu_ms { FrameBound::Gpu } else { FrameBound::Cpu }
            }
            _ => FrameBound::Unknown,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            FrameBound::Unknown => "unknown (no GPU timestamps)",
            FrameBound::Cpu => "CPU-bound",
            FrameBound::Gpu => "GPU-bound",
        }
    }
}

/// Internal resolution controller. It only ever lowers the resolution when
/// the **GPU** is the bottleneck: the smoothed GPU time (timestamp
/// queries) over the `target_fps` budget while the CPU work of the frame
/// is under it. A CPU-bound frame (draw submission, culling, game systems)
/// would not get faster with fewer pixels, only blurrier, so it never
/// lowers it; without timestamps it stays at the configured scale. Steps
/// of 5% with hysteresis (lowers after ~0.25 s over budget, raises after
/// ~1 s well under it) so render targets are not reallocated every frame.
#[derive(Clone, Debug)]
pub struct DynamicResolution {
    pub scale: f32,
    over: u32,
    under: u32,
    avg_gpu: f32,
    avg_cpu: f32,
    /// Why the last change happened (for the log and F11).
    pub reason: &'static str,
}

impl DynamicResolution {
    pub fn new(scale: f32) -> Self {
        Self { scale, over: 0, under: 0, avg_gpu: 0.0, avg_cpu: 0.0, reason: "start" }
    }

    /// Feeds one frame's GPU time (`None` without timestamps) and CPU
    /// work (ms); returns the scale to use.
    pub fn update(&mut self, gpu_ms: Option<f32>, cpu_ms: f32, cfg: &GraphicsConfig) -> f32 {
        self.update_capped(gpu_ms, cpu_ms, cfg, cfg.render_scale)
    }

    /// [`Self::update`] with the largest scale for the output size
    /// ([`GraphicsConfig::max_scale_for`]).
    pub fn update_capped(&mut self, gpu_ms: Option<f32>, cpu_ms: f32, cfg: &GraphicsConfig, max: f32) -> f32 {
        let max = max.min(cfg.render_scale);
        let min = cfg.min_render_scale.min(max);
        let gpu = gpu_ms.filter(|g| *g > 0.0);
        if !cfg.dynamic_resolution || gpu.is_none() {
            if self.scale != max {
                self.reason = if cfg.dynamic_resolution { "dynamic resolution needs GPU timestamps: fixed scale" } else { "fixed render scale" };
            }
            self.scale = max;
            self.over = 0;
            self.under = 0;
            return max;
        }
        let gpu = gpu.unwrap_or(0.0);
        let smooth = |avg: f32, v: f32| if avg == 0.0 { v } else { avg * 0.85 + v * 0.15 };
        self.avg_gpu = smooth(self.avg_gpu, gpu);
        self.avg_cpu = smooth(self.avg_cpu, cpu_ms.max(0.0));
        // Leave ~10% headroom for the CPU-GPU overlap and the UI.
        let budget = 1000.0 / cfg.target_fps * 0.9;
        let gpu_bound = self.avg_gpu > budget && self.avg_cpu < budget;
        if gpu_bound {
            self.over += 1;
            self.under = 0;
        } else if self.avg_gpu < budget * 0.75 {
            self.under += 1;
            self.over = 0;
        } else {
            self.over = 0;
            self.under = 0;
        }
        let step = 0.05;
        if self.over > 15 && self.scale > min {
            // GPU time scales roughly with pixel count (scale^2).
            let want = self.scale * (budget / self.avg_gpu).sqrt();
            self.scale = (want.max(self.scale - 0.15)).max(min);
            self.scale = (self.scale / step).floor() * step;
            self.scale = self.scale.max(min);
            self.over = 0;
            self.avg_gpu = 0.0;
            self.reason = "GPU-bound: GPU time over budget, CPU under it";
        } else if self.under > 60 && self.scale < max {
            self.scale = (self.scale + step).min(max);
            self.under = 0;
            self.avg_gpu = 0.0;
            self.reason = "GPU under budget";
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
        // Native resolution on every preset, no pixel budget, no DRS.
        for p in Preset::ALL {
            let c = GraphicsConfig::for_preset(p);
            assert_eq!((c.render_scale, c.max_render_mpix, c.dynamic_resolution), (1.0, 0.0, false), "{p:?}");
            assert_eq!(c.max_scale_for(2880, 1800), 1.0);
        }
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
        assert!(!c.motion_blur && c.volumetric_fog == 0 && c.ssao && c.ssr && c.bloom && c.fsr && c.taa);
        // The 2880x1800 panel renders at its native resolution.
        assert_eq!(c.max_scale_for(2880, 1800), 1.0);
        assert!(!c.dynamic_resolution);
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
        // A v0.1.3 file (no version) with the old Intel budget starts native.
        let t = "auto = true\npreset = \"medium\"\nrender_scale = 0.9\nmax_render_mpix = 1.2\nmin_render_scale = 0.35\ndynamic_resolution = true\n";
        let c = GraphicsConfig::from_toml(t).unwrap();
        assert_eq!((c.render_scale, c.max_render_mpix, c.dynamic_resolution, c.version), (1.0, 0.0, false, CONFIG_VERSION));
        // Current files keep the player's choices across a preset.
        let mut c = GraphicsConfig::for_preset(Preset::High);
        c.auto = false;
        c.render_scale = 0.75;
        c.dynamic_resolution = true;
        c.display_mode = DisplayMode::Windowed;
        c.fps_cap = 90;
        let back = GraphicsConfig::from_toml(&c.to_toml()).unwrap();
        assert_eq!((back.render_scale, back.dynamic_resolution, back.display_mode, back.fps_cap), (0.75, true, DisplayMode::Windowed, 90));
        c.apply_preset(Preset::Low);
        assert_eq!(c.render_scale, 0.75, "presets do not touch the render scale");
    }

    #[test]
    fn dynamic_resolution_tracks_budget() {
        let mut cfg = GraphicsConfig::for_preset(Preset::Medium);
        cfg.dynamic_resolution = true;
        let mut d = DynamicResolution::new(cfg.render_scale);
        // GPU-bound: GPU 30 ms, CPU 5 ms at a 60 FPS target.
        for _ in 0..40 {
            d.update(Some(30.0), 5.0, &cfg);
        }
        assert!(d.scale < cfg.render_scale);
        assert!(d.scale >= cfg.min_render_scale);
        let low = d.scale;
        for _ in 0..400 {
            d.update(Some(5.0), 5.0, &cfg);
        }
        assert!(d.scale > low);
        assert!(d.scale <= cfg.render_scale);
    }

    #[test]
    fn dynamic_resolution_ignores_cpu_bound_frames() {
        let mut cfg = GraphicsConfig::for_preset(Preset::Medium);
        cfg.dynamic_resolution = true;
        let mut d = DynamicResolution::new(1.0);
        // ~10 FPS limited by the CPU (90 ms of CPU work, GPU 30 ms): the
        // resolution must stay native.
        for _ in 0..600 {
            assert_eq!(d.update(Some(30.0), 90.0, &cfg), 1.0);
        }
        // Without GPU timestamps it never moves either.
        let mut d = DynamicResolution::new(1.0);
        for _ in 0..600 {
            assert_eq!(d.update(None, 5.0, &cfg), 1.0);
        }
        // Off (the default) holds the configured scale.
        let mut cfg = GraphicsConfig::for_preset(Preset::Medium);
        cfg.render_scale = 0.75;
        let mut d = DynamicResolution::new(1.0);
        assert_eq!(d.update(Some(100.0), 1.0, &cfg), 0.75);
        assert_eq!(FrameBound::of(20.0, Some(8.0)), FrameBound::Cpu);
        assert_eq!(FrameBound::of(5.0, Some(8.0)), FrameBound::Gpu);
        assert_eq!(FrameBound::of(5.0, None), FrameBound::Unknown);
    }
}
