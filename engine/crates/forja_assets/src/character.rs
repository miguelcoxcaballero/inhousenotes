//! Material set-up for realistic humans, whatever file they came from:
//! subsurface skin, glossy eyeballs split from the head, and two-pass hair
//! (alpha-tested core that writes depth plus alpha-blended soft fringes).
//!
//! Hair cards get extra care. Rocketbox avatars (and many game rigs) paint
//! the hair into the head texture and lay thin "shell" cards on the scalp
//! for fuzz. Rendered as is, those cards are much darker than the painted
//! hair under them and sit within a millimetre of the scalp, so the head
//! reads as a dark-and-pale plaid (cards, holes and depth fighting). The
//! cards are therefore tinted to the painted colour beneath them and lifted
//! a few millimetres off the scalp, keeping the strand detail and the soft
//! silhouette without the patchwork.

use forja_math::{Vec2, Vec3};

use crate::{AlphaMode, MaterialData, MeshData, SceneData, TextureData};

/// Joint names that drive eyeballs (not lids or brows).
fn is_eye_joint(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    (n.ends_with("eye") || n.contains("eye.") || n.contains("eye_") || n.ends_with(" leye") || n.ends_with(" reye") || n.contains("eyeball"))
        && !n.contains("blink")
        && !n.contains("brow")
        && !n.contains("lid")
        && !n.contains("lash")
}

/// Hair cards closer than this to the scalp are lifted to it (metres).
const SCALP_LIFT: f32 = 0.0035;

/// How hair cards are rendered.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum HairCards {
    /// [`HairCards::Wisps`] for cards that hug the scalp (painted hair
    /// underneath, as in Rocketbox), [`HairCards::Full`] otherwise.
    #[default]
    Auto,
    /// Alpha-tested core that writes depth plus a blended soft pass, tinted
    /// to the painted hair (hair modelled with free-standing cards).
    Full,
    /// Only the wispy strand ends, tinted to the painted hair; the solid
    /// card interiors fade out so the painted hair shows through. Removes
    /// the dark/pale patchwork of scalp shells while keeping a soft outline.
    Wisps,
    /// No cards: the painted hair alone.
    Painted,
}

/// Cards whose 90th-percentile distance to the scalp is below this are
/// scalp shells (metres).
const SHELL_DISTANCE: f32 = 0.03;

pub fn make_photoreal(scene: &mut SceneData) {
    make_photoreal_with(scene, HairCards::Auto);
}

