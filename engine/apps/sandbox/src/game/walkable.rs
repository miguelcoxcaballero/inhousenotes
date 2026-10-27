//! Where people walk, for any street layout.
//!
//! The static colliders are rasterised top-down into a height grid and
//! classified: pavement (a step above the roadway), roadway, and obstacles
//! (buildings, trees, anything taller than a step). Roadway in lanes too
//! narrow for traffic (about 7 m between walls) is walkable as well:
//! old-town callejuelas have no sidewalks and people walk down the middle.
//!
//! From that mask comes a signed distance field to the edge of the
//! walkable area. Pedestrians follow its iso-lines, which trace every
//! sidewalk round blocks of any shape, curved and organic streets, plazas
//! and alleys alike: nothing here knows about the street network, so the
//! crowd works with whatever layout the city generator produces.

use forja_math::{Iso, Vec2, Vec3};
use forja_physics::{ColliderDesc, Shape};

/// Pavement is at least this much above the roadway (m).
const STEP: f32 = 0.06;
/// Anything higher above the roadway than this is an obstacle (m).
const OBSTACLE: f32 = 0.45;
/// Roadway narrower than about twice this between obstacles is a lane
/// people walk in (m).
const LANE_HALF_WIDTH: f32 = 3.6;
/// Walkable patches smaller than this are gaps between buildings, not
/// streets (m²).
const MIN_AREA: f32 = 250.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ground {
    Void,
    Road,
    Pavement,
    Obstacle,
}

pub struct Walkable {
    origin: Vec2,
    cell: f32,
    w: usize,
    h: usize,
    /// Floor height: pavement and roadway as rasterised; obstacles and
    /// gaps at pavement level so blending near them stays on the ground.
    floor: Vec<f32>,
    ground: Vec<Ground>,
    walk: Vec<bool>,
    /// Signed distance to the edge of the walkable area (m), positive inside.
    sdf: Vec<f32>,
    /// Half width of the walkway around each cell (m).
    half: Vec<f32>,
    /// Walkable cells away from the edges, for spawning.
    open: Vec<u32>,
}

/// The walkable field at a point.
#[derive(Clone, Copy, Debug)]
pub struct Sample {
    /// Signed distance to the walkway edge (m), positive inside.
    pub sdf: f32,
    /// Unit direction away from the nearest edge (towards the middle).
    pub inward: Vec2,
    /// Local half width of the walkway (m).
    pub half: f32,
    /// Ground height.
    pub height: f32,
}

impl Walkable {
    /// Rasterises `colliders` over the square of half size `extent`
    /// around `center` (XZ) with `cell`-metre cells.
    pub fn from_colliders(colliders: &[(ColliderDesc, Iso)], center: Vec2, extent: Vec2, cell: f32) -> Option<Self> {
        let origin = center - extent;
        let (w, h) = (((extent.x * 2.0) / cell).ceil() as usize, ((extent.y * 2.0) / cell).ceil() as usize);
        if w < 8 || h < 8 || w * h > 16_000_000 {
            return None;
        }
        let mut r = Raster { origin, cell, w, h, height: vec![f32::NEG_INFINITY; w * h] };
        for (desc, iso) in colliders {
            r.shape(&desc.shape, &iso.mul(&desc.local));
        }
        Some(Self::from_heights(r.origin, cell, w, h, r.height))
    }

