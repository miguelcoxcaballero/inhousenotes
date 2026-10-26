//! Straight skeleton of a plan polygon with holes: every edge moves inward
//! at unit speed, vertices slide along their bisectors, and the traces
//! form the skeleton. Each input edge sweeps one *face*; lifted by its
//! distance from the edge, the faces are the planes of a hip roof of equal
//! pitch, exact for any simple polygon — concave street blocks, L and T
//! plans, plots with small OSM dents, courtyards.
//!
//! The simulation (Felkel–Obdržálek style, in `f64`) recomputes the next
//! event each step instead of keeping a queue, which keeps simultaneous
//! events (rectangles, equal-width wings) simple and robust; building
//! plans have at most a few hundred vertices.

use forja_math::Vec2;
use forja_math::glam::DVec2;

use super::poly::{self, Shape};

/// One roof face: the region swept by input edge `edge` (ring, index),
/// as plan points with their distance from the edge (`t`).
#[derive(Clone, Debug)]
pub struct Face {
    pub ring: usize,
    pub edge: usize,
    pub pts: Vec<Vec2>,
    pub t: Vec<f32>,
}

/// A skeleton arc (ridge, hip or valley) with its two end distances and
/// the turn between the two faces it separates (degrees in plan: ~0 for
/// faces of nearly collinear edges, 90 for a hip of a square corner).
#[derive(Clone, Debug)]
pub struct Arc {
    pub a: Vec2,
    pub b: Vec2,
    pub ta: f32,
    pub tb: f32,
    pub turn: f32,
}

/// Faces and interior arcs of the straight skeleton.
#[derive(Clone, Debug)]
pub struct Skeleton {
    pub faces: Vec<Face>,
    pub arcs: Vec<Arc>,
    /// Largest distance reached (the ridge height over unit pitch).
    pub max_t: f32,
}

struct Edge {
    d: DVec2,
    n: DVec2,
    off: f64,
    ring: usize,
    index: usize,
    a_node: usize,
    b_node: usize,
}

#[derive(Clone, Copy)]
struct Vtx {
    c: DVec2,
    v: DVec2,
    el: usize,
    er: usize,
    prev: usize,
    next: usize,
    alive: bool,
    node: usize,
    reflex: bool,
}

impl Vtx {
    fn at(&self, t: f64) -> DVec2 {
        self.c + self.v * t
    }
}

struct Sim {
    edges: Vec<Edge>,
    vs: Vec<Vtx>,
    nodes: Vec<(DVec2, f64)>,
    /// Boundary segments of each face (node pairs).
    face_segs: Vec<Vec<(usize, usize)>>,
    /// Arcs with the faces on their sides.
    arcs: Vec<(usize, usize, usize, usize)>,
    tol: f64,
}

fn cross(a: DVec2, b: DVec2) -> f64 {
    a.x * b.y - a.y * b.x
}

impl Sim {
    fn node(&mut self, p: DVec2, t: f64) -> usize {
        if let Some(i) = self.nodes.iter().position(|(q, s)| (*q - p).length() < self.tol && (s - t).abs() < self.tol * 10.0) {
            return i;
        }
        self.nodes.push((p, t));
        self.nodes.len() - 1
    }

    fn velocity(&self, el: usize, er: usize) -> DVec2 {
        let (nl, nr) = (self.edges[el].n, self.edges[er].n);
        let det = cross(nl, nr);
        if det.abs() < 1e-9 {
            // Collinear edges move together; antiparallel ones have met.
            return if nl.dot(nr) > 0.0 { nl } else { DVec2::ZERO };
        }
        DVec2::new((nr.y - nl.y) / det, (nl.x - nr.x) / det)
    }

    fn add_vertex(&mut self, p: DVec2, t: f64, el: usize, er: usize, node: usize) -> usize {
        let v = self.velocity(el, er);
        let reflex = cross(self.edges[el].d, self.edges[er].d) < -1e-12;
        self.vs.push(Vtx { c: p - v * t, v, el, er, prev: usize::MAX, next: usize::MAX, alive: true, node, reflex });
        self.vs.len() - 1
    }

