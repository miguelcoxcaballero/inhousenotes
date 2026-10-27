//! Walls with openings of any shape. A facade is a planar region in
//! (u along the wall, v = height) coordinates; openings are cut as holes,
//! or as notches when they reach an edge (doors at the ground, arches
//! under an arcade wall, windows spanning two facets). Reveals follow each
//! opening's outline through recessed orders or splays to the infill
//! plane, so arched, round and lobed openings get real depth.

use std::sync::Arc;

use forja_math::{Mat4, Vec2, Vec3};

use super::mesh::{self, Geo, MatId, Plane};
use super::poly::{self, Shape};
use super::resolve::EdgeRef;
use super::runs::{self, Bend};

/// A planar wall region.
#[derive(Clone, Debug)]
pub struct Facade {
    /// Origin at the left end (seen from outside) at y = 0; u along the
    /// wall, v = world up, n = outward.
    pub plane: Plane,
    pub len: f32,
    pub v0: f32,
    pub v1: f32,
    /// Texture u offset, for continuity around a building.
    pub uv_off: f32,
    pub mat: MatId,
    /// Non-rectangular convex outline (gables), in (u, v).
    pub outline: Option<Vec<Vec2>>,
    /// Period of a closed run (round towers): u wraps at this length.
    pub wrap: Option<f32>,
}

impl Facade {
    pub fn rect(&self) -> Vec<Vec2> {
        match &self.outline {
            Some(o) => o.clone(),
            None => vec![Vec2::new(0.0, self.v0), Vec2::new(self.len, self.v0), Vec2::new(self.len, self.v1), Vec2::new(0.0, self.v1)],
        }
    }

    pub fn inside(&self, p: Vec2, eps: f32) -> bool {
        match &self.outline {
            Some(o) => {
                let n = o.len();
                (0..n).all(|i| {
                    let (a, b) = (o[i], o[(i + 1) % n]);
                    (b - a).perp_dot(p - a) / (b - a).length().max(1e-6) > eps
                })
            }
            None => p.x > eps && p.x < self.len - eps && p.y > self.v0 + eps && p.y < self.v1 - eps,
        }
    }
}

/// One opening cut through a facade.
#[derive(Clone, Debug)]
pub struct Cut {
    /// Rings from the wall face inward with their depths (positive
    /// inward). Consecutive rings with equal point counts are joined.
    pub rings: Vec<(Vec<Vec2>, f32)>,
    pub reveal_mat: MatId,
    /// Material of the plane closing the recess at the last depth.
    pub back: Option<MatId>,
    pub round: bool,
}

impl Cut {
    pub fn simple(outline: Vec<Vec2>, depth: f32, reveal_mat: MatId, back: Option<MatId>, round: bool) -> Self {
        Cut { rings: vec![(outline.clone(), 0.0), (outline, depth)], reveal_mat, back, round }
    }

    pub fn outer(&self) -> &[Vec2] {
        &self.rings[0].0
    }

    pub fn inner(&self) -> &[Vec2] {
        &self.rings[self.rings.len() - 1].0
    }

    pub fn depth(&self) -> f32 {
        self.rings[self.rings.len() - 1].1
    }
}

const EPS: f32 = 2e-3;

fn clip_to(f: &Facade, ring: &[Vec2]) -> Vec<Vec2> {
    let r = f.rect();
    let ccw = if poly::area(&r) >= 0.0 { r } else { r.into_iter().rev().collect() };
    poly::clip_convex(ring, &ccw)
}

/// Which rectangle side a point lies on (0 bottom, 1 right, 2 top, 3 left).
fn side_of(f: &Facade, p: Vec2) -> Option<usize> {
    if (p.y - f.v0).abs() < EPS {
        Some(0)
    } else if (p.x - f.len).abs() < EPS {
        Some(1)
    } else if (p.y - f.v1).abs() < EPS {
        Some(2)
    } else if p.x.abs() < EPS {
        Some(3)
    } else {
        None
    }
}

fn side_param(f: &Facade, side: usize, p: Vec2) -> f32 {
    match side {
        0 => p.x,
        1 => p.y - f.v0,
        2 => f.len - p.x,
        _ => f.v1 - p.y,
    }
}

