//! Whole procedural Spanish city rendered from a few viewpoints:
//! `cargo run --release -p forja_render --example city -- out_prefix [small]`
//! (`CITY_SHOTS=boulevard,aerial` renders only those shots; `CITY_MOTION=30`
//! also saves `<shot>_motion.png`, the last of 30 real-time 60 fps frames
//! walking into the shot, to judge TAA and the screen-space effects in
//! motion; `CITY_TREES=scanned` uses the scanned library tree instead of
//! the procedural `forja_flora` species, `CITY_SEASON=winter|spring|autumn`).

use std::path::PathBuf;

use forja_city::kit::Kit;
use forja_city::layout::{CityConfig, PropKind, build_city};
use forja_city::style::StyleLibrary;
use forja_math::{Mat4, Vec3};
use forja_render::*;

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
    ];
    let only = std::env::var("CITY_SHOTS").ok();
    for (name, eye, at) in shots {
        if only.as_deref().is_some_and(|o| !o.split(',').any(|n| n.trim() == name)) {
            continue;
        }
        frame.camera = Camera::look_at(eye, at);
        frame.camera.fov_y = 60f32.to_radians();
        let img = r.render_to_image(&frame, 1280, 720);
        let path = format!("{prefix}_{name}.png");
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
                let path = format!("{prefix}_{name}_motion.png");
                img.save(&path).unwrap();
                println!("{path}");
            }
        }
    }
}
