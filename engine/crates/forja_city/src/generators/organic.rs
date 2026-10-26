//! Organic old towns, driven by accessibility.
//!
//! The rule behind old Mediterranean fabric is that every plot must be
//! reachable: streets are cut only where land is too deep. One loop runs
//! over tiers, from spines to callejones. While a piece of land still has
//! points farther than the tier's `d_max` from any public space (its
//! *core*), paths are carved across it between its gates (where streets
//! end against it, plus spaced points on its edge): the minimum spanning
//! tree of the gates through the core, a few loops, and, if the core
//! survives, an adarve (dead end) or a through path from the core's pole.
//!
//! Termination: an adarve ends at the pole of the current core, which is
//! more than `d_max` from all public space, earlier paths included; the
//! successive poles are therefore more than `d_max` apart and there are at
//! most about `area / (π d_max² / 4)` of them. A cap of four times that
//! bound guards the loop (and is asserted in tests).

use forja_math::Vec2;
use serde::Deserialize;

use super::Zone;
use super::carve::Lattice;
use crate::fabric::stage;
use crate::geom::{Rng, point_key};
use crate::plan::line::seg_dist;
use crate::plan::{Draft, Join, Located, Plaza, PlazaKind, Polyline, Region, SegIndex, StreetGraph, Way, noise1};
use crate::profile::ProfileLibrary;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tier {
    pub profile: String,
    /// Deepest a plot may lie from public space after this tier (m).
    pub d_max: f32,
    /// Chance that a leftover core gets a dead end instead of a through path.
    #[serde(default)]
    pub adarve: f32,
    /// Chance of extra loops between gates.
    #[serde(default)]
    pub loops: f32,
    /// Chance of a small square at a new junction.
    #[serde(default)]
    pub plazuelas: f32,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MainPlaza {
    pub radius: [f32; 2],
    pub vertices: [u32; 2],
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrganicSpec {
    #[serde(default)]
    pub plaza: Option<MainPlaza>,
    #[serde(default = "d_meander")]
    pub meander: f32,
    #[serde(default = "d_lattice")]
    pub lattice: f32,
    pub tiers: Vec<Tier>,
}

fn d_meander() -> f32 {
    0.6
}

fn d_lattice() -> f32 {
    2.0
}

/// A jittered convex polygon.
/// An irregular square: `n` corners around `c` at about radius `r`, with
/// unequal sides (angles jittered by a third of a step, radii by ±20%).
fn blob(c: Vec2, r: f32, n: u32, rng: &mut Rng) -> Vec<Vec2> {
    let a0 = rng.range(0.0, std::f32::consts::TAU);
    let step = std::f32::consts::TAU / n as f32;
    (0..n).map(|i| c + Vec2::from_angle(a0 + step * (i as f32 + rng.range(-0.33, 0.33))) * r * rng.range(0.8, 1.15)).collect()
}

struct State<'a> {
    lib: &'a ProfileLibrary,
    zi: u16,
    seed: u64,
    land: Region,
    out: Draft,
    /// Centrelines that new paths may end on (existing and carved), with
    /// their widest width.
    lines: Vec<(Vec<Vec2>, f32)>,
    /// Lattice step: how far a path end may stop short of a street's edge.
    lattice: f32,
    /// Graph nodes of the streets around: T stems continue into the land.
    stems: Vec<(Vec2, Vec2, f32)>,
    iterations: usize,
}

impl State<'_> {
    /// Segment index of the lines paths may join (payload: line index),
    /// with the widest line's reach.
    fn line_index(&self) -> (SegIndex, f32) {
        let mut ix = SegIndex::new(16.0);
        for (i, (l, _)) in self.lines.iter().enumerate() {
            for s in l.windows(2) {
                ix.push(s[0], s[1], i as u32);
            }
        }
        (ix, self.reach_of(self.lines.iter().map(|l| l.1).fold(0.0, f32::max)))
    }

    /// Half a line's width plus the inset and one and a half lattice steps:
    /// how far short of a centreline a path end may stop and still join it.
    fn reach_of(&self, w: f32) -> f32 {
        w * 0.5 + 1.0 + 1.5 * self.lattice
    }

    /// Whether a path ending at `p` would join a line (see `attach`).
    fn reaches(&self, (ix, max): &(SegIndex, f32), p: Vec2) -> bool {
        ix.around(p, *max).into_iter().any(|k| {
            let (a, b, li) = ix.seg(k);
            seg_dist(p, a, b) < self.reach_of(self.lines[li as usize].1)
        })
    }
}

