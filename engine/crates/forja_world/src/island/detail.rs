//! Detailed buildings and street dressing of the island near the player.
//!
//! The island draws every building as a shell ([`super::shells`]). Around
//! the player the game swaps the shells of 64 m *cells* (a quarter of a
//! shell tile's side) for what this module builds on a worker thread:
//!
//! * every footprint of the cell as a `forja_city` building lot: its edges
//!   classified from the island's own data as street fronts (a road in
//!   front, before any other building), party walls (another footprint
//!   against it), courtyard walls (open, seen from a street) or blind walls
//!   (the inside of a block, seen from nowhere: a blank wall), its storeys
//!   from the island's heights, its style from the district;
//! * the street around it: raised pavements with kerbs along the island's
//!   roads (the road surface itself is the island's and is not touched),
//!   parking lanes, lamps, trees, benches, bins, containers and traffic
//!   lights, with their night lights.
//!
//! Buildings that cannot be seen from any street, tiny outbuildings and
//! footprints that do not make a valid lot keep their shell (returned as a
//! small shell mesh of the cell), as does whatever exceeds the cell's
//! triangle budget.
//!
//! Everything is deterministic by cell: the same cell builds the same
//! content every time it streams in.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use forja_assets::SceneData;
use forja_city::building::{self, CityOut, ModuleLib};
use forja_city::furniture::{self, Aspect};
use forja_city::geom::Parts;
use forja_city::kit::{Kit, Mat, assemble_roles, block_shadow_role};
use forja_city::lot::{EdgeKind, Front, FrontRules, Lot, Rhythm, SKIRT};
use forja_city::street_waste::{self, BinModel};
use forja_city::{CityLight, Style, StyleLibrary, nightlight};
use forja_math::{Mat4, Quat, Vec2, Vec3};

use super::data::{PolyKind, RoadClass, point_in_ring, ring_area};
use super::shells::{self, BTILE, Building};
use super::{Island, Materials};

/// Side of a detail cell (m): a quarter of a shell tile's side.
pub const DCELL: f32 = BTILE / 4.0;
/// Triangle budget of a cell (walls, roofs and module instances at their
/// finest level); buildings beyond it keep their shell.
pub const CELL_TRIANGLES: usize = 1_500_000;
/// Block meshes from this many triangles per material get LOD chains.
const BLOCK_LOD_MIN: usize = 3000;
/// A footprint whose ground falls this much (m) is cut into level slices,
/// each falling about [`SLICE_DROP`] and at least [`SLICE_MIN`] m wide.
const SLOPE_DROP: f32 = 1.5;
const SLICE_DROP: f32 = 1.1;
const SLICE_MIN: f32 = 4.5;
/// How far around a cell the neighbours (footprints, roads) are read.
const MARGIN: f32 = 90.0;

pub fn cell_of(p: Vec2) -> (i32, i32) {
    ((p.x / DCELL).floor() as i32, (p.y / DCELL).floor() as i32)
}

/// World bounds `(min, max)` of a cell in `(x, z)`.
pub fn cell_bounds(key: (i32, i32)) -> (Vec2, Vec2) {
    let lo = Vec2::new(key.0 as f32 * DCELL, key.1 as f32 * DCELL);
    (lo, lo + Vec2::splat(DCELL))
}

/// The shell tile a cell belongs to.
pub fn tile_of_cell(key: (i32, i32)) -> (i32, i32) {
    (key.0.div_euclid(4), key.1.div_euclid(4))
}

/// The districts the styles follow.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Zone {
    OldTown,
    Eixample,
    Russafa,
    Cabanyal,
    Periphery,
    Village,
    Industrial,
}

/// The district of a point of the island (never [`Zone::Industrial`],
/// which is decided per building). World metres, origin at the Plaça de
/// l'Ajuntament, `x` east, `y` south.
pub fn zone_at(p: Vec2) -> Zone {
    let ellipse = |c: Vec2, r: Vec2| ((p - c) / r).length_squared() < 1.0;
    // Ciutat Vella: inside the old walls (Guillem de Castro, Xàtiva,
    // Colón, the old river bed).
    if ellipse(Vec2::new(-150.0, -420.0), Vec2::new(820.0, 800.0)) {
        Zone::OldTown
    } else if ellipse(Vec2::new(430.0, 980.0), Vec2::new(430.0, 520.0)) {
        Zone::Russafa
    } else if p.x > 150.0 && p.x < 1650.0 && p.y > -500.0 && p.y < 1100.0 {
        Zone::Eixample
    } else if p.x > 3550.0 && p.x < 4750.0 && p.y > -1400.0 && p.y < 850.0 {
        Zone::Cabanyal
    } else if p.length() < 6800.0 {
        Zone::Periphery
    } else {
        Zone::Village
    }
}

/// The styles of each district, derived from the style library.
pub struct IslandStyles {
    styles: Vec<Style>,
}

const S_CARMEN: usize = 0;
const S_ENSANCHE: usize = 1;
const S_MODERNISTA: usize = 2;
const S_RUZAFA: usize = 3;
const S_CABANYAL: usize = 4;
const S_BLOQUE: usize = 5;
const S_PUEBLO: usize = 6;
const S_NAVE: usize = 7;
const S_CASA: usize = 8;

impl IslandStyles {
    pub fn new(lib: &StyleLibrary) -> Self {
        let builtin = StyleLibrary::builtin();
        let get = |id: &str| lib.get(id).or_else(|| builtin.get(id)).or_else(|| lib.styles.first()).or_else(|| builtin.styles.first()).cloned().expect("a style");
        let carmen = get("valencia_carmen");
        let bloque = get("desarrollismo");
        // A village house: lime-washed, few shops, quiet.
        let mut pueblo = carmen.clone();
        pueblo.id = "isla_pueblo".into();
        pueblo.walls = ["stucco #f4f0e6", "stucco #efe6d2", "stucco #f6f3ec", "plaster #e6cfa0", "stucco #e9d9b8", "stucco #f1ece0", "stucco #d9b98a"].map(String::from).to_vec();
        pueblo.shops = 0.08;
        pueblo.graffiti = 0.05;
        pueblo.clutter = 0.45;
        pueblo.atico = 0.0;
        // An industrial unit: tall ground floor with roller doors, a row
        // of windows above, render and brick, nothing else.
        let mut nave = bloque.clone();
        nave.id = "isla_nave".into();
        nave.ground_height = 5.2;
        nave.floor_height = 3.0;
        nave.bay = [4.6, 6.0];
        nave.window_width = 1.8;
        nave.window_height = 1.2;
        nave.walls = ["plaster #d6d2c8", "plaster #c9c6bd", "brick #f0d8c0", "stucco #e4e0d6", "plaster #b9bcbd"].map(String::from).to_vec();
        nave.grounds = Vec::new();
        nave.ground = "wall".into();
        nave.balcony = "none".into();
        nave.shops = 0.5;
        nave.graffiti = 0.6;
        nave.awnings = 0.0;
        nave.flower_pots = 0.0;
        nave.clutter = 0.25;
        nave.atico = 0.0;
        // A plain modern house or low block of the outskirts.
        let mut casa = bloque.clone();
        casa.id = "isla_casa".into();
        casa.shops = 0.2;
        casa.clutter = 0.6;
        casa.atico = 0.0;
        IslandStyles { styles: vec![carmen, get("valencia_ensanche"), get("valencia_modernista"), get("valencia_ruzafa"), get("cabanyal"), bloque, pueblo, nave, casa] }
    }

    /// The style of a building of `storeys` in `zone`; `pick` in 0..100.
    pub fn pick(&self, zone: Zone, storeys: u32, pick: u32) -> &Style {
        let i = if storeys >= 10 {
            S_BLOQUE
        } else {
            match zone {
                Zone::OldTown => match (storeys, pick) {
                    (0..=4, 0..=84) => S_CARMEN,
                    (0..=4, _) => S_RUZAFA,
                    (_, 0..=39) => S_ENSANCHE,
                    (_, 40..=59) => S_MODERNISTA,
                    (_, 60..=84) => S_RUZAFA,
                    _ => S_BLOQUE,
                },
                Zone::Eixample => match (storeys, pick) {
                    (0..=3, _) => S_RUZAFA,
                    (_, 0..=44) => S_ENSANCHE,
                    (_, 45..=69) => S_MODERNISTA,
                    (_, 70..=84) => S_RUZAFA,
                    _ => S_BLOQUE,
                },
                Zone::Russafa => match (storeys, pick) {
                    (0..=2, 0..=49) => S_CARMEN,
                    (_, 0..=59) => S_RUZAFA,
                    (_, 60..=74) => S_ENSANCHE,
                    _ => S_BLOQUE,
                },
                Zone::Cabanyal => match (storeys, pick) {
                    (0..=3, 0..=79) => S_CABANYAL,
                    (0..=3, _) => S_PUEBLO,
                    (4..=5, 0..=39) => S_RUZAFA,
                    _ => S_BLOQUE,
                },
                Zone::Periphery => match (storeys, pick) {
                    (0..=2, 0..=59) => S_PUEBLO,
                    (0..=2, _) => S_CASA,
                    (3..=4, 0..=39) => S_RUZAFA,
                    (3..=4, 40..=59) => S_PUEBLO,
                    _ => S_BLOQUE,
                },
                Zone::Village => match (storeys, pick) {
                    (0..=3, 0..=69) => S_PUEBLO,
                    (0..=3, 70..=84) => S_CARMEN,
                    (0..=3, _) => S_CASA,
                    (4, 0..=49) => S_RUZAFA,
                    _ => S_BLOQUE,
                },
                Zone::Industrial => S_NAVE,
            }
        };
        &self.styles[i]
    }
}

