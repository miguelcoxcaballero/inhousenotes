//! Facade runs and bay plans: how a street frontage of any shape is cut
//! into bays.
//!
//! A **run** is a chain of consecutive street-facing edges that turn by at
//! most `CREASE_DEG` at their inner vertices: a straight front, a curved
//! front given as chords, or a rounded chaflán together with its two sides.
//! A sharp chamfer or an L corner ends a run. Each run gets one bay rhythm
//! (the bay target is drawn once per lot) with end margins that depend on
//! the corner angle at each end (`end_clearance`).
//!
//! A run made of several edges is then **fitted**: its vertices are
//! replaced by its piers, so every bay is one planar chord. Openings,
//! reveals, balconies and miradores never straddle a bend, and the rest of
//! the building code keeps working with plain straight edges. The run's
//! end vertices never move, so neighbours keep sharing party vertices.

use std::f32::consts::{FRAC_PI_2, PI, TAU};

use forja_math::Vec2;

use crate::geom::Rng;
use crate::lot::{EdgeKind, Front, Lot};
use crate::modules::REVEAL;
use crate::style::Style;

/// Largest turn (degrees) inside a run.
pub const CREASE_DEG: f32 = 25.0;
/// Smallest blank wall between a run end and its first opening (m).
pub const MIN_MARGIN: f32 = 0.55;
/// Fronts shorter than this are blank walls (m).
pub const MIN_FACADE: f32 = 2.0;
/// Narrowest bay worth an opening (m).
const MIN_BAY: f32 = 0.9;
/// Furthest a fitted chord may stray into the lot from the frontage it
/// replaces (m), and out over the street (concave fronts); a run that would
/// stray more keeps its vertices and is planned per edge.
pub const FIT_TOLERANCE: f32 = 0.12;
pub const FIT_OUTWARD: f32 = 0.05;

/// Consecutive street edges of a lot that turn by at most a crease.
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    /// Edge indices in ring order.
    pub edges: Vec<usize>,
    pub len: f32,
    /// Interior angle of the lot (radians) at the run's first and last vertex.
    pub interior: [f32; 2],
}

/// How far the lot outline turns at vertex `v` (radians, 0 = straight).
pub fn turn(lot: &Lot, v: usize) -> f32 {
    (PI - lot.interior_angle(v)).abs()
}

/// The facade runs of `lot`, split at vertices turning more than
/// `crease_deg` and at every non-street edge.
pub fn runs(lot: &Lot, crease_deg: f32) -> Vec<Run> {
    let n = lot.ring.len();
    let crease = crease_deg.to_radians();
    let joins = |v: usize| lot.edges[(v + n - 1) % n].is_street() && lot.edges[v].is_street() && turn(lot, v) <= crease;
    // Start at a vertex no run passes through; a lot that is all smooth
    // street front starts at its sharpest vertex.
    let start = (0..n).find(|&v| !joins(v)).unwrap_or_else(|| (0..n).fold(0, |b, v| if turn(lot, v) > turn(lot, b) { v } else { b }));
    let mut out = Vec::new();
    let mut cur: Vec<usize> = Vec::new();
    let close = |cur: &mut Vec<usize>, out: &mut Vec<Run>| {
        if cur.is_empty() {
            return;
        }
        let edges = std::mem::take(cur);
        let len = edges.iter().map(|&i| edge_len(lot, i)).sum();
        let interior = [lot.interior_angle(edges[0]), lot.interior_angle((edges[edges.len() - 1] + 1) % n)];
        out.push(Run { edges, len, interior });
    };
    for k in 0..n {
        let i = (start + k) % n;
        if k > 0 && !joins(i) {
            close(&mut cur, &mut out);
        }
        if lot.edges[i].is_street() {
            cur.push(i);
        }
    }
    close(&mut cur, &mut out);
    out
}

fn edge_len(lot: &Lot, i: usize) -> f32 {
    let (a, b) = lot.edge(i);
    a.distance(b)
}

