//! Procedural bark and foliage textures.
//!
//! Every species gets a tileable bark set (albedo sRGB, normal and
//! occlusion/roughness linear) and a foliage atlas: the top half holds leaf
//! cards (twig sprays drawn leaf by leaf), flower cards, a fruit patch and
//! palm fronds; the bottom half is reserved for the 8 impostor views.
//! Transparent texels are filled by pull-push dilation so mip filtering
//! never bleeds black into the leaf edges; the renderer then builds alpha
//! coverage preserving mips for masked materials.
//!
//! Normal maps use the engine's tangent convention (green follows +v, i.e.
//! down the image); the glTF exporter flips green for other viewers.

use std::sync::Arc;

use forja_assets::TextureData;
use forja_math::{Vec2, Vec3};
use rayon::prelude::*;

use crate::mesh::Rect;
use crate::rng::{Rng, Tile2, id_f};
use crate::species::{BarkStyle, LeafShape, PalmKind, Species, hex, linear_to_srgb};
use crate::{Pruning, Season};

pub const ATLAS: usize = 2048;
/// v at which the impostor half of the atlas starts.
pub const IMPOSTOR_V: f32 = 0.5;

/// Leaf-card slot `i` (4 × 2 grid in the top half).
pub fn slot_rect(i: usize) -> Rect {
    let (c, r) = ((i % 4) as f32, (i / 4) as f32);
    [c * 0.25, r * 0.25, c * 0.25 + 0.25, r * 0.25 + 0.25]
}

/// A column spanning both rows of the top half (palm fronds).
pub fn tall_rect(col: usize) -> Rect {
    let c = col as f32 * 0.25;
    [c, 0.0, c + 0.25, 0.5]
}

/// Impostor view `k` (4 × 2 grid in the bottom half).
pub fn impostor_rect(k: usize) -> Rect {
    let (c, r) = ((k % 4) as f32, (k / 4) as f32);
    [c * 0.25, 0.5 + r * 0.25, c * 0.25 + 0.25, 0.5 + r * 0.25 + 0.25]
}

/// Linear RGBA float image.
#[derive(Clone)]
pub struct Canvas {
    pub w: usize,
    pub h: usize,
    pub px: Vec<[f32; 4]>,
}

impl Canvas {
    pub fn new(w: usize, h: usize) -> Self {
        Canvas { w, h, px: vec![[0.0; 4]; w * h] }
    }

    #[inline]
    pub fn over(&mut self, x: usize, y: usize, c: [f32; 3], a: f32) {
        if x >= self.w || y >= self.h || a <= 0.0 {
            return;
        }
        let p = &mut self.px[y * self.w + x];
        let a = a.min(1.0);
        let oa = p[3];
        let na = a + oa * (1.0 - a);
        if na > 0.0 {
            for k in 0..3 {
                p[k] = (c[k] * a + p[k] * oa * (1.0 - a)) / na;
            }
        }
        p[3] = na;
    }

    /// Pull-push: transparent texels take the colour of nearby opaque ones
    /// (alpha and the colour of covered texels are unchanged).
    pub fn dilate(&mut self) {
        let mut levels: Vec<(usize, usize, Vec<[f32; 4]>)> = vec![(self.w, self.h, self.px.iter().map(|p| [p[0] * p[3], p[1] * p[3], p[2] * p[3], p[3]]).collect())];
        loop {
            let (w, h, src) = levels.last().unwrap();
            if *w < 2 || *h < 2 {
                break;
            }
            let (nw, nh) = (w / 2, h / 2);
            let mut dst = vec![[0.0f32; 4]; nw * nh];
            for y in 0..nh {
                for x in 0..nw {
                    let mut s = [0.0f32; 4];
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let p = src[(y * 2 + dy).min(h - 1) * w + (x * 2 + dx).min(w - 1)];
                        for k in 0..4 {
                            s[k] += p[k] * 0.25;
                        }
                    }
                    dst[y * nw + x] = s;
                }
            }
            levels.push((nw, nh, dst));
        }
        // Coarsest: normalise to an opaque fill colour.
        let mut fill: Vec<[f32; 3]> = {
            let (_, _, c) = levels.last().unwrap();
            c.iter().map(|p| if p[3] > 1e-6 { [p[0] / p[3], p[1] / p[3], p[2] / p[3]] } else { [0.0; 3] }).collect()
        };
        for li in (1..levels.len() - 1).rev() {
            let (w, h, ref cur) = levels[li];
            let cw = levels[li + 1].0;
            let ch = levels[li + 1].1;
            let mut nf = vec![[0.0f32; 3]; w * h];
            for y in 0..h {
                for x in 0..w {
                    let p = cur[y * w + x];
                    let f = fill[(y / 2).min(ch - 1) * cw + (x / 2).min(cw - 1)];
                    for k in 0..3 {
                        nf[y * w + x][k] = p[k] + (1.0 - p[3]) * f[k];
                    }
                }
            }
            fill = nf;
        }
        let (cw, ch) = if levels.len() > 1 { (levels[1].0, levels[1].1) } else { (self.w, self.h) };
        for y in 0..self.h {
            for x in 0..self.w {
                let i = y * self.w + x;
                if self.px[i][3] <= 1e-6 && levels.len() > 1 {
                    let f = fill[(y / 2).min(ch - 1) * cw + (x / 2).min(cw - 1)];
                    self.px[i] = [f[0], f[1], f[2], 0.0];
                }
            }
        }
    }

    pub fn to_texture(&self, name: &str, srgb: bool) -> TextureData {
        let data: Vec<u8> = self
            .px
            .iter()
            .flat_map(|p| {
                let e = |v: f32| if srgb { linear_to_srgb(v) } else { v.clamp(0.0, 1.0) };
                [(e(p[0]) * 255.0 + 0.5) as u8, (e(p[1]) * 255.0 + 0.5) as u8, (e(p[2]) * 255.0 + 0.5) as u8, (p[3].clamp(0.0, 1.0) * 255.0 + 0.5) as u8]
            })
            .collect();
        TextureData { name: name.into(), width: self.w as u32, height: self.h as u32, rgba8: data.into(), srgb }
    }
}

/// Tangent-space normal map (engine convention) from a height field.
fn normal_map(h: &[f32], w: usize, hh: usize, strength: f32, wrap: bool) -> Vec<[f32; 4]> {
    (0..w * hh)
        .into_par_iter()
        .map(|i| {
            let (x, y) = (i % w, i / w);
            let at = |x: isize, y: isize| -> f32 {
                let (x, y) = if wrap { (x.rem_euclid(w as isize), y.rem_euclid(hh as isize)) } else { (x.clamp(0, w as isize - 1), y.clamp(0, hh as isize - 1)) };
                h[y as usize * w + x as usize]
            };
            let (xi, yi) = (x as isize, y as isize);
            let dx = (at(xi + 1, yi) - at(xi - 1, yi)) * 0.5 * strength;
            let dy = (at(xi, yi + 1) - at(xi, yi - 1)) * 0.5 * strength;
            let n = Vec3::new(-dx, -dy, 1.0).normalize();
            [n.x * 0.5 + 0.5, n.y * 0.5 + 0.5, n.z * 0.5 + 0.5, 1.0]
        })
        .collect()
}

fn to_tex(px: Vec<[f32; 4]>, w: usize, h: usize, name: &str, srgb: bool) -> TextureData {
    Canvas { w, h, px }.to_texture(name, srgb)
}

/// Box-downsamples a float image by 2.
fn half(px: &[[f32; 4]], w: usize, h: usize) -> Vec<[f32; 4]> {
    let (nw, nh) = (w / 2, h / 2);
    (0..nw * nh)
        .map(|i| {
            let (x, y) = (i % nw, i / nw);
            let mut s = [0.0; 4];
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let p = px[(y * 2 + dy) * w + x * 2 + dx];
                for k in 0..4 {
                    s[k] += p[k] * 0.25;
                }
            }
            s
        })
        .collect()
}

/// A material's texture triple.
#[derive(Clone, Debug)]
pub struct TextureSet {
    pub albedo: TextureData,
    pub normal: TextureData,
    /// R = occlusion, G = roughness, B = metallic (0).
    pub orm: TextureData,
}

// ------------------------------------------------------------------ bark

pub const BARK: usize = 1024;

fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    let t = t.clamp(0.0, 1.0);
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

