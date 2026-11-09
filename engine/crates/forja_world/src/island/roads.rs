//! Road meshes of the island: every road, path, track and railway as a
//! ribbon lying on the ground (the ground is levelled along the roads, see
//! [`Ground`]), bridges as decks between their banks.
//!
//! Ribbons are cut into the terrain's 512 m tiles, one primitive per
//! surface, in two sets: the main roads (always drawn) and the streets and
//! paths (drawn within a kilometre or so). Ponds, fountains and narrow
//! channels are drawn the same way, as sheets over the ground.

use std::collections::HashMap;

use forja_assets::MeshData;
use forja_math::{Vec2, Vec3, Vec4};

use super::data::{Line, Poly, RoadClass};
use super::ground::{Ground, TILE};
use super::tex::{ROAD_V, Surface};

/// Surface and height above the ground of a road class (higher classes
/// lie over lower ones where they cross).
pub fn style(class: RoadClass) -> (Surface, f32) {
    use RoadClass::*;
    match class {
        Motorway | Trunk => (Surface::Highway, 0.17),
        MotorwayLink | TrunkLink => (Surface::Street, 0.155),
        Primary => (Surface::Road, 0.14),
        Secondary => (Surface::Road, 0.13),
        Tertiary => (Surface::Road, 0.12),
        PrimaryLink | SecondaryLink | TertiaryLink => (Surface::Street, 0.11),
        Residential | Unclassified | Busway | Raceway => (Surface::Street, 0.10),
        LivingStreet => (Surface::Street, 0.095),
        Service => (Surface::Street, 0.09),
        Pedestrian | Promenade => (Surface::Paved, 0.07),
        Cycleway => (Surface::Cycle, 0.06),
        Footway | Path | Steps => (Surface::Path, 0.05),
        Track => (Surface::Dirt, 0.045),
        Rail | Subway => (Surface::Rail, 0.18),
        Tram => (Surface::Tram, 0.15),
        Runway => (Surface::Runway, 0.13),
        Taxiway => (Surface::Concrete, 0.105),
    }
}

/// Main roads are drawn at any distance.
fn major(class: RoadClass) -> bool {
    use RoadClass::*;
    matches!(class, Motorway | Trunk | MotorwayLink | TrunkLink | Primary | Secondary | Tertiary | Runway | Taxiway | Rail)
}

/// Water bodies smaller than this (m2) are drawn as sheets over the
/// ground; larger ones are part of the ground itself (see [`Ground`]).
pub const SHEET_MAX: f32 = 60_000.0;
/// Water bodies from this size (m2) lower the ground under them.
pub const BASIN_MIN: f32 = 12_000.0;

/// Whether a line is drawn (tunnels and the metro are underground).
pub fn visible(l: &Line) -> bool {
    !l.tunnel() && l.class != RoadClass::Subway
}

#[derive(Default)]
struct Part {
    mesh: MeshData,
}

/// The road meshes of one terrain tile.
#[derive(Default)]
pub struct RoadTile {
    /// Main roads, railways, the runway and water sheets.
    pub major: Vec<(Surface, MeshData)>,
    /// Streets, paths and tracks.
    pub minor: Vec<(Surface, MeshData)>,
}

#[derive(Default)]
pub struct Roads {
    /// Meshes per terrain tile `(tx, tz)` (world position / 512 m).
    pub tiles: HashMap<(i32, i32), RoadTile>,
    /// Collision of the bridge decks and their parapets.
    pub col_verts: Vec<Vec3>,
    pub col_tris: Vec<[u32; 3]>,
    pub triangles: usize,
}

fn key(p: Vec2) -> (i32, i32) {
    ((p.x * 4.0).round() as i32, (p.y * 4.0).round() as i32)
}

/// Deck height of the bridges: each bridge (lines joined end to end) spans
/// between its banks, the ends where a road on the ground arrives.
struct Bridges {
    /// Anchors `(position, height)` per bridge line index.
    anchors: HashMap<usize, std::sync::Arc<Vec<(Vec2, f32)>>>,
}

