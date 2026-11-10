//! Small procedural textures of the island: ground covers, road surfaces
//! and the facades and roofs of the building shells. All of them tile, are
//! generated in milliseconds and are shared by every tile (one GPU copy).

use std::sync::Arc;

use forja_assets::{MaterialData, TextureData};
use forja_math::{Vec3, Vec4};

use super::ground::Cover;

fn hash2(x: i32, y: i32, seed: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x85eb_ca6b) ^ (y as u32).wrapping_mul(0xc2b2_ae35) ^ seed.wrapping_mul(0x27d4_eb2f);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297a_2d39);
    h ^= h >> 15;
    (h >> 8) as f32 / (1u32 << 24) as f32
}

/// Tiling value noise: `period` lattice cells across the texture.
fn vnoise(u: f32, v: f32, period: i32, seed: u32) -> f32 {
    let (x, y) = (u * period as f32, v * period as f32);
    let (xi, yi) = (x.floor() as i32, y.floor() as i32);
    let (fx, fy) = (x - xi as f32, y - yi as f32);
    let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let g = |i: i32, j: i32| hash2(i.rem_euclid(period), j.rem_euclid(period), seed);
    let a = g(xi, yi) + (g(xi + 1, yi) - g(xi, yi)) * sx;
    let b = g(xi, yi + 1) + (g(xi + 1, yi + 1) - g(xi, yi + 1)) * sx;
    a + (b - a) * sy
}

fn fbm(u: f32, v: f32, seed: u32) -> f32 {
    vnoise(u, v, 4, seed) * 0.5 + vnoise(u, v, 9, seed + 1) * 0.3 + vnoise(u, v, 23, seed + 2) * 0.2
}

fn to8(c: [f32; 3]) -> [u8; 4] {
    [(c[0].clamp(0.0, 1.0) * 255.0) as u8, (c[1].clamp(0.0, 1.0) * 255.0) as u8, (c[2].clamp(0.0, 1.0) * 255.0) as u8, 255]
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

fn rgb(hex: u32) -> [f32; 3] {
    [((hex >> 16) & 255) as f32 / 255.0, ((hex >> 8) & 255) as f32 / 255.0, (hex & 255) as f32 / 255.0]
}

/// An sRGB texture painted per texel from `(u, v, x, y)`.
pub fn paint(name: &str, size: u32, f: impl Fn(f32, f32, u32, u32) -> [f32; 3]) -> TextureData {
    let mut px = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            px.extend(to8(f((x as f32 + 0.5) / size as f32, (y as f32 + 0.5) / size as f32, x, y)));
        }
    }
    TextureData { name: name.into(), width: size, height: size, rgba8: Arc::from(px), srgb: true }
}

fn mottled(name: &str, a: u32, b: u32, grain: f32, seed: u32) -> TextureData {
    let (a, b) = (rgb(a), rgb(b));
    paint(name, 128, |u, v, x, y| {
        let n = fbm(u, v, seed);
        let g = (hash2(x as i32, y as i32, seed + 7) - 0.5) * grain;
        let c = mix(a, b, n);
        [c[0] + g, c[1] + g, c[2] + g]
    })
}