fn scale3(a: [f32; 3], s: f32) -> [f32; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Bark texel: (linear colour, height 0..1, roughness).
fn bark_texel(style: BarkStyle, pal: &[[f32; 3]], u: f32, v: f32, seed: u32) -> ([f32; 3], f32, f32) {
    let t = Tile2 { seed };
    let t2 = Tile2 { seed: seed ^ 0x55aa };
    let g = |i: usize| pal[i.min(pal.len() - 1)];
    match style {
        BarkStyle::Plane => {
            // Exfoliating patches: thresholds of two domain-warped noise
            // fields give irregular, soft-edged plates of different ages.
            let wu = u + 0.06 * t.fbm(u, v, 3, 3, 4);
            let wv = v + 0.06 * t2.fbm(u, v, 3, 3, 4);
            let a = t.fbm(wu, wv, 5, 4, 5);
            let b = t2.fbm(wv, wu, 4, 6, 4);
            let fine = t2.fbm(u, v, 40, 40, 3);
            let w = 0.012;
            let k1 = smoothstep(-w, w, a - 0.05);
            let k2 = smoothstep(-w, w, b + 0.1);
            let k3 = smoothstep(-w, w, a + b * 0.6 - 0.32);
            let mut c = mix3(g(0), g(3), k2);
            c = mix3(c, g(2), k1);
            c = mix3(c, g(1), k3);
            c = scale3(c, 0.93 + 0.1 * fine);
            // Newer layers sit a little lower: a soft step at patch edges.
            let h = 0.5 + 0.1 * (k1 - k3) + 0.05 * k2 + 0.04 * fine;
            (c, h, 0.78 - 0.12 * k3)
        }
        BarkStyle::Olive => {
            let wu = u + 0.06 * t.fbm(u, v, 3, 2, 3);
            let ridge = 1.0 - (t2.fbm(wu, v, 10, 3, 4)).abs() * 2.2;
            let ridge = ridge.clamp(0.0, 1.0).powf(1.6);
            let fine = t.fbm(u, v, 32, 24, 3);
            let lichen = smoothstep(0.55, 0.75, t.value(u, v, 12, 12) * 0.6 + 0.4 * t2.value(u, v, 30, 30));
            let mut c = mix3(scale3(g(0), 0.45), g(1), ridge * 0.9 + 0.1);
            c = mix3(c, g(2), lichen * 0.5);
            c = scale3(c, 0.9 + 0.12 * fine);
            (c, ridge * 0.85 + 0.1 * fine, 0.9)
        }
        BarkStyle::Pine => {
            // Tall reddish plates split by deep grey-brown fissures; plate
            // surfaces flake in thin layers and weather grey.
            let wu = u + 0.04 * t.fbm(u, v, 4, 3, 3);
            let wv = v + 0.03 * t2.fbm(u, v, 3, 4, 3);
            let (f1, f2, id) = t.worley(wu, wv, 8, 3);
            let gap = smoothstep(0.02, 0.2, f2 - f1);
            let fine = t2.fbm(u, v, 48, 32, 3);
            let k = id_f(id, 1);
            let flakes = (t.fbm(u, v, 6, 40, 3) * 0.5 + 0.5).powf(1.5);
            let plate = mix3(g(0), g(1), k * 0.8 + 0.2 * flakes);
            let weather = smoothstep(0.35, 0.95, f1 + 0.3 * t2.value(u, v, 10, 10)) * 0.65;
            let c = mix3(mix3(plate, g(2), weather), g(3), (1.0 - gap) * 0.9);
            let c = scale3(c, 0.9 + 0.14 * fine);
            (c, gap * (0.7 + 0.15 * flakes) + 0.05 * fine, 0.88)
        }
        BarkStyle::Smooth => {
            let fine = t.fbm(u, v, 24, 24, 4);
            let streak = t2.fbm(u, v, 30, 4, 3);
            let lent = smoothstep(0.72, 0.8, t.value(u, v, 40, 60));
            let c = mix3(g(0), g(1), 0.5 + 0.5 * streak);
            let c = mix3(scale3(c, 0.95 + 0.1 * fine), g(2), lent * 0.6);
            (c, 0.5 + 0.12 * fine + 0.08 * streak - lent * 0.1, 0.65)
        }
        BarkStyle::Elephant => {
            // Smooth grey with sparse irregular horizontal wrinkles and
            // darker blotches (ficus, hackberry).
            let warp = t.fbm(u, v, 4, 6, 3) * 0.06;
            let wr = ((v + warp) * 46.0 + t2.fbm(u, v, 6, 6, 3) * 4.0).sin();
            let mask = smoothstep(0.15, 0.6, t.fbm(u, v, 5, 9, 3));
            let wrinkle = smoothstep(0.75, 1.0, wr) * mask;
            let fine = t2.fbm(u, v, 34, 34, 3);
            let blotch = t.fbm(u, v, 3, 4, 4);
            let lichen = smoothstep(0.62, 0.8, t2.value(u, v, 9, 9)) * 0.5;
            let c = mix3(g(0), g(1), 0.45 + 0.6 * blotch);
            let c = mix3(c, hex("#8f9268"), lichen * 0.4);
            let c = scale3(c, 0.93 + 0.1 * fine - wrinkle * 0.25);
            (c, 0.5 + 0.07 * fine - wrinkle * 0.2 + blotch * 0.06, 0.72)
        }
        BarkStyle::Scaly => {
            // Small, vertically elongated scales with soft cracks.
            let wu = u + 0.02 * t.fbm(u, v, 6, 6, 3);
            let (f1, f2, id) = t.worley(wu, v, 20, 9);
            let gap = smoothstep(0.0, 0.18, f2 - f1);
            let fine = t2.fbm(u, v, 40, 40, 3);
            let c = mix3(g(0), g(1), id_f(id, 4));
            let c = mix3(scale3(g(2), 0.7), c, gap * 0.85 + 0.15);
            (scale3(c, 0.9 + 0.14 * fine), gap * 0.6 + 0.12 * fine, 0.85)
        }
        BarkStyle::Fibrous => {
            let wu = u + 0.02 * t.fbm(u, v, 4, 2, 3);
            let strip = (t2.fbm(wu, v, 28, 2, 4) * 0.5 + 0.5).powf(1.2);
            let peel = smoothstep(0.55, 0.8, t.value(wu, v, 16, 3));
            let c = mix3(scale3(g(0), 0.6), g(1), strip);
            let c = mix3(c, g(2), peel * 0.6);
            (c, strip * 0.8 + peel * 0.2, 0.9)
        }
        BarkStyle::Furrowed => {
            let wu = u + 0.04 * t.fbm(u, v, 4, 3, 3);
            let ridge = 1.0 - t2.fbm(wu, v, 9, 2, 4).abs() * 2.5;
            let ridge = ridge.clamp(0.0, 1.0);
            let cross = smoothstep(0.65, 0.9, t.value(wu, v, 9, 14));
            let fine = t.fbm(u, v, 40, 30, 3);
            let h = ridge * (1.0 - cross * 0.5);
            let c = mix3(scale3(g(0), 0.5), g(1), h);
            (scale3(c, 0.9 + 0.12 * fine), h * 0.85 + 0.08 * fine, 0.88)
        }
        BarkStyle::Datepalm => {
            // Cut leaf bases in a spiral lattice: each stub is a rounded
            // diamond, wider than tall, with fibre between the stubs.
            let wu = u + 0.025 * t.fbm(u, v, 4, 4, 3);
            let wv = v + 0.02 * t2.fbm(u, v, 4, 4, 3);
            let (x, y) = (wu * 7.0, wv * 9.0);
            let a = x + y;
            let b = x - y;
            let (ca, cb) = (a.floor(), b.floor());
            let (fa, fb) = (a - ca - 0.5, b - cb - 0.5);
            let cell = (ca as i32 * 73 + cb as i32 * 19) as u32;
            let r = id_f(cell, 1);
            let d = fa.abs().max(fb.abs()) * (0.92 + 0.16 * r);
            let stub = 1.0 - smoothstep(0.28, 0.47, d);
            // The cut face (upper half) is lighter and smoother.
            let face = smoothstep(-0.05, 0.25, -(fa - fb)) * stub;
            let fiber = t.fbm(u, v, 70, 10, 3);
            let base = mix3(g(0), g(1), 0.3 + 0.5 * r);
            let c = mix3(scale3(g(2), 0.6 + 0.3 * fiber), mix3(base, scale3(g(1), 1.12), face * 0.6), stub);
            let c = scale3(c, 0.88 + 0.16 * fiber);
            (c, stub * (0.55 + 0.35 * face) + 0.08 * fiber, 0.9 - 0.15 * face)
        }
        BarkStyle::Fanpalm => {
            // Grey-brown with irregular darker ring scars and vertical cracks.
            let ring_v = v * 11.0 + 0.25 * t.fbm(u, v, 3, 4, 3);
            let ring = (ring_v.fract() - 0.5).abs() * 2.0;
            let scar = smoothstep(0.82, 0.98, ring) * (0.6 + 0.4 * t2.value(u, v, 8, 11));
            let crack = smoothstep(0.7, 0.95, 1.0 - t2.fbm(u, v, 16, 3, 3).abs() * 3.0);
            let fine = t.fbm(u, v, 50, 40, 3);
            let c = mix3(g(0), g(1), 0.5 + 0.5 * t.fbm(u, v, 3, 6, 3));
            let c = scale3(c, (0.9 + 0.14 * fine) * (1.0 - 0.35 * scar) * (1.0 - 0.3 * crack));
            (c, 0.55 - 0.25 * scar - 0.2 * crack + 0.06 * fine, 0.85)
        }
    }
}

fn bark_palette(sp: &Species) -> Vec<[f32; 3]> {
    if !sp.bark.colors.is_empty() {
        return sp.bark.colors.iter().map(|c| hex(c)).collect();
    }
    let p: &[&str] = match sp.bark.style {
        BarkStyle::Plane => &["#6e6e56", "#c4bc9c", "#8f937f", "#86848a"],
        BarkStyle::Olive => &["#3e3a33", "#8f8b80", "#9a9a70"],
        BarkStyle::Pine => &["#7c4428", "#9a5a34", "#6b625a", "#2e241d"],
        BarkStyle::Smooth => &["#6c6a5a", "#7d7a66", "#9a9578"],
        BarkStyle::Elephant => &["#66645e", "#7f7c74"],
        BarkStyle::Scaly => &["#54493f", "#6c5f52", "#2f2721"],
        BarkStyle::Fibrous => &["#5a4636", "#7f6650", "#9a8a7a"],
        BarkStyle::Furrowed => &["#3b332b", "#6e6358"],
        BarkStyle::Datepalm => &["#6e5c45", "#8f7a5a", "#3a3026"],
        BarkStyle::Fanpalm => &["#6a645a", "#837c70"],
    };
    p.iter().map(|c| hex(c)).collect()
}

pub fn bark_set(sp: &Species, seed: u32) -> TextureSet {
    let pal = bark_palette(sp);
    let n = BARK;
    let style = sp.bark.style;
    let texels: Vec<([f32; 3], f32, f32)> = (0..n * n)
        .into_par_iter()
        .map(|i| {
            let (x, y) = (i % n, i / n);
            bark_texel(style, &pal, (x as f32 + 0.5) / n as f32, (y as f32 + 0.5) / n as f32, seed)
        })
        .collect();
    let albedo: Vec<[f32; 4]> = texels.iter().map(|(c, _, _)| [c[0], c[1], c[2], 1.0]).collect();
    let height: Vec<f32> = texels.iter().map(|(_, h, _)| *h).collect();
    let strength = match style {
        BarkStyle::Plane | BarkStyle::Smooth | BarkStyle::Elephant => 9.0,
        BarkStyle::Fanpalm => 14.0,
        _ => 24.0,
    };
    let normal = normal_map(&height, n, n, strength, true);
    let orm: Vec<[f32; 4]> = texels.iter().map(|(_, h, r)| [(0.45 + 0.55 * h.clamp(0.0, 1.0)).min(1.0), (*r * sp.bark.roughness / 0.85).clamp(0.3, 1.0), 0.0, 1.0]).collect();
    // Data maps at half resolution keep memory down.
    let n2 = n / 2;
    TextureSet {
        albedo: to_tex(albedo, n, n, &format!("{}_bark_albedo", sp.id), true),
        normal: to_tex(half(&normal, n, n), n2, n2, &format!("{}_bark_normal", sp.id), false),
        orm: to_tex(half(&orm, n, n), n2, n2, &format!("{}_bark_orm", sp.id), false),
    }
}

// ---------------------------------------------------------------- foliage

/// What one atlas slot holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotKind {
    Leaves,
    Inner,
    Bloom,
    InnerBloom,
    Bare,
    Fruit,
    Frond,
    FrondInner,
    FrondDead,
    Fan,
    FanInner,
    FanDead,
    Petiole,
}