impl Bridges {
    fn new(lines: &[Line], g: &Ground) -> Self {
        let mut ends: HashMap<(i32, i32), (Vec<usize>, bool)> = HashMap::new();
        for (i, l) in lines.iter().enumerate() {
            if !visible(l) {
                continue;
            }
            if l.bridge() {
                for p in [l.pts[0], *l.pts.last().expect("points")] {
                    ends.entry(key(p)).or_default().0.push(i);
                }
            }
        }
        for l in lines.iter().filter(|l| visible(l) && !l.bridge()) {
            for p in &l.pts {
                if let Some(e) = ends.get_mut(&key(*p)) {
                    e.1 = true;
                }
            }
        }
        // Union the lines that share an end.
        let mut parent: HashMap<usize, usize> = HashMap::new();
        fn find(parent: &mut HashMap<usize, usize>, i: usize) -> usize {
            let p = *parent.entry(i).or_insert(i);
            if p == i {
                return i;
            }
            let r = find(parent, p);
            parent.insert(i, r);
            r
        }
        for (ids, _) in ends.values() {
            for w in ids.windows(2) {
                let (a, b) = (find(&mut parent, w[0]), find(&mut parent, w[1]));
                if a != b {
                    parent.insert(a, b);
                }
            }
            if let Some(&i) = ids.first() {
                find(&mut parent, i);
            }
        }
        let mut groups: HashMap<usize, Vec<(Vec2, f32)>> = HashMap::new();
        let mut fallback: HashMap<usize, Vec<(Vec2, f32)>> = HashMap::new();
        for (i, l) in lines.iter().enumerate() {
            if !l.bridge() || !visible(l) {
                continue;
            }
            let root = find(&mut parent, i);
            for p in [l.pts[0], *l.pts.last().expect("points")] {
                let (ids, grounded) = &ends[&key(p)];
                let a = (p, g.height(p.x, p.y));
                if *grounded || ids.len() == 1 {
                    groups.entry(root).or_default().push(a);
                }
                fallback.entry(root).or_default().push(a);
            }
        }
        let mut shared: HashMap<usize, std::sync::Arc<Vec<(Vec2, f32)>>> = HashMap::new();
        let mut anchors = HashMap::new();
        for (i, l) in lines.iter().enumerate() {
            if !l.bridge() || !visible(l) {
                continue;
            }
            let root = find(&mut parent, i);
            let a = shared.entry(root).or_insert_with(|| std::sync::Arc::new(groups.remove(&root).or_else(|| fallback.remove(&root)).unwrap_or_default())).clone();
            anchors.insert(i, a);
        }
        Bridges { anchors }
    }

    /// Deck height at `p` of bridge line `i`.
    fn height(&self, i: usize, p: Vec2, g: &Ground) -> f32 {
        let ground = g.height(p.x, p.y);
        let Some(a) = self.anchors.get(&i).filter(|a| !a.is_empty()) else { return ground };
        let (mut sum, mut w) = (0.0, 0.0);
        for (q, h) in a.iter() {
            let d2 = q.distance_squared(p);
            if d2 < 0.01 {
                return h.max(ground);
            }
            sum += h / d2;
            w += 1.0 / d2;
        }
        (sum / w).max(ground)
    }
}

