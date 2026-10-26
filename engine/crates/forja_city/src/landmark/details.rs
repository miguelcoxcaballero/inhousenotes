//! Details along polygon edges: mouldings (cornices, string courses,
//! plinths) with modillions and dentils, parapets, balustrades,
//! battlements, machicolations and gargoyles.

use forja_math::{Vec2, Vec3};

use super::build::{Builder, Out, Scope};
use super::mesh::{self, MatId, Plane};
use super::parse::{Error, Result};
use super::poly::{self, Shape};
use super::profiles;
use super::resolve;
use super::runs::Path2;
use super::spec::*;

/// Corners sharper than this split repeated details into runs.
const RUN_ANGLE: f32 = super::facade::RUN_ANGLE;

/// Chains of selected edges as 2D polylines (outward = right of travel).
fn paths(shape: &Shape, edges: &EdgeSel) -> Vec<(Vec<Vec2>, bool)> {
    let sel = resolve::select(shape, edges);
    resolve::chains(shape, &sel).into_iter().map(|c| (resolve::chain_points(shape, c), c.3)).collect()
}

fn offset_path(p: &[Vec2], closed: bool, d: f32) -> Vec<Vec2> {
    if d.abs() < 1e-6 {
        return p.to_vec();
    }
    let n = mesh::polyline_offsets_2d(p, closed);
    p.iter().zip(n).map(|(q, n)| *q + n * d).collect()
}

/// Profile points scaled to `size` (height) and `projection` (depth).
pub fn profile_points(p: &ProfileSpec, size: f32, projection: Option<f32>) -> Vec<Vec2> {
    let raw: Vec<Vec2> = match p {
        ProfileSpec::Named(n) => profiles::named(n),
        ProfileSpec::Points(v) => {
            // Explicit profiles are in metres already.
            let mut pts: Vec<Vec2> = v.iter().map(|q| Vec2::new(q[0], q[1])).collect();
            if pts.first().is_some_and(|q| q.x.abs() > 1e-4) {
                pts.insert(0, Vec2::new(0.0, pts[0].y));
            }
            if pts.last().is_some_and(|q| q.x.abs() > 1e-4) {
                let y = pts.last().unwrap().y;
                pts.push(Vec2::new(0.0, y));
            }
            if poly::area(&pts) < 0.0 {
                pts.reverse();
            }
            return pts;
        }
    };
    let max_out = raw.iter().map(|q| q.x).fold(0.0, f32::max).max(1e-3);
    let sx = projection.map(|pj| pj / max_out).unwrap_or(size);
    raw.into_iter().map(|q| Vec2::new(q.x * sx, q.y * size)).collect()
}

/// Sweeps a moulding along selected edges at height `y`.
#[allow(clippy::too_many_arguments)]
pub fn moulding_on(_b: &mut Builder, shape: &Shape, edges: &EdgeSel, y: f32, profile: &ProfileSpec, size: f32, projection: Option<f32>, offset: f32, mat: MatId, modillions: Option<&ModillionSpec>, dentils: Option<&DentilSpec>, o: &mut Out) -> Result<()> {
    let prof = profile_points(profile, size, projection);
    let height = prof.iter().map(|q| q.y).fold(0.0, f32::max);
    let depth = prof.iter().map(|q| q.x).fold(0.0, f32::max);
    for (path, closed) in paths(shape, edges) {
        let path = offset_path(&path, closed, offset);
        moulding_path(o, &path, closed, y, &prof, mat);
        if let Some(m) = modillions {
            let d = m.depth.unwrap_or(depth * 0.78);
            let top = y + height * 0.32;
            along(&path, closed, m.spacing, 0.3, |p, n, t| {
                let p3 = Vec3::new(p.x, 0.0, p.y);
                let n3 = Vec3::new(n.x, 0.0, n.y);
                let t3 = Vec3::new(t.x, 0.0, t.y);
                let g = o.geo.get(mat);
                let hw = m.width * 0.5;
                match m.style.as_str() {
                    "block" => g.obox(p3 + n3 * (d * 0.5) + Vec3::Y * (top - m.height * 0.5), [t3, Vec3::Y, n3], Vec3::new(hw, m.height * 0.5, d * 0.5), 1.0),
                    _ => {
                        // Bracket: deep top block over a shorter lower scroll.
                        g.obox(p3 + n3 * (d * 0.5) + Vec3::Y * (top - m.height * 0.22), [t3, Vec3::Y, n3], Vec3::new(hw, m.height * 0.22, d * 0.5), 1.0);
                        g.obox(p3 + n3 * (d * 0.3) + Vec3::Y * (top - m.height * 0.62), [t3, Vec3::Y, n3], Vec3::new(hw * 0.9, m.height * 0.2, d * 0.3), 1.0);
                        g.cylinder(p3 + n3 * (d * 0.62) - t3 * hw + Vec3::Y * (top - m.height * 0.5), p3 + n3 * (d * 0.62) + t3 * hw + Vec3::Y * (top - m.height * 0.5), m.height * 0.12, 8, true);
                    }
                }
            });
        }
        if let Some(dn) = dentils {
            let y0 = y + height * 0.17;
            along(&path, closed, dn.spacing, 0.05, |p, n, t| {
                let p3 = Vec3::new(p.x, 0.0, p.y);
                let n3 = Vec3::new(n.x, 0.0, n.y);
                let t3 = Vec3::new(t.x, 0.0, t.y);
                o.geo.get(mat).obox(p3 + n3 * (depth * 0.3 + dn.depth * 0.5) + Vec3::Y * (y0 + dn.height * 0.5), [t3, Vec3::Y, n3], Vec3::new(dn.width * 0.5, dn.height * 0.5, dn.depth * 0.5), 1.0);
            });
        }
    }
    Ok(())
}

