//! `parcel`: cuts one block into lots by the nearest-front partition.
//!
//! The frontage (the block edges facing public space) is cut into lot
//! widths, run by run between corners. Corner lots reach far enough along
//! both sides to wrap their corner at any angle. Every frontage piece
//! (chord between ring vertices) then becomes a site of a power diagram:
//! its midpoint pushed inward by `δ`, weighted by its half length squared.
//! Every frontage vertex then has the same power `δ²` to the two pieces it
//! joins, so each party wall starts exactly at its cut point, on straight
//! runs, on curves and at corners alike. The cell edges between different
//! lots, plus the contour of a patio de manzana, slice the block once into
//! an exact, crack-free partition on the millimetre grid. A single repair
//! rule then merges any piece that is not a good lot into the neighbour it
//! shares most boundary with.

use forja_math::Vec2;

use crate::generators::LotRules;
use crate::geom::Rng;
use crate::plan::line::left;
use crate::plan::{Join, Region, SegIndex, Shape};

/// What a block edge is, for the parcel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tag {
    /// Faces public space: frontage.
    Front,
    /// The chamfer or fillet chord of corner `c` (faces public space).
    Chord(u16),
    /// Faces nothing public (map edge, a footprint).
    Blind,
}

/// A block outline ready to be parcelled.
#[derive(Clone, Debug)]
pub struct BlockRing {
    /// Outer ring, counter-clockwise.
    pub pts: Vec<Vec2>,
    /// Per edge `pts[i] -> pts[i + 1]`.
    pub tags: Vec<Tag>,
    /// Unbevelled corner at a vertex: its corner id.
    pub apex: Vec<Option<u16>>,
    /// Interior angle (radians) of each corner id before bevelling.
    pub theta: Vec<f32>,
    /// Ground level at each vertex (m); empty on flat ground.
    pub z: Vec<f32>,
}

/// The lots of one block (simple rings, CCW) and what is left as courts.
#[derive(Clone, Debug, Default)]
pub struct Parcels {
    pub lots: Vec<Region>,
    pub courts: Region,
}

#[derive(Clone, Copy, Debug)]
struct Site {
    s: Vec2,
    w: f64,
    owner: u32,
}

fn power(x: Vec2, s: &Site) -> f64 {
    let d = x - s.s;
    (d.x as f64).powi(2) + (d.y as f64).powi(2) - s.w
}

/// Turn (radians) at vertex `i` of a closed ring.
fn turn(p: &[Vec2], i: usize) -> f32 {
    let n = p.len();
    let (a, b, c) = (p[(i + n - 1) % n], p[i], p[(i + 1) % n]);
    let (d0, d1) = ((b - a).normalize_or(Vec2::X), (c - b).normalize_or(Vec2::X));
    d0.perp_dot(d1).atan2(d0.dot(d1)).abs()
}

