//! Street of procedural Spanish buildings, one per style, rendered at eye
//! level: `cargo run --release -p forja_render --example street -- out_prefix`
//! `NIGHT=1` / `DUSK=1` / `RAIN=1` / `WET=1` light it at night with street
//! lamps, shop windows spilling light, neon signs and a low `along` shot
//! down the (wet) road. `STREET_SHOTS=wide,along` renders only those shots.

use std::path::PathBuf;

use forja_city::building::{self, CityOut, EdgeKind, Lot, ModuleLib};
use forja_city::geom::Parts;
use forja_city::kit::{Kit, Mat, assemble};
use forja_city::style::StyleLibrary;
use forja_math::{Mat4, Vec2, Vec3};
use forja_render::*;

#[path = "common/night.rs"]
mod night;

fn main() {
    let prefix = std::env::args().nth(1).unwrap_or_else(|| "street".into());
    let only: Option<String> = std::env::args().nth(2);
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dl = root.join("assets/downloaded");
    let t0 = std::time::Instant::now();
    let kit = Kit::load(Some(&dl));
    let styles = StyleLibrary::load(Some(&root.join("data/city/styles.toml")));
    let mut lib = ModuleLib::new(&kit);
    let mut out = CityOut::default();
    let list: Vec<_> = styles.styles.iter().filter(|s| only.as_deref().is_none_or(|o| s.id == o)).collect();
    let mut x = -(list.len() as f32) * 17.0 * 0.5;
    let mut centers = Vec::new();
    for (i, s) in list.iter().enumerate() {
        for k in 0..2 {
            let w = 15.0 + ((i * 7 + k * 3) % 5) as f32;
            let lot = Lot {
                ring: vec![Vec2::new(x, -18.0), Vec2::new(x + w, -18.0), Vec2::new(x + w, 0.0), Vec2::new(x, 0.0)],
                edges: vec![EdgeKind::Courtyard, EdgeKind::Party, EdgeKind::Street, EdgeKind::Party],
                base: 0.15,
            };
            let h = building::build(&mut lib, &mut out, &lot, s, (i * 31 + k) as u64 + 5);
            centers.push((s.id.clone(), x + w * 0.5, h));
            x += w;
        }
    }
    // Sidewalk, curb and road.
    let mut p: Parts<Mat> = Parts::new();
    let (x0, x1) = (-(list.len() as f32) * 17.0 * 0.5 - 40.0, x + 40.0);
    p.get(Mat::SidewalkHex).polygon(&[Vec2::new(x0, 0.0), Vec2::new(x1, 0.0), Vec2::new(x1, 4.5), Vec2::new(x0, 4.5)], 0.15, true, Mat::SidewalkHex.scale());
    p.get(Mat::Curb).aabox(Vec3::new(x0, 0.0, 4.5), Vec3::new(x1, 0.15, 4.75), 1.0);
    p.get(Mat::Asphalt).polygon(&[Vec2::new(x0, 4.75), Vec2::new(x1, 4.75), Vec2::new(x1, 40.0), Vec2::new(x0, 40.0)], 0.0, true, Mat::Asphalt.scale());
    let mut k = x0;
    while k < x1 {
        p.get(Mat::Paint([235, 235, 230])).aabox(Vec3::new(k, 0.0, 9.9), Vec3::new(k + 3.5, 0.006, 10.05), 1.0);
        k += 9.0;
    }
    // Night dressing: lamps along the kerb, neon on some shopfronts.
    let night_env = night::environment();
    let mut lamps: Vec<Light> = Vec::new();
    if night_env.is_some() {
        let lamp = lib.named(&mut out, "street_lamp", || forja_city::furniture::street_lamp(false));
        let mut k = x0 + 6.0;
        let mut i = 0;
        while k < x1 {
            out.placements.push((lamp, Mat4::from_translation(Vec3::new(k, 0.15, 4.1))));
            lamps.push(Light::point(Vec3::new(k, 3.45, 4.1), kelvin(2700.0), 4000.0).with_source_radius(0.15));
            // Every other bay a lit shop: light spilling out of the window.
            for s in [0.0f32, 9.0] {
                if (i + s as usize) % 3 != 2 {
                    lamps.push(night::shop_spill(Vec3::new(k + s, 2.6, -0.4), Vec3::Z, 5000.0));
                }
            }
            k += 18.0;
            i += 1;
        }
        // Neon: red arches over three shopfronts and a few green and cyan
        // signs (`Mat::Lamp` is emissive).
        let mut neon: Parts<Mat> = Parts::new();
        let red = Mat::Lamp([255, 30, 24]);
        let mut cx = x0 + 52.0;
        for _ in 0..3 {
            for a in 0..5 {
                let c = cx + a as f32 * 3.1;
                let mut prev = Vec3::new(c - 1.35, 3.0, 0.25);
                for s in 1..=10 {
                    let t = s as f32 / 10.0 * std::f32::consts::PI;
                    let q = Vec3::new(c - 1.35 * t.cos(), 3.0 + 1.0 * t.sin(), 0.25);
                    neon.get(red).beam(prev, q, Vec3::Z, 0.05, 0.05, 1.0);
                    prev = q;
                }
            }
            lamps.push(Light::point(Vec3::new(cx + 6.2, 3.6, 0.9), Vec3::new(3.2, 0.12, 0.08), 2500.0).with_range(12.0).with_source_radius(1.5));
            cx += 70.0;
        }
        for (j, (col, light)) in [([40u8, 255u8, 90u8], Vec3::new(0.1, 1.3, 0.25)), ([60, 230, 255], Vec3::new(0.12, 1.0, 1.6))].iter().enumerate() {
            let sx = x0 + 30.0 + j as f32 * 95.0;
            neon.get(Mat::Lamp(*col)).aabox(Vec3::new(sx, 3.1, 0.15), Vec3::new(sx + 2.4, 3.16, 0.22), 1.0);
            neon.get(Mat::Lamp(*col)).aabox(Vec3::new(sx, 3.7, 0.15), Vec3::new(sx + 2.4, 3.76, 0.22), 1.0);
            lamps.push(Light::point(Vec3::new(sx + 1.2, 3.4, 0.8), *light, 3000.0).with_range(9.0).with_source_radius(1.0));
        }
        let neon = out.add_scene(assemble(&kit, "neon", neon, None));
        out.placements.push((neon, Mat4::IDENTITY));
    }
    let street = out.add_scene(assemble(&kit, "street", p, None));
    out.placements.push((street, Mat4::IDENTITY));
    let tris: usize = out.placements.iter().map(|(s, _)| out.scenes[*s].triangle_count()).sum();
    println!("built {} buildings, {} scenes, {} placements, {} tris in {:.2}s", out.buildings, out.scenes.len(), out.placements.len(), tris, t0.elapsed().as_secs_f32());

    let mut r = Renderer::new(&create_instance(), None).unwrap();
    if let Ok(img) = forja_assets::load_hdr(&dl.join("hdri_wide_street/wide_street_01_2k.hdr")) {
        r.set_environment(Some(&env::EnvMap::from_hdr(&img, 14.0)));
    }
    let handles: Vec<ModelHandle> = out.scenes.iter().map(|s| r.upload_scene(s)).collect();
    println!("uploaded in {:.2}s", t0.elapsed().as_secs_f32());
    // FLIP=1 turns the street around so the facades face the sun.
    let flip = if std::env::var("FLIP").is_ok() { Mat4::from_rotation_y(std::f32::consts::PI) } else { Mat4::IDENTITY };
    let mut frame = FrameDesc::default();
    for (s, m) in &out.placements {
        frame.instances.push(DrawInstance::new(handles[*s], flip * *m));
    }
    frame.env.shadow_distance = 80.0;
    let suffix = night_env.as_ref().map(|(n, _)| format!("_{n}")).unwrap_or_default();
    if let Some((_, env)) = night_env {
        frame.env = env;
        frame.local_lights = lamps.iter().map(|l| Light { position: flip.transform_point3(l.position), spot: l.spot.map(|s| Spot { direction: flip.transform_vector3(s.direction), ..s }), ..*l }).collect();
        frame.local_lights.extend(night::city_lights(&out.lights.iter().map(|(p, c)| (flip.transform_point3(*p), *c)).collect::<Vec<_>>()));
        println!("night: {} lights", frame.local_lights.len());
    }
    let mut shots: Vec<(String, Vec3, Vec3)> = vec![("wide".into(), Vec3::new(-30.0, 1.7, 22.0), Vec3::new(10.0, 9.0, 0.0))];
    if !suffix.is_empty() {
        // Low along the road, like a photo of a wet street at night.
        shots.push(("along".into(), Vec3::new(x0 + 45.0, 1.4, 8.5), Vec3::new(x0 + 130.0, 3.0, 4.0)));
    }
    for (id, cx, h) in centers.iter().step_by(2) {
        shots.push((id.clone(), Vec3::new(cx + 6.0, 1.7, 13.0), Vec3::new(*cx + 2.0, h * 0.55, 0.0)));
    }
    let legacy = suffix == "_before";
    let points: Vec<(Vec3, Vec3)> = frame.local_lights.iter().map(|l| (l.position, l.color)).collect();
    let only = std::env::var("STREET_SHOTS").ok();
    for (name, eye, at) in shots {
        if only.as_deref().is_some_and(|o| !o.split(',').any(|n| n.trim() == name)) {
            continue;
        }
        if legacy {
            frame.local_lights.clear();
            frame.lights = night::legacy_lights(&points, flip.transform_point3(eye));
        }
        frame.camera = Camera::look_at(flip.transform_point3(eye), flip.transform_point3(at));
        frame.camera.fov_y = 62f32.to_radians();
        let img = r.render_to_image(&frame, 1280, 720);
        let path = format!("{prefix}_{name}{suffix}.png");
        img.save(&path).unwrap();
        println!("{path}: {:?}", r.stats);
    }
}
