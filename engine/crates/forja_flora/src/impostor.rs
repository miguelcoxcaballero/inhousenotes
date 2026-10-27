//! Far LOD: an 8-quad billboard (4 crossed planes, one view per face) whose
//! views are rendered into the bottom half of the foliage atlas.
//!
//! Views are orthographic, horizontal, 45° apart. They can be baked on the
//! CPU ([`bake_cpu`], a small alpha-tested rasterizer used when no GPU is
//! around, e.g. while a world is built) or by any renderer through
//! [`views`] + [`apply`] (the `treegen` tool uses the engine's own
//! offscreen renderer, see `forja_render::capture`). Back faces of the
//! quads are discarded by the foliage material (`backface_cull_v`), so each
//! face shows only its own view.

use forja_math::{Vec2, Vec3};
use rayon::prelude::*;

use crate::mesh::MeshBuilder;
use crate::species::srgb_to_linear;
use crate::texture::{Canvas, impostor_rect};
use crate::{ALPHA_CUTOFF, Billboard, FOLIAGE_ALBEDO, TreeAsset};

pub const VIEWS: usize = 8;
/// Resolution of one view tile in the atlas.
pub const TILE: usize = 512;

/// Direction from the tree toward the camera of view `k`.
pub fn view_dir(k: usize) -> Vec3 {
    let a = k as f32 * std::f32::consts::TAU / VIEWS as f32;
    Vec3::new(a.cos(), 0.0, a.sin())
}

/// Screen-right vector of a camera looking along `-d`.
pub fn view_right(d: Vec3) -> Vec3 {
    (-d).cross(Vec3::Y).normalize()
}

/// One orthographic view to render.
#[derive(Clone, Copy, Debug)]
pub struct View {
    pub center: Vec3,
    /// From the centre toward the camera.
    pub dir: Vec3,
    pub right: Vec3,
    /// Half size of the square view (m).
    pub half: f32,
}

pub fn views(bb: &Billboard) -> Vec<View> {
    (0..VIEWS).map(|k| View { center: bb.center, dir: view_dir(k), right: view_right(view_dir(k)), half: bb.half }).collect()
}

/// 8 quads: view `k` on a plane facing `view_dir(k)`, nudged toward its
/// camera so opposite faces of a plane do not z-fight.
pub fn billboard_mesh(bb: &Billboard, crown_center: Vec3) -> MeshBuilder {
    let mut mb = MeshBuilder::default();
    let h = bb.half;
    for k in 0..VIEWS {
        let d = view_dir(k);
        let r = view_right(d);
        let rect = impostor_rect(k);
        let c = bb.center + d * 0.01 * h;
        let corners = [(c - r * h - Vec3::Y * h, Vec2::new(rect[0], rect[3])), (c + r * h - Vec3::Y * h, Vec2::new(rect[2], rect[3])), (c + r * h + Vec3::Y * h, Vec2::new(rect[2], rect[1])), (c - r * h + Vec3::Y * h, Vec2::new(rect[0], rect[1]))];
        let ids: Vec<u32> = corners
            .iter()
            .map(|(p, uv)| {
                let n = (d * 0.75 + (*p - crown_center) / h * 0.3 + Vec3::Y * 0.3).normalize();
                mb.vertex(*p, n, *uv, 0.15)
            })
            .collect();
        mb.tri(ids[0], ids[1], ids[2]);
        mb.tri(ids[0], ids[2], ids[3]);
    }
    mb
}

/// Writes rendered views (straight alpha, linear RGB) into the asset's atlas.
pub fn apply(asset: &mut TreeAsset, views: &[Canvas]) {
    let tex = crate::texture::write_impostors(&asset.scene.textures[FOLIAGE_ALBEDO], views);
    asset.scene.textures[FOLIAGE_ALBEDO] = tex;
}

/// Reuses another variant's impostor atlas (same species and season).
pub fn share(asset: &mut TreeAsset, from: &TreeAsset) {
    asset.scene.textures[FOLIAGE_ALBEDO] = from.scene.textures[FOLIAGE_ALBEDO].clone();
}

struct Tex<'a> {
    w: usize,
    h: usize,
    data: &'a [u8],
    lut: &'a [f32; 256],
    wrap: bool,
}

impl Tex<'_> {
    fn sample(&self, uv: Vec2) -> [f32; 4] {
        let (u, v) = if self.wrap { (uv.x.rem_euclid(1.0), uv.y.rem_euclid(1.0)) } else { (uv.x.clamp(0.0, 0.9999), uv.y.clamp(0.0, 0.9999)) };
        let x = ((u * self.w as f32) as usize).min(self.w - 1);
        let y = ((v * self.h as f32) as usize).min(self.h - 1);
        let o = (y * self.w + x) * 4;
        [self.lut[self.data[o] as usize], self.lut[self.data[o + 1] as usize], self.lut[self.data[o + 2] as usize], self.data[o + 3] as f32 / 255.0]
    }
}

