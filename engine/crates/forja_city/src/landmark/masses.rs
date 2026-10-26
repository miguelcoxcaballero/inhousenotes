//! Masses (extruded plans with facades), slabs, free-standing walls and
//! towers.

use forja_math::{Mat4, Vec2, Vec3};

use super::build::{Builder, Out, Scope};
use super::facade::{self, Cut, Facade};
use super::mesh::{self, MatId, Plane};
use super::openings::{self, OpCtx};
use super::parse::{Error, Result};
use super::poly::{self, Shape};
use super::resolve::{self, EdgeRef};
use super::spec::*;
use super::{details, roofs};

/// Top height of a mass from y1 / height / floors.
pub fn mass_top(b: &Builder, m: &Mass) -> f32 {
    if let Some(y1) = m.y1 {
        return y1;
    }
    if let Some(h) = m.height {
        return m.y0 + h;
    }
    if let Some(f) = m.floors {
        return m.y0 + f * m.floor_height.unwrap_or(b.floor_height());
    }
    m.y0 + 10.0
}

/// One facade per edge of every ring of `shape`.
pub fn facades(shape: &Shape, y0: f32, y1: f32, mat: MatId) -> Vec<(Facade, EdgeRef)> {
    let mut out = Vec::new();
    for (r, (ring, _)) in shape.rings().enumerate() {
        let n = ring.len();
        let per = poly::perimeter(ring);
        let mut d = 0.0;
        for i in 0..n {
            let (a, bb) = (ring[i], ring[(i + 1) % n]);
            let len = (bb - a).length();
            d += len;
            if len < 1e-4 {
                continue;
            }
            let u = Vec3::new(a.x - bb.x, 0.0, a.y - bb.y) / len;
            let plane = Plane::new(Vec3::new(bb.x, 0.0, bb.y), u, Vec3::Y);
            out.push((Facade { plane, len, v0: y0, v1: y1, uv_off: per - d, mat, outline: None }, EdgeRef { ring: r, i }));
        }
    }
    out
}

/// Inline details shared by masses and tower stages.
pub struct Inline<'s> {
    pub cornice: &'s Flex<MouldingSpec>,
    pub plinth: &'s Flex<MouldingSpec>,
    pub string_courses: &'s [f32],
    pub parapet: &'s Flex<ParapetSpec>,
    pub crenellations: &'s Flex<CrenelSpec>,
    pub balustrade: &'s Flex<BalustradeSpec>,
    pub roof: Option<&'s Flex<RoofSpec>>,
}