/// Wall face polygon: the facade outline minus the cuts (holes and
/// notches). Returns the shape and warnings.
pub fn wall_shape(f: &Facade, cuts: &[Vec<Vec2>]) -> (Shape, Vec<String>) {
    let mut warnings = Vec::new();
    let mut holes = Vec::new();
    // (side, t_start, t_end, path)
    let mut notches: Vec<(usize, f32, f32, Vec<Vec2>)> = Vec::new();
    for ring in cuts {
        let mut c = clip_to(f, ring);
        c = poly::normalize(&c, true).unwrap_or_default();
        if c.len() < 3 {
            continue;
        }
        let n = c.len();
        let touching: Vec<bool> = c.iter().map(|p| !f.inside(*p, EPS)).collect();
        if !touching.iter().any(|t| *t) {
            holes.push(c);
            continue;
        }
        if f.outline.is_some() {
            // Gables: keep openings strictly inside.
            let ctr = poly::centroid(&c);
            let mut s = c.clone();
            for _ in 0..12 {
                if s.iter().all(|p| f.inside(*p, EPS * 2.0)) {
                    break;
                }
                s = s.iter().map(|p| ctr + (*p - ctr) * 0.94).collect();
            }
            if s.iter().all(|p| f.inside(*p, EPS * 2.0)) {
                warnings.push("an opening reaching past a gable edge was shrunk to fit".into());
                holes.push(s);
            } else {
                warnings.push("an opening outside a gable wall was dropped".into());
            }
            continue;
        }
        // Boundary segments running along a side in the side's direction.
        let mut segs = Vec::new();
        for i in 0..n {
            let (a, b) = (c[i], c[(i + 1) % n]);
            if let (Some(sa), Some(sb)) = (side_of(f, a), side_of(f, b))
                && sa == sb
                && side_param(f, sa, b) > side_param(f, sa, a) + 1e-4
            {
                segs.push((i, sa));
            }
        }
        match segs.len() {
            0 => {
                // Touches at a point: pull those vertices inside.
                let ctr = poly::centroid(&c);
                holes.push(c.iter().map(|p| if f.inside(*p, EPS) { *p } else { *p + (ctr - *p).normalize_or(Vec2::ZERO) * 0.01 }).collect());
            }
            1 => {
                let (i, side) = segs[0];
                // Path from c[i] backwards around the hole to c[i + 1].
                let mut path = Vec::with_capacity(n);
                let mut k = i;
                loop {
                    path.push(c[k]);
                    if k == (i + 1) % n {
                        break;
                    }
                    k = (k + n - 1) % n;
                }
                let t0 = side_param(f, side, c[i]);
                let t1 = side_param(f, side, c[(i + 1) % n]);
                notches.push((side, t0, t1, path));
            }
            _ => warnings.push("an opening spanning a facade corner was skipped".into()),
        }
    }
    if f.outline.is_some() {
        return (Shape { outer: f.rect(), holes }, warnings);
    }
    notches.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
    let corners = [Vec2::new(0.0, f.v0), Vec2::new(f.len, f.v0), Vec2::new(f.len, f.v1), Vec2::new(0.0, f.v1)];
    let mut outer = Vec::new();
    for (side, corner) in corners.iter().enumerate() {
        outer.push(*corner);
        let mut last_end = -1.0;
        for (s, t0, t1, path) in &notches {
            if *s != side {
                continue;
            }
            if *t0 < last_end - 1e-4 {
                warnings.push("overlapping openings: one was skipped".into());
                continue;
            }
            outer.extend(path.iter().copied());
            last_end = *t1;
        }
    }
    let outer = poly::normalize(&outer, true).unwrap_or_else(|| f.rect());
    (Shape { outer, holes }, warnings)
}

/// Copies of cuts crossing the seam of a closed run, shifted by one
/// period, so both ends of the developed wall get their part.
fn wrapped(f: &Facade, cuts: &[Cut]) -> Vec<Cut> {
    let Some(l) = f.wrap else { return cuts.to_vec() };
    let mut out = cuts.to_vec();
    for c in cuts {
        let lo = c.rings.iter().flat_map(|(r, _)| r.iter().map(|p| p.x)).fold(f32::MAX, f32::min);
        let hi = c.rings.iter().flat_map(|(r, _)| r.iter().map(|p| p.x)).fold(f32::MIN, f32::max);
        for shift in [l, -l] {
            if (shift > 0.0 && lo < 0.0) || (shift < 0.0 && hi > l) {
                let mut k = c.clone();
                for (r, _) in &mut k.rings {
                    for p in r.iter_mut() {
                        p.x += shift;
                    }
                }
                out.push(k);
            }
        }
    }
    out
}

