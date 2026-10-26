//! Loads the real downloaded packs when present (run `forja_fetch fetch --all` first).

use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/downloaded")
}

#[test]
fn every_downloaded_model_loads() {
    let dir = root();
    if !dir.exists() {
        eprintln!("no downloaded assets; skipping");
        return;
    }
    let mut stack = vec![dir];
    let (mut ok, mut tris, mut clips) = (0, 0, 0);
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
            if !["glb", "gltf", "obj"].contains(&ext) {
                continue;
            }
            let s = forja_assets::load_model(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
            assert!(s.triangle_count() > 0, "{} has no triangles", p.display());
            for m in s.meshes.iter().flatten() {
                assert_eq!(m.normals.len(), m.positions.len());
                assert_eq!(m.tangents.len(), m.positions.len());
                assert!(m.indices.iter().all(|i| (*i as usize) < m.positions.len()));
                assert!(m.material < s.materials.len());
            }
            tris += s.triangle_count();
            clips += s.clips.len();
            ok += 1;
        }
    }
    eprintln!("loaded {ok} models, {tris} triangles, {clips} animation clips");
    assert!(ok > 100);
}

#[test]
fn animated_characters_have_skins_and_clips() {
    let robot = root().join("quaternius_robot_expressive/RobotExpressive.glb");
    if !robot.exists() {
        return;
    }
    let s = forja_assets::load_gltf(&robot).unwrap();
    assert!(!s.skins.is_empty());
    let names: Vec<_> = s.clips.iter().map(|c| c.name.as_str()).collect();
    eprintln!("robot clips: {names:?}");
    for want in ["Idle", "Walking", "Running", "Jump"] {
        assert!(names.contains(&want), "missing {want}");
    }
    let blocky = forja_assets::load_gltf(&root().join("kenney_blocky_characters/character-a.glb")).unwrap();
    eprintln!("blocky clips: {:?}", blocky.clips.iter().map(|c| &c.name).collect::<Vec<_>>());
}
