//! Far stand-ins of landmarks for open worlds.
//!
//! [`super::assemble`] gives a landmark distance LODs that keep every
//! opening, balcony and moulding (they only thin out smooth surfaces), so a
//! cathedral is still hundreds of thousands of triangles a kilometre away.
//! A world draws [`far_scene`] instead beyond its detail distance: the same
//! masses and materials within about a pixel at that distance, in a few
//! thousand triangles.

use forja_assets::{MaterialData, MeshData, NodeData, SceneData};
use forja_math::{Aabb, Vec3};

/// Geometric error (m) the stand-in may have: a pixel at ~600 m.
pub const FAR_ERROR: f32 = 0.6;

fn adapter(positions: &[Vec3]) -> Option<meshopt::VertexDataAdapter<'_>> {
    let bytes: &[u8] = unsafe { std::slice::from_raw_parts(positions.as_ptr().cast::<u8>(), std::mem::size_of_val(positions)) };
    meshopt::VertexDataAdapter::new(bytes, 12, 0).ok()
}

/// Adds coarser distance LODs to every large primitive of `scene`, after
/// the ones it has: each level about half the triangles of the one before,
/// within a growing error (4 cm, 8 cm, ... 1.5 m). The renderer picks a
/// level by the triangles the primitive's size on screen deserves, so a
/// level is only drawn where its error is below a pixel or two.
pub fn extend_lods(scene: &mut SceneData) {
    for m in scene.meshes.iter_mut().flatten() {
        extend_mesh_lods(m);
    }
}

fn extend_mesh_lods(m: &mut MeshData) {
    let extent = (m.aabb.max - m.aabb.min).max_element().max(1e-3);
    let mut prev: Vec<u32> = m.lods.last().cloned().unwrap_or_else(|| m.indices.clone());
    let mut error = 0.04f32;
    let mut levels = Vec::new();
    {
        let Some(ad) = adapter(&m.positions) else { return };
        while prev.len() / 3 > 300 && m.lods.len() + levels.len() < 12 {
            let target = (prev.len() / 6).max(150) * 3;
            // Edge collapse (clean silhouettes); where hard edges and UV
            // seams stop it, vertex clustering.
            let mut next = meshopt::simplify(&prev, &ad, target, error, meshopt::SimplifyOptions::ErrorAbsolute, None);
            if next.len() < 3 || next.len() > prev.len() * 3 / 4 {
                let s = meshopt::simplify_sloppy(&prev, &ad, target, (error * 2.0 / extent).min(0.3), None);
                if s.len() >= 3 && (next.len() < 3 || s.len() < next.len()) {
                    next = s;
                }
            }
            let last = error >= 1.5;
            error = (error * 2.0).min(1.5);
            if next.len() < 3 || next.len() > prev.len() * 17 / 20 {
                if last {
                    break;
                }
                continue;
            }
            let next = meshopt::optimize_vertex_cache(&next, m.positions.len());
            levels.push(next.clone());
            prev = next;
        }
    }
    m.lods.extend(levels);
}

/// Simplifies one primitive to about `target` triangles.
fn far_prim(m: &MeshData, target: usize, error: f32) -> Option<MeshData> {
    // Start from the coarsest level the landmark already has.
    let src: &[u32] = m.lods.last().map_or(&m.indices, |l| l.as_slice());
    if src.len() < 3 || m.positions.is_empty() {
        return None;
    }
    let extent = (m.aabb.max - m.aabb.min).max_element().max(1e-3);
    let mut idx: Vec<u32> = src.to_vec();
    if idx.len() / 3 > target {
        let ad = adapter(&m.positions)?;
        let a = meshopt::simplify(&idx, &ad, target * 3, error, meshopt::SimplifyOptions::ErrorAbsolute, None);
        if a.len() >= 3 {
            idx = a;
        }
        if idx.len() / 3 > target * 3 / 2 {
            let b = meshopt::simplify_sloppy(&idx, &ad, target * 3, (error * 4.0 / extent).min(0.3), None);
            if b.len() >= 3 {
                idx = b;
            }
        }
    }
    if idx.len() < 3 {
        return None;
    }
    // Only the vertices still in use.
    let mut remap = vec![u32::MAX; m.positions.len()];
    let mut out = MeshData { material: m.material, shadow: m.shadow, ..Default::default() };
    let has_t = m.tangents.len() == m.positions.len();
    for i in &mut idx {
        let k = *i as usize;
        if remap[k] == u32::MAX {
            remap[k] = out.positions.len() as u32;
            out.positions.push(m.positions[k]);
            out.normals.push(m.normals[k]);
            out.uvs.push(m.uvs[k]);
            if has_t {
                out.tangents.push(m.tangents[k]);
            }
        }
        *i = remap[k];
    }
    out.indices = idx;
    out.aabb = Aabb::from_points(out.positions.iter().copied());
    Some(out)
}