/// Texture and material of a ground cover.
pub fn cover_material(c: Cover) -> (TextureData, MaterialData) {
    let name = format!("isla_{c:?}").to_lowercase();
    let seed = c as u32 * 31 + 5;
    let tex = match c {
        Cover::Scrub => mottled(&name, 0x8a8560, 0x74774e, 0.06, seed),
        Cover::Rock => mottled(&name, 0x8d8477, 0x6e675d, 0.08, seed),
        Cover::Grass => mottled(&name, 0x858a55, 0x777f4c, 0.05, seed),
        Cover::Park => mottled(&name, 0x5f8446, 0x6c8f4e, 0.04, seed),
        Cover::Forest => mottled(&name, 0x4a5a34, 0x5a673c, 0.07, seed),
        Cover::Sand => mottled(&name, 0xd3c6a2, 0xc8ba94, 0.03, seed),
        Cover::Wetland => mottled(&name, 0x5d6f3e, 0x4c6a52, 0.05, seed),
        Cover::Rice => {
            let (a, b) = (rgb(0x7f9450), rgb(0x8c9f58));
            paint(&name, 128, |u, v, x, y| {
                let n = fbm(u, v, seed);
                let row = if (v * 32.0).fract() < 0.18 { 0.86 } else { 1.0 };
                let g = (hash2(x as i32, y as i32, seed) - 0.5) * 0.04;
                mix(a, b, n).map(|k| k * row + g)
            })
        }
        Cover::Concrete => mottled(&name, 0xa9a69f, 0x9a978f, 0.04, seed),
        Cover::Gravel => mottled(&name, 0xa39a86, 0x8c8472, 0.10, seed),
        Cover::Sport => mottled(&name, 0x6a8f52, 0x739858, 0.02, seed),
        Cover::Water => mottled(&name, 0x2f6f86, 0x2a637b, 0.0, seed),
        Cover::FieldGreen | Cover::FieldSoil | Cover::FieldDry => {
            let (a, b) = match c {
                Cover::FieldGreen => (rgb(0x758c4a), rgb(0x8a7c58)),
                Cover::FieldSoil => (rgb(0x8f6f4c), rgb(0x7d5f40)),
                _ => (rgb(0xb3a680), rgb(0xa49870)),
            };
            paint(&name, 128, |u, v, x, y| {
                // Furrows every 1.5 m.
                let furrow = ((v * 8.0).fract() - 0.5).abs() * 2.0;
                let n = fbm(u, v, seed);
                let g = (hash2(x as i32, y as i32, seed) - 0.5) * 0.05;
                mix(a, b, furrow * 0.7 + n * 0.3).map(|k| k + g)
            })
        }
        Cover::Orchard => {
            let (soil, leaf) = (rgb(0x9a8462), rgb(0x47633a));
            paint(&name, 128, |u, v, x, y| {
                // Rows of round crowns seen from far away, 6 m apart.
                let (cu, cv) = ((u * 2.0).fract() - 0.5, (v * 2.0).fract() - 0.5);
                let d = (cu * cu + cv * cv).sqrt();
                let g = (hash2(x as i32, y as i32, seed) - 0.5) * 0.05;
                let t = 1.0 - ((d - 0.2) / 0.1).clamp(0.0, 1.0);
                mix(soil, leaf, t * 0.55).map(|k| k + g + (fbm(u, v, seed) - 0.5) * 0.06)
            })
        }
        Cover::Paving => {
            let (a, b) = (rgb(0xb4b0a8), rgb(0xa5a199));
            paint(&name, 128, |_, _, x, y| {
                // Slabs of 0.75 m with dark joints.
                let joint = (x % 16 == 0) || (y % 16 == 0);
                let slab = hash2(x as i32 / 16, y as i32 / 16, seed);
                let g = (hash2(x as i32, y as i32, seed) - 0.5) * 0.03;
                let c = mix(a, b, slab);
                c.map(|k| k * if joint { 0.86 } else { 1.0 } + g)
            })
        }
    };
    let (roughness, specular) = match c {
        Cover::Water => (0.12, 1.0),
        Cover::Paving | Cover::Concrete => (0.85, 0.8),
        _ => (0.95, 0.6),
    };
    (tex, MaterialData { name, base_color: Vec4::ONE, base_color_tex: Some(0), roughness, specular, ..Default::default() })
}

/// Road surfaces: `u` across the road (0 left edge, 1 right edge), `v`
/// along it (one tile every [`ROAD_V`] metres).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Surface {
    /// Dual carriageway side: edge lines and a broken lane line.
    Highway,
    /// Two-way road with a centre line and edge lines.
    Road,
    /// Plain asphalt.
    Street,
    /// Stone paving of pedestrian streets and promenades.
    Paved,
    /// Light footpath.
    Path,
    /// Red cycle lane.
    Cycle,
    /// Compacted earth.
    Dirt,
    /// Ballast, sleepers and two rails.
    Rail,
    /// Tram track set in paving.
    Tram,
    /// Runway: centre line and threshold-white edges.
    Runway,
    /// Concrete (taxiways, bridge sides).
    Concrete,
    /// Ponds, fountains and narrow channels drawn over the ground.
    Water,
}

pub const SURFACES: [Surface; 12] = [Surface::Highway, Surface::Road, Surface::Street, Surface::Paved, Surface::Path, Surface::Cycle, Surface::Dirt, Surface::Rail, Surface::Tram, Surface::Runway, Surface::Concrete, Surface::Water];

/// Length of one texture tile along a road (m).
pub const ROAD_V: f32 = 12.0;

