//! Roofs on any plan: flat, gable, shed (planes split along the ridge,
//! with gable walls), hip (exact straight skeleton on any plan, concave
//! blocks and courtyards included),
//! pyramid / tent / spire / cone, mansard, barrel vault and dome. Tile UVs
//! run along the eaves and up the slope.

use forja_math::{Mat4, Vec2, Vec3};

use super::build::{Builder, Out, Scope};
use super::facade::{self, Facade, Run};
use super::mesh::{self, MatId, Plane};
use super::openings::{self, OpCtx, Placed, RunOut};
use super::parse::{Error, Result};
use super::poly::{self, Ring, Shape};
use super::resolve::EdgeRef;
use super::spec::*;
use crate::geom::Prim;

/// Roof settings shared by the element and the inline `roof = {...}`.
#[derive(Clone, Debug)]
pub struct RoofParams {
    pub shape: RoofShape,
    pub pitch: Option<f32>,
    pub height: Option<f32>,
    pub ridge: Option<RidgeDir>,
    pub overhang: Option<f32>,
    pub thickness: Option<f32>,
    pub material: Option<String>,
    pub wall_material: Option<String>,
    pub ribs: Option<RibSpec>,
    pub ridge_cap: Option<bool>,
    pub mansard: Option<MansardSpec>,
    pub vault: Option<ArchKind>,
    pub gable_openings: Vec<Openings>,
}

impl RoofParams {
    pub fn from_spec(r: &RoofSpec) -> Self {
        RoofParams {
            shape: r.shape,
            pitch: r.pitch,
            height: r.height,
            ridge: r.ridge,
            overhang: r.overhang,
            thickness: r.thickness,
            material: r.material.clone(),
            wall_material: r.wall_material.clone(),
            ribs: r.ribs.0.clone(),
            ridge_cap: r.ridge_cap,
            mansard: r.mansard.clone(),
            vault: None,
            gable_openings: Vec::new(),
        }
    }

    pub fn from_el(r: &Roof) -> Self {
        RoofParams {
            shape: r.shape,
            pitch: r.pitch,
            height: r.height,
            ridge: r.ridge,
            overhang: r.overhang,
            thickness: r.thickness,
            material: r.material.clone(),
            wall_material: r.wall_material.clone(),
            ribs: r.ribs.0.clone(),
            ridge_cap: r.ridge_cap,
            mansard: r.mansard.clone(),
            vault: r.vault,
            gable_openings: r.gable_openings.clone(),
        }
    }
}

pub fn roof(b: &mut Builder, r: &Roof, sc: &Scope, o: &mut Out) -> Result<()> {
    let (shapes, yr) = b.poly_or_on(r.polygon.as_ref(), r.on.as_deref(), sc)?;
    let y = r.y.or(yr.map(|v| v.1)).or(sc.parent.as_ref().map(|_| 0.0)).ok_or_else(|| Error::msg("roof needs `y` (eave height) or `on` a named mass"))?;
    let p = RoofParams::from_el(r);
    // The element's own material is the roof covering.
    let p = RoofParams { material: p.material.or(r.material.clone()), ..p };
    for s in &shapes {
        roof_shape(b, &p, s, y, sc, o)?;
    }
    Ok(())
}