pub fn make_photoreal_with(scene: &mut SceneData, hair: HairCards) {
    // Skin.
    for m in &mut scene.materials {
        let n = m.name.to_ascii_lowercase();
        if n.contains("head") || n.contains("face") || n.contains("skin") {
            m.subsurface = 1.0;
        } else if n.contains("body") {
            m.subsurface = 0.55;
        }
    }
    let hair_mats: Vec<bool> = scene.materials.iter().map(|m| matches!(m.alpha, AlphaMode::Mask(_))).collect();
    // Painted colour under the cards of each hair material (linear RGB sum,
    // weight) and how far the cards stand off the scalp.
    let mut scalp: Vec<(Vec3, f32)> = vec![(Vec3::ZERO, 0.0); scene.materials.len()];
    let mut standoff: Vec<Vec<f32>> = vec![Vec::new(); scene.materials.len()];
    for prims in &mut scene.meshes {
        let head: Vec<usize> = (0..prims.len()).filter(|&i| scene.materials[prims[i].material].subsurface > 0.9).collect();
        if head.is_empty() {
            continue;
        }
        let tris = HeadSurface::new(prims, &head);
        for p in prims.iter_mut().filter(|p| hair_mats[p.material]) {
            let tex = |m: usize| scene.materials[m].base_color_tex.and_then(|t| scene.textures.get(t));
            for v in 0..p.positions.len() {
                let Some(hit) = tris.closest(p.positions[v]) else { continue };
                standoff[p.material].push((p.positions[v] - hit.point).length());
                // Lift onto (or keep above) the scalp along its normal.
                let above = (p.positions[v] - hit.point).dot(hit.normal);
                if above < SCALP_LIFT {
                    p.positions[v] += hit.normal * (SCALP_LIFT - above);
                }
                if let Some(t) = tex(hit.material) {
                    let c = sample_linear(t, hit.uv);
                    let e = &mut scalp[p.material];
                    e.0 += c;
                    e.1 += 1.0;
                }
            }
            p.finalize_bounds();
        }
    }
    // Hair materials: `core` keeps the card material (alpha tested, writes
    // depth), `soft` is an extra blended pass over the same cards.
    let mut core_of: Vec<Option<usize>> = vec![None; scene.materials.len()];
    let mut soft_of: Vec<Option<usize>> = vec![None; scene.materials.len()];
    for (mi, is_hair) in hair_mats.iter().enumerate() {
        if !is_hair {
            continue;
        }
        let mode = match hair {
            HairCards::Auto => {
                let d = &mut standoff[mi];
                d.sort_by(f32::total_cmp);
                let shell = !d.is_empty() && d[(d.len() - 1) * 9 / 10] < SHELL_DISTANCE;
                if shell { HairCards::Wisps } else { HairCards::Full }
            }
            m => m,
        };
        if mode == HairCards::Painted {
            continue;
        }
        let mat = scene.materials[mi].clone();
        let tex = mat.base_color_tex.and_then(|t| scene.textures.get(t)).cloned();
        let tex = tex.as_ref();
        let tint = match (tex.and_then(mean_opaque_color), scalp[mi]) {
            (Some(hair), (sum, n)) if n > 0.0 => {
                let want = sum / n;
                // Per-channel gain, limited so a bad match cannot recolour wildly.
                (want / hair.max(Vec3::splat(1e-3))).clamp(Vec3::splat(0.35), Vec3::splat(4.0))
            }
            // No painted reference: strands shadow each other, so cards lit
            // like solid planes read as pale tiles; darken them.
            _ => Vec3::new(0.5, 0.48, 0.46),
        };
        // Strand occlusion from the texture: dark texels are deep in the
        // hair volume and see less sky (also damps grazing sky reflections).
        let occlusion = tex.map(|t| {
            scene.textures.push(strand_occlusion(t));
            scene.textures.len() - 1
        });
        let base = MaterialData { roughness: 0.72, metallic_roughness_tex: None, normal_tex: None, occlusion_tex: occlusion, double_sided: true, ..mat.clone() };
        if mode == HairCards::Wisps {
            let wisps = tex.map(|t| {
                scene.textures.push(wisp_texture(t, tint));
                scene.textures.len() - 1
            });
            scene.materials.push(MaterialData { name: format!("{}_wisps", mat.name), alpha: AlphaMode::Blend, base_color_tex: wisps.or(mat.base_color_tex), ..base });
        } else {
            let base = MaterialData { base_color: mat.base_color * tint.extend(1.0), ..base };
            scene.materials[mi] = MaterialData { alpha: AlphaMode::Mask(200), ..base.clone() };
            core_of[mi] = Some(mi);
            scene.materials.push(MaterialData { name: format!("{}_soft", mat.name), alpha: AlphaMode::Blend, ..base });
        }
        soft_of[mi] = Some(scene.materials.len() - 1);
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
                        foliage: None,
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
            // Hair cards: the alpha-tested core (if any) plus a blended copy
            // for the soft strand ends.
            if hair_mats.get(p.material).copied().unwrap_or(false) {
                if let Some(soft) = soft_of[p.material] {
                    extra.push(MeshData { material: soft, ..p.clone() });
                }
                if core_of[p.material].is_none() {
                    continue;
                }
            }
            kept.push(p);
        }
        kept.extend(extra);
        scene.meshes[mi] = kept;
    }
}

impl MeshData {
    fn finalize_bounds(&mut self) {
        self.aabb = forja_math::Aabb::from_points(self.positions.iter().copied());
    }
}

struct SurfaceHit {
    point: Vec3,
    normal: Vec3,
    uv: Vec2,
    material: usize,
}

/// Triangles of the head mesh for closest-point queries.
struct HeadSurface {
    tris: Vec<([Vec3; 3], [Vec2; 3], [Vec3; 3], usize)>,
}

