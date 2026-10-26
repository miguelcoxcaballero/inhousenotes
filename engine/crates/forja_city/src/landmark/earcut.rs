//! Polygon triangulation with holes (ear clipping with hole bridging,
//! after the earcut algorithm by Vladimir Agafonkin / Mapbox, ISC).
//!
//! Works on concave rings, holes, collinear and duplicate points, and
//! falls back to splitting the polygon when no ear can be found, so it
//! always returns a triangulation for valid input and degrades
//! gracefully on self-touching rings.

use forja_math::Vec2;

#[derive(Clone, Copy)]
struct Node {
    i: usize,
    x: f64,
    y: f64,
    prev: usize,
    next: usize,
    steiner: bool,
}

struct List {
    n: Vec<Node>,
}

impl List {
    fn insert(&mut self, i: usize, x: f64, y: f64, last: Option<usize>) -> usize {
        let k = self.n.len();
        self.n.push(Node { i, x, y, prev: k, next: k, steiner: false });
        if let Some(l) = last {
            let ln = self.n[l].next;
            self.n[k].next = ln;
            self.n[k].prev = l;
            self.n[ln].prev = k;
            self.n[l].next = k;
        }
        k
    }

    fn remove(&mut self, p: usize) {
        let (pr, nx) = (self.n[p].prev, self.n[p].next);
        self.n[nx].prev = pr;
        self.n[pr].next = nx;
    }

    fn area(&self, p: usize, q: usize, r: usize) -> f64 {
        let (p, q, r) = (&self.n[p], &self.n[q], &self.n[r]);
        (q.y - p.y) * (r.x - q.x) - (q.x - p.x) * (r.y - q.y)
    }

    fn equals(&self, a: usize, b: usize) -> bool {
        self.n[a].x == self.n[b].x && self.n[a].y == self.n[b].y
    }

    fn next(&self, p: usize) -> usize {
        self.n[p].next
    }

    fn prev(&self, p: usize) -> usize {
        self.n[p].prev
    }

    fn linked(&mut self, pts: &[[f64; 2]], start: usize, end: usize, clockwise: bool) -> Option<usize> {
        if end <= start {
            return None;
        }
        let mut sum = 0.0;
        let mut j = end - 1;
        for i in start..end {
            sum += (pts[j][0] - pts[i][0]) * (pts[i][1] + pts[j][1]);
            j = i;
        }
        let mut last = None;
        if clockwise == (sum > 0.0) {
            for i in start..end {
                last = Some(self.insert(i, pts[i][0], pts[i][1], last));
            }
        } else {
            for i in (start..end).rev() {
                last = Some(self.insert(i, pts[i][0], pts[i][1], last));
            }
        }
        if let Some(l) = last
            && self.equals(l, self.next(l))
        {
            let nx = self.next(l);
            self.remove(l);
            last = Some(nx);
        }
        last
    }

    fn filter(&mut self, start: usize, end: Option<usize>) -> usize {
        let mut end = end.unwrap_or(start);
        let mut p = start;
        loop {
            let mut again = false;
            let nx = self.next(p);
            if !self.n[p].steiner && (self.equals(p, nx) || self.area(self.prev(p), p, nx) == 0.0) {
                let pr = self.prev(p);
                self.remove(p);
                p = pr;
                end = pr;
                if p == self.next(p) {
                    break;
                }
                again = true;
            } else {
                p = nx;
            }
            if !again && p == end {
                break;
            }
        }
        end
    }

    fn point_in_triangle(ax: f64, ay: f64, bx: f64, by: f64, cx: f64, cy: f64, px: f64, py: f64) -> bool {
        (cx - px) * (ay - py) >= (ax - px) * (cy - py) && (ax - px) * (by - py) >= (bx - px) * (ay - py) && (bx - px) * (cy - py) >= (cx - px) * (by - py)
    }