#[derive(Clone, Debug)]
pub struct FoliageAtlas {
    pub tex: TextureSet,
    pub slots: Vec<(SlotKind, Rect)>,
}

impl FoliageAtlas {
    pub fn of(&self, k: SlotKind) -> Vec<Rect> {
        self.slots.iter().filter(|s| s.0 == k).map(|s| s.1).collect()
    }
}

/// Atlas layers drawn together.
struct Layers {
    w: usize,
    col: Canvas,
    height: Vec<f32>,
    rough: Vec<f32>,
}

impl Layers {
    fn new(w: usize, h: usize) -> Self {
        Layers { w, col: Canvas::new(w, h), height: vec![0.0; w * h], rough: vec![0.6; w * h] }
    }

    #[inline]
    fn put(&mut self, x: i32, y: i32, c: [f32; 3], a: f32, h: f32, r: f32) {
        if x < 0 || y < 0 || x as usize >= self.w || y as usize >= self.col.h || a <= 0.0 {
            return;
        }
        let (x, y) = (x as usize, y as usize);
        let i = y * self.w + x;
        self.col.over(x, y, c, a);
        let a = a.min(1.0);
        self.height[i] = self.height[i] * (1.0 - a) + h * a;
        self.rough[i] = self.rough[i] * (1.0 - a) + r * a;
    }
}

/// Region of the layers a slot draws into (pixels).
#[derive(Clone, Copy)]
struct Area {
    x0: f32,
    y0: f32,
    w: f32,
    h: f32,
}

impl Area {
    fn of(r: Rect) -> Self {
        let s = ATLAS as f32;
        Area { x0: r[0] * s, y0: r[1] * s, w: (r[2] - r[0]) * s, h: (r[3] - r[1]) * s }
    }

    /// Normalised slot coords (0..1, y down) to pixels.
    fn p(&self, q: Vec2) -> Vec2 {
        Vec2::new(self.x0 + q.x * self.w, self.y0 + q.y * self.h)
    }

    fn contains(&self, x: i32, y: i32) -> bool {
        x as f32 >= self.x0 && (x as f32) < self.x0 + self.w && y as f32 >= self.y0 && (y as f32) < self.y0 + self.h
    }
}

/// Thick tapered line (twigs, petioles, needles).
fn stroke(l: &mut Layers, area: &Area, a: Vec2, b: Vec2, wa: f32, wb: f32, c: [f32; 3], rough: f32) {
    let lo = a.min(b) - Vec2::splat(wa.max(wb) + 2.0);
    let hi = a.max(b) + Vec2::splat(wa.max(wb) + 2.0);
    let d = b - a;
    let len2 = d.length_squared().max(1e-6);
    for y in lo.y.floor() as i32..=hi.y.ceil() as i32 {
        for x in lo.x.floor() as i32..=hi.x.ceil() as i32 {
            if !area.contains(x, y) {
                continue;
            }
            let p = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
            let t = ((p - a).dot(d) / len2).clamp(0.0, 1.0);
            let q = a + d * t;
            let r = (wa + (wb - wa) * t) * 0.5;
            let dist = (p - q).length();
            let cov = (r - dist + 0.5).clamp(0.0, 1.0);
            if cov > 0.0 {
                let shade = 1.0 - 0.35 * (dist / r.max(0.5)).powi(2);
                let h = 0.6 + 0.4 * (1.0 - (dist / r.max(0.5)).powi(2)).max(0.0);
                l.put(x, y, scale3(c, shade), cov, h, rough);
            }
        }
    }
}

