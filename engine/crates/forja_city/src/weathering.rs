//! Facade weathering decals: thin alpha-blended quads a few millimetres in
//! front of the wall, cut around openings, carrying generated dirt.
//!
//! Real Madrid and Valencia facades are never uniform: rising damp and
//! splash-back darken the plinth with a pale salt tide line on top, water
//! running off sills and balcony slabs leaves streaks (rust-coloured under
//! wrought iron), and the sheltered strip under the main cornice collects
//! soot that drips down. The textures are generic (dark, low-saturation
//! colours with alpha), so they read correctly over any wall tint, and one
//! material per kind and dirt level is shared by the whole city.

use forja_assets::TextureData;
use forja_math::{Vec2, Vec3};

use crate::geom::{Parts, Prim, Rng};
use crate::kit::Mat;
use crate::noise::{fbm, fbm_aniso, hashf, linear_to_srgb, smoothstep};

/// Decal kinds (each one texture shared city-wide).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Decal {
    /// Rising damp, splash-back and traffic grime along the plinth. Tiles
    /// along the wall every [`PLINTH_REPEAT`] m; v spans the band height.
    Plinth,
    /// Streaks running down from sills and balcony slabs. Atlas of
    /// [`STREAK_VARIANTS`] columns (the last one is rust); one column spans
    /// the streak region, v runs from the sill down to the streak ends.
    Streaks,
    /// Soot band and drips under a cornice. Tiles along the wall every
    /// [`SOOT_REPEAT`] m.
    Soot,
}

pub const PLINTH_REPEAT: f32 = 4.0;
pub const SOOT_REPEAT: f32 = 4.0;
pub const STREAK_VARIANTS: u32 = 4;
/// Dirt levels: alpha multiplier of `Mat::Decal(_, level)`.
pub const LEVELS: [f32; 4] = [0.55, 0.72, 0.87, 1.0];

/// Generated decal textures.
pub struct DecalTextures {
    pub plinth: TextureData,
    pub streaks: TextureData,
    pub soot: TextureData,
}

impl DecalTextures {
    pub fn generate() -> Self {
        DecalTextures { plinth: plinth(1024, 256, 7), streaks: streaks(512, 512, 19), soot: soot(1024, 256, 31) }
    }

    pub fn get(&self, d: Decal) -> &TextureData {
        match d {
            Decal::Plinth => &self.plinth,
            Decal::Streaks => &self.streaks,
            Decal::Soot => &self.soot,
        }
    }
}

/// Premultiplied accumulator: layers composite "over" each other.
#[derive(Clone, Copy, Default)]
struct Px {
    c: [f32; 3],
    a: f32,
}

impl Px {
    /// Paints `color` (linear) with coverage `a` over what is there.
    fn over(&mut self, color: [f32; 3], a: f32) {
        let a = a.clamp(0.0, 1.0);
        for k in 0..3 {
            self.c[k] = color[k] * a + self.c[k] * (1.0 - a);
        }
        self.a = a + self.a * (1.0 - a);
    }

    /// Straight-alpha sRGB bytes.
    fn rgba8(&self) -> [u8; 4] {
        let a = self.a.clamp(0.0, 1.0);
        let c = if a > 1e-4 { self.c.map(|v| v / a) } else { [0.05, 0.045, 0.04] };
        let e = |v: f32| (linear_to_srgb(v) * 255.0 + 0.5) as u8;
        [e(c[0]), e(c[1]), e(c[2]), (a * 255.0 + 0.5) as u8]
    }
}

fn to_texture(name: &str, w: u32, h: u32, px: &[Px]) -> TextureData {
    let mut rgba = Vec::with_capacity(px.len() * 4);
    for p in px {
        rgba.extend(p.rgba8());
    }
    TextureData { name: name.into(), width: w, height: h, rgba8: rgba.into(), srgb: true }
}