    fn is_ear(&self, ear: usize) -> bool {
        let (a, b, c) = (self.prev(ear), ear, self.next(ear));
        if self.area(a, b, c) >= 0.0 {
            return false;
        }
        let (na, nb, nc) = (&self.n[a], &self.n[b], &self.n[c]);
        let x0 = na.x.min(nb.x).min(nc.x);
        let y0 = na.y.min(nb.y).min(nc.y);
        let x1 = na.x.max(nb.x).max(nc.x);
        let y1 = na.y.max(nb.y).max(nc.y);
        let mut p = self.next(c);
        while p != a {
            let np = &self.n[p];
            if np.x >= x0
                && np.x <= x1
                && np.y >= y0
                && np.y <= y1
                && Self::point_in_triangle(na.x, na.y, nb.x, nb.y, nc.x, nc.y, np.x, np.y)
                && self.area(self.prev(p), p, self.next(p)) >= 0.0
            {
                return false;
            }
            p = self.next(p);
        }
        true
    }

    fn earcut_linked(&mut self, ear: Option<usize>, tris: &mut Vec<[usize; 3]>, pass: u8, budget: &mut usize) {
        let Some(mut ear) = ear else { return };
        if *budget == 0 {
            return;
        }
        *budget -= 1;
        let mut stop = ear;
        while self.prev(ear) != self.next(ear) {
            let (prev, next) = (self.prev(ear), self.next(ear));
            if self.is_ear(ear) {
                tris.push([self.n[prev].i, self.n[ear].i, self.n[next].i]);
                self.remove(ear);
                ear = self.next(next);
                stop = ear;
                continue;
            }
            ear = next;
            if ear == stop {
                match pass {
                    0 => {
                        let f = self.filter(ear, None);
                        self.earcut_linked(Some(f), tris, 1, budget);
                    }
                    1 => {
                        let f = self.filter(ear, None);
                        let e = self.cure_local_intersections(f, tris);
                        self.earcut_linked(Some(e), tris, 2, budget);
                    }
                    _ => self.split_earcut(ear, tris, budget),
                }
                break;
            }
        }
    }

    fn sign(v: f64) -> i8 {
        if v > 0.0 {
            1
        } else if v < 0.0 {
            -1
        } else {
            0
        }
    }

    fn on_segment(&self, p: usize, q: usize, r: usize) -> bool {
        let (p, q, r) = (&self.n[p], &self.n[q], &self.n[r]);
        q.x <= p.x.max(r.x) && q.x >= p.x.min(r.x) && q.y <= p.y.max(r.y) && q.y >= p.y.min(r.y)
    }

    fn intersects(&self, p1: usize, q1: usize, p2: usize, q2: usize) -> bool {
        let o1 = Self::sign(self.area(p1, q1, p2));
        let o2 = Self::sign(self.area(p1, q1, q2));
        let o3 = Self::sign(self.area(p2, q2, p1));
        let o4 = Self::sign(self.area(p2, q2, q1));
        if o1 != o2 && o3 != o4 {
            return true;
        }
        (o1 == 0 && self.on_segment(p1, p2, q1)) || (o2 == 0 && self.on_segment(p1, q2, q1)) || (o3 == 0 && self.on_segment(p2, p1, q2)) || (o4 == 0 && self.on_segment(p2, q1, q2))
    }

    fn locally_inside(&self, a: usize, b: usize) -> bool {
        let (ap, an) = (self.prev(a), self.next(a));
        if self.area(ap, a, an) < 0.0 { self.area(a, b, an) >= 0.0 && self.area(a, ap, b) >= 0.0 } else { self.area(a, b, ap) < 0.0 || self.area(a, an, b) < 0.0 }
    }

    fn cure_local_intersections(&mut self, start: usize, tris: &mut Vec<[usize; 3]>) -> usize {
        let mut start = start;
        let mut p = start;
        loop {
            let a = self.prev(p);
            let b = self.next(self.next(p));
            if !self.equals(a, b) && self.intersects(a, p, self.next(p), b) && self.locally_inside(a, b) && self.locally_inside(b, a) {
                tris.push([self.n[a].i, self.n[p].i, self.n[b].i]);
                let pn = self.next(p);
                self.remove(p);
                self.remove(pn);
                p = b;
                start = b;
            }
            p = self.next(p);
            if p == start {
                break;
            }
        }
        self.filter(p, None)
    }

