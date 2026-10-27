//! `Fabric`: any `Draft` turned into lots.
//!
//! The city is one planar partition: lots are the positive space, public
//! space is the negative space and courts are the enclosed pieces of
//! negative space. `build` is generic and pure 2D:
//!
//! 1. graph and corridors (the street space, building line to building line);
//! 2. land = masks − corridors − plazas, opened per zone to remove necks;
//! 3. blocks = land pieces, with street corners bevelled (chaflanes, fillets,
//!    and chamfers that keep acute tips at least `min_tip` wide);
//! 4. lots = OSM footprints plus `parcel` of the rest, jittered setbacks;
//! 5. courts vs public decided topologically;
//! 6. every lot edge classified by one probe rule and every street edge
//!    given a measured `Front` (clear width, walk, depth, limits, lamps).
//!
//! Nothing here knows which generator wrote the draft.

use std::collections::{BTreeMap, HashMap};
use std::ops::Range;

use forja_math::Vec2;

use crate::generators::{CityPlan, LotRules};
use crate::geom::{Rng, point_key};
use crate::ground::Ground;
use crate::lot::{EdgeKind, Front, Lot, MAX_CHORD, MIN_CORNER_ANGLE, Rhythm};
use crate::parcel::{self, BlockRing, Tag};
use crate::plan::line::{mix64, seg_dist};
use crate::plan::{Draft, Join, Located, Region, SegIndex, Shape, StreetGraph, StreetId};
use crate::profile::{Class, Mount, ProfileLibrary};

/// Random stages (for `Rng::keyed`).
pub mod stage {
    pub const BEVEL: u64 = 1;
    pub const PARCEL: u64 = 2;
    pub const JITTER: u64 = 3;
    pub const LOT: u64 = 4;
    pub const DRESS: u64 = 5;
    pub const CARVE: u64 = 6;
    pub const GRID: u64 = 7;
}

/// Stage timer for `FORJA_TIMING=1` (stderr), a no-op otherwise.
pub struct Laps(Option<std::time::Instant>);

impl Laps {
    pub fn new() -> Self {
        Laps(std::env::var_os("FORJA_TIMING").map(|_| std::time::Instant::now()))
    }

    pub fn lap(&mut self, what: &str) {
        if let Some(t) = self.0 {
            eprintln!("  {what}: {:.3}s", t.elapsed().as_secs_f32());
            self.0 = Some(std::time::Instant::now());
        }
    }
}

impl Default for Laps {
    fn default() -> Self {
        Self::new()
    }
}

/// Lot vertices closer than this merge into one (region debris), so a lot
/// may cut across a block-outline vertex by at most this much.
pub const WELD: f32 = 0.06;

/// A street corner of a block: where two street-facing sides meet.
#[derive(Clone, Debug, PartialEq)]
pub struct CornerMark {
    pub apex: Vec2,
    /// Interior angle before bevelling (radians).
    pub theta: f32,
    /// The chamfer chord or fillet chords (empty when not bevelled).
    pub chords: Vec<[Vec2; 2]>,
    pub corner: bool,
}

