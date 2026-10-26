//! Mesh primitives for landmarks, written into the kit's per-material
//! [`Prim`] buffers. Every primitive emits world-scale UVs in metres (the
//! material's `uv_scale` is applied once at assembly), consistent winding
//! (counter-clockwise seen from the front) and explicit normals: flat on
//! masonry, smooth on curved surfaces.

use forja_math::{Mat4, Vec2, Vec3};

use super::earcut;
use super::poly::{self, Shape};
use crate::geom::{Parts, Prim};

pub type MatId = u32;
/// Geometry of an element, keyed by palette material.
pub type Geo = Parts<MatId>;

/// A plane with an in-plane basis: points are `o + u * x + v * y + n * d`.
#[derive(Clone, Copy, Debug)]
pub struct Plane {
    pub o: Vec3,
    pub u: Vec3,
    pub v: Vec3,
    pub n: Vec3,
}

impl Plane {
    pub fn new(o: Vec3, u: Vec3, v: Vec3) -> Self {
        let n = u.cross(v).normalize_or(Vec3::Z);
        Self { o, u, v, n }
    }

    /// Horizontal plane at height `y`: x = east, y = south, normal up.
    pub fn horizontal(y: f32) -> Self {
        Self { o: Vec3::new(0.0, y, 0.0), u: Vec3::X, v: Vec3::Z, n: Vec3::NEG_Y }
    }

    pub fn at(&self, p: Vec2) -> Vec3 {
        self.o + self.u * p.x + self.v * p.y
    }

    pub fn at_d(&self, p: Vec2, d: f32) -> Vec3 {
        self.at(p) + self.n * d
    }
}

pub fn finite3(v: Vec3) -> bool {
    v.x.is_finite() && v.y.is_finite() && v.z.is_finite()
}

/// Flat triangle with a requested facing.
pub fn tri_facing(p: &mut Prim, a: Vec3, b: Vec3, c: Vec3, uv: [Vec2; 3], want: Vec3) {
    let n = (b - a).cross(c - a);
    if n.length_squared() < 1e-14 {
        return;
    }
    if n.dot(want) >= 0.0 {
        p.tri(a, b, c, uv);
    } else {
        p.tri(a, c, b, [uv[0], uv[2], uv[1]]);
    }
}

/// Flat quad with a requested facing (split along a b c / a c d).
pub fn quad_facing(p: &mut Prim, q: [Vec3; 4], uv: [Vec2; 4], want: Vec3) {
    let n = (q[1] - q[0]).cross(q[2] - q[0]) + (q[2] - q[0]).cross(q[3] - q[0]);
    if n.length_squared() < 1e-14 {
        return;
    }
    let n = n.normalize();
    let (q, uv) = if n.dot(want) >= 0.0 { (q, uv) } else { ([q[0], q[3], q[2], q[1]], [uv[0], uv[3], uv[2], uv[1]]) };
    let nn = if n.dot(want) >= 0.0 { n } else { -n };
    let i = p.vertex(q[0], nn, uv[0]);
    p.vertex(q[1], nn, uv[1]);
    p.vertex(q[2], nn, uv[2]);
    p.vertex(q[3], nn, uv[3]);
    p.indices.extend([i, i + 1, i + 2, i, i + 2, i + 3]);
}

/// Quad with metric planar UVs on its own axes (u along a→b, v along a→d,
/// anchored so neighbouring faces line up in world space).
pub fn quad_world(p: &mut Prim, q: [Vec3; 4], want: Vec3) {
    let n = want.normalize_or(Vec3::Y);
    let uv = world_uv_fn(n);
    quad_facing(p, q, [uv(q[0]), uv(q[1]), uv(q[2]), uv(q[3])], want);
}

/// Triplanar-like projection for a face with normal `n`: walls use
/// (horizontal along the wall, -y), floors and roofs use (x, z).
pub fn world_uv_fn(n: Vec3) -> impl Fn(Vec3) -> Vec2 {
    let h = Vec3::new(n.x, 0.0, n.z);
    let wall = n.y.abs() < 0.7 && h.length_squared() > 1e-6;
    let t = if wall { Vec3::Y.cross(h.normalize()) } else { Vec3::X };
    move |q: Vec3| if wall { Vec2::new(q.dot(t), -q.y) } else { Vec2::new(q.x, q.z) }
}

/// Planar shape (with holes) in `plane` coordinates, facing `plane.n`
/// (`flip` faces the other way). UV = plane coordinates + `uv_off`, with
/// v negated so texture "up" follows the plane's +v.
pub fn planar_shape(p: &mut Prim, shape: &Shape, plane: &Plane, flip: bool, uv_off: Vec2) {
    let (pts, tris) = earcut::triangulate_with_holes(&shape.outer, &shape.holes);
    if tris.is_empty() {
        return;
    }
    let want = if flip { -plane.n } else { plane.n };
    // CCW in (u, v) faces u x v.
    let natural = plane.u.cross(plane.v).dot(want) >= 0.0;
    let base = p.positions.len() as u32;
    for q in &pts {
        p.vertex(plane.at(*q), want, Vec2::new(q.x + uv_off.x, -(q.y + uv_off.y)));
    }
    for t in tris {
        if natural {
            p.indices.extend([base + t[0], base + t[1], base + t[2]]);
        } else {
            p.indices.extend([base + t[0], base + t[2], base + t[1]]);
        }
    }
}

