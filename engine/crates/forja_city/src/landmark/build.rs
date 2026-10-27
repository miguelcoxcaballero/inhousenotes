//! Spec → geometry. Elements are built in their own frame into a local
//! [`Out`], then placed with their transform (and mirrored copies) in the
//! parent's frame; everything is merged per material.

use std::collections::HashMap;

use forja_math::{EulerRot, Mat4, Quat, Vec2, Vec3};

use super::footprint::Footprint;
use super::mesh::{self, Geo, MatId};
use super::palette::Palette;
use super::parse::{Error, Result, line_col};
use super::poly::{self, Shape};
use super::resolve::{self, PolyCtx};
use super::spec::*;
use super::{details, domes, generic, masses, roofs, supports};

/// Geometry plus collision triangles of one element (or a subtree).
#[derive(Clone, Default)]
pub struct Out {
    pub geo: Geo,
    pub col: Vec<[Vec3; 3]>,
}

impl Out {
    pub fn append(&mut self, other: &Out, m: &Mat4) {
        self.geo.append(&other.geo, m);
        // Mirrors and negative scales flip winding: re-wind so the
        // collider stays outward (as `Prim::append` does for the mesh).
        let flip = m.determinant() < 0.0;
        self.col.extend(other.col.iter().map(|t| {
            let (a, b, c) = (m.transform_point3(t[0]), m.transform_point3(t[1]), m.transform_point3(t[2]));
            if flip { [a, c, b] } else { [a, b, c] }
        }));
    }

    pub fn tris(&self) -> usize {
        self.geo.tri_count()
    }

    /// Copies all triangles of `geo` into the collider.
    pub fn collide_all(&mut self) {
        for p in self.geo.prims.values() {
            for t in p.indices.chunks_exact(3) {
                self.col.push([p.positions[t[0] as usize], p.positions[t[1] as usize], p.positions[t[2] as usize]]);
            }
        }
    }
}

/// Per-element build statistics.
#[derive(Clone, Debug)]
pub struct ElemStat {
    pub name: String,
    pub kind: &'static str,
    pub min: Vec3,
    pub max: Vec3,
    pub triangles: usize,
    pub line: Option<usize>,
}

/// Plan and height range of a named element, for `on = "..."`.
#[derive(Clone, Debug)]
pub struct Named {
    pub shapes: Vec<Shape>,
    pub y0: f32,
    pub y1: f32,
}

/// Inherited context while building nested elements.
#[derive(Clone)]
pub struct Scope {
    pub mat: MatId,
    pub parent: Option<Shape>,
    pub center: Vec2,
    /// Element frame → landmark-local frame.
    pub xf: Mat4,
    /// Error context ("element `nave` (mass)").
    pub ctx: String,
}

pub struct Builder<'a> {
    pub spec: &'a Spec,
    pub fp: Option<&'a Footprint>,
    pub pal: Palette,
    pub named: HashMap<String, Named>,
    /// Standalone `openings` with `on`, keyed by target name.
    pub extra_openings: HashMap<String, Vec<Openings>>,
    pub warnings: Vec<String>,
    pub stats: Vec<ElemStat>,
    pub detail: f32,
    pub default_mat: MatId,
    /// Collider triangle ranges of each top-level element.
    pub col_groups: Vec<(usize, usize)>,
    /// Triangles already built by finished top-level elements.
    pub built_tris: usize,
    depth: u32,
    line: Option<usize>,
}

/// Hard limit on the triangles of one landmark (the cathedral needs about
/// 400k; anything near this is a runaway array or resolution).
pub const MAX_TRIANGLES: usize = 2_000_000;
/// Hard limit on array instances (after nesting is multiplied out).
pub const MAX_INSTANCES: usize = 20_000;

pub fn transform_matrix(t: &Transform) -> Mat4 {
    let p = Vec3::from_array(t.pivot);
    let r = Quat::from_rotation_y(t.rotate_y.to_radians()) * Quat::from_euler(EulerRot::XYZ, t.rotate[0].to_radians(), t.rotate[1].to_radians(), t.rotate[2].to_radians());
    Mat4::from_translation(Vec3::from_array(t.translate) + p) * Mat4::from_quat(r) * Mat4::from_scale(Vec3::from_array(t.scale.0)) * Mat4::from_translation(-p)
}

