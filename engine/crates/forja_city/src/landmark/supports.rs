//! Columns (orders, flutes, Solomonic twists, Gothic clustered piers),
//! pilasters, buttresses with set-offs, flying buttresses, pinnacles,
//! finials, statues, stairs and arcades / loggias.

use forja_math::{Mat4, Vec2, Vec3};

use super::build::{Builder, Out, Scope, facing_matrix};
use super::facade::{self, Cut, Facade};
use super::mesh::{self, MatId, Plane};
use super::parse::{Error, Result};
use super::poly::{self, Shape};
use super::profiles;
use super::resolve;
use super::spec::*;
use crate::geom::Prim;

fn lathe_full(p: &mut Prim, c: Vec3, prof: &[(f32, f32)], seg: usize) {
    let v: Vec<Vec2> = prof.iter().map(|(r, y)| Vec2::new(*r, *y)).collect();
    mesh::lathe(p, c, &v, seg, 0.0, std::f32::consts::TAU);
}

/// Column geometry in its own frame (base centre at the origin).
#[allow(clippy::too_many_arguments)]
pub fn column_geo(b: &Builder, p: &mut Prim, height: f32, r: f32, order: ColumnOrder, entasis: Option<f32>, flutes: u32, base: bool, capital: bool, pedestal: Option<f32>) {
    column_geo_n(b, p, height, r, order, entasis, flutes, base, capital, pedestal, None)
}

/// [`column_geo`] with an explicit shaft resolution.
#[allow(clippy::too_many_arguments)]
pub fn column_geo_n(b: &Builder, p: &mut Prim, height: f32, r: f32, order: ColumnOrder, entasis: Option<f32>, flutes: u32, base: bool, capital: bool, pedestal: Option<f32>, segments: Option<usize>) {
    let seg = segments.unwrap_or_else(|| b.segs(r, 12, 40)).max(flutes as usize * 2);
    let mut y = 0.0;
    if let Some(ph) = pedestal {
        let w = r * 2.8;
        p.aabox(Vec3::new(-w * 0.5, 0.0, -w * 0.5), Vec3::new(w * 0.5, ph, w * 0.5), 1.0);
        p.aabox(Vec3::new(-w * 0.56, ph - 0.12, -w * 0.56), Vec3::new(w * 0.56, ph, w * 0.56), 1.0);
        y = ph;
    }
    let total = height - y;
    let has_base = base && !matches!(order, ColumnOrder::Cylinder);
    let has_cap = capital && !matches!(order, ColumnOrder::Cylinder);
    let base_h = if has_base { (r * 0.75).min(total * 0.12) } else { 0.0 };
    let cap_h = if has_cap {
        match order {
            ColumnOrder::Corinthian => (r * 2.3).min(total * 0.2),
            ColumnOrder::Ionic => (r * 1.0).min(total * 0.14),
            ColumnOrder::Gothic => (r * 1.2).min(total * 0.15),
            _ => (r * 0.85).min(total * 0.12),
        }
    } else {
        0.0
    };
    let o = Vec3::new(0.0, y, 0.0);
    // Base.
    if has_base {
        match order {
            ColumnOrder::Pier => p.aabox(o + Vec3::new(-r * 1.25, 0.0, -r * 1.25), o + Vec3::new(r * 1.25, base_h, r * 1.25), 1.0),
            _ => {
                p.aabox(o + Vec3::new(-r * 1.4, 0.0, -r * 1.4), o + Vec3::new(r * 1.4, base_h * 0.3, r * 1.4), 1.0);
                let k = base_h;
                lathe_full(p, o + Vec3::Y * (base_h * 0.3), &[(0.0, 0.0), (r * 1.3, 0.0), (r * 1.33, k * 0.12), (r * 1.25, k * 0.3), (r * 1.08, k * 0.38), (r * 1.04, k * 0.46), (r * 1.16, k * 0.52), (r * 1.12, k * 0.64), (r * 1.0, k * 0.7), (0.0, k * 0.7)], seg);
            }
        }
    }
    // Shaft.
    let s0 = y + base_h;
    let s1 = height - cap_h;
    let sh = (s1 - s0).max(0.05);
    let ent = entasis.unwrap_or(match order {
        ColumnOrder::Doric | ColumnOrder::Tuscan | ColumnOrder::Ionic | ColumnOrder::Corinthian => 0.04,
        _ => 0.0,
    });
    let taper = match order {
        ColumnOrder::Doric | ColumnOrder::Tuscan | ColumnOrder::Ionic | ColumnOrder::Corinthian => 0.86,
        _ => 1.0,
    };
    let rad = move |t: f32| r * (1.0 + (taper - 1.0) * t + ent * (std::f32::consts::PI * t).sin());
    match order {
        ColumnOrder::Pier => p.aabox(Vec3::new(-r, s0, -r), Vec3::new(r, s1, r), 1.0),
        ColumnOrder::Octagonal => {
            let ring = poly::regular(Vec2::ZERO, r / (std::f32::consts::PI / 8.0).cos(), 8, 22.5);
            mesh::prism(p, &Shape::new(ring), s0, s1, false, false);
        }
        ColumnOrder::Gothic => {
            // Clustered pier: core with four attached shafts.
            p.cylinder(Vec3::new(0.0, s0, 0.0), Vec3::new(0.0, s1, 0.0), r * 0.8, seg as u32, false);
            for k in 0..4 {
                let a = k as f32 * std::f32::consts::FRAC_PI_2 + std::f32::consts::FRAC_PI_4;
                let d = Vec3::new(a.cos(), 0.0, a.sin()) * (r * 0.72);
                p.cylinder(Vec3::new(0.0, s0, 0.0) + d, Vec3::new(0.0, s1, 0.0) + d, r * 0.34, 12, false);
            }
            for k in 0..4 {
                let a = k as f32 * std::f32::consts::FRAC_PI_2;
                let d = Vec3::new(a.cos(), 0.0, a.sin()) * (r * 0.82);
                p.cylinder(Vec3::new(0.0, s0, 0.0) + d, Vec3::new(0.0, s1, 0.0) + d, r * 0.2, 10, false);
            }
        }
        _ => {
            let fl = flutes as f32;
            let twist = order == ColumnOrder::Salomonic;
            let nu = if flutes > 0 { (flutes as usize * 4).max(seg) } else { seg };
            let nv = if twist { 48 } else { 8 };
            mesh::grid_surface(p, nu, nv, true, true, |s, t| {
                let a = s * std::f32::consts::TAU;
                let mut rr = rad(t);
                if flutes > 0 {
                    rr *= 1.0 - 0.06 * (1.0 - ((a * fl * 0.5).sin()).abs()).powf(0.5);
                }
                if twist {
                    rr *= 1.0 + 0.2 * (2.0 * a + t * sh / r * 1.1).sin();
                }
                Vec3::new(rr * a.cos(), s0 + sh * t, rr * a.sin())
            });
        }
    }
    // Capital.
    if has_cap {
        let c = Vec3::new(0.0, s1, 0.0);
        let rt = rad(1.0);
        let ab = cap_h * 0.28;
        match order {
            ColumnOrder::Corinthian => {
                lathe_full(p, c, &[(0.0, 0.0), (rt * 1.04, 0.0), (rt * 1.0, cap_h * 0.12), (rt * 1.12, cap_h * 0.5), (rt * 1.4, cap_h * 0.72), (0.0, cap_h * 0.72)], seg);
                for row in 0..2 {
                    for k in 0..8 {
                        let a = (k as f32 + row as f32 * 0.5) / 8.0 * std::f32::consts::TAU;
                        let d = Vec3::new(a.cos(), 0.0, a.sin());
                        let y0 = cap_h * (0.05 + 0.25 * row as f32);
                        let base_p = c + d * (rt * 1.02) + Vec3::Y * y0;
                        let tip = base_p + d * (rt * 0.32) + Vec3::Y * cap_h * 0.32;
                        p.cone(base_p, tip, rt * 0.22, rt * 0.04, 6, true);
                    }
                }
                let w = rt * 1.55;
                p.aabox(c + Vec3::new(-w, cap_h - ab, -w), c + Vec3::new(w, cap_h, w), 1.0);
            }
            ColumnOrder::Ionic => {
                lathe_full(p, c, &[(0.0, 0.0), (rt * 1.02, 0.0), (rt * 1.2, cap_h * 0.35), (rt * 1.1, cap_h * 0.55), (0.0, cap_h * 0.55)], seg);
                for s in [-1.0f32, 1.0] {
                    let vc = c + Vec3::new(s * rt * 1.15, cap_h * 0.42, 0.0);
                    p.cylinder(vc - Vec3::Z * rt * 1.1, vc + Vec3::Z * rt * 1.1, cap_h * 0.32, 14, true);
                }
                let w = rt * 1.45;
                p.aabox(c + Vec3::new(-w, cap_h - ab, -w * 0.9), c + Vec3::new(w, cap_h, w * 0.9), 1.0);
            }
            ColumnOrder::Gothic => {
                lathe_full(p, c, &[(0.0, 0.0), (r * 1.1, 0.0), (r * 1.15, cap_h * 0.15), (r * 1.25, cap_h * 0.55), (r * 1.45, cap_h * 0.72), (0.0, cap_h * 0.72)], seg);
                let ring = poly::regular(Vec2::ZERO, r * 1.6, 8, 22.5);
                mesh::prism(p, &Shape::new(ring), c.y + cap_h * 0.72, c.y + cap_h, true, true);
            }
            ColumnOrder::Pier => {
                p.aabox(c + Vec3::new(-r * 1.2, 0.0, -r * 1.2), c + Vec3::new(r * 1.2, cap_h * 0.6, r * 1.2), 1.0);
                p.aabox(c + Vec3::new(-r * 1.35, cap_h * 0.6, -r * 1.35), c + Vec3::new(r * 1.35, cap_h, r * 1.35), 1.0);
            }
            _ => {
                lathe_full(p, c, &[(0.0, 0.0), (rt * 1.04, 0.0), (rt * 1.04, cap_h * 0.12), (rt * 1.0, cap_h * 0.2), (rt * 1.3, cap_h * 0.65), (0.0, cap_h * 0.65)], seg);
                let w = rt * 1.42;
                p.aabox(c + Vec3::new(-w, cap_h * 0.65, -w), c + Vec3::new(w, cap_h, w), 1.0);
            }
        }
    }
}

