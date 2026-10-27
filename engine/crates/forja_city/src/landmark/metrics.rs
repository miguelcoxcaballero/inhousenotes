//! `landmark check` metrics: triangle counts, bounds, element heights,
//! plan coverage near the ground versus the OSM outlines and Catastro
//! parcels (IoU), modelled heights versus tagged heights, outline edges.

use forja_math::{Vec2, Vec3};
use serde_json::{Value, json};

use super::build::Model;
use super::footprint::{FpPoly, Footprint};
use super::poly::{self, Shape};

/// Raster over the plan.
pub struct Grid {
    pub min: Vec2,
    pub cell: f32,
    pub w: usize,
    pub h: usize,
}

impl Grid {
    pub fn new(min: Vec2, max: Vec2, target: usize) -> Grid {
        let ext = (max - min).max(Vec2::splat(1.0));
        let cell = (ext.max_element() / target as f32).max(0.1);
        Grid { min, cell, w: (ext.x / cell).ceil() as usize + 1, h: (ext.y / cell).ceil() as usize + 1 }
    }

    pub fn center(&self, i: usize, j: usize) -> Vec2 {
        self.min + Vec2::new((i as f32 + 0.5) * self.cell, (j as f32 + 0.5) * self.cell)
    }

    fn row_z(&self, j: usize) -> f32 {
        self.min.y + (j as f32 + 0.5) * self.cell
    }

    /// Cells whose centre lies in any of the shapes.
    pub fn fill_shapes<'a>(&self, shapes: impl IntoIterator<Item = &'a Shape>) -> Vec<bool> {
        let mut m = vec![false; self.w * self.h];
        for s in shapes {
            let (mn, mx) = s.bbox();
            let i0 = (((mn.x - self.min.x) / self.cell).floor().max(0.0)) as usize;
            let i1 = (((mx.x - self.min.x) / self.cell).ceil() as usize).min(self.w);
            let j0 = (((mn.y - self.min.y) / self.cell).floor().max(0.0)) as usize;
            let j1 = (((mx.y - self.min.y) / self.cell).ceil() as usize).min(self.h);
            for j in j0..j1 {
                for i in i0..i1 {
                    if s.contains(self.center(i, j)) {
                        m[j * self.w + i] = true;
                    }
                }
            }
        }
        m
    }
}

/// Slices closed triangle groups at `y` and fills the interior (nonzero
/// winding; rows that do not close fall back to even-odd).
pub fn slice_fill(grid: &Grid, tris: &[[Vec3; 3]], groups: &[(usize, usize)], y: f32, out: &mut [bool]) {
    for &(a, b) in groups {
        // Segments of this group in plan, with the facing of the surface.
        let mut segs: Vec<(Vec2, Vec2, f32)> = Vec::new();
        for t in &tris[a..b] {
            let mut pts = Vec::with_capacity(2);
            for k in 0..3 {
                let (p, q) = (t[k], t[(k + 1) % 3]);
                if (p.y <= y) != (q.y <= y) {
                    let f = (y - p.y) / (q.y - p.y);
                    let r = p.lerp(q, f);
                    pts.push(Vec2::new(r.x, r.z));
                }
            }
            if pts.len() == 2 {
                let n = (t[1] - t[0]).cross(t[2] - t[0]);
                if n.x.abs() + n.z.abs() < 1e-9 {
                    continue;
                }
                segs.push((pts[0], pts[1], n.x));
            }
        }
        if segs.is_empty() {
            continue;
        }
        let mut events: Vec<Vec<(f32, i32)>> = vec![Vec::new(); grid.h];
        for (p, q, nx) in &segs {
            let (lo, hi) = if p.y < q.y { (*p, *q) } else { (*q, *p) };
            if (hi.y - lo.y).abs() < 1e-7 {
                continue;
            }
            let j0 = (((lo.y - grid.min.y) / grid.cell - 0.5).ceil().max(0.0)) as usize;
            let j1 = (((hi.y - grid.min.y) / grid.cell - 0.5).ceil().max(0.0) as usize).min(grid.h);
            for (j, ev) in events.iter_mut().enumerate().take(j1).skip(j0) {
                let z = grid.row_z(j);
                if z < lo.y || z >= hi.y {
                    continue;
                }
                let x = lo.x + (hi.x - lo.x) * (z - lo.y) / (hi.y - lo.y);
                ev.push((x, if *nx < 0.0 { 1 } else { -1 }));
            }
        }
        for (j, ev) in events.iter_mut().enumerate() {
            if ev.is_empty() {
                continue;
            }
            ev.sort_by(|a, b| a.0.total_cmp(&b.0));
            let balanced = ev.iter().map(|e| e.1).sum::<i32>() == 0;
            let mut wind = 0i32;
            let mut parity = false;
            let mut k = 0;
            for i in 0..grid.w {
                let x = grid.min.x + (i as f32 + 0.5) * grid.cell;
                while k < ev.len() && ev[k].0 <= x {
                    wind += ev[k].1;
                    parity = !parity;
                    k += 1;
                }
                let inside = if balanced { wind != 0 } else { parity };
                if inside {
                    out[j * grid.w + i] = true;
                }
            }
        }
    }
}

