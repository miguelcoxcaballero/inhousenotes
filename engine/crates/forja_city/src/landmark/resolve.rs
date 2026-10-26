//! Resolution of polygon references and edge selections.

use forja_math::Vec2;

use super::footprint::Footprint;
use super::poly::{self, Ring, Shape};
use super::spec::{EdgeSel, EdgeTok, PolySpec, PolyTable};

/// What a polygon may refer to besides the footprint.
pub struct PolyCtx<'a> {
    pub fp: Option<&'a Footprint>,
    /// `"parent"`: the enclosing element's plan (tower stage, mass).
    pub parent: Option<&'a Shape>,
    /// Default centre for shapes without `center`.
    pub center: Vec2,
}

fn regular_sides(name: &str) -> Option<usize> {
    Some(match name {
        "triangle" => 3,
        "square" => 4,
        "pentagon" => 5,
        "hexagon" => 6,
        "heptagon" => 7,
        "octagon" => 8,
        "decagon" => 10,
        "dodecagon" => 12,
        _ => return None,
    })
}

pub const SHAPES: &[&str] = &["square", "rect", "circle", "ellipse", "triangle", "pentagon", "hexagon", "heptagon", "octagon", "decagon", "dodecagon", "polygon"];

/// Builds a shape from `shape = ...` fields.
#[allow(clippy::too_many_arguments)]
fn make_shape(name: &str, center: Vec2, radius: Option<f32>, width: Option<f32>, size: Option<[f32; 2]>, sides: Option<u32>, rotation: f32, detail: f32) -> Result<Ring, String> {
    let n = name.trim().to_ascii_lowercase();
    let need = |what: &str| format!("shape `{n}` needs {what}");
    match n.as_str() {
        "rect" => {
            let s = size.or(width.map(|w| [w, w])).ok_or_else(|| need("`size = [w, d]`"))?;
            Ok(poly::rect(center, Vec2::new(s[0], s[1]), rotation))
        }
        "circle" | "ellipse" => {
            let (rx, rz) = match (size, radius, width) {
                (Some(s), _, _) => (s[0] * 0.5, s[1] * 0.5),
                (None, Some(r), _) => (r, r),
                (None, None, Some(w)) => (w * 0.5, w * 0.5),
                _ => return Err(need("`radius` or `size`")),
            };
            let seg = ((rx.max(rz) * 6.0 * detail).clamp(16.0, 96.0) as usize / 4) * 4;
            let ring: Ring = poly::regular(Vec2::ZERO, 1.0, seg, rotation).into_iter().map(|p| center + Vec2::new(p.x * rx, p.y * rz)).collect();
            Ok(ring)
        }
        _ => {
            let sides = if n == "polygon" { sides.ok_or_else(|| need("`sides`"))? as usize } else { regular_sides(&n).ok_or_else(|| format!("unknown shape `{name}`{}; expected one of {}", super::parse::did_you_mean(&n, SHAPES), SHAPES.join(", ")))? };
            if n == "square" && size.is_some() {
                let s = size.unwrap();
                return Ok(poly::rect(center, Vec2::new(s[0], s[1]), rotation));
            }
            let r = match (radius, width) {
                (Some(r), _) => r,
                // Width across flats -> circumradius.
                (None, Some(w)) => w * 0.5 / (std::f32::consts::PI / sides as f32).cos(),
                _ => return Err(need("`radius` (circumradius) or `width` (across flats)")),
            };
            Ok(poly::regular(center, r, sides, rotation))
        }
    }
}

/// Shorthands: `"circle:4"`, `"octagon:5"` (circumradius), `"square:8"`
/// (side), `"rect:10x6"`.
fn shorthand(s: &str, center: Vec2, detail: f32) -> Option<Result<Ring, String>> {
    let (name, arg) = s.split_once(':')?;
    let n = name.trim().to_ascii_lowercase();
    if !SHAPES.contains(&n.as_str()) {
        return None;
    }
    let arg = arg.trim();
    Some((|| {
        if n == "rect" {
            let (a, b) = arg.split_once(['x', 'X']).ok_or("use rect:<w>x<d>")?;
            let (w, d) = (a.trim().parse::<f32>().map_err(|e| e.to_string())?, b.trim().parse::<f32>().map_err(|e| e.to_string())?);
            return make_shape("rect", center, None, None, Some([w, d]), None, 0.0, detail);
        }
        let v = arg.parse::<f32>().map_err(|_| format!("bad size in `{s}`"))?;
        if n == "square" {
            return make_shape("rect", center, None, None, Some([v, v]), None, 0.0, detail);
        }
        make_shape(&n, center, Some(v), None, None, None, 0.0, detail)
    })())
}