/// Builds the wall face, reveals and recess planes of a facade.
pub fn build(geo: &mut Geo, f: &Facade, cuts: &[Cut]) -> Vec<String> {
    let cuts = &wrapped(f, cuts)[..];
    let outers: Vec<Vec<Vec2>> = cuts.iter().map(|c| c.outer().to_vec()).collect();
    let (shape, warnings) = wall_shape(f, &outers);
    mesh::planar_shape(geo.get(f.mat), &shape, &f.plane, false, Vec2::new(f.uv_off, 0.0));
    for c in cuts {
        reveal(geo, f, c);
        if let Some(bm) = c.back {
            let inner = clip_to(f, c.inner());
            if let Some(r) = poly::normalize(&inner, true) {
                let pl = Plane { o: f.plane.o - f.plane.n * c.depth(), ..f.plane };
                mesh::planar_shape(geo.get(bm), &Shape::new(r), &pl, false, Vec2::new(f.uv_off, 0.0));
            }
        }
    }
    warnings
}

/// Reveal surfaces of one cut (the tube from the wall face to the infill).
pub fn reveal(geo: &mut Geo, f: &Facade, c: &Cut) {
    let p = geo.get(c.reveal_mat);
    let pl = &f.plane;
    for w in c.rings.windows(2) {
        let ((ra, da), (rb, db)) = (&w[0], &w[1]);
        if ra.len() != rb.len() || ra.len() < 2 {
            continue;
        }
        let n = ra.len();
        let segs = if c.round { n } else { n };
        let mut acc = 0.0;
        for i in 0..segs {
            let j = (i + 1) % n;
            let mid = (ra[i] + ra[j]) * 0.5;
            let mid_b = (rb[i] + rb[j]) * 0.5;
            let seg_len = (ra[j] - ra[i]).length().max((rb[j] - rb[i]).length());
            if seg_len < 1e-5 {
                continue;
            }
            // Skip parts on or beyond the facade border (door thresholds,
            // notches into neighbouring facets).
            if !f.inside(mid, EPS * 0.5) && !f.inside(mid_b, EPS * 0.5) {
                acc += seg_len;
                continue;
            }
            let q = [pl.at_d(ra[i], -da), pl.at_d(ra[j], -da), pl.at_d(rb[j], -db), pl.at_d(rb[i], -db)];
            // Face the opening's axis (inward of a CCW outline) and outward.
            let d = ra[j] - ra[i];
            let inward2 = Vec2::new(-d.y, d.x).normalize_or(Vec2::ZERO);
            let want = pl.u * inward2.x + pl.v * inward2.y + pl.n * 0.5;
            let uv = [Vec2::new(acc, -da), Vec2::new(acc + seg_len, -da), Vec2::new(acc + seg_len, -db), Vec2::new(acc, -db)];
            mesh::quad_facing(p, q, uv, want);
            acc += seg_len;
        }
    }
}

/// Scales an outline about its bottom centre so its width grows by `dw`
/// and its height by `dh` (bottom fixed).
pub fn grow(outline: &[Vec2], center_u: f32, bottom: f32, w: f32, h: f32, dw: f32, dh: f32) -> Vec<Vec2> {
    let sx = (w + dw) / w.max(1e-4);
    let sy = (h + dh) / h.max(1e-4);
    outline.iter().map(|p| Vec2::new(center_u + (p.x - center_u) * sx, bottom + (p.y - bottom) * sy)).collect()
}

/// A facade with its place in the plan: one straight edge (gables,
/// arcades: `bend = None`, the facade plane is in world space) or a run of
/// edges developed flat (`bend`, the facade plane is the canonical flat
/// frame and geometry is bent onto the plan by [`Run::place`]).
#[derive(Clone, Debug)]
pub struct Run {
    pub facade: Facade,
    pub bend: Option<Arc<Bend>>,
    /// Plan edges of the run with their `[u0, u1]` range on the facade.
    pub edges: Vec<(EdgeRef, f32, f32)>,
}

impl Run {
    /// A single facade in world space.
    pub fn plain(facade: Facade, e: EdgeRef) -> Run {
        let len = facade.len;
        Run { facade, bend: None, edges: vec![(e, 0.0, len)] }
    }

    /// World point of facade coordinates `(u, v)` at `d` in front.
    pub fn world(&self, u: f32, v: f32, d: f32) -> Vec3 {
        match &self.bend {
            Some(b) => {
                let (p, n) = b.frame(u);
                Vec3::new(p.x + n.x * d, v, p.y + n.y * d)
            }
            None => self.facade.plane.at_d(Vec2::new(u, v), d),
        }
    }

    /// Outward plan normal at `u` (the facet's own, not smoothed).
    pub fn normal(&self, u: f32) -> Vec2 {
        match &self.bend {
            Some(b) => b.seg_normal(u),
            None => Vec2::new(self.facade.plane.n.x, self.facade.plane.n.z).normalize_or(Vec2::Y),
        }
    }

