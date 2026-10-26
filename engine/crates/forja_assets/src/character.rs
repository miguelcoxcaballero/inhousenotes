//! Material set-up for realistic humans, whatever file they came from:
//! subsurface skin, glossy eyeballs split from the head, and two-pass hair
//! (alpha-tested core that writes depth plus alpha-blended soft fringes).

use crate::{AlphaMode, MaterialData, MeshData, SceneData};

/// Joint names that drive eyeballs (not lids or brows).
fn is_eye_joint(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    (n.ends_with("eye") || n.contains("eye.") || n.contains("eye_") || n.ends_with(" leye") || n.ends_with(" reye") || n.contains("eyeball"))
        && !n.contains("blink")
        && !n.contains("brow")
        && !n.contains("lid")
        && !n.contains("lash")
}

pub fn make_photoreal(scene: &mut SceneData) {
    // Skin.
    for m in &mut scene.materials {
        let n = m.name.to_ascii_lowercase();
        if n.contains("head") || n.contains("face") || n.contains("skin") {
            m.subsurface = 1.0;
        } else if n.contains("body") {
            m.subsurface = 0.55;
        }
    }
    // Eyes and hair, per primitive.
    let skins: Vec<Option<usize>> = (0..scene.meshes.len()).map(|mi| scene.nodes.iter().find(|n| n.mesh == Some(mi)).and_then(|n| n.skin)).collect();
    for mi in 0..scene.meshes.len() {
        let mut extra: Vec<MeshData> = Vec::new();
        let eye_joints: Vec<u16> = skins[mi]
            .and_then(|s| scene.skins.get(s))
            .map(|s| s.joints.iter().enumerate().filter(|(_, n)| is_eye_joint(&scene.nodes[**n].name)).map(|(j, _)| j as u16).collect())
            .unwrap_or_default();
        let prims = std::mem::take(&mut scene.meshes[mi]);
        let mut kept = Vec::with_capacity(prims.len());
        for mut p in prims {
            let mat = scene.materials[p.material].clone();
            // Eyeballs: triangles whose vertices all follow an eye joint.
            if !eye_joints.is_empty() && p.is_skinned() && mat.subsurface > 0.9 {
                let eye_vertex: Vec<bool> = p
                    .joints
                    .iter()
                    .zip(&p.weights)
                    .map(|(j, w)| j.iter().zip(w).any(|(j, w)| *w > 0.5 && eye_joints.contains(j)))
                    .collect();
                let (mut eye_idx, mut rest_idx) = (Vec::new(), Vec::new());
                for t in p.indices.chunks_exact(3) {
                    if t.iter().all(|i| eye_vertex[*i as usize]) {
                        eye_idx.extend_from_slice(t);
                    } else {
                        rest_idx.extend_from_slice(t);
                    }
                }
                if !eye_idx.is_empty() {
                    let eye_mat = MaterialData {
                        name: format!("{}_eye", mat.name),
                        roughness: 0.04,
                        metallic_roughness_tex: None,
                        occlusion_tex: None,
                        normal_tex: None,
                        subsurface: 0.0,
                        ..mat.clone()
                    };
                    scene.materials.push(eye_mat);
                    let mut eye = MeshData { indices: eye_idx, lods: Vec::new(), material: scene.materials.len() - 1, ..p.clone() };
                    eye.finalize();
                    extra.push(eye);
                    p.indices = rest_idx;
                    p.lods.clear();
                    p.finalize();
                }
            }
            // Hair cards: an alpha-tested pass for the dense core (writes
            // depth, casts shadows) and a blended pass for the soft ends.
            if let AlphaMode::Mask(_) = mat.alpha {
                // Strands shadow each other: cards lit like solid planes
                // read as pale tiles, so darken them (self-occlusion) and
                // keep the sheen broad.
                let mat = MaterialData { base_color: mat.base_color * forja_math::Vec4::new(0.5, 0.48, 0.46, 1.0), roughness: 0.8, metallic_roughness_tex: None, normal_tex: None, ..mat.clone() };
                let core = MaterialData { alpha: AlphaMode::Mask(200), ..mat.clone() };
                scene.materials[p.material] = core;
                let soft = MaterialData { name: format!("{}_soft", mat.name), alpha: AlphaMode::Blend, ..mat.clone() };
                scene.materials.push(soft);
                extra.push(MeshData { material: scene.materials.len() - 1, ..p.clone() });
            }
            kept.push(p);
        }
        kept.extend(extra);
        scene.meshes[mi] = kept;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eye_joint_names() {
        assert!(is_eye_joint("Bip01 LEye"));
        assert!(is_eye_joint("eye.L"));
        assert!(!is_eye_joint("Bip01 LEyeBlinkTop"));
        assert!(!is_eye_joint("Bip01 LInnerEyebrow"));
    }
}