pub fn mirror_matrices(m: &Mirror) -> Vec<Mat4> {
    let mx = Mat4::from_translation(Vec3::new(m.at[0], 0.0, 0.0)) * Mat4::from_scale(Vec3::new(-1.0, 1.0, 1.0)) * Mat4::from_translation(Vec3::new(-m.at[0], 0.0, 0.0));
    let mz = Mat4::from_translation(Vec3::new(0.0, 0.0, m.at[1])) * Mat4::from_scale(Vec3::new(1.0, 1.0, -1.0)) * Mat4::from_translation(Vec3::new(0.0, 0.0, -m.at[1]));
    match (m.x, m.z) {
        (true, false) => vec![mx],
        (false, true) => vec![mz],
        (true, true) => vec![mx, mz, mx * mz],
        _ => vec![],
    }
}

/// Rotation turning the local +z (facade outward) into a compass facing.
pub fn facing_matrix(at: [f32; 3], facing: Option<Facing>) -> Mat4 {
    let az = facing.map(|f| f.0).unwrap_or(180.0);
    let d = poly::from_azimuth(az);
    // Local frame: x right along the wall, y up, z = facing direction.
    let z = Vec3::new(d.x, 0.0, d.y);
    let x = Vec3::Y.cross(z);
    Mat4::from_cols(x.extend(0.0), Vec3::Y.extend(0.0), z.extend(0.0), Vec3::from_array(at).extend(1.0))
}

fn default_collide(e: &Element) -> bool {
    matches!(
        e,
        Element::Mass(_)
            | Element::Slab(_)
            | Element::Wall(_)
            | Element::Roof(_)
            | Element::Dome(_)
            | Element::Tower(_)
            | Element::Column(_)
            | Element::Buttress(_)
            | Element::FlyingButtress(_)
            | Element::Stairs(_)
            | Element::Arcade(_)
            | Element::Box(_)
            | Element::Cylinder(..)
            | Element::Sphere(_)
            | Element::Lathe(_)
            | Element::Extrude(_)
            | Element::Loft(_)
            | Element::Shell(_)
            | Element::Mesh(_)
            | Element::Parapet(_)
            | Element::Group(_)
            | Element::Array(_)
            | Element::Instance(_)
    )
}

impl<'a> Builder<'a> {
    pub fn new(spec: &'a Spec, fp: Option<&'a Footprint>) -> Result<Self> {
        let mut pal = Palette::new(&spec.material).map_err(Error::msg)?;
        let dm = spec.defaults.material.clone().unwrap_or_else(|| "stone".into());
        let default_mat = pal.id(&dm).map_err(|e| Error::msg(format!("[defaults] material: {e}")))?;
        Ok(Builder {
            spec,
            fp,
            pal,
            named: HashMap::new(),
            extra_openings: HashMap::new(),
            warnings: Vec::new(),
            stats: Vec::new(),
            detail: spec.defaults.detail.unwrap_or(1.0).clamp(0.25, 4.0),
            default_mat,
            col_groups: Vec::new(),
            built_tris: 0,
            depth: 0,
            line: None,
        })
    }

    pub fn warn(&mut self, sc: &Scope, msg: impl AsRef<str>) {
        let w = format!("{}: {}", sc.ctx, msg.as_ref());
        if !self.warnings.contains(&w) {
            self.warnings.push(w);
        }
    }

    pub fn mat(&mut self, name: Option<&str>, fallback: MatId) -> Result<MatId> {
        match name {
            Some(n) => self.pal.id(n).map_err(Error::msg),
            None => Ok(fallback),
        }
    }

    /// Material by name with a default name (e.g. glass) when unset.
    pub fn mat_or(&mut self, name: Option<&str>, default_name: &str) -> Result<MatId> {
        let n = name.unwrap_or(default_name);
        self.pal.id(n).map_err(Error::msg)
    }

    pub fn roof_mat(&mut self, name: Option<&str>) -> Result<MatId> {
        let d = self.spec.defaults.roof_material.clone().unwrap_or_else(|| "roof".into());
        self.mat_or(name, &d)
    }

    pub fn glass_mat(&mut self, name: Option<&str>) -> Result<MatId> {
        let d = self.spec.defaults.glass_material.clone().unwrap_or_else(|| "glass".into());
        self.mat_or(name, &d)
    }

    pub fn frame_mat(&mut self, name: Option<&str>) -> Result<MatId> {
        let d = self.spec.defaults.frame_material.clone().unwrap_or_else(|| "frame".into());
        self.mat_or(name, &d)
    }