/// Grime colours (linear albedo): street dirt is a dark warm grey; damp
/// stone a little browner; efflorescence a pale chalky grey.
const GRIME: [f32; 3] = [0.040, 0.036, 0.031];
const DAMP: [f32; 3] = [0.052, 0.043, 0.032];
const SALT: [f32; 3] = [0.50, 0.49, 0.46];
const SOOT: [f32; 3] = [0.028, 0.027, 0.026];
const RUST: [f32; 3] = [0.115, 0.048, 0.018];

/// Rising damp band, `w` px = [`PLINTH_REPEAT`] m along the wall, v = 0 at
/// the top of the band and 1 at the pavement.
pub fn plinth(w: u32, h: u32, seed: u32) -> TextureData {
    let mut px = vec![Px::default(); (w * h) as usize];
    for y in 0..h {
        let v = (y as f32 + 0.5) / h as f32;
        let s = 1.0 - v; // height fraction above the pavement
        for x in 0..w {
            let u = (x as f32 + 0.5) / w as f32;
            let p = &mut px[(y * w + x) as usize];
            // Damp front: wavy, higher in places (capillary rise is patchy).
            let front = 0.42 + 0.38 * fbm(u, 0.0, 7, 4, seed) + 0.06 * (fbm(u, v, 24, 2, seed + 1) - 0.5);
            let mottle = 0.55 + 0.9 * fbm(u, v * 0.25, 18, 4, seed + 2);
            // Traffic grime: strongest at the bottom, everywhere.
            p.over(GRIME, 0.28 * (1.0 - s).powf(1.3) * mottle);
            // Damp zone with vertical wicking structure.
            let wick = 0.75 + 0.5 * fbm_aniso(u, v, 90, 4, 3, seed + 3);
            let damp = smoothstep(front + 0.015, front - 0.18, s) * (0.55 + 0.45 * (1.0 - s / front.max(0.05)).clamp(0.0, 1.0));
            p.over(DAMP, 0.58 * damp * wick * mottle.min(1.2));
            // Salt tide line at the damp front.
            let tide = (-((s - front) / 0.022).powi(2)).exp() * (0.25 + 0.75 * fbm(u, 0.0, 40, 3, seed + 4));
            p.over(SALT, 0.34 * tide);
            // Splash-back: speckled, dark, the bottom ~20 % of the band.
            let splash = (1.0 - s / 0.2).clamp(0.0, 1.0).powf(1.4);
            let speck = smoothstep(0.35, 0.8, fbm(u, v, 160, 2, seed + 5));
            p.over(GRIME, splash * (0.45 + 0.4 * speck));
        }
    }
    to_texture("decal_plinth", w, h, &px)
}

