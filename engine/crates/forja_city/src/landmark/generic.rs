//! Generic geometry: boxes, cylinders and cones, spheres, tori, lathes,
//! extrusions, sweeps, lofts, parametric shells with thickness (clipped
//! by planes, with ribs) and raw meshes.

use std::collections::HashMap;

use forja_math::{Mat4, Vec2, Vec3};

use super::build::{Builder, Out, Scope};
use super::mesh::{self, Plane};
use super::parse::{Error, Result};
use super::poly::{self, Shape};
use super::shapes;
use super::spec::*;
use crate::geom::Prim;

pub fn box_el(_b: &mut Builder, x: &BoxEl, sc: &Scope, o: &mut Out) -> Result<()> {
    let (mn, mx) = match (x.min, x.max, x.center, x.size) {
        (Some(a), Some(c), _, _) => (Vec3::from_array(a), Vec3::from_array(c)),
        (_, _, c, Some(s)) => {
            let c = Vec3::from_array(c.unwrap_or([0.0, s[1] * 0.5, 0.0]));
            let h = Vec3::from_array(s) * 0.5;
            (c - h, c + h)
        }
        _ => return Err(Error::msg("box needs `size` (with optional `center`) or `min` and `max`")),
    };
    let (mn, mx) = (mn.min(mx), mn.max(mx));
    if (mx - mn).min_element() <= 0.0 {
        return Err(Error::msg("box has zero size"));
    }
    o.geo.get(sc.mat).aabox(mn, mx, 1.0);
    Ok(())
}

pub fn cylinder(b: &mut Builder, c: &Cylinder, cone: bool, sc: &Scope, o: &mut Out) -> Result<()> {
    let (a, e) = match (c.from, c.to) {
        (Some(f), Some(t)) => (Vec3::from_array(f), Vec3::from_array(t)),
        _ => {
            let a = Vec3::from_array(c.at);
            let h = c.height.ok_or_else(|| Error::msg("cylinder needs `height` (or `from` and `to`)"))?;
            (a, a + Vec3::Y * h)
        }
    };
    if (e - a).length() < 1e-4 {
        return Err(Error::msg("cylinder has zero length"));
    }
    let r1 = c.radius_top.unwrap_or(if cone { 0.0 } else { c.radius });
    let seg = c.segments.map(|s| s as usize).unwrap_or_else(|| b.segs(c.radius.max(r1), 8, 64)).max(3);
    let p = o.geo.get(sc.mat);
    if c.faceted {
        let axis = (e - a).normalize();
        let x = axis.any_orthonormal_vector();
        let z = axis.cross(x);
        let ring = poly::regular(Vec2::ZERO, 1.0, seg, 0.0);
        let mut pr = Prim::default();
        let lo: Vec<Vec3> = ring.iter().map(|q| a + (x * q.x + z * q.y) * c.radius).collect();
        let hi: Vec<Vec3> = ring.iter().map(|q| e + (x * q.x + z * q.y) * r1.max(1e-4)).collect();
        mesh::ring_band(&mut pr, &lo, &hi, true, None);
        mesh::fix_orientation_by_centroid(&mut pr, |q| {
            let t = (q - a).dot(axis);
            a + axis * t
        });
        p.append(&pr, &Mat4::IDENTITY);
        if c.caps {
            p.cone(a, a + (e - a) * 1e-4, c.radius, c.radius, seg as u32, true);
        }
    } else {
        p.cone(a, e, c.radius, r1, seg as u32, c.caps);
    }
    Ok(())
}

pub fn sphere(b: &mut Builder, s: &Sphere, sc: &Scope, o: &mut Out) -> Result<()> {
    let r = match (s.radii, s.radius) {
        (Some(r), _) => Vec3::from_array(r),
        (None, Some(r)) => Vec3::splat(r),
        _ => return Err(Error::msg("sphere needs `radius` or `radii`")),
    };
    let lat = s.lat.unwrap_or([-90.0, 90.0]);
    let lon = s.lon.unwrap_or([0.0, 360.0]);
    let nu = s.segments.map(|v| v as usize).unwrap_or_else(|| b.segs(r.max_element(), 16, 96));
    let nv = (nu / 2).max(4);
    mesh::ellipsoid(o.geo.get(sc.mat), Vec3::from_array(s.center), r, (lon[0].to_radians(), lon[1].to_radians()), (lat[0].to_radians(), lat[1].to_radians()), nu, nv);
    Ok(())
}