/// Generates an organic zone inside `mask`, around the streets of `d`.
pub fn generate(o: &OrganicSpec, zone: &Zone, zi: u16, mask: &Region, d: &Draft, lib: &ProfileLibrary, seed: u64) -> (Draft, super::Carved) {
    let graph = StreetGraph::build(&d.ways, &d.nodes, lib, d.snap.max(0.05));
    let corridors = graph.corridor(|s, i| s.width[i]);
    let plazas = Region::rings(d.plazas.iter().map(|p| p.ring.as_slice()));
    let land = mask.minus(&corridors).minus(&plazas);
    let lines = graph.streets.iter().map(|s| (s.line.pts().to_vec(), s.width.iter().copied().fold(0.0, f32::max))).collect();
    // T stems: a node of degree 3 whose odd arm points away has a mouth
    // continuing across the street.
    let mut stems = Vec::new();
    for n in 0..graph.nodes.len() as u32 {
        let star = graph.star(n);
        if star.len() != 3 {
            continue;
        }
        let dirs: Vec<Vec2> = star.iter().map(|a| graph.street(a.0).heading(a.1, 4.0)).collect();
        for k in 0..3 {
            let (a, b) = (dirs[(k + 1) % 3], dirs[(k + 2) % 3]);
            if a.dot(b) < -0.9 && dirs[k].dot(a).abs() < 0.5 {
                let s = graph.street(star[k].0);
                let through = graph.street(star[(k + 1) % 3].0);
                let _ = s;
                stems.push((graph.nodes[n as usize].p, -dirs[k], through.width_at(through.end_s(star[(k + 1) % 3].1))));
            }
        }
    }
    let mut st = State { lib, zi, seed, land, out: Draft::default(), lines, lattice: o.lattice, stems, iterations: 0 };
    let mut rng = Rng::keyed(seed, stage::CARVE, zi as u64);
    // The main plaza, at the deepest point of the zone.
    if let (Some(mp), Some((c, r))) = (&o.plaza, st.land.pole()) {
        let rad = rng.range(mp.radius[0], mp.radius[1]).min(r * 0.6);
        let n = mp.vertices[0] + rng.below((mp.vertices[1].saturating_sub(mp.vertices[0]) + 1) as usize) as u32;
        if rad > 6.0 {
            let ring = blob(c, rad, n.max(4), &mut rng);
            st.land = st.land.minus(&Region::ring(&ring));
            st.out.plazas.push(Plaza { ring, zone: zi, kind: zone.plaza_kind });
        }
    }
    let mut capped = false;
    for (ti, tier) in o.tiers.iter().enumerate() {
        capped |= !carve_tier(&mut st, o, tier, ti);
    }
    let d_max = o.tiers.last().map(|t| t.d_max).unwrap_or(f32::INFINITY);
    (st.out, super::Carved { zone: zi, d_max, iterations: st.iterations, capped })
}