pub fn column_el(b: &mut Builder, c: &Column, sc: &Scope, o: &mut Out) -> Result<()> {
    let mut p = Prim::default();
    let seg = c.segments.map(|_| b.user_segs(c.segments, 16, 3, 512, "segments")).transpose()?;
    column_geo_n(b, &mut p, c.height, c.radius, c.order, c.entasis, c.flutes, c.base, c.capital, c.pedestal, seg);
    o.geo.get(sc.mat).append(&p, &Mat4::from_translation(Vec3::from_array(c.at)));
    Ok(())
}

pub fn pilaster(b: &mut Builder, pl: &Pilaster, sc: &Scope, o: &mut Out) -> Result<()> {
    let m = facing_matrix(pl.at, pl.facing);
    let mut p = Prim::default();
    let (w, d, h) = (pl.width, pl.depth, pl.height);
    let bh = if pl.base { (w * 0.4).min(h * 0.1) } else { 0.0 };
    let ch = if pl.capital { (w * 0.55).min(h * 0.12) } else { 0.0 };
    if pl.base {
        p.aabox(Vec3::new(-w * 0.62, 0.0, 0.0), Vec3::new(w * 0.62, bh, d + 0.06), 1.0);
    }
    p.aabox(Vec3::new(-w * 0.5, bh, 0.0), Vec3::new(w * 0.5, h - ch, d), 1.0);
    if pl.flutes > 0 {
        let n = pl.flutes as usize;
        for k in 0..n {
            let x = -w * 0.38 + w * 0.76 * (k as f32 + 0.5) / n as f32;
            p.aabox(Vec3::new(x - w * 0.02, bh + 0.15, d), Vec3::new(x + w * 0.02, h - ch - 0.15, d + 0.015), 1.0);
        }
    }
    if pl.capital {
        p.aabox(Vec3::new(-w * 0.55, h - ch, 0.0), Vec3::new(w * 0.55, h - ch * 0.35, d + 0.04), 1.0);
        p.aabox(Vec3::new(-w * 0.66, h - ch * 0.35, 0.0), Vec3::new(w * 0.66, h, d + 0.09), 1.0);
        if pl.order == ColumnOrder::Ionic || pl.order == ColumnOrder::Corinthian {
            for s in [-1.0f32, 1.0] {
                let c = Vec3::new(s * w * 0.45, h - ch * 0.55, d + 0.02);
                p.cylinder(c - Vec3::Z * 0.05, c + Vec3::Z * 0.08, ch * 0.3, 10, true);
            }
        }
    }
    let _ = b;
    o.geo.get(sc.mat).append(&p, &m);
    Ok(())
}

/// Side-profile polygon (z out, y up) of a buttress.
fn buttress_profile(bt: &Buttress) -> Vec<Vec2> {
    let mut stages: Vec<(f32, f32)> = Vec::new(); // (top y, depth)
    if !bt.steps.is_empty() {
        let mut y = 0.0;
        for s in &bt.steps {
            y += s.height;
            stages.push((y, s.depth));
        }
    } else {
        let n = bt.setoffs as usize + 1;
        for k in 0..n {
            let y = bt.height * (k + 1) as f32 / n as f32;
            let d = bt.depth * (1.0 - 0.45 * k as f32 / n.max(1) as f32);
            stages.push((y, d));
        }
    }
    let mut pts = vec![Vec2::new(0.0, 0.0), Vec2::new(stages[0].1, 0.0)];
    for k in 0..stages.len() {
        let (top, d) = stages[k];
        if k + 1 < stages.len() {
            let nd = stages[k + 1].1;
            let wh = ((d - nd) * 0.9).clamp(0.1, 0.6);
            pts.push(Vec2::new(d, top - wh));
            pts.push(Vec2::new(nd, top));
        } else {
            let cap = match bt.cap.as_str() {
                "flat" | "pinnacle" => 0.0,
                _ => d * 0.75,
            };
            pts.push(Vec2::new(d, top));
            pts.push(Vec2::new(0.0, top + cap));
        }
    }
    pts
}

