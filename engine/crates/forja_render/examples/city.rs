//! Whole procedural Spanish city rendered from a few viewpoints:
//! `cargo run --release -p forja_render --example city -- out_prefix [preset] [shot,...]`
//!
//! Presets: `default` (grids, boulevard and the Austrias and Carmen old
//! towns), `classic`, `small`, `carmen`, `austrias`, `organic`,
//! `osm:<file.json|file.osm>`. Shots are picked from the realized streets:
//! aerial, aerial_old, avenue, avenue_walk, ensanche, madrid, curved, alley (eye level 1.7 m),
//! slope (a sloping 2-3.5 m alley), stairs, plaza (the main square),
//! plazuela, acute (corner), deadend, overview.
//!
//! `NIGHT=1` renders the city at 22:30 (`HOUR=<0..24>` picks the time)
//! with lamps, lit windows, shops, neon and floodlights; until the
//! renderer has clustered lighting each shot gets the most important
//! lights ([`forja_city::nightlight::select`]) plus far glow points.
//! `CITY_MOTION=30` also saves `<shot>_motion.png`, the last of 30
//! real-time 60 fps frames walking into the shot (TAA and screen-space
//! effects in motion).
//! `DUSK=1` (19:36), `RAIN=1` and `WET=1` select the other night presets
//! (clustered point and spot lights under [`Environment::night`]) and
//! `LEGACY=1` the previous path; night files get a `_night`, `_dusk`,
//! `_rain`, `_night_wet` or `_before` suffix. `CITY_SHOTS=a,b` also picks
//! shots. `CITY_TREES=scanned` uses the scanned library tree instead of the
//! procedural `forja_flora` species (`CITY_SEASON=winter|spring|autumn`).
//! `CITY_RADIUS=r` uploads only what lies within r m of the selected shots.

use std::path::PathBuf;

use forja_city::kit::Kit;
use forja_city::layout::{CityConfig, PropKind, build_city};
use forja_city::plan::End;
use forja_city::profile::Class;
use forja_city::style::StyleLibrary;
use forja_math::{Mat4, Vec2, Vec3};
use forja_render::*;