/// Emits a planar roof face (plan polygon lifted by `h`), with UVs along
/// `along` (u) and up the slope (v, from the height so texture rows stay
/// parallel to the eaves and upright).
fn face(prim: &mut Prim, ring: &[Vec2], h: &dyn Fn(Vec2) -> f32, along: Vec2, origin: Vec2, cos_pitch: f32, up: bool) {
    let sin_p = (1.0 - cos_pitch * cos_pitch).max(0.0).sqrt();
    let Some(r) = poly::normalize(ring, true) else { return };
    let (pts, tris) = super::earcut::triangulate_with_holes(&r, &[]);
    if tris.is_empty() {
        return;
    }
    let p3: Vec<Vec3> = pts.iter().map(|q| Vec3::new(q.x, h(*q), q.y)).collect();
    // Plane normal from the largest triangle.
    let mut nrm = Vec3::ZERO;
    for t in &tris {
        let g = (p3[t[2] as usize] - p3[t[0] as usize]).cross(p3[t[1] as usize] - p3[t[0] as usize]);
        if g.length_squared() > nrm.length_squared() {
            nrm = g;
        }
    }
    let mut nrm = nrm.normalize_or(Vec3::Y);
    if (nrm.y < 0.0) == up {
        nrm = -nrm;
    }
    let across = Vec2::new(-along.y, along.x);
    let base = prim.positions.len() as u32;
    for (k, q) in pts.iter().enumerate() {
        let v = if sin_p > 0.08 { -p3[k].y / sin_p } else { (*q - origin).dot(across) };
        prim.vertex(p3[k], nrm, Vec2::new((*q - origin).dot(along), v));
    }
    for t in tris {
        let (a, bb, c) = (p3[t[0] as usize], p3[t[1] as usize], p3[t[2] as usize]);
        if (bb - a).cross(c - a).dot(nrm) >= 0.0 {
            prim.indices.extend([base + t[0], base + t[1], base + t[2]]);
        } else {
            prim.indices.extend([base + t[0], base + t[2], base + t[1]]);
        }
    }
}

/// Vertical strip under the roof edge (fascia) and the soffit.
fn fascia(prim: &mut Prim, ring: &[Vec2], h: &dyn Fn(Vec2) -> f32, thickness: f32) {
    let n = ring.len();
    let mut u = 0.0;
    for i in 0..n {
        let (a, c) = (ring[i], ring[(i + 1) % n]);
        let len = (c - a).length();
        if len < 1e-4 {
            continue;
        }
        let (ha, hc) = (h(a), h(c));
        let q = [Vec3::new(c.x, hc - thickness, c.y), Vec3::new(a.x, ha - thickness, a.y), Vec3::new(a.x, ha, a.y), Vec3::new(c.x, hc, c.y)];
        let out = Vec3::new(c.y - a.y, 0.0, a.x - c.x);
        mesh::quad_facing(prim, q, [Vec2::new(u, 0.0), Vec2::new(u + len, 0.0), Vec2::new(u + len, -thickness), Vec2::new(u, -thickness)], out);
        u += len;
    }
}

/// Ridge / hip caps: half-round tiles along a 3D segment.
fn cap(prim: &mut Prim, a: Vec3, c: Vec3) {
    if (c - a).length() > 0.2 {
        prim.cylinder(a, c, 0.13, 8, false);
    }
}

pub fn roof_shape(b: &mut Builder, p: &RoofParams, shape: &Shape, y: f32, sc: &Scope, o: &mut Out) -> Result<()> {
    let tiled = !matches!(p.shape, RoofShape::Flat | RoofShape::None | RoofShape::Dome);
    let mat = if matches!(p.shape, RoofShape::Flat) { b.mat(p.material.as_deref(), sc.mat)? } else { b.roof_mat(p.material.as_deref())? };
    let wall = b.mat(p.wall_material.as_deref(), sc.mat)?;
    let overhang = p.overhang.unwrap_or(if tiled { 0.35 } else { 0.05 });
    let thick = p.thickness.unwrap_or(if tiled { 0.16 } else { 0.2 });
    let caps = p.ridge_cap.unwrap_or(matches!(b.pal.get(mat).spec.kind.as_str(), "roof_tiles" | "clay_tiles" | "glazed_tiles"));
    let outer = shape.outer.clone();
    match p.shape {
        RoofShape::None => Ok(()),
        RoofShape::Flat => {
            let s = poly::grow_shape(shape, overhang);
            mesh::prism(o.geo.get(mat), &s, y, y + thick, true, true);
            o.collide_all();
            Ok(())
        }
        RoofShape::Gable | RoofShape::Shed => {
            if !shape.holes.is_empty() {
                b.warn(sc, format!("{} roof over a plan with courtyards: the courtyards are covered; use a hip roof (follows courtyards) or a roof per wing", p.shape.name()));
            }
            planar(b, p, &outer, y, overhang, thick, mat, wall, caps, sc, o)
        }
        RoofShape::Hip => {
            if hip(b, p, shape, y, overhang, thick, mat, caps, o)? {
                Ok(())
            } else {
                b.warn(sc, "hip roof: the straight skeleton of this plan failed (self-intersecting outline?); built as a gable");
                planar(b, &RoofParams { shape: RoofShape::Gable, ..p.clone() }, &outer, y, overhang, thick, mat, wall, caps, sc, o)
            }
        }
        RoofShape::Pyramid | RoofShape::Tent | RoofShape::Spire | RoofShape::Cone => apex(b, p, &outer, y, overhang, thick, mat, o),
        RoofShape::Mansard => mansard(b, p, &outer, y, overhang, thick, mat, caps, sc, o),
        RoofShape::Barrel => barrel(b, p, &outer, y, overhang, thick, mat, wall, sc, o),
        RoofShape::Dome => {
            let c = poly::centroid(&outer);
            let (mn, mx) = poly::bbox(&outer);
            let r = (0..outer.len()).map(|i| poly::seg_dist(c, outer[i], outer[(i + 1) % outer.len()])).fold(f32::MAX, f32::min);
            let rx = ((mx.x - mn.x) * 0.5).min(r * 1.6);
            let rz = ((mx.y - mn.y) * 0.5).min(r * 1.6);
            let h = p.height.unwrap_or(rx.min(rz));
            super::domes::shell_geo(b, mat, Vec3::new(c.x, y, c.y), rx, rz, h, DomeProfile::Hemisphere, None, 0, 0.0, None, 0.0, None, o);
            o.collide_all();
            Ok(())
        }
    }
}

