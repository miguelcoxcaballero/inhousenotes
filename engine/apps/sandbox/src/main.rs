//! Forja sandbox.
//!
//! ```text
//! cargo run --release -p sandbox -- --map kit
//! cargo run --release -p sandbox -- --map osm --screenshot shot.png --script drive
//! ```
//! Maps: `kit` (Kenney city), `osm` (Miami Beach), `track`, `terrain`.

mod game;
mod ui;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use forja_core::App;
use forja_input::{ActionMap, Input};
use forja_math::{Vec2, Vec3};
use forja_render::{DrawInstance, FrameDesc, ModelHandle, Renderer};
use game::{Hud, Intent, MapKind, Paths, RenderScene};
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
}

fn parse_vec3(s: &str) -> Option<Vec3> {
    let v: Vec<f32> = s.split(',').filter_map(|x| x.trim().parse().ok()).collect();
    (v.len() == 3).then(|| Vec3::new(v[0], v[1], v[2]))
}

fn parse_args() -> Args {
    let mut a = Args { map: MapKind::Spain, screenshot: None, script: "idle".into(), frames: 90, width: 1600, height: 900, hours: None, camera: None };
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

/// Uploads new models and converts the scene into a render frame.
struct Extract {
    handles: Vec<ModelHandle>,
    env_loaded: bool,
}

impl Extract {
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
        let mut frame = FrameDesc { camera: rs.camera, env: rs.env.clone(), lights: rs.lights.clone(), lines: rs.lines.clone(), time: app.world.res::<forja_core::Time>().elapsed as f32, ..Default::default() };
        for item in rs.statics.iter().chain(rs.dynamic.iter()) {
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
    let mut extract = Extract { handles: Vec::new(), env_loaded: false };
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
        Self { args, gpu: None, app, map, input, extract: Extract { handles: Vec::new(), env_loaded: false }, ui: ui::UiState::default(), captured: false, last: Instant::now(), fps_acc: (0.0, 0), want_screenshot: false }
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
        self.extract = Extract { handles: Vec::new(), env_loaded: false };
        if let Some(gpu) = &mut self.gpu {
            // Fresh GPU resources for the new map.
            let renderer = Renderer::with_device(gpu.renderer.device.clone(), gpu.renderer.queue.clone(), gpu.renderer.adapter_info.clone(), gpu.renderer.output_format);
            gpu.renderer = renderer;
            gpu.renderer.resize(gpu.config.width, gpu.config.height);
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
        let frame = self.extract.frame(&mut gpu.renderer, &self.app);
        let surface_tex = match gpu.surface.get_current_texture() {
            forja_render::wgpu::CurrentSurfaceTexture::Success(t) | forja_render::wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            forja_render::wgpu::CurrentSurfaceTexture::Outdated | forja_render::wgpu::CurrentSurfaceTexture::Lost => {
                gpu.surface.configure(&gpu.renderer.device, &gpu.config);
                return;
            }
            _ => return,
        };
        let view = surface_tex.texture.create_view(&Default::default());
        gpu.renderer.render(&frame, &view);
        self.ui.render_stats = gpu.renderer.stats;
        self.ui.adapter = format!("{} ({:?})", gpu.renderer.adapter_info.name, gpu.renderer.adapter_info.backend);
        // UI.
        let raw = gpu.egui_state.take_egui_input(&gpu.window);
        let ctx = gpu.egui_state.egui_ctx().clone();
        let world = &mut self.app.world;
        let ui_state = &mut self.ui;
        let out = ctx.run_ui(raw, |ui| ui::draw(ui.ctx(), world, ui_state));
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
        gpu.window.pre_present_notify();
        gpu.renderer.queue.present(surface_tex);
        if std::mem::take(&mut self.want_screenshot) {
            let img = gpu.renderer.render_to_image(&frame, gpu.config.width, gpu.config.height);
            gpu.renderer.resize(gpu.config.width, gpu.config.height);
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
        let mut config = surface
            .get_default_config(&forja_render::wgpu::Adapter::clone(&pollster::block_on(instance.request_adapter(&forja_render::wgpu::RequestAdapterOptions { compatible_surface: Some(&surface), ..Default::default() })).expect("adapter")), size.width.max(1), size.height.max(1))
            .expect("surface config");
        config.format = renderer.output_format;
        config.present_mode = forja_render::wgpu::PresentMode::AutoVsync;
        surface.configure(&renderer.device, &config);
        renderer.resize(config.width, config.height);
        let egui_ctx = egui::Context::default();
        let egui_state = egui_winit::State::new(egui_ctx, egui::ViewportId::ROOT, &window, Some(window.scale_factor() as f32), None, None);
        let egui_renderer = egui_wgpu::Renderer::new(&renderer.device, renderer.output_format, Default::default());
        log::info!("{} on {:?}", renderer.adapter_info.name, renderer.adapter_info.backend);
        self.gpu = Some(Gpu { window, surface, config, renderer, egui_state, egui_renderer });
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
                    if name == "F10" && event.state == ElementState::Pressed && !event.repeat {
                        self.ui.help = !self.ui.help;
                    }
                    match event.state {
                        ElementState::Pressed => self.input.key_down(&name),
                        ElementState::Released => self.input.key_up(&name),
                    }
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if !self.captured && state == ElementState::Pressed && button == MouseButton::Left && !self.ui.inventory_open {
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

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info,wgpu_core=warn,wgpu_hal=warn,naga=warn")).init();
    let args = parse_args();
    if args.screenshot.is_some() {
        run_headless(&args);
        return;
    }
    let event_loop = EventLoop::new().expect("event loop");
    let mut sandbox = Sandbox::new(args);
    event_loop.run_app(&mut sandbox).expect("event loop");
}