/// Blank wall needed at a run end whose corner has interior angle
/// `interior` (radians), for features projecting `projection` metres:
/// at sharp corners neither the reveals (inside) nor balconies and
/// cornices (outside) may cross the other face, and in re-entrant corners
/// the balconies and cornices of the two faces must not collide.
pub fn end_clearance(interior: f32, projection: f32) -> f32 {
    let mut m = MIN_MARGIN;
    if interior < FRAC_PI_2 {
        // The party line leans over the front: keep reveals inside and
        // projections outside clear of it.
        m = m.max(REVEAL.max(projection) / interior.tan());
    }
    if interior > PI {
        m = m.max(projection / ((TAU - interior) * 0.5).tan() + 0.4);
    }
    m
}

/// Depth of a style's main cornice (corona) at full size (m).
pub fn cornice_depth(style: &Style) -> f32 {
    match style.cornice.as_str() {
        "eave" => 0.62,
        "modillions" => 0.68,
        "slab" => 0.45,
        _ => 0.51,
    }
}

/// Deepest thing a style hangs on a facade facing `front` (m).
pub fn projection(style: &Style, front: &Front) -> f32 {
    let balcony: f32 = match style.balcony.as_str() {
        "none" => 0.0,
        "terrace" => 1.1,
        "corrido" => 0.7,
        _ => 0.6,
    };
    let mirador = if style.miradores > 0.0 && front.miradores { 0.92 } else { 0.0 };
    balcony.max(mirador).max(cornice_depth(style)).min(front.reach)
}

/// Bays of one edge in its facade frame (`u` = 0 at `ring[i + 1]`): `n`
/// bays of width `w` between margins `m0` and `m1`. `run` is the index of
/// the edge's run in `FacadePlan::runs` and `k0` the run-wide index of the
/// edge's bay 0, counting bays in the direction of `u`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EdgeBays {
    pub m0: f32,
    pub m1: f32,
    pub n: usize,
    pub w: f32,
    pub run: usize,
    pub k0: usize,
}

impl EdgeBays {
    /// A wall with no bays (off the street, or too short).
    pub const BLANK: EdgeBays = EdgeBays { m0: 0.0, m1: 0.0, n: 0, w: 0.0, run: usize::MAX, k0: 0 };

    /// `u` of the axis of bay `k`.
    pub fn center(&self, k: usize) -> f32 {
        self.m0 + (k as f32 + 0.5) * self.w
    }
}

/// The wall strip between two bays (or at the end of a run), where
/// pilasters, tile panels and lanterns go.
#[derive(Clone, Debug, PartialEq)]
pub struct Pier {
    pub pos: Vec2,
    /// Along the facade (the frame's `u` direction) and out of it; mitred
    /// where the pier sits on a fitted vertex.
    pub t: Vec2,
    pub n: Vec2,
    /// `(edge, u)` on every edge the pier touches (two on a fitted vertex).
    pub at: Vec<(usize, f32)>,
    /// First or last pier of its run.
    pub outer: bool,
}

/// One run of the fitted lot.
#[derive(Clone, Debug, PartialEq)]
pub struct RunPlan {
    /// Edges of the fitted lot, in ring order.
    pub edges: Vec<usize>,
    pub len: f32,
    pub bays: usize,
    pub w: f32,
    /// Piers in the direction of `u` (pier `K` is left of bay `K`).
    pub piers: Vec<Pier>,
    /// Every edge of the run is a `Corner` edge (a chamfer).
    pub corner: bool,
}

pub struct FacadePlan {
    /// The lot with every fitted run replaced by its pier chords.
    pub lot: Lot,
    /// One per edge of `lot`.
    pub bays: Vec<EdgeBays>,
    pub runs: Vec<RunPlan>,
    /// `(edge, bay)` of the street door.
    pub portal: Option<(usize, usize)>,
}

/// Bay layout of one run: margins at its first and last vertex, count and width.
fn lay(lot: &Lot, style: &Style, run: &Run, target: f32) -> (f32, f32, usize, f32) {
    let (first, last) = (run.edges[0], run.edges[run.edges.len() - 1]);
    let m0 = end_clearance(run.interior[0], projection(style, &lot.front(first)));
    let m1 = end_clearance(run.interior[1], projection(style, &lot.front(last)));
    let free = run.len - m0 - m1;
    if run.len < MIN_FACADE || free < MIN_BAY {
        return (m0, m1, 0, 0.0);
    }
    let n = (free / target).round().max(1.0) as usize;
    (m0, m1, n, free / n as f32)
}