    fn intersects_polygon(&self, a: usize, b: usize) -> bool {
        let mut p = a;
        loop {
            let pn = self.next(p);
            let (pi, pni, ai, bi) = (self.n[p].i, self.n[pn].i, self.n[a].i, self.n[b].i);
            if pi != ai && pni != ai && pi != bi && pni != bi && self.intersects(p, pn, a, b) {
                return true;
            }
            p = pn;
            if p == a {
                return false;
            }
        }
    }

    fn middle_inside(&self, a: usize, b: usize) -> bool {
        let mut p = a;
        let mut inside = false;
        let px = (self.n[a].x + self.n[b].x) / 2.0;
        let py = (self.n[a].y + self.n[b].y) / 2.0;
        loop {
            let (np, nn) = (&self.n[p], &self.n[self.next(p)]);
            if (np.y > py) != (nn.y > py) && nn.y != np.y && px < (nn.x - np.x) * (py - np.y) / (nn.y - np.y) + np.x {
                inside = !inside;
            }
            p = self.next(p);
            if p == a {
                return inside;
            }
        }
    }

    fn is_valid_diagonal(&self, a: usize, b: usize) -> bool {
        let (an, ap, bn, bp) = (self.next(a), self.prev(a), self.next(b), self.prev(b));
        self.n[an].i != self.n[b].i
            && self.n[ap].i != self.n[b].i
            && !self.intersects_polygon(a, b)
            && ((self.locally_inside(a, b) && self.locally_inside(b, a) && self.middle_inside(a, b) && (self.area(ap, a, bp) != 0.0 || self.area(a, bp, b) != 0.0))
                || (self.equals(a, b) && self.area(ap, a, an) > 0.0 && self.area(bp, b, bn) > 0.0))
    }

    fn split_polygon(&mut self, a: usize, b: usize) -> usize {
        let (ai, ax, ay) = (self.n[a].i, self.n[a].x, self.n[a].y);
        let (bi, bx, by) = (self.n[b].i, self.n[b].x, self.n[b].y);
        let a2 = self.n.len();
        self.n.push(Node { i: ai, x: ax, y: ay, prev: a2, next: a2, steiner: false });
        let b2 = self.n.len();
        self.n.push(Node { i: bi, x: bx, y: by, prev: b2, next: b2, steiner: false });
        let an = self.next(a);
        let bp = self.prev(b);
        self.n[a].next = b;
        self.n[b].prev = a;
        self.n[a2].next = an;
        self.n[an].prev = a2;
        self.n[b2].next = a2;
        self.n[a2].prev = b2;
        self.n[bp].next = b2;
        self.n[b2].prev = bp;
        b2
    }

    fn split_earcut(&mut self, start: usize, tris: &mut Vec<[usize; 3]>, budget: &mut usize) {
        let mut a = start;
        loop {
            let mut b = self.next(self.next(a));
            while b != self.prev(a) {
                if self.n[a].i != self.n[b].i && self.is_valid_diagonal(a, b) {
                    let c = self.split_polygon(a, b);
                    let an = self.next(a);
                    let a = self.filter(a, Some(an));
                    let cn = self.next(c);
                    let c = self.filter(c, Some(cn));
                    self.earcut_linked(Some(a), tris, 0, budget);
                    self.earcut_linked(Some(c), tris, 0, budget);
                    return;
                }
                b = self.next(b);
            }
            a = self.next(a);
            if a == start {
                return;
            }
        }
    }

    fn leftmost(&self, start: usize) -> usize {
        let (mut p, mut l) = (start, start);
        loop {
            let (np, nl) = (&self.n[p], &self.n[l]);
            if np.x < nl.x || (np.x == nl.x && np.y < nl.y) {
                l = p;
            }
            p = self.next(p);
            if p == start {
                return l;
            }
        }
    }