    fn seg(&mut self, a: usize, b: usize, faces: &[usize]) {
        if a == b {
            return;
        }
        for &f in faces {
            self.face_segs[f].push((a, b));
        }
        match self.arcs.iter_mut().find(|(x, y, _, _)| (*x == a && *y == b) || (*x == b && *y == a)) {
            // A ridge recorded once per side: complete its second face.
            Some(arc) => {
                if arc.2 == arc.3 && faces.first().is_some_and(|f| *f != arc.2) {
                    arc.3 = faces[0];
                }
            }
            None => {
                let f0 = faces.first().copied().unwrap_or(0);
                let f1 = faces.get(1).copied().unwrap_or(f0);
                self.arcs.push((a, b, f0, f1));
            }
        }
    }

    fn lav(&self, start: usize) -> Vec<usize> {
        let mut out = vec![start];
        let mut k = self.vs[start].next;
        while k != start && out.len() <= self.vs.len() {
            out.push(k);
            k = self.vs[k].next;
        }
        out
    }

    /// Ends a wavefront loop that has shrunk to a point or a line: every
    /// vertex runs to where it is now and consecutive points are joined.
    fn collapse(&mut self, start: usize, t: f64) {
        let ids = self.lav(start);
        let mut groups: Vec<(usize, Vec<usize>)> = Vec::new();
        for &i in &ids {
            let p = self.vs[i].at(t);
            let k = self.node(p, t);
            match groups.last_mut() {
                Some((g, members)) if *g == k => members.push(i),
                _ => groups.push((k, vec![i])),
            }
        }
        if groups.len() > 1 && groups[0].0 == groups[groups.len() - 1].0 {
            let last = groups.pop().unwrap();
            groups[0].1.splice(0..0, last.1);
        }
        for (k, members) in &groups {
            for &i in members {
                let (n0, el, er) = (self.vs[i].node, self.vs[i].el, self.vs[i].er);
                self.seg(n0, *k, &[el, er]);
            }
        }
        let g = groups.len();
        if g > 1 {
            for j in 0..g {
                let (ka, ref ma) = groups[j];
                let kb = groups[(j + 1) % g].0;
                if g == 2 && j == 1 {
                    // Back along the same ridge: the other face.
                    let e = self.vs[*ma.last().unwrap()].er;
                    self.seg(ka, kb, &[e]);
                    continue;
                }
                let e = self.vs[*ma.last().unwrap()].er;
                self.seg(ka, kb, &[e]);
            }
        }
        for i in ids {
            self.vs[i].alive = false;
        }
    }

    /// Signed area and perimeter of the loop through `start` at `t`.
    fn lav_area(&self, start: usize, t: f64) -> (f64, f64) {
        let ids = self.lav(start);
        let pts: Vec<DVec2> = ids.iter().map(|&i| self.vs[i].at(t)).collect();
        let n = pts.len();
        let (mut a, mut per) = (0.0, 0.0);
        for i in 0..n {
            a += cross(pts[i], pts[(i + 1) % n]);
            per += (pts[(i + 1) % n] - pts[i]).length();
        }
        (a * 0.5, per)
    }

    /// Collapses the loop through `v` when it is degenerate.
    fn settle(&mut self, v: usize, t: f64) {
        if !self.vs[v].alive {
            return;
        }
        let n = self.lav(v).len();
        let (a, per) = self.lav_area(v, t);
        if n <= 2 || a.abs() < self.tol * 10.0 * per {
            // Two vertices left, or the loop has gone flat: a ridge.
            self.collapse(v, t);
        }
    }

