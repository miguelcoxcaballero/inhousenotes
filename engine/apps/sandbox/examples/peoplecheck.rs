//! Review sheet of people as the game animates them (same library, motion
//! capture, controller and procedural life), one row per avatar:
//! face looking at the camera, half blink, closed eyes, looking down,
//! three-quarter face, standing idle, mid-stride side view, walking front.
//!
//! ```text
//! cargo run --release -p sandbox --example peoplecheck -- out.png [Male_Adult_08 Female_Adult_01 ..] [--size 300]
//! ```
//! With no names, every downloaded Rocketbox avatar gets a row.
//! `NOLOOK=1` leaves the head to the motion capture (compare the look-at).

use forja_math::{Mat4, Vec3};
use forja_render::*;
use sandbox::game::people::{PeopleLibrary, WALKS};
use sandbox::game::{Paths, RenderScene};

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let out = args.remove(0);
    let mut size = 300u32;
    if let Some(i) = args.iter().position(|a| a == "--size") {
        size = args[i + 1].parse().expect("size");
        args.drain(i..i + 2);
    }
    let root = forja_assets::manifest::find_engine_root().expect("engine root");
    let paths = Paths { data: root.join("data"), downloaded: root.join("assets/downloaded") };
    let files: Vec<_> = PeopleLibrary::catalogue(&paths).into_iter().filter(|f| args.is_empty() || args.iter().any(|a| f.file_stem().is_some_and(|s| s == a.as_str()))).collect();
    let mut lib = PeopleLibrary::default();
    let mut rs = RenderScene::default();
    let kinds = lib.load(&paths, &mut rs, &files, 2048);
    let mut r = Renderer::new(&create_instance(), None).expect("GPU");
    if let Ok(img) = forja_assets::load_hdr(&paths.asset("hdri_wide_street/wide_street_01_2k.hdr")) {
        r.set_environment(Some(&env::EnvMap::from_hdr(&img, 14.0)));
    }
    let handles: Vec<_> = rs.models.iter().map(|m| r.upload_scene(m)).collect();
    let views = 8u32;
    let mut sheet = image::RgbaImage::new(size * views, size * kinds.len() as u32);
    for (row, kind) in kinds.iter().enumerate() {
        let body = Mat4::IDENTITY;
        for col in 0..views {
            let mut p = lib.spawn(&paths, kind, WALKS[0], 3).expect("person");
            // Settle into the pose: standing for faces, walking for gait views.
            let walking = col >= 6;
            let speed = if walking { p.gait.walk } else { 0.0 };
            let head0 = {
                p.update(0.0, 0.0, &body, None);
                p.pose()[0].w_axis.truncate().with_y(p.eye_height())
            };
            let cam_face = head0 + Vec3::new(0.0, 0.02, 0.55);
            let look = match col {
                3 => Some(head0 + Vec3::new(0.0, -0.9, 0.5)),
                0..=4 if std::env::var("NOLOOK").is_err() => Some(cam_face),
                _ => None,
            };
            p.life.lid_override = match col {
                1 => Some(0.5),
                2 => Some(1.0),
                _ => Some(0.0),
            };
            let steps = if walking { 30 + col as usize * 7 } else { 45 };
            for _ in 0..steps {
                p.update(1.0 / 30.0, speed, &body, look);
            }
            let eye_h = p.eye_height();
            let face = Vec3::new(0.0, eye_h - 0.04, 0.0);
            let (eye, at, fov) = match col {
                0..=3 => (face + Vec3::new(0.0, 0.03, 0.55), face, 24.0),
                4 => (face + Vec3::new(0.42, 0.05, 0.38), face, 26.0),
                5 => (Vec3::new(0.6, 1.0, 3.0), Vec3::new(0.0, 0.9, 0.0), 36.0),
                6 => (Vec3::new(3.2, 1.0, 0.0), Vec3::new(0.0, 0.9, 0.0), 36.0),
                _ => (Vec3::new(0.3, 1.1, 3.2), Vec3::new(0.0, 0.9, 0.0), 36.0),
            };
            let item = p.render_item(body);
            let mut frame = FrameDesc::default();
            let mut inst = DrawInstance::new(handles[item.model], item.transform);
            inst.joints = item.joints.clone();
            frame.instances.push(inst);
            frame.camera = Camera::look_at(eye, at);
            frame.camera.fov_y = (fov as f32).to_radians();
            frame.camera.near = 0.02;
            frame.env.shadow_distance = 6.0;
            let img = r.render_to_image(&frame, size, size);
            image::imageops::overlay(&mut sheet, &img, (col * size) as i64, (row as u32 * size) as i64);
        }
    }
    sheet.save(&out).expect("save");
    println!("{out}: {} avatars", kinds.len());
}
