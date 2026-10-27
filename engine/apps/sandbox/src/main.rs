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
    /// Headless: after the screenshot, cross dusk (or dawn) as the game
    /// does, building the other city in the background while frames keep
    /// rendering, and save `<screenshot>_switched.png`.
    night_switch: bool,
    /// Headless shadow stability test (`--stability N`): N frames with the
    /// sun creeping (0.03° per frame, fixed camera), then the camera
    /// walking 3 m away and back in N frames each; prints frame-to-frame
    /// and return-to-pose differences (see `docs/performance.md`).
    stability: usize,
    /// `--adapter auto|warp|dx12|vulkan|<name>` (or `FORJA_ADAPTER`).
    adapter: forja_render::gpu::AdapterChoice,
    /// `--smoke N`: render N frames, save `--screenshot` (default
    /// `smoke.png`), exit non-zero on a GPU error or a black image.
    smoke: usize,
    /// `--window`: run the smoke test in a window (swapchain path).
    window: bool,
}

fn parse_vec3(s: &str) -> Option<Vec3> {
    let v: Vec<f32> = s.split(',').filter_map(|x| x.trim().parse().ok()).collect();
    (v.len() == 3).then(|| Vec3::new(v[0], v[1], v[2]))
}

fn parse_args() -> Args {
    let mut a = Args { map: MapKind::Spain, screenshot: None, script: "idle".into(), frames: 90, width: 1600, height: 900, hours: None, camera: None, preset: None, bench: 0, csv: None, night_switch: false, stability: 0, adapter: forja_render::gpu::AdapterChoice::from_env(), smoke: 0, window: false };
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
            "--stability" => a.stability = v.parse().expect("stability frames"),
            "--night-switch" => {
                a.night_switch = true;
                i += 1;
                continue;
            }
            "--adapter" => a.adapter = forja_render::gpu::AdapterChoice::parse(&v),
            "--smoke" => a.smoke = v.parse().expect("smoke frames"),
            "--window" => {
                a.window = true;
                i += 1;
                continue;
            }
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
    statics_uploaded: Option<(usize, usize)>,
    /// `FORJA_LEGACY_STATICS=1`: resubmit the whole world every frame as
    /// before the static grid (for before/after measurements).
    legacy: bool,
    /// Upload time spent in earlier slices of [`Extract::upload_for`].
    upload_ms: f64,
    freed: usize,
    /// Streaming of the city tiles' block meshes (see `sandbox::stream`).
    stream: Option<sandbox::stream::Streamer>,
}

impl Extract {
    fn new() -> Self {
        Self { handles: Vec::new(), env_loaded: false, statics_uploaded: None, legacy: std::env::var_os("FORJA_LEGACY_STATICS").is_some(), upload_ms: 0.0, freed: 0, stream: None }
    }

    /// Uploads the models added since the last frame. The world's scenes
    /// lose their CPU copies (vertices, indices, textures) as soon as they
    /// are on the GPU, so the load never holds both (see
    /// [`game::world::strip`]); flushes keep the staging memory bounded.
    fn upload(&mut self, renderer: &mut Renderer, app: &mut App) {
        self.upload_for(renderer, app, None);
    }