    fn run(&mut self) -> bool {
        let mut now = 0.0f64;
        let limit = self.vs.len() * 8 + 64;
        for _ in 0..limit {
            let alive: Vec<usize> = (0..self.vs.len()).filter(|&i| self.vs[i].alive).collect();
            if alive.is_empty() {
                return true;
            }
            // Earliest event: (time, kind, a, b) — kind 0 edge, 1 split.
            let mut best: Option<(f64, u8, usize, usize)> = None;
            let eps = self.tol;
            for &i in &alive {
                let vi = self.vs[i];
                let j = vi.next;
                let vj = self.vs[j];
                let d = self.edges[vi.er].d;
                let den = d.dot(vi.v - vj.v);
                if den > 1e-12 {
                    let t = d.dot(vj.c - vi.c) / den;
                    if t >= now - eps && best.is_none_or(|b| t < b.0 - eps * 0.1 || (t < b.0 + eps * 0.1 && b.1 == 1)) {
                        best = Some((t.max(now), 0, i, j));
                    }
                }
            }
            for &i in &alive {
                let vi = self.vs[i];
                if !vi.reflex {
                    continue;
                }
                for &w in &alive {
                    let vw = self.vs[w];
                    let f = vw.er;
                    if f == vi.el || f == vi.er || w == i || vw.next == i {
                        continue;
                    }
                    let e = &self.edges[f];
                    let den = e.n.dot(vi.v) - 1.0;
                    if den > -1e-12 {
                        continue;
                    }
                    let t = (e.off - e.n.dot(vi.c)) / den;
                    if t < now - eps || best.is_some_and(|b| t >= b.0 - eps * 0.1) {
                        continue;
                    }
                    let p = vi.at(t);
                    let s0 = e.d.dot(vw.at(t));
                    let s1 = e.d.dot(self.vs[vw.next].at(t));
                    let sp = e.d.dot(p);
                    if sp >= s0 - eps && sp <= s1 + eps {
                        best = Some((t.max(now), 1, i, w));
                    }
                }
            }
            let Some((t, kind, a, b)) = best else { return false };
            now = t;
            if kind == 0 {
                self.edge_event(a, b, t);
            } else {
                self.split_event(a, b, t);
            }
        }
        false
    }

    fn edge_event(&mut self, a: usize, b: usize, t: f64) {
        let (va, vb) = (self.vs[a], self.vs[b]);
        let p = (va.at(t) + vb.at(t)) * 0.5;
        let k = self.node(p, t);
        self.seg(va.node, k, &[va.el, va.er]);
        self.seg(vb.node, k, &[vb.el, vb.er]);
        let n = self.lav(a).len();
        if n <= 3 {
            self.vs[a].node = k;
            self.vs[b].node = k;
            self.collapse(a, t);
            return;
        }
        let w = self.add_vertex(p, t, va.el, vb.er, k);
        let (pv, nx) = (va.prev, vb.next);
        self.vs[w].prev = pv;
        self.vs[w].next = nx;
        self.vs[pv].next = w;
        self.vs[nx].prev = w;
        self.vs[a].alive = false;
        self.vs[b].alive = false;
        self.settle(w, t);
    }

    fn split_event(&mut self, v: usize, w: usize, t: f64) {
        let vv = self.vs[v];
        let x = self.vs[w].next;
        let f = self.vs[w].er;
        let p = vv.at(t);
        let k = self.node(p, t);
        self.seg(vv.node, k, &[vv.el, vv.er]);
        let v1 = self.add_vertex(p, t, vv.el, f, k);
        let v2 = self.add_vertex(p, t, f, vv.er, k);
        let (vp, vn) = (vv.prev, vv.next);
        // vp -> v1 -> x  and  w -> v2 -> vn.
        self.vs[v1].prev = vp;
        self.vs[v1].next = x;
        self.vs[vp].next = v1;
        self.vs[x].prev = v1;
        self.vs[v2].prev = w;
        self.vs[v2].next = vn;
        self.vs[w].next = v2;
        self.vs[vn].prev = v2;
        self.vs[v].alive = false;
        self.settle(v1, t);
        self.settle(v2, t);
    }
}