/// Carves until no land of this zone is deeper than `tier.d_max`. False if
/// the loop stopped at its safety cap instead.
fn carve_tier(st: &mut State, o: &OrganicSpec, tier: &Tier, ti: usize) -> bool {
    let Some(prof) = st.lib.by_name(&tier.profile) else { return true };
    let [w0, w1] = prof.width;
    let area = st.land.area() as f32;
    let cap = (4.0 * area / (std::f32::consts::PI * tier.d_max * tier.d_max / 4.0)).ceil() as usize + 8;
    let mut guard = 0;
    let mut shallow = std::collections::HashSet::new();
    loop {
        guard += 1;
        if guard > cap {
            log::warn!("organic zone {}: tier {ti} hit its safety cap", st.zi);
            return false;
        }
        // The largest piece that still has a core. Land only shrinks, so a
        // piece seen before without a core never gets one: remembered by hash.
        let mut pieces = st.land.components();
        pieces.sort_by(|a, b| b.area().total_cmp(&a.area()));
        let mut best: Option<(Region, Region)> = None;
        for piece in pieces {
            let h = piece.hash64();
            if shallow.contains(&h) {
                continue;
            }
            let core = piece.offset(-tier.d_max, Join::Round);
            if core.is_empty() {
                shallow.insert(h);
                continue;
            }
            best = Some((piece, core));
            break;
        }
        let Some((piece, core)) = best else { break };
        st.iterations += 1;
        let key = point_key(piece.bbox().map(|b| b[0]).unwrap_or(Vec2::ZERO)) ^ (ti as u64) << 56;
        let mut rng = Rng::keyed(st.seed, stage::CARVE, key);
        let inside = Located::new(piece.offset(-1.0, Join::Round));
        let edge = Located::new(piece.clone());
        let mut lat = Lattice::new(&inside, &edge, o.lattice, o.meander, st.seed ^ key);
        if lat.is_empty() {
            // Too thin for the lattice yet deep: cut straight through its pole.
            let Some((p, _)) = core.pole() else { break };
            let line = straight_through(&piece, p);
            add_way(st, line, prof.width, &tier.profile, &mut rng);
            continue;
        }
        let gates = gates(st, &piece, tier, &mut rng);
        let ways_before = st.out.ways.len();
        let core_loc = Located::new(core.clone());
        let mut carved_any = false;
        // Minimum spanning tree of the gates; edges through the core.
        let mut pairs: Vec<(f32, usize, usize)> = Vec::new();
        for i in 0..gates.len() {
            for j in i + 1..gates.len() {
                pairs.push((gates[i].0.distance(gates[j].0), i, j));
            }
        }
        pairs.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
        let mut uf: Vec<usize> = (0..gates.len()).collect();
        let find = |uf: &mut Vec<usize>, mut i: usize| {
            while uf[i] != i {
                uf[i] = uf[uf[i]];
                i = uf[i];
            }
            i
        };
        let mut chosen = Vec::new();
        for &(_, i, j) in &pairs {
            let (ri, rj) = (find(&mut uf, i), find(&mut uf, j));
            let crosses = crosses_core(&core_loc, gates[i].0, gates[j].0);
            if ri != rj {
                uf[ri] = rj;
                if crosses {
                    chosen.push((i, j));
                }
            } else if crosses && rng.chance(tier.loops) {
                chosen.push((i, j));
            }
        }
        for (i, j) in chosen {
            for line in carve(st, &mut lat, &piece, gates[i], gates[j]) {
                add_way(st, line, [w0, w1], &tier.profile, &mut rng);
                carved_any = true;
            }
        }
        // A core that survives (or a piece with too few gates) gets a dead
        // end or a through path from its pole.
        let core_left = st.land.and(&piece).offset(-tier.d_max, Join::Round);
        if !carved_any || !core_left.is_empty() {
            let Some((p, _)) = core_left.pole().or(core.pole()) else { continue };
            let Some(from) = lat.nearest(p) else { continue };
            // Paths leave the pole for the land's edge where a street runs
            // (not the edge of a plaza or the map): there they can join it.
            let idx = st.line_index();
            let to_edge = |u: u32| lat.edge_dist[u as usize] < o.lattice * 1.2 && st.reaches(&idx, lat.pts[u as usize]);
            let Some(a) = lat.path(from, to_edge) else {
                add_way(st, straight_through(&piece, p), [w0, w1], &tier.profile, &mut rng);
                continue;
            };
            let mut nodes = a;
            if !rng.chance(tier.adarve) {
                // Through path: also from the pole to the far side.
                let first_end = lat.pts[*nodes.last().unwrap() as usize];
                let away = |u: u32| to_edge(u) && lat.pts[u as usize].distance(first_end) > tier.d_max;
                if let Some(mut b) = lat.path(from, away) {
                    b.reverse();
                    b.pop();
                    b.extend(nodes);
                    nodes = b;
                }
            }
            // The end at the land's edge reaches its street; a dead end
            // keeps its square end at the pole.
            let edge_end = |u: u32| lat.edge_dist[u as usize] < o.lattice * 1.2;
            let (first, last) = (nodes[0], *nodes.last().unwrap());
            let (sp, ep) = (edge_end(first).then(|| lat.pts[first as usize]), edge_end(last).then(|| lat.pts[last as usize]));
            for line in runs(st, &mut lat, &piece, &nodes, sp, ep) {
                add_way(st, line, [w0, w1], &tier.profile, &mut rng);
            }
        }
        // Small squares at some of the junctions made in this pass: where a
        // new way ends on an earlier one.
        if tier.plazuelas > 0.0 && rng.chance(tier.plazuelas) {
            let older = st.lines.len().saturating_sub(st.out.ways.len() - ways_before);
            let ends: Vec<Vec2> = st.out.ways[ways_before..].iter().flat_map(|w| [w.line[0], *w.line.last().unwrap()]).collect();
            let at = ends.into_iter().find(|p| st.lines[..older].iter().any(|(l, _)| l.windows(2).any(|s| seg_dist(*p, s[0], s[1]) < 0.1)));
            if let Some(p) = at {
                let (rad, corners) = (rng.range(4.5, 7.0), 4 + rng.below(2) as u32);
                let ring = blob(p, rad, corners, &mut rng);
                st.land = st.land.minus(&Region::ring(&ring));
                st.out.plazas.push(Plaza { ring, zone: st.zi, kind: PlazaKind::Paved });
            }
        }
    }
    true
}