/// Gable and shed roofs: planes meeting at a ridge line, closed by gable
/// walls wherever the roof rises above the eaves.
#[allow(clippy::too_many_arguments)]
fn planar(b: &mut Builder, p: &RoofParams, outer: &[Vec2], y: f32, overhang: f32, thick: f32, mat: MatId, wall: MatId, caps: bool, sc: &Scope, o: &mut Out) -> Result<()> {
    // Ridge direction.
    let rect = poly::min_rect(outer);
    let e0 = rect[1] - rect[0];
    let e1 = rect[2] - rect[1];
    let long = if e0.length() >= e1.length() { e0.normalize() } else { e1.normalize() };
    let ridge_dir = match p.ridge {
        Some(RidgeDir::Short) => Vec2::new(-long.y, long.x),
        Some(RidgeDir::Azimuth(a)) => {
            if p.shape == RoofShape::Shed {
                // For sheds `ridge` is the direction the slope faces.
                let d = poly::from_azimuth(a);
                Vec2::new(-d.y, d.x)
            } else {
                poly::from_azimuth(a)
            }
        }
        _ => long,
    };
    let across = Vec2::new(-ridge_dir.y, ridge_dir.x);
    let s_of = |q: Vec2| q.dot(across);
    let (smin, smax) = outer.iter().map(|q| s_of(*q)).fold((f32::MAX, f32::MIN), |(a, c), v| (a.min(v), c.max(v)));
    let half = ((smax - smin) * 0.5).max(0.05);
    let shed = p.shape == RoofShape::Shed;
    let shed_down = match p.ridge {
        Some(RidgeDir::Azimuth(a)) => poly::from_azimuth(a).dot(across) >= 0.0,
        _ => true,
    };
    let run = if shed { 2.0 * half } else { half };
    let pitch = p.pitch.unwrap_or(if shed { 15.0 } else { 30.0 }).clamp(1.0, 85.0);
    let rise = p.height.unwrap_or(pitch.to_radians().tan() * run);
    let tanp = rise / run;
    let smid = (smin + smax) * 0.5;
    let h = move |q: Vec2| -> f32 {
        let s = s_of(q);
        if shed {
            let t = if shed_down { smax - s } else { s - smin };
            y + tanp * t
        } else {
            y + rise - tanp * (s - smid).abs()
        }
    };
    let cos_p = 1.0 / (1.0 + tanp * tanp).sqrt();
    let ring = poly::offset(outer, overhang).unwrap_or(outer.to_vec());
    let halves: Vec<(Ring, Vec2)> = if shed {
        vec![(ring.clone(), Vec2::ZERO)]
    } else {
        vec![
            (poly::clip_half_plane(&ring, across, smid), across * smid),
            (poly::clip_half_plane(&ring, -across, -smid), across * smid),
        ]
    };
    {
        let prim = o.geo.get(mat);
        for (hr, origin) in &halves {
            if hr.len() < 3 {
                continue;
            }
            let org = if shed { across * if shed_down { smax } else { smin } } else { *origin };
            face(prim, hr, &h, ridge_dir, org, cos_p, true);
            let hd = |q: Vec2| h(q) - thick;
            face(prim, hr, &hd, ridge_dir, org, cos_p, false);
        }
        fascia(prim, &ring, &h, thick);
        if caps && !shed {
            let on: Vec<f32> = ring
                .iter()
                .zip(ring.iter().cycle().skip(1))
                .filter_map(|(a, c)| {
                    let (sa, sc2) = (s_of(*a) - smid, s_of(*c) - smid);
                    (sa * sc2 <= 0.0 && (sa - sc2).abs() > 1e-6).then(|| a.lerp(*c, sa / (sa - sc2)).dot(ridge_dir))
                })
                .collect();
            if on.len() >= 2 {
                let (t0, t1) = on.iter().fold((f32::MAX, f32::MIN), |(a, c), v| (a.min(*v), c.max(*v)));
                let at = |t: f32| {
                    let q = ridge_dir * t + across * smid;
                    Vec3::new(q.x, y + rise + 0.04, q.y)
                };
                cap(prim, at(t0), at(t1));
            }
        }
    }
    // Gable walls on the original plan's edges.
    let shape = Shape::new(outer.to_vec());
    let fs = super::masses::facades(&shape, y, y, wall);
    let mut gables: Vec<(Facade, EdgeRef)> = Vec::new();
    for (f, e) in fs {
        let (a, c) = (outer[e.i], outer[(e.i + 1) % outer.len()]);
        // Facade u runs from c (left end) to a.
        let mut pts = vec![Vec2::new(0.0, y), Vec2::new(f.len, y)];
        let ha = h(a);
        let hc = h(c);
        let mut top = vec![Vec2::new(f.len, ha)];
        let (sa, s2) = (s_of(a) - smid, s_of(c) - smid);
        if !shed && sa * s2 < 0.0 {
            let t = sa / (sa - s2);
            top.push(Vec2::new(f.len * (1.0 - t), y + rise));
        }
        top.push(Vec2::new(0.0, hc));
        if top.iter().all(|q| q.y - y < 0.05) {
            continue;
        }
        pts.extend(top);
        let Some(outline) = poly::normalize(&pts, true) else { continue };
        let v1 = outline.iter().map(|q| q.y).fold(y, f32::max);
        gables.push((Facade { v1, outline: Some(outline), ..f }, e));
    }
    gable_walls(b, p, gables, &shape, wall, sc, o)?;
    o.collide_all();
    Ok(())
}