/// Walls with openings, caps, collider and inline details of a prism.
#[allow(clippy::too_many_arguments)]
pub fn prism_with_facades(b: &mut Builder, shape: &Shape, y0: f32, y1: f32, batter: f32, smooth: bool, top: Option<MatId>, bottom: bool, ops: &[&Openings], sc: &Scope, o: &mut Out) -> Result<()> {
    let wall = sc.mat;
    if y1 <= y0 + 1e-3 {
        return Err(Error::msg(format!("height must be positive (y0 = {y0}, top = {y1})")));
    }
    if batter.abs() > 1e-4 {
        // Battered (sloping) walls: a band between the base and the inset top.
        let top_ring = poly::offset(&shape.outer, -batter).filter(|r| r.len() == shape.outer.len()).unwrap_or_else(|| {
            let c = poly::centroid(&shape.outer);
            let (mn, mx) = poly::bbox(&shape.outer);
            let k = 1.0 - 2.0 * batter / (mx - mn).min_element().max(0.1);
            shape.outer.iter().map(|p| c + (*p - c) * k).collect()
        });
        let lo: Vec<Vec3> = shape.outer.iter().map(|p| Vec3::new(p.x, y0, p.y)).collect();
        let hi: Vec<Vec3> = top_ring.iter().map(|p| Vec3::new(p.x, y1, p.y)).collect();
        let c = poly::centroid(&shape.outer);
        mesh::ring_band(o.geo.get(wall), &lo, &hi, true, Some(Vec3::new(c.x, (y0 + y1) * 0.5, c.y)));
        for h in &shape.holes {
            mesh::ring_walls(o.geo.get(wall), h, y0, y1, 0.0);
        }
        if !ops.is_empty() {
            b.warn(sc, "openings are not cut into battered walls");
        }
        let top_shape = Shape { outer: top_ring.clone(), holes: shape.holes.clone() };
        if let Some(tm) = top {
            mesh::horizontal_shape(o.geo.get(tm), &top_shape, y1, true);
        }
        if bottom {
            mesh::horizontal_shape(o.geo.get(wall), shape, y0, false);
        }
        o.collide_all();
        return Ok(());
    }
    let fs = facades(shape, y0, y1, wall);
    let (bmn, bmx) = shape.bbox();
    let room = ((bmx - bmn).min_element() * 0.4 - 0.4).clamp(0.2, 1.4);
    let cx = OpCtx { wall, trim: wall, through: None, room };
    let cuts: Vec<Vec<Cut>> = if ops.is_empty() { vec![Vec::new(); fs.len()] } else { openings::on_facades(b, ops, &fs, shape, sc, &cx, &mut o.geo)? };
    for ((f, _), c) in fs.iter().zip(cuts.iter()) {
        for w in facade::build(&mut o.geo, f, c) {
            b.warn(sc, w);
        }
    }
    if smooth {
        mesh::auto_normals(o.geo.get(wall), 40.0);
    }
    if let Some(tm) = top {
        mesh::horizontal_shape(o.geo.get(tm), shape, y1, true);
    }
    if bottom {
        mesh::horizontal_shape(o.geo.get(wall), shape, y0, false);
    }
    // Simple closed prism for collisions and plan metrics.
    let mut col = crate::geom::Prim::default();
    mesh::prism(&mut col, shape, y0, y1, true, true);
    for t in col.indices.chunks_exact(3) {
        o.col.push([col.positions[t[0] as usize], col.positions[t[1] as usize], col.positions[t[2] as usize]]);
    }
    Ok(())
}

/// Cornice, plinth, string courses, parapets, battlements, balustrade
/// and roof on a prism.
pub fn inline_details(b: &mut Builder, shape: &Shape, y0: f32, y1: f32, d: &Inline, sc: &Scope, o: &mut Out) -> Result<()> {
    let all = EdgeSel::default();
    if let Some(c) = d.plinth.get() {
        let size = c.size.unwrap_or(0.7);
        let prof = c.profile.clone().unwrap_or(ProfileSpec::Named("plinth".into()));
        let m = b.mat(c.material.as_deref(), sc.mat)?;
        details::moulding_on(b, shape, c.edges.as_ref().unwrap_or(&all), c.y.unwrap_or(y0), &prof, size, c.projection, 0.0, m, None, None, o)?;
    }
    for &y in d.string_courses {
        details::moulding_on(b, shape, &all, y, &ProfileSpec::Named("string".into()), 0.28, None, 0.0, sc.mat, None, None, o)?;
    }
    let has_top = d.parapet.get().is_some() || d.crenellations.get().is_some() || d.balustrade.get().is_some();
    if let Some(c) = d.cornice.get() {
        let size = c.size.unwrap_or(0.8);
        let prof = c.profile.clone().unwrap_or(ProfileSpec::Named("cornice".into()));
        let m = b.mat(c.material.as_deref(), sc.mat)?;
        let y = c.y.unwrap_or(y1 - size * if has_top { 0.85 } else { 1.0 });
        details::moulding_on(b, shape, c.edges.as_ref().unwrap_or(&all), y, &prof, size, c.projection, 0.0, m, c.modillions.get(), c.dentils.get(), o)?;
    }
    if let Some(p) = d.parapet.get() {
        let m = b.mat(p.material.as_deref(), sc.mat)?;
        details::parapet_on(b, shape, p.edges.as_ref().unwrap_or(&all), y1, p.height, p.thickness, p.coping, 0.0, m, o)?;
    }
    if let Some(c) = d.crenellations.get() {
        let m = b.mat(c.material.as_deref(), sc.mat)?;
        details::crenel_on(b, shape, c.edges.as_ref().unwrap_or(&all), y1, c, 0.0, m, o)?;
    }
    if let Some(bl) = d.balustrade.get() {
        let m = b.mat(bl.material.as_deref(), sc.mat)?;
        details::balustrade_on(b, shape, bl.edges.as_ref().unwrap_or(&all), y1, bl.height, bl.spacing, bl.pedestals, &bl.finials, 0.2, m, o)?;
    }
    if let Some(r) = d.roof.and_then(|r| r.get()) {
        let y = r.y.unwrap_or(y1);
        roofs::roof_shape(b, &roofs::RoofParams::from_spec(r), shape, y, sc, o)?;
    }
    Ok(())
}

