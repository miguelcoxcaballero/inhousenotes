//! Domes (hemispherical, pointed, onion, ellipsoidal, segmental, bell,
//! parabolic; ribbed or gored), drums with windows, lanterns and finials.

use forja_math::{Mat4, Vec2, Vec3};

use super::build::{Builder, Out, Scope};
use super::masses;
use super::mesh::{self, MatId};
use super::parse::{Error, Result};
use super::poly::{self, Shape};
use super::shapes;
use super::spec::*;

/// Unit profile (radius fraction, height fraction) at parameter t (0 =
/// springing, 1 = apex), for a dome of base radius 1 and rise `hn` (in
/// units of the radius).
fn profile(kind: DomeProfile, t: f32, hn: f32, k: f32) -> (f32, f32) {
    let t = t.clamp(0.0, 1.0);
    match kind {
        DomeProfile::Hemisphere | DomeProfile::Ellipsoid | DomeProfile::Ribbed => {
            let a = t * std::f32::consts::FRAC_PI_2;
            (a.cos(), a.sin())
        }
        DomeProfile::Pointed => {
            // Two arcs meeting at the apex; k widens the point.
            let h = hn * (1.0 + 0.0 * k);
            let big = (1.0 + h * h) / 2.0;
            let a1 = h.atan2(big - 1.0);
            let a = t * a1;
            ((1.0 - big) + big * a.cos(), big * a.sin() / h)
        }
        DomeProfile::Segmental => {
            let h = hn.clamp(0.02, 1.0);
            let rho = (1.0 + h * h) / (2.0 * h);
            let yc = h - rho;
            let a0 = (-yc / rho).clamp(-1.0, 1.0).asin();
            let a = a0 + (std::f32::consts::FRAC_PI_2 - a0) * t;
            (rho * a.cos(), (yc + rho * a.sin()) / h)
        }
        DomeProfile::Parabolic => {
            let y = t;
            ((1.0 - y).max(0.0).sqrt(), y)
        }
        DomeProfile::Onion | DomeProfile::Bell => {
            let pts: &[[f32; 2]] = if kind == DomeProfile::Onion {
                &[[1.0, 0.0], [1.0 + 0.16 * (1.0 + k), 0.2], [1.02, 0.45], [0.5, 0.72], [0.14, 0.9], [0.0, 1.0]]
            } else {
                &[[1.0, 0.0], [0.93, 0.12], [0.7, 0.32], [0.46, 0.58], [0.24, 0.84], [0.0, 1.0]]
            };
            let v: Vec<Vec3> = pts.iter().map(|p| Vec3::new(p[0], p[1], 0.0)).collect();
            let curve = shapes::curve_through(&v, 16, false);
            // Index by arc length.
            let mut acc = vec![0.0f32];
            for i in 1..curve.len() {
                let d = acc[i - 1] + (curve[i] - curve[i - 1]).length();
                acc.push(d);
            }
            let target = t * acc[acc.len() - 1];
            let i = acc.iter().position(|a| *a >= target).unwrap_or(acc.len() - 1).max(1);
            let f = ((target - acc[i - 1]) / (acc[i] - acc[i - 1]).max(1e-6)).clamp(0.0, 1.0);
            let p = curve[i - 1].lerp(curve[i], f);
            (p.x.max(0.0), p.y)
        }
    }
}

fn default_rise(kind: DomeProfile, r: f32) -> f32 {
    match kind {
        DomeProfile::Pointed => r * 1.35,
        DomeProfile::Onion => r * 1.5,
        DomeProfile::Segmental => r * 0.4,
        DomeProfile::Bell => r * 1.25,
        DomeProfile::Parabolic => r * 1.2,
        _ => r,
    }
}