    /// Smoothed outward plan normal at `u`.
    pub fn smooth_normal(&self, u: f32) -> Vec2 {
        match &self.bend {
            Some(b) => b.frame(u).1,
            None => self.normal(u),
        }
    }

    /// Facade u of a point at parameter `t` along plan edge `e`.
    pub fn u_of(&self, e: EdgeRef, t: f32) -> Option<f32> {
        self.edges.iter().find(|(x, _, _)| *x == e).map(|(_, u0, u1)| u1 - (u1 - u0) * t)
    }

    /// Moves flat facade geometry into the world.
    pub fn place(&self, flat: &Geo, dst: &mut Geo) {
        match &self.bend {
            Some(b) => b.warp_geo(flat, dst),
            None => dst.append(flat, &Mat4::IDENTITY),
        }
    }

    pub fn closed(&self) -> bool {
        self.bend.as_ref().is_some_and(|b| b.closed)
    }

    /// Which plan edges this facade is, for messages: "edge 3",
    /// "edges 0..10", "courtyard 1 edge 2".
    pub fn label(&self) -> String {
        let mut ids: Vec<usize> = self.edges.iter().map(|(e, _, _)| e.i).collect();
        ids.sort_unstable();
        let ring = self.edges.first().map(|(e, _, _)| e.ring).unwrap_or(0);
        let head = if ring == 0 { String::new() } else { format!("courtyard {ring} ") };
        match ids.as_slice() {
            [] => format!("{head}facade"),
            [one] => format!("{head}edge {one}"),
            [a, .., b] if b - a + 1 == ids.len() => format!("{head}edges {a}..{b}"),
            _ => format!("{head}edges {}", ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(", ")),
        }
    }
}

/// Default turn below which consecutive plan edges form one facade run.
pub const RUN_ANGLE: f32 = 25.0;

/// Developed facades of every ring of `shape`: edges are chained into
/// runs wherever the plan turns by less than `run_angle` degrees, and
/// split at sharper corners and around `skip` edges (open sides).
/// `smooth` shades every vertex of a run smoothly (otherwise only those
/// turning less than 20 degrees).
pub fn runs_of(shape: &Shape, y0: f32, y1: f32, mat: MatId, run_angle: f32, smooth: bool, skip: &[EdgeRef]) -> Vec<Run> {
    let mut out = Vec::new();
    for (r, (ring, _)) in shape.rings().enumerate() {
        ring_runs(ring, r, true, y0, y1, mat, run_angle, smooth, skip, &mut out);
    }
    out
}

/// Runs along an open (or closed) polyline whose facade faces its right
/// side (free-standing walls). Edge `i` is the segment `i -> i + 1`.
pub fn path_runs(path: &[Vec2], closed: bool, y0: f32, y1: f32, mat: MatId, run_angle: f32, smooth: bool) -> Vec<Run> {
    let mut out = Vec::new();
    ring_runs(path, 0, closed, y0, y1, mat, run_angle, smooth, &[], &mut out);
    out
}

#[allow(clippy::too_many_arguments)]
fn ring_runs(ring: &[Vec2], r: usize, closed_ring: bool, y0: f32, y1: f32, mat: MatId, run_angle: f32, smooth: bool, skip: &[EdgeRef], out: &mut Vec<Run>) {
    let flat = Plane::new(Vec3::ZERO, Vec3::X, Vec3::Y);
    let n = ring.len();
    if n < 2 || (closed_ring && n < 3) {
        return;
    }
    let edges_n = if closed_ring { n } else { n - 1 };
    let per = poly::perimeter(ring);
    // Ring position of each vertex (for continuous texture u).
    let mut pos = vec![0.0f32; n + 1];
    for i in 0..n {
        pos[i + 1] = pos[i] + (ring[(i + 1) % n] - ring[i]).length();
    }
    let open: Vec<bool> = (0..n).map(|i| i >= edges_n || skip.contains(&EdgeRef { ring: r, i }) || (ring[(i + 1) % n] - ring[i]).length() < 1e-4).collect();
    let corner = runs::corners(ring, closed_ring, run_angle);
    // A vertex breaks a run when it is a corner or touches an open edge.
    let breaks: Vec<bool> = (0..n).map(|i| corner[i] || open[i] || open[(i + n - 1) % n]).collect();
    let mut chains: Vec<(usize, usize, bool)> = Vec::new();
    if !breaks.iter().any(|b| *b) {
        chains.push((0, n, true));
    } else {
        let start = (0..n).find(|&i| breaks[i]).unwrap();
        let mut k = 0;
        while k < n {
            let i = (start + k) % n;
            if open[i] {
                k += 1;
                continue;
            }
            // Chain of edges from vertex i until the next break.
            let mut len = 1;
            while k + len < n && !breaks[(start + k + len) % n] {
                len += 1;
            }
            chains.push((i, len, false));
            k += len;
        }
    }
    for (first, count, closed) in chains {
        // Facade order runs against the ring: from the chain's last vertex
        // back to its first.
        let mut pts: Vec<Vec2> = (0..=count).map(|k| ring[(first + count - k) % n]).collect();
        if closed {
            pts.pop();
        }
        let Some(bend) = Bend::new(&pts, closed, if smooth { 179.0 } else { 20.0 }) else { continue };
        let len = bend.len();
        let mut edges = Vec::with_capacity(count);
        // Edge i spans u from its end vertex (smaller u) to its start.
        let mut u = 0.0;
        for k in 0..count {
            let i = (first + count - 1 - k) % n;
            let l = (ring[(i + 1) % n] - ring[i]).length();
            edges.push((EdgeRef { ring: r, i }, u, u + l));
            u += l;
        }
        let end = first + count;
        let end_pos = if end >= n { pos[end - n] + if end - n == 0 { per } else { 0.0 } } else { pos[end] };
        let uv_off = if closed { 0.0 } else { per - end_pos };
        let facade = Facade { plane: flat, len, v0: y0, v1: y1, uv_off, mat, outline: None, wrap: closed.then_some(len) };
        out.push(Run { facade, bend: Some(Arc::new(bend)), edges });
    }
}