/// Point at arc length `s` along the run's polyline, and the run edge it lies on.
fn at_arc(lot: &Lot, run: &Run, s: f32) -> (Vec2, usize) {
    let mut acc = 0.0;
    for &i in &run.edges {
        let (a, b) = lot.edge(i);
        let l = a.distance(b);
        if s <= acc + l || i == run.edges[run.edges.len() - 1] {
            let t = ((s - acc) / l.max(1e-6)).clamp(0.0, 1.0);
            return (a.lerp(b, t), i);
        }
        acc += l;
    }
    unreachable!("runs have edges")
}

fn seg_dist(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let d = b - a;
    let t = ((p - a).dot(d) / d.length_squared().max(1e-12)).clamp(0.0, 1.0);
    p.distance(a + d * t)
}

/// Arc lengths of the fitted vertices of a run (start, piers, end).
fn fit_stations(run: &Run, m0: f32, n: usize, w: f32) -> Vec<f32> {
    let mut s = vec![0.0];
    s.extend((0..=n).map(|j| m0 + j as f32 * w));
    s.push(run.len);
    s
}

/// Whether the fitted chords stay close enough to the run's vertices:
/// within `FIT_TOLERANCE` inside the lot and `FIT_OUTWARD` over the street.
fn fits(lot: &Lot, run: &Run, stations: &[f32]) -> bool {
    let mut s = 0.0;
    for &i in &run.edges[..run.edges.len() - 1] {
        s += edge_len(lot, i);
        let v = lot.edge(i).1;
        let k = stations.windows(2).position(|w| s <= w[1]).unwrap_or(stations.len() - 2);
        let (a, b) = (at_arc(lot, run, stations[k]).0, at_arc(lot, run, stations[k + 1]).0);
        // The lot is on the left of a chord: a vertex there means the chord
        // passes outside it, over the street.
        let outward = (b - a).perp_dot(v - a) > 0.0;
        if seg_dist(v, a, b) > if outward { FIT_OUTWARD } else { FIT_TOLERANCE } {
            return false;
        }
    }
    true
}

/// The front of the original edge `src` re-measured for a chord `a -> b`:
/// the limits carry over and `along` follows the chord's ends.
fn chord_front(lot: &Lot, src: usize, a: Vec2, b: Vec2) -> Front {
    let f = lot.front(src);
    let (p, q) = lot.edge(src);
    let d = q - p;
    let t = |x: Vec2| (x - p).dot(d) / d.length_squared().max(1e-12);
    Front { along: [f.along_at(t(a)).max(0.0), f.along_at(t(b)).max(0.0)], ..f }
}

/// One planned span of the facade: a run with its layout, fitted or not.
struct Span {
    run: Run,
    m0: f32,
    m1: f32,
    n: usize,
    w: f32,
    fit: bool,
}

/// Plans the facade of `lot`: runs, bays, the fitted lot and the portal bay.
pub fn plan(lot: &Lot, style: &Style, rng: &mut Rng) -> FacadePlan {
    let target = rng.range(style.bay[0], style.bay[1]).max(MIN_BAY);
    let mut spans = Vec::new();
    for run in runs(lot, CREASE_DEG) {
        let (m0, m1, n, w) = lay(lot, style, &run, target);
        let fit = run.edges.len() > 1 && n > 0 && fits(lot, &run, &fit_stations(&run, m0, n, w));
        if fit || run.edges.len() == 1 || n == 0 {
            spans.push(Span { run, m0, m1, n, w, fit });
            continue;
        }
        // Too bent to fit: every edge is its own little facade.
        for &i in &run.edges {
            let n_ = lot.ring.len();
            let r = Run { edges: vec![i], len: edge_len(lot, i), interior: [lot.interior_angle(i), lot.interior_angle((i + 1) % n_)] };
            let (m0, m1, n, w) = lay(lot, style, &r, target);
            spans.push(Span { run: r, m0, m1, n, w, fit: false });
        }
    }
    let fitted = build(lot, &spans);
    if spans.iter().any(|s| s.fit) && fitted.lot.check().is_err() && lot.check().is_ok() {
        // A fit that breaks the lot (only on pathological inputs): plan
        // every edge on its own instead.
        let spans: Vec<Span> = spans
            .into_iter()
            .flat_map(|s| {
                if !s.fit {
                    return vec![s];
                }
                let n_ = lot.ring.len();
                s.run
                    .edges
                    .iter()
                    .map(|&i| {
                        let r = Run { edges: vec![i], len: edge_len(lot, i), interior: [lot.interior_angle(i), lot.interior_angle((i + 1) % n_)] };
                        let (m0, m1, n, w) = lay(lot, style, &r, target);
                        Span { run: r, m0, m1, n, w, fit: false }
                    })
                    .collect()
            })
            .collect();
        return build(lot, &spans);
    }
    fitted
}

