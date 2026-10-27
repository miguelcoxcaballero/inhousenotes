//! Whole procedural Spanish city rendered from a few viewpoints:
//! `cargo run --release -p forja_render --example city -- out_prefix [small]`
//! (`CITY_SHOTS=boulevard,aerial` renders only those shots; `CITY_MOTION=30`
//! also saves `<shot>_motion.png`, the last of 30 real-time 60 fps frames
//! walking into the shot, to judge TAA and the screen-space effects in
//! motion; `CITY_TREES=scanned` uses the scanned library tree instead of
//! the procedural `forja_flora` species, `CITY_SEASON=winter|spring|autumn`).
//! `NIGHT=1` renders the city at 22:30 (`HOUR=<0..24>` picks the
//! time) with lamps, lit windows, shops, neon and floodlights, every one of
//! them a clustered point or spot light, under [`Environment::night`];
//! `DUSK=1` (19:36), `RAIN=1` and `WET=1` select the other presets and
//! `LEGACY=1` the previous path (64 selected point lights, far glow
//! geometry, fixed exposure) for comparisons. Night files get a `_night`,
//! `_dusk`, `_rain`, `_night_wet` or `_before` suffix.

use std::path::PathBuf;

use forja_city::kit::Kit;
use forja_city::layout::{CityConfig, PropKind, build_city};
use forja_city::style::StyleLibrary;
use forja_math::{Mat4, Vec3};
use forja_render::*;

#[path = "common/night.rs"]
mod night;

