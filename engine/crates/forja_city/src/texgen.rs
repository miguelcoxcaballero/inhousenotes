//! Procedural textures: glazed tiles (azulejos), Spanish traffic sign faces,
//! shop signs, graffiti, awnings and window interiors. Everything is drawn
//! with anti-aliased signed-distance shapes and a built-in stroke font.

use std::sync::Arc;

use forja_assets::TextureData;
use forja_math::{Vec2, Vec3};

use crate::geom::Rng;

/// RGBA float canvas in sRGB-encoded values, origin top-left, y down.
#[derive(Clone)]
pub struct Canvas {
    pub w: u32,
    pub h: u32,
    pub px: Vec<[f32; 4]>,
}

pub fn rgb(hex: u32) -> [f32; 4] {
    [((hex >> 16) & 255) as f32 / 255.0, ((hex >> 8) & 255) as f32 / 255.0, (hex & 255) as f32 / 255.0, 1.0]
}

impl Canvas {
    pub fn new(w: u32, h: u32, fill: [f32; 4]) -> Self {
        Self { w, h, px: vec![fill; (w * h) as usize] }
    }

    pub fn from_texture(t: &TextureData) -> Self {
        let px = t.rgba8.chunks_exact(4).map(|c| [c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0, c[3] as f32 / 255.0]).collect();
        Self { w: t.width, h: t.height, px }
    }

    pub fn blend(&mut self, x: u32, y: u32, c: [f32; 4], cover: f32) {
        let a = (c[3] * cover).clamp(0.0, 1.0);
        if a <= 0.0 {
            return;
        }
        let p = &mut self.px[(y * self.w + x) as usize];
        for k in 0..3 {
            p[k] = p[k] * (1.0 - a) + c[k] * a;
        }
        p[3] = p[3] + (1.0 - p[3]) * a;
    }

    /// Fills where `sdf(p) < 0` (pixel units), anti-aliased, within
    /// `[min, max]`.
    pub fn fill(&mut self, min: Vec2, max: Vec2, c: [f32; 4], sdf: impl Fn(Vec2) -> f32) {
        let x0 = (min.x.floor() as i64 - 1).max(0) as u32;
        let y0 = (min.y.floor() as i64 - 1).max(0) as u32;
        let x1 = ((max.x.ceil() as i64 + 1).max(0) as u32).min(self.w);
        let y1 = ((max.y.ceil() as i64 + 1).max(0) as u32).min(self.h);
        for y in y0..y1 {
            for x in x0..x1 {
                let d = sdf(Vec2::new(x as f32 + 0.5, y as f32 + 0.5));
                let cover = (0.5 - d).clamp(0.0, 1.0);
                if cover > 0.0 {
                    self.blend(x, y, c, cover);
                }
            }
        }
    }

    pub fn rect(&mut self, min: Vec2, max: Vec2, c: [f32; 4]) {
        let (ctr, half) = ((min + max) * 0.5, (max - min) * 0.5);
        self.fill(min, max, c, |p| sd_box(p - ctr, half));
    }

    pub fn circle(&mut self, ctr: Vec2, r: f32, c: [f32; 4]) {
        self.fill(ctr - r, ctr + r, c, |p| (p - ctr).length() - r);
    }

    pub fn ring(&mut self, ctr: Vec2, r: f32, w: f32, c: [f32; 4]) {
        self.fill(ctr - r - w, ctr + r + w, c, |p| ((p - ctr).length() - r).abs() - w * 0.5);
    }

    /// Convex polygon (any winding).
    pub fn poly(&mut self, pts: &[Vec2], c: [f32; 4]) {
        let (mut mn, mut mx) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
        for p in pts {
            mn = mn.min(*p);
            mx = mx.max(*p);
        }
        let area: f32 = (0..pts.len()).map(|i| pts[i].perp_dot(pts[(i + 1) % pts.len()])).sum();
        let s = if area >= 0.0 { 1.0 } else { -1.0 };
        let pts = pts.to_vec();
        self.fill(mn, mx, c, move |p| {
            let mut d = f32::MIN;
            for i in 0..pts.len() {
                let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
                let e = (b - a).normalize_or(Vec2::X);
                // Signed distance to the edge's half plane (positive outside).
                d = d.max(-e.perp_dot(p - a) * s);
            }
            d
        });
    }

    pub fn line(&mut self, a: Vec2, b: Vec2, w: f32, c: [f32; 4]) {
        let (mn, mx) = (a.min(b) - w, a.max(b) + w);
        self.fill(mn, mx, c, |p| sd_segment(p, a, b) - w * 0.5);
    }

    /// Text in the stroke font. `pos` is the bottom-left of the first glyph,
    /// `size` the cap height in pixels.
    pub fn text(&mut self, s: &str, pos: Vec2, size: f32, weight: f32, c: [f32; 4]) {
        let k = size / 6.0;
        let mut x = pos.x;
        for ch in s.chars() {
            let strokes = glyph(ch);
            let segs: Vec<(Vec2, Vec2)> = strokes
                .iter()
                .flat_map(|poly| poly.windows(2).map(|w| (w[0], w[1])).collect::<Vec<_>>())
                .map(|(a, b)| (Vec2::new(x + a.x * k, pos.y - a.y * k), Vec2::new(x + b.x * k, pos.y - b.y * k)))
                .collect();
            if !segs.is_empty() {
                let w = weight * k;
                let min = Vec2::new(x - w, pos.y - 8.0 * k - w);
                let max = Vec2::new(x + 4.0 * k + w, pos.y + w);
                self.fill(min, max, c, |p| segs.iter().map(|(a, b)| sd_segment(p, *a, *b)).fold(f32::MAX, f32::min) - w * 0.5);
            }
            x += advance(ch) * k;
        }
    }

    pub fn text_width(s: &str, size: f32) -> f32 {
        let k = size / 6.0;
        s.chars().map(|c| advance(c) * k).sum::<f32>() - 1.5 * k
    }

    pub fn text_centered(&mut self, s: &str, center: Vec2, size: f32, weight: f32, c: [f32; 4]) {
        let w = Self::text_width(s, size);
        self.text(s, Vec2::new(center.x - w * 0.5, center.y + size * 0.5), size, weight, c);
    }

    pub fn to_texture(&self, name: &str, srgb: bool) -> TextureData {
        let mut rgba = Vec::with_capacity(self.px.len() * 4);
        for p in &self.px {
            rgba.extend(p.iter().map(|v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8));
        }
        TextureData { name: name.into(), width: self.w, height: self.h, rgba8: Arc::from(rgba), srgb }
    }
}

pub fn sd_box(p: Vec2, half: Vec2) -> f32 {
    let d = p.abs() - half;
    d.max(Vec2::ZERO).length() + d.x.max(d.y).min(0.0)
}

pub fn sd_segment(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let (pa, ba) = (p - a, b - a);
    let h = (pa.dot(ba) / ba.dot(ba).max(1e-9)).clamp(0.0, 1.0);
    (pa - ba * h).length()
}