pub fn surface_material(s: Surface) -> (TextureData, MaterialData) {
    let name = format!("isla_road_{s:?}").to_lowercase();
    let seed = s as u32 * 17 + 3;
    let asphalt = |u: f32, v: f32, x: u32, y: u32, dark: f32| -> [f32; 3] {
        let n = fbm(u, v, seed) * 0.05 + (hash2(x as i32, y as i32, seed) - 0.5) * 0.05;
        [dark + n, dark + n, dark + n + 0.005]
    };
    let white = [0.86, 0.86, 0.84];
    let kerb = [0.62, 0.61, 0.58];
    let tex = match s {
        Surface::Highway => paint(&name, 128, |u, v, x, y| {
            let edge = (0.03..0.055).contains(&u) || (0.945..0.97).contains(&u);
            let lane = (u - 0.5).abs() < 0.012 && v < 0.4;
            if edge || lane { white } else { asphalt(u, v, x, y, 0.27) }
        }),
        Surface::Road => paint(&name, 128, |u, v, x, y| {
            let edge = (0.03..0.05).contains(&u) || (0.95..0.97).contains(&u);
            let centre = (u - 0.5).abs() < 0.012 && v < 0.5;
            if u < 0.02 || u > 0.98 { kerb } else if edge || centre { white } else { asphalt(u, v, x, y, 0.29) }
        }),
        // A granite kerb line along both edges: the street reads as a
        // roadway between pavements.
        Surface::Street => paint(&name, 128, |u, v, x, y| if u < 0.035 || u > 0.965 { kerb } else { asphalt(u, v, x, y, 0.31) }),
        Surface::Runway => paint(&name, 128, |u, v, x, y| {
            let edge = (0.02..0.035).contains(&u) || (0.965..0.98).contains(&u);
            let centre = (u - 0.5).abs() < 0.008 && v < 0.6;
            if edge || centre { white } else { asphalt(u, v, x, y, 0.3) }
        }),
        Surface::Paved => {
            let (a, b) = (rgb(0xb9b1a2), rgb(0xa79f90));
            paint(&name, 128, |u, v, x, y| {
                let joint = x % 16 == 0 || (y + (x / 16 % 2) * 8) % 16 == 0;
                let c = mix(a, b, hash2(x as i32 / 16, (y as i32 + (x as i32 / 16 % 2) * 8) / 16, seed));
                let g = (hash2(x as i32, y as i32, seed) - 0.5) * 0.03 + (fbm(u, v, seed) - 0.5) * 0.04;
                c.map(|k| k * if joint { 0.85 } else { 1.0 } + g)
            })
        }
        Surface::Path => mottled(&name, 0xc4bca8, 0xb3aa95, 0.04, seed),
        Surface::Cycle => mottled(&name, 0x9a4a3c, 0x8c4236, 0.04, seed),
        Surface::Dirt => mottled(&name, 0xa08a66, 0x8a7654, 0.08, seed),
        Surface::Concrete => mottled(&name, 0xa7a49c, 0x99968e, 0.04, seed),
        Surface::Water => mottled(&name, 0x2f6f86, 0x2a637b, 0.0, seed),
        Surface::Rail | Surface::Tram => {
            let tram = s == Surface::Tram;
            let ballast = if tram { rgb(0x8f8c86) } else { rgb(0x77706a) };
            paint(&name, 128, |u, v, x, y| {
                let g = (hash2(x as i32, y as i32, seed) - 0.5) * 0.12;
                let rail = (u - 0.26).abs() < 0.022 || (u - 0.74).abs() < 0.022;
                let sleeper = !tram && (v * 20.0).fract() < 0.35 && (0.12..0.88).contains(&u);
                if rail {
                    [0.42, 0.40, 0.38]
                } else if sleeper {
                    [0.36 + g * 0.3, 0.33 + g * 0.3, 0.30 + g * 0.3]
                } else {
                    ballast.map(|k| k + g)
                }
            })
        }
    };
    let roughness = match s {
        Surface::Paved | Surface::Path | Surface::Concrete | Surface::Tram => 0.85,
        Surface::Dirt | Surface::Rail => 0.95,
        Surface::Water => 0.12,
        _ => 0.9,
    };
    (tex, MaterialData { name, base_color: Vec4::ONE, base_color_tex: Some(0), roughness, specular: 0.8, ..Default::default() })
}