/// Streak atlas: `STREAK_VARIANTS` columns; v = 0 just under the sill.
pub fn streaks(w: u32, h: u32, seed: u32) -> TextureData {
    let mut px = vec![Px::default(); (w * h) as usize];
    let cols = STREAK_VARIANTS;
    let cw = w / cols;
    let gutter = 6.0 / cw as f32;
    for col in 0..cols {
        let mut rng = Rng::new(seed as u64 * 31 + col as u64);
        let rust = col == cols - 1;
        let color = if rust { RUST } else { GRIME };
        // Streak list: (x, width, length, strength, wobble phase).
        let n = 4 + rng.below(6);
        let mut list = Vec::new();
        for _ in 0..n {
            let x = if rng.chance(0.65) {
                if rng.chance(0.5) { rng.range(0.13, 0.22) } else { rng.range(0.78, 0.87) }
            } else {
                rng.range(0.24, 0.76)
            };
            list.push((x, rng.range(0.025, 0.085), rng.range(0.35, 1.0), rng.range(0.35, 0.75), rng.range(0.0, 6.28)));
        }
        let wash = rng.range(0.3, 0.5);
        for y in 0..h {
            let v = (y as f32 + 0.5) / h as f32;
            for lx in 0..cw {
                let u = (lx as f32 + 0.5) / cw as f32;
                if u < gutter || u > 1.0 - gutter {
                    continue;
                }
                let p = &mut px[(y * w + col * cw + lx) as usize];
                let edge = smoothstep(gutter, gutter + 0.06, u) * smoothstep(1.0 - gutter, 1.0 - gutter - 0.06, u);
                // Wash right under the sill, ragged lower edge.
                let reach = 0.08 + 0.14 * fbm(u, col as f32 * 0.37, 12, 3, seed + col);
                let inside = smoothstep(0.08, 0.16, u) * smoothstep(0.92, 0.84, u);
                p.over(color, wash * inside * smoothstep(reach, 0.0, v) * edge);
                for &(x0, sw, len, st, ph) in &list {
                    if v > len {
                        continue;
                    }
                    let t = v / len;
                    let xc = x0 + 0.012 * (v * 9.0 + ph).sin() + 0.02 * (fbm(x0, v, 1, 3, seed + 9 + col) - 0.5);
                    // Drips narrow and fade as they run down.
                    let width = sw * (1.0 - 0.5 * t);
                    let across = (-((u - xc) / width).powi(2)).exp();
                    let along = (1.0 - t).powf(1.1) * (0.6 + 0.4 * fbm(u, v, 6, 3, seed + 17 + col)) * smoothstep(0.0, 0.03, v + 0.02);
                    p.over(color, st * across * along * edge);
                }
                // Fine grain so the drips are not airbrushed.
                let g = hashf((col * cw + lx) as i32, y as i32, seed);
                p.a *= 0.85 + 0.3 * g;
                p.c = p.c.map(|c| c * (0.85 + 0.3 * g));
            }
        }
    }
    to_texture("decal_streaks", w, h, &px)
}

/// Soot under a cornice: a dark sheltered strip with drips; v = 0 right
/// under the cornice, `w` px = [`SOOT_REPEAT`] m.
pub fn soot(w: u32, h: u32, seed: u32) -> TextureData {
    let mut px = vec![Px::default(); (w * h) as usize];
    let mut rng = Rng::new(seed as u64);
    let drips: Vec<(f32, f32, f32, f32)> = (0..22).map(|_| (rng.f(), rng.range(0.005, 0.016), rng.range(0.25, 1.0), rng.range(0.3, 0.65))).collect();
    for y in 0..h {
        let v = (y as f32 + 0.5) / h as f32;
        for x in 0..w {
            let u = (x as f32 + 0.5) / w as f32;
            let p = &mut px[(y * w + x) as usize];
            let edge = 0.16 + 0.18 * fbm(u, 0.0, 9, 4, seed);
            let mottle = 0.6 + 0.8 * fbm(u, v * 0.3, 20, 4, seed + 1);
            p.over(SOOT, 0.62 * smoothstep(edge + 0.08, 0.0, v) * mottle.min(1.3));
            p.over(SOOT, 0.12 * smoothstep(0.9, 0.0, v) * fbm_aniso(u, v, 120, 3, 3, seed + 2));
            for &(x0, sw, len, st) in &drips {
                if v > len {
                    continue;
                }
                let du = crate::noise::wrap_delta(u - x0);
                if du.abs() > 0.05 {
                    continue;
                }
                let t = v / len;
                let across = (-(du / (sw * (1.0 - 0.5 * t))).powi(2)).exp();
                p.over(SOOT, st * across * (1.0 - t).powf(1.2));
            }
        }
    }
    to_texture("decal_soot", w, h, &px)
}

/// One street facade to weather, in facade coordinates (u along the wall
/// from `origin`, v up from the pavement).
pub struct FacadeWear<'a> {
    pub origin: Vec3,
    /// Along the facade.
    pub t: Vec3,
    /// Out of the wall.
    pub n: Vec3,
    pub len: f32,
    /// Ground floor height and facade height (cornice line).
    pub ground: f32,
    pub top: f32,
    pub floor_h: f32,
    pub floors: u32,
    /// Ground floor with rusticated courses standing proud of the wall.
    pub rustic: bool,
    /// Openings `[u0, u1, v0, v1]`.
    pub ground_holes: &'a [[f32; 4]],
    pub upper_holes: &'a [[f32; 4]],
    pub seed: u64,
}