/// Cuts a block into lots. `inradius` is the block's inscribed radius,
/// `court` an optional patio region, `rng` keyed by the block.
pub fn parcel(block: &Shape, ring: &BlockRing, rules: &LotRules, inradius: f32, rng: &mut Rng) -> Parcels {
    let region = Region::from_shapes(std::slice::from_ref(block));
    if region.is_empty() {
        return Parcels::default();
    }
    let (pts, tags, cut_at) = cut_frontage(ring, rules, rng);
    let n = pts.len();
    // Owners: a new lot starts at every cut vertex and after every blind
    // stretch; corner chords stay with the lot wrapping them.
    let start = (0..n).find(|&i| cut_at[i] || (tags[(i + n - 1) % n] == Tag::Blind && tags[i] != Tag::Blind)).unwrap_or(0);
    let mut owner = vec![u32::MAX; n];
    let mut cur = 0u32;
    for k in 0..n {
        let i = (start + k) % n;
        if k > 0 && (cut_at[i] || (tags[(i + n - 1) % n] == Tag::Blind && tags[i] != Tag::Blind)) {
            cur += 1;
        }
        if tags[i] != Tag::Blind {
            owner[i] = cur;
        }
    }
    let lots_n = if owner.iter().any(|&o| o != u32::MAX) { cur + 1 } else { 0 };
    if lots_n == 0 {
        return Parcels { lots: Vec::new(), courts: region };
    }
    // Sites.
    let delta = (rules.depth * 0.5).min(inradius * 0.5).max(0.5);
    let mut sites: Vec<Site> = Vec::new();
    for i in 0..n {
        if owner[i] == u32::MAX {
            continue;
        }
        let (a, b) = (pts[i], pts[(i + 1) % n]);
        let len = a.distance(b);
        if len < 1e-4 {
            continue;
        }
        let m = (a + b) * 0.5;
        sites.push(Site { s: m + left((b - a) / len) * delta, w: (len as f64 * 0.5).powi(2), owner: owner[i] });
    }
    // Patio de manzana: the block eroded by the lot depth, when it is big
    // enough to be a real court.
    let court = {
        let c = region.offset(-rules.depth, Join::Miter(20f32.to_radians())).opening(1.0);
        let keep: Vec<Region> = c.components().into_iter().filter(|r| r.area() >= rules.court_min as f64 && r.pole().is_some_and(|p| p.1 >= 3.0)).collect();
        Region::union_all(keep.iter())
    };
    // Cut network: power-cell edges between different owners.
    let mut cuts = power_cuts(&sites, &region, rules, rng);
    for s in court.shapes() {
        for r in s.rings() {
            let mut l = r.clone();
            l.push(r[0]);
            cuts.push(l);
        }
    }
    let pieces = region.slice(&cuts);
    if std::env::var("FORJA_PARCEL_CUTS").is_ok() {
        eprintln!("court {:?}", court.shapes());
        eprintln!("cuts {}", cuts.len());
        for c in &cuts {
            eprintln!("  cut {:?}", c);
        }
        for p in &pieces {
            eprintln!("  piece {:?}", p.outer);
        }
    }
    // Group pieces: courts, then by owner (minimum power at an interior point).
    let mut groups: Vec<Vec<Shape>> = vec![Vec::new(); lots_n as usize];
    let mut courts: Vec<Shape> = Vec::new();
    for p in pieces {
        let Some(x) = interior_point(&p) else { continue };
        if court.contains(x) {
            courts.push(p);
            continue;
        }
        let best = sites.iter().min_by(|a, b| power(x, a).total_cmp(&power(x, b))).map(|s| s.owner).unwrap_or(0);
        groups[best as usize].push(p);
    }
    // Lots and orphans.
    let front_ix = {
        let mut ix = SegIndex::new(8.0);
        for i in 0..n {
            if tags[i] != Tag::Blind {
                ix.push(pts[i], pts[(i + 1) % n], owner[i]);
            }
        }
        ix
    };
    let mut units: Vec<Region> = Vec::new();
    for g in groups {
        if g.is_empty() {
            continue;
        }
        let r = Region::from_shapes(&g);
        let mut comps = r.components();
        comps.sort_by(|a, b| frontage(b, &front_ix).total_cmp(&frontage(a, &front_ix)).then(b.area().total_cmp(&a.area())));
        units.extend(comps);
    }
    let mut court_r = Region::from_shapes(&courts);
    // Small courts join the lots around them.
    for c in court_r.components() {
        if c.area() < 25.0 {
            units.push(c);
        }
    }
    court_r = Region::union_all(court_r.components().iter().filter(|c| c.area() >= 25.0));
    let nunits = units.len();
    if std::env::var("FORJA_PARCEL_DEBUG").is_ok() {
        for u in &units {
            let a = u.area() as f32;
            let per: f32 = u.edges().map(|(p, q)| ((q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2)).sqrt() as f32).sum();
            eprintln!("  unit: shapes {} holes {} area {a:.1} front {:.2} thick {:.2}", u.len(), u.shapes().first().map(|s| s.holes.len()).unwrap_or(0), frontage(u, &front_ix), 2.0 * a / per);
        }
    }
    let lots = repair(units, rules, &front_ix);
    if std::env::var("FORJA_PARCEL_DEBUG").is_ok() && lots.is_empty() {
        eprintln!("parcel: {} sites, {} owners, {} cuts, {} units -> 0 lots", sites.len(), lots_n, cuts.len(), nunits);
    }
    Parcels { lots, courts: court_r }
}