/// Filled ellipse/disc (fruit, flowers, seedballs).
fn blob(l: &mut Layers, area: &Area, c0: Vec2, rx: f32, ry: f32, ang: f32, col: [f32; 3], rough: f32, light: f32) {
    let r = rx.max(ry) + 1.5;
    let (ca, sa) = (ang.cos(), ang.sin());
    for y in (c0.y - r).floor() as i32..=(c0.y + r).ceil() as i32 {
        for x in (c0.x - r).floor() as i32..=(c0.x + r).ceil() as i32 {
            if !area.contains(x, y) {
                continue;
            }
            let p = Vec2::new(x as f32 + 0.5, y as f32 + 0.5) - c0;
            let q = Vec2::new(p.x * ca + p.y * sa, -p.x * sa + p.y * ca);
            let d = ((q.x / rx).powi(2) + (q.y / ry).powi(2)).sqrt();
            let cov = ((1.0 - d) * rx.min(ry) + 0.5).clamp(0.0, 1.0);
            if cov > 0.0 {
                let nrm = (1.0 - d * d).max(0.0).sqrt();
                let shade = 0.75 + light * 0.25 * (nrm - q.y / ry * 0.3);
                l.put(x, y, scale3(col, shade), cov, 0.5 + 0.5 * nrm, rough);
            }
        }
    }
}

/// Leaf half-width (fraction of length) at `t` along the midrib.
fn leaf_profile(shape: LeafShape, t: f32, width: f32) -> f32 {
    let pi = std::f32::consts::PI;
    match shape {
        LeafShape::Ovate | LeafShape::Serrate => {
            // Widest at ~35 %, acuminate tip.
            let s = (pi * t.powf(0.75)).sin().max(0.0);
            0.5 * width * s.powf(0.9) * (1.0 - 0.25 * t)
        }
        LeafShape::Elliptic => 0.5 * width * (pi * t).sin().max(0.0).powf(0.75),
        LeafShape::Lanceolate => 0.5 * width * (pi * t.powf(0.8)).sin().max(0.0).powf(0.7) * (1.0 - 0.3 * t),
        _ => 0.5 * width * (pi * t).sin().max(0.0),
    }
}

/// Plane-tree leaf outline: radius at polar angle `phi` (0 = along the
/// midrib), in units of the leaf length.
fn palmate_radius(phi: f32, lobes: &[(f32, f32, f32)]) -> f32 {
    let mut r: f32 = 0.22;
    for (a, len, w) in lobes {
        let d = (phi - a).abs();
        let g = (1.0 - d / w).max(0.0);
        r = r.max(len * g.powf(0.55));
    }
    r * (1.0 + 0.035 * (phi * 23.0).sin())
}

#[derive(Clone, Copy)]
struct LeafPaint {
    color: [f32; 3],
    vein: [f32; 3],
    rough: f32,
    underside: bool,
}

/// Draws one leaf blade from `base` along `dir` (pixels).
fn leaf(l: &mut Layers, area: &Area, shape: LeafShape, base: Vec2, dir: Vec2, len: f32, width: f32, paint: LeafPaint, rng: &mut Rng) {
    let dir = dir.normalize_or(Vec2::new(0.0, -1.0));
    let side = Vec2::new(-dir.y, dir.x);
    let curl = rng.signed() * 0.12;
    let palm_lobes: Vec<(f32, f32, f32)> = if shape == LeafShape::Palmate {
        let s = rng.range(0.9, 1.1);
        vec![(0.0, 1.0, 0.55), (0.85 * s, 0.82, 0.5), (-0.85 * s, 0.82, 0.5), (1.65, 0.48, 0.45), (-1.65, 0.48, 0.45)]
    } else {
        Vec::new()
    };
    let reach = if shape == LeafShape::Palmate { len } else { len.max(width * len) };
    let lo = base - Vec2::splat(reach + 2.0);
    let hi = base + Vec2::splat(reach + 2.0);
    let serr = shape == LeafShape::Serrate;
    for y in lo.y.floor() as i32..=hi.y.ceil() as i32 {
        for x in lo.x.floor() as i32..=hi.x.ceil() as i32 {
            if !area.contains(x, y) {
                continue;
            }
            let p = Vec2::new(x as f32 + 0.5, y as f32 + 0.5) - base;
            let along = p.dot(dir);
            let across = p.dot(side) - curl * along * along / len.max(1.0);
            let (cov, t, rel, inside_d) = if shape == LeafShape::Palmate {
                // Polar outline around a point a third up the blade.
                let c = dir * len * 0.32;
                let q = p - c;
                let rho = q.length();
                let phi = q.dot(side).atan2(q.dot(dir));
                let rr = palmate_radius(phi, &palm_lobes) * len * 0.68;
                let cov = (rr - rho + 0.5).clamp(0.0, 1.0);
                (cov, (along / len).clamp(0.0, 1.0), (rho / rr.max(1.0)).min(1.0), rr - rho)
            } else {
                let t = along / len;
                if !(0.0..=1.0).contains(&t) {
                    continue;
                }
                let mut hw = leaf_profile(shape, t, width) * len;
                if serr {
                    hw *= 1.0 - 0.06 * ((t * 38.0).fract() - 0.5).abs() * 2.0;
                }
                let cov = (hw - across.abs() + 0.5).clamp(0.0, 1.0);
                (cov, t, (across.abs() / hw.max(0.5)).min(1.0), hw - across.abs())
            };
            if cov <= 0.0 {
                continue;
            }
            // Venation: midrib and lateral veins.
            let mid = (1.0 - (across.abs() / (len * 0.012 + 0.6))).clamp(0.0, 1.0);
            let lat_phase = (t * 9.0 - rel * 2.2).fract();
            let lat = (1.0 - (lat_phase - 0.5).abs() * 14.0).clamp(0.0, 1.0) * (1.0 - rel) * 0.5;
            let lat = if shape == LeafShape::Palmate {
                let q = p - dir * len * 0.32;
                let phi = q.dot(side).atan2(q.dot(dir));
                palm_lobes.iter().map(|(a, _, _)| (1.0 - ((phi - a).abs() * q.length() / (len * 0.01 + 0.7))).clamp(0.0, 1.0)).fold(0.0, f32::max) * 0.8
            } else {
                lat
            };
            let grad = 0.82 + 0.28 * t;
            let edge = smoothstep(0.0, 2.5, inside_d);
            let mut c = scale3(paint.color, grad * (0.85 + 0.15 * edge));
            c = mix3(c, paint.vein, (mid + lat).min(1.0) * 0.55);
            let dome = 1.0 - rel * rel;
            let h = 0.35 + 0.35 * dome - 0.25 * mid - 0.08 * lat;
            let r = paint.rough + if paint.underside { 0.15 } else { 0.0 };
            l.put(x, y, c, cov, h, r);
        }
    }
}

fn vary_color(c: [f32; 3], rng: &mut Rng, amount: f32) -> [f32; 3] {
    let v = 1.0 + amount * rng.signed();
    let warm = amount * 0.6 * rng.signed();
    [c[0] * v * (1.0 + warm), c[1] * v, c[2] * v * (1.0 - warm)]
}

/// Shared inputs for one atlas.
pub struct FoliageStyle<'a> {
    pub sp: &'a Species,
    pub season: Season,
    pub health: f32,
    pub pruning: Pruning,
}

impl FoliageStyle<'_> {
    pub fn leafless(&self) -> bool {
        self.sp.deciduous && self.season == Season::Winter
    }

    pub fn blooming(&self) -> bool {
        self.sp.bloom.is_some() && self.season == Season::Spring
    }
}

pub fn foliage_atlas(st: &FoliageStyle, seed: u32) -> FoliageAtlas {
    let mut l = Layers::new(ATLAS, ATLAS);
    let mut slots = Vec::new();
    let mut rng = Rng::new(seed as u64 ^ 0xf011a6e);
    if let Some(pd) = &st.sp.palm {
        palm_slots(st, pd, &mut l, &mut slots, &mut rng);
    } else {
        let leafless = st.leafless();
        let bloom = st.blooming();
        let before = st.sp.bloom.as_ref().is_some_and(|b| b.before_leaves);
        // Outer variants 0..3, inner variants 4..6, misc 7.
        let kinds: [SlotKind; 4] = if leafless {
            [SlotKind::Bare; 4]
        } else if bloom && before {
            [SlotKind::Leaves, SlotKind::Bloom, SlotKind::Bloom, SlotKind::Bloom]
        } else if bloom {
            [SlotKind::Leaves, SlotKind::Leaves, SlotKind::Leaves, SlotKind::Bloom]
        } else {
            [SlotKind::Leaves; 4]
        };
        for (i, k) in kinds.iter().enumerate() {
            let r = slot_rect(i);
            draw_card(st, &mut l, &Area::of(r), *k, &mut rng.fork(i as u64 + 1));
            slots.push((*k, r));
        }
        for i in 0..3 {
            let src = slot_rect(i);
            let dst = slot_rect(4 + i);
            copy_dark(&mut l, src, dst, if kinds[i] == SlotKind::Bare { 0.8 } else { 0.62 });
            let k = match kinds[i] {
                SlotKind::Bloom => SlotKind::InnerBloom,
                SlotKind::Bare => SlotKind::Bare,
                _ => SlotKind::Inner,
            };
            slots.push((k, dst));
        }
        let misc = slot_rect(7);
        fruit_patch(st, &mut l, &Area::of(misc), &mut rng.fork(99));
        slots.push((SlotKind::Fruit, [misc[0], misc[1], misc[0] + 0.125, misc[1] + 0.125]));
    }
    finish_atlas(st.sp, l, slots)
}