impl Roads {
    pub fn build(lines: &[Line], polys: &[Poly], g: &Ground) -> Roads {
        let bridges = Bridges::new(lines, g);
        let mut parts: HashMap<((i32, i32), Surface, bool), Part> = HashMap::new();
        let mut roads = Roads::default();
        // Junction discs: the widest road arriving at each line end.
        let mut caps: HashMap<(i32, i32), (f32, usize, Vec2)> = HashMap::new();
        for (li, l) in lines.iter().enumerate() {
            if !visible(l) {
                continue;
            }
            let (surface, lift) = style(l.class);
            let imp = major(l.class);
            let bridge = l.bridge();
            let half = l.width * 0.5;
            if !bridge && l.width >= 5.0 && !l.class.is_rail() {
                for p in [l.pts[0], *l.pts.last().expect("points")] {
                    let e = caps.entry(key(p)).or_insert((0.0, li, p));
                    if l.width > e.0 {
                        *e = (l.width, li, p);
                    }
                }
            }
            // Densify where the ground is not flat, so the ribbon follows it.
            let mut pts: Vec<Vec2> = Vec::with_capacity(l.pts.len());
            for w in l.pts.windows(2) {
                let (a, b) = (w[0], w[1]);
                let len = a.distance(b);
                let step = if bridge {
                    10.0
                } else {
                    let m = (a + b) * 0.5;
                    let (ha, hb, hm) = (g.height(a.x, a.y), g.height(b.x, b.y), g.height(m.x, m.y));
                    let q = (a * 3.0 + b) * 0.25;
                    let hq = g.height(q.x, q.y);
                    if (ha - hb).abs() < 0.03 && (hm - ha).abs() < 0.03 && (hq - ha).abs() < 0.03 { f32::MAX } else { 6.0 }
                };
                let k = (len / step).ceil().max(1.0) as usize;
                for s in 0..k {
                    pts.push(a.lerp(b, s as f32 / k as f32));
                }
            }
            pts.push(*l.pts.last().expect("points"));
            let n = pts.len();
            let height = |p: Vec2| -> f32 { if bridge { bridges.height(li, p, g) + lift + 0.1 } else { g.height(p.x, p.y) + lift } };
            // Mitred sides.
            let dir = |i: usize| (pts[(i + 1).min(n - 1)] - pts[i.min(n - 2)]).normalize_or(Vec2::X);
            let mut prev: Option<((i32, i32), u32, u32)> = None;
            let mut along = 0.0f32;
            let mut side_prev: Option<(Vec3, Vec3)> = None;
            for i in 0..n {
                let (d0, d1) = (dir(i.saturating_sub(1)), dir(i));
                let nrm = (Vec2::new(d0.y, -d0.x) + Vec2::new(d1.y, -d1.x)).normalize_or(Vec2::new(d1.y, -d1.x));
                let miter = (1.0 / nrm.dot(Vec2::new(d1.y, -d1.x)).max(0.5)).min(2.0);
                if i > 0 {
                    along += pts[i].distance(pts[i - 1]);
                }
                let (l2, r2) = (pts[i] - nrm * half * miter, pts[i] + nrm * half * miter);
                let (hl, hr) = if bridge {
                    let h = height(pts[i]);
                    (h, h)
                } else {
                    (height(l2), height(r2))
                };
                let (pl, pr) = (Vec3::new(l2.x, hl, l2.y), Vec3::new(r2.x, hr, r2.y));
                // The quad from the previous cross-section belongs to the
                // tile of its middle.
                let mid = if i > 0 { (pts[i] + pts[i - 1]) * 0.5 } else { pts[i] };
                let tile = ((mid.x / TILE).floor() as i32, (mid.y / TILE).floor() as i32);
                let part = parts.entry((tile, surface, imp)).or_default();
                let v = along / ROAD_V;
                let push = |part: &mut Part, p: Vec3, u: f32, v: f32| -> u32 {
                    let k = part.mesh.positions.len() as u32;
                    part.mesh.positions.push(p);
                    part.mesh.normals.push(Vec3::Y);
                    part.mesh.uvs.push(Vec2::new(u, v));
                    part.mesh.tangents.push(Vec4::new(1.0, 0.0, 0.0, 1.0));
                    k
                };
                if i > 0 {
                    let (a, b) = match prev {
                        Some((t, a, b)) if t == tile => (a, b),
                        _ => {
                            // Previous cross-section again, in this tile.
                            let (pl0, pr0) = side_prev.expect("previous side");
                            let v0 = (along - pts[i].distance(pts[i - 1])) / ROAD_V;
                            (push(part, pl0, 0.0, v0), push(part, pr0, 1.0, v0))
                        }
                    };
                    let (c, d) = (push(part, pl, 0.0, v), push(part, pr, 1.0, v));
                    // Left is -normal: (a, b) = (left, right) of the previous
                    // section; counter-clockwise seen from above.
                    let idx = [a, c, d, a, d, b];
                    part.mesh.indices.extend(idx);
                    prev = Some((tile, c, d));
                    if bridge {
                        let (pl0, pr0) = side_prev.expect("previous side");
                        let conc = parts.entry((tile, Surface::Concrete, imp)).or_default();
                        roads.deck(conc, [pl0, pl, pr, pr0]);
                    }
                }
                side_prev = Some((pl, pr));
            }
        }
        for (_, (w, li, p)) in caps {
            let l = &lines[li];
            let (surface, lift) = style(l.class);
            let tile = ((p.x / TILE).floor() as i32, (p.y / TILE).floor() as i32);
            let part = parts.entry((tile, surface, major(l.class))).or_default();
            let base = part.mesh.positions.len() as u32;
            let r = w * 0.5;
            let c = Vec3::new(p.x, g.height(p.x, p.y) + lift - 0.004, p.y);
            part.mesh.positions.push(c);
            for k in 0..8 {
                let a = k as f32 * std::f32::consts::TAU / 8.0;
                let q = p + Vec2::new(a.cos(), a.sin()) * r;
                part.mesh.positions.push(Vec3::new(q.x, g.height(q.x, q.y) + lift - 0.004, q.y));
            }
            for k in 0..9 {
                part.mesh.normals.push(Vec3::Y);
                // Plain asphalt of the texture, away from the markings.
                part.mesh.uvs.push(Vec2::new(0.25, 0.7 + k as f32 * 0.01));
                part.mesh.tangents.push(Vec4::new(1.0, 0.0, 0.0, 1.0));
            }
            for k in 0..8u32 {
                let idx = [base, base + 1 + (k + 1) % 8, base + 1 + k];
                part.mesh.indices.extend(idx);
            }
        }
        // Small water: ponds, fountains, docks and channels narrower than
        // the ground's cells, as sheets just over the ground.
        for p in polys.iter().filter(|p| p.kind.is_water() && super::data::ring_area(&p.rings[0]).abs() < SHEET_MAX) {
            let (pts, tris) = forja_city::landmark::earcut::triangulate_with_holes(&p.rings[0], &p.rings[1..]);
            if tris.is_empty() {
                continue;
            }
            let c = super::data::ring_centroid(&p.rings[0]);
            let tile = ((c.x / TILE).floor() as i32, (c.y / TILE).floor() as i32);
            let part = parts.entry((tile, Surface::Water, true)).or_default();
            let base = part.mesh.positions.len() as u32;
            for q in &pts {
                part.mesh.positions.push(Vec3::new(q.x, g.height(q.x, q.y) + 0.03, q.y));
                part.mesh.normals.push(Vec3::Y);
                part.mesh.uvs.push(*q / 6.0);
                part.mesh.tangents.push(Vec4::new(1.0, 0.0, 0.0, 1.0));
            }
            for t in &tris {
                part.mesh.indices.extend([base + t[0], base + t[2], base + t[1]]);
            }
        }
        for ((tile, surface, main), mut part) in parts {
            if part.mesh.indices.is_empty() {
                continue;
            }
            roads.triangles += part.mesh.indices.len() / 3;
            part.mesh.aabb = forja_math::Aabb::from_points(part.mesh.positions.iter().copied());
            let t = roads.tiles.entry(tile).or_default();
            if main { t.major.push((surface, part.mesh)) } else { t.minor.push((surface, part.mesh)) }
        }
        for t in roads.tiles.values_mut() {
            t.major.sort_by_key(|(s, _)| *s);
            t.minor.sort_by_key(|(s, _)| *s);
        }
        roads
    }