pub fn mass(b: &mut Builder, m: &Mass, sc: &Scope, o: &mut Out) -> Result<()> {
    let shapes = match &m.polygon {
        Some(p) => b.poly(p, sc)?,
        None => vec![sc.parent.clone().ok_or_else(|| Error::msg("mass needs `polygon` (e.g. \"outline:0\")"))?],
    };
    let y1 = mass_top(b, m);
    if m.y1.is_none() && m.height.is_none() && m.floors.is_none() {
        b.warn(sc, "mass has no y1/height/floors; using 10 m");
    }
    let mut ops: Vec<&Openings> = m.openings.iter().collect();
    let extra = m.name.as_ref().and_then(|n| b.extra_openings.get(n)).cloned().unwrap_or_default();
    ops.extend(extra.iter());
    let top = if m.top && m.roof.get().is_none() { Some(b.mat(m.top_material.as_deref(), sc.mat)?) } else if m.top { Some(b.mat(m.top_material.as_deref(), sc.mat)?) } else { None };
    let bottom = m.bottom.unwrap_or(m.y0 > 0.05);
    for s in &shapes {
        let inner = Scope { parent: Some(s.clone()), ..sc.clone() };
        prism_with_facades(b, s, m.y0, y1, m.batter, m.smooth, top, bottom, &ops, &inner, o)?;
        let d = Inline { cornice: &m.cornice, plinth: &m.plinth, string_courses: &m.string_courses, parapet: &m.parapet, crenellations: &m.crenellations, balustrade: &m.balustrade, roof: Some(&m.roof) };
        let top_shape = if m.batter.abs() > 1e-4 { Shape { outer: poly::offset(&s.outer, -m.batter).unwrap_or(s.outer.clone()), holes: s.holes.clone() } } else { s.clone() };
        inline_details(b, &top_shape, m.y0, y1, &d, &inner, o)?;
    }
    Ok(())
}

pub fn slab(b: &mut Builder, s: &Slab, sc: &Scope, o: &mut Out) -> Result<()> {
    let shapes = match &s.polygon {
        Some(p) => b.poly(p, sc)?,
        None => vec![sc.parent.clone().ok_or_else(|| Error::msg("slab needs `polygon`"))?],
    };
    for sh in &shapes {
        mesh::prism(o.geo.get(sc.mat), sh, s.y - s.thickness.max(0.01), s.y, true, s.y - s.thickness > 0.02);
    }
    o.collide_all();
    Ok(())
}

