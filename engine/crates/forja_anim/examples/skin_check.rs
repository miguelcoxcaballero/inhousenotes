//! Verifies that a model's skeleton reproduces its bind pose: rest
//! skinning matrices must be identity. `skin_check <model>`
fn main() {
    let path = std::env::args().nth(1).expect("model");
    let scene = forja_assets::load_model(std::path::Path::new(&path)).expect("load");
    let skel = forja_anim::Skeleton::from_scene(&scene, 0).expect("skin");
    let mut m = Vec::new();
    skel.skinning_matrices(&skel.rest_pose(), &mut m);
    let mut worst = (0.0f32, 0usize);
    for (i, x) in m.iter().enumerate() {
        let d = (*x - forja_math::Mat4::IDENTITY).to_cols_array().iter().map(|v| v.abs()).fold(0.0, f32::max);
        if d > worst.0 {
            worst = (d, i);
        }
        if d > 1e-3 && i < 200 {
            let j = &skel.joints[i];
            println!("{:>3} {:28} dev {d:.4} parent {:?} rest t {:?} s {:?}", i, j.name, j.parent.map(|p| skel.joints[p].name.clone()), j.rest.t, j.rest.s);
        }
    }
    println!("{} joints, worst deviation {:.5} at {}", m.len(), worst.0, skel.joints[worst.1].name);
    println!("root_parent {:?}", skel.root_parent);
    // Retarget a mocap take and compare bone directions with the source.
    if let Some(bvh) = std::env::args().nth(2) {
        use forja_anim::retarget::{Role, Side, humanoid_roles, RetargetMap, retarget};
        let src_scene = forja_assets::bvh::load_bvh(std::path::Path::new(&bvh), forja_assets::bvh::CMU_SCALE).unwrap();
        let src = forja_anim::Skeleton::from_scene_any(&src_scene);
        let clip = forja_anim::Clip::bind_all(&src_scene, &src, None).remove(0);
        let map = RetargetMap::auto(&src, &skel);
        let rc = retarget(&clip, &src, &skel, &map, 30.0);
        let (rs, rt) = (humanoid_roles(&src), humanoid_roles(&skel));
        for t in [0.0f32, 0.5] {
            let mut sp = src.rest_pose();
            clip.sample(t, false, &mut sp);
            let mut sm = Vec::new();
            src.model_matrices(&sp, &mut sm);
            let mut tp = skel.rest_pose();
            rc.sample(t, false, &mut tp);
            let mut tm = Vec::new();
            skel.model_matrices(&tp, &mut tm);
            // Clavicle elevation: angle of clavicle→upper arm above horizontal.
            let elev = |m: &Vec<forja_math::Mat4>, r: &std::collections::HashMap<Role, usize>| {
                let d = (m[r[&Role::UpperArm(Side::L)]].w_axis - m[r[&Role::Clavicle(Side::L)]].w_axis).truncate().normalize();
                d.y.asin().to_degrees()
            };
            let mut tr = Vec::new();
            skel.model_matrices(&skel.rest_pose(), &mut tr);
            let mut srm = Vec::new();
            src.model_matrices(&src.rest_pose(), &mut srm);
            println!("t={t} clavicle elevation: src rest {:.1} src now {:.1} | tgt rest {:.1} tgt now {:.1}", elev(&srm, &rs), elev(&sm, &rs), elev(&tr, &rt), elev(&tm, &rt));
            let pairs = [(Role::UpperArm(Side::L), Role::LowerArm(Side::L)), (Role::LowerArm(Side::L), Role::Hand(Side::L)), (Role::UpperLeg(Side::R), Role::LowerLeg(Side::R)), (Role::Spine, Role::Chest), (Role::Neck, Role::Head), (Role::Hips, Role::Spine)];
            for (a, b) in pairs {
                let d = |m: &Vec<forja_math::Mat4>, r: &std::collections::HashMap<Role, usize>| (m[r[&b]].w_axis - m[r[&a]].w_axis).truncate().normalize();
                let (ds, dt) = (d(&sm, &rs), d(&tm, &rt));
                println!("t={t} {a:?}->{b:?}: src {ds:.2?} tgt {dt:.2?} angle {:.1}", ds.angle_between(dt).to_degrees());
            }
        }
    }
}