pub fn torus(b: &mut Builder, t: &Torus, sc: &Scope, o: &mut Out) -> Result<()> {
    let nu = t.segments.map(|v| v as usize).unwrap_or_else(|| b.segs(t.radius, 16, 96));
    let mut p = Prim::default();
    mesh::torus(&mut p, Vec3::from_array(t.center), Vec3::from_array(t.axis.unwrap_or([0.0, 1.0, 0.0])), t.radius, t.tube, nu, b.segs(t.tube, 8, 24));
    o.geo.get(sc.mat).append(&p, &Mat4::IDENTITY);
    Ok(())
}

pub fn lathe(b: &mut Builder, l: &Lathe, sc: &Scope, o: &mut Out) -> Result<()> {
    if l.profile.len() < 2 {
        return Err(Error::msg("lathe profile needs at least 2 points [[r, y], ...]"));
    }
    if l.profile.iter().any(|p| p[0] < 0.0) {
        return Err(Error::msg("lathe profile radii must be >= 0"));
    }
    let prof: Vec<Vec2> = l.profile.iter().map(|p| Vec2::new(p[0], p[1])).collect();
    let rmax = prof.iter().map(|p| p.x).fold(0.0, f32::max);
    let seg = l.segments.map(|s| s as usize).unwrap_or_else(|| b.segs(rmax, 12, 96));
    let [a0, a1] = l.angle.unwrap_or([0.0, 360.0]);
    mesh::lathe(o.geo.get(sc.mat), Vec3::from_array(l.at), &prof, seg, a0.to_radians(), a1.to_radians());
    Ok(())
}

pub fn extrude(b: &mut Builder, x: &Extrude, sc: &Scope, o: &mut Out) -> Result<()> {
    let shapes = b.poly(&x.profile, &Scope { parent: None, ..sc.clone() })?;
    let o0 = Vec3::from_array(x.origin);
    let (u, v, n) = match x.plane.as_deref().unwrap_or("xy") {
        "xy" => (Vec3::X, Vec3::Y, Vec3::Z),
        "xz" => (Vec3::X, Vec3::Z, Vec3::Y),
        "zy" | "yz" => (Vec3::Z, Vec3::Y, Vec3::X),
        p => return Err(Error::msg(format!("bad extrude plane `{p}`; use xy, xz or zy"))),
    };
    let start = if x.center { o0 - n * (x.length * 0.5) } else { o0 };
    let plane = Plane { o: start, u, v, n };
    for s in &shapes {
        mesh::extrude_shape(o.geo.get(sc.mat), s, &plane, x.length);
    }
    Ok(())
}

/// Frames along a path: (side, up) per point, optionally level.
fn frames(path: &[Vec3], closed: bool, up_hint: Vec3, level: bool) -> (Vec<Vec3>, Vec<Vec3>, Vec<Vec3>) {
    let (ts, sides, ups) = mesh::rmf(path, up_hint, closed);
    if !level {
        return (ts, sides, ups);
    }
    let mut s2 = Vec::with_capacity(path.len());
    let mut u2 = Vec::with_capacity(path.len());
    for t in &ts {
        let side = t.cross(Vec3::Y);
        let side = if side.length_squared() < 1e-8 { Vec3::X } else { side.normalize() };
        let up = side.cross(*t).normalize_or(Vec3::Y);
        s2.push(-side);
        u2.push(if up.y < 0.0 { -up } else { up });
    }
    (ts, s2, u2)
}