    fn from_heights(origin: Vec2, cell: f32, w: usize, h: usize, height: Vec<f32>) -> Self {
        let n = w * h;
        // The roadway is the commonest low surface: the 10th percentile.
        let mut known: Vec<f32> = height.iter().copied().filter(|v| v.is_finite()).collect();
        let base = if known.is_empty() {
            0.0
        } else {
            let k = known.len() / 10;
            *known.select_nth_unstable_by(k, f32::total_cmp).1
        };
        let ground: Vec<Ground> = height
            .iter()
            .map(|&y| match y - base {
                d if !d.is_finite() => Ground::Void,
                d if d > OBSTACLE => Ground::Obstacle,
                d if d > STEP => Ground::Pavement,
                _ => Ground::Road,
            })
            .collect();
        // Lanes: roadway whose widest point nearby is close to walls on
        // both sides, plus a few metres where they open into wider streets
        // so they connect to the sidewalks.
        let blocked: Vec<bool> = ground.iter().map(|g| matches!(g, Ground::Obstacle | Ground::Void)).collect();
        let to_wall = distance(&blocked, w, h, cell);
        let widest = max_filter(&to_wall, w, h, (LANE_HALF_WIDTH / cell).round() as usize);
        let lane: Vec<bool> = (0..n).map(|i| ground[i] == Ground::Road && widest[i] < LANE_HALF_WIDTH).collect();
        let near_lane = distance(&lane, w, h, cell);
        let mut walk: Vec<bool> = (0..n).map(|i| ground[i] == Ground::Pavement || (ground[i] == Ground::Road && near_lane[i] < 3.0)).collect();
        keep_street_patches(&mut walk, &ground, w, h, (MIN_AREA / (cell * cell)) as usize);
        let not_walk: Vec<bool> = walk.iter().map(|x| !x).collect();
        let inside = distance(&not_walk, w, h, cell);
        let outside = distance(&walk, w, h, cell);
        let sdf: Vec<f32> = (0..n).map(|i| if walk[i] { inside[i] - cell * 0.5 } else { -(outside[i] - cell * 0.5) }).collect();
        let r = (3.0 / cell).round() as usize;
        let half = max_filter(&sdf, w, h, r);
        let open = (0..n).filter(|&i| walk[i] && sdf[i] > 0.6).map(|i| i as u32).collect();
        let floor = (0..n).map(|i| if matches!(ground[i], Ground::Road | Ground::Pavement) { height[i] } else { base + 0.15 }).collect();
        Self { origin, cell, w, h, floor, ground, walk, sdf, half, open }
    }