    /// Sides, underside lip and parapets of a bridge deck quad
    /// `[left0, left1, right1, right0]`, with their collision.
    fn deck(&mut self, part: &mut Part, q: [Vec3; 4]) {
        const THICK: f32 = 0.7;
        const RAIL: f32 = 1.0;
        let quad = |part: &mut Part, p: [Vec3; 4]| {
            let n = (p[1] - p[0]).cross(p[3] - p[0]).normalize_or(Vec3::Y);
            let b = part.mesh.positions.len() as u32;
            let len = p[0].distance(p[1]);
            let hgt = p[0].distance(p[3]);
            for (k, v) in p.iter().enumerate() {
                part.mesh.positions.push(*v);
                part.mesh.normals.push(n);
                part.mesh.uvs.push(Vec2::new(if k == 1 || k == 2 { len / 4.0 } else { 0.0 }, if k >= 2 { hgt / 4.0 } else { 0.0 }));
                part.mesh.tangents.push(Vec4::new(1.0, 0.0, 0.0, 1.0));
            }
            let idx = [b, b + 1, b + 2, b, b + 2, b + 3];
            part.mesh.indices.extend(idx);
        };
        let down = Vec3::Y * THICK;
        let up = Vec3::Y * RAIL;
        // Left side (facing left of the travel direction) and right side.
        for (a, b) in [(q[1], q[0]), (q[3], q[2])] {
            quad(part, [a - down, b - down, b + up, a + up]);
            quad(part, [b - down, a - down, a + up, b + up]);
        }
        // Collision: the deck and both parapets.
        let base = self.col_verts.len() as u32;
        self.col_verts.extend([q[0], q[1], q[2], q[3], q[0] + up, q[1] + up, q[2] + up, q[3] + up]);
        self.col_tris.extend([[base, base + 1, base + 2], [base, base + 2, base + 3], [base, base + 1, base + 5], [base, base + 5, base + 4], [base + 3, base + 2, base + 6], [base + 3, base + 6, base + 7]]);
    }
}