    fn sector_contains_sector(&self, m: usize, p: usize) -> bool {
        self.area(self.prev(m), m, self.prev(p)) < 0.0 && self.area(self.next(p), m, self.next(m)) < 0.0
    }

    fn find_hole_bridge(&self, hole: usize, outer: usize) -> Option<usize> {
        let mut p = outer;
        let (hx, hy) = (self.n[hole].x, self.n[hole].y);
        let mut qx = f64::NEG_INFINITY;
        let mut m: Option<usize> = None;
        loop {
            let (np, nn) = (&self.n[p], &self.n[self.next(p)]);
            if hy <= np.y && hy >= nn.y && nn.y != np.y {
                let x = np.x + (hy - np.y) * (nn.x - np.x) / (nn.y - np.y);
                if x <= hx && x > qx {
                    qx = x;
                    m = Some(if np.x < nn.x { p } else { self.next(p) });
                    if x == hx {
                        return m;
                    }
                }
            }
            p = self.next(p);
            if p == outer {
                break;
            }
        }
        let mut m = m?;
        let stop = m;
        let (mx, my) = (self.n[m].x, self.n[m].y);
        let mut tan_min = f64::INFINITY;
        p = m;
        loop {
            let (px, py) = (self.n[p].x, self.n[p].y);
            if hx >= px && px >= mx && hx != px && Self::point_in_triangle(if hy < my { hx } else { qx }, hy, mx, my, if hy < my { qx } else { hx }, hy, px, py) {
                let tan = (hy - py).abs() / (hx - px);
                if self.locally_inside(p, hole) && (tan < tan_min || (tan == tan_min && (px > self.n[m].x || (px == self.n[m].x && self.sector_contains_sector(m, p))))) {
                    m = p;
                    tan_min = tan;
                }
            }
            p = self.next(p);
            if p == stop {
                break;
            }
        }
        Some(m)
    }

    fn eliminate_holes(&mut self, pts: &[[f64; 2]], holes: &[usize], mut outer: usize) -> usize {
        let mut queue = Vec::new();
        for (k, &start) in holes.iter().enumerate() {
            let end = holes.get(k + 1).copied().unwrap_or(pts.len());
            if let Some(list) = self.linked(pts, start, end, false) {
                if list == self.next(list) {
                    self.n[list].steiner = true;
                }
                queue.push(self.leftmost(list));
            }
        }
        queue.sort_by(|&a, &b| {
            let (na, nb) = (&self.n[a], &self.n[b]);
            na.x.total_cmp(&nb.x).then(na.y.total_cmp(&nb.y))
        });
        for h in queue {
            if let Some(bridge) = self.find_hole_bridge(h, outer) {
                let rev = self.split_polygon(bridge, h);
                let rn = self.next(rev);
                self.filter(rev, Some(rn));
                let bn = self.next(bridge);
                outer = self.filter(bridge, Some(bn));
            }
        }
        outer
    }
}

/// Triangulates a polygon given as a flat point list where the outer ring
/// is `pts[..holes[0]]` and hole `k` starts at `holes[k]`. Returns
/// counter-clockwise triangles (in x, y) as indices into `pts`.
pub fn earcut(pts: &[Vec2], holes: &[usize]) -> Vec<[u32; 3]> {
    let data: Vec<[f64; 2]> = pts.iter().map(|p| [p.x as f64, p.y as f64]).collect();
    if data.iter().any(|p| !p[0].is_finite() || !p[1].is_finite()) {
        return Vec::new();
    }
    let outer_len = holes.first().copied().unwrap_or(data.len());
    let mut list = List { n: Vec::with_capacity(data.len() * 3 / 2 + 8) };
    let Some(mut outer) = list.linked(&data, 0, outer_len, true) else { return Vec::new() };
    if list.next(outer) == list.prev(outer) {
        return Vec::new();
    }
    if !holes.is_empty() {
        outer = list.eliminate_holes(&data, holes, outer);
    }
    let mut tris = Vec::with_capacity(data.len() + holes.len() * 2);
    let mut budget = data.len() * 4 + 64;
    list.earcut_linked(Some(outer), &mut tris, 0, &mut budget);
    // Earcut emits clockwise (screen) = counter-clockwise (math) triangles;
    // normalise to math CCW in case of degenerate input.
    tris.into_iter()
        .filter(|t| t[0] != t[1] && t[1] != t[2] && t[0] != t[2])
        .map(|t| {
            let (a, b, c) = (pts[t[0]], pts[t[1]], pts[t[2]]);
            if (b - a).perp_dot(c - a) < 0.0 { [t[0] as u32, t[2] as u32, t[1] as u32] } else { [t[0] as u32, t[1] as u32, t[2] as u32] }
        })
        .collect()
}

