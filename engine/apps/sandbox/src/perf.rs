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

use forja_render::{DisplayMode, DynamicResolution, FrameBound, GraphicsConfig, Preset, RENDER_SCALES, RenderStats, Renderer};

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
    /// Window (swapchain) size in physical pixels.
    pub output: (u32, u32),
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
    /// What limited the last frame (CPU work vs GPU timestamps).
    pub bound: FrameBound,
    /// Internal resolution last rendered (logged when it changes).
    pub internal: (u32, u32),
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
            // iGPUs: see `GraphicsConfig::for_adapter`), with the player's
            // display choices (window mode, render scale, frame cap...).
            cfg = GraphicsConfig { auto: true, ..auto_cfg };
            cfg.keep_player_choices(&keep);
            if forja_render::quality::is_intel_integrated(&renderer.adapter_info) {
                log::info!("graphics: Intel integrated GPU profile ({} look at native resolution)", auto_preset.label());
            }
        }
        if let Some(p) = std::env::var("FORJA_PRESET").ok().and_then(|p| parse_preset(&p)) {
            cfg.auto = false;
            cfg.apply_preset(p);
        }
        let timing = renderer.profiler.supported();
        log::info!(
            "graphics: preset {} (auto {} -> {}), render scale {:.0}%, dynamic resolution {}, GPU timestamps {}",
            cfg.preset.label(),
            cfg.auto,
            auto_preset.label(),
            cfg.render_scale * 100.0,
            if !cfg.dynamic_resolution { "off" } else if timing { "on (GPU-bound frames only)" } else { "on but inactive (no timestamps)" },
            if timing { "yes" } else { "no" }
        );
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
            gpu_timing: timing,
            bound: FrameBound::Unknown,
            internal: (0, 0),
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
        self.bound = FrameBound::of(sample.cpu_ms(), self.gpu_ms_of(&sample));
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

    /// GPU time of a frame from timestamps (`None` when unsupported or
    /// not read back yet).
    fn gpu_ms_of(&self, s: &FrameSample) -> Option<f32> {
        (self.gpu_timing && s.gpu_ms > 0.0).then_some(s.gpu_ms)
    }

    /// Internal resolution scale for a `width` x `height` output: the
    /// render scale (1.0 = native), lowered by the dynamic resolution only
    /// while the GPU is the bottleneck (never for CPU-bound frames, never
    /// without timestamps; the frame time is not used as a stand-in).
    pub fn next_scale(&mut self, width: u32, height: u32) -> f32 {
        let gpu = self.gpu_ms_of(&self.last);
        let max = self.cfg.max_scale_for(width, height);
        self.drs.update_capped(gpu, self.last.cpu_ms(), &self.cfg, max)
    }

    /// Logs the internal resolution when it changes (and the first time).
    pub fn note_internal(&mut self, iw: u32, ih: u32, out_w: u32, out_h: u32) {
        if self.internal != (iw, ih) {
            log::info!(
                "internal resolution {iw}x{ih} ({:.0}% of the {out_w}x{out_h} window{}): {}",
                iw as f32 / out_w.max(1) as f32 * 100.0,
                if iw < out_w { ", FSR 1 upscale" } else { ", native, no upscaling" },
                self.drs.reason
            );
            self.internal = (iw, ih);
        }
    }

    pub fn save(&mut self) {
        match std::fs::write(&self.cfg_path, self.cfg.to_toml()) {
            Ok(()) => self.message = Some(format!("Saved {}", self.cfg_path.display())),
            Err(e) => self.message = Some(format!("{}: {e}", self.cfg_path.display())),
        }
    }
}