/// The dome shell (and its ribs) springing at `c`. Returns the apex height
/// above `c` actually built (the top of an oculus rim).
#[allow(clippy::too_many_arguments)]
pub fn shell_geo(b: &mut Builder, mat: MatId, c: Vec3, rx: f32, rz: f32, h: f32, kind: DomeProfile, ribs: Option<&RibSpec>, gores: u32, gore_depth: f32, oculus: Option<f32>, rotation: f32, segments: Option<u32>, o: &mut Out) -> f32 {
    let r = rx.max(rz);
    let hn = h / r.max(1e-3);
    let mut nu = segments.map(|s| s as usize).unwrap_or_else(|| b.segs(r, 32, 128));
    if gores > 0 {
        nu = nu.div_ceil(gores as usize * 4) * gores as usize * 4;
    }
    let nv = (nu / 4).max(10);
    // Open top for an oculus.
    let t_top = match oculus {
        Some(orad) if orad > 0.0 => {
            let mut t = 1.0;
            for k in 0..200 {
                let tt = k as f32 / 200.0;
                if profile(kind, tt, hn, 0.5).0 * r <= orad {
                    t = tt;
                    break;
                }
            }
            t
        }
        _ => 1.0,
    };
    let rot = rotation.to_radians();
    let gore = |a: f32| -> f32 {
        if gores == 0 {
            return 1.0;
        }
        let f = (a / std::f32::consts::TAU * gores as f32).rem_euclid(1.0);
        1.0 - gore_depth * (1.0 - (f * std::f32::consts::PI).sin())
    };
    let point = |s: f32, t: f32| -> Vec3 {
        let a = s * std::f32::consts::TAU + rot;
        let (pr, py) = profile(kind, t * t_top, hn, 0.5);
        let g = gore(a - rot);
        c + Vec3::new(rx * pr * g * a.cos(), h * py, rz * pr * g * a.sin())
    };
    let mut prim = crate::geom::Prim::default();
    mesh::grid_surface(&mut prim, nu, nv, true, true, point);
    o.geo.get(mat).append(&prim, &Mat4::IDENTITY);
    if let Some(rs) = ribs {
        let rm = b.mat(rs.material.as_deref(), mat).unwrap_or(mat);
        let n = rs.count.max(1);
        let rg = o.geo.get(rm);
        for k in 0..n {
            let s = k as f32 / n as f32;
            let m = nv.max(8);
            let path: Vec<Vec3> = (0..=m).map(|j| point(s, j as f32 / m as f32)).collect();
            let eps = 1e-3;
            let side: Vec<Vec3> = (0..=m).map(|j| (point(s + eps, j as f32 / m as f32) - point(s - eps, j as f32 / m as f32)).normalize_or(Vec3::X)).collect();
            let up: Vec<Vec3> = (0..=m)
                .map(|j| {
                    let t = j as f32 / m as f32;
                    let dt = (point(s, (t + 0.01).min(1.0)) - point(s, (t - 0.01).max(0.0))).normalize_or(Vec3::Y);
                    let n = side[j].cross(dt).normalize_or(Vec3::Y);
                    let out = (path[j] - c) * Vec3::new(1.0, 0.0, 1.0);
                    if n.dot(out) + n.y * 0.2 < 0.0 { -n } else { n }
                })
                .collect();
            let hw = rs.width * 0.5;
            let prof = [Vec2::new(-hw, -0.02), Vec2::new(hw, -0.02), Vec2::new(hw * 0.8, rs.depth), Vec2::new(-hw * 0.8, rs.depth)];
            mesh::sweep(rg, &path, &side, &up, &prof, false, true, true, true);
        }
    }
    if let Some(orad) = oculus
        && orad > 0.0
    {
        let top = point(0.0, 1.0);
        mesh::torus(o.geo.get(mat), Vec3::new(c.x, top.y, c.z), Vec3::Y, orad + 0.08, 0.12, b.segs(orad, 16, 48), 8);
    }
    h * profile(kind, t_top, hn, 0.5).1
}

/// Synthetic window block for drums and lanterns.
#[allow(clippy::too_many_arguments)]
fn ring_openings(kind: OpeningKind, arch: ArchKind, infill: Infill, w: f32, h: f32, level: f32, edges: Vec<i64>, frame: f32) -> Openings {
    Openings {
        name: None,
        material: None,
        transform: None,
        mirror: None,
        enabled: true,
        collide: None,
        note: None,
        solid: None,
        on: None,
        edges: EdgeSel(edges.into_iter().map(EdgeTok::Index).collect()),
        // One opening per drum / lantern face, even on round drums.
        per_edge: true,
        kind,
        arch: Some(arch),
        width: w,
        height: h,
        rise: None,
        levels: Some(Levels(vec![level])),
        bays: Flex(Some(Bays { count: Some(1), ..Default::default() })),
        skip: Vec::new(),
        depth: 0.45,
        splay: 0.0,
        infill: Some(infill),
        frame,
        mullions: 0,
        transoms: 0,
        panes: None,
        surround: Flex(None),
        hood: Flex(None),
        sill: Flex(None),
        keystone: None,
        orders: 0,
        order_step: 0.2,
        colonnettes: false,
        tympanum: false,
        tracery: None,
        spokes: 0,
        rings: 0,
        balcony: Flex(None),
        voussoirs: Flex(None),
        bells: false,
        glass_material: None,
        frame_material: None,
        door_material: None,
        iron_material: None,
        stone_material: None,
    }
}