impl HeadSurface {
    fn new(prims: &[MeshData], head: &[usize]) -> Self {
        let mut tris = Vec::new();
        for &h in head {
            let p = &prims[h];
            for t in p.indices.chunks_exact(3) {
                let i = [t[0] as usize, t[1] as usize, t[2] as usize];
                tris.push((i.map(|k| p.positions[k]), i.map(|k| p.uvs.get(k).copied().unwrap_or_default()), i.map(|k| p.normals.get(k).copied().unwrap_or(Vec3::Y)), p.material));
            }
        }
        Self { tris }
    }

    fn closest(&self, p: Vec3) -> Option<SurfaceHit> {
        let mut best: Option<(f32, usize, [f32; 3])> = None;
        for (k, (v, _, _, _)) in self.tris.iter().enumerate() {
            let (q, bary) = closest_on_triangle(p, v[0], v[1], v[2]);
            let d = (p - q).length_squared();
            if best.is_none_or(|b| d < b.0) {
                best = Some((d, k, bary));
            }
        }
        let (_, k, b) = best?;
        let (v, uv, n, material) = &self.tris[k];
        let point = v[0] * b[0] + v[1] * b[1] + v[2] * b[2];
        let mut normal = (n[0] * b[0] + n[1] * b[1] + n[2] * b[2]).normalize_or_zero();
        if normal == Vec3::ZERO {
            normal = (v[1] - v[0]).cross(v[2] - v[0]).normalize_or(Vec3::Y);
        }
        Some(SurfaceHit { point, normal, uv: uv[0] * b[0] + uv[1] * b[1] + uv[2] * b[2], material: *material })
    }
}