#[path = "common/night.rs"]
mod night;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let prefix = args.get(1).cloned().unwrap_or_else(|| "city".into());
    let preset = args.get(2).cloned().unwrap_or_else(|| "default".into());
    let only: Option<Vec<String>> = args.get(3).map(|s| s.split(',').map(String::from).collect());
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
    let cfg = if let Some(path) = preset.strip_prefix("osm:") {
        forja_world::spain::SpanishCity::config_from_osm(std::path::Path::new(path)).unwrap_or_else(|e| panic!("osm {path}: {e}"))
    } else {
        CityConfig::preset(&preset).unwrap_or_else(|| panic!("unknown preset {preset}"))
    };
    let mut city = build_city(&kit, &styles, &cfg);
    let tris: usize = city.out.placements.iter().map(|(s, _)| city.out.scenes[*s].triangle_count()).sum();
    println!("city {preset}: {:?}, {} scenes, {} placements, {} tris, extent {:?} in {:.2}s", city.stats, city.out.scenes.len(), city.out.placements.len(), tris, city.extent, t0.elapsed().as_secs_f32());
    let only = only.or_else(|| std::env::var("CITY_SHOTS").ok().map(|s| s.split(',').map(|x| x.trim().to_string()).collect()));
    let shots: Vec<_> = shots(&city).into_iter().filter(|(name, _, _)| only.as_ref().is_none_or(|o| o.iter().any(|x| x == name))).collect();
    // `CITY_RADIUS=r`: only what lies within r metres of the selected
    // shots' sight lines is uploaded (eye-level shots of a large map in
    // a fraction of the memory).
    let radius = std::env::var("CITY_RADIUS").ok().and_then(|v| v.parse::<f32>().ok());
    let near = |b: forja_math::Aabb, m: &Mat4| {
        let Some(r) = radius else { return true };
        let c = m.transform_point3((b.min + b.max) * 0.5);
        let half = (b.max - b.min).length() * 0.5;
        shots.iter().any(|(_, e, a)| {
            let d = *a - *e;
            let t = ((c - *e).dot(d) / d.length_squared().max(1e-6)).clamp(0.0, 1.0);
            (*e + d * t).distance(c) < r + half
        })
    };
    let scenes = std::mem::take(&mut city.out.scenes);
    let out = &city.out;
    let lib = forja_world::ContentLibrary::load(&dl);
    let mut r = Renderer::new(&create_instance(), None).unwrap();
    if let Ok(img) = forja_assets::load_hdr(&dl.join("hdri_wide_street/wide_street_01_2k.hdr")) {
        r.set_environment(Some(&env::EnvMap::from_hdr(&img, 14.0)));
    }
    // Scenes are uploaded on first use, then the CPU copies dropped.
    let mut handles: Vec<Option<ModelHandle>> = vec![None; scenes.len()];
    let mut frame = FrameDesc::default();
    for (s, m) in &out.placements {
        if !near(scenes[*s].bounds(), m) {
            continue;
        }
        let h = *handles[*s].get_or_insert_with(|| r.upload_scene(&scenes[*s]));
        frame.instances.push(DrawInstance::new(h, *m));
    }
    drop(scenes);
    let scanned = std::env::var("CITY_TREES").is_ok_and(|v| v == "scanned");
    let tree = if scanned { lib.prop("polyhaven_island_tree").map(|s| r.upload_scene(&s)) } else { None };
    if !scanned {
        let t1 = std::time::Instant::now();
        let species = forja_flora::SpeciesLibrary::load(Some(&root.join("data/flora/species.toml")));
        let season = std::env::var("CITY_SEASON").ok().and_then(|s| forja_flora::Season::parse(&s)).unwrap_or_default();
        let opt = forja_flora::city::PlantOptions { variants: 3, season, seed: cfg.seed };
        let forest = forja_flora::city::plant(&city.props, &city.prop_sites, &species, &opt);
        let handles: Vec<ModelHandle> = forest.models.iter().map(|t| r.upload_scene(&t.scene)).collect();
        println!("trees: {} instances of {} models ({:?}) in {:.2}s", forest.instances.len(), handles.len(), forest.by_species.keys().collect::<Vec<_>>(), t1.elapsed().as_secs_f32());
        let point = forja_math::Aabb { min: Vec3::splat(-4.0), max: Vec3::splat(4.0) };
        frame.instances.extend(forest.instances.iter().filter(|(_, t)| near(point, t)).map(|(m, t)| DrawInstance::new(handles[*m], *t)));
    }
    let bench = lib.prop("polyhaven_street_bench").map(|s| r.upload_scene(&s));
    println!("uploaded {} of {} placements in {:.2}s", frame.instances.len(), out.placements.len(), t0.elapsed().as_secs_f32());
    // Countryside under and around the map, so aerials do not look down
    // into the environment map.
    let mut land: forja_city::geom::Parts<forja_city::Mat> = forja_city::geom::Parts::new();
    let w = 4000.0;
    land.get(forja_city::Mat::Grass).polygon(&[Vec2::new(-w, -w), Vec2::new(w, -w), Vec2::new(w, w), Vec2::new(-w, w)], -0.3, true, 80.0);
    let land = r.upload_scene(&forja_city::kit::assemble(&kit, "countryside", land, None));
    frame.instances.push(DrawInstance { cast_shadows: false, ..DrawInstance::new(land, Mat4::IDENTITY) });
    for (k, m) in &city.props {
        let h = match k {
            PropKind::Tree | PropKind::Palm => tree,
            PropKind::Bench => bench,
        };
        if let Some(h) = h.filter(|_| near(forja_math::Aabb { min: Vec3::splat(-2.0), max: Vec3::splat(2.0) }, m)) {
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
    for (name, eye, at) in shots {
        frame.camera = Camera::look_at(eye, at);
        frame.camera.fov_y = if matches!(name, "alley" | "deadend" | "slope" | "stairs") { 70f32.to_radians() } else { 60f32.to_radians() };
        let fov = frame.camera.fov_y;
        if night && legacy {
            frame.instances = base_instances.clone();
            frame.lights = night_lights(&out.city_lights, eye, at - eye);
            let glow = forja_city::nightlight::glow_parts(&out.city_lights, eye, 45.0, 2.0 * (fov * 0.5).tan() / 720.0, 1.6);
            let h = r.upload_scene(&forja_city::kit::assemble(&kit, "glow", glow, None));
            frame.instances.push(DrawInstance { cast_shadows: false, ..DrawInstance::new(h, Mat4::IDENTITY) });
        }
        if let Some(n) = std::env::var("CITY_BENCH").ok().and_then(|v| v.parse::<u32>().ok()) {
            bench_frames(&mut r, &frame, name, n);
        }
        let img = r.render_to_image(&frame, 1280, 720);
        let path = format!("{prefix}_{name}{suffix}.png");
        img.save(&path).unwrap();
        println!("{path}: eye {eye} at {at} {:?}", r.stats);
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
                frame.camera.fov_y = fov;
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

/// The lights the current renderer can take, chosen by importance.
fn night_lights(all: &[forja_city::CityLight], eye: Vec3, fwd: Vec3) -> Vec<PointLight> {
    forja_city::nightlight::select(all, eye, fwd, 64)
        .into_iter()
        .map(|i| {
            let (position, color, intensity, radius) = all[i].as_point();
            PointLight { position, radius, color, intensity }
        })
        .collect()
}

/// An outline that is not a (chamfered) rectangle: some corner turns by
/// an angle a grid does not make.
fn irregular(o: &[Vec2]) -> bool {
    let n = o.len();
    (0..n).any(|i| {
        let (a, b) = ((o[i] - o[(i + n - 1) % n]).normalize_or(Vec2::X), (o[(i + 1) % n] - o[i]).normalize_or(Vec2::X));
        let turn = a.dot(b).clamp(-1.0, 1.0).acos().to_degrees();
        // Square, straight and chaflán (45°) turns are regular.
        [0.0f32, 45.0, 90.0].iter().all(|r| (turn - r).abs() > 10.0)
    })
}

fn v3(p: Vec2, y: f32) -> Vec3 {
    Vec3::new(p.x, y, p.y)
}

/// Viewpoints picked from the realized layout.
fn shots(city: &forja_city::layout::CityLayout) -> Vec<(&'static str, Vec3, Vec3)> {
    let f = &city.fabric;
    let g = &f.graph;
    let mut out = Vec::new();
    let [lo, hi] = f.extent;
    let c = (lo + hi) * 0.5;
    let size = hi - lo;
    // Oblique, from the middle of a long side so the city spans the frame
    // (16:9 at 60° vertical fov is about 91° across).
    let (long, side) = if size.x >= size.y { (size.x, Vec2::new(0.0, 1.0)) } else { (size.y, Vec2::new(1.0, 0.0)) };
    let reach = long * 0.5 + size.min_element() * 0.35;
    out.push(("aerial", v3(c + side * (size.min_element() * 0.5 + reach * 0.62), reach * 0.55), v3(c - side * size.min_element() * 0.08, 0.0)));
    out.push(("overview", v3(c + Vec2::new(0.0, size.y * 0.02), size.max_element() * 1.05), v3(c + Vec2::new(0.0, -size.y * 0.001), 0.0)));
    // An eye stands in open public space: 0.8 m from facades, 1.5 m from
    // any lamp, tree, sign or other item.
    let public = forja_city::plan::Located::new(f.public.clone());
    let things: Vec<Vec2> = city.items.iter().map(|i| Vec2::new(i.pos.x, i.pos.z)).chain(city.props.iter().map(|p| Vec2::new(p.1.w_axis.x, p.1.w_axis.z))).collect();
    let open = |p: Vec2, r: f32| public.contains_disc(p, r) && things.iter().all(|q| q.distance(p) > 1.5);
    let clear = |p: Vec2| open(p, 0.8);
    // Eye-level shot along street `s` from about `t`, `side` from the centre:
    // the first clear spot stepping 1.5 m along the street.
    let along = |s: usize, t: f32, side: f32, look: f32, eye_h: f32| {
        let st = &g.streets[s];
        let len = st.len();
        let at = |t: f32| st.line.at(t) + st.line.normal(t) * side;
        let t = (0..40).map(|k| t + 1.5 * k as f32).filter(|t| *t < len - 2.0).find(|t| clear(at(*t))).unwrap_or(t);
        let p = at(t);
        let q = st.line.at((t + look).min(len)) + st.line.normal((t + look).min(len)) * side * 0.2;
        (v3(p, eye_h + f.ground.height_at(p)), v3(q, eye_h + 1.5 + f.ground.height_at(q)))
    };
    let relief = &f.ground.relief;
    // Away from the edge of the map.
    let inner = |p: Vec2| f.domain.boundary_distance(p) > 35.0;
    let longest = |pred: &dyn Fn(&forja_city::plan::Street) -> bool| g.streets.iter().enumerate().filter(|(_, s)| pred(s)).max_by(|a, b| a.1.len().total_cmp(&b.1.len())).map(|x| x.0);
    let prof = |s: &forja_city::plan::Street| forja_city::profile::ProfileLibrary::builtin().get(s.profile).clone();
    let lib = forja_city::profile::ProfileLibrary::builtin();
    // The grandest avenue (highest rank, then longest).
    let grand = g.streets.iter().enumerate().filter(|(_, s)| lib.get(s.profile).class == Class::Avenue).max_by(|a, b| (lib.get(a.1.profile).rank, a.1.len()).partial_cmp(&(lib.get(b.1.profile).rank, b.1.len())).unwrap()).map(|x| x.0);
    if let Some(s) = grand {
        let st = &g.streets[s];
        // From the middle of one carriageway (lanes, median, trees in
        // view), and from its sidewalk.
        let (e, a) = along(s, st.len() * 0.3, st.carriageway * 0.3, 80.0, 2.2);
        out.push(("avenue", e, a));
        let (e, a) = along(s, st.len() * 0.3, st.carriageway * 0.5 + 1.2, 60.0, 1.7);
        out.push(("avenue_walk", e, a));
    }
    if let Some(s) = longest(&|s| lib.get(s.profile).id == "ensanche") {
        let st = &g.streets[s];
        let (e, a) = along(s, 8.0, st.width_at(8.0) * 0.5 - 1.2, 45.0, 1.7);
        out.push(("ensanche", e, a));
    }
    if let Some(s) = longest(&|s| lib.get(s.profile).id == "madrid_calle") {
        let st = &g.streets[s];
        let (e, a) = along(s, 6.0, st.width_at(6.0) * 0.5 - 1.0, 40.0, 1.7);
        out.push(("madrid", e, a));
    }
    // The most curved street of some length.
    // How far the street turns from its start to its end (the overall
    // bend, not the wobble of its chords).
    let turn = |s: &forja_city::plan::Street| {
        let l = s.len();
        s.line.normal(4.0).angle_to(s.line.normal(l - 4.0)).abs()
    };
    if let Some((s, _)) = g.streets.iter().enumerate().filter(|(_, s)| s.len() > 40.0 && s.carriageway > 0.0 && inner(s.line.at(s.len() * 0.5))).max_by(|a, b| turn(a.1).total_cmp(&turn(b.1))) {
        let st = &g.streets[s];
        let (e, a) = along(s, 4.0, st.width_at(4.0) * 0.25, 30.0, 1.7);
        out.push(("curved", e, a));
    }
    // A narrow alley at eye level, looking into the old town (not out at
    // the edge of the map).
    if let Some((s, _)) = g.streets.iter().enumerate().filter(|(_, s)| lib.get(s.profile).class == Class::Alley && s.len() > 25.0 && inner(s.line.at((s.len() * 0.3 + 18.0).min(s.len())))).min_by(|a, b| a.1.min_width().total_cmp(&b.1.min_width()).then(b.1.len().total_cmp(&a.1.len()))) {
        let st = &g.streets[s];
        // From inside the alley, not from the square it opens onto.
        let (e, a) = along(s, st.len() * 0.3, 0.0, 18.0, 1.7);
        out.push(("alley", e, a));
        let _ = st;
    }
    // A sloping narrow alley (2-3.5 m, a ramp, the steepest), looking up
    // from its low end.
    let uphill = |s: usize, back: f32| {
        let st = &g.streets[s];
        let len = st.len();
        let up = relief.street_z(s as u32, len) > relief.street_z(s as u32, 0.0);
        let t_of = |k: f32| if up { k } else { len - k };
        let t = (0..20).map(|k| t_of(back + k as f32)).find(|t| clear(st.line.at(*t))).unwrap_or(t_of(back));
        let look = t_of(back + (len - 2.0 * back).clamp(8.0, 30.0));
        let (p, q) = (st.line.at(t), st.line.at(look));
        (v3(p, 1.7 + f.ground.height_at(p)), v3(q, 1.7 + f.ground.height_at(q)))
    };
    if !relief.is_flat() {
        if let Some((s, _)) = g.streets.iter().enumerate().filter(|(i, s)| lib.get(s.profile).class == Class::Alley && s.len() > 25.0 && s.min_width() <= 3.6 && relief.steps(*i as u32).is_none()).max_by(|a, b| relief.grade(a.0 as u32).total_cmp(&relief.grade(b.0 as u32))) {
            let (e, a) = uphill(s, 3.0);
            out.push(("slope", e, a));
        }
        // A stair street: the one with most steps, from the foot of them.
        if let Some((s, _)) = g.streets.iter().enumerate().filter_map(|(i, s)| relief.steps(i as u32).map(|st| (i, st.n)).filter(|_| s.len() > 12.0)).max_by_key(|x| x.1) {
            let st = relief.steps(s as u32).unwrap();
            let up = relief.street_z(s as u32, st.s1 + 0.1) > relief.street_z(s as u32, st.s0 - 0.1);
            let line = &g.streets[s].line;
            let foot = if up { st.s0 - 5.0 } else { st.s1 + 5.0 }.clamp(0.5, line.len() - 0.5);
            let head = if up { st.s1 + 4.0 } else { st.s0 - 4.0 }.clamp(0.5, line.len() - 0.5);
            let (p, q) = (line.at(foot), line.at(head));
            out.push(("stairs", v3(p, 1.7 + f.ground.height_at(p)), v3(q, 1.2 + f.ground.height_at(q))));
        }
    }
    // The main square (the largest plaza, an irregular one if there is
    // any), seen from across it, and the old town around it from above.
    let comps = f.ground.plaza.components();
    // Judged on the designed outline (the level terrace) where there is one.
    let designed = |c: &forja_city::plan::Region| {
        let mut t = f.ground.relief.terraces.iter().filter(|t| c.contains(t.ring.iter().copied().sum::<Vec2>() / t.ring.len() as f32)).peekable();
        if t.peek().is_some() { t.any(|t| irregular(&t.ring)) } else { c.simplify(0.5).shapes().iter().any(|s| irregular(&s.outer)) }
    };
    let mut plazas: Vec<(Vec2, f32, bool)> = comps.iter().filter_map(|c| c.pole().map(|p| (p.0, p.1, designed(c)))).filter(|p| p.1 > 5.0).collect();
    plazas.sort_by(|a, b| (a.2, a.1).partial_cmp(&(b.2, b.1)).unwrap());
    if let Some(&(p, r, _)) = plazas.last() {
        let eye = (0..24).map(|k| p + Vec2::from_angle(2.0 + k as f32 * 0.9) * r * (0.9 - 0.02 * k as f32)).find(|q| clear(*q)).unwrap_or(p + Vec2::new(r * 0.8, r * 0.5));
        out.push(("plaza", v3(eye, 1.7 + f.ground.height_at(eye)), v3(p, 2.5 + f.ground.height_at(p))));
        let h = f.ground.height_at(p);
        out.push(("aerial_old", v3(p + Vec2::new(-95.0, 120.0), h + 105.0), v3(p + Vec2::new(0.0, -15.0), h)));
    }
    plazas.sort_by(|a, b| a.1.total_cmp(&b.1));
    // A plazuela: the smallest plaza, seen from its edge.
    if let Some(&(p, r, _)) = plazas.first() {
        let eye = (0..24).map(|k| p + Vec2::from_angle(k as f32 * 0.9) * r * (0.85 - 0.02 * k as f32)).find(|q| clear(*q)).unwrap_or(p + Vec2::new(r * 0.8, r * 0.5));
        // Past the fountain or parterre at its pole, at the facades beyond.
        let q = p + (p - eye).normalize_or(Vec2::X) * r;
        out.push(("plazuela", v3(eye, 1.7 + f.ground.height_at(eye)), v3(q, 4.0 + f.ground.height_at(q))));
    }
    // The sharpest corner, from across its junction.
    let mut best: Option<(f32, Vec2, Vec2)> = None;
    for b in &f.blocks {
        let n = b.shape.outer.len();
        for c in &b.corners {
            if best.is_none_or(|x| c.theta < x.0) {
                let i = b.shape.outer.iter().position(|q| q.distance(c.apex) < 15.0).unwrap_or(0);
                let a = b.shape.outer[(i + n / 2) % n];
                best = Some((c.theta, c.apex, (c.apex - a).normalize_or(Vec2::X)));
            }
        }
    }
    if let Some((_, apex, out_dir)) = best {
        let eye = (0..30).map(|k| apex + out_dir.rotate(Vec2::from_angle((k as f32 * 0.7).sin() * 0.5)) * (14.0 + k as f32 * 0.5)).find(|p| open(*p, 2.5)).unwrap_or(apex + out_dir * 18.0);
        out.push(("acute", v3(eye, 1.7 + f.ground.height_at(eye)), v3(apex, 6.0 + f.ground.height_at(apex))));
    }
    // A dead end (an adarve, the longest), from 15 m back.
    let dead = g.streets.iter().flat_map(|s| [End::A, End::B].map(|e| (s, e))).filter(|(s, e)| g.degree(s.node(*e)) == 1 && s.len() > 18.0);
    if let Some((s, e)) = dead.max_by(|a, b| (lib.get(a.0.profile).class == Class::Alley, a.0.len()).partial_cmp(&(lib.get(b.0.profile).class == Class::Alley, b.0.len())).unwrap()) {
        let toward = |k: usize| if e == End::A { 15.0 + k as f32 } else { s.len() - 15.0 - k as f32 };
        let t = (0..10).map(toward).find(|t| clear(s.line.at(*t))).unwrap_or(toward(0));
        let (p, end) = (s.line.at(t), s.line.at(s.end_s(e)));
        let _ = prof(s);
        out.push(("deadend", v3(p, 1.7 + f.ground.height_at(p)), v3(end, 3.0 + f.ground.height_at(end))));
    }
    out
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