/// Free-standing wall with through openings.
pub fn wall(b: &mut Builder, w: &Wall, sc: &Scope, o: &mut Out) -> Result<()> {
    let y0 = w.y0;
    let y1 = w.y1.or(w.height.map(|h| y0 + h)).ok_or_else(|| Error::msg("wall needs `y1` or `height`"))?;
    if y1 <= y0 {
        return Err(Error::msg("wall top must be above its base"));
    }
    let t = w.thickness.max(0.02);
    let mut paths: Vec<(Vec<Vec2>, bool)> = Vec::new();
    if let Some(p) = &w.path {
        if p.len() < 2 {
            return Err(Error::msg("wall path needs at least 2 points"));
        }
        paths.push((p.iter().map(|q| Vec2::new(q[0], q[1])).collect(), w.closed));
    } else if let Some(pg) = &w.polygon {
        for s in b.poly(pg, sc)? {
            let edges = resolve::select(&s, &w.edges);
            for c in resolve::chains(&s, &edges) {
                paths.push((resolve::chain_points(&s, c), c.3));
            }
        }
    } else {
        return Err(Error::msg("wall needs `path` or `polygon`"));
    }
    let align = match w.align.as_deref().unwrap_or("center") {
        "center" | "centre" => 0.0,
        "left" => -0.5,
        "right" => 0.5,
        o2 => return Err(Error::msg(format!("bad wall align `{o2}`; use center, left or right"))),
    };
    let ops: Vec<&Openings> = w.openings.iter().collect();
    for (path, closed) in paths {
        let offs = mesh::polyline_offsets_2d(&path, closed);
        // Right side (positive offset) is the front.
        let right: Vec<Vec2> = path.iter().zip(&offs).map(|(p, n)| *p + *n * t * (0.5 + align)).collect();
        let left: Vec<Vec2> = path.iter().zip(&offs).map(|(p, n)| *p - *n * t * (0.5 - align)).collect();
        let m = path.len();
        let segs = if closed { m } else { m - 1 };
        // Front facades along `right`, seen from the right side: they run
        // a -> b from the viewer's right to left, so build with the plane
        // origin at b.
        let mut front = Vec::new();
        let mut back = Vec::new();
        for i in 0..segs {
            let j = (i + 1) % m;
            let (a, bb) = (right[i], right[j]);
            let len = (bb - a).length();
            if len < 1e-4 {
                continue;
            }
            let u = Vec3::new(a.x - bb.x, 0.0, a.y - bb.y) / len;
            let plane = Plane::new(Vec3::new(bb.x, 0.0, bb.y), u, Vec3::Y);
            front.push((Facade { plane, len, v0: y0, v1: y1, uv_off: 0.0, mat: sc.mat, outline: None }, EdgeRef { ring: 0, i }));
            let (la, lb) = (left[i], left[j]);
            let llen = (lb - la).length();
            let ub = Vec3::new(lb.x - la.x, 0.0, lb.y - la.y) / llen.max(1e-6);
            let bplane = Plane::new(Vec3::new(la.x, 0.0, la.y), ub, Vec3::Y);
            back.push(Facade { plane: bplane, len: llen, v0: y0, v1: y1, uv_off: 0.0, mat: sc.mat, outline: None });
        }
        // Openings select edges of the path as if it were a polygon ring.
        let fake = Shape::new(right.clone());
        let cx = OpCtx { wall: sc.mat, trim: sc.mat, through: Some(t), room: 0.0 };
        let cuts = if ops.is_empty() { vec![Vec::new(); front.len()] } else { openings::on_facades(b, &ops, &front, &fake, sc, &cx, &mut o.geo)? };
        for (k, ((f, _), c)) in front.iter().zip(cuts.iter()).enumerate() {
            for wmsg in facade::build(&mut o.geo, f, c) {
                b.warn(sc, wmsg);
            }
            // Back face: same holes mirrored along the segment.
            let bf = &back[k];
            let mirrored: Vec<Vec<Vec2>> = c.iter().map(|cut| cut.outer().iter().rev().map(|p| Vec2::new(f.len - p.x, p.y)).collect()).collect();
            let scale = bf.len / f.len.max(1e-6);
            let mirrored: Vec<Vec<Vec2>> = mirrored.into_iter().map(|r| r.into_iter().map(|p| Vec2::new(p.x * scale, p.y)).collect()).collect();
            let (shape, warns) = facade::wall_shape(bf, &mirrored);
            for wmsg in warns {
                b.warn(sc, wmsg);
            }
            mesh::planar_shape(o.geo.get(sc.mat), &shape, &bf.plane, false, Vec2::ZERO);
        }
        // Top and ends.
        let top: Vec<Vec2> = right.iter().copied().chain(left.iter().rev().copied()).collect();
        if closed {
            let s = Shape { outer: right.clone(), holes: vec![left.clone()] };
            if let Some(s) = s.normalized() {
                mesh::horizontal_shape(o.geo.get(sc.mat), &s, y1, true);
            }
        } else if let Some(r) = poly::normalize(&top, true) {
            mesh::horizontal_shape(o.geo.get(sc.mat), &Shape::new(r), y1, true);
            for (p, q, inward) in [(right[0], left[0], path[1] - path[0]), (right[m - 1], left[m - 1], path[m - 2] - path[m - 1])] {
                let quad = [Vec3::new(p.x, y0, p.y), Vec3::new(q.x, y0, q.y), Vec3::new(q.x, y1, q.y), Vec3::new(p.x, y1, p.y)];
                mesh::quad_world(o.geo.get(sc.mat), quad, -Vec3::new(inward.x, 0.0, inward.y));
            }
        }
        if w.coping {
            let s = Shape::new(poly::normalize(&top, true).unwrap_or(top.clone()));
            if let Some(g) = poly::offset(&s.outer, 0.06) {
                mesh::prism(o.geo.get(sc.mat), &Shape::new(g), y1, y1 + 0.12, true, true);
            }
        }
        // Collider: plain slab.
        if let Some(r) = poly::normalize(&top, true) {
            let mut col = crate::geom::Prim::default();
            mesh::prism(&mut col, &Shape::new(r), y0, y1, true, true);
            for tr in col.indices.chunks_exact(3) {
                o.col.push([col.positions[tr[0] as usize], col.positions[tr[1] as usize], col.positions[tr[2] as usize]]);
            }
        }
    }
    Ok(())
}

