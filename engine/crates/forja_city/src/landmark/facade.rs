//! Walls with openings of any shape. A facade is a planar region in
//! (u along the wall, v = height) coordinates; openings are cut as holes,
//! or as notches when they reach an edge (doors at the ground, arches
//! under an arcade wall, windows spanning two facets). Reveals follow each
//! opening's outline through recessed orders or splays to the infill
//! plane, so arched, round and lobed openings get real depth.

use forja_math::{Vec2, Vec3};

use super::mesh::{self, Geo, MatId, Plane};
use super::poly::{self, Shape};

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

/// Builds the wall face, reveals and recess planes of a facade.
pub fn build(geo: &mut Geo, f: &Facade, cuts: &[Cut]) -> Vec<String> {
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
        Facade { plane: Plane::new(Vec3::ZERO, Vec3::X, Vec3::Y), len: 10.0, v0: 0.0, v1: 6.0, uv_off: 0.0, mat: 0, outline: None }
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