    /// Top-down map for checking a layout: buildings dark, roadway grey,
    /// pavement tan, walkway tinted by its distance field; `marks` in red.
    pub fn debug_image(&self, marks: &[Vec2]) -> image::RgbImage {
        let mut img = image::RgbImage::new(self.w as u32, self.h as u32);
        for y in 0..self.h {
            for x in 0..self.w {
                let i = y * self.w + x;
                let base: [u8; 3] = match self.ground[i] {
                    Ground::Void => [0, 0, 0],
                    Ground::Road => [90, 90, 95],
                    Ground::Pavement => [170, 150, 120],
                    Ground::Obstacle => [35, 30, 40],
                };
                let px = if self.walk[i] {
                    let band = ((self.sdf[i] / 0.5) as i32 % 2 == 0) as u8;
                    [base[0].saturating_add(40 * band), base[1].saturating_add(60), base[2].saturating_add(20 * band)]
                } else {
                    base
                };
                // North (+z) up.
                img.put_pixel(x as u32, (self.h - 1 - y) as u32, image::Rgb(px));
            }
        }
        for m in marks {
            let q = (*m - self.origin) / self.cell;
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (-1, 0), (0, -1)] {
                let (x, y) = (q.x as i32 + dx, q.y as i32 + dy);
                if x >= 0 && y >= 0 && (x as usize) < self.w && (y as usize) < self.h {
                    img.put_pixel(x as u32, (self.h - 1 - y as usize) as u32, image::Rgb([230, 30, 30]));
                }
            }
        }
        img
    }

    /// Walkable area (m²).
    pub fn area(&self) -> f32 {
        self.walk.iter().filter(|w| **w).count() as f32 * self.cell * self.cell
    }

    fn index(&self, p: Vec2) -> Option<usize> {
        let q = (p - self.origin) / self.cell;
        let (x, y) = (q.x.floor(), q.y.floor());
        (x >= 0.0 && y >= 0.0 && (x as usize) < self.w && (y as usize) < self.h).then(|| y as usize * self.w + x as usize)
    }

    pub fn ground(&self, p: Vec2) -> Ground {
        self.index(p).map(|i| self.ground[i]).unwrap_or(Ground::Void)
    }

    pub fn walkable(&self, p: Vec2) -> bool {
        self.index(p).is_some_and(|i| self.walk[i])
    }

    /// Bilinear sample of a field at cell-centre coordinates.
    fn bilinear(&self, f: &[f32], p: Vec2) -> f32 {
        let q = (p - self.origin) / self.cell - Vec2::splat(0.5);
        let (x0, y0) = (q.x.floor(), q.y.floor());
        let (fx, fy) = (q.x - x0, q.y - y0);
        let at = |x: f32, y: f32| {
            let (x, y) = ((x.max(0.0) as usize).min(self.w - 1), (y.max(0.0) as usize).min(self.h - 1));
            f[y * self.w + x]
        };
        let a = at(x0, y0) + (at(x0 + 1.0, y0) - at(x0, y0)) * fx;
        let b = at(x0, y0 + 1.0) + (at(x0 + 1.0, y0 + 1.0) - at(x0, y0 + 1.0)) * fx;
        a + (b - a) * fy
    }

    pub fn sample(&self, p: Vec2) -> Sample {
        let e = self.cell;
        let gx = self.bilinear(&self.sdf, p + Vec2::X * e) - self.bilinear(&self.sdf, p - Vec2::X * e);
        let gz = self.bilinear(&self.sdf, p + Vec2::Y * e) - self.bilinear(&self.sdf, p - Vec2::Y * e);
        // Kerbs between a lane and its sidewalks blend over a cell.
        let height = self.bilinear(&self.floor, p);
        Sample { sdf: self.bilinear(&self.sdf, p), inward: Vec2::new(gx, gz).normalize_or_zero(), half: self.bilinear(&self.half, p), height }
    }

    /// A random point on the walkway within `[rmin, rmax]` of `center`
    /// for which `accept` holds.
    pub fn random_spot(&self, rng: &mut super::people::Rng, center: Vec2, rmin: f32, rmax: f32, mut accept: impl FnMut(Vec2) -> bool) -> Option<Vec2> {
        if self.open.is_empty() {
            return None;
        }
        for _ in 0..200 {
            let a = rng.range(0.0, std::f32::consts::TAU);
            // Uniform over the ring's area.
            let r = (rng.range(rmin * rmin, rmax * rmax)).sqrt();
            let p = center + Vec2::new(a.cos(), a.sin()) * r;
            if self.index(p).is_some_and(|i| self.walk[i] && self.sdf[i] > 0.6) && accept(p) {
                return Some(p);
            }
        }
        None
    }

    /// Walkable area within `r` of `center` (m²).
    pub fn area_near(&self, center: Vec2, r: f32) -> f32 {
        let (c0, c1) = (((center - Vec2::splat(r) - self.origin) / self.cell).floor(), ((center + Vec2::splat(r) - self.origin) / self.cell).ceil());
        let mut n = 0usize;
        for y in (c0.y.max(0.0) as usize)..(c1.y.max(0.0) as usize).min(self.h) {
            for x in (c0.x.max(0.0) as usize)..(c1.x.max(0.0) as usize).min(self.w) {
                let p = self.origin + (Vec2::new(x as f32, y as f32) + 0.5) * self.cell;
                if self.walk[y * self.w + x] && p.distance(center) <= r {
                    n += 1;
                }
            }
        }
        n as f32 * self.cell * self.cell
    }

    /// Distance across the roadway from `p` along `dir` to the next
    /// walkway (crossing a street), if there is one within `max` metres
    /// and nothing but roadway in between.
    pub fn crossing(&self, p: Vec2, dir: Vec2, max: f32) -> Option<f32> {
        let step = self.cell * 0.5;
        let mut t = 0.0;
        let mut left = false;
        while t < max {
            t += step;
            let q = p + dir * t;
            if self.walkable(q) {
                if left && self.sample(q).sdf > 0.6 {
                    return Some(t);
                }
            } else if self.ground(q) == Ground::Road {
                left = true;
            } else {
                return None;
            }
        }
        None
    }
}