fn advance(c: char) -> f32 {
    match c {
        ' ' => 3.5,
        '.' | ',' | '\'' | '·' | ':' => 2.5,
        'I' | '1' => 5.0,
        _ => 5.5,
    }
}

/// Stroke font on a 4 × 6 grid (y up). Accented capitals add a mark above.
fn glyph(c: char) -> Vec<Vec<Vec2>> {
    let p = |pts: &[(f32, f32)]| pts.iter().map(|&(x, y)| Vec2::new(x, y)).collect::<Vec<_>>();
    let o = || p(&[(1., 0.), (0., 1.), (0., 5.), (1., 6.), (3., 6.), (4., 5.), (4., 1.), (3., 0.), (1., 0.)]);
    let pp = || p(&[(0., 0.), (0., 6.), (3., 6.), (4., 5.), (4., 4.), (3., 3.), (0., 3.)]);
    let accent = || p(&[(1.6, 6.9), (2.8, 7.9)]);
    let base = |c: char| glyph(c);
    match c {
        'A' => vec![p(&[(0., 0.), (0., 4.), (2., 6.), (4., 4.), (4., 0.)]), p(&[(0., 3.), (4., 3.)])],
        'B' => vec![p(&[(0., 0.), (0., 6.), (3., 6.), (4., 5.), (4., 4.), (3., 3.), (0., 3.)]), p(&[(3., 3.), (4., 2.), (4., 1.), (3., 0.), (0., 0.)])],
        'C' => vec![p(&[(4., 5.), (3., 6.), (1., 6.), (0., 5.), (0., 1.), (1., 0.), (3., 0.), (4., 1.)])],
        'D' => vec![p(&[(0., 0.), (0., 6.), (2., 6.), (4., 4.), (4., 2.), (2., 0.), (0., 0.)])],
        'E' => vec![p(&[(4., 6.), (0., 6.), (0., 0.), (4., 0.)]), p(&[(0., 3.), (3., 3.)])],
        'F' => vec![p(&[(4., 6.), (0., 6.), (0., 0.)]), p(&[(0., 3.), (3., 3.)])],
        'G' => vec![p(&[(4., 5.), (3., 6.), (1., 6.), (0., 5.), (0., 1.), (1., 0.), (3., 0.), (4., 1.), (4., 3.), (2., 3.)])],
        'H' => vec![p(&[(0., 0.), (0., 6.)]), p(&[(4., 0.), (4., 6.)]), p(&[(0., 3.), (4., 3.)])],
        'I' => vec![p(&[(1., 6.), (3., 6.)]), p(&[(2., 6.), (2., 0.)]), p(&[(1., 0.), (3., 0.)])],
        'J' => vec![p(&[(4., 6.), (4., 1.), (3., 0.), (1., 0.), (0., 1.)])],
        'K' => vec![p(&[(0., 0.), (0., 6.)]), p(&[(4., 6.), (0., 2.)]), p(&[(1.3, 3.3), (4., 0.)])],
        'L' => vec![p(&[(0., 6.), (0., 0.), (4., 0.)])],
        'M' => vec![p(&[(0., 0.), (0., 6.), (2., 3.), (4., 6.), (4., 0.)])],
        'N' => vec![p(&[(0., 0.), (0., 6.), (4., 0.), (4., 6.)])],
        'O' | '0' => vec![o()],
        'P' => vec![pp()],
        'Q' => vec![o(), p(&[(2.5, 1.5), (4., 0.)])],
        'R' => vec![pp(), p(&[(2., 3.), (4., 0.)])],
        'S' => vec![p(&[(4., 5.), (3., 6.), (1., 6.), (0., 5.), (0., 4.), (1., 3.), (3., 3.), (4., 2.), (4., 1.), (3., 0.), (1., 0.), (0., 1.)])],
        'T' => vec![p(&[(0., 6.), (4., 6.)]), p(&[(2., 6.), (2., 0.)])],
        'U' => vec![p(&[(0., 6.), (0., 1.), (1., 0.), (3., 0.), (4., 1.), (4., 6.)])],
        'V' => vec![p(&[(0., 6.), (2., 0.), (4., 6.)])],
        'W' => vec![p(&[(0., 6.), (1., 0.), (2., 4.), (3., 0.), (4., 6.)])],
        'X' => vec![p(&[(0., 0.), (4., 6.)]), p(&[(0., 6.), (4., 0.)])],
        'Y' => vec![p(&[(0., 6.), (2., 3.), (4., 6.)]), p(&[(2., 3.), (2., 0.)])],
        'Z' => vec![p(&[(0., 6.), (4., 6.), (0., 0.), (4., 0.)])],
        'Ñ' => {
            let mut g = base('N');
            g.push(p(&[(0.8, 7.2), (1.6, 7.8), (2.4, 7.2), (3.2, 7.8)]));
            g
        }
        'Á' | 'É' | 'Í' | 'Ó' | 'Ú' => {
            let plain = match c {
                'Á' => 'A',
                'É' => 'E',
                'Í' => 'I',
                'Ó' => 'O',
                _ => 'U',
            };
            let mut g = base(plain);
            g.push(accent());
            g
        }
        'À' | 'È' | 'Ò' => {
            let mut g = base(match c {
                'À' => 'A',
                'È' => 'E',
                _ => 'O',
            });
            g.push(p(&[(1.4, 7.9), (2.6, 6.9)]));
            g
        }
        'Ç' => {
            let mut g = base('C');
            g.push(p(&[(2., 0.), (2.2, -0.8), (1.4, -1.4)]));
            g
        }
        '·' => vec![p(&[(1., 3.), (1., 3.3)])],
        ':' => vec![p(&[(1., 1.), (1., 1.3)]), p(&[(1., 4.), (1., 4.3)])],
        '1' => vec![p(&[(1., 5.), (2., 6.), (2., 0.)]), p(&[(1., 0.), (3., 0.)])],
        '2' => vec![p(&[(0., 5.), (1., 6.), (3., 6.), (4., 5.), (4., 4.), (0., 0.), (4., 0.)])],
        '3' => vec![p(&[(0., 5.), (1., 6.), (3., 6.), (4., 5.), (4., 4.), (3., 3.), (4., 2.), (4., 1.), (3., 0.), (1., 0.), (0., 1.)]), p(&[(1.5, 3.), (3., 3.)])],
        '4' => vec![p(&[(3., 0.), (3., 6.), (0., 2.), (4., 2.)])],
        '5' => vec![p(&[(4., 6.), (0., 6.), (0., 3.5), (3., 3.5), (4., 2.5), (4., 1.), (3., 0.), (0., 0.)])],
        '6' => vec![p(&[(4., 5.), (3., 6.), (1., 6.), (0., 5.), (0., 1.), (1., 0.), (3., 0.), (4., 1.), (4., 2.), (3., 3.), (0., 3.)])],
        '7' => vec![p(&[(0., 6.), (4., 6.), (1.5, 0.)])],
        '8' => vec![p(&[(1., 3.), (0., 4.), (0., 5.), (1., 6.), (3., 6.), (4., 5.), (4., 4.), (3., 3.), (1., 3.), (0., 2.), (0., 1.), (1., 0.), (3., 0.), (4., 1.), (4., 2.), (3., 3.)])],
        '9' => vec![p(&[(4., 3.), (1., 3.), (0., 4.), (0., 5.), (1., 6.), (3., 6.), (4., 5.), (4., 1.), (3., 0.), (1., 0.)])],
        '.' => vec![p(&[(1., 0.), (1., 0.3)])],
        ',' => vec![p(&[(1., 0.3), (0.5, -1.)])],
        '\'' => vec![p(&[(1., 6.), (1., 4.5)])],
        '-' => vec![p(&[(0.8, 3.), (3.2, 3.)])],
        '/' => vec![p(&[(0., 0.), (4., 6.)])],
        '&' => vec![p(&[(4., 0.), (0.5, 4.5), (1., 6.), (2.5, 6.), (3., 4.5), (0., 2.), (0., 1.), (1., 0.), (2.5, 0.), (4., 2.)])],
        _ => vec![],
    }
}