/// Closest point on triangle `abc` to `p` and its barycentric weights
/// (Ericson, Real-Time Collision Detection 5.1.5).
fn closest_on_triangle(p: Vec3, a: Vec3, b: Vec3, c: Vec3) -> (Vec3, [f32; 3]) {
    let (ab, ac, ap) = (b - a, c - a, p - a);
    let (d1, d2) = (ab.dot(ap), ac.dot(ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return (a, [1.0, 0.0, 0.0]);
    }
    let bp = p - b;
    let (d3, d4) = (ab.dot(bp), ac.dot(bp));
    if d3 >= 0.0 && d4 <= d3 {
        return (b, [0.0, 1.0, 0.0]);
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        return (a + ab * v, [1.0 - v, v, 0.0]);
    }
    let cp = p - c;
    let (d5, d6) = (ab.dot(cp), ac.dot(cp));
    if d6 >= 0.0 && d5 <= d6 {
        return (c, [0.0, 0.0, 1.0]);
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        return (a + ac * w, [1.0 - w, 0.0, w]);
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return (b + (c - b) * w, [0.0, 1.0 - w, w]);
    }
    let denom = 1.0 / (va + vb + vc);
    let (v, w) = (vb * denom, vc * denom);
    (a + ab * v + ac * w, [1.0 - v - w, v, w])
}

fn srgb_to_linear(c: u8) -> f32 {
    let c = c as f32 / 255.0;
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

/// Linear RGB at `uv` (nearest texel, wrapped; glTF/FBX uv origin top-left
/// after loading).
fn sample_linear(t: &TextureData, uv: Vec2) -> Vec3 {
    let (w, h) = (t.width as i64, t.height as i64);
    if w == 0 || h == 0 {
        return Vec3::ONE;
    }
    let x = ((uv.x * w as f32).floor() as i64).rem_euclid(w);
    let y = ((uv.y * h as f32).floor() as i64).rem_euclid(h);
    let i = ((y * w + x) * 4) as usize;
    let px = &t.rgba8[i..i + 4];
    let lin = |c: u8| if t.srgb { srgb_to_linear(c) } else { c as f32 / 255.0 };
    Vec3::new(lin(px[0]), lin(px[1]), lin(px[2]))
}

/// Mean linear colour of the solid texels (alpha > 50%).
fn mean_opaque_color(t: &TextureData) -> Option<Vec3> {
    let (mut sum, mut n) = (Vec3::ZERO, 0.0f32);
    let step = (t.rgba8.len() / 4 / 65536).max(1);
    for px in t.rgba8.chunks_exact(4).step_by(step) {
        if px[3] > 128 {
            let lin = |c: u8| if t.srgb { srgb_to_linear(c) } else { c as f32 / 255.0 };
            sum += Vec3::new(lin(px[0]), lin(px[1]), lin(px[2]));
            n += 1.0;
        }
    }
    (n > 0.0).then(|| sum / n)
}

/// Hair-card texture with the colour matched to the painted hair (`tint`,
/// linear gain) and only the wispy strand ends kept: texels whose
/// surroundings are solid (a card's interior) fade out.
fn wisp_texture(t: &TextureData, tint: Vec3) -> TextureData {
    let (w, h) = (t.width as usize, t.height as usize);
    // Mean alpha in a window ~0.8% of the texture wide (8 texels at 2K).
    let r = (w.max(h) / 256).max(1);
    let alpha: Vec<f32> = t.rgba8.chunks_exact(4).map(|p| p[3] as f32 / 255.0).collect();
    let mean = box_mean(&alpha, w, h, r);
    let to_srgb = |c: f32| {
        let c = c.clamp(0.0, 1.0);
        let s = if c <= 0.0031308 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 };
        (s * 255.0 + 0.5) as u8
    };
    let lin = |c: u8| if t.srgb { srgb_to_linear(c) } else { c as f32 / 255.0 };
    let mut rgba = Vec::with_capacity(t.rgba8.len());
    for (i, px) in t.rgba8.chunks_exact(4).enumerate() {
        let solid = ((mean[i] - 0.55) / (0.92 - 0.55)).clamp(0.0, 1.0);
        let keep = 1.0 - solid * solid * (3.0 - 2.0 * solid);
        let a = alpha[i] * keep;
        let c = Vec3::new(lin(px[0]), lin(px[1]), lin(px[2])) * tint;
        rgba.extend([to_srgb(c.x), to_srgb(c.y), to_srgb(c.z), (a * 255.0 + 0.5) as u8]);
    }
    TextureData { name: format!("{}_wisps", t.name), width: t.width, height: t.height, rgba8: rgba.into(), srgb: true }
}

/// Box filter of radius `r` (summed-area table, clamped at the borders).
fn box_mean(v: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
    let mut sat = vec![0.0f64; (w + 1) * (h + 1)];
    for y in 0..h {
        let mut row = 0.0f64;
        for x in 0..w {
            row += v[y * w + x] as f64;
            sat[(y + 1) * (w + 1) + x + 1] = sat[y * (w + 1) + x + 1] + row;
        }
    }
    let mut out = vec![0.0f32; w * h];
    for y in 0..h {
        let (y0, y1) = (y.saturating_sub(r), (y + r + 1).min(h));
        for x in 0..w {
            let (x0, x1) = (x.saturating_sub(r), (x + r + 1).min(w));
            let s = sat[y1 * (w + 1) + x1] - sat[y0 * (w + 1) + x1] - sat[y1 * (w + 1) + x0] + sat[y0 * (w + 1) + x0];
            out[y * w + x] = (s / ((y1 - y0) * (x1 - x0)) as f64) as f32;
        }
    }
    out
}

/// Ambient occlusion for hair cards from strand brightness: the brightest
/// strands are on the surface (0.85), the darkest deep inside (0.35).
fn strand_occlusion(t: &TextureData) -> TextureData {
    let lum = |px: &[u8]| 0.3 * px[0] as f32 + 0.59 * px[1] as f32 + 0.11 * px[2] as f32;
    let step = (t.rgba8.len() / 4 / 65536).max(1);
    let max = t.rgba8.chunks_exact(4).step_by(step).filter(|p| p[3] > 128).map(lum).fold(1.0f32, f32::max);
    let mut rgba = Vec::with_capacity(t.rgba8.len());
    for px in t.rgba8.chunks_exact(4) {
        let k = (lum(px) / max).clamp(0.0, 1.0).sqrt();
        let ao = (0.35 + 0.5 * k) * 255.0;
        rgba.extend([ao as u8, ao as u8, ao as u8, 255]);
    }
    TextureData { name: format!("{}_strand_ao", t.name), width: t.width, height: t.height, rgba8: rgba.into(), srgb: false }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NodeData, TextureData};

    #[test]
    fn eye_joint_names() {
        assert!(is_eye_joint("Bip01 LEye"));
        assert!(is_eye_joint("eye.L"));
        assert!(!is_eye_joint("Bip01 LEyeBlinkTop"));
        assert!(!is_eye_joint("Bip01 LInnerEyebrow"));
    }

    #[test]
    fn closest_point_inside_and_on_edges() {
        let (a, b, c) = (Vec3::ZERO, Vec3::X, Vec3::Z);
        let (q, w) = closest_on_triangle(Vec3::new(0.2, 1.0, 0.2), a, b, c);
        assert!((q - Vec3::new(0.2, 0.0, 0.2)).length() < 1e-6);
        assert!((w.iter().sum::<f32>() - 1.0).abs() < 1e-6);
        let (q, _) = closest_on_triangle(Vec3::new(2.0, 0.0, -1.0), a, b, c);
        assert!((q - b).length() < 1e-6);
    }

    fn tex(rgba: [u8; 4], srgb: bool) -> TextureData {
        TextureData { name: "t".into(), width: 2, height: 2, rgba8: rgba.repeat(4).into(), srgb }
    }

    /// A flat "scalp" quad painted light brown with a dark hair card lying
    /// on it.
    fn scalp_scene() -> SceneData {
        let quad = |y: f32, material: usize| {
            let mut m = MeshData {
                positions: vec![Vec3::new(-1.0, y, -1.0), Vec3::new(1.0, y, -1.0), Vec3::new(1.0, y, 1.0), Vec3::new(-1.0, y, 1.0)],
                normals: vec![Vec3::Y; 4],
                uvs: vec![Vec2::ZERO, Vec2::X, Vec2::ONE, Vec2::Y],
                indices: vec![0, 2, 1, 0, 3, 2],
                material,
                ..Default::default()
            };
            m.finalize();
            m
        };
        SceneData {
            nodes: vec![NodeData { mesh: Some(0), ..Default::default() }],
            roots: vec![0],
            meshes: vec![vec![quad(0.0, 0), quad(0.0005, 1)]],
            materials: vec![
                MaterialData { name: "x_head".into(), base_color_tex: Some(0), ..Default::default() },
                MaterialData { name: "x_opacity".into(), base_color_tex: Some(1), alpha: AlphaMode::Mask(110), ..Default::default() },
            ],
            textures: vec![tex([180, 150, 120, 255], true), tex([40, 30, 20, 255], true)],
            ..Default::default()
        }
    }

    #[test]
    fn full_cards_are_lifted_and_tinted() {
        let mut scene = scalp_scene();
        make_photoreal_with(&mut scene, HairCards::Full);
        let card = &scene.meshes[0][1];
        assert!(card.positions.iter().all(|p| (p.y - SCALP_LIFT).abs() < 1e-5), "{:?}", card.positions);
        let m = &scene.materials[1];
        assert_eq!(m.alpha, AlphaMode::Mask(200));
        // Tinted towards the (much lighter) painted scalp.
        assert!(m.base_color.x > 2.0, "{:?}", m.base_color);
        assert!(m.occlusion_tex.is_some());
        // The soft pass exists and uses the blended twin.
        assert_eq!(scene.meshes[0].len(), 3);
        assert_eq!(scene.materials[scene.meshes[0][2].material].alpha, AlphaMode::Blend);
    }

    #[test]
    fn scalp_shells_become_wisps_or_go() {
        // Auto: the card hugs the scalp, so only a blended wisp pass with a
        // pre-tinted texture remains (no alpha-tested core).
        let mut scene = scalp_scene();
        make_photoreal(&mut scene);
        assert_eq!(scene.meshes[0].len(), 2);
        let m = &scene.materials[scene.meshes[0][1].material];
        assert_eq!(m.alpha, AlphaMode::Blend);
        let t = &scene.textures[m.base_color_tex.unwrap()];
        // 40 -> ~82 (sRGB) with the 4x gain limit.
        assert!(t.rgba8[0] > 75, "tint baked into the texture: {:?}", &t.rgba8[..4]);
        // A fully solid card interior fades out completely.
        assert_eq!(t.rgba8[3], 0);
        let mut scene = scalp_scene();
        make_photoreal_with(&mut scene, HairCards::Painted);
        assert_eq!(scene.meshes[0].len(), 1);
    }

    #[test]
    fn box_mean_averages_window() {
        let v = [0.0, 1.0, 0.0, 1.0];
        let m = box_mean(&v, 2, 2, 1);
        assert!(m.iter().all(|x| (x - 0.5).abs() < 1e-6));
    }
}