/// Plans and height ranges of every stage.
pub fn tower_stage_plans(b: &Builder, t: &Tower, base: &Shape, sc: &Scope) -> Vec<(Shape, f32, f32)> {
    let center = poly::centroid(&base.outer);
    let mut cur = base.clone();
    let mut y = t.y;
    let mut out = Vec::new();
    for st in &t.stages {
        let shape = if let Some(p) = &st.polygon {
            let s2 = Scope { center, parent: Some(cur.clone()), ..sc.clone() };
            b.poly(p, &s2).ok().and_then(|v| v.into_iter().next()).unwrap_or(cur.clone())
        } else if st.inset.abs() > 1e-4 {
            match poly::offset(&cur.outer, -st.inset) {
                Some(r) => Shape::new(r),
                None => cur.clone(),
            }
        } else {
            cur.clone()
        };
        out.push((shape.clone(), y, y + st.height));
        y += st.height;
        cur = if st.batter.abs() > 1e-4 { poly::offset(&shape.outer, -st.batter).map(Shape::new).unwrap_or(shape) } else { shape };
    }
    out
}

pub fn tower(b: &mut Builder, t: &Tower, sc: &Scope, o: &mut Out) -> Result<()> {
    let base = match &t.polygon {
        Some(p) => b.poly(p, sc)?.into_iter().next().unwrap(),
        None => sc.parent.clone().ok_or_else(|| Error::msg("tower needs `polygon` (e.g. { shape = \"square\", center = [x, z], width = 8 })"))?,
    };
    if t.stages.is_empty() {
        return Err(Error::msg("tower needs at least one [[...stages]] entry"));
    }
    let plans = tower_stage_plans(b, t, &base, sc);
    let center = poly::centroid(&base.outer);
    let mut top_y = t.y;
    let mut top_shape = base.clone();
    for (k, (st, (shape, y0, y1))) in t.stages.iter().zip(plans.iter()).enumerate() {
        let mat = b.mat(st.material.as_deref(), sc.mat)?;
        let ctx = match &st.name {
            Some(n) => format!("{} > stage `{n}`", sc.ctx),
            None => format!("{} > stage #{k}", sc.ctx),
        };
        let ss = Scope { mat, parent: Some(shape.clone()), center, ctx, ..sc.clone() };
        let mut ops: Vec<&Openings> = st.openings.iter().collect();
        let belfry;
        if st.belfry {
            let min_face = (0..shape.outer.len()).map(|i| (shape.outer[(i + 1) % shape.outer.len()] - shape.outer[i]).length()).fold(f32::MAX, f32::min);
            let w = (min_face * 0.45).clamp(0.6, 3.2);
            let h = (st.height * 0.62).min(w * 2.4).max(w * 1.2);
            belfry = Openings {
                name: None,
                material: None,
                transform: None,
                mirror: None,
                enabled: true,
                collide: None,
                note: None,
                on: None,
                edges: EdgeSel::default(),
                kind: OpeningKind::Arch,
                arch: Some(ArchKind::Round),
                width: w,
                height: h,
                rise: None,
                levels: Some(Levels(vec![y0 + (st.height - h) * 0.45])),
                bays: Flex(Some(Bays { count: Some(if min_face > 3.0 * w + 1.0 { 2 } else { 1 }), ..Default::default() })),
                skip: Vec::new(),
                depth: 0.9,
                splay: 0.0,
                infill: Some(Infill::Dark),
                frame: 0.0,
                mullions: 0,
                transoms: 0,
                panes: None,
                surround: Flex(None),
                hood: Flex(None),
                sill: Flex(None),
                keystone: None,
                orders: 1,
                order_step: 0.12,
                colonnettes: false,
                tympanum: false,
                tracery: None,
                spokes: 0,
                rings: 0,
                balcony: Flex(None),
                voussoirs: Flex(None),
                bells: true,
                glass_material: None,
                frame_material: None,
                door_material: None,
                iron_material: None,
                stone_material: None,
            };
            ops.push(&belfry);
        }
        if let Some(n) = &st.name
            && let Some(extra) = b.extra_openings.get(n).cloned()
        {
            let extra: Vec<Openings> = extra;
            let mut tmp = Out::default();
            let refs: Vec<&Openings> = ops.iter().copied().chain(extra.iter()).collect();
            prism_with_facades(b, shape, *y0, *y1, st.batter, st.smooth, Some(mat), false, &refs, &ss, &mut tmp)?;
            o.append(&tmp, &Mat4::IDENTITY);
        } else {
            prism_with_facades(b, shape, *y0, *y1, st.batter, st.smooth, Some(mat), false, &ops, &ss, o)?;
        }
        let d = Inline { cornice: &st.cornice, plinth: &st.plinth, string_courses: &st.string_courses, parapet: &st.parapet, crenellations: &st.crenellations, balustrade: &st.balustrade, roof: None };
        let ts = if st.batter.abs() > 1e-4 { Shape::new(poly::offset(&shape.outer, -st.batter).unwrap_or(shape.outer.clone())) } else { shape.clone() };
        inline_details(b, &ts, *y0, *y1, &d, &ss, o)?;
        if let Some(p) = st.pinnacles.get() {
            let pm = b.mat(p.material.as_deref(), mat)?;
            let c = poly::centroid(&ts.outer);
            for q in &ts.outer {
                let at = *q + (c - *q).normalize_or(Vec2::ZERO) * (p.width * 0.75);
                let mut g = Out::default();
                super::supports::pinnacle_geo(b, pm, p.width, p.height, p.sides, p.crockets, 0.45, &mut g);
                o.append(&g, &Mat4::from_translation(Vec3::new(at.x, *y1, at.y)));
            }
        }
        top_y = *y1;
        top_shape = ts;
    }
    // Top elements in a frame at the top centre.
    if !t.top.0.is_empty() {
        let c = poly::centroid(&top_shape.outer);
        let local_shape = top_shape.map(|p| p - c);
        let local = Mat4::from_translation(Vec3::new(c.x, top_y, c.y));
        let ts = Scope { ctx: format!("{} > top", sc.ctx), ..sc.clone() };
        b.build_children(&t.top.0, &ts, local, Some(local_shape), o)?;
    }
    Ok(())
}
