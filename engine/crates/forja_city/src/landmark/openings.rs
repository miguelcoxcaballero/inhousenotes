//! Openings on facades: layout in levels × bays, cuts (with orders and
//! splays), and their architecture: frames, mullions, tracery, rose
//! windows, doors, shutters, grilles, surrounds, hoods, sills, keystones,
//! voussoirs, balconies and bells.

use forja_math::{Vec2, Vec3};

use super::build::{Builder, Scope};
use super::facade::{self, Cut, Facade};
use super::mesh::{self, Geo, MatId, Plane};
use super::parse::{Error, Result};
use super::poly;
use super::profiles::{self, Outline};
use super::spec::*;

/// Opening settings after per-kind defaults.
#[derive(Clone, Debug)]
pub struct Eff {
    pub kind: OpeningKind,
    pub arch: ArchKind,
    pub infill: Infill,
    pub w: f32,
    pub h: f32,
    pub rise: Option<f32>,
    pub depth: f32,
    pub splay: f32,
    pub orders: u32,
    pub tympanum: bool,
    pub spokes: u32,
    pub rings: u32,
    pub mullions: u32,
}

pub fn effective(op: &Openings) -> Eff {
    let k = op.kind;
    let (arch, infill) = match k {
        OpeningKind::Window => (ArchKind::Flat, Infill::Glass),
        OpeningKind::Door => (ArchKind::Flat, Infill::Door),
        OpeningKind::Portal => (ArchKind::Round, Infill::Door),
        OpeningKind::Arch => (ArchKind::Round, Infill::Dark),
        OpeningKind::Oculus => (ArchKind::Circle, Infill::Glass),
        OpeningKind::Rose => (ArchKind::Circle, Infill::Stained),
        OpeningKind::Lancet => (ArchKind::Lancet, Infill::Glass),
        OpeningKind::Twin => (ArchKind::Round, Infill::Glass),
        OpeningKind::Niche => (ArchKind::Round, Infill::Blind),
        OpeningKind::Blind => (ArchKind::Round, Infill::Blind),
        OpeningKind::Slit => (ArchKind::Flat, Infill::Dark),
    };
    let arch = op.arch.unwrap_or(arch);
    let infill = op.infill.unwrap_or(infill);
    let depth_default = match k {
        OpeningKind::Portal => 0.9,
        OpeningKind::Blind => 0.08,
        OpeningKind::Slit => 0.7,
        OpeningKind::Arch => 1.2,
        _ => 0.35,
    };
    // `depth` keeps its serde default (0.35) unless written; kinds with a
    // different natural depth use theirs when the default was left.
    let depth = if (op.depth - 0.35).abs() < 1e-6 { depth_default } else { op.depth };
    let (w, h) = if matches!(arch, ArchKind::Circle | ArchKind::Quatrefoil) { (op.width, op.width) } else { (op.width, op.height) };
    Eff {
        kind: k,
        arch,
        infill,
        w,
        h,
        rise: op.rise,
        depth: if infill == Infill::Void || infill == Infill::Dark { depth.max(0.6) } else { depth },
        splay: if k == OpeningKind::Slit && op.splay == 0.0 { 0.25 } else { op.splay },
        orders: if k == OpeningKind::Portal && op.orders == 0 { 3 } else { op.orders },
        tympanum: op.tympanum || (k == OpeningKind::Portal && arch != ArchKind::Flat),
        spokes: if k == OpeningKind::Rose && op.spokes == 0 { 12 } else { op.spokes },
        rings: if k == OpeningKind::Rose && op.rings == 0 { 1 } else { op.rings },
        mullions: op.mullions,
    }
}

/// Bottom heights of the rows.
fn levels(op: &Openings, e: &Eff, f: &Facade) -> Vec<f32> {
    match &op.levels {
        Some(l) => l.0.clone(),
        None => match e.kind {
            OpeningKind::Door | OpeningKind::Portal | OpeningKind::Arch => vec![f.v0],
            OpeningKind::Rose | OpeningKind::Oculus => vec![(f.v0 + (f.v1 - f.v0) * 0.62 - e.w * 0.5).max(f.v0 + 0.5)],
            _ => vec![f.v0 + 1.0],
        },
    }
}

/// Bay centres along a facade of length `len` (from its left end).
pub fn bay_centres(bays: Option<&Bays>, len: f32, w: f32, kind: OpeningKind) -> Vec<f32> {
    let default_one = matches!(kind, OpeningKind::Door | OpeningKind::Portal | OpeningKind::Rose | OpeningKind::Oculus);
    let b = bays.cloned().unwrap_or_default();
    let (ml, mr) = match (b.margins, b.margin) {
        (Some(m), _) => (m[0], m[1]),
        (None, Some(m)) => (m, m),
        (None, None) => {
            let m = (w * 0.8).max(0.6);
            (m, m)
        }
    };
    let mut v: Vec<f32> = if let Some(at) = &b.at {
        at.clone()
    } else if let Some(off) = &b.offsets {
        off.iter().map(|o| len * 0.5 + o).collect()
    } else {
        let usable = len - ml - mr;
        match (b.count, b.spacing) {
            (Some(c), Some(sp)) => {
                let c = c as usize;
                let start = ml + usable * 0.5 - sp * (c as f32 - 1.0) * 0.5;
                (0..c).map(|k| start + sp * k as f32).collect()
            }
            (Some(c), None) => {
                let c = c as usize;
                (0..c).map(|k| ml + usable * (k as f32 + 0.5) / c as f32).collect()
            }
            (None, sp) => {
                if default_one && sp.is_none() {
                    vec![len * 0.5]
                } else {
                    let sp = sp.unwrap_or((w * 2.4).max(w + 1.4));
                    if usable < w {
                        Vec::new()
                    } else {
                        let c = ((usable - w) / sp).floor() as usize + 1;
                        let start = ml + usable * 0.5 - sp * (c as f32 - 1.0) * 0.5;
                        (0..c).map(|k| start + sp * k as f32).collect()
                    }
                }
            }
        }
    };
    let n = v.len() as i64;
    let skip: Vec<usize> = b.skip.iter().map(|s| s.rem_euclid(n.max(1)) as usize).collect();
    let mut k = 0;
    v.retain(|_| {
        let keep = !skip.contains(&k);
        k += 1;
        keep
    });
    v
}

fn translate(o: &Outline, du: f32, dv: f32) -> Vec<Vec2> {
    o.pts.iter().map(|p| *p + Vec2::new(du, dv)).collect()
}

/// Highest point of a ring at a given u (for mullions under arches).
fn top_at(ring: &[Vec2], u: f32) -> Option<f32> {
    let n = ring.len();
    let mut best: Option<f32> = None;
    for i in 0..n {
        let (a, b) = (ring[i], ring[(i + 1) % n]);
        if (a.x - u) * (b.x - u) <= 0.0 && (a.x - b.x).abs() > 1e-6 {
            let t = (u - a.x) / (b.x - a.x);
            let y = a.y + (b.y - a.y) * t;
            best = Some(best.map_or(y, |v: f32| v.max(y)));
        }
    }
    best
}