/// Builds gable / end walls with their openings.
fn gable_walls(b: &mut Builder, p: &RoofParams, gables: Vec<(Facade, EdgeRef)>, shape: &Shape, wall: MatId, sc: &Scope, o: &mut Out) -> Result<()> {
    let runs: Vec<Run> = gables.into_iter().map(|(f, e)| Run::plain(f, e)).collect();
    let ops: Vec<&Openings> = p.gable_openings.iter().collect();
    let cx = OpCtx { wall, trim: wall, through: None, solid: Some(shape) };
    let placed = if ops.is_empty() { Placed { runs: vec![RunOut::default(); runs.len()], world: mesh::Geo::new(), passages: 0 } } else { openings::on_runs(b, &ops, &runs, shape, sc, &cx)? };
    for (run, ro) in runs.iter().zip(placed.runs) {
        let mut g = ro.geo;
        for w in facade::build(&mut g, &run.facade, &ro.cuts) {
            b.warn(sc, w);
        }
        run.place(&g, &mut o.geo);
    }
    o.geo.append(&placed.world, &Mat4::IDENTITY);
    Ok(())
}

/// Hip roof on any plan: the faces of the straight skeleton of the eave
/// ring, lifted at the pitch. `false` when the skeleton fails.
#[allow(clippy::too_many_arguments)]
fn hip(b: &mut Builder, p: &RoofParams, shape: &Shape, y: f32, overhang: f32, thick: f32, mat: MatId, caps: bool, o: &mut Out) -> Result<bool> {
    let pitch = p.pitch.unwrap_or(30.0).clamp(1.0, 85.0);
    let mut tanp = pitch.to_radians().tan();
    let eave = poly::grow_shape(shape, overhang);
    let Some(sk) = super::skeleton::skeleton(&eave) else { return Ok(false) };
    if let Some(hh) = p.height {
        tanp = hh / (sk.max_t - overhang).max(0.1);
    }
    let cos_p = 1.0 / (1.0 + tanp * tanp).sqrt();
    let h_of = move |t: f32| y + tanp * (t - overhang);
    let prim = o.geo.get(mat);
    for f in &sk.faces {
        let ring = if f.ring == 0 { &eave.outer } else { &eave.holes[f.ring - 1] };
        let n = ring.len();
        let (a, c) = (ring[f.edge], ring[(f.edge + 1) % n]);
        let along = (c - a).normalize_or(Vec2::X);
        let inward = Vec2::new(-along.y, along.x);
        let hi = |q: Vec2| h_of((q - a).dot(inward));
        face(prim, &f.pts, &hi, along, a, cos_p, true);
        let hd = |q: Vec2| hi(q) - thick;
        face(prim, &f.pts, &hd, along, a, cos_p, false);
    }
    if caps {
        // Caps on ridges and hips, not on the faint creases between the
        // faces of a curved eave (facets of a curved street front).
        for arc in sk.arcs.iter().filter(|a| a.turn > 20.0) {
            cap(prim, Vec3::new(arc.a.x, h_of(arc.ta) + 0.03, arc.a.y), Vec3::new(arc.b.x, h_of(arc.tb) + 0.03, arc.b.y));
        }
    }
    let he = h_of(0.0);
    let flat = |_: Vec2| he;
    for (ring, _) in eave.rings() {
        fascia(prim, ring, &flat, thick);
    }
    o.collide_all();
    let _ = b;
    Ok(true)
}