/// Inserts the frontage cuts as ring vertices: returns the new ring, its
/// tags and which vertices are cuts.
fn cut_frontage(ring: &BlockRing, rules: &LotRules, rng: &mut Rng) -> (Vec<Vec2>, Vec<Tag>, Vec<bool>) {
    let n = ring.pts.len();
    let p = &ring.pts;
    // A vertex breaks a run if it is an unbevelled corner or a tag change.
    let breaks = |i: usize| ring.apex[i].is_some() || ring.tags[(i + n - 1) % n] != Tag::Front || ring.tags[i] != Tag::Front;
    let corner_theta = |i: usize, before: bool| -> Option<f32> {
        if let Some(c) = ring.apex[i] {
            return ring.theta.get(c as usize).copied();
        }
        let t = if before { ring.tags[(i + n - 1) % n] } else { ring.tags[i] };
        match t {
            Tag::Chord(c) => ring.theta.get(c as usize).copied(),
            _ => None,
        }
    };
    // (edge, t) of every cut.
    let mut cuts: Vec<(usize, f32)> = Vec::new();
    let mut runs: Vec<Vec<usize>> = Vec::new();
    let first = (0..n).find(|&i| ring.tags[i] == Tag::Front && breaks(i));
    match first {
        Some(f) => {
            let mut k = 0;
            while k < n {
                let i = (f + k) % n;
                if ring.tags[i] == Tag::Front && (k == 0 || breaks(i)) {
                    let mut run = vec![i];
                    let mut j = (i + 1) % n;
                    while j != f && ring.tags[j] == Tag::Front && !breaks(j) {
                        run.push(j);
                        j = (j + 1) % n;
                    }
                    k += run.len();
                    runs.push(run);
                } else {
                    k += 1;
                }
            }
        }
        None => {
            // A closed frontage with no corners: start at the sharpest vertex.
            if ring.tags.iter().all(|t| *t == Tag::Front) {
                let s = (0..n).max_by(|&a, &b| turn(p, a).total_cmp(&turn(p, b)).then(b.cmp(&a))).unwrap_or(0);
                runs.push((0..n).map(|k| (s + k) % n).collect());
                cuts.push((s, 0.0));
            }
        }
    }
    let width = |rng: &mut Rng| rng.range(rules.width[0], rules.width[1].max(rules.width[0]));
    for run in &runs {
        let lens: Vec<f32> = run.iter().map(|&i| p[i].distance(p[(i + 1) % n])).collect();
        let total: f32 = lens.iter().sum();
        // Ground level at arc length `t` along the run.
        let z_at = |t: f32| -> f32 {
            if ring.z.is_empty() {
                return 0.0;
            }
            let mut acc = 0.0;
            for (k, &i) in run.iter().enumerate() {
                if t <= acc + lens[k] || k + 1 == run.len() {
                    let f = ((t - acc) / lens[k].max(1e-4)).clamp(0.0, 1.0);
                    return ring.z[i] + (ring.z[(i + 1) % n] - ring.z[i]) * f;
                }
                acc += lens[k];
            }
            0.0
        };
        // On a slope a house steps down with its street: its frontage
        // drops at most `rules.step`, so narrower houses climb stair streets.
        // Corner lots too: a corner house on a stair street gives up its
        // reach rather than stand on a storey of plinth.
        let fit = |t: f32, mut w: f32, dir: f32| {
            let floor = rules.min_width.max(rules.width[0] * 0.6);
            while w > floor && (z_at(t + dir * w) - z_at(t)).abs() > rules.step {
                w = (w * 0.85).max(floor);
            }
            w
        };
        let (i0, i1) = (run[0], (run[run.len() - 1] + 1) % n);
        let reach = |theta: Option<f32>, rng: &mut Rng| match theta {
            Some(t) => {
                let cot = 1.0 / (t * 0.5).tan().max(0.05);
                (rules.min_width.max(rules.depth * cot.min(4.0)) + rng.range(2.5, 6.0)).min(total * 0.5)
            }
            None => 0.0,
        };
        let (ts, te) = (corner_theta(i0, true), corner_theta(i1, false));
        let (r0, r1) = (reach(ts, rng), reach(te, rng));
        let (mut r0, mut r1) = (if r0 > 0.0 { fit(0.0, r0, 1.0) } else { 0.0 }, if r1 > 0.0 { fit(total, r1, -1.0) } else { 0.0 });
        let mut pos: Vec<f32> = Vec::new();
        if total - r0 - r1 < rules.width[0] {
            // No room for a middle lot: the corner lots share the run.
            match (ts.is_some(), te.is_some()) {
                (true, true) => pos.push(total * 0.5),
                (true, false) if total - r0 >= rules.min_width => pos.push(r0),
                (false, true) if total - r1 >= rules.min_width => pos.push(total - r1),
                _ => {}
            }
        } else {
            if r0 > 0.0 {
                pos.push(r0);
            }
            let end = total - r1;
            let mut t = r0;
            while end - t > 0.5 {
                let mut w = fit(t, width(rng), 1.0);
                if end - (t + w) < rules.width[0] {
                    w = end - t;
                }
                t += w;
                if t < total - 0.5 {
                    pos.push(t);
                }
            }
            if r1 > 0.0 && pos.last().is_none_or(|l| (l - (total - r1)).abs() > 0.5) {
                pos.push(total - r1);
            }
            r0 = 0.0;
            r1 = 0.0;
        }
        let _ = (r0, r1);
        // Arc positions of the run's vertices, to snap cuts onto bends.
        let mut acc = vec![0.0f32];
        for l in &lens {
            acc.push(acc.last().unwrap() + l);
        }
        for c in pos {
            if c <= 0.05 || c >= total - 0.05 {
                continue;
            }
            // Snap to a nearby bend (no short facades beside a party wall).
            let mut c = c;
            for (k, a) in acc.iter().enumerate().skip(1).take(run.len().saturating_sub(1)) {
                let sharp = turn(p, run[k]) > 10f32.to_radians();
                if (sharp && (c - a).abs() < 1.0) || (c - a).abs() < 0.3 {
                    c = *a;
                    break;
                }
            }
            let k = acc.partition_point(|a| *a <= c).saturating_sub(1).min(run.len() - 1);
            let t = if lens[k] > 0.0 { (c - acc[k]) / lens[k] } else { 0.0 };
            cuts.push((run[k], t.clamp(0.0, 1.0)));
        }
    }
    for c in cuts.iter_mut() {
        if c.1 >= 1.0 - 1e-4 {
            *c = ((c.0 + 1) % n, 0.0);
        }
    }
    cuts.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
    cuts.dedup_by(|a, b| a.0 == b.0 && (a.1 - b.1).abs() < 1e-4);
    // Rebuild the ring with the cut points.
    let mut out = Vec::with_capacity(n + cuts.len());
    let mut tags = Vec::with_capacity(n + cuts.len());
    let mut is_cut = Vec::with_capacity(n + cuts.len());
    for i in 0..n {
        let on_edge: Vec<f32> = cuts.iter().filter(|c| c.0 == i).map(|c| c.1).collect();
        out.push(p[i]);
        tags.push(ring.tags[i]);
        is_cut.push(on_edge.iter().any(|t| *t <= 1e-4));
        for t in on_edge.into_iter().filter(|t| *t > 1e-4) {
            out.push(crate::plan::region::snap(p[i].lerp(p[(i + 1) % n], t)));
            tags.push(ring.tags[i]);
            is_cut.push(true);
        }
    }
    // Remove duplicate consecutive vertices (merging cut flags).
    let mut k = 0;
    while k < out.len() && out.len() > 3 {
        let j = (k + 1) % out.len();
        if out[k].distance(out[j]) < 1e-4 {
            is_cut[k] = is_cut[k] || is_cut[j];
            let t = tags[j];
            out.remove(j);
            tags.remove(j);
            is_cut.remove(j);
            if j > k {
                tags[k] = t;
            }
        } else {
            k += 1;
        }
    }
    (out, tags, is_cut)
}

