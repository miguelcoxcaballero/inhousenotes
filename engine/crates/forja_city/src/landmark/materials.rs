//! Landmark materials: the kit's photographic sets (tinted) plus
//! procedural sets built on them or from scratch — ashlar coursing with
//! per-block stone grain, marble, travertine, white concrete, glazed roof
//! tiles (scales, chevrons, stripes), trencadís, polychrome ceramic,
//! stained glass, curtain walls, metals (zinc, lead, copper patina,
//! bronze, gold leaf, white steel) — and optional weathering. All
//! procedural textures tile seamlessly and carry normal and ORM maps.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use forja_assets::{MaterialData, SceneData, TextureData};
use forja_math::{Vec2, Vec3, Vec4};

use super::palette::Entry;
use super::spec::Color;
use crate::kit::{Assembler, Kit, Mat, Rgb};
use crate::texgen::TilePattern;

// ------------------------------------------------------------------ noise

fn hash(x: i32, y: i32, s: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x8da6_b343) ^ (y as u32).wrapping_mul(0xd816_3841) ^ s.wrapping_mul(0xcb1a_b31f);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1_e995);
    h ^= h >> 15;
    (h & 0x00ff_ffff) as f32 / 16_777_216.0
}

/// Periodic value noise (period `p` cells).
fn vnoise(x: f32, y: f32, p: i32, s: u32) -> f32 {
    let (xi, yi) = (x.floor() as i32, y.floor() as i32);
    let (fx, fy) = (x - xi as f32, y - yi as f32);
    let w = |v: i32| v.rem_euclid(p.max(1));
    let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let a = hash(w(xi), w(yi), s);
    let b = hash(w(xi + 1), w(yi), s);
    let c = hash(w(xi), w(yi + 1), s);
    let d = hash(w(xi + 1), w(yi + 1), s);
    let ab = a + (b - a) * sx;
    let cd = c + (d - c) * sx;
    ab + (cd - ab) * sy
}

/// Tileable fBm over [0, 1)² with base frequency `f` (integer).
fn fbm(u: f32, v: f32, f: i32, oct: u32, s: u32) -> f32 {
    let mut sum = 0.0;
    let mut amp = 0.5;
    let mut norm = 0.0;
    let mut freq = f;
    for k in 0..oct {
        sum += amp * vnoise(u * freq as f32, v * freq as f32, freq, s.wrapping_add(k * 7919));
        norm += amp;
        amp *= 0.5;
        freq *= 2;
    }
    sum / norm
}

/// Periodic value noise with separate integer periods in x and y.
fn vnoise2(x: f32, y: f32, px: i32, py: i32, s: u32) -> f32 {
    let (xi, yi) = (x.floor() as i32, y.floor() as i32);
    let (fx, fy) = (x - xi as f32, y - yi as f32);
    let wx = |v: i32| v.rem_euclid(px.max(1));
    let wy = |v: i32| v.rem_euclid(py.max(1));
    let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let a = hash(wx(xi), wy(yi), s);
    let b = hash(wx(xi + 1), wy(yi), s);
    let c = hash(wx(xi), wy(yi + 1), s);
    let d = hash(wx(xi + 1), wy(yi + 1), s);
    let ab = a + (b - a) * sx;
    let cd = c + (d - c) * sx;
    ab + (cd - ab) * sy
}

/// Tileable fBm over [0, 1)² with stretched cells: `fx` x `fy` cells at
/// the base octave (both integers, so the repeat stays seamless).
fn fbm2(u: f32, v: f32, fx: i32, fy: i32, oct: u32, s: u32) -> f32 {
    let mut sum = 0.0;
    let mut amp = 0.5;
    let mut norm = 0.0;
    let (mut ax, mut ay) = (fx.max(1), fy.max(1));
    for k in 0..oct {
        sum += amp * vnoise2(u * ax as f32, v * ay as f32, ax, ay, s.wrapping_add(k * 7919));
        norm += amp;
        amp *= 0.5;
        ax *= 2;
        ay *= 2;
    }
    sum / norm
}

/// Periodic Voronoi: (distance to nearest, distance to second, cell id,
/// offset from the nearest site in cell units).
fn voronoi(u: f32, v: f32, n: i32, s: u32) -> (f32, f32, u32, Vec2) {
    let (x, y) = (u * n as f32, v * n as f32);
    let (xi, yi) = (x.floor() as i32, y.floor() as i32);
    let (mut d1, mut d2, mut id) = (f32::MAX, f32::MAX, 0u32);
    let mut off = Vec2::ZERO;
    for dy in -1..=1 {
        for dx in -1..=1 {
            let (cx, cy) = (xi + dx, yi + dy);
            let (wx, wy) = (cx.rem_euclid(n), cy.rem_euclid(n));
            let px = cx as f32 + 0.1 + 0.8 * hash(wx, wy, s);
            let py = cy as f32 + 0.1 + 0.8 * hash(wx, wy, s ^ 0x5555);
            let d = ((px - x).powi(2) + (py - y).powi(2)).sqrt();
            if d < d1 {
                d2 = d1;
                d1 = d;
                id = (wx * 7349 + wy * 19) as u32;
                off = Vec2::new(x - px, y - py);
            } else if d < d2 {
                d2 = d;
            }
        }
    }
    (d1, d2, id, off)
}

fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

fn mul3(a: [f32; 3], k: f32) -> [f32; 3] {
    [a[0] * k, a[1] * k, a[2] * k]
}

