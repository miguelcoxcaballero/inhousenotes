//! Whole procedural Spanish city rendered from a few viewpoints:
//! `cargo run --release -p forja_render --example city -- out_prefix [preset] [shot,...]`
//!
//! Presets: `default` (grids, boulevard and the Austrias and Carmen old
//! towns), `classic`, `small`, `carmen`, `austrias`, `organic`,
//! `osm:<file.json|file.osm>`. Shots are picked from the realized streets:
//! aerial, avenue, ensanche, madrid, curved, alley (eye level 1.7 m),
//! plazuela, acute (corner), deadend, overview.

use std::path::PathBuf;

use forja_city::kit::Kit;
use forja_city::layout::{CityConfig, PropKind, build_city};
use forja_city::plan::End;
use forja_city::profile::Class;
use forja_city::style::StyleLibrary;
use forja_math::{Mat4, Vec2, Vec3};
use forja_render::*;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let prefix = args.get(1).cloned().unwrap_or_else(|| "city".into());
    let preset = args.get(2).cloned().unwrap_or_else(|| "default".into());
    let only: Option<Vec<String>> = args.get(3).map(|s| s.split(',').map(String::from).collect());
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dl = root.join("assets/downloaded");
    let t0 = std::time::Instant::now();
    let kit = Kit::load(Some(&dl));
    let styles = StyleLibrary::load(Some(&root.join("data/city/styles.toml")));
    let cfg = if let Some(path) = preset.strip_prefix("osm:") {
        forja_world::spain::SpanishCity::config_from_osm(std::path::Path::new(path)).unwrap_or_else(|e| panic!("osm {path}: {e}"))
    } else {
        CityConfig::preset(&preset).unwrap_or_else(|| panic!("unknown preset {preset}"))
    };
    let city = build_city(&kit, &styles, &cfg);
    let out = &city.out;
    let tris: usize = out.placements.iter().map(|(s, _)| out.scenes[*s].triangle_count()).sum();
    println!("city {preset}: {:?}, {} scenes, {} placements, {} tris, extent {:?} in {:.2}s", city.stats, out.scenes.len(), out.placements.len(), tris, city.extent, t0.elapsed().as_secs_f32());
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
    let shots = shots(&city);
    for (name, eye, at) in shots {
        if only.as_ref().is_some_and(|o| !o.iter().any(|x| x == name)) {
            continue;
        }
        frame.camera = Camera::look_at(eye, at);
        frame.camera.fov_y = if name == "alley" || name == "deadend" { 70f32.to_radians() } else { 60f32.to_radians() };
        let img = r.render_to_image(&frame, 1280, 720);
        let path = format!("{prefix}_{name}.png");
        img.save(&path).unwrap();
        println!("{path}: eye {eye} at {at} {:?}", r.stats);
    }
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
    out.push(("aerial", v3(Vec2::new(lo.x - size.x * 0.1, hi.y + size.y * 0.25), size.max_element() * 0.45), v3(c, 0.0)));
    out.push(("overview", v3(c + Vec2::new(0.0, size.y * 0.02), size.max_element() * 1.05), v3(c + Vec2::new(0.0, -size.y * 0.001), 0.0)));
    // Eye-level shot from `t` along street `s`, `side` from the centre.
    let along = |s: usize, t: f32, side: f32, look: f32, eye_h: f32| {
        let st = &g.streets[s];
        let p = st.line.at(t) + st.line.normal(t) * side;
        let q = st.line.at((t + look).min(st.len())) + st.line.normal((t + look).min(st.len())) * side * 0.2;
        (v3(p, eye_h + f.ground.height_at(p)), v3(q, eye_h + 3.0))
    };
    let longest = |pred: &dyn Fn(&forja_city::plan::Street) -> bool| g.streets.iter().enumerate().filter(|(_, s)| pred(s)).max_by(|a, b| a.1.len().total_cmp(&b.1.len())).map(|x| x.0);
    let prof = |s: &forja_city::plan::Street| forja_city::profile::ProfileLibrary::builtin().get(s.profile).clone();
    let lib = forja_city::profile::ProfileLibrary::builtin();
    if let Some(s) = longest(&|s| lib.get(s.profile).class == Class::Avenue) {
        let st = &g.streets[s];
        let (e, a) = along(s, st.len() * 0.3, st.carriageway * 0.5 + 1.2, 60.0, 1.7);
        out.push(("avenue", e, a));
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
    let turn = |s: &forja_city::plan::Street| {
        let p = s.line.pts();
        p.windows(3).map(|w| (w[1] - w[0]).normalize_or(Vec2::X).angle_to((w[2] - w[1]).normalize_or(Vec2::X)).abs()).sum::<f32>()
    };
    if let Some((s, _)) = g.streets.iter().enumerate().filter(|(_, s)| s.len() > 40.0 && s.carriageway > 0.0).max_by(|a, b| turn(a.1).total_cmp(&turn(b.1))) {
        let st = &g.streets[s];
        let (e, a) = along(s, 4.0, st.width_at(4.0) * 0.25, 30.0, 1.7);
        out.push(("curved", e, a));
    }
    // A narrow alley at eye level.
    if let Some((s, _)) = g.streets.iter().enumerate().filter(|(_, s)| lib.get(s.profile).class == Class::Alley && s.len() > 25.0).min_by(|a, b| a.1.min_width().total_cmp(&b.1.min_width()).then(b.1.len().total_cmp(&a.1.len()))) {
        let st = &g.streets[s];
        let (e, a) = along(s, 3.0, 0.0, 18.0, 1.7);
        out.push(("alley", e, a));
        let _ = st;
    }
    // A plazuela: the smallest plaza, seen from its edge.
    let mut plazas: Vec<(Vec2, f32)> = f.ground.plaza.components().iter().filter_map(|c| c.pole()).filter(|p| p.1 > 5.0).collect();
    plazas.sort_by(|a, b| a.1.total_cmp(&b.1));
    if let Some(&(p, r)) = plazas.first() {
        let eye = p + Vec2::new(r * 0.8, r * 0.5);
        out.push(("plazuela", v3(eye, 1.7 + f.ground.height_at(eye)), v3(p, 2.0)));
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
        let eye = apex + out_dir * 18.0;
        out.push(("acute", v3(eye, 1.7 + f.ground.height_at(eye)), v3(apex, 6.0)));
    }
    // A dead end, from 15 m back.
    for (si, s) in g.streets.iter().enumerate() {
        for e in [End::A, End::B] {
            if g.degree(s.node(e)) == 1 && s.len() > 18.0 {
                let t = if e == End::A { 15.0 } else { s.len() - 15.0 };
                let end = s.line.at(s.end_s(e));
                let p = s.line.at(t);
                out.push(("deadend", v3(p, 1.7 + f.ground.height_at(p)), v3(end, 3.0)));
                let _ = (si, prof(s));
                return out;
            }
        }
    }
    out
}