/// 3D point helper for details on a facade.
pub fn pt(f: &Facade, u: f32, v: f32, out: f32) -> Vec3 {
    f.plane.at_d(Vec2::new(u, v), out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::landmark::profiles;
    use crate::landmark::spec::ArchKind;

    fn facade() -> Facade {
        Facade { plane: Plane::new(Vec3::ZERO, Vec3::X, Vec3::Y), len: 10.0, v0: 0.0, v1: 6.0, uv_off: 0.0, mat: 0, outline: None, wrap: None }
    }

    #[test]
    fn window_hole_and_door_notch() {
        let f = facade();
        let win: Vec<Vec2> = profiles::outline(ArchKind::Round, 1.2, 2.4, None, 8).pts.iter().map(|p| *p + Vec2::new(3.0, 1.5)).collect();
        let door: Vec<Vec2> = profiles::outline(ArchKind::Pointed, 2.0, 3.5, None, 8).pts.iter().map(|p| *p + Vec2::new(7.0, 0.0)).collect();
        let (s, w) = wall_shape(&f, &[win.clone(), door.clone()]);
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(s.holes.len(), 1);
        let expect = 60.0 - poly::area(&win) - poly::area(&door);
        assert!((s.area() - expect).abs() < 1e-2, "{} vs {expect}", s.area());
        let (pts, tris) = s.triangulate();
        let tri_area: f32 = tris.iter().map(|t| (pts[t[1] as usize] - pts[t[0] as usize]).perp_dot(pts[t[2] as usize] - pts[t[0] as usize]) * 0.5).sum();
        assert!((tri_area - expect).abs() < 1e-2, "{tri_area} vs {expect}");
    }

    #[test]
    fn notch_across_facet_side() {
        let f = facade();
        let win: Vec<Vec2> = profiles::outline(ArchKind::Flat, 2.0, 2.0, None, 4).pts.iter().map(|p| *p + Vec2::new(9.6, 2.0)).collect();
        let (s, w) = wall_shape(&f, &[win]);
        assert!(w.is_empty());
        assert!(s.holes.is_empty());
        assert!((s.area() - (60.0 - 1.4 * 2.0)).abs() < 1e-2, "{}", s.area());
    }

    #[test]
    fn built_facade_is_finite_and_faces_out() {
        let f = facade();
        let win: Vec<Vec2> = profiles::outline(ArchKind::Horseshoe, 1.2, 2.4, None, 8).pts.iter().map(|p| *p + Vec2::new(3.0, 1.5)).collect();
        let mut g = Geo::new();
        let warn = build(&mut g, &f, &[Cut::simple(win, 0.4, 1, Some(2), false)]);
        assert!(warn.is_empty());
        for (k, p) in &g.prims {
            assert!(p.positions.iter().all(|q| mesh::finite3(*q)));
            if *k == 0 {
                assert!(p.normals.iter().all(|n| n.z > 0.99));
            }
        }
        // Reveals point into the opening (or outward), never into the wall.
        let rev = &g.prims[&1];
        assert!(rev.tri_count() > 20);
    }
}