fn srgb_to_lin(v: f32) -> f32 {
    if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

fn hexc(c: u32) -> [f32; 3] {
    [((c >> 16) & 255) as f32 / 255.0, ((c >> 8) & 255) as f32 / 255.0, (c & 255) as f32 / 255.0]
}

/// A generated PBR texel.
#[derive(Clone, Copy)]
struct Texel {
    /// sRGB color.
    c: [f32; 3],
    /// Height for the normal map (0..1).
    h: f32,
    rough: f32,
    metal: f32,
    ao: f32,
}

/// Generated color, normal and ORM textures.
#[derive(Clone)]
pub struct Set {
    pub color: TextureData,
    pub normal: TextureData,
    pub orm: TextureData,
}

/// Generates a tileable set from a per-texel function over (u, v) in [0, 1).
fn generate(name: &str, n: u32, normal_strength: f32, f: impl Fn(f32, f32) -> Texel + Sync) -> Set {
    let size = (n * n) as usize;
    let mut tex = vec![Texel { c: [0.0; 3], h: 0.0, rough: 1.0, metal: 0.0, ao: 1.0 }; size];
    let threads = std::thread::available_parallelism().map(|v| v.get()).unwrap_or(4).clamp(1, 8);
    let rows_per = (n as usize).div_ceil(threads);
    std::thread::scope(|s| {
        for (k, chunk) in tex.chunks_mut(rows_per * n as usize).enumerate() {
            let f = &f;
            s.spawn(move || {
                for (i, t) in chunk.iter_mut().enumerate() {
                    let idx = k * rows_per * n as usize + i;
                    let (x, y) = ((idx % n as usize) as f32, (idx / n as usize) as f32);
                    *t = f((x + 0.5) / n as f32, (y + 0.5) / n as f32);
                }
            });
        }
    });
    let mut color = Vec::with_capacity(size * 4);
    let mut orm = Vec::with_capacity(size * 4);
    for t in &tex {
        color.extend([(t.c[0].clamp(0.0, 1.0) * 255.0 + 0.5) as u8, (t.c[1].clamp(0.0, 1.0) * 255.0 + 0.5) as u8, (t.c[2].clamp(0.0, 1.0) * 255.0 + 0.5) as u8, 255]);
        orm.extend([(t.ao.clamp(0.0, 1.0) * 255.0) as u8, (t.rough.clamp(0.02, 1.0) * 255.0) as u8, (t.metal.clamp(0.0, 1.0) * 255.0) as u8, 255]);
    }
    let w = n as i64;
    let hgt = |x: i64, y: i64| tex[(y.rem_euclid(w) * w + x.rem_euclid(w)) as usize].h;
    let mut normal = Vec::with_capacity(size * 4);
    let k = normal_strength * n as f32 / 256.0;
    for y in 0..w {
        for x in 0..w {
            let dx = (hgt(x + 1, y) - hgt(x - 1, y)) * k;
            let dy = (hgt(x, y + 1) - hgt(x, y - 1)) * k;
            let v = Vec3::new(-dx, dy, 1.0).normalize();
            normal.extend([((v.x * 0.5 + 0.5) * 255.0) as u8, ((v.y * 0.5 + 0.5) * 255.0) as u8, ((v.z * 0.5 + 0.5) * 255.0) as u8, 255]);
        }
    }
    Set {
        color: TextureData { name: format!("{name}_color"), width: n, height: n, rgba8: Arc::from(color), srgb: true },
        normal: TextureData { name: format!("{name}_normal"), width: n, height: n, rgba8: Arc::from(normal), srgb: false },
        orm: TextureData { name: format!("{name}_orm"), width: n, height: n, rgba8: Arc::from(orm), srgb: false },
    }
}

/// Bilinear sample of an sRGB texture at (u, v) (wrapping), 0..1 floats.
fn sample(t: &TextureData, u: f32, v: f32) -> [f32; 3] {
    let (w, h) = (t.width as f32, t.height as f32);
    let (x, y) = (u.rem_euclid(1.0) * w - 0.5, v.rem_euclid(1.0) * h - 0.5);
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let px = |xx: f32, yy: f32| {
        let xi = (xx as i64).rem_euclid(t.width as i64) as usize;
        let yi = (yy as i64).rem_euclid(t.height as i64) as usize;
        let i = (yi * t.width as usize + xi) * 4;
        [t.rgba8[i] as f32 / 255.0, t.rgba8[i + 1] as f32 / 255.0, t.rgba8[i + 2] as f32 / 255.0]
    };
    let a = mix3(px(x0, y0), px(x0 + 1.0, y0), fx);
    let b = mix3(px(x0, y0 + 1.0), px(x0 + 1.0, y0 + 1.0), fx);
    mix3(a, b, fy)
}

fn lum(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

// --------------------------------------------------------- procedural sets

/// Ashlar coursing over a square repeat of `span` metres: `rows` courses
/// of blocks around `cols` per row with staggered, varying lengths. Each
/// block takes its own patch of the photographic stone grain and its own
/// tone; mortar joints (`joint` metres) are close to the stone colour and
/// sunk in the normal map; arrises are slightly worn; large-scale patina
/// keeps walls from looking uniform.
fn ashlar(grain: Option<&TextureData>, span: f32, rows: u32, cols: u32, joint: f32, seed: u32, rough: f32) -> Set {
    let rows = rows.max(1);
    let cols = cols.max(1);
    let grain_mean = grain.map(|g| {
        let mut s = 0.0;
        for k in 0..256 {
            s += lum(sample(g, (k as f32 * 0.137) % 1.0, (k as f32 * 0.291) % 1.0));
        }
        s / 256.0
    });
    let jrel = (joint / span).max(0.0004);
    let n = if span > 5.0 { 2048 } else { 1024 };
    // Course boundaries jittered (periodic), so courses differ in height.
    let bounds: Vec<f32> = (0..=rows).map(|k| if k == 0 || k == rows { k as f32 } else { k as f32 + (hash(k as i32, 17, seed) - 0.5) * 0.36 }).collect();
    generate("ashlar", n, 1.4, move |u, v| {
        let y = v * rows as f32;
        let mut row = (y.floor() as i32).clamp(0, rows as i32 - 1);
        while row > 0 && y < bounds[row as usize] {
            row -= 1;
        }
        while (row as u32) < rows - 1 && y >= bounds[row as usize + 1] {
            row += 1;
        }
        let (r0, r1) = (bounds[row as usize], bounds[row as usize + 1]);
        let fy = ((y - r0) / (r1 - r0).max(1e-3)).clamp(0.0, 1.0);
        let row_h = (r1 - r0).max(0.2);
        // Per-row cut positions (blocks of varying length), staggered.
        let total: f32 = (0..cols).map(|i| 0.65 + 0.7 * hash(row, i as i32, seed ^ 0x77)).sum();
        let off = hash(row, 3, seed) * 0.6 + if row % 2 == 0 { 0.0 } else { 0.5 };
        let xm = (u * cols as f32 + off).rem_euclid(cols as f32);
        let mut acc = 0.0;
        let mut start = 0.0;
        let mut end = cols as f32;
        let mut bi = 0;
        for i in 0..cols {
            let w = (0.65 + 0.7 * hash(row, i as i32, seed ^ 0x77)) * cols as f32 / total;
            if xm < acc + w || i == cols - 1 {
                start = acc;
                end = acc + w;
                bi = i as i32;
                break;
            }
            acc += w;
        }
        let fx = (xm - start) / (end - start).max(1e-3);
        // Distances to the joints in texture units.
        let dx = fx.min(1.0 - fx) * (end - start) / cols as f32;
        let dy = fy.min(1.0 - fy) * row_h / rows as f32;
        let d = dx.min(dy);
        let id = hash(row, bi, seed);
        // Most blocks close in tone, some clearly replaced or stained.
        let odd = hash(row, bi, seed ^ 0x4d);
        let tone = 0.88 + 0.18 * id + if odd > 0.93 { 0.1 } else if odd < 0.06 { -0.12 } else { 0.0 };
        let warm = (hash(row, bi, seed ^ 0x99) - 0.5) * 0.09 + if odd > 0.96 { 0.05 } else { 0.0 };
        let stone = match grain {
            Some(g) => {
                let gu = u * span / 2.0 + id * 7.13 + row as f32 * 0.37;
                let gv = v * span / 2.0 + hash(row, bi, seed ^ 0x3) * 5.7;
                let l = lum(sample(g, gu, gv)) / grain_mean.unwrap_or(0.6).max(0.05);
                0.8 * (0.72 + 0.28 * l)
            }
            None => 0.74 + 0.12 * fbm(u, v, 16, 4, seed),
        };
        let patina = fbm(u, v, 3, 4, seed ^ 0x51);
        let spots = fbm(u, v, 24, 3, seed ^ 0x123);
        let pit = if fbm(u, v, 160, 2, seed ^ 0x9) > 0.8 { 0.9 } else { 1.0 };
        let l = stone * tone * pit * (0.92 + 0.16 * patina) * (1.0 - 0.08 * (spots - 0.5).max(0.0));
        let face = [l * (1.0 + warm), l, l * (1.0 - warm * 1.4)];
        // Worn arrises: a little darker and lower close to the joints.
        let arris = (d / (jrel * 3.0)).clamp(0.0, 1.0);
        let face = mul3(face, 0.93 + 0.07 * arris);
        let mortar = mul3([stone * 0.94, stone * 0.92, stone * 0.88], 0.95 + 0.1 * patina);
        if d < jrel {
            Texel { c: mortar, h: 0.0, rough: 0.95, metal: 0.0, ao: 0.78 }
        } else {
            Texel { c: face, h: 0.35 + 0.45 * arris - 0.06 * spots, rough: rough + 0.08 * (spots - 0.5), metal: 0.0, ao: 0.9 + 0.1 * arris }
        }
    })
}

fn marble(seed: u32, vein: [f32; 3]) -> Set {
    generate("marble", 1024, 0.6, move |u, v| {
        let t = fbm(u, v, 4, 5, seed);
        let w = ((u * 6.0 + v * 2.0 + t * 7.0) * std::f32::consts::PI).sin().abs();
        let vein_t = (1.0 - w).powf(10.0) * 0.8 + (1.0 - ((u * 15.0 + t * 12.0) * std::f32::consts::PI).sin().abs()).powf(30.0) * 0.4;
        let base = [0.93, 0.92, 0.9];
        let cloud = fbm(u, v, 8, 4, seed ^ 0x44);
        let c = mix3(mix3(base, [0.86, 0.85, 0.84], cloud * 0.6), vein, vein_t.min(1.0));
        Texel { c, h: 0.5 - vein_t * 0.05, rough: 0.22 + 0.1 * cloud, metal: 0.0, ao: 1.0 }
    })
}

fn travertine(seed: u32) -> Set {
    generate("travertine", 1024, 1.6, move |u, v| {
        let band = fbm(u * 0.25, v, 6, 3, seed);
        let layers = ((v * 22.0 + band * 6.0) * std::f32::consts::PI).sin() * 0.5 + 0.5;
        let pores = fbm(u, v * 3.0, 24, 2, seed ^ 0x11);
        let pore = ((pores - 0.62) * 8.0).clamp(0.0, 1.0);
        let c0 = mix3([0.86, 0.8, 0.68], [0.8, 0.72, 0.58], layers * 0.6 + band * 0.3);
        let c = mix3(c0, [0.55, 0.48, 0.38], pore);
        Texel { c, h: 0.6 - pore * 0.5, rough: 0.62, metal: 0.0, ao: 1.0 - pore * 0.3 }
    })
}

/// White concrete: smooth mottled surface, formwork joints and tie holes.
fn concrete(seed: u32, panel: (u32, u32)) -> Set {
    generate("concrete", 1024, 1.2, move |u, v| {
        let m = fbm(u, v, 6, 5, seed);
        let fine = fbm(u, v, 96, 2, seed ^ 0x2);
        let px = (u * panel.0 as f32).fract();
        let py = (v * panel.1 as f32).fract();
        let joint = (px.min(1.0 - px) * 1024.0 / panel.0 as f32).min(py.min(1.0 - py) * 1024.0 / panel.1 as f32);
        let j = (1.0 - (joint / 2.0)).clamp(0.0, 1.0);
        let ties = {
            let tx = (px * 4.0).fract() - 0.5;
            let ty = (py * 3.0).fract() - 0.5;
            ((tx * tx + ty * ty).sqrt() < 0.035) as i32 as f32
        };
        let l = 0.9 + 0.05 * (m - 0.5) + 0.02 * (fine - 0.5) - 0.12 * j - 0.25 * ties;
        Texel { c: [l * 0.99, l * 0.99, l * 0.97], h: 0.5 + 0.03 * fine - 0.3 * j - 0.4 * ties, rough: 0.7 + 0.1 * fine, metal: 0.0, ao: 1.0 - 0.3 * ties }
    })
}

fn terracotta(seed: u32) -> Set {
    generate("terracotta", 512, 1.0, move |u, v| {
        let n = fbm(u, v, 8, 5, seed);
        let speck = (fbm(u, v, 128, 1, seed ^ 0x8) > 0.8) as i32 as f32;
        let c = mix3([0.66, 0.33, 0.2], [0.56, 0.26, 0.16], n);
        Texel { c: mix3(c, [0.35, 0.18, 0.12], speck * 0.5), h: n * 0.4, rough: 0.85, metal: 0.0, ao: 1.0 }
    })
}

/// Valencian glazed tiles: imbricated scales, chevrons, stripes, checks or
/// diamonds in a palette, glossy glaze over thin grout. With `weights`,
/// tiles take colours at random in those proportions (blue domes with a
/// sprinkle of white and gold); otherwise the pattern cycles the palette.
fn glazed_tiles(pattern: &str, colors: &[[f32; 3]], weights: &[f32], seed: u32) -> Set {
    let pal: Vec<[f32; 3]> = if colors.is_empty() { vec![hexc(0x1d4f9a), hexc(0xf2efe6)] } else { colors.to_vec() };
    let wsum: f32 = weights.iter().take(pal.len()).map(|w| w.max(0.0)).sum();
    let cum: Vec<f32> = if wsum > 0.0 {
        let mut acc = 0.0;
        weights.iter().take(pal.len()).map(|w| {
            acc += w.max(0.0) / wsum;
            acc
        }).collect()
    } else {
        Vec::new()
    };
    let npal = pal.len().max(1);
    let pick = move |cyc: usize, col: i32, row: i32| -> usize {
        if cum.is_empty() {
            return cyc % npal;
        }
        let r = hash(col, row, seed ^ 0x6b);
        cum.iter().position(|c| r < *c).unwrap_or(cum.len() - 1)
    };
    let pal2 = pal.clone();
    let pat = pattern.to_string();
    // One texture repeat = 8 x 8 tiles.
    generate(&format!("glazed_{pat}"), 1024, 2.2, move |u, v| {
        let n = 8.0;
        let (x, y) = (u * n, v * n);
        let (row, col) = (y.floor() as i32, x.floor() as i32);
        let (fx, fy) = (x.fract(), y.fract());
        let np = pal2.len() as i32;
        let (idx, d_edge, bulge) = match pat.as_str() {
            "scales" => {
                // Imbricated fish scales (v down = towards the eave): each
                // row overlaps the one below with its rounded lower edge,
                // offset half a tile; every texel belongs to a scale.
                let mut found = None;
                for dr in [-1i32, 0] {
                    let r = row + dr;
                    let off = if r.rem_euclid(2) == 0 { 0.0 } else { 0.5 };
                    let xs = x + off;
                    let c = xs.floor();
                    let lx = xs - c - 0.5;
                    let ly = y - r as f32;
                    // Scale r covers ly in [0, 1 + bottom(lx)] with a
                    // round bottom of radius 0.5.
                    let bottom = 0.5 + (0.25 - lx * lx).max(0.0).sqrt();
                    if ly >= 0.0 && ly <= 0.5 + bottom * 0.999 {
                        let rim = 0.5 + bottom - ly;
                        let side = 0.5 - lx.abs();
                        found = Some((pick(((c as i32) + r * 3).rem_euclid(np.max(1)) as usize, c as i32, r), rim.min(side + 0.2), (ly / (0.5 + bottom)).clamp(0.0, 1.0)));
                        break;
                    }
                }
                found.unwrap_or((0, 0.0, 0.0))
            }
            "chevron" | "zigzag" => {
                let z = ((x * 0.5).fract() - 0.5).abs() * 2.0;
                let band = ((y + z * 2.0) / 2.0).floor() as i32;
                (pick(band.rem_euclid(np) as usize, col, band), fx.min(1.0 - fx).min(fy.min(1.0 - fy)), 0.0)
            }
            "stripes" => (pick(col.rem_euclid(np) as usize, col, 0), fx.min(1.0 - fx).min(fy.min(1.0 - fy)), 0.0),
            "checker" => (pick((row + col).rem_euclid(np) as usize, col, row), fx.min(1.0 - fx).min(fy.min(1.0 - fy)), 0.0),
            "diamond" => {
                let dd = (fx - 0.5).abs() + (fy - 0.5).abs();
                (if dd < 0.32 { 0 } else { 1 % pal2.len() }, fx.min(1.0 - fx).min(fy.min(1.0 - fy)), 0.0)
            }
            _ => (pick(0, col, row), fx.min(1.0 - fx).min(fy.min(1.0 - fy)), 0.0),
        };
        let grout = d_edge < 0.022;
        let jitter = 0.94 + 0.1 * hash(col, row, seed);
        let glaze = fbm(u, v, 64, 2, seed ^ 0x31);
        let base = mul3(pal2[idx % pal2.len()], jitter * (0.96 + 0.08 * glaze));
        // Thin, dark-ish grout: it barely dilutes the colour at a distance.
        let c = if grout { mul3(pal2[idx % pal2.len()], 0.45) } else { base };
        Texel { c, h: if grout { 0.0 } else { 0.6 + 0.25 * (d_edge * 4.0).min(1.0) - 0.15 * bulge }, rough: if grout { 0.7 } else { 0.05 + 0.05 * glaze }, metal: 0.0, ao: if grout { 0.75 } else { 1.0 } }
    })
}

/// Trencadís: broken glazed shards in a palette, white grout, each shard
/// tilted a little so it catches light on its own.
fn trencadis(colors: &[[f32; 3]], seed: u32) -> Set {
    let pal: Vec<[f32; 3]> = if colors.is_empty() { vec![hexc(0xfbfbf8), hexc(0xf1f2ee), hexc(0xe9ece9), hexc(0xf6f4ee)] } else { colors.to_vec() };
    generate("trencadis", 1024, 3.5, move |u, v| {
        let (d1, d2, id, off) = voronoi(u, v, 22, seed);
        let edge = d2 - d1;
        let grout = edge < 0.055;
        let k = (hash(id as i32, 1, seed) * pal.len() as f32) as usize % pal.len();
        let c = mul3(pal[k], 0.94 + 0.08 * hash(id as i32, 2, seed));
        // Per-shard tilt: each shard is a plane leaning its own way.
        let a = hash(id as i32, 3, seed) * std::f32::consts::TAU;
        let tilt = (off.x * a.cos() + off.y * a.sin()) * (0.15 + 0.35 * hash(id as i32, 4, seed));
        Texel {
            c: if grout { [0.8, 0.79, 0.76] } else { c },
            h: if grout { 0.0 } else { 0.5 + tilt + 0.3 * (edge / 0.2).min(1.0) },
            rough: if grout { 0.9 } else { 0.08 + 0.06 * hash(id as i32, 5, seed) },
            metal: 0.0,
            ao: if grout { 0.75 } else { 1.0 },
        }
    })
}

/// Polychrome ceramic: square tiles with a four-fold floral motif.
fn polychrome(colors: &[[f32; 3]], seed: u32) -> Set {
    let pal: Vec<[f32; 3]> = if colors.len() >= 3 { colors.to_vec() } else { vec![hexc(0xf3efe2), hexc(0x1f4e9c), hexc(0xd9a521), hexc(0x2f7a4a), hexc(0xb0382c)] };
    generate("polychrome", 1024, 2.0, move |u, v| {
        let n = 6.0;
        let (x, y) = (u * n, v * n);
        let (fx, fy) = (x.fract() - 0.5, y.fract() - 0.5);
        let grout = fx.abs() > 0.47 || fy.abs() > 0.47;
        let r = (fx * fx + fy * fy).sqrt();
        let a = fy.atan2(fx);
        let petal = (a * 4.0).cos() * 0.12 + 0.26;
        let k = if r < 0.08 {
            2
        } else if r < petal {
            1
        } else if (fx.abs() - fy.abs()).abs() < 0.03 {
            3 % pal.len()
        } else if r > 0.44 {
            4 % pal.len()
        } else {
            0
        };
        let c = mul3(pal[k % pal.len()], 0.95 + 0.08 * hash(x as i32, y as i32, seed));
        Texel { c: if grout { [0.6, 0.58, 0.55] } else { c }, h: if grout { 0.0 } else { 0.6 }, rough: if grout { 0.85 } else { 0.12 }, metal: 0.0, ao: 1.0 }
    })
}

/// Stained glass seen from outside in daylight: dark jewel tones between
/// lead cames.
fn stained(colors: &[[f32; 3]], seed: u32) -> Set {
    let pal: Vec<[f32; 3]> = if colors.is_empty() { vec![hexc(0x1b3f8f), hexc(0x9c1c22), hexc(0xd2a02a), hexc(0x2b6b3c), hexc(0x5b2b78), hexc(0x2a7fa6)] } else { colors.to_vec() };
    generate("stained", 512, 2.5, move |u, v| {
        let (d1, d2, id, _) = voronoi(u, v, 9, seed);
        let came = d2 - d1 < 0.07;
        let k = (hash(id as i32, 7, seed) * pal.len() as f32) as usize % pal.len();
        let c = mul3(pal[k], 0.45 + 0.25 * fbm(u, v, 16, 2, seed));
        Texel { c: if came { [0.05, 0.05, 0.05] } else { c }, h: if came { 1.0 } else { 0.3 }, rough: if came { 0.6 } else { 0.06 }, metal: if came { 0.4 } else { 0.0 }, ao: 1.0 }
    })
}

/// Curtain wall: reflective glass panes in an aluminium mullion grid
/// (one repeat = 2 x 2 panels).
fn curtain_wall(seed: u32, tint: [f32; 3]) -> Set {
    generate("curtain_wall", 1024, 2.0, move |u, v| {
        let (x, y) = (u * 2.0, v * 2.0);
        let (fx, fy) = (x.fract(), y.fract());
        let m = 0.025;
        let mullion = fx < m || fx > 1.0 - m || fy < m * 0.7 || fy > 1.0 - m * 0.7;
        let transom = (fy - 0.25).abs() < 0.006;
        let pane = hash(x as i32, y as i32, seed);
        if mullion || transom {
            Texel { c: [0.62, 0.64, 0.66], h: 1.0, rough: 0.35, metal: 0.9, ao: 1.0 }
        } else {
            let c = mul3(tint, 0.9 + 0.12 * pane);
            Texel { c, h: 0.2, rough: 0.04 + 0.03 * pane, metal: 0.55, ao: 1.0 }
        }
    })
}

fn metal(kind: &str, seed: u32) -> Set {
    let k = kind.to_string();
    generate(&format!("metal_{k}"), 512, 1.5, move |u, v| {
        let n = fbm(u, v, 8, 5, seed);
        let fine = fbm(u, v, 64, 2, seed ^ 0x5);
        match k.as_str() {
            "zinc" | "lead" => {
                // Standing seams every 1/4 repeat.
                let s = (u * 4.0).fract();
                let seam = (s - 0.5).abs() > 0.48;
                let base = if k == "zinc" { [0.62, 0.64, 0.66] } else { [0.42, 0.43, 0.45] };
                let c = mul3(base, 0.9 + 0.15 * n);
                Texel { c, h: if seam { 1.0 } else { 0.3 + 0.05 * fine }, rough: 0.45 + 0.2 * n, metal: if k == "zinc" { 0.85 } else { 0.6 }, ao: 1.0 }
            }
            "copper" => {
                // Verdigris over brown copper, streaked vertically.
                let streak = fbm(u * 0.2, v * 3.0, 12, 4, seed ^ 0x77);
                let patina = ((n * 0.7 + streak * 0.6) - 0.35).clamp(0.0, 1.0);
                let c = mix3([0.45, 0.25, 0.15], mix3([0.36, 0.62, 0.52], [0.52, 0.72, 0.62], fine), patina);
                Texel { c, h: fine * 0.3, rough: 0.55 + 0.25 * patina, metal: 0.8 * (1.0 - patina), ao: 1.0 }
            }
            "bronze" => {
                let c = mix3([0.34, 0.24, 0.13], [0.2, 0.3, 0.24], (n - 0.4).max(0.0) * 1.5);
                Texel { c, h: fine * 0.4, rough: 0.4 + 0.2 * n, metal: 0.85, ao: 1.0 }
            }
            "gold" => {
                // Gold leaf squares with slight differences.
                let (x, y) = ((u * 10.0).floor() as i32, (v * 10.0).floor() as i32);
                let leaf = hash(x, y, seed);
                let c = mul3([1.0, 0.78, 0.36], 0.9 + 0.12 * leaf);
                Texel { c, h: leaf * 0.1 + fine * 0.05, rough: 0.18 + 0.12 * leaf, metal: 1.0, ao: 1.0 }
            }
            _ => {
                // White painted steel.
                let l = 0.93 + 0.03 * (n - 0.5);
                Texel { c: [l, l, l * 0.99], h: fine * 0.1, rough: 0.32 + 0.08 * n, metal: 0.0, ao: 1.0 }
            }
        }
    })
}

fn flat_noise(c: [f32; 3], rough: f32, seed: u32) -> Set {
    generate("flat", 256, 0.4, move |u, v| {
        let n = fbm(u, v, 8, 4, seed);
        Texel { c: mul3(c, 0.94 + 0.12 * n), h: n * 0.2, rough, metal: 0.0, ao: 1.0 }
    })
}

/// Dirt and streaks multiplied into a color texture (vertical runs along
/// -v). Every term is periodic in both directions (stretched cells with
/// integer periods), so the texture still tiles without seams.
fn weathered(t: &TextureData, amount: f32, seed: u32) -> TextureData {
    let (w, h) = (t.width, t.height);
    let mut out = vec![0u8; t.rgba8.len()];
    let rows_per = (h as usize).div_ceil(8).max(1);
    std::thread::scope(|sc| {
        for (k, chunk) in out.chunks_mut(rows_per * w as usize * 4).enumerate() {
            let src = &t.rgba8;
            sc.spawn(move || {
                for (i, px) in chunk.chunks_exact_mut(4).enumerate() {
                    let idx = k * rows_per * w as usize + i;
                    let (x, y) = (idx % w as usize, idx / w as usize);
                    let (u, v) = ((x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32);
                    // Thin vertical runs (48 x 3 cells, 5 octaves) warped a
                    // little so they do not follow the noise grid.
                    let warp = fbm(u, v, 6, 2, seed ^ 0x71) - 0.5;
                    let streak = fbm2(u + warp * 0.03, v, 48, 3, 5, seed);
                    let blot = fbm(u, v, 4, 5, seed ^ 0x33);
                    let grime = fbm(u, v, 32, 3, seed ^ 0x5a);
                    let d = ((streak - 0.5).max(0.0) * 1.3 + (blot - 0.52).max(0.0) * 0.6 + (grime - 0.5) * 0.08) * amount;
                    let kk = 1.0 - d.clamp(-0.03, 0.4);
                    let j = idx * 4;
                    let warm = [kk, kk * 0.985, kk * 0.96];
                    for c in 0..3 {
                        px[c] = ((src[j + c] as f32) * warm[c]).clamp(0.0, 255.0) as u8;
                    }
                    px[3] = 255;
                }
            });
        }
    });
    TextureData { name: format!("{}_weathered", t.name), width: w, height: h, rgba8: Arc::from(out), srgb: true }
}

fn mean_channel(t: &TextureData, ch: usize) -> f32 {
    let n = (t.width * t.height) as usize;
    let step = (n / 4096).max(1);
    let mut s = 0.0;
    let mut k = 0.0;
    for i in (0..n).step_by(step) {
        s += t.rgba8[i * 4 + ch] as f32 / 255.0;
        k += 1.0;
    }
    s / k
}

// --------------------------------------------------------------- library

fn rgb8(c: Color) -> Rgb {
    [(c.0[0] * 255.0).round().clamp(0.0, 255.0) as u8, (c.0[1] * 255.0).round().clamp(0.0, 255.0) as u8, (c.0[2] * 255.0).round().clamp(0.0, 255.0) as u8]
}

fn lin(c: [f32; 3]) -> Vec4 {
    Vec4::new(srgb_to_lin(c[0]), srgb_to_lin(c[1]), srgb_to_lin(c[2]), 1.0)
}

/// A material with its own textures (indices into `textures`).
#[derive(Clone)]
pub struct Built {
    pub data: MaterialData,
    pub textures: Vec<TextureData>,
}

/// Textures and materials for landmarks, cached across landmarks.
pub struct Library<'k> {
    pub kit: &'k Kit,
    sets: Mutex<HashMap<String, Arc<Set>>>,
    kit_mats: Mutex<HashMap<String, Built>>,
    misc: Mutex<HashMap<String, TextureData>>,
}

impl<'k> Library<'k> {
    pub fn new(kit: &'k Kit) -> Self {
        Library { kit, sets: Mutex::new(HashMap::new()), kit_mats: Mutex::new(HashMap::new()), misc: Mutex::new(HashMap::new()) }
    }

    fn set(&self, key: &str, make: impl FnOnce() -> Set) -> Arc<Set> {
        if let Some(s) = self.sets.lock().unwrap().get(key) {
            return s.clone();
        }
        let s = Arc::new(make());
        self.sets.lock().unwrap().insert(key.to_string(), s.clone());
        s
    }

    /// The kit's material for `m` (with its textures).
    fn kit_mat(&self, m: Mat) -> Built {
        let key = format!("{m:?}");
        if let Some(b) = self.kit_mats.lock().unwrap().get(&key) {
            return b.clone();
        }
        let mut a = Assembler::new(self.kit, "lm");
        let i = a.material(m);
        let s: SceneData = a.finish();
        let b = Built { data: s.materials[i].clone(), textures: s.textures };
        self.kit_mats.lock().unwrap().insert(key, b.clone());
        b
    }

    fn from_set(set: &Set, name: &str) -> Built {
        Built {
            data: MaterialData {
                name: name.into(),
                base_color: Vec4::ONE,
                base_color_tex: Some(0),
                normal_tex: Some(1),
                metallic_roughness_tex: Some(2),
                occlusion_tex: Some(2),
                metallic: 1.0,
                roughness: 1.0,
                ..Default::default()
            },
            textures: vec![set.color.clone(), set.normal.clone(), set.orm.clone()],
        }
    }

    /// Builds the render material of a palette entry.
    pub fn material(&self, e: &Entry) -> Built {
        let s = &e.spec;
        let seed = s.seed.unwrap_or(7) as u32;
        let white: Rgb = [255; 3];
        let tint_rgb = s.tint.map(rgb8).unwrap_or(white);
        let colors: Vec<[f32; 3]> = s.colors.iter().map(|c| c.0).collect();
        let kind = s.kind.as_str();
        let mut b = match kind {
            "ashlar" | "stone" | "limestone" => {
                let block = s.block.unwrap_or(if kind == "limestone" { [1.15, 0.52] } else { [0.95, 0.45] });
                let size = e.uv_scale;
                let rows = (size / block[1].max(0.05)).round().max(1.0) as u32;
                let cols = (size / block[0].max(0.05)).round().max(1.0) as u32;
                let joint = if kind == "limestone" { 0.004 } else { 0.007 };
                let grain = self.kit_mat(Mat::Moulding(white));
                let gtex = grain.data.base_color_tex.map(|i| grain.textures[i].clone());
                let key = format!("ashlar_{size}_{rows}_{cols}_{joint}_{seed}");
                let set = self.set(&key, || ashlar(gtex.as_ref(), size, rows, cols, joint, seed, 0.8));
                let mut b = Self::from_set(&set, kind);
                b.data.base_color = lin(s.tint.map(|c| c.0).unwrap_or(hexc(0xe6dcc6)));
                b
            }
            "sandstone" => self.kit_mat(Mat::Ashlar(tint_rgb)),
            "brick" => self.kit_mat(Mat::Brick(tint_rgb)),
            "stucco" | "render" => self.kit_mat(Mat::Stucco(tint_rgb)),
            "plaster" => self.kit_mat(Mat::Plaster(tint_rgb)),
            "moulding" => self.kit_mat(Mat::Moulding(tint_rgb)),
            "granite" => self.kit_mat(Mat::Granite),
            "roof_tiles" | "clay_tiles" => self.kit_mat(Mat::RoofClay),
            "slate" => self.kit_mat(Mat::RoofSlate),
            "glass" => self.kit_mat(Mat::Glass),
            "interior" => self.kit_mat(Mat::Interior),
            "wood" => self.kit_mat(Mat::Wood(tint_rgb)),
            "painted_wood" => self.kit_mat(Mat::PaintedWood(tint_rgb)),
            "water" => self.kit_mat(Mat::Water),
            "paving" => self.kit_mat(Mat::SidewalkSlab),
            "cobble" => self.kit_mat(Mat::Cobble),
            "asphalt" => self.kit_mat(Mat::Asphalt),
            "grass" => self.kit_mat(Mat::Grass),
            "iron" => {
                let set = self.set("iron", || flat_noise([0.09, 0.09, 0.09], 0.5, 3));
                let mut b = Self::from_set(&set, "iron");
                b.data.metallic = 0.0;
                b
            }
            "azulejo" => {
                let p = match s.pattern.as_deref().unwrap_or("checker_blue") {
                    "checker_green" => TilePattern::CheckerGreen,
                    "diamond_blue" | "diamond" => TilePattern::DiamondBlue,
                    "frieze" => TilePattern::Frieze,
                    "white" => TilePattern::White,
                    _ => TilePattern::CheckerBlue,
                };
                self.kit_mat(Mat::Tiles(p))
            }
            "marble" => {
                let vein = colors.first().copied().unwrap_or([0.5, 0.5, 0.52]);
                let set = self.set(&format!("marble_{seed}_{vein:?}"), || marble(seed, vein));
                Self::from_set(&set, "marble")
            }
            "travertine" => Self::from_set(&self.set(&format!("travertine_{seed}"), || travertine(seed)), "travertine"),
            "concrete" => Self::from_set(&self.set(&format!("concrete_{seed}"), || concrete(seed, (2, 1))), "concrete"),
            "terracotta" => Self::from_set(&self.set(&format!("terracotta_{seed}"), || terracotta(seed)), "terracotta"),
            "glazed_tiles" => {
                let pat = s.pattern.clone().unwrap_or_else(|| "scales".into());
                let key = format!("glazed_{pat}_{colors:?}_{:?}_{seed}", s.weights);
                Self::from_set(&self.set(&key, || glazed_tiles(&pat, &colors, &s.weights, seed)), "glazed_tiles")
            }
            "trencadis" => Self::from_set(&self.set(&format!("trencadis_{colors:?}_{seed}"), || trencadis(&colors, seed)), "trencadis"),
            "polychrome" => Self::from_set(&self.set(&format!("poly_{colors:?}_{seed}"), || polychrome(&colors, seed)), "polychrome"),
            "stained_glass" => {
                let mut b = Self::from_set(&self.set(&format!("stained_{colors:?}_{seed}"), || stained(&colors, seed)), "stained_glass");
                b.data.emissive = Vec3::splat(0.02);
                b.data.emissive_tex = Some(0);
                b
            }
            "curtain_wall" => {
                let t = s.color.map(|c| c.0).unwrap_or(hexc(0x3c4b55));
                Self::from_set(&self.set(&format!("curtain_{t:?}_{seed}"), || curtain_wall(seed, t)), "curtain_wall")
            }
            "zinc" | "lead" | "copper" | "bronze" | "gold" | "steel" | "white_steel" => {
                let k = if kind == "steel" { "white_steel" } else { kind };
                Self::from_set(&self.set(&format!("metal_{k}_{seed}"), || metal(k, seed)), k)
            }
            "dark" => Built { data: MaterialData { name: "dark".into(), base_color: Vec4::new(0.012, 0.011, 0.01, 1.0), roughness: 0.95, ..Default::default() }, textures: vec![] },
            _ => {
                let c = s.color.or(s.tint).map(|c| c.0).unwrap_or([0.8, 0.8, 0.8]);
                Self::from_set(&self.set(&format!("flat_{c:?}"), || flat_noise(c, 0.8, seed)), "flat")
            }
        };
        // Overrides.
        let procedural_tinted = matches!(kind, "ashlar" | "stone" | "limestone");
        let kit_tinted = matches!(kind, "sandstone" | "brick" | "stucco" | "render" | "plaster" | "moulding" | "wood" | "painted_wood");
        if let Some(t) = s.tint
            && !procedural_tinted
            && !kit_tinted
        {
            let l = lin(t.0);
            b.data.base_color = Vec4::new(b.data.base_color.x * l.x, b.data.base_color.y * l.y, b.data.base_color.z * l.z, b.data.base_color.w);
        }
        if let Some(c) = s.color
            && matches!(kind, "flat" | "dark" | "iron")
        {
            b.data.base_color = lin(c.0);
        }
        if let Some(r) = s.roughness {
            let mean = b.data.metallic_roughness_tex.map(|i| mean_channel(&b.textures[i], 1)).unwrap_or(1.0).max(0.05);
            b.data.roughness = (r / mean).clamp(0.02, 4.0);
        }
        if let Some(m) = s.metallic {
            match b.data.metallic_roughness_tex {
                Some(i) if mean_channel(&b.textures[i], 2) > 0.05 => {
                    b.data.metallic = (m / mean_channel(&b.textures[i], 2)).clamp(0.0, 4.0);
                }
                Some(i) => {
                    // Rewrite the metal channel.
                    let t = &b.textures[i];
                    let mut px: Vec<u8> = t.rgba8.to_vec();
                    for c in px.chunks_exact_mut(4) {
                        c[2] = (m.clamp(0.0, 1.0) * 255.0) as u8;
                    }
                    b.textures[i] = TextureData { name: format!("{}_m", t.name), rgba8: Arc::from(px), ..t.clone() };
                    b.data.metallic = 1.0;
                }
                None => b.data.metallic = m,
            }
        }
        if let Some(n) = s.normal_strength {
            b.data.normal_scale = n;
        }
        // Glazes read as wet-bright tiles: a clear coat over the colour.
        let coat = s.clearcoat.unwrap_or(if matches!(kind, "glazed_tiles" | "trencadis" | "polychrome" | "azulejo") { 0.8 } else { 0.0 });
        if coat > 0.0 {
            b.data.clearcoat = coat.clamp(0.0, 1.0);
            b.data.clearcoat_roughness = 0.06;
        }
        if let Some(em) = s.emissive {
            b.data.emissive = Vec3::splat(em);
            if b.data.emissive_tex.is_none() {
                b.data.emissive_tex = b.data.base_color_tex;
            }
        }
        if let Some(w) = s.weathering
            && w > 0.0
            && let Some(ci) = b.data.base_color_tex
        {
            let src = b.textures[ci].clone();
            let key = format!("{}_{:p}_{w}", src.name, Arc::as_ptr(&src.rgba8));
            let t = {
                let mut misc = self.misc.lock().unwrap();
                misc.entry(key).or_insert_with(|| weathered(&src, w, seed)).clone()
            };
            b.textures[ci] = t;
        }
        b.data.name = e.name.clone();
        b
    }
}

/// UV transform for an entry (metric UVs to texture space).
pub fn uv_transform(e: &Entry) -> impl Fn(Vec2) -> Vec2 + use<> {
    let scale = if e.metric { 1.0 / e.uv_scale.max(1e-3) } else { 1.0 };
    let rot = e.spec.uv_rotate.unwrap_or(0.0).to_radians();
    let r = Vec2::from_angle(rot);
    move |uv: Vec2| r.rotate(uv) * scale
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn procedural_sets_tile_and_vary() {
        let s = glazed_tiles("scales", &[], &[], 1);
        let px = |t: &TextureData, x: u32, y: u32| {
            let i = ((y * t.width + x) * 4) as usize;
            [t.rgba8[i], t.rgba8[i + 1], t.rgba8[i + 2]]
        };
        // Some blue and some white.
        let blue = (0..1024).step_by(7).flat_map(|x| (0..1024).step_by(13).map(move |y| (x, y))).filter(|&(x, y)| {
            let c = px(&s.color, x, y);
            c[2] > 120 && c[0] < 80
        });
        assert!(blue.count() > 100);
        // Glossy glaze and rough grout.
        let r: Vec<u8> = s.orm.rgba8.chunks_exact(4).map(|c| c[1]).collect();
        assert!(r.iter().any(|&v| v < 50) && r.iter().any(|&v| v > 150));
        let t = trencadis(&[], 3);
        assert_eq!(t.color.width, 1024);
        let a = ashlar(None, 3.6, 8, 4, 0.02, 1, 0.8);
        // Rough mortar joints between smoother block faces.
        let joints = a.orm.rgba8.chunks_exact(4).filter(|c| c[1] > 235).count();
        assert!(joints > 1000 && joints < 1024 * 1024 / 3, "{joints}");
    }

    #[test]
    fn weathering_tiles_without_seams() {
        let t = flat_noise([0.8, 0.78, 0.74], 0.8, 2).color;
        let w = weathered(&t, 1.0, 9);
        let (wd, ht) = (w.width as usize, w.height as usize);
        let row = |y: usize| -> Vec<f32> { (0..wd).map(|x| w.rgba8[(y * wd + x) * 4] as f32).collect() };
        let diff = |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(p, q)| (p - q).abs()).sum::<f32>() / a.len() as f32;
        // Across the wrap (last row -> first row) no worse than inside.
        let wrap = diff(&row(ht - 1), &row(0));
        let inner = diff(&row(ht / 2), &row(ht / 2 + 1));
        assert!(wrap < inner * 3.0 + 1.5, "wrap {wrap} vs inner {inner}");
        // Scales cover every texel: little grout, saturated blue.
        let g = glazed_tiles("scales", &[hexc(0x1d4a9a), hexc(0x2a5fb4), hexc(0xf2eee2)], &[0.45, 0.45, 0.1], 3);
        let px = g.color.rgba8.chunks_exact(4).collect::<Vec<_>>();
        let blue = px.iter().filter(|c| c[2] as i32 > c[0] as i32 + 50).count() as f32 / px.len() as f32;
        assert!(blue > 0.7, "{blue}");
    }

    #[test]
    fn noise_is_periodic() {
        let a = fbm(0.0, 0.3, 8, 4, 5);
        let b = fbm(1.0, 0.3, 8, 4, 5);
        assert!((a - b).abs() < 1e-4);
        let (d1, _, _, _) = voronoi(0.999, 0.5, 10, 2);
        let (e1, _, _, _) = voronoi(-0.001, 0.5, 10, 2);
        assert!((d1 - e1).abs() < 1e-3);
    }
}