/// Power-diagram cell edges between sites of different owners.
fn power_cuts(sites: &[Site], region: &Region, rules: &LotRules, rng: &mut Rng) -> Vec<Vec<Vec2>> {
    let Some([lo, hi]) = region.bbox() else { return Vec::new() };
    let (lo, hi) = (lo - Vec2::splat(2.0), hi + Vec2::splat(2.0));
    let wmax = sites.iter().map(|s| s.w).fold(0.0, f64::max);
    // Sites by position for pruning.
    let mut order: Vec<usize> = (0..sites.len()).collect();
    let mut cuts: Vec<Vec<Vec2>> = Vec::new();
    for (i, si) in sites.iter().enumerate() {
        // Cell polygon in f64 with the generator of each edge (edge k runs
        // from poly[k] to poly[k + 1]).
        let mut poly: Vec<([f64; 2], Option<usize>)> = vec![([lo.x as f64, lo.y as f64], None), ([hi.x as f64, lo.y as f64], None), ([hi.x as f64, hi.y as f64], None), ([lo.x as f64, hi.y as f64], None)];
        order.sort_by(|&a, &b| si.s.distance_squared(sites[a].s).total_cmp(&si.s.distance_squared(sites[b].s)).then(a.cmp(&b)));
        for &j in &order {
            if j == i {
                continue;
            }
            let sj = &sites[j];
            // Prune: no site this far can cut the cell.
            let rho = poly.iter().map(|(p, _)| ((p[0] - si.s.x as f64).powi(2) + (p[1] - si.s.y as f64).powi(2)).sqrt()).fold(0.0, f64::max);
            let d = si.s.distance(sj.s) as f64;
            if d > rho && (d - rho).powi(2) - wmax > rho * rho - si.w {
                break;
            }
            // Keep x with power(x, i) <= power(x, j):
            // 2 x·(sj - si) <= |sj|² - wj - |si|² + wi.
            let (ax, ay) = (2.0 * (sj.s.x as f64 - si.s.x as f64), 2.0 * (sj.s.y as f64 - si.s.y as f64));
            let c = (sj.s.x as f64).powi(2) + (sj.s.y as f64).powi(2) - sj.w - (si.s.x as f64).powi(2) - (si.s.y as f64).powi(2) + si.w;
            poly = clip_half(&poly, ax, ay, c, j);
            if poly.len() < 3 {
                break;
            }
        }
        for k in 0..poly.len() {
            let (p, g) = poly[k];
            let q = poly[(k + 1) % poly.len()].0;
            let Some(j) = g else { continue };
            if j < i || sites[j].owner == si.owner {
                continue;
            }
            let (a, b) = (Vec2::new(p[0] as f32, p[1] as f32), Vec2::new(q[0] as f32, q[1] as f32));
            let len = a.distance(b);
            if len < 1e-3 {
                continue;
            }
            // Party walls may wave a little (both neighbours share it).
            let amp = rules.wobble.min(len * 0.1);
            if amp > 0.02 && len > 3.0 {
                let nrm = left((b - a) / len);
                let k = 1 + rng.below(2);
                let mut line = vec![a];
                for m in 1..=k {
                    let t = m as f32 / (k + 1) as f32;
                    line.push(a.lerp(b, t) + nrm * amp * rng.range(-1.0, 1.0));
                }
                line.push(b);
                cuts.push(line);
            } else {
                cuts.push(vec![a, b]);
            }
        }
    }
    cuts
}

