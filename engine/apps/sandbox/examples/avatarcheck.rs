//! Close-up review renders of Rocketbox (or any FBX/glTF) characters.
//!
//! ```text
//! cargo run --release -p sandbox --example avatarcheck -- out.png <file.fbx>.. [--views front,side,top,back] [--size 480]
//! ```
//! Env: `RAW=1` skips `make_photoreal`; `HAIR=none|core|soft|red` isolates
//! the hair-card passes.

use std::path::PathBuf;

use forja_anim::Skeleton;
use forja_assets::AlphaMode;
use forja_math::{Mat4, Vec3, Vec4};
use forja_render::*;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let out = args.first().cloned().expect("out.png");
    let mut files = Vec::new();
    let mut views = vec!["front".to_string(), "side".into(), "top".into()];
    let mut size = 480u32;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--views" => {
                views = args[i + 1].split(',').map(str::to_string).collect();
                i += 1;
            }
            "--size" => {
                size = args[i + 1].parse().expect("size");
                i += 1;
            }
            f => files.push(PathBuf::from(f)),
        }
        i += 1;
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/downloaded");
    let mut r = Renderer::new(&create_instance(), None).expect("GPU");
    if let Ok(img) = forja_assets::load_hdr(&root.join("hdri_wide_street/wide_street_01_2k.hdr")) {
        r.set_environment(Some(&env::EnvMap::from_hdr(&img, 14.0)));
    }
    let mut sheet = image::RgbaImage::new(size * views.len() as u32, size * files.len() as u32);
    for (row, f) in files.iter().enumerate() {
        let path = if f.exists() { f.clone() } else { root.join(f) };
        let mut scene = forja_assets::load_model(&path).expect("model");
        if std::env::var("RAW").is_err() {
            forja_assets::character::make_photoreal(&mut scene);
        }
        if let Ok(mode) = std::env::var("HAIR") {
            let hair: Vec<AlphaMode> = scene.materials.iter().map(|m| m.alpha).collect();
            for prims in &mut scene.meshes {
                prims.retain(|p| match (mode.as_str(), hair[p.material]) {
                    (_, AlphaMode::Opaque) => true,
                    ("core", AlphaMode::Mask(_)) | ("soft" | "fringe", AlphaMode::Blend) | ("red", _) => true,
                    _ => false,
                });
            }
            if mode == "fringe" {
                let k: f32 = std::env::var("FRINGE").ok().and_then(|v| v.parse().ok()).unwrap_or(0.75);
                for m in &mut scene.materials {
                    if m.alpha == AlphaMode::Blend {
                        m.base_color.w *= k;
                    }
                }
            }
            if mode == "red" {
                for m in &mut scene.materials {
                    if m.alpha != AlphaMode::Opaque {
                        m.base_color = Vec4::new(1.0, 0.0, 0.0, 1.0);
                        m.base_color_tex = None;
                    }
                }
            }
        }
        let skel = Skeleton::from_scene(&scene, 0).expect("skin");
        let h = r.upload_scene(&scene);
        let mut m = Vec::new();
        skel.model_matrices(&skel.rest_pose(), &mut m);
        let head = skel.find("Bip01 Head").map(|j| m[j].w_axis.truncate()).unwrap_or(Vec3::new(0.0, 1.65, 0.0));
        let at = head + Vec3::Y * 0.07;
        for (col, v) in views.iter().enumerate() {
            let (eye, look, fov) = match v.as_str() {
                "front" => (at + Vec3::new(0.0, 0.03, 0.62), at, 30.0),
                "side" => (at + Vec3::new(0.5, 0.06, 0.38), at, 30.0),
                "top" => (at + Vec3::new(0.15, 0.45, 0.35), at, 30.0),
                "back" => (at + Vec3::new(-0.2, 0.1, -0.6), at, 30.0),
                "body" => (Vec3::new(0.4, 1.0, 3.2), Vec3::new(0.0, 0.9, 0.0), 38.0),
                other => panic!("unknown view {other}"),
            };
            let mut frame = FrameDesc::default();
            frame.instances.push(DrawInstance::new(h, Mat4::IDENTITY));
            frame.camera = Camera::look_at(eye, look);
            frame.camera.fov_y = (fov as f32).to_radians();
            frame.camera.near = 0.02;
            frame.env.shadow_distance = 5.0;
            let img = r.render_to_image(&frame, size, size);
            image::imageops::overlay(&mut sheet, &img, (col as u32 * size) as i64, (row as u32 * size) as i64);
        }
    }
    sheet.save(&out).expect("save");
    println!("{out}");
}