    pub fn door_mat(&mut self, name: Option<&str>) -> Result<MatId> {
        let d = self.spec.defaults.door_material.clone().unwrap_or_else(|| "door".into());
        self.mat_or(name, &d)
    }

    pub fn iron_mat(&mut self, name: Option<&str>) -> Result<MatId> {
        let d = self.spec.defaults.iron_material.clone().unwrap_or_else(|| "iron".into());
        self.mat_or(name, &d)
    }

    pub fn floor_height(&self) -> f32 {
        self.spec.defaults.floor_height.unwrap_or(3.2)
    }

    /// Curve segments for a circle of radius `r`.
    pub fn segs(&self, r: f32, min: usize, max: usize) -> usize {
        (((r.abs() * 8.0 + 8.0) * self.detail) as usize).clamp(min, max)
    }

    /// A user-given resolution, checked against `min..=max`.
    pub fn user_segs(&self, v: Option<u32>, auto: usize, min: usize, max: usize, what: &str) -> Result<usize> {
        match v {
            None => Ok(auto.clamp(min, max)),
            Some(n) if (min..=max).contains(&(n as usize)) => Ok(n as usize),
            Some(n) => Err(Error::msg(format!("{what} = {n} is out of range ({min}..={max})"))),
        }
    }

    /// Fails when `extra` more triangles would pass [`MAX_TRIANGLES`].
    pub fn budget(&self, extra: usize, what: &str) -> Result<()> {
        let total = self.built_tris.saturating_add(extra);
        if total > MAX_TRIANGLES {
            return Err(Error::msg(format!("{what} would make the landmark {total} triangles (limit {MAX_TRIANGLES}); lower counts or segments")));
        }
        Ok(())
    }

    pub fn poly(&self, spec: &PolySpec, sc: &Scope) -> Result<Vec<Shape>> {
        let ctx = PolyCtx { fp: self.fp, parent: sc.parent.as_ref(), center: sc.center };
        resolve::resolve(spec, &ctx, self.detail).map_err(Error::msg)
    }

    /// Polygon from `polygon`, else `on`, else the scope's parent.
    pub fn poly_or_on(&self, polygon: Option<&PolySpec>, on: Option<&str>, sc: &Scope) -> Result<(Vec<Shape>, Option<(f32, f32)>)> {
        if let Some(p) = polygon {
            return Ok((self.poly(p, sc)?, on.and_then(|n| self.named.get(n)).map(|n| (n.y0, n.y1))));
        }
        if let Some(n) = on {
            let named = self.named.get(n).ok_or_else(|| {
                let names: Vec<&str> = self.named.keys().map(|s| s.as_str()).collect();
                Error::msg(format!("`on = \"{n}\"`: no named mass, tower stage, roof or slab called `{n}`{}", super::parse::did_you_mean(n, &names)))
            })?;
            return Ok((named.shapes.clone(), Some((named.y0, named.y1))));
        }
        if let Some(p) = &sc.parent {
            return Ok((vec![p.clone()], None));
        }
        Err(Error::msg("needs `polygon` or `on`"))
    }

    fn register(&mut self, name: &str, shapes: Vec<Shape>, y0: f32, y1: f32) {
        self.named.insert(name.to_string(), Named { shapes, y0, y1 });
    }

    /// First pass: names of masses, stages, roofs and slabs for `on`, and
    /// standalone openings grouped by target.
    pub fn prepass(&mut self, list: &[Spanned<Element>]) {
        let sc = self.root_scope();
        for e in list {
            self.prepass_one(e.get_ref(), &sc);
        }
    }

    fn prepass_one(&mut self, e: &Element, sc: &Scope) {
        match e {
            Element::Mass(m) => {
                if let Some(n) = &m.name
                    && let Some(p) = &m.polygon
                    && let Ok(shapes) = self.poly(p, sc)
                {
                    let y1 = masses::mass_top(self, m);
                    self.register(n, shapes, m.y0, y1);
                }
            }
            Element::Slab(s) => {
                if let (Some(n), Some(p)) = (&s.name, &s.polygon)
                    && let Ok(shapes) = self.poly(p, sc)
                {
                    self.register(n, shapes, s.y - s.thickness, s.y);
                }
            }
            Element::Roof(r) => {
                if let (Some(n), Some(p)) = (&r.name, &r.polygon)
                    && let Ok(shapes) = self.poly(p, sc)
                {
                    let y = r.y.unwrap_or(0.0);
                    self.register(n, shapes, y, y);
                }
            }
            Element::Tower(t) => {
                if let Some(p) = &t.polygon
                    && let Ok(shapes) = self.poly(p, sc)
                {
                    let stages = masses::tower_stage_plans(self, t, &shapes[0], sc).unwrap_or_default();
                    let mut top = t.y;
                    for (st, (shape, y0, y1)) in t.stages.iter().zip(stages) {
                        if let Some(n) = &st.name {
                            self.register(n, vec![shape.clone()], y0, y1);
                        }
                        top = y1;
                    }
                    if let Some(n) = &t.name {
                        self.register(n, shapes, t.y, top);
                    }
                }
            }
            Element::Openings(o) => {
                if let Some(target) = &o.on {
                    self.extra_openings.entry(target.clone()).or_default().push(o.clone());
                }
            }
            _ => {}
        }
    }

