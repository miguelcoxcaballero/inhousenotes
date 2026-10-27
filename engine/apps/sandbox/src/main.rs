//! Forja sandbox.
//!
//! ```text
//! cargo run --release -p sandbox -- --map kit
//! cargo run --release -p sandbox -- --map osm --screenshot shot.png --script drive
//! ```
//! Maps: `kit` (Kenney city), `osm` (Miami Beach), `track`, `terrain`.
//!
//! Performance: `--preset low|medium|high|ultra` (or `FORJA_PRESET`),
//! `--bench N` renders N real-time frames headless after the warm-up and
//! prints CPU/GPU timings (`--csv file.csv` logs every frame). In the game,
//! F11 shows the frame profiler and F9 the graphics options
//! (`data/graphics.toml`).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use forja_core::App;
use forja_input::{ActionMap, Input};
use forja_math::{Vec2, Vec3};
use forja_render::{DrawInstance, FrameDesc, ModelHandle, Renderer};
use sandbox::game::{self, Hud, Intent, MapKind, Paths, RenderScene};
use sandbox::perf::{self, FrameSample, PerfState};
use sandbox::ui;
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::PhysicalKey;
use winit::window::{CursorGrabMode, Window, WindowId};

struct Args {
    map: MapKind,
    screenshot: Option<PathBuf>,
    script: String,
    frames: usize,
    width: u32,
    height: u32,
    hours: Option<f32>,
    camera: Option<(Vec3, Vec3)>,
    preset: Option<forja_render::Preset>,
    bench: usize,
    csv: Option<PathBuf>,
}

fn parse_vec3(s: &str) -> Option<Vec3> {
    let v: Vec<f32> = s.split(',').filter_map(|x| x.trim().parse().ok()).collect();
    (v.len() == 3).then(|| Vec3::new(v[0], v[1], v[2]))
}

fn parse_args() -> Args {
    let mut a = Args { map: MapKind::Spain, screenshot: None, script: "idle".into(), frames: 90, width: 1600, height: 900, hours: None, camera: None, preset: None, bench: 0, csv: None };
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    let mut eye = None;
    let mut look = None;
    while i < args.len() {
        let v = args.get(i + 1).cloned().unwrap_or_default();
        match args[i].as_str() {
            "--map" => a.map = MapKind::parse(&v).unwrap_or_else(|| panic!("unknown map {v}")),
            "--screenshot" => a.screenshot = Some(v.into()),
            "--script" => a.script = v,
            "--frames" => a.frames = v.parse().expect("frames"),
            "--size" => {
                let (w, h) = v.split_once('x').expect("WxH");
                a.width = w.parse().expect("width");
                a.height = h.parse().expect("height");
            }
            "--time" => a.hours = v.parse().ok(),
            "--eye" => eye = parse_vec3(&v),
            "--look" => look = parse_vec3(&v),
            "--preset" => a.preset = Some(perf::parse_preset(&v).unwrap_or_else(|| panic!("unknown preset {v}"))),
            "--bench" => a.bench = v.parse().expect("bench frames"),
            "--csv" => a.csv = Some(v.into()),
            _ => {
                eprintln!("unknown argument {}", args[i]);
                std::process::exit(2);
            }
        }
        i += 2;
    }
    if let (Some(e), Some(l)) = (eye, look) {
        a.camera = Some((e, l));
    }
    a
}

fn paths() -> Paths {
    let root = forja_assets::manifest::find_engine_root().unwrap_or_else(|| PathBuf::from("."));
    Paths { data: root.join("data"), downloaded: root.join("assets/downloaded") }
}

