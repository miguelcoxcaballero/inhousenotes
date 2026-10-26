//! For each primitive of a model with an alpha texture: share of triangle
//! centroids whose UV lands on opaque texels, with V as loaded and flipped.
fn main() {
    let path = std::env::args().nth(1).expect("model");
    let scene = forja_assets::load_model(std::path::Path::new(&path)).expect("load");
    for prims in &scene.meshes {
        for p in prims {
            let m = &scene.materials[p.material];
            let Some(t) = m.base_color_tex.map(|i| &scene.textures[i]) else { continue };
            let alpha = |u: f32, v: f32| {
                let x = ((u.rem_euclid(1.0)) * t.width as f32) as u32 % t.width;
                let y = ((v.rem_euclid(1.0)) * t.height as f32) as u32 % t.height;
                t.rgba8[((y * t.width + x) * 4 + 3) as usize]
            };
            let (mut a, mut b, mut n) = (0, 0, 0);
            let (mut lo, mut hi) = (forja_math::Vec2::splat(9.0), forja_math::Vec2::splat(-9.0));
            for tri in p.indices.chunks_exact(3) {
                let c = (p.uvs[tri[0] as usize] + p.uvs[tri[1] as usize] + p.uvs[tri[2] as usize]) / 3.0;
                lo = lo.min(c);
                hi = hi.max(c);
                n += 1;
                if alpha(c.x, c.y) > 128 { a += 1; }
                if alpha(c.x, 1.0 - c.y) > 128 { b += 1; }
            }
            println!("{:24} tris {:6} uv [{:.2},{:.2}]..[{:.2},{:.2}] opaque as-is {:.0}% flipped {:.0}%", m.name, n, lo.x, lo.y, hi.x, hi.y, 100.0 * a as f32 / n as f32, 100.0 * b as f32 / n as f32);
        }
    }
}