/// Top-down height raster.
struct Raster {
    origin: Vec2,
    cell: f32,
    w: usize,
    h: usize,
    height: Vec<f32>,
}

impl Raster {
    fn put(&mut self, x: usize, y: usize, v: f32) {
        let c = &mut self.height[y * self.w + x];
        *c = c.max(v);
    }

    /// Cell range covering `[a, b]` along one axis.
    fn span(&self, a: f32, b: f32, axis: usize) -> std::ops::Range<usize> {
        let (o, n) = if axis == 0 { (self.origin.x, self.w) } else { (self.origin.y, self.h) };
        let lo = ((a - o) / self.cell - 0.5).ceil().max(0.0) as usize;
        let hi = (((b - o) / self.cell - 0.5).floor() + 1.0).clamp(0.0, n as f32) as usize;
        lo.min(n)..hi
    }

    fn centre(&self, x: usize, y: usize) -> Vec2 {
        self.origin + (Vec2::new(x as f32, y as f32) + 0.5) * self.cell
    }

    fn shape(&mut self, shape: &Shape, iso: &Iso) {
        match shape {
            Shape::TriMesh(m) => {
                for t in &m.indices {
                    let [a, b, c] = t.map(|i| iso.transform_point(m.vertices[i as usize]));
                    self.triangle(a, b, c);
                }
            }
            Shape::Cuboid { half } => {
                let top: Vec<Vec3> = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)].iter().map(|(x, z)| iso.transform_point(Vec3::new(half.x * x, half.y, half.z * z))).collect();
                self.triangle(top[0], top[1], top[2]);
                self.triangle(top[0], top[2], top[3]);
            }
            Shape::Capsule { half_height, radius } => {
                let top = iso.pos.y + half_height + radius;
                self.disk(Vec2::new(iso.pos.x, iso.pos.z), *radius, top);
            }
            Shape::Sphere { radius } => self.disk(Vec2::new(iso.pos.x, iso.pos.z), *radius, iso.pos.y + radius),
            other => {
                // Convex hulls and terrain: their bounding box (conservative).
                let b = other.aabb(iso);
                for y in self.span(b.min.z, b.max.z, 1) {
                    for x in self.span(b.min.x, b.max.x, 0) {
                        self.put(x, y, b.max.y);
                    }
                }
            }
        }
    }

    fn disk(&mut self, c: Vec2, r: f32, top: f32) {
        // At least the cell holding the centre.
        let r = r.max(self.cell * 0.75);
        for y in self.span(c.y - r, c.y + r, 1) {
            for x in self.span(c.x - r, c.x + r, 0) {
                if self.centre(x, y).distance(c) <= r {
                    self.put(x, y, top);
                }
            }
        }
    }

    fn triangle(&mut self, a: Vec3, b: Vec3, c: Vec3) {
        let n = (b - a).cross(c - a);
        let len = n.length();
        if len < 1e-9 {
            return;
        }
        if n.y.abs() / len < 0.3 {
            // Walls and kerbs: their footprint is a segment; mark the cells
            // it crosses with the top of the face.
            let top = a.y.max(b.y).max(c.y);
            let pts = [a, b, c].map(|p| Vec2::new(p.x, p.z));
            let (mut p, mut q, mut best) = (pts[0], pts[1], -1.0);
            for i in 0..3 {
                for j in i + 1..3 {
                    let d = pts[i].distance_squared(pts[j]);
                    if d > best {
                        (p, q, best) = (pts[i], pts[j], d);
                    }
                }
            }
            let steps = (p.distance(q) / (self.cell * 0.5)).ceil().max(1.0) as usize;
            for k in 0..=steps {
                let s = p.lerp(q, k as f32 / steps as f32);
                let g = ((s - self.origin) / self.cell).floor();
                if g.x >= 0.0 && g.y >= 0.0 && (g.x as usize) < self.w && (g.y as usize) < self.h {
                    self.put(g.x as usize, g.y as usize, top);
                }
            }
            return;
        }
        let (pa, pb, pc) = (Vec2::new(a.x, a.z), Vec2::new(b.x, b.z), Vec2::new(c.x, c.z));
        let det = (pb - pa).perp_dot(pc - pa);
        let (min, max) = (pa.min(pb).min(pc), pa.max(pb).max(pc));
        for y in self.span(min.y, max.y, 1) {
            for x in self.span(min.x, max.x, 0) {
                let p = self.centre(x, y);
                let w1 = (p - pa).perp_dot(pc - pa) / det;
                let w2 = (pb - pa).perp_dot(p - pa) / det;
                let w0 = 1.0 - w1 - w2;
                if w0 >= -1e-4 && w1 >= -1e-4 && w2 >= -1e-4 {
                    self.put(x, y, a.y * w0 + b.y * w1 + c.y * w2);
                }
            }
        }
    }
}