/// Sweep of a profile along a horizontal polyline at height `y`.
pub fn moulding_path(o: &mut Out, path: &[Vec2], closed: bool, y: f32, prof: &[Vec2], mat: MatId) {
    let p3: Vec<Vec3> = path.iter().map(|q| Vec3::new(q.x, y, q.y)).collect();
    if p3.len() < 2 {
        return;
    }
    let (sides, ups) = mesh::level_frames(&p3, closed, 1.0);
    mesh::sweep(o.geo.get(mat), &p3, &sides, &ups, prof, closed, true, false, !closed);
}

/// Calls `f(point, outward normal, tangent)` at regular spacing by arc
/// length along each smooth run of a polyline (curved plans are one run;
/// sharp corners split runs and keep `margin` clear).
fn along(path: &[Vec2], closed: bool, spacing: f32, margin: f32, mut f: impl FnMut(Vec2, Vec2, Vec2)) {
    if path.len() < 2 {
        return;
    }
    let p = Path2::new(path, closed, RUN_ANGLE);
    for d in p.spaced(spacing, margin) {
        let (q, n, t) = p.at(d);
        f(q, n, t);
    }
}

/// Solid band along chains (outer face offset `overhang` from the edge,
/// `thickness` deep), from `y0` to `y1`.
fn band_solid(o: &mut Out, path: &[Vec2], closed: bool, y0: f32, y1: f32, thickness: f32, overhang: f32, mat: MatId) {
    let outer = offset_path(path, closed, overhang);
    let inner = offset_path(path, closed, overhang - thickness);
    let shape = if closed {
        Shape { outer: outer.clone(), holes: vec![inner] }
    } else {
        let mut ring = outer.clone();
        ring.extend(inner.iter().rev());
        Shape::new(ring)
    };
    if let Some(s) = shape.normalized() {
        mesh::prism(o.geo.get(mat), &s, y0, y1, true, true);
        let mut col = crate::geom::Prim::default();
        mesh::prism(&mut col, &s, y0, y1, true, true);
        for t in col.indices.chunks_exact(3) {
            o.col.push([col.positions[t[0] as usize], col.positions[t[1] as usize], col.positions[t[2] as usize]]);
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn parapet_on(_b: &mut Builder, shape: &Shape, edges: &EdgeSel, y: f32, height: f32, thickness: f32, coping: bool, overhang: f32, mat: MatId, o: &mut Out) -> Result<()> {
    for (path, closed) in paths(shape, edges) {
        band_solid(o, &path, closed, y, y + height, thickness, overhang, mat);
        if coping {
            band_solid(o, &path, closed, y + height, y + height + 0.1, thickness + 0.1, overhang + 0.05, mat);
        }
    }
    Ok(())
}

/// Merlon outline in (u, v) with its base at v = 0, centred on u = 0.
pub fn merlon_outline(kind: MerlonKind, w: f32, h: f32) -> Vec<Vec2> {
    let hw = w * 0.5;
    match kind {
        MerlonKind::Rect => vec![Vec2::new(-hw, 0.0), Vec2::new(hw, 0.0), Vec2::new(hw, h), Vec2::new(-hw, h)],
        MerlonKind::Swallowtail => vec![Vec2::new(-hw, 0.0), Vec2::new(hw, 0.0), Vec2::new(hw, h), Vec2::new(0.0, h * 0.72), Vec2::new(-hw, h)],
        MerlonKind::Pyramidal => vec![Vec2::new(-hw, 0.0), Vec2::new(hw, 0.0), Vec2::new(hw, h * 0.6), Vec2::new(0.0, h), Vec2::new(-hw, h * 0.6)],
        MerlonKind::Rounded => {
            let mut v = vec![Vec2::new(-hw, 0.0), Vec2::new(hw, 0.0)];
            for k in 0..=8 {
                let a = k as f32 / 8.0 * std::f32::consts::PI;
                v.push(Vec2::new(hw * a.cos(), h - hw + hw * a.sin()));
            }
            v
        }
        MerlonKind::Stepped => {
            let s = h / 3.0;
            vec![
                Vec2::new(-hw, 0.0),
                Vec2::new(hw, 0.0),
                Vec2::new(hw, s),
                Vec2::new(hw * 0.66, s),
                Vec2::new(hw * 0.66, 2.0 * s),
                Vec2::new(hw * 0.33, 2.0 * s),
                Vec2::new(hw * 0.33, h),
                Vec2::new(-hw * 0.33, h),
                Vec2::new(-hw * 0.33, 2.0 * s),
                Vec2::new(-hw * 0.66, 2.0 * s),
                Vec2::new(-hw * 0.66, s),
                Vec2::new(-hw, s),
            ]
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn crenel_on(b: &mut Builder, shape: &Shape, edges: &EdgeSel, y: f32, c: &CrenelSpec, overhang: f32, mat: MatId, o: &mut Out) -> Result<()> {
    let sel = resolve::select(shape, edges);
    for (path, closed) in paths(shape, edges) {
        if c.wall > 0.0 {
            band_solid(o, &path, closed, y, y + c.wall, c.thickness, overhang, mat);
        }
    }
    let dark = if c.loopholes { Some(b.mat_or(None, "dark")?) } else { None };
    let outline = merlon_outline(c.shape, c.width, c.height);
    let top = y + c.wall;
    let _ = sel;
    for (path, closed) in paths(shape, edges) {
        let p = Path2::new(&path, closed, RUN_ANGLE);
        // Merlons by arc length along each run: a merlon at both ends of
        // a run (corners), evenly around a round tower.
        let mut at: Vec<f32> = Vec::new();
        if p.full_loop() {
            let l = p.total();
            let n = ((l / (c.width + c.gap)).round() as usize).max(1);
            at.extend((0..n).map(|k| l * k as f32 / n as f32));
        } else {
            for &(a, bb) in &p.runs {
                let len = bb - a;
                if len < c.width * 0.6 {
                    continue;
                }
                let count = (((len + c.gap) / (c.width + c.gap)).round() as usize).max(1);
                let pitch = if count > 1 { (len - c.width) / (count - 1) as f32 } else { 0.0 };
                at.extend((0..count).map(|k| if count > 1 { a + c.width * 0.5 + pitch * k as f32 } else { a + len * 0.5 }));
            }
        }
        for s in at {
            let (q, nrm, t) = p.at(s);
            let q = q + nrm * overhang;
            // Plane on the outer face: u along the run, v up, n inward.
            let plane = Plane::new(Vec3::new(q.x, top, q.y), Vec3::new(t.x, 0.0, t.y), Vec3::Y);
            if let Some(r) = poly::normalize(&outline, true) {
                mesh::extrude_shape(o.geo.get(mat), &Shape::new(r), &plane, c.thickness);
            }
            if let Some(dk) = dark {
                let ctr = Vec3::new(q.x, top + c.height * 0.45, q.y) + Vec3::new(nrm.x, 0.0, nrm.y) * 0.005;
                o.geo.get(dk).obox(ctr, [Vec3::new(t.x, 0.0, t.y), Vec3::Y, Vec3::new(nrm.x, 0.0, nrm.y)], Vec3::new(0.05, c.height * 0.3, 0.004), 1.0);
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn balustrade_on(b: &mut Builder, shape: &Shape, edges: &EdgeSel, y: f32, height: f32, spacing: f32, pedestals: f32, finials: &str, inset: f32, mat: MatId, o: &mut Out) -> Result<()> {
    let k = height / 0.95;
    let rail = vec![Vec2::new(-0.17, 0.0), Vec2::new(0.17, 0.0), Vec2::new(0.17, 0.16 * k), Vec2::new(-0.17, 0.16 * k)];
    let top_rail = vec![Vec2::new(-0.18, 0.0), Vec2::new(0.18, 0.0), Vec2::new(0.2, 0.06 * k), Vec2::new(0.2, 0.1 * k), Vec2::new(-0.2, 0.1 * k), Vec2::new(-0.2, 0.06 * k)];
    let baluster: Vec<Vec2> = [(0.0, 0.16), (0.055, 0.16), (0.065, 0.19), (0.048, 0.25), (0.055, 0.3), (0.09, 0.42), (0.08, 0.5), (0.042, 0.62), (0.036, 0.66), (0.058, 0.7), (0.065, 0.76), (0.0, 0.76)]
        .iter()
        .map(|(r, yy)| Vec2::new(r * k.min(1.4), yy * k))
        .collect();
    for (path, closed) in paths(shape, edges) {
        let path = offset_path(&path, closed, -inset);
        let p3 = |q: &Vec2, h: f32| Vec3::new(q.x, h, q.y);
        let _ = (spacing, pedestals);
        let base_path: Vec<Vec3> = path.iter().map(|q| p3(q, y)).collect();
        let (sides, ups) = mesh::level_frames(&base_path, closed, 1.0);
        mesh::sweep(o.geo.get(mat), &base_path, &sides, &ups, &rail, closed, true, false, !closed);
        let top_path: Vec<Vec3> = path.iter().map(|q| p3(q, y + height - 0.1 * k)).collect();
        mesh::sweep(o.geo.get(mat), &top_path, &sides, &ups, &top_rail, closed, true, false, !closed);
        // Pedestals at the corners and every `pedestals` metres along each
        // run; balusters evenly between (by arc length, so curved
        // balustrades are not broken up by their facets).
        let p2 = Path2::new(&path, closed, RUN_ANGLE);
        let mut peds: Vec<Vec2> = Vec::new();
        let mut ped_s: Vec<f32> = Vec::new();
        if !p2.full_loop() {
            for &(a, c) in &p2.runs {
                ped_s.push(a);
                ped_s.push(c);
            }
        }
        if pedestals > 0.5 {
            for &(a, c) in &p2.runs {
                let len = c - a;
                let n = (len / pedestals).round() as usize;
                for j in if p2.full_loop() { 0..n } else { 1..n } {
                    ped_s.push(a + len * j as f32 / n as f32);
                }
            }
        }
        ped_s.sort_by(|a, b| a.total_cmp(b));
        ped_s.dedup_by(|a, b| (*a - *b).abs() < 0.3);
        for s in &ped_s {
            peds.push(p2.at(*s).0);
        }
        for d in p2.spaced(spacing.max(0.12), 0.0) {
            if ped_s.iter().any(|s| (s - d).abs() < 0.3 || (p2.closed && (s - d).abs() > p2.total() - 0.3)) {
                continue;
            }
            mesh::lathe(o.geo.get(mat), p3(&p2.at(d).0, y), &baluster, 8, 0.0, std::f32::consts::TAU);
        }
        for q in &peds {
            let g = o.geo.get(mat);
            g.aabox(p3(q, y) - Vec3::new(0.24, 0.0, 0.24), p3(q, y + height) + Vec3::new(0.24, 0.0, 0.24), 1.0);
            g.aabox(p3(q, y + height) - Vec3::new(0.28, 0.0, 0.28), p3(q, y + height + 0.08) + Vec3::new(0.28, 0.0, 0.28), 1.0);
            let top = p3(q, y + height + 0.08);
            match finials {
                "urn" => mesh::lathe(g, top, &[Vec2::new(0.0, 0.0), Vec2::new(0.12, 0.0), Vec2::new(0.13, 0.05), Vec2::new(0.08, 0.12), Vec2::new(0.17, 0.26), Vec2::new(0.21, 0.4), Vec2::new(0.17, 0.52), Vec2::new(0.11, 0.56), Vec2::new(0.14, 0.6), Vec2::new(0.05, 0.66), Vec2::new(0.03, 0.74), Vec2::new(0.0, 0.78)], 12, 0.0, std::f32::consts::TAU),
                "ball" => {
                    mesh::lathe(g, top, &[Vec2::new(0.0, 0.0), Vec2::new(0.1, 0.0), Vec2::new(0.06, 0.08)], 10, 0.0, std::f32::consts::TAU);
                    mesh::ellipsoid(g, top + Vec3::Y * 0.24, Vec3::splat(0.17), (0.0, std::f32::consts::TAU), (-1.5, 1.5), 12, 8);
                }
                "pinecone" => mesh::lathe(g, top, &[Vec2::new(0.0, 0.0), Vec2::new(0.12, 0.0), Vec2::new(0.06, 0.08), Vec2::new(0.14, 0.2), Vec2::new(0.13, 0.36), Vec2::new(0.07, 0.5), Vec2::new(0.0, 0.58)], 10, 0.0, std::f32::consts::TAU),
                _ => {}
            }
        }
    }
    let _ = b;
    Ok(())
}

fn shapes_y(b: &Builder, polygon: Option<&PolySpec>, on: Option<&str>, sc: &Scope) -> Result<(Vec<Shape>, Option<(f32, f32)>)> {
    b.poly_or_on(polygon, on, sc)
}

pub fn moulding(b: &mut Builder, m: &Moulding, kind: MouldingKind, sc: &Scope, o: &mut Out) -> Result<()> {
    let (def_prof, def_size) = match kind {
        MouldingKind::Cornice => ("cornice", 0.8),
        MouldingKind::StringCourse => ("string", 0.28),
        MouldingKind::Plinth => ("plinth", 0.7),
        MouldingKind::Generic => ("cyma", 0.4),
    };
    let size = m.size.unwrap_or(def_size);
    let prof = m.profile.clone().unwrap_or(ProfileSpec::Named(def_prof.into()));
    if let Some(path) = &m.path {
        let pts: Vec<Vec2> = path.iter().map(|q| Vec2::new(q[0], q[1])).collect();
        let y = m.y.ok_or_else(|| Error::msg("a moulding along `path` needs `y`"))?;
        let pr = profile_points(&prof, size, m.projection);
        let pts = offset_path(&pts, false, m.offset);
        moulding_path(o, &pts, false, y, &pr, sc.mat);
        return Ok(());
    }
    let (shapes, yr) = shapes_y(b, m.polygon.as_ref(), m.on.as_deref(), sc)?;
    let y = match (m.y, kind, yr) {
        (Some(y), _, _) => y,
        (None, MouldingKind::Cornice, Some((_, top))) => top - size,
        (None, MouldingKind::Plinth, Some((base, _))) => base,
        (None, MouldingKind::Plinth, None) => 0.0,
        _ => return Err(Error::msg("moulding needs `y` (or `on` a named mass for cornices and plinths)")),
    };
    for s in &shapes {
        moulding_on(b, s, &m.edges, y, &prof, size, m.projection, m.offset, sc.mat, m.modillions.get(), m.dentils.get(), o)?;
    }
    Ok(())
}

pub fn parapet(b: &mut Builder, p: &Parapet, sc: &Scope, o: &mut Out) -> Result<()> {
    let (shapes, yr) = shapes_y(b, p.polygon.as_ref(), p.on.as_deref(), sc)?;
    let y = p.y.or(yr.map(|r| r.1)).or(sc.parent.as_ref().map(|_| 0.0)).ok_or_else(|| Error::msg("parapet needs `y` or `on`"))?;
    for s in &shapes {
        parapet_on(b, s, &p.edges, y, p.height, p.thickness, p.coping, p.overhang, sc.mat, o)?;
    }
    Ok(())
}

pub fn balustrade_el(b: &mut Builder, p: &Balustrade, sc: &Scope, o: &mut Out) -> Result<()> {
    let (shapes, yr) = shapes_y(b, p.polygon.as_ref(), p.on.as_deref(), sc)?;
    let y = p.y.or(yr.map(|r| r.1)).or(sc.parent.as_ref().map(|_| 0.0)).ok_or_else(|| Error::msg("balustrade needs `y` or `on`"))?;
    for s in &shapes {
        balustrade_on(b, s, &p.edges, y, p.height, p.spacing, p.pedestals, &p.finials, p.inset, sc.mat, o)?;
    }
    Ok(())
}

pub fn crenellations(b: &mut Builder, c: &Crenellations, sc: &Scope, o: &mut Out) -> Result<()> {
    let (shapes, yr) = shapes_y(b, c.polygon.as_ref(), c.on.as_deref(), sc)?;
    let y = c.y.or(yr.map(|r| r.1)).or(sc.parent.as_ref().map(|_| 0.0)).ok_or_else(|| Error::msg("crenellations need `y` or `on`"))?;
    let spec = CrenelSpec { shape: c.shape, width: c.width, gap: c.gap, height: c.height, thickness: c.thickness, wall: c.wall, loopholes: c.loopholes, material: None, edges: None };
    for s in &shapes {
        crenel_on(b, s, &c.edges, y, &spec, c.overhang, sc.mat, o)?;
    }
    Ok(())
}

/// Projecting parapet on stepped corbels; with `arches`, the Valencian
/// arcuated machicolation: arches spanning between the corbel heads, the
/// openings between corbels dark, the parapet (and merlons) on top.
pub fn machicolations(b: &mut Builder, m: &Machicolations, sc: &Scope, o: &mut Out) -> Result<()> {
    let (shapes, yr) = shapes_y(b, m.polygon.as_ref(), m.on.as_deref(), sc)?;
    let y = m.y.or(yr.map(|r| r.1 - m.corbel_height)).ok_or_else(|| Error::msg("machicolations need `y` or `on`"))?;
    let ch = m.corbel_height.max(0.2);
    let courses = m.corbel_courses.clamp(1, 12) as usize;
    let cw = m.corbel_width.max(0.1);
    let proj = m.projection.max(0.1);
    let dark = b.mat_or(None, "dark")?;
    let panel_t = (proj * 0.3).clamp(0.15, 0.35);
    for s in &shapes {
        for (path, closed) in paths(s, &m.edges) {
            let p2 = Path2::new(&path, closed, RUN_ANGLE);
            let at = p2.spaced(m.spacing, cw * 0.6);
            for &d in &at {
                let (p, n, t) = p2.at(d);
                let p3 = Vec3::new(p.x, 0.0, p.y);
                let n3 = Vec3::new(n.x, 0.0, n.y);
                let t3 = Vec3::new(t.x, 0.0, t.y);
                let g = o.geo.get(sc.mat);
                // Stepped courses, deeper towards the top; each slightly
                // rounded off at its lower front edge by a smaller block.
                for k in 0..courses {
                    let depth = proj * (k + 1) as f32 / courses as f32;
                    let h = ch / courses as f32;
                    let yy = y + h * k as f32;
                    g.obox(p3 + n3 * (depth * 0.5) + Vec3::Y * (yy + h * 0.5), [t3, Vec3::Y, n3], Vec3::new(cw * 0.5, h * 0.5, depth * 0.5), 1.0);
                }
            }
            if m.arches {
                // Arches between consecutive corbels of a run, on the front.
                let pairs: Vec<(f32, f32)> = at.windows(2).map(|w| (w[0], w[1])).chain((p2.full_loop() && at.len() > 1).then(|| (*at.last().unwrap(), at[0] + p2.total()))).collect();
                for (s0, s1) in pairs {
                    if p2.runs.iter().all(|r| !(s0 >= r.0 - 1e-3 && s1 <= r.1 + 1e-3)) && !p2.full_loop() {
                        continue;
                    }
                    let (pa, na, _) = p2.at(s0 + cw * 0.5);
                    let (pb, nb, _) = p2.at(s1 - cw * 0.5);
                    let qa = pa + na * proj;
                    let qb = pb + nb * proj;
                    let gap = (qb - qa).length();
                    if gap < 0.2 {
                        continue;
                    }
                    let rise = super::profiles::default_rise(m.arch, gap).min(ch * 0.8);
                    let spring = (ch - rise - 0.12).max(0.05);
                    let ol = super::profiles::outline(m.arch, gap, spring + rise, Some(rise), 12);
                    let head: Vec<Vec2> = ol.pts[ol.head.0..ol.head.1.min(ol.pts.len())].iter().map(|q| Vec2::new(q.x + gap * 0.5, q.y)).collect();
                    // Panel above the arch: right side up, top, left side
                    // down, then the arch from left to right.
                    let mut ring = vec![Vec2::new(gap, spring), Vec2::new(gap, ch), Vec2::new(0.0, ch), Vec2::new(0.0, spring)];
                    let mut hd = head.clone();
                    if hd.first().is_some_and(|q| q.x > gap * 0.5) {
                        hd.reverse();
                    }
                    ring.extend(hd.into_iter().filter(|q| q.y > spring + 1e-3));
                    let dir = (qb - qa) / gap;
                    let plane = Plane::new(Vec3::new(qa.x, y, qa.y), Vec3::new(dir.x, 0.0, dir.y), Vec3::Y);
                    if let Some(r) = poly::normalize(&ring, true) {
                        mesh::extrude_shape(o.geo.get(sc.mat), &Shape::new(r), &plane, panel_t);
                    }
                }
            }
            // Projecting parapet with dark slots in its floor.
            band_solid(o, &path, closed, y + ch, y + ch + m.parapet, m.thickness, proj, sc.mat);
            band_solid(o, &path, closed, y + ch - 0.06, y + ch, (proj - panel_t - 0.02).max(0.05), proj - panel_t, dark);
        }
        if let Some(c) = m.merlons.get() {
            crenel_on(b, s, &m.edges, y + ch + m.parapet, &CrenelSpec { wall: 0.0, ..c.clone() }, proj, sc.mat, o)?;
        }
    }
    Ok(())
}

pub fn gargoyles(b: &mut Builder, g: &Gargoyles, sc: &Scope, o: &mut Out) -> Result<()> {
    let (shapes, yr) = shapes_y(b, g.polygon.as_ref(), g.on.as_deref(), sc)?;
    let y = g.y.or(yr.map(|r| r.1 - 0.6)).ok_or_else(|| Error::msg("gargoyles need `y` or `on`"))?;
    let tilt = g.tilt.to_radians();
    let put = |o: &mut Out, p: Vec2, n: Vec2| {
        let n3 = Vec3::new(n.x, 0.0, n.y).normalize_or(Vec3::Z);
        let dir = (n3 * tilt.cos() - Vec3::Y * tilt.sin()).normalize();
        let a = Vec3::new(p.x, y, p.y) - n3 * 0.15;
        let e = a + dir * g.length;
        let pm = o.geo.get(sc.mat);
        pm.beam(a, e, Vec3::Y.cross(n3), g.size, g.size * 0.85, 1.0);
        // Head: snout and ears.
        mesh::ellipsoid(pm, e, Vec3::new(g.size * 0.7, g.size * 0.6, g.size * 0.7), (0.0, std::f32::consts::TAU), (-1.4, 1.4), 10, 6);
        pm.cone(e, e + dir * g.size * 0.9, g.size * 0.35, g.size * 0.12, 8, true);
    };
    for s in &shapes {
        for (path, closed) in paths(s, &g.edges) {
            along(&path, closed, g.spacing, g.spacing * 0.5, |p, n, _| put(o, p, n));
            if g.corners {
                let m = path.len();
                let sharp = super::runs::corners(&path, closed, RUN_ANGLE);
                for i in 0..m {
                    if !closed && (i == 0 || i == m - 1) || !sharp[i] {
                        continue;
                    }
                    let (a, c, d) = (path[(i + m - 1) % m], path[i], path[(i + 1) % m]);
                    if (c - a).perp_dot(d - c) > 0.0 {
                        let n0 = (c - a).normalize_or(Vec2::X);
                        let n1 = (d - c).normalize_or(Vec2::X);
                        let nn = (Vec2::new(n0.y, -n0.x) + Vec2::new(n1.y, -n1.x)).normalize_or(Vec2::Y);
                        put(o, c, nn);
                    }
                }
            }
        }
    }
    Ok(())
}
