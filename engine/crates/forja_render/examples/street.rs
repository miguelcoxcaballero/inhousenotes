//! Procedural Spanish buildings rendered at eye level, for visual review:
//! `cargo run --release -p forja_render --example street -- out_prefix [mode|style]`
//!
//! Modes:
//! - (default, or a style id) a straight street, two houses per style;
//! - `alley`: a 3 m callejón of old-town houses lit from the facades;
//! - `curve`: a lane curving at R = 30 m with houses on both sides;
//! - `corner`: a rounded Ensanche chaflán and a 60° corner bevelled to a
//!   2.5 m tip, with their neighbours.
//!
//! `NIGHT=1` (or `HOUR=<h>`) renders any of them at night, lit by their own
//! lanterns, windows and shops.
//!
//! Night presets: `DUSK=1`, `RAIN=1`, `WET=1` pick the other environments,
//! `LEGACY=1` the previous 64-light path. At night the default street also
//! gets kerbside candelabras, two small neon signs, shop light spilling onto
//! the pavement and a low `along` shot down the road. `STREET_SHOTS=wide,along`
//! renders only those shots.

use std::path::PathBuf;

use forja_city::building::{self, CityOut, EdgeKind, Lot, ModuleLib};
use forja_city::geom::Parts;
use forja_city::kit::{Kit, Mat, assemble};
use forja_city::style::StyleLibrary;
use forja_city::{Front, FrontRules, Rhythm};
use forja_math::{Mat4, Vec2, Vec3};
use forja_render::*;

use EdgeKind::*;

fn v(x: f32, z: f32) -> Vec2 {
    Vec2::new(x, z)
}

/// A lot whose street edges face `fr`, with `along` measured by `along`.
fn lot(ring: Vec<Vec2>, edges: Vec<EdgeKind>, base: f32, fr: Front, along: impl Fn(Vec2) -> f32) -> Lot {
    let n = ring.len();
    let fronts = (0..n).map(|i| if edges[i].is_street() { Front { along: [along(ring[i]), along(ring[(i + 1) % n])], ..fr } } else { Front::UNCONSTRAINED }).collect();
    let l = Lot { ring, edges, base, fronts, levels: None, roof_levels: None };
    if let Err(e) = l.check() {
        panic!("review lot breaks the contract: {e}");
    }
    l
}

