//! A real street layout from OpenStreetMap as a `forja_city` draft: the
//! proof that the city generator works on any street layout.
//!
//! Streets keep their real centrelines; widths are measured from the gaps
//! between the real building footprints (`forja_city::generators::measure`).
//! Footprints become lots (Catastro `building:part`s when they tile an
//! outline, so rear wings keep their own heights), de-overlapped in id
//! order with hairline gaps closed; the map edge cuts them into party
//! walls, so the extract reads as a city that continues. Unmapped plots are
//! parcelled procedurally by the fabric.
//!
//! Input is untrusted data: everything is parsed, never executed, numbers
//! are validated and nothing here panics on malformed input.

use std::collections::HashMap;

use forja_city::generators::measure::{self, OSM_PREFIX};
use forja_city::plan::{Draft, Footprint, Mask, NodeMark, Plaza, PlazaKind, Region, Shape, Way};
use forja_city::profile::ProfileLibrary;
use forja_math::Vec2;

use crate::WorldError;
use crate::osm::{MemberKind, OsmData, OsmWay, Projection};

pub struct OsmOptions {
    /// The buildable mask is the bounding box shrunk by this (m).
    pub inset: f32,
    /// Streets are kept this far beyond the mask (m).
    pub pad: f32,
}

impl Default for OsmOptions {
    fn default() -> Self {
        OsmOptions { inset: 5.0, pad: 5.0 }
    }
}

/// Highways that make streets.
const KEEP: [&str; 15] = ["primary", "secondary", "tertiary", "primary_link", "secondary_link", "tertiary_link", "residential", "unclassified", "living_street", "pedestrian", "service", "footway", "path", "steps", "trunk"];

/// Whether an OSM way is a street the city should have.
pub fn is_street(w: &OsmWay) -> bool {
    let Some(h) = w.tag("highway") else { return false };
    if !KEEP.contains(&h) {
        return false;
    }
    let no = |k: &str| w.tag(k).is_some_and(|v| v != "no");
    if matches!(w.tag("footway"), Some("sidewalk" | "crossing" | "traffic_island")) || matches!(w.tag("service"), Some("parking_aisle" | "driveway")) || matches!(w.tag("access"), Some("private" | "no")) {
        return false;
    }
    if w.tag("area") == Some("yes") || w.tag("layer").is_some_and(|l| l.trim() != "0") || no("bridge") || no("tunnel") || no("indoor") || w.tag("covered") == Some("yes") {
        return false;
    }
    true
}

fn parse_len(s: &str) -> Option<f32> {
    let t = s.trim().trim_end_matches('m').trim();
    t.parse::<f32>().ok().filter(|v| v.is_finite() && *v > 0.3 && *v < 100.0)
}

fn parse_levels(s: Option<&str>) -> Option<u32> {
    s.and_then(|v| v.trim().parse::<f32>().ok()).filter(|v| v.is_finite() && *v >= 0.0 && *v <= 60.0).map(|v| v.round() as u32)
}

/// Liang–Barsky clip of a polyline to a rectangle: the pieces inside.
fn clip_rect(pts: &[Vec2], lo: Vec2, hi: Vec2) -> Vec<Vec<Vec2>> {
    let mut out: Vec<Vec<Vec2>> = Vec::new();
    let mut cur: Vec<Vec2> = Vec::new();
    for w in pts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let d = b - a;
        let (mut t0, mut t1) = (0.0f32, 1.0f32);
        let mut ok = true;
        for (p, q) in [(-d.x, a.x - lo.x), (d.x, hi.x - a.x), (-d.y, a.y - lo.y), (d.y, hi.y - a.y)] {
            if p.abs() < 1e-12 {
                if q < 0.0 {
                    ok = false;
                }
            } else {
                let r = q / p;
                if p < 0.0 {
                    t0 = t0.max(r);
                } else {
                    t1 = t1.min(r);
                }
            }
        }
        if !ok || t0 > t1 {
            if cur.len() >= 2 {
                out.push(std::mem::take(&mut cur));
            }
            cur.clear();
            continue;
        }
        let (p0, p1) = (a + d * t0, a + d * t1);
        if cur.last().is_none_or(|l| l.distance(p0) > 1e-4) {
            if cur.len() >= 2 {
                out.push(std::mem::take(&mut cur));
            }
            cur = vec![p0];
        }
        cur.push(p1);
        if t1 < 1.0 {
            out.push(std::mem::take(&mut cur));
        }
    }
    if cur.len() >= 2 {
        out.push(cur);
    }
    out
}