pub fn sweep(b: &mut Builder, s: &Sweep, sc: &Scope, o: &mut Out) -> Result<()> {
    let (path, closed) = shapes::path_points(&s.path, b.detail).map_err(Error::msg)?;
    let res = b.segs(s.profile.size[0].max(s.profile.size[1]) * 0.5, 8, 32);
    let (prof, closed_prof) = shapes::section_points(&s.profile, res);
    let up_hint = Vec3::from_array(s.up.unwrap_or([0.0, 1.0, 0.0]));
    let (_ts, mut sides, mut ups) = frames(&path, closed, up_hint, s.level);
    let m = path.len();
    let total: f32 = (1..m).map(|i| (path[i] - path[i - 1]).length()).sum::<f32>().max(1e-6);
    let mut acc = 0.0;
    let end = s.scale_end.map(|v| v.0).unwrap_or([1.0; 3]);
    for i in 0..m {
        if i > 0 {
            acc += (path[i] - path[i - 1]).length();
        }
        let t = acc / total;
        let tw = (s.twist * t).to_radians();
        let (sn, cs) = tw.sin_cos();
        let (sd, up) = (sides[i], ups[i]);
        let sd2 = sd * cs + up * sn;
        let up2 = up * cs - sd * sn;
        let (kx, ky) = (1.0 + (end[0] - 1.0) * t, 1.0 + (end[1] - 1.0) * t);
        sides[i] = sd2 * kx;
        ups[i] = up2 * ky;
    }
    let smooth = s.path.points.is_none();
    mesh::sweep(o.geo.get(sc.mat), &path, &sides, &ups, &prof, closed, closed_prof, smooth, s.caps && !closed);
    Ok(())
}

pub fn loft(_b: &mut Builder, l: &Loft, sc: &Scope, o: &mut Out) -> Result<()> {
    if l.stations.len() < 2 {
        return Err(Error::msg("loft needs at least 2 stations"));
    }
    let res = l.resolution.clamp(4, 256) as usize;
    // Station sections resampled to `res` points.
    let mut secs: Vec<Vec<Vec2>> = Vec::new();
    for (k, st) in l.stations.iter().enumerate() {
        let sec = st.section.as_ref().or(l.section.as_ref()).ok_or_else(|| Error::msg(format!("loft station {k} has no section (set `section` on the loft or the station)")))?;
        let (pts, closed) = shapes::section_points(sec, res);
        if !closed {
            return Err(Error::msg("loft sections must be closed"));
        }
        let mut pts = shapes::resample(&pts, res, true);
        if let Some(sc2) = st.scale {
            pts = pts.into_iter().map(|q| q * Vec2::new(sc2[0], sc2[1])).collect();
        }
        if st.twist != 0.0 {
            let r = Vec2::from_angle(st.twist.to_radians());
            pts = pts.into_iter().map(|q| r.rotate(q)).collect();
        }
        // Start each section at its rightmost point for consistent joins.
        let start = (0..pts.len()).max_by(|&a, &c| (pts[a].x - pts[a].y * 1e-3).total_cmp(&(pts[c].x - pts[c].y * 1e-3))).unwrap_or(0);
        pts.rotate_left(start);
        secs.push(pts);
    }
    let centers: Vec<Vec3> = l.stations.iter().map(|s| Vec3::from_array(s.at)).collect();
    let n = centers.len();
    let per = l.samples.max(1) as usize;
    let spans = if l.closed { n } else { n - 1 };
    let get = |i: isize| -> usize {
        if l.closed {
            i.rem_euclid(n as isize) as usize
        } else {
            i.clamp(0, n as isize - 1) as usize
        }
    };
    let mut path = Vec::new();
    let mut rings: Vec<Vec<Vec2>> = Vec::new();
    for s in 0..spans as isize {
        let steps = if !l.closed && s as usize == spans - 1 { per + 1 } else { per };
        for k in 0..steps {
            let t = k as f32 / per as f32;
            let (i0, i1, i2, i3) = (get(s - 1), get(s), get(s + 1), get(s + 2));
            let p = if l.smooth {
                let pc = |i: usize| centers[i];
                let (a, b2, c, d) = (pc(i0), pc(i1), pc(i2), pc(i3));
                let a = if !l.closed && s == 0 { b2 * 2.0 - c } else { a };
                let d = if !l.closed && s as usize == spans - 1 { c * 2.0 - b2 } else { d };
                shapes::catmull(a, b2, c, d, t)
            } else {
                centers[i1].lerp(centers[i2], t)
            };
            path.push(p);
            let ring: Vec<Vec2> = (0..res)
                .map(|j| {
                    if l.smooth {
                        let q = |i: usize| secs[i][j].extend(0.0);
                        let (a, b2, c, d) = (q(i0), q(i1), q(i2), q(i3));
                        let a = if !l.closed && s == 0 { b2 * 2.0 - c } else { a };
                        let d = if !l.closed && s as usize == spans - 1 { c * 2.0 - b2 } else { d };
                        shapes::catmull(a, b2, c, d, t).truncate()
                    } else {
                        secs[i1][j].lerp(secs[i2][j], t)
                    }
                })
                .collect();
            rings.push(ring);
        }
    }
    let up_hint = Vec3::from_array(l.up.unwrap_or([0.0, 1.0, 0.0]));
    let (ts, sides, ups) = mesh::rmf(&path, up_hint, l.closed);
    let m = path.len();
    let pts: Vec<Vec<Vec3>> = (0..m).map(|i| rings[i].iter().map(|q| path[i] + sides[i] * q.x + ups[i] * q.y).collect()).collect();
    let mut p = Prim::default();
    let rows = if l.closed { m + 1 } else { m };
    mesh::grid_surface(&mut p, res, rows - 1, true, false, |s, t| {
        let i = ((s * res as f32).round() as usize) % res;
        let j = ((t * (rows - 1) as f32).round() as usize) % m;
        pts[j][i]
    });
    // Orient outward (away from the path).
    let mid = m / 2;
    let probe = pts[mid][0];
    let flip = p.positions.iter().zip(&p.normals).find(|(q, _)| (**q - probe).length() < 1e-4).is_some_and(|(q, nn)| nn.dot(*q - path[mid]) < 0.0);
    if flip {
        for t in p.indices.chunks_exact_mut(3) {
            t.swap(1, 2);
        }
        for nn in &mut p.normals {
            *nn = -*nn;
        }
    }
    if l.caps && !l.closed {
        for (i, dir) in [(0usize, -1.0f32), (m - 1, 1.0)] {
            let shape = Shape::new(rings[i].clone());
            if let Some(s2) = shape.normalized() {
                let plane = Plane { o: path[i], u: sides[i], v: ups[i], n: ts[i] * dir };
                mesh::planar_shape(&mut p, &s2, &plane, false, Vec2::ZERO);
            }
        }
    }
    o.geo.get(sc.mat).append(&p, &Mat4::IDENTITY);
    Ok(())
}

