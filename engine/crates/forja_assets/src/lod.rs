//! Level-of-detail chains with meshoptimizer: each LOD keeps the shared
//! vertex buffer and gets its own index list, so switching LOD is free.

use crate::{MeshData, SceneData};

/// Builds LODs for meshes above `min_triangles`, halving the triangle count
/// per level until the simplifier stops making progress.
pub fn generate_mesh_lods(mesh: &mut MeshData, max_levels: usize, min_triangles: usize) {
    mesh.lods.clear();
    if mesh.triangle_count() < min_triangles || mesh.positions.is_empty() {
        return;
    }
    let bytes: &[u8] = bytemuck_cast(&mesh.positions);
    let Ok(adapter) = meshopt::VertexDataAdapter::new(bytes, 12, 0) else { return };
    mesh.indices = meshopt::optimize_vertex_cache(&mesh.indices, mesh.positions.len());
    // Each level simplifies the previous one (much faster on dense scans).
    let mut prev: Vec<u32> = mesh.indices.clone();
    for level in 1..=max_levels {
        let target = (prev.len() / 2) / 3 * 3;
        if target < 300 {
            break;
        }
        // Error budget grows with distance: 1% of the extent per level.
        let mut err = 0.0;
        let mut lod = meshopt::simplify(&prev, &adapter, target, 0.01 * level as f32, meshopt::SimplifyOptions::None, Some(&mut err));
        if lod.len() as f32 > prev.len() as f32 * 0.85 {
            // Foliage and other non-manifold scans resist edge collapse:
            // fall back to cluster simplification.
            lod = meshopt::simplify_sloppy(&prev, &adapter, target, 0.02 * level as f32, Some(&mut err));
            if lod.len() as f32 > prev.len() as f32 * 0.85 || lod.is_empty() {
                break;
            }
        }
        lod = meshopt::optimize_vertex_cache(&lod, mesh.positions.len());
        mesh.lods.push(lod.clone());
        prev = lod;
    }
}

pub fn generate_lods(scene: &mut SceneData, max_levels: usize, min_triangles: usize) {
    for m in scene.meshes.iter_mut().flatten() {
        generate_mesh_lods(m, max_levels, min_triangles);
    }
}

fn bytemuck_cast(v: &[forja_math::Vec3]) -> &[u8] {
    // SAFETY: Vec3 is three packed f32s (glam's non-SIMD Vec3, 12 bytes, no padding).
    unsafe { std::slice::from_raw_parts(v.as_ptr().cast::<u8>(), std::mem::size_of_val(v)) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forja_math::{Vec2, Vec3};

    #[test]
    fn sphere_gets_smaller_lods() {
        // UV sphere with 64x64 segments.
        let (n, m) = (64, 64);
        let mut mesh = MeshData::default();
        for i in 0..=n {
            for j in 0..=m {
                let (t, p) = (i as f32 / n as f32 * std::f32::consts::PI, j as f32 / m as f32 * std::f32::consts::TAU);
                mesh.positions.push(Vec3::new(t.sin() * p.cos(), t.cos(), t.sin() * p.sin()));
                mesh.uvs.push(Vec2::new(j as f32 / m as f32, i as f32 / n as f32));
            }
        }
        for i in 0..n {
            for j in 0..m {
                let a = (i * (m + 1) + j) as u32;
                let b = a + m as u32 + 1;
                mesh.indices.extend([a, b, a + 1, a + 1, b, b + 1]);
            }
        }
        mesh.finalize();
        let base = mesh.triangle_count();
        generate_mesh_lods(&mut mesh, 4, 1000);
        assert!(mesh.lods.len() >= 2, "lods: {}", mesh.lods.len());
        assert!(mesh.lods[0].len() / 3 < base);
        assert!(mesh.lods.windows(2).all(|w| w[1].len() < w[0].len()));
    }
}