fn finish_atlas(sp: &Species, mut l: Layers, slots: Vec<(SlotKind, Rect)>) -> FoliageAtlas {
    l.col.dilate();
    let albedo = l.col.to_texture(&format!("{}_foliage_albedo", sp.id), true);
    drop(l.col);
    // Data maps at half resolution, built directly at that size.
    let n = ATLAS;
    let n2 = n / 2;
    let down = |v: &[f32]| -> Vec<f32> { (0..n2 * n2).map(|i| { let (x, y) = (i % n2 * 2, i / n2 * 2); (v[y * n + x] + v[y * n + x + 1] + v[(y + 1) * n + x] + v[(y + 1) * n + x + 1]) * 0.25 }).collect() };
    let h2 = down(&l.height);
    let r2 = down(&l.rough);
    drop(l.height);
    drop(l.rough);
    let normal = normal_map(&h2, n2, n2, 1.5, false);
    let orm: Vec<[f32; 4]> = r2.iter().map(|r| [1.0, r.clamp(0.2, 1.0), 0.0, 1.0]).collect();
    FoliageAtlas {
        tex: TextureSet {
            albedo,
            normal: to_tex(normal, n2, n2, &format!("{}_foliage_normal", sp.id), false),
            orm: to_tex(orm, n2, n2, &format!("{}_foliage_orm", sp.id), false),
        },
        slots,
    }
}

fn copy_dark(l: &mut Layers, src: Rect, dst: Rect, k: f32) {
    let s = ATLAS as f32;
    let (sx, sy) = ((src[0] * s) as usize, (src[1] * s) as usize);
    let (dx, dy) = ((dst[0] * s) as usize, (dst[1] * s) as usize);
    let (w, h) = (((src[2] - src[0]) * s) as usize, ((src[3] - src[1]) * s) as usize);
    for y in 0..h {
        for x in 0..w {
            let i = (sy + y) * l.w + sx + x;
            let j = (dy + y) * l.w + dx + x;
            let p = l.col.px[i];
            // Inner foliage: darker, slightly bluer (sky-lit shade).
            l.col.px[j] = [p[0] * k * 0.92, p[1] * k, p[2] * k * 1.08, p[3]];
            l.height[j] = l.height[i];
            l.rough[j] = l.rough[i];
        }
    }
}

fn fruit_patch(st: &FoliageStyle, l: &mut Layers, a: &Area, rng: &mut Rng) {
    let c = st.sp.fruit.as_ref().map(|f| hex(&f.color)).unwrap_or([0.3, 0.25, 0.15]);
    let pw = a.w * 0.5;
    let t = Tile2 { seed: rng.next_u64() as u32 };
    for y in 0..pw as i32 {
        for x in 0..pw as i32 {
            let (u, v) = (x as f32 / pw, y as f32 / pw);
            let dimple = t.fbm(u, v, 24, 24, 2);
            let shade = 0.88 + 0.18 * (1.0 - (v - 0.35).abs()) + 0.05 * dimple;
            l.put((a.x0 + x as f32) as i32, (a.y0 + y as f32) as i32, scale3(c, shade), 1.0, 0.5 + 0.1 * dimple, 0.35);
        }
    }
}

/// One twig spray card.
fn draw_card(st: &FoliageStyle, l: &mut Layers, a: &Area, kind: SlotKind, rng: &mut Rng) {
    let lf = &st.sp.leaves;
    let px_per_m = a.w / lf.card_size.max(0.05);
    let twig_c = hex(&lf.twig);
    let bare = kind == SlotKind::Bare;
    // Main twig with a gentle S-curve, side twigs alternating.
    let top_y = 0.06;
    let mut twigs: Vec<(Vec2, Vec2, f32)> = Vec::new();
    let bend = rng.signed() * 0.08;
    let mut prev = Vec2::new(0.5, 1.0);
    let segs = 8;
    let main_w = (0.007 * px_per_m).clamp(2.0, 9.0);
    for i in 1..=segs {
        let t = i as f32 / segs as f32;
        let p = Vec2::new(0.5 + bend * (t * std::f32::consts::PI).sin() + rng.signed() * 0.012, 1.0 - t * (1.0 - top_y));
        twigs.push((prev, p, main_w * (1.0 - 0.7 * t)));
        prev = p;
    }
    let n_side = if bare { 5 } else { 4 + rng.below(3) };
    let mut sides = Vec::new();
    for k in 0..n_side {
        let t = 0.22 + 0.6 * (k as f32 + 0.5 + 0.3 * rng.signed()) / n_side as f32;
        let base = Vec2::new(0.5 + bend * (t * std::f32::consts::PI).sin(), 1.0 - t * (1.0 - top_y));
        let s = if k % 2 == 0 { 1.0 } else { -1.0 };
        let ang = (rng.range(32.0, 58.0) * s).to_radians();
        let dir = Vec2::new(ang.sin(), -ang.cos());
        let len = rng.range(0.3, 0.48) * (1.0 - t * 0.45);
        let mut p0 = base;
        let n = 4;
        for j in 1..=n {
            let q = base + dir * len * j as f32 / n as f32 + Vec2::new(0.0, -0.02 * (j * j) as f32 / (n * n) as f32);
            twigs.push((p0, q, main_w * 0.55 * (1.0 - 0.6 * j as f32 / n as f32)));
            p0 = q;
        }
        sides.push((base, dir, len));
        if bare {
            // Winter: finer sub-twigs and buds.
            for _ in 0..2 {
                let tt = rng.range(0.3, 0.9);
                let b = base + dir * len * tt;
                let a2 = ang + (rng.range(25.0, 50.0) * if rng.chance(0.5) { 1.0 } else { -1.0 }).to_radians();
                let d2 = Vec2::new(a2.sin(), -a2.cos());
                twigs.push((b, b + d2 * len * 0.35, main_w * 0.3));
            }
        }
    }
    for (p, q, w) in &twigs {
        stroke(l, a, a.p(*p), a.p(*q), *w, *w * 0.8, scale3(twig_c, rng.range(0.85, 1.1)), 0.8);
    }
    if bare {
        let seed_balls = st.sp.fruit.as_ref().is_some_and(|f| f.kind == crate::species::FruitKind::Seedball);
        for (b, d, len) in &sides {
            if seed_balls && rng.chance(0.6) {
                let at = *b + *d * *len * rng.range(0.4, 1.0);
                let stalk = at + Vec2::new(rng.signed() * 0.03, 0.12);
                stroke(l, a, a.p(at), a.p(stalk), 1.5, 1.2, twig_c, 0.8);
                blob(l, a, a.p(stalk), 0.018 * px_per_m, 0.018 * px_per_m, 0.0, hex("#6b5233"), 0.9, 1.0);
            }
            let tip = *b + *d * *len;
            blob(l, a, a.p(tip), 2.5, 3.5, d.x.atan2(-d.y), scale3(twig_c, 0.8), 0.7, 0.5);
        }
        return;
    }
    if kind == SlotKind::Bloom {
        draw_flowers(st, l, a, &twigs, &sides, px_per_m, rng);
        return;
    }
    // Attachment points: along side twigs and the main twig's upper part.
    let mut spots: Vec<(Vec2, Vec2)> = Vec::new();
    for (b, d, len) in &sides {
        let n = 4 + rng.below(3);
        for j in 0..n {
            let t = (j as f32 + 0.6) / n as f32;
            spots.push((*b + *d * *len * t, *d));
        }
    }
    for j in 0..8 {
        let t = 0.25 + 0.75 * j as f32 / 7.0;
        spots.push((Vec2::new(0.5 + bend * (t * std::f32::consts::PI).sin(), 1.0 - t * (1.0 - top_y)), Vec2::new(0.0, -1.0)));
    }
    match lf.shape {
        LeafShape::Needle => draw_needles(st, l, a, &spots, px_per_m, rng),
        LeafShape::Shortneedle => draw_short_needles(st, l, a, &twigs, px_per_m, rng),
        LeafShape::Scale => draw_scale_spray(st, l, a, px_per_m, rng),
        LeafShape::Bipinnate => draw_bipinnate(st, l, a, &spots, px_per_m, rng),
        _ => draw_leaves(st, l, a, &spots, px_per_m, rng),
    }
    // Orange blossom among the leaves.
    if st.blooming() && !st.sp.bloom.as_ref().is_some_and(|b| b.before_leaves) && st.sp.fruit.is_some() {
        let bc = hex(&st.sp.bloom.as_ref().unwrap().color);
        for _ in 0..6 {
            let (p, _) = spots[rng.below(spots.len())];
            star(l, a, a.p(p + Vec2::new(rng.signed() * 0.05, rng.signed() * 0.05)), 0.012 * px_per_m, bc, rng);
        }
    }
}