pub fn buttress(b: &mut Builder, bt: &Buttress, sc: &Scope, o: &mut Out) -> Result<()> {
    if bt.height <= 0.0 || bt.width <= 0.0 || bt.depth <= 0.0 {
        return Err(Error::msg("buttress needs positive height, width and depth"));
    }
    let m = facing_matrix(bt.at, bt.facing);
    let prof = buttress_profile(bt);
    let mut p = Prim::default();
    if let Some(r) = poly::normalize(&prof, true) {
        // Profile in (z, y), extruded across x.
        let plane = Plane { o: Vec3::new(bt.width * 0.5, 0.0, 0.0), u: Vec3::Z, v: Vec3::Y, n: Vec3::NEG_X };
        mesh::extrude_shape(&mut p, &Shape::new(r), &plane, bt.width);
    }
    if bt.plinth {
        let d = prof.iter().map(|q| q.x).fold(0.0, f32::max);
        p.aabox(Vec3::new(-bt.width * 0.5 - 0.12, 0.0, 0.0), Vec3::new(bt.width * 0.5 + 0.12, 0.6, d + 0.12), 1.0);
    }
    o.geo.get(sc.mat).append(&p, &m);
    let top = prof.iter().map(|q| q.y).fold(0.0, f32::max);
    if let Some(pn) = bt.pinnacle.get().or(if bt.cap == "pinnacle" { Some(&PinnacleSpec { width: 0.0, height: 0.0, sides: 4, crockets: 3, material: None }) } else { None }) {
        let last_d = prof[prof.len() - 2].x;
        let w = if pn.width > 0.0 { pn.width } else { bt.width.min(last_d) * 0.8 };
        let h = if pn.height > 0.0 { pn.height } else { w * 4.5 };
        let pm = b.mat(pn.material.as_deref(), sc.mat)?;
        let mut g = Out::default();
        pinnacle_geo(b, pm, w, h, pn.sides, pn.crockets, 0.45, &mut g);
        o.append(&g, &(m * Mat4::from_translation(Vec3::new(0.0, top, last_d * 0.5))));
    }
    Ok(())
}

pub fn flying_buttress(b: &mut Builder, f: &FlyingButtress, sc: &Scope, o: &mut Out) -> Result<()> {
    let a = Vec3::from_array(f.from);
    let c = Vec3::from_array(f.to);
    let hd = Vec2::new(c.x - a.x, c.z - a.z);
    let span = hd.length();
    if span < 0.2 {
        return Err(Error::msg("flying buttress `from` and `to` are too close in plan"));
    }
    let dir = hd / span;
    let side = Vec3::new(-dir.y, 0.0, dir.x);
    let t = f.thickness;
    // Profile in (s along the span, y).
    let n = 16;
    let mut pts = vec![Vec2::new(0.0, a.y - t), Vec2::new(0.0, a.y + t * 0.25)];
    pts.push(Vec2::new(span, c.y));
    pts.push(Vec2::new(span, c.y - t * 0.6));
    // Underside: arc sagging below the chord.
    for k in (1..n).rev() {
        let s = k as f32 / n as f32;
        let chord = (a.y - t) + ((c.y - t * 0.6) - (a.y - t)) * s;
        let sag = f.sag * span * 0.5 * (std::f32::consts::PI * s).sin();
        pts.push(Vec2::new(span * s, chord - sag));
    }
    let mut p = Prim::default();
    if let Some(r) = poly::normalize(&pts, true) {
        let u = Vec3::new(dir.x, 0.0, dir.y);
        let plane = Plane::new(Vec3::new(a.x, 0.0, a.z) - side * (f.width * 0.5) * if u.cross(Vec3::Y).dot(side) > 0.0 { 1.0 } else { -1.0 }, u, Vec3::Y);
        mesh::extrude_shape(&mut p, &Shape::new(r), &plane, f.width);
    }
    o.geo.get(sc.mat).append(&p, &Mat4::IDENTITY);
    if let Some(pr) = f.pier.get() {
        let pm = b.mat(pr.material.as_deref(), sc.mat)?;
        let d = pr.depth.unwrap_or(pr.width * 1.6);
        let center = Vec3::new(a.x, 0.0, a.z) - Vec3::new(dir.x, 0.0, dir.y) * (d * 0.5 - 0.1);
        o.geo.get(pm).obox(center + Vec3::Y * ((a.y + t * 0.25) * 0.5), [side, Vec3::Y, Vec3::new(dir.x, 0.0, dir.y)], Vec3::new(pr.width * 0.5, (a.y + t * 0.25) * 0.5, d * 0.5), 1.0);
        if let Some(pn) = f.pinnacle.get() {
            let pm2 = b.mat(pn.material.as_deref(), pm)?;
            let mut g = Out::default();
            pinnacle_geo(b, pm2, pn.width, pn.height, pn.sides, pn.crockets, 0.45, &mut g);
            o.append(&g, &Mat4::from_translation(center + Vec3::Y * (a.y + t * 0.25)));
        }
    }
    o.collide_all();
    Ok(())
}

/// Pinnacle with its base at the origin.
#[allow(clippy::too_many_arguments)]
pub fn pinnacle_geo(b: &Builder, mat: MatId, w: f32, h: f32, sides: u32, crockets: u32, shaft: f32, o: &mut Out) {
    let sides = sides.max(4) as usize;
    let p = o.geo.get(mat);
    let sh = h * shaft.clamp(0.1, 0.9);
    let rr = w * 0.5 / (std::f32::consts::PI / sides as f32).cos();
    let ring = poly::regular(Vec2::ZERO, rr, sides, if sides == 4 { 45.0 } else { 0.0 });
    let ring = if sides == 4 { poly::rect(Vec2::ZERO, Vec2::new(w, w), 0.0) } else { ring };
    mesh::prism(p, &Shape::new(ring.clone()), 0.0, sh, false, true);
    // Gablets on each face.
    let gh = w * 0.6;
    let m = ring.len();
    for i in 0..m {
        let (a, c) = (ring[i], ring[(i + 1) % m]);
        let mid = (a + c) * 0.5;
        let nn = poly::edge_normal(&ring, i);
        let apex = Vec3::new(mid.x + nn.x * 0.02, sh + gh, mid.y + nn.y * 0.02);
        mesh::tri_facing(p, Vec3::new(c.x, sh, c.y), Vec3::new(a.x, sh, a.y), apex, [Vec2::ZERO, Vec2::new(w, 0.0), Vec2::new(w * 0.5, -gh)], Vec3::new(nn.x, 0.2, nn.y));
    }
    // Spire.
    let top = Vec3::new(0.0, h * 0.94, 0.0);
    let sr = rr * 0.92;
    let sring = if sides == 4 { poly::rect(Vec2::ZERO, Vec2::new(w * 0.9, w * 0.9), 0.0) } else { poly::regular(Vec2::ZERO, sr, sides, 0.0) };
    let k = sring.len();
    for i in 0..k {
        let (a, c) = (sring[i], sring[(i + 1) % k]);
        let a3 = Vec3::new(a.x, sh, a.y);
        let c3 = Vec3::new(c.x, sh, c.y);
        let out = Vec3::new(a.x + c.x, 0.0, a.y + c.y);
        mesh::tri_facing(p, a3, c3, top, [Vec2::ZERO, Vec2::new(w, 0.0), Vec2::new(w * 0.5, -(h - sh))], out + Vec3::Y * 0.1);
    }
    // Crockets along the spire's edges.
    for i in 0..k {
        let a = Vec3::new(sring[i].x, sh, sring[i].y);
        for j in 1..=crockets {
            let t = j as f32 / (crockets + 1) as f32;
            let q = a.lerp(top, t);
            let out = Vec3::new(q.x, 0.0, q.z).normalize_or(Vec3::X);
            let s = w * 0.12 * (1.0 - t * 0.5);
            p.cone(q, q + out * s * 1.6 + Vec3::Y * s, s, s * 0.2, 5, true);
        }
    }
    // Finial (fleuron).
    p.cone(top - Vec3::Y * 0.05, top + Vec3::Y * h * 0.06, w * 0.1, 0.0, 6, true);
    mesh::ellipsoid(p, top, Vec3::splat(w * 0.11), (0.0, std::f32::consts::TAU), (-1.4, 1.4), 8, 5);
    let _ = b;
}