// ------------------------------------------------------------- material sets

/// Color, normal and ORM maps of a generated material.
#[derive(Clone)]
pub struct GenSet {
    pub color: TextureData,
    pub normal: Option<TextureData>,
    pub orm: Option<TextureData>,
}

/// Height field to a tangent-space normal map (OpenGL convention).
fn normal_from_height(h: &[f32], w: u32, hh: u32, strength: f32) -> TextureData {
    let at = |x: i64, y: i64| h[((y.rem_euclid(hh as i64)) * w as i64 + x.rem_euclid(w as i64)) as usize];
    let mut rgba = Vec::with_capacity((w * hh * 4) as usize);
    for y in 0..hh as i64 {
        for x in 0..w as i64 {
            let dx = (at(x + 1, y) - at(x - 1, y)) * strength;
            let dy = (at(x, y + 1) - at(x, y - 1)) * strength;
            let n = Vec3::new(-dx, dy, 1.0).normalize();
            rgba.extend([((n.x * 0.5 + 0.5) * 255.0) as u8, ((n.y * 0.5 + 0.5) * 255.0) as u8, ((n.z * 0.5 + 0.5) * 255.0) as u8, 255]);
        }
    }
    TextureData { name: "normal".into(), width: w, height: hh, rgba8: Arc::from(rgba), srgb: false }
}

fn orm_from(rough: &[f32], w: u32, h: u32) -> TextureData {
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for r in rough {
        rgba.extend([255u8, (r.clamp(0.0, 1.0) * 255.0) as u8, 0, 255]);
    }
    TextureData { name: "orm".into(), width: w, height: h, rgba8: Arc::from(rgba), srgb: false }
}

/// Glazed tile patterns seen on Valencian facades.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TilePattern {
    /// Green and white checkerboard (Cabanyal).
    CheckerGreen,
    /// Blue and white checkerboard.
    CheckerBlue,
    /// Blue diamonds on white.
    DiamondBlue,
    /// Painted frieze: blue and ochre floral band.
    Frieze,
    /// Plain glossy white tiles.
    White,
}

/// One texture repeat = 4 × 4 tiles of 15 cm (60 cm).
pub fn tiles(pattern: TilePattern, seed: u64) -> GenSet {
    let n = 512u32;
    let tiles = 4u32;
    let tpx = n / tiles;
    let grout = 3.0f32;
    let mut rng = Rng::new(seed);
    let mut c = Canvas::new(n, n, rgb(0xd8d2c4));
    let mut height = vec![0.0f32; (n * n) as usize];
    let mut rough = vec![0.85f32; (n * n) as usize];
    let white = rgb(0xf4f1e8);
    let green = rgb(0x1f6e4a);
    let blue = rgb(0x1d4f9a);
    let ochre = rgb(0xd69a2d);
    for ty in 0..tiles {
        for tx in 0..tiles {
            let o = Vec2::new((tx * tpx) as f32, (ty * tpx) as f32);
            let s = tpx as f32;
            let jitter = rng.range(-0.04, 0.04);
            let tint = |col: [f32; 4]| [col[0] * (1.0 + jitter), col[1] * (1.0 + jitter), col[2] * (1.0 + jitter), 1.0];
            let inner_min = o + grout;
            let inner_max = o + s - grout;
            let base = match pattern {
                TilePattern::CheckerGreen => {
                    if (tx + ty) % 2 == 0 { green } else { white }
                }
                TilePattern::CheckerBlue => {
                    if (tx + ty) % 2 == 0 { blue } else { white }
                }
                _ => white,
            };
            c.rect(inner_min, inner_max, tint(base));
            let ctr = o + s * 0.5;
            match pattern {
                TilePattern::DiamondBlue => {
                    let r = s * 0.32;
                    c.poly(&[ctr + Vec2::new(0.0, -r), ctr + Vec2::new(r, 0.0), ctr + Vec2::new(0.0, r), ctr + Vec2::new(-r, 0.0)], tint(blue));
                    c.poly(&[ctr + Vec2::new(0.0, -r * 0.45), ctr + Vec2::new(r * 0.45, 0.0), ctr + Vec2::new(0.0, r * 0.45), ctr + Vec2::new(-r * 0.45, 0.0)], tint(white));
                }
                TilePattern::Frieze => {
                    // Quarter circles meeting at tile corners form rosettes.
                    for k in 0..4 {
                        let corner = o + Vec2::new(if k & 1 == 0 { grout } else { s - grout }, if k & 2 == 0 { grout } else { s - grout });
                        c.fill(o, o + s, tint(blue), |p| ((p - corner).length() - s * 0.42).abs() - s * 0.05);
                    }
                    c.circle(ctr, s * 0.16, tint(ochre));
                    c.ring(ctr, s * 0.24, s * 0.04, tint(blue));
                }
                _ => {}
            }
            // Glaze: pillowed tile surface, glossy; grout recessed and rough.
            for y in (ty * tpx)..((ty + 1) * tpx) {
                for x in (tx * tpx)..((tx + 1) * tpx) {
                    let p = Vec2::new(x as f32 + 0.5, y as f32 + 0.5) - ctr;
                    let d = -sd_box(p, Vec2::splat(s * 0.5 - grout));
                    let i = (y * n + x) as usize;
                    if d > 0.0 {
                        height[i] = (d / 6.0).min(1.0);
                        rough[i] = 0.12 + rng.f() * 0.05;
                    } else {
                        height[i] = 0.0;
                        rough[i] = 0.9;
                    }
                }
            }
        }
    }
    GenSet { color: c.to_texture(&format!("tiles_{pattern:?}"), true), normal: Some(normal_from_height(&height, n, n, 2.0)), orm: Some(orm_from(&rough, n, n)) }
}