/// Hip roof on a convex plan: lower envelope of the eave planes (kept for
/// reference tests; [`hip`] covers every plan).
#[allow(clippy::too_many_arguments, dead_code)]
fn hip_convex(b: &mut Builder, p: &RoofParams, outer: &[Vec2], y: f32, overhang: f32, thick: f32, mat: MatId, caps: bool, o: &mut Out) -> Result<()> {
    let n = outer.len();
    let pitch = p.pitch.unwrap_or(30.0).clamp(1.0, 85.0);
    let mut tanp = pitch.to_radians().tan();
    let normals: Vec<Vec2> = (0..n).map(|i| poly::edge_normal(outer, i)).collect();
    let dist = |i: usize, q: Vec2| -(q - outer[i]).dot(normals[i]);
    if let Some(hh) = p.height {
        // Rise at the deepest point of the plan.
        let c = poly::centroid(outer);
        let inr = (0..n).map(|i| dist(i, c)).fold(f32::MAX, f32::min).max(0.1);
        tanp = hh / inr;
    }
    let cos_p = 1.0 / (1.0 + tanp * tanp).sqrt();
    let ring = poly::offset(outer, overhang).unwrap_or(outer.to_vec());
    let h = |q: Vec2| -> f32 { y + tanp * (0..n).map(|i| dist(i, q)).fold(f32::MAX, f32::min) };
    let mut caps_done: Vec<(Vec3, Vec3)> = Vec::new();
    let prim = o.geo.get(mat);
    for i in 0..n {
        let mut f: Ring = ring.clone();
        for j in 0..n {
            if j == i {
                continue;
            }
            // h_i <= h_j  <=>  dist_i <= dist_j  <=>  p.(n_j - n_i) <= a_j.n_j - a_i.n_i.
            let nn = normals[j] - normals[i];
            if nn.length_squared() < 1e-10 {
                continue;
            }
            let d = outer[j].dot(normals[j]) - outer[i].dot(normals[i]);
            f = poly::clip_half_plane(&f, nn, d);
            if f.len() < 3 {
                break;
            }
        }
        if f.len() < 3 || poly::area(&f).abs() < 1e-4 {
            continue;
        }
        let along = (outer[(i + 1) % n] - outer[i]).normalize_or(Vec2::X);
        let hi = |q: Vec2| y + tanp * dist(i, q);
        face(prim, &f, &hi, along, outer[i], cos_p, true);
        let hd = |q: Vec2| hi(q) - thick;
        face(prim, &f, &hd, along, outer[i], cos_p, false);
        if caps {
            // Face edges not on the eave ring are hips or ridges.
            let m = f.len();
            for k in 0..m {
                let (a, c) = (f[k], f[(k + 1) % m]);
                let on_eave = (hi(a) - h(a)).abs() < 1e-3 && (hi(c) - h(c)).abs() < 1e-3 && (dist(i, a) < -overhang * 0.5 + 1e-3 || ring.iter().any(|r| (*r - a).length() < 1e-3) && ring.iter().any(|r| (*r - c).length() < 1e-3));
                if on_eave {
                    continue;
                }
                let a3 = Vec3::new(a.x, hi(a) + 0.03, a.y);
                let c3 = Vec3::new(c.x, hi(c) + 0.03, c.y);
                if caps_done.iter().any(|(x, z)| ((*x - c3).length() < 0.05 && (*z - a3).length() < 0.05) || ((*x - a3).length() < 0.05 && (*z - c3).length() < 0.05)) {
                    continue;
                }
                caps_done.push((a3, c3));
            }
        }
    }
    for (a, c) in caps_done {
        cap(prim, a, c);
    }
    fascia(prim, &ring, &h, thick);
    o.collide_all();
    let _ = b;
    Ok(())
}