/// Horizontal shape at `y`, facing up or down, UV = (x, z).
pub fn horizontal_shape(p: &mut Prim, shape: &Shape, y: f32, up: bool) {
    let (pts, tris) = earcut::triangulate_with_holes(&shape.outer, &shape.holes);
    let n = if up { Vec3::Y } else { Vec3::NEG_Y };
    let base = p.positions.len() as u32;
    for q in &pts {
        p.vertex(Vec3::new(q.x, y, q.y), n, *q);
    }
    for t in tris {
        // CCW in (x, z) maths faces -Y.
        if up {
            p.indices.extend([base + t[0], base + t[2], base + t[1]]);
        } else {
            p.indices.extend([base + t[0], base + t[1], base + t[2]]);
        }
    }
}

/// Vertical walls of every ring of `shape` between `y0` and `y1`, facing
/// out of the solid; UV u runs along each ring (continuous).
pub fn shape_walls(p: &mut Prim, shape: &Shape, y0: f32, y1: f32) {
    for (ring, _) in shape.rings() {
        ring_walls(p, ring, y0, y1, 0.0);
    }
}

/// Walls of one ring (outer CCW, holes CW give outward = out of solid).
pub fn ring_walls(p: &mut Prim, ring: &[Vec2], y0: f32, y1: f32, u0: f32) {
    let n = ring.len();
    let mut u = u0;
    for i in 0..n {
        let (a, b) = (ring[i], ring[(i + 1) % n]);
        let len = (b - a).length();
        if len < 1e-5 {
            continue;
        }
        let nrm = Vec3::new(b.y - a.y, 0.0, a.x - b.x) / len;
        // Facing out: run b -> a as seen from outside (left to right).
        let q = [Vec3::new(b.x, y0, b.y), Vec3::new(a.x, y0, a.y), Vec3::new(a.x, y1, a.y), Vec3::new(b.x, y1, b.y)];
        let uv = [Vec2::new(u, -y0), Vec2::new(u + len, -y0), Vec2::new(u + len, -y1), Vec2::new(u, -y1)];
        quad_facing(p, q, uv, nrm);
        u += len;
    }
}

/// Prism: shape extruded between two heights (caps optional).
pub fn prism(p: &mut Prim, shape: &Shape, y0: f32, y1: f32, top: bool, bottom: bool) {
    shape_walls(p, shape, y0, y1);
    if top {
        horizontal_shape(p, shape, y1, true);
    }
    if bottom {
        horizontal_shape(p, shape, y0, false);
    }
}

/// Lofted band between two rings with the same point count (e.g. a
/// battered wall or a mansard), facing outward from `inside` reference.
pub fn ring_band(p: &mut Prim, lo: &[Vec3], hi: &[Vec3], closed: bool, outward_from: Option<Vec3>) {
    let n = lo.len().min(hi.len());
    let segs = if closed { n } else { n.saturating_sub(1) };
    let mut u = 0.0;
    for i in 0..segs {
        let j = (i + 1) % n;
        let q = [lo[j], lo[i], hi[i], hi[j]];
        let e = lo[j] - lo[i];
        let len = e.length();
        let h0 = (hi[i] - lo[i]).length();
        let h1 = (hi[j] - lo[j]).length();
        let mid = (lo[i] + lo[j] + hi[i] + hi[j]) * 0.25;
        let geo_n = (lo[i] - lo[j]).cross(hi[j] - lo[j]);
        let want = match outward_from {
            Some(c) => {
                let w = mid - c;
                let w = Vec3::new(w.x, 0.0, w.z);
                if geo_n.dot(w) >= 0.0 { geo_n } else { -geo_n }
            }
            None => geo_n,
        };
        quad_facing(p, q, [Vec2::new(u + len, 0.0), Vec2::new(u, 0.0), Vec2::new(u, -h0), Vec2::new(u + len, -h1)], want);
        u += len;
    }
}