#[derive(Clone, Debug)]
pub struct Block {
    pub shape: Shape,
    pub zone: u16,
    pub key: u64,
    pub corners: Vec<CornerMark>,
    pub lots: Range<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LotInfo {
    pub zone: u16,
    pub block: u32,
    /// The highest-ranked street the lot faces.
    pub main: Option<StreetId>,
    /// Style pool key (`street`, `avenue`, `outer`, `alley`, `plaza`).
    pub pool: String,
    pub key: u64,
    /// Built from a real footprint.
    pub footprint: bool,
}

#[derive(Default)]
pub struct Fabric {
    pub graph: StreetGraph,
    pub corridors: Region,
    pub plazas: Region,
    pub masks: Region,
    pub domain: Region,
    pub blocks: Vec<Block>,
    pub lots: Vec<Lot>,
    pub info: Vec<LotInfo>,
    pub courts: Region,
    pub public: Region,
    pub ground: Ground,
    pub extent: [Vec2; 2],
    pub stats: Vec<(String, usize)>,
    /// Lots that could not be made valid (dropped into courts).
    pub dropped: usize,
    /// Accessibility depths reached per organic zone (for validation).
    pub zone_depth: BTreeMap<u16, f32>,
}

/// A street edge's street: id, arc length, side (+1 left) and how far the
/// edge is from that street's building line.
#[derive(Clone, Copy, Debug)]
struct Hit {
    s: StreetId,
    side: f32,
    miss: f32,
}

fn street_of(g: &StreetGraph, a: Vec2, b: Vec2) -> Option<Hit> {
    let m = (a + b) * 0.5;
    let mut best: Option<Hit> = None;
    for (s, t, o) in g.near(m, 40.0) {
        let st = g.street(s);
        let miss = (o.abs() - st.width_at(t) * 0.5).abs();
        // The edge must run along the street, not across it.
        let along = st.line.tangent(t).dot((b - a).normalize_or(Vec2::X)).abs();
        let miss = if a.distance(b) > 1.0 && along < 0.5 { miss + 5.0 } else { miss };
        if best.is_none_or(|h| miss < h.miss) {
            best = Some(Hit { s, side: o.signum(), miss });
        }
    }
    best
}

/// Interior angle (radians) at vertex `i` of a CCW ring.
fn interior(p: &[Vec2], i: usize) -> f32 {
    let n = p.len();
    let (a, b, c) = (p[(i + n - 1) % n], p[i], p[(i + 1) % n]);
    let (din, dout) = (b - a, c - b);
    std::f32::consts::PI - din.perp_dot(dout).atan2(din.dot(dout))
}

/// Removes ring vertices that are within `tol` of their neighbours' chord
/// when the merged chord stays under `MAX_CHORD`, and near-duplicates.
fn clean_ring(r: &[Vec2], tol: f32) -> Vec<Vec2> {
    let mut p = r.to_vec();
    let mut changed = true;
    while changed && p.len() > 3 {
        changed = false;
        let mut i = 0;
        while i < p.len() && p.len() > 3 {
            let n = p.len();
            let (a, b, c) = (p[(i + n - 1) % n], p[i], p[(i + 1) % n]);
            if a.distance(b) < 0.01 || (seg_dist(b, a, c) < tol && a.distance(c) <= MAX_CHORD.max(a.distance(b).max(b.distance(c)))) {
                p.remove(i);
                changed = true;
            } else {
                i += 1;
            }
        }
    }
    p
}

/// Bevels the street corners of a block ring. Returns the new ring and
/// the corner marks.
fn bevel(ring: &[Vec2], faces: &dyn Fn(Vec2, Vec2) -> bool, chamfer_of: &dyn Fn(Vec2, Vec2) -> (f32, f32), rules: &LotRules, rng_key: (u64, u64)) -> (Vec<Vec2>, Vec<CornerMark>) {
    let n = ring.len();
    let mut s = vec![0.0f32; n + 1];
    for i in 0..n {
        s[i + 1] = s[i] + ring[i].distance(ring[(i + 1) % n]);
    }
    let total = s[n];
    // Corners: convex vertices sharper than 150° between public edges.
    let mut corners: Vec<(usize, f32, f32, bool)> = Vec::new();
    for i in 0..n {
        let th = interior(ring, i);
        if th >= 150f32.to_radians() || th <= 0.0 {
            continue;
        }
        let (e0, e1) = ((ring[(i + n - 1) % n], ring[i]), (ring[i], ring[(i + 1) % n]));
        if !faces(e0.0, e0.1) || !faces(e1.0, e1.1) {
            continue;
        }
        let (c0, round) = chamfer_of(e0.0, e0.1);
        let (c1, round1) = chamfer_of(e1.0, e1.1);
        let mut a = c0.max(c1);
        if th < MIN_CORNER_ANGLE.to_radians() {
            a = a.max(rules.min_tip / (2.0 * (th * 0.5).sin()));
        }
        let mut rng = Rng::keyed(rng_key.0, stage::BEVEL, rng_key.1 ^ point_key(ring[i]));
        let round = rng.chance(round.max(round1)) && a > 0.5;
        corners.push((i, th, a, round));
    }
    // Clamp each bevel to 0.45 of the sides between corners.
    let m = corners.len();
    for k in 0..m {
        let (i, _, a, _) = corners[k];
        let prev = if m > 1 { corners[(k + m - 1) % m].0 } else { i };
        let next = if m > 1 { corners[(k + 1) % m].0 } else { i };
        let back = (s[i] - s[prev]).rem_euclid(total);
        let fwd = (s[next] - s[i]).rem_euclid(total);
        let (back, fwd) = (if back <= 0.0 { total } else { back }, if fwd <= 0.0 { total } else { fwd });
        corners[k].2 = a.min(0.45 * back).min(0.45 * fwd);
    }
    let at = |d: f32| -> Vec2 {
        let d = d.rem_euclid(total);
        let k = s.partition_point(|v| *v <= d).saturating_sub(1).min(n - 1);
        let l = s[k + 1] - s[k];
        ring[k].lerp(ring[(k + 1) % n], if l > 0.0 { (d - s[k]) / l } else { 0.0 })
    };
    let dir_at = |d: f32| -> Vec2 {
        let d = d.rem_euclid(total);
        let k = s.partition_point(|v| *v <= d).saturating_sub(1).min(n - 1);
        (ring[(k + 1) % n] - ring[k]).normalize_or(Vec2::X)
    };
    // Intervals to replace.
    let mut marks = Vec::new();
    let mut cut: Vec<(f32, f32, Vec<Vec2>)> = Vec::new();
    for &(i, th, a, round) in &corners {
        if a < 0.05 {
            marks.push(CornerMark { apex: ring[i], theta: th, chords: Vec::new(), corner: false });
            continue;
        }
        let (d0, d1) = (s[i] - a, s[i] + a);
        let (p0, p1) = (at(d0), at(d1));
        let mut pts = vec![p0];
        if round {
            let (t0, t1) = (dir_at(d0 + 1e-4), dir_at(d1 - 1e-4));
            let den = t0.perp_dot(t1);
            if den.abs() > 1e-3 {
                let u = (p1 - p0).perp_dot(t1) / den;
                let q = p0 + t0 * u;
                let w = (std::f32::consts::PI - (t0.perp_dot(t1).atan2(t0.dot(t1))).abs()) * 0.5;
                let w = w.sin();
                for k in 1..6 {
                    let u = k as f32 / 6.0;
                    let (b0, b1, b2) = ((1.0 - u) * (1.0 - u), 2.0 * u * (1.0 - u) * w, u * u);
                    pts.push((p0 * b0 + q * b1 + p1 * b2) / (b0 + b1 + b2));
                }
            }
        }
        pts.push(p1);
        let pts: Vec<Vec2> = pts.into_iter().map(crate::plan::region::snap).collect();
        let chords = pts.windows(2).map(|w| [w[0], w[1]]).collect();
        marks.push(CornerMark { apex: ring[i], theta: th, chords, corner: true });
        cut.push((d0, d1, pts));
    }
    if cut.is_empty() {
        return (ring.to_vec(), marks);
    }
    // Walk the ring, replacing each interval by its bevel.
    let mut out = Vec::new();
    let inside = |d: f32| cut.iter().any(|(a, b, _)| {
        let (a, b) = (*a, *b);
        if a < 0.0 { d > a + total || d < b } else if b > total { d > a || d < b - total } else { d > a && d < b }
    });
    // Events: original vertices and bevel starts, by position.
    let mut ev: Vec<(f32, usize, bool)> = (0..n).map(|i| (s[i], i, false)).collect();
    for (k, c) in cut.iter().enumerate() {
        ev.push((c.0.rem_euclid(total), k, true));
    }
    ev.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.2.cmp(&b.2)));
    for (d, k, is_cut) in ev {
        if is_cut {
            out.extend(cut[k].2.iter().copied());
        } else if !inside(d) {
            out.push(ring[k]);
        }
    }
    out.dedup_by(|a, b| a.distance(*b) < 1e-3);
    (out, marks)
}