pub fn dome(b: &mut Builder, d: &Dome, sc: &Scope, o: &mut Out) -> Result<()> {
    let r = d.radius.or(d.rx.zip(d.rz).map(|(a, c)| a.max(c))).ok_or_else(|| Error::msg("dome needs `radius` (or `rx` and `rz`)"))?;
    let rx = d.rx.unwrap_or(r);
    let rz = d.rz.unwrap_or(r);
    if rx <= 0.0 || rz <= 0.0 {
        return Err(Error::msg("dome radius must be positive"));
    }
    if let Some(n) = d.segments
        && !(8..=512).contains(&n)
    {
        return Err(Error::msg(format!("segments = {n} is out of range (8..=512)")));
    }
    let c2 = Vec2::new(d.center[0], d.center[1]);
    let mut y = d.y;
    // Drum.
    if let Some(dr) = d.drum.get() {
        let dm = b.mat(dr.material.as_deref().or(d.drum_material.as_deref()), sc.mat)?;
        let rr = dr.radius.unwrap_or(rx.max(rz) + 0.3);
        let round = dr.sides == 0;
        let n = if round { (dr.windows as usize * 2).max(b.segs(rr, 24, 64)).next_multiple_of(dr.windows.max(1) as usize) } else { dr.sides as usize };
        let ring = poly::regular(c2, rr / (std::f32::consts::PI / n as f32).cos().max(0.5) * if round { (std::f32::consts::PI / n as f32).cos() } else { 1.0 }, n, d.rotation);
        let shape = Shape::new(ring);
        let ds = Scope { mat: dm, parent: Some(shape.clone()), ..sc.clone() };
        let ops = if dr.windows > 0 {
            let step = (n / dr.windows as usize).max(1);
            let edges: Vec<i64> = (0..dr.windows as usize).map(|k| (k * step) as i64).collect();
            let face = 2.0 * rr * (std::f32::consts::PI / n as f32).sin();
            let ww = if round { dr.window_width } else { dr.window_width.min(face * 0.7) };
            let wh = dr.window_height.min(dr.height * 0.85);
            vec![ring_openings(OpeningKind::Window, dr.window_arch, dr.window_infill, ww, wh, y + (dr.height - wh) * 0.5, edges, 0.06)]
        } else {
            Vec::new()
        };
        let mut refs: Vec<&Openings> = ops.iter().collect();
        refs.extend(dr.openings.iter());
        masses::prism_with_facades(b, &shape, y, y + dr.height, 0.0, round, Some(dm), y > 0.05, &refs, &ds, o)?;
        if dr.pilasters && !round {
            for q in &shape.outer {
                let out = (*q - c2).normalize_or(Vec2::Y);
                let p3 = Vec3::new(q.x, y, q.y);
                let n3 = Vec3::new(out.x, 0.0, out.y);
                o.geo.get(dm).obox(p3 + n3 * 0.05 + Vec3::Y * (dr.height * 0.5), [Vec3::Y.cross(n3), Vec3::Y, n3], Vec3::new(0.25, dr.height * 0.5, 0.12), 1.0);
            }
        }
        let cornice = dr.cornice.get().cloned().unwrap_or(MouldingSpec { profile: Some(ProfileSpec::Named("cornice".into())), size: Some((dr.height * 0.12).clamp(0.3, 0.8)), projection: None, y: None, edges: None, material: None, modillions: Flex(None), dentils: Flex(None) });
        let flex = Flex(Some(cornice));
        let none_m = Flex(None);
        let d0 = masses::Inline { cornice: &flex, plinth: &none_m, string_courses: &[], parapet: &Flex(None), crenellations: &Flex(None), balustrade: &Flex(None), roof: None };
        masses::inline_details(b, &shape, y, y + dr.height, &d0, &ds, o)?;
        y += dr.height;
    }
    let h = d.height.unwrap_or_else(|| default_rise(d.profile, rx.max(rz)));
    let ribs = d.ribs.get().cloned().or_else(|| (d.profile == DomeProfile::Ribbed).then(|| RibSpec { count: 8, width: 0.3 * (r / 5.0).max(0.4), depth: 0.18, material: None }));
    if d.base_ring {
        let m = b.segs(r, 24, 96);
        let circle: Vec<Vec2> = (0..m).map(|k| c2 + Vec2::from_angle(k as f32 / m as f32 * std::f32::consts::TAU) * Vec2::new(rx, rz)).collect();
        let circle = poly::normalize(&circle, true).unwrap_or(circle);
        let prof = super::details::profile_points(&ProfileSpec::Named("torus".into()), 0.35, Some(0.2));
        super::details::moulding_path(o, &circle, true, y - 0.1, &prof, sc.mat);
    }
    let top = shell_geo(b, sc.mat, Vec3::new(c2.x, y, c2.y), rx, rz, h, d.profile, ribs.as_ref(), d.gores, d.gore_depth, d.oculus, d.rotation, d.segments, o);
    o.collide_all();
    // Lantern and finial.
    let apex = Vec3::new(c2.x, y + top, c2.y);
    let mut lantern_top = 0.0;
    if !d.lantern.0.is_empty() {
        let mut lo = Out::default();
        let parent = Shape::new(poly::regular(Vec2::ZERO, d.oculus.unwrap_or(r * 0.16).max(0.3), 16, 0.0));
        b.build_children(&d.lantern.0, sc, Mat4::from_translation(apex), Some(parent), &mut lo)?;
        if let Some((_, mx)) = mesh::bounds(&lo.geo) {
            lantern_top = mx.y - apex.y;
        }
        o.append(&lo, &Mat4::IDENTITY);
    }
    if !d.finial.0.is_empty() {
        b.build_children(&d.finial.0, sc, Mat4::from_translation(apex + Vec3::Y * lantern_top), None, o)?;
    }
    Ok(())
}