    pub fn root_scope(&self) -> Scope {
        Scope { mat: self.default_mat, parent: None, center: Vec2::ZERO, xf: Mat4::IDENTITY, ctx: "landmark".into() }
    }

    /// Builds a top-level element list.
    pub fn build_list(&mut self, list: &[Spanned<Element>], text: &str, origin: &str, out: &mut Out) -> Result<()> {
        let sc = self.root_scope();
        for (k, e) in list.iter().enumerate() {
            let start = out.col.len();
            let span = e.span();
            let (line, col) = line_col(text, span.start);
            self.line = Some(line);
            let el = e.get_ref();
            let label = el.common().name.map(|n| format!("element `{n}` ({})", el.type_name())).unwrap_or_else(|| format!("element #{k} ({})", el.type_name()));
            let scope = Scope { ctx: format!("{origin}:{line} {label}"), ..sc.clone() };
            let snippet = super::parse::snippet(text, span.start, text[span.start..].lines().next().map(|l| l.chars().count()).unwrap_or(1));
            self.build_into(el, &scope, out).map_err(|err| err.context(&label).at(format!("{origin}:{line}:{col}"), Some(snippet)))?;
            if out.col.len() > start {
                self.col_groups.push((start, out.col.len()));
            }
            self.built_tris = out.tris();
        }
        self.line = None;
        Ok(())
    }

    /// Builds `e` (in `sc`'s frame) into `out`.
    pub fn build_into(&mut self, e: &Element, sc: &Scope, out: &mut Out) -> Result<()> {
        let c = e.common();
        if !c.enabled {
            return Ok(());
        }
        if self.depth > 24 {
            return Err(Error::msg("elements nested too deeply (recursive component?)"));
        }
        let mat = self.mat(c.material, sc.mat)?;
        let local = c.transform.map(transform_matrix).unwrap_or(Mat4::IDENTITY);
        let ctx = match c.name {
            Some(n) if !sc.ctx.contains(n) => format!("{} > `{n}` ({})", sc.ctx, e.type_name()),
            _ => sc.ctx.clone(),
        };
        let scope = Scope { mat, xf: sc.xf * local, ctx, ..sc.clone() };
        let mut o = Out::default();
        self.depth += 1;
        let r = self.dispatch(e, &scope, &mut o);
        self.depth -= 1;
        r?;
        let mirrors_n = c.mirror.map(mirror_matrices).map(|m| m.len()).unwrap_or(0);
        self.budget(o.tris().saturating_mul(1 + mirrors_n), &format!("{} ({})", c.name.unwrap_or("element"), e.type_name()))?;
        if c.collide.unwrap_or_else(|| default_collide(e)) {
            if o.col.is_empty() && !matches!(e, Element::Group(_) | Element::Array(_) | Element::Instance(_)) {
                o.collide_all();
            }
        } else {
            o.col.clear();
        }
        // Statistics in the landmark frame.
        let mirrors = c.mirror.map(mirror_matrices).unwrap_or_default();
        if let Some((mn, mx)) = mesh::bounds(&o.geo) {
            let mut lo = Vec3::splat(f32::MAX);
            let mut hi = Vec3::splat(f32::MIN);
            for m in std::iter::once(Mat4::IDENTITY).chain(mirrors.iter().copied()) {
                let w = sc.xf * m * local;
                for k in 0..8 {
                    let p = Vec3::new(if k & 1 == 0 { mn.x } else { mx.x }, if k & 2 == 0 { mn.y } else { mx.y }, if k & 4 == 0 { mn.z } else { mx.z });
                    let q = w.transform_point3(p);
                    lo = lo.min(q);
                    hi = hi.max(q);
                }
            }
            let tris = mesh::tris(&o.geo) * (1 + mirrors.len());
            let name = c.name.map(|s| s.to_string());
            if let Some(n) = name {
                self.stats.push(ElemStat { name: n, kind: e.type_name(), min: lo, max: hi, triangles: tris, line: self.line });
            } else if self.depth == 0 {
                self.stats.push(ElemStat { name: String::new(), kind: e.type_name(), min: lo, max: hi, triangles: tris, line: self.line });
            }
        }
        out.append(&o, &local);
        for m in mirrors {
            out.append(&o, &(m * local));
        }
        Ok(())
    }