impl FacadeWear<'_> {
    fn p(&self, u: f32, v: f32, out: f32) -> Vec3 {
        self.origin + self.t * u + Vec3::Y * v + self.n * out
    }
}

/// Emits the solid cells of `[u0, u1] × [v0, v1]` (not inside a hole) at
/// `out` in front of the wall; `uv` maps facade coordinates to the decal.
#[allow(clippy::too_many_arguments)]
fn region(prim: &mut Prim, f: &FacadeWear, u0: f32, u1: f32, v0: f32, v1: f32, holes: &[[f32; 4]], out: f32, uv: impl Fn(f32, f32) -> Vec2) {
    let (u0, u1) = (u0.max(0.0), u1.min(f.len));
    if u1 - u0 < 0.05 || v1 - v0 < 0.05 {
        return;
    }
    let inside: Vec<[f32; 4]> = holes.iter().copied().filter(|h| h[1] > u0 && h[0] < u1 && h[3] > v0 && h[2] < v1).collect();
    let mut us = vec![u0, u1];
    let mut vs = vec![v0, v1];
    for h in &inside {
        us.extend([h[0].clamp(u0, u1), h[1].clamp(u0, u1)]);
        vs.extend([h[2].clamp(v0, v1), h[3].clamp(v0, v1)]);
    }
    us.sort_by(f32::total_cmp);
    us.dedup_by(|a, b| (*a - *b).abs() < 1e-4);
    vs.sort_by(f32::total_cmp);
    vs.dedup_by(|a, b| (*a - *b).abs() < 1e-4);
    for j in 0..vs.len() - 1 {
        let (va, vb) = (vs[j], vs[j + 1]);
        let vm = (va + vb) * 0.5;
        let mut i = 0;
        while i < us.len() - 1 {
            let solid = |i: usize| {
                let um = (us[i] + us[i + 1]) * 0.5;
                !inside.iter().any(|h| um > h[0] && um < h[1] && vm > h[2] && vm < h[3])
            };
            if !solid(i) {
                i += 1;
                continue;
            }
            let start = i;
            while i < us.len() - 1 && solid(i) {
                i += 1;
            }
            let (ua, ub) = (us[start], us[i]);
            prim.quad_uv([f.p(ua, va, out), f.p(ub, va, out), f.p(ub, vb, out), f.p(ua, vb, out)], [uv(ua, va), uv(ub, va), uv(ub, vb), uv(ua, vb)]);
        }
    }
}

