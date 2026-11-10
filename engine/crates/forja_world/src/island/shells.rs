//! Building shells: every footprint of the island extruded to its height
//! with tiling facade textures (windows, balconies, shop fronts) and a
//! roof. A shell is a few dozen triangles, so the whole island's 43 000
//! buildings are always drawn; near the player a tile's shells give way to
//! the detailed facades (see `sandbox::game::island`).

use forja_assets::MeshData;
use forja_math::{Vec2, Vec3, Vec4};

use super::data::{Poly, ring_area, ring_centroid};
use super::ground::Ground;
use super::tex::{FACADE_TILE, STYLE_MODERN, STYLE_SHED};

/// Side of a building tile (m): the unit of shells, detail and collision.
pub const BTILE: f32 = 256.0;
pub const STOREY: f32 = 3.2;

/// One building of the island, ready to mesh.
#[derive(Clone, Debug)]
pub struct Building {
    /// Index into `IslandData::polys`.
    pub poly: u32,
    pub centre: Vec2,
    /// Ground floor level (the highest ground under the footprint) and the
    /// lowest ground under it.
    pub floor: f32,
    pub low: f32,
    /// Height of the eave above `floor`.
    pub height: f32,
    pub storeys: u32,
    pub style: u8,
    pub roof: u8,
    pub seed: u32,
    /// The landmark site that replaces this building's shell (it still
    /// collides as its prism).
    pub site: Option<u16>,
}

fn hash(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^ (x >> 16)
}

impl Building {
    pub fn new(index: usize, p: &Poly, g: &Ground) -> Building {
        let centre = ring_centroid(&p.rings[0]);
        let (mut floor, mut low) = (f32::MIN, f32::MAX);
        for q in p.rings[0].iter().chain(std::iter::once(&centre)) {
            let h = g.height(q.x, q.y);
            floor = floor.max(h);
            low = low.min(h);
        }
        let height = if p.height > 1.0 { p.height } else { 6.4 };
        let storeys = (height / STOREY).round().max(1.0) as u32;
        let seed = hash(index as u32 ^ 0x51ed_270b);
        let area = ring_area(&p.rings[0]).abs();
        let central = centre.length() < 1700.0;
        let pick = seed % 100;
        let style = if p.flags & 1 != 0 {
            if area > 2500.0 { STYLE_MODERN } else { STYLE_SHED }
        } else if storeys <= 3 && area > 1400.0 {
            STYLE_SHED
        } else if storeys <= 2 {
            8 + (seed >> 8) as u8 % 2
        } else if storeys >= 11 {
            if pick < 45 { STYLE_MODERN } else { 5 + (seed >> 8) as u8 % 3 }
        } else if pick < if central { 88 } else { 45 } {
            (seed >> 8) as u8 % 5
        } else {
            5 + (seed >> 8) as u8 % 3
        };
        let roof = ((seed >> 12) % 3 == 0) as u8;
        Building { poly: index as u32, centre, floor, low, height, storeys, style, roof, seed, site: None }
    }

    pub fn tile(&self) -> (i32, i32) {
        ((self.centre.x / BTILE).floor() as i32, (self.centre.y / BTILE).floor() as i32)
    }

    /// Whether the ground floor is a shop front (taller, its own texture).
    fn shops(&self) -> bool {
        self.storeys >= 3 || self.style == STYLE_SHED
    }

    fn ground_height(&self) -> f32 {
        if self.style == STYLE_SHED {
            self.height.min(4.5)
        } else if self.shops() {
            (self.height - (self.storeys - 1) as f32 * 3.0).clamp(3.2, 4.4)
        } else {
            0.0
        }
    }
}

fn quad(m: &mut MeshData, p: [Vec3; 4], uv: [Vec2; 4], n: Vec3) {
    let b = m.positions.len() as u32;
    // Tangent along +u (p0 -> p1).
    let t = (p[1] - p[0]).normalize_or(Vec3::X);
    for k in 0..4 {
        m.positions.push(p[k]);
        m.normals.push(n);
        m.uvs.push(uv[k]);
        m.tangents.push(t.extend(1.0));
    }
    m.indices.extend([b, b + 1, b + 2, b, b + 2, b + 3]);
}

