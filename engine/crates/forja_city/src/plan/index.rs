//! `SegIndex`: a uniform grid of segments for fast point location, rays
//! and distances over large regions (a whole city's public space has tens
//! of thousands of edges).

use std::collections::HashMap;

use forja_math::Vec2;

use super::region::Region;

#[derive(Clone, Debug, Default)]
pub struct SegIndex {
    cell: f32,
    segs: Vec<(Vec2, Vec2, u32)>,
    map: HashMap<(i32, i32), Vec<u32>>,
    lo: (i32, i32),
    hi: (i32, i32),
}

impl SegIndex {
    pub fn new(cell: f32) -> Self {
        SegIndex { cell, segs: Vec::new(), map: HashMap::new(), lo: (i32::MAX, i32::MAX), hi: (i32::MIN, i32::MIN) }
    }

    /// The edges of a region (payload 0).
    pub fn of_region(r: &Region, cell: f32) -> Self {
        let mut ix = SegIndex::new(cell);
        for (a, b) in r.edges() {
            ix.push(Vec2::new(a[0] as f32, a[1] as f32), Vec2::new(b[0] as f32, b[1] as f32), 0);
        }
        ix
    }

    fn key(&self, p: Vec2) -> (i32, i32) {
        ((p.x / self.cell).floor() as i32, (p.y / self.cell).floor() as i32)
    }

