//! `StreetGraph`: the planarized street network.
//!
//! `build` turns any soup of `Way`s (grid lines, carved alleys, OSM ways,
//! in any order and orientation) into a planar graph: crossings are split,
//! line ends lying on another street become T-junctions, close nodes merge,
//! collinear duplicates collapse into the higher-ranked street, tiny edges
//! and stubs go, and only the largest connected network stays. Node and
//! street ids are canonical (sorted by position), so the same streets in a
//! different order give the same graph bit for bit.

use std::collections::{BTreeMap, HashMap};

use forja_math::Vec2;

use super::draft::{NodeMark, Way};
use super::line::{Polyline, left, mix64};
use super::region::Region;
use crate::lot::MAX_CHORD;
use crate::profile::{ProfileId, ProfileLibrary};

pub type NodeId = u32;
pub type StreetId = u32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum End {
    A,
    B,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Node {
    pub p: Vec2,
    pub signals: bool,
    pub end: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Street {
    pub a: NodeId,
    pub b: NodeId,
    pub line: Polyline,
    /// Building line to building line, per vertex of `line`.
    pub width: Vec<f32>,
    pub profile: ProfileId,
    /// Roadway width (0 = shared surface).
    pub carriageway: f32,
    /// +1 along `line`, -1 against it, 0 both ways.
    pub oneway: i8,
    /// Index into `StreetGraph::names` (0 = unnamed).
    pub name: u16,
    pub zone: u16,
    pub osm: Option<i64>,
    /// Stable key from geometry and profile.
    pub key: u64,
}

impl Street {
    pub fn len(&self) -> f32 {
        self.line.len()
    }

    pub fn is_empty(&self) -> bool {
        self.line.is_empty()
    }

    pub fn node(&self, e: End) -> NodeId {
        match e {
            End::A => self.a,
            End::B => self.b,
        }
    }

    /// Width at arc length `t`.
    pub fn width_at(&self, t: f32) -> f32 {
        let (i, f) = self.line.locate(t);
        let n = self.width.len();
        if n == 0 {
            return 0.0;
        }
        let (a, b) = (self.width[i.min(n - 1)], self.width[(i + 1).min(n - 1)]);
        a + (b - a) * f
    }

    pub fn min_width(&self) -> f32 {
        self.width.iter().copied().fold(f32::INFINITY, f32::min)
    }

    /// Arc length of end `e`.
    pub fn end_s(&self, e: End) -> f32 {
        match e {
            End::A => 0.0,
            End::B => self.len(),
        }
    }

    /// Unit direction leaving the node at end `e`, measured over the first
    /// `reach` metres.
    pub fn heading(&self, e: End, reach: f32) -> Vec2 {
        let l = self.len();
        let r = reach.min(l * 0.5).max(1e-3);
        match e {
            End::A => (self.line.at(r) - self.line.at(0.0)).normalize_or(Vec2::X),
            End::B => (self.line.at(l - r) - self.line.at(l)).normalize_or(Vec2::X),
        }
    }
}

/// Segment index for neighbourhood queries.
#[derive(Clone, Debug, Default, PartialEq)]
struct SegGrid {
    cell: f32,
    map: HashMap<(i32, i32), Vec<(StreetId, u32)>>,
}

impl SegGrid {
    fn key(&self, p: Vec2) -> (i32, i32) {
        ((p.x / self.cell).floor() as i32, (p.y / self.cell).floor() as i32)
    }

    fn insert(&mut self, a: Vec2, b: Vec2, id: (StreetId, u32)) {
        let (lo, hi) = (self.key(a.min(b)), self.key(a.max(b)));
        for x in lo.0..=hi.0 {
            for z in lo.1..=hi.1 {
                self.map.entry((x, z)).or_default().push(id);
            }
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct StreetGraph {
    pub nodes: Vec<Node>,
    pub streets: Vec<Street>,
    pub names: Vec<String>,
    stars: Vec<Vec<(StreetId, End, f32)>>,
    grid: SegGrid,
}

// ------------------------------------------------------------------ build

type P = (i64, i64);

fn mm(p: Vec2) -> P {
    (super::region::to_mm(p.x) as i64, super::region::to_mm(p.y) as i64)
}

fn pf(p: P) -> Vec2 {
    Vec2::new((p.0 as f64 / 1000.0) as f32, (p.1 as f64 / 1000.0) as f32)
}

fn orient(a: P, b: P, c: P) -> i128 {
    (b.0 - a.0) as i128 * (c.1 - a.1) as i128 - (b.1 - a.1) as i128 * (c.0 - a.0) as i128
}

/// Attributes that must agree along one street.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct Attr {
    profile: ProfileId,
    /// Carriageway override in mm, or -1.
    carriage: i64,
    name: u16,
    zone: u16,
}

#[derive(Clone, Debug)]
struct Edge {
    u: usize,
    v: usize,
    wu: f32,
    wv: f32,
    /// Oneway relative to u -> v.
    oneway: i8,
    attr: Attr,
    rank: u8,
    osm: Option<i64>,
}

struct Uf(Vec<usize>);

impl Uf {
    fn find(&mut self, mut i: usize) -> usize {
        while self.0[i] != i {
            self.0[i] = self.0[self.0[i]];
            i = self.0[i];
        }
        i
    }
}

impl StreetGraph {
    /// Planarizes `ways` into a canonical graph. `snap` is the node merge
    /// distance (m).
    pub fn build(ways: &[Way], marks: &[NodeMark], lib: &ProfileLibrary, snap: f32) -> Self {
        let snap_mm = (snap.max(0.001) * 1000.0).round() as i64;
        // Names in sorted order so ids do not depend on way order.
        let mut names: Vec<String> = ways.iter().filter_map(|w| w.name.clone()).collect();
        names.sort();
        names.dedup();
        names.insert(0, String::new());
        // 1. Thin, resample and snap each way.
        struct Pre {
            pts: Vec<P>,
            w: Vec<f32>,
            attr: Attr,
            rank: u8,
            oneway: i8,
            osm: Option<i64>,
        }
        let mut pre: Vec<Pre> = Vec::new();
        for way in ways {
            let Some(pid) = lib.id(&way.profile) else {
                log::warn!("street graph: unknown profile {}", way.profile);
                continue;
            };
            let prof = lib.get(pid);
            let fallback = prof.default_width();
            // Canonical direction (by endpoints on the mm grid), so thinning and
            // resampling do not depend on how the way was drawn.
            let raw: Vec<(Vec2, f32)> = way.line.iter().enumerate().filter(|(_, p)| p.is_finite()).map(|(i, p)| (*p, way.width_at(i, fallback).max(0.0))).collect();
            let backwards = match (raw.first(), raw.last()) {
                (Some(f), Some(l)) if mm(f.0) != mm(l.0) => mm(f.0) > mm(l.0),
                _ => raw.len() > 2 && mm(raw[1].0) > mm(raw[raw.len() - 2].0),
            };
            let raw: Vec<(Vec2, f32)> = if backwards { raw.into_iter().rev().collect() } else { raw };
            let oneway = if backwards { -way.oneway.signum() } else { way.oneway.signum() };
            let mut pts: Vec<(Vec2, f32)> = Vec::new();
            for (i, &(p, w)) in raw.iter().enumerate() {
                let p = &p;
                let last = i + 1 == raw.len();
                match pts.last() {
                    Some((q, _)) if q.distance(*p) < 0.5 && !last => continue,
                    Some((q, _)) if q.distance(*p) < 0.5 && last && pts.len() > 1 => {
                        pts.pop();
                    }
                    _ => {}
                }
                if pts.last().is_some_and(|(q, _)| q.distance(*p) < 1e-3) {
                    continue;
                }
                pts.push((*p, if w.is_finite() { w } else { fallback }));
            }
            if pts.len() < 2 {
                continue;
            }
            let mut rp: Vec<P> = Vec::new();
            let mut rw: Vec<f32> = Vec::new();
            for i in 0..pts.len() {
                if i > 0 {
                    let ((a, wa), (b, wb)) = (pts[i - 1], pts[i]);
                    let k = (a.distance(b) / MAX_CHORD).ceil().max(1.0) as usize;
                    for j in 1..k {
                        let t = j as f32 / k as f32;
                        rp.push(mm(a.lerp(b, t)));
                        rw.push(wa + (wb - wa) * t);
                    }
                }
                let q = mm(pts[i].0);
                if rp.last() != Some(&q) {
                    rp.push(q);
                    rw.push(pts[i].1);
                }
            }
            if rp.len() < 2 {
                continue;
            }
            let name = way.name.as_ref().and_then(|n| names.binary_search(n).ok()).unwrap_or(0) as u16;
            let carriage = way.carriageway.map(|c| (c.max(0.0) * 1000.0).round() as i64).unwrap_or(-1);
            pre.push(Pre { pts: rp, w: rw, attr: Attr { profile: pid, carriage, name, zone: way.zone }, rank: prof.rank, oneway, osm: way.osm });
        }
        // 2. Split points per segment: crossings and vertices snapped onto
        // segments (T-junctions, collinear overlaps).
        let cell = 32_000i64;
        let mut grid: HashMap<(i64, i64), Vec<(usize, usize)>> = HashMap::new();
        for (wi, w) in pre.iter().enumerate() {
            for si in 0..w.pts.len() - 1 {
                let (a, b) = (w.pts[si], w.pts[si + 1]);
                let (x0, x1) = ((a.0.min(b.0) - snap_mm).div_euclid(cell), (a.0.max(b.0) + snap_mm).div_euclid(cell));
                let (z0, z1) = ((a.1.min(b.1) - snap_mm).div_euclid(cell), (a.1.max(b.1) + snap_mm).div_euclid(cell));
                for x in x0..=x1 {
                    for z in z0..=z1 {
                        grid.entry((x, z)).or_default().push((wi, si));
                    }
                }
            }
        }
        let mut splits: HashMap<(usize, usize), Vec<(f64, P)>> = HashMap::new();
        let mut cells: Vec<&(i64, i64)> = grid.keys().collect();
        cells.sort();
        let mut seen = std::collections::HashSet::new();
        for c in cells {
            let list = &grid[c];
            for (i, &(wa, sa)) in list.iter().enumerate() {
                for &(wb, sb) in &list[i + 1..] {
                    let pair = if (wa, sa) < (wb, sb) { ((wa, sa), (wb, sb)) } else { ((wb, sb), (wa, sa)) };
                    if wa == wb && sa.abs_diff(sb) <= 1 || !seen.insert(pair) {
                        continue;
                    }
                    // Geometric order of the pair, so a crossing rounds to the same
                    // millimetre whatever the order of the ways.
                    let seg = |w: usize, k: usize| (pre[w].pts[k], pre[w].pts[k + 1]);
                    let ((wa, sa), (wb, sb)) = if seg(wa, sa) <= seg(wb, sb) { ((wa, sa), (wb, sb)) } else { ((wb, sb), (wa, sa)) };
                    let (p1, q1) = seg(wa, sa);
                    let (p2, q2) = seg(wb, sb);
                    let (o1, o2, o3, o4) = (orient(p1, q1, p2), orient(p1, q1, q2), orient(p2, q2, p1), orient(p2, q2, q1));
                    if o1.signum() * o2.signum() < 0 && o3.signum() * o4.signum() < 0 {
                        // Proper crossing.
                        let t = o3 as f64 / (o3 - o4) as f64;
                        let x = (p1.0 as f64 + (q1.0 - p1.0) as f64 * t).round() as i64;
                        let z = (p1.1 as f64 + (q1.1 - p1.1) as f64 * t).round() as i64;
                        let u = o1 as f64 / (o1 - o2) as f64;
                        splits.entry((wa, sa)).or_default().push((t, (x, z)));
                        splits.entry((wb, sb)).or_default().push((u, (x, z)));
                    }
                    // Vertices of one lying within snap of the other.
                    for (v, (ws, ss), (s0, s1)) in [(p2, (wa, sa), (p1, q1)), (q2, (wa, sa), (p1, q1)), (p1, (wb, sb), (p2, q2)), (q1, (wb, sb), (p2, q2))] {
                        if let Some((t, q)) = snap_onto(v, s0, s1, snap_mm) {
                            splits.entry((ws, ss)).or_default().push((t, q));
                        }
                    }
                }
            }
        }
        // 3. Point table, merged by distance (union-find).
        let mut pts: Vec<P> = Vec::new();
        let mut index: HashMap<P, usize> = HashMap::new();
        let mut incid: Vec<u32> = Vec::new();
        let mut id_of = |p: P, pts: &mut Vec<P>, incid: &mut Vec<u32>| -> usize {
            *index.entry(p).or_insert_with(|| {
                pts.push(p);
                incid.push(0);
                pts.len() - 1
            })
        };
        let mut seqs: Vec<Vec<(usize, f32)>> = Vec::new();
        for (wi, w) in pre.iter().enumerate() {
            let mut seq: Vec<(usize, f32)> = Vec::new();
            for si in 0..w.pts.len() {
                let id = id_of(w.pts[si], &mut pts, &mut incid);
                seq.push((id, w.w[si]));
                if si + 1 < w.pts.len() {
                    if let Some(sp) = splits.get_mut(&(wi, si)) {
                        sp.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
                        for (t, q) in sp.iter() {
                            let id = id_of(*q, &mut pts, &mut incid);
                            let wt = w.w[si] + (w.w[si + 1] - w.w[si]) * (*t as f32);
                            seq.push((id, wt));
                        }
                    }
                }
            }
            for (k, (id, _)) in seq.iter().enumerate() {
                incid[*id] += if k == 0 || k + 1 == seq.len() { 1 } else { 2 };
            }
            seqs.push(seq);
        }
        let mut uf = Uf((0..pts.len()).collect());
        {
            let c = snap_mm.max(1) * 2;
            let mut g: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
            for (i, p) in pts.iter().enumerate() {
                g.entry((p.0.div_euclid(c), p.1.div_euclid(c))).or_default().push(i);
            }
            let s2 = (snap_mm * snap_mm) as i128;
            for (i, p) in pts.iter().enumerate() {
                let (cx, cz) = (p.0.div_euclid(c), p.1.div_euclid(c));
                for x in cx - 1..=cx + 1 {
                    for z in cz - 1..=cz + 1 {
                        if let Some(l) = g.get(&(x, z)) {
                            for &j in l {
                                let q = pts[j];
                                let d2 = (p.0 - q.0) as i128 * (p.0 - q.0) as i128 + (p.1 - q.1) as i128 * (p.1 - q.1) as i128;
                                if j != i && d2 <= s2 {
                                    let (ri, rj) = (uf.find(i), uf.find(j));
                                    if ri != rj {
                                        uf.0[ri.max(rj)] = ri.min(rj);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        // Representative of each cluster: most incident point, then smallest.
        let mut rep: HashMap<usize, usize> = HashMap::new();
        for i in 0..pts.len() {
            let r = uf.find(i);
            let e = rep.entry(r).or_insert(i);
            let better = |a: usize, b: usize| (std::cmp::Reverse(incid[a]), pts[a]) < (std::cmp::Reverse(incid[b]), pts[b]);
            if better(i, *e) {
                *e = i;
            }
        }
        let node_of = |i: usize, uf: &mut Uf| rep[&uf.find(i)];
        // 4. Edges, deduplicated by node pair (higher rank wins).
        let mut edges: Vec<Edge> = Vec::new();
        for (wi, seq) in seqs.iter().enumerate() {
            let w = &pre[wi];
            let mut prev: Option<(usize, f32)> = None;
            for &(id, wt) in seq {
                let n = node_of(id, &mut uf);
                if let Some((pn, pw)) = prev {
                    if pn != n {
                        edges.push(Edge { u: pn, v: n, wu: pw, wv: wt, oneway: w.oneway, attr: w.attr.clone(), rank: w.rank, osm: w.osm });
                    }
                }
                if prev.is_none_or(|(pn, _)| pn != n) {
                    prev = Some((n, wt));
                }
            }
        }
        let mut node_pos: HashMap<usize, P> = HashMap::new();
        for e in &edges {
            node_pos.insert(e.u, pts[e.u]);
            node_pos.insert(e.v, pts[e.v]);
        }
        // Contract edges shorter than 0.5 m (junction slivers), keeping the
        // more connected end.
        let mut deg: HashMap<usize, u32> = HashMap::new();
        for e in &edges {
            *deg.entry(e.u).or_default() += 1;
            *deg.entry(e.v).or_default() += 1;
        }
        let mut cuf: HashMap<usize, usize> = HashMap::new();
        fn cfind(m: &mut HashMap<usize, usize>, mut i: usize) -> usize {
            while let Some(&j) = m.get(&i) {
                if j == i {
                    break;
                }
                i = j;
            }
            i
        }
        let mut short: Vec<(usize, usize)> = edges.iter().filter(|e| dist(pts[e.u], pts[e.v]) < 500.0).map(|e| (e.u.min(e.v), e.u.max(e.v))).collect();
        short.sort_by_key(|&(u, v)| (pts[u], pts[v]));
        short.dedup();
        for (u, v) in short {
            let (ru, rv) = (cfind(&mut cuf, u), cfind(&mut cuf, v));
            if ru == rv {
                continue;
            }
            let key = |n: usize| (std::cmp::Reverse(deg.get(&n).copied().unwrap_or(0)), pts[n]);
            let (keep, gone) = if key(ru) <= key(rv) { (ru, rv) } else { (rv, ru) };
            cuf.insert(gone, keep);
        }
        for e in edges.iter_mut() {
            e.u = cfind(&mut cuf, e.u);
            e.v = cfind(&mut cuf, e.v);
        }
        edges.retain(|e| e.u != e.v);
        // Collinear duplicates: one edge per node pair.
        let mut by_pair: BTreeMap<(P, P), Edge> = BTreeMap::new();
        for mut e in edges {
            if pts[e.u] > pts[e.v] {
                std::mem::swap(&mut e.u, &mut e.v);
                std::mem::swap(&mut e.wu, &mut e.wv);
                e.oneway = -e.oneway;
            }
            let k = (pts[e.u], pts[e.v]);
            let score = |e: &Edge| (e.rank, ((e.wu.max(e.wv)) * 1000.0) as i64, e.attr.clone(), e.oneway, std::cmp::Reverse(e.osm), (e.wu * 1000.0) as i64, (e.wv * 1000.0) as i64);
            match by_pair.get(&k) {
                Some(old) if score(old) >= score(&e) => {}
                _ => {
                    by_pair.insert(k, e);
                }
            }
        }
        let mut edges: Vec<Edge> = by_pair.into_values().collect();
        // Marks by node.
        let mark_at = |p: P| marks.iter().filter(|m| m.p.is_finite() && dist(mm(m.p), p) <= snap_mm.max(300) as f64).fold((false, false), |acc, m| (acc.0 || m.signals, acc.1 || m.end));
        // 5. Prune short stubs until stable, then keep the largest component.
        loop {
            let chains = chain(&edges, &pts);
            let mut deg: HashMap<usize, u32> = HashMap::new();
            for e in &edges {
                *deg.entry(e.u).or_default() += 1;
                *deg.entry(e.v).or_default() += 1;
            }
            let mut drop = vec![false; edges.len()];
            let mut any = false;
            for c in &chains {
                let len: f64 = c.edges.iter().map(|&i| dist(pts[edges[i].u], pts[edges[i].v])).sum();
                let (da, db) = (deg[&c.a], deg[&c.b]);
                let dangling = (da == 1 && !mark_at(pts[c.a]).1 && db > 1) || (db == 1 && !mark_at(pts[c.b]).1 && da > 1);
                if dangling && len < 4000.0 {
                    for &i in &c.edges {
                        drop[i] = true;
                    }
                    any = true;
                }
            }
            if !any {
                break;
            }
            let mut k = 0;
            edges.retain(|_| {
                k += 1;
                !drop[k - 1]
            });
        }
        // Largest component by length.
        {
            let mut cu: HashMap<usize, usize> = HashMap::new();
            for e in &edges {
                let (a, b) = (cfind(&mut cu, e.u), cfind(&mut cu, e.v));
                cu.entry(a).or_insert(a);
                cu.entry(b).or_insert(b);
                if a != b {
                    let (lo, hi) = if pts[a] < pts[b] { (a, b) } else { (b, a) };
                    cu.insert(hi, lo);
                }
            }
            let mut size: HashMap<usize, (f64, P)> = HashMap::new();
            for e in &edges {
                let r = cfind(&mut cu, e.u);
                let s = size.entry(r).or_insert((0.0, pts[r]));
                s.0 += dist(pts[e.u], pts[e.v]);
            }
            if let Some(best) = size.iter().max_by(|a, b| a.1.0.total_cmp(&b.1.0).then(b.1.1.cmp(&a.1.1))).map(|(k, _)| *k) {
                edges.retain(|e| cfind(&mut cu, e.u) == best);
            }
        }
        // 6. Chains become streets; canonical ids.
        let chains = chain(&edges, &pts);
        let mut used: Vec<usize> = chains.iter().flat_map(|c| [c.a, c.b]).collect();
        used.sort_by_key(|&i| pts[i]);
        used.dedup();
        let nid: HashMap<usize, NodeId> = used.iter().enumerate().map(|(k, &i)| (i, k as NodeId)).collect();
        let nodes: Vec<Node> = used
            .iter()
            .map(|&i| {
                let (signals, end) = mark_at(pts[i]);
                Node { p: pf(pts[i]), signals, end }
            })
            .collect();
        let mut streets: Vec<Street> = Vec::new();
        for c in &chains {
            let mut vs: Vec<usize> = vec![c.a];
            let mut ws: Vec<f32> = Vec::new();
            let mut oneway = 0i8;
            let mut osm = None;
            let mut cur = c.a;
            let first = &edges[c.edges[0]];
            for (k, &ei) in c.edges.iter().enumerate() {
                let e = &edges[ei];
                let fwd = e.u == cur;
                let (wa, wb) = if fwd { (e.wu, e.wv) } else { (e.wv, e.wu) };
                if k == 0 {
                    ws.push(wa);
                    oneway = if fwd { e.oneway } else { -e.oneway };
                    osm = e.osm;
                } else {
                    let l = ws.len() - 1;
                    ws[l] = (ws[l] + wa) * 0.5;
                }
                ws.push(wb);
                cur = if fwd { e.v } else { e.u };
                vs.push(cur);
            }
            let prof = first.attr.profile;
            let p = lib.get(prof);
            let mut line: Vec<Vec2> = vs.iter().map(|&i| pf(pts[i])).collect();
            let (mut a, mut b) = (nid[&c.a], nid[&c.b]);
            // Orientation: lower node id first; loops by their interior.
            let flip = if a != b { a > b } else { line.len() > 2 && pts[vs[1]] > pts[vs[vs.len() - 2]] };
            if flip {
                line.reverse();
                ws.reverse();
                oneway = -oneway;
                std::mem::swap(&mut a, &mut b);
            }
            let minw = ws.iter().copied().fold(f32::INFINITY, f32::min);
            let carriage = if first.attr.carriage >= 0 { first.attr.carriage as f32 / 1000.0 } else { p.carriageway };
            let carriageway = carriage.min(minw).max(0.0);
            let q = |v: Vec2| ((v.x * 100.0).round() as i64 as u64, (v.y * 100.0).round() as i64 as u64);
            let (qa, qb) = (q(line[0]), q(*line.last().unwrap()));
            let key = mix64(mix64(mix64(qa.0 ^ qb.0.rotate_left(17)) ^ qa.1 ^ qb.1.rotate_left(29)) ^ prof.0 as u64 ^ (line.len() as u64) << 40);
            streets.push(Street {
                a,
                b,
                line: Polyline::new(line),
                width: ws,
                profile: prof,
                carriageway,
                oneway: if p.vehicles { oneway } else { 0 },
                name: first.attr.name,
                zone: first.attr.zone,
                osm,
                key,
            });
        }
        let interior = |s: &Street| {
            let p = s.line.pts();
            mm(if p.len() > 2 { p[1] } else { (p[0] + p[1]) * 0.5 })
        };
        streets.sort_by(|x, y| (x.a.min(x.b), x.a.max(x.b), interior(x)).cmp(&(y.a.min(y.b), y.a.max(y.b), interior(y))));
        let mut g = StreetGraph { nodes, streets, names, stars: Vec::new(), grid: SegGrid { cell: 16.0, map: HashMap::new() } };
        g.index();
        g
    }

    fn index(&mut self) {
        self.stars = vec![Vec::new(); self.nodes.len()];
        for (si, s) in self.streets.iter().enumerate() {
            for e in [End::A, End::B] {
                let h = s.heading(e, 3.0);
                self.stars[s.node(e) as usize].push((si as StreetId, e, h.y.atan2(h.x)));
            }
            let p = s.line.pts();
            for k in 0..p.len() - 1 {
                self.grid.insert(p[k], p[k + 1], (si as StreetId, k as u32));
            }
        }
        for st in &mut self.stars {
            st.sort_by(|a, b| a.2.total_cmp(&b.2).then(a.0.cmp(&b.0)).then(a.1.cmp(&b.1)));
        }
    }

    /// Arms around node `n`, counter-clockwise by heading (radians).
    pub fn star(&self, n: NodeId) -> &[(StreetId, End, f32)] {
        &self.stars[n as usize]
    }

    pub fn degree(&self, n: NodeId) -> usize {
        self.stars[n as usize].len()
    }

    pub fn street(&self, s: StreetId) -> &Street {
        &self.streets[s as usize]
    }

    pub fn width_at(&self, s: StreetId, t: f32) -> f32 {
        self.streets[s as usize].width_at(t)
    }

    /// Half the kerb-to-kerb width of an arm: the roadway, or the whole
    /// street on a shared surface.
    pub fn kerb_half(&self, s: StreetId, e: End) -> f32 {
        let st = self.street(s);
        if st.carriageway > 0.0 { st.carriageway * 0.5 } else { st.width_at(st.end_s(e)) * 0.5 }
    }

    /// Arc length from end `e` of street `s` where its junction ends: past
    /// the kerb corners with both neighbouring arms plus the kerb radius.
    pub fn mouth(&self, s: StreetId, e: End, lib: &ProfileLibrary) -> f32 {
        let st = self.street(s);
        let n = st.node(e);
        let star = self.star(n);
        let len = st.len();
        if star.len() < 2 {
            return 0.0;
        }
        let me = star.iter().position(|a| a.0 == s && a.1 == e).unwrap_or(0);
        let p0 = self.nodes[n as usize].p;
        let da = st.heading(e, 6.0);
        let ha = self.kerb_half(s, e);
        let mut t = 0.0f32;
        let mut sharp = star.len() > 2;
        for (k, side) in [((me + 1) % star.len(), 1.0f32), ((me + star.len() - 1) % star.len(), -1.0)] {
            let (os, oe, _) = star[k];
            if os == s && oe == e {
                continue;
            }
            let o = self.street(os);
            let db = o.heading(oe, 6.0);
            let hb = self.kerb_half(os, oe);
            if da.dot(db) > -0.866 {
                sharp = true;
            }
            // Kerb of A on the side facing B, and B's kerb facing A.
            let pa = p0 + left(da) * ha * side;
            let pb = p0 - left(db) * hb * side;
            let den = da.perp_dot(db);
            if den.abs() < 1e-4 {
                continue;
            }
            let ta = (pb - pa).perp_dot(db) / den;
            let tb = (pb - pa).perp_dot(da) / den;
            if ta > 0.0 && tb > 0.0 {
                t = t.max(ta.min(40.0));
            }
        }
        let r = if sharp { lib.get(st.profile).kerb_radius } else { 0.0 };
        let w = st.width_at(st.end_s(e)) * 0.5;
        (t + r).clamp(if sharp { w.min(len * 0.5) } else { 0.0 }, len * 0.5)
    }

    /// Streets passing within `r` of `p`: `(street, arc length, signed
    /// offset (+ left))` of the closest point on each.
    pub fn near(&self, p: Vec2, r: f32) -> Vec<(StreetId, f32, f32)> {
        let (lo, hi) = (self.grid.key(p - Vec2::splat(r)), self.grid.key(p + Vec2::splat(r)));
        let mut best: BTreeMap<StreetId, (f32, f32, f32)> = BTreeMap::new();
        for x in lo.0..=hi.0 {
            for z in lo.1..=hi.1 {
                let Some(l) = self.grid.map.get(&(x, z)) else { continue };
                for &(si, k) in l {
                    let line = &self.streets[si as usize].line;
                    let (a, b) = (line.pts()[k as usize], line.pts()[k as usize + 1]);
                    let e = b - a;
                    let l2 = e.length_squared();
                    let t = if l2 > 0.0 { ((p - a).dot(e) / l2).clamp(0.0, 1.0) } else { 0.0 };
                    let q = a + e * t;
                    let d = q.distance(p);
                    if d <= r && best.get(&si).is_none_or(|v| d < v.0) {
                        let side = e.perp_dot(p - a);
                        best.insert(si, (d, line.stations()[k as usize] + t * l2.sqrt(), if side >= 0.0 { d } else { -d }));
                    }
                }
            }
        }
        best.into_iter().map(|(s, (_, t, o))| (s, t, o)).collect()
    }

    /// The street space: a quad per segment with the widths `w(street,
    /// vertex)` at its ends, round join wedges at bends, and the convex
    /// hull of the arm ends at every junction. Dead ends stay square.
    pub fn corridor(&self, w: impl Fn(&Street, usize) -> f32) -> Region {
        let mut rings: Vec<Vec<Vec2>> = Vec::new();
        for s in &self.streets {
            rings.extend(super::line::stroke_rings(s.line.pts(), |k| w(s, k) * 0.5));
        }
        for (ni, star) in self.stars.iter().enumerate() {
            if star.len() < 2 {
                continue;
            }
            let c = self.nodes[ni].p;
            let mut hull = vec![c];
            for &(si, e, _) in star {
                let s = &self.streets[si as usize];
                let k = if e == End::A { 0 } else { s.line.pts().len() - 1 };
                let h = w(s, k) * 0.5;
                let d = s.heading(e, 0.01);
                hull.push(c + left(d) * h);
                hull.push(c - left(d) * h);
            }
            rings.push(convex_hull(hull));
        }
        Region::rings(rings.iter().map(|r| r.as_slice()))
    }

    /// Connected street sets.
    pub fn components(&self) -> Vec<Vec<StreetId>> {
        let mut comp = vec![usize::MAX; self.nodes.len()];
        let mut out = Vec::new();
        for start in 0..self.nodes.len() {
            if comp[start] != usize::MAX {
                continue;
            }
            let id = out.len();
            let mut list = Vec::new();
            let mut stack = vec![start];
            comp[start] = id;
            while let Some(n) = stack.pop() {
                for &(s, e, _) in &self.stars[n] {
                    let st = &self.streets[s as usize];
                    let o = st.node(if e == End::A { End::B } else { End::A }) as usize;
                    if e == End::A || st.a == st.b {
                        list.push(s);
                    }
                    if comp[o] == usize::MAX {
                        comp[o] = id;
                        stack.push(o);
                    }
                }
            }
            list.sort();
            list.dedup();
            if !list.is_empty() {
                out.push(list);
            }
        }
        out
    }

    /// Exact hash of the graph (ids, geometry, attributes).
    pub fn hash64(&self) -> u64 {
        let mut h = mix64(self.nodes.len() as u64);
        for n in &self.nodes {
            h = mix64(h ^ (super::region::to_mm(n.p.x) as u64) ^ ((super::region::to_mm(n.p.y) as u64) << 32));
        }
        for s in &self.streets {
            h = mix64(h ^ s.key ^ ((s.a as u64) << 32 | s.b as u64));
            for (p, w) in s.line.pts().iter().zip(&s.width) {
                h = mix64(h ^ (super::region::to_mm(p.x) as u64) ^ ((super::region::to_mm(p.y) as u64) << 21) ^ ((w * 1000.0) as u64) << 42);
            }
            h = mix64(h ^ (s.oneway as u8 as u64) ^ (s.profile.0 as u64) << 8 ^ (s.zone as u64) << 24);
        }
        h
    }
}

fn dist(a: P, b: P) -> f64 {
    (((a.0 - b.0) as f64).powi(2) + ((a.1 - b.1) as f64).powi(2)).sqrt()
}

/// Projection of `v` onto segment `ab` if within `snap` and away from both
/// ends (those merge as nodes instead).
fn snap_onto(v: P, a: P, b: P, snap: i64) -> Option<(f64, P)> {
    let (ex, ez) = ((b.0 - a.0) as f64, (b.1 - a.1) as f64);
    let l2 = ex * ex + ez * ez;
    if l2 <= 0.0 {
        return None;
    }
    let t = (((v.0 - a.0) as f64) * ex + ((v.1 - a.1) as f64) * ez) / l2;
    let l = l2.sqrt();
    let s = snap as f64;
    if t * l <= s || (1.0 - t) * l <= s {
        return None;
    }
    let q = ((a.0 as f64 + ex * t).round() as i64, (a.1 as f64 + ez * t).round() as i64);
    (dist(q, v) <= s).then_some((t, q))
}

struct Chain {
    a: usize,
    b: usize,
    edges: Vec<usize>,
}

/// Maximal chains of edges through nodes of degree 2 whose two edges share
/// attributes (oneway compared along the chain).
fn chain(edges: &[Edge], pts: &[P]) -> Vec<Chain> {
    let mut inc: HashMap<usize, Vec<usize>> = HashMap::new();
    for (i, e) in edges.iter().enumerate() {
        inc.entry(e.u).or_default().push(i);
        inc.entry(e.v).or_default().push(i);
    }
    let through = |n: usize| -> bool {
        let l = &inc[&n];
        if l.len() != 2 || l[0] == l[1] {
            return false;
        }
        let (e0, e1) = (&edges[l[0]], &edges[l[1]]);
        // Oneway along a path entering by e0 and leaving by e1.
        let ow0 = if e0.v == n { e0.oneway } else { -e0.oneway };
        let ow1 = if e1.u == n { e1.oneway } else { -e1.oneway };
        e0.attr == e1.attr && ow0 == ow1
    };
    let mut nodes: Vec<usize> = inc.keys().copied().collect();
    nodes.sort_by_key(|&n| pts[n]);
    let mut used = vec![false; edges.len()];
    let mut out = Vec::new();
    let walk = |start: usize, first: usize, used: &mut Vec<bool>| -> Chain {
        let mut list = vec![first];
        used[first] = true;
        let mut cur = if edges[first].u == start { edges[first].v } else { edges[first].u };
        while cur != start && through(cur) {
            let l = &inc[&cur];
            let next = if l[0] == *list.last().unwrap() { l[1] } else { l[0] };
            if used[next] {
                break;
            }
            used[next] = true;
            list.push(next);
            cur = if edges[next].u == cur { edges[next].v } else { edges[next].u };
        }
        Chain { a: start, b: cur, edges: list }
    };
    for &n in &nodes {
        if through(n) {
            continue;
        }
        let mut l = inc[&n].clone();
        l.sort_by_key(|&i| {
            let o = if edges[i].u == n { edges[i].v } else { edges[i].u };
            (pts[o], i)
        });
        for e in l {
            if !used[e] {
                out.push(walk(n, e, &mut used));
            }
        }
    }
    // Pure cycles: start at their smallest node.
    for &n in &nodes {
        let mut l = inc[&n].clone();
        l.sort();
        for e in l {
            if !used[e] {
                out.push(walk(n, e, &mut used));
            }
        }
    }
    out
}

/// Convex hull (monotone chain), counter-clockwise.
pub fn convex_hull(mut p: Vec<Vec2>) -> Vec<Vec2> {
    p.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    p.dedup();
    if p.len() < 3 {
        return p;
    }
    let cross = |o: Vec2, a: Vec2, b: Vec2| (a - o).perp_dot(b - o);
    let mut lower: Vec<Vec2> = Vec::new();
    for &q in &p {
        while lower.len() >= 2 && cross(lower[lower.len() - 2], lower[lower.len() - 1], q) <= 0.0 {
            lower.pop();
        }
        lower.push(q);
    }
    let mut upper: Vec<Vec2> = Vec::new();
    for &q in p.iter().rev() {
        while upper.len() >= 2 && cross(upper[upper.len() - 2], upper[upper.len() - 1], q) <= 0.0 {
            upper.pop();
        }
        upper.push(q);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(x: f32, z: f32) -> Vec2 {
        Vec2::new(x, z)
    }

    fn lib() -> ProfileLibrary {
        ProfileLibrary::builtin()
    }

    fn way(pts: &[(f32, f32)], p: &str) -> Way {
        Way::new(pts.iter().map(|q| v(q.0, q.1)).collect(), p)
    }

    #[test]
    fn planarize_splits_crossings() {
        let ws = [way(&[(-50.0, 0.0), (50.0, 0.0)], "madrid_calle"), way(&[(0.0, -50.0), (0.0, 50.0)], "madrid_calle")];
        let g = StreetGraph::build(&ws, &[], &lib(), 0.05);
        assert_eq!(g.streets.len(), 4);
        assert_eq!(g.nodes.len(), 5);
        let centre = g.nodes.iter().position(|n| n.p.length() < 1e-3).unwrap() as NodeId;
        assert_eq!(g.degree(centre), 4);
    }

    #[test]
    fn snaps_t_junctions() {
        let ws = [way(&[(-50.0, 0.0), (50.0, 0.0)], "madrid_calle"), way(&[(3.03, 0.04), (3.0, 40.0)], "madrid_calle")];
        let g = StreetGraph::build(&ws, &[], &lib(), 0.05);
        assert_eq!(g.streets.len(), 3, "{:?}", g.streets.iter().map(|s| s.line.pts().to_vec()).collect::<Vec<_>>());
        assert!(g.nodes.iter().any(|n| (n.p - v(3.0, 0.0)).length() < 0.06));
    }

    #[test]
    fn merges_collinear_duplicates() {
        // A ronda lying on a grid's outer street, partly overlapping.
        let ws = [way(&[(0.0, 0.0), (100.0, 0.0)], "madrid_calle"), way(&[(30.0, 0.0), (130.0, 0.0)], "ronda")];
        let g = StreetGraph::build(&ws, &[], &lib(), 0.05);
        // The madrid street up to the ronda, then the ronda all the way.
        assert_eq!(g.streets.len(), 2);
        let ronda = lib().id("ronda").unwrap();
        let overlap = g.streets.iter().find(|s| s.line.at(s.len() * 0.5).x > 40.0 && s.line.at(s.len() * 0.5).x < 90.0).unwrap();
        assert_eq!(overlap.profile, ronda);
    }

    #[test]
    fn keeps_dead_ends_and_prunes_stubs_and_small_components() {
        let ws = [
            way(&[(0.0, 0.0), (100.0, 0.0)], "callejuela"),
            way(&[(50.0, 0.0), (50.0, 30.0)], "callejon"),        // dead end, kept
            way(&[(20.0, 0.0), (20.0, 2.5)], "callejon"),         // stub, pruned
            way(&[(70.0, 0.0), (70.0, 3.0)], "callejon"),         // stub marked as a real end
            way(&[(500.0, 0.0), (520.0, 0.0)], "callejon"),       // separate small network
        ];
        let marks = [NodeMark { p: v(70.0, 3.0), signals: false, end: true }];
        let g = StreetGraph::build(&ws, &marks, &lib(), 0.05);
        let ends: Vec<Vec2> = g.nodes.iter().map(|n| n.p).collect();
        assert!(ends.iter().any(|p| (*p - v(50.0, 30.0)).length() < 1e-3));
        assert!(!ends.iter().any(|p| (*p - v(20.0, 2.5)).length() < 1e-3));
        assert!(ends.iter().any(|p| (*p - v(70.0, 3.0)).length() < 1e-3));
        assert!(!ends.iter().any(|p| p.x > 400.0));
        assert!(g.nodes.iter().any(|n| n.end));
    }

    #[test]
    fn star_is_ccw_and_mouth_covers_kerb_corner() {
        let ws = [way(&[(-50.0, 0.0), (50.0, 0.0)], "ensanche"), way(&[(0.0, 0.0), (35.0, 35.0)], "ensanche")];
        let g = StreetGraph::build(&ws, &[], &lib(), 0.05);
        let c = g.nodes.iter().position(|n| n.p.length() < 1e-3).unwrap() as NodeId;
        let st = g.star(c);
        assert_eq!(st.len(), 3);
        assert!(st.windows(2).all(|w| w[0].2 <= w[1].2));
        // The diagonal arm at 45°: the kerb corners lie at 5.5 / tan(22.5°) ≈ 13.3 m.
        let (s, e, _) = *st.iter().find(|a| (a.2 - std::f32::consts::FRAC_PI_4).abs() < 0.01).unwrap();
        let m = g.mouth(s, e, &lib());
        assert!(m > 13.0 + 6.0 - 0.5 && m < 13.3 + 6.0 + 0.5, "{m}");
    }

    #[test]
    fn canonical_ids_independent_of_input_order() {
        let mut ws = vec![
            way(&[(-50.0, 0.0), (50.0, 0.0)], "madrid_calle"),
            way(&[(0.0, -50.0), (0.0, 50.0)], "madrid_calle"),
            way(&[(-40.0, -30.0), (20.0, 10.0), (45.0, 40.0)], "callejuela"),
            way(&[(10.0, -60.0), (12.0, 0.0)], "callejon"),
        ];
        ws[0].oneway = 1;
        let a = StreetGraph::build(&ws, &[], &lib(), 0.05);
        ws.reverse();
        for w in ws.iter_mut().step_by(2) {
            w.line.reverse();
            w.oneway = -w.oneway;
        }
        let b = StreetGraph::build(&ws, &[], &lib(), 0.05);
        assert_eq!(a.hash64(), b.hash64());
        assert_eq!(a.streets, b.streets);
    }

    #[test]
    fn corridor_fills_junctions_and_keeps_dead_ends_square() {
        let ws = [way(&[(-50.0, 0.0), (50.0, 0.0)], "madrid_calle"), way(&[(0.0, 0.0), (0.0, 30.0)], "madrid_calle")];
        let g = StreetGraph::build(&ws, &[], &lib(), 0.05);
        let c = g.corridor(|s, i| s.width[i]);
        assert_eq!(c.len(), 1);
        // Area: 100 x 11 plus the stem 30 - 5.5 long.
        let expect = 100.0 * 11.0 + (30.0 - 5.5) * 11.0;
        assert!((c.area() - expect).abs() < 1.0, "{} vs {expect}", c.area());
        assert!(c.contains(v(5.4, 29.9)) && !c.contains(v(0.0, 30.1)));
    }
}