fn sgnpow(v: f32, e: f32) -> f32 {
    v.signum() * v.abs().powf(e)
}

/// Parametric point of a shell surface for s, t in [0, 1].
fn shell_point(s: &Shell, ss: f32, tt: f32) -> Result<Vec3> {
    let c = Vec3::from_array(s.center);
    let r = Vec3::from_array(s.radii.unwrap_or([s.radius.unwrap_or(5.0); 3]));
    let ur = s.u_range.unwrap_or([0.0, 360.0]);
    let lerp = |a: [f32; 2], t: f32| (a[0] + (a[1] - a[0]) * t).to_radians();
    Ok(match s.surface {
        SurfaceKind::Ellipsoid | SurfaceKind::Superellipsoid => {
            let vr = s.v_range.unwrap_or([0.0, 90.0]);
            let (u, v) = (lerp(ur, ss), lerp(vr, tt));
            let [e1, e2] = if s.surface == SurfaceKind::Superellipsoid { s.exponents.unwrap_or([2.0, 2.0]) } else { [2.0, 2.0] };
            let (m1, m2) = (2.0 / e1.max(0.1), 2.0 / e2.max(0.1));
            let cv = sgnpow(v.cos(), m1);
            c + Vec3::new(r.x * cv * sgnpow(u.cos(), m2), r.y * sgnpow(v.sin(), m1), r.z * cv * sgnpow(u.sin(), m2))
        }
        SurfaceKind::Paraboloid => {
            let vr = s.v_range.unwrap_or([0.0, 1.0]);
            let u = lerp(ur, ss);
            let v = vr[0] + (vr[1] - vr[0]) * tt;
            let h = s.height.unwrap_or(r.y);
            c + Vec3::new(r.x * v * u.cos(), h * (1.0 - v * v), r.z * v * u.sin())
        }
        SurfaceKind::Hypar => {
            let k = s.corners.as_ref().ok_or_else(|| Error::msg("hypar shell needs `corners` (4 points)"))?;
            if k.len() != 4 {
                return Err(Error::msg("hypar `corners` needs exactly 4 points"));
            }
            let p = |i: usize| Vec3::from_array(k[i]);
            let a = p(0).lerp(p(1), ss);
            let d = p(3).lerp(p(2), ss);
            c + a.lerp(d, tt)
        }
        SurfaceKind::Cylinder => {
            let ur = s.u_range.unwrap_or([0.0, 180.0]);
            let u = lerp(ur, ss);
            let len = s.height.unwrap_or(2.0 * r.x);
            c + Vec3::new(len * (tt - 0.5), r.y * u.sin(), r.z * u.cos())
        }
        SurfaceKind::Torus => {
            let vr = s.v_range.unwrap_or([0.0, 360.0]);
            let (u, v) = (lerp(ur, ss), lerp(vr, tt));
            let (big, small) = (r.x, r.y);
            c + Vec3::new((big + small * v.cos()) * u.cos(), small * v.sin(), (big + small * v.cos()) * u.sin())
        }
        SurfaceKind::Cone => {
            let u = lerp(ur, ss);
            let h = s.height.unwrap_or(r.y);
            c + Vec3::new(r.x * (1.0 - tt) * u.cos(), h * tt, r.z * (1.0 - tt) * u.sin())
        }
    })
}

