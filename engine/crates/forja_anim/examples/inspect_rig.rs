//! Prints the joints and clips of a rigged glTF: `cargo run -p forja_anim --example inspect_rig -- file.glb`
fn main() {
    let path = std::env::args().nth(1).expect("usage: inspect_rig <file.glb>");
    let scene = forja_assets::load_gltf(std::path::Path::new(&path)).expect("load");
    let skels: Vec<_> = if scene.skins.is_empty() {
        vec![forja_anim::Skeleton::from_nodes(&scene)]
    } else {
        (0..scene.skins.len()).filter_map(|i| forja_anim::Skeleton::from_scene(&scene, i)).collect()
    };
    for (i, skel) in skels.iter().enumerate() {
        println!("rig {i} ({}): {} joints", if scene.skins.is_empty() { "node hierarchy" } else { "skin" }, skel.len());
        for j in &skel.joints {
            println!("  {} <- {:?}", j.name, j.parent.map(|p| skel.joints[p].name.as_str()));
        }
        for c in forja_anim::Clip::bind_all(&scene, skel, None) {
            println!("  clip {:<28} {:>5.2}s coverage {:>3.0}%", c.name, c.duration, c.coverage * 100.0);
        }
    }
}