fn base_shapes(src_ref: Option<&str>, t: Option<&PolyTable>, points: Option<&Vec<[f32; 2]>>, ctx: &PolyCtx, detail: f32) -> Result<Vec<Shape>, String> {
    if let Some(p) = points {
        let ring: Ring = p.iter().map(|q| Vec2::new(q[0], q[1])).collect();
        return Ok(vec![Shape::new(ring)]);
    }
    if let Some(r) = src_ref {
        let rt = r.trim();
        if rt.eq_ignore_ascii_case("parent") {
            return ctx.parent.cloned().map(|s| vec![s]).ok_or_else(|| "`parent` polygon used outside a tower stage or mass".to_string());
        }
        if let Some(res) = shorthand(rt, ctx.center, detail) {
            return Ok(vec![Shape::new(res?)]);
        }
        if SHAPES.contains(&rt.to_ascii_lowercase().as_str()) {
            return Err(format!("shape `{rt}` needs a size: write \"{rt}:<radius>\" or {{ shape = \"{rt}\", radius = ... }}"));
        }
        let fp = ctx.fp.ok_or_else(|| format!("polygon `{rt}` refers to footprint.json, but there is no footprint"))?;
        let hits = fp.resolve(rt)?;
        return Ok(hits.into_iter().map(|p| p.shape.clone()).collect());
    }
    if let Some(t) = t
        && let Some(name) = &t.shape
    {
        let c = t.center.map(|c| Vec2::new(c[0], c[1])).unwrap_or(ctx.center);
        return Ok(vec![Shape::new(make_shape(name, c, t.radius, t.width, t.size, t.sides, t.rotation, detail)?)]);
    }
    Err("a polygon table needs `ref`, `points` or `shape`".into())
}

fn chamfer(ring: &[Vec2], d: f32) -> Ring {
    let n = ring.len();
    let mut out = Vec::with_capacity(n * 2);
    for i in 0..n {
        let (a, b, c) = (ring[(i + n - 1) % n], ring[i], ring[(i + 1) % n]);
        let (la, lc) = ((a - b).length(), (c - b).length());
        let k = d.min(la * 0.45).min(lc * 0.45);
        out.push(b + (a - b).normalize_or(Vec2::ZERO) * k);
        out.push(b + (c - b).normalize_or(Vec2::ZERO) * k);
    }
    out
}

