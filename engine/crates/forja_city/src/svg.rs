//! Debug drawing of a fabric as SVG: ground classes, courts, ways, lots
//! with their edges coloured by kind, corner marks. Lets the generators be
//! reviewed without a GPU.

use std::fmt::Write;

use forja_math::Vec2;

use crate::fabric::Fabric;
use crate::lot::EdgeKind;
use crate::plan::Region;

fn path_of(r: &Region, f: impl Fn(Vec2) -> (f32, f32)) -> String {
    let mut d = String::new();
    for s in r.shapes() {
        for ring in s.rings() {
            for (i, p) in ring.iter().enumerate() {
                let (x, y) = f(*p);
                let _ = write!(d, "{}{:.2},{:.2} ", if i == 0 { "M" } else { "L" }, x, y);
            }
            d.push_str("Z ");
        }
    }
    d
}

impl Fabric {
    /// The fabric as an SVG document, `px` pixels per metre, optionally
    /// cropped to a window `[min, max]`.
    pub fn to_svg(&self, px: f32, window: Option<[Vec2; 2]>) -> String {
        let [lo, hi] = window.unwrap_or(self.extent);
        let (w, h) = ((hi.x - lo.x) * px, (hi.y - lo.y) * px);
        let f = |p: Vec2| ((p.x - lo.x) * px, (p.y - lo.y) * px);
        let mut s = String::new();
        let _ = writeln!(s, r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w:.0}" height="{h:.0}" viewBox="0 0 {w:.0} {h:.0}">"#);
        let _ = writeln!(s, r##"<rect width="100%" height="100%" fill="#222"/>"##);
        let g = &self.ground;
        for (r, c) in [(&g.carriage, "#555"), (&g.median, "#7a7"), (&g.walk, "#bbb"), (&g.shared, "#c9a77a"), (&g.plaza, "#e0cf9a"), (&self.courts, "#6a8a5a")] {
            let _ = writeln!(s, r#"<path d="{}" fill="{c}" fill-rule="evenodd"/>"#, path_of(r, f));
        }
        // Lots, then their edges by kind.
        for l in &self.lots {
            let mut d = String::new();
            for (i, p) in l.ring.iter().enumerate() {
                let (x, y) = f(*p);
                let _ = write!(d, "{}{:.2},{:.2} ", if i == 0 { "M" } else { "L" }, x, y);
            }
            let _ = writeln!(s, r##"<path d="{d}Z" fill="#d9b38c" stroke="none"/>"##);
            let n = l.ring.len();
            for i in 0..n {
                let (a, b) = (f(l.ring[i]), f(l.ring[(i + 1) % n]));
                let (c, wdt) = match l.edges[i] {
                    EdgeKind::Street => ("#c0392b", 1.6),
                    EdgeKind::Corner => ("#8e44ad", 2.2),
                    EdgeKind::Courtyard => ("#27ae60", 1.0),
                    EdgeKind::Party => ("#333", 0.6),
                };
                let _ = writeln!(s, r#"<line x1="{:.2}" y1="{:.2}" x2="{:.2}" y2="{:.2}" stroke="{c}" stroke-width="{wdt}"/>"#, a.0, a.1, b.0, b.1);
            }
        }
        // Street centrelines.
        for st in &self.graph.streets {
            let mut d = String::new();
            for (i, p) in st.line.pts().iter().enumerate() {
                let (x, y) = f(*p);
                let _ = write!(d, "{}{:.2},{:.2} ", if i == 0 { "M" } else { "L" }, x, y);
            }
            let _ = writeln!(s, r##"<path d="{d}" fill="none" stroke="#2980b9" stroke-width="0.5" stroke-dasharray="3,2"/>"##);
        }
        for b in &self.blocks {
            for c in &b.corners {
                let (x, y) = f(c.apex);
                let _ = writeln!(s, r##"<circle cx="{x:.2}" cy="{y:.2}" r="1.5" fill="{}"/>"##, if c.corner { "#8e44ad" } else { "#f39c12" });
            }
        }
        for j in &g.junctions {
            let (x, y) = f(j.c);
            let _ = writeln!(s, r##"<circle cx="{x:.2}" cy="{y:.2}" r="{:.2}" fill="none" stroke="#f1c40f" stroke-width="0.5"/>"##, j.r * px);
        }
        s.push_str("</svg>\n");
        s
    }
}

/// Regions with fill colours as an SVG document (debugging aid).
pub fn regions(list: &[(&Region, &str)], px: f32) -> String {
    let mut lo = Vec2::splat(f32::MAX);
    let mut hi = Vec2::splat(f32::MIN);
    for (r, _) in list {
        if let Some([a, b]) = r.bbox() {
            lo = lo.min(a);
            hi = hi.max(b);
        }
    }
    let (lo, hi) = (lo - Vec2::splat(2.0), hi + Vec2::splat(2.0));
    let (w, h) = ((hi.x - lo.x) * px, (hi.y - lo.y) * px);
    let f = |p: Vec2| ((p.x - lo.x) * px, (p.y - lo.y) * px);
    let mut s = String::new();
    let _ = writeln!(s, r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w:.0}" height="{h:.0}" viewBox="0 0 {w:.0} {h:.0}">"#);
    for (r, c) in list {
        let _ = writeln!(s, r##"<path d="{}" fill="{c}" stroke="#000" stroke-width="0.6" fill-rule="evenodd"/>"##, path_of(r, f));
    }
    s.push_str("</svg>\n");
    s
}
