//! Runs the shipped controller definitions against the downloaded packs.

use std::path::PathBuf;

use forja_anim::{AnimController, Clip, ControllerDef, Skeleton};

fn engine() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn run(model: &str, def: &str) {
    let path = engine().join("assets/downloaded").join(model);
    if !path.exists() {
        eprintln!("{model} not downloaded; skipping");
        return;
    }
    let scene = forja_assets::load_gltf(&path).unwrap();
    let skel = Skeleton::from_scene_any(&scene);
    let clips = Clip::bind_all(&scene, &skel, None);
    let def = ControllerDef::from_toml(&std::fs::read_to_string(engine().join("data/anim").join(def)).unwrap()).unwrap();
    let mut c = AnimController::new(&def, &skel, clips).unwrap_or_else(|e| panic!("{model}: {e}"));
    let mut mats = Vec::new();
    let mut last = None;
    let mut changed = 0;
    for frame in 0..240 {
        c.set("speed", (frame as f32 / 40.0).min(6.0));
        c.set("aiming", if frame > 120 { 1.0 } else { 0.0 });
        c.update(1.0 / 60.0);
        skel.skinning_matrices(c.pose(), &mut mats);
        assert!(mats.iter().all(|m| m.is_finite()), "{model}: non-finite matrix at frame {frame}");
        if last.as_ref().is_some_and(|l: &Vec<forja_math::Mat4>| l != &mats) {
            changed += 1;
        }
        last = Some(mats.clone());
    }
    assert!(changed > 200, "{model}: pose barely changes ({changed} frames)");
    c.set("dead", 1.0);
    c.update(0.016);
    assert_eq!(c.state(), "dead");
}

#[test]
fn blocky_controller_runs() {
    run("kenney_blocky_characters/character-a.glb", "blocky_humanoid.toml");
}

#[test]
fn robot_controller_runs() {
    run("quaternius_robot_expressive/RobotExpressive.glb", "robot_humanoid.toml");
}

#[test]
fn every_blocky_character_shares_the_rig() {
    let dir = engine().join("assets/downloaded/kenney_blocky_characters");
    if !dir.exists() {
        return;
    }
    let def = ControllerDef::from_toml(&std::fs::read_to_string(engine().join("data/anim/blocky_humanoid.toml")).unwrap()).unwrap();
    // Clips from character-a drive every other character: modular animation sets.
    let source = forja_assets::load_gltf(&dir.join("character-a.glb")).unwrap();
    for c in b'b'..=b'r' {
        let target = forja_assets::load_gltf(&dir.join(format!("character-{}.glb", c as char))).unwrap();
        let skel = Skeleton::from_scene_any(&target);
        let clips = Clip::bind_all(&source, &skel, None);
        assert!(AnimController::new(&def, &skel, clips).is_ok(), "character-{}", c as char);
    }
}