/// First launch of a release: when assets of the play set are missing and
/// `forja_fetch` sits next to the executable, run it (progress goes to this
/// console) before building the world. `FORJA_NO_FETCH=1` skips it.
fn fetch_missing_assets() {
    if std::env::var_os("FORJA_NO_FETCH").is_some() {
        return;
    }
    let Some(root) = forja_assets::manifest::find_engine_root() else { return };
    let missing = forja_assets::manifest::missing_play_assets(&root);
    if missing.is_empty() {
        return;
    }
    let fetcher = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join(format!("forja_fetch{}", std::env::consts::EXE_SUFFIX))));
    let Some(fetcher) = fetcher.filter(|f| f.is_file()) else {
        log::warn!("{} assets missing ({}...); run `forja_fetch fetch --play --missing`", missing.len(), missing[0]);
        return;
    };
    println!("Forja: descargando {} paquetes de contenido (solo la primera vez, ~1 GB)...", missing.len());
    match std::process::Command::new(&fetcher).args(["fetch", "--play", "--missing", "--root"]).arg(&root).status() {
        Ok(s) if s.success() => println!("Forja: contenido listo."),
        Ok(_) => println!("Forja: algunos paquetes fallaron; el juego usara sustitutos y se reintentaran en el proximo arranque."),
        Err(e) => log::warn!("{}: {e}", fetcher.display()),
    }
}

/// Uploads new models and converts the scene into a render frame.
struct Extract {
    handles: Vec<ModelHandle>,
    env_loaded: bool,
    /// Static items handed to the renderer's culling grid.
    statics_uploaded: Option<usize>,
    /// `FORJA_LEGACY_STATICS=1`: resubmit the whole world every frame as
    /// before the static grid (for before/after measurements).
    legacy: bool,
}

impl Extract {
    fn new() -> Self {
        Self { handles: Vec::new(), env_loaded: false, statics_uploaded: None, legacy: std::env::var_os("FORJA_LEGACY_STATICS").is_some() }
    }

    fn frame(&mut self, renderer: &mut Renderer, app: &App) -> FrameDesc {
        let rs = app.world.res::<RenderScene>();
        if !self.env_loaded {
            self.env_loaded = true;
            if let Some(p) = &rs.hdri {
                match forja_assets::load_hdr(p) {
                    Ok(img) => renderer.set_environment(Some(&forja_render::env::EnvMap::from_hdr(&img, 14.0))),
                    Err(e) => log::warn!("{e}"),
                }
            }
        }
        while self.handles.len() < rs.models.len() {
            let m = &rs.models[self.handles.len()];
            self.handles.push(renderer.upload_scene(m));
        }
        // The world never moves: it is uploaded once into the renderer's
        // static culling grid; only dynamic items are resubmitted per frame.
        if !self.legacy && self.statics_uploaded != Some(rs.statics.len()) {
            let statics: Vec<DrawInstance> = rs.statics.iter().map(|item| DrawInstance { model: self.handles[item.model], transform: item.transform, nodes: item.nodes.clone(), joints: None, tint: item.tint, cast_shadows: true }).collect();
            renderer.set_static_instances(&statics);
            self.statics_uploaded = Some(rs.statics.len());
        }
        let mut frame = FrameDesc { camera: rs.camera, env: rs.env.clone(), lights: rs.lights.clone(), lines: rs.lines.clone(), time: app.world.res::<forja_core::Time>().elapsed as f32, ..Default::default() };
        let legacy_statics = if self.legacy { &rs.statics[..] } else { &[] };
        frame.instances.reserve(rs.dynamic.len() + legacy_statics.len());
        for item in legacy_statics.iter().chain(rs.dynamic.iter()) {
            frame.instances.push(DrawInstance { model: self.handles[item.model], transform: item.transform, nodes: item.nodes.clone(), joints: item.joints.clone(), tint: item.tint, cast_shadows: true });
        }
        frame
    }
}

/// Scripted intents for unattended screenshots.
fn script_intent(script: &str, frame: usize, i: &mut Intent) {
    match script {
        "walk" => i.movement = Vec2::new(0.0, 1.0),
        "drive" => {
            if frame == 2 {
                i.enter = true;
            }
            if frame > 5 {
                i.movement = Vec2::new(if frame > 60 { 0.35 } else { 0.0 }, 1.0);
            }
        }
        "shoot" => {
            if frame == 1 {
                i.movement = Vec2::new(0.0, 1.0);
            }
            if frame == 50 {
                i.movement = Vec2::ZERO;
                i.select = Some(forja_gameplay::WeaponCategory::Pistol);
            }
            i.aim = frame > 55;
            i.fire = frame > 60 && frame % 6 == 0;
        }
        _ => {}
    }
}