    /// Builds child elements placed with `local` (relative to `sc`).
    pub fn build_children(&mut self, list: &[Element], sc: &Scope, local: Mat4, parent: Option<Shape>, out: &mut Out) -> Result<()> {
        if list.is_empty() {
            return Ok(());
        }
        let child = Scope { xf: sc.xf * local, parent: parent.or_else(|| sc.parent.clone()), center: Vec2::ZERO, ..sc.clone() };
        let mut o = Out::default();
        for (k, e) in list.iter().enumerate() {
            let s = Scope { ctx: format!("{} > child #{k} ({})", sc.ctx, e.type_name()), ..child.clone() };
            self.build_into(e, &s, &mut o)?;
        }
        out.append(&o, &local);
        Ok(())
    }

    fn component(&mut self, name: &str, sc: &Scope, local: Mat4, out: &mut Out) -> Result<()> {
        let comp = self.spec.component.get(name).cloned().ok_or_else(|| {
            let names: Vec<&str> = self.spec.component.keys().map(|s| s.as_str()).collect();
            Error::msg(format!("unknown component `{name}`{}; define [component.{name}]", super::parse::did_you_mean(name, &names)))
        })?;
        let mat = self.mat(comp.material.as_deref(), sc.mat)?;
        let s = Scope { mat, ctx: format!("{} > component `{name}`", sc.ctx), ..sc.clone() };
        self.build_children(&comp.children, &s, local, None, out)
    }

    fn dispatch(&mut self, e: &Element, sc: &Scope, o: &mut Out) -> Result<()> {
        match e {
            Element::Mass(m) => masses::mass(self, m, sc, o),
            Element::Slab(s) => masses::slab(self, s, sc, o),
            Element::Wall(w) => masses::wall(self, w, sc, o),
            Element::Tower(t) => masses::tower(self, t, sc, o),
            Element::Openings(op) => {
                if op.on.is_none() {
                    self.warn(sc, "standalone `openings` needs `on = \"<mass name>\"`; ignored");
                } else if !self.named.contains_key(op.on.as_deref().unwrap()) {
                    let n = op.on.clone().unwrap();
                    let names: Vec<&str> = self.named.keys().map(|s| s.as_str()).collect();
                    return Err(Error::msg(format!("openings `on = \"{n}\"`: no named mass{}", super::parse::did_you_mean(&n, &names))));
                }
                Ok(())
            }
            Element::Moulding(m, k) => details::moulding(self, m, *k, sc, o),
            Element::Parapet(p) => details::parapet(self, p, sc, o),
            Element::Balustrade(b) => details::balustrade_el(self, b, sc, o),
            Element::Crenellations(c) => details::crenellations(self, c, sc, o),
            Element::Machicolations(m) => details::machicolations(self, m, sc, o),
            Element::Gargoyles(g) => details::gargoyles(self, g, sc, o),
            Element::Roof(r) => roofs::roof(self, r, sc, o),
            Element::Dome(d) => domes::dome(self, d, sc, o),
            Element::Lantern(l) => domes::lantern(self, l, sc, o),
            Element::Column(c) => supports::column_el(self, c, sc, o),
            Element::Pilaster(p) => supports::pilaster(self, p, sc, o),
            Element::Buttress(b) => supports::buttress(self, b, sc, o),
            Element::FlyingButtress(f) => supports::flying_buttress(self, f, sc, o),
            Element::Pinnacle(p) => supports::pinnacle_el(self, p, sc, o),
            Element::Finial(f) => supports::finial_el(self, f, sc, o),
            Element::Statue(s) => supports::statue(self, s, sc, o),
            Element::Stairs(s) => supports::stairs(self, s, sc, o),
            Element::Arcade(a) => supports::arcade(self, a, sc, o),
            Element::Box(b) => generic::box_el(self, b, sc, o),
            Element::Cylinder(c, cone) => generic::cylinder(self, c, *cone, sc, o),
            Element::Sphere(s) => generic::sphere(self, s, sc, o),
            Element::Torus(t) => generic::torus(self, t, sc, o),
            Element::Lathe(l) => generic::lathe(self, l, sc, o),
            Element::Extrude(x) => generic::extrude(self, x, sc, o),
            Element::Sweep(s) => generic::sweep(self, s, sc, o),
            Element::Loft(l) => generic::loft(self, l, sc, o),
            Element::Shell(s) => generic::shell(self, s, sc, o),
            Element::Mesh(m) => generic::mesh_el(self, m, sc, o),
            Element::Group(g) => {
                self.build_children(&g.children, sc, Mat4::IDENTITY, None, o)?;
                if let Some(c) = &g.component {
                    self.component(c, sc, Mat4::IDENTITY, o)?;
                }
                Ok(())
            }
            Element::Instance(i) => {
                let m = if i.at.is_some() || i.facing.is_some() { facing_matrix(i.at.unwrap_or([0.0; 3]), i.facing.or(Some(Facing(180.0)))) } else { Mat4::IDENTITY };
                self.component(&i.component, sc, m, o)
            }
            Element::Array(a) => self.array(a, sc, o),
        }
    }