/// Per-cell maximum height of all triangles (top-down height map).
pub fn height_map(grid: &Grid, model: &Model) -> Vec<f32> {
    let mut hm = vec![f32::NEG_INFINITY; grid.w * grid.h];
    for p in model.geo.prims.values() {
        for t in p.indices.chunks_exact(3) {
            let (a, b, c) = (p.positions[t[0] as usize], p.positions[t[1] as usize], p.positions[t[2] as usize]);
            let (pa, pb, pc) = (Vec2::new(a.x, a.z), Vec2::new(b.x, b.z), Vec2::new(c.x, c.z));
            let mn = pa.min(pb).min(pc);
            let mx = pa.max(pb).max(pc);
            let i0 = (((mn.x - grid.min.x) / grid.cell - 0.5).ceil().max(0.0)) as usize;
            let i1 = ((((mx.x - grid.min.x) / grid.cell - 0.5).floor() + 1.0).max(0.0) as usize).min(grid.w);
            let j0 = (((mn.y - grid.min.y) / grid.cell - 0.5).ceil().max(0.0)) as usize;
            let j1 = ((((mx.y - grid.min.y) / grid.cell - 0.5).floor() + 1.0).max(0.0) as usize).min(grid.h);
            let den = (pb - pa).perp_dot(pc - pa);
            if den.abs() < 1e-10 {
                // Vertical triangle: mark its top edge cells by the vertices.
                for q in [a, b, c] {
                    let i = ((q.x - grid.min.x) / grid.cell).floor();
                    let j = ((q.z - grid.min.y) / grid.cell).floor();
                    if i >= 0.0 && j >= 0.0 && (i as usize) < grid.w && (j as usize) < grid.h {
                        let k = j as usize * grid.w + i as usize;
                        hm[k] = hm[k].max(q.y);
                    }
                }
                continue;
            }
            for j in j0..j1 {
                for i in i0..i1 {
                    let q = grid.center(i, j);
                    let w1 = (q - pa).perp_dot(pc - pa) / den;
                    let w2 = (pb - pa).perp_dot(q - pa) / den;
                    let w0 = 1.0 - w1 - w2;
                    if w0 >= -1e-4 && w1 >= -1e-4 && w2 >= -1e-4 {
                        let y = a.y * w0 + b.y * w1 + c.y * w2;
                        let k = j * grid.w + i;
                        hm[k] = hm[k].max(y);
                    }
                }
            }
        }
    }
    hm
}

fn iou(a: &[bool], b: &[bool], cell_area: f32) -> Value {
    let (mut inter, mut uni, mut na, mut nb) = (0usize, 0usize, 0usize, 0usize);
    for (x, y) in a.iter().zip(b) {
        inter += (*x && *y) as usize;
        uni += (*x || *y) as usize;
        na += *x as usize;
        nb += *y as usize;
    }
    let r = |v: f32| (v * 1000.0).round() / 1000.0;
    json!({
        "iou": r(if uni > 0 { inter as f32 / uni as f32 } else { 0.0 }),
        "precision": r(if na > 0 { inter as f32 / na as f32 } else { 0.0 }),
        "recall": r(if nb > 0 { inter as f32 / nb as f32 } else { 0.0 }),
        "model_area_m2": r(na as f32 * cell_area),
        "reference_area_m2": r(nb as f32 * cell_area),
    })
}

fn percentile(mut v: Vec<f32>, p: f32) -> Option<f32> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    Some(v[((v.len() - 1) as f32 * p).round() as usize])
}

/// Heights of the model inside a plan polygon (shrunk to avoid neighbours).
/// (median, 90th percentile, maximum, covered fraction).
fn heights_in(grid: &Grid, hm: &[f32], shape: &Shape) -> (Option<f32>, Option<f32>, Option<f32>, Option<f32>) {
    let inner = poly::offset(&shape.outer, -0.6).map(|o| Shape { outer: o, holes: shape.holes.clone() }).unwrap_or(shape.clone());
    let mask = grid.fill_shapes([&inner]);
    let hs: Vec<f32> = mask.iter().zip(hm).filter(|(m, h)| **m && h.is_finite()).map(|(_, h)| *h).collect();
    let cover = if mask.iter().any(|m| *m) { Some(hs.len() as f32 / mask.iter().filter(|m| **m).count() as f32) } else { None };
    (percentile(hs.clone(), 0.5), percentile(hs.clone(), 0.9), percentile(hs, 1.0), cover)
}