/// The building shells' atlas: every facade style in one texture (so a
/// tile of shells is one draw), twelve style tiles in a 4 x 3 grid.
///
/// A style tile is four bays wide (3.2 m each; a wall is cut every four
/// bays so it never leaves its tile) and, from the top: four upper storeys
/// (half the tile), the ground floor (shop fronts and doors), a blank wall
/// (party walls, narrow edges) and two roof colours.
struct Style {
    wall: u32,
    frame: u32,
    shutter: u32,
    balcony: bool,
    window: (f32, f32),
    glass: u32,
    roofs: [u32; 2],
}

const TERRACOTTA: u32 = 0xa9735e;
const TERRACE: u32 = 0xb9ae9d;
const GRAVEL: u32 = 0x9a968c;
const METAL: u32 = 0x8f9698;

const STYLES: [Style; 12] = [
    // Ensanche: ochres, creams and pinks with balconies.
    Style { wall: 0xd9c39a, frame: 0xf0e8d6, shutter: 0x5d6b52, balcony: true, window: (0.34, 0.62), glass: 0x2b3038, roofs: [TERRACE, TERRACOTTA] },
    Style { wall: 0xe8dcc0, frame: 0xf6f1e4, shutter: 0x6b5a48, balcony: true, window: (0.34, 0.62), glass: 0x2b3038, roofs: [TERRACE, TERRACOTTA] },
    Style { wall: 0xd7a98c, frame: 0xf2e6d8, shutter: 0x4f5f4a, balcony: true, window: (0.32, 0.6), glass: 0x30343a, roofs: [TERRACE, 0xa06a55] },
    Style { wall: 0xc9b8a0, frame: 0xe9e2d2, shutter: 0x7a6a55, balcony: true, window: (0.36, 0.6), glass: 0x2b3038, roofs: [TERRACE, TERRACOTTA] },
    Style { wall: 0xe3cf9f, frame: 0xf4ecd8, shutter: 0x3f5a4f, balcony: true, window: (0.3, 0.64), glass: 0x2d3238, roofs: [TERRACE, TERRACOTTA] },
    // Brick blocks of the 60s and 70s.
    Style { wall: 0xa9673f, frame: 0xd8d2c6, shutter: 0xc9c2b2, balcony: false, window: (0.5, 0.45), glass: 0x39424c, roofs: [GRAVEL, 0xa7846a] },
    Style { wall: 0xb9845c, frame: 0xdcd6ca, shutter: 0xbdb6a6, balcony: true, window: (0.46, 0.5), glass: 0x39424c, roofs: [GRAVEL, TERRACE] },
    Style { wall: 0xc8c2b6, frame: 0xe6e2da, shutter: 0xa9a294, balcony: false, window: (0.52, 0.46), glass: 0x3a4650, roofs: [GRAVEL, 0x8a867c] },
    // Village houses: whitewash, small windows.
    Style { wall: 0xf1ede2, frame: 0xf6f3ea, shutter: 0x4a6a8a, balcony: false, window: (0.26, 0.42), glass: 0x2a2e34, roofs: [TERRACOTTA, 0x9d6753] },
    Style { wall: 0xe9dfc8, frame: 0xf3eee0, shutter: 0x5a7046, balcony: false, window: (0.26, 0.42), glass: 0x2a2e34, roofs: [TERRACOTTA, 0xb27e66] },
    // Modern: curtain wall and concrete grid.
    Style { wall: 0x8f9aa3, frame: 0xcfd6da, shutter: 0x8f9aa3, balcony: false, window: (0.86, 0.72), glass: 0x41566a, roofs: [GRAVEL, METAL] },
    // Warehouses: sheet metal, no windows.
    Style { wall: 0xb0b3b2, frame: 0xb0b3b2, shutter: 0x8c9090, balcony: false, window: (0.0, 0.0), glass: 0x30343a, roofs: [METAL, 0xa9a59a] },
];

pub const STYLE_COUNT: u8 = STYLES.len() as u8;
pub const STYLE_MODERN: u8 = 10;
pub const STYLE_SHED: u8 = 11;
/// World size of four bays or four storeys (one style tile).
pub const FACADE_TILE: f32 = 12.8;

const COLS: u32 = 4;
const ROWS: u32 = 3;
/// Bay (and storey) size in texels.
const BAY: u32 = 64;
const TILE_W: u32 = BAY * 4;
const TILE_H: u32 = BAY * 8;