/// Euclidean distance (m) from every cell to the nearest `true` cell
/// (Felzenszwalb & Huttenlocher, exact, linear time).
pub fn distance(seed: &[bool], w: usize, h: usize, cell: f32) -> Vec<f32> {
    const INF: f32 = 1e12;
    let mut d: Vec<f32> = seed.iter().map(|s| if *s { 0.0 } else { INF }).collect();
    let mut buf_f = vec![0.0f32; w.max(h)];
    let mut buf_d = vec![0.0f32; w.max(h)];
    let mut v = vec![0usize; w.max(h)];
    let mut z = vec![0.0f32; w.max(h) + 1];
    for x in 0..w {
        for y in 0..h {
            buf_f[y] = d[y * w + x];
        }
        edt_1d(&buf_f[..h], &mut buf_d[..h], &mut v, &mut z);
        for y in 0..h {
            d[y * w + x] = buf_d[y];
        }
    }
    for y in 0..h {
        buf_f[..w].copy_from_slice(&d[y * w..(y + 1) * w]);
        edt_1d(&buf_f[..w], &mut buf_d[..w], &mut v, &mut z);
        d[y * w..(y + 1) * w].copy_from_slice(&buf_d[..w]);
    }
    d.iter().map(|s| s.sqrt() * cell).collect()
}

/// 1D squared distance transform of sampled function `f`.
fn edt_1d(f: &[f32], d: &mut [f32], v: &mut [usize], z: &mut [f32]) {
    let n = f.len();
    let mut k = 0;
    v[0] = 0;
    z[0] = f32::NEG_INFINITY;
    z[1] = f32::INFINITY;
    for q in 1..n {
        loop {
            let p = v[k];
            let s = ((f[q] + (q * q) as f32) - (f[p] + (p * p) as f32)) / (2.0 * q as f32 - 2.0 * p as f32);
            if s <= z[k] && k > 0 {
                k -= 1;
                continue;
            }
            if s <= z[k] {
                // k == 0: replace the only parabola.
                v[0] = q;
                z[0] = f32::NEG_INFINITY;
                z[1] = f32::INFINITY;
                break;
            }
            k += 1;
            v[k] = q;
            z[k] = s;
            z[k + 1] = f32::INFINITY;
            break;
        }
    }
    k = 0;
    for (q, out) in d.iter_mut().enumerate().take(n) {
        while z[k + 1] < q as f32 {
            k += 1;
        }
        let p = v[k];
        let dq = q as f32 - p as f32;
        *out = dq * dq + f[p];
    }
}

/// Maximum over a `(2r+1)²` window (separable).
fn max_filter(f: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
    let mut tmp = vec![0.0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let (a, b) = (x.saturating_sub(r), (x + r + 1).min(w));
            tmp[y * w + x] = f[y * w + a..y * w + b].iter().copied().fold(f32::MIN, f32::max);
        }
    }
    let mut out = vec![0.0f32; w * h];
    for y in 0..h {
        let (a, b) = (y.saturating_sub(r), (y + r + 1).min(h));
        for x in 0..w {
            out[y * w + x] = (a..b).map(|yy| tmp[yy * w + x]).fold(f32::MIN, f32::max);
        }
    }
    out
}