/// Appends the shell of `b` (walls of every ring, roof) to `m`, textured
/// from the shell atlas ([`super::tex::shell_material`]).
fn shell(m: &mut MeshData, b: &Building, p: &Poly) {
    use super::tex::{BAND_BLANK, BAND_GROUND, BAND_ROOF, BAND_UPPER, style_rect};
    let (su, sv, sw, sh) = style_rect(b.style);
    // Half a texel inside each band, so neighbours do not bleed in.
    let eps = 0.002;
    let band = |(t, bt): (f32, f32), f: f32| sv + (t + eps + (bt - t - 2.0 * eps) * f.clamp(0.0, 1.0)) * sh;
    let gh = b.ground_height();
    let top = b.floor + b.height;
    let bottom = b.low - 0.8;
    let split = if gh > 0.0 && top - (b.floor + gh) > 1.0 { b.floor + gh } else if gh > 0.0 { top } else { bottom };
    let upper_storeys = if gh > 0.0 { b.storeys.saturating_sub(1).max(1) } else { b.storeys };
    for (ri, ring) in p.rings.iter().enumerate() {
        let n = ring.len();
        // Outward side: right of the edge for a counter-clockwise outer
        // ring; holes face the other way.
        let sign = if (ring_area(ring) > 0.0) == (ri == 0) { 1.0 } else { -1.0 };
        for i in 0..n {
            let (a, c) = (ring[i], ring[(i + 1) % n]);
            let len = a.distance(c);
            if len < 0.08 {
                continue;
            }
            let d = (c - a) / len;
            let out = Vec3::new(d.y, 0.0, -d.x) * sign;
            // Seen from outside the wall runs left to right.
            let (l, r) = if sign > 0.0 { (c, a) } else { (a, c) };
            let at = |t: f32, y: f32| {
                let q = l.lerp(r, t);
                Vec3::new(q.x, y, q.y)
            };
            if len < 2.4 {
                let (u0, u1) = (su + sw * 0.1, su + sw * (0.1 + (len / FACADE_TILE).min(0.8)));
                let (v0, v1) = (band(BAND_BLANK, 1.0), band(BAND_BLANK, 0.0));
                quad(m, [at(0.0, bottom), at(1.0, bottom), at(1.0, top), at(0.0, top)], [Vec2::new(u0, v0), Vec2::new(u1, v0), Vec2::new(u1, v1), Vec2::new(u0, v1)], out);
                continue;
            }
            // Whole bays, at most four per quad (a style tile).
            let bays = (len / STOREY).round().max(1.0) as u32;
            let mut done = 0;
            while done < bays {
                let nb = (bays - done).min(4);
                let first = hash(b.seed ^ (i as u32) << 8 ^ done) % (5 - nb);
                let (t0, t1) = (done as f32 / bays as f32, (done + nb) as f32 / bays as f32);
                let (u0, u1) = (su + sw * (first as f32 * 0.25 + eps), su + sw * ((first + nb) as f32 * 0.25 - eps));
                let mut wall = |y0: f32, y1: f32, v0: f32, v1: f32| {
                    quad(m, [at(t0, y0), at(t1, y0), at(t1, y1), at(t0, y1)], [Vec2::new(u0, v0), Vec2::new(u1, v0), Vec2::new(u1, v1), Vec2::new(u0, v1)], out);
                };
                if gh > 0.0 {
                    // Ground floor: the storey from the floor up; below the
                    // floor (sloping ground) the blank band under it.
                    let under = ((b.floor - bottom) / gh * (BAND_GROUND.1 - BAND_GROUND.0)).min(BAND_BLANK.1 - BAND_BLANK.0 - 2.0 * eps);
                    wall(bottom, split, sv + (BAND_GROUND.1 - eps + under) * sh, band(BAND_GROUND, 0.0));
                }
                if split < top {
                    // Upper storeys, four per quad from the bottom up.
                    let per = (top - split.max(b.floor)) / upper_storeys as f32;
                    let mut s0 = 0;
                    while s0 < upper_storeys {
                        let ns = (upper_storeys - s0).min(4);
                        let y0 = if s0 == 0 { split } else { split.max(b.floor) + s0 as f32 * per };
                        let y1 = split.max(b.floor) + (s0 + ns) as f32 * per;
                        wall(y0, y1, band(BAND_UPPER, ns as f32 * 0.25), band(BAND_UPPER, 0.0));
                        s0 += ns;
                    }
                }
                done += nb;
            }
        }
    }
    // Roof: one colour of the style's roof band per building.
    let (pts, tris) = forja_city::landmark::earcut::triangulate_with_holes(&p.rings[0], &p.rings[1..]);
    if !tris.is_empty() {
        let uv = Vec2::new(su + sw * (0.05 + 0.9 * (hash(b.seed ^ 0x77) % 1000) as f32 / 1000.0), sv + BAND_ROOF[(b.roof % 2) as usize] * sh);
        let base = m.positions.len() as u32;
        for q in &pts {
            m.positions.push(Vec3::new(q.x, top, q.y));
            m.normals.push(Vec3::Y);
            m.uvs.push(uv);
            m.tangents.push(Vec4::new(1.0, 0.0, 0.0, 1.0));
        }
        for t in &tris {
            // Counter-clockwise in (x, z) is clockwise seen from above.
            m.indices.extend([base + t[0], base + t[2], base + t[1]]);
        }
    }
}

/// The shell mesh of a set of buildings (empty without any).
pub fn tile_mesh(buildings: &[Building], polys: &[Poly]) -> MeshData {
    let mut m = MeshData::default();
    for b in buildings {
        shell(&mut m, b, &polys[b.poly as usize]);
    }
    m.aabb = forja_math::Aabb::from_points(m.positions.iter().copied());
    m
}

/// Collision of a set of buildings: walls and roofs as triangles.
pub fn tile_collider(buildings: &[Building], polys: &[Poly]) -> (Vec<Vec3>, Vec<[u32; 3]>) {
    let (mut verts, mut tris) = (Vec::new(), Vec::new());
    for b in buildings {
        let p = &polys[b.poly as usize];
        let (top, bottom) = (b.floor + b.height, b.low - 0.8);
        for ring in &p.rings {
            let n = ring.len();
            let base = verts.len() as u32;
            for q in ring {
                verts.push(Vec3::new(q.x, bottom, q.y));
                verts.push(Vec3::new(q.x, top, q.y));
            }
            for i in 0..n as u32 {
                let j = (i + 1) % n as u32;
                let (a0, a1, b0, b1) = (base + i * 2, base + i * 2 + 1, base + j * 2, base + j * 2 + 1);
                tris.push([a0, b0, b1]);
                tris.push([a0, b1, a1]);
            }
        }
        let (pts, rt) = forja_city::landmark::earcut::triangulate_with_holes(&p.rings[0], &p.rings[1..]);
        let base = verts.len() as u32;
        verts.extend(pts.iter().map(|q| Vec3::new(q.x, top, q.y)));
        tris.extend(rt.iter().map(|t| [base + t[0], base + t[2], base + t[1]]));
    }
    (verts, tris)
}