pub fn lantern(b: &mut Builder, l: &Lantern, sc: &Scope, o: &mut Out) -> Result<()> {
    let at = Vec3::from_array(l.at);
    let sides = l.sides.max(4) as usize;
    let c = Vec2::new(at.x, at.z);
    let ring = poly::regular(c, l.radius, sides, 0.0);
    let shape = Shape::new(ring.clone());
    let y0 = at.y;
    // Base plinth.
    let base_ring = poly::regular(c, l.radius * 1.12, sides, 0.0);
    mesh::prism(o.geo.get(sc.mat), &Shape::new(base_ring), y0, y0 + 0.25, true, true);
    let face = 2.0 * l.radius * (std::f32::consts::PI / sides as f32).sin();
    let wh = (l.height - 0.6).max(0.5);
    let ops = vec![ring_openings(OpeningKind::Arch, ArchKind::Round, Infill::Glass, (face * 0.55).max(0.2), wh, y0 + 0.4, (0..sides as i64).collect(), 0.04)];
    let refs: Vec<&Openings> = ops.iter().collect();
    let ls = Scope { parent: Some(shape.clone()), ..sc.clone() };
    masses::prism_with_facades(b, &shape, y0 + 0.25, y0 + l.height, 0.0, false, Some(sc.mat), false, &refs, &ls, o)?;
    let corn = Flex(Some(MouldingSpec { profile: Some(ProfileSpec::Named("cornice".into())), size: Some(0.3), projection: None, y: None, edges: None, material: None, modillions: Flex(None), dentils: Flex(None) }));
    let none = Flex(None);
    let d0 = masses::Inline { cornice: &corn, plinth: &none, string_courses: &[], parapet: &Flex(None), crenellations: &Flex(None), balustrade: &Flex(None), roof: None };
    masses::inline_details(b, &shape, y0, y0 + l.height, &d0, &ls, o)?;
    let cap_m = b.mat(l.cap_material.as_deref(), sc.mat)?;
    let top = y0 + l.height;
    let r = l.radius * 1.05;
    let cap_h = l.cap_height.unwrap_or(match l.cap.as_str() {
        "cone" | "spire" => l.radius * 2.6,
        "onion" => l.radius * 1.8,
        "flat" => 0.2,
        _ => l.radius * 1.0,
    });
    let cap_top;
    match l.cap.as_str() {
        "flat" => {
            mesh::prism(o.geo.get(cap_m), &Shape::new(poly::regular(c, r, sides, 0.0)), top, top + cap_h, true, false);
            cap_top = top + cap_h;
        }
        "cone" | "spire" => {
            let base = Vec3::new(c.x, top, c.y);
            o.geo.get(cap_m).cone(base, base + Vec3::Y * cap_h, r, 0.02, if l.cap == "spire" { sides as u32 } else { 24 }, true);
            cap_top = top + cap_h;
        }
        "onion" => {
            cap_top = top + shell_geo(b, cap_m, Vec3::new(c.x, top, c.y), r, r, cap_h, DomeProfile::Onion, None, 0, 0.0, None, 0.0, None, o);
        }
        "dome" => {
            cap_top = top + shell_geo(b, cap_m, Vec3::new(c.x, top, c.y), r, r, cap_h, DomeProfile::Hemisphere, None, 0, 0.0, None, 0.0, None, o);
        }
        other => return Err(Error::msg(format!("unknown lantern cap `{other}`{}; use dome, cone, onion, spire or flat", super::parse::did_you_mean(other, &["dome", "cone", "onion", "spire", "flat"])))),
    }
    if !l.finial.0.is_empty() {
        b.build_children(&l.finial.0, sc, Mat4::from_translation(Vec3::new(c.x, cap_top, c.y)), None, o)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_start_at_the_base_and_end_at_the_apex() {
        for (name, k) in DomeProfile::NAMES {
            let (r0, y0) = profile(*k, 0.0, 1.2, 0.5);
            let (r1, y1) = profile(*k, 1.0, 1.2, 0.5);
            assert!((r0 - 1.0).abs() < 0.02 && y0.abs() < 0.02, "{name} base {r0} {y0}");
            assert!(r1.abs() < 0.03 && (y1 - 1.0).abs() < 0.03, "{name} apex {r1} {y1}");
        }
    }
}