fn main() {
    let prefix = std::env::args().nth(1).unwrap_or_else(|| "city".into());
    let small = std::env::args().nth(2).is_some_and(|a| a == "small");
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dl = root.join("assets/downloaded");
    let t0 = std::time::Instant::now();
    let mut kit = Kit::load(Some(&dl));
    let mode = night::environment();
    let hour = night_hour().or(mode.as_ref().map(|(n, _)| if n.starts_with("dusk") { 19.6 } else { 22.5 }));
    let legacy = mode.as_ref().is_none_or(|(n, _)| *n == "before");
    if let Some(h) = hour {
        kit.set_hour(h);
    }
    let styles = StyleLibrary::load(Some(&root.join("data/city/styles.toml")));
    let cfg = if small { CityConfig::small() } else { CityConfig::default() };
    let city = build_city(&kit, &styles, &cfg);
    let out = &city.out;
    let tris: usize = out.placements.iter().map(|(s, _)| out.scenes[*s].triangle_count()).sum();
    println!("city: {:?}, {} scenes, {} placements, {} tris, extent {:?} in {:.2}s", city.stats, out.scenes.len(), out.placements.len(), tris, city.extent, t0.elapsed().as_secs_f32());
    let mut names = std::collections::BTreeMap::new();
    for sc in &out.scenes {
        *names.entry(sc.name.clone()).or_insert(0usize) += 1;
    }
    println!("scenes by name: {names:?}");
    let lib = forja_world::ContentLibrary::load(&dl);
    let mut r = Renderer::new(&create_instance(), None).unwrap();
    if let Ok(img) = forja_assets::load_hdr(&dl.join("hdri_wide_street/wide_street_01_2k.hdr")) {
        r.set_environment(Some(&env::EnvMap::from_hdr(&img, 14.0)));
    }
    let handles: Vec<ModelHandle> = out.scenes.iter().map(|s| r.upload_scene(s)).collect();
    let scanned = std::env::var("CITY_TREES").is_ok_and(|v| v == "scanned");
    let tree = if scanned { lib.prop("polyhaven_island_tree").map(|s| r.upload_scene(&s)) } else { None };
    let mut forest_instances = Vec::new();
    if !scanned {
        let t1 = std::time::Instant::now();
        let species = forja_flora::SpeciesLibrary::load(Some(&root.join("data/flora/species.toml")));
        let season = std::env::var("CITY_SEASON").ok().and_then(|s| forja_flora::Season::parse(&s)).unwrap_or_default();
        let opt = forja_flora::city::PlantOptions { variants: 3, season, seed: cfg.seed };
        let forest = forja_flora::city::plant(&city.props, &city.prop_sites, &species, &opt);
        let handles: Vec<ModelHandle> = forest.models.iter().map(|t| r.upload_scene(&t.scene)).collect();
        println!("trees: {} instances of {} models ({:?}) in {:.2}s", forest.instances.len(), handles.len(), forest.by_species.keys().collect::<Vec<_>>(), t1.elapsed().as_secs_f32());
        forest_instances = forest.instances.iter().map(|(m, t)| DrawInstance::new(handles[*m], *t)).collect();
    }
    let bench = lib.prop("polyhaven_street_bench").map(|s| r.upload_scene(&s));
    println!("uploaded in {:.2}s", t0.elapsed().as_secs_f32());
    let mut frame = FrameDesc::default();
    for (s, m) in &out.placements {
        frame.instances.push(DrawInstance::new(handles[*s], *m));
    }
    frame.instances.extend(forest_instances);
    for (k, m) in &city.props {
        let h = match k {
            PropKind::Tree | PropKind::Palm => tree,
            PropKind::Bench => bench,
        };
        if let Some(h) = h {
            let s = if *k == PropKind::Bench { 1.0 } else { 1.5 };
            frame.instances.push(DrawInstance::new(h, *m * Mat4::from_scale(Vec3::splat(s))));
        }
    }
    frame.env.shadow_distance = 120.0;
    let night = hour.is_some_and(|_| kit.night.is_night());
    if night {
        match &mode {
            Some((_, env)) if !legacy => {
                frame.env = env.clone();
                frame.local_lights = out.city_lights.iter().map(night::from_city).collect();
            }
            _ => night_env(&mut frame.env, kit.night.dark),
        }
        println!("night at {:.1} h: {} city lights", kit.night.hour, out.city_lights.len());
    }
    let suffix = match &mode {
        Some((n, _)) => format!("_{n}"),
        None if night => "_night_old".to_string(),
        None => String::new(),
    };
    // `CITY_STATIC=1`: the city goes into the renderer's static culling
    // grid once (as the game does) instead of being resubmitted per frame.
    if std::env::var_os("CITY_STATIC").is_some() && !(night && legacy) {
        r.set_static_instances(&frame.instances);
        frame.instances.clear();
    }
    let base_instances = frame.instances.clone();
    let e = city.extent;
    // Eye-level shots along real streets of each district.
    let street_shot = |district: usize, along_z: bool, pick: usize| {
        let cands: Vec<_> = city.streets.iter().filter(|s| s.3 == district && !s.4 && ((s.1 - s.0).y.abs() > 1.0) == along_z).collect();
        let s = cands[pick.min(cands.len() - 1)];
        let dir = (s.1 - s.0).normalize();
        let side = forja_math::Vec2::new(-dir.y, dir.x) * (s.2 * 0.5 - 1.2);
        let eye = s.0 + dir * 4.0 + side;
        let at = s.0 + dir * 40.0 - side * 0.3;
        (Vec3::new(eye.x, 1.7, eye.y), Vec3::new(at.x, 5.0, at.y))
    };
    let (m_eye, m_at) = street_shot(0, true, 6);
    let (m2_eye, m2_at) = street_shot(0, false, 5);
    let (v_eye, v_at) = street_shot(1, true, 4);
    let (v2_eye, v2_at) = street_shot(1, false, 3);
    // From a corner of each plaza toward the facades across it.
    let plaza_shot = |district: usize| {
        let (a, b, _) = city.plazas.iter().find(|p| p.2 == district).copied().unwrap_or((forja_math::Vec2::ZERO, forja_math::Vec2::ONE, 0));
        let c = (a + b) * 0.5;
        let eye = forja_math::Vec2::new(c.x, a.y + (b.y - a.y) * 0.12);
        (Vec3::new(eye.x, 1.7, eye.y), Vec3::new(c.x, 9.0, b.y + 10.0))
    };
    let (pm_eye, pm_at) = plaza_shot(0);
    let (pv_eye, pv_at) = plaza_shot(1);
    let shots = [
        ("boulevard", Vec3::new(-6.0, 1.7, -40.0), Vec3::new(-12.0, 6.0, 20.0)),
        ("madrid_street", m_eye, m_at),
        ("madrid_street2", m2_eye, m2_at),
        ("valencia_street", v_eye, v_at),
        ("valencia_street2", v2_eye, v2_at),
        ("plaza_madrid", pm_eye, pm_at),
        ("plaza_valencia", pv_eye, pv_at),
        ("aerial", Vec3::new(-e.x * 0.8, 120.0, -e.y * 1.1), Vec3::new(0.0, 0.0, 0.0)),
        ("aerial_far", Vec3::new(-e.x * 1.6, 260.0, -e.y * 2.2), Vec3::new(0.0, 0.0, 0.0)),
    ];
    let only = std::env::var("CITY_SHOTS").ok();
    for (name, eye, at) in shots {
        if only.as_deref().is_some_and(|o| !o.split(',').any(|n| n.trim() == name)) {
            continue;
        }
        frame.camera = Camera::look_at(eye, at);
        frame.camera.fov_y = 60f32.to_radians();
        if night && legacy {
            frame.instances = base_instances.clone();
            frame.lights = night_lights(&out.city_lights, eye, at - eye);
            let glow = forja_city::nightlight::glow_parts(&out.city_lights, eye, 45.0, 2.0 * (frame.camera.fov_y * 0.5).tan() / 720.0, 1.6);
            let h = r.upload_scene(&forja_city::kit::assemble(&kit, "glow", glow, None));
            frame.instances.push(DrawInstance { cast_shadows: false, ..DrawInstance::new(h, Mat4::IDENTITY) });
        }
        if let Some(n) = std::env::var("CITY_BENCH").ok().and_then(|v| v.parse::<u32>().ok()) {
            bench_frames(&mut r, &frame, name, n);
        }
        let img = r.render_to_image(&frame, 1280, 720);
        let path = format!("{prefix}_{name}{suffix}.png");
        img.save(&path).unwrap();
        println!("{path}: {:?}", r.stats);
        if let Some(n) = std::env::var("CITY_MOTION").ok().and_then(|v| v.parse::<u32>().ok()) {
            // Walk (1.4 m/s) toward the shot while turning slightly.
            let fwd = (at - eye).normalize();
            let side = fwd.cross(Vec3::Y).normalize();
            r.reset_history();
            let mut img = None;
            for i in 0..n {
                let k = (n - 1 - i) as f32 / 60.0;
                let e = eye - fwd * 1.4 * k + Vec3::Y * (0.03 * (k * 11.0).sin());
                frame.camera = Camera::look_at(e, at + side * (2.0 * k));
                frame.camera.fov_y = 60f32.to_radians();
                frame.time = i as f32 / 60.0;
                img = Some(r.render_live_to_image(&frame, 1280, 720));
            }
            frame.time = 0.0;
            if let Some(img) = img {
                let path = format!("{prefix}_{name}{suffix}_motion.png");
                img.save(&path).unwrap();
                println!("{path}");
            }
        }
    }
}