    /// [`Self::upload`] for at most `budget` (the loading screen keeps
    /// drawing between slices); `true` once everything is on the GPU.
    fn upload_for(&mut self, renderer: &mut Renderer, app: &mut App, budget: Option<std::time::Duration>) -> bool {
        let n = app.world.res::<RenderScene>().models.len();
        if self.handles.len() >= n {
            return true;
        }
        let (t, first) = (Instant::now(), self.handles.len());
        if first == 0
            && let Some(gw) = app.world.resources.get::<game::world::GameWorld>()
        {
            let d = &gw.desc;
            let tiles = d
                .hlod_tiles
                .iter()
                .enumerate()
                .map(|(k, tile)| (k as u32 + 1, tile.detail.clone().map(|i| d.placements[i].0).filter(|&s| d.scenes[s].name == "block").map(|s| gw.scene_base + s).collect()))
                .collect();
            self.stream = sandbox::stream::Streamer::new(tiles);
        }
        let deadline = budget.map(|b| t + b);
        let stream = &self.stream;
        let mut freed = 0usize;
        let handles = &mut self.handles;
        let mut upload = |rs: &mut RenderScene, mut world: Option<(&mut game::world::GameWorld, usize)>| {
            while handles.len() < n {
                if deadline.is_some_and(|d| Instant::now() >= d) {
                    break;
                }
                let i = handles.len();
                handles.push(renderer.upload_scene(&rs.models[i]));
                if let Some((gw, base)) = world.as_mut()
                    && let Some(d) = i.checked_sub(*base).and_then(|j| gw.desc.scenes.get_mut(j))
                {
                    if let Some(st) = stream
                        && st.streams(i)
                    {
                        st.keep(i, Arc::clone(&rs.models[i]));
                    }
                    let (s, b) = game::world::strip(&rs.models[i]);
                    freed += b;
                    let s = Arc::new(s);
                    *d = Arc::clone(&s);
                    rs.models[i] = s;
                    if i % 512 == 0 {
                        renderer.prune_texture_cache();
                        perf::trim_heap();
                    }
                }
            }
        };
        if app.world.resources.get::<game::world::GameWorld>().is_some_and(|w| !w.released) {
            app.world.scope::<game::world::GameWorld, _>(|world, gw| {
                let base = gw.scene_base;
                upload(world.res_mut::<RenderScene>(), Some((&mut *gw, base)));
            });
        } else {
            upload(app.world.res_mut::<RenderScene>(), None);
        }
        renderer.flush_uploads();
        forja_assets::progress::advance(0.85 + 0.15 * self.handles.len() as f32 / n.max(1) as f32);
        self.freed += freed;
        if self.handles.len() < n {
            self.upload_ms += t.elapsed().as_secs_f64() * 1000.0;
            return false;
        }
        let freed = std::mem::take(&mut self.freed);
        if let Some(gw) = app.world.resources.get_mut::<game::world::GameWorld>() {
            gw.released = true;
        }
        let total_ms = std::mem::take(&mut self.upload_ms) + t.elapsed().as_secs_f64() * 1000.0;
        renderer.prune_texture_cache();
        perf::trim_heap();
        let (mesh, tex) = renderer.uploaded_bytes();
        let _ = first;
        log::info!(
            "uploaded {} models in {:.2}s: meshes {:.0} MB, textures {:.0} MB (total); dropped {:.0} MB of CPU mesh copies, resident {:.0} MB",
            n,
            total_ms / 1000.0,
            mesh as f64 / 1048576.0,
            tex as f64 / 1048576.0,
            freed as f64 / 1048576.0,
            perf::resident_mb()
        );
        true
    }

    fn frame(&mut self, renderer: &mut Renderer, app: &mut App) -> FrameDesc {
        self.upload(renderer, app);
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
        // The world never moves: it is uploaded once into the renderer's
        // static culling grid; only dynamic items are resubmitted per frame.
        // Items listed in `volatile_statics` (signal heads) change at run
        // time and are submitted per frame instead.
        let key = (rs.statics.len(), rs.volatile_statics.len());
        if !self.legacy && self.statics_uploaded != Some(key) {
            let kept = || rs.statics.iter().enumerate().filter(|(i, _)| rs.volatile_statics.binary_search(i).is_err());
            let statics: Vec<DrawInstance> = kept().map(|(_, item)| DrawInstance { model: self.handles[item.model], transform: item.transform, nodes: item.nodes.clone(), joints: None, tint: item.tint, cast_shadows: true, motion_id: 0 }).collect();
            let lod: Vec<forja_render::StaticLod> = kept().map(|(i, _)| rs.static_lod.get(i).map_or(forja_render::StaticLod::default(), |&(group, hlod)| forja_render::StaticLod { group, hlod })).collect();
            renderer.set_static_instances_lod(&statics, &lod);
            self.statics_uploaded = Some(key);
        }
        if let Some(st) = &mut self.stream {
            st.update(renderer, &self.handles, rs.camera.position);
        }
        let mut frame = FrameDesc { camera: rs.camera, env: rs.env.clone(), lights: rs.lights.clone(), local_lights: rs.local_lights.clone(), lines: rs.lines.clone(), time: app.world.res::<forja_core::Time>().elapsed as f32, ..Default::default() };
        if frame.env.camera == forja_render::CameraModel::Night {
            // Gameplay lights (headlights, brake lights, signals, muzzle
            // flashes) are tuned in the legacy units of the old fixed night
            // exposure (5.5); under the night pipeline's camera they become
            // clustered lights with the same brightness relative to it.
            let k = 5.5 / frame.env.exposure.max(1e-3);
            frame.local_lights.extend(frame.lights.drain(..).map(|p| {
                let l = forja_render::Light::from(p);
                forja_render::Light { intensity: l.intensity * k, ..l }
            }));
        }
        let legacy_statics = if self.legacy { &rs.statics[..] } else { &[] };
        let volatile = if self.legacy { &[][..] } else { &rs.volatile_statics[..] };
        frame.instances.reserve(rs.dynamic.len() + legacy_statics.len() + volatile.len());
        for item in legacy_statics.iter().chain(volatile.iter().filter_map(|&i| rs.statics.get(i))).chain(rs.dynamic.iter()) {
            frame.instances.push(DrawInstance { model: self.handles[item.model], transform: item.transform, nodes: item.nodes.clone(), joints: item.joints.clone(), tint: item.tint, cast_shadows: true, motion_id: item.id });
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
            heavy: Vec::new(),
        };
        acc.push(sample.clone());
        perf_state.record(sample);
    }
    perf_state.stop_csv();
    print_bench(&acc);
    if let Some(st) = &extract.stream {
        println!("  stream: {} tiles' blocks released ({:.0} MB), {} reloaded; meshes resident {:.0} MB", st.released_tiles, st.released_bytes as f64 / 1048576.0, st.loads, renderer.uploaded_bytes().0 as f64 / 1048576.0);
    }
    for shadow in [false, true] {
        let list: Vec<String> = renderer.heaviest_meshes(10, shadow).iter().map(|(n, k, t)| format!("{n} x{k} {:.2}M", *t as f64 / 1e6)).collect();
        println!("  heaviest {}: {}", if shadow { "shadow" } else { "camera" }, list.join(", "));
    }
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
            "  last: draws {} shadow draws {} | instances {} shadow {} culled {} | tris {:.2} M shadow {:.2} M | lights {} | static {} in {} cells, {} HLOD tiles | vram ~{:.0} MB",
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
            st.hlod_tiles,
            st.vram_mb
        );
    }
}