impl Fabric {
    /// Turns any draft into lots.
    pub fn build(d: &Draft, plan: &CityPlan, lib: &ProfileLibrary, seed: u64) -> Fabric {
        let mut laps = Laps::new();
        let graph = StreetGraph::build(&d.ways, &d.nodes, lib, d.snap);
        let relief = crate::plan::Relief::solve(&graph, lib, &d.hills, &d.plazas.iter().map(|p| p.ring.clone()).collect::<Vec<_>>());
        laps.lap("graph and grades");
        let corridors = graph.corridor(|s, i| s.width[i]);
        laps.lap("corridors");
        let plazas = Region::rings(d.plazas.iter().map(|p| p.ring.as_slice()));
        // Zone masks: earlier zones win where masks overlap.
        let mut by_zone: BTreeMap<u16, Vec<&[Vec2]>> = BTreeMap::new();
        for m in &d.masks {
            by_zone.entry(m.zone).or_default().push(m.ring.as_slice());
        }
        let mut zones: Vec<(u16, Region)> = Vec::new();
        let mut so_far = Region::empty();
        for (z, rings) in by_zone {
            let r = Region::rings(rings).minus(&so_far);
            so_far = so_far.union(&r);
            zones.push((z, r));
        }
        let masks = so_far;
        let domain = Region::union_all([&masks, &corridors, &plazas]);
        let land = masks.minus(&corridors.union(&plazas));
        let domain_loc = Located::new(domain.clone());
        laps.lap("land");
        // Footprints (OSM): de-overlapped in id order beforehand by the importer.
        let fps: Vec<(Region, &crate::plan::Footprint)> = d
            .footprints
            .iter()
            .map(|f| (Region::from_shapes(&[Shape { outer: f.ring.clone(), holes: f.holes.clone() }]).and(&land), f))
            .filter(|(r, _)| r.area() > 4.0)
            .collect();
        let mut blocks: Vec<Block> = Vec::new();
        let mut lot_regions: Vec<(Region, u32, Option<usize>)> = Vec::new();
        let mut parcel_courts: Vec<Region> = Vec::new();
        for (z, mask) in &zones {
            let zone = plan.zone(*z);
            let rules = zone.map(|z| z.lots).unwrap_or_default();
            let land_z = land.and(mask).opening(rules.neck.max(0.5));
            for shape in land_z.shapes() {
                let outer = clean_ring(&shape.outer, 0.005);
                if outer.len() < 3 {
                    continue;
                }
                let faces = |a: Vec2, b: Vec2| {
                    let d = (b - a).normalize_or(Vec2::X);
                    domain_loc.contains((a + b) * 0.5 + Vec2::new(d.y, -d.x) * 0.05)
                };
                let chamfer_of = |a: Vec2, b: Vec2| match street_of(&graph, a, b) {
                    Some(h) if h.miss < 1.5 => {
                        let p = lib.get(graph.street(h.s).profile);
                        (p.chamfer, p.round_corners)
                    }
                    _ => (0.0, 0.0),
                };
                let (ring, corners) = bevel(&outer, &faces, &chamfer_of, &rules, (seed, *z as u64));
                // The bevel only ever removes land (a chamfer chord across a concave
                // stretch would otherwise bulge into the street), and holes (public
                // space enclosed by land, e.g. a square no street reaches) stay public:
                // the block is cut through them so every block ring is simple.
                let land_piece = Region::from_shapes(std::slice::from_ref(&shape));
                let block_r = Region::ring(&ring).and(&land_piece);
                let pieces: Vec<Shape> = split_holes(&block_r).iter().flat_map(|r| r.shapes()).collect();
                for bs in pieces {
                    let key = bs.outer.iter().fold(Vec2::splat(f32::MAX), |m, p| if (p.x, p.y) < (m.x, m.y) { *p } else { m });
                    let key = point_key(key);
                    let bi = blocks.len() as u32;
                    let bregion = Region::from_shapes(std::slice::from_ref(&bs));
                    // Footprint lots first (their centroid in the block).
                    let mut rest = bregion.clone();
                    for (fi, (fr, _)) in fps.iter().enumerate() {
                        let Some([lo, hi]) = fr.bbox() else { continue };
                        let c = (lo + hi) * 0.5;
                        if !bregion.contains(c) && !bregion.intersects(fr) {
                            continue;
                        }
                        let piece = fr.and(&bregion);
                        if piece.area() < 4.0 {
                            continue;
                        }
                        rest = rest.minus(&piece);
                        for comp in split_holes(&piece) {
                            lot_regions.push((comp, bi, Some(fi)));
                        }
                    }
                    // Procedural lots on what is left.
                    for rs in rest.shapes() {
                        let r = Region::from_shapes(std::slice::from_ref(&rs));
                        let outer = rs.outer.clone();
                        let nn = outer.len();
                        let tags: Vec<Tag> = (0..nn)
                            .map(|i| {
                                let (a, b) = (outer[i], outer[(i + 1) % nn]);
                                let dd = (b - a).normalize_or(Vec2::X);
                                let q = (a + b) * 0.5 + Vec2::new(dd.y, -dd.x) * 0.05;
                                if !domain_loc.contains(q) || bregion.contains(q) {
                                    return Tag::Blind;
                                }
                                for (ci, c) in corners.iter().enumerate() {
                                    if c.chords.iter().any(|ch| seg_dist(a, ch[0], ch[1]) < 0.003 && seg_dist(b, ch[0], ch[1]) < 0.003) {
                                        return Tag::Chord(ci as u16);
                                    }
                                }
                                Tag::Front
                            })
                            .collect();
                        let apex: Vec<Option<u16>> = outer.iter().map(|p| corners.iter().position(|c| !c.corner && c.apex.distance(*p) < 0.003).map(|i| i as u16)).collect();
                        let theta = corners.iter().map(|c| c.theta).collect();
                        let front_len: f32 = (0..nn).filter(|&i| tags[i] != Tag::Blind).map(|i| outer[i].distance(outer[(i + 1) % nn])).sum();
                        if front_len < rules.min_width {
                            parcel_courts.push(r);
                            continue;
                        }
                        let z = if relief.is_flat() { Vec::new() } else { outer.iter().map(|q| relief.elev(*q)).collect() };
                        let br = BlockRing { pts: outer, tags, apex, theta, z };
                        let inr = r.pole().map(|p| p.1).unwrap_or(1.0);
                        let mut rng = Rng::keyed(seed, stage::PARCEL, key ^ point_key(rs.outer[0]));
                        let dbg = std::env::var("FORJA_FABRIC_DEBUG").is_ok().then(|| (br.pts.len(), br.tags.iter().filter(|t| **t == Tag::Front).count(), corners.len(), r.area()));
                        let parts = parcel::parcel(&rs, &br, &rules, inr, &mut rng);
                        if let Some(d) = dbg {
                            eprintln!("block {bi}: {} pts, {} front, {} corners, area {:.0} -> {} lots, court {:.0}", d.0, d.1, d.2, d.3, parts.lots.len(), parts.courts.area());
                        }
                        for l in parts.lots {
                            lot_regions.push((l, bi, None));
                        }
                        parcel_courts.push(parts.courts);
                    }
                    blocks.push(Block { shape: bs, zone: *z, key, corners: corners.clone(), lots: 0..0 });
                }
            }
        }
        laps.lap("blocks and parcels");
        // Setback jitter: each facade end steps back up to `jitter`.
        let front_ix = {
            let mut ix = SegIndex::new(8.0);
            for (bi, b) in blocks.iter().enumerate() {
                let o = &b.shape.outer;
                for i in 0..o.len() {
                    ix.push(o[i], o[(i + 1) % o.len()], bi as u32);
                }
            }
            ix
        };
        for (r, bi, fp) in lot_regions.iter_mut() {
            let b = &blocks[*bi as usize];
            let jitter = plan.zone(b.zone).map(|z| z.lots.jitter).unwrap_or(0.0);
            if jitter <= 0.0 || fp.is_some() {
                continue;
            }
            let key = r.shapes().first().map(|s| point_key(s.outer[0])).unwrap_or(0);
            let mut rng = Rng::keyed(seed, stage::JITTER, key);
            *r = setback(r, &front_ix, jitter, &mut rng, &domain_loc);
        }
        laps.lap("setbacks");
        // Lots never leave their block (setbacks and merges are region algebra);
        // the weld then clears the millimetre debris on the shared vertex set.
        let block_regions: Vec<Region> = blocks.iter().map(|b| Region::from_shapes(std::slice::from_ref(&b.shape))).collect();
        for (r, bi, _) in lot_regions.iter_mut() {
            let inside = r.and(&block_regions[*bi as usize]);
            if let Some(best) = inside.components().into_iter().max_by(|a, b| a.area().total_cmp(&b.area())) {
                *r = best;
            }
        }
        weld(&mut lot_regions, &front_ix, WELD);
        laps.lap("clip and weld");
        // Courts vs public: what is not lot, split topologically. Merging two
        // adjacent lots never changes it; dropping a lot does (`stale`).
        let streets = corridors.union(&plazas).tiles(crate::ground::TILE);
        let touches_streets = |c: &Region| {
            let Some([lo, hi]) = c.bbox() else { return false };
            let (k0, k1) = (streets.key(lo), streets.key(hi));
            (k0.0..=k1.0).any(|x| (k0.1..=k1.1).any(|z| streets.get((x, z)).is_some_and(|t| t.intersects(c))))
        };
        let split = |lots: &[(Region, u32, Option<usize>)]| {
            let rest = domain.minus(&Region::union_all(lots.iter().map(|l| &l.0)));
            let (public, courts): (Vec<Region>, Vec<Region>) = rest.components().into_iter().partition(touches_streets);
            let public = Region::union_all(public.iter());
            let ground = Ground::classes(&graph, lib, &public, &plazas, relief.clone());
            (public, Region::union_all(courts.iter()), ground)
        };
        let (mut public, mut courts, mut ground) = split(&lot_regions);
        let (mut stale, mut last) = (false, false);
        let mut cache: Vec<Option<(Lot, bool)>> = Vec::new();
        let mut stale_cache = false;
        // Classification and fronts; failing lots merge into a neighbour (or
        // finally become courts) and the lots are classified again.
        let mut dropped = 0;
        let mut round = 0;
        loop {
            round += 1;
            if std::mem::take(&mut stale) {
                (public, courts, ground) = split(&lot_regions);
                stale_cache = true;
            }
            laps.lap("public, courts and ground");
            if stale_cache {
                cache.clear();
                stale_cache = false;
            }
            let ctx = Ctx::new(&graph, lib, &lot_regions, &courts, &public, &domain, &ground, &blocks);
            if cache.len() != lot_regions.len() {
                cache = vec![None; lot_regions.len()];
            }
            let mut lots = Vec::new();
            let mut bad: Vec<usize> = Vec::new();
            for (i, (r, _bi, fp)) in lot_regions.iter().enumerate() {
                // A lot's classification depends only on the lots touching it
                // and the split: lots away from this round's merges keep theirs.
                if let Some((lot, is_bad)) = cache[i].take() {
                    if is_bad {
                        bad.push(i);
                    }
                    lots.push(lot);
                    continue;
                }
                let mut lot = ctx.classify(i, r);
                if let Some(fi) = fp {
                    lot.levels = fps[*fi].1.levels;
                    lot.roof_levels = fps[*fi].1.roof_levels;
                }
                let procedural = fp.is_none();
                let street_ok = !procedural || (0..lot.ring.len()).any(|k| lot.edges[k].is_street() && lot.ring[k].distance(lot.ring[(k + 1) % lot.ring.len()]) >= 2.0);
                if lot.check().is_err() || !street_ok {
                    if std::env::var("FORJA_FABRIC_DEBUG").is_ok() {
                        eprintln!("round {round} lot {i}: {:?} street_ok {street_ok} ring {:?} edges {:?}", lot.check(), lot.ring, lot.edges);
                    }
                    bad.push(i);
                }
                lots.push(lot);
            }
            laps.lap("classify");
            if !bad.is_empty() && round >= 3 && !last {
                // Lots that merging could not repair leave: the plot joins
                // public space (or a court) and every lot is classified once
                // more against the new split.
                let mut keep = vec![true; lot_regions.len()];
                for &i in &bad {
                    keep[i] = false;
                }
                dropped += bad.len();
                let mut k = 0;
                lot_regions.retain(|_| {
                    k += 1;
                    keep[k - 1]
                });
                stale = true;
                last = true;
                continue;
            }
            if bad.is_empty() || last {
                // Final (bad lots here are a last resort: they become courts).
                let mut keep = vec![true; lots.len()];
                for &i in &bad {
                    keep[i] = false;
                    dropped += 1;
                }
                let mut infos = Vec::new();
                let mut out_lots = Vec::new();
                let mut order: Vec<usize> = (0..lots.len()).filter(|&i| keep[i]).collect();
                order.sort_by_key(|&i| lot_regions[i].1);
                for i in order {
                    let (_, bi, fp) = &lot_regions[i];
                    let lot = std::mem::take(&mut lots[i]);
                    infos.push(ctx.info(&lot, *bi, blocks[*bi as usize].zone, fp.is_some()));
                    out_lots.push(lot);
                }
                if !bad.is_empty() {
                    // The dropped lots join the courts.
                    let lost = Region::union_all(bad.iter().map(|&i| &lot_regions[i].0));
                    let courts = courts.union(&lost);
                    return Fabric::finish(graph, corridors, plazas, masks, domain, blocks, out_lots, infos, courts, public, ground, dropped, lib);
                }
                return Fabric::finish(graph, corridors, plazas, masks, domain, blocks, out_lots, infos, courts, public, ground, dropped, lib);
            }
            // Merge each bad lot into its best neighbour in the same block.
            let mut gone = vec![false; lot_regions.len()];
            let mut changed: Vec<[Vec2; 2]> = Vec::new();
            for &i in &bad {
                if gone[i] {
                    continue;
                }
                let bi = lot_regions[i].1;
                let best = (0..lot_regions.len())
                    .filter(|&j| j != i && !gone[j] && lot_regions[j].1 == bi && lot_regions[j].2.is_none() == lot_regions[i].2.is_none())
                    .map(|j| (parcel::shared_len(&lot_regions[i].0, &lot_regions[j].0), j))
                    .filter(|(l, _)| *l > 0.1)
                    .max_by(|a, b| a.0.total_cmp(&b.0).then(b.1.cmp(&a.1)));
                // A lot that cannot merge stays bad until the final round,
                // which drops every bad lot at once.
                if let Some((_, j)) = best {
                    let m = lot_regions[i].0.union(&lot_regions[j].0);
                    if m.len() == 1 && m.shapes()[0].holes.is_empty() {
                        changed.extend(m.bbox());
                        lot_regions[j].0 = m;
                        gone[i] = true;
                    }
                }
            }
            // Keep the classification of every lot clear of the merges.
            let near = |r: &Region| r.bbox().is_some_and(|[lo, hi]| changed.iter().any(|[a, b]| lo.x <= b.x + 0.1 && hi.x >= a.x - 0.1 && lo.y <= b.y + 0.1 && hi.y >= a.y - 0.1));
            cache = lots.into_iter().zip(lot_regions.iter()).enumerate().map(|(i, (lot, (r, _, _)))| (!near(r)).then(|| (lot, bad.contains(&i)))).collect();
            let mut k = 0;
            lot_regions.retain(|_| {
                k += 1;
                !gone[k - 1]
            });
            let mut k = 0;
            cache.retain(|_| {
                k += 1;
                !gone[k - 1]
            });
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn finish(graph: StreetGraph, corridors: Region, plazas: Region, masks: Region, domain: Region, mut blocks: Vec<Block>, lots: Vec<Lot>, info: Vec<LotInfo>, courts: Region, public: Region, ground: Ground, dropped: usize, lib: &ProfileLibrary) -> Fabric {
        // Lot ranges per block (lots are sorted by block).
        for b in blocks.iter_mut() {
            b.lots = 0..0;
        }
        let mut i = 0;
        while i < info.len() {
            let bi = info[i].block as usize;
            let start = i;
            while i < info.len() && info[i].block as usize == bi {
                i += 1;
            }
            blocks[bi].lots = start..i;
        }
        let extent = domain.bbox().unwrap_or([Vec2::ZERO, Vec2::ZERO]);
        let mut stats = vec![
            ("blocks".to_string(), blocks.iter().filter(|b| !b.lots.is_empty()).count()),
            ("lots".to_string(), lots.len()),
            ("streets".to_string(), graph.streets.len()),
            ("junctions".to_string(), ground.junctions.len()),
            ("dropped_lots".to_string(), dropped),
        ];
        let alleys = graph.streets.iter().filter(|s| lib.get(s.profile).class == Class::Alley).count();
        stats.push(("alleys".to_string(), alleys));
        Fabric { graph, corridors, plazas, masks, domain, blocks, lots, info, courts, public, ground, extent, stats, dropped, zone_depth: BTreeMap::new() }
    }

    /// Exact hash of the lots (rings on the mm grid, kinds, fronts' ways).
    pub fn hash64(&self) -> u64 {
        let mut h = mix64(self.lots.len() as u64 ^ self.graph.hash64());
        for (l, inf) in self.lots.iter().zip(&self.info) {
            for (p, e) in l.ring.iter().zip(&l.edges) {
                let q = crate::plan::region::snap(*p);
                h = mix64(h ^ crate::plan::region::to_mm(q.x) as u64 ^ (crate::plan::region::to_mm(q.y) as u64) << 32 ^ (*e as u64) << 60);
            }
            h = mix64(h ^ inf.key ^ (l.base * 1000.0) as u64);
        }
        h
    }
}

/// Welds lot vertices closer than `tol` into one (globally, so shared
/// party walls stay shared), preferring vertices on the block frontage,
/// and removes the spikes this leaves. Clears the millimetre debris of
/// region algebra (micro edges, slivers) before the contract check.
fn weld(lots: &mut [(Region, u32, Option<usize>)], front: &SegIndex, tol: f32) {
    let rings: Vec<Vec<Vec2>> = lots.iter().map(|(r, _, _)| r.shapes().into_iter().next().map(|s| s.outer).unwrap_or_default()).collect();
    let key = |p: Vec2| (crate::plan::region::to_mm(p.x), crate::plan::region::to_mm(p.y));
    let mut ids: HashMap<(i32, i32), usize> = HashMap::new();
    let mut pts: Vec<Vec2> = Vec::new();
    for r in &rings {
        for p in r {
            ids.entry(key(*p)).or_insert_with(|| {
                pts.push(*p);
                pts.len() - 1
            });
        }
    }
    // Union-find by distance on a hash grid.
    let mut uf: Vec<usize> = (0..pts.len()).collect();
    fn find(uf: &mut [usize], mut i: usize) -> usize {
        while uf[i] != i {
            uf[i] = uf[uf[i]];
            i = uf[i];
        }
        i
    }
    let cell = tol * 2.0;
    let mut grid: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    for (i, p) in pts.iter().enumerate() {
        grid.entry(((p.x / cell).floor() as i64, (p.y / cell).floor() as i64)).or_default().push(i);
    }
    for (i, p) in pts.iter().enumerate() {
        let (cx, cz) = ((p.x / cell).floor() as i64, (p.y / cell).floor() as i64);
        for x in cx - 1..=cx + 1 {
            for z in cz - 1..=cz + 1 {
                for &j in grid.get(&(x, z)).into_iter().flatten() {
                    if j > i && pts[j].distance(*p) < tol {
                        let (a, b) = (find(&mut uf, i), find(&mut uf, j));
                        if a != b {
                            uf[a.max(b)] = a.min(b);
                        }
                    }
                }
            }
        }
    }
    let on_front = |p: Vec2| front.nearest(p, 0.002).is_some();
    let mut rep: HashMap<usize, usize> = HashMap::new();
    for i in 0..pts.len() {
        let r = find(&mut uf, i);
        let e = rep.entry(r).or_insert(i);
        let better = |a: usize, b: usize| (!on_front(pts[a]), key(pts[a])) < (!on_front(pts[b]), key(pts[b]));
        if better(i, *e) {
            *e = i;
        }
    }
    let mut map = |p: Vec2| pts[rep[&find(&mut uf, ids[&key(p)])]];
    for (k, ring) in rings.iter().enumerate() {
        if ring.is_empty() {
            continue;
        }
        let mut r: Vec<Vec2> = ring.iter().map(|p| map(*p)).collect();
        // Drop repeats and back-and-forth spikes until stable.
        loop {
            let n = r.len();
            if n < 3 {
                break;
            }
            let mut changed = false;
            for i in 0..n {
                let (a, b, c) = (r[(i + n - 1) % n], r[i], r[(i + 1) % n]);
                if a == b || a == c {
                    r.remove(i);
                    changed = true;
                    break;
                }
                let _ = c;
            }
            if !changed {
                break;
            }
        }
        if r.len() >= 3 && crate::geom::signed_area(&r) > 0.0 {
            let reg = Region::ring(&r);
            if let Some(best) = reg.components().into_iter().max_by(|a, b| a.area().total_cmp(&b.area())) {
                lots[k].0 = best;
            }
        }
    }
}

/// Splits a region with holes into simple pieces (a cut through each hole
/// to the outside), so every lot ring is simple.
fn split_holes(r: &Region) -> Vec<Region> {
    let mut out = Vec::new();
    for s in r.shapes() {
        if s.holes.is_empty() {
            out.push(Region::from_shapes(&[s]));
            continue;
        }
        let one = Region::from_shapes(std::slice::from_ref(&s));
        let mut cuts = Vec::new();
        for h in &s.holes {
            let c = h.iter().copied().sum::<Vec2>() / h.len() as f32;
            // A vertical line through the hole's centroid splits the shape.
            let [lo, hi] = one.bbox().unwrap_or([c, c]);
            cuts.push(vec![Vec2::new(c.x, lo.y - 1.0), Vec2::new(c.x, hi.y + 1.0)]);
        }
        for piece in one.slice(&cuts) {
            out.push(Region::from_shapes(&[Shape { outer: piece.outer, holes: Vec::new() }]).minus(&Region::rings(s.holes.iter().map(|h| h.as_slice()))));
        }
    }
    out.into_iter().flat_map(|r| r.components()).filter(|r| r.area() > 1.0).collect()
}

/// Steps each facade end back by up to `jitter` (and so rotates it a
/// little): the lot loses a strip along its frontage that joins public
/// space. Lots only shrink, nothing enters a street.
fn setback(r: &Region, front: &SegIndex, jitter: f32, rng: &mut Rng, domain: &Located) -> Region {
    let Some(s) = r.shapes().into_iter().next() else { return r.clone() };
    let o = &s.outer;
    let n = o.len();
    // Frontage chain: ring edges on a block's outline facing public space.
    let on_front: Vec<bool> = (0..n)
        .map(|i| {
            let (a, b) = (o[i], o[(i + 1) % n]);
            let d = (b - a).normalize_or(Vec2::X);
            let q = (a + b) * 0.5 + Vec2::new(d.y, -d.x) * 0.05;
            parcel::overlap_len(a, b, front) > a.distance(b) * 0.9 && domain.contains(q)
        })
        .collect();
    if !on_front.iter().any(|f| *f) {
        return r.clone();
    }
    // Longest consecutive chain.
    let start = (0..n).find(|&i| on_front[i] && !on_front[(i + n - 1) % n]).unwrap_or(0);
    let mut chain = vec![o[start]];
    let mut k = start;
    while on_front[k] && chain.len() <= n {
        chain.push(o[(k + 1) % n]);
        k = (k + 1) % n;
    }
    let (d0, d1) = (rng.range(0.0, jitter), rng.range(0.0, jitter));
    let pl = crate::plan::Polyline::new(chain);
    let len = pl.len();
    if len < 0.5 {
        return r.clone();
    }
    // Extend 1 m past both ends so oblique party walls leave no sliver.
    let p = pl.pts();
    let (t0, t1) = ((p[1] - p[0]).normalize_or(Vec2::X), (p[p.len() - 1] - p[p.len() - 2]).normalize_or(Vec2::X));
    let mut ext = vec![p[0] - t0];
    ext.extend_from_slice(p);
    ext.push(p[p.len() - 1] + t1);
    let st = crate::plan::Polyline::new(ext.clone());
    let delta = |k: usize| d0 + (d1 - d0) * ((st.stations()[k] - 1.0) / len).clamp(0.0, 1.0);
    let strip = crate::plan::line::stroke(&ext, delta);
    let out = r.minus(&strip);
    out.components().into_iter().max_by(|a, b| a.area().total_cmp(&b.area())).unwrap_or_else(|| r.clone())
}

/// Lookups shared by the classification of every lot.
struct Ctx<'a> {
    graph: &'a StreetGraph,
    lib: &'a ProfileLibrary,
    rings: Vec<Vec<Vec2>>,
    lot_of_block: Vec<u32>,
    lot_grid: HashMap<(i32, i32), Vec<u32>>,
    vertex_grid: HashMap<(i64, i64), Vec<(u32, Vec2)>>,
    obstacles: SegIndex,
    courts: Located,
    public: Located,
    ground: &'a Ground,
    blocks: &'a [Block],
}

fn ring_contains(r: &[Vec2], p: Vec2) -> bool {
    let mut c = false;
    let n = r.len();
    for i in 0..n {
        let (a, b) = (r[i], r[(i + 1) % n]);
        if (a.y > p.y) != (b.y > p.y) && p.x < a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x) {
            c = !c;
        }
    }
    c
}

