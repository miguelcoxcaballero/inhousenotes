//! Prints the skeleton, materials and mesh stats of a character file.
//! `cargo run --release -p sandbox --example avatar_probe -- <file.fbx>`

use forja_anim::Skeleton;

fn main() {
    let path = std::env::args().nth(1).expect("file");
    let t = std::time::Instant::now();
    let mut scene = forja_assets::load_model(std::path::Path::new(&path)).unwrap();
    if std::env::var("PHOTO").is_ok() {
        forja_assets::character::make_photoreal(&mut scene);
    }
    println!("loaded in {:.2?}", t.elapsed());
    for (i, m) in scene.materials.iter().enumerate() {
        let tn = |t: Option<usize>| t.map(|i| scene.textures[i].name.clone()).unwrap_or_default();
        println!("mat {i} {:?} base {:?} normal {:?} orm {:?} alpha {:?} color {:?} occ {:?}", m.name, tn(m.base_color_tex), tn(m.normal_tex), tn(m.metallic_roughness_tex), m.alpha, m.base_color, tn(m.occlusion_tex));
    }
    for (mi, prims) in scene.meshes.iter().enumerate() {
        for p in prims {
            println!("mesh {mi} mat {} verts {} tris {} aabb {:?}", p.material, p.positions.len(), p.triangle_count(), p.aabb);
        }
    }
    // Hair cards vs. head surface.
    if std::env::var("HAIR").is_ok() {
        for prims in &scene.meshes {
            let head: Vec<[forja_math::Vec3; 3]> = prims.iter().filter(|p| scene.materials[p.material].name.contains("head")).flat_map(|p| p.indices.chunks_exact(3).map(|t| [p.positions[t[0] as usize], p.positions[t[1] as usize], p.positions[t[2] as usize]]).collect::<Vec<_>>()).collect();
            for p in prims.iter().filter(|p| scene.materials[p.material].alpha != forja_assets::AlphaMode::Opaque) {
                let mut d: Vec<f32> = p.positions.iter().map(|v| head.iter().map(|t| point_tri(*v, t)).fold(f32::MAX, f32::min)).collect();
                d.sort_by(f32::total_cmp);
                let q = |f: f32| d[((d.len() - 1) as f32 * f) as usize];
                println!("HAIR {} verts {} dist p50 {:.4} p90 {:.4} max {:.4}", path, d.len(), q(0.5), q(0.9), q(1.0));
            }
        }
        return;
    }
    let skel = Skeleton::from_scene(&scene, 0).unwrap();
    let mut m = Vec::new();
    skel.model_matrices(&skel.rest_pose(), &mut m);
    for (i, j) in skel.joints.iter().enumerate() {
        let p = m[i].w_axis.truncate();
        let r = forja_math::Quat::from_mat4(&m[i]).normalize();
        println!("{i:3} {:32} parent {:?} pos ({:+.3},{:+.3},{:+.3}) x {:?} y {:?} z {:?}", j.name, j.parent, p.x, p.y, p.z, (r * forja_math::Vec3::X).to_array().map(|v| (v * 100.0).round() / 100.0), (r * forja_math::Vec3::Y).to_array().map(|v| (v * 100.0).round() / 100.0), (r * forja_math::Vec3::Z).to_array().map(|v| (v * 100.0).round() / 100.0));
    }
}

fn point_tri(p: forja_math::Vec3, t: &[forja_math::Vec3; 3]) -> f32 {
    // Distance from p to triangle t (Ericson, closest point).
    let (a, b, c) = (t[0], t[1], t[2]);
    let (ab, ac, ap) = (b - a, c - a, p - a);
    let (d1, d2) = (ab.dot(ap), ac.dot(ap));
    if d1 <= 0.0 && d2 <= 0.0 { return (p - a).length(); }
    let bp = p - b;
    let (d3, d4) = (ab.dot(bp), ac.dot(bp));
    if d3 >= 0.0 && d4 <= d3 { return (p - b).length(); }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 { let v = d1 / (d1 - d3); return (p - (a + ab * v)).length(); }
    let cp = p - c;
    let (d5, d6) = (ab.dot(cp), ac.dot(cp));
    if d6 >= 0.0 && d5 <= d6 { return (p - c).length(); }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 { let w = d2 / (d2 - d6); return (p - (a + ac * w)).length(); }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 { let w = (d4 - d3) / ((d4 - d3) + (d5 - d6)); return (p - (b + (c - b) * w)).length(); }
    let denom = 1.0 / (va + vb + vc);
    let (v, w) = (vb * denom, vc * denom);
    (p - (a + ab * v + ac * w)).length()
}