fn run_headless(args: &Args) {
    let instance = forja_render::create_instance();
    let mut renderer = Renderer::new_with(&instance, None, &args.adapter).expect("GPU");
    let t_build = Instant::now();
    let mut app = game::build_at(args.map, paths(), args.hours);
    log::info!("world built in {:.2?}, resident {:.0} MB", t_build.elapsed(), perf::resident_mb());
    {
        let (mesh, tex) = perf::upload_estimate(&app.world.res::<RenderScene>().models, &[1024, 2048, 4096]);
        let mb = |b: u64| b as f64 / 1048576.0;
        log::info!("GPU upload estimate: meshes {:.0} MB, textures {:.0} / {:.0} / {:.0} MB (cap 1024 / 2048 / 4096)", mb(mesh), mb(tex[0]), mb(tex[1]), mb(tex[2]));
        // Mesh memory by scene name (what streaming could release).
        let mut by: std::collections::BTreeMap<String, (u64, usize)> = Default::default();
        for m in &app.world.res::<RenderScene>().models {
            let b: u64 = m.meshes.iter().flatten().map(|p| (p.positions.len() * 48 + p.indices.len() * 4 + p.lods.iter().map(|l| l.len() * 4).sum::<usize>()) as u64).sum();
            let key = m.name.split(|c: char| c.is_ascii_digit() || c == '_').next().unwrap_or("").to_string();
            let e = by.entry(key).or_default();
            e.0 += b;
            e.1 += 1;
        }
        let mut v: Vec<_> = by.into_iter().collect();
        v.sort_by(|a, b| b.1.0.cmp(&a.1.0));
        log::info!("mesh memory by scene: {}", v.iter().take(12).map(|(k, (b, n))| format!("{k} x{n} {:.0} MB", mb(*b))).collect::<Vec<_>>().join(", "));
    }
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
    if args.stability > 0 {
        stability(args, &mut app, &mut renderer, &mut extract);
        return;
    }
    let t_up = Instant::now();
    let frame = extract.frame(&mut renderer, &mut app);
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
    if args.night_switch {
        night_switch(args, app, renderer, extract, &out_path);
        return;
    }
    println!(
        "{}: {} | sim {:.2} ms/frame | render {:.0} ms | {:?}",
        out_path.display(),
        app.world.res::<Hud>().map,
        sim_ms,
        render_ms,
        renderer.stats
    );
}