impl<'a> Ctx<'a> {
    #[allow(clippy::too_many_arguments)]
    fn new(graph: &'a StreetGraph, lib: &'a ProfileLibrary, lots: &[(Region, u32, Option<usize>)], courts: &Region, public: &Region, domain: &Region, ground: &'a Ground, blocks: &'a [Block]) -> Self {
        let rings: Vec<Vec<Vec2>> = lots.iter().map(|(r, _, _)| r.shapes().into_iter().next().map(|s| s.outer).unwrap_or_default()).collect();
        let mut lot_grid: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        let mut vertex_grid: HashMap<(i64, i64), Vec<(u32, Vec2)>> = HashMap::new();
        let mut obstacles = SegIndex::new(8.0);
        for (i, r) in rings.iter().enumerate() {
            if r.is_empty() {
                continue;
            }
            let (lo, hi) = r.iter().fold((Vec2::splat(f32::MAX), Vec2::splat(f32::MIN)), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
            for x in (lo.x / 16.0).floor() as i32..=(hi.x / 16.0).floor() as i32 {
                for z in (lo.y / 16.0).floor() as i32..=(hi.y / 16.0).floor() as i32 {
                    lot_grid.entry((x, z)).or_default().push(i as u32);
                }
            }
            for k in 0..r.len() {
                let p = r[k];
                vertex_grid.entry(((p.x / 4.0).floor() as i64, (p.y / 4.0).floor() as i64)).or_default().push((i as u32, p));
                obstacles.push(p, r[(k + 1) % r.len()], i as u32);
            }
        }
        for (a, b) in domain.edges() {
            obstacles.push(Vec2::new(a[0] as f32, a[1] as f32), Vec2::new(b[0] as f32, b[1] as f32), u32::MAX);
        }
        Ctx { graph, lib, rings, lot_of_block: lots.iter().map(|l| l.1).collect(), lot_grid, vertex_grid, obstacles, courts: Located::new(courts.clone()), public: Located::new(public.clone()), ground, blocks }
    }

    fn lot_at(&self, p: Vec2, not: usize) -> Option<usize> {
        let k = ((p.x / 16.0).floor() as i32, (p.y / 16.0).floor() as i32);
        self.lot_grid.get(&k)?.iter().map(|&i| i as usize).find(|&i| i != not && ring_contains(&self.rings[i], p))
    }

    /// Edges, kinds, fronts and base of lot `li`.
    fn classify(&self, li: usize, _r: &Region) -> Lot {
        let src = &self.rings[li];
        if src.len() < 3 {
            return Lot::default();
        }
        // Split edges at other lots' vertices lying on them.
        let mut ring: Vec<Vec2> = Vec::new();
        let n = src.len();
        for i in 0..n {
            let (a, b) = (src[i], src[(i + 1) % n]);
            ring.push(a);
            let len = a.distance(b);
            let d = (b - a) / len.max(1e-6);
            let mut extra: Vec<(f32, Vec2)> = Vec::new();
            let (lo, hi) = (a.min(b), a.max(b));
            for x in (lo.x / 4.0).floor() as i64..=(hi.x / 4.0).floor() as i64 {
                for z in (lo.y / 4.0).floor() as i64..=(hi.y / 4.0).floor() as i64 {
                    let Some(l) = self.vertex_grid.get(&(x, z)) else { continue };
                    for &(o, p) in l {
                        if o as usize == li {
                            continue;
                        }
                        let t = (p - a).dot(d);
                        if t > 0.05 && t < len - 0.05 && seg_dist(p, a, b) < 0.0015 {
                            extra.push((t, p));
                        }
                    }
                }
            }
            extra.sort_by(|x, y| x.0.total_cmp(&y.0));
            extra.dedup_by(|x, y| (x.0 - y.0).abs() < 0.05);
            ring.extend(extra.into_iter().map(|e| e.1));
        }
        let n = ring.len();
        let block = &self.blocks[self.lot_of_block[li] as usize];
        // Probe each edge.
        let mut edges = Vec::with_capacity(n);
        let mut hits: Vec<Option<Hit>> = vec![None; n];
        for i in 0..n {
            let (a, b) = (ring[i], ring[(i + 1) % n]);
            let d = (b - a).normalize_or(Vec2::X);
            let out = Vec2::new(d.y, -d.x);
            let q = (a + b) * 0.5 + out * 0.05;
            let kind = if self.lot_at(q, li).is_some() {
                EdgeKind::Party
            } else if self.courts.contains(q) {
                EdgeKind::Courtyard
            } else if self.public.contains(q) {
                hits[i] = street_of(self.graph, a, b);
                let on_chord = block.corners.iter().any(|c| c.chords.iter().any(|ch| seg_dist(a, ch[0], ch[1]) < 0.003 && seg_dist(b, ch[0], ch[1]) < 0.003));
                if on_chord { EdgeKind::Corner } else { EdgeKind::Street }
            } else {
                EdgeKind::Party
            };
            edges.push(kind);
        }
        // Short returns beside party walls are party walls too.
        for i in 0..n {
            if edges[i].is_street() && ring[i].distance(ring[(i + 1) % n]) < 0.6 && (edges[(i + n - 1) % n] == EdgeKind::Party || edges[(i + 1) % n] == EdgeKind::Party) {
                edges[i] = EdgeKind::Party;
            }
        }
        // Fronts.
        let mut fronts = vec![Front::UNCONSTRAINED; n];
        let mut base = f32::NEG_INFINITY;
        let mut any_street = false;
        for i in 0..n {
            if !edges[i].is_street() {
                continue;
            }
            any_street = true;
            fronts[i] = self.front(li, &ring, i, hits[i]);
            let (a, b) = (ring[i], ring[(i + 1) % n]);
            let d = (b - a).normalize_or(Vec2::X);
            let out = Vec2::new(d.y, -d.x);
            for t in [0.15, 0.5, 0.85] {
                base = base.max(self.ground.height_at(a.lerp(b, t) + out * 0.2));
            }
        }
        // Corner edges reach no further than either street allows.
        for i in 0..n {
            if edges[i] != EdgeKind::Corner {
                continue;
            }
            let mut reach = fronts[i].reach;
            for j in [(i + n - 1) % n, (i + 1) % n] {
                if edges[j].is_street() {
                    reach = reach.min(fronts[j].reach);
                }
            }
            fronts[i].reach = reach;
            fronts[i].miradores = fronts[i].miradores && reach >= 0.8;
        }
        if !any_street {
            let c = ring.iter().copied().sum::<Vec2>() / n as f32;
            base = 0.15 + self.ground.relief.elev(c);
        }
        let mut lot = Lot::new(ring, edges, base);
        lot.fronts = fronts;
        lot
    }

    fn front(&self, li: usize, ring: &[Vec2], i: usize, hit: Option<Hit>) -> Front {
        let n = ring.len();
        let (a, b) = (ring[i], ring[(i + 1) % n]);
        let len = a.distance(b);
        let d = (b - a) / len.max(1e-6);
        let out = Vec2::new(d.y, -d.x);
        let m = (a + b) * 0.5;
        // Clear width: to the first facade or map edge opposite.
        let clear = self.obstacles.ray_filter(m + out * 0.01, out, 60.0, |k| self.obstacles.seg(k).2 as usize != li).map(|h| h.0).unwrap_or(60.0);
        let probe = m + out * (clear * 0.5).min(30.0);
        let open = self.ground.in_plaza(probe) || self.ground.in_plaza(m + out * 0.3);
        let clear = if open { self.ground.plaza_radius(probe).or(self.ground.plaza_radius(m + out * 0.3)).map(|r| (2.0 * r).max(clear)).unwrap_or(clear) } else { clear };
        let street = hit.filter(|h| h.miss < 3.0);
        let prof = street.map(|h| self.lib.get(self.graph.street(h.s).profile)).or_else(|| self.lib.by_name("plaza")).unwrap_or(&self.lib.list[0]);
        let shared = street.is_none_or(|h| self.graph.street(h.s).carriageway <= 0.0) || open;
        let walk = if shared { (clear * 0.5).min(6.0) } else { self.ground.to_carriage(m + out * 0.01, out, clear).unwrap_or(clear).min(clear) };
        let mut f = prof.front.front(clear.max(0.5), walk.max(0.0));
        f.open = open;
        // Depth: to the lot's far side.
        let inward = -out;
        let mut depth = f32::INFINITY;
        for k in 0..n {
            if k == i {
                continue;
            }
            let (p, q) = (ring[k], ring[(k + 1) % n]);
            let e = q - p;
            let den = inward.perp_dot(e);
            if den.abs() < 1e-9 {
                continue;
            }
            let w = p - m;
            let t = w.perp_dot(e) / den;
            let u = w.perp_dot(inward) / den;
            if t > 1e-3 && (0.0..=1.0).contains(&u) {
                depth = depth.min(t);
            }
        }
        f.depth = if depth.is_finite() { depth } else { 0.0 };
        if let Some(h) = street {
            let st = self.graph.street(h.s);
            f.way = h.s;
            f.along = [st.line.project(a).0, st.line.project(b).0];
            if prof.lamps.mount == Mount::Wall {
                let pitch = prof.lamps.pitch.max(4.0);
                f.lamps = Some(Rhythm { pitch, phase: if h.side > 0.0 { 0.0 } else { pitch * 0.5 } });
            }
        }
        f
    }

    fn info(&self, lot: &Lot, block: u32, zone: u16, footprint: bool) -> LotInfo {
        // The pool of the highest-ranked street faced (plazas rank as 5).
        let mut best: Option<(u8, String, Option<StreetId>)> = None;
        for (k, f) in lot.fronts.iter().enumerate() {
            if !lot.edges[k].is_street() {
                continue;
            }
            let (rank, pool, s) = if f.open {
                (5, "plaza".to_string(), None)
            } else if f.way != Front::NO_WAY {
                let p = self.lib.get(self.graph.street(f.way).profile);
                (p.rank, p.pool().to_string(), Some(f.way))
            } else {
                (0, "street".to_string(), None)
            };
            let len = lot.ring[k].distance(lot.ring[(k + 1) % lot.ring.len()]);
            if len < 2.0 {
                continue;
            }
            if best.as_ref().is_none_or(|b| rank > b.0) {
                best = Some((rank, pool, s));
            }
        }
        let (_, pool, main) = best.unwrap_or((0, "street".into(), None));
        let start = lot.ring.first().copied().unwrap_or(Vec2::ZERO);
        LotInfo { zone, block, main, pool, key: point_key(start) ^ (block as u64) << 48, footprint }
    }
}

/// Invariant failures found by `Fabric::validate`.
#[derive(Clone, Debug, PartialEq)]
pub struct Violation(pub String);

impl Fabric {
    /// Checks the layout guarantees (L1–L9) and the ground partition.
    pub fn validate(&self) -> Result<(), Vec<Violation>> {
        let mut v = Vec::new();
        let mut area = 0.0f64;
        for (i, l) in self.lots.iter().enumerate() {
            if let Err(e) = l.check() {
                v.push(Violation(format!("lot {i}: {e}")));
            }
            if l.fronts.len() != l.ring.len() {
                v.push(Violation(format!("lot {i}: {} fronts", l.fronts.len())));
            }
            if !self.info[i].footprint && !(0..l.ring.len()).any(|k| l.edges[k].is_street() && l.ring[k].distance(l.ring[(k + 1) % l.ring.len()]) >= 2.0) {
                v.push(Violation(format!("lot {i}: no street edge of 2 m")));
            }
            // L8: base is the highest ground in front of the facades (the
            // layout fills any lower ground with a plinth).
            let ground_max = (0..l.ring.len()).filter(|&k| l.edges[k].is_street()).flat_map(|k| {
                let (a, b) = l.edge(k);
                let d = (b - a).normalize_or(Vec2::X);
                [0.15, 0.5, 0.85].map(|t| self.ground.height_at(a.lerp(b, t) + Vec2::new(d.y, -d.x) * 0.2))
            }).fold(f32::NEG_INFINITY, f32::max);
            if ground_max.is_finite() && (l.base - ground_max).abs() > 0.02 {
                v.push(Violation(format!("lot {i}: base {} vs ground {ground_max}", l.base)));
            }
            area += l.area() as f64;
        }
        let union = Region::union_all(self.lots.iter().map(|l| Region::ring(&l.ring)).collect::<Vec<_>>().iter());
        if (union.area() - area).abs() > 1e-3 * area.max(1.0) + 0.5 {
            v.push(Violation(format!("lots overlap: {} vs {}", union.area(), area)));
        }
        let cover = Region::union_all([&union, &self.courts, &self.public]);
        if (cover.area() - self.domain.area()).abs() > 1.0 + 1e-4 * self.domain.area() {
            v.push(Violation(format!("partition: {} vs domain {}", cover.area(), self.domain.area())));
        }
        let classes = [&self.ground.carriage, &self.ground.median, &self.ground.shared, &self.ground.plaza, &self.ground.walk];
        let sum: f64 = classes.iter().map(|c| c.area()).sum();
        if (sum - self.public.area()).abs() > 1.0 + 1e-4 * self.public.area() {
            v.push(Violation(format!("ground classes {} vs public {}", sum, self.public.area())));
        }
        // Nothing deeper than the weld tolerance inside the street space.
        let on_streets = union.and(&self.corridors.offset(-WELD, Join::Miter(0.3)));
        if on_streets.area() > 0.01 {
            let at: Vec<String> = on_streets.components().iter().take(6).map(|c| {
                let [lo, hi] = c.bbox().unwrap_or_default();
                format!("{:.3} m² at ({:.1}, {:.1})", c.area(), (lo.x + hi.x) * 0.5, (lo.y + hi.y) * 0.5)
            }).collect();
            v.push(Violation(format!("lots on streets: {:.3} m²: {}", on_streets.area(), at.join(", "))));
        }
        if v.is_empty() { Ok(()) } else { Err(v) }
    }
}