/// CPU rasterizer: per view, the albedo (straight alpha) and the crown
/// self-shadowing term (deeper texels darker), at `n`² pixels.
fn raster(asset: &TreeAsset, n: usize) -> Vec<(Canvas, Vec<f32>)> {
    let scene = asset.lod_scene(0);
    let lut: [f32; 256] = std::array::from_fn(|i| srgb_to_linear(i as f32 / 255.0));
    let center = asset.crown_center;
    let crown_r = asset.crown_radii.max_element().max(0.5);
    let vs = views(&asset.billboard);
    // Two views at a time keeps the peak memory low.
    vs.chunks(2)
        .flat_map(|pair| pair.par_iter().map(|v| {
            let mut col = Canvas::new(n, n);
            let mut shade_px = vec![1.0f32; n * n];
            let mut depth = vec![f32::MIN; n * n];
            for prim in &scene.meshes[0] {
                let mat = &scene.materials[prim.material];
                let Some(ti) = mat.base_color_tex else { continue };
                let t = &scene.textures[ti];
                let tex = Tex { w: t.width as usize, h: t.height as usize, data: &t.rgba8, lut: &lut, wrap: !mat.double_sided };
                let masked = matches!(mat.alpha, forja_assets::AlphaMode::Mask(_));
                let proj = |p: Vec3| {
                    let q = p - v.center;
                    Vec3::new((q.dot(v.right) / v.half * 0.5 + 0.5) * n as f32, (0.5 - q.y / v.half * 0.5) * n as f32, q.dot(v.dir))
                };
                for tri in prim.indices.chunks_exact(3) {
                    let ids = [tri[0] as usize, tri[1] as usize, tri[2] as usize];
                    let s = ids.map(|i| proj(prim.positions[i]));
                    let uv = ids.map(|i| prim.uvs[i]);
                    let w = ids.map(|i| prim.positions[i]);
                    let area = (s[1].x - s[0].x) * (s[2].y - s[0].y) - (s[2].x - s[0].x) * (s[1].y - s[0].y);
                    if area.abs() < 1e-9 {
                        continue;
                    }
                    let x0 = s.iter().map(|p| p.x).fold(f32::MAX, f32::min).floor().max(0.0) as usize;
                    let x1 = s.iter().map(|p| p.x).fold(f32::MIN, f32::max).ceil().min(n as f32 - 1.0).max(0.0) as usize;
                    let y0 = s.iter().map(|p| p.y).fold(f32::MAX, f32::min).floor().max(0.0) as usize;
                    let y1 = s.iter().map(|p| p.y).fold(f32::MIN, f32::max).ceil().min(n as f32 - 1.0).max(0.0) as usize;
                    for y in y0..=y1 {
                        for x in x0..=x1 {
                            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                            let b0 = ((s[1].x - px) * (s[2].y - py) - (s[2].x - px) * (s[1].y - py)) / area;
                            let b1 = ((s[2].x - px) * (s[0].y - py) - (s[0].x - px) * (s[2].y - py)) / area;
                            let b2 = 1.0 - b0 - b1;
                            if b0 < 0.0 || b1 < 0.0 || b2 < 0.0 {
                                continue;
                            }
                            let z = s[0].z * b0 + s[1].z * b1 + s[2].z * b2;
                            let i = y * n + x;
                            if z <= depth[i] {
                                continue;
                            }
                            let c = tex.sample(uv[0] * b0 + uv[1] * b1 + uv[2] * b2);
                            if masked && c[3] * 255.0 < ALPHA_CUTOFF as f32 {
                                continue;
                            }
                            depth[i] = z;
                            let p = w[0] * b0 + w[1] * b1 + w[2] * b2;
                            let zr = (p - center).dot(v.dir) / crown_r;
                            shade_px[i] = 0.5 + 0.5 * ((zr + 0.9) / 1.6).clamp(0.0, 1.0);
                            col.px[i] = [c[0], c[1], c[2], 1.0];
                        }
                    }
                }
            }
            (col, shade_px)
        }).collect::<Vec<_>>())
        .collect()
}

/// CPU bake of all 8 views (2× supersampled).
pub fn bake_cpu(asset: &mut TreeAsset) {
    let n = TILE * 2;
    let canvases: Vec<Canvas> = raster(asset, n)
        .into_iter()
        .map(|(mut c, shade)| {
            for (p, s) in c.px.iter_mut().zip(&shade) {
                p[0] *= s;
                p[1] *= s;
                p[2] *= s;
            }
            crate::texture::resample(&c, TILE)
        })
        .collect();
    apply(asset, &canvases);
}

/// Applies the crown self-shadowing term to views rendered elsewhere
/// (e.g. unlit GPU captures, which only carry albedo and coverage).
pub fn shade(asset: &TreeAsset, views: &mut [Canvas]) {
    let maps = raster(asset, TILE);
    for (v, (_, s)) in views.iter_mut().zip(maps) {
        if v.w != TILE || v.h != TILE {
            continue;
        }
        for (i, p) in v.px.iter_mut().enumerate() {
            let k = s[i];
            p[0] *= k;
            p[1] *= k;
            p[2] *= k;
        }
    }
}