fn leaf_paint(st: &FoliageStyle, rng: &mut Rng) -> LeafPaint {
    let lf = &st.sp.leaves;
    let under = rng.chance(lf.underside_share);
    let mut c = if under { hex(&lf.underside) } else if rng.chance(0.5) { hex(&lf.color) } else { hex(&lf.color_alt) };
    if st.season == Season::Autumn && st.sp.deciduous {
        c = mix3(c, hex(&lf.autumn), rng.range(0.5, 1.0));
    }
    // Unhealthy trees: yellowing and brown leaves.
    let sick = (1.0 - st.health).clamp(0.0, 1.0);
    if rng.chance(sick * 0.6) {
        c = mix3(c, if rng.chance(0.5) { hex("#a89a4a") } else { hex("#7a5a36") }, rng.range(0.4, 0.9));
    }
    let c = vary_color(c, rng, lf.color_var);
    let vein = mix3(c, hex("#c8c878"), if under { 0.25 } else { 0.4 });
    LeafPaint { color: c, vein, rough: lf.roughness, underside: under }
}

fn draw_leaves(st: &FoliageStyle, l: &mut Layers, a: &Area, spots: &[(Vec2, Vec2)], px_per_m: f32, rng: &mut Rng) {
    let lf = &st.sp.leaves;
    let n = lf.per_card.max(3) as usize;
    let twig_c = hex(&lf.twig);
    let missing = (1.0 - st.health).clamp(0.0, 1.0) * 0.5;
    let mut order: Vec<usize> = (0..n).collect();
    for i in (1..n).rev() {
        order.swap(i, rng.below(i + 1));
    }
    for k in order {
        if rng.chance(missing) {
            continue;
        }
        let (p, d) = spots[k % spots.len()];
        let side = if k % 2 == 0 { 1.0 } else { -1.0 };
        let ang = rng.range(25.0, 70.0).to_radians() * side;
        let (s, c) = ang.sin_cos();
        let dir = Vec2::new(d.x * c - d.y * s, d.x * s + d.y * c);
        let len_m = lf.length * (1.0 + lf.length_var * rng.signed());
        let len = len_m * px_per_m;
        let pet = len * if lf.shape == LeafShape::Palmate { 0.35 } else { 0.12 };
        let base = a.p(p);
        let blade0 = base + dir * pet;
        stroke(l, a, base, blade0, (len * 0.03).max(1.2), (len * 0.02).max(1.0), mix3(twig_c, hex(&lf.color), 0.5), 0.6);
        let paint = leaf_paint(st, rng);
        // Foreshortening: some leaves seen at an angle look narrower.
        let fs = rng.range(0.55, 1.0);
        leaf(l, a, lf.shape, blade0, dir, len, lf.width * fs, paint, rng);
    }
}

fn draw_needles(st: &FoliageStyle, l: &mut Layers, a: &Area, spots: &[(Vec2, Vec2)], px_per_m: f32, rng: &mut Rng) {
    let lf = &st.sp.leaves;
    // Tufts at the twig tips: many needles fanning forward.
    let tips: Vec<(Vec2, Vec2)> = spots.iter().enumerate().filter(|(i, _)| i % 2 == 0).map(|(_, s)| *s).collect();
    for (p, d) in tips {
        let n = (lf.per_card as usize).max(20);
        for _ in 0..n {
            let ang = rng.normal() * 0.75;
            let (s, c) = ang.sin_cos();
            let dir = Vec2::new(d.x * c - d.y * s, d.x * s + d.y * c);
            let base = a.p(p) + dir * rng.range(0.0, 4.0);
            let len = lf.length * px_per_m * rng.range(0.7, 1.1);
            let paint = leaf_paint(st, rng);
            let tip = base + dir * len + Vec2::new(0.0, len * 0.08 * rng.f32());
            stroke(l, a, base, tip, 2.2, 1.0, paint.color, lf.roughness);
        }
    }
}

fn draw_short_needles(st: &FoliageStyle, l: &mut Layers, a: &Area, twigs: &[(Vec2, Vec2, f32)], px_per_m: f32, rng: &mut Rng) {
    let lf = &st.sp.leaves;
    for (p, q, _) in twigs {
        let d = (*q - *p).normalize_or(Vec2::Y);
        let seg = (*q - *p).length() * a.w;
        let n = (seg / 2.2) as usize;
        for k in 0..n {
            let t = k as f32 / n.max(1) as f32;
            let base = a.p(*p + (*q - *p) * t);
            for s in [-1.0f32, 1.0] {
                let ang = (rng.range(40.0, 75.0) * s).to_radians();
                let (sn, c) = ang.sin_cos();
                let dir = Vec2::new(d.x * c - d.y * sn, d.x * sn + d.y * c);
                let len = lf.length * px_per_m * rng.range(0.7, 1.1);
                let paint = leaf_paint(st, rng);
                stroke(l, a, base, base + dir * len, 2.4, 1.2, paint.color, lf.roughness);
            }
        }
    }
}

fn draw_scale_spray(st: &FoliageStyle, l: &mut Layers, a: &Area, px_per_m: f32, rng: &mut Rng) {
    // Recursive flattened sprays of scale leaves (lumpy thick strokes).
    fn branch(st: &FoliageStyle, l: &mut Layers, a: &Area, p: Vec2, d: Vec2, len: f32, w: f32, depth: u32, rng: &mut Rng) {
        let n = 5;
        let mut q = p;
        let paint = leaf_paint(st, rng);
        for i in 0..n {
            let t = (i + 1) as f32 / n as f32;
            let nq = p + d * len * t + Vec2::new(rng.signed(), rng.signed()) * 0.004;
            let ww = w * (1.0 - 0.5 * t);
            stroke(l, a, a.p(q), a.p(nq), ww, ww * 0.85, scale3(paint.color, rng.range(0.85, 1.12)), st.sp.leaves.roughness);
            q = nq;
        }
        if depth == 0 {
            return;
        }
        let k = 5 + rng.below(3);
        for j in 0..k {
            let t = 0.15 + 0.8 * j as f32 / k as f32;
            let s = if j % 2 == 0 { 1.0 } else { -1.0 };
            let ang = (rng.range(30.0, 50.0) * s).to_radians();
            let (sn, c) = ang.sin_cos();
            let nd = Vec2::new(d.x * c - d.y * sn, d.x * sn + d.y * c);
            branch(st, l, a, p + d * len * t, nd, len * 0.42 * (1.0 - 0.5 * t), w * 0.75, depth - 1, rng);
        }
    }
    let w = (st.sp.leaves.length * px_per_m).clamp(4.0, 14.0);
    branch(st, l, a, Vec2::new(0.5, 0.98), Vec2::new(0.0, -1.0), 0.9, w, 2, rng);
}