    fn array(&mut self, a: &Array, sc: &Scope, o: &mut Out) -> Result<()> {
        if a.children.is_empty() && a.component.is_none() {
            return Err(Error::msg("array needs `children` or `component`"));
        }
        let mats = self.array_placements(a, sc)?;
        // Build the item once, then place copies.
        let mut item = Out::default();
        let item_scope = Scope { xf: sc.xf, ..sc.clone() };
        self.build_children(&a.children, &item_scope, Mat4::IDENTITY, None, &mut item)?;
        if let Some(c) = &a.component {
            self.component(c, &item_scope, Mat4::IDENTITY, &mut item)?;
        }
        let n = mats.len();
        self.budget(item.tris().saturating_mul(n), &format!("array of {n} x {} triangles", item.tris()))?;
        // Without `rotate`, copies keep their orientation and only move to
        // where the layout would carry their reference point (the item's
        // centre for radial arrays around a centre, else its origin).
        let pivot = if a.mode == ArrayMode::Radial && a.radius.is_none() { mesh::bounds(&item.geo).map(|(mn, mx)| (mn + mx) * 0.5).unwrap_or(Vec3::ZERO) } else { Vec3::ZERO };
        let rotate = a.rotate || matches!(a.mode, ArrayMode::Linear | ArrayMode::Grid | ArrayMode::Points);
        let scale_end = a.scale_end.as_ref().map(|s| Vec3::from_array(s.0));
        for (k, m) in mats.iter().enumerate() {
            if a.skip.iter().any(|s| s.rem_euclid(n as i64) as usize == k) {
                continue;
            }
            let base = if rotate { *m } else { Mat4::from_translation(m.transform_point3(pivot) - pivot) };
            // Per-instance ramps in the instance frame.
            let f = if n > 1 { k as f32 / (n - 1) as f32 } else { 0.0 };
            let mut local = Mat4::IDENTITY;
            if let Some(off) = a.offset_step {
                local *= Mat4::from_translation(Vec3::from_array(off) * k as f32);
            }
            if a.rotate_step != 0.0 {
                local *= Mat4::from_rotation_y((a.rotate_step * k as f32).to_radians());
            }
            if let Some(se) = scale_end {
                local *= Mat4::from_scale(Vec3::ONE.lerp(se, f));
            }
            o.append(&item, &(base * local));
        }
        Ok(())
    }

