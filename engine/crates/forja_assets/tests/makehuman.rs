use std::path::PathBuf;

#[test]
fn builds_a_dressed_rigged_human() {
    let dl = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/downloaded");
    if !dl.join("makehuman_base/base.obj").exists() || !dl.join("makehuman_assets").exists() {
        return;
    }
    let t = std::time::Instant::now();
    let s = forja_assets::makehuman::build_human(&dl.join("makehuman_base"), &dl.join("makehuman_assets"), &forja_assets::makehuman::HumanSpec::casual_male()).unwrap();
    let h = forja_assets::makehuman::human_height(&s);
    eprintln!("human: {:.2} m, {} tris, {} bones, {} parts, {:?}", h, s.triangle_count(), s.skins[0].joints.len(), s.meshes[0].len(), t.elapsed());
    assert!((1.5..2.0).contains(&h), "height {h}");
    assert_eq!(s.skins[0].joints.len(), 163);
    for m in &s.meshes[0] {
        assert!(m.is_skinned());
        assert!(m.weights.iter().all(|w| (w.iter().sum::<f32>() - 1.0).abs() < 1e-3));
    }
    // Rest pose skinning is the identity: joint world * inverse bind = I.
    let skel_world = s.world_matrices();
    for (k, j) in s.skins[0].joints.iter().enumerate() {
        let m = skel_world[*j] * s.skins[0].inverse_bind[k];
        assert!(m.abs_diff_eq(forja_math::Mat4::IDENTITY, 1e-3), "joint {k}");
    }
}