    pub fn push(&mut self, a: Vec2, b: Vec2, payload: u32) {
        let id = self.segs.len() as u32;
        self.segs.push((a, b, payload));
        let (k0, k1) = (self.key(a.min(b)), self.key(a.max(b)));
        self.lo = (self.lo.0.min(k0.0), self.lo.1.min(k0.1));
        self.hi = (self.hi.0.max(k1.0), self.hi.1.max(k1.1));
        for x in k0.0..=k1.0 {
            for z in k0.1..=k1.1 {
                self.map.entry((x, z)).or_default().push(id);
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.segs.is_empty()
    }

    pub fn seg(&self, i: u32) -> (Vec2, Vec2, u32) {
        self.segs[i as usize]
    }

    /// Crossing parity of the half-line from `p` towards +x: inside for a
    /// set of closed rings.
    pub fn contains(&self, p: Vec2) -> bool {
        if self.segs.is_empty() {
            return false;
        }
        let k = self.key(p);
        if k.1 < self.lo.1 || k.1 > self.hi.1 {
            return false;
        }
        let (x, y) = (p.x as f64, p.y as f64);
        let mut inside = false;
        for cx in k.0.max(self.lo.0)..=self.hi.0 {
            let Some(l) = self.map.get(&(cx, k.1)) else { continue };
            let (x0, x1) = (cx as f64 * self.cell as f64, (cx + 1) as f64 * self.cell as f64);
            for &i in l {
                let (a, b, _) = self.segs[i as usize];
                let (ay, by) = (a.y as f64, b.y as f64);
                if (ay > y) != (by > y) {
                    let t = (y - ay) / (by - ay);
                    let xc = a.x as f64 + t * (b.x as f64 - a.x as f64);
                    // Count each crossing once: in the cell that holds it.
                    if xc > x && xc >= x0 && xc < x1 {
                        inside = !inside;
                    }
                }
            }
        }
        inside
    }

    /// First segment hit by the ray `from + t dir`, `0 < t <= max`.
    pub fn ray(&self, from: Vec2, dir: Vec2, max: f32) -> Option<(f32, u32)> {
        self.ray_filter(from, dir, max, |_| true)
    }

    pub fn ray_filter(&self, from: Vec2, dir: Vec2, max: f32, keep: impl Fn(u32) -> bool) -> Option<(f32, u32)> {
        if self.segs.is_empty() {
            return None;
        }
        let (ox, oy, dx, dy) = (from.x as f64, from.y as f64, dir.x as f64, dir.y as f64);
        let mut best: Option<(f64, u32)> = None;
        // Walk the cells along the ray (DDA).
        let c = self.cell as f64;
        let (mut cx, mut cy) = ((ox / c).floor() as i32, (oy / c).floor() as i32);
        let step_x = if dx > 0.0 { 1 } else { -1 };
        let step_y = if dy > 0.0 { 1 } else { -1 };
        let next = |cell: i32, step: i32| (cell + if step > 0 { 1 } else { 0 }) as f64 * c;
        let mut t_max_x = if dx.abs() < 1e-12 { f64::INFINITY } else { (next(cx, step_x) - ox) / dx };
        let mut t_max_y = if dy.abs() < 1e-12 { f64::INFINITY } else { (next(cy, step_y) - oy) / dy };
        let (t_dx, t_dy) = (if dx.abs() < 1e-12 { f64::INFINITY } else { c / dx.abs() }, if dy.abs() < 1e-12 { f64::INFINITY } else { c / dy.abs() });
        let mut t_cell: f64;
        loop {
            if let Some(l) = self.map.get(&(cx, cy)) {
                for &i in l {
                    let (a, b, _) = self.segs[i as usize];
                    let (ex, ey) = (b.x as f64 - a.x as f64, b.y as f64 - a.y as f64);
                    let den = dx * ey - dy * ex;
                    if den.abs() < 1e-12 {
                        continue;
                    }
                    let (wx, wy) = (a.x as f64 - ox, a.y as f64 - oy);
                    let t = (wx * ey - wy * ex) / den;
                    let u = (wx * dy - wy * dx) / den;
                    if t > 1e-6 && t <= max as f64 && (-1e-9..=1.0 + 1e-9).contains(&u) && best.is_none_or(|b| t < b.0) && keep(i) {
                        best = Some((t, i));
                    }
                }
            }
            // Every nearer hit lies in a cell already visited.
            if best.is_some_and(|b| b.0 <= t_max_x.min(t_max_y)) {
                break;
            }
            if t_max_x < t_max_y {
                t_cell = t_max_x;
                t_max_x += t_dx;
                cx += step_x;
            } else {
                t_cell = t_max_y;
                t_max_y += t_dy;
                cy += step_y;
            }
            if t_cell > max as f64 {
                break;
            }
        }
        best.map(|(t, i)| (t as f32, i))
    }

    /// Nearest segment within `max`: distance and id.
    pub fn nearest(&self, p: Vec2, max: f32) -> Option<(f32, u32)> {
        let k = self.key(p);
        let rings = (max / self.cell).ceil() as i32 + 1;
        let mut best: Option<(f32, u32)> = None;
        for r in 0..=rings {
            for x in k.0 - r..=k.0 + r {
                for z in k.1 - r..=k.1 + r {
                    if (x - k.0).abs() != r && (z - k.1).abs() != r {
                        continue;
                    }
                    let Some(l) = self.map.get(&(x, z)) else { continue };
                    for &i in l {
                        let (a, b, _) = self.segs[i as usize];
                        let d = super::line::seg_dist(p, a, b);
                        if d <= max && best.is_none_or(|bb| d < bb.0) {
                            best = Some((d, i));
                        }
                    }
                }
            }
            // Cells beyond ring r are at least r * cell away.
            if best.is_some_and(|b| b.0 <= r as f32 * self.cell) {
                break;
            }
        }
        best
    }

    /// Segments whose cells touch the disc.
    pub fn around(&self, p: Vec2, r: f32) -> Vec<u32> {
        let (k0, k1) = (self.key(p - Vec2::splat(r)), self.key(p + Vec2::splat(r)));
        let mut out = Vec::new();
        for x in k0.0..=k1.0 {
            for z in k0.1..=k1.1 {
                if let Some(l) = self.map.get(&(x, z)) {
                    out.extend_from_slice(l);
                }
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }
}

/// A region with its index: fast `contains`, `distance` and `ray`.
#[derive(Clone, Debug, Default)]
pub struct Located {
    pub region: Region,
    pub index: SegIndex,
}

impl Located {
    pub fn new(region: Region) -> Self {
        let index = SegIndex::of_region(&region, 8.0);
        Located { region, index }
    }

    pub fn contains(&self, p: Vec2) -> bool {
        self.index.contains(p)
    }

    pub fn distance(&self, p: Vec2, max: f32) -> f32 {
        self.index.nearest(p, max).map(|d| d.0).unwrap_or(max)
    }

    pub fn contains_disc(&self, p: Vec2, r: f32) -> bool {
        self.contains(p) && self.distance(p, r + 0.01) >= r
    }

    pub fn ray(&self, from: Vec2, dir: Vec2, max: f32) -> Option<f32> {
        self.index.ray(from, dir, max).map(|h| h.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_agrees_with_the_region() {
        let mut rng = crate::geom::Rng::new(9);
        let rings: Vec<Vec<Vec2>> = (0..30)
            .map(|k| {
                let c = Vec2::new(rng.range(-80.0, 80.0), rng.range(-80.0, 80.0));
                (0..7).map(|i| c + Vec2::from_angle(i as f32 * 0.9 + k as f32) * rng.range(3.0, 14.0)).collect()
            })
            .collect();
        let r = Region::rings(rings.iter().map(|r| r.as_slice()));
        let l = Located::new(r.clone());
        for _ in 0..3000 {
            let p = Vec2::new(rng.range(-100.0, 100.0), rng.range(-100.0, 100.0));
            if r.boundary_distance(p) < 1e-3 {
                continue;
            }
            assert_eq!(l.contains(p), r.contains(p), "{p}");
            assert!((l.distance(p, 500.0) - r.boundary_distance(p)).abs() < 1e-3);
            let d = Vec2::from_angle(rng.range(0.0, 6.28));
            let (a, b) = (l.ray(p, d, 300.0), r.ray(p, d, 300.0));
            assert_eq!(a.is_some(), b.is_some());
            if let (Some(a), Some(b)) = (a, b) {
                assert!((a - b).abs() < 1e-3, "{a} {b}");
            }
        }
    }
}