/// Joins way pieces into closed rings by their end nodes (multipolygons).
fn join_rings(parts: &[Vec<i64>]) -> Vec<Vec<i64>> {
    let mut left: Vec<Vec<i64>> = parts.iter().filter(|p| p.len() >= 2).cloned().collect();
    let mut rings = Vec::new();
    let mut guard = 0;
    while let Some(mut cur) = left.pop() {
        guard += 1;
        if guard > 100_000 {
            break;
        }
        loop {
            if cur.len() > 3 && cur.first() == cur.last() {
                rings.push(cur);
                break;
            }
            let end = *cur.last().unwrap();
            if let Some(i) = left.iter().position(|p| p[0] == end || *p.last().unwrap() == end) {
                let mut p = left.swap_remove(i);
                if p[0] != end {
                    p.reverse();
                }
                cur.extend_from_slice(&p[1..]);
            } else {
                break;
            }
        }
    }
    rings
}

/// The draft of an OSM extract.
pub fn draft(data: &OsmData, lib: &ProfileLibrary, opt: &OsmOptions) -> Result<Draft, WorldError> {
    let bbox = data.bbox().ok_or_else(|| WorldError::Parse("osm: no nodes".into()))?;
    let proj = Projection::new(bbox);
    let (c0, c1) = (proj.point(bbox[0], bbox[3]), proj.point(bbox[2], bbox[1]));
    let (lo, hi) = (c0.min(c1), c0.max(c1));
    if !(hi - lo).is_finite() || (hi - lo).min_element() < 2.0 * opt.inset + 10.0 || (hi - lo).max_element() > 20_000.0 {
        return Err(WorldError::Parse("osm: extract too small or too large".into()));
    }
    let (mlo, mhi) = (lo + Vec2::splat(opt.inset), hi - Vec2::splat(opt.inset));
    let mask = vec![mlo, Vec2::new(mhi.x, mlo.y), mhi, Vec2::new(mlo.x, mhi.y)];
    let node = |id: &i64| data.nodes.get(id).map(|(lon, lat)| proj.point(*lon, *lat));
    let ring_of = |refs: &[i64]| -> Option<Vec<Vec2>> {
        let mut r: Vec<Vec2> = refs.iter().filter_map(node).collect();
        if r.len() > 1 && r.first() == r.last() {
            r.pop();
        }
        (r.len() >= 3 && forja_city::geom::signed_area(&r).abs() > 1.0).then(|| {
            if forja_city::geom::signed_area(&r) < 0.0 {
                r.reverse();
            }
            r
        })
    };
    let mut ways: Vec<&OsmWay> = data.ways.iter().collect();
    ways.sort_by_key(|w| w.id);
    let mut d = Draft { snap: 0.3, ..Default::default() };
    d.masks.push(Mask { zone: 1, ring: mask.clone() });
    let (plo, phi) = (mlo - Vec2::splat(opt.pad), mhi + Vec2::splat(opt.pad));
    for w in &ways {
        if is_street(w) {
            let pts: Vec<Vec2> = w.refs.iter().filter_map(node).collect();
            for piece in clip_rect(&pts, plo, phi) {
                let mut way = Way::new(piece, &format!("{OSM_PREFIX}{}", w.tag("highway").unwrap_or("residential")));
                way.osm = Some(w.id);
                way.zone = 1;
                way.name = w.tag("name").map(|s| s.chars().take(80).collect());
                way.oneway = match w.tag("oneway") {
                    Some("yes" | "1" | "true") => 1,
                    Some("-1" | "reverse") => -1,
                    _ => 0,
                };
                if let Some(width) = w.tag("width").or(w.tag("est_width")).and_then(parse_len) {
                    way.width = vec![width];
                }
                d.ways.push(way);
            }
        }
        // Squares: pedestrian areas and place=square.
        let square = (w.tag("highway") == Some("pedestrian") && w.tag("area") == Some("yes")) || w.tag("place") == Some("square");
        if square {
            if let Some(r) = ring_of(&w.refs) {
                d.plazas.push(Plaza { ring: r, zone: 1, kind: PlazaKind::Paved });
            }
        }
    }
    // Tagged nodes: signals and real dead ends.
    let mut marks: Vec<(i64, NodeMark)> = Vec::new();
    for (id, t) in &data.node_tags {
        let signals = t.get("highway").is_some_and(|h| h == "traffic_signals");
        let end = t.get("noexit").is_some_and(|v| v == "yes") || t.get("highway").is_some_and(|h| h == "turning_circle");
        if signals || end {
            if let Some(p) = node(id) {
                marks.push((*id, NodeMark { p, signals, end }));
            }
        }
    }
    marks.sort_by_key(|m| m.0);
    d.nodes = marks.into_iter().map(|m| m.1).collect();
    // Footprints: outlines and parts.
    struct Fp {
        id: i64,
        shape: Shape,
        levels: Option<u32>,
        roof: Option<u32>,
    }
    let mut outlines: Vec<Fp> = Vec::new();
    let mut parts: Vec<Fp> = Vec::new();
    for w in &ways {
        let b = w.tag("building");
        let part = w.tag("building:part");
        let is_b = b.is_some_and(|v| !matches!(v, "no" | "roof" | "construction" | "ruins"));
        let is_p = part.is_some_and(|v| !matches!(v, "no" | "roof"));
        if !(is_b || is_p) || w.tag("layer").is_some_and(|l| l.trim().starts_with('-')) {
            continue;
        }
        let Some(r) = ring_of(&w.refs) else { continue };
        let f = Fp { id: w.id, shape: Shape { outer: r, holes: Vec::new() }, levels: parse_levels(w.tag("building:levels")), roof: parse_levels(w.tag("roof:levels")) };
        if is_p { parts.push(f) } else { outlines.push(f) }
    }
    let way_by_id: HashMap<i64, &OsmWay> = ways.iter().map(|w| (w.id, *w)).collect();
    let mut rels: Vec<_> = data.relations.iter().filter(|r| r.tag("type") == Some("multipolygon") && r.tag("building").is_some_and(|v| !matches!(v, "no" | "roof" | "construction"))).collect();
    rels.sort_by_key(|r| r.id);
    for r in rels {
        let pieces = |role: &str| -> Vec<Vec<i64>> { r.members.iter().filter(|m| m.kind == MemberKind::Way && (m.role == role || (role == "outer" && m.role.is_empty()))).filter_map(|m| way_by_id.get(&m.id).map(|w| w.refs.clone())).collect() };
        let inners: Vec<Vec<Vec2>> = join_rings(&pieces("inner")).iter().filter_map(|r| ring_of(r)).collect();
        for outer in join_rings(&pieces("outer")).iter().filter_map(|r| ring_of(r)) {
            let holes: Vec<Vec<Vec2>> = inners.iter().filter(|h| point_in(&outer, h[0])).cloned().collect();
            outlines.push(Fp { id: r.id, shape: Shape { outer, holes }, levels: parse_levels(r.tag("building:levels")), roof: parse_levels(r.tag("roof:levels")) });
        }
    }
    // Parts replace an outline they tile (95% or more of it).
    let mut used_parts = vec![false; parts.len()];
    let mut fps: Vec<Fp> = Vec::new();
    for o in outlines {
        let or = Region::from_shapes(std::slice::from_ref(&o.shape));
        let inside: Vec<usize> = (0..parts.len()).filter(|&i| !used_parts[i]).filter(|&i| {
            let c = parts[i].shape.outer.iter().copied().sum::<Vec2>() / parts[i].shape.outer.len() as f32;
            or.contains(c)
        }).collect();
        let cover = Region::union_all(inside.iter().map(|&i| Region::from_shapes(std::slice::from_ref(&parts[i].shape))).collect::<Vec<_>>().iter()).and(&or);
        if !inside.is_empty() && cover.area() >= 0.95 * or.area() {
            for i in inside {
                used_parts[i] = true;
                let p = &parts[i];
                fps.push(Fp { id: p.id, shape: p.shape.clone(), levels: p.levels.or(o.levels), roof: p.roof.or(o.roof) });
            }
            // Patios of the outline stay open.
            if !o.shape.holes.is_empty() {
                let holes = Region::rings(o.shape.holes.iter().map(|h| h.as_slice()));
                for f in fps.iter_mut().rev().take(8) {
                    let r = Region::from_shapes(std::slice::from_ref(&f.shape)).minus(&holes);
                    if let Some(s) = r.shapes().into_iter().max_by(|a, b| forja_city::geom::signed_area(&a.outer).total_cmp(&forja_city::geom::signed_area(&b.outer))) {
                        f.shape = s;
                    }
                }
            }
        } else {
            fps.push(o);
        }
    }
    // De-overlap in id order, close hairline gaps, clip to the mask.
    fps.sort_by_key(|f| f.id);
    let mask_r = Region::ring(&mask);
    let mut taken = Region::empty();
    let mut regions: Vec<(Region, usize)> = Vec::new();
    for (i, f) in fps.iter().enumerate() {
        let r = Region::from_shapes(std::slice::from_ref(&f.shape)).and(&mask_r).minus(&taken);
        if r.area() < 4.0 {
            continue;
        }
        taken = taken.union(&r);
        regions.push((r, i));
    }
    let gaps = taken.closing(0.15).minus(&taken);
    for g in gaps.components() {
        if g.area() > 2.0 {
            continue;
        }
        // To the lowest id that touches it.
        let grown = g.offset(0.01, forja_city::plan::Join::Bevel);
        if let Some(k) = regions.iter().position(|(r, _)| r.intersects(&grown)) {
            regions[k].0 = regions[k].0.union(&g);
        }
    }
    for (r, i) in regions {
        let f = &fps[i];
        for s in r.shapes() {
            if forja_city::geom::signed_area(&s.outer) < 4.0 {
                continue;
            }
            d.footprints.push(Footprint { ring: s.outer, holes: s.holes, levels: f.levels, roof_levels: f.roof, osm: f.id, zone: 1 });
        }
    }
    measure::measure(&mut d, lib);
    Ok(d)
}