/// Real-time frames headless: simulation, extraction and rendering of each
/// frame timed separately (the GPU is waited for, so `frame` includes it).
fn bench(args: &Args, app: &mut App, renderer: &mut Renderer, extract: &mut Extract, perf_state: &mut PerfState) {
    let (w, h) = (args.width, args.height);
    renderer.resize(w, h);
    let tex = renderer.device.create_texture(&forja_render::wgpu::TextureDescriptor {
        label: Some("bench"),
        size: forja_render::wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: forja_render::wgpu::TextureDimension::D2,
        format: renderer.output_format,
        usage: forja_render::wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = tex.create_view(&Default::default());
    // Warm-up: static upload, pipelines, first readbacks.
    let frame = extract.frame(renderer, app);
    renderer.render(&frame, &view);
    let _ = renderer.device.poll(forja_render::wgpu::PollType::wait_indefinitely());
    let mut acc: Vec<FrameSample> = Vec::new();
    for f in 0..args.bench {
        let t = Instant::now();
        script_intent(&args.script, args.frames + f, app.world.res_mut::<Intent>());
        app.update_with_dt(1.0 / 60.0);
        if let Some(cam) = args.camera {
            app.world.res_mut::<game::camera::CameraRig>().fixed = Some(cam);
        }
        let sim_ms = t.elapsed().as_secs_f32() * 1000.0;
        let te = Instant::now();
        let frame = extract.frame(renderer, app);
        let extract_ms = te.elapsed().as_secs_f32() * 1000.0;
        let tr = Instant::now();
        renderer.render(&frame, &view);
        let render_cpu_ms = tr.elapsed().as_secs_f32() * 1000.0;
        let _ = renderer.device.poll(forja_render::wgpu::PollType::wait_indefinitely());
        let timings = app.world.res::<forja_core::SystemTimings>();
        let sample = FrameSample {
            frame_ms: t.elapsed().as_secs_f32() * 1000.0,
            sim_ms,
            extract_ms,
            render_cpu_ms,
            ui_ms: 0.0,
            gpu_ms: renderer.profiler.total_ms,
            systems: timings.last.clone(),
            render_scopes: renderer.cpu.scopes.clone(),
            gpu_passes: renderer.profiler.last.clone(),
            fixed_steps: timings.fixed_steps,
            stats: renderer.stats,
            render_scale: 1.0,
        };
        acc.push(sample.clone());
        perf_state.record(sample);
    }
    perf_state.stop_csv();
    print_bench(&acc);
}

fn print_bench(acc: &[FrameSample]) {
    let n = acc.len().max(1) as f32;
    let mean = |f: &dyn Fn(&FrameSample) -> f32| acc.iter().map(f).sum::<f32>() / n;
    let mut frames: Vec<f32> = acc.iter().map(|s| s.frame_ms).collect();
    frames.sort_by(f32::total_cmp);
    let p = |q: f32| frames.get(((frames.len() as f32 - 1.0) * q) as usize).copied().unwrap_or(0.0);
    println!("bench {} frames: frame {:.2} ms (p50 {:.2}, p95 {:.2}) | sim {:.2} | extract {:.2} | render cpu {:.2} | gpu {:.2} ms", acc.len(), mean(&|s| s.frame_ms), p(0.5), p(0.95), mean(&|s| s.sim_ms), mean(&|s| s.extract_ms), mean(&|s| s.render_cpu_ms), mean(&|s| s.gpu_ms));
    let avg_list = |get: &dyn Fn(&FrameSample) -> &Vec<(&'static str, f32)>| {
        let mut names: Vec<&'static str> = Vec::new();
        for s in acc {
            for (k, _) in get(s) {
                if !names.contains(k) {
                    names.push(k);
                }
            }
        }
        names.iter().map(|k| (*k, acc.iter().map(|s| get(s).iter().find(|e| e.0 == *k).map_or(0.0, |e| e.1)).sum::<f32>() / n)).collect::<Vec<_>>()
    };
    let fmt = |v: Vec<(&'static str, f32)>| v.iter().filter(|e| e.1 >= 0.01).map(|(k, ms)| format!("{k} {ms:.2}")).collect::<Vec<_>>().join(", ");
    println!("  systems: {}", fmt(avg_list(&|s| &s.systems)));
    println!("  render cpu: {}", fmt(avg_list(&|s| &s.render_scopes)));
    println!("  gpu passes: {}", fmt(avg_list(&|s| &s.gpu_passes)));
    if let Some(s) = acc.last() {
        let st = &s.stats;
        println!(
            "  last: draws {} shadow draws {} | instances {} shadow {} culled {} | tris {:.2} M shadow {:.2} M | lights {} | static {} in {} cells | vram ~{:.0} MB",
            st.draw_calls,
            st.shadow_draws,
            st.instances_drawn,
            st.shadow_instances,
            st.instances_culled,
            st.triangles as f64 / 1e6,
            st.shadow_triangles as f64 / 1e6,
            st.lights,
            st.static_instances,
            st.static_cells,
            st.vram_mb
        );
    }
}

fn run_headless(args: &Args) {
    let instance = forja_render::create_instance();
    let mut renderer = Renderer::new(&instance, None).expect("GPU");
    let t_build = Instant::now();
    let mut app = game::build(args.map, paths());
    log::info!("world built in {:.2?}", t_build.elapsed());
    if let Some(h) = args.hours {
        let t = app.world.res_mut::<game::env::TimeOfDay>();
        t.hours = h;
        t.photo_sky = false;
    }
    let ctx = egui::Context::default();
    let mut egui_renderer = egui_wgpu::Renderer::new(&renderer.device, renderer.output_format, Default::default());
    let mut ui_state = ui::UiState { adapter: renderer.adapter_info.name.clone(), fps: 60.0, ..Default::default() };
    let mut extract = Extract::new();
    let mut perf_state = PerfState::load(&paths().data.join("graphics.toml"), &renderer);
    if let Some(p) = args.preset {
        perf_state.cfg.apply_preset(p);
    }
    if args.preset.is_some() || std::env::var_os("FORJA_PRESET").is_some() {
        // Headless captures keep the High defaults unless a preset is asked.
        renderer.apply_graphics(&perf_state.cfg);
        // Explicit overrides still win over the preset.
        if let Ok(fx) = std::env::var("FORJA_FX") {
            renderer.settings.apply_fx(&fx);
        }
        if let Some(n) = std::env::var("FORJA_MAX_TEXTURE").ok().and_then(|v| v.parse().ok()) {
            renderer.settings.max_texture_size = n;
        }
    }
    if let Some(c) = &args.csv {
        perf_state.start_csv(c);
    }
    let t0 = Instant::now();
    for f in 0..args.frames {
        script_intent(&args.script, f, app.world.res_mut::<Intent>());
        app.update_with_dt(1.0 / 60.0);
        if let Some(cam) = args.camera {
            app.world.res_mut::<game::camera::CameraRig>().fixed = Some(cam);
        }
    }
    let sim_ms = t0.elapsed().as_secs_f32() * 1000.0 / args.frames as f32;
    // One more frame so a fixed camera takes effect.
    app.update_with_dt(1.0 / 60.0);
    if args.bench > 0 {
        bench(args, &mut app, &mut renderer, &mut extract, &mut perf_state);
    }
    let t_up = Instant::now();
    let frame = extract.frame(&mut renderer, &app);
    log::info!("GPU upload in {:.2?}", t_up.elapsed());
    let t1 = Instant::now();
    let tex = renderer.render_offscreen(&frame, args.width, args.height);
    let render_ms = t1.elapsed().as_secs_f32() * 1000.0;
    ui_state.render_stats = renderer.stats;
    {
        let hud = app.world.res_mut::<Hud>();
        hud.physics_ms = sim_ms;
    }
    // HUD on top.
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(args.width as f32, args.height as f32))), ..Default::default() };
    let out = ctx.run_ui(raw, |ui| ui::draw(ui.ctx(), &mut app.world, &mut ui_state));
    let jobs = ctx.tessellate(out.shapes, 1.0);
    let screen = egui_wgpu::ScreenDescriptor { size_in_pixels: [args.width, args.height], pixels_per_point: 1.0 };
    for (id, deltas) in &out.textures_delta.set {
        for delta in deltas {
            egui_renderer.update_texture(&renderer.device, &renderer.queue, *id, delta);
        }
    }
    let mut enc = renderer.device.create_command_encoder(&Default::default());
    let extra = egui_renderer.update_buffers(&renderer.device, &renderer.queue, &mut enc, &jobs, &screen);
    {
        let view = tex.create_view(&Default::default());
        let pass = enc.begin_render_pass(&forja_render::wgpu::RenderPassDescriptor {
            label: Some("egui"),
            color_attachments: &[Some(forja_render::wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: forja_render::wgpu::Operations { load: forja_render::wgpu::LoadOp::Load, store: forja_render::wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        egui_renderer.render(&mut pass.forget_lifetime(), &jobs, &screen);
    }
    renderer.queue.submit(extra.into_iter().chain([enc.finish()]));
    let img = renderer.read_image(&tex);
    let out_path = args.screenshot.clone().expect("screenshot path");
    if let Some(dir) = out_path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    img.save(&out_path).expect("save screenshot");
    println!(
        "{}: {} | sim {:.2} ms/frame | render {:.0} ms | {:?}",
        out_path.display(),
        app.world.res::<Hud>().map,
        sim_ms,
        render_ms,
        renderer.stats
    );
}

struct Gpu {
    window: Arc<Window>,
    surface: forja_render::wgpu::Surface<'static>,
    config: forja_render::wgpu::SurfaceConfiguration,
    renderer: Renderer,
    egui_state: egui_winit::State,
    egui_renderer: egui_wgpu::Renderer,
    present_modes: Vec<forja_render::wgpu::PresentMode>,
}

struct Sandbox {
    args: Args,
    gpu: Option<Gpu>,
    app: App,
    map: MapKind,
    input: Input,
    extract: Extract,
    ui: ui::UiState,
    captured: bool,
    last: Instant,
    fps_acc: (f32, u32),
    want_screenshot: bool,
    perf: Option<PerfState>,
}

impl Sandbox {
    fn new(args: Args) -> Self {
        let paths = paths();
        let map_text = std::fs::read_to_string(paths.data.join("input.toml")).unwrap_or_default();
        let input = Input::new(ActionMap::from_toml(&map_text).unwrap_or_default());
        let map = args.map;
        let mut app = game::build(map, paths);
        if let Some(h) = args.hours {
            app.world.res_mut::<game::env::TimeOfDay>().hours = h;
        }
        Self { args, gpu: None, app, map, input, extract: Extract::new(), ui: ui::UiState::default(), captured: false, last: Instant::now(), fps_acc: (0.0, 0), want_screenshot: false, perf: None }
    }

    fn capture(&mut self, on: bool) {
        let Some(gpu) = &self.gpu else { return };
        self.captured = on;
        if on {
            let _ = gpu.window.set_cursor_grab(CursorGrabMode::Locked).or_else(|_| gpu.window.set_cursor_grab(CursorGrabMode::Confined));
        } else {
            let _ = gpu.window.set_cursor_grab(CursorGrabMode::None);
        }
        gpu.window.set_cursor_visible(!on);
    }

    fn switch_map(&mut self, map: MapKind) {
        self.map = map;
        self.app = game::build(map, paths());
        self.extract = Extract::new();
        if let Some(gpu) = &mut self.gpu {
            // Fresh GPU resources for the new map.
            let renderer = Renderer::with_device(gpu.renderer.device.clone(), gpu.renderer.queue.clone(), gpu.renderer.adapter_info.clone(), gpu.renderer.output_format);
            gpu.renderer = renderer;
            gpu.renderer.resize(gpu.config.width, gpu.config.height);
            if let Some(p) = &mut self.perf {
                // New renderer: apply the graphics options (texture cap too).
                p.dirty = true;
            }
        }
    }

    fn sample_input(&mut self) {
        let inp = &self.input;
        let playing = self.captured && !self.ui.inventory_open;
        let i = self.app.world.res_mut::<Intent>();
        i.movement = Vec2::new(inp.axis("left", "right"), inp.axis("back", "forward"));
        i.look = if playing { inp.look() } else { Vec2::ZERO };
        i.sprint = inp.held("sprint");
        i.aim = playing && inp.held("aim");
        i.fire = playing && inp.held("fire");
        i.handbrake = inp.held("handbrake");
        i.jump |= inp.pressed("jump");
        i.enter |= inp.pressed("enter_vehicle");
        i.reload |= inp.pressed("reload");
        i.holster |= inp.pressed("holster");
        for (action, cat) in game::combat::WHEEL_KEYS {
            if inp.pressed(action) {
                i.select = Some(cat);
            }
        }
        if inp.pressed("use_item") {
            i.use_item = Some("medkit".into());
        }
    }

    fn redraw(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        let dt = (now - self.last).as_secs_f32();
        self.last = now;
        self.fps_acc.0 += dt;
        self.fps_acc.1 += 1;
        if self.fps_acc.0 > 0.5 {
            self.ui.fps = self.fps_acc.1 as f32 / self.fps_acc.0;
            self.fps_acc = (0.0, 0);
        }
        if self.input.pressed("quit") {
            if self.captured {
                self.capture(false);
            } else {
                event_loop.exit();
                return;
            }
        }
        if self.input.pressed("inventory") {
            self.ui.inventory_open = !self.ui.inventory_open;
            self.capture(!self.ui.inventory_open);
        }
        if self.input.pressed("debug_physics") {
            self.ui.debug = !self.ui.debug;
        }
        if self.input.pressed("next_map") {
            let next = self.map.next();
            self.switch_map(next);
        }
        if self.input.pressed("time_of_day") {
            let t = self.app.world.res_mut::<game::env::TimeOfDay>();
            // First press leaves the photo sky for the simulated atmosphere.
            if t.photo_sky {
                t.photo_sky = false;
            } else {
                t.hours = (t.hours + 3.0) % 24.0;
            }
        }
        if self.input.pressed("screenshot") {
            self.want_screenshot = true;
        }
        self.sample_input();
        self.input.end_frame();
        let t = Instant::now();
        self.app.update_with_dt(dt);
        let sim_ms = t.elapsed().as_secs_f32() * 1000.0;
        {
            let bodies = self.app.world.res::<forja_physics::PhysicsWorld>().body_count();
            let hud = self.app.world.res_mut::<Hud>();
            hud.physics_ms = sim_ms;
            hud.bodies = bodies;
        }
        let Some(gpu) = &mut self.gpu else { return };
        let Some(perf_state) = &mut self.perf else { return };
        if std::mem::take(&mut perf_state.dirty) {
            gpu.renderer.apply_graphics(&perf_state.cfg);
            let mode = present_mode(&gpu.present_modes, perf_state.cfg.vsync);
            let latency = perf_state.cfg.frame_latency;
            if gpu.config.present_mode != mode || gpu.config.desired_maximum_frame_latency != latency {
                gpu.config.present_mode = mode;
                gpu.config.desired_maximum_frame_latency = latency;
                gpu.surface.configure(&gpu.renderer.device, &gpu.config);
            }
        }
        // Internal resolution (dynamic resolution); the final pass
        // upsamples to the window.
        let scale = perf_state.next_scale();
        let iw = ((gpu.config.width as f32 * scale) as u32).max(64);
        let ih = ((gpu.config.height as f32 * scale) as u32).max(64);
        gpu.renderer.resize(iw, ih);
        let te = Instant::now();
        let frame = self.extract.frame(&mut gpu.renderer, &self.app);
        let extract_ms = te.elapsed().as_secs_f32() * 1000.0;
        let surface_tex = match gpu.surface.get_current_texture() {
            forja_render::wgpu::CurrentSurfaceTexture::Success(t) | forja_render::wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            forja_render::wgpu::CurrentSurfaceTexture::Outdated | forja_render::wgpu::CurrentSurfaceTexture::Lost => {
                gpu.surface.configure(&gpu.renderer.device, &gpu.config);
                return;
            }
            _ => return,
        };
        let view = surface_tex.texture.create_view(&Default::default());
        let tr = Instant::now();
        gpu.renderer.render(&frame, &view);
        let render_cpu_ms = tr.elapsed().as_secs_f32() * 1000.0;
        let tu = Instant::now();
        self.ui.render_stats = gpu.renderer.stats;
        self.ui.adapter = format!("{} ({:?})", gpu.renderer.adapter_info.name, gpu.renderer.adapter_info.backend);
        // UI.
        let raw = gpu.egui_state.take_egui_input(&gpu.window);
        let ctx = gpu.egui_state.egui_ctx().clone();
        let world = &mut self.app.world;
        let ui_state = &mut self.ui;
        let out = ctx.run_ui(raw, |ui| {
            ui::draw(ui.ctx(), world, ui_state);
            perf::draw_overlay(ui.ctx(), perf_state);
            perf::draw_options(ui.ctx(), perf_state);
        });
        gpu.egui_state.handle_platform_output(&gpu.window, out.platform_output);
        let jobs = ctx.tessellate(out.shapes, out.pixels_per_point);
        let screen = egui_wgpu::ScreenDescriptor { size_in_pixels: [gpu.config.width, gpu.config.height], pixels_per_point: out.pixels_per_point };
        for (id, deltas) in &out.textures_delta.set {
            for delta in deltas {
                gpu.egui_renderer.update_texture(&gpu.renderer.device, &gpu.renderer.queue, *id, delta);
            }
        }
        let mut enc = gpu.renderer.device.create_command_encoder(&Default::default());
        let extra = gpu.egui_renderer.update_buffers(&gpu.renderer.device, &gpu.renderer.queue, &mut enc, &jobs, &screen);
        {
            let pass = enc.begin_render_pass(&forja_render::wgpu::RenderPassDescriptor {
                label: Some("egui"),
                color_attachments: &[Some(forja_render::wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: forja_render::wgpu::Operations { load: forja_render::wgpu::LoadOp::Load, store: forja_render::wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            gpu.egui_renderer.render(&mut pass.forget_lifetime(), &jobs, &screen);
        }
        gpu.renderer.queue.submit(extra.into_iter().chain([enc.finish()]));
        for id in &out.textures_delta.free {
            gpu.egui_renderer.free_texture(id);
        }
        let ui_ms = tu.elapsed().as_secs_f32() * 1000.0;
        gpu.window.pre_present_notify();
        gpu.renderer.queue.present(surface_tex);
        {
            let timings = self.app.world.res::<forja_core::SystemTimings>();
            perf_state.record(FrameSample {
                frame_ms: dt * 1000.0,
                sim_ms,
                extract_ms,
                render_cpu_ms,
                ui_ms,
                gpu_ms: gpu.renderer.profiler.total_ms,
                systems: timings.last.clone(),
                render_scopes: gpu.renderer.cpu.scopes.clone(),
                gpu_passes: gpu.renderer.profiler.last.clone(),
                fixed_steps: timings.fixed_steps,
                stats: gpu.renderer.stats,
                render_scale: scale,
            });
        }
        if std::mem::take(&mut self.want_screenshot) {
            let img = gpu.renderer.render_to_image(&frame, gpu.config.width, gpu.config.height);
            gpu.renderer.resize(iw, ih);
            let name = format!("screenshots/forja-{}.png", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0));
            let _ = std::fs::create_dir_all("screenshots");
            match img.save(&name) {
                Ok(()) => self.app.world.res_mut::<Hud>().message(format!("Saved {name}")),
                Err(e) => log::warn!("screenshot: {e}"),
            }
        }
        gpu.window.request_redraw();
    }
}

impl ApplicationHandler for Sandbox {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.gpu.is_some() {
            return;
        }
        let attrs = Window::default_attributes().with_title("Forja sandbox").with_inner_size(winit::dpi::PhysicalSize::new(self.args.width, self.args.height));
        let window = Arc::new(event_loop.create_window(attrs).expect("window"));
        let instance = forja_render::create_instance_with_display(Box::new(event_loop.owned_display_handle()));
        let surface = instance.create_surface(window.clone()).expect("surface");
        let mut renderer = Renderer::new(&instance, Some(&surface)).expect("GPU");
        let size = window.inner_size();
        let adapter = pollster::block_on(instance.request_adapter(&forja_render::wgpu::RequestAdapterOptions {
            power_preference: forja_render::wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .expect("adapter");
        let present_modes = surface.get_capabilities(&adapter).present_modes;
        let mut config = surface.get_default_config(&adapter, size.width.max(1), size.height.max(1)).expect("surface config");
        config.format = renderer.output_format;
        let perf_state = PerfState::load(&paths().data.join("graphics.toml"), &renderer);
        renderer.apply_graphics(&perf_state.cfg);
        config.present_mode = present_mode(&present_modes, perf_state.cfg.vsync);
        config.desired_maximum_frame_latency = perf_state.cfg.frame_latency;
        surface.configure(&renderer.device, &config);
        self.perf = Some(perf_state);
        renderer.resize(config.width, config.height);
        let egui_ctx = egui::Context::default();
        let egui_state = egui_winit::State::new(egui_ctx, egui::ViewportId::ROOT, &window, Some(window.scale_factor() as f32), None, None);
        let egui_renderer = egui_wgpu::Renderer::new(&renderer.device, renderer.output_format, Default::default());
        log::info!("{} on {:?}", renderer.adapter_info.name, renderer.adapter_info.backend);
        self.gpu = Some(Gpu { window, surface, config, renderer, egui_state, egui_renderer, present_modes });
        self.capture(true);
        self.last = Instant::now();
        self.gpu.as_ref().expect("just set").window.request_redraw();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        if let Some(gpu) = &mut self.gpu {
            let resp = gpu.egui_state.on_window_event(&gpu.window, &event);
            if resp.consumed && !self.captured {
                return;
            }
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(gpu) = &mut self.gpu
                    && size.width > 0
                    && size.height > 0
                {
                    gpu.config.width = size.width;
                    gpu.config.height = size.height;
                    gpu.surface.configure(&gpu.renderer.device, &gpu.config);
                    gpu.renderer.resize(size.width, size.height);
                }
            }
            WindowEvent::Focused(false) => {
                self.input.reset();
                self.capture(false);
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    let name = format!("{code:?}");
                    if event.state == ElementState::Pressed && !event.repeat {
                        match name.as_str() {
                            "F10" => self.ui.help = !self.ui.help,
                            "F11" => {
                                if let Some(p) = &mut self.perf {
                                    p.overlay = !p.overlay;
                                }
                            }
                            "F9" => {
                                let open = self.perf.as_mut().map(|p| {
                                    p.options = !p.options;
                                    p.options
                                });
                                // The options menu needs the mouse.
                                if let Some(open) = open {
                                    self.capture(!open);
                                }
                            }
                            _ => {}
                        }
                    }
                    match event.state {
                        ElementState::Pressed => self.input.key_down(&name),
                        ElementState::Released => self.input.key_up(&name),
                    }
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let menu = self.perf.as_ref().is_some_and(|p| p.options);
                if !self.captured && state == ElementState::Pressed && button == MouseButton::Left && !self.ui.inventory_open && !menu {
                    self.capture(true);
                    return;
                }
                let name = match button {
                    MouseButton::Left => "MouseLeft",
                    MouseButton::Right => "MouseRight",
                    MouseButton::Middle => "MouseMiddle",
                    _ => return,
                };
                match state {
                    ElementState::Pressed => self.input.key_down(name),
                    ElementState::Released => self.input.key_up(name),
                }
            }
            WindowEvent::RedrawRequested => self.redraw(event_loop),
            _ => {}
        }
    }

    fn device_event(&mut self, _: &ActiveEventLoop, _: winit::event::DeviceId, event: DeviceEvent) {
        if let DeviceEvent::MouseMotion { delta } = event
            && self.captured
        {
            self.input.mouse_moved(Vec2::new(delta.0 as f32, delta.1 as f32));
        }
    }
}

/// FIFO with v-sync; without it Mailbox (no tearing, lowest latency) or
/// Immediate when the surface lacks Mailbox.
fn present_mode(modes: &[forja_render::wgpu::PresentMode], vsync: bool) -> forja_render::wgpu::PresentMode {
    use forja_render::wgpu::PresentMode;
    if vsync {
        PresentMode::AutoVsync
    } else if modes.contains(&PresentMode::Mailbox) {
        PresentMode::Mailbox
    } else {
        PresentMode::AutoNoVsync
    }
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info,wgpu_core=warn,wgpu_hal=warn,naga=warn")).init();
    let args = parse_args();
    if args.screenshot.is_none() {
        fetch_missing_assets();
    }
    if args.screenshot.is_some() {
        run_headless(&args);
        return;
    }
    let event_loop = EventLoop::new().expect("event loop");
    let mut sandbox = Sandbox::new(args);
    event_loop.run_app(&mut sandbox).expect("event loop");
}