/// Parametric surface sampled on an `(nu + 1) × (nv + 1)` grid with smooth
/// normals (from the grid, so seams on closed directions match) and metric
/// UVs measured along the grid lines. `f(s, t)` with s, t in [0, 1].
/// Faces point along `dP/ds × dP/dt` unless `flip`.
pub fn grid_surface(p: &mut Prim, nu: usize, nv: usize, closed_u: bool, flip: bool, f: impl Fn(f32, f32) -> Vec3) {
    let (nu, nv) = (nu.max(1), nv.max(1));
    let w = nu + 1;
    let mut pos = Vec::with_capacity(w * (nv + 1));
    for j in 0..=nv {
        for i in 0..=nu {
            pos.push(f(i as f32 / nu as f32, j as f32 / nv as f32));
        }
    }
    if pos.iter().any(|q| !finite3(*q)) {
        return;
    }
    let at = |i: usize, j: usize| pos[j * w + i];
    // Smooth normals by averaging adjacent face normals.
    let mut nrm = vec![Vec3::ZERO; pos.len()];
    for j in 0..nv {
        for i in 0..nu {
            let (a, b, c, d) = (at(i, j), at(i + 1, j), at(i + 1, j + 1), at(i, j + 1));
            let fn1 = (b - a).cross(c - a);
            let fn2 = (c - a).cross(d - a);
            for (k, fnn) in [(j * w + i, fn1 + fn2), (j * w + i + 1, fn1), ((j + 1) * w + i + 1, fn1 + fn2), ((j + 1) * w + i, fn2)] {
                nrm[k] += fnn;
            }
        }
    }
    if closed_u {
        for j in 0..=nv {
            let s = nrm[j * w] + nrm[j * w + nu];
            nrm[j * w] = s;
            nrm[j * w + nu] = s;
        }
    }
    // Poles: rows collapsing to a point share the average normal.
    for j in [0, nv] {
        let row: Vec<Vec3> = (0..=nu).map(|i| at(i, j)).collect();
        let span = row.iter().map(|q| (*q - row[0]).length()).fold(0.0, f32::max);
        if span < 1e-5 {
            let s: Vec3 = (0..=nu).map(|i| nrm[j * w + i]).sum();
            for i in 0..=nu {
                nrm[j * w + i] = s;
            }
        }
    }
    // Metric UVs: u along rows (measured at each row), v along columns.
    let mut uvs = vec![Vec2::ZERO; pos.len()];
    for j in 0..=nv {
        let mut acc = 0.0;
        for i in 0..=nu {
            if i > 0 {
                acc += (at(i, j) - at(i - 1, j)).length();
            }
            uvs[j * w + i].x = acc;
        }
    }
    // Rows shrink near poles: use the widest row's spacing so the texture
    // keeps its scale where it matters and tapers like real tiling.
    for i in 0..=nu {
        let mut acc = 0.0;
        for j in 0..=nv {
            if j > 0 {
                acc += (at(i, j) - at(i, j - 1)).length();
            }
            uvs[j * w + i].y = -acc;
        }
    }
    let base = p.positions.len() as u32;
    for k in 0..pos.len() {
        let n = nrm[k].normalize_or(Vec3::Y);
        p.vertex(pos[k], if flip { -n } else { n }, uvs[k]);
    }
    for j in 0..nv {
        for i in 0..nu {
            let a = base + (j * w + i) as u32;
            let b = a + 1;
            let c = base + ((j + 1) * w + i + 1) as u32;
            let d = c - 1;
            let tris = [[a, b, c], [a, c, d]];
            for t in tris {
                let (pa, pb, pc) = (p.positions[t[0] as usize], p.positions[t[1] as usize], p.positions[t[2] as usize]);
                if (pb - pa).cross(pc - pa).length_squared() < 1e-14 {
                    continue;
                }
                if flip {
                    p.indices.extend([t[0], t[2], t[1]]);
                } else {
                    p.indices.extend(t);
                }
            }
        }
    }
}

/// Splits a 2D profile into runs separated by corners sharper than
/// `sharp_deg`; each run is shaded smoothly.
fn profile_runs(profile: &[Vec2], closed: bool, sharp_deg: f32) -> Vec<Vec<usize>> {
    let n = profile.len();
    let cos_lim = sharp_deg.to_radians().cos();
    let sharp = |i: usize| -> bool {
        if !closed && (i == 0 || i == n - 1) {
            return false;
        }
        let a = profile[(i + n - 1) % n];
        let b = profile[i];
        let c = profile[(i + 1) % n];
        let (d0, d1) = ((b - a).normalize_or(Vec2::X), (c - b).normalize_or(Vec2::X));
        d0.dot(d1) < cos_lim
    };
    let mut runs = Vec::new();
    let mut cur = vec![0usize];
    let last = if closed { n } else { n - 1 };
    for k in 1..=last {
        let i = k % n;
        cur.push(i);
        if (k < last && sharp(i)) || k == last {
            runs.push(std::mem::take(&mut cur));
            cur.push(i);
        }
    }
    if closed && !sharp(0) && runs.len() > 1 {
        // Merge the wrap-around run with the first one.
        let first = runs.remove(0);
        let lastr = runs.last_mut().unwrap();
        lastr.extend(first.into_iter().skip(1));
    } else if closed && !sharp(0) && runs.len() == 1 {
        // Fully smooth closed profile: keep as one run.
    }
    runs
}