fn round2(v: f32) -> f32 {
    (v * 100.0).round() / 100.0
}

/// Full metrics JSON.
pub fn compute(model: &Model, fp: Option<&Footprint>, slug: &str, build_ms: f32) -> Value {
    let mut warnings: Vec<String> = model.warnings.clone();
    let mut by_mat = serde_json::Map::new();
    let mut total = 0usize;
    for k in &model.geo.order {
        let t = model.geo.prims[k].tri_count();
        total += t;
        let name = model.palette.get(*k).name.clone();
        let prev = by_mat.get(&name).and_then(|v| v.as_u64()).unwrap_or(0);
        by_mat.insert(name, json!(prev + t as u64));
    }
    let bounds = super::mesh::bounds(&model.geo);
    let (mn, mx) = bounds.unwrap_or((Vec3::ZERO, Vec3::ZERO));
    if bounds.is_none() {
        warnings.push("the model is empty".into());
    }
    let elements: Vec<Value> = model
        .stats
        .iter()
        .map(|s| {
            json!({
                "name": s.name, "type": s.kind, "line": s.line,
                "bottom": round2(s.min.y), "top": round2(s.max.y), "height": round2(s.max.y - s.min.y),
                "plan_min": [round2(s.min.x), round2(s.min.z)], "plan_max": [round2(s.max.x), round2(s.max.z)],
                "triangles": s.triangles,
            })
        })
        .collect();
    if mn.y < -0.5 {
        warnings.push(format!("geometry goes {:.1} m below the ground", -mn.y));
    }
    if total > 400_000 {
        warnings.push(format!("{total} triangles: above the 400k budget"));
    } else if total > 150_000 {
        warnings.push(format!("{total} triangles: above the typical 150k budget (fine for major landmarks)"));
    }
    let mut out = json!({
        "slug": slug,
        "ok": true,
        "build_ms": round2(build_ms),
        "triangles": total,
        "draw_calls": model.geo.order.len(),
        "by_material": by_mat,
        "bbox": { "min": [round2(mn.x), round2(mn.y), round2(mn.z)], "max": [round2(mx.x), round2(mx.y), round2(mx.z)] },
        "size": [round2(mx.x - mn.x), round2(mx.y - mn.y), round2(mx.z - mn.z)],
        "max_height": round2(mx.y),
        "elements": elements,
        "collider_triangles": model.collider.len(),
    });
    if let Some(fp) = fp {
        // Plan grid over the model and the reference polygons.
        let mut pmin = Vec2::new(mn.x, mn.z);
        let mut pmax = Vec2::new(mx.x, mx.z);
        if let Some((a, b)) = fp.bounds() {
            pmin = pmin.min(a);
            pmax = pmax.max(b);
        }
        let grid = Grid::new(pmin - Vec2::splat(2.0), pmax + Vec2::splat(2.0), 700);
        let mut cover = vec![false; grid.w * grid.h];
        for y in [1.0f32, 2.0, 3.0] {
            slice_fill(&grid, &model.collider, &model.col_groups, y, &mut cover);
        }
        let ca = grid.cell * grid.cell;
        let mut fpj = serde_json::Map::new();
        fpj.insert("slice_heights".into(), json!([1.0, 2.0, 3.0]));
        fpj.insert("cell_m".into(), json!(round2(grid.cell)));
        if !fp.outlines.is_empty() {
            let osm = grid.fill_shapes(fp.outlines.iter().map(|p| &p.shape));
            let v = iou(&cover, &osm, ca);
            if v["iou"].as_f64().unwrap_or(0.0) < 0.75 {
                warnings.push(format!("footprint IoU vs OSM outlines is {:.2}: check the masses against the outline (plan.png)", v["iou"].as_f64().unwrap_or(0.0)));
            }
            fpj.insert("osm".into(), v);
        }
        if !fp.catastro.is_empty() {
            let cat = grid.fill_shapes(fp.catastro.iter().map(|p| &p.shape));
            fpj.insert("catastro".into(), iou(&cover, &cat, ca));
        }
        out["footprint"] = Value::Object(fpj);
        let hm = height_map(&grid, model);
        let part_rows = |list: &[FpPoly], what: &str| -> Vec<Value> {
            list.iter()
                .enumerate()
                .map(|(i, p)| {
                    let (med, p90, top, cov) = heights_in(&grid, &hm, &p.shape);
                    let tag = p.tag_height();
                    let mut v = json!({
                        "ref": format!("{what}:{i}"), "id": p.id,
                        "tag_height": tag, "tag_min_height": p.tag_min_height(),
                        "model_median": med.map(round2), "model_p90": p90.map(round2), "model_max": top.map(round2),
                        "covered": cov.map(round2),
                    });
                    if let (Some(t), Some(m), Some(md), Some(mx)) = (tag, p90, med, top) {
                        v["diff"] = json!(round2(m - t));
                        // Clearly off: even the median is above, or even the
                        // highest point below, the tagged height.
                        v["height_check"] = json!(if md - t > 3.0 { "model too high" } else if t - mx > 3.0 { "model too low" } else { "ok" });
                    }
                    if let Some(f) = p.floors_above {
                        v["floors_above"] = json!(f);
                        v["approx_eave_height"] = json!(round2(f * 3.2 + 1.0));
                    }
                    for k in ["building", "building:part", "roof:shape", "building:levels", "name"] {
                        if let Some(t) = p.tag(k) {
                            v[k] = json!(t);
                        }
                    }
                    v
                })
                .collect()
        };
        out["outlines"] = json!(part_rows(&fp.outlines, "outline"));
        out["parts"] = json!(part_rows(&fp.parts, "part"));
        out["catastro"] = json!(part_rows(&fp.catastro, "catastro"));
        for v in out["parts"].as_array().unwrap().iter().chain(out["outlines"].as_array().unwrap()) {
            if let Some(c) = v.get("height_check").and_then(|d| d.as_str())
                && c != "ok"
            {
                warnings.push(format!(
                    "{} ({}): {c}: median {:.1} m / p90 {:.1} m vs tagged height {:.1} m",
                    v["ref"].as_str().unwrap_or(""),
                    v["id"].as_str().unwrap_or(""),
                    v["model_median"].as_f64().unwrap_or(0.0),
                    v["model_p90"].as_f64().unwrap_or(0.0),
                    v["tag_height"].as_f64().unwrap_or(0.0)
                ));
            }
        }
        // Edges of each outline (for `edges = ...` selectors).
        let edges: Vec<Value> = fp
            .outlines
            .iter()
            .enumerate()
            .map(|(k, p)| {
                let r = &p.shape.outer;
                let list: Vec<Value> = (0..r.len())
                    .map(|i| {
                        let a = r[i];
                        let b = r[(i + 1) % r.len()];
                        let az = poly::azimuth(poly::edge_normal(r, i));
                        json!({ "i": i, "length": round2((b - a).length()), "facing": poly::compass(az), "azimuth": az.round(), "from": [round2(a.x), round2(a.y)], "to": [round2(b.x), round2(b.y)] })
                    })
                    .collect();
                json!({ "outline": k, "id": p.id, "area_m2": round2(p.shape.area()), "edges": list })
            })
            .collect();
        out["outline_edges"] = json!(edges);
    } else {
        warnings.push("no footprint.json: footprint metrics skipped".into());
    }
    out["warnings"] = json!(warnings);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Prim;
    use crate::landmark::mesh;

    #[test]
    fn slice_of_a_prism_fills_its_plan() {
        let s = Shape::new(poly::rect(Vec2::new(5.0, 5.0), Vec2::new(10.0, 6.0), 15.0));
        let mut p = Prim::default();
        mesh::prism(&mut p, &s, 0.0, 10.0, true, true);
        let tris: Vec<[Vec3; 3]> = p.indices.chunks_exact(3).map(|t| [p.positions[t[0] as usize], p.positions[t[1] as usize], p.positions[t[2] as usize]]).collect();
        let grid = Grid::new(Vec2::new(-5.0, -5.0), Vec2::new(15.0, 15.0), 200);
        let mut cover = vec![false; grid.w * grid.h];
        slice_fill(&grid, &tris, &[(0, tris.len())], 2.0, &mut cover);
        let truth = grid.fill_shapes([&s]);
        let v = iou(&cover, &truth, grid.cell * grid.cell);
        assert!(v["iou"].as_f64().unwrap() > 0.97, "{v}");
        // Two overlapping prisms (separate groups) union correctly.
        let s2 = Shape::new(poly::rect(Vec2::new(9.0, 5.0), Vec2::new(6.0, 6.0), 0.0));
        let mut p2 = Prim::default();
        mesh::prism(&mut p2, &s2, 0.0, 10.0, true, true);
        let mut all = tris.clone();
        let n1 = all.len();
        all.extend(p2.indices.chunks_exact(3).map(|t| [p2.positions[t[0] as usize], p2.positions[t[1] as usize], p2.positions[t[2] as usize]]));
        let mut cover2 = vec![false; grid.w * grid.h];
        slice_fill(&grid, &all, &[(0, n1), (n1, all.len())], 2.0, &mut cover2);
        let truth2 = grid.fill_shapes([&s, &s2]);
        assert!(iou(&cover2, &truth2, 1.0)["iou"].as_f64().unwrap() > 0.97);
    }
}