// ------------------------------------------------------------- traffic signs

/// Spanish road signs (Reglamento General de Circulación catalogue codes).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SignKind {
    /// R-2 STOP.
    Stop,
    /// R-1 ceda el paso.
    Yield,
    /// R-301 velocidad máxima 50.
    Speed50,
    /// R-301 velocidad máxima 30.
    Speed30,
    /// S-13 paso de peatones.
    Crossing,
    /// R-101 entrada prohibida.
    NoEntry,
    /// R-400 sentido obligatorio (straight ahead).
    AheadOnly,
    /// R-308 estacionamiento prohibido.
    NoParking,
    /// S-17 estacionamiento (P).
    Parking,
}

impl SignKind {
    pub const ALL: [SignKind; 9] = [SignKind::Stop, SignKind::Yield, SignKind::Speed50, SignKind::Speed30, SignKind::Crossing, SignKind::NoEntry, SignKind::AheadOnly, SignKind::NoParking, SignKind::Parking];

    /// Plate outline for the 3D model.
    pub fn shape(self) -> SignShape {
        match self {
            SignKind::Stop => SignShape::Octagon,
            SignKind::Yield => SignShape::TriangleDown,
            SignKind::Crossing | SignKind::Parking => SignShape::Square,
            _ => SignShape::Circle,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignShape {
    Circle,
    Octagon,
    TriangleDown,
    Square,
}

/// 256² sign face, filling the texture square.
pub fn sign_face(kind: SignKind) -> TextureData {
    let n = 256.0;
    let mut c = Canvas::new(256, 256, [0.75, 0.75, 0.75, 1.0]);
    let ctr = Vec2::splat(n * 0.5);
    let red = rgb(0xc8102e);
    let white = rgb(0xf7f7f5);
    let blue = rgb(0x0052a5);
    let black = rgb(0x141414);
    match kind {
        SignKind::Stop => {
            let oct = |r: f32| (0..8).map(|i| ctr + Vec2::from_angle(std::f32::consts::PI / 8.0 + i as f32 * std::f32::consts::FRAC_PI_4) * r).collect::<Vec<_>>();
            c.poly(&oct(128.0), white);
            c.poly(&oct(118.0), red);
            c.text_centered("STOP", ctr, 58.0, 1.25, white);
        }
        SignKind::Yield => {
            let tri = |r: f32| vec![ctr + Vec2::new(-r, -r * 0.62), ctr + Vec2::new(r, -r * 0.62), ctr + Vec2::new(0.0, r * 1.05)];
            c.poly(&tri(128.0), red);
            c.poly(&tri(88.0), white);
        }
        SignKind::Speed50 | SignKind::Speed30 => {
            c.circle(ctr, 127.0, red);
            c.circle(ctr, 100.0, white);
            c.text_centered(if kind == SignKind::Speed50 { "50" } else { "30" }, ctr, 96.0, 1.3, black);
        }
        SignKind::NoEntry => {
            c.circle(ctr, 127.0, red);
            c.rect(Vec2::new(40.0, 108.0), Vec2::new(216.0, 148.0), white);
        }
        SignKind::AheadOnly => {
            c.circle(ctr, 127.0, white);
            c.circle(ctr, 121.0, blue);
            c.rect(Vec2::new(116.0, 100.0), Vec2::new(140.0, 205.0), white);
            c.poly(&[Vec2::new(78.0, 110.0), Vec2::new(128.0, 48.0), Vec2::new(178.0, 110.0)], white);
        }
        SignKind::NoParking => {
            c.circle(ctr, 127.0, red);
            c.circle(ctr, 104.0, blue);
            c.line(Vec2::new(58.0, 58.0), Vec2::new(198.0, 198.0), 24.0, red);
        }
        SignKind::Crossing => {
            c.rect(Vec2::ZERO, Vec2::splat(n), white);
            c.rect(Vec2::splat(8.0), Vec2::splat(n - 8.0), blue);
            c.poly(&[Vec2::new(128.0, 30.0), Vec2::new(226.0, 214.0), Vec2::new(30.0, 214.0)], white);
            // Pedestrian on the zebra.
            c.circle(Vec2::new(134.0, 92.0), 11.0, black);
            c.line(Vec2::new(130.0, 106.0), Vec2::new(122.0, 150.0), 11.0, black);
            c.line(Vec2::new(122.0, 150.0), Vec2::new(104.0, 186.0), 9.0, black);
            c.line(Vec2::new(122.0, 150.0), Vec2::new(146.0, 184.0), 9.0, black);
            c.line(Vec2::new(128.0, 116.0), Vec2::new(150.0, 136.0), 8.0, black);
            c.line(Vec2::new(128.0, 116.0), Vec2::new(108.0, 134.0), 8.0, black);
            for k in 0..4 {
                let x = 70.0 + k as f32 * 32.0;
                c.rect(Vec2::new(x, 194.0), Vec2::new(x + 18.0, 204.0), black);
            }
        }
        SignKind::Parking => {
            c.rect(Vec2::ZERO, Vec2::splat(n), white);
            c.rect(Vec2::splat(8.0), Vec2::splat(n - 8.0), blue);
            c.text_centered("P", ctr, 150.0, 1.6, white);
        }
    }
    c.to_texture(&format!("sign_{kind:?}"), true)
}

/// Pedestrian signal lens: standing red man or walking green man.
pub fn pedestrian_lens(walk: bool) -> TextureData {
    let mut c = Canvas::new(128, 128, rgb(0x050505));
    let col = if walk { rgb(0x34ff8a) } else { rgb(0xff2a1a) };
    c.circle(Vec2::new(64.0, 22.0), 11.0, col);
    if walk {
        c.line(Vec2::new(62.0, 36.0), Vec2::new(56.0, 74.0), 16.0, col);
        c.line(Vec2::new(56.0, 74.0), Vec2::new(38.0, 112.0), 11.0, col);
        c.line(Vec2::new(56.0, 74.0), Vec2::new(80.0, 110.0), 11.0, col);
        c.line(Vec2::new(60.0, 44.0), Vec2::new(84.0, 66.0), 9.0, col);
        c.line(Vec2::new(60.0, 44.0), Vec2::new(40.0, 64.0), 9.0, col);
    } else {
        c.line(Vec2::new(64.0, 36.0), Vec2::new(64.0, 76.0), 22.0, col);
        c.line(Vec2::new(56.0, 76.0), Vec2::new(56.0, 116.0), 10.0, col);
        c.line(Vec2::new(72.0, 76.0), Vec2::new(72.0, 116.0), 10.0, col);
        c.line(Vec2::new(48.0, 40.0), Vec2::new(46.0, 76.0), 8.0, col);
        c.line(Vec2::new(80.0, 40.0), Vec2::new(82.0, 76.0), 8.0, col);
    }
    c.to_texture(if walk { "ped_walk" } else { "ped_stop" }, true)
}

// ------------------------------------------------------------- street dressing

/// Shop names for the sign atlas (16 rows).
pub const SHOPS: [(&str, u32, u32); 16] = [
    ("FARMACIA", 0x0b7a3e, 0xffffff),
    ("CAFÉ BAR", 0x6b1d1d, 0xf2d48a),
    ("PANADERÍA", 0x8a5a2b, 0xfff4dc),
    ("HORCHATERÍA", 0x1d4f9a, 0xffffff),
    ("ESTANCO", 0x7a1220, 0xf5c518),
    ("FRUTERÍA", 0x2e7d32, 0xfff59d),
    ("LIBRERÍA", 0x263238, 0xe0e0e0),
    ("ÓPTICA", 0xeceff1, 0x0d47a1),
    ("ULTRAMARINOS", 0x3e2723, 0xffcc80),
    ("PELUQUERÍA", 0x111111, 0xf8bbd0),
    ("ZAPATERÍA", 0x4e342e, 0xffffff),
    ("MERCERÍA", 0x5d4037, 0xffe0b2),
    ("BANCO", 0x0d2c54, 0xffffff),
    ("TABERNA", 0x1b1b1b, 0xd4af37),
    ("BODEGA", 0x5b0e1a, 0xf1e3c6),
    ("FLORISTERÍA", 0xf1f8e9, 0x33691e),
];

/// 1024 × 1024 atlas: row `i` (64 px high) is the sign of `SHOPS[i]`.
pub fn shop_sign_atlas() -> TextureData {
    let mut c = Canvas::new(1024, 1024, rgb(0x202020));
    for (i, (name, bg, fg)) in SHOPS.iter().enumerate() {
        let y0 = i as f32 * 64.0;
        c.rect(Vec2::new(0.0, y0), Vec2::new(1024.0, y0 + 64.0), rgb(*bg));
        c.rect(Vec2::new(6.0, y0 + 5.0), Vec2::new(1018.0, y0 + 7.0), rgb(*fg));
        c.rect(Vec2::new(6.0, y0 + 57.0), Vec2::new(1018.0, y0 + 59.0), rgb(*fg));
        let size = 30.0f32.min(900.0 / (name.chars().count() as f32 * 5.5 / 6.0));
        c.text_centered(name, Vec2::new(512.0, y0 + 30.0), size, 1.1, rgb(*fg));
        if *name == "FARMACIA" {
            // Green cross either side.
            for x in [70.0, 954.0] {
                c.rect(Vec2::new(x - 18.0, y0 + 26.0), Vec2::new(x + 18.0, y0 + 38.0), rgb(0x39ff6a));
                c.rect(Vec2::new(x - 6.0, y0 + 14.0), Vec2::new(x + 6.0, y0 + 50.0), rgb(0x39ff6a));
            }
        }
    }
    c.to_texture("shop_signs", true)
}

/// Graffiti tags painted over a roller shutter texture.
pub fn graffiti_over(base: &TextureData, seed: u64) -> TextureData {
    let mut c = Canvas::from_texture(base);
    let mut rng = Rng::new(seed);
    let s = c.w as f32;
    let palette = [0xe53935, 0x43a047, 0x1e88e5, 0xfdd835, 0x8e24aa, 0xffffff, 0x111111, 0xfb8c00, 0x00acc1];
    // Fill blobs then outlined tags, like the photos of Madrid shutters.
    for _ in 0..rng.below(4) + 3 {
        let ctr = Vec2::new(rng.range(0.1, 0.9), rng.range(0.15, 0.85)) * s;
        let col = rgb(*rng.pick(&palette));
        let r = rng.range(0.06, 0.16) * s;
        let k = rng.range(0.6, 1.6);
        let a = [col[0], col[1], col[2], 0.9];
        c.fill(ctr - r * 2.0, ctr + r * 2.0, a, |p| {
            let d = (p - ctr) / Vec2::new(r * k, r);
            d.length() - 1.0 + 0.15 * (d.x * 7.0).sin() * (d.y * 5.0).cos()
        });
    }
    let words = ["MDZ", "KAOS", "LVP", "SKR", "VLC", "ZETA", "OKUP", "RAYO", "TOXIC"];
    for _ in 0..rng.below(3) + 2 {
        let w = *rng.pick(&words);
        let pos = Vec2::new(rng.range(0.05, 0.5), rng.range(0.3, 0.95)) * s;
        let size = rng.range(0.1, 0.22) * s;
        let col = rgb(*rng.pick(&palette));
        let outline = rgb(*rng.pick(&[0x111111u32, 0xffffff]));
        c.text(w, pos, size, 2.4, outline);
        c.text(w, pos, size, 1.4, col);
    }
    c.to_texture("graffiti", true)
}

/// Striped awning fabric (one repeat = 1 m).
pub fn awning(stripe_a: u32, stripe_b: u32) -> TextureData {
    let mut c = Canvas::new(256, 64, rgb(stripe_a));
    for k in 0..4 {
        let x = k as f32 * 64.0;
        c.rect(Vec2::new(x + 32.0, 0.0), Vec2::new(x + 64.0, 64.0), rgb(stripe_b));
    }
    c.to_texture("awning", true)
}

/// Rooms behind the windows: atlas of 4 columns (128 × 512 each), one
/// room per column. Kept dark like real interiors seen from the street,
/// with a ceiling lamp, furniture, a picture and sheer curtains.
pub fn window_interiors() -> TextureData {
    let (w, h) = (512u32, 512u32);
    let mut c = Canvas::new(w, h, rgb(0x1a1612));
    let mut rng = Rng::new(77);
    let walls = [0x3a3128u32, 0x2e3236, 0x3b2f2a, 0x34352c];
    for k in 0..4 {
        let x0 = k as f32 * 128.0;
        let wall = rgb(walls[k]);
        for y in 0..h {
            let t = y as f32 / h as f32;
            let f = 1.15 - 0.75 * t;
            for x in 0..128 {
                c.px[(y * w + x0 as u32 + x) as usize] = [wall[0] * f, wall[1] * f, wall[2] * f, 1.0];
            }
        }
        // Ceiling strip (top 5%) and floor (bottom 5%) used by the room box.
        c.rect(Vec2::new(x0, 0.0), Vec2::new(x0 + 128.0, 26.0), [0.42, 0.40, 0.36, 1.0]);
        c.rect(Vec2::new(x0, 486.0), Vec2::new(x0 + 128.0, 512.0), [0.16, 0.11, 0.08, 1.0]);
        // Warm lamp glow near the ceiling.
        let lamp = Vec2::new(x0 + 64.0 + rng.range(-20.0, 20.0), 40.0);
        c.fill(Vec2::new(x0, 0.0), Vec2::new(x0 + 128.0, 220.0), [0.95, 0.78, 0.52, 0.22], |p| ((p - lamp) / Vec2::new(70.0, 90.0)).length() - 1.0);
        // Picture frame or shelf.
        if k % 2 == 0 {
            let px = x0 + rng.range(30.0, 70.0);
            c.rect(Vec2::new(px, 170.0), Vec2::new(px + 36.0, 210.0), [0.55, 0.45, 0.30, 1.0]);
            c.rect(Vec2::new(px + 4.0, 174.0), Vec2::new(px + 32.0, 206.0), rgb(*rng.pick(&[0x6b7f5a, 0x8a6a4a, 0x5a6b8a])));
        } else {
            c.rect(Vec2::new(x0 + 20.0, 250.0), Vec2::new(x0 + 108.0, 256.0), [0.25, 0.18, 0.12, 1.0]);
            for b in 0..6 {
                let bx = x0 + 24.0 + b as f32 * 13.0;
                c.rect(Vec2::new(bx, 226.0), Vec2::new(bx + 9.0, 250.0), rgb(*rng.pick(&[0x7a2e2e, 0x2e4a7a, 0x8a7a3a, 0x3a5a3a])));
            }
        }
        // Furniture silhouette (sofa / table / wardrobe).
        let dark = [0.07, 0.06, 0.05, 1.0];
        match k {
            0 => c.rect(Vec2::new(x0 + 10.0, 400.0), Vec2::new(x0 + 118.0, 486.0), dark),
            1 => {
                c.rect(Vec2::new(x0 + 30.0, 380.0), Vec2::new(x0 + 100.0, 392.0), dark);
                c.rect(Vec2::new(x0 + 36.0, 392.0), Vec2::new(x0 + 42.0, 486.0), dark);
                c.rect(Vec2::new(x0 + 88.0, 392.0), Vec2::new(x0 + 94.0, 486.0), dark);
            }
            2 => c.rect(Vec2::new(x0 + 64.0, 230.0), Vec2::new(x0 + 124.0, 486.0), [0.18, 0.12, 0.08, 1.0]),
            _ => c.rect(Vec2::new(x0 + 0.0, 420.0), Vec2::new(x0 + 70.0, 486.0), dark),
        }
        // Sheer curtains drawn to the sides.
        let curtain = *rng.pick(&[[0.82, 0.80, 0.74, 0.55], [0.70, 0.60, 0.46, 0.65], [0.88, 0.88, 0.86, 0.45], [0.50, 0.32, 0.26, 0.7]]);
        let cw = rng.range(20.0, 40.0);
        for side in [x0, x0 + 128.0 - cw] {
            c.fill(Vec2::new(side, 26.0), Vec2::new(side + cw, 486.0), curtain, |p| {
                let folds = ((p.x - side) * 0.6).sin() * 1.5;
                sd_box(p - Vec2::new(side + cw * 0.5, 256.0), Vec2::new(cw * 0.5 + folds, 230.0))
            });
        }
    }
    c.to_texture("interiors", true)
}

/// Shop interiors: shelves of products under bright ceiling lights.
pub fn shop_interiors() -> TextureData {
    let (w, h) = (512u32, 512u32);
    let mut c = Canvas::new(w, h, rgb(0x2a2622));
    let mut rng = Rng::new(91);
    let walls = [0xd8d2c4u32, 0x6b4a32, 0xe4e8ea, 0x3a3a3a];
    for k in 0..4 {
        let x0 = k as f32 * 128.0;
        c.rect(Vec2::new(x0, 0.0), Vec2::new(x0 + 128.0, 512.0), rgb(walls[k]));
        c.rect(Vec2::new(x0, 0.0), Vec2::new(x0 + 128.0, 26.0), [0.95, 0.93, 0.88, 1.0]);
        for lx in [x0 + 32.0, x0 + 96.0] {
            c.rect(Vec2::new(lx - 14.0, 8.0), Vec2::new(lx + 14.0, 16.0), [1.0, 1.0, 0.96, 1.0]);
        }
        c.rect(Vec2::new(x0, 486.0), Vec2::new(x0 + 128.0, 512.0), [0.45, 0.42, 0.38, 1.0]);
        // Shelves with products.
        for row in 0..6 {
            let y = 90.0 + row as f32 * 52.0;
            c.rect(Vec2::new(x0 + 4.0, y + 40.0), Vec2::new(x0 + 124.0, y + 44.0), [0.3, 0.25, 0.2, 1.0]);
            let mut x = x0 + 6.0;
            while x < x0 + 120.0 {
                let pw = rng.range(3.0, 7.0);
                let ph = rng.range(10.0, 30.0);
                let col = rgb(*rng.pick(&[0x9a3a32, 0x3a5a8a, 0xb89a50, 0x4a6a44, 0xd8d4c8, 0x6a4a6a, 0xa86a3a, 0x6a5a4a, 0x2a2a2a]));
                let k = rng.range(0.55, 0.9);
                c.rect(Vec2::new(x, y + 40.0 - ph), Vec2::new((x + pw).min(x0 + 122.0), y + 40.0), [col[0] * k, col[1] * k, col[2] * k, 1.0]);
                x += pw + rng.range(0.5, 2.5);
            }
        }
        // Counter.
        c.rect(Vec2::new(x0 + 10.0, 420.0), Vec2::new(x0 + 118.0, 486.0), [0.22, 0.16, 0.12, 1.0]);
        c.rect(Vec2::new(x0 + 10.0, 414.0), Vec2::new(x0 + 118.0, 420.0), [0.7, 0.68, 0.62, 1.0]);
    }
    c.to_texture("shop_interiors", true)
}

/// Window glass imperfections over one window (UV 0..1 across both
/// leaves): a slight pillow bulge per leaf and horizontal ripples (old
/// drawn glass) in the normal map, and smudges, water spots and a dusty
/// lower edge in the roughness, so reflections wobble and soften a little
/// differently on every pane instead of being a perfect mirror. The normal
/// map is stored 4x exaggerated; use `normal_scale` 0.25.
pub fn glass() -> GenSet {
    use crate::noise::{fbm, fbm_aniso, smoothstep};
    let n = 256u32;
    let mut height = vec![0.0f32; (n * n) as usize];
    let mut rough = vec![0.0f32; (n * n) as usize];
    let mut rng = Rng::new(0x61a55);
    let spots: Vec<(Vec2, f32)> = (0..40).map(|_| (Vec2::new(rng.f(), rng.f()), rng.range(0.004, 0.012))).collect();
    for y in 0..n {
        let v = (y as f32 + 0.5) / n as f32;
        for x in 0..n {
            let u = (x as f32 + 0.5) / n as f32;
            let i = (y * n + x) as usize;
            // Pillow: each leaf bulges a little at its centre.
            let s = (u * 2.0).fract();
            let pillow = (std::f32::consts::PI * s).sin() * (std::f32::consts::PI * v).sin();
            let ripple = fbm_aniso(u, v, 2, 12, 3, 7) - 0.5;
            height[i] = 0.6 * pillow + 0.5 * ripple;
            // Roughness: clean glass 0.03, wiped smudges, dust at the bottom.
            let smudge = smoothstep(0.55, 0.75, fbm(u, v, 5, 4, 11)) * 0.07;
            let dust = smoothstep(0.75, 1.0, v) * (0.06 + 0.06 * fbm(u, v, 12, 3, 13));
            let mut r = 0.03 + smudge + dust + 0.01 * fbm(u, v, 32, 2, 17);
            for (c, rad) in &spots {
                let d = (Vec2::new(u, v) - *c).length();
                r += 0.08 * smoothstep(*rad, rad * 0.4, d);
            }
            rough[i] = r;
        }
    }
    // Normal amplitude: steepest slope about 0.1 (4x the real ~0.025).
    let mut max_g = 1e-6f32;
    for y in 0..n as i64 {
        for x in 0..n as i64 {
            let at = |x: i64, y: i64| height[(y.rem_euclid(n as i64) * n as i64 + x.rem_euclid(n as i64)) as usize];
            let g = (at(x + 1, y) - at(x - 1, y)).abs().max((at(x, y + 1) - at(x, y - 1)).abs());
            max_g = max_g.max(g);
        }
    }
    let normal = normal_from_height(&height, n, n, 0.1 / max_g);
    let orm = orm_from(&rough, n, n);
    let color = Canvas::new(4, 4, [1.0; 4]).to_texture("glass_color", true);
    GenSet { color, normal: Some(TextureData { name: "glass_normal".into(), ..normal }), orm: Some(TextureData { name: "glass_orm".into(), ..orm }) }
}

/// Worn road paint, one repeat per metre of world XZ (markings are planar
/// mapped at scale 1): tyres and weather erode it in patches and pits so
/// the asphalt shows through (alpha, for a 0.5 mask), the rest is greyed by
/// dirt and tyre rubber, and roughness rises where it is dirty.
pub fn road_paint() -> GenSet {
    use crate::noise::{fbm, fbm_aniso, hashf, smoothstep};
    let n = 512u32;
    let mut c = Canvas::new(n, n, [1.0; 4]);
    let mut rough = vec![0.0f32; (n * n) as usize];
    for y in 0..n {
        let v = (y as f32 + 0.5) / n as f32;
        for x in 0..n {
            let u = (x as f32 + 0.5) / n as f32;
            let i = (y * n + x) as usize;
            // Erosion: only small chips and pits go through to the asphalt
            // (a line must stay a line); large-scale wear just greys it.
            let patch = fbm(u, v, 4, 5, 31);
            let pits = fbm(u, v, 96, 2, 37);
            let worn = smoothstep(0.7, 0.76, pits) * (0.6 + 0.8 * smoothstep(0.5, 0.7, patch)) + 0.5 * smoothstep(0.6, 0.75, patch) * hashf(x as i32, y as i32, 41);
            let alpha = if worn > 0.5 { 0.0 } else { 1.0 };
            // Dirt: tyre rubber streaks (anisotropic) and general grime.
            let streak = fbm_aniso(u, v, 2, 24, 3, 43);
            let grime = fbm(u, v, 8, 4, 47);
            let k = (1.0 - 0.22 * smoothstep(0.35, 0.8, grime) - 0.15 * smoothstep(0.5, 0.75, streak) - 0.12 * smoothstep(0.45, 0.65, patch)).clamp(0.4, 1.0);
            let g = k * (0.97 + 0.06 * hashf(x as i32, y as i32, 53));
            c.px[i] = [g, g, g * 0.985, alpha];
            rough[i] = 0.5 + 0.3 * (1.0 - k);
        }
    }
    GenSet { color: c.to_texture("road_paint", true), normal: None, orm: Some(TextureData { name: "road_paint_orm".into(), ..orm_from(&rough, n, n) }) }
}

/// Uniform linear texture (e.g. an occlusion term for fake interiors).
pub fn solid(name: &str, value: f32) -> TextureData {
    let b = (value.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    TextureData { name: name.into(), width: 4, height: 4, rgba8: Arc::from(vec![b, b, b, 255].repeat(16)), srgb: false }
}

/// Emission of [`shop_interiors`]: only the ceiling fittings glow, with a
/// faint pool of light on the wall under them, so a shop reads as a dim
/// room with lamps rather than a self-lit picture.
pub fn shop_interior_lights() -> TextureData {
    let (w, h) = (512u32, 512u32);
    let mut c = Canvas::new(w, h, [0.0, 0.0, 0.0, 1.0]);
    for k in 0..4 {
        let x0 = k as f32 * 128.0;
        for lx in [x0 + 32.0, x0 + 96.0] {
            let at = Vec2::new(lx, 40.0);
            c.fill(Vec2::new(lx - 50.0, 16.0), Vec2::new(lx + 50.0, 200.0), [0.5, 0.45, 0.36, 0.25], |p| ((p - at) / Vec2::new(45.0, 140.0)).length() - 1.0);
            c.rect(Vec2::new(lx - 14.0, 8.0), Vec2::new(lx + 14.0, 16.0), [1.0, 1.0, 0.96, 1.0]);
        }
    }
    c.to_texture("shop_interior_lights", true)
}

/// Street name plate, Madrid ceramic style: blue frame, white field.
pub fn street_plate(name: &str) -> TextureData {
    let mut c = Canvas::new(512, 192, rgb(0xf4f1e6));
    let blue = rgb(0x1d3f8a);
    c.rect(Vec2::new(0.0, 0.0), Vec2::new(512.0, 12.0), blue);
    c.rect(Vec2::new(0.0, 180.0), Vec2::new(512.0, 192.0), blue);
    c.rect(Vec2::new(0.0, 0.0), Vec2::new(12.0, 192.0), blue);
    c.rect(Vec2::new(500.0, 0.0), Vec2::new(512.0, 192.0), blue);
    // Tile joints.
    for x in [128.0, 256.0, 384.0] {
        c.rect(Vec2::new(x - 1.0, 12.0), Vec2::new(x + 1.0, 180.0), rgb(0xd0ccc0));
    }
    c.text_centered("CALLE", Vec2::new(256.0, 52.0), 26.0, 1.0, blue);
    let size = 52.0f32.min(470.0 / (name.chars().count() as f32 * 5.5 / 6.0));
    c.text_centered(name, Vec2::new(256.0, 120.0), size, 1.1, blue);
    c.to_texture("street_plate", true)
}

// ------------------------------------------------------------------- night

/// Night emission of [`window_interiors`]: the same rooms lit by a ceiling
/// lamp (bright near the fitting and on the ceiling, falling off toward
/// the floor and the corners), stored as sRGB color to be scaled by the
/// room tone. The sheer curtains glow where the lamp lights them.
pub fn window_interiors_lit(base: &TextureData) -> TextureData {
    let (w, h) = (base.width as usize, base.height as usize);
    let cols = 4usize;
    let cw = w / cols;
    let mut rng = Rng::new(78);
    let lamps: Vec<Vec2> = (0..cols).map(|_| Vec2::new(rng.range(0.35, 0.65), rng.range(0.08, 0.16))).collect();
    let lin = |c: u8| crate::noise::srgb_to_linear(c as f32 / 255.0);
    let s = |f: f32| (crate::noise::linear_to_srgb(f.clamp(0.0, 1.0)) * 255.0 + 0.5) as u8;
    let mut out = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            let k = (x / cw).min(cols - 1);
            let p = Vec2::new((x - k * cw) as f32 / cw as f32, y as f32 / h as f32);
            let i = (y * w + x) * 4;
            let c = Vec3::new(lin(base.rgba8[i]), lin(base.rgba8[i + 1]), lin(base.rgba8[i + 2]));
            // Lamp pool: inverse-square-ish from the fitting, plus a soft
            // fill so the room never goes black.
            let d = (p - lamps[k]) * Vec2::new(1.6, 1.0);
            let pool = 1.0 / (1.0 + 9.0 * d.length_squared());
            let ceiling = if p.y < 0.06 { 0.8 } else { 0.0 };
            let light = 0.25 + 1.6 * pool + ceiling;
            // Wall albedo ~0.05 in the atlas: lift it to read as a lit room.
            let mut v = (c * light * 5.5).min(Vec3::ONE);
            // The fitting itself.
            if ((p - lamps[k]) * Vec2::new(5.0, 9.0)).length() < 0.25 {
                v = Vec3::ONE;
            }
            out.extend([s(v.x), s(v.y), s(v.z), 255]);
        }
    }
    TextureData { name: "interiors_lit".into(), width: w as u32, height: h as u32, rgba8: Arc::from(out), srgb: true }
}

/// Drawn curtains and blinds glowing with the room light behind them: four
/// fabrics (cream sheer, ochre, white blind with slats, rust) in the
/// interior atlas layout, brightest where the lamp is.
pub fn curtains_lit() -> TextureData {
    let (w, h) = (512u32, 512u32);
    let mut c = Canvas::new(w, h, [0.0, 0.0, 0.0, 1.0]);
    let fabrics = [[0.95, 0.86, 0.70], [0.92, 0.66, 0.36], [0.96, 0.95, 0.92], [0.80, 0.42, 0.30]];
    for (k, f) in fabrics.iter().enumerate() {
        let x0 = k as u32 * 128;
        for y in 0..h {
            for x in 0..128u32 {
                let p = Vec2::new(x as f32 / 128.0, y as f32 / h as f32);
                let pool = 1.0 / (1.0 + 6.0 * ((p - Vec2::new(0.5, 0.25)) * Vec2::new(1.2, 1.0)).length_squared());
                let pattern = if k == 2 {
                    // Venetian blind: bright slats, dark gaps.
                    if (p.y * 64.0).fract() < 0.18 { 0.35 } else { 1.0 }
                } else {
                    // Vertical folds.
                    0.75 + 0.25 * (p.x * 38.0 + (p.x * 7.0).sin()).sin()
                };
                let v = 0.6 * (0.35 + 0.65 * pool) * pattern;
                c.px[(y * w + x0 + x) as usize] = [f[0] * v, f[1] * v, f[2] * v, 1.0];
            }
        }
    }
    c.to_texture("curtains_lit", true)
}

/// Night emission of [`shop_sign_atlas`]: the letters and borders glow at
/// full strength, the panel behind them only faintly (channel letters);
/// dark letters on a light panel make the panel the lightbox instead.
pub fn shop_sign_atlas_lit() -> TextureData {
    let mut c = Canvas::new(1024, 1024, [0.0, 0.0, 0.0, 1.0]);
    let lum = |c: [f32; 4]| 0.3 * c[0] + 0.6 * c[1] + 0.1 * c[2];
    for (i, (name, bg, fg)) in SHOPS.iter().enumerate() {
        let y0 = i as f32 * 64.0;
        let b = rgb(*bg);
        let mut f = rgb(*fg);
        let panel = if lum(f) < lum(b) {
            f = [f[0] * 0.15, f[1] * 0.15, f[2] * 0.15, 1.0];
            0.85
        } else {
            0.12
        };
        c.rect(Vec2::new(0.0, y0), Vec2::new(1024.0, y0 + 64.0), [b[0] * panel, b[1] * panel, b[2] * panel, 1.0]);
        c.rect(Vec2::new(6.0, y0 + 5.0), Vec2::new(1018.0, y0 + 7.0), f);
        c.rect(Vec2::new(6.0, y0 + 57.0), Vec2::new(1018.0, y0 + 59.0), f);
        let size = 30.0f32.min(900.0 / (name.chars().count() as f32 * 5.5 / 6.0));
        c.text_centered(name, Vec2::new(512.0, y0 + 30.0), size, 1.1, f);
        if *name == "FARMACIA" {
            for x in [70.0, 954.0] {
                c.rect(Vec2::new(x - 18.0, y0 + 26.0), Vec2::new(x + 18.0, y0 + 38.0), rgb(0x39ff6a));
                c.rect(Vec2::new(x - 6.0, y0 + 14.0), Vec2::new(x + 6.0, y0 + 50.0), rgb(0x39ff6a));
            }
        }
    }
    c.to_texture("shop_signs_lit", true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn road_paint_is_worn_but_mostly_intact() {
        let g = road_paint();
        let px: Vec<&[u8]> = g.color.rgba8.chunks_exact(4).collect();
        let holes = px.iter().filter(|p| p[3] < 128).count() as f32 / px.len() as f32;
        assert!(holes > 0.01 && holes < 0.12, "worn fraction {holes}");
        // Dirt greys it: never pure white on average.
        let mean = px.iter().map(|p| p[0] as f32).sum::<f32>() / px.len() as f32;
        assert!(mean < 245.0 && mean > 180.0, "mean {mean}");
    }

    #[test]
    fn stop_sign_is_red_with_white_letters() {
        let t = sign_face(SignKind::Stop);
        let px = |x: u32, y: u32| {
            let i = ((y * 256 + x) * 4) as usize;
            [t.rgba8[i], t.rgba8[i + 1], t.rgba8[i + 2]]
        };
        let edge = px(128, 20);
        assert!(edge[0] > 150 && edge[1] < 60, "red field {edge:?}");
        // Some white pixels inside (the letters).
        let white = (60..196).flat_map(|x| (100..156).map(move |y| (x, y))).filter(|&(x, y)| px(x, y)[1] > 200).count();
        assert!(white > 500, "letters {white}");
    }

    #[test]
    fn tiles_have_gloss_and_grout() {
        let s = tiles(TilePattern::CheckerGreen, 1);
        let orm = s.orm.unwrap();
        let r: Vec<u8> = orm.rgba8.chunks_exact(4).map(|c| c[1]).collect();
        assert!(r.iter().any(|&v| v < 60) && r.iter().any(|&v| v > 200));
    }

    #[test]
    fn font_covers_shop_names() {
        for (name, _, _) in SHOPS {
            for ch in name.chars() {
                assert!(ch == ' ' || !glyph(ch).is_empty(), "missing glyph {ch}");
            }
        }
    }
}