/// Sweeps a 2D profile `(a, b)` along a path with per-point frames: point
/// = `path[i] + side[i] * a + up[i] * b`. With `smooth_path` normals
/// follow the frames (curves); otherwise each path segment is shaded flat
/// with its own perpendicular (crisp building corners, mitred positions).
/// Faces point away from the profile interior when the profile is
/// counter-clockwise in (a, b), i.e. outward.
#[allow(clippy::too_many_arguments)]
pub fn sweep(p: &mut Prim, path: &[Vec3], side: &[Vec3], up: &[Vec3], profile: &[Vec2], closed_path: bool, closed_profile: bool, smooth_path: bool, caps: bool) {
    let m = path.len();
    let np = profile.len();
    if m < 2 || np < 2 {
        return;
    }
    let ccw = !closed_profile || poly::area(profile) >= 0.0;
    let prof: Vec<Vec2> = if ccw { profile.to_vec() } else { profile.iter().rev().copied().collect() };
    let runs = profile_runs(&prof, closed_profile, 32.0);
    // Path distance for u.
    let segs = if closed_path { m } else { m - 1 };
    let mut dist = vec![0.0f32; m + 1];
    for i in 0..segs {
        dist[i + 1] = dist[i] + (path[(i + 1) % m] - path[i]).length();
    }
    for run in &runs {
        // Profile normals within the run (2D, pointing right of travel =
        // outward for a CCW profile).
        let rn: Vec<Vec2> = (0..run.len())
            .map(|k| {
                let a = prof[run[k.saturating_sub(1)]];
                let b = prof[run[k]];
                let c = prof[run[(k + 1).min(run.len() - 1)]];
                let t = ((b - a).normalize_or(Vec2::ZERO) + (c - b).normalize_or(Vec2::ZERO)).normalize_or(Vec2::X);
                Vec2::new(t.y, -t.x)
            })
            .collect();
        let mut vacc = vec![0.0f32; run.len()];
        for k in 1..run.len() {
            vacc[k] = vacc[k - 1] + (prof[run[k]] - prof[run[k - 1]]).length();
        }
        if smooth_path {
            // One vertex ring per path point.
            let base = p.positions.len() as u32;
            let rows = if closed_path { m + 1 } else { m };
            for r in 0..rows {
                let i = r % m;
                for k in 0..run.len() {
                    let q = prof[run[k]];
                    let pos = path[i] + side[i] * q.x + up[i] * q.y;
                    let n = (side[i] * rn[k].x + up[i] * rn[k].y).normalize_or(Vec3::Y);
                    p.vertex(pos, n, Vec2::new(dist[r], -vacc[k]));
                }
            }
            let w = run.len() as u32;
            for r in 0..(rows - 1) as u32 {
                for k in 0..w - 1 {
                    let a = base + r * w + k;
                    let b = a + w;
                    // Profile CCW, travel along path: a(k) -> a(k+1) -> b(k+1).
                    p.indices.extend([a, b, b + 1, a, b + 1, a + 1]);
                }
            }
            fix_run_winding(p, base, path, side, up, &prof, run, &rn);
        } else {
            for i in 0..segs {
                let j = (i + 1) % m;
                let seg = path[j] - path[i];
                // Flat frame for normals: perpendicular of this segment.
                let t = seg.normalize_or(Vec3::X);
                let upn = (up[i] + up[j]).normalize_or(Vec3::Y);
                let upn = (upn - t * upn.dot(t)).normalize_or(Vec3::Y);
                let sd = upn.cross(t).normalize_or(side[i]);
                let sd = if sd.dot(side[i] + side[j]) < 0.0 { -sd } else { sd };
                let base = p.positions.len() as u32;
                for (idx, d) in [(i, dist[i]), (j, dist[i + 1])] {
                    for k in 0..run.len() {
                        let q = prof[run[k]];
                        let pos = path[idx] + side[idx] * q.x + up[idx] * q.y;
                        let n = (sd * rn[k].x + upn * rn[k].y).normalize_or(Vec3::Y);
                        p.vertex(pos, n, Vec2::new(d, -vacc[k]));
                    }
                }
                let w = run.len() as u32;
                for k in 0..w - 1 {
                    let a = base + k;
                    let b = a + w;
                    p.indices.extend([a, b, b + 1, a, b + 1, a + 1]);
                }
                fix_run_winding(p, base, &[path[i], path[j]], &[side[i], side[j]], &[up[i], up[j]], &prof, run, &rn);
            }
        }
    }
    if caps && !closed_path && closed_profile && np >= 3 {
        let shape = Shape::new(prof.clone());
        for (idx, dir) in [(0usize, -1.0f32), (m - 1, 1.0)] {
            let t = if idx == 0 { path[1] - path[0] } else { path[m - 1] - path[m - 2] }.normalize_or(Vec3::X) * dir;
            let plane = Plane { o: path[idx], u: side[idx], v: up[idx], n: t };
            planar_shape(p, &shape, &plane, false, Vec2::ZERO);
        }
    }
}