/// Bands of a style tile as fractions of its height: `(top, bottom)`.
pub const BAND_UPPER: (f32, f32) = (0.0, 0.5);
pub const BAND_GROUND: (f32, f32) = (0.5, 0.625);
pub const BAND_BLANK: (f32, f32) = (0.625, 0.75);
pub const BAND_ROOF: [f32; 2] = [0.8125, 0.9375];

/// Atlas rectangle of style `k`: `(u0, v0, width, height)`.
pub fn style_rect(k: u8) -> (f32, f32, f32, f32) {
    let k = (k % STYLE_COUNT) as u32;
    ((k % COLS) as f32 / COLS as f32, (k / COLS) as f32 / ROWS as f32, 1.0 / COLS as f32, 1.0 / ROWS as f32)
}

/// What a facade texel is: 0 wall, 1 frame, 2 glass, 3 shutter, 4 balcony
/// slab, 5 railing, 6 sign band, 7 door.
fn facade_texel(k: u8, st: &Style, ground: bool, bay: i32, u: f32, v: f32) -> u8 {
    let (shed, modern) = (k == STYLE_SHED, k == STYLE_MODERN);
    if shed {
        // One roller door every four bays.
        return if ground && bay == 1 && (0.1..0.9).contains(&u) && v > 0.2 { 3 } else { 0 };
    }
    if ground {
        // v = 0 is the top of the storey: a sign band over a wide opening.
        if modern {
            return if v > 0.12 && (0.04..0.96).contains(&u) { 2 } else { 1 };
        }
        if (0.1..0.9).contains(&u) {
            if (0.1..0.24).contains(&v) {
                return 6;
            }
            if v >= 0.24 {
                // By bay: a shop window, a shutter down, a doorway, a window.
                let inside = (0.14..0.86).contains(&u) && v < 0.97;
                return match bay {
                    1 if inside => 3,
                    2 => {
                        if (0.3..0.7).contains(&u) && v > 0.3 {
                            7
                        } else {
                            0
                        }
                    }
                    _ if inside => 2,
                    _ => 1,
                };
            }
        }
        return 0;
    }
    let (ww, wh) = st.window;
    let (u0, u1) = (0.5 - ww * 0.5, 0.5 + ww * 0.5);
    let (v0, v1) = if st.balcony { (0.96 - wh, 0.96) } else { (0.5 - wh * 0.5, 0.5 + wh * 0.5) };
    if modern {
        return if (u0..u1).contains(&u) && (v0..v1).contains(&v) { 2 } else { 1 };
    }
    if st.balcony {
        if v > 0.955 && (u0 - 0.14..u1 + 0.14).contains(&u) {
            return 4;
        }
        if (0.66..0.955).contains(&v) && (u0 - 0.13..u1 + 0.13).contains(&u) && ((u * BAY as f32) as u32 % 3 == 0 || (0.66..0.69).contains(&v)) {
            return 5;
        }
    }
    if (u0..u1).contains(&u) && (v0..v1).contains(&v) {
        // A shutter leaf over part of the window.
        return if (u - u0) / ww < 0.28 { 3 } else { 2 };
    }
    if (u0 - 0.04..u1 + 0.04).contains(&u) && (v0 - 0.04..v1 + 0.03).contains(&v) {
        return 1;
    }
    0
}