/// Resolves a polygon spec to normalised shapes (outer CCW, holes CW).
pub fn resolve(spec: &PolySpec, ctx: &PolyCtx, detail: f32) -> Result<Vec<Shape>, String> {
    let shapes = match spec {
        PolySpec::Points(p) => base_shapes(None, None, Some(p), ctx, detail)?,
        PolySpec::Ref(r) => base_shapes(Some(r), None, None, ctx, detail)?,
        PolySpec::Table(t) => {
            let mut v = base_shapes(t.reference.as_deref(), Some(t), t.points.as_ref(), ctx, detail)?;
            if let Some(i) = t.index {
                if i >= v.len() {
                    return Err(format!("index {i} out of range: the reference has {} polygon(s)", v.len()));
                }
                v = vec![v.swap_remove(i)];
            }
            let mut out = Vec::with_capacity(v.len());
            for mut s in v {
                if !t.keep_holes {
                    s.holes.clear();
                }
                if t.hull {
                    s = Shape::new(poly::convex_hull(&s.outer));
                }
                if t.rect {
                    s = Shape::new(poly::min_rect(&s.outer));
                }
                if let Some(tol) = t.simplify {
                    s.outer = poly::simplify(&s.outer, tol);
                    s.holes = s.holes.iter().map(|h| poly::simplify(h, tol)).collect();
                }
                let grow = t.offset.unwrap_or(0.0) - t.inset.unwrap_or(0.0);
                if grow != 0.0 {
                    let outer = poly::offset(&s.outer, grow).ok_or_else(|| format!("offset/inset of {grow:+} m collapses the polygon"))?;
                    // Holes shrink when the shape grows.
                    let holes = s.holes.iter().filter_map(|h| poly::offset(h, -grow)).collect();
                    s = Shape { outer, holes };
                }
                if let Some(d) = t.chamfer {
                    s.outer = chamfer(&s.outer, d);
                }
                for h in &t.holes {
                    for hs in resolve(h, ctx, detail)? {
                        s.holes.push(hs.outer);
                    }
                }
                let mut pieces = vec![s];
                if let Some(c) = &t.clip {
                    let clips = resolve(c, ctx, detail)?;
                    let mut res = Vec::new();
                    for p in &pieces {
                        for cl in &clips {
                            for r in poly::intersect(&p.outer, &cl.outer) {
                                let holes = p.holes.iter().filter(|h| h.iter().all(|q| poly::point_in_ring(&r, *q))).cloned().collect();
                                res.push(Shape { outer: r, holes });
                            }
                        }
                    }
                    if res.is_empty() {
                        return Err("clip leaves nothing (the polygons do not overlap)".into());
                    }
                    pieces = res;
                }
                for p in pieces {
                    let mut p = p;
                    let about = t.about.map(|a| Vec2::new(a[0], a[1])).unwrap_or_else(|| poly::centroid(&p.outer));
                    if let Some(k) = t.scale {
                        p = p.map(|q| about + (q - about) * k);
                    }
                    if let Some(r) = t.rotate {
                        p = p.map(|q| about + poly::rotate(q - about, r));
                    }
                    if let Some(tr) = t.translate {
                        p = p.map(|q| q + Vec2::new(tr[0], tr[1]));
                    }
                    out.push(p);
                }
            }
            out
        }
    };
    let n = shapes.len();
    let res: Vec<Shape> = shapes.into_iter().filter_map(|s| s.normalized()).collect();
    if res.is_empty() {
        return Err(format!("polygon is degenerate ({n} ring(s), none with area)"));
    }
    Ok(res)
}

/// A selected edge: ring 0 = outer, k + 1 = hole k.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct EdgeRef {
    pub ring: usize,
    pub i: usize,
}

pub fn ring_of(shape: &Shape, r: usize) -> &[Vec2] {
    if r == 0 { &shape.outer } else { &shape.holes[r - 1] }
}

fn matches(tok: &EdgeTok, shape: &Shape, e: EdgeRef, ranks: &[(usize, f32)]) -> bool {
    let ring = ring_of(shape, e.ring);
    let n = ring.len();
    let len = (ring[(e.i + 1) % n] - ring[e.i]).length();
    let outer_n = shape.outer.len() as i64;
    match tok {
        EdgeTok::All => true,
        EdgeTok::Outer => e.ring == 0,
        EdgeTok::Inner => e.ring > 0,
        EdgeTok::Index(k) => e.ring == 0 && (k.rem_euclid(outer_n.max(1))) as usize == e.i,
        EdgeTok::Range(a, b) => e.ring == 0 && (*a..*b).any(|k| k.rem_euclid(outer_n.max(1)) as usize == e.i),
        EdgeTok::Longest(k) => e.ring == 0 && ranks.iter().take(*k).any(|(i, _)| *i == e.i),
        EdgeTok::Shortest(k) => e.ring == 0 && ranks.iter().rev().take(*k).any(|(i, _)| *i == e.i),
        EdgeTok::Facing(deg, tol) => {
            let az = poly::azimuth(poly::edge_normal(ring, e.i));
            let d = ((az - deg + 540.0).rem_euclid(360.0) - 180.0).abs();
            d <= *tol + 1e-3
        }
        EdgeTok::MinLength(m) => len >= *m,
        EdgeTok::Not(t) => !matches(t, shape, e, ranks),
    }
}

/// Edges selected by `sel`, in ring order.
pub fn select(shape: &Shape, sel: &EdgeSel) -> Vec<EdgeRef> {
    let mut ranks: Vec<(usize, f32)> = (0..shape.outer.len()).map(|i| (i, (shape.outer[(i + 1) % shape.outer.len()] - shape.outer[i]).length())).collect();
    ranks.sort_by(|a, b| b.1.total_cmp(&a.1));
    let positive: Vec<&EdgeTok> = sel.0.iter().filter(|t| !matches!(t, EdgeTok::Not(_))).collect();
    let negative: Vec<&EdgeTok> = sel.0.iter().filter(|t| matches!(t, EdgeTok::Not(_))).collect();
    let mut out = Vec::new();
    for r in 0..=shape.holes.len() {
        let n = ring_of(shape, r).len();
        for i in 0..n {
            let e = EdgeRef { ring: r, i };
            let pos = if positive.is_empty() { true } else { positive.iter().any(|t| matches(t, shape, e, &ranks)) };
            if pos && negative.iter().all(|t| matches(t, shape, e, &ranks)) {
                out.push(e);
            }
        }
    }
    out
}