/// The fitted lot and per-edge bays from the planned spans.
fn build(lot: &Lot, spans: &[Span]) -> FacadePlan {
    let n = lot.ring.len();
    // Which span (if fitted) owns each original edge, and whether it starts there.
    let mut owner: Vec<Option<usize>> = vec![None; n];
    for (si, s) in spans.iter().enumerate() {
        for &i in &s.run.edges {
            owner[i] = Some(si);
        }
    }
    let mut ring = Vec::new();
    let mut edges = Vec::new();
    let mut fronts = Vec::new();
    // Per new edge: (span, bay index along the run in ring order, or None for a margin).
    let mut tags: Vec<(Option<usize>, Option<usize>)> = Vec::new();
    for i in 0..n {
        match owner[i] {
            Some(si) if spans[si].fit => {
                let s = &spans[si];
                if s.run.edges[0] != i {
                    continue;
                }
                let st = fit_stations(&s.run, s.m0, s.n, s.w);
                let pts: Vec<(Vec2, usize)> = st.iter().map(|&x| at_arc(lot, &s.run, x)).collect();
                for k in 0..st.len() - 1 {
                    let (a, b) = (pts[k].0, pts[k + 1].0);
                    let mid = at_arc(lot, &s.run, (st[k] + st[k + 1]) * 0.5).1;
                    ring.push(a);
                    edges.push(lot.edges[mid]);
                    fronts.push(chord_front(lot, mid, a, b));
                    let bay = (k >= 1 && k <= s.n).then(|| k - 1);
                    tags.push((Some(si), bay));
                }
            }
            o => {
                ring.push(lot.ring[i]);
                edges.push(lot.edges[i]);
                fronts.push(lot.front(i));
                tags.push((o, None));
            }
        }
    }
    let keep_fronts = !lot.fronts.is_empty();
    let fitted = Lot { ring, edges, base: lot.base, fronts: if keep_fronts { fronts } else { Vec::new() }, levels: lot.levels, roof_levels: lot.roof_levels };
    let m = fitted.ring.len();
    let mut bays = vec![EdgeBays::BLANK; m];
    let mut runs: Vec<RunPlan> = Vec::new();
    let mut span_run = vec![usize::MAX; spans.len()];
    // Runs in the order their first edge appears in the fitted ring.
    for e in 0..m {
        let Some(si) = tags[e].0 else { continue };
        if span_run[si] == usize::MAX {
            span_run[si] = runs.len();
            let s = &spans[si];
            runs.push(RunPlan { edges: Vec::new(), len: s.run.len, bays: s.n, w: s.w, piers: Vec::new(), corner: true });
        }
        let r = span_run[si];
        runs[r].edges.push(e);
        runs[r].corner &= fitted.edges[e] == EdgeKind::Corner;
    }
    // Runs whose first edge sits at the end of the ring wrap around: their
    // edges were pushed out of ring order.
    for r in &mut runs {
        let k = (1..r.edges.len()).find(|&k| r.edges[k] != r.edges[k - 1] + 1);
        if let Some(k) = k {
            r.edges.rotate_left(k);
        }
    }
    for (si, s) in spans.iter().enumerate() {
        let r = span_run[si];
        if r == usize::MAX {
            continue;
        }
        let run = &mut runs[r];
        let total = s.n;
        if s.fit {
            for &e in &run.edges {
                if let Some(j) = tags[e].1 {
                    let (a, b) = fitted.edge(e);
                    bays[e] = EdgeBays { m0: 0.0, m1: 0.0, n: 1, w: a.distance(b), run: r, k0: total - 1 - j };
                } else {
                    bays[e] = EdgeBays { run: r, ..EdgeBays::BLANK };
                }
            }
            // Piers at the fitted vertices: the end of each bay chord.
            let bay_edges: Vec<usize> = run.edges.iter().copied().filter(|&e| tags[e].1.is_some()).collect();
            let mut piers = Vec::new();
            for (j, &e) in bay_edges.iter().enumerate() {
                let v = e; // vertex at the chord's start (ring order)
                let prev = (e + m - 1) % m;
                piers.push(vertex_pier(&fitted, v, prev, e, j == 0));
                if j + 1 == bay_edges.len() {
                    let next = (e + 1) % m;
                    piers.push(vertex_pier(&fitted, next, e, next, true));
                }
            }
            piers.reverse(); // u order runs against the ring
            run.piers = piers;
        } else {
            for &e in &run.edges {
                bays[e] = EdgeBays { run: r, ..EdgeBays::BLANK };
            }
            // Unfitted spans are single edges (or blank fronts).
            let e = run.edges[0];
            if s.n > 0 {
                let (a, b) = fitted.edge(e);
                bays[e] = EdgeBays { m0: s.m1, m1: s.m0, n: s.n, w: s.w, run: r, k0: 0 };
                let f = frame2(a, b);
                run.piers = (0..=s.n)
                    .map(|k| {
                        let u = s.m1 + k as f32 * s.w;
                        Pier { pos: b + f.0 * u, t: f.0, n: f.1, at: vec![(e, u)], outer: k == 0 || k == s.n }
                    })
                    .collect();
            }
        }
    }
    // The portal: middle bay of the longest run with a street (not
    // corner) edge, the longest run otherwise.
    let pick = |want_street: bool| {
        (0..runs.len())
            .filter(|&r| runs[r].bays > 0 && (!want_street || runs[r].edges.iter().any(|&e| fitted.edges[e] == EdgeKind::Street)))
            .fold(None::<usize>, |best, r| match best {
                Some(b) if runs[b].len >= runs[r].len => Some(b),
                _ => Some(r),
            })
    };
    let portal = pick(true).or_else(|| pick(false)).and_then(|r| {
        let k = runs[r].bays / 2;
        runs[r].edges.iter().find_map(|&e| {
            let b = bays[e];
            (b.n > 0 && k >= b.k0 && k < b.k0 + b.n).then(|| (e, k - b.k0))
        })
    });
    FacadePlan { lot: fitted, bays, runs, portal }
}