pub fn pinnacle_el(b: &mut Builder, pn: &Pinnacle, sc: &Scope, o: &mut Out) -> Result<()> {
    let mut g = Out::default();
    pinnacle_geo(b, sc.mat, pn.width, pn.height, pn.sides, pn.crockets, pn.shaft, &mut g);
    o.append(&g, &Mat4::from_translation(Vec3::from_array(pn.at)));
    Ok(())
}

/// Finial geometry with its base at the origin, `size` tall.
pub fn finial_geo(p: &mut Prim, style: FinialStyle, size: f32) {
    let s = size;
    match style {
        FinialStyle::Cross => {
            mesh::ellipsoid(p, Vec3::Y * (s * 0.12), Vec3::splat(s * 0.1), (0.0, std::f32::consts::TAU), (-1.5, 1.5), 10, 6);
            let bw = s * 0.06;
            p.aabox(Vec3::new(-bw, s * 0.2, -bw), Vec3::new(bw, s, bw), 1.0);
            p.aabox(Vec3::new(-s * 0.28, s * 0.66 - bw, -bw), Vec3::new(s * 0.28, s * 0.66 + bw, bw), 1.0);
        }
        FinialStyle::Ball => {
            lathe_full(p, Vec3::ZERO, &[(0.0, 0.0), (s * 0.18, 0.0), (s * 0.08, s * 0.2), (0.0, s * 0.22)], 10);
            mesh::ellipsoid(p, Vec3::Y * (s * 0.6), Vec3::splat(s * 0.38), (0.0, std::f32::consts::TAU), (-1.5, 1.5), 14, 8);
        }
        FinialStyle::Spike => p.cone(Vec3::ZERO, Vec3::Y * s, s * 0.08, 0.0, 8, true),
        FinialStyle::Weathervane => {
            p.cylinder(Vec3::ZERO, Vec3::Y * s, s * 0.02, 6, true);
            mesh::ellipsoid(p, Vec3::Y * (s * 0.35), Vec3::splat(s * 0.07), (0.0, std::f32::consts::TAU), (-1.5, 1.5), 8, 5);
            p.aabox(Vec3::new(-s * 0.35, s * 0.78, -0.01), Vec3::new(s * 0.35, s * 0.82, 0.01), 1.0);
            p.aabox(Vec3::new(s * 0.2, s * 0.72, -0.008), Vec3::new(s * 0.35, s * 0.88, 0.008), 1.0);
            p.aabox(Vec3::new(-0.01, s * 0.55, -s * 0.2), Vec3::new(0.01, s * 0.57, s * 0.2), 1.0);
            p.aabox(Vec3::new(-s * 0.2, s * 0.55, -0.01), Vec3::new(s * 0.2, s * 0.57, 0.01), 1.0);
        }
        FinialStyle::Urn => lathe_full(p, Vec3::ZERO, &[(0.0, 0.0), (s * 0.2, 0.0), (s * 0.22, s * 0.06), (s * 0.12, s * 0.15), (s * 0.26, s * 0.35), (s * 0.3, s * 0.52), (s * 0.24, s * 0.68), (s * 0.14, s * 0.72), (s * 0.18, s * 0.78), (s * 0.06, s * 0.86), (s * 0.04, s * 0.95), (0.0, s)], 14),
        FinialStyle::Pinecone => lathe_full(p, Vec3::ZERO, &[(0.0, 0.0), (s * 0.16, 0.0), (s * 0.08, s * 0.1), (s * 0.2, s * 0.3), (s * 0.19, s * 0.55), (s * 0.1, s * 0.82), (0.0, s)], 12),
        FinialStyle::Fleur => {
            p.cylinder(Vec3::ZERO, Vec3::Y * (s * 0.4), s * 0.05, 8, true);
            for k in 0..3 {
                let a = k as f32 * std::f32::consts::TAU / 3.0;
                let d = Vec3::new(a.cos(), 0.0, a.sin());
                p.cone(Vec3::Y * (s * 0.4), Vec3::Y * (s * 0.4) + d * (s * 0.3) + Vec3::Y * (s * 0.35), s * 0.08, 0.0, 6, true);
            }
            p.cone(Vec3::Y * (s * 0.4), Vec3::Y * s, s * 0.09, 0.0, 6, true);
        }
        FinialStyle::Crown => {
            p.cylinder(Vec3::ZERO, Vec3::Y * (s * 0.35), s * 0.3, 16, true);
            for k in 0..8 {
                let a = k as f32 * std::f32::consts::TAU / 8.0;
                let d = Vec3::new(a.cos(), 0.0, a.sin()) * (s * 0.27);
                p.cone(d + Vec3::Y * (s * 0.35), d + Vec3::Y * (s * 0.7), s * 0.05, 0.0, 5, true);
            }
            mesh::ellipsoid(p, Vec3::Y * (s * 0.8), Vec3::splat(s * 0.12), (0.0, std::f32::consts::TAU), (-1.5, 1.5), 10, 6);
        }
        FinialStyle::Star => {
            p.cylinder(Vec3::ZERO, Vec3::Y * (s * 0.5), s * 0.03, 6, true);
            let pts: Vec<Vec2> = (0..10).map(|k| Vec2::from_angle(k as f32 * std::f32::consts::TAU / 10.0 + std::f32::consts::FRAC_PI_2) * (if k % 2 == 0 { s * 0.25 } else { s * 0.1 }) + Vec2::new(0.0, s * 0.72)).collect();
            if let Some(r) = poly::normalize(&pts, true) {
                mesh::extrude_shape(p, &Shape::new(r), &Plane::new(Vec3::new(0.0, 0.0, -0.02), Vec3::X, Vec3::Y), 0.04);
            }
        }
        FinialStyle::Statue => statue_geo(p, StatueStyle::Figure, s),
    }
}

pub fn finial_el(b: &mut Builder, f: &Finial, sc: &Scope, o: &mut Out) -> Result<()> {
    let mut p = Prim::default();
    finial_geo(&mut p, f.style, f.size);
    let m = facing_matrix(f.at, f.facing);
    let _ = b;
    o.geo.get(sc.mat).append(&p, &m);
    Ok(())
}

