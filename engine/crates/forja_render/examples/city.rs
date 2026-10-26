//! Whole procedural Spanish city rendered from a few viewpoints:
//! `cargo run --release -p forja_render --example city -- out_prefix [small]`
//! (`CITY_SHOTS=boulevard,aerial` renders only those shots; `CITY_MOTION=30`
//! also saves `<shot>_motion.png`, the last of 30 real-time 60 fps frames
//! walking into the shot, to judge TAA and the screen-space effects in
//! motion). `NIGHT=1`, `DUSK=1`, `RAIN=1` and `WET=1` render the shots at
//! night, at dusk, on a rainy night or with wet streets (files get a
//! `_night`, `_dusk`, `_rain` or `_night_wet` suffix) with the city's lights
//! as physical lamps, floodlights on the plaza facades and a lit fountain.

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
    let kit = Kit::load(Some(&dl));
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
    let tree = lib.prop("polyhaven_island_tree").map(|s| r.upload_scene(&s));
    let bench = lib.prop("polyhaven_street_bench").map(|s| r.upload_scene(&s));
    println!("uploaded in {:.2}s", t0.elapsed().as_secs_f32());
    let mut frame = FrameDesc::default();
    for (s, m) in &out.placements {
        frame.instances.push(DrawInstance::new(handles[*s], *m));
    }
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
    let night_env = night::environment();
    // The plaza: its fountain, or the middle of the plaza lamps.
    let fountain = out.placements.iter().find(|(s, _)| out.scenes[*s].name.starts_with("fountain")).map(|(_, m)| m.w_axis.truncate());
    let plaza = fountain.unwrap_or(Vec3::ZERO);
    if let Some((_, env)) = &night_env {
        frame.env = env.clone();
        frame.local_lights = night::city_lights(&out.lights);
        // Plaza lamps ring the square at 4 m: floodlights at their feet light
        // the facades across the street, and the fountain glows cyan.
        let ring: Vec<Vec3> = out.lights.iter().map(|l| l.0).filter(|p| (p.y - 4.0).abs() < 0.01 && (*p - plaza).length() < 70.0).collect();
        for p in &ring {
            let out_dir = Vec3::new(p.x - plaza.x, 0.0, p.z - plaza.z).normalize_or_zero();
            let at = Vec3::new(p.x, 0.4, p.z) + out_dir * 1.0;
            frame.local_lights.push(night::floodlight(at, at + out_dir * 22.0 + Vec3::Y * 13.0, 14000.0));
        }
        if fountain.is_some() {
            for k in 0..6 {
                let a = k as f32 / 6.0 * std::f32::consts::TAU;
                let p = plaza + Vec3::new(a.cos() * 3.0, 0.6, a.sin() * 3.0);
                frame.local_lights.push(Light::spot(p, Vec3::new(-a.cos() * 0.3, 1.0, -a.sin() * 0.3), Vec3::new(0.25, 0.95, 1.6), 2500.0, 0.3, 0.7).with_range(14.0));
            }
        }
        println!("night: {} lights ({} floodlights)", frame.local_lights.len(), ring.len());
    }
    let suffix = night_env.as_ref().map(|(n, _)| format!("_{n}")).unwrap_or_default();
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
    let shots = [
        ("boulevard", Vec3::new(-6.0, 1.7, -40.0), Vec3::new(-12.0, 6.0, 20.0)),
        ("madrid_street", m_eye, m_at),
        ("madrid_street2", m2_eye, m2_at),
        ("valencia_street", v_eye, v_at),
        ("valencia_street2", v2_eye, v2_at),
        ("aerial", Vec3::new(-e.x * 0.8, 120.0, -e.y * 1.1), Vec3::new(0.0, 0.0, 0.0)),
        ("plaza", plaza + Vec3::new(-28.0, 1.7, -26.0), plaza + Vec3::new(0.0, 7.0, 0.0)),
        ("aerial_far", Vec3::new(-e.x * 1.6, 260.0, -e.y * 2.2), Vec3::new(0.0, 0.0, 0.0)),
    ];
    let only = std::env::var("CITY_SHOTS").ok();
    for (name, eye, at) in shots {
        if only.as_deref().is_some_and(|o| !o.split(',').any(|n| n.trim() == name)) {
            continue;
        }
        if night_env.as_ref().is_some_and(|(n, _)| *n == "before") {
            frame.local_lights.clear();
            frame.lights = night::legacy_lights(&out.lights, eye);
        }
        frame.camera = Camera::look_at(eye, at);
        frame.camera.fov_y = 60f32.to_radians();
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