/// Triangulates `outer` with `holes` (any winding). Returns the points
/// (outer then holes) and counter-clockwise triangles over them.
pub fn triangulate_with_holes(outer: &[Vec2], holes: &[Vec<Vec2>]) -> (Vec<Vec2>, Vec<[u32; 3]>) {
    let mut pts = outer.to_vec();
    let mut idx = Vec::with_capacity(holes.len());
    for h in holes {
        if h.len() < 3 {
            continue;
        }
        idx.push(pts.len());
        pts.extend_from_slice(h);
    }
    let tris = earcut(&pts, &idx);
    (pts, tris)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(pts: &[Vec2], tris: &[[u32; 3]]) -> f32 {
        tris.iter().map(|t| (pts[t[1] as usize] - pts[t[0] as usize]).perp_dot(pts[t[2] as usize] - pts[t[0] as usize]) * 0.5).sum()
    }

    #[test]
    fn square_with_hole() {
        let outer = vec![Vec2::new(0., 0.), Vec2::new(10., 0.), Vec2::new(10., 10.), Vec2::new(0., 10.)];
        let hole = vec![Vec2::new(3., 3.), Vec2::new(3., 7.), Vec2::new(7., 7.), Vec2::new(7., 3.)];
        let (pts, tris) = triangulate_with_holes(&outer, &[hole]);
        assert!((area(&pts, &tris) - 84.0).abs() < 1e-3, "{}", area(&pts, &tris));
        assert!(tris.iter().all(|t| (pts[t[1] as usize] - pts[t[0] as usize]).perp_dot(pts[t[2] as usize] - pts[t[0] as usize]) > 0.0));
    }

    #[test]
    fn concave_comb_and_two_holes() {
        // A comb (very concave) with two holes in its spine.
        let mut outer = vec![Vec2::new(0., 0.), Vec2::new(20., 0.), Vec2::new(20., 4.)];
        for k in 0..5 {
            let x = 20.0 - k as f32 * 4.0;
            outer.extend([Vec2::new(x, 10.), Vec2::new(x - 2., 10.), Vec2::new(x - 2., 4.)]);
        }
        outer.push(Vec2::new(0., 4.));
        outer.reverse(); // any winding
        let h1 = vec![Vec2::new(1., 1.), Vec2::new(3., 1.), Vec2::new(3., 3.), Vec2::new(1., 3.)];
        let h2 = vec![Vec2::new(12., 1.), Vec2::new(15., 1.), Vec2::new(15., 2.)];
        let (pts, tris) = triangulate_with_holes(&outer, &[h1, h2]);
        let expect = crate::landmark::poly::area(&outer).abs() - 4.0 - 1.5;
        let got = area(&pts, &tris);
        assert!((got - expect).abs() < 1e-2, "{got} vs {expect}");
    }

    #[test]
    fn collinear_and_duplicate_points() {
        let outer = vec![Vec2::new(0., 0.), Vec2::new(5., 0.), Vec2::new(5., 0.), Vec2::new(10., 0.), Vec2::new(10., 5.), Vec2::new(10., 10.), Vec2::new(0., 10.)];
        let (pts, tris) = triangulate_with_holes(&outer, &[]);
        assert!((area(&pts, &tris) - 100.0).abs() < 1e-3);
    }
}