/// Shadow stability: how much the image changes between frames when only
/// the sun creeps, and how far a frame differs from the first after the
/// camera walks away and comes back to the same pose. Static geometry
/// casts the same shadow for the same camera and sun, so whatever changes
/// is flicker (caster LOD following the camera, re-fitted cascades, thin
/// casters aliasing).
fn stability(args: &Args, app: &mut App, renderer: &mut Renderer, extract: &mut Extract) {
    let (eye, look) = args.camera.unwrap_or_else(|| {
        let c = app.world.res::<RenderScene>().camera;
        (c.position, c.position + c.rotation * Vec3::NEG_Z * 10.0)
    });
    let (w, h) = (args.width, args.height);
    let n = args.stability;
    let shot = |app: &mut App, renderer: &mut Renderer, extract: &mut Extract, eye: Vec3, look: Vec3| -> Vec<f32> {
        app.world.res_mut::<game::camera::CameraRig>().fixed = Some((eye, look));
        app.update_with_dt(0.0);
        let frame = extract.frame(renderer, app);
        let tex = renderer.render_offscreen(&frame, w, h);
        renderer.read_image(&tex).pixels().map(|p| (p[0] as f32 + p[1] as f32 + p[2] as f32) / 3.0).collect()
    };
    let diff = |a: &[f32], b: &[f32]| {
        let mut d: Vec<f32> = a.iter().zip(b).map(|(x, y)| (x - y).abs()).collect();
        let mean = d.iter().sum::<f32>() / d.len().max(1) as f32;
        let changed = d.iter().filter(|v| **v > 8.0).count() as f32 / d.len().max(1) as f32;
        d.sort_by(f32::total_cmp);
        (mean, d[(d.len() as f32 * 0.999) as usize % d.len().max(1)], changed)
    };
    // Per-frame noise (TAA jitter, rotating GTAO and PCF patterns) is
    // averaged out over 8 frames, so what remains is the shadow itself.
    let avg = |app: &mut App, renderer: &mut Renderer, extract: &mut Extract, eye: Vec3, look: Vec3| -> Vec<f32> {
        let mut acc = shot(app, renderer, extract, eye, look);
        for _ in 1..8 {
            for (a, b) in acc.iter_mut().zip(shot(app, renderer, extract, eye, look)) {
                *a += b;
            }
        }
        acc.iter_mut().for_each(|a| *a /= 8.0);
        acc
    };
    {
        let t = app.world.res_mut::<game::env::TimeOfDay>();
        t.photo_sky = false;
        t.speed = 0.0;
    }
    let shot = avg;
    // Settle (TAA history, cached cascades), then the noise floor.
    shot(app, renderer, extract, eye, look);
    let floor = diff(&shot(app, renderer, extract, eye, look), &shot(app, renderer, extract, eye, look));
    let mut prev = shot(app, renderer, extract, eye, look);
    let (mut sm, mut sp, mut sc) = (0.0, 0.0f32, 0.0);
    for _ in 0..n {
        app.world.res_mut::<game::env::TimeOfDay>().hours += 0.002;
        let img = shot(app, renderer, extract, eye, look);
        let (m, p, c) = diff(&img, &prev);
        sm += m / n as f32;
        sp = sp.max(p);
        sc += c / n as f32;
        prev = img;
    }
    let first = prev.clone();
    let dir = (look - eye).normalize_or_zero().cross(Vec3::Y).normalize_or_zero() * 3.0;
    let step = |app: &mut App, renderer: &mut Renderer, extract: &mut Extract, t: f32| {
        app.world.res_mut::<game::camera::CameraRig>().fixed = Some((eye + dir * t, look + dir * t));
        app.update_with_dt(0.0);
        let frame = extract.frame(renderer, app);
        let _ = renderer.render_offscreen(&frame, w, h);
    };
    for i in 1..=n {
        step(app, renderer, extract, i as f32 / n as f32);
    }
    for i in (0..n).rev() {
        step(app, renderer, extract, i as f32 / n as f32);
    }
    shot(app, renderer, extract, eye, look);
    let back = shot(app, renderer, extract, eye, look);
    let (rm, rp, rc) = diff(&back, &first);
    println!(
        "stability ({n} steps, 8-frame means): noise floor mean {:.3} changed {:.3}% | sun creep mean {sm:.3} p99.9 {sp:.0} changed {:.3}% | walk away and back: mean {rm:.3} p99.9 {rp:.0} changed {:.3}%",
        floor.0,
        floor.2 * 100.0,
        sc * 100.0,
        rc * 100.0
    );
}

/// Headless dusk/dawn switch as in the game: the other city is built on a
/// worker thread while the current one keeps rendering; then the swap
/// (fresh GPU resources and the upload) is the only main-thread stall.
fn night_switch(args: &Args, mut app: App, mut renderer: Renderer, mut extract: Extract, out: &std::path::Path) {
    let night = !app.world.res::<game::world::GameWorld>().night_content;
    let hours = if night { 21.5 } else { 11.0 };
    let map = args.map;
    let t0 = Instant::now();
    let pending = spawn_build(map, Some(hours), true);
    let (w, h) = (args.width.min(480), args.height.min(270));
    let mut frames: Vec<f32> = Vec::new();
    while !pending.handle.is_finished() {
        let t = Instant::now();
        app.update_with_dt(1.0 / 60.0);
        let frame = extract.frame(&mut renderer, &mut app);
        let _ = renderer.render_offscreen(&frame, w, h);
        let _ = renderer.device.poll(forja_render::wgpu::PollType::wait_indefinitely());
        frames.push(t.elapsed().as_secs_f32() * 1000.0);
    }
    let built = t0.elapsed().as_secs_f32();
    let new_app = pending.handle.join().expect("world build");
    let ts = Instant::now();
    drop(app);
    let renderer2 = Renderer::with_device(renderer.device.clone(), renderer.queue.clone(), renderer.adapter_info.clone(), renderer.output_format);
    let settings = renderer.settings.clone();
    drop(renderer);
    perf::trim_heap();
    let mut renderer = renderer2;
    renderer.settings = settings;
    let mut app = new_app;
    app.world.res_mut::<game::env::TimeOfDay>().photo_sky = false;
    let mut extract = Extract::new();
    extract.upload(&mut renderer, &mut app);
    let upload = ts.elapsed().as_secs_f32();
    app.update_with_dt(1.0 / 60.0);
    if let Some(cam) = args.camera {
        app.world.res_mut::<game::camera::CameraRig>().fixed = Some(cam);
        app.update_with_dt(1.0 / 60.0);
    }
    let frame = extract.frame(&mut renderer, &mut app);
    let tex = renderer.render_offscreen(&frame, args.width, args.height);
    let swap = ts.elapsed().as_secs_f32();
    let path = out.with_file_name(format!("{}_switched.png", out.file_stem().and_then(|s| s.to_str()).unwrap_or("shot")));
    renderer.read_image(&tex).save(&path).expect("save screenshot");
    frames.sort_by(f32::total_cmp);
    let max = frames.last().copied().unwrap_or(0.0);
    let mean = frames.iter().sum::<f32>() / frames.len().max(1) as f32;
    println!(
        "night switch ({}): built in the background in {built:.1}s while {} frames rendered (mean {mean:.0} ms, max {max:.0} ms); main-thread stall: upload {upload:.2}s, swap + first frame {swap:.2}s; resident {:.0} MB -> {}",
        if night { "dusk" } else { "dawn" },
        frames.len(),
        perf::resident_mb(),
        path.display()
    );
}