/// `NIGHT=1` (22:30) or `HOUR=<h>`.
fn night_hour() -> Option<f32> {
    if let Some(h) = std::env::var("HOUR").ok().and_then(|v| v.parse::<f32>().ok()) {
        return Some(h);
    }
    std::env::var("NIGHT").ok().filter(|v| v != "0").map(|_| 22.5)
}

/// Test night for the current renderer: a dim moon as the sun, a dark
/// analytic sky, exposure for streets lit at ~15 lux.
fn night_env(env: &mut Environment, dark: f32) {
    env.use_ibl = false;
    env.camera = CameraModel::Night;
    env.sun_dir = Vec3::new(-0.35, 0.72, -0.55).normalize();
    env.sun_illuminance = 0.025 + 14.0 * (1.0 - dark).powi(3);
    env.set_linear_exposure(0.55 + 3.0 * dark);
    env.bloom = 0.04 + 0.04 * dark;
}

/// The 64 lights the current renderer can take, chosen by importance.
fn night_lights(all: &[forja_city::CityLight], eye: Vec3, fwd: Vec3) -> Vec<PointLight> {
    forja_city::nightlight::select(all, eye, fwd, 64)
        .into_iter()
        .map(|i| {
            let (position, color, intensity, radius) = all[i].as_point();
            PointLight { position, radius, color, intensity }
        })
        .collect()
}

/// `CITY_BENCH=N`: N real-time frames at 1280x720 into an offscreen target;
/// prints the renderer's CPU time (scopes) and the wall time with the GPU
/// (lavapipe here, so only the CPU side is meaningful).
fn bench_frames(r: &mut Renderer, frame: &FrameDesc, name: &str, n: u32) {
    use forja_render::wgpu;
    r.resize(1280, 720);
    r.reset_history();
    let tex = r.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("bench"),
        size: wgpu::Extent3d { width: 1280, height: 720, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: r.output_format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = tex.create_view(&Default::default());
    r.render(frame, &view);
    let _ = r.device.poll(wgpu::PollType::wait_indefinitely());
    let (mut cpu, mut wall) = (0.0f32, 0.0f32);
    let mut scopes: Vec<(&'static str, f32)> = Vec::new();
    for i in 0..n {
        let mut f = frame.clone();
        f.time = (i + 1) as f32 / 60.0;
        let t = std::time::Instant::now();
        r.render(&f, &view);
        cpu += t.elapsed().as_secs_f32() * 1000.0;
        let _ = r.device.poll(wgpu::PollType::wait_indefinitely());
        wall += t.elapsed().as_secs_f32() * 1000.0;
        for (k, ms) in &r.cpu.scopes {
            match scopes.iter_mut().find(|e| e.0 == *k) {
                Some(e) => e.1 += ms,
                None => scopes.push((k, *ms)),
            }
        }
    }
    let n = n.max(1) as f32;
    let list: Vec<String> = scopes.iter().map(|(k, ms)| format!("{k} {:.2}", ms / n)).collect();
    let st = &r.stats;
    println!(
        "bench {name}: render cpu {:.2} ms, wall {:.1} ms | {} | draws {} shadow {} | inst {} shadow {} | tris {:.2} M shadow {:.2} M | lights {}",
        cpu / n,
        wall / n,
        list.join(", "),
        st.draw_calls,
        st.shadow_draws,
        st.instances_drawn,
        st.shadow_instances,
        st.triangles as f64 / 1e6,
        st.shadow_triangles as f64 / 1e6,
        st.lights
    );
}