/// Footprints and buildings of the island by cell.
pub struct DetailIndex {
    /// Indices into [`Island::buildings`] by the cell of their centre.
    pub cells: HashMap<(i32, i32), Vec<u32>>,
    /// Every building footprint (the landmarks' too) by the cells its
    /// bounds touch: indices into the island's polygons.
    footprints: HashMap<(i32, i32), Vec<u32>>,
    /// Industrial estates: polygon indices.
    industrial: Vec<u32>,
}

impl DetailIndex {
    pub fn new(island: &Island) -> Self {
        let mut cells: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        for (i, b) in island.buildings.iter().enumerate() {
            cells.entry(cell_of(b.centre)).or_default().push(i as u32);
        }
        let mut footprints: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        let mut industrial = Vec::new();
        for (i, p) in island.data.polys.iter().enumerate() {
            match p.kind {
                PolyKind::Building => {
                    let (lo, hi) = p.bounds();
                    let (a, b) = (cell_of(lo), cell_of(hi));
                    // A broken footprint spanning the island indexes nothing.
                    if (b.0 - a.0) > 40 || (b.1 - a.1) > 40 {
                        continue;
                    }
                    for z in a.1..=b.1 {
                        for x in a.0..=b.0 {
                            footprints.entry((x, z)).or_default().push(i as u32);
                        }
                    }
                }
                PolyKind::Industrial | PolyKind::Apron | PolyKind::Airfield => industrial.push(i as u32),
                _ => {}
            }
        }
        DetailIndex { cells, footprints, industrial }
    }

    /// The cells of a shell tile that have buildings.
    pub fn cells_of_tile(&self, tile: (i32, i32)) -> Vec<(i32, i32)> {
        let mut v = Vec::new();
        for z in 0..4 {
            for x in 0..4 {
                let k = (tile.0 * 4 + x, tile.1 * 4 + z);
                if self.cells.contains_key(&k) {
                    v.push(k);
                }
            }
        }
        v
    }

    fn footprints_in(&self, lo: Vec2, hi: Vec2) -> Vec<u32> {
        let (a, b) = (cell_of(lo), cell_of(hi));
        let mut v = Vec::new();
        for z in a.1..=b.1 {
            for x in a.0..=b.0 {
                if let Some(c) = self.footprints.get(&(x, z)) {
                    v.extend_from_slice(c);
                }
            }
        }
        v.sort_unstable();
        v.dedup();
        v
    }

    /// The shells of the buildings of a cell without `skip` (island
    /// polygon indices); `None` without any.
    pub fn cell_shell(&self, island: &Island, key: (i32, i32), skip: &HashSet<u32>) -> Option<SceneData> {
        let list: Vec<Building> = self.cells.get(&key)?.iter().map(|&i| island.buildings[i as usize].clone()).filter(|b| !skip.contains(&b.poly)).collect();
        shell_scene(island, &list)
    }
}

fn shell_scene(island: &Island, list: &[Building]) -> Option<SceneData> {
    if list.is_empty() {
        return None;
    }
    let mesh = shells::tile_mesh(list, &island.data.polys);
    (!mesh.indices.is_empty()).then(|| Materials::shell_mesh_scene(&island.shell, mesh))
}

/// A piece of road near a cell.
#[derive(Clone, Copy)]
struct RoadSeg {
    a: Vec2,
    b: Vec2,
    half: f32,
    class: RoadClass,
    /// Index into the map's roads.
    road: u32,
    /// Arc length along the road at `a`.
    s0: f32,
}

/// A wall of a footprint near a cell.
#[derive(Clone, Copy)]
struct Wall {
    a: Vec2,
    b: Vec2,
}

struct Ring<'a> {
    poly: u32,
    pts: &'a [Vec2],
    lo: Vec2,
    hi: Vec2,
}

/// What surrounds a cell: footprints and roads within [`MARGIN`].
struct Ctx<'a> {
    island: &'a Island,
    rings: Vec<Ring<'a>>,
    walls: Vec<Wall>,
    roads: Vec<RoadSeg>,
}

fn seg_dist(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let d = b - a;
    let t = ((p - a).dot(d) / d.length_squared().max(1e-9)).clamp(0.0, 1.0);
    p.distance(a + d * t)
}

/// Distance along the ray `o + t d` (`d` unit) to the segment `a b`.
fn ray_seg(o: Vec2, d: Vec2, a: Vec2, b: Vec2) -> Option<f32> {
    let e = b - a;
    let den = d.perp_dot(e);
    if den.abs() < 1e-9 {
        return None;
    }
    let t = (a - o).perp_dot(e) / den;
    let u = (a - o).perp_dot(d) / den;
    (t > 1e-3 && (0.0..=1.0).contains(&u)).then_some(t)
}

fn facade_class(c: RoadClass) -> bool {
    use RoadClass::*;
    matches!(c, Primary | PrimaryLink | Secondary | SecondaryLink | Tertiary | TertiaryLink | Residential | Unclassified | LivingStreet | Service | Pedestrian | Footway | Path | Steps | Cycleway | Track | Busway | Promenade | Trunk | TrunkLink | Tram)
}

impl<'a> Ctx<'a> {
    fn new(island: &'a Island, index: &DetailIndex, lo: Vec2, hi: Vec2) -> Self {
        let (lo, hi) = (lo - Vec2::splat(MARGIN), hi + Vec2::splat(MARGIN));
        let mut rings = Vec::new();
        let mut walls = Vec::new();
        for pi in index.footprints_in(lo, hi) {
            let p = &island.data.polys[pi as usize];
            let (a, b) = p.bounds();
            if a.x > hi.x || b.x < lo.x || a.y > hi.y || b.y < lo.y {
                continue;
            }
            rings.push(Ring { poly: pi, pts: &p.rings[0], lo: a, hi: b });
            for r in &p.rings {
                for i in 0..r.len() {
                    let (p0, p1) = (r[i], r[(i + 1) % r.len()]);
                    if p0.distance_squared(p1) > 1e-4 {
                        walls.push(Wall { a: p0, b: p1 });
                    }
                }
            }
        }
        let mut roads = Vec::new();
        for ri in island.map.roads_in(lo, hi) {
            let r = &island.map.roads[ri as usize];
            if r.tunnel || r.bridge || r.class.is_rail() && r.class != RoadClass::Tram || matches!(r.class, RoadClass::Runway | RoadClass::Taxiway | RoadClass::Subway) {
                continue;
            }
            let mut s = 0.0;
            for w in r.points.windows(2) {
                let len = w[0].distance(w[1]);
                if len > 1e-3 && w[0].max(w[1]).cmpge(lo).all() && w[0].min(w[1]).cmple(hi).all() {
                    roads.push(RoadSeg { a: w[0], b: w[1], half: r.width * 0.5, class: r.class, road: ri, s0: s });
                }
                s += len;
            }
        }
        Ctx { island, rings, walls, roads }
    }

    /// Distance from `o` along `d` to the first footprint wall (`max`
    /// without one).
    fn wall_ahead(&self, o: Vec2, d: Vec2, max: f32) -> f32 {
        let (lo, hi) = (o.min(o + d * max) - Vec2::splat(0.01), o.max(o + d * max) + Vec2::splat(0.01));
        let mut best = max;
        for w in &self.walls {
            if w.a.x.max(w.b.x) < lo.x || w.a.x.min(w.b.x) > hi.x || w.a.y.max(w.b.y) < lo.y || w.a.y.min(w.b.y) > hi.y {
                continue;
            }
            if let Some(t) = ray_seg(o, d, w.a, w.b) {
                best = best.min(t);
            }
        }
        best
    }

    /// The first road reached from `o` along `d` within `max`: its
    /// segment and the distance to its paved edge.
    fn road_ahead(&self, o: Vec2, d: Vec2, max: f32, step: f32) -> Option<(usize, f32)> {
        let end = o + d * max;
        let (lo, hi) = (o.min(end), o.max(end));
        let near: Vec<usize> = (0..self.roads.len())
            .filter(|&i| {
                let r = &self.roads[i];
                let m = r.half + 0.5;
                r.a.x.max(r.b.x) + m >= lo.x && r.a.x.min(r.b.x) - m <= hi.x && r.a.y.max(r.b.y) + m >= lo.y && r.a.y.min(r.b.y) - m <= hi.y
            })
            .collect();
        if near.is_empty() {
            return None;
        }
        let mut s = 0.0;
        while s <= max {
            let p = o + d * s;
            let mut hit: Option<(usize, f32)> = None;
            for &i in &near {
                let r = &self.roads[i];
                let dist = seg_dist(p, r.a, r.b);
                if dist <= r.half + 0.3 && hit.is_none_or(|(j, _)| (r.class as u8) < (self.roads[j].class as u8)) {
                    hit = Some((i, s));
                }
            }
            if hit.is_some() {
                return hit;
            }
            s += step;
        }
        None
    }

    /// The footprint (other than `own`) containing `p`.
    fn inside(&self, p: Vec2, own: u32) -> bool {
        self.rings.iter().any(|r| r.poly != own && p.x >= r.lo.x && p.x <= r.hi.x && p.y >= r.lo.y && p.y <= r.hi.y && point_in_ring(r.pts, p))
    }

