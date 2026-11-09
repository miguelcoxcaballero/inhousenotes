//! Where the island's trees stand: scattered by land cover (pine woods,
//! parks, orange groves, scrub with scattered pines, street trees), never
//! on a road, a building or water. The trees themselves are `forja_flora`
//! species, instanced (a few models for the whole island).

use forja_flora::Pruning;
use forja_math::{Vec2, Vec3};

use super::data::{IslandData, PolyKind};
use super::ground::{CELL, Cover, Ground, scan};
use super::roads::visible;

/// Occupancy raster cell (m).
const OCC: f32 = 4.0;

struct Occupancy {
    x0: f32,
    z0: f32,
    cols: usize,
    rows: usize,
    bits: Vec<u64>,
}

impl Occupancy {
    fn set(&mut self, c: usize, r: usize) {
        let i = r * self.cols + c;
        self.bits[i / 64] |= 1 << (i % 64);
    }

    fn get(&self, p: Vec2) -> bool {
        let (c, r) = (((p.x - self.x0) / OCC) as isize, ((p.y - self.z0) / OCC) as isize);
        if c < 0 || r < 0 || c as usize >= self.cols || r as usize >= self.rows {
            return true;
        }
        let i = r as usize * self.cols + c as usize;
        self.bits[i / 64] >> (i % 64) & 1 != 0
    }

    fn stamp(&mut self, p: Vec2, radius: f32) {
        let c0 = ((p.x - radius - self.x0) / OCC).floor().max(0.0) as usize;
        let c1 = (((p.x + radius - self.x0) / OCC).floor().max(0.0) as usize).min(self.cols - 1);
        let r0 = ((p.y - radius - self.z0) / OCC).floor().max(0.0) as usize;
        let r1 = (((p.y + radius - self.z0) / OCC).floor().max(0.0) as usize).min(self.rows - 1);
        for r in r0..=r1 {
            for c in c0..=c1 {
                self.set(c, r);
            }
        }
    }
}

fn hash(mut x: u64) -> u64 {
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
    x ^= x >> 33;
    x = x.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    x ^ (x >> 33)
}

fn unit(x: u64) -> f32 {
    (x >> 40) as f32 / (1u64 << 24) as f32
}

/// A tree to plant: species, pruning, position and whether it is a palm.
pub struct Tree {
    pub species: &'static str,
    pub pruning: Pruning,
    pub pos: Vec3,
    pub palm: bool,
}

/// Scatters the island's trees; `density` scales every count (1 = the
/// full island, about 250 000 trees).
pub fn scatter(d: &IslandData, g: &Ground, density: f32) -> Vec<Tree> {
    let (cols, rows) = (((g.cols - 1) as f32 * CELL / OCC) as usize, ((g.rows - 1) as f32 * CELL / OCC) as usize);
    let mut occ = Occupancy { x0: g.x0, z0: g.z0, cols, rows, bits: vec![0; (cols * rows).div_ceil(64)] };
    for l in d.lines.iter().filter(|l| visible(l)) {
        let radius = l.width * 0.5 + 2.0;
        for w in l.pts.windows(2) {
            let k = (w[0].distance(w[1]) / OCC).ceil().max(1.0) as usize;
            for s in 0..=k {
                occ.stamp(w[0].lerp(w[1], s as f32 / k as f32), radius);
            }
        }
    }
    for p in d.polys.iter().filter(|p| p.kind == PolyKind::Building) {
        let mut runs = Vec::new();
        scan(&p.rings[..1], g.x0, g.z0, 0.5, 0.5, OCC, cols, rows, |r, c0, c1| runs.push((r, c0, c1)));
        for (r, c0, c1) in runs {
            for c in c0.saturating_sub(1)..(c1 + 1).min(cols) {
                occ.set(c, r);
                if r > 0 {
                    occ.set(c, r - 1);
                }
                if r + 1 < rows {
                    occ.set(c, r + 1);
                }
            }
        }
        for q in &p.rings[0] {
            occ.stamp(*q, 2.0);
        }
    }
    let mut out = Vec::new();
    let cc = g.cols - 1;
    for r in 0..g.rows - 1 {
        for c in 0..cc {
            for t in 0..2u64 {
                let cover = Cover::from_u8(g.cover[(r * cc + c) * 2 + t as usize]);
                // Trees per triangle (128 m2).
                let n = match cover {
                    Cover::Forest => 0.2,
                    Cover::Park => 0.12,
                    Cover::Orchard => 0.035,
                    Cover::Grass => 0.012,
                    Cover::Scrub => 0.008,
                    Cover::Paving => 0.012,
                    Cover::Wetland => 0.003,
                    _ => 0.0,
                } * density;
                if n <= 0.0 {
                    continue;
                }
                let seed = hash((r as u64) << 32 | (c as u64) << 2 | t);
                let count = n.floor() as u32 + u32::from(unit(seed) < n.fract());
                for k in 0..count {
                    let h = hash(seed ^ (k as u64 + 1).wrapping_mul(0x9e37_79b9_7f4a_7c15));
                    // A point of the triangle.
                    let (mut u, mut v) = (unit(h), unit(hash(h)));
                    if u + v > 1.0 {
                        (u, v) = (1.0 - u, 1.0 - v);
                    }
                    let (fx, fz) = if t == 0 { (u, u + v) } else { (u + v, u) };
                    let (fx, fz) = (fx.clamp(0.02, 0.98), fz.clamp(0.02, 0.98));
                    let p = Vec2::new(g.x0 + (c as f32 + fx) * CELL, g.z0 + (r as f32 + fz) * CELL);
                    if occ.get(p) || g.cover_at(p.x, p.y) != cover {
                        continue;
                    }
                    let y = g.height(p.x, p.y);
                    let pick = unit(hash(h ^ 0xabcd));
                    // Stands keep one species over ~60 m.
                    let stand = unit(hash(((p.x / 60.0).floor() as i64 as u64) << 32 ^ (p.y / 60.0).floor() as i64 as u64));
                    let (species, pruning, palm) = match cover {
                        Cover::Forest => (if stand < 0.75 || pick < 0.5 { "pino_pinonero" } else if pick < 0.8 { "olivo" } else { "cipres" }, Pruning::Natural, false),
                        Cover::Park => match (stand * 5.0) as u32 {
                            0 => ("pino_pinonero", Pruning::Natural, false),
                            1 => ("platanus", Pruning::Lifted, false),
                            2 if g.coast_at(p.x, p.y) < 900.0 => ("phoenix", Pruning::Trimmed, true),
                            2 => ("olivo", Pruning::Natural, false),
                            3 => ("naranjo", Pruning::Lollipop, false),
                            _ => ("platanus", Pruning::Lifted, false),
                        },
                        Cover::Orchard => ("naranjo", Pruning::Lollipop, false),
                        Cover::Paving => (if stand < 0.5 { "platanus" } else { "naranjo" }, if stand < 0.5 { Pruning::Lifted } else { Pruning::Lollipop }, false),
                        Cover::Wetland => ("phoenix", Pruning::Trimmed, true),
                        _ => (if pick < 0.6 { "pino_pinonero" } else { "olivo" }, Pruning::Natural, false),
                    };
                    out.push(Tree { species, pruning, pos: Vec3::new(p.x, y - 0.05, p.y), palm });
                }
            }
        }
    }
    out
}