/// Straight skeleton of a normalised shape (outer CCW, holes CW).
pub fn skeleton(shape: &Shape) -> Option<Skeleton> {
    let shape = shape.normalized()?;
    let c = poly::centroid(&shape.outer);
    let (mn, mx) = shape.bbox();
    let scale = ((mx - mn).max_element() as f64).max(1e-3);
    let tol = scale * 1e-7 + 1e-7;
    let mut sim = Sim { edges: Vec::new(), vs: Vec::new(), nodes: Vec::new(), face_segs: Vec::new(), arcs: Vec::new(), tol };
    let rings: Vec<Vec<DVec2>> = shape.rings().map(|(r, _)| r.iter().map(|p| DVec2::new((p.x - c.x) as f64, (p.y - c.y) as f64)).collect()).collect();
    let total: usize = rings.iter().map(|r| r.len()).sum();
    if total > 2000 {
        return None;
    }
    // Edges and their faces.
    let mut ring_start = Vec::new();
    for (r, ring) in rings.iter().enumerate() {
        ring_start.push(sim.edges.len());
        let n = ring.len();
        for i in 0..n {
            let (a, b) = (ring[i], ring[(i + 1) % n]);
            let d = (b - a).normalize_or_zero();
            if d == DVec2::ZERO {
                return None;
            }
            // Interior on the left (outer CCW, holes CW).
            let nrm = DVec2::new(-d.y, d.x);
            let an = sim.node(a, 0.0);
            let bn = sim.node(b, 0.0);
            sim.edges.push(Edge { d, n: nrm, off: nrm.dot(a), ring: r, index: i, a_node: an, b_node: bn });
            sim.face_segs.push(Vec::new());
        }
    }
    for (r, ring) in rings.iter().enumerate() {
        let n = ring.len();
        let base = sim.vs.len();
        for i in 0..n {
            let el = ring_start[r] + (i + n - 1) % n;
            let er = ring_start[r] + i;
            let node = sim.edges[er].a_node;
            sim.add_vertex(ring[i], 0.0, el, er, node);
        }
        for i in 0..n {
            sim.vs[base + i].prev = base + (i + n - 1) % n;
            sim.vs[base + i].next = base + (i + 1) % n;
        }
    }
    if !sim.run() {
        return None;
    }
    // Chain each face: eave b -> ... -> a.
    let mut faces = Vec::with_capacity(sim.edges.len());
    for (ei, e) in sim.edges.iter().enumerate() {
        let segs = &sim.face_segs[ei];
        let mut chain = vec![e.a_node, e.b_node];
        let mut used = vec![false; segs.len()];
        let mut cur = e.b_node;
        let mut prev = e.a_node;
        let mut ok = false;
        for _ in 0..segs.len() + 1 {
            let next = segs.iter().enumerate().find(|(k, (x, y))| !used[*k] && ((*x == cur && *y != prev) || (*y == cur && *x != prev))).map(|(k, (x, y))| (k, if *x == cur { *y } else { *x }));
            let Some((k, nx)) = next.or_else(|| segs.iter().enumerate().find(|(k, (x, y))| !used[*k] && (*x == cur || *y == cur)).map(|(k, (x, y))| (k, if *x == cur { *y } else { *x }))) else { break };
            used[k] = true;
            if nx == e.a_node {
                ok = true;
                break;
            }
            chain.push(nx);
            prev = cur;
            cur = nx;
        }
        if !ok {
            return None;
        }
        let pts: Vec<Vec2> = chain.iter().map(|&k| Vec2::new(sim.nodes[k].0.x as f32 + c.x, sim.nodes[k].0.y as f32 + c.y)).collect();
        let t: Vec<f32> = chain.iter().map(|&k| sim.nodes[k].1 as f32).collect();
        faces.push(Face { ring: e.ring, edge: e.index, pts, t });
    }
    // Faces must tile the plan.
    let sum: f32 = faces.iter().map(|f| poly::area(&f.pts).abs()).sum();
    let area = shape.area();
    if (sum - area).abs() > area * 0.01 + 1e-3 {
        return None;
    }
    let boundary: Vec<usize> = sim.edges.iter().flat_map(|e| [e.a_node, e.b_node]).collect();
    let arcs = sim
        .arcs
        .iter()
        .filter(|(a, b, _, _)| !(boundary.contains(a) && boundary.contains(b) && sim.nodes[*a].1 == 0.0 && sim.nodes[*b].1 == 0.0))
        .map(|&(a, b, f0, f1)| Arc {
            a: Vec2::new(sim.nodes[a].0.x as f32 + c.x, sim.nodes[a].0.y as f32 + c.y),
            b: Vec2::new(sim.nodes[b].0.x as f32 + c.x, sim.nodes[b].0.y as f32 + c.y),
            ta: sim.nodes[a].1 as f32,
            tb: sim.nodes[b].1 as f32,
            turn: sim.edges[f0].d.dot(sim.edges[f1].d).clamp(-1.0, 1.0).acos().to_degrees() as f32,
        })
        .collect();
    let max_t = sim.nodes.iter().map(|n| n.1).fold(0.0, f64::max) as f32;
    Some(Skeleton { faces, arcs, max_t })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(s: &Shape, faces: usize) -> Skeleton {
        let sk = skeleton(s).unwrap_or_else(|| panic!("no skeleton for {s:?}"));
        assert_eq!(sk.faces.len(), faces);
        let sum: f32 = sk.faces.iter().map(|f| poly::area(&f.pts).abs()).sum();
        assert!((sum - s.area()).abs() < s.area() * 0.005, "{sum} vs {}", s.area());
        // Every face point is at its distance from the face's edge line.
        for f in &sk.faces {
            let ring = if f.ring == 0 { &s.outer } else { &s.holes[f.ring - 1] };
            let n = ring.len();
            let (a, b) = (ring[f.edge], ring[(f.edge + 1) % n]);
            let d = (b - a).normalize();
            let inward = Vec2::new(-d.y, d.x);
            for (p, t) in f.pts.iter().zip(&f.t) {
                assert!(((*p - a).dot(inward) - t).abs() < 1e-3, "face {} point {p} t {t}", f.edge);
            }
        }
        sk
    }

    #[test]
    fn rectangle_has_a_ridge() {
        let s = Shape::new(poly::rect(Vec2::ZERO, Vec2::new(20.0, 10.0), 0.0)).normalized().unwrap();
        let sk = check(&s, 4);
        assert!((sk.max_t - 5.0).abs() < 1e-3);
        // Ridge of length 10 among the arcs.
        assert!(sk.arcs.iter().any(|a| ((a.a - a.b).length() - 10.0).abs() < 1e-3), "{:?}", sk.arcs);
    }

    #[test]
    fn square_and_triangle() {
        let s = Shape::new(poly::rect(Vec2::ZERO, Vec2::new(10.0, 10.0), 0.0)).normalized().unwrap();
        check(&s, 4);
        let t = Shape::new(vec![Vec2::ZERO, Vec2::new(12.0, 0.0), Vec2::new(0.0, 9.0)]).normalized().unwrap();
        let sk = check(&t, 3);
        assert!((sk.max_t - 3.0).abs() < 1e-3);
    }

    #[test]
    fn concave_plans() {
        // L with equal and unequal wings.
        for w2 in [6.0, 8.0] {
            let l = Shape::new(vec![Vec2::ZERO, Vec2::new(20.0, 0.0), Vec2::new(20.0, 6.0), Vec2::new(w2, 6.0), Vec2::new(w2, 20.0), Vec2::new(0.0, 20.0)]).normalized().unwrap();
            check(&l, 6);
        }
        // T plan, a U plan and a dented rectangle (OSM noise).
        let t = Shape::new(vec![Vec2::ZERO, Vec2::new(30.0, 0.0), Vec2::new(30.0, 8.0), Vec2::new(19.0, 8.0), Vec2::new(19.0, 25.0), Vec2::new(11.0, 25.0), Vec2::new(11.0, 8.0), Vec2::new(0.0, 8.0)]).normalized().unwrap();
        check(&t, 8);
        let u = Shape::new(vec![Vec2::ZERO, Vec2::new(24.0, 0.0), Vec2::new(24.0, 20.0), Vec2::new(17.0, 20.0), Vec2::new(17.0, 7.0), Vec2::new(7.0, 7.0), Vec2::new(7.0, 20.0), Vec2::new(0.0, 20.0)]).normalized().unwrap();
        check(&u, 8);
        let dent = Shape::new(vec![Vec2::ZERO, Vec2::new(5.0, 0.0), Vec2::new(5.1, 0.14), Vec2::new(5.3, 0.0), Vec2::new(12.0, 0.0), Vec2::new(12.0, 9.0), Vec2::new(0.0, 9.0)]).normalized().unwrap();
        check(&dent, 7);
    }

    #[test]
    fn curved_block_and_courtyard() {
        // A block bending along a curved street.
        let mut ring: Vec<Vec2> = (0..=10).map(|k| Vec2::from_angle(-0.6 + k as f32 * 0.12) * 40.0).collect();
        ring.extend((0..=10).rev().map(|k| Vec2::from_angle(-0.6 + k as f32 * 0.12) * 28.0));
        let s = Shape::new(ring).normalized().unwrap();
        check(&s, 22);
        // Courtyard block.
        let c = Shape { outer: poly::rect(Vec2::ZERO, Vec2::new(30.0, 24.0), 0.0), holes: vec![poly::rect(Vec2::new(1.0, 0.5), Vec2::new(14.0, 10.0), 0.0)] }.normalized().unwrap();
        check(&c, 8);
    }

    #[test]
    fn regular_polygons() {
        for n in [5, 8, 24] {
            let s = Shape::new(poly::regular(Vec2::new(3.0, -2.0), 6.0, n, 10.0)).normalized().unwrap();
            check(&s, n);
        }
    }
}