impl FrameSample {
    /// CPU work of the frame (game, extraction, renderer, UI), without the
    /// wait for the swapchain.
    pub fn cpu_ms(&self) -> f32 {
        self.sim_ms + self.extract_ms + self.render_cpu_ms + self.ui_ms
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
        u.label(format!(
            "internal {}x{} ({:.0}%{}) -> window {}x{} · preset {}",
            st.width,
            st.height,
            s.render_scale * 100.0,
            if s.render_scale < 1.0 { ", FSR 1" } else { ", native" },
            s.output.0,
            s.output.1,
            p.cfg.preset.label()
        ));
        let gpu_txt = if p.gpu_timing && s.gpu_ms > 0.0 { format!("{:.1} ms", s.gpu_ms) } else { "n/a".into() };
        u.label(egui::RichText::new(format!("{} · CPU {:.1} ms / GPU {} · dynamic res {}", p.bound.label(), s.cpu_ms(), gpu_txt, if p.cfg.dynamic_resolution { if p.gpu_timing { "on" } else { "on (inactive: no timestamps)" } } else { "off" })).color(match p.bound {
            FrameBound::Cpu => egui::Color32::from_rgb(240, 190, 90),
            FrameBound::Gpu => egui::Color32::from_rgb(130, 220, 130),
            FrameBound::Unknown => egui::Color32::GRAY,
        }));
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

/// F9: graphics options window. Changes apply at once (texture size on the
/// next map load); "Guardar" writes `data/graphics.toml`.
pub fn draw_options(ctx: &egui::Context, p: &mut PerfState) {
    if !p.options {
        return;
    }
    let mut save = false;
    egui::Window::new("Gráficos (F9)").anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).resizable(false).collapsible(false).show(ctx, |u| {
        graphics_options(u, p);
        u.separator();
        u.horizontal(|u| {
            if u.button("Guardar").clicked() {
                save = true;
            }
            if u.button("Cerrar").clicked() {
                p.options = false;
            }
            if let Some(m) = &p.message {
                u.small(m);
            }
        });
    });
    if save {
        p.save();
    }
}

/// The graphics settings (F9 window and the pause menu's options): preset,
/// display (window mode, v-sync, frame cap), resolution (render scale,
/// dynamic resolution) and, folded, every cost knob.
pub fn graphics_options(u: &mut egui::Ui, p: &mut PerfState) {
    let before = p.cfg.clone();
    u.horizontal_wrapped(|u| {
        u.label("Calidad");
        for pr in Preset::ALL {
            let name = match pr {
                Preset::Low => "Baja",
                Preset::Medium => "Media",
                Preset::High => "Alta",
                Preset::Ultra => "Ultra",
                Preset::Custom => "Personalizada",
            };
            if u.selectable_label(p.cfg.preset == pr && !p.cfg.auto, name).clicked() {
                p.cfg.apply_preset(pr);
                p.cfg.auto = false;
            }
        }
        if u.selectable_label(p.cfg.auto, format!("Auto ({})", p.auto_preset.label())).clicked() {
            p.cfg.auto = true;
            p.cfg.apply_preset(p.auto_preset);
        }
        if p.cfg.preset == Preset::Custom {
            u.small("(personalizada)");
        }
    });
    u.separator();
    let internal = (p.last.stats.width, p.last.stats.height);
    let output = p.last.output;
    let gpu_timing = p.gpu_timing;
    let c = &mut p.cfg;
    egui::Grid::new("display").num_columns(2).striped(true).show(u, |g| {
        g.label("Pantalla");
        g.horizontal(|g| {
            for m in DisplayMode::ALL {
                g.selectable_value(&mut c.display_mode, m, m.label());
            }
        });
        g.end_row();
        g.label("Sincronización vertical");
        g.checkbox(&mut c.vsync, "");
        g.end_row();
        g.label("Límite de FPS");
        g.horizontal(|g| {
            for (n, label) in [(0, "sin límite"), (30, "30"), (60, "60"), (90, "90"), (120, "120"), (144, "144")] {
                g.selectable_value(&mut c.fps_cap, n, label);
            }
        });
        g.end_row();
        g.label("Escala de render");
        g.horizontal(|g| {
            for s in RENDER_SCALES {
                let on = (c.render_scale - s).abs() < 0.005;
                if g.selectable_label(on, format!("{:.0}%", s * 100.0)).clicked() {
                    c.render_scale = s;
                }
            }
        });
        g.end_row();
        g.label("");
        g.small(if c.render_scale < 0.995 { format!("FSR 1 reescala a la ventana · interna {}x{} -> {}x{}", internal.0, internal.1, output.0, output.1) } else { format!("nativa, sin reescalado · {}x{}", output.0, output.1) });
        g.end_row();
        g.label("Resolución dinámica");
        g.checkbox(&mut c.dynamic_resolution, if gpu_timing { "solo si la GPU limita" } else { "sin marcas de tiempo de GPU: no actúa" });
        g.end_row();
        if c.dynamic_resolution {
            g.label("  FPS objetivo");
            g.add(egui::Slider::new(&mut c.target_fps, 30.0..=144.0));
            g.end_row();
            g.label("  Escala mínima");
            g.add(egui::Slider::new(&mut c.min_render_scale, 0.5..=1.0));
            g.end_row();
        }
    });
    let mut custom = false;
    egui::CollapsingHeader::new("Avanzado (coste de cada efecto)").show(u, |u| {
        egui::ScrollArea::vertical().max_height(320.0).show(u, |u| {
            egui::Grid::new("gfx").num_columns(2).striped(true).show(u, |g| {
                g.label("Resolución de sombras");
                g.horizontal(|g| {
                    for r in [1024, 1536, 2048, 4096] {
                        custom |= g.selectable_value(&mut c.shadow_resolution, r, r.to_string()).changed();
                    }
                });
                g.end_row();
                g.label("Cascadas con sombra suave (PCSS)");
                custom |= g.add(egui::Slider::new(&mut c.pcss_cascades, 0..=4)).changed();
                g.end_row();
                g.label("Sombras lejanas cada");
                custom |= g.add(egui::Slider::new(&mut c.far_cascade_interval, 1..=4).suffix(" fotogramas")).changed();
                g.end_row();
                g.label("Distancia de sombras");
                custom |= g.add(egui::Slider::new(&mut c.shadow_distance_scale, 0.5..=1.5)).changed();
                g.end_row();
                g.label("Oclusión ambiental");
                g.horizontal(|g| {
                    custom |= g.checkbox(&mut c.ssao, "sí").changed();
                    custom |= g.checkbox(&mut c.ssao_half_res, "media resolución").changed();
                });
                g.end_row();
                g.label("Reflejos (SSR)");
                g.horizontal(|g| {
                    custom |= g.checkbox(&mut c.ssr, "sí").changed();
                    custom |= g.add(egui::Slider::new(&mut c.ssr_steps, 8..=48).text("pasos")).changed();
                });
                g.end_row();
                g.label("Sombras de contacto");
                custom |= g.checkbox(&mut c.contact_shadows, "").changed();
                g.end_row();
                g.label("Resplandor (bloom)");
                custom |= g.checkbox(&mut c.bloom, "").changed();
                g.end_row();
                g.label("Profundidad de campo (escenas)");
                custom |= g.checkbox(&mut c.dof, "").changed();
                g.end_row();
                g.label("Desenfoque de movimiento");
                g.horizontal(|g| {
                    custom |= g.checkbox(&mut c.motion_blur, "sí").changed();
                    custom |= g.add(egui::Slider::new(&mut c.motion_blur_samples, 4..=24).text("muestras")).changed();
                });
                g.end_row();
                g.label("Niebla volumétrica");
                g.horizontal(|g| {
                    for (n, label) in [(0, "no"), (1, "baja"), (2, "alta"), (3, "ultra")] {
                        custom |= g.selectable_value(&mut c.volumetric_fog, n, label).changed();
                    }
                });
                g.end_row();
                g.label("Oclusión con normales curvadas");
                custom |= g.checkbox(&mut c.bent_normals, "").changed();
                g.end_row();
                g.label("FSR 1 (bajo 100%)");
                custom |= g.checkbox(&mut c.fsr, "").changed();
                g.end_row();
                g.label("Luces por fotograma");
                g.horizontal(|g| {
                    for (n, label) in [(512, "512"), (2048, "2k"), (4096, "4k"), (65535, "todas")] {
                        custom |= g.selectable_value(&mut c.max_lights, n, label).changed();
                    }
                });
                g.end_row();
                g.label("Focos con sombra");
                custom |= g.add(egui::Slider::new(&mut c.spot_shadows, 0..=4)).changed();
                g.end_row();
                g.label("Omitir sombras pequeñas (texels)");
                custom |= g.add(egui::Slider::new(&mut c.shadow_caster_texels, 1.0..=8.0)).changed();
                g.end_row();
                g.label("Detalle de ciudad (HLOD más allá)");
                g.horizontal(|g| {
                    for (n, label) in [(150.0, "150 m"), (250.0, "250 m"), (400.0, "400 m"), (0.0, "todo")] {
                        custom |= g.selectable_value(&mut c.hlod_distance, n, label).changed();
                    }
                });
                g.end_row();
                g.label("Sombras simplificadas desde");
                g.horizontal(|g| {
                    for (n, label) in [(1, "c1"), (2, "c2"), (3, "c3"), (4, "no")] {
                        custom |= g.selectable_value(&mut c.shadow_proxy_cascade, n, label).changed();
                    }
                });
                g.end_row();
                g.label("Detalle de LOD");
                custom |= g.add(egui::Slider::new(&mut c.lod_detail, 0.25..=2.0)).changed();
                g.end_row();
                g.label("Texturas (siguiente mapa)");
                g.horizontal(|g| {
                    for r in [1024, 2048, 4096] {
                        custom |= g.selectable_value(&mut c.max_texture_size, r, r.to_string()).changed();
                    }
                });
                g.end_row();
            });
        });
    });
    if custom {
        c.preset = Preset::Custom;
        c.auto = false;
    }
    if p.cfg != before {
        p.cfg = p.cfg.clone().sanitized();
        if p.cfg.render_scale != before.render_scale || p.cfg.dynamic_resolution != before.dynamic_resolution {
            p.drs = DynamicResolution::new(p.cfg.render_scale);
            p.drs.reason = "changed in the options";
        }
        p.dirty = true;
    }
}

/// Resident memory of this process in MB (Linux: RSS; Windows: working
/// set; 0 elsewhere).
pub fn resident_mb() -> f64 {
    memory_mb().0
}

/// Peak resident memory of this process in MB (Linux: VmHWM; Windows:
/// peak working set; 0 elsewhere).
pub fn peak_resident_mb() -> f64 {
    memory_mb().1
}

/// `(current, peak)` resident memory in MB.
#[cfg(windows)]
fn memory_mb() -> (f64, f64) {
    #[repr(C)]
    #[derive(Default)]
    #[allow(dead_code)] // filled by Windows
    struct Counters {
        cb: u32,
        page_faults: u32,
        peak_working_set: usize,
        working_set: usize,
        quota: [usize; 4],
        pagefile: usize,
        peak_pagefile: usize,
    }
    unsafe extern "system" {
        fn GetCurrentProcess() -> isize;
        fn K32GetProcessMemoryInfo(process: isize, counters: *mut Counters, cb: u32) -> i32;
    }
    let mut c = Counters { cb: std::mem::size_of::<Counters>() as u32, ..Default::default() };
    // SAFETY: kernel32 call on this process's pseudo handle with a
    // PROCESS_MEMORY_COUNTERS of the size it states.
    let cb = c.cb;
    if unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut c, cb) } != 0 {
        return (c.working_set as f64 / 1048576.0, c.peak_working_set as f64 / 1048576.0);
    }
    (0.0, 0.0)
}

/// `(current, peak)` resident memory in MB.
#[cfg(not(windows))]
fn memory_mb() -> (f64, f64) {
    let current = std::fs::read_to_string("/proc/self/statm").ok().and_then(|s| s.split_whitespace().nth(1)?.parse::<f64>().ok()).map_or(0.0, |pages| pages * 4096.0 / 1048576.0);
    let peak = std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| s.lines().find_map(|l| l.strip_prefix("VmHWM:").and_then(|v| v.trim().trim_end_matches("kB").trim().parse::<f64>().ok())))
        .map_or(0.0, |kb| kb / 1024.0);
    (current, peak)
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
/// map), from mimalloc and, on Linux, from glibc.
pub fn trim_heap() {
    // The game's allocator (mimalloc): return freed pages to the OS.
    // SAFETY: mi_collect only releases memory mimalloc no longer uses.
    unsafe {
        libmimalloc_sys::mi_collect(true);
    }
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