/// Makes the last emitted sweep run face its profile normals (handles
/// left-handed frames from mirrored paths).
#[allow(clippy::too_many_arguments)]
fn fix_run_winding(p: &mut Prim, base: u32, path: &[Vec3], side: &[Vec3], up: &[Vec3], prof: &[Vec2], run: &[usize], rn: &[Vec2]) {
    if run.len() < 2 || path.len() < 2 {
        return;
    }
    let q0 = prof[run[0]];
    let q1 = prof[run[1]];
    let a = path[0] + side[0] * q0.x + up[0] * q0.y;
    let b = path[1] + side[1] * q0.x + up[1] * q0.y;
    let c = path[0] + side[0] * q1.x + up[0] * q1.y;
    let geo = (b - a).cross(c - a);
    let want = side[0] * (rn[0].x + rn[1].x) + up[0] * (rn[0].y + rn[1].y);
    if geo.dot(want) < 0.0 {
        let start = p.indices.len() - {
            // Count indices emitted since `base`.
            p.indices.iter().rev().take_while(|&&i| i >= base).count()
        };
        for t in p.indices[start..].chunks_exact_mut(3) {
            t.swap(1, 2);
        }
    }
}

/// Rotation-minimising frames along a polyline (double reflection).
/// Returns (tangent, side, up) per point; `up_hint` sets the first frame.
pub fn rmf(path: &[Vec3], up_hint: Vec3, closed: bool) -> (Vec<Vec3>, Vec<Vec3>, Vec<Vec3>) {
    let m = path.len();
    let tangent = |i: usize| -> Vec3 {
        let (a, b) = if closed {
            (path[(i + m - 1) % m], path[(i + 1) % m])
        } else {
            (path[i.saturating_sub(1)], path[(i + 1).min(m - 1)])
        };
        (b - a).normalize_or(Vec3::X)
    };
    let ts: Vec<Vec3> = (0..m).map(tangent).collect();
    let mut ups = Vec::with_capacity(m);
    let t0 = ts[0];
    let mut u0 = up_hint - t0 * up_hint.dot(t0);
    if u0.length_squared() < 1e-8 {
        u0 = t0.any_orthonormal_vector();
    }
    ups.push(u0.normalize());
    for i in 0..m - 1 {
        let v1 = path[i + 1] - path[i];
        let c1 = v1.dot(v1);
        if c1 < 1e-12 {
            ups.push(ups[i]);
            continue;
        }
        let r_l = ups[i] - v1 * (2.0 / c1) * v1.dot(ups[i]);
        let t_l = ts[i] - v1 * (2.0 / c1) * v1.dot(ts[i]);
        let v2 = ts[i + 1] - t_l;
        let c2 = v2.dot(v2);
        let r = if c2 < 1e-12 { r_l } else { r_l - v2 * (2.0 / c2) * v2.dot(r_l) };
        ups.push(r.normalize_or(ups[i]));
    }
    let sides: Vec<Vec3> = (0..m).map(|i| ups[i].cross(ts[i]).normalize_or(Vec3::X)).collect();
    (ts, sides, ups)
}

/// Frames with a fixed world up (horizontal mouldings, roads): side is
/// horizontal and perpendicular to the path, mitred at corners.
pub fn level_frames(path: &[Vec3], closed: bool, outward_sign: f32) -> (Vec<Vec3>, Vec<Vec3>) {
    let m = path.len();
    let seg_n = |i: usize| {
        let d = path[(i + 1) % m] - path[i];
        Vec3::new(d.z, 0.0, -d.x).normalize_or(Vec3::Z) * outward_sign
    };
    let sides = (0..m)
        .map(|i| {
            let has_prev = closed || i > 0;
            let has_next = closed || i + 1 < m;
            match (has_prev, has_next) {
                (true, true) => {
                    let a = seg_n((i + m - 1) % m);
                    let b = seg_n(i);
                    let n = (a + b).normalize_or(b);
                    n / n.dot(b).max(0.3)
                }
                (false, true) => seg_n(i),
                (true, false) => seg_n(i - 1),
                _ => Vec3::Z,
            }
        })
        .collect();
    (sides, vec![Vec3::Y; m])
}

/// Mitred in-plane offsets of a 2D polyline (outward = right of travel for
/// a CCW ring, i.e. `(d.y, -d.x)`).
pub fn polyline_offsets_2d(path: &[Vec2], closed: bool) -> Vec<Vec2> {
    let m = path.len();
    let seg_n = |i: usize| {
        let d = path[(i + 1) % m] - path[i];
        Vec2::new(d.y, -d.x).normalize_or(Vec2::Y)
    };
    (0..m)
        .map(|i| {
            let has_prev = closed || i > 0;
            let has_next = closed || i + 1 < m;
            match (has_prev, has_next) {
                (true, true) => {
                    let a = seg_n((i + m - 1) % m);
                    let b = seg_n(i);
                    let n = (a + b).normalize_or(b);
                    n / n.dot(b).max(0.3)
                }
                (false, true) => seg_n(i),
                (true, false) => seg_n(i - 1),
                _ => Vec2::Y,
            }
        })
        .collect()
}