/// Horizontal extent of a ring at height v.
fn span_at(ring: &[Vec2], v: f32) -> Option<(f32, f32)> {
    let n = ring.len();
    let mut lo = f32::MAX;
    let mut hi = f32::MIN;
    for i in 0..n {
        let (a, b) = (ring[i], ring[(i + 1) % n]);
        if (a.y - v) * (b.y - v) <= 0.0 && (a.y - b.y).abs() > 1e-6 {
            let t = (v - a.y) / (b.y - a.y);
            let x = a.x + (b.x - a.x) * t;
            lo = lo.min(x);
            hi = hi.max(x);
        }
    }
    (hi > lo).then_some((lo, hi))
}

/// Box in facade coordinates: u0..u1, v0..v1, out0..out1 (out positive
/// = in front of the wall).
pub fn fbox(p: &mut crate::geom::Prim, f: &Facade, u: (f32, f32), v: (f32, f32), out: (f32, f32)) {
    let c = Vec2::new((u.0 + u.1) * 0.5, (v.0 + v.1) * 0.5);
    let center = f.plane.at_d(c, (out.0 + out.1) * 0.5);
    let half = Vec3::new((u.1 - u.0).abs() * 0.5, (v.1 - v.0).abs() * 0.5, (out.1 - out.0).abs() * 0.5);
    if half.min_element() < 1e-5 {
        return;
    }
    p.obox(center, [f.plane.u, f.plane.v, f.plane.n], half, 1.0);
}

/// Bar between two facade points (u, v) at depth range `out`.
fn fbar(p: &mut crate::geom::Prim, f: &Facade, a: Vec2, b: Vec2, width: f32, out: (f32, f32)) {
    let d = b - a;
    let len = d.length();
    if len < 1e-4 {
        return;
    }
    let mid = (a + b) * 0.5;
    let dir = d / len;
    let x = f.plane.u * dir.x + f.plane.v * dir.y;
    let y = f.plane.u * -dir.y + f.plane.v * dir.x;
    let center = f.plane.at_d(mid, (out.0 + out.1) * 0.5);
    p.obox(center, [x, y, f.plane.n], Vec3::new(len * 0.5 + width * 0.5, width * 0.5, (out.1 - out.0).abs() * 0.5), 1.0);
}

/// Bar following a curve in the facade plane (square-ish section).
fn fcurve(p: &mut crate::geom::Prim, f: &Facade, pts: &[Vec2], width: f32, out: (f32, f32), closed: bool) {
    if pts.len() < 2 {
        return;
    }
    let offs = mesh::polyline_offsets_2d(pts, closed);
    let path: Vec<Vec3> = pts.iter().map(|q| f.plane.at_d(*q, out.0)).collect();
    let side: Vec<Vec3> = offs.iter().map(|o| f.plane.u * o.x + f.plane.v * o.y).collect();
    let up = vec![f.plane.n; pts.len()];
    let hw = width * 0.5;
    let d = out.1 - out.0;
    let prof = [Vec2::new(-hw, 0.0), Vec2::new(hw, 0.0), Vec2::new(hw, d), Vec2::new(-hw, d)];
    mesh::sweep(p, &path, &side, &up, &prof, closed, true, true, !closed);
}

/// Flat band between two corresponding rings at `out`, plus its outer and
/// inner edges back to the wall when `sides`.
fn band(p: &mut crate::geom::Prim, f: &Facade, inner: &[Vec2], outer: &[Vec2], out: f32, range: Option<(usize, usize)>, sides: bool, base: f32) {
    let n = inner.len().min(outer.len());
    let (s, e) = range.unwrap_or((0, n));
    for i in s..e {
        let j = (i + 1) % n;
        if range.is_some() && i + 1 >= e && range.unwrap().1 < n {
            break;
        }
        let q = [f.plane.at_d(inner[i], out), f.plane.at_d(inner[j], out), f.plane.at_d(outer[j], out), f.plane.at_d(outer[i], out)];
        mesh::quad_world(p, q, f.plane.n);
        if sides {
            let o = [f.plane.at_d(outer[i], base), f.plane.at_d(outer[j], base), f.plane.at_d(outer[j], out), f.plane.at_d(outer[i], out)];
            let d = outer[j] - outer[i];
            let w2 = Vec2::new(d.y, -d.x);
            mesh::quad_world(p, o, f.plane.u * w2.x + f.plane.v * w2.y);
            let ii = [f.plane.at_d(inner[i], base), f.plane.at_d(inner[j], base), f.plane.at_d(inner[j], out), f.plane.at_d(inner[i], out)];
            let d = inner[j] - inner[i];
            let w2 = Vec2::new(-d.y, d.x);
            mesh::quad_world(p, ii, f.plane.u * w2.x + f.plane.v * w2.y);
        }
    }
}

/// Context for building the openings of one facade.
pub struct OpCtx {
    pub wall: MatId,
    pub trim: MatId,
    /// Wall thickness for through openings (free-standing walls).
    pub through: Option<f32>,
    /// Depth available for the fake room behind glazed openings.
    pub room: f32,
}