/// Groups selected edges into runs of consecutive edges per ring:
/// `(ring, first vertex index, edge count, closed)`.
pub fn chains(shape: &Shape, edges: &[EdgeRef]) -> Vec<(usize, usize, usize, bool)> {
    let mut out = Vec::new();
    for r in 0..=shape.holes.len() {
        let n = ring_of(shape, r).len();
        let sel: Vec<bool> = (0..n).map(|i| edges.contains(&EdgeRef { ring: r, i })).collect();
        if sel.iter().all(|s| *s) {
            out.push((r, 0, n, true));
            continue;
        }
        let Some(start) = (0..n).find(|&i| !sel[i]) else { continue };
        let mut k = 1;
        while k <= n {
            let i = (start + k) % n;
            if sel[i] {
                let first = i;
                let mut len = 0;
                while k <= n && sel[(start + k) % n] {
                    len += 1;
                    k += 1;
                }
                out.push((r, first, len, false));
            } else {
                k += 1;
            }
        }
    }
    out
}

/// Polyline of a chain: `count + 1` points (or `count` when closed).
pub fn chain_points(shape: &Shape, c: (usize, usize, usize, bool)) -> Vec<Vec2> {
    let ring = ring_of(shape, c.0);
    let n = ring.len();
    let m = if c.3 { c.2 } else { c.2 + 1 };
    (0..m).map(|k| ring[(c.1 + k) % n]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::landmark::spec::EdgeTok;

    fn ctx() -> PolyCtx<'static> {
        PolyCtx { fp: None, parent: None, center: Vec2::ZERO }
    }

    #[test]
    fn shapes_and_ops() {
        let oct = resolve(&PolySpec::Ref("octagon:5".into()), &ctx(), 1.0).unwrap();
        assert_eq!(oct[0].outer.len(), 8);
        let t = PolyTable { shape: Some("square".into()), width: Some(10.0), inset: Some(1.0), ..Default::default() };
        let s = resolve(&PolySpec::Table(Box::new(t)), &ctx(), 1.0).unwrap();
        assert!((s[0].area() - 64.0).abs() < 0.01, "{}", s[0].area());
        let t = PolyTable {
            points: Some(vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]]),
            clip: Some(PolySpec::Points(vec![[5.0, -5.0], [15.0, -5.0], [15.0, 5.0], [5.0, 5.0]])),
            ..Default::default()
        };
        let s = resolve(&PolySpec::Table(Box::new(t)), &ctx(), 1.0).unwrap();
        assert!((s[0].area() - 25.0).abs() < 0.05);
        assert!(resolve(&PolySpec::Ref("outline:0".into()), &ctx(), 1.0).unwrap_err().contains("no footprint"));
        assert!(resolve(&PolySpec::Ref("octogon:4".into()), &ctx(), 1.0).is_err());
    }

    #[test]
    fn edge_selection_and_chains() {
        let s = Shape::new(poly::rect(Vec2::ZERO, Vec2::new(20.0, 10.0), 0.0));
        let south = select(&s, &EdgeSel(vec![EdgeTok::Facing(180.0, 45.0)]));
        assert_eq!(south.len(), 1);
        let n = poly::edge_normal(&s.outer, south[0].i);
        assert!(n.y > 0.9);
        let long = select(&s, &EdgeSel(vec![EdgeTok::Longest(2)]));
        assert_eq!(long.len(), 2);
        let not0 = select(&s, &EdgeSel(vec![EdgeTok::Not(Box::new(EdgeTok::Index(0)))]));
        assert_eq!(not0.len(), 3);
        let c = chains(&s, &not0);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].2, 3);
        assert!(!c[0].3);
        assert_eq!(chain_points(&s, c[0]).len(), 4);
        let all = select(&s, &EdgeSel::default());
        assert_eq!(chains(&s, &all), vec![(0, 0, 4, true)]);
    }
}