    fn ground(&self, p: Vec2) -> f32 {
        self.island.ground.height(p.x, p.y)
    }

    /// Whether `p` lies on the paving of a road other than `own`.
    fn on_road(&self, p: Vec2, own: u32, margin: f32) -> bool {
        self.roads.iter().any(|r| r.road != own && r.class.drivable() && seg_dist(p, r.a, r.b) < r.half + margin)
    }
}

/// What one point of a wall faces.
#[derive(Clone, Copy, Debug)]
enum Face {
    Party,
    Street { seg: usize, to_road: f32, clear: f32 },
    Open { clear: f32 },
}

/// A footprint as a lot, with what the builder needs to know about it.
pub struct LotPlan {
    /// Index into [`Island::buildings`].
    pub building: u32,
    pub lot: Lot,
    pub zone: Zone,
    /// Metres of street front (the build order under the budget).
    pub frontage: f32,
    pub seed: u64,
    /// 0 for a whole footprint; the slices of a footprint on a slope are
    /// numbered from 1.
    pub piece: u32,
}

fn mix64(mut x: u64) -> u64 {
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
    x ^= x >> 33;
    x = x.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    x ^ (x >> 33)
}

/// The outer ring cleaned for a lot: counter-clockwise, no repeated or
/// collinear vertices.
fn clean_ring(ring: &[Vec2]) -> Option<Vec<Vec2>> {
    let mut r: Vec<Vec2> = Vec::with_capacity(ring.len());
    for p in ring {
        if !p.is_finite() {
            return None;
        }
        if r.last().is_none_or(|q| q.distance(*p) > 0.15) {
            r.push(*p);
        }
    }
    while r.len() > 3 && r[0].distance(r[r.len() - 1]) <= 0.15 {
        r.pop();
    }
    if ring_area(&r) < 0.0 {
        r.reverse();
    }
    // Vertices that add nothing (collinear, or a spike).
    loop {
        let n = r.len();
        if n < 4 {
            break;
        }
        let drop = (0..n).find(|&i| {
            let (p, c, q) = (r[(i + n - 1) % n], r[i], r[(i + 1) % n]);
            let d = q - p;
            let len = d.length();
            if len < 0.05 {
                return true;
            }
            let off = (c - p).perp_dot(d).abs() / len;
            let along = (c - p).dot(d) / (len * len);
            off < 0.05 && (-0.02..=1.02).contains(&along)
        });
        match drop {
            Some(i) => {
                r.remove(i);
            }
            None => break,
        }
    }
    (r.len() >= 3 && ring_area(&r) > 12.0).then_some(r)
}

