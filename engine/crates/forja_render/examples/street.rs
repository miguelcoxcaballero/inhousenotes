//! Street of procedural Spanish buildings, one per style, rendered at eye
//! level: `cargo run --release -p forja_render --example street -- out_prefix`
//! (`NIGHT=1` or `HOUR=<h>` for a night street with candelabras; `DUSK=1`,
//! `RAIN=1`, `WET=1` pick the other presets, `LEGACY=1` the previous
//! 64-light path). At night the street also gets two small neon signs,
//! shop light spilling onto the pavement and a low `along` shot down the
//! road. `STREET_SHOTS=wide,along` renders only those shots;
//! `STREET_CLOSE=1` adds facade close-ups.

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
    let street = out.add_scene(assemble(&kit, "street", p, None));
    out.placements.push((street, Mat4::IDENTITY));
    let mut extra: Vec<Light> = Vec::new();
    if night {
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
    if night && !legacy {
        frame.env = mode.as_ref().map(|(_, e)| e.clone()).unwrap_or_else(Environment::night);
        frame.local_lights = out.city_lights.iter().map(night::from_city).chain(if legacy { Vec::new() } else { extra.clone() }).map(|l| night::transformed(&l, &flip)).collect();
        println!("night at {:.1} h: {} lights", kit.night.hour, frame.local_lights.len());
    } else if night {
        frame.env.use_ibl = false;
        frame.env.camera = CameraModel::Night;
        frame.env.sun_dir = Vec3::new(-0.35, 0.72, -0.55).normalize();
        frame.env.sun_illuminance = 0.025;
        frame.env.set_linear_exposure(0.55 + 3.0 * kit.night.dark);
        frame.env.bloom = 0.08;
        println!("night at {:.1} h: {} city lights", kit.night.hour, out.city_lights.len());
    }
    let mut shots: Vec<(String, Vec3, Vec3)> = vec![("wide".into(), Vec3::new(-30.0, 1.7, 22.0), Vec3::new(10.0, 9.0, 0.0))];
    for (id, cx, h) in centers.iter().step_by(2) {
        shots.push((id.clone(), Vec3::new(cx + 6.0, 1.7, 13.0), Vec3::new(*cx + 2.0, h * 0.55, 0.0)));
        // STREET_CLOSE=1: material close-ups, 4 m from the facade and
        // grazing along it (where a plastic sheen shows first).
        if std::env::var("STREET_CLOSE").is_ok() {
            shots.push((format!("{id}_close"), Vec3::new(cx + 4.0, 1.7, 4.0), Vec3::new(*cx - 1.0, 4.5, 0.0)));
            shots.push((format!("{id}_graze"), Vec3::new(cx + 9.0, 1.8, 1.6), Vec3::new(*cx - 5.0, 6.5, 0.0)));
        }
    }
    if night {
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