/// Clips a cell to `ax x + ay y <= c`; new edges remember generator `j`.
fn clip_half(poly: &[([f64; 2], Option<usize>)], ax: f64, ay: f64, c: f64, j: usize) -> Vec<([f64; 2], Option<usize>)> {
    let n = poly.len();
    let f = |p: [f64; 2]| ax * p[0] + ay * p[1] - c;
    let mut out = Vec::with_capacity(n + 2);
    for k in 0..n {
        let (p, g) = poly[k];
        let q = poly[(k + 1) % n].0;
        let (fp, fq) = (f(p), f(q));
        if fp <= 0.0 {
            out.push((p, g));
            if fq > 0.0 {
                let t = fp / (fp - fq);
                out.push(([p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t], Some(j)));
            }
        } else if fq <= 0.0 {
            let t = fp / (fp - fq);
            out.push(([p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t], g));
        }
    }
    out
}

/// A point strictly inside a shape.
pub fn interior_point(s: &Shape) -> Option<Vec2> {
    let r = Region::from_shapes(std::slice::from_ref(s));
    if r.is_empty() {
        return None;
    }
    let n = s.outer.len();
    let c = s.outer.iter().copied().sum::<Vec2>() / n as f32;
    if r.contains(c) && r.boundary_distance(c) > 0.02 {
        return Some(c);
    }
    // Next to the longest edge.
    if let Some(i) = (0..n).max_by(|&a, &b| s.outer[a].distance(s.outer[(a + 1) % n]).total_cmp(&s.outer[b].distance(s.outer[(b + 1) % n]))) {
        let (a, b) = (s.outer[i], s.outer[(i + 1) % n]);
        let q = (a + b) * 0.5 + left((b - a).normalize_or(Vec2::X)) * 0.01;
        if r.contains(q) {
            return Some(q);
        }
    }
    r.pole().map(|p| p.0)
}