/// Pyramid, tent, spire and cone: faces from each edge to an apex.
#[allow(clippy::too_many_arguments)]
fn apex(b: &mut Builder, p: &RoofParams, outer: &[Vec2], y: f32, overhang: f32, thick: f32, mat: MatId, o: &mut Out) -> Result<()> {
    let n = outer.len();
    let c = poly::centroid(outer);
    let inr = (0..n).map(|i| poly::seg_dist(c, outer[i], outer[(i + 1) % n])).fold(f32::MAX, f32::min).max(0.1);
    let default_pitch = match p.shape {
        RoofShape::Spire => 78.0,
        RoofShape::Cone => 55.0,
        _ => 40.0,
    };
    let rise = p.height.unwrap_or_else(|| p.pitch.unwrap_or(default_pitch).clamp(1.0, 89.0).to_radians().tan() * inr);
    let top = Vec3::new(c.x, y + rise, c.y);
    let ov = if p.shape == RoofShape::Spire { overhang.min(0.15) } else { overhang };
    let ring = poly::offset(outer, ov).unwrap_or(outer.to_vec());
    let drop = ov * rise / inr;
    let smooth = n >= 16 || p.shape == RoofShape::Cone;
    let mut prim = Prim::default();
    let m = ring.len();
    let mut u = 0.0;
    for i in 0..m {
        let (a, cc) = (ring[i], ring[(i + 1) % m]);
        let a3 = Vec3::new(a.x, y - drop, a.y);
        let c3 = Vec3::new(cc.x, y - drop, cc.y);
        let len = (cc - a).length();
        let slant = ((a + cc) * 0.5 - c).length().hypot(rise + drop);
        let mid_u = u + len * 0.5;
        mesh::tri_facing(&mut prim, a3, c3, top, [Vec2::new(u, 0.0), Vec2::new(u + len, 0.0), Vec2::new(mid_u, -slant)], Vec3::new(a.x + cc.x - 2.0 * c.x, 0.0, a.y + cc.y - 2.0 * c.y) + Vec3::Y * 0.01);
        u += len;
    }
    if smooth {
        mesh::auto_normals(&mut prim, 35.0);
    }
    let g = o.geo.get(mat);
    g.append(&prim, &Mat4::IDENTITY);
    // Underside at the eaves.
    if ov > 0.01 {
        let s = Shape { outer: ring.clone(), holes: vec![outer.iter().rev().copied().collect()] };
        if let Some(s) = s.normalized() {
            mesh::horizontal_shape(g, &s, y - drop - thick * 0.3, false);
        }
        let hfn = |_: Vec2| y - drop;
        fascia(g, &ring, &hfn, thick * 0.3);
    }
    if let Some(r) = &p.ribs {
        let rm = b.mat(r.material.as_deref(), mat)?;
        let rg = o.geo.get(rm);
        let k = (r.count as usize).max(1);
        for j in 0..k {
            let i = j * m / k;
            let a = ring[i];
            rg.beam(Vec3::new(a.x, y - drop, a.y), top, Vec3::Y, r.width, r.depth, 1.0);
        }
    }
    o.collide_all();
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn mansard(b: &mut Builder, p: &RoofParams, outer: &[Vec2], y: f32, overhang: f32, thick: f32, mat: MatId, caps: bool, sc: &Scope, o: &mut Out) -> Result<()> {
    let m = p.mansard.clone().unwrap_or(MansardSpec { height: 3.0, inset: 1.0, upper: "hip".into(), upper_pitch: 15.0 });
    let c = poly::centroid(outer);
    let lo_ring = poly::offset(outer, overhang).unwrap_or(outer.to_vec());
    let hi_ring = poly::offset(outer, -m.inset).filter(|r| r.len() == outer.len()).unwrap_or_else(|| {
        let inr = (0..outer.len()).map(|i| poly::seg_dist(c, outer[i], outer[(i + 1) % outer.len()])).fold(f32::MAX, f32::min).max(0.2);
        outer.iter().map(|q| c + (*q - c) * (1.0 - m.inset / inr)).collect()
    });
    let lo_ring = if lo_ring.len() == hi_ring.len() { lo_ring } else { outer.to_vec() };
    let lo: Vec<Vec3> = lo_ring.iter().map(|q| Vec3::new(q.x, y - overhang * 0.5, q.y)).collect();
    let hi: Vec<Vec3> = hi_ring.iter().map(|q| Vec3::new(q.x, y + m.height, q.y)).collect();
    mesh::ring_band(o.geo.get(mat), &lo, &hi, true, Some(Vec3::new(c.x, y, c.y)));
    let hfn = |_: Vec2| y - overhang * 0.5;
    fascia(o.geo.get(mat), &lo_ring, &hfn, thick);
    let up = RoofParams { shape: if m.upper == "flat" { RoofShape::Flat } else { RoofShape::Hip }, pitch: Some(m.upper_pitch), height: None, overhang: Some(0.05), ..p.clone() };
    if m.upper == "flat" {
        mesh::horizontal_shape(o.geo.get(mat), &Shape::new(hi_ring.clone()), y + m.height, true);
    } else if m.upper == "pyramid" {
        apex(b, &RoofParams { shape: RoofShape::Pyramid, ..up }, &hi_ring, y + m.height, 0.05, thick, mat, o)?;
    } else if !hip(b, &up, &Shape::new(hi_ring.clone()), y + m.height, 0.05, thick, mat, caps, o)? {
        planar(b, &RoofParams { shape: RoofShape::Gable, ..up }, &hi_ring, y + m.height, 0.05, thick, mat, mat, caps, sc, o)?;
    }
    o.collide_all();
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn barrel(b: &mut Builder, p: &RoofParams, outer: &[Vec2], y: f32, overhang: f32, thick: f32, mat: MatId, wall: MatId, sc: &Scope, o: &mut Out) -> Result<()> {
    let rect = poly::min_rect(outer);
    let (e0, e1) = (rect[1] - rect[0], rect[2] - rect[1]);
    let (along, span_v, start) = if e0.length() >= e1.length() { (e0, e1, rect[0]) } else { (e1, -e0, rect[1]) };
    let len = along.length();
    let span = span_v.length();
    let ad = along / len;
    let sd = span_v / span;
    let kind = p.vault.unwrap_or(ArchKind::Round);
    let rise = p.height.unwrap_or_else(|| super::profiles::default_rise(kind, span).max(span * 0.15));
    let n = b.segs(span * 0.5, 10, 40);
    let ol = super::profiles::outline(kind, span, rise, Some(rise), n / 2);
    let head: Vec<Vec2> = ol.pts[ol.head.0..ol.head.1].to_vec();
    // Section point (across, up) -> 3D at distance t along.
    let at = |q: Vec2, t: f32, grow: f32| {
        let c = start + sd * (span * 0.5 + q.x * (1.0 + grow / span)) + ad * t;
        Vec3::new(c.x, y + q.y * (1.0 + grow / rise.max(0.1)), c.y)
    };
    let (t0, t1) = (-overhang, len + overhang);
    let k = head.len();
    let prim = o.geo.get(mat);
    for (grow, out) in [(thick, true), (0.0, false)] {
        let base = prim.positions.len() as u32;
        let mut acc = 0.0;
        for i in 0..k {
            if i > 0 {
                acc += (head[i] - head[i - 1]).length();
            }
            // Radial from the springing centre.
            let nn = head[i].normalize_or(Vec2::Y);
            let n3 = (Vec3::new(sd.x, 0.0, sd.y) * nn.x + Vec3::Y * nn.y).normalize_or(Vec3::Y);
            let n3 = if out { n3 } else { -n3 };
            for (t, u) in [(t0, t0), (t1, t1)] {
                prim.vertex(at(head[i], t, grow), n3, Vec2::new(u, -acc));
            }
        }
        for i in 0..k as u32 - 1 {
            let a = base + i * 2;
            let tri = [[a, a + 1, a + 3], [a, a + 3, a + 2]];
            for t in tri {
                let (pa, pb, pc) = (prim.positions[t[0] as usize], prim.positions[t[1] as usize], prim.positions[t[2] as usize]);
                let g = (pb - pa).cross(pc - pa);
                if g.dot(prim.normals[t[0] as usize]) >= 0.0 {
                    prim.indices.extend(t);
                } else {
                    prim.indices.extend([t[0], t[2], t[1]]);
                }
            }
        }
    }
    // End walls (arched gables) with optional openings.
    let shape = Shape::new(rect.clone());
    let mut gables = Vec::new();
    for (f, e) in super::masses::facades(&shape, y, y, wall) {
        let d = (rect[(e.i + 1) % 4] - rect[e.i]).normalize_or(Vec2::X);
        if d.dot(ad).abs() > 0.5 {
            continue;
        }
        let ol2 = super::profiles::outline(kind, f.len, rise, Some(rise), n / 2);
        let outline: Vec<Vec2> = ol2.pts.iter().map(|q| Vec2::new(q.x + f.len * 0.5, y + q.y)).collect();
        if let Some(outline) = poly::normalize(&outline, true) {
            let v1 = y + rise;
            gables.push((Facade { v1, outline: Some(outline), ..f }, e));
        }
    }
    gable_walls(b, p, gables, &shape, wall, sc, o)?;
    let _ = Plane::horizontal(0.0);
    o.collide_all();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hip_on_rectangle_has_four_faces_and_ridge() {
        // Build only the face polygons via the same clipping as `hip`.
        let outer = poly::rect(Vec2::ZERO, Vec2::new(20.0, 10.0), 0.0);
        let n = outer.len();
        let normals: Vec<Vec2> = (0..n).map(|i| poly::edge_normal(&outer, i)).collect();
        let mut areas = Vec::new();
        for i in 0..n {
            let mut f = outer.clone();
            for j in 0..n {
                if i == j {
                    continue;
                }
                let nn = normals[j] - normals[i];
                if nn.length_squared() < 1e-10 {
                    continue;
                }
                let d = outer[j].dot(normals[j]) - outer[i].dot(normals[i]);
                f = poly::clip_half_plane(&f, nn, d);
            }
            // The face of edge i touches edge i.
            let mid = (outer[i] + outer[(i + 1) % n]) * 0.5 - normals[i] * 0.1;
            assert!(poly::point_in_ring(&f, mid), "face {i} misses its eave");
            areas.push(poly::area(&f));
        }
        let total: f32 = areas.iter().sum();
        assert!((total - 200.0).abs() < 0.1, "{areas:?}");
        assert!(areas.iter().all(|a| *a > 10.0));
    }
}
