//! Frame profiler overlay (F11), graphics options (F9), dynamic resolution
//! and the CSV frame log.
//!
//! Every frame records the wall time, the CPU time of each game system
//! (from [`forja_core::SystemTimings`]), the renderer's CPU scopes and the
//! GPU time of each render pass (timestamp queries, when the adapter has
//! them), plus draw calls, triangles, instances, lights and a VRAM estimate.

use std::collections::VecDeque;
use std::io::Write;
use std::path::{Path, PathBuf};

use forja_render::{DynamicResolution, GraphicsConfig, Preset, RenderStats, Renderer};

const HISTORY: usize = 240;

/// One frame of measurements.
#[derive(Clone, Debug, Default)]
pub struct FrameSample {
    pub frame_ms: f32,
    pub sim_ms: f32,
    pub extract_ms: f32,
    pub render_cpu_ms: f32,
    pub ui_ms: f32,
    /// Sum of the GPU passes (0 when timestamps are unsupported).
    pub gpu_ms: f32,
    pub systems: Vec<(&'static str, f32)>,
    pub render_scopes: Vec<(&'static str, f32)>,
    pub gpu_passes: Vec<(&'static str, f32)>,
    pub fixed_steps: u32,
    pub stats: RenderStats,
    pub render_scale: f32,
    /// Scenes with most camera triangles `(name, instances, triangles)`
    /// (filled only while the overlay is open).
    pub heavy: Vec<(std::sync::Arc<str>, u32, u64)>,
}

pub struct PerfState {
    pub overlay: bool,
    pub options: bool,
    pub cfg: GraphicsConfig,
    /// Preset chosen by `auto` for this adapter.
    pub auto_preset: Preset,
    /// The renderer must re-apply `cfg` (and the surface its present mode).
    pub dirty: bool,
    pub drs: DynamicResolution,
    pub history: VecDeque<(f32, f32)>,
    pub last: FrameSample,
    pub gpu_timing: bool,
    cfg_path: PathBuf,
    csv: Option<std::io::BufWriter<std::fs::File>>,
    pub csv_path: Option<PathBuf>,
    frame: u64,
    pub message: Option<String>,
}

impl PerfState {
    /// Loads `graphics.toml` (or the defaults) and resolves `auto`.
    pub fn load(cfg_path: &Path, renderer: &Renderer) -> Self {
        let mut cfg = match std::fs::read_to_string(cfg_path) {
            Ok(t) => GraphicsConfig::from_toml(&t).unwrap_or_else(|e| {
                log::warn!("{}: {e}; using defaults", cfg_path.display());
                GraphicsConfig::default()
            }),
            Err(_) => GraphicsConfig::default(),
        };
        let (auto_preset, auto_cfg) = GraphicsConfig::for_adapter(&renderer.adapter_info);
        if cfg.auto {
            let keep = cfg.clone();
            // The preset of this adapter class with its tuning (Intel
            // iGPUs: see `GraphicsConfig::for_adapter`).
            cfg = GraphicsConfig { auto: true, ..auto_cfg };
            if forja_render::quality::is_intel_integrated(&renderer.adapter_info) {
                log::info!("graphics: Intel integrated GPU profile ({} look, {:.1} MP internal, FSR 1 upscale)", auto_preset.label(), cfg.max_render_mpix);
            }
            cfg.vsync = keep.vsync;
            cfg.frame_latency = keep.frame_latency;
            cfg.target_fps = keep.target_fps;
            cfg.profiler_overlay = keep.profiler_overlay;
            cfg.profiler_csv = keep.profiler_csv.clone();
        }
        if let Some(p) = std::env::var("FORJA_PRESET").ok().and_then(|p| parse_preset(&p)) {
            cfg.auto = false;
            cfg.apply_preset(p);
        }
        log::info!("graphics: preset {} (auto {} -> {}), scale {:.2}{}", cfg.preset.label(), cfg.auto, auto_preset.label(), cfg.render_scale, if cfg.dynamic_resolution { " dynamic" } else { "" });
        let csv_path = std::env::var("FORJA_PROFILE_CSV").ok().or_else(|| (!cfg.profiler_csv.is_empty()).then(|| cfg.profiler_csv.clone())).map(PathBuf::from);
        let mut s = Self {
            overlay: cfg.profiler_overlay,
            options: false,
            drs: DynamicResolution::new(cfg.render_scale),
            cfg,
            auto_preset,
            dirty: true,
            history: VecDeque::with_capacity(HISTORY),
            last: FrameSample::default(),
            gpu_timing: renderer.profiler.supported(),
            cfg_path: cfg_path.to_path_buf(),
            csv: None,
            csv_path: None,
            frame: 0,
            message: None,
        };
        if let Some(p) = csv_path {
            s.start_csv(&p);
        }
        s
    }

    pub fn start_csv(&mut self, path: &Path) {
        match std::fs::File::create(path) {
            Ok(f) => {
                let mut w = std::io::BufWriter::new(f);
                let _ = writeln!(w, "frame,frame_ms,sim_ms,extract_ms,render_cpu_ms,ui_ms,gpu_ms,fixed_steps,draws,shadow_draws,instances,shadow_instances,triangles,shadow_triangles,lights,width,height,render_scale,vram_mb,systems,render_scopes,gpu_passes");
                self.csv = Some(w);
                self.csv_path = Some(path.to_path_buf());
                log::info!("profiling to {}", path.display());
            }
            Err(e) => log::warn!("{}: {e}", path.display()),
        }
    }

    pub fn stop_csv(&mut self) {
        if let Some(mut w) = self.csv.take() {
            let _ = w.flush();
        }
    }

    pub fn csv_active(&self) -> bool {
        self.csv.is_some()
    }

    /// Records a finished frame (history graph and CSV).
    pub fn record(&mut self, sample: FrameSample) {
        self.frame += 1;
        if self.history.len() == HISTORY {
            self.history.pop_front();
        }
        self.history.push_back((sample.frame_ms, sample.gpu_ms));
        if let Some(w) = &mut self.csv {
            let list = |v: &[(&'static str, f32)]| v.iter().map(|(n, ms)| format!("{n}:{ms:.3}")).collect::<Vec<_>>().join(";");
            let s = &sample.stats;
            let _ = writeln!(
                w,
                "{},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{},{},{},{},{},{},{},{},{},{},{:.2},{:.0},\"{}\",\"{}\",\"{}\"",
                self.frame,
                sample.frame_ms,
                sample.sim_ms,
                sample.extract_ms,
                sample.render_cpu_ms,
                sample.ui_ms,
                sample.gpu_ms,
                sample.fixed_steps,
                s.draw_calls,
                s.shadow_draws,
                s.instances_drawn,
                s.shadow_instances,
                s.triangles,
                s.shadow_triangles,
                s.lights,
                s.width,
                s.height,
                sample.render_scale,
                s.vram_mb,
                list(&sample.systems),
                list(&sample.render_scopes),
                list(&sample.gpu_passes)
            );
        }
        self.last = sample;
    }

    /// Render scale for the next frame (dynamic resolution from GPU time;
    /// without timestamp queries the frame time stands in for it).
    /// Internal resolution scale for a `width` x `height` output (dynamic
    /// resolution within the preset's pixel budget).
    pub fn next_scale(&mut self, width: u32, height: u32) -> f32 {
        let gpu = if self.gpu_timing { self.last.gpu_ms } else { self.last.frame_ms * 0.9 };
        let max = self.cfg.max_scale_for(width, height);
        self.drs.update_capped(gpu, &self.cfg, max)
    }

    pub fn save(&mut self) {
        match std::fs::write(&self.cfg_path, self.cfg.to_toml()) {
            Ok(()) => self.message = Some(format!("Saved {}", self.cfg_path.display())),
            Err(e) => self.message = Some(format!("{}: {e}", self.cfg_path.display())),
        }
    }
}

pub fn parse_preset(s: &str) -> Option<Preset> {
    Preset::parse(s)
}

/// F11: frame profiler overlay.
pub fn draw_overlay(ctx: &egui::Context, p: &mut PerfState) {
    if !p.overlay {
        return;
    }
    let s = p.last.clone();
    egui::Window::new("Profiler (F11)").anchor(egui::Align2::LEFT_TOP, [12.0, 64.0]).resizable(false).collapsible(true).default_width(340.0).show(ctx, |u| {
        let avg = if p.history.is_empty() { 0.0 } else { p.history.iter().map(|h| h.0).sum::<f32>() / p.history.len() as f32 };
        let worst = p.history.iter().map(|h| h.0).fold(0.0f32, f32::max);
        u.label(egui::RichText::new(format!("{:.0} fps  ·  {:.2} ms avg  ·  {:.1} ms worst", 1000.0 / avg.max(0.01), avg, worst)).strong());
        // Frame time graph: CPU frame (white), GPU (green); lines at 16.7
        // and 33.3 ms.
        let (rect, _) = u.allocate_exact_size(egui::vec2(320.0, 70.0), egui::Sense::hover());
        let painter = u.painter_at(rect);
        painter.rect_filled(rect, 2.0, egui::Color32::from_black_alpha(160));
        let max_ms = 40.0;
        let y = |ms: f32| rect.bottom() - (ms / max_ms).min(1.0) * rect.height();
        for (ms, c) in [(16.7, egui::Color32::from_rgb(90, 160, 90)), (33.3, egui::Color32::from_rgb(170, 90, 60))] {
            painter.line_segment([egui::pos2(rect.left(), y(ms)), egui::pos2(rect.right(), y(ms))], egui::Stroke::new(1.0, c));
        }
        let n = p.history.len().max(2);
        let x = |i: usize| rect.left() + i as f32 / (HISTORY - 1) as f32 * rect.width();
        let pts: Vec<egui::Pos2> = p.history.iter().enumerate().map(|(i, h)| egui::pos2(x(i + HISTORY - n.max(p.history.len())), y(h.0))).collect();
        painter.add(egui::Shape::line(pts, egui::Stroke::new(1.2, egui::Color32::WHITE)));
        if p.gpu_timing {
            let pts: Vec<egui::Pos2> = p.history.iter().enumerate().map(|(i, h)| egui::pos2(x(i + HISTORY - n.max(p.history.len())), y(h.1))).collect();
            painter.add(egui::Shape::line(pts, egui::Stroke::new(1.2, egui::Color32::from_rgb(120, 230, 120))));
        }
        let st = &s.stats;
        u.label(format!("{}x{} (scale {:.2}) · preset {}", st.width, st.height, s.render_scale, p.cfg.preset.label()));
        u.label(format!("draws {} + shadow {} · tris {:.2} M + shadow {:.2} M", st.draw_calls, st.shadow_draws, st.triangles as f64 / 1e6, st.shadow_triangles as f64 / 1e6));
        u.label(format!("instances {} (+{} shadow, {} culled) · lights {}", st.instances_drawn, st.shadow_instances, st.instances_culled, st.lights));
        u.label(format!("static {} parts in {} cells · cascades {:04b} · VRAM ~{:.0} MB", st.static_instances, st.static_cells, st.cascades_updated, st.vram_mb));
        u.separator();
        u.columns(2, |cols| {
            cols[0].label(egui::RichText::new(format!("CPU {:.2} ms", s.sim_ms + s.extract_ms + s.render_cpu_ms + s.ui_ms)).strong());
            let mut sys: Vec<_> = s.systems.iter().filter(|e| e.1 > 0.02).collect();
            sys.sort_by(|a, b| b.1.total_cmp(&a.1));
            cols[0].small(format!("game {:.2} ({} fixed steps)", s.sim_ms, s.fixed_steps));
            for (n, ms) in sys.iter().take(8) {
                cols[0].small(format!("  {n} {ms:.2}"));
            }
            cols[0].small(format!("extract {:.2}", s.extract_ms));
            cols[0].small(format!("render {:.2}", s.render_cpu_ms));
            for (n, ms) in &s.render_scopes {
                cols[0].small(format!("  {n} {ms:.2}"));
            }
            cols[0].small(format!("ui {:.2}", s.ui_ms));
            if p.gpu_timing {
                cols[1].label(egui::RichText::new(format!("GPU {:.2} ms", s.gpu_ms)).strong());
                for (n, ms) in &s.gpu_passes {
                    cols[1].small(format!("{n} {ms:.2}"));
                }
            } else {
                cols[1].label("GPU timing unsupported");
            }
        });
        if !s.heavy.is_empty() {
            u.separator();
            u.small("heaviest scenes (camera):");
            for (name, n, tris) in &s.heavy {
                u.small(format!("  {name} x{n}: {:.2} M tris", *tris as f64 / 1e6));
            }
        }
        u.separator();
        u.horizontal(|u| {
            if p.csv_active() {
                if u.button("Stop CSV").clicked() {
                    p.stop_csv();
                }
                u.small(p.csv_path.as_ref().map(|c| c.display().to_string()).unwrap_or_default());
            } else if u.button("Log CSV").clicked() {
                let name = format!("forja-profile-{}.csv", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0));
                p.start_csv(Path::new(&name));
            }
        });
    });
}

/// F9: graphics options. Changes apply at once (texture size on the next
/// map load); "Save" writes `data/graphics.toml`.
pub fn draw_options(ctx: &egui::Context, p: &mut PerfState) {
    if !p.options {
        return;
    }
    let before = p.cfg.clone();
    let mut save = false;
    egui::Window::new("Graphics (F9)").anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).resizable(false).collapsible(false).show(ctx, |u| {
        u.horizontal(|u| {
            u.label("Preset");
            for pr in Preset::ALL {
                if u.selectable_label(p.cfg.preset == pr, pr.label()).clicked() {
                    p.cfg.apply_preset(pr);
                    p.cfg.auto = false;
                }
            }
            if u.selectable_label(p.cfg.auto, format!("Auto ({})", p.auto_preset.label())).clicked() {
                p.cfg.auto = true;
                p.cfg.apply_preset(p.auto_preset);
            }
        });
        u.separator();
        let c = &mut p.cfg;
        let mut custom = false;
        egui::Grid::new("gfx").num_columns(2).striped(true).show(u, |g| {
            g.label("V-sync");
            g.checkbox(&mut c.vsync, "");
            g.end_row();
            g.label("Target FPS");
            g.add(egui::Slider::new(&mut c.target_fps, 30.0..=240.0));
            g.end_row();
            g.label("Dynamic resolution");
            custom |= g.checkbox(&mut c.dynamic_resolution, "").changed();
            g.end_row();
            g.label("Render scale (max)");
            custom |= g.add(egui::Slider::new(&mut c.render_scale, 0.5..=1.0)).changed();
            g.end_row();
            g.label("Min render scale");
            custom |= g.add(egui::Slider::new(&mut c.min_render_scale, 0.5..=1.0)).changed();
            g.end_row();
            g.label("Shadow resolution");
            g.horizontal(|g| {
                for r in [1024, 1536, 2048, 4096] {
                    custom |= g.selectable_value(&mut c.shadow_resolution, r, r.to_string()).changed();
                }
            });
            g.end_row();
            g.label("Soft shadow cascades (PCSS)");
            custom |= g.add(egui::Slider::new(&mut c.pcss_cascades, 0..=4)).changed();
            g.end_row();
            g.label("Far shadow update every");
            custom |= g.add(egui::Slider::new(&mut c.far_cascade_interval, 1..=4).suffix(" frames")).changed();
            g.end_row();
            g.label("Shadow distance");
            custom |= g.add(egui::Slider::new(&mut c.shadow_distance_scale, 0.5..=1.5)).changed();
            g.end_row();
            g.label("Ambient occlusion");
            g.horizontal(|g| {
                custom |= g.checkbox(&mut c.ssao, "on").changed();
                custom |= g.checkbox(&mut c.ssao_half_res, "half res").changed();
            });
            g.end_row();
            g.label("Reflections (SSR)");
            g.horizontal(|g| {
                custom |= g.checkbox(&mut c.ssr, "on").changed();
                custom |= g.add(egui::Slider::new(&mut c.ssr_steps, 8..=48).text("steps")).changed();
            });
            g.end_row();
            g.label("Contact shadows");
            custom |= g.checkbox(&mut c.contact_shadows, "").changed();
            g.end_row();
            g.label("Bloom");
            custom |= g.checkbox(&mut c.bloom, "").changed();
            g.end_row();
            g.label("Depth of field (cutscenes)");
            custom |= g.checkbox(&mut c.dof, "").changed();
            g.end_row();
            g.label("Motion blur");
            g.horizontal(|g| {
                custom |= g.checkbox(&mut c.motion_blur, "on").changed();
                custom |= g.add(egui::Slider::new(&mut c.motion_blur_samples, 4..=24).text("samples")).changed();
            });
            g.end_row();
            g.label("Volumetric fog");
            g.horizontal(|g| {
                for (n, label) in [(0, "off"), (1, "low"), (2, "high"), (3, "ultra")] {
                    custom |= g.selectable_value(&mut c.volumetric_fog, n, label).changed();
                }
            });
            g.end_row();
            g.label("Bent-normal occlusion");
            custom |= g.checkbox(&mut c.bent_normals, "").changed();
            g.end_row();
            g.label("FSR 1 upscaling");
            custom |= g.checkbox(&mut c.fsr, "").changed();
            g.end_row();
            g.label("Max internal pixels");
            custom |= g.add(egui::Slider::new(&mut c.max_render_mpix, 0.5..=8.3).suffix(" MP")).changed();
            g.end_row();
            g.label("Lights per frame");
            g.horizontal(|g| {
                for (n, label) in [(512, "512"), (2048, "2k"), (4096, "4k"), (65535, "all")] {
                    custom |= g.selectable_value(&mut c.max_lights, n, label).changed();
                }
            });
            g.end_row();
            g.label("Shadowed spot lights");
            custom |= g.add(egui::Slider::new(&mut c.spot_shadows, 0..=4)).changed();
            g.end_row();
            g.label("Small shadow casters (texels)");
            custom |= g.add(egui::Slider::new(&mut c.shadow_caster_texels, 1.0..=8.0)).changed();
            g.end_row();
            g.label("LOD detail");
            custom |= g.add(egui::Slider::new(&mut c.lod_detail, 0.25..=2.0)).changed();
            g.end_row();
            g.label("Texture size (next map)");
            g.horizontal(|g| {
                for r in [1024, 2048, 4096] {
                    custom |= g.selectable_value(&mut c.max_texture_size, r, r.to_string()).changed();
                }
            });
            g.end_row();
        });
        if custom {
            c.preset = Preset::Custom;
            c.auto = false;
        }
        u.separator();
        u.horizontal(|u| {
            if u.button("Save").clicked() {
                save = true;
            }
            if u.button("Close").clicked() {
                p.options = false;
            }
            if let Some(m) = &p.message {
                u.small(m);
            }
        });
    });
    if p.cfg != before {
        p.cfg = p.cfg.clone().sanitized();
        p.drs = DynamicResolution::new(p.cfg.render_scale);
        p.dirty = true;
    }
    if save {
        p.save();
    }
}

/// Resident memory of this process in MB (Linux; 0 elsewhere).
pub fn resident_mb() -> f64 {
    std::fs::read_to_string("/proc/self/statm").ok().and_then(|s| s.split_whitespace().nth(1)?.parse::<f64>().ok()).map_or(0.0, |pages| pages * 4096.0 / 1048576.0)
}

/// GPU bytes the renderer will allocate for `models` (meshes with their
/// LOD index lists; textures with mips, shared buffers once, first level
/// within each preset cap in `caps`, see [`forja_render::texture_cap`]):
/// `(mesh, [texture per cap])`.
pub fn upload_estimate(models: &[std::sync::Arc<forja_assets::SceneData>], caps: &[u32]) -> (u64, Vec<u64>) {
    let mut seen = std::collections::HashSet::new();
    let mut mesh = 0u64;
    let mut tex = vec![0u64; caps.len()];
    for s in models {
        for m in s.meshes.iter().flatten() {
            mesh += (m.positions.len() * if m.is_skinned() { 72 } else { 48 } + m.indices.len() * 4 + m.lods.iter().map(|l| l.len() * 4).sum::<usize>()) as u64;
        }
        for t in &s.textures {
            if t.rgba8.is_empty() || !seen.insert((t.rgba8.as_ptr() as usize, t.srgb)) {
                continue;
            }
            for (k, cap) in caps.iter().enumerate() {
                let (mut w, mut h) = (t.width.max(1), t.height.max(1));
                loop {
                    if w.max(h) <= forja_render::texture_cap(*cap, t.srgb) || (w == 1 && h == 1) {
                        tex[k] += w as u64 * h as u64 * 4;
                    }
                    if w == 1 && h == 1 {
                        break;
                    }
                    (w, h) = ((w / 2).max(1), (h / 2).max(1));
                }
            }
        }
    }
    (mesh, tex)
}

/// Hands freed heap pages back to the system (glibc keeps the temporaries
/// of the city build otherwise: ~1 GB of resident memory on the default
/// map). A no-op elsewhere.
pub fn trim_heap() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    {
        unsafe extern "C" {
            fn malloc_trim(pad: usize) -> i32;
        }
        // SAFETY: malloc_trim only releases free pages of the heap.
        unsafe {
            malloc_trim(0);
        }
    }
}