/// Mean luma (0..1) and the share of pixels that are neither black nor
/// white: a smoke screenshot that rendered nothing fails the run.
fn image_check(img: &image::RgbaImage) -> (f32, f32) {
    let n = (img.width() * img.height()).max(1) as f32;
    let mut sum = 0.0f32;
    let mut mid = 0u32;
    for p in img.pixels() {
        let l = (0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32) / 255.0;
        sum += l;
        if l > 0.02 && l < 0.98 {
            mid += 1;
        }
    }
    (sum / n, mid as f32 / n)
}

/// Verdict of a smoke run: exits the process with 0 when every frame
/// rendered without a GPU error and the image is not blank.
fn smoke_verdict(frames: usize, img: &image::RgbaImage, out: &std::path::Path, ms: &[f32]) -> ! {
    if let Some(dir) = out.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = img.save(out) {
        log::error!("smoke: saving {}: {e}", out.display());
    }
    let errors = forja_render::gpu::GPU_ERRORS.load(std::sync::atomic::Ordering::Relaxed);
    let (luma, mid) = image_check(img);
    let mut sorted = ms.to_vec();
    sorted.sort_by(f32::total_cmp);
    let mean = ms.iter().sum::<f32>() / ms.len().max(1) as f32;
    let p95 = sorted.get(((sorted.len() as f32 - 1.0) * 0.95) as usize).copied().unwrap_or(0.0);
    let ok = errors == 0 && frames > 0 && luma > 0.01 && mid > 0.05;
    println!(
        "SMOKE {}: {frames} frames, {errors} GPU errors, frame mean {mean:.1} ms p95 {p95:.1} ms, image luma {luma:.3} ({:.0}% mid-tones) -> {}",
        if ok { "OK" } else { "FAILED" },
        mid * 100.0,
        out.display()
    );
    std::process::exit(if ok { 0 } else { 1 });
}