/// The shell atlas, its glow map (lit windows, `None` by day) and the
/// material; `night` is 0 by day and 1 in full night.
pub fn shell_material(night: f32) -> (TextureData, Option<TextureData>, MaterialData) {
    let (w, h) = (TILE_W * COLS, TILE_H * ROWS);
    let mut px = vec![0u8; (w * h * 4) as usize];
    let mut gl = if night > 0.0 { vec![0u8; (w * h * 4) as usize] } else { Vec::new() };
    for (k, st) in STYLES.iter().enumerate() {
        let k = k as u8;
        let (ox, oy) = ((k as u32 % COLS) * TILE_W, (k as u32 / COLS) * TILE_H);
        let (wall, frame, shutter, glass) = (rgb(st.wall), rgb(st.frame), rgb(st.shutter), rgb(st.glass));
        let seed = k as u32 * 13 + 1;
        let (shed, modern) = (k == STYLE_SHED, k == STYLE_MODERN);
        for y in 0..TILE_H {
            for x in 0..TILE_W {
                let i = (((oy + y) * w + ox + x) * 4) as usize;
                let grain = (hash2(x as i32, y as i32, seed) - 0.5) * 0.03 + (fbm(x as f32 / TILE_W as f32, y as f32 / TILE_W as f32, seed) - 0.5) * 0.05;
                let (bx, lx) = ((x / BAY) as i32, (x % BAY) as i32);
                let u = (lx as f32 + 0.5) / BAY as f32;
                let mut lit = false;
                let c = if y < BAY * 5 {
                    // Four upper storeys, then the ground floor.
                    let ground = y >= BAY * 4;
                    let (by, ly) = ((y / BAY) as i32, (y % BAY) as i32);
                    let v = (ly as f32 + 0.5) / BAY as f32;
                    let mut t = facade_texel(k, st, ground, bx, u, v);
                    // A quarter of the windows have their blinds down.
                    if t == 2 && !ground && !modern && hash2(bx, by, seed + 5) < 0.25 {
                        t = 3;
                    }
                    lit = t == 2 && hash2(bx, by, seed + 77) < if ground { 0.45 } else { 0.3 } * night;
                    match t {
                        1 => frame.map(|c| c + grain * 0.5),
                        2 => glass.map(|c| c + (1.0 - v) * 0.05),
                        3 => shutter.map(|c| c * if ly % 3 == 0 { 0.8 } else { 1.0 }),
                        4 => frame.map(|c| c * 0.8),
                        5 => [0.13, 0.13, 0.14],
                        6 => mix(shutter, [0.2, 0.2, 0.22], 0.3 + 0.4 * hash2(bx, 0, seed + 9)),
                        7 => [0.30, 0.20, 0.13].map(|c: f32| c * if lx % 9 == 0 { 0.7 } else { 1.0 }),
                        2 if ground => mix(glass, [0.55, 0.47, 0.36], 0.35 + 0.3 * (1.0 - v)),
                        3 if ground => [0.62, 0.63, 0.64].map(|c: f32| c * if ly % 4 == 0 { 0.75 } else { 1.0 }),
                        _ => {
                            // Storey line.
                            let band = if v < 0.03 { 0.92 } else { 1.0 };
                            let sheet = if shed && lx % 8 == 0 { 0.9 } else { 1.0 };
                            wall.map(|c| c * band * sheet + grain)
                        }
                    }
                } else if y < BAY * 6 {
                    wall.map(|c| c * 0.93 + grain)
                } else {
                    // Roof colours, a little different along the tile.
                    let r = rgb(st.roofs[usize::from(y >= BAY * 7)]);
                    let tone = 0.88 + 0.24 * vnoise(x as f32 / TILE_W as f32, 0.5, 8, seed + 3);
                    r.map(|c| c * tone + grain * 0.5)
                };
                px[i..i + 4].copy_from_slice(&to8(c));
                if night > 0.0 {
                    let warm = hash2(bx, (y / BAY) as i32, seed + 78);
                    gl[i..i + 4].copy_from_slice(&to8(if lit { [1.0, 0.72 + warm * 0.2, 0.42 + warm * 0.3] } else { [0.0; 3] }));
                }
            }
        }
    }
    let tex = TextureData { name: "isla_shells".into(), width: w, height: h, rgba8: Arc::from(px), srgb: true };
    let glow = (night > 0.0).then(|| TextureData { name: "isla_shells_glow".into(), width: w, height: h, rgba8: Arc::from(gl), srgb: true });
    let mat = MaterialData {
        name: "isla_shells".into(),
        base_color: Vec4::ONE,
        base_color_tex: Some(0),
        roughness: 0.88,
        specular: 0.7,
        emissive: if glow.is_some() { Vec3::splat(0.55) } else { Vec3::ZERO },
        emissive_tex: glow.as_ref().map(|_| 1),
        ..Default::default()
    };
    (tex, glow, mat)
}

/// Mean colour (sRGB bytes) of a texture, for far-distance stand-ins.
pub fn mean_color(t: &TextureData) -> [u8; 3] {
    let n = (t.rgba8.len() / 4).max(1);
    let mut acc = [0u64; 3];
    for p in t.rgba8.chunks_exact(4) {
        for k in 0..3 {
            acc[k] += p[k] as u64;
        }
    }
    acc.map(|a| (a / n as u64) as u8)
}
