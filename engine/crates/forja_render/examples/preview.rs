//! Model viewer: renders a model (or `human`) lit by the street HDRI.
//! `cargo run --release -p forja_render --example preview -- <file|human> out.png [yaw_deg] [dist]`

use std::path::PathBuf;

use forja_math::{Mat4, Quat, Vec3};
use forja_render::*;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let target = args.get(1).expect("model path or 'human'");
    let out = args.get(2).cloned().unwrap_or_else(|| "preview.png".into());
    let yaw: f32 = args.get(3).and_then(|v| v.parse().ok()).unwrap_or(25.0);
    let dist_mul: f32 = args.get(4).and_then(|v| v.parse().ok()).unwrap_or(1.0);
    let dl = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/downloaded");
    let is_human = target == "human" || target.starts_with("human:");
    let mut scene = if is_human {
        let name = target.strip_prefix("human:").unwrap_or("casual_male");
        let spec = forja_assets::makehuman::HumanSpec::presets().into_iter().find(|s| s.name == name).expect("preset");
        forja_assets::makehuman::build_human(&dl.join("makehuman_base"), &dl.join("makehuman_assets"), &spec).unwrap()
    } else {
        forja_assets::load_model(std::path::Path::new(target)).unwrap()
    };
    forja_assets::lod::generate_lods(&mut scene, 6, 20_000);
    let mut r = Renderer::new(&create_instance(), None).unwrap();
    if let Ok(img) = forja_assets::load_hdr(&dl.join("hdri_wide_street/wide_street_01_2k.hdr")) {
        r.set_environment(Some(&env::EnvMap::from_hdr(&img, 14.0)));
    }
    let b = scene.bounds();
    let h = r.upload_scene(&scene);
    // Ground plane.
    let mut ground = forja_assets::SceneData::default();
    {
        let mut m = forja_assets::MeshData {
            positions: vec![Vec3::new(-50.0, 0.0, -50.0), Vec3::new(50.0, 0.0, -50.0), Vec3::new(50.0, 0.0, 50.0), Vec3::new(-50.0, 0.0, 50.0)],
            indices: vec![0, 2, 1, 0, 3, 2],
            ..Default::default()
        };
        m.finalize();
        ground.meshes.push(vec![m]);
        ground.materials.push(forja_assets::MaterialData { base_color: forja_math::Vec4::new(0.35, 0.35, 0.33, 1.0), roughness: 0.9, ..Default::default() });
        ground.nodes.push(forja_assets::NodeData { mesh: Some(0), ..Default::default() });
        ground.roots.push(0);
    }
    let g = r.upload_scene(&ground);
    let mut frame = FrameDesc::default();
    frame.instances.push(DrawInstance::new(g, Mat4::from_translation(Vec3::new(0.0, b.min.y, 0.0))));
    let mut inst = DrawInstance::new(h, Mat4::from_quat(Quat::IDENTITY));
    // Optional: pose the human with a retargeted robot clip at a time.
    if let (Some(clip_name), true) = (args.get(5), is_human) {
        let t: f32 = args.get(6).and_then(|v| v.parse().ok()).unwrap_or(0.3);
        let robot = forja_assets::load_gltf(&dl.join("quaternius_robot_expressive/RobotExpressive.glb")).unwrap();
        let src = forja_anim::Skeleton::from_scene(&robot, 0).unwrap();
        let tgt = forja_anim::Skeleton::from_scene(&scene, 0).unwrap();
        let clips = forja_anim::Clip::bind_all(&robot, &src, None);
        let clip = clips.iter().find(|c| c.name == *clip_name).expect("clip");
        let rc = forja_anim::retarget::retarget(clip, &src, &tgt, &forja_anim::retarget::RetargetMap::auto(&src, &tgt), 30.0);
        let mut pose = tgt.rest_pose();
        rc.sample(t, true, &mut pose);
        let mut mats = Vec::new();
        tgt.skinning_matrices(&pose, &mut mats);
        inst.joints = Some(std::sync::Arc::new(mats));
    }
    frame.instances.push(inst);
    let c = b.center();
    let radius = b.half_extents().length() * 2.2 * dist_mul;
    let a = yaw.to_radians();
    let eye = c + Vec3::new(a.sin() * radius, radius * 0.25, a.cos() * radius);
    frame.camera = Camera::look_at(eye, c);
    frame.camera.fov_y = 40f32.to_radians();
    frame.env.shadow_distance = 30.0;
    let img = r.render_to_image(&frame, 900, 900);
    img.save(&out).unwrap();
    println!("{out}: {:?}", r.stats);
}