impl Ctx<'_> {
    fn face(&self, p: Vec2, n: Vec2, own: u32, whole: Option<&[Vec2]>) -> Face {
        // Against another footprint, or against the next slice of its own.
        if self.inside(p + n * 0.45, own) || whole.is_some_and(|w| point_in_ring(w, p + n * 0.45)) {
            return Face::Party;
        }
        let o = p + n * 0.05;
        let clear = self.wall_ahead(o, n, 80.0);
        if clear < 1.2 {
            // Another wall right against this one (the footprints do not
            // quite touch): still a party wall.
            return Face::Party;
        }
        match self.road_ahead(o, n, clear.min(32.0), 1.0) {
            Some((seg, s)) if facade_class(self.roads[seg].class) => Face::Street { seg, to_road: s, clear },
            _ => Face::Open { clear },
        }
    }

    /// Whether a wall facing open ground at `p` can be seen from a road.
    fn seen(&self, p: Vec2, n: Vec2, clear: f32) -> bool {
        if clear > 28.0 {
            return true;
        }
        let o = p + n * 0.05;
        for deg in [-65.0f32, -40.0, -20.0, 20.0, 40.0, 65.0] {
            let d = Vec2::from_angle(deg.to_radians()).rotate(n);
            let free = self.wall_ahead(o, d, 70.0);
            if free > 2.5 && self.road_ahead(o, d, free, 2.0).is_some() {
                return true;
            }
        }
        false
    }

    /// Arc length along road `seg` nearest to `p`.
    fn along(&self, seg: usize, p: Vec2) -> f32 {
        let r = &self.roads[seg];
        let d = r.b - r.a;
        let t = ((p - r.a).dot(d) / d.length_squared().max(1e-9)).clamp(0.0, 1.0);
        r.s0 + t * d.length()
    }

    /// The lot of a building, or `None` when it keeps its shell (seen from
    /// nowhere, too small, or not a valid lot).
    fn plan(&self, index: &DetailIndex, bi: u32, b: &Building) -> Vec<LotPlan> {
        let poly = &self.island.data.polys[b.poly as usize];
        let Some(ring) = clean_ring(&poly.rings[0]) else { return Vec::new() };
        let area = ring_area(&ring);
        // Sheds, kiosks and garden huts stay shells.
        if area < 28.0 || (b.storeys <= 1 && area < 70.0) || b.height < 2.6 {
            return Vec::new();
        }
        // On a slope the footprint is cut into level slices across the
        // fall line, each standing on its own ground like a terrace of
        // houses: one level lot would float at its low end.
        if let Some(pieces) = self.slices(&ring) {
            let plans: Vec<LotPlan> = pieces.iter().enumerate().filter_map(|(k, piece)| self.plan_ring(index, bi, b, piece.clone(), Some(&ring), k as u32 + 1)).collect();
            if plans.len() == pieces.len() {
                return plans;
            }
        }
        self.plan_ring(index, bi, b, ring, None, 0).into_iter().collect()
    }

    /// The level slices of a footprint whose ground falls more than a
    /// plinth can take (`None`: it stands as one lot).
    fn slices(&self, ring: &[Vec2]) -> Option<Vec<Vec<Vec2>>> {
        let hs: Vec<f32> = ring.iter().map(|p| self.ground(*p)).collect();
        let (lo, hi) = hs.iter().fold((f32::MAX, f32::MIN), |(a, b), h| (a.min(*h), b.max(*h)));
        if hi - lo < SLOPE_DROP {
            return None;
        }
        // The fall line: the least-squares plane through the corners.
        let n = ring.len() as f32;
        let c = ring.iter().copied().sum::<Vec2>() / n;
        let hm = hs.iter().sum::<f32>() / n;
        let (mut sxx, mut sxy, mut syy, mut sxh, mut syh) = (0.0f32, 0.0f32, 0.0f32, 0.0f32, 0.0f32);
        for (p, h) in ring.iter().zip(&hs) {
            let d = *p - c;
            sxx += d.x * d.x;
            sxy += d.x * d.y;
            syy += d.y * d.y;
            sxh += d.x * (h - hm);
            syh += d.y * (h - hm);
        }
        let det = sxx * syy - sxy * sxy;
        if det.abs() < 1e-3 {
            return None;
        }
        let grad = Vec2::new((sxh * syy - syh * sxy) / det, (syh * sxx - sxh * sxy) / det);
        let slope = grad.length();
        if slope < 0.01 {
            return None;
        }
        let dir = grad / slope;
        let (t0, t1) = ring.iter().fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p.dot(dir)), b.max(p.dot(dir))));
        let count = (((hi - lo) / SLICE_DROP).ceil() as usize).min(((t1 - t0) / SLICE_MIN).floor() as usize).min(24);
        if count < 2 {
            return None;
        }
        let across = Vec2::new(-dir.y, dir.x);
        let reach = ring.iter().map(|p| (*p - c).length()).fold(0.0, f32::max) + 5.0;
        let cuts: Vec<Vec<Vec2>> = (1..count)
            .map(|k| {
                let o = c + dir * (t0 + (t1 - t0) * k as f32 / count as f32 - c.dot(dir));
                vec![o - across * reach, o + across * reach]
            })
            .collect();
        let shapes = forja_city::plan::Region::ring(ring).slice(&cuts);
        let mut pieces = Vec::new();
        for sh in shapes {
            if !sh.holes.is_empty() {
                return None;
            }
            // Slivers left at a corner are dropped, real pieces must clean.
            if ring_area(&sh.outer).abs() < 6.0 {
                continue;
            }
            pieces.push(clean_ring(&sh.outer)?);
        }
        (pieces.len() >= 2).then_some(pieces)
    }

    /// The lot on `ring`: a footprint, or one slice of the footprint
    /// `whole` (`piece` numbers the slices from 1).
    fn plan_ring(&self, index: &DetailIndex, bi: u32, b: &Building, ring: Vec<Vec2>, whole: Option<&[Vec2]>, piece: u32) -> Option<LotPlan> {
        let poly = &self.island.data.polys[b.poly as usize];
        let area = ring_area(&ring);
        let n = ring.len();
        let mut edges = vec![EdgeKind::Party; n];
        let mut fronts = vec![Front::UNCONSTRAINED; n];
        let mut lens = vec![0.0f32; n];
        let mut open_clear = vec![0.0f32; n];
        let rules = FrontRules::default();
        let depth = area.sqrt();
        let mut base = f32::MIN;
        let mut any_seen = false;
        for i in 0..n {
            let (a, c) = (ring[i], ring[(i + 1) % n]);
            let len = a.distance(c);
            lens[i] = len;
            if len < 0.9 {
                continue;
            }
            let d = (c - a) / len;
            let out = Vec2::new(d.y, -d.x);
            let ts: &[f32] = if len < 4.0 { &[0.5] } else if len < 12.0 { &[0.25, 0.5, 0.75] } else { &[0.1, 0.3, 0.5, 0.7, 0.9] };
            let faces: Vec<Face> = ts.iter().map(|t| self.face(a + d * (len * t), out, b.poly, whole)).collect();
            let party = faces.iter().filter(|f| matches!(f, Face::Party)).count();
            if party * 2 > faces.len() {
                continue;
            }
            let street: Vec<(usize, f32, f32)> = faces.iter().filter_map(|f| if let Face::Street { seg, to_road, clear } = f { Some((*seg, *to_road, *clear)) } else { None }).collect();
            let mid = (a + c) * 0.5;
            if street.len() * 2 >= faces.len() - party && !street.is_empty() {
                // The main road it faces, the nearest kerb, the tightest gap.
                let seg = street.iter().map(|s| s.0).min_by_key(|&s| self.roads[s].class as u8).unwrap_or(street[0].0);
                let to_road = street.iter().map(|s| s.1).fold(f32::MAX, f32::min);
                let clear = street.iter().map(|s| s.2).fold(f32::MAX, f32::min).clamp(2.0, 60.0);
                let class = self.roads[seg].class;
                let walk = if class.drivable() && class != RoadClass::LivingStreet { to_road.clamp(0.5, 8.0) } else { (clear * 0.5).clamp(0.5, 8.0) };
                let mut f = rules.front(clear, walk);
                f.depth = depth;
                f.open = clear > 45.0;
                f.along = [self.along(seg, a), self.along(seg, c)];
                f.way = self.roads[seg].road;
                f.shops = match class {
                    RoadClass::Service | RoadClass::Footway | RoadClass::Path | RoadClass::Steps | RoadClass::Cycleway | RoadClass::Track => 0.15,
                    RoadClass::Trunk | RoadClass::TrunkLink => 0.3,
                    _ => 1.0,
                };
                // Alleys are lit from their walls.
                if clear < 7.5 {
                    f.lamps = Some(Rhythm { pitch: 17.0, phase: 3.0 });
                }
                edges[i] = EdgeKind::Street;
                fronts[i] = f;
                any_seen = true;
                for t in [0.0, 0.5, 1.0] {
                    let q = a + d * (len * t) + out * 0.6;
                    base = base.max(self.ground(q));
                }
            } else {
                let clear = faces.iter().filter_map(|f| if let Face::Open { clear } | Face::Street { clear, .. } = f { Some(*clear) } else { None }).fold(f32::MAX, f32::min);
                open_clear[i] = clear;
                if len >= 2.0 && self.seen(mid, out, clear) {
                    edges[i] = EdgeKind::Courtyard;
                    any_seen = true;
                }
            }
        }
        if !any_seen {
            // A whole footprint seen from nowhere keeps its shell; a slice
            // in the middle of its footprint is built blind (walls and a
            // roof), level with the ground around it.
            whole?;
            let base = ring.iter().map(|p| self.ground(*p)).fold(f32::MIN, f32::max);
            let lot = Lot::new(ring, edges, base);
            lot.check().ok()?;
            return Some(LotPlan { building: bi, lot, zone: zone_at(b.centre), frontage: 0.0, seed: mix64(b.poly as u64 ^ 0x15_1a_de7a ^ (piece as u64) << 40), piece });
        }
        // Short jogs take the kind of the longer wall next to them.
        for i in 0..n {
            if lens[i] < 0.9 {
                let (p, q) = ((i + n - 1) % n, (i + 1) % n);
                let k = if lens[p] >= lens[q] { p } else { q };
                if lens[k] >= 0.9 {
                    edges[i] = if edges[k] == EdgeKind::Street { EdgeKind::Courtyard } else { edges[k] };
                }
            }
        }
        // A building seen only across open ground still has a front: its
        // longest open wall, with a door and no shops.
        if !edges.iter().any(|e| e.is_street()) {
            let i = (0..n).filter(|&i| edges[i] == EdgeKind::Courtyard).max_by(|&a, &b| lens[a].total_cmp(&lens[b]))?;
            let clear = open_clear[i].clamp(3.0, 60.0);
            let mut f = rules.front(clear, (clear * 0.5).min(4.0));
            f.depth = depth;
            f.shops = 0.0;
            f.along = [0.0, lens[i]];
            edges[i] = EdgeKind::Street;
            fronts[i] = f;
            let (a, c) = (ring[i], ring[(i + 1) % n]);
            let d = (c - a) / lens[i];
            base = base.max(self.ground((a + c) * 0.5 + Vec2::new(d.y, -d.x) * 0.6));
        }
        if base == f32::MIN {
            base = b.floor;
        }
        let mut lot = Lot::new(ring, edges, base);
        lot.fronts = fronts;
        // The contract's corner angles: a street front that meets another
        // wall too sharply becomes a plain courtyard wall.
        for _ in 0..n {
            let mut changed = false;
            for i in 0..n {
                let ang = lot.interior_angle(i).to_degrees();
                let (p, q) = ((i + n - 1) % n, i);
                let (e0, e1) = (lot.edges[p], lot.edges[q]);
                let min = if e0.is_street() && e1.is_street() { forja_city::lot::MIN_CORNER_ANGLE } else if e0.is_street() || e1.is_street() { forja_city::lot::MIN_STREET_ANGLE } else { continue };
                if ang < min + 0.5 || ang > 359.0 {
                    let k = if !e0.is_street() { q } else if !e1.is_street() { p } else if lens[p] < lens[q] { p } else { q };
                    lot.edges[k] = EdgeKind::Courtyard;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        if (whole.is_none() && !lot.edges.iter().any(|e| e.is_street())) || lot.check().is_err() {
            return None;
        }
        let frontage = (0..n).filter(|&i| lot.edges[i].is_street()).map(|i| lens[i]).sum();
        let centre = b.centre;
        let industrial = poly.flags & 1 != 0 || (b.storeys <= 3 && area > 1400.0) || (b.storeys <= 3 && area > 350.0 && index.industrial.iter().any(|&pi| self.island.data.polys[pi as usize].contains(centre)));
        let zone = if industrial { Zone::Industrial } else { zone_at(centre) };
        Some(LotPlan { building: bi, lot, zone, frontage, seed: mix64(b.poly as u64 ^ 0x15_1a_de7a ^ (piece as u64) << 40), piece })
    }
}

/// The lots of a cell (without building them) and the buildings that keep
/// their shell (indices into [`Island::buildings`]).
pub fn plan_cell(island: &Island, index: &DetailIndex, key: (i32, i32)) -> (Vec<LotPlan>, Vec<u32>) {
    let (lo, hi) = cell_bounds(key);
    let ctx = Ctx::new(island, index, lo, hi);
    let (mut plans, mut rest) = (Vec::new(), Vec::new());
    for &bi in index.cells.get(&key).map_or(&[][..], |v| v.as_slice()) {
        let p = ctx.plan(index, bi, &island.buildings[bi as usize]);
        if p.is_empty() {
            rest.push(bi);
        }
        plans.extend(p);
    }
    (plans, rest)
}

/// Storeys (ground floor included) of `style` that best reach `height`.
fn levels_for(style: &Style, height: f32) -> u32 {
    let upper = ((height - style.ground_height) / style.floor_height.max(2.0)).round().max(1.0) as u32;
    (upper + 1).min(40)
}

/// A parked car: on the ground at `pos`, heading `yaw`.
#[derive(Clone, Copy, Debug)]
pub struct CarSpot {
    pub pos: Vec3,
    pub yaw: f32,
    pub seed: u32,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct CellStats {
    pub buildings: usize,
    pub detailed: usize,
    /// Kept as shells: seen from nowhere, tiny, or not a valid lot.
    pub shells: usize,
    /// Kept as shells because the cell's budget was spent.
    pub over_budget: usize,
    /// Triangles at the finest level: the cell's own meshes and its
    /// module instances.
    pub block_triangles: usize,
    pub module_triangles: usize,
    pub instances: usize,
    pub lights: usize,
    pub ms: f32,
}

/// The detail of one cell.
#[derive(Default)]
pub struct CellDetail {
    pub key: (i32, i32),
    /// Walls, roofs, pavements and kerbs of the cell (kit materials).
    pub block: Option<SceneData>,
    /// Far-cascade shadow masses of its buildings.
    pub proxy: Option<SceneData>,
    /// Shells of the buildings that keep theirs.
    pub rest: Option<SceneData>,
    /// Shared modules first used by this cell: `(id, scene)`. Ids are
    /// stable for the builder's life; a module is sent once.
    pub modules: Vec<(u32, Arc<SceneData>)>,
    /// `(module id, world transform)`.
    pub placements: Vec<(u32, Mat4)>,
    pub lights: Vec<CityLight>,
    pub cars: Vec<CarSpot>,
    pub stats: CellStats,
}

/// A scene without its vertex data (bounds, nodes, materials and textures
/// stay): what the builder keeps of a module once the game has it.
fn hollow(s: &SceneData) -> SceneData {
    let meshes = s.meshes.iter().map(|prims| prims.iter().map(|m| forja_assets::MeshData { material: m.material, aabb: m.aabb, lod_sync: m.lod_sync, shadow: m.shadow, ..Default::default() }).collect()).collect();
    SceneData { name: s.name.clone(), nodes: s.nodes.clone(), roots: s.roots.clone(), meshes, materials: s.materials.clone(), textures: Vec::new(), skins: Vec::new(), clips: Vec::new() }
}

/// Street trees of the dressing.
#[derive(Clone, Copy, PartialEq, Eq)]
enum TreeKind {
    Orange,
    Plane,
    Palm,
}

/// What the detail is built from: the city kit (its textures are shared
/// by every cell) and the style library.
pub struct DetailKit {
    kit: Kit,
    styles: StyleLibrary,
}

impl DetailKit {
    /// `hour`: the clock hour of night content (`None` by day).
    pub fn load(downloaded: Option<&Path>, styles: Option<&Path>, hour: Option<f32>) -> Self {
        let mut kit = Kit::load(downloaded);
        if let Some(h) = hour {
            kit.set_hour(h);
        }
        DetailKit { kit, styles: StyleLibrary::load(styles) }
    }

    /// A scene with one small triangle of each of the kit's main
    /// materials, far under the ground: uploading it puts the kit's
    /// textures on the GPU before the first cell needs them.
    pub fn warm_scene(&self) -> SceneData {
        use forja_city::texgen::TilePattern;
        let c = [230, 224, 212];
        let mats = [
            Mat::Stucco(c),
            Mat::Plaster(c),
            Mat::Brick(c),
            Mat::Ashlar(c),
            Mat::Moulding(c),
            Mat::Granite,
            Mat::RoofClay,
            Mat::RoofSlate,
            Mat::RoofTerrace,
            Mat::Glass,
            Mat::Interior,
            Mat::ShopInterior,
            Mat::PaintedWood(c),
            Mat::Wood(c),
            Mat::MetalShutter(0),
            Mat::MetalShutter(1),
            Mat::MetalShutter(2),
            Mat::MetalShutter(3),
            Mat::Tiles(TilePattern::CheckerGreen),
            Mat::Tiles(TilePattern::CheckerBlue),
            Mat::Tiles(TilePattern::DiamondBlue),
            Mat::Tiles(TilePattern::Frieze),
            Mat::Tiles(TilePattern::White),
            Mat::Awning(0),
            Mat::ShopSign,
            Mat::ShopSignLit,
            Mat::Asphalt,
            Mat::SidewalkHex,
            Mat::SidewalkSlab,
            Mat::Curb,
            Mat::Cobble,
            Mat::Hdpe(c),
            Mat::StreetLabel,
            Mat::RoomLit(0, false),
            Mat::RoomLit(0, true),
            Mat::Grass,
            Mat::Plate(0),
        ];
        let mut parts: Parts<Mat> = Parts::new();
        for (i, m) in mats.into_iter().enumerate() {
            let o = Vec3::new(i as f32 * 0.1, -900.0, 0.0);
            parts.get(m).tri(o, o + Vec3::X * 0.01, o + Vec3::Z * 0.01, [Vec2::ZERO, Vec2::X, Vec2::Y]);
        }
        forja_city::kit::assemble(&self.kit, "isla_kit_warm", parts, None)
    }

    /// Drops the pixels of the textures `warm` holds; see
    /// [`Kit::hollow_textures`] for what the renderer must be told.
    pub fn hollow(&mut self, warm: &SceneData) -> Vec<(forja_assets::TextureData, forja_assets::TextureData)> {
        self.kit.hollow_textures(warm)
    }

    pub fn builder(&self, island: Arc<Island>, index: Arc<DetailIndex>, species: Option<&Path>) -> DetailBuilder<'_> {
        DetailBuilder::new(island, index, &self.kit, &self.styles, species)
    }
}

/// Builds cells. Lives on one worker thread: the module library it fills
/// is shared by every cell it builds.
pub struct DetailBuilder<'k> {
    island: Arc<Island>,
    index: Arc<DetailIndex>,
    lib: ModuleLib<'k>,
    out: CityOut,
    styles: IslandStyles,
    species: Option<std::path::PathBuf>,
    /// Module ids of the tree variants by kind (empty until first used).
    trees: Option<[Vec<usize>; 3]>,
    /// Triangles of each module (finest level).
    module_tris: Vec<u32>,
    /// Modules already handed to the game.
    sent: usize,
    /// Street dressing on (`false`: buildings only).
    pub dress: bool,
}

impl<'k> DetailBuilder<'k> {
    pub fn new(island: Arc<Island>, index: Arc<DetailIndex>, kit: &'k Kit, styles: &StyleLibrary, species: Option<&Path>) -> Self {
        DetailBuilder { island, index, lib: ModuleLib::new(kit), out: CityOut::default(), styles: IslandStyles::new(styles), species: species.map(Path::to_path_buf), trees: None, module_tris: Vec::new(), sent: 0, dress: true }
    }

    fn tree(&mut self, kind: TreeKind, pick: usize) -> Option<usize> {
        if self.trees.is_none() {
            use forja_city::layout::PropKind;
            use forja_flora::Pruning;
            let lib = forja_flora::SpeciesLibrary::load(self.species.as_deref());
            let props = vec![(PropKind::Tree, Mat4::IDENTITY), (PropKind::Tree, Mat4::IDENTITY), (PropKind::Palm, Mat4::IDENTITY)];
            let wanted = vec![(0usize, "naranjo", Pruning::Lollipop), (1, "platanus", Pruning::Lifted), (2, "washingtonia", Pruning::Trimmed)];
            let forest = forja_flora::city::plant_species(&props, wanted, &lib, &forja_flora::city::PlantOptions { variants: 2, season: forja_flora::Season::Summer, seed: 0x15_1a });
            let mut by: [Vec<usize>; 3] = Default::default();
            for t in forest.models {
                let k = match t.species.as_str() {
                    "naranjo" => 0,
                    "platanus" => 1,
                    _ => 2,
                };
                by[k].push(self.out.add_scene(t.scene));
            }
            self.trees = Some(by);
        }
        let list = &self.trees.as_ref()?[kind as usize];
        (!list.is_empty()).then(|| list[pick % list.len()])
    }

    /// Builds the detail of cell `key`; buildings in `skip` (island
    /// polygon indices: landmarks placed by hand) are left out entirely.
    pub fn build_cell(&mut self, key: (i32, i32), skip: &HashSet<u32>) -> CellDetail {
        let t0 = std::time::Instant::now();
        let island = self.island.clone();
        let index = self.index.clone();
        let (lo, hi) = cell_bounds(key);
        let ctx = Ctx::new(&island, &index, lo, hi);
        let ids: Vec<u32> = index.cells.get(&key).cloned().unwrap_or_default().into_iter().filter(|&i| !skip.contains(&island.buildings[i as usize].poly)).collect();
        let mut stats = CellStats { buildings: ids.len(), ..Default::default() };
        let mut plans: Vec<LotPlan> = Vec::new();
        let mut rest: Vec<Building> = Vec::new();
        for &bi in &ids {
            let b = &island.buildings[bi as usize];
            let p = ctx.plan(&index, bi, b);
            if p.is_empty() {
                rest.push(b.clone());
            }
            plans.extend(p);
        }
        stats.shells = rest.len();
        // The longest street fronts first: they are what is seen.
        let mut front_of: HashMap<u32, f32> = HashMap::new();
        for p in &plans {
            *front_of.entry(p.building).or_default() += p.frontage;
        }
        plans.sort_by(|a, b| front_of[&b.building].total_cmp(&front_of[&a.building]).then(a.building.cmp(&b.building)).then(a.piece.cmp(&b.piece)));
        let mut parts: Parts<Mat> = Parts::new();
        let first_wall = self.out.party_walls.len();
        let first_mass = self.out.shadow_masses.len();
        let mut budget = CELL_TRIANGLES as i64;
        let mut over: HashSet<u32> = HashSet::new();
        for plan in &plans {
            let b = &island.buildings[plan.building as usize];
            // Slices of a footprint are built together or not at all.
            if budget <= 0 && plan.piece <= 1 {
                over.insert(plan.building);
                rest.push(b.clone());
                stats.over_budget += 1;
            }
            if over.contains(&plan.building) {
                continue;
            }
            let style = self.styles.pick(plan.zone, b.storeys, (plan.seed >> 40) as u32 % 100).clone();
            let mut lot = plan.lot.clone();
            // A slice keeps the building's storeys on its own ground.
            lot.levels = Some(levels_for(&style, if plan.piece > 0 { b.height } else { b.floor + b.height - lot.base }));
            // Built apart, so a lot the generator cannot build leaves
            // nothing behind.
            let marks = (self.out.placements.len(), self.out.party_walls.len(), self.out.shadow_masses.len(), self.out.city_lights.len(), self.out.frontages.len(), self.out.lights.len(), self.out.col_verts.len(), self.out.col_tris.len());
            let mut own: Parts<Mat> = Parts::new();
            let (lib, out) = (&mut self.lib, &mut self.out);
            let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| building::build_into(lib, out, &mut own, &lot, &style, plan.seed)));
            if built.is_err() {
                log::warn!("island detail: building {} of cell {key:?} could not be built; it keeps its shell", b.poly);
                self.out.placements.truncate(marks.0);
                self.out.party_walls.truncate(marks.1);
                self.out.shadow_masses.truncate(marks.2);
                self.out.city_lights.truncate(marks.3);
                self.out.frontages.truncate(marks.4);
                self.out.lights.truncate(marks.5);
                self.out.col_verts.truncate(marks.6);
                self.out.col_tris.truncate(marks.7);
                // A slice that fails leaves a gap (its footprint's other
                // slices stand); a whole footprint keeps its shell.
                if plan.piece == 0 {
                    rest.push(b.clone());
                    stats.shells += 1;
                }
                continue;
            }
            plinth(&mut own, &ctx, &lot);
            // The collision stays the shells': the generator's copy is dropped.
            self.out.col_verts.truncate(marks.6);
            self.out.col_tris.truncate(marks.7);
            let mut tris = own.tri_count();
            for k in marks.0..self.out.placements.len() {
                let m = self.out.placements[k].0;
                tris += self.tris_of(m);
            }
            budget -= tris as i64;
            parts.append(&own, &Mat4::IDENTITY);
            stats.detailed += (plan.piece <= 1) as usize;
        }
        building::medianeras(&mut self.lib, &mut self.out, &mut parts, first_wall);
        let mut cars = Vec::new();
        if self.dress {
            self.dress_cell(&ctx, key, &mut parts, &mut cars);
        }
        let kit = self.lib.kit;
        let proxy = building::proxy_scene(kit, &self.out.shadow_masses[first_mass..]);
        self.out.party_walls.clear();
        self.out.shadow_masses.clear();
        self.out.frontages.clear();
        self.out.lights.clear();
        self.out.col_verts.clear();
        self.out.col_tris.clear();
        if self.out.pharmacies.len() > 64 {
            self.out.pharmacies.drain(..32);
        }
        stats.block_triangles = parts.tri_count();
        let block = (parts.tri_count() > 0).then(|| assemble_roles(kit, "isla_block", parts, Some(BLOCK_LOD_MIN), &block_shadow_role));
        let placements: Vec<(u32, Mat4)> = std::mem::take(&mut self.out.placements).into_iter().map(|(m, t)| (m as u32, t)).collect();
        stats.instances = placements.len();
        stats.module_triangles = placements.iter().map(|(m, _)| self.tris_of(*m as usize)).sum();
        let lights = std::mem::take(&mut self.out.city_lights);
        stats.lights = lights.len();
        // New modules go to the game once; the builder keeps their bounds.
        let mut modules = Vec::new();
        for i in self.sent..self.out.scenes.len() {
            let full = std::mem::replace(&mut self.out.scenes[i], Arc::new(SceneData::default()));
            self.out.scenes[i] = Arc::new(hollow(&full));
            modules.push((i as u32, full));
        }
        self.sent = self.out.scenes.len();
        stats.ms = t0.elapsed().as_secs_f32() * 1000.0;
        CellDetail { key, block, proxy, rest: shell_scene(&island, &rest), modules, placements, lights, cars, stats }
    }

    fn tris_of(&mut self, m: usize) -> usize {
        while self.module_tris.len() < self.out.scenes.len() {
            let i = self.module_tris.len();
            self.module_tris.push(self.out.scenes[i].triangle_count() as u32);
        }
        self.module_tris.get(m).copied().unwrap_or(0) as usize
    }

    fn named(&mut self, name: &str, build: impl FnOnce() -> Parts<Mat>) -> usize {
        self.lib.named(&mut self.out, name, build)
    }

    fn place(&mut self, m: usize, pos: Vec3, yaw: f32) {
        self.out.placements.push((m, Mat4::from_rotation_translation(Quat::from_rotation_y(yaw), pos)));
    }
}

