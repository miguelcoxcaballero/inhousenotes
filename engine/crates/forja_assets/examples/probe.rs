//! Prints where raised geometry (curbs) sits: `cargo run -p forja_assets --example probe -- file.glb`
fn main() {
    for p in std::env::args().skip(1) {
        let s = forja_assets::load_model(std::path::Path::new(&p)).unwrap();
        let w = s.world_matrices();
        let mut raised = forja_math::Aabb::EMPTY;
        for (i, n) in s.nodes.iter().enumerate() {
            let Some(m) = n.mesh else { continue };
            for prim in &s.meshes[m] {
                for v in &prim.positions {
                    let v = w[i].transform_point3(*v);
                    if v.y > 0.005 {
                        raised.grow_point(v);
                    }
                }
            }
        }
        println!("{p}: raised {:?}..{:?} materials {:?}", raised.min, raised.max, s.materials.iter().map(|m| (&m.name, m.base_color_tex.is_some())).collect::<Vec<_>>());
    }
}