/// Sculpture placeholder, base at the origin, facing +z.
pub fn statue_geo(p: &mut Prim, style: StatueStyle, h: f32) {
    let figure = |p: &mut Prim, o: Vec3, h: f32| {
        // Robed body, shoulders, arms and head.
        lathe_full(p, o, &[(0.0, 0.0), (h * 0.14, 0.0), (h * 0.15, h * 0.08), (h * 0.12, h * 0.4), (h * 0.1, h * 0.62), (h * 0.12, h * 0.78), (h * 0.06, h * 0.84), (0.0, h * 0.85)], 14);
        mesh::ellipsoid(p, o + Vec3::Y * (h * 0.92), Vec3::new(h * 0.065, h * 0.08, h * 0.07), (0.0, std::f32::consts::TAU), (-1.5, 1.5), 12, 8);
        for s in [-1.0f32, 1.0] {
            let sh = o + Vec3::new(s * h * 0.12, h * 0.78, 0.0);
            let el = o + Vec3::new(s * h * 0.15, h * 0.58, h * 0.04);
            let ha = o + Vec3::new(s * h * 0.08, h * 0.5, h * 0.1);
            p.beam(sh, el, Vec3::X, h * 0.05, h * 0.05, 1.0);
            p.beam(el, ha, Vec3::X, h * 0.045, h * 0.045, 1.0);
        }
    };
    match style {
        StatueStyle::Figure => figure(p, Vec3::ZERO, h),
        StatueStyle::Angel => {
            figure(p, Vec3::ZERO, h);
            for s in [-1.0f32, 1.0] {
                let wing: Vec<Vec2> = vec![Vec2::new(0.0, h * 0.5), Vec2::new(s * h * 0.12, h * 0.85), Vec2::new(s * h * 0.42, h * 1.05), Vec2::new(s * h * 0.3, h * 0.7), Vec2::new(s * h * 0.1, h * 0.45)];
                if let Some(r) = poly::normalize(&wing, true) {
                    mesh::extrude_shape(p, &Shape::new(r), &Plane::new(Vec3::new(0.0, 0.0, -h * 0.1), Vec3::X, Vec3::Y), h * 0.03);
                }
            }
        }
        StatueStyle::Bust => {
            lathe_full(p, Vec3::ZERO, &[(0.0, 0.0), (h * 0.2, 0.0), (h * 0.2, h * 0.08), (h * 0.12, h * 0.15), (h * 0.3, h * 0.35), (h * 0.28, h * 0.5), (h * 0.1, h * 0.56), (h * 0.08, h * 0.62), (0.0, h * 0.62)], 14);
            mesh::ellipsoid(p, Vec3::Y * (h * 0.78), Vec3::new(h * 0.13, h * 0.17, h * 0.15), (0.0, std::f32::consts::TAU), (-1.5, 1.5), 14, 8);
        }
        StatueStyle::Seated => {
            p.aabox(Vec3::new(-h * 0.22, 0.0, -h * 0.2), Vec3::new(h * 0.22, h * 0.35, h * 0.2), 1.0);
            p.aabox(Vec3::new(-h * 0.16, h * 0.35, -h * 0.05), Vec3::new(h * 0.16, h * 0.42, h * 0.3), 1.0);
            figure(p, Vec3::new(0.0, h * 0.25, -h * 0.05), h * 0.75);
        }
        StatueStyle::Equestrian => {
            let body_y = h * 0.42;
            mesh::ellipsoid(p, Vec3::Y * body_y, Vec3::new(h * 0.13, h * 0.13, h * 0.32), (0.0, std::f32::consts::TAU), (-1.5, 1.5), 14, 8);
            for (x, z) in [(-0.08, 0.22), (0.08, 0.22), (-0.08, -0.22), (0.08, -0.22)] {
                p.cylinder(Vec3::new(x * h, body_y - h * 0.05, z * h), Vec3::new(x * h, 0.0, z * h * 1.05), h * 0.03, 8, true);
            }
            let neck0 = Vec3::new(0.0, body_y + h * 0.05, h * 0.25);
            let neck1 = Vec3::new(0.0, body_y + h * 0.3, h * 0.38);
            p.beam(neck0, neck1, Vec3::X, h * 0.08, h * 0.1, 1.0);
            mesh::ellipsoid(p, neck1 + Vec3::new(0.0, -h * 0.02, h * 0.07), Vec3::new(h * 0.05, h * 0.06, h * 0.12), (0.0, std::f32::consts::TAU), (-1.5, 1.5), 10, 6);
            figure(p, Vec3::new(0.0, body_y + h * 0.05, 0.0), h * 0.55);
        }
        StatueStyle::Group => {
            figure(p, Vec3::ZERO, h);
            figure(p, Vec3::new(-h * 0.3, 0.0, -h * 0.1), h * 0.9);
            figure(p, Vec3::new(h * 0.3, 0.0, -h * 0.08), h * 0.85);
        }
    }
}

pub fn statue(b: &mut Builder, s: &Statue, sc: &Scope, o: &mut Out) -> Result<()> {
    let m = facing_matrix(s.at, s.facing);
    let mut ped_h = 0.0;
    if let Some(ped) = s.pedestal.get() {
        let pm = b.mat(ped.material.as_deref(), sc.mat)?;
        let mut pp = Prim::default();
        let w = ped.width;
        pp.aabox(Vec3::new(-w * 0.5, 0.0, -w * 0.5), Vec3::new(w * 0.5, ped.height, w * 0.5), 1.0);
        pp.aabox(Vec3::new(-w * 0.58, ped.height - 0.1, -w * 0.58), Vec3::new(w * 0.58, ped.height, w * 0.58), 1.0);
        pp.aabox(Vec3::new(-w * 0.6, 0.0, -w * 0.6), Vec3::new(w * 0.6, 0.15, w * 0.6), 1.0);
        o.geo.get(pm).append(&pp, &m);
        ped_h = ped.height;
    }
    let mut p = Prim::default();
    statue_geo(&mut p, s.style, s.height);
    o.geo.get(sc.mat).append(&p, &(m * Mat4::from_translation(Vec3::Y * ped_h)));
    Ok(())
}