/// Keeps the 4-connected walkable patches that belong to the street: at
/// least `min_cells` big and touching the roadway. Courtyards closed in by
/// buildings (an Ensanche block's patio) and gaps between buildings go.
fn keep_street_patches(walk: &mut [bool], ground: &[Ground], w: usize, h: usize, min_cells: usize) {
    let mut label = vec![u32::MAX; w * h];
    let mut stack = Vec::new();
    let mut patch = Vec::new();
    for start in 0..w * h {
        if !walk[start] || label[start] != u32::MAX {
            continue;
        }
        patch.clear();
        stack.push(start);
        label[start] = start as u32;
        let mut street = false;
        while let Some(i) = stack.pop() {
            patch.push(i);
            let (x, y) = (i % w, i / w);
            street |= ground[i] == Ground::Road;
            let mut visit = |j: usize| {
                if walk[j] && label[j] == u32::MAX {
                    label[j] = start as u32;
                    stack.push(j);
                }
                street |= !walk[j] && ground[j] == Ground::Road;
            };
            if x > 0 {
                visit(i - 1);
            }
            if x + 1 < w {
                visit(i + 1);
            }
            if y > 0 {
                visit(i - w);
            }
            if y + 1 < h {
                visit(i + w);
            }
        }
        if patch.len() < min_cells || !street {
            for &i in &patch {
                walk[i] = false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Heights for a test street: a 10 m wide road between 3 m sidewalks
    /// and walls, plus a 5 m alley with no sidewalks.
    fn street(cell: f32) -> Walkable {
        let (w, h) = ((100.0 / cell) as usize, (60.0 / cell) as usize);
        let mut height = vec![0.0f32; w * h];
        for y in 0..h {
            for x in 0..w {
                let p = (Vec2::new(x as f32, y as f32) + 0.5) * cell;
                let v = if p.y < 20.0 {
                    // Buildings below z = 20, cut by a 5 m alley at x 30..35.
                    if (30.0..35.0).contains(&p.x) { 0.0 } else { 12.0 }
                } else if p.y < 23.0 || (36.0..39.0).contains(&p.y) {
                    0.15
                } else if p.y < 36.0 {
                    0.0
                } else {
                    12.0
                };
                height[y * w + x] = v;
            }
        }
        Walkable::from_heights(Vec2::ZERO, cell, w, h, height)
    }

    #[test]
    fn sidewalks_and_lanes_are_walkable_roads_are_not() {
        let wk = street(0.5);
        assert!(wk.walkable(Vec2::new(10.0, 21.5)), "sidewalk");
        assert!(wk.walkable(Vec2::new(10.0, 37.5)), "far sidewalk");
        assert!(!wk.walkable(Vec2::new(10.0, 29.0)), "roadway");
        assert!(!wk.walkable(Vec2::new(10.0, 10.0)), "building");
        assert!(wk.walkable(Vec2::new(32.5, 10.0)), "alley without sidewalks");
        // The alley joins the sidewalk it opens onto.
        assert!(wk.walkable(Vec2::new(32.5, 19.8)));
    }

    #[test]
    fn field_points_inward_and_measures_width() {
        let wk = street(0.5);
        let s = wk.sample(Vec2::new(10.0, 21.0));
        assert!(s.inward.y > 0.9, "near the wall the middle is +z: {s:?}");
        assert!((s.half - 1.5).abs() < 0.3, "3 m sidewalk: {s:?}");
        let s = wk.sample(Vec2::new(10.0, 22.6));
        assert!(s.inward.y < -0.9, "near the kerb the middle is -z: {s:?}");
        assert!((wk.sample(Vec2::new(10.0, 21.5)).height - 0.15).abs() < 1e-3);
        let s = wk.sample(Vec2::new(32.5, 10.0));
        assert!((s.half - 2.5).abs() < 0.4, "5 m alley: {s:?}");
    }

    #[test]
    fn closed_courtyards_are_not_streets() {
        let cell = 0.5;
        let (w, h) = ((80.0 / cell) as usize, (80.0 / cell) as usize);
        // A block of buildings (10..70) with a 30 m paved patio inside,
        // ringed by a sidewalk on a road.
        let height: Vec<f32> = (0..w * h)
            .map(|i| {
                let p = (Vec2::new((i % w) as f32, (i / w) as f32) + 0.5) * cell;
                let d = (p - Vec2::splat(40.0)).abs().max_element();
                if d < 15.0 { 0.15 } else if d < 30.0 { 15.0 } else if d < 33.0 { 0.15 } else { 0.0 }
            })
            .collect();
        let wk = Walkable::from_heights(Vec2::ZERO, cell, w, h, height);
        assert!(!wk.walkable(Vec2::splat(40.0)), "patio");
        assert!(wk.walkable(Vec2::new(40.0, 71.5)), "sidewalk");
    }

    #[test]
    fn crossing_finds_the_far_sidewalk() {
        let wk = street(0.5);
        let t = wk.crossing(Vec2::new(10.0, 22.5), Vec2::Y, 25.0).expect("crossing");
        assert!((13.0..16.0).contains(&t), "{t}");
        assert!(wk.crossing(Vec2::new(10.0, 22.5), -Vec2::Y, 25.0).is_none(), "into a building");
    }

    #[test]
    fn distance_transform_is_euclidean() {
        let (w, h) = (9, 9);
        let mut seed = vec![false; w * h];
        seed[4 * w + 4] = true;
        let d = distance(&seed, w, h, 1.0);
        assert!((d[0] - (32.0f32).sqrt()).abs() < 1e-4);
        assert!((d[4 * w + 8] - 4.0).abs() < 1e-4);
    }

    #[test]
    fn triangles_fill_their_footprint() {
        let mut r = Raster { origin: Vec2::ZERO, cell: 1.0, w: 10, h: 10, height: vec![f32::NEG_INFINITY; 100] };
        r.triangle(Vec3::new(0.0, 1.0, 0.0), Vec3::new(10.0, 1.0, 0.0), Vec3::new(0.0, 1.0, 10.0));
        let filled = r.height.iter().filter(|h| h.is_finite()).count();
        assert_eq!(filled, 55, "cells with centres under the triangle");
        assert!(r.height[0].is_finite() && !r.height[99].is_finite());
    }

    #[test]
    fn rasterises_collider_shapes() {
        let ground = (ColliderDesc::new(Shape::cuboid(Vec3::new(20.0, 1.0, 20.0))).translation(Vec3::new(0.0, -1.0, 0.0)), Iso::IDENTITY);
        let walk = forja_physics::TriMesh::new(
            vec![Vec3::new(-10.0, 0.15, -2.0), Vec3::new(10.0, 0.15, -2.0), Vec3::new(10.0, 0.15, 2.0), Vec3::new(-10.0, 0.15, 2.0)],
            vec![[0, 2, 1], [0, 3, 2]],
        );
        let tree = (ColliderDesc::new(Shape::capsule(1.5, 0.3)), Iso::from_pos(Vec3::new(0.0, 1.8, 0.0)));
        let wk = Walkable::from_colliders(&[ground, (ColliderDesc::new(Shape::TriMesh(std::sync::Arc::new(walk))), Iso::IDENTITY), tree], Vec2::ZERO, Vec2::splat(20.0), 0.5).unwrap();
        assert_eq!(wk.ground(Vec2::new(5.0, 0.0)), Ground::Pavement);
        assert_eq!(wk.ground(Vec2::new(5.0, 8.0)), Ground::Road);
        assert_eq!(wk.ground(Vec2::new(0.1, 0.1)), Ground::Obstacle, "tree trunk");
    }
}