/// The far stand-in of an assembled landmark scene: every material's mesh
/// simplified to a share of `budget` triangles (parts smaller than the
/// error are dropped), with the colour maps only.
pub fn far_scene(scene: &SceneData, budget: usize) -> SceneData {
    let mut out = SceneData { name: format!("{}_far", scene.name), ..Default::default() };
    let prims: Vec<&MeshData> = scene.meshes.iter().flatten().collect();
    let total: usize = prims.iter().map(|m| m.indices.len() / 3).sum();
    let ratio = (budget as f32 / total.max(1) as f32).min(1.0);
    let mut used: Vec<Option<usize>> = vec![None; scene.textures.len()];
    let mut meshes = Vec::new();
    for m in prims {
        let size = m.aabb.max - m.aabb.min;
        // Railings, finials, lamps: nothing to see from afar.
        if size.max_element() < FAR_ERROR * 3.0 {
            continue;
        }
        let tris = m.indices.len() / 3;
        let target = ((tris as f32 * ratio) as usize).max(8).min(tris);
        let Some(mut f) = far_prim(m, target, FAR_ERROR) else { continue };
        let Some(src) = scene.materials.get(m.material) else { continue };
        let mut mat: MaterialData = src.clone();
        if mat.metallic_roughness_tex.take().is_some() {
            // The map carried the values: plain masonry without it.
            mat.metallic = 0.0;
            mat.roughness = 0.85;
        }
        mat.normal_tex = None;
        mat.occlusion_tex = None;
        for t in [&mut mat.base_color_tex, &mut mat.emissive_tex] {
            if let Some(i) = *t {
                *t = match scene.textures.get(i) {
                    Some(tex) => Some(*used[i].get_or_insert_with(|| {
                        out.textures.push(tex.clone());
                        out.textures.len() - 1
                    })),
                    None => None,
                };
            }
        }
        f.material = out.materials.len();
        out.materials.push(mat);
        meshes.push(f);
    }
    if !meshes.is_empty() {
        out.meshes.push(meshes);
        out.nodes.push(NodeData { name: out.name.clone(), mesh: Some(0), ..Default::default() });
        out.roots.push(0);
    }
    out
}

/// The framework's own source, for cache keys of built landmarks: any
/// change to how a spec becomes a mesh invalidates them.
pub const SOURCES: &[&str] = &[
    include_str!("build.rs"),
    include_str!("details.rs"),
    include_str!("domes.rs"),
    include_str!("earcut.rs"),
    include_str!("facade.rs"),
    include_str!("far.rs"),
    include_str!("footprint.rs"),
    include_str!("generic.rs"),
    include_str!("masses.rs"),
    include_str!("materials.rs"),
    include_str!("mesh.rs"),
    include_str!("mod.rs"),
    include_str!("openings.rs"),
    include_str!("palette.rs"),
    include_str!("parse.rs"),
    include_str!("poly.rs"),
    include_str!("profiles.rs"),
    include_str!("resolve.rs"),
    include_str!("roofs.rs"),
    include_str!("runs.rs"),
    include_str!("scaffold.rs"),
    include_str!("shapes.rs"),
    include_str!("skeleton.rs"),
    include_str!("spec.rs"),
    include_str!("supports.rs"),
];