fn draw_bipinnate(st: &FoliageStyle, l: &mut Layers, a: &Area, spots: &[(Vec2, Vec2)], px_per_m: f32, rng: &mut Rng) {
    let lf = &st.sp.leaves;
    let n = (lf.per_card as usize).max(3);
    for k in 0..n {
        let (p, d) = spots[(k * 3) % spots.len()];
        let side = if k % 2 == 0 { 1.0 } else { -1.0 };
        let ang = rng.range(20.0, 55.0).to_radians() * side;
        let (s, c) = ang.sin_cos();
        let dir = Vec2::new(d.x * c - d.y * s, d.x * s + d.y * c);
        let len = lf.length * px_per_m * rng.range(0.7, 1.1);
        let base = a.p(p);
        let rach = base + dir * len;
        let paint = leaf_paint(st, rng);
        stroke(l, a, base, rach, 1.6, 1.0, mix3(paint.color, hex("#8a8a50"), 0.3), 0.6);
        // Pinnae with tiny leaflets.
        let pinnae = 7;
        let nrm = Vec2::new(-dir.y, dir.x);
        for j in 0..pinnae {
            let t = 0.15 + 0.8 * j as f32 / pinnae as f32;
            for s2 in [-1.0f32, 1.0] {
                let pb = base + dir * len * t;
                let pd = (nrm * s2 + dir * 0.6).normalize();
                let pl = len * 0.32 * (1.0 - 0.5 * t);
                let leaflets = 7;
                for m in 0..leaflets {
                    let tt = (m as f32 + 0.5) / leaflets as f32;
                    let lb = pb + pd * pl * tt;
                    let pdn = Vec2::new(-pd.y, pd.x);
                    for s3 in [-1.0f32, 1.0] {
                        let ld = (pdn * s3 + pd * 0.7).normalize();
                        let ll = (pl * 0.17).max(3.0);
                        let p2 = leaf_paint(st, rng);
                        leaf(l, a, LeafShape::Elliptic, lb, ld, ll, 0.36, p2, rng);
                    }
                }
            }
        }
    }
}

fn star(l: &mut Layers, a: &Area, c: Vec2, r: f32, col: [f32; 3], rng: &mut Rng) {
    let rot = rng.f32() * std::f32::consts::TAU;
    for k in 0..5 {
        let ang = rot + k as f32 / 5.0 * std::f32::consts::TAU;
        let d = Vec2::new(ang.cos(), ang.sin());
        blob(l, a, c + d * r * 0.6, r * 0.55, r * 0.3, ang, col, 0.5, 1.0);
    }
    blob(l, a, c, r * 0.25, r * 0.25, 0.0, hex("#e8d860"), 0.5, 0.5);
}

fn draw_flowers(st: &FoliageStyle, l: &mut Layers, a: &Area, twigs: &[(Vec2, Vec2, f32)], sides: &[(Vec2, Vec2, f32)], px_per_m: f32, rng: &mut Rng) {
    let b = st.sp.bloom.clone().unwrap_or_default();
    let (c1, c2) = (hex(&b.color), hex(&b.color_alt));
    let fr = b.size * px_per_m;
    let twig_c = hex(&st.sp.leaves.twig);
    // A few leaves first (jacaranda still carries some).
    let mut spots: Vec<(Vec2, Vec2)> = sides.iter().map(|(p, d, len)| (*p + *d * *len * 0.5, *d)).collect();
    if let Some((_, q, _)) = twigs.last() {
        spots.push((*q, Vec2::new(0.0, -1.0)));
    }
    if !b.before_leaves {
        draw_leaves(st, l, a, &spots, px_per_m, rng);
    }
    // Panicles: conical clusters of bell flowers at the twig ends.
    let mut ends: Vec<(Vec2, Vec2)> = sides.iter().map(|(p, d, len)| (*p + *d * *len, *d)).collect();
    if let Some((p, q, _)) = twigs.get(7) {
        ends.push((*q, (*q - *p).normalize_or(Vec2::new(0.0, -1.0))));
    }
    for (tip, d) in ends {
        let n = 26 + rng.below(14);
        let tipp = a.p(tip);
        for k in 0..n {
            let t = k as f32 / n as f32;
            let off = Vec2::new(rng.normal() * 0.35, rng.normal() * 0.35) * fr * 2.2 * (1.0 - 0.5 * t);
            let along = d * fr * 3.5 * t;
            let p = tipp + along + off;
            stroke(l, a, tipp + along * 0.5, p, 1.0, 0.8, twig_c, 0.7);
            let col = vary_color(mix3(c1, c2, rng.f32()), rng, 0.12);
            let ang = rng.f32() * std::f32::consts::TAU;
            blob(l, a, p, fr * 0.55, fr * 0.33, ang, col, 0.5, 1.0);
            blob(l, a, p + Vec2::new(ang.cos(), ang.sin()) * fr * 0.25, fr * 0.18, fr * 0.14, ang, scale3(col, 0.55), 0.5, 0.3);
        }
    }
}

// ------------------------------------------------------------------ palms

fn palm_slots(st: &FoliageStyle, pd: &crate::species::PalmDef, l: &mut Layers, slots: &mut Vec<(SlotKind, Rect)>, rng: &mut Rng) {
    let (g1, g2, dead) = (hex(&pd.color), hex(&pd.color_alt), hex(&pd.dead));
    match pd.kind {
        PalmKind::Pinnate => {
            let r0 = tall_rect(0);
            pinnate_frond(l, &Area::of(r0), g1, g2, pd, false, rng);
            slots.push((SlotKind::Frond, r0));
            let r1 = tall_rect(1);
            pinnate_frond(l, &Area::of(r1), scale3(g1, 0.62), scale3(g2, 0.6), pd, false, rng);
            slots.push((SlotKind::FrondInner, r1));
            let r2 = tall_rect(2);
            pinnate_frond(l, &Area::of(r2), dead, scale3(dead, 0.8), pd, true, rng);
            slots.push((SlotKind::FrondDead, r2));
        }
        PalmKind::Fan => {
            for (i, (k, c, d)) in [(SlotKind::Fan, g1, false), (SlotKind::Fan, g2, false), (SlotKind::FanDead, dead, true), (SlotKind::FanInner, scale3(g1, 0.62), false), (SlotKind::FanDead, scale3(dead, 0.85), true)].into_iter().enumerate() {
                let idx = [0, 1, 2, 4, 5][i];
                let r = slot_rect(idx);
                fan_blade(l, &Area::of(r), c, d, rng);
                slots.push((k, r));
            }
            let pr = slot_rect(6);
            petiole_strip(l, &Area::of(pr), hex("#6f7a48"), rng);
            slots.push((SlotKind::Petiole, pr));
        }
    }
    let misc = slot_rect(7);
    fruit_patch(st, l, &Area::of(misc), rng);
    slots.push((SlotKind::Fruit, [misc[0], misc[1], misc[0] + 0.125, misc[1] + 0.125]));
}

/// Half of a pinnate frond: rachis along the left edge (u = 0), leaflets
/// reaching right and toward the tip (v = 0 is the tip).
fn pinnate_frond(l: &mut Layers, a: &Area, c1: [f32; 3], c2: [f32; 3], pd: &crate::species::PalmDef, dead: bool, rng: &mut Rng) {
    // Pixels per metre differ across (width) and along (length).
    let sx = a.w / pd.blade_width.max(0.1);
    let sy = a.h / pd.frond_length.max(0.5);
    let rach_c = if dead { scale3(c1, 0.85) } else { mix3(c1, hex("#b8b070"), 0.45) };
    // Rachis.
    let n = 40;
    for i in 0..n {
        let t0 = i as f32 / n as f32;
        let t1 = (i + 1) as f32 / n as f32;
        let w0 = 4.0 + 26.0 * t0;
        let w1 = 4.0 + 26.0 * t1;
        stroke(l, a, Vec2::new(a.x0 + 2.0, a.y0 + t0 * a.h), Vec2::new(a.x0 + 2.0, a.y0 + t1 * a.h), w0, w1, rach_c, 0.6);
    }
    let spacing = 0.035;
    let count = (pd.frond_length / spacing) as usize;
    for k in 0..count {
        let s = (k as f32 + rng.signed() * 0.3) / count as f32; // 0 tip .. 1 base
        if dead && rng.chance(0.25) {
            continue;
        }
        // Leaflet length profile: spines at the base, longest at 40 %.
        let prof = if s > 0.82 { 0.25 * (1.0 - (s - 0.82) / 0.18) + 0.08 } else { (std::f32::consts::PI * (0.12 + s / 0.82 * 0.88)).sin().max(0.15) };
        let len_m = pd.blade_width * prof * rng.range(0.85, 1.05);
        let spine = s > 0.82;
        let ang = rng.range(28.0, 42.0).to_radians();
        let dirm = Vec2::new(ang.sin(), -ang.cos());
        let base = Vec2::new(a.x0 + 3.0, a.y0 + s * a.h);
        let tip = base + Vec2::new(dirm.x * len_m * sx, dirm.y * len_m * sy);
        let col = vary_color(mix3(c1, c2, rng.f32()), rng, 0.08);
        let w = if spine { 3.0 } else { (0.022 * (sx + sy) * 0.5).clamp(3.0, 9.0) };
        // Leaflets taper to a point; the tip browns a little.
        let mid = base + (tip - base) * 0.75;
        stroke(l, a, base, mid, w, w * 0.8, col, if dead { 0.9 } else { 0.45 });
        let tipc = if dead { col } else { mix3(col, hex("#a09060"), 0.25) };
        stroke(l, a, mid, tip, w * 0.8, 1.0, tipc, 0.5);
        // Light midrib line.
        stroke(l, a, base, base + (tip - base) * 0.85, 1.0, 0.6, mix3(col, hex("#d0d090"), 0.4), 0.4);
    }
}