pub fn stairs(b: &mut Builder, s: &Stairs, sc: &Scope, o: &mut Out) -> Result<()> {
    if let Some(pg) = &s.polygon {
        let n = s.steps.unwrap_or(3).max(1);
        let rh = s.rise.map(|r| r / n as f32).unwrap_or(s.riser);
        for sh in b.poly(pg, sc)? {
            for k in 0..n {
                let grow = (n - k) as f32 * s.tread;
                let ring = poly::offset(&sh.outer, grow).unwrap_or(sh.outer.clone());
                mesh::prism(o.geo.get(sc.mat), &Shape::new(ring), s.y + k as f32 * rh, s.y + (k + 1) as f32 * rh, true, false);
            }
        }
        o.collide_all();
        return Ok(());
    }
    let (a, c) = match (s.from, s.to) {
        (Some(a), Some(c)) => (Vec2::new(a[0], a[1]), Vec2::new(c[0], c[1])),
        _ => return Err(Error::msg("stairs need `from` and `to` (or `polygon` for steps around a podium)")),
    };
    let run = (c - a).length();
    if run < 0.1 {
        return Err(Error::msg("stairs `from` and `to` are too close"));
    }
    let n = s.steps.unwrap_or_else(|| match s.rise {
        Some(r) => (r / s.riser).round().max(1.0) as u32,
        None => (run / s.tread).floor().max(1.0) as u32,
    }) as usize;
    let rh = s.rise.map(|r| r / n as f32).unwrap_or(s.riser);
    let tread = run / n as f32;
    let y = s.y;
    let mut pts = vec![Vec2::new(0.0, y), Vec2::new(run, y), Vec2::new(run, y + n as f32 * rh)];
    for k in (1..n).rev() {
        pts.push(Vec2::new(tread * k as f32, y + (k + 1) as f32 * rh));
        pts.push(Vec2::new(tread * k as f32, y + k as f32 * rh));
    }
    pts.push(Vec2::new(0.0, y + rh));
    let d = (c - a) / run;
    let u = Vec3::new(d.x, 0.0, d.y);
    let side = u.cross(Vec3::Y);
    if let Some(r) = poly::normalize(&pts, true) {
        let plane = Plane::new(Vec3::new(a.x, 0.0, a.y) - side * (s.width * 0.5), u, Vec3::Y);
        let plane = if plane.n.dot(side) < 0.0 { Plane { o: Vec3::new(a.x, 0.0, a.y) + side * (s.width * 0.5), ..plane } } else { plane };
        mesh::extrude_shape(o.geo.get(sc.mat), &Shape::new(r), &plane, s.width);
    }
    if s.cheeks {
        let top = y + n as f32 * rh;
        for sg in [-1.0f32, 1.0] {
            let off = side * (sg * (s.width * 0.5 + 0.15));
            let cheek = vec![Vec2::new(0.0, y), Vec2::new(run, y), Vec2::new(run, top + 0.9), Vec2::new(0.0, y + rh + 0.9)];
            if let Some(r) = poly::normalize(&cheek, true) {
                let plane = Plane::new(Vec3::new(a.x, 0.0, a.y) + off - side * 0.15, u, Vec3::Y);
                let plane = if plane.n.dot(side) < 0.0 { Plane { o: Vec3::new(a.x, 0.0, a.y) + off + side * 0.15, ..plane } } else { plane };
                mesh::extrude_shape(o.geo.get(sc.mat), &Shape::new(r), &plane, 0.3);
            }
        }
    }
    o.collide_all();
    Ok(())
}

pub fn arcade(b: &mut Builder, a: &Arcade, sc: &Scope, o: &mut Out) -> Result<()> {
    // Polylines in arcade order (u along them, the front on their left):
    // a straight `from`/`to` line, or chains of selected polygon edges split
    // into smooth runs, so an arcade round a curved square is one arcade
    // bent along the curve with its bays by arc length.
    let mut lines: Vec<(Vec<Vec2>, bool)> = Vec::new();
    let flip = a.face.as_deref() == Some("left");
    if let (Some(p0), Some(p1)) = (a.from, a.to) {
        lines.push((vec![Vec2::new(p1[0], p1[1]), Vec2::new(p0[0], p0[1])], false));
    } else if let Some(pg) = &a.polygon {
        for s in b.poly(pg, sc)? {
            let sel = resolve::select(&s, &a.edges);
            for c in resolve::chains(&s, &sel) {
                let mut pts = resolve::chain_points(&s, c);
                if c.3 {
                    // Closed chains start at a corner, so no run wraps.
                    let cor = super::runs::corners(&pts, true, super::facade::RUN_ANGLE);
                    if let Some(k) = cor.iter().position(|x| *x) {
                        pts.rotate_left(k);
                    }
                }
                let p2 = super::runs::Path2::new(&pts, c.3, super::facade::RUN_ANGLE);
                if p2.full_loop() {
                    lines.push((pts, true));
                    continue;
                }
                for &(r0, r1) in &p2.runs {
                    // Points of the run [r0, r1] along the chain.
                    let mut run = vec![p2.at(r0).0];
                    let m = pts.len();
                    for (k, sk) in p2.s.iter().enumerate().take(if c.3 { m } else { m }) {
                        if *sk > r0 + 1e-4 && *sk < r1 - 1e-4 {
                            run.push(pts[k % m]);
                        }
                    }
                    run.push(p2.at(r1 - 1e-5).0);
                    lines.push((run, false));
                }
            }
        }
    } else {
        return Err(Error::msg("arcade needs `from` and `to`, or `polygon` (+ `edges`)"));
    }
    let mut built = Out::default();
    for (mut pts, closed) in lines {
        if flip {
            pts.reverse();
        }
        let Some(bend) = super::runs::Bend::new(&pts, closed, 20.0) else { continue };
        let mut flat = Out::default();
        arcade_flat(b, a, bend.len(), closed, sc, &mut flat)?;
        bend.warp_geo(&flat.geo, &mut built.geo);
    }
    built.collide_all();
    o.append(&built, &Mat4::IDENTITY);
    Ok(())
}