fn point_in(r: &[Vec2], p: Vec2) -> bool {
    let mut c = false;
    let n = r.len();
    for i in 0..n {
        let (a, b) = (r[i], r[(i + 1) % n]);
        if (a.y > p.y) != (b.y > p.y) && p.x < a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x) {
            c = !c;
        }
    }
    c
}

/// Spanish city of an extract: Madrid west of 2° W, else Valencia.
pub fn city_of(data: &OsmData) -> &'static str {
    match data.bbox() {
        Some(b) if (b[0] + b[2]) * 0.5 < -2.0 => "madrid",
        _ => "valencia",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::osm::{parse_osm, parse_osm_json};

    /// A hand-written extract (no real data): two streets crossing without
    /// a shared node, a sidewalk footway and a building passage (both
    /// skipped), a short stub, a detached far street, a pedestrian square,
    /// signals, buildings with parts and a multipolygon with a patio, and a
    /// building cut by the map edge.
    pub const SYNTH_XML: &str = r#"<?xml version="1.0"?>
<osm version="0.6">
 <bounds minlat="39.4760" minlon="-0.3800" maxlat="39.4776" maxlon="-0.3780"/>
 <node id="1" lat="39.4768" lon="-0.37995"/>
 <node id="2" lat="39.4768" lon="-0.37805"/>
 <node id="3" lat="39.47605" lon="-0.3790"/>
 <node id="4" lat="39.47755" lon="-0.3790"/>
 <node id="5" lat="39.47685" lon="-0.3795"/>
 <node id="6" lat="39.47685" lon="-0.3785"/>
 <node id="7" lat="39.4768" lon="-0.3786"><tag k="highway" v="traffic_signals"/></node>
 <node id="8" lat="39.47681" lon="-0.37935"/>
 <node id="9" lat="39.47683" lon="-0.37935"/>
 <node id="10" lat="39.47700" lon="-0.37980"/>
 <node id="11" lat="39.47700" lon="-0.37920"/>
 <node id="12" lat="39.47740" lon="-0.37920"/>
 <node id="13" lat="39.47740" lon="-0.37980"/>
 <node id="14" lat="39.47710" lon="-0.37970"/>
 <node id="15" lat="39.47710" lon="-0.37930"/>
 <node id="16" lat="39.47730" lon="-0.37930"/>
 <node id="17" lat="39.47730" lon="-0.37970"/>
 <node id="18" lat="39.47700" lon="-0.37950"/>
 <node id="19" lat="39.47740" lon="-0.37950"/>
 <node id="20" lat="39.47610" lon="-0.37870"/>
 <node id="21" lat="39.47610" lon="-0.37820"/>
 <node id="22" lat="39.47650" lon="-0.37820"/>
 <node id="23" lat="39.47650" lon="-0.37870"/>
 <node id="24" lat="39.47700" lon="-0.38020"/>
 <node id="25" lat="39.47740" lon="-0.38020"/>
 <node id="30" lat="39.4750" lon="-0.3700"/>
 <node id="31" lat="39.4751" lon="-0.3700"/>
 <node id="40" lat="39.47640" lon="-0.37960"/>
 <node id="41" lat="39.47640" lon="-0.37920"/>
 <node id="42" lat="39.47660" lon="-0.37920"/>
 <node id="43" lat="39.47660" lon="-0.37960"/>
 <way id="100"><nd ref="1"/><nd ref="7"/><nd ref="2"/><tag k="highway" v="residential"/><tag k="name" v="Carrer de Dalt"/></way>
 <way id="101"><nd ref="3"/><nd ref="4"/><tag k="highway" v="pedestrian"/></way>
 <way id="102"><nd ref="5"/><nd ref="6"/><tag k="highway" v="footway"/><tag k="footway" v="sidewalk"/></way>
 <way id="103"><nd ref="8"/><nd ref="9"/><tag k="highway" v="footway"/></way>
 <way id="104"><nd ref="30"/><nd ref="31"/><tag k="highway" v="residential"/></way>
 <way id="105"><nd ref="5"/><nd ref="1"/><tag k="highway" v="footway"/><tag k="tunnel" v="building_passage"/></way>
 <way id="200"><nd ref="10"/><nd ref="18"/><nd ref="19"/><nd ref="13"/><nd ref="10"/><tag k="building:part" v="yes"/><tag k="building:levels" v="3"/></way>
 <way id="201"><nd ref="18"/><nd ref="11"/><nd ref="12"/><nd ref="19"/><nd ref="18"/><tag k="building:part" v="yes"/><tag k="building:levels" v="5"/><tag k="roof:levels" v="1"/></way>
 <way id="202"><nd ref="10"/><nd ref="11"/><nd ref="12"/><nd ref="13"/><nd ref="10"/><tag k="building" v="yes"/><tag k="building:levels" v="4"/></way>
 <way id="210"><nd ref="20"/><nd ref="21"/><nd ref="22"/><nd ref="23"/><nd ref="20"/></way>
 <way id="211"><nd ref="14"/><nd ref="15"/><nd ref="16"/><nd ref="17"/><nd ref="14"/></way>
 <way id="212"><nd ref="24"/><nd ref="10"/><nd ref="13"/><nd ref="25"/><nd ref="24"/><tag k="building" v="yes"/></way>
 <way id="220"><nd ref="40"/><nd ref="41"/><nd ref="42"/><nd ref="43"/><nd ref="40"/><tag k="highway" v="pedestrian"/><tag k="area" v="yes"/></way>
 <relation id="300"><member type="way" ref="210" role="outer"/><tag k="type" v="multipolygon"/><tag k="building" v="yes"/></relation>
</osm>"#;

    #[test]
    fn xml_and_json_agree_and_malformed_input_errors() {
        let x = parse_osm(SYNTH_XML).unwrap();
        assert_eq!(x.ways.len(), 13);
        assert_eq!(x.relations.len(), 1);
        assert!(x.node_tags[&7].get("highway").is_some_and(|v| v == "traffic_signals"));
        let json = r#"{"version":"0.6","bounds":{"minlat":39.4760,"minlon":-0.3800,"maxlat":39.4776,"maxlon":-0.3780},"elements":[
            {"type":"node","id":1,"lat":39.4768,"lon":-0.37995},{"type":"node","id":7,"lat":39.4768,"lon":-0.3786,"tags":{"highway":"traffic_signals"}},
            {"type":"way","id":100,"nodes":[1,7],"tags":{"highway":"residential"}},
            {"type":"relation","id":300,"members":[{"type":"way","ref":210,"role":"outer"}],"tags":{"type":"multipolygon"}}]}"#;
        let j = parse_osm_json(json).unwrap();
        assert_eq!(j.ways[0].refs, vec![1, 7]);
        assert_eq!(j.node_tags[&7].get("highway").map(String::as_str), Some("traffic_signals"));
        assert_eq!(j.relations[0].members.len(), 1);
        assert_eq!(j.nodes[&1], x.nodes[&1]);
        for bad in ["", "{", "{\"elements\": 3}", "{\"elements\":[{\"type\":\"node\",\"id\":1,\"lat\":1e999,\"lon\":0}]}"] {
            let r = parse_osm_json(bad);
            assert!(r.is_err() || r.unwrap().nodes.is_empty());
        }
        assert!(parse_osm("<osm><node id='1' lat='NaN' lon='0'/></osm>").map(|d| d.nodes.is_empty()).unwrap_or(true));
    }

    #[test]
    fn synthetic_extract_becomes_a_draft() {
        let data = parse_osm(SYNTH_XML).unwrap();
        let lib = ProfileLibrary::builtin();
        let d = draft(&data, &lib, &OsmOptions::default()).unwrap();
        // Sidewalk, passage, far street skipped; stub kept in the draft (the
        // graph prunes it).
        let ids: Vec<i64> = d.ways.iter().filter_map(|w| w.osm).collect();
        assert!(ids.contains(&100) && ids.contains(&101) && !ids.contains(&102) && !ids.contains(&105) && !ids.contains(&104));
        assert!(d.ways.iter().all(|w| lib.id(&w.profile).is_some()), "{:?}", d.ways.iter().map(|w| &w.profile).collect::<Vec<_>>());
        assert_eq!(d.plazas.len(), 1);
        assert!(d.nodes.iter().any(|n| n.signals));
        // Parts replace their outline: two footprints with 3 and 5 levels.
        let levels: Vec<Option<u32>> = d.footprints.iter().map(|f| f.levels).collect();
        assert!(levels.contains(&Some(3)) && levels.contains(&Some(5)) && !levels.contains(&Some(4)), "{levels:?}");
        assert!(d.footprints.iter().any(|f| f.roof_levels == Some(1)));
        // No footprint overlaps another.
        let total: f64 = d.footprints.iter().map(|f| Region::from_shapes(&[Shape { outer: f.ring.clone(), holes: f.holes.clone() }]).area()).sum();
        let union = Region::union_all(d.footprints.iter().map(|f| Region::from_shapes(&[Shape { outer: f.ring.clone(), holes: f.holes.clone() }])).collect::<Vec<_>>().iter());
        assert!((union.area() - total).abs() < 0.5);
    }
}