/// A stone plinth under every wall that stands over lower ground: the
/// building is level at its highest street front and the ground falls
/// away along it.
fn plinth(parts: &mut Parts<Mat>, ctx: &Ctx, lot: &Lot) {
    let n = lot.ring.len();
    let top = lot.base - SKIRT + 0.02;
    for i in 0..n {
        let (a, b) = lot.edge(i);
        let len = a.distance(b);
        if len < 0.3 {
            continue;
        }
        let d = (b - a) / len;
        let out = Vec2::new(d.y, -d.x);
        let steps = (len / 6.0).ceil().max(1.0) as usize;
        let low = (0..=steps).map(|k| ctx.ground(a + d * (len * k as f32 / steps as f32) + out * 0.3)).fold(f32::MAX, f32::min);
        if low > top - 0.05 {
            continue;
        }
        let m = if lot.edges[i] == EdgeKind::Party { Mat::Plaster([190, 184, 174]) } else { Mat::Granite };
        let y0 = low - 0.7;
        // Seen from outside the wall runs from b to a (left to right).
        parts.get(m).quad(Vec3::new(b.x, y0, b.y), Vec3::new(a.x, y0, a.y), Vec3::new(a.x, top, a.y), Vec3::new(b.x, top, b.y), m.scale());
    }
}