/// A review scene: lots with their style ids, the ground, eye-level shots.
struct Scene {
    lots: Vec<(Lot, &'static str)>,
    ground: Parts<Mat>,
    shots: Vec<(String, Vec3, Vec3)>,
    /// The x range of the default review street (night dressing goes along it).
    road: Option<(f32, f32)>,
}

/// Two rows of old-town houses across a 3.2 m alley along x, on a shared
/// cobbled surface with a central gutter, lanterns every 14 m staggered.
fn alley() -> Scene {
    let widths = [4.2, 5.6, 3.8, 6.4, 4.6, 5.2, 3.6, 6.0, 4.4, 5.0];
    let clear = 3.2;
    let mut lots = Vec::new();
    for (row, phase) in [(0, 0.0f32), (1, 7.0)] {
        let mut x = -25.0;
        for (k, w) in widths.iter().enumerate() {
            let w = if row == 0 { *w } else { widths[(k + 3) % widths.len()] };
            let depth = 9.0 + ((k * 7 + row * 3) % 5) as f32;
            let fr = Front { lamps: Some(Rhythm { pitch: 14.0, phase }), depth, ..FrontRules::default().front(clear, clear * 0.5) };
            let style = if (k + row) % 3 == 2 { "madrid_popular" } else { "valencia_carmen" };
            let (x0, x1) = (x, x + w);
            let l = if row == 0 {
                lot(vec![v(x0, -depth), v(x1, -depth), v(x1, 0.0), v(x0, 0.0)], vec![Courtyard, Party, Street, Party], 0.15, fr, |p| p.x + 30.0)
            } else {
                lot(vec![v(x0, clear), v(x1, clear), v(x1, clear + depth), v(x0, clear + depth)], vec![Street, Party, Courtyard, Party], 0.15, fr, |p| p.x + 30.0)
            };
            lots.push((l, style));
            x = x1;
        }
    }
    let mut ground: Parts<Mat> = Parts::new();
    let (x0, x1) = (-26.0, 26.0);
    ground.get(Mat::Cobble).polygon(&[v(x0, 0.0), v(x1, 0.0), v(x1, clear), v(x0, clear)], 0.15, true, Mat::Cobble.scale());
    ground.get(Mat::Granite).polygon(&[v(x0, clear * 0.5 - 0.18), v(x1, clear * 0.5 - 0.18), v(x1, clear * 0.5 + 0.18), v(x0, clear * 0.5 + 0.18)], 0.155, true, 1.0);
    let shots = vec![
        ("along".into(), Vec3::new(-20.0, 1.7, 1.1), Vec3::new(10.0, 3.6, 2.0)),
        ("up".into(), Vec3::new(-2.0, 1.6, 2.2), Vec3::new(4.0, 9.0, 0.6)),
        ("back".into(), Vec3::new(19.0, 1.7, 2.1), Vec3::new(-10.0, 4.0, 1.2)),
    ];
    Scene { lots, ground, shots, road: None }
}

/// A lane 7 m wide curving at R = 30 m (centreline), houses on the convex
/// and the concave side, setts between narrow sidewalks.
fn curve() -> Scene {
    let (rc, half) = (30.0f32, 3.5f32);
    let at = |a: f32, r: f32| v(a.sin(), -a.cos()) * r;
    let along = move |p: Vec2| 100.0 + rc * p.x.atan2(-p.y);
    let fr = Front { lamps: Some(Rhythm { pitch: 15.0, phase: 0.0 }), ..FrontRules::default().front(2.0 * half, 1.2) };
    let styles = ["madrid_popular", "madrid_centro", "madrid_ladrillo", "madrid_popular"];
    let mut lots = Vec::new();
    for side in [0, 1] {
        let (r, back) = if side == 0 { (rc - half, rc - half - 12.0) } else { (rc + half, rc + half + 12.0) };
        let mut a = -0.75f32;
        let mut k = 0;
        while a < 0.75 {
            let wid = [5.5f32, 7.0, 4.5, 8.0, 6.0][(k + side * 2) % 5];
            let b = (a + wid / r).min(0.75);
            let m = ((b - a) * r / 2.8).ceil().max(1.0) as usize;
            let fr = Front { lamps: Some(Rhythm { pitch: 15.0, phase: if side == 0 { 0.0 } else { 7.5 } }), depth: 12.0, ..fr };
            let (mut ring, mut edges);
            if side == 0 {
                ring = (0..=m).map(|i| at(a + (b - a) * i as f32 / m as f32, r)).collect::<Vec<_>>();
                edges = vec![Street; m];
                ring.extend([at(b, back), at(a, back)]);
            } else {
                ring = (0..=m).map(|i| at(b - (b - a) * i as f32 / m as f32, r)).collect::<Vec<_>>();
                edges = vec![Street; m];
                ring.extend([at(a, back), at(b, back)]);
            }
            edges.extend([Party, Courtyard, Party]);
            lots.push((lot(ring, edges, 0.15, fr, along), styles[(k + side) % styles.len()]));
            a = b;
            k += 1;
        }
    }
    let mut ground: Parts<Mat> = Parts::new();
    let band = |r0: f32, r1: f32| -> Vec<Vec2> {
        let n = 60;
        let mut p: Vec<Vec2> = (0..=n).map(|i| at(-0.8 + 1.6 * i as f32 / n as f32, r1)).collect();
        p.extend((0..=n).rev().map(|i| at(-0.8 + 1.6 * i as f32 / n as f32, r0)));
        if forja_city::geom::signed_area(&p) < 0.0 {
            p.reverse();
        }
        p
    };
    ground.get(Mat::SidewalkSlab).polygon(&band(rc - half, rc - half + 1.2), 0.15, true, Mat::SidewalkSlab.scale());
    ground.get(Mat::SidewalkSlab).polygon(&band(rc + half - 1.2, rc + half), 0.15, true, Mat::SidewalkSlab.scale());
    ground.get(Mat::Cobble).polygon(&band(rc - half + 1.2, rc + half - 1.2), 0.02, true, Mat::Cobble.scale());
    let eye = |a: f32, r: f32, h: f32| {
        let p = at(a, r);
        Vec3::new(p.x, h, p.y)
    };
    let shots = vec![
        ("along".into(), eye(-0.45, rc - 1.0, 1.7), eye(0.05, rc + 1.0, 4.5)),
        ("concave".into(), eye(-0.2, rc - 2.0, 1.7), eye(0.1, rc + half, 6.0)),
        ("convex".into(), eye(0.15, rc + 2.0, 1.7), eye(-0.15, rc - half, 6.0)),
    ];
    Scene { lots, ground, shots, road: None }
}

/// A rounded Ensanche chaflán (r = 9, six chords) and, further along, a
/// 60° Madrid corner bevelled to a 2.5 m tip, each with neighbours.
fn corner() -> Scene {
    let mut lots = Vec::new();
    // Rounded corner block quarter: streets along x = 20 (east) and z = 20 (north).
    let ens = FrontRules::default().front(20.0, 4.5);
    let mut ring = vec![v(0.0, 0.0), v(20.0, 0.0), v(20.0, 11.0)];
    let mut edges = vec![Party, Street];
    let c = v(11.0, 11.0);
    for k in 0..6 {
        let a = (k as f32 + 1.0) * 15f32.to_radians();
        ring.push(c + v(a.cos(), a.sin()) * 9.0);
        edges.push(Corner);
    }
    edges.push(Street);
    ring.push(v(0.0, 20.0));
    edges.push(Party);
    lots.push((lot(ring, edges, 0.15, ens, |p| p.x + p.y), "valencia_modernista"));
    lots.push((lot(vec![v(0.0, -14.0), v(20.0, -14.0), v(20.0, 0.0), v(0.0, 0.0)], vec![Courtyard, Street, Party, Party], 0.15, ens, |p| p.y + 20.0), "valencia_ensanche"));
    lots.push((lot(vec![v(-14.0, 0.0), v(0.0, 0.0), v(0.0, 20.0), v(-14.0, 20.0)], vec![Courtyard, Party, Street, Party], 0.15, ens, |p| p.x + 20.0), "valencia_ensanche"));
    // The 60° corner, at x = 60.
    let o = v(60.0, 0.0);
    let mad = FrontRules::default().front(11.0, 2.0);
    let u2 = v(0.5, 3f32.sqrt() * 0.5);
    let in2 = v(u2.y, -u2.x);
    let (p1, q1, r1) = (o + v(2.5, 0.0), o + v(16.0, 0.0), o + v(16.0, 7.0));
    let (p2, q2) = (o + u2 * 2.5, o + u2 * 16.0);
    let r2 = q2 + in2 * 7.0;
    let dist = move |p: Vec2| (p - o).length();
    lots.push((lot(vec![p1, q1, r1, r2, q2, p2], vec![Street, Party, Courtyard, Party, Street, Corner], 0.15, mad, dist), "madrid_centro"));
    lots.push((lot(vec![q1, o + v(28.0, 0.0), o + v(28.0, 7.0), r1], vec![Street, Party, Courtyard, Party], 0.15, mad, dist), "madrid_ladrillo"));
    let (q3, r3) = (o + u2 * 26.0, o + u2 * 26.0 + in2 * 7.0);
    lots.push((lot(vec![q3, q2, r2, r3], vec![Street, Party, Courtyard, Party], 0.15, mad, dist), "madrid_popular"));
    // Ground: asphalt with sidewalks along every street front.
    let mut ground: Parts<Mat> = Parts::new();
    ground.get(Mat::Asphalt).polygon(&[v(-60.0, -60.0), v(140.0, -60.0), v(140.0, 80.0), v(-60.0, 80.0)], 0.0, true, Mat::Asphalt.scale());
    for (l, _) in &lots {
        let n = l.ring.len();
        for i in 0..n {
            if !l.edges[i].is_street() {
                continue;
            }
            let (a, b) = l.edge(i);
            let d = (b - a).normalize();
            let nrm = v(d.y, -d.x) * l.front(i).walk.min(4.5);
            ground.get(Mat::SidewalkSlab).polygon(&[a, a + nrm, b + nrm, b], 0.15, true, Mat::SidewalkSlab.scale());
        }
    }
    let shots = vec![
        ("chaflan".into(), Vec3::new(34.0, 1.7, 34.0), Vec3::new(14.0, 9.0, 14.0)),
        ("chaflan_side".into(), Vec3::new(27.0, 1.7, -8.0), Vec3::new(16.0, 8.0, 14.0)),
        ("acute".into(), Vec3::new(55.0, 1.7, -9.0), Vec3::new(64.0, 7.0, 5.0)),
        ("acute_tip".into(), Vec3::new(57.5, 1.7, -2.5), Vec3::new(61.5, 6.0, 1.8)),
    ];
    Scene { lots, ground, shots, road: None }
}

/// The original review street: two houses per style along a 4.5 m sidewalk.
fn street(styles: &StyleLibrary, only: Option<&str>) -> Scene {
    let list: Vec<_> = styles.styles.iter().filter(|s| only.is_none_or(|o| s.id == o)).collect();
    let mut x = -(list.len() as f32) * 17.0 * 0.5;
    let x_start = x;
    let mut lots = Vec::new();
    let mut shots: Vec<(String, Vec3, Vec3)> = vec![("wide".into(), Vec3::new(-30.0, 1.7, 22.0), Vec3::new(10.0, 9.0, 0.0))];
    for (i, s) in list.iter().enumerate() {
        for k in 0..2 {
            let w = 15.0 + ((i * 7 + k * 3) % 5) as f32;
            let l = Lot {
                ring: vec![v(x, -18.0), v(x + w, -18.0), v(x + w, 0.0), v(x, 0.0)],
                edges: vec![Courtyard, Party, Street, Party],
                base: 0.15,
                ..Default::default()
            };
            let id: &'static str = Box::leak(s.id.clone().into_boxed_str());
            lots.push((l, id));
            if k == 0 {
                shots.push((s.id.clone(), Vec3::new(x + w * 0.5 + 6.0, 1.7, 13.0), Vec3::new(x + w * 0.5 + 2.0, 10.0, 0.0)));
            }
            x += w;
        }
    }
    let mut p: Parts<Mat> = Parts::new();
    let (x0, x1) = (x_start - 40.0, x + 40.0);
    p.get(Mat::SidewalkHex).polygon(&[v(x0, 0.0), v(x1, 0.0), v(x1, 4.5), v(x0, 4.5)], 0.15, true, Mat::SidewalkHex.scale());
    p.get(Mat::Curb).aabox(Vec3::new(x0, 0.0, 4.5), Vec3::new(x1, 0.15, 4.75), 1.0);
    p.get(Mat::Asphalt).polygon(&[v(x0, 4.75), v(x1, 4.75), v(x1, 40.0), v(x0, 40.0)], 0.0, true, Mat::Asphalt.scale());
    let mut k = x0;
    while k < x1 {
        p.get(Mat::Paint([235, 235, 230])).aabox(Vec3::new(k, 0.0, 9.9), Vec3::new(k + 3.5, 0.006, 10.05), 1.0);
        k += 9.0;
    }
    Scene { lots, ground: p, shots, road: Some((x0, x1)) }
}

#[path = "common/night.rs"]
mod night;

fn main() {
    let prefix = std::env::args().nth(1).unwrap_or_else(|| "street".into());
    let arg: Option<String> = std::env::args().nth(2);
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dl = root.join("assets/downloaded");
    let t0 = std::time::Instant::now();
    let mut kit = Kit::load(Some(&dl));
    let mode = night::environment();
    let hour = std::env::var("HOUR")
        .ok()
        .and_then(|v| v.parse::<f32>().ok())
        .or_else(|| std::env::var("NIGHT").ok().filter(|v| v != "0").map(|_| 22.5))
        .or(mode.as_ref().map(|(n, _)| if n.starts_with("dusk") { 19.6 } else { 22.5 }));
    let legacy = mode.as_ref().is_none_or(|(n, _)| *n == "before");
    if let Some(h) = hour {
        kit.set_hour(h);
    }
    let night = kit.night.is_night();
    let styles = StyleLibrary::load(Some(&root.join("data/city/styles.toml")));
    let scene = match arg.as_deref() {
        Some("alley") => alley(),
        Some("curve") => curve(),
        Some("corner") => corner(),
        only => street(&styles, only),
    };
    let mut lib = ModuleLib::new(&kit);
    let mut out = CityOut::default();
    // One block: every house first, then the party walls between them.
    let mut parts: Parts<Mat> = Parts::new();
    for (i, (l, id)) in scene.lots.iter().enumerate() {
        let style = styles.get(id).unwrap_or_else(|| panic!("no style {id}"));
        building::build_into(&mut lib, &mut out, &mut parts, l, style, i as u64 * 31 + 5);
    }
    let mut extra: Vec<Light> = Vec::new();
    if let (true, Some((x0, x1))) = (night, scene.road) {
        // Valencian candelabras along the kerb, every 18 m.
        let mut x = x0 + 20.0;
        while x < x1 - 20.0 {
            let base = Vec3::new(x, 0.15, 3.9);
            let bulb = base + Vec3::Y * forja_city::furniture::street_lamp_bulb(true);
            let (color, _) = forja_city::nightlight::street_lamp_color(bulb, false);
            let g = forja_city::nightlight::to_rgb(color);
            let m = lib.named(&mut out, &format!("lamp_{g:?}"), || forja_city::furniture::street_lamp_lit(true, g));
            out.placements.push((m, Mat4::from_translation(base)));
            out.city_lights.push(forja_city::nightlight::street_lamp(bulb, color, true));
            x += 18.0;
        }
        // Two small neon shop signs (`Mat::Lamp` is emissive) with the
        // light they cast.
        let mut neon: Parts<Mat> = Parts::new();
        for (j, (col, light)) in [([40u8, 255u8, 90u8], Vec3::new(0.1, 1.3, 0.25)), ([60, 230, 255], Vec3::new(0.12, 1.0, 1.6))].iter().enumerate() {
            let sx = x0 + 30.0 + j as f32 * 95.0;
            neon.get(Mat::Lamp(*col)).aabox(Vec3::new(sx, 3.1, 0.15), Vec3::new(sx + 2.4, 3.16, 0.22), 1.0);
            neon.get(Mat::Lamp(*col)).aabox(Vec3::new(sx, 3.7, 0.15), Vec3::new(sx + 2.4, 3.76, 0.22), 1.0);
            extra.push(Light::point(Vec3::new(sx + 1.2, 3.4, 0.8), *light, 1200.0).with_range(7.0).with_source_radius(1.0));
        }
        // Lit shop windows spill light onto the pavement.
        let mut k = x0 + 6.0;
        let mut i = 0;
        while k < x1 {
            if i % 3 != 2 {
                extra.push(night::shop_spill(Vec3::new(k, 2.6, -0.4), Vec3::Z, 5000.0));
            }
            k += 9.0;
            i += 1;
        }
        let neon = out.add_scene(assemble(&kit, "neon", neon, None));
        out.placements.push((neon, Mat4::IDENTITY));
    }
    building::medianeras(&mut lib, &mut out, &mut parts, 0);
    let block = out.add_scene(assemble(&kit, "block", parts, None));
    out.placements.push((block, Mat4::IDENTITY));
    let ground = out.add_scene(assemble(&kit, "ground", scene.ground, None));
    out.placements.push((ground, Mat4::IDENTITY));
    let tris: usize = out.placements.iter().map(|(s, _)| out.scenes[*s].triangle_count()).sum();
    println!("built {} buildings, {} scenes, {} placements, {} tris, {} lamps in {:.2}s", out.buildings, out.scenes.len(), out.placements.len(), tris, out.lights.len(), t0.elapsed().as_secs_f32());

    let mut r = Renderer::new(&create_instance(), None).unwrap();
    if let Ok(img) = forja_assets::load_hdr(&dl.join("hdri_wide_street/wide_street_01_2k.hdr")) {
        r.set_environment(Some(&env::EnvMap::from_hdr(&img, 14.0)));
    }
    let handles: Vec<ModelHandle> = out.scenes.iter().map(|s| r.upload_scene(s)).collect();
    println!("uploaded in {:.2}s", t0.elapsed().as_secs_f32());
    // FLIP=1 turns the scene around so the facades face the sun.
    let flip = if std::env::var("FLIP").is_ok() { Mat4::from_rotation_y(std::f32::consts::PI) } else { Mat4::IDENTITY };
    let mut frame = FrameDesc::default();
    for (s, m) in &out.placements {
        frame.instances.push(DrawInstance::new(handles[*s], flip * *m));
    }
    frame.env.shadow_distance = 80.0;
    if night && !legacy {
        frame.env = mode.as_ref().map(|(_, e)| e.clone()).unwrap_or_else(Environment::night);
        frame.local_lights = out.city_lights.iter().map(night::from_city).chain(if legacy { Vec::new() } else { extra.clone() }).map(|l| night::transformed(&l, &flip)).collect();
        println!("night at {:.1} h: {} lights", kit.night.hour, frame.local_lights.len());
    } else if night {
        frame.env.use_ibl = false;
        frame.env.sun_dir = Vec3::new(-0.35, 0.72, -0.55).normalize();
        frame.env.sun_illuminance = 0.025;
        frame.env.exposure = 0.55 + 3.0 * kit.night.dark;
        frame.env.bloom = 0.08;
        println!("night at {:.1} h: {} city lights", kit.night.hour, out.city_lights.len());
    }
    let mut shots = scene.shots;
    if let (true, Some((x0, _))) = (night, scene.road) {
        // Low along the road, like a photo of a wet street at night.
        shots.push(("along".into(), Vec3::new(x0 + 45.0, 1.4, 8.5), Vec3::new(x0 + 130.0, 3.0, 4.0)));
    }
    let suffix = match &mode {
        Some((n, _)) => format!("_{n}"),
        None if night => "_night_old".to_string(),
        None => String::new(),
    };
    let only = std::env::var("STREET_SHOTS").ok();
    for (name, eye, at) in shots {
        if only.as_deref().is_some_and(|o| !o.split(',').any(|n| n.trim() == name)) {
            continue;
        }
        frame.camera = Camera::look_at(flip.transform_point3(eye), flip.transform_point3(at));
        frame.camera.fov_y = 62f32.to_radians();
        if night && legacy {
            frame.lights = forja_city::nightlight::select(&out.city_lights, eye, at - eye, 64)
                .into_iter()
                .map(|i| {
                    let (position, color, intensity, radius) = out.city_lights[i].as_point();
                    PointLight { position: flip.transform_point3(position), radius, color, intensity }
                })
                .collect();
        }
        let img = r.render_to_image(&frame, 1280, 720);
        let path = format!("{prefix}_{name}{suffix}.png");
        img.save(&path).unwrap();
        println!("{path}: {:?}", r.stats);
    }
}
