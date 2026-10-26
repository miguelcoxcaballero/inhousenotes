//! Street of procedural Spanish buildings, one per style, rendered at eye
//! level: `cargo run --release -p forja_render --example street -- out_prefix`

use std::path::PathBuf;

use forja_city::building::{self, CityOut, EdgeKind, Lot, ModuleLib};
use forja_city::geom::Parts;
use forja_city::kit::{Kit, Mat, assemble};
use forja_city::style::StyleLibrary;
use forja_math::{Mat4, Vec2, Vec3};
use forja_render::*;

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
    p.get(Mat::Asphalt).polygon(&[Vec2::new(x0, 4.75), Vec2::new(x1, 4.75), Vec2::new(x1, 40.0), Vec2::new(x0, 40.0)], 0.0, true, 5.0);
    let mut k = x0;
    while k < x1 {
        p.get(Mat::Paint([235, 235, 230])).aabox(Vec3::new(k, 0.0, 9.9), Vec3::new(k + 3.5, 0.006, 10.05), 1.0);
        k += 9.0;
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
    let mut shots: Vec<(String, Vec3, Vec3)> = vec![("wide".into(), Vec3::new(-30.0, 1.7, 22.0), Vec3::new(10.0, 9.0, 0.0))];
    for (id, cx, h) in centers.iter().step_by(2) {
        shots.push((id.clone(), Vec3::new(cx + 6.0, 1.7, 13.0), Vec3::new(*cx + 2.0, h * 0.55, 0.0)));
    }
    for (name, eye, at) in shots {
        frame.camera = Camera::look_at(flip.transform_point3(eye), flip.transform_point3(at));
        frame.camera.fov_y = 62f32.to_radians();
        let img = r.render_to_image(&frame, 1280, 720);
        let path = format!("{prefix}_{name}.png");
        img.save(&path).unwrap();
        println!("{path}: {:?}", r.stats);
    }
}