#[derive(Clone, Copy)]
struct V {
    p: Vec3,
    n: Vec3,
    uv: Vec2,
}

fn lerp_v(a: V, b: V, t: f32) -> V {
    V { p: a.p.lerp(b.p, t), n: a.n.lerp(b.n, t).normalize_or(a.n), uv: a.uv.lerp(b.uv, t) }
}

/// Clips a polygon of vertices against n·p <= d.
fn clip_poly(poly: &[V], n: Vec3, d: f32) -> Vec<V> {
    let mut out = Vec::with_capacity(poly.len() + 2);
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        let (da, db) = (a.p.dot(n) - d, b.p.dot(n) - d);
        if da <= 0.0 {
            out.push(a);
        }
        if (da < 0.0 && db > 0.0) || (da > 0.0 && db < 0.0) {
            out.push(lerp_v(a, b, da / (da - db)));
        }
    }
    out
}

pub fn shell(b: &mut Builder, s: &Shell, sc: &Scope, o: &mut Out) -> Result<()> {
    let r = s.radii.map(|r| r[0].max(r[1]).max(r[2])).or(s.radius).unwrap_or(5.0);
    let [nu, nv] = s.segments.unwrap_or([b.segs(r, 24, 160) as u32, (b.segs(r, 24, 160) / 2).max(12) as u32]);
    let (nu, nv) = (nu.max(2) as usize, nv.max(2) as usize);
    let c = Vec3::from_array(s.center);
    let closed_u = matches!(s.surface, SurfaceKind::Torus | SurfaceKind::Ellipsoid | SurfaceKind::Superellipsoid | SurfaceKind::Paraboloid | SurfaceKind::Cone) && s.u_range.is_none_or(|u| (u[1] - u[0]).abs() >= 359.99);
    let w = nu + 1;
    let mut grid = Vec::with_capacity(w * (nv + 1));
    for j in 0..=nv {
        for i in 0..=nu {
            grid.push(shell_point(s, i as f32 / nu as f32, j as f32 / nv as f32)?);
        }
    }
    if grid.iter().any(|p| !mesh::finite3(*p)) {
        return Err(Error::msg("shell produced non-finite points (check radii and exponents)"));
    }
    let at = |i: usize, j: usize| grid[j * w + i];
    // Normals from neighbouring grid points, oriented away from the centre
    // for closed-ish surfaces.
    let outwardish = !matches!(s.surface, SurfaceKind::Hypar);
    let mut nrm = vec![Vec3::Y; grid.len()];
    for j in 0..=nv {
        for i in 0..=nu {
            let (i0, i1) = if closed_u { ((i + nu - 1) % nu, (i + 1) % nu) } else { (i.saturating_sub(1), (i + 1).min(nu)) };
            let (j0, j1) = (j.saturating_sub(1), (j + 1).min(nv));
            let du = at(i1, j) - at(i0, j);
            let dv = at(i, j1) - at(i, j0);
            let mut n = du.cross(dv).normalize_or(Vec3::ZERO);
            if n == Vec3::ZERO {
                n = (at(i, j) - c).normalize_or(Vec3::Y);
            }
            if outwardish && n.dot(at(i, j) - c) < 0.0 {
                n = -n;
            }
            nrm[j * w + i] = n;
        }
    }
    // Metric UVs.
    let mut uv = vec![Vec2::ZERO; grid.len()];
    for j in 0..=nv {
        let mut a = 0.0;
        for i in 0..=nu {
            if i > 0 {
                a += (at(i, j) - at(i - 1, j)).length();
            }
            uv[j * w + i].x = a;
        }
    }
    for i in 0..=nu {
        let mut a = 0.0;
        for j in 0..=nv {
            if j > 0 {
                a += (at(i, j) - at(i, j - 1)).length();
            }
            uv[j * w + i].y = -a;
        }
    }
    let vtx = |i: usize, j: usize| V { p: at(i, j), n: nrm[j * w + i], uv: uv[j * w + i] };
    let planes: Vec<(Vec3, f32)> = s.clip.iter().map(|c4| (Vec3::new(c4[0], c4[1], c4[2]), c4[3])).collect();
    let mut tris: Vec<[V; 3]> = Vec::new();
    for j in 0..nv {
        for i in 0..nu {
            let q = [vtx(i, j), vtx(i + 1, j), vtx(i + 1, j + 1), vtx(i, j + 1)];
            for t in [[q[0], q[1], q[2]], [q[0], q[2], q[3]]] {
                let mut poly_v: Vec<V> = t.to_vec();
                for (n, d) in &planes {
                    if poly_v.len() < 3 {
                        break;
                    }
                    poly_v = clip_poly(&poly_v, *n, *d);
                }
                if poly_v.len() < 3 {
                    continue;
                }
                for k in 1..poly_v.len() - 1 {
                    let tri = [poly_v[0], poly_v[k], poly_v[k + 1]];
                    let g = (tri[1].p - tri[0].p).cross(tri[2].p - tri[0].p);
                    if g.length_squared() < 1e-12 {
                        continue;
                    }
                    let nsum = tri[0].n + tri[1].n + tri[2].n;
                    tris.push(if g.dot(nsum) >= 0.0 { tri } else { [tri[0], tri[2], tri[1]] });
                }
            }
        }
    }
    if tris.is_empty() {
        return Err(Error::msg("shell is empty after clipping"));
    }
    let t = s.thickness.max(0.0);
    // Collide with (and slice for plan metrics) the outer surface only:
    // its sections close around the enclosed space.
    o.col.extend(tris.iter().map(|tr| [tr[0].p, tr[1].p, tr[2].p]));
    let p = o.geo.get(sc.mat);
    for tri in &tris {
        let base = p.positions.len() as u32;
        for v in tri {
            p.vertex(v.p, v.n, v.uv);
        }
        p.indices.extend([base, base + 1, base + 2]);
        if t > 0.0 {
            let base = p.positions.len() as u32;
            for v in tri {
                p.vertex(v.p - v.n * t, -v.n, v.uv);
            }
            p.indices.extend([base, base + 2, base + 1]);
        }
    }
    // Rim along the boundary.
    if t > 0.0 {
        let key = |v: Vec3| ((v.x * 2000.0).round() as i64, (v.y * 2000.0).round() as i64, (v.z * 2000.0).round() as i64);
        let mut count: HashMap<((i64, i64, i64), (i64, i64, i64)), (usize, V, V)> = HashMap::new();
        for tri in &tris {
            for k in 0..3 {
                let (a, bb) = (tri[k], tri[(k + 1) % 3]);
                let (ka, kb) = (key(a.p), key(bb.p));
                let k2 = if ka < kb { (ka, kb) } else { (kb, ka) };
                let e = count.entry(k2).or_insert((0, a, bb));
                e.0 += 1;
            }
        }
        for (_, (n, a, bb)) in count {
            if n != 1 {
                continue;
            }
            let q = [a.p, bb.p, bb.p - bb.n * t, a.p - a.n * t];
            let want = (bb.p - a.p).cross(a.n + bb.n);
            let len = (bb.p - a.p).length();
            mesh::quad_facing(p, q, [Vec2::new(0.0, 0.0), Vec2::new(len, 0.0), Vec2::new(len, -t), Vec2::new(0.0, -t)], want);
        }
    }
    // Ribs along iso-parameter curves.
    if let Some(rb) = s.ribs.get() {
        let rm = b.mat(rb.material.as_deref(), sc.mat)?;
        let along_u = rb.along == "u";
        let count = rb.count.max(1) as usize;
        let samples = if along_u { nu } else { nv }.max(8) * 2;
        let inside = |q: Vec3| planes.iter().all(|(n, d)| q.dot(*n) <= *d + 1e-4);
        let rg = o.geo.get(rm);
        for k in 0..count {
            let fixed = if count == 1 { 0.5 } else { (k as f32 + 0.5) / count as f32 };
            let mut run: Vec<(Vec3, Vec3, Vec3)> = Vec::new();
            let mut best: Vec<(Vec3, Vec3, Vec3)> = Vec::new();
            for j in 0..=samples {
                let t = j as f32 / samples as f32;
                let (ss, tt) = if along_u { (t, fixed) } else { (fixed, t) };
                let q = shell_point(s, ss, tt)?;
                let e = 1e-3;
                let (qa, qb) = if along_u { (shell_point(s, (t - e).max(0.0), fixed)?, shell_point(s, (t + e).min(1.0), fixed)?) } else { (shell_point(s, fixed, (t - e).max(0.0))?, shell_point(s, fixed, (t + e).min(1.0))?) };
                let (qc, qd) = if along_u { (shell_point(s, t, (fixed - e).max(0.0))?, shell_point(s, t, (fixed + e).min(1.0))?) } else { (shell_point(s, (fixed - e).max(0.0), t)?, shell_point(s, (fixed + e).min(1.0), t)?) };
                let tan = (qb - qa).normalize_or(Vec3::X);
                let mut n = (qb - qa).cross(qd - qc).normalize_or(Vec3::Y);
                if outwardish && n.dot(q - c) < 0.0 {
                    n = -n;
                }
                if rb.inside {
                    n = -n;
                }
                let side = n.cross(tan).normalize_or(Vec3::X);
                if inside(q) {
                    run.push((q + n * if rb.inside { t.max(0.0) } else { 0.0 }, side, n));
                } else if !run.is_empty() {
                    if run.len() > best.len() {
                        best = std::mem::take(&mut run);
                    }
                    run.clear();
                }
            }
            if run.len() > best.len() {
                best = run;
            }
            if best.len() < 2 {
                continue;
            }
            let path: Vec<Vec3> = best.iter().map(|x| x.0 - x.2 * 0.02).collect();
            let side: Vec<Vec3> = best.iter().map(|x| x.1).collect();
            let up: Vec<Vec3> = best.iter().map(|x| x.2).collect();
            let hw = rb.width * 0.5;
            let prof = [Vec2::new(-hw, 0.0), Vec2::new(hw, 0.0), Vec2::new(hw * 0.7, rb.depth), Vec2::new(-hw * 0.7, rb.depth)];
            mesh::sweep(rg, &path, &side, &up, &prof, false, true, true, true);
        }
    }
    Ok(())
}