/// Whether the segment `ab` passes through the core.
fn crosses_core(core: &Located, a: Vec2, b: Vec2) -> bool {
    let n = (a.distance(b) / 2.0).ceil().max(1.0) as usize;
    (0..=n).any(|k| core.contains(a.lerp(b, k as f32 / n as f32)))
}

/// Gates of a piece: continuations of street stems ending against it, and
/// points spaced along its edge. The flag marks junction-like gates.
fn gates(st: &State, piece: &Region, tier: &Tier, rng: &mut Rng) -> Vec<(Vec2, bool)> {
    let edge = Located::new(piece.clone());
    let mut out: Vec<(Vec2, bool)> = Vec::new();
    for &(p, dir, w) in &st.stems {
        if let Some(t) = edge.ray(p, dir, w * 0.5 + 3.0) {
            let q = p + dir * (t + 0.01);
            if edge.contains(q + dir * 0.5) {
                out.push((p + dir * t, true));
            }
        }
    }
    let spacing = (tier.d_max * rng.range(2.2, 3.4)).clamp(20.0, 60.0);
    for s in piece.shapes() {
        for ring in s.rings() {
            let mut r = ring.clone();
            r.push(ring[0]);
            let pl = Polyline::new(r);
            let len = pl.len();
            let mut t = rng.range(0.0, spacing);
            while t < len {
                let p = pl.at(t);
                // Away from corners and from other gates.
                let corner = ring.iter().enumerate().any(|(i, v)| {
                    let n = ring.len();
                    let (a, c) = (ring[(i + n - 1) % n], ring[(i + 1) % n]);
                    let turn = (*v - a).normalize_or(Vec2::X).angle_to((c - *v).normalize_or(Vec2::X)).abs();
                    turn > 0.5 && v.distance(p) < 6.0
                });
                if !corner && out.iter().all(|g| g.0.distance(p) > spacing * 0.5) {
                    out.push((p, false));
                }
                t += spacing * rng.range(0.8, 1.2);
            }
        }
    }
    out
}

/// A least-cost path between two gates, as ways: the stretches that run
/// along paths already carved are left out, so new paths branch off and
/// join old ones at T-junctions instead of doubling them.
fn carve(st: &State, lat: &mut Lattice, piece: &Region, a: (Vec2, bool), b: (Vec2, bool)) -> Vec<Vec<Vec2>> {
    let (Some(ia), Some(ib)) = (lat.nearest(a.0), lat.nearest(b.0)) else { return Vec::new() };
    if ia == ib {
        return Vec::new();
    }
    let Some(nodes) = lat.path(ia, |u| u == ib) else { return Vec::new() };
    runs(st, lat, piece, &nodes, Some(a.0), Some(b.0))
}

/// Splits a lattice path into the runs not carved before, each smoothed
/// and attached at both ends (to a gate's street or to an earlier path).
fn runs(st: &State, lat: &mut Lattice, piece: &Region, nodes: &[u32], start: Option<Vec2>, end: Option<Vec2>) -> Vec<Vec<Vec2>> {
    let old: Vec<bool> = nodes.iter().map(|&u| lat.carved[u as usize]).collect();
    for u in nodes {
        lat.carved[*u as usize] = true;
    }
    let mut out = Vec::new();
    let mut k = 0;
    while k < nodes.len() {
        if old[k] {
            k += 1;
            continue;
        }
        let s0 = k;
        while k < nodes.len() && !old[k] {
            k += 1;
        }
        // Include the carved node on each side as the junction.
        let (from, to) = (s0.saturating_sub(1), k.min(nodes.len() - 1));
        let mut line: Vec<Vec2> = nodes[from..=to].iter().map(|&u| lat.pts[u as usize]).collect();
        if s0 == 0 {
            if let Some(p) = start {
                line.insert(0, p);
            }
        }
        if k == nodes.len() {
            if let Some(p) = end {
                line.push(p);
            }
        }
        let mut line = smooth(line);
        if line.len() >= 2 {
            attach(st, piece, &mut line, false);
            attach(st, piece, &mut line, true);
            out.push(line);
        }
    }
    out
}