/// `--smoke N` headless: the real game loop (simulation, extraction,
/// rendering into an offscreen target) for N frames on the chosen adapter.
fn run_smoke(args: &Args) {
    let instance = forja_render::create_instance();
    let t_gpu = Instant::now();
    let mut renderer = match Renderer::new_with(&instance, None, &args.adapter) {
        Ok(r) => r,
        Err(e) => {
            println!("SMOKE FAILED: {e}");
            std::process::exit(1);
        }
    };
    log::info!("smoke: renderer (pipelines compiled) in {:.2?} on {} ({:?})", t_gpu.elapsed(), renderer.adapter_info.name, renderer.adapter_info.backend);
    let t_build = Instant::now();
    let mut app = game::build_at(args.map, paths(), args.hours);
    log::info!("smoke: world built in {:.2?}, resident {:.0} MB", t_build.elapsed(), perf::resident_mb());
    if let Some(h) = args.hours {
        let t = app.world.res_mut::<game::env::TimeOfDay>();
        t.hours = h;
        t.photo_sky = false;
    }
    let mut perf_state = PerfState::load(&paths().data.join("graphics.toml"), &renderer);
    perf_state.cfg.apply_preset(args.preset.unwrap_or(forja_render::Preset::Low));
    renderer.apply_graphics(&perf_state.cfg);
    if let Ok(fx) = std::env::var("FORJA_FX") {
        renderer.settings.apply_fx(&fx);
    }
    let mut extract = Extract::new();
    let (w, h) = (args.width, args.height);
    renderer.resize(w, h);
    let tex = renderer.device.create_texture(&forja_render::wgpu::TextureDescriptor {
        label: Some("smoke"),
        size: forja_render::wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: forja_render::wgpu::TextureDimension::D2,
        format: renderer.output_format,
        usage: forja_render::wgpu::TextureUsages::RENDER_ATTACHMENT | forja_render::wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = tex.create_view(&Default::default());
    let mut ms = Vec::with_capacity(args.smoke);
    for f in 0..args.smoke {
        let t = Instant::now();
        script_intent(&args.script, f, app.world.res_mut::<Intent>());
        app.update_with_dt(1.0 / 60.0);
        if let Some(cam) = args.camera {
            app.world.res_mut::<game::camera::CameraRig>().fixed = Some(cam);
        }
        let frame = extract.frame(&mut renderer, &mut app);
        renderer.render(&frame, &view);
        let _ = renderer.device.poll(forja_render::wgpu::PollType::wait_indefinitely());
        let dt = t.elapsed().as_secs_f32() * 1000.0;
        if f > 0 {
            ms.push(dt);
        }
        if f % 10 == 0 || f + 1 == args.smoke {
            let st = renderer.stats;
            log::info!("smoke frame {f}: {dt:.1} ms, gpu {:.1} ms, {} draws, {:.2} M tris, {} shadow draws, {} lights, GPU errors {}", renderer.profiler.total_ms, st.draw_calls, st.triangles as f64 / 1e6, st.shadow_draws, st.lights, forja_render::gpu::GPU_ERRORS.load(std::sync::atomic::Ordering::Relaxed));
        }
    }
    println!("smoke: gpu passes: {}", renderer.profiler.last.iter().map(|(k, v)| format!("{k} {v:.2}")).collect::<Vec<_>>().join(", "));
    let img = renderer.read_image(&tex);
    let out = args.screenshot.clone().unwrap_or_else(|| PathBuf::from("smoke.png"));
    smoke_verdict(args.smoke, &img, &out, &ms);
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
    /// Frames presented (windowed `--smoke`).
    frames: usize,
    smoke_ms: Vec<f32>,
    /// A world being built on a worker thread.
    pending: Option<Pending>,
    /// The loading screen is up (building in the foreground, or uploading
    /// a new world to the GPU): its title.
    loading: Option<String>,
    /// Clock and player carried into a world rebuilt at dusk or dawn.
    carry: Option<Carry>,
}

/// A world built on a worker thread ([`App`] is `Send`).
struct Pending {
    handle: std::thread::JoinHandle<App>,
    /// Built while the current world keeps running (dusk and dawn); the
    /// swap only costs the GPU upload.
    background: bool,
    started: Instant,
}

struct Carry {
    hours: f32,
    photo_sky: bool,
    speed: f32,
    player: Option<(Vec3, f32)>,
    message: &'static str,
}

/// Starts building `map` at `hour` on a worker thread.
fn spawn_build(map: MapKind, hour: Option<f32>, background: bool) -> Pending {
    let handle = std::thread::Builder::new()
        .name("world build".into())
        .spawn(move || {
            let mut app = game::build_at(map, paths(), hour);
            if let Some(h) = hour {
                app.world.res_mut::<game::env::TimeOfDay>().hours = h;
            }
            app
        })
        .expect("world build thread");
    Pending { handle, background, started: Instant::now() }
}

impl Sandbox {
    fn new(args: Args) -> Self {
        let paths = paths();
        let map_text = std::fs::read_to_string(paths.data.join("input.toml")).unwrap_or_default();
        let input = Input::new(ActionMap::from_toml(&map_text).unwrap_or_default());
        let map = args.map;
        // The window opens at once with a loading screen while the world
        // is built on a worker thread.
        forja_assets::progress::reset("Preparando el mapa");
        let pending = Some(spawn_build(map, args.hours, false));
        Self {
            args,
            gpu: None,
            app: App::new(),
            map,
            input,
            extract: Extract::new(),
            ui: ui::UiState::default(),
            captured: false,
            last: Instant::now(),
            fps_acc: (0.0, 0),
            want_screenshot: false,
            perf: None,
            pending,
            loading: Some(format!("Cargando {}", map.label())),
            carry: None,
            frames: 0,
            smoke_ms: Vec::new(),
        }
    }

    /// A finished build replaces the running world: fresh GPU resources,
    /// then the upload runs under the loading screen.
    fn install(&mut self, mut app: App) {
        if let Some(c) = self.carry.take() {
            {
                let t = app.world.res_mut::<game::env::TimeOfDay>();
                t.hours = c.hours;
                t.photo_sky = c.photo_sky;
                t.speed = c.speed;
            }
            if let Some((pos, facing)) = c.player {
                app.world.scope::<game::player::Player, _>(|w, p| {
                    p.controller.teleport(w.res_mut::<forja_physics::PhysicsWorld>(), pos);
                    p.facing = facing;
                });
            }
            app.world.res_mut::<game::Hud>().message(c.message);
        }
        // Free the old world (CPU and GPU) before uploading the new one.
        self.app = App::new();
        self.extract = Extract::new();
        if let Some(gpu) = &mut self.gpu {
            let renderer = Renderer::with_device(gpu.renderer.device.clone(), gpu.renderer.queue.clone(), gpu.renderer.adapter_info.clone(), gpu.renderer.output_format);
            gpu.renderer = renderer;
            gpu.renderer.resize(gpu.config.width, gpu.config.height);
            if let Some(p) = &mut self.perf {
                // New renderer: apply the graphics options (texture cap too).
                p.dirty = true;
            }
        }
        perf::trim_heap();
        self.app = app;
        forja_assets::progress::stage("Subiendo la ciudad a la tarjeta gráfica", 0.85);
        if self.loading.is_none() {
            self.loading = Some("Preparando la ciudad".into());
        }
    }

    /// Loading screen: the title, the current stage and a progress bar.
    fn draw_loading(&mut self) {
        let Some(gpu) = &mut self.gpu else { return };
        let surface_tex = match gpu.surface.get_current_texture() {
            forja_render::wgpu::CurrentSurfaceTexture::Success(t) | forja_render::wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            forja_render::wgpu::CurrentSurfaceTexture::Outdated | forja_render::wgpu::CurrentSurfaceTexture::Lost => {
                gpu.surface.configure(&gpu.renderer.device, &gpu.config);
                return;
            }
            _ => return,
        };
        let view = surface_tex.texture.create_view(&Default::default());
        let (stage, fraction) = forja_assets::progress::get();
        let title = self.loading.clone().unwrap_or_default();
        let elapsed = self.pending.as_ref().map(|p| p.started.elapsed().as_secs_f32());
        let raw = gpu.egui_state.take_egui_input(&gpu.window);
        let ctx = gpu.egui_state.egui_ctx().clone();
        let out = ctx.run_ui(raw, |ui| ui::draw_loading(ui.ctx(), &title, &stage, fraction, elapsed));
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
                label: Some("loading"),
                color_attachments: &[Some(forja_render::wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: forja_render::wgpu::Operations { load: forja_render::wgpu::LoadOp::Clear(forja_render::wgpu::Color { r: 0.012, g: 0.011, b: 0.01, a: 1.0 }), store: forja_render::wgpu::StoreOp::Store },
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

    /// Another map (F2): the old one is freed first and the new one is
    /// built behind the loading screen.
    fn switch_map(&mut self, map: MapKind) {
        if self.pending.is_some() {
            return;
        }
        self.map = map;
        // Keep the clock (and build the city's night content for it).
        let (hours, photo, speed) = {
            let t = self.app.world.res::<game::env::TimeOfDay>();
            (t.hours, t.photo_sky, t.speed)
        };
        self.app = App::new();
        self.extract = Extract::new();
        if let Some(gpu) = &mut self.gpu {
            gpu.renderer = Renderer::with_device(gpu.renderer.device.clone(), gpu.renderer.queue.clone(), gpu.renderer.adapter_info.clone(), gpu.renderer.output_format);
            gpu.renderer.resize(gpu.config.width, gpu.config.height);
        }
        perf::trim_heap();
        forja_assets::progress::reset("Preparando el mapa");
        self.carry = Some(Carry { hours, photo_sky: photo, speed, player: None, message: map.label() });
        self.pending = Some(spawn_build(map, (!photo).then_some(hours), false));
        self.loading = Some(format!("Cargando {}", map.label()));
    }

    /// Dusk or dawn: the city with the other content is built on a worker
    /// thread while this one keeps running; the player, the clock and the
    /// camera carry over when it is swapped in.
    fn rebuild_for_time(&mut self, night: bool) {
        if self.pending.is_some() {
            return;
        }
        let (hours, photo, speed) = {
            let t = self.app.world.res::<game::env::TimeOfDay>();
            (t.hours, t.photo_sky, t.speed)
        };
        log::info!("time of day crossed {}: building the city in the background", if night { "dusk" } else { "dawn" });
        forja_assets::progress::reset(if night { "Anochece" } else { "Amanece" });
        let driving = self.app.world.res::<game::vehicles::Driving>().0.is_some();
        let player = (!driving).then(|| {
            let p = self.app.world.res::<game::player::Player>();
            (p.position(), p.facing)
        });
        self.carry = Some(Carry { hours, photo_sky: photo, speed, player, message: if night { "Anochece en la ciudad" } else { "Amanece en la ciudad" } });
        self.pending = Some(spawn_build(self.map, Some(hours), true));
    }

    /// Polls the worker; `true` while the loading screen owns the frame.
    fn update_loading(&mut self) -> bool {
        if self.pending.as_ref().is_some_and(|p| p.handle.is_finished()) {
            let p = self.pending.take().expect("checked");
            match p.handle.join() {
                Ok(app) => {
                    log::info!("world ready after {:.1}s{}", p.started.elapsed().as_secs_f32(), if p.background { " (built in the background)" } else { "" });
                    // A dusk build that dawn overtook (or the reverse) is dropped.
                    let stale = p.background && self.map == MapKind::Spain && {
                        let want = self.app.world.res::<game::env::TimeOfDay>().night_content();
                        app.world.res::<game::world::GameWorld>().night_content != want
                    };
                    if stale {
                        self.carry = None;
                    } else {
                        if p.background {
                            // Keep the clock as it is now (time ran on).
                            let t = self.app.world.res::<game::env::TimeOfDay>();
                            if let Some(c) = &mut self.carry {
                                c.hours = t.hours;
                            }
                        }
                        self.install(app);
                    }
                }
                Err(_) => {
                    log::error!("the world build panicked");
                    std::process::exit(1);
                }
            }
        }
        if self.pending.as_ref().is_some_and(|p| !p.background) {
            self.draw_loading();
            return true;
        }
        if self.loading.is_some() {
            let Some(gpu) = &mut self.gpu else { return true };
            if let Some(p) = &mut self.perf
                && std::mem::take(&mut p.dirty)
            {
                gpu.renderer.apply_graphics(&p.cfg);
            }
            let done = self.extract.upload_for(&mut gpu.renderer, &mut self.app, Some(std::time::Duration::from_millis(40)));
            if done {
                self.loading = None;
                self.last = Instant::now();
                return false;
            }
            self.draw_loading();
            return true;
        }
        false
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
        if self.update_loading() {
            if self.input.pressed("quit") {
                event_loop.exit();
            }
            self.input.end_frame();
            if let Some(gpu) = &self.gpu {
                gpu.window.request_redraw();
            }
            return;
        }
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
        // The procedural city bakes lit windows and lamps for the hour it was
        // built at: crossing dusk or dawn rebuilds it for the new time, in
        // the background (the game keeps running until it is ready).
        if self.map == MapKind::Spain {
            let want = self.app.world.res::<game::env::TimeOfDay>().night_content();
            let built = self.app.world.res::<game::world::GameWorld>().night_content;
            if want != built {
                self.rebuild_for_time(want);
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
        let scale = perf_state.next_scale(gpu.config.width, gpu.config.height);
        let iw = ((gpu.config.width as f32 * scale) as u32).max(64);
        let ih = ((gpu.config.height as f32 * scale) as u32).max(64);
        gpu.renderer.resize(iw, ih);
        let te = Instant::now();
        let frame = self.extract.frame(&mut gpu.renderer, &mut self.app);
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
                heavy: if perf_state.overlay { gpu.renderer.heaviest_meshes(6, false) } else { Vec::new() },
            });
        }
        self.frames += 1;
        if self.args.smoke > 0 {
            if self.frames > 1 {
                self.smoke_ms.push(dt * 1000.0);
            }
            if self.frames % 10 == 0 {
                log::info!("smoke frame {}: {:.1} ms, gpu {:.1} ms, GPU errors {}", self.frames, dt * 1000.0, gpu.renderer.profiler.total_ms, forja_render::gpu::GPU_ERRORS.load(std::sync::atomic::Ordering::Relaxed));
            }
            if self.frames >= self.args.smoke {
                let img = gpu.renderer.render_live_to_image(&frame, gpu.config.width, gpu.config.height);
                let out = self.args.screenshot.clone().unwrap_or_else(|| PathBuf::from("smoke.png"));
                smoke_verdict(self.frames, &img, &out, &self.smoke_ms);
            }
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
        let mut renderer = match Renderer::new_with(&instance, Some(&surface), &self.args.adapter) {
            Ok(r) => r,
            Err(e) => {
                // No usable GPU: say so instead of a bare panic.
                log::error!("{e}");
                eprintln!("Forja: no se pudo iniciar la GPU (DirectX 12 / Vulkan): {e}");
                std::process::exit(3);
            }
        };
        let size = window.inner_size();
        let adapter = renderer.adapter.clone().expect("adapter of a new renderer");
        let present_modes = surface.get_capabilities(&adapter).present_modes;
        let mut config = surface.get_default_config(&adapter, size.width.max(1), size.height.max(1)).expect("surface config");
        config.format = renderer.output_format;
        let mut perf_state = PerfState::load(&paths().data.join("graphics.toml"), &renderer);
        if let Some(p) = self.args.preset {
            // `--preset` overrides the saved options and the automatic pick.
            perf_state.cfg.auto = false;
            perf_state.cfg.apply_preset(p);
        }
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
    if args.smoke > 0 && !args.window {
        run_smoke(&args);
        return;
    }
    if args.screenshot.is_none() && args.smoke == 0 {
        fetch_missing_assets();
    }
    if args.screenshot.is_some() && args.smoke == 0 {
        run_headless(&args);
        return;
    }
    let event_loop = EventLoop::new().expect("event loop");
    let mut sandbox = Sandbox::new(args);
    event_loop.run_app(&mut sandbox).expect("event loop");
}