pub fn mesh_el(_b: &mut Builder, m: &MeshEl, sc: &Scope, o: &mut Out) -> Result<()> {
    let n = m.vertices.len() as u32;
    let mut tris: Vec<[u32; 3]> = m.triangles.clone();
    for q in &m.quads {
        tris.push([q[0], q[1], q[2]]);
        tris.push([q[0], q[2], q[3]]);
    }
    if tris.is_empty() {
        return Err(Error::msg("mesh needs `triangles` or `quads`"));
    }
    if let Some(bad) = tris.iter().flatten().find(|i| **i >= n) {
        return Err(Error::msg(format!("mesh index {bad} out of range ({n} vertices)")));
    }
    let mut p = Prim::default();
    for t in tris {
        let (a, bb, c) = (Vec3::from_array(m.vertices[t[0] as usize]), Vec3::from_array(m.vertices[t[1] as usize]), Vec3::from_array(m.vertices[t[2] as usize]));
        let nn = (bb - a).cross(c - a);
        if nn.length_squared() < 1e-14 {
            continue;
        }
        let uvf = mesh::world_uv_fn(nn.normalize());
        p.tri(a, bb, c, [uvf(a), uvf(bb), uvf(c)]);
    }
    if m.smooth > 0.0 {
        mesh::auto_normals(&mut p, m.smooth);
    }
    o.geo.get(sc.mat).append(&p, &Mat4::IDENTITY);
    Ok(())
}