/// Chaikin twice, RDP at 0.3 m, chords of at most 3 m.
fn smooth(line: Vec<Vec2>) -> Vec<Vec2> {
    Polyline::new(line).chaikin(2).simplify(0.3).resample(3.0).pts().to_vec()
}

/// Extends an end of a path to the nearest centreline (across the street
/// at a gate, or onto an earlier path), so the planar graph forms a
/// T-junction there. Ends deep inside the land (dead ends) stay.
fn attach(st: &State, piece: &Region, line: &mut Vec<Vec2>, last: bool) {
    let p = if last { *line.last().unwrap() } else { line[0] };
    let _ = piece;
    let mut best: Option<(f32, Vec2)> = None;
    for (l, w) in &st.lines {
        for k in 0..l.len().saturating_sub(1) {
            let (a, b) = (l[k], l[k + 1]);
            let d = seg_dist(p, a, b);
            if d < st.reach_of(*w) && d > 0.01 && best.is_none_or(|x| d < x.0) {
                let e = b - a;
                let t = ((p - a).dot(e) / e.length_squared().max(1e-9)).clamp(0.0, 1.0);
                best = Some((d, a + e * t));
            }
        }
    }
    if let Some((_, q)) = best {
        if last {
            line.push(q);
        } else {
            line.insert(0, q);
        }
    }
}

/// A straight cut through `p` across the piece, along its short axis.
fn straight_through(piece: &Region, p: Vec2) -> Vec<Vec2> {
    let mut best = (f32::INFINITY, Vec2::X);
    for k in 0..12 {
        let d = Vec2::from_angle(k as f32 * std::f32::consts::PI / 12.0);
        let (a, b) = (piece.ray(p, d, 500.0).unwrap_or(500.0), piece.ray(p, -d, 500.0).unwrap_or(500.0));
        if a + b < best.0 {
            best = (a + b, d);
        }
    }
    let d = best.1;
    let (a, b) = (piece.ray(p, d, 500.0).unwrap_or(10.0), piece.ray(p, -d, 500.0).unwrap_or(10.0));
    vec![p - d * (b + 2.0), p + d * (a + 2.0)]
}

/// Adds a carved way with breathing widths, and removes its corridor from
/// the land.
fn add_way(st: &mut State, line: Vec<Vec2>, range: [f32; 2], profile: &str, rng: &mut Rng) {
    if line.len() < 2 {
        return;
    }
    let pl = Polyline::new(line);
    if pl.len() < 2.0 {
        return;
    }
    let key = rng.next_u32() as u64;
    let (mid, half) = ((range[0] + range[1]) * 0.5, (range[1] - range[0]) * 0.5);
    let len = pl.len();
    let width: Vec<f32> = pl
        .stations()
        .iter()
        .map(|&s| {
            let w = mid + half * noise1(st.seed ^ key, s / 12.0);
            let flare = 1.0 + 0.15 * (1.0 - (s.min(len - s) / 3.0).min(1.0));
            (w * flare).clamp(range[0], range[1] * 1.15)
        })
        .collect();
    // Only a way that joins the network may consume land: a floating one
    // would be dropped by the planar graph and leave its land unserved.
    let joins = |p: Vec2| st.lines.iter().any(|(l, _)| l.windows(2).any(|s| seg_dist(p, s[0], s[1]) < 0.05));
    if !joins(pl.pts()[0]) && !joins(*pl.pts().last().unwrap()) {
        log::debug!("organic zone {}: dropped a path that joins no street", st.zi);
        return;
    }
    let corridor = crate::plan::line::stroke(pl.pts(), |k| width[k] * 0.5);
    st.land = st.land.minus(&corridor);
    st.lines.push((pl.pts().to_vec(), width.iter().copied().fold(0.0, f32::max)));
    let mut w = Way::new(pl.pts().to_vec(), profile).zone(st.zi);
    w.width = width;
    st.out.ways.push(w);
}