/// A quad facing up, with planar UVs.
fn quad_up(prim: &mut forja_city::geom::Prim, p: [Vec3; 4], scale: f32) {
    let uv = |q: Vec3| Vec2::new(q.x, q.z) / scale;
    let n = (p[1] - p[0]).cross(p[3] - p[0]);
    let p = if n.y >= 0.0 { p } else { [p[3], p[2], p[1], p[0]] };
    prim.quad_uv(p, [uv(p[0]), uv(p[1]), uv(p[2]), uv(p[3])]);
}

/// A quad facing `want`, with UVs along its length and height.
fn quad_to(prim: &mut forja_city::geom::Prim, p: [Vec3; 4], want: Vec3, scale: f32) {
    let n = (p[1] - p[0]).cross(p[3] - p[0]);
    let p = if n.dot(want) >= 0.0 { p } else { [p[1], p[0], p[3], p[2]] };
    prim.quad(p[0], p[1], p[2], p[3], scale);
}

/// A slatted bench: wood on cast-iron legs, facing +Z.
fn bench() -> Parts<Mat> {
    let mut p: Parts<Mat> = Parts::new();
    let wood = Mat::Wood([120, 82, 48]);
    for k in 0..4 {
        let z = -0.2 + k as f32 * 0.13;
        p.get(wood).aabox(Vec3::new(-0.9, 0.42, z), Vec3::new(0.9, 0.45, z + 0.1), 1.2);
    }
    for k in 0..3 {
        let y = 0.55 + k as f32 * 0.13;
        p.get(wood).aabox(Vec3::new(-0.9, y, -0.27 - k as f32 * 0.02), Vec3::new(0.9, y + 0.1, -0.24 - k as f32 * 0.02), 1.2);
    }
    for x in [-0.75f32, 0.75] {
        let iron = p.get(Mat::Iron);
        iron.aabox(Vec3::new(x - 0.03, 0.0, -0.3), Vec3::new(x + 0.03, 0.42, -0.24), 1.0);
        iron.aabox(Vec3::new(x - 0.03, 0.0, 0.18), Vec3::new(x + 0.03, 0.42, 0.24), 1.0);
        iron.aabox(Vec3::new(x - 0.03, 0.38, -0.3), Vec3::new(x + 0.03, 0.42, 0.24), 1.0);
        iron.aabox(Vec3::new(x - 0.03, 0.42, -0.33), Vec3::new(x + 0.03, 0.95, -0.27), 1.0);
    }
    p
}

/// Kerb height of the pavements over the ground.
pub const KERB: f32 = 0.14;
/// Width of a parking lane between the road and the kerb.
const PARKING: f32 = 2.2;

/// One side of a road at one sample: where the paving runs.
#[derive(Clone, Copy, Default)]
struct Side {
    /// Width of the parking lane (0: none) and of the pavement behind it
    /// (0: none).
    park: f32,
    walk: f32,
}