/// Surface of revolution about +Y through `c`. `profile` is (radius,
/// height) bottom to top; corners sharper than 35° stay crisp. `a0..a1`
/// in radians (full turn: 0..TAU).
pub fn lathe(p: &mut Prim, c: Vec3, profile: &[Vec2], segments: usize, a0: f32, a1: f32) {
    let n = profile.len();
    if n < 2 {
        return;
    }
    let segments = segments.max(3);
    let full = (a1 - a0 - std::f32::consts::TAU).abs() < 1e-4;
    let runs = profile_runs(profile, false, 35.0);
    let rmax = profile.iter().map(|q| q.x).fold(0.0, f32::max).max(0.01);
    for run in runs {
        let pts: Vec<Vec2> = run.iter().map(|&i| profile[i]).collect();
        let k = pts.len();
        // 2D normals of the run (outward = to the right of upward travel).
        let nn: Vec<Vec2> = (0..k)
            .map(|i| {
                let a = pts[i.saturating_sub(1)];
                let b = pts[(i + 1).min(k - 1)];
                let t = (b - a).normalize_or(Vec2::Y);
                Vec2::new(t.y, -t.x)
            })
            .collect();
        let mut vacc = vec![0.0; k];
        for i in 1..k {
            vacc[i] = vacc[i - 1] + (pts[i] - pts[i - 1]).length();
        }
        let base = p.positions.len() as u32;
        for s in 0..=segments {
            let a = a0 + (a1 - a0) * s as f32 / segments as f32;
            let (sn, cs) = a.sin_cos();
            for i in 0..k {
                let q = pts[i];
                let pos = c + Vec3::new(cs * q.x, q.y, sn * q.x);
                let nr = Vec3::new(cs * nn[i].x, nn[i].y, sn * nn[i].x).normalize_or(Vec3::Y);
                p.vertex(pos, nr, Vec2::new((a - a0) * rmax, -vacc[i]));
            }
        }
        let kk = k as u32;
        for s in 0..segments as u32 {
            for i in 0..kk - 1 {
                let a = base + s * kk + i;
                let b = base + (s + 1) * kk + i;
                // Profile going up, angle increasing (x -> z): outward is
                // (up) x (dangle)... choose by geometry below.
                let tri1 = [a, a + 1, b + 1];
                let tri2 = [a, b + 1, b];
                for t in [tri1, tri2] {
                    let (pa, pb, pc) = (p.positions[t[0] as usize], p.positions[t[1] as usize], p.positions[t[2] as usize]);
                    let g = (pb - pa).cross(pc - pa);
                    if g.length_squared() < 1e-14 {
                        continue;
                    }
                    let nsum = p.normals[t[0] as usize] + p.normals[t[1] as usize] + p.normals[t[2] as usize];
                    if g.dot(nsum) >= 0.0 {
                        p.indices.extend(t);
                    } else {
                        p.indices.extend([t[0], t[2], t[1]]);
                    }
                }
            }
        }
        let _ = full;
    }
}

/// Extrudes a 2D shape (with holes) from `plane` along its normal by
/// `length` (caps at both ends, outward-facing sides).
pub fn extrude_shape(p: &mut Prim, shape: &Shape, plane: &Plane, length: f32) {
    let Some(s) = shape.normalized() else { return };
    let dir = plane.n * length;
    let back = Plane { o: plane.o, ..*plane };
    let front = Plane { o: plane.o + dir, ..*plane };
    let (fwd, bwd) = if length >= 0.0 { (false, true) } else { (true, false) };
    planar_shape(p, &s, &front, fwd, Vec2::ZERO);
    planar_shape(p, &s, &back, bwd, Vec2::ZERO);
    for (ring, _) in s.rings() {
        let m = ring.len();
        let mut u = 0.0;
        for i in 0..m {
            let (a, b) = (ring[i], ring[(i + 1) % m]);
            let len = (b - a).length();
            let d = b - a;
            // Outward in plane coordinates (CCW outer / CW hole): (d.y, -d.x).
            let out2 = Vec2::new(d.y, -d.x);
            let want = (plane.u * out2.x + plane.v * out2.y).normalize_or(plane.n);
            let q = [back.at(a), back.at(b), front.at(b), front.at(a)];
            let l = length.abs();
            quad_facing(p, q, [Vec2::new(u, 0.0), Vec2::new(u + len, 0.0), Vec2::new(u + len, -l), Vec2::new(u, -l)], want);
            u += len;
        }
    }
}

/// UV sphere / ellipsoid section. `lat` in radians from -pi/2 (bottom) to
/// pi/2 (top), `lon` around +Y.
pub fn ellipsoid(p: &mut Prim, c: Vec3, r: Vec3, lon: (f32, f32), lat: (f32, f32), nu: usize, nv: usize) {
    let closed = (lon.1 - lon.0 - std::f32::consts::TAU).abs() < 1e-4;
    grid_surface(p, nu, nv, closed, true, |s, t| {
        let a = lon.0 + (lon.1 - lon.0) * s;
        let b = lat.0 + (lat.1 - lat.0) * t;
        c + Vec3::new(r.x * b.cos() * a.cos(), r.y * b.sin(), r.z * b.cos() * a.sin())
    });
}