/// One straight arcade of length `len` in the flat frame (x along it from
/// its left end seen from the front, z out of the front).
fn arcade_flat(b: &mut Builder, a: &Arcade, len: f32, closed: bool, sc: &Scope, o: &mut Out) -> Result<()> {
    let runs = vec![(Vec2::new(len, 0.0), Vec2::ZERO)];
    let wall_m = b.mat(a.wall_material.as_deref(), sc.mat)?;
    let col = a.column.get().cloned();
    let pier = a.piers.get().cloned();
    let use_cols = col.is_some() || pier.is_none();
    let col = col.unwrap_or(ColumnSpec { radius: 0.3, order: ColumnOrder::Tuscan, flutes: 0, material: None, double: false });
    let col_m = b.mat(col.material.as_deref(), sc.mat)?;
    for (p0, p1) in runs {
        let len = (p1 - p0).length();
        if len < 0.5 {
            continue;
        }
        let support = if use_cols { col.radius * 2.7 } else { pier.as_ref().unwrap().width };
        let bays = a.bays.get().cloned().unwrap_or_default();
        let count = bays.count.map(|c| c as usize).unwrap_or_else(|| {
            let sp = bays.spacing.unwrap_or((a.impost * 0.8).max(2.4));
            ((len / sp).round() as usize).max(1)
        });
        let spacing = len / count as f32;
        let span = spacing - support;
        if span <= 0.2 {
            return Err(Error::msg(format!("arcade bays are narrower than their supports (spacing {spacing:.2} m)")));
        }
        let rise = a.rise.unwrap_or(profiles::default_rise(a.arch, span));
        let top = a.height.unwrap_or(a.y + a.impost + rise + 0.9);
        let depth = a.depth;
        // Facade frame: origin at p1 so u runs from p1 (left seen from the
        // front) to p0; the front faces right of p0 -> p1.
        let d = (p1 - p0) / len;
        let front_n = Vec3::new(d.y, 0.0, -d.x);
        let half = front_n * (depth * 0.5);
        let u = Vec3::new(-d.x, 0.0, -d.y);
        let wall_y0 = if use_cols { a.y + a.impost } else { a.y };
        let f = Facade { plane: Plane::new(Vec3::new(p1.x, 0.0, p1.y) + half, u, Vec3::Y), len, v0: wall_y0, v1: top, uv_off: 0.0, mat: wall_m, outline: None, wrap: None, top: None };
        let mut cuts = Vec::new();
        let n_arc = b.segs(span * 0.5, 8, 24);
        for k in 0..count {
            let uc = spacing * (k as f32 + 0.5);
            let (bottom, h) = if use_cols { (a.y + a.impost - 1.0, 1.0 + rise) } else { (a.y, a.impost + rise) };
            let ol = profiles::outline(a.arch, span, h, Some(rise), n_arc);
            let ring: Vec<Vec2> = ol.pts.iter().map(|q| *q + Vec2::new(uc, bottom)).collect();
            cuts.push(Cut::simple(ring, depth, wall_m, None, false));
        }
        for w in facade::build(&mut o.geo, &f, &cuts) {
            b.warn(sc, w);
        }
        // Back face with the same holes.
        let bf = Facade { plane: Plane::new(Vec3::new(p0.x, 0.0, p0.y) - half, -u, Vec3::Y), ..f.clone() };
        let mirrored: Vec<Vec<Vec2>> = cuts.iter().map(|c| c.outer().iter().rev().map(|q| Vec2::new(len - q.x, q.y)).collect()).collect();
        let (shape, _) = facade::wall_shape(&bf, &mirrored);
        mesh::planar_shape(o.geo.get(wall_m), &shape, &bf.plane, false, Vec2::ZERO);
        // Top, ends and underside of the spandrel wall.
        let corners = [Vec3::new(p0.x, 0.0, p0.y) + half, Vec3::new(p1.x, 0.0, p1.y) + half, Vec3::new(p1.x, 0.0, p1.y) - half, Vec3::new(p0.x, 0.0, p0.y) - half];
        let at = |q: Vec3, y: f32| Vec3::new(q.x, y, q.z);
        mesh::quad_world(o.geo.get(wall_m), [at(corners[0], top), at(corners[1], top), at(corners[2], top), at(corners[3], top)], Vec3::Y);
        mesh::quad_world(o.geo.get(wall_m), [at(corners[0], wall_y0), at(corners[3], wall_y0), at(corners[3], top), at(corners[0], top)], -Vec3::new(d.x, 0.0, d.y));
        mesh::quad_world(o.geo.get(wall_m), [at(corners[1], wall_y0), at(corners[2], wall_y0), at(corners[2], top), at(corners[1], top)], Vec3::new(d.x, 0.0, d.y));
        // Supports.
        let cap_h = 0.25f32;
        for k in 0..=count {
            if closed && k == count {
                continue;
            }
            let at = if closed { spacing * k as f32 } else { (spacing * k as f32).clamp(support * 0.5, len - support * 0.5) };
            let pos = p0 + d * at;
            let base = Vec3::new(pos.x, a.y, pos.y);
            if use_cols {
                let mut p = Prim::default();
                column_geo(b, &mut p, a.impost - cap_h, col.radius, col.order, None, col.flutes, true, true, None);
                let offs: Vec<f32> = if col.double { vec![-depth * 0.25, depth * 0.25] } else { vec![0.0] };
                for off in offs {
                    o.geo.get(col_m).append(&p, &Mat4::from_translation(base + front_n * off));
                }
                // Impost block carrying the arches.
                let ctr = base + Vec3::Y * (a.impost - cap_h * 0.5);
                o.geo.get(wall_m).obox(ctr, [Vec3::new(d.x, 0.0, d.y), Vec3::Y, front_n], Vec3::new(support * 0.5, cap_h * 0.5, depth * 0.5 + 0.04), 1.0);
            } else {
                let pw = pier.as_ref().unwrap();
                let pd = pw.depth.unwrap_or(depth);
                let pm = b.mat(pw.material.as_deref(), wall_m)?;
                let ctr = base + Vec3::Y * ((a.impost) * 0.5);
                let _ = pd;
                o.geo.get(pm).obox(ctr + Vec3::Y * 0.0, [Vec3::new(d.x, 0.0, d.y), Vec3::Y, front_n], Vec3::new(0.001, 0.001, 0.001), 1.0);
                // Impost moulding on the piers.
                o.geo.get(pm).obox(base + Vec3::Y * (a.impost - 0.1), [Vec3::new(d.x, 0.0, d.y), Vec3::Y, front_n], Vec3::new(support * 0.5 + 0.06, 0.1, depth * 0.5 + 0.06), 1.0);
            }
        }
        if let Some(h) = a.hood.get() {
            let hm = b.mat(h.material.as_deref(), wall_m)?;
            for c in &cuts {
                let pts = c.outer();
                let head: Vec<Vec2> = pts.iter().copied().filter(|q| q.y >= a.y + a.impost - 1e-3).collect();
                if head.len() < 3 {
                    continue;
                }
                let ctr = Vec2::new(head.iter().map(|q| q.x).sum::<f32>() / head.len() as f32, a.y + a.impost);
                let grown: Vec<Vec2> = head.iter().map(|q| *q + (*q - ctr).normalize_or(Vec2::Y) * h.gap).collect();
                let offs = mesh::polyline_offsets_2d(&grown, false);
                let p3: Vec<Vec3> = grown.iter().map(|q| f.plane.at(*q)).collect();
                let side: Vec<Vec3> = offs.iter().map(|q| f.plane.u * q.x + f.plane.v * q.y).collect();
                let up = vec![f.plane.n; grown.len()];
                let s = h.size;
                mesh::sweep(o.geo.get(hm), &p3, &side, &up, &[Vec2::new(0.0, 0.0), Vec2::new(s, 0.0), Vec2::new(s, s * 0.45), Vec2::new(s * 0.4, s), Vec2::new(0.0, s)], false, true, false, true);
            }
        }
        if let Some(cn) = a.cornice.get() {
            let size = cn.size.unwrap_or(0.45);
            let prof = cn.profile.clone().unwrap_or(ProfileSpec::Named("cornice".into()));
            let pts = super::details::profile_points(&prof, size, cn.projection);
            let line = vec![p0 + Vec2::new(front_n.x, front_n.z) * (depth * 0.5), p1 + Vec2::new(front_n.x, front_n.z) * (depth * 0.5)];
            super::details::moulding_path(o, &line, false, top - size, &pts, wall_m);
        }
        if let Some(g) = a.gallery {
            // Back wall, ceiling and floor of a loggia.
            let back = -front_n * (depth * 0.5 + g);
            let bw = 0.5f32;
            let q0 = Vec3::new(p0.x, 0.0, p0.y) + back;
            let q1 = Vec3::new(p1.x, 0.0, p1.y) + back;
            let ctr = (q0 + q1) * 0.5 - front_n * (bw * 0.5);
            o.geo.get(wall_m).obox(ctr + Vec3::Y * ((top + a.y) * 0.5), [Vec3::new(d.x, 0.0, d.y), Vec3::Y, front_n], Vec3::new(len * 0.5, (top - a.y) * 0.5, bw * 0.5), 1.0);
            let mid = (Vec3::new(p0.x, 0.0, p0.y) + Vec3::new(p1.x, 0.0, p1.y)) * 0.5 - front_n * (depth * 0.5 + g * 0.5);
            o.geo.get(wall_m).obox(mid + Vec3::Y * (top - 0.3), [Vec3::new(d.x, 0.0, d.y), Vec3::Y, front_n], Vec3::new(len * 0.5, 0.3, g * 0.5 + 0.01), 1.0);
            for sg in [0.0f32, 1.0] {
                let e = p0.lerp(p1, sg);
                let c3 = Vec3::new(e.x, 0.0, e.y) - front_n * (depth * 0.5 + g * 0.5);
                o.geo.get(wall_m).obox(c3 + Vec3::Y * ((top + a.y) * 0.5), [Vec3::new(d.x, 0.0, d.y), Vec3::Y, front_n], Vec3::new(0.25, (top - a.y) * 0.5, g * 0.5), 1.0);
            }
        }
    }
    o.collide_all();
    Ok(())
}