    /// Instance transforms of an array.
    pub fn array_placements(&mut self, a: &Array, sc: &Scope) -> Result<Vec<Mat4>> {
        let mut v = Vec::new();
        match a.mode {
            ArrayMode::Linear => {
                let n = a.count.ok_or_else(|| Error::msg("linear array needs `count`"))?;
                check_instances(n as u64, "linear array `count`")?;
                let step = Vec3::from_array(a.step.ok_or_else(|| Error::msg("linear array needs `step = [dx, dy, dz]`"))?);
                for k in 0..n {
                    v.push(Mat4::from_translation(step * k as f32));
                }
            }
            ArrayMode::Grid => {
                let counts = a.counts.clone().ok_or_else(|| Error::msg("grid array needs `counts = [nx, nz]` or [nx, ny, nz]"))?;
                let steps = a.steps.clone().ok_or_else(|| Error::msg("grid array needs `steps` matching `counts`"))?;
                if counts.len() != steps.len() || !(2..=3).contains(&counts.len()) {
                    return Err(Error::msg("grid `counts` and `steps` need 2 or 3 matching entries"));
                }
                let (nx, ny, nz, sx, sy, sz) = if counts.len() == 2 { (counts[0], 1, counts[1], steps[0], 0.0, steps[1]) } else { (counts[0], counts[1], counts[2], steps[0], steps[1], steps[2]) };
                check_instances(nx as u64 * ny as u64 * nz as u64, "grid array `counts`")?;
                for i in 0..nx {
                    for j in 0..ny {
                        for k in 0..nz {
                            v.push(Mat4::from_translation(Vec3::new(i as f32 * sx, j as f32 * sy, k as f32 * sz)));
                        }
                    }
                }
            }
            ArrayMode::Radial => {
                let n = a.count.ok_or_else(|| Error::msg("radial array needs `count`"))?.max(1);
                check_instances(n as u64, "radial array `count`")?;
                let c = Vec3::from_array(a.center.unwrap_or([0.0; 3]));
                let axis = Vec3::from_array(a.axis.unwrap_or([0.0, 1.0, 0.0])).normalize_or(Vec3::Y);
                let sweep = a.sweep.unwrap_or(360.0);
                let full = (sweep.abs() - 360.0).abs() < 1e-3;
                let div = if full { n as f32 } else { (n.max(2) - 1) as f32 };
                for k in 0..n {
                    let ang = (a.start + sweep * k as f32 / div).to_radians();
                    let q = Quat::from_axis_angle(axis, ang);
                    let mut m = Mat4::from_translation(c) * Mat4::from_quat(q);
                    if let Some(r) = a.radius {
                        // Offset along the local +z (then rotated).
                        m *= Mat4::from_translation(Vec3::new(0.0, 0.0, r));
                    }
                    if a.radius.is_none() {
                        m *= Mat4::from_translation(-c);
                    }
                    v.push(m);
                }
            }
            ArrayMode::Edges => {
                let (shapes, yr) = self.poly_or_on(a.polygon.as_ref(), a.on.as_deref(), sc)?;
                let y = a.y.or(yr.map(|r| r.0)).unwrap_or(0.0);
                let corners = a.corners.unwrap_or(CornerMode::Skip);
                for s in &shapes {
                    for e in resolve::select(s, &a.edges) {
                        let ring = resolve::ring_of(s, e.ring);
                        let n = ring.len();
                        let (p0, p1) = (ring[e.i], ring[(e.i + 1) % n]);
                        let len = (p1 - p0).length();
                        let nrm = poly::edge_normal(ring, e.i);
                        let frame = |p: Vec2, out: Vec2| {
                            let z = Vec3::new(out.x, 0.0, out.y);
                            let x = Vec3::Y.cross(z);
                            Mat4::from_cols(x.extend(0.0), Vec3::Y.extend(0.0), z.extend(0.0), Vec3::new(p.x + out.x * a.offset, y, p.y + out.y * a.offset).extend(1.0))
                        };
                        if corners != CornerMode::Skip {
                            // Corner at the edge start, facing the mitre.
                            let prev = poly::edge_normal(ring, (e.i + n - 1) % n);
                            let mitre = (prev + nrm).normalize_or(nrm);
                            let convex = (ring[e.i] - ring[(e.i + n - 1) % n]).perp_dot(p1 - p0) >= 0.0;
                            if convex || corners == CornerMode::Only {
                                v.push(frame(p0, mitre));
                            }
                        }
                        if corners == CornerMode::Only {
                            continue;
                        }
                        let usable = len - 2.0 * a.margin;
                        if usable <= 0.0 {
                            continue;
                        }
                        let count = match (a.count, a.spacing) {
                            (Some(c), _) => c as usize,
                            (None, Some(sp)) => ((usable / sp.max(0.05)).floor() as usize + 1).max(1),
                            (None, None) => return Err(Error::msg("edge array needs `spacing` or `count`")),
                        };
                        check_instances((v.len() + count) as u64, "edge array")?;
                        let dir = (p1 - p0) / len;
                        for k in 0..count {
                            let t = if count == 1 { 0.5 } else if corners == CornerMode::Include && a.margin == 0.0 { (k as f32 + 0.5) / count as f32 } else { k as f32 / (count - 1) as f32 };
                            let t = if count > 1 && !(corners == CornerMode::Include && a.margin == 0.0) { t } else { t };
                            let p = p0 + dir * (a.margin + usable * t);
                            v.push(frame(p, nrm));
                        }
                    }
                }
            }
            ArrayMode::Path => {
                let path = a.path.as_ref().ok_or_else(|| Error::msg("path array needs `path`"))?;
                let (pts, closed) = super::shapes::path_points(path, self.detail).map_err(Error::msg)?;
                let spacing = a.spacing;
                let count = a.count;
                let mut acc = vec![0.0f32];
                let m = pts.len();
                let segs = if closed { m } else { m - 1 };
                for i in 0..segs {
                    let d = acc[i] + (pts[(i + 1) % m] - pts[i]).length();
                    acc.push(d);
                }
                let total = *acc.last().unwrap();
                let n = match (count, spacing) {
                    (Some(c), _) => c as usize,
                    (None, Some(s)) => (total / s.max(0.05)).floor() as usize + if closed { 0 } else { 1 },
                    _ => return Err(Error::msg("path array needs `spacing` or `count`")),
                };
                check_instances(n as u64, "path array")?;
                for k in 0..n {
                    let d = if closed { total * k as f32 / n as f32 } else if n > 1 { total * k as f32 / (n - 1) as f32 } else { 0.0 };
                    let mut j = 0;
                    while j + 1 < segs && acc[j + 1] < d {
                        j += 1;
                    }
                    let (a0, a1) = (pts[j], pts[(j + 1) % m]);
                    let t = ((d - acc[j]) / (acc[j + 1] - acc[j]).max(1e-6)).clamp(0.0, 1.0);
                    let p = a0.lerp(a1, t);
                    let tan = (a1 - a0).normalize_or(Vec3::X);
                    let mut mm = Mat4::from_translation(p);
                    if a.rotate {
                        // Local +x along the path, +y up (as far as possible).
                        let x = tan;
                        let z = x.cross(Vec3::Y).normalize_or(Vec3::Z);
                        let y = z.cross(x);
                        mm = Mat4::from_cols(x.extend(0.0), y.extend(0.0), z.extend(0.0), p.extend(1.0));
                    }
                    v.push(mm);
                }
            }
            ArrayMode::Points => {
                let pts = a.points.as_ref().ok_or_else(|| Error::msg("points array needs `points`"))?;
                check_instances(pts.len() as u64, "points array")?;
                for (k, p) in pts.iter().enumerate() {
                    let f = a.facings.as_ref().and_then(|f| f.get(k)).copied();
                    v.push(match f {
                        Some(f) => facing_matrix(*p, Some(Facing(f))),
                        None => Mat4::from_translation(Vec3::from_array(*p)),
                    });
                }
            }
        }
        Ok(v)
    }
}