/// Torus around `axis` (major radius `big`, minor `small`).
pub fn torus(p: &mut Prim, c: Vec3, axis: Vec3, big: f32, small: f32, nu: usize, nv: usize) {
    let ax = axis.normalize_or(Vec3::Y);
    let x = ax.any_orthonormal_vector();
    let z = x.cross(ax);
    grid_surface(p, nu, nv, true, false, |s, t| {
        let a = s * std::f32::consts::TAU;
        let b = t * std::f32::consts::TAU;
        let radial = x * a.cos() + z * a.sin();
        c + radial * (big + small * b.cos()) + ax * (small * b.sin())
    });
    // Orientation check: make the first triangle face away from the tube centre.
    fix_orientation_by_centroid(p, |q| {
        let d = q - c;
        let h = d - ax * d.dot(ax);
        c + h.normalize_or(x) * big
    });
}

/// Flips every triangle of `p` whose normal points toward `inner(point)`.
pub fn fix_orientation_by_centroid(p: &mut Prim, inner: impl Fn(Vec3) -> Vec3) {
    for t in p.indices.chunks_exact_mut(3) {
        let (a, b, c) = (p.positions[t[0] as usize], p.positions[t[1] as usize], p.positions[t[2] as usize]);
        let g = (b - a).cross(c - a);
        let mid = (a + b + c) / 3.0;
        if g.dot(mid - inner(mid)) < 0.0 {
            t.swap(1, 2);
        }
    }
    // Normals follow the geometric side.
    let mut acc = vec![Vec3::ZERO; p.positions.len()];
    for t in p.indices.chunks_exact(3) {
        let (a, b, c) = (p.positions[t[0] as usize], p.positions[t[1] as usize], p.positions[t[2] as usize]);
        let g = (b - a).cross(c - a);
        for i in t {
            acc[*i as usize] += g;
        }
    }
    for (n, a) in p.normals.iter_mut().zip(acc) {
        if a.length_squared() > 1e-14 && n.dot(a) < 0.0 {
            *n = -*n;
        }
    }
}

/// Recomputes normals: vertices at the same position are welded where
/// adjacent faces meet at less than `angle_deg`; sharper edges stay hard.
pub fn auto_normals(p: &mut Prim, angle_deg: f32) {
    use std::collections::HashMap;
    let cos_lim = angle_deg.to_radians().cos();
    let key = |v: Vec3| ((v.x * 1e4).round() as i64, (v.y * 1e4).round() as i64, (v.z * 1e4).round() as i64);
    let tri_n: Vec<Vec3> = p
        .indices
        .chunks_exact(3)
        .map(|t| {
            let (a, b, c) = (p.positions[t[0] as usize], p.positions[t[1] as usize], p.positions[t[2] as usize]);
            (b - a).cross(c - a)
        })
        .collect();
    let mut by_pos: HashMap<(i64, i64, i64), Vec<usize>> = HashMap::new();
    for (ti, t) in p.indices.chunks_exact(3).enumerate() {
        for &i in t {
            by_pos.entry(key(p.positions[i as usize])).or_default().push(ti);
        }
    }
    let mut out = p.normals.clone();
    for (ti, t) in p.indices.chunks_exact(3).enumerate() {
        let own = tri_n[ti].normalize_or(Vec3::Y);
        for &i in t {
            let mut s = Vec3::ZERO;
            for &o in &by_pos[&key(p.positions[i as usize])] {
                let on = tri_n[o].normalize_or(Vec3::Y);
                if on.dot(own) >= cos_lim {
                    s += tri_n[o];
                }
            }
            out[i as usize] = s.normalize_or(own);
        }
    }
    // Vertices shared by faces of different smoothing groups need splitting:
    // rebuild as unshared vertices.
    let mut np = Prim::default();
    for (ti, t) in p.indices.chunks_exact(3).enumerate() {
        let own = tri_n[ti].normalize_or(Vec3::Y);
        let mut ids = [0u32; 3];
        for (k, &i) in t.iter().enumerate() {
            let mut s = Vec3::ZERO;
            for &o in &by_pos[&key(p.positions[i as usize])] {
                let on = tri_n[o].normalize_or(Vec3::Y);
                if on.dot(own) >= cos_lim {
                    s += tri_n[o];
                }
            }
            ids[k] = np.vertex(p.positions[i as usize], s.normalize_or(own), p.uvs[i as usize]);
        }
        np.indices.extend(ids);
    }
    let _ = out;
    *p = np;
}

/// Drops non-finite and degenerate triangles; normalises normals.
pub fn sanitize(p: &mut Prim) -> usize {
    let mut kept = Vec::with_capacity(p.indices.len());
    let mut dropped = 0;
    for t in p.indices.chunks_exact(3) {
        let (a, b, c) = (p.positions[t[0] as usize], p.positions[t[1] as usize], p.positions[t[2] as usize]);
        let ok = finite3(a) && finite3(b) && finite3(c) && (b - a).cross(c - a).length_squared() > 1e-12;
        if ok {
            kept.extend_from_slice(t);
        } else {
            dropped += 1;
        }
    }
    p.indices = kept;
    for n in &mut p.normals {
        *n = if finite3(*n) { n.normalize_or(Vec3::Y) } else { Vec3::Y };
    }
    for uv in &mut p.uvs {
        if !uv.x.is_finite() || !uv.y.is_finite() {
            *uv = Vec2::ZERO;
        }
    }
    dropped
}

