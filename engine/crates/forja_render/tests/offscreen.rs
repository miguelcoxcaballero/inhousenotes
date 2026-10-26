//! Renders real scenes offscreen (works on CI with a software Vulkan driver).

use std::path::PathBuf;
use std::sync::Arc;

use forja_math::{Mat4, Quat, Vec3, Vec4};
use forja_render::*;
use forja_world::{TestTrack, WorldSource};

fn engine() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn renderer() -> Option<Renderer> {
    match Renderer::new(&create_instance(), None) {
        Ok(r) => Some(r),
        Err(e) => {
            eprintln!("no GPU ({e}); skipping");
            None
        }
    }
}

fn save(img: &image::RgbaImage, name: &str) {
    let dir = engine().join("screenshots");
    std::fs::create_dir_all(&dir).unwrap();
    img.save(dir.join(name)).unwrap();
}

#[test]
fn test_track_with_car_and_robot() {
    let Some(mut r) = renderer() else { return };
    eprintln!("adapter: {:?}", r.adapter_info);
    let world = TestTrack::default().build().unwrap();
    let mut frame = FrameDesc::default();
    for (_, scene, m) in world.instances() {
        let h = r.upload_scene(scene);
        frame.instances.push(DrawInstance::new(h, m));
    }
    let car_path = engine().join("assets/downloaded/kenney_car_kit/sedan-sports.glb");
    if car_path.exists() {
        let car = forja_assets::load_gltf(&car_path).unwrap();
        let h = r.upload_scene(&car);
        for (i, tint) in [Vec4::ONE, Vec4::new(1.0, 0.4, 0.3, 1.0)].iter().enumerate() {
            let mut inst = DrawInstance::new(h, Mat4::from_scale_rotation_translation(Vec3::splat(1.8), Quat::from_rotation_y(0.6 + i as f32), Vec3::new(3.0 + i as f32 * 5.0, 0.0, 4.0)));
            inst.tint = *tint;
            frame.instances.push(inst);
        }
    }
    let robot_path = engine().join("assets/downloaded/quaternius_robot_expressive/RobotExpressive.glb");
    if robot_path.exists() {
        let scene = forja_assets::load_gltf(&robot_path).unwrap();
        let skel = forja_anim::Skeleton::from_scene(&scene, 0).unwrap();
        let clips = forja_anim::Clip::bind_all(&scene, &skel, None);
        let run = clips.iter().find(|c| c.name == "Running").unwrap();
        let mut pose = skel.rest_pose();
        run.sample(0.3, true, &mut pose);
        let mut mats = Vec::new();
        skel.skinning_matrices(&pose, &mut mats);
        let h = r.upload_scene(&scene);
        let mut inst = DrawInstance::new(h, Mat4::from_scale_rotation_translation(Vec3::splat(0.4), Quat::from_rotation_y(2.4), Vec3::new(-1.0, 0.0, 6.0)));
        inst.joints = Some(Arc::new(mats));
        frame.instances.push(inst);
    }
    frame.lights.push(PointLight { position: Vec3::new(0.0, 2.0, 6.0), radius: 10.0, color: Vec3::new(1.0, 0.6, 0.3), intensity: 20.0 });
    frame.camera = Camera::look_at(Vec3::new(-6.0, 3.0, 14.0), Vec3::new(3.0, 0.8, 4.0));
    let t = std::time::Instant::now();
    let img = r.render_to_image(&frame, 960, 540);
    eprintln!("rendered in {:?}: {:?}", t.elapsed(), r.stats);
    save(&img, "test_track.png");
    // Sky at the top is bluish, ground below is darker, the image has detail.
    let top = img.get_pixel(480, 10).0;
    let bottom = img.get_pixel(480, 530).0;
    assert!(top[2] > top[0], "sky not blue: {top:?}");
    assert!(top != bottom);
    let mut hist = std::collections::HashSet::new();
    for p in img.pixels().step_by(97) {
        hist.insert(p.0);
    }
    assert!(hist.len() > 200, "image looks flat ({} colors)", hist.len());
    assert!(r.stats.draw_calls > 3 && r.stats.shadow_draws > 0);
}

#[test]
fn sunset_is_warm() {
    let Some(mut r) = renderer() else { return };
    let world = TestTrack::default().build().unwrap();
    let mut frame = FrameDesc::default();
    for (_, scene, m) in world.instances() {
        let h = r.upload_scene(scene);
        frame.instances.push(DrawInstance::new(h, m));
    }
    frame.env.sun_dir = Vec3::new(1.0, 0.06, 0.2).normalize();
    frame.env.exposure = 2.5;
    frame.camera = Camera::look_at(Vec3::new(-20.0, 2.0, 0.0), Vec3::new(10.0, 2.0, 2.0));
    let img = r.render_to_image(&frame, 640, 360);
    save(&img, "sunset.png");
    let horizon = img.get_pixel(320, 150).0;
    assert!(horizon[0] > horizon[2], "sunset horizon should be warm: {horizon:?}");
}