/// Adds weathering decals for one street facade to `parts`.
pub fn facade(parts: &mut Parts<Mat>, f: &FacadeWear) {
    if f.len < 1.0 {
        return;
    }
    let mut rng = Rng::new(f.seed ^ 0x77ea_7e57);
    // Per-building dirt: some facades were restored recently, most not.
    let lvl = if rng.chance(0.18) { 0 } else { 1 + rng.below(3) as u8 };
    let level = |rng: &mut Rng, base: u8, spread: i32| -> u8 { (base as i32 + rng.below((2 * spread + 1) as usize) as i32 - spread).clamp(0, 3) as u8 };
    let all: Vec<[f32; 4]> = f.ground_holes.iter().chain(f.upper_holes).copied().collect();
    let base_out = if f.rustic { 0.032 } else { 0.006 };

    // Plinth: damp and street grime, cut around shops, doors and windows.
    let band = rng.range(0.9, 2.0).min(f.ground - 0.5).max(0.4);
    let uoff = rng.range(0.0, 40.0);
    let m = Mat::Decal(Decal::Plinth, level(&mut rng, lvl.max(1), 1));
    region(parts.get(m), f, 0.0, f.len, 0.0, band, f.ground_holes, base_out, |u, v| Vec2::new((u + uoff) / PLINTH_REPEAT, (band - v) / band));

    // Soot under the main cornice and, lighter, under the ground cornice.
    let hs = rng.range(0.6, 1.5);
    let soff = rng.range(0.0, 40.0);
    let m = Mat::Decal(Decal::Soot, level(&mut rng, lvl, 1));
    region(parts.get(m), f, 0.0, f.len, f.top - hs, f.top, &all, 0.006, |u, v| Vec2::new((u + soff) / SOOT_REPEAT, (f.top - v) / hs));
    if f.ground > 3.0 {
        let g0 = f.ground - 0.32;
        let hg = rng.range(0.35, 0.8);
        let m = Mat::Decal(Decal::Soot, level(&mut rng, lvl.saturating_sub(1), 0));
        region(parts.get(m), f, 0.0, f.len, g0 - hg, g0, &all, base_out, |u, v| Vec2::new((u + soff * 1.7) / SOOT_REPEAT, (g0 - v) / hg));
    }

    // Streaks under every upper opening (sills and balcony slabs).
    let floor_levels: Vec<f32> = (0..f.floors).map(|k| f.ground + k as f32 * f.floor_h).collect();
    let cols = STREAK_VARIANTS as f32;
    for h in f.upper_holes {
        if !rng.chance(0.85) {
            continue;
        }
        let door = floor_levels.iter().any(|y| (h[2] - y).abs() < 0.03);
        let (top, margin) = if door { (h[2] - 0.17, 0.42) } else { (h[2] - 0.1, 0.26) };
        let len = rng.range(0.5, 1.5) * (0.8 + 0.25 * lvl as f32);
        let bottom = (top - len).max(0.3);
        if top - bottom < 0.15 {
            continue;
        }
        let variant = if door && rng.chance(0.35) { STREAK_VARIANTS - 1 } else { rng.below(STREAK_VARIANTS as usize - 1) as u32 };
        let (u0, u1) = (h[0] - margin, h[1] + margin);
        let c0 = variant as f32 / cols;
        let m = Mat::Decal(Decal::Streaks, level(&mut rng, lvl, 1));
        let full = top - (top - len);
        region(parts.get(m), f, u0, u1, bottom, top, &all, 0.007, |u, v| Vec2::new(c0 + (u - u0) / (u1 - u0) / cols, (top - v) / full));
    }
    // Ground floor windows drip onto the plinth too.
    for h in f.ground_holes {
        if h[2] < 0.3 || !rng.chance(0.7) {
            continue;
        }
        let top = h[2] - 0.1;
        let len = rng.range(0.3, 0.8);
        let bottom = (top - len).max(0.05);
        let variant = rng.below(STREAK_VARIANTS as usize - 1) as u32;
        let (u0, u1) = (h[0] - 0.26, h[1] + 0.26);
        let c0 = variant as f32 / cols;
        let m = Mat::Decal(Decal::Streaks, level(&mut rng, lvl, 1));
        region(parts.get(m), f, u0, u1, bottom, top, &all, base_out + 0.001, |u, v| Vec2::new(c0 + (u - u0) / (u1 - u0) / cols, (top - v) / len));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alpha(t: &TextureData, x: u32, y: u32) -> u8 {
        t.rgba8[((y * t.width + x) * 4 + 3) as usize]
    }

    #[test]
    fn plinth_is_dirtier_at_the_bottom() {
        let t = plinth(128, 64, 3);
        let row = |y: u32| (0..128).map(|x| alpha(&t, x, y) as u32).sum::<u32>();
        assert!(row(62) > row(2) * 3, "bottom {} top {}", row(62), row(2));
        // Dark: the bottom rows are much darker than mid grey.
        let i = ((62 * 128 + 5) * 4) as usize;
        assert!(t.rgba8[i] < 110, "{:?}", &t.rgba8[i..i + 4]);
    }

    #[test]
    fn streaks_fade_downwards_and_rust_is_warm() {
        let t = streaks(256, 128, 5);
        let cw = 256 / STREAK_VARIANTS;
        let row = |y: u32| (0..256).map(|x| alpha(&t, x, y) as u32).sum::<u32>();
        assert!(row(4) > row(124), "{} {}", row(4), row(124));
        // Gutters stay transparent so mips do not bleed between columns.
        assert_eq!(alpha(&t, cw, 10), 0);
        // Rust column: red channel above blue where there is coverage.
        let rust = (0..cw).flat_map(|x| (0..40).map(move |y| (x + cw * (STREAK_VARIANTS - 1), y))).filter(|&(x, y)| alpha(&t, x, y) > 60).map(|(x, y)| {
            let i = ((y * 256 + x) * 4) as usize;
            t.rgba8[i] as i32 - t.rgba8[i + 2] as i32
        });
        let v: Vec<i32> = rust.collect();
        assert!(!v.is_empty() && v.iter().sum::<i32>() / v.len() as i32 > 20);
    }

    #[test]
    fn facade_decals_avoid_openings() {
        let holes_g = [[2.0, 5.0, 0.0, 3.2]];
        let holes_u = [[1.0, 2.2, 4.2, 6.5], [6.0, 7.2, 5.1, 6.6]];
        let f = FacadeWear {
            origin: Vec3::ZERO,
            t: Vec3::X,
            n: Vec3::Z,
            len: 10.0,
            ground: 4.2,
            top: 10.6,
            floor_h: 3.2,
            floors: 2,
            rustic: false,
            ground_holes: &holes_g,
            upper_holes: &holes_u,
            seed: 9,
        };
        let mut parts: Parts<Mat> = Parts::new();
        facade(&mut parts, &f);
        assert!(parts.prims.contains_key(&Mat::Decal(Decal::Plinth, 1)) || parts.prims.keys().any(|k| matches!(k, Mat::Decal(Decal::Plinth, _))));
        for (k, p) in &parts.prims {
            assert!(matches!(k, Mat::Decal(..)));
            for t in p.indices.chunks_exact(3) {
                let c = (p.positions[t[0] as usize] + p.positions[t[1] as usize] + p.positions[t[2] as usize]) / 3.0;
                assert!(c.z > 0.0 && c.z < 0.05, "decal off the wall {c:?}");
                for h in holes_g.iter().chain(&holes_u) {
                    let inside = c.x > h[0] + 1e-3 && c.x < h[1] - 1e-3 && c.y > h[2] + 1e-3 && c.y < h[3] - 1e-3;
                    assert!(!inside, "decal triangle inside an opening {c:?} {h:?}");
                }
                // Faces the street.
                let n = (p.positions[t[1] as usize] - p.positions[t[0] as usize]).cross(p.positions[t[2] as usize] - p.positions[t[0] as usize]);
                assert!(n.z > 0.0);
            }
        }
    }

    #[test]
    fn decal_materials_blend_with_texture_alpha() {
        let kit = crate::kit::Kit::load(None);
        let mut a = crate::kit::Assembler::new(&kit, "t");
        let mi = a.material(Mat::Decal(Decal::Plinth, 3));
        let s = a.finish();
        let m = &s.materials[mi];
        assert_eq!(m.alpha, forja_assets::AlphaMode::Blend);
        let t = &s.textures[m.base_color_tex.unwrap()];
        let mean = t.rgba8.chunks_exact(4).map(|p| p[3] as f32).sum::<f32>() / (t.width * t.height) as f32;
        assert!(mean > 20.0, "alpha mean {mean} of {}", t.name);
    }

    #[test]
    #[ignore]
    fn dump_decals() {
        let d = DecalTextures::generate();
        let dir = std::env::var("DUMP_DIR").unwrap();
        for (n, t) in [("plinth", &d.plinth), ("streaks", &d.streaks), ("soot", &d.soot)] {
            std::fs::write(format!("{dir}/{n}_{}x{}.rgba", t.width, t.height), &t.rgba8[..]).unwrap();
        }
    }
}