fn check_instances(n: u64, what: &str) -> Result<()> {
    if n > MAX_INSTANCES as u64 {
        return Err(Error::msg(format!("{what} makes {n} instances (max {MAX_INSTANCES})")));
    }
    Ok(())
}

/// Built geometry of a whole landmark (before materials).
pub struct Model {
    pub geo: Geo,
    pub collider: Vec<[Vec3; 3]>,
    /// Collider ranges per top-level element (closed solids for slicing).
    pub col_groups: Vec<(usize, usize)>,
    pub palette: Palette,
    pub stats: Vec<ElemStat>,
    pub warnings: Vec<String>,
}

impl Model {
    /// Material id of a palette name used by the model.
    pub fn palette_id(&self, name: &str) -> Option<MatId> {
        self.palette.find(name)
    }
}

/// Builds every element of `spec`.
pub fn build(spec: &Spec, fp: Option<&Footprint>, text: &str, origin: &str) -> Result<Model> {
    let mut b = Builder::new(spec, fp)?;
    b.prepass(&spec.element);
    let mut out = Out::default();
    b.build_list(&spec.element, text, origin, &mut out)?;
    let mut dropped = 0;
    for p in out.geo.prims.values_mut() {
        dropped += mesh::sanitize(p);
    }
    if dropped > 0 {
        log::debug!("dropped {dropped} degenerate triangles");
    }
    let mut warnings = b.warnings;
    if let Some(fp) = fp {
        warnings.extend(fp.warnings.iter().cloned());
    }
    Ok(Model { geo: out.geo, collider: out.col, col_groups: b.col_groups, palette: b.pal, stats: b.stats, warnings })
}