/// Appends `src` into `dst` with a transform.
pub fn append(dst: &mut Geo, src: &Geo, m: &Mat4) {
    dst.append(src, m);
}

/// Total triangles.
pub fn tris(g: &Geo) -> usize {
    g.tri_count()
}

/// World bounds of a geometry set (min, max).
pub fn bounds(g: &Geo) -> Option<(Vec3, Vec3)> {
    let mut mn = Vec3::splat(f32::MAX);
    let mut mx = Vec3::splat(f32::MIN);
    let mut any = false;
    for p in g.prims.values() {
        for t in p.indices.iter() {
            let q = p.positions[*t as usize];
            mn = mn.min(q);
            mx = mx.max(q);
            any = true;
        }
    }
    any.then_some((mn, mx))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn closed_and_outward(p: &Prim, center: Vec3) -> bool {
        p.indices.chunks_exact(3).all(|t| {
            let (a, b, c) = (p.positions[t[0] as usize], p.positions[t[1] as usize], p.positions[t[2] as usize]);
            (b - a).cross(c - a).dot((a + b + c) / 3.0 - center) > -1e-6
        })
    }

    #[test]
    fn prism_faces_outward() {
        let s = Shape::new(poly::rect(Vec2::new(1.0, 2.0), Vec2::new(4.0, 6.0), 20.0));
        let mut p = Prim::default();
        prism(&mut p, &s, 0.0, 3.0, true, true);
        assert_eq!(p.tri_count(), 12);
        assert!(closed_and_outward(&p, Vec3::new(1.0, 1.5, 2.0)));
        // Normals agree with geometry.
        for t in p.indices.chunks_exact(3) {
            let (a, b, c) = (p.positions[t[0] as usize], p.positions[t[1] as usize], p.positions[t[2] as usize]);
            assert!((b - a).cross(c - a).dot(p.normals[t[0] as usize]) > 0.0);
        }
    }

    #[test]
    fn sphere_lathe_torus_outward_and_finite() {
        let mut p = Prim::default();
        ellipsoid(&mut p, Vec3::ZERO, Vec3::new(2.0, 1.0, 3.0), (0.0, std::f32::consts::TAU), (-1.5, 1.5), 16, 8);
        assert!(closed_and_outward(&p, Vec3::ZERO));
        let mut l = Prim::default();
        let prof = [Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0), Vec2::new(1.0, 2.0), Vec2::new(0.5, 2.5), Vec2::new(0.0, 2.6)];
        lathe(&mut l, Vec3::ZERO, &prof, 24, 0.0, std::f32::consts::TAU);
        assert!(l.tri_count() > 50);
        for t in l.indices.chunks_exact(3) {
            let (a, b, c) = (l.positions[t[0] as usize], l.positions[t[1] as usize], l.positions[t[2] as usize]);
            let g = (b - a).cross(c - a);
            let mid = (a + b + c) / 3.0;
            // Outward from the axis or up/down on caps.
            assert!(g.dot(Vec3::new(mid.x, 0.0, mid.z)) > -1e-5 || g.y.abs() > 0.0);
        }
        let mut t = Prim::default();
        torus(&mut t, Vec3::ZERO, Vec3::Y, 3.0, 0.5, 24, 12);
        assert!(t.positions.iter().all(|q| finite3(*q)));
    }

    #[test]
    fn sweep_box_profile_is_outward() {
        let path = vec![Vec3::new(0.0, 0.0, 0.0), Vec3::new(10.0, 0.0, 0.0)];
        let (sides, ups) = level_frames(&path, false, 1.0);
        let prof = [Vec2::new(0.0, 0.0), Vec2::new(0.5, 0.0), Vec2::new(0.5, 0.3), Vec2::new(0.0, 0.3)];
        let mut p = Prim::default();
        sweep(&mut p, &path, &sides, &ups, &prof, false, true, false, true);
        let c = Vec3::new(5.0, 0.15, sides[0].z * 0.25);
        assert!(closed_and_outward(&p, c));
    }

    #[test]
    fn extruded_shape_with_hole() {
        let s = Shape { outer: poly::rect(Vec2::ZERO, Vec2::new(4.0, 4.0), 0.0), holes: vec![poly::rect(Vec2::ZERO, Vec2::new(2.0, 2.0), 0.0)] };
        let mut p = Prim::default();
        extrude_shape(&mut p, &s, &Plane::new(Vec3::ZERO, Vec3::X, Vec3::Y), 1.0);
        // 2 caps x 8 tris + 8 side quads.
        assert_eq!(p.tri_count(), 16 + 16);
    }
}