/// Length of a region's boundary lying on the frontage.
fn frontage(r: &Region, front: &SegIndex) -> f32 {
    let mut total = 0.0;
    for (a, b) in r.edges() {
        let (a, b) = (Vec2::new(a[0] as f32, a[1] as f32), Vec2::new(b[0] as f32, b[1] as f32));
        total += overlap_len(a, b, front);
    }
    total
}

/// Length of segment `ab` lying (within 2 mm) on segments of `ix`.
pub fn overlap_len(a: Vec2, b: Vec2, ix: &SegIndex) -> f32 {
    let len = a.distance(b);
    if len < 1e-5 {
        return 0.0;
    }
    let d = (b - a) / len;
    let mut iv: Vec<(f32, f32)> = Vec::new();
    for k in ix.around((a + b) * 0.5, len * 0.5 + 0.01) {
        let (p, q, _) = ix.seg(k);
        if crate::plan::line::seg_dist(p, a, b) > 0.002 && crate::plan::line::seg_dist(q, a, b) > 0.002 {
            // Not touching the line at the ends: may still contain ab.
            if !(crate::plan::line::seg_dist(a, p, q) < 0.002 && crate::plan::line::seg_dist(b, p, q) < 0.002) {
                continue;
            }
        }
        let e = q - p;
        if e.length() < 1e-5 || e.normalize().perp_dot(d).abs() > 0.01 {
            continue;
        }
        if crate::plan::line::seg_dist(p, a - d * 1e3, b + d * 1e3) > 0.002 {
            continue;
        }
        let (t0, t1) = ((p - a).dot(d), (q - a).dot(d));
        let (lo, hi) = (t0.min(t1).max(0.0), t0.max(t1).min(len));
        if hi > lo {
            iv.push((lo, hi));
        }
    }
    iv.sort_by(|x, y| x.0.total_cmp(&y.0));
    let mut total = 0.0;
    let mut cur: Option<(f32, f32)> = None;
    for (lo, hi) in iv {
        match cur {
            Some((c0, c1)) if lo <= c1 => cur = Some((c0, c1.max(hi))),
            Some((c0, c1)) => {
                total += c1 - c0;
                cur = Some((lo, hi));
            }
            None => cur = Some((lo, hi)),
        }
    }
    if let Some((c0, c1)) = cur {
        total += c1 - c0;
    }
    total
}

/// Length of the boundary two regions share.
pub fn shared_len(a: &Region, b: &Region) -> f32 {
    let mut ix = SegIndex::new(8.0);
    for (p, q) in b.edges() {
        ix.push(Vec2::new(p[0] as f32, p[1] as f32), Vec2::new(q[0] as f32, q[1] as f32), 0);
    }
    a.edges().map(|(p, q)| overlap_len(Vec2::new(p[0] as f32, p[1] as f32), Vec2::new(q[0] as f32, q[1] as f32), &ix)).sum()
}