impl DetailBuilder<'_> {
    /// Pavements, kerbs, parking and street furniture of the roads of a
    /// cell (each stretch of road belongs to the cell its points fall in).
    fn dress_cell(&mut self, ctx: &Ctx, key: (i32, i32), parts: &mut Parts<Mat>, cars: &mut Vec<CarSpot>) {
        let island = self.island.clone();
        let (lo, hi) = cell_bounds(key);
        let inside = |p: Vec2| p.x >= lo.x && p.x < hi.x && p.y >= lo.y && p.y < hi.y;
        // Nothing is left standing where the player starts.
        let start = island.map.start;
        let free = move |p: Vec2| p.distance(start) > 8.0;
        let night = self.lib.kit.night.is_night();
        let zone = zone_at((lo + hi) * 0.5);
        let valencia = !matches!(zone, Zone::Village);
        let paving = if matches!(zone, Zone::OldTown | Zone::Eixample | Zone::Russafa | Zone::Cabanyal) { Mat::SidewalkHex } else { Mat::SidewalkSlab };
        let roads = island.map.roads_in(lo - Vec2::splat(2.0), hi + Vec2::splat(2.0));
        for ri in roads {
            let r = &island.map.roads[ri as usize];
            use RoadClass::*;
            if r.tunnel || r.bridge {
                continue;
            }
            let kerbed = matches!(r.class, Primary | Secondary | Tertiary | Residential | Unclassified | PrimaryLink | SecondaryLink | TertiaryLink);
            let shared = matches!(r.class, Pedestrian | LivingStreet);
            if !kerbed && !shared {
                continue;
            }
            let half = r.width * 0.5;
            let main = matches!(r.class, Primary | Secondary);
            let seed = mix64(ri as u64 ^ 0x57_ee7);
            // Samples every ~2.5 m of arc length.
            let mut pts: Vec<(Vec2, Vec2, f32)> = Vec::new();
            let mut s = 0.0;
            for w in r.points.windows(2) {
                let len = w[0].distance(w[1]);
                if len < 1e-3 {
                    continue;
                }
                let d = (w[1] - w[0]) / len;
                let k = (len / 2.5).ceil().max(1.0) as usize;
                for j in 0..k {
                    pts.push((w[0] + d * (len * j as f32 / k as f32), d, s + len * j as f32 / k as f32));
                }
                s += len;
                if std::ptr::eq(w, r.points.windows(2).last().unwrap_or(w)) {
                    pts.push((w[1], d, s));
                }
            }
            let total = s;
            if pts.len() < 2 || !pts.iter().any(|p| inside(p.0)) {
                continue;
            }
            if shared {
                // No kerbs: lamps along the wider pedestrian streets (the
                // alleys are lit from their walls).
                if r.width >= 4.5 {
                    self.along_road(ctx, &pts, total, 24.0, seed, &inside, |me, p, d, k| {
                        let side = if k % 2 == 0 { 1.0 } else { -1.0 };
                        let out = Vec2::new(d.y, -d.x) * side;
                        let q = p + out * (half - 0.45);
                        if ctx.wall_ahead(p, out, half) < half - 0.2 {
                            return;
                        }
                        me.lamp(ctx, q, valencia && r.width >= 7.0, night);
                    });
                }
                continue;
            }
            // Each side: free width to the facades, parking, pavement.
            for side in [1.0f32, -1.0] {
                let sides: Vec<Side> = pts
                    .iter()
                    .map(|&(p, d, _)| {
                        let out = Vec2::new(d.y, -d.x) * side;
                        let e = p + out * half;
                        if !inside(p) || ctx.inside(e + out * 0.3, u32::MAX) || ctx.on_road(e + out * 0.5, ri, 0.1) {
                            return Side::default();
                        }
                        let free = ctx.wall_ahead(e - out * 0.5, out, 26.0) - 0.5;
                        if free < 0.8 {
                            return Side::default();
                        }
                        // No facade near: a plain pavement where the street
                        // is built up at all, nothing in the open.
                        if free > 12.0 {
                            return if free < 25.0 { Side { park: 0.0, walk: 2.2 } } else { Side::default() };
                        }
                        let park = if free >= PARKING + 1.6 && !matches!(r.class, PrimaryLink | SecondaryLink | TertiaryLink) { PARKING } else { 0.0 };
                        Side { park, walk: (free - park - 0.02).min(6.5) }
                    })
                    .collect();
                // The parking lane is kept only over whole car lengths and
                // away from the junctions.
                let mut sides = sides;
                let mut i = 0;
                while i < sides.len() {
                    if sides[i].park <= 0.0 {
                        i += 1;
                        continue;
                    }
                    let start = i;
                    while i < sides.len() && sides[i].park > 0.0 {
                        i += 1;
                    }
                    let len = pts[i - 1].2 - pts[start].2;
                    let near_end = |k: usize| pts[k].2 < 9.0 || pts[k].2 > total - 9.0;
                    for k in start..i {
                        if len < 11.0 || near_end(k) {
                            sides[k].walk = (sides[k].walk + sides[k].park).min(6.5);
                            sides[k].park = 0.0;
                        }
                    }
                }
                let y = |q: Vec2, up: f32| Vec3::new(q.x, ctx.ground(q) + up, q.y);
                for k in 0..pts.len() - 1 {
                    let (a, b) = (sides[k], sides[k + 1]);
                    if a.walk <= 0.0 || b.walk <= 0.0 || pts[k + 1].2 - pts[k].2 > 4.0 {
                        continue;
                    }
                    let (p0, d0, _) = pts[k];
                    let (p1, d1, _) = pts[k + 1];
                    let (o0, o1) = (Vec2::new(d0.y, -d0.x) * side, Vec2::new(d1.y, -d1.x) * side);
                    let at0 = |w: f32| p0 + o0 * (half + w);
                    let at1 = |w: f32| p1 + o1 * (half + w);
                    let toward = Vec3::new(-o0.x, 0.0, -o0.y);
                    if a.park > 0.0 && b.park > 0.0 {
                        quad_up(parts.get(Mat::Asphalt), [y(at0(-0.05), 0.026), y(at1(-0.05), 0.026), y(at1(b.park), 0.026), y(at0(a.park), 0.026)], Mat::Asphalt.scale());
                    } else if a.park != b.park {
                        // The lane ends: a kerb across it.
                        let (q, w) = if a.park > 0.0 { (1, a.park) } else { (0, b.park) };
                        let (e0, e1) = if q == 1 { (at1(0.0), at1(w)) } else { (at0(0.0), at0(w)) };
                        let face = if q == 1 { Vec3::new(-d1.x, 0.0, -d1.y) } else { Vec3::new(d0.x, 0.0, d0.y) };
                        quad_to(parts.get(Mat::Curb), [y(e0, -0.1), y(e1, -0.1), y(e1, KERB), y(e0, KERB)], face, Mat::Curb.scale());
                    }
                    let (k0, k1) = (a.park, b.park);
                    // Kerb: its face to the road and a stone strip on top.
                    quad_to(parts.get(Mat::Curb), [y(at0(k0), -0.1), y(at1(k1), -0.1), y(at1(k1), KERB), y(at0(k0), KERB)], toward, Mat::Curb.scale());
                    quad_up(parts.get(Mat::Curb), [y(at0(k0), KERB), y(at1(k1), KERB), y(at1(k1 + 0.16), KERB), y(at0(k0 + 0.16), KERB)], Mat::Curb.scale());
                    quad_up(parts.get(paving), [y(at0(k0 + 0.16), KERB), y(at1(k1 + 0.16), KERB), y(at1(k1 + b.walk), KERB), y(at0(k0 + a.walk), KERB)], paving.scale());
                }
                // Furniture along this side, by arc length (so it does not
                // depend on which cell builds which stretch).
                let lamp_pitch = if main { 30.0 } else { 26.0 };
                let phase = if side > 0.0 { 0.0 } else { lamp_pitch * 0.5 };
                let mut next_lamp = phase + 4.0;
                let mut next_tree = 7.0 + (seed % 5) as f32;
                let mut next_bin = 17.0 + (seed % 23) as f32;
                let mut next_bench = 31.0 + (seed % 37) as f32;
                let mut next_cont = 45.0 + (seed % 61) as f32;
                let mut next_car: f32 = 11.0;
                let tree = if main { TreeKind::Plane } else if (seed >> 8) % 7 == 0 && valencia { TreeKind::Palm } else { TreeKind::Orange };
                let planted = r.width >= 6.0 && (seed >> 16) % 4 != 0;
                for k in 0..pts.len() {
                    let (p, d, s) = pts[k];
                    let sd = sides[k];
                    let out = Vec2::new(d.y, -d.x) * side;
                    let yaw_in = (-out.x).atan2(-out.y);
                    let near_end = s < 7.0 || s > total - 7.0;
                    let at = |w: f32| p + out * (half + sd.park + w);
                    if s >= next_lamp {
                        next_lamp += lamp_pitch * ((s - next_lamp) / lamp_pitch).floor().max(0.0) + lamp_pitch;
                        if sd.walk >= 1.2 && inside(p) {
                            let q = at(0.5);
                            if main && r.width >= 11.0 {
                                self.road_light(ctx, q, yaw_in, night);
                            } else {
                                self.lamp(ctx, q, main && valencia, night);
                            }
                        }
                    }
                    if s >= next_tree {
                        next_tree += 9.0;
                        if planted && sd.walk >= 2.8 && !near_end && inside(p) {
                            let q = at(0.95);
                            let pos = Vec3::new(q.x, ctx.ground(q) + KERB, q.y);
                            let grate = self.named("tree_grate", furniture::tree_grate);
                            self.place(grate, pos + Vec3::Y * 0.012, d.x.atan2(d.y));
                            let pick = mix64(seed ^ (s as u64) << 8);
                            if let Some(t) = self.tree(tree, pick as usize) {
                                let sc = 0.85 + (pick >> 20 & 0xff) as f32 / 255.0 * 0.3;
                                self.out.placements.push((t, Mat4::from_scale_rotation_translation(Vec3::splat(sc), Quat::from_rotation_y((pick >> 8 & 0xff) as f32 * 0.0246), pos)));
                            }
                        }
                    }
                    if s >= next_bin {
                        next_bin += 43.0;
                        if sd.walk >= 1.8 && !near_end && inside(p) && free(p) {
                            let q = at(0.42);
                            let (name, model) = if valencia { ("papelera_double", BinModel::PostDouble) } else { ("papelera_single", BinModel::PostSingle) };
                            let m = self.named(name, || street_waste::papelera(model, 0));
                            self.place(m, Vec3::new(q.x, ctx.ground(q) + KERB, q.y), yaw_in + std::f32::consts::PI);
                        }
                    }
                    if s >= next_bench {
                        next_bench += 61.0;
                        if sd.walk >= 3.4 && !near_end && inside(p) && free(p) {
                            let q = at(sd.walk - 0.55);
                            let m = self.named("isla_bench", bench);
                            self.place(m, Vec3::new(q.x, ctx.ground(q) + KERB, q.y), yaw_in);
                        }
                    }
                    if s >= next_cont {
                        next_cont += 95.0;
                        if sd.park > 0.0 && !near_end && inside(p) && free(p) && k + 2 < pts.len() && sides[k + 1].park > 0.0 {
                            // A row of containers in the parking lane.
                            for (j, kind) in [3u8, 0, 1].into_iter().enumerate() {
                                let q = p + d * (j as f32 * 1.7) + out * (half + sd.park - 0.75);
                                let m = self.named(&format!("container_{kind}"), || furniture::container(kind));
                                self.place(m, Vec3::new(q.x, ctx.ground(q) + 0.03, q.y), yaw_in);
                            }
                            next_car = next_car.max(s + 7.5);
                        }
                    }
                    if s >= next_car {
                        next_car = s + 5.6;
                        let h = mix64(seed ^ (s * 3.0) as u64 ^ if side > 0.0 { 0x9e37 } else { 0x79b9 });
                        let run = k + 2 < pts.len() && sides[k + 1].park > 0.0 && sides[k + 2].park > 0.0;
                        if sd.park > 0.0 && run && inside(p) && free(p) && h % 100 < 62 {
                            let q = p + d * 2.3 + out * (half + sd.park * 0.5 - 0.05);
                            // Parked with the traffic on the right-hand side.
                            let fwd = if side > 0.0 { d } else { -d };
                            cars.push(CarSpot { pos: Vec3::new(q.x, ctx.ground(q) + 0.03, q.y), yaw: fwd.x.atan2(fwd.y), seed: (h >> 16) as u32 });
                        }
                    }
                }
            }
        }
        self.signals(ctx, &inside, night);
    }

    /// Calls `f(self, point, direction, k)` every `pitch` metres of road.
    fn along_road(&mut self, _ctx: &Ctx, pts: &[(Vec2, Vec2, f32)], _total: f32, pitch: f32, seed: u64, inside: &dyn Fn(Vec2) -> bool, mut f: impl FnMut(&mut Self, Vec2, Vec2, usize)) {
        let mut next = 5.0 + (seed % 7) as f32;
        let mut k = 0;
        for &(p, d, s) in pts {
            if s >= next {
                next += pitch;
                k += 1;
                if inside(p) {
                    f(self, p, d, k);
                }
            }
        }
    }

    fn lamp(&mut self, ctx: &Ctx, q: Vec2, candelabra: bool, night: bool) {
        let pos = Vec3::new(q.x, ctx.ground(q) + KERB, q.y);
        let bulb = pos + Vec3::Y * furniture::street_lamp_bulb(candelabra);
        let (color, _) = nightlight::street_lamp_color(bulb, false);
        self.out.city_lights.push(nightlight::street_lamp(bulb, color, candelabra));
        let m = if night {
            let g = nightlight::to_rgb(color);
            self.named(&format!("street_lamp_{candelabra}_{g:?}"), || furniture::street_lamp_lit(candelabra, g))
        } else {
            self.named(&format!("street_lamp_{candelabra}"), || furniture::street_lamp(candelabra))
        };
        self.place(m, pos, 0.0);
    }

    fn road_light(&mut self, ctx: &Ctx, q: Vec2, yaw: f32, night: bool) {
        let pos = Vec3::new(q.x, ctx.ground(q) + KERB, q.y);
        let led = nightlight::kelvin(3000.0);
        let m = if night {
            let g = nightlight::to_rgb(led);
            self.named("road_light_lit", || furniture::road_light_lit(g))
        } else {
            self.named("road_light", furniture::road_light)
        };
        let at = Mat4::from_rotation_translation(Quat::from_rotation_y(yaw), pos);
        self.out.placements.push((m, at));
        self.out.city_lights.push(nightlight::road_light(at.transform_point3(furniture::ROAD_LIGHT_HEAD) - Vec3::Y * 0.05, led));
    }

    /// Traffic lights where main roads cross inside the cell: one per
    /// arriving arm, on the right-hand kerb before the junction.
    fn signals(&mut self, ctx: &Ctx, inside: &dyn Fn(Vec2) -> bool, night: bool) {
        struct Arm {
            dir: Vec2,
            half: f32,
            class: RoadClass,
            arrives: bool,
        }
        let island = self.island.clone();
        let key = |p: Vec2| ((p.x * 4.0).round() as i32, (p.y * 4.0).round() as i32);
        let mut nodes: HashMap<(i32, i32), (Vec2, Vec<Arm>)> = HashMap::new();
        let mut seen: HashSet<u32> = HashSet::new();
        for seg in &ctx.roads {
            if !seen.insert(seg.road) {
                continue;
            }
            let r = &island.map.roads[seg.road as usize];
            if !r.class.drivable() || matches!(r.class, RoadClass::Service | RoadClass::Motorway | RoadClass::MotorwayLink) || r.points.len() < 2 {
                continue;
            }
            let n = r.points.len();
            for (i, p) in r.points.iter().enumerate() {
                if !inside(*p) {
                    continue;
                }
                let e = nodes.entry(key(*p)).or_insert_with(|| (*p, Vec::new()));
                if i + 1 < n {
                    // Leaving along the road's direction: cars arrive here
                    // on it only when it is two-way.
                    e.1.push(Arm { dir: (r.points[i + 1] - *p).normalize_or(Vec2::X), half: r.width * 0.5, class: r.class, arrives: !r.oneway });
                }
                if i > 0 {
                    e.1.push(Arm { dir: (r.points[i - 1] - *p).normalize_or(Vec2::X), half: r.width * 0.5, class: r.class, arrives: true });
                }
            }
        }
        let mut list: Vec<((i32, i32), (Vec2, Vec<Arm>))> = nodes.into_iter().collect();
        list.sort_by_key(|n| n.0);
        for (_, (p, arms)) in list {
            let main = arms.iter().filter(|a| a.class <= RoadClass::TertiaryLink).count();
            if arms.len() < 3 || main < 2 || !arms.iter().any(|a| a.class <= RoadClass::Secondary) {
                continue;
            }
            // The straightest pair of the best class runs on green.
            let mut best = (0, 1, f32::INFINITY);
            for i in 0..arms.len() {
                for j in i + 1..arms.len() {
                    let score = arms[i].dir.dot(arms[j].dir) + (arms[i].class as u8 + arms[j].class as u8) as f32 * 0.05;
                    if score < best.2 {
                        best = (i, j, score);
                    }
                }
            }
            let reach = arms.iter().map(|a| a.half).fold(0.0, f32::max);
            for (k, a) in arms.iter().enumerate() {
                if !a.arrives {
                    continue;
                }
                // The kerb side of the arriving traffic (`forja_city`'s right
                // of a direction in (x, z), as its lanes and its lights).
                let travel = -a.dir;
                let right = Vec2::new(travel.y, -travel.x);
                let q = p + a.dir * (reach + 3.0) + right * (a.half + 0.55);
                if ctx.inside(q, u32::MAX) || ctx.on_road(q, u32::MAX, -0.2) {
                    continue;
                }
                let green = k == best.0 || k == best.1;
                let aspect = if green { Aspect::Green } else { Aspect::Red };
                let arm = if a.half >= 6.5 { 5.0 } else { 0.0 };
                let ped_yaw = std::f32::consts::FRAC_PI_2;
                let m = self.named(&format!("tl_{aspect:?}_{}_{arm}", !green), || furniture::traffic_light(aspect, !green, ped_yaw, arm));
                let at = Mat4::from_rotation_translation(Quat::from_rotation_y(a.dir.x.atan2(a.dir.y)), Vec3::new(q.x, ctx.ground(q) + KERB, q.y));
                self.out.placements.push((m, at));
                if night {
                    self.out.city_lights.extend(furniture::signal_lights(aspect, !green, ped_yaw, arm).into_iter().map(|mut l| {
                        l.pos = at.transform_point3(l.pos);
                        if let Some(s) = &mut l.spot {
                            s.dir = at.transform_vector3(s.dir);
                        }
                        l
                    }));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rings_are_cleaned_for_lots() {
        // Clockwise, with a repeated point and a collinear one.
        let r = vec![Vec2::new(0.0, 0.0), Vec2::new(0.0, 10.0), Vec2::new(5.0, 10.0), Vec2::new(10.0, 10.0), Vec2::new(10.0, 0.0), Vec2::new(10.0, 0.05), Vec2::new(0.0, 0.0)];
        let c = clean_ring(&r).unwrap();
        assert_eq!(c.len(), 4);
        assert!(ring_area(&c) > 0.0);
        assert!(clean_ring(&[Vec2::ZERO, Vec2::X, Vec2::Y]).is_none());
    }

    #[test]
    fn zones_follow_the_city() {
        assert_eq!(zone_at(Vec2::new(-100.0, -300.0)), Zone::OldTown);
        assert_eq!(zone_at(Vec2::new(450.0, 950.0)), Zone::Russafa);
        assert_eq!(zone_at(Vec2::new(900.0, 100.0)), Zone::Eixample);
        assert_eq!(zone_at(Vec2::new(4060.0, -120.0)), Zone::Cabanyal);
        assert_eq!(zone_at(Vec2::new(-3000.0, 2500.0)), Zone::Periphery);
        assert_eq!(zone_at(Vec2::new(-9000.0, 9000.0)), Zone::Village);
    }

    #[test]
    fn rays_meet_walls() {
        let t = ray_seg(Vec2::ZERO, Vec2::X, Vec2::new(5.0, -1.0), Vec2::new(5.0, 1.0)).unwrap();
        assert!((t - 5.0).abs() < 1e-4);
        assert!(ray_seg(Vec2::ZERO, Vec2::X, Vec2::new(-5.0, -1.0), Vec2::new(-5.0, 1.0)).is_none());
        assert!((seg_dist(Vec2::new(1.0, 2.0), Vec2::ZERO, Vec2::new(4.0, 0.0)) - 2.0).abs() < 1e-5);
    }
}