/// Costapalmate fan blade: hastula at the bottom centre, segments split
/// halfway with drooping tips and a few threads.
fn fan_blade(l: &mut Layers, a: &Area, c: [f32; 3], dead: bool, rng: &mut Rng) {
    let center = Vec2::new(a.x0 + a.w * 0.5, a.y0 + a.h * 0.5);
    let r = a.w * 0.48;
    let segs = 56;
    for y in a.y0 as i32..(a.y0 + a.h) as i32 {
        for x in a.x0 as i32..(a.x0 + a.w) as i32 {
            let p = Vec2::new(x as f32 + 0.5, y as f32 + 0.5) - center;
            let rho = p.length() / r;
            if rho > 1.0 {
                continue;
            }
            let th = p.y.atan2(p.x);
            let f = (th / std::f32::consts::TAU * segs as f32).rem_euclid(1.0);
            let k = (th / std::f32::consts::TAU * segs as f32).floor() as i32;
            // Splits between segments open beyond ~35 % of the radius and
            // the free segments narrow to points.
            let split = smoothstep(0.3, 0.52, rho);
            let half_w = 0.5 - 0.3 * split - 0.16 * smoothstep(0.6, 1.0, rho);
            let edge = half_w - (f - 0.5).abs();
            let rag = if dead { 0.8 + 0.2 * id_f(k as u32, 5) } else { 0.86 + 0.14 * id_f(k as u32, 5) };
            let cov = (edge * (std::f32::consts::TAU * rho * r / segs as f32) + 0.5).clamp(0.0, 1.0) * ((rag - rho) * r + 0.5).clamp(0.0, 1.0);
            if cov <= 0.0 {
                continue;
            }
            // Pleats: alternate light and dark folds.
            let pleat = (f * std::f32::consts::TAU).sin();
            let var = 0.92 + 0.12 * id_f(k as u32, 9);
            let tipdry = if dead { 1.0 } else { 1.0 - 0.25 * smoothstep(0.85, 1.0, rho) };
            let col = scale3(mix3(c, hex("#9a8a50"), if dead { 0.0 } else { 0.3 * smoothstep(0.85, 1.0, rho) }), var * (0.9 + 0.12 * pleat) * tipdry);
            l.put(x, y, col, cov, 0.5 + 0.3 * pleat, if dead { 0.9 } else { 0.5 });
        }
    }
    // Hastula and threads.
    blob(l, a, center, a.w * 0.035, a.w * 0.025, 0.0, scale3(c, 0.7), 0.6, 0.5);
    let threads = if dead { 4 } else { 10 };
    for _ in 0..threads {
        let th = rng.f32() * std::f32::consts::TAU;
        let p0 = center + Vec2::new(th.cos(), th.sin()) * r * 0.6;
        let p1 = p0 + Vec2::new(th.cos(), th.sin()) * r * 0.3 + Vec2::new(0.0, r * 0.1);
        stroke(l, a, p0, p1, 1.0, 0.8, hex("#cfc6a8"), 0.8);
    }
}

fn petiole_strip(l: &mut Layers, a: &Area, c: [f32; 3], rng: &mut Rng) {
    for y in a.y0 as i32..(a.y0 + a.h) as i32 {
        for x in a.x0 as i32..(a.x0 + a.w) as i32 {
            let u = (x as f32 - a.x0) / a.w;
            let v = (y as f32 - a.y0) / a.h;
            // Hooked teeth along the edges (Washingtonia robusta).
            let tooth = ((v * 40.0).fract() < 0.18) as i32 as f32;
            let lim = 0.38 + 0.1 * tooth;
            let d = (u - 0.5).abs();
            let cov = ((lim - d) * a.w + 0.5).clamp(0.0, 1.0);
            if cov > 0.0 {
                let shade = 0.85 + 0.2 * (1.0 - d / lim);
                let col = scale3(mix3(c, hex("#8a6a40"), v * 0.6 + 0.1 * rng.f32()), shade);
                l.put(x, y, col, cov, 0.6 - d, 0.6);
            }
        }
    }
}

/// Copies impostor views (straight-alpha linear RGBA, `size`²) into the
/// bottom half of an atlas albedo texture (sRGB RGBA8), dilating colour
/// into the transparent texels of each tile.
pub fn write_impostors(albedo: &TextureData, views: &[Canvas]) -> TextureData {
    let mut data = albedo.rgba8.to_vec();
    let w = albedo.width as usize;
    for (k, v) in views.iter().enumerate().take(8) {
        let r = impostor_rect(k);
        let (x0, y0) = ((r[0] * w as f32) as usize, (r[1] * albedo.height as f32) as usize);
        let tile = ((r[2] - r[0]) * w as f32) as usize;
        let mut c = v.clone();
        if c.w != tile {
            c = resample(&c, tile);
        }
        c.dilate();
        for y in 0..tile {
            for x in 0..tile {
                let p = c.px[y * tile + x];
                let o = ((y0 + y) * w + x0 + x) * 4;
                data[o] = (linear_to_srgb(p[0]) * 255.0 + 0.5) as u8;
                data[o + 1] = (linear_to_srgb(p[1]) * 255.0 + 0.5) as u8;
                data[o + 2] = (linear_to_srgb(p[2]) * 255.0 + 0.5) as u8;
                data[o + 3] = (p[3].clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            }
        }
    }
    TextureData { name: albedo.name.clone(), width: albedo.width, height: albedo.height, rgba8: Arc::from(data), srgb: true }
}

/// Box/bilinear resample of a square canvas to `n`².
pub fn resample(c: &Canvas, n: usize) -> Canvas {
    let mut o = Canvas::new(n, n);
    let s = c.w as f32 / n as f32;
    for y in 0..n {
        for x in 0..n {
            let mut acc = [0.0f32; 4];
            let mut cnt = 0.0;
            let (sx0, sy0) = ((x as f32 * s) as usize, (y as f32 * s) as usize);
            let (sx1, sy1) = (((x + 1) as f32 * s).ceil() as usize, ((y + 1) as f32 * s).ceil() as usize);
            for yy in sy0..sy1.min(c.h).max(sy0 + 1) {
                for xx in sx0..sx1.min(c.w).max(sx0 + 1) {
                    let p = c.px[yy.min(c.h - 1) * c.w + xx.min(c.w - 1)];
                    acc[0] += p[0] * p[3];
                    acc[1] += p[1] * p[3];
                    acc[2] += p[2] * p[3];
                    acc[3] += p[3];
                    cnt += 1.0;
                }
            }
            let a = acc[3] / cnt;
            o.px[y * n + x] = if acc[3] > 1e-6 { [acc[0] / acc[3], acc[1] / acc[3], acc[2] / acc[3], a] } else { [0.0; 4] };
        }
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dilation_fills_transparent_texels_without_touching_alpha() {
        let mut c = Canvas::new(16, 16);
        c.px[5 * 16 + 5] = [1.0, 0.0, 0.0, 1.0];
        c.dilate();
        assert_eq!(c.px[5 * 16 + 5], [1.0, 0.0, 0.0, 1.0]);
        let far = c.px[15 * 16 + 15];
        assert!(far[0] > 0.9 && far[3] == 0.0, "{far:?}");
    }
}