/// Whether a unit is a good lot on its own.
fn good(r: &Region, rules: &LotRules, front: &SegIndex) -> bool {
    if r.len() != 1 || r.shapes()[0].holes.len() > 0 {
        return false;
    }
    let a = r.area() as f32;
    let per: f32 = r.edges().map(|(p, q)| ((q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2)).sqrt() as f32).sum();
    let fr = frontage(r, front);
    fr >= rules.min_width.min(rules.width[0]) && a >= rules.min_area && 2.0 * a / per.max(1e-3) >= 1.8
}

/// The single repair rule: the smallest unit that is not a good lot merges
/// into the neighbour it shares the longest boundary with, until every
/// unit is good (slivers and orphans first, so enclosed pieces fill the
/// holes they make before anything larger moves).
fn repair(mut units: Vec<Region>, rules: &LotRules, front: &SegIndex) -> Vec<Region> {
    let mut ok: Vec<bool> = units.iter().map(|u| good(u, rules, front)).collect();
    let mut stuck = vec![false; units.len()];
    for _ in 0..units.len() * 2 + 4 {
        let Some(i) = (0..units.len()).filter(|&i| !ok[i] && !stuck[i]).min_by(|&a, &b| units[a].area().total_cmp(&units[b].area()).then(a.cmp(&b))) else { break };
        let bb = units[i].bbox();
        let best = (0..units.len())
            .filter(|&j| j != i)
            .filter(|&j| match (bb, units[j].bbox()) {
                (Some(a), Some(b)) => a[0].x <= b[1].x + 0.01 && b[0].x <= a[1].x + 0.01 && a[0].y <= b[1].y + 0.01 && b[0].y <= a[1].y + 0.01,
                _ => false,
            })
            .map(|j| (shared_len(&units[i], &units[j]), j))
            .filter(|(l, _)| *l > 0.05)
            .max_by(|a, b| a.0.total_cmp(&b.0).then(b.1.cmp(&a.1)));
        match best {
            Some((_, j)) => {
                let m = units[i].union(&units[j]);
                let (lo, hi) = (i.min(j), i.max(j));
                if std::env::var("FORJA_REPAIR_DEBUG").is_ok() {
                    eprintln!("merge {:.1} into {:.1} (good {}) -> shapes {} holes {} good {}", units[i].area(), units[j].area(), ok[j], m.len(), m.shapes().first().map(|s| s.holes.len()).unwrap_or(0), good(&m, rules, front));
                }
                units[lo] = m;
                ok[lo] = good(&units[lo], rules, front);
                stuck[lo] = false;
                units.remove(hi);
                ok.remove(hi);
                stuck.remove(hi);
            }
            None => stuck[i] = true,
        }
    }
    units.into_iter().zip(ok).filter(|(_, g)| *g).map(|(u, _)| u).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules() -> LotRules {
        LotRules { width: [8.0, 14.0], depth: 13.0, min_width: 3.5, min_area: 30.0, court_min: 80.0, neck: 3.0, min_tip: 2.5, jitter: 0.0, wobble: 0.0, step: 1.5 }
    }

    fn rect_ring(w: f32, h: f32) -> BlockRing {
        BlockRing {
            pts: vec![Vec2::ZERO, Vec2::new(w, 0.0), Vec2::new(w, h), Vec2::new(0.0, h)],
            tags: vec![Tag::Front; 4],
            apex: vec![Some(0), Some(1), Some(2), Some(3)],
            theta: vec![std::f32::consts::FRAC_PI_2; 4],
            z: vec![],
        }
    }

    #[test]
    fn rectangle_block_gets_corner_lots_and_a_court() {
        let ring = rect_ring(60.0, 50.0);
        let shape = Shape { outer: ring.pts.clone(), holes: vec![] };
        let p = parcel(&shape, &ring, &rules(), 25.0, &mut Rng::new(1));
        let cols = ["#e74c3c", "#3498db", "#2ecc71", "#f1c40f", "#9b59b6", "#1abc9c", "#e67e22"];
        let mut list: Vec<(&Region, &str)> = p.lots.iter().enumerate().map(|(i, l)| (l, cols[i % cols.len()])).collect();
        list.push((&p.courts, "#555"));
        let _ = std::fs::write(std::env::temp_dir().join("parcel_rect.svg"), crate::svg::regions(&list, 8.0));
        assert!(p.lots.len() >= 10, "{} lots", p.lots.len());
        let lots: f64 = p.lots.iter().map(|l| l.area()).sum();
        assert!((lots + p.courts.area() - 3000.0).abs() < 0.5, "{lots} + {}", p.courts.area());
        assert!((p.courts.area() - (60.0 - 26.0) * (50.0 - 26.0)).abs() < 1.0, "court {}", p.courts.area());
        // Every corner of the block belongs to one lot that wraps it.
        for c in [Vec2::new(0.5, 0.5), Vec2::new(59.5, 0.5), Vec2::new(59.5, 49.5), Vec2::new(0.5, 49.5)] {
            let l = p.lots.iter().find(|l| l.contains(c)).unwrap();
            let [lo, hi] = l.bbox().unwrap();
            assert!(hi.x - lo.x >= 13.0 && hi.y - lo.y >= 13.0, "corner lot {lo} {hi}");
        }
        // Disjoint.
        let u = Region::union_all(p.lots.iter());
        assert!((u.area() - lots).abs() < 0.01);
    }

    #[test]
    fn houses_step_down_a_slope() {
        // A long block climbing 12 m along x (a 15 % street): flat, lots
        // are 8-14 m wide; sloping, no lot's frontage drops more than
        // `step` beyond what its narrowest allowed width implies.
        let flat = rect_ring(100.0, 30.0);
        let shape = Shape { outer: flat.pts.clone(), holes: vec![] };
        let widest = |p: &Parcels| p.lots.iter().filter_map(|l| l.bbox()).filter(|[lo, hi]| lo.y < 0.1 && hi.x - lo.x < 40.0 && lo.x > 20.0 && hi.x < 80.0).map(|[lo, hi]| hi.x - lo.x).fold(0.0, f32::max);
        let a = parcel(&shape, &flat, &rules(), 15.0, &mut Rng::new(5));
        let slope = BlockRing { z: flat.pts.iter().map(|p| p.x * 0.15).collect(), ..flat.clone() };
        let b = parcel(&shape, &slope, &rules(), 15.0, &mut Rng::new(5));
        let (wa, wb) = (widest(&a), widest(&b));
        assert!(wa >= 8.0 && wb <= 1.5 / 0.15 + 0.01, "flat {wa}, sloping {wb}");
        assert!(b.lots.len() > a.lots.len(), "{} vs {}", b.lots.len(), a.lots.len());
    }

    #[test]
    fn curved_block_party_walls_start_at_cuts() {
        // A block between an arc of radius 40 and its chord.
        let mut pts = Vec::new();
        for i in 0..=30 {
            let a = std::f32::consts::PI * (1.0 - i as f32 / 30.0);
            pts.push(crate::plan::region::snap(Vec2::new(a.cos(), a.sin()) * 40.0));
        }
        pts.reverse();
        // CCW: from (-40,0) around the top... reversed arc goes from (40,0)? Make CCW.
        let mut ring_pts = pts.clone();
        if crate::geom::signed_area(&ring_pts) < 0.0 {
            ring_pts.reverse();
        }
        let n = ring_pts.len();
        let ring = BlockRing { pts: ring_pts.clone(), tags: vec![Tag::Front; n], apex: vec![None; n], theta: vec![], z: vec![] };
        let shape = Shape { outer: ring_pts, holes: vec![] };
        let p = parcel(&shape, &ring, &LotRules { depth: 12.0, ..rules() }, 15.0, &mut Rng::new(3));
        assert!(p.lots.len() >= 8, "{}", p.lots.len());
        let total: f64 = p.lots.iter().map(|l| l.area()).sum::<f64>() + p.courts.area();
        assert!((total - Region::from_shapes(&[shape]).area()).abs() < 0.5);
    }
}