/// Outline of a relief panel of `w` x `h` with its bottom centre at the
/// origin (x along the wall, y up).
pub fn relief_outline(shape: ReliefShape, w: f32, h: f32, seg: usize) -> Vec<Vec2> {
    let (hw, n) = (w * 0.5, seg.max(8));
    let arc = |c: Vec2, r: Vec2, a0: f32, a1: f32, k: usize| -> Vec<Vec2> { (0..=k).map(|i| c + Vec2::from_angle(a0 + (a1 - a0) * i as f32 / k as f32) * r).collect() };
    use std::f32::consts::{FRAC_PI_2, PI, TAU};
    match shape {
        ReliefShape::Rect => vec![Vec2::new(-hw, 0.0), Vec2::new(hw, 0.0), Vec2::new(hw, h), Vec2::new(-hw, h)],
        ReliefShape::Arched => {
            // Semicircular top (segmental when the panel is low).
            let rise = hw.min(h * 0.6);
            let spring = h - rise;
            let r = (hw * hw + rise * rise) / (2.0 * rise.max(1e-3));
            let c = Vec2::new(0.0, h - r);
            let a0 = (spring - c.y).atan2(hw);
            let mut v = vec![Vec2::new(-hw, 0.0), Vec2::new(hw, 0.0)];
            v.extend(arc(c, Vec2::splat(r), a0, PI - a0, n));
            v
        }
        ReliefShape::Circle => {
            let r = hw.min(h * 0.5);
            arc(Vec2::new(0.0, h * 0.5), Vec2::splat(r), 0.0, TAU * (1.0 - 1.0 / n as f32), n - 1)
        }
        ReliefShape::Oval => arc(Vec2::new(0.0, h * 0.5), Vec2::new(hw, h * 0.5), 0.0, TAU * (1.0 - 1.0 / n as f32), n - 1),
        ReliefShape::Shield => {
            // Flat top, straight sides down to 45 %, curving to a point.
            let mut v = vec![Vec2::new(-hw, h), Vec2::new(-hw, h * 0.45)];
            let k = n / 2;
            for i in 1..k {
                let t = i as f32 / k as f32;
                let a = t * FRAC_PI_2;
                v.push(Vec2::new(-hw * a.cos(), h * 0.45 * (1.0 - a.sin())));
            }
            v.push(Vec2::new(0.0, 0.0));
            for i in (1..k).rev() {
                let t = i as f32 / k as f32;
                let a = t * FRAC_PI_2;
                v.push(Vec2::new(hw * a.cos(), h * 0.45 * (1.0 - a.sin())));
            }
            v.push(Vec2::new(hw, h * 0.45));
            v.push(Vec2::new(hw, h));
            v.reverse();
            v
        }
        ReliefShape::Cartouche => {
            // Rectangle with concave quarter-round corners.
            let r = (w.min(h) * 0.18).max(0.01);
            let k = (n / 4).max(3);
            let mut v = Vec::new();
            for (c, a0) in [(Vec2::new(-hw, 0.0), 0.0f32), (Vec2::new(hw, 0.0), FRAC_PI_2), (Vec2::new(hw, h), PI), (Vec2::new(-hw, h), 1.5 * PI)] {
                // Corner arc centred on the corner, bulging inward.
                let pts = arc(c, Vec2::splat(r), a0 + FRAC_PI_2, a0, k);
                v.extend(pts);
            }
            v
        }
    }
}

/// Relief panel / plaque on a wall.
pub fn relief(b: &mut Builder, r: &Relief, sc: &Scope, o: &mut Out) -> Result<()> {
    let [w, h] = r.size;
    if w <= 0.0 || h <= 0.0 {
        return Err(Error::msg("relief `size` must be positive [width, height]"));
    }
    if r.depth <= 0.0 {
        return Err(Error::msg("relief `depth` must be positive (its projection from the wall)"));
    }
    let m = facing_matrix(r.at, r.facing);
    let seg = b.segs(w.max(h) * 0.5, 16, 64);
    let outline = poly::normalize(&relief_outline(r.shape, w, h, seg), true).ok_or_else(|| Error::msg("relief outline is degenerate"))?;
    let field_m = b.mat(r.field_material.as_deref(), sc.mat)?;
    let plane = |z: f32| Plane::new(Vec3::new(0.0, 0.0, z), Vec3::X, Vec3::Y);
    // The field (slightly into the wall so no gap shows).
    let mut field = Prim::default();
    mesh::extrude_shape(&mut field, &Shape::new(outline.clone()), &plane(-0.01), r.depth + 0.01);
    let mut frame = Prim::default();
    if r.frame > 0.0
        && let Some(outer) = poly::offset(&outline, r.frame)
        && let Some(ring) = (Shape { outer, holes: vec![outline.iter().rev().copied().collect()] }).normalized()
    {
        mesh::extrude_shape(&mut frame, &ring, &plane(-0.01), r.depth + r.frame_depth.max(0.0) + 0.01);
    }
    let z = r.depth;
    let lift = (w.min(h) * 0.06).clamp(0.01, 0.12);
    match r.relief.as_deref().unwrap_or("none") {
        "boss" => {
            // A low half-ellipsoid; its axis (y) turned into the wall normal (z).
            let c = poly::centroid(&outline);
            let mut boss = Prim::default();
            mesh::ellipsoid(&mut boss, Vec3::ZERO, Vec3::new(w * 0.28, lift * 1.5, h * 0.28), (0.0, std::f32::consts::TAU), (0.0, std::f32::consts::FRAC_PI_2), 24, 8);
            field.append(&boss, &(Mat4::from_translation(Vec3::new(c.x, c.y, z - 0.005)) * Mat4::from_rotation_x(std::f32::consts::FRAC_PI_2)));
        }
        "panel" => {
            if let Some(inner) = poly::offset(&outline, -(w.min(h) * 0.15)) {
                mesh::extrude_shape(&mut field, &Shape::new(inner), &plane(z - 0.005), lift);
            }
        }
        "figure" => {
            let body: Vec<Vec2> = (0..24).map(|k| Vec2::new(0.0, h * 0.42) + Vec2::from_angle(k as f32 / 24.0 * std::f32::consts::TAU) * Vec2::new(w * 0.16, h * 0.3)).collect();
            let head: Vec<Vec2> = (0..16).map(|k| Vec2::new(0.0, h * 0.8) + Vec2::from_angle(k as f32 / 16.0 * std::f32::consts::TAU) * Vec2::splat(h * 0.08)).collect();
            for s in [body, head] {
                if let Some(rg) = poly::normalize(&s, true) {
                    mesh::extrude_shape(&mut field, &Shape::new(rg), &plane(z - 0.005), lift);
                }
            }
        }
        _ => {}
    }
    o.geo.get(field_m).append(&field, &m);
    o.geo.get(sc.mat).append(&frame, &m);
    Ok(())
}