/// Cuts and details of one openings block on facade `f`.
pub fn place(b: &mut Builder, op: &Openings, f: &Facade, sc: &Scope, cx: &OpCtx, cuts: &mut Vec<Cut>, geo: &mut Geo) -> Result<()> {
    let e = effective(op);
    if e.w <= 0.05 || e.h <= 0.05 {
        return Err(Error::msg("opening width and height must be positive"));
    }
    let lv = levels(op, &e, f);
    let centres = bay_centres(op.bays.get(), f.len, e.w, e.kind);
    let n_arc = b.segs(e.w * 0.5, 6, 24);
    let stone = b.mat(op.stone_material.as_deref(), cx.wall)?;
    for (li, &vb) in lv.iter().enumerate() {
        for (bi, &uc) in centres.iter().enumerate() {
            if op.skip.iter().any(|s| s[0] == li as i64 && s[1] == bi as i64) {
                continue;
            }
            if vb + e.h <= f.v0 + 0.01 || vb >= f.v1 - 0.01 {
                continue;
            }
            one(b, op, &e, f, sc, cx, stone, uc, vb, n_arc, cuts, geo)?;
        }
        // Continuous balcony along the level.
        if let Some(bal) = op.balcony.get()
            && bal.continuous
            && !centres.is_empty()
        {
            let u0 = centres[0] - e.w * 0.5 - bal.extend;
            let u1 = centres[centres.len() - 1] + e.w * 0.5 + bal.extend;
            balcony(b, bal, f, cx, (u0.max(0.05), u1.min(f.len - 0.05)), vb, geo)?;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn one(b: &mut Builder, op: &Openings, e: &Eff, f: &Facade, sc: &Scope, cx: &OpCtx, stone: MatId, uc: f32, vb: f32, n_arc: usize, cuts: &mut Vec<Cut>, geo: &mut Geo) -> Result<()> {
    let base = profiles::outline(e.arch, e.w, e.h, e.rise, n_arc);
    let o = translate(&base, uc, vb);
    let spring = vb + base.spring;
    let depth = cx.through.unwrap_or(e.depth);
    let through = cx.through.is_some();
    let back = |b: &mut Builder| -> Result<Option<MatId>> {
        if through {
            return Ok(None);
        }
        Ok(Some(match e.infill {
            Infill::Glass | Infill::Grille | Infill::Tracery => b.mat_or(None, "interior")?,
            Infill::Stained => b.mat_or(op.glass_material.as_deref(), "stained_glass")?,
            Infill::Door => b.door_mat(op.door_material.as_deref())?,
            Infill::Shutters | Infill::Louvres => b.frame_mat(op.frame_material.as_deref())?,
            Infill::Void | Infill::Dark => b.mat_or(None, "dark")?,
            Infill::Blind => cx.wall,
            Infill::CurtainWall => b.mat_or(op.glass_material.as_deref(), "curtain_wall")?,
        }))
    };
    let back_mat = back(b)?;
    let glazed = matches!(e.infill, Infill::Glass | Infill::Grille | Infill::Tracery);
    // Twin lights: two openings sharing a colonnette.
    if e.kind == OpeningKind::Twin {
        let col_w = (e.w * 0.12).clamp(0.12, 0.3);
        let lw = (e.w - col_w) * 0.5;
        for s in [-1.0f32, 1.0] {
            let lo = profiles::outline(e.arch, lw, e.h, e.rise.map(|r| r * lw / e.w), n_arc);
            let ring = translate(&lo, uc + s * (col_w + lw) * 0.5, vb);
            let mut c = Cut::simple(ring.clone(), depth, stone, if glazed { None } else { back_mat }, false);
            if glazed {
                c.back = None;
            }
            cuts.push(c);
            if glazed {
                glass_window(b, op, e, f, &ring, uc + s * (col_w + lw) * 0.5, vb, lw, lo.spring, depth, cx.room, geo)?;
            }
        }
        // Colonnette with capital.
        let col_h = base.spring.max(0.5);
        let ctr = f.plane.at_d(Vec2::new(uc, vb), -depth * 0.45);
        let up = f.plane.v;
        let r = col_w * 0.32;
        let p = geo.get(stone);
        p.cylinder(ctr, ctr + up * (col_h - 0.12), r, 12, true);
        fbox(p, f, (uc - col_w * 0.6, uc + col_w * 0.6), (vb + col_h - 0.14, vb + col_h), (-depth * 0.75, -depth * 0.15));
        fbox(p, f, (uc - col_w * 0.5, uc + col_w * 0.5), (vb, vb + 0.08), (-depth * 0.7, -depth * 0.2));
        // A blind spandrel between the two heads and the outer arch.
        trims(b, op, e, f, cx, &base, &o, uc, vb, spring, geo)?;
        return Ok(());
    }
    // Rings of the cut: orders, splay or plain.
    let mut rings: Vec<(Vec<Vec2>, f32)> = Vec::new();
    if e.orders > 0 {
        let s = op.order_step.max(0.05);
        let n = e.orders as f32;
        let dd = s.min(depth / (n + 1.0)).max(0.05);
        for k in 0..=e.orders {
            let extra = (n - k as f32) * s;
            let rk = e.rise.map(|r| r + extra).or_else(|| Some(profiles::default_rise(e.arch, e.w + 2.0 * extra)));
            let ok = translate(&profiles::outline(e.arch, e.w + 2.0 * extra, e.h + extra, rk, n_arc), uc, vb);
            let z0 = k as f32 * dd;
            let z1 = if k == e.orders { depth.max(z0 + 0.05) } else { (k + 1) as f32 * dd };
            rings.push((ok.clone(), z0));
            rings.push((ok, z1));
        }
    } else if e.splay > 0.0 {
        rings.push((facade::grow(&o, uc, vb, e.w, e.h, 2.0 * e.splay, e.splay), 0.0));
        rings.push((o.clone(), depth));
    } else {
        rings.push((o.clone(), 0.0));
        rings.push((o.clone(), depth));
    }
    let cut = Cut { rings, reveal_mat: stone, back: if glazed { None } else { back_mat }, round: base.round };
    cuts.push(cut.clone());
    // Colonnettes and archivolt rolls on portals.
    if e.orders > 0 && op.colonnettes {
        let s = op.order_step.max(0.05);
        let p = geo.get(stone);
        for k in 0..e.orders as usize {
            let (ring, z) = (&cut.rings[2 * k + 1].0, cut.rings[2 * k + 1].1);
            let r = (s * 0.42).min(0.16);
            let wk = (ring.iter().map(|q| q.x).fold(f32::MIN, f32::max) - uc).abs();
            for sgn in [-1.0f32, 1.0] {
                let u = uc + sgn * (wk - s * 0.5);
                let a = f.plane.at_d(Vec2::new(u, vb), -z + r * 0.9);
                p.cylinder(a, a + f.plane.v * (base.spring - 0.2).max(0.3), r, 10, true);
                fbox(p, f, (u - r * 1.4, u + r * 1.4), (spring - 0.22, spring), (-z, -z + r * 2.0));
            }
            // Roll moulding along the order's arch.
            let head: Vec<Vec2> = ring[base.head.0..base.head.1.min(ring.len())].iter().map(|q| *q + (Vec2::new(uc, spring) - *q).normalize_or(Vec2::ZERO) * (s * 0.5)).collect();
            fcurve(p, f, &head, r * 1.8, (-z, -z + r * 1.6), false);
        }
    }
    match e.infill {
        Infill::Glass | Infill::Grille | Infill::Tracery => {
            glass_window(b, op, e, f, &o, uc, vb, e.w, base.spring, depth, cx.room, geo)?;
            if e.infill == Infill::Tracery || e.mullions > 0 || op.tracery.is_some() {
                tracery(b, op, e, f, stone, &o, &base, uc, vb, depth, geo)?;
            }
            if e.infill == Infill::Grille {
                grille(b, op, f, &o, uc, vb, e.w, geo)?;
            }
        }
        Infill::Stained => {
            if base.round {
                rose(b, e, f, stone, &o, uc, vb, depth, geo);
            } else {
                tracery(b, op, e, f, stone, &o, &base, uc, vb, depth, geo)?;
            }
        }
        Infill::Door => door(b, op, e, f, stone, &o, &base, uc, vb, depth, geo)?,
        Infill::Shutters | Infill::Louvres => louvres(b, op, f, &o, uc, vb, e.w, depth, geo)?,
        _ => {}
    }
    if op.bells {
        let bronze = b.mat_or(None, "bronze")?;
        let bh = (e.w * 0.55).min(base.spring * 0.8).max(0.3);
        let ctr = f.plane.at_d(Vec2::new(uc, vb + base.spring * 0.25), -depth - 0.5);
        let prof: Vec<Vec2> = [(0.0, bh), (0.22, bh), (0.3, bh * 0.92), (0.33, bh * 0.6), (0.4, bh * 0.25), (0.5, bh * 0.06), (0.52, 0.0), (0.44, 0.0), (0.0, 0.02)].iter().map(|(r, y)| Vec2::new(r * bh, *y)).collect();
        let mut prof = prof;
        prof.reverse();
        // Profile from bottom (mouth) to top.
        let mut sorted: Vec<Vec2> = prof.clone();
        sorted.sort_by(|a, b| a.y.total_cmp(&b.y));
        mesh::lathe(geo.get(bronze), ctr, &[Vec2::new(0.0, 0.0), Vec2::new(0.52 * bh, 0.0), Vec2::new(0.5 * bh, 0.06 * bh), Vec2::new(0.4 * bh, 0.25 * bh), Vec2::new(0.33 * bh, 0.6 * bh), Vec2::new(0.3 * bh, 0.92 * bh), Vec2::new(0.22 * bh, bh), Vec2::new(0.0, bh)], 16, 0.0, std::f32::consts::TAU);
        let wood = b.mat_or(None, "wood")?;
        let yoke = f.plane.at_d(Vec2::new(uc, vb + base.spring * 0.25 + bh), -depth - 0.5);
        geo.get(wood).beam(yoke - f.plane.u * (e.w * 0.5), yoke + f.plane.u * (e.w * 0.5), f.plane.n, 0.18, 0.2, 1.0);
    }
    trims(b, op, e, f, cx, &base, &o, uc, vb, spring, geo)?;
    if let Some(bal) = op.balcony.get()
        && !bal.continuous
    {
        balcony(b, bal, f, cx, (uc - e.w * 0.5 - bal.extend, uc + e.w * 0.5 + bal.extend), vb, geo)?;
    }
    let _ = sc;
    Ok(())
}

/// Frame, glazing bars, glass and the room behind.
#[allow(clippy::too_many_arguments)]
fn glass_window(b: &mut Builder, op: &Openings, e: &Eff, f: &Facade, o: &[Vec2], uc: f32, vb: f32, w: f32, spring_rel: f32, depth: f32, room: f32, geo: &mut Geo) -> Result<()> {
    let glass = b.glass_mat(op.glass_material.as_deref())?;
    let frame_m = b.frame_mat(op.frame_material.as_deref())?;
    let interior = b.mat_or(None, "interior")?;
    let zf = depth - 0.04;
    // Glass pane slightly in front of the recess end.
    if let Some(r) = poly::normalize(o, true) {
        let pl = Plane { o: f.plane.o - f.plane.n * (zf + 0.015), ..f.plane };
        mesh::planar_shape(geo.get(glass), &poly::Shape::new(r), &pl, false, Vec2::new(-uc + w * 0.5, -vb));
    }
    // Room box behind (atlas column from the position).
    let (mn, mx) = poly::bbox(o);
    let room = room.clamp(0.15, 1.4);
    let k = ((uc * 7.3 + vb * 3.1).abs() as usize) % 4;
    let ua = |t: f32| k as f32 * 0.25 + 0.02 + t * 0.21;
    let (x0, x1, y0, y1) = (mn.x - 0.5, mx.x + 0.5, mn.y - 0.3, mx.y + 0.4);
    let zb = depth + room;
    let pi = geo.get(interior);
    let at = |u: f32, v: f32, z: f32| f.plane.at_d(Vec2::new(u, v), -z);
    let n = f.plane.n;
    mesh::quad_facing(pi, [at(x0, y0, zb), at(x1, y0, zb), at(x1, y1, zb), at(x0, y1, zb)], [Vec2::new(ua(0.0), 1.0), Vec2::new(ua(1.0), 1.0), Vec2::new(ua(1.0), 0.05), Vec2::new(ua(0.0), 0.05)], n);
    mesh::quad_facing(pi, [at(x0, y1, zb), at(x1, y1, zb), at(x1, y1, depth), at(x0, y1, depth)], [Vec2::new(ua(0.0), 0.05), Vec2::new(ua(1.0), 0.05), Vec2::new(ua(1.0), 0.0), Vec2::new(ua(0.0), 0.0)], -f.plane.v);
    mesh::quad_facing(pi, [at(x0, y0, depth), at(x1, y0, depth), at(x1, y0, zb), at(x0, y0, zb)], [Vec2::new(ua(0.0), 1.0), Vec2::new(ua(1.0), 1.0), Vec2::new(ua(1.0), 0.95), Vec2::new(ua(0.0), 0.95)], f.plane.v);
    mesh::quad_facing(pi, [at(x0, y0, depth), at(x0, y0, zb), at(x0, y1, zb), at(x0, y1, depth)], [Vec2::new(ua(0.0), 1.0), Vec2::new(ua(0.1), 1.0), Vec2::new(ua(0.1), 0.05), Vec2::new(ua(0.0), 0.05)], f.plane.u);
    mesh::quad_facing(pi, [at(x1, y0, zb), at(x1, y0, depth), at(x1, y1, depth), at(x1, y1, zb)], [Vec2::new(ua(0.9), 1.0), Vec2::new(ua(1.0), 1.0), Vec2::new(ua(1.0), 0.05), Vec2::new(ua(0.9), 0.05)], -f.plane.u);
    // Closing ring between the reveal end and the room front.
    let ring_out = [at(x0, y0, depth), at(x1, y0, depth), at(x1, y1, depth), at(x0, y1, depth)];
    let _ = ring_out;
    // Joinery frame.
    let fw = op.frame;
    if fw > 0.0 {
        let h = (mx.y - mn.y).max(0.1);
        let inner: Vec<Vec2> = facade::grow(o, uc, vb, w, h, -2.0 * fw, -2.0 * fw).into_iter().map(|q| q + Vec2::new(0.0, fw)).collect();
        let p = geo.get(frame_m);
        band(p, f, &inner, o, -zf + 0.05, None, false, -zf);
        // Inner edge of the frame back to the glass.
        let m = inner.len();
        for i in 0..m {
            let j = (i + 1) % m;
            let q = [f.plane.at_d(inner[i], -zf), f.plane.at_d(inner[j], -zf), f.plane.at_d(inner[j], -zf + 0.05), f.plane.at_d(inner[i], -zf + 0.05)];
            let d = inner[j] - inner[i];
            let want = f.plane.u * -d.y + f.plane.v * d.x;
            mesh::quad_world(p, q, want);
        }
        // Mullions, transoms and glazing bars.
        let mut bars: Vec<(Vec2, Vec2, f32)> = Vec::new();
        let (cols, rows) = op.panes.map(|p| (p[0].max(1), p[1].max(1))).unwrap_or(if e.kind == OpeningKind::Window && e.arch == ArchKind::Flat && w > 0.8 { (2, 1) } else { (1, 1) });
        for k in 1..cols {
            let u = uc - w * 0.5 + w * k as f32 / cols as f32;
            if let Some(t) = top_at(o, u) {
                bars.push((Vec2::new(u, vb), Vec2::new(u, t), if k * 2 == cols { fw * 0.9 } else { fw * 0.45 }));
            }
        }
        let spring = vb + spring_rel;
        let t_rows = rows.max(op.transoms + 1);
        for k in 1..t_rows {
            let v = vb + (spring - vb) * k as f32 / t_rows as f32;
            if let Some((a, c)) = span_at(o, v) {
                bars.push((Vec2::new(a, v), Vec2::new(c, v), if k as u32 <= op.transoms { fw * 0.9 } else { fw * 0.45 }));
            }
        }
        if e.arch != ArchKind::Flat && !matches!(e.arch, ArchKind::Circle | ArchKind::Quatrefoil) && spring - vb > 0.3 {
            if let Some((a, c)) = span_at(o, spring) {
                bars.push((Vec2::new(a, spring), Vec2::new(c, spring), fw * 0.9));
            }
        }
        for (a, c, wd) in bars {
            fbar(p, f, a, c, wd, (-zf - 0.01, -zf + 0.04));
        }
    }
    Ok(())
}

/// Stone mullions and tracery in the head of a window.
#[allow(clippy::too_many_arguments)]
fn tracery(b: &mut Builder, op: &Openings, e: &Eff, f: &Facade, stone: MatId, o: &[Vec2], base: &Outline, uc: f32, vb: f32, depth: f32, geo: &mut Geo) -> Result<()> {
    let lights = (e.mullions + 1).max(if op.tracery.is_some() || e.infill == Infill::Tracery { 2 } else { 1 });
    let mw = (e.w * 0.07).clamp(0.08, 0.2);
    let z = (-depth * 0.55, -depth * 0.2);
    let spring = vb + base.spring;
    if e.infill == Infill::Stained {
        // Lead-lined glass behind the stonework.
        let sg = b.mat_or(op.glass_material.as_deref(), "stained_glass")?;
        if let Some(r) = poly::normalize(o, true) {
            let pl = Plane { o: f.plane.o - f.plane.n * (depth - 0.02), ..f.plane };
            mesh::planar_shape(geo.get(sg), &poly::Shape::new(r), &pl, false, Vec2::ZERO);
        }
    }
    let p = geo.get(stone);
    let lw = e.w / lights as f32;
    for k in 1..lights {
        let u = uc - e.w * 0.5 + lw * k as f32;
        let top = top_at(o, u).unwrap_or(spring);
        fbar(p, f, Vec2::new(u, vb), Vec2::new(u, top.min(spring + (top - spring) * 0.55)), mw, z);
    }
    if lights >= 2 && !matches!(e.arch, ArchKind::Flat | ArchKind::Circle | ArchKind::Quatrefoil) {
        // Sub-arches over each light.
        let n = (base.head.1 - base.head.0).max(4);
        for k in 0..lights {
            let lc = uc - e.w * 0.5 + lw * (k as f32 + 0.5);
            let sub = profiles::outline(e.arch, lw, (spring - vb) + profiles::default_rise(e.arch, lw), None, n / 2);
            let head: Vec<Vec2> = sub.pts[sub.head.0..sub.head.1.min(sub.pts.len())].iter().map(|q| *q + Vec2::new(lc, vb)).collect();
            fcurve(p, f, &head, mw * 0.8, z, false);
        }
        // Ornament in the head.
        let rise = base.pts.iter().map(|q| q.y).fold(0.0, f32::max) - base.spring;
        let sub_rise = profiles::default_rise(e.arch, lw);
        let free = rise - sub_rise;
        if free > 0.25 {
            let r = (free * 0.42).min(e.w * 0.24);
            let c = Vec2::new(uc, spring + sub_rise + free * 0.48);
            let kind = op.tracery.unwrap_or(TraceryKind::Circle);
            let lobes = match kind {
                TraceryKind::Trefoil => 3,
                TraceryKind::Quatrefoil | TraceryKind::Geometric => 4,
                _ => 0,
            };
            let m = 32;
            let ring: Vec<Vec2> = (0..m)
                .map(|i| {
                    let a = i as f32 / m as f32 * std::f32::consts::TAU + std::f32::consts::FRAC_PI_2;
                    let lobe = if lobes > 0 { 1.0 - 0.22 * (1.0 - ((a - std::f32::consts::FRAC_PI_2) * lobes as f32 * 0.5).cos().abs()) } else { 1.0 };
                    c + Vec2::from_angle(a) * r * lobe
                })
                .collect();
            fcurve(p, f, &ring, mw * 0.7, z, true);
            if lobes == 0 {
                let ring2: Vec<Vec2> = (0..m).map(|i| c + Vec2::from_angle(i as f32 / m as f32 * std::f32::consts::TAU) * r * 0.45).collect();
                fcurve(p, f, &ring2, mw * 0.5, z, true);
            }
        }
    }
    Ok(())
}

/// Rose window: spokes, rings and a boss.
#[allow(clippy::too_many_arguments)]
fn rose(b: &mut Builder, e: &Eff, f: &Facade, stone: MatId, o: &[Vec2], uc: f32, vb: f32, depth: f32, geo: &mut Geo) {
    let r = e.w * 0.5;
    let c = Vec2::new(uc, vb + r);
    if let Ok(sg) = b.mat_or(None, "stained_glass")
        && let Some(ring) = poly::normalize(o, true)
    {
        let pl = Plane { o: f.plane.o - f.plane.n * (depth - 0.02), ..f.plane };
        mesh::planar_shape(geo.get(sg), &poly::Shape::new(ring), &pl, false, Vec2::ZERO);
    }
    let p = geo.get(stone);
    let z = (-depth * 0.6, -depth * 0.15);
    let bw = (r * 0.045).clamp(0.06, 0.18);
    let hub = r * 0.18;
    let m = 48;
    let circle = |rad: f32| -> Vec<Vec2> { (0..m).map(|i| c + Vec2::from_angle(i as f32 / m as f32 * std::f32::consts::TAU) * rad).collect() };
    fcurve(p, f, &circle(hub), bw, z, true);
    for k in 1..=e.rings {
        let rr = hub + (r - hub) * k as f32 / (e.rings + 1) as f32;
        fcurve(p, f, &circle(rr), bw * 0.8, z, true);
    }
    for k in 0..e.spokes {
        let a = k as f32 / e.spokes as f32 * std::f32::consts::TAU + std::f32::consts::FRAC_PI_2;
        let d = Vec2::from_angle(a);
        fbar(p, f, c + d * hub, c + d * (r - 0.02), bw * 0.8, z);
        // Small lobed arch between spokes near the rim.
        let a2 = a + std::f32::consts::PI / e.spokes as f32;
        let span = std::f32::consts::PI / e.spokes as f32 * 0.9;
        let arc: Vec<Vec2> = (0..=8).map(|i| {
            let t = a2 - span + 2.0 * span * i as f32 / 8.0;
            c + Vec2::from_angle(t) * (r * 0.8 + r * 0.06 * ((i as f32 / 8.0) * std::f32::consts::PI).sin())
        }).collect();
        fcurve(p, f, &arc, bw * 0.55, z, false);
    }
    // Boss.
    let boss = f.plane.at_d(c, z.1);
    p.cone(boss, boss + f.plane.n * 0.12, hub * 0.6, hub * 0.3, 12, true);
}

/// Door leaves, panels, tympanum or fanlight.
#[allow(clippy::too_many_arguments)]
fn door(b: &mut Builder, op: &Openings, e: &Eff, f: &Facade, stone: MatId, o: &[Vec2], base: &Outline, uc: f32, vb: f32, depth: f32, geo: &mut Geo) -> Result<()> {
    let dm = b.door_mat(op.door_material.as_deref())?;
    let iron = b.iron_mat(op.iron_material.as_deref())?;
    let spring = vb + base.spring;
    let arched = e.arch != ArchKind::Flat && base.spring < e.h - 0.05;
    let leaf_top = if arched { spring } else { vb + e.h };
    let z = depth - 0.01;
    let (u0, u1) = (uc - e.w * 0.5, uc + e.w * 0.5);
    if arched && e.tympanum {
        // Stone tympanum over a lintel.
        let lintel = 0.32f32.min((leaf_top - vb) * 0.15);
        let p = geo.get(stone);
        fbox(p, f, (u0, u1), (leaf_top - lintel, leaf_top), (-z, -z + 0.12));
        let head: Vec<Vec2> = o.iter().copied().filter(|q| q.y >= spring - 1e-3).collect();
        if head.len() >= 3
            && let Some(r) = poly::normalize(&head, true)
        {
            let pl = Plane { o: f.plane.o - f.plane.n * (z - 0.06), ..f.plane };
            mesh::planar_shape(p, &poly::Shape::new(r), &pl, false, Vec2::ZERO);
        }
    } else if arched {
        // Fanlight: glass and radial bars.
        let glass = b.glass_mat(op.glass_material.as_deref())?;
        let head: Vec<Vec2> = o.iter().copied().filter(|q| q.y >= spring - 1e-3).collect();
        if head.len() >= 3
            && let Some(r) = poly::normalize(&head, true)
        {
            let pl = Plane { o: f.plane.o - f.plane.n * (z - 0.02), ..f.plane };
            mesh::planar_shape(geo.get(glass), &poly::Shape::new(r), &pl, false, Vec2::ZERO);
        }
        let c = Vec2::new(uc, spring);
        let pi = geo.get(iron);
        for k in 1..6 {
            let a = std::f32::consts::PI * k as f32 / 6.0;
            let d = Vec2::from_angle(a);
            if let Some(t) = top_at(o, c.x + d.x * 0.01) {
                let len = ((t - spring) / d.y.max(0.2)).min(e.w * 0.5);
                fbar(pi, f, c, c + d * len, 0.03, (-z + 0.02, -z + 0.05));
            }
        }
        fbox(geo.get(dm), f, (u0, u1), (spring - 0.1, spring), (-z, -z + 0.08));
    }
    // Leaves with raised panels.
    let p = geo.get(dm);
    let leaf_h = (leaf_top - vb - if arched && e.tympanum { 0.32f32.min((leaf_top - vb) * 0.15) } else { 0.0 }).max(0.5);
    let leaves = if e.w > 1.3 { 2 } else { 1 };
    let lw = e.w / leaves as f32;
    for l in 0..leaves {
        let a = u0 + lw * l as f32;
        fbox(p, f, (a + 0.01, a + lw - 0.01), (vb, vb + leaf_h), (-z, -z + 0.05));
        let rows = ((leaf_h / 0.9).round() as usize).clamp(2, 6);
        let ph = (leaf_h - 0.24) / rows as f32;
        for r in 0..rows {
            let v = vb + 0.12 + ph * r as f32;
            fbox(p, f, (a + 0.1, a + lw - 0.1), (v + 0.05, v + ph - 0.05), (-z + 0.05, -z + 0.085));
        }
    }
    if leaves == 2 {
        fbox(p, f, (uc - 0.025, uc + 0.025), (vb, vb + leaf_h), (-z + 0.05, -z + 0.09));
    }
    // Iron knockers / handles.
    let pi = geo.get(iron);
    for s in [-1.0f32, 1.0] {
        if leaves == 2 || s > 0.0 {
            let u = uc + s * 0.12;
            let c = f.plane.at_d(Vec2::new(u, vb + 1.05), -z + 0.1);
            pi.cylinder(c - f.plane.v * 0.06, c + f.plane.v * 0.06, 0.015, 6, true);
        }
    }
    // Step / threshold.
    fbox(geo.get(stone), f, (u0 - 0.05, u1 + 0.05), (vb, vb + 0.03), (-depth, 0.08));
    Ok(())
}

/// Closed louvred shutters in the opening.
#[allow(clippy::too_many_arguments)]
fn louvres(b: &mut Builder, op: &Openings, f: &Facade, o: &[Vec2], uc: f32, vb: f32, w: f32, depth: f32, geo: &mut Geo) -> Result<()> {
    let m = b.frame_mat(op.frame_material.as_deref())?;
    let p = geo.get(m);
    let (mn, mx) = poly::bbox(o);
    let z = depth * 0.5;
    let top = mx.y - 0.02;
    let mut v = vb + 0.08;
    while v < top - 0.05 {
        if let Some((a, c)) = span_at(o, v + 0.03) {
            // Tilted slat.
            let c0 = f.plane.at_d(Vec2::new((a + c) * 0.5, v + 0.03), -z);
            let tilt = (f.plane.v * 0.7 + f.plane.n * 0.7).normalize();
            p.obox(c0, [f.plane.u, tilt, tilt.cross(f.plane.u)], Vec3::new((c - a) * 0.5 - 0.02, 0.05, 0.008), 1.0);
        }
        v += 0.075;
    }
    fbox(p, f, (uc - 0.03, uc + 0.03), (vb, top), (-z - 0.03, -z + 0.03));
    fbox(p, f, (mn.x, mn.x + 0.06), (vb, top), (-z - 0.03, -z + 0.03));
    fbox(p, f, (mx.x - 0.06, mx.x), (vb, top), (-z - 0.03, -z + 0.03));
    let _ = w;
    Ok(())
}

/// Wrought-iron window grille.
fn grille(b: &mut Builder, op: &Openings, f: &Facade, o: &[Vec2], uc: f32, vb: f32, w: f32, geo: &mut Geo) -> Result<()> {
    let iron = b.iron_mat(op.iron_material.as_deref())?;
    let p = geo.get(iron);
    let n = ((w / 0.12).round() as usize).max(2);
    let z = (-0.1, -0.08);
    for k in 1..n {
        let u = uc - w * 0.5 + w * k as f32 / n as f32;
        if let Some(t) = top_at(o, u) {
            fbar(p, f, Vec2::new(u, vb), Vec2::new(u, t), 0.02, z);
        }
    }
    let (_, mx) = poly::bbox(o);
    let mut v = vb + 0.3;
    while v < mx.y - 0.1 {
        if let Some((a, c)) = span_at(o, v) {
            fbar(p, f, Vec2::new(a, v), Vec2::new(c, v), 0.025, (z.0 - 0.01, z.1 + 0.01));
        }
        v += 0.6;
    }
    Ok(())
}

/// Surround, ears, pediment, hood, voussoirs, keystone and sill.
#[allow(clippy::too_many_arguments)]
fn trims(b: &mut Builder, op: &Openings, e: &Eff, f: &Facade, cx: &OpCtx, base: &Outline, o: &[Vec2], uc: f32, vb: f32, spring: f32, geo: &mut Geo) -> Result<()> {
    let at_bottom = vb <= f.v0 + 0.02;
    let h_tot = base.pts.iter().map(|q| q.y).fold(0.0, f32::max).max(0.1);
    let w_out = if e.orders > 0 { e.w + 2.0 * e.orders as f32 * op.order_step } else { e.w + 2.0 * e.splay };
    let h_out = if e.orders > 0 { h_tot + e.orders as f32 * op.order_step } else { h_tot + e.splay };
    let ring_out: Vec<Vec2> = if (w_out - e.w).abs() > 1e-4 { facade::grow(o, uc, vb, e.w, h_tot, w_out - e.w, h_out - h_tot) } else { o.to_vec() };
    if let Some(s) = op.surround.get() {
        let m = b.mat(s.material.as_deref(), cx.trim)?;
        let outer = facade::grow(&ring_out, uc, vb, w_out, h_out, 2.0 * s.width, s.width);
        let n = ring_out.len();
        let range = if base.round { None } else { Some((0, n - 1)) };
        let p = geo.get(m);
        band(p, f, &ring_out, &outer, s.projection, range, true, 0.0);
        if !base.round && !at_bottom && op.sill.get().is_none() {
            // Bottom rail of the frame.
            fbox(p, f, (uc - w_out * 0.5 - s.width, uc + w_out * 0.5 + s.width), (vb - s.width, vb), (0.0, s.projection));
        }
        if s.ears {
            let top = vb + h_out;
            for sg in [-1.0f32, 1.0] {
                let u = uc + sg * (w_out * 0.5 + s.width);
                fbox(p, f, (u.min(u + sg * s.width * 0.8), u.max(u + sg * s.width * 0.8)), (top - s.width * 1.6, top + s.width), (0.0, s.projection));
            }
        }
        if let Some(ped) = &s.pediment {
            let top = vb + h_out + s.width;
            let half = w_out * 0.5 + s.width + 0.12;
            fbox(p, f, (uc - half, uc + half), (top, top + 0.14), (0.0, s.projection + 0.1));
            let rise = (half * 0.42).min(0.9);
            let apex = Vec2::new(uc, top + 0.14 + rise);
            if ped == "segmental" {
                let arc: Vec<Vec2> = (0..=12).map(|i| {
                    let t = std::f32::consts::PI * i as f32 / 12.0;
                    Vec2::new(uc + half * t.cos(), top + 0.14 + rise * t.sin())
                }).collect();
                fcurve(p, f, &arc, 0.16, (0.0, s.projection + 0.1), false);
            } else {
                fbar(p, f, Vec2::new(uc - half, top + 0.14), apex, 0.16, (0.0, s.projection + 0.1));
                fbar(p, f, Vec2::new(uc + half, top + 0.14), apex, 0.16, (0.0, s.projection + 0.1));
            }
            // Tympanum face.
            let tri = vec![Vec2::new(uc - half + 0.1, top + 0.14), Vec2::new(uc + half - 0.1, top + 0.14), apex - Vec2::new(0.0, 0.1)];
            if let Some(r) = poly::normalize(&tri, true) {
                let pl = Plane { o: f.plane.o + f.plane.n * (s.projection * 0.5), ..f.plane };
                mesh::planar_shape(p, &poly::Shape::new(r), &pl, false, Vec2::ZERO);
            }
        }
    }
    if let Some(v) = op.voussoirs.get()
        && !base.round
        && base.head.1 > base.head.0 + 2
    {
        let m1 = b.mat(v.material.as_deref(), cx.trim)?;
        let m2 = b.mat(v.alternate.as_deref(), m1)?;
        let outer = facade::grow(&ring_out, uc, vb, w_out, h_out, 2.0 * v.width, v.width);
        let (h0, h1) = (base.head.0, base.head.1.min(ring_out.len() - 1));
        let count = v.count.max(3) as usize;
        let span = h1 - h0;
        for k in 0..count {
            let s0 = h0 + span * k / count;
            let s1 = (h0 + span * (k + 1) / count).max(s0 + 1);
            let m = if k % 2 == 0 { m1 } else { m2 };
            band(geo.get(m), f, &ring_out, &outer, v.projection, Some((s0, s1)), true, 0.0);
        }
    }
    if let Some(ks) = op.keystone {
        if ks.0 > 0.0 && !base.round {
            let top = vb + h_out;
            let k = ks.0;
            let shape = vec![Vec2::new(uc - k * 0.32, top - k * 0.55), Vec2::new(uc + k * 0.32, top - k * 0.55), Vec2::new(uc + k * 0.45, top + k * 0.45), Vec2::new(uc - k * 0.45, top + k * 0.45)];
            if let Some(r) = poly::normalize(&shape, true) {
                let m = cx.trim;
                mesh::extrude_shape(geo.get(m), &poly::Shape::new(r), &f.plane, 0.12);
            }
        }
    }
    if let Some(h) = op.hood.get() {
        let m = b.mat(h.material.as_deref(), cx.trim)?;
        let grown = facade::grow(&ring_out, uc, vb, w_out, h_out, 2.0 * h.gap, h.gap);
        let (h0, h1) = (base.head.0, base.head.1.min(grown.len()));
        let mut path: Vec<Vec2> = if base.round { grown.clone() } else { grown[h0..h1].to_vec() };
        if !base.round && path.len() >= 2 {
            if h.stops {
                let first = path[0];
                let last = *path.last().unwrap();
                path.insert(0, first + Vec2::new(h.size * 2.2, 0.0));
                path.push(last - Vec2::new(h.size * 2.2, 0.0));
            }
            let _ = spring;
        }
        if path.len() >= 2 {
            let offs = mesh::polyline_offsets_2d(&path, base.round);
            let p3: Vec<Vec3> = path.iter().map(|q| f.plane.at(*q)).collect();
            let side: Vec<Vec3> = offs.iter().map(|o2| f.plane.u * o2.x + f.plane.v * o2.y).collect();
            let up = vec![f.plane.n; path.len()];
            let s = h.size;
            let prof = [Vec2::new(0.0, 0.0), Vec2::new(s, 0.0), Vec2::new(s, s * 0.45), Vec2::new(s * 0.45, s), Vec2::new(0.0, s)];
            mesh::sweep(geo.get(m), &p3, &side, &up, &prof, base.round, true, false, !base.round);
        }
    }
    if let Some(s) = op.sill.get()
        && !at_bottom
        && !base.round
    {
        let m = b.mat(s.material.as_deref(), cx.trim)?;
        let half = w_out * 0.5 + s.extend;
        fbox(geo.get(m), f, (uc - half, uc + half), (vb - s.height, vb), (-0.05, s.depth));
    }
    Ok(())
}

/// Balcony slab on corbels with a railing.
fn balcony(b: &mut Builder, s: &BalconySpec, f: &Facade, cx: &OpCtx, u: (f32, f32), vb: f32, geo: &mut Geo) -> Result<()> {
    let slab = b.mat(s.material.as_deref(), cx.trim)?;
    let p = geo.get(slab);
    let (u0, u1) = u;
    fbox(p, f, (u0, u1), (vb - s.thickness, vb), (0.0, s.depth));
    fbox(p, f, (u0 - 0.03, u1 + 0.03), (vb - s.thickness - 0.05, vb - s.thickness + 0.02), (0.0, s.depth + 0.04));
    if s.corbels {
        let n = (((u1 - u0) / 1.4).floor() as usize + 1).max(2);
        for k in 0..n {
            let uu = u0 + 0.25 + (u1 - u0 - 0.5) * k as f32 / (n - 1) as f32;
            fbox(p, f, (uu - 0.07, uu + 0.07), (vb - s.thickness - 0.45, vb - s.thickness), (0.0, 0.12));
            fbox(p, f, (uu - 0.06, uu + 0.06), (vb - s.thickness - 0.25, vb - s.thickness), (0.12, s.depth * 0.6));
        }
    }
    let rh = s.railing_height;
    match s.railing.as_str() {
        "none" => {}
        "stone" | "balustrade" => {
            let pts = [Vec2::new(u0 + 0.12, 0.0), Vec2::new(u0 + 0.12, s.depth - 0.12), Vec2::new(u1 - 0.12, s.depth - 0.12), Vec2::new(u1 - 0.12, 0.0)];
            // Balusters along the three sides.
            for w in pts.windows(2) {
                let (a, c) = (w[0], w[1]);
                let len = (c - a).length();
                let n = (len / 0.26).floor().max(1.0) as usize;
                for k in 0..n {
                    let t = (k as f32 + 0.5) / n as f32;
                    let q = a.lerp(c, t);
                    let base = f.plane.at_d(Vec2::new(q.x, vb), q.y);
                    let prof: Vec<Vec2> = [(0.05, 0.0), (0.06, 0.04), (0.045, 0.1), (0.08, 0.32), (0.04, 0.52), (0.035, 0.6), (0.05, 0.66)].iter().map(|(r, y)| Vec2::new(*r, y * rh / 0.8)).collect();
                    let mut pr = vec![Vec2::new(0.0, 0.0)];
                    pr.extend(prof);
                    pr.push(Vec2::new(0.0, 0.66 * rh / 0.8));
                    mesh::lathe(p, base, &pr, 8, 0.0, std::f32::consts::TAU);
                }
            }
            fbox(p, f, (u0, u1), (vb + rh * 0.82, vb + rh), (s.depth - 0.22, s.depth));
            fbox(p, f, (u0, u0 + 0.22), (vb + rh * 0.82, vb + rh), (0.0, s.depth));
            fbox(p, f, (u1 - 0.22, u1), (vb + rh * 0.82, vb + rh), (0.0, s.depth));
        }
        "glass" => {
            let g = b.glass_mat(None)?;
            fbox(geo.get(g), f, (u0 + 0.05, u1 - 0.05), (vb, vb + rh), (s.depth - 0.06, s.depth - 0.04));
        }
        _ => {
            let iron = b.iron_mat(s.railing_material.as_deref())?;
            let p = geo.get(iron);
            let zf = s.depth - 0.04;
            fbox(p, f, (u0 + 0.02, u1 - 0.02), (vb + rh - 0.045, vb + rh), (zf - 0.025, zf + 0.025));
            fbox(p, f, (u0 + 0.02, u1 - 0.02), (vb + 0.05, vb + 0.08), (zf - 0.015, zf + 0.015));
            fbox(p, f, (u0 + 0.02, u1 - 0.02), (vb + rh - 0.17, vb + rh - 0.15), (zf - 0.012, zf + 0.012));
            let n = (((u1 - u0) / 0.11).floor() as usize).max(2);
            for k in 0..=n {
                let uu = u0 + 0.04 + (u1 - u0 - 0.08) * k as f32 / n as f32;
                fbar(p, f, Vec2::new(uu, vb + 0.05), Vec2::new(uu, vb + rh), 0.016, (zf - 0.008, zf + 0.008));
            }
            for side in [u0 + 0.03, u1 - 0.03] {
                fbox(p, f, (side - 0.02, side + 0.02), (vb + rh - 0.045, vb + rh), (0.0, zf));
                let m = ((zf / 0.11).floor() as usize).max(1);
                for k in 0..m {
                    let z = 0.08 + (zf - 0.1) * k as f32 / m as f32;
                    fbox(p, f, (side - 0.008, side + 0.008), (vb + 0.05, vb + rh), (z - 0.008, z + 0.008));
                }
            }
        }
    }
    Ok(())
}

/// Openings on a set of facades (one per selected edge).
pub fn on_facades(b: &mut Builder, list: &[&Openings], facades: &[(Facade, super::resolve::EdgeRef)], shape: &poly::Shape, sc: &Scope, cx: &OpCtx, geo: &mut Geo) -> Result<Vec<Vec<Cut>>> {
    let mut all = vec![Vec::new(); facades.len()];
    for (k, op) in list.iter().enumerate() {
        let edges = super::resolve::select(shape, &op.edges);
        let trim = b.mat(op.material.as_deref(), cx.trim)?;
        let c = OpCtx { wall: cx.wall, trim, through: cx.through, room: cx.room };
        let mut any = false;
        for (fi, (f, e)) in facades.iter().enumerate() {
            if !edges.contains(e) {
                continue;
            }
            any = true;
            place(b, op, f, sc, &c, &mut all[fi], geo).map_err(|err| err.context(&format!("openings[{k}]")))?;
        }
        if !any {
            b.warn(sc, format!("openings[{k}] ({}): edge selection matched no facade", op.kind.name()));
        }
    }
    Ok(all)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bays_layouts() {
        let c = bay_centres(Some(&Bays { count: Some(3), ..Default::default() }), 12.0, 1.2, OpeningKind::Window);
        assert_eq!(c.len(), 3);
        assert!((c[1] - 6.0).abs() < 1e-4);
        let c = bay_centres(Some(&Bays { offsets: Some(vec![-3.0, 3.0]), ..Default::default() }), 20.0, 1.0, OpeningKind::Window);
        assert_eq!(c, vec![7.0, 13.0]);
        let c = bay_centres(Some(&Bays { spacing: Some(3.0), margin: Some(1.0), skip: vec![0], ..Default::default() }), 14.0, 1.2, OpeningKind::Window);
        assert!(c.iter().all(|u| *u > 1.0 && *u < 13.0));
        assert_eq!(bay_centres(None, 10.0, 2.0, OpeningKind::Portal), vec![5.0]);
        assert!(bay_centres(None, 0.8, 1.2, OpeningKind::Window).is_empty());
    }
}