/// Along-facade and outward directions of edge `a -> b` (2D, in x/z).
fn frame2(a: Vec2, b: Vec2) -> (Vec2, Vec2) {
    let d = (b - a).normalize_or(Vec2::X);
    let n = Vec2::new(d.y, -d.x);
    (-d, n)
}

/// The pier on vertex `v` of the fitted lot, between edges `before`
/// (ending at `v`) and `after` (starting at `v`).
fn vertex_pier(lot: &Lot, v: usize, before: usize, after: usize, outer: bool) -> Pier {
    let p = lot.ring[v];
    let (a0, b0) = lot.edge(before);
    let (a1, b1) = lot.edge(after);
    let n = (frame2(a0, b0).1 + frame2(a1, b1).1).normalize_or(frame2(a1, b1).1);
    let t = Vec2::new(n.y, -n.x);
    let mut at = Vec::new();
    if lot.edges[before].is_street() {
        at.push((before, 0.0));
    }
    if lot.edges[after].is_street() {
        at.push((after, a1.distance(b1)));
    }
    Pier { pos: p, t, n, at, outer }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::StyleLibrary;
    use EdgeKind::*;

    fn deg(d: f32) -> f32 {
        d.to_radians()
    }

    #[test]
    fn end_clearance_by_angle() {
        // Sharp corners keep the reveals off the other face.
        assert!((end_clearance(deg(20.0), 0.2) - REVEAL / deg(20.0).tan()).abs() < 1e-5);
        assert!((end_clearance(deg(20.0), 0.5) - 0.5 / deg(20.0).tan()).abs() < 1e-5);
        assert_eq!(end_clearance(deg(45.0), 0.5), MIN_MARGIN);
        assert_eq!(end_clearance(deg(90.0), 0.5), MIN_MARGIN);
        assert_eq!(end_clearance(deg(135.0), 0.5), MIN_MARGIN);
        // Inside corners keep balconies of the two faces apart.
        assert!((end_clearance(deg(270.0), 0.5) - 0.9).abs() < 1e-4);
        assert!(end_clearance(deg(270.0), 1.0) > end_clearance(deg(270.0), 0.5));
    }

    fn lot(pts: &[(f32, f32)], e: &[EdgeKind]) -> Lot {
        Lot::new(pts.iter().map(|p| Vec2::new(p.0, p.1)).collect(), e.to_vec(), 0.0)
    }

    /// Lot inside a street curving round it at radius `r` (a convex
    /// front): frontage chords of about `chord` metres over `arc` radians,
    /// `depth` deep, party walls on the radii.
    pub fn curved(r: f32, arc: f32, chord: f32, depth: f32) -> Lot {
        let k = (r * arc / chord).ceil() as usize;
        let at = |a: f32, rad: f32| Vec2::new(a.sin(), -a.cos()) * rad;
        let a0 = -arc * 0.5;
        let mut ring: Vec<Vec2> = (0..=k).map(|i| at(a0 + arc * i as f32 / k as f32, r)).collect();
        let mut edges = vec![Street; k];
        ring.extend([at(-a0, r - depth), at(a0, r - depth)]);
        edges.extend([Party, Courtyard, Party]);
        Lot::new(ring, edges, 0.0)
    }

    #[test]
    fn rounded_chaflan_stays_in_one_run_and_sharp_chamfers_split() {
        // A 90° corner cut by a 45° chamfer: three runs.
        let sharp = lot(&[(0.0, 0.0), (20.0, 0.0), (20.0, 16.0), (16.0, 20.0), (0.0, 20.0)], &[Party, Street, Corner, Street, Party]);
        sharp.check().unwrap();
        assert_eq!(runs(&sharp, CREASE_DEG).len(), 3);
        // A rounded corner of six 15° chords: one run.
        let mut ring = vec![Vec2::new(0.0, 0.0), Vec2::new(20.0, 0.0), Vec2::new(20.0, 11.0)];
        let mut e = vec![Party, Street];
        let c = Vec2::new(11.0, 11.0);
        for k in 0..6 {
            let a = (k as f32 + 1.0) * deg(15.0);
            ring.push(c + Vec2::new(a.cos(), a.sin()) * 9.0);
            e.push(Corner);
        }
        e.push(Street);
        ring.push(Vec2::new(0.0, 20.0));
        e.push(Party);
        let round = Lot::new(ring, e, 0.0);
        round.check().unwrap();
        let rs = runs(&round, CREASE_DEG);
        assert_eq!(rs.len(), 1, "{rs:?}");
        assert_eq!(rs[0].edges.len(), 8);
    }

    #[test]
    fn fit_keeps_run_ends_and_puts_a_vertex_on_every_pier() {
        let styles = StyleLibrary::builtin();
        let l = curved(25.0, 0.5, 2.5, 12.0);
        l.check().unwrap();
        let p = plan(&l, styles.get("madrid_centro").unwrap(), &mut Rng::new(3));
        p.lot.check().unwrap();
        assert_eq!(p.runs.len(), 1);
        let r = &p.runs[0];
        assert!(r.bays >= 3);
        // Party vertices unchanged.
        for v in &l.ring {
            let on_street = |q: &Vec2| p.lot.ring.iter().any(|x| x.distance(*q) < 1e-5);
            let idx = l.ring.iter().position(|x| x == v).unwrap();
            let n = l.ring.len();
            let party = !l.edges[idx].is_street() || !l.edges[(idx + n - 1) % n].is_street();
            if party {
                assert!(on_street(v), "{v:?} moved");
            }
        }
        // Every bay is one chord, piers are its vertices.
        assert_eq!(r.piers.len(), r.bays + 1);
        for e in &r.edges {
            let b = p.bays[*e];
            assert!(b.n <= 1);
        }
        for pier in &r.piers {
            assert!(p.lot.ring.iter().any(|x| x.distance(pier.pos) < 1e-5));
        }
        // Bays cover the run in u order without gaps.
        let mut ks: Vec<usize> = r.edges.iter().filter(|e| p.bays[**e].n == 1).map(|e| p.bays[*e].k0).collect();
        ks.sort();
        assert_eq!(ks, (0..r.bays).collect::<Vec<_>>());
        assert!(p.portal.is_some());
    }
}
