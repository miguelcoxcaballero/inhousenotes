//! Street widths measured from the gaps between real footprints.
//!
//! OpenStreetMap rarely tags widths (one of about 127 ways in the Carmen
//! tile has one), but the buildings are mapped: the clear width of a
//! street is the gap between the facades on either side. Every 2 m along
//! each centreline, rays cast left and right to the first footprint edge
//! give the clearance. Its median picks the profile (`[[osm]]` table in
//! `streets.toml`), its smoothed value the per-vertex width, and its 10th
//! percentile how much roadway fits (shared surface when under 2.6 m).
//! Centrelines drawn off-centre are moved to the middle of the gap, except
//! at vertices other ways share (junctions stay put).

use std::collections::HashMap;

use forja_math::Vec2;

use crate::plan::{Draft, Polyline, SegIndex};
use crate::profile::ProfileLibrary;

/// Profile placeholder written by importers: `osm:<highway>`.
pub const OSM_PREFIX: &str = "osm:";

/// Narrowest roadway that still takes a car (m).
pub const MIN_CARRIAGE: f32 = 2.6;

fn quant(p: Vec2) -> (i64, i64) {
    ((p.x * 100.0).round() as i64, (p.y * 100.0).round() as i64)
}

/// Resolves every `osm:<highway>` way of the draft into a real profile
/// with measured widths and roadway.
pub fn measure(d: &mut Draft, lib: &ProfileLibrary) -> Vec<(String, f32, f32)> {
    let mut fp = SegIndex::new(8.0);
    for f in &d.footprints {
        for ring in std::iter::once(&f.ring).chain(f.holes.iter()) {
            for k in 0..ring.len() {
                fp.push(ring[k], ring[(k + 1) % ring.len()], 0);
            }
        }
    }
    // Vertices used by more than one way (or twice) are junctions.
    let mut uses: HashMap<(i64, i64), u32> = HashMap::new();
    for w in &d.ways {
        for p in &w.line {
            *uses.entry(quant(*p)).or_default() += 1;
        }
    }
    let mut report = Vec::new();
    for w in d.ways.iter_mut() {
        let Some(highway) = w.profile.strip_prefix(OSM_PREFIX).map(str::to_string) else { continue };
        if w.line.len() < 2 {
            continue;
        }
        let fixed: Vec<bool> = w.line.iter().enumerate().map(|(i, p)| i == 0 || i + 1 == w.line.len() || uses.get(&quant(*p)).copied().unwrap_or(0) > 1).collect();
        // Resample to 3 m chords (new vertices are free to move).
        let mut pts: Vec<Vec2> = Vec::new();
        let mut fix: Vec<bool> = Vec::new();
        for i in 0..w.line.len() {
            if i > 0 {
                let (a, b) = (w.line[i - 1], w.line[i]);
                let k = (a.distance(b) / 3.0).ceil().max(1.0) as usize;
                for j in 1..k {
                    pts.push(a.lerp(b, j as f32 / k as f32));
                    fix.push(false);
                }
            }
            pts.push(w.line[i]);
            fix.push(fixed[i]);
        }
        let given = w.width.clone();
        let rays = |pts: &[Vec2]| -> Vec<(Option<f32>, Option<f32>)> {
            let pl = Polyline::new(pts.to_vec());
            (0..pts.len())
                .map(|i| {
                    let n = pl.vertex_normal(i).normalize_or(Vec2::Y);
                    let p = pts[i];
                    (fp.ray(p, n, 30.0).map(|h| h.0), fp.ray(p, -n, 30.0).map(|h| h.0))
                })
                .collect()
        };
        // Centre free vertices between the facades (smoothed shifts).
        let r0 = rays(&pts);
        let pl = Polyline::new(pts.clone());
        let shift: Vec<f32> = r0.iter().map(|(l, r)| match (l, r) {
            (Some(l), Some(r)) if (l - r).abs() < 5.0 => (l - r) * 0.5,
            _ => 0.0,
        }).collect();
        let n = pts.len();
        for i in 0..n {
            if fix[i] {
                continue;
            }
            let s = (shift[i.saturating_sub(1)] + 2.0 * shift[i] + shift[(i + 1).min(n - 1)]) * 0.25;
            pts[i] += pl.vertex_normal(i).normalize_or(Vec2::Y) * s;
        }
        let r1 = rays(&pts);
        let clear: Vec<Option<f32>> = r1.iter().map(|(l, r)| match (l, r) {
            (Some(l), Some(r)) => Some((l + r).min(30.0)),
            _ => None,
        }).collect();
        let mut samples: Vec<f32> = clear.iter().flatten().copied().collect();
        samples.sort_by(|a, b| a.total_cmp(b));
        let median = samples.get(samples.len() / 2).copied().unwrap_or(8.0);
        let p10 = samples.get(samples.len() / 10).copied().unwrap_or(median);
        let Some(pid) = lib.by_clearance(&highway, median).or_else(|| lib.by_clearance("residential", median)) else { continue };
        let prof = lib.get(pid);
        // Width per vertex: the gap, never wider than twice the nearer
        // facade (off-centre junction vertices), else the profile default.
        let raw: Vec<f32> = (0..n)
            .map(|i| match (clear[i], r1[i]) {
                (Some(c), (Some(l), Some(r))) => c.min(2.0 * l.min(r) + 0.6).max(1.8),
                _ => prof.default_width(),
            })
            .collect();
        let width: Vec<f32> = (0..n).map(|i| (raw[i.saturating_sub(1)] + 2.0 * raw[i] + raw[(i + 1).min(n - 1)]) * 0.25).collect();
        // Tagged widths win: the whole street on foot ways, the roadway on
        // car ways.
        let pedestrian = matches!(highway.as_str(), "pedestrian" | "footway" | "path" | "steps");
        let tagged = given.first().copied().filter(|w| w.is_finite() && *w > 0.5);
        w.width = match (tagged, pedestrian) {
            (Some(t), true) => vec![t; n],
            _ => width,
        };
        if prof.vehicles {
            let fit = (p10 - 2.0 * prof.walk_min).max(0.0);
            let c = match (tagged, pedestrian) {
                (Some(t), false) => t,
                _ => prof.carriageway.min(fit),
            };
            w.carriageway = Some(if c < MIN_CARRIAGE { 0.0 } else { c });
        }
        w.line = pts;
        w.profile = prof.id.clone();
        report.push((highway, median, p10));
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::{Footprint, Way};

    #[test]
    fn a_three_metre_gap_becomes_a_callejon_centred_between_facades() {
        let lib = ProfileLibrary::builtin();
        let rect = |x0: f32, z0: f32, x1: f32, z1: f32| vec![Vec2::new(x0, z0), Vec2::new(x1, z0), Vec2::new(x1, z1), Vec2::new(x0, z1)];
        let mut d = Draft { snap: 0.3, ..Default::default() };
        d.footprints.push(Footprint { ring: rect(0.0, -20.0, 60.0, -1.5), holes: vec![], levels: None, roof_levels: None, osm: 1, zone: 1 });
        d.footprints.push(Footprint { ring: rect(0.0, 1.5, 60.0, 20.0), holes: vec![], levels: None, roof_levels: None, osm: 2, zone: 1 });
        // Drawn 0.8 m off-centre.
        d.ways.push(Way::new(vec![Vec2::new(-5.0, 0.8), Vec2::new(65.0, 0.8)], "osm:footway"));
        let rep = measure(&mut d, &lib);
        assert!((rep[0].1 - 3.0).abs() < 0.05, "{rep:?}");
        let w = &d.ways[0];
        assert_eq!(w.profile, "callejon");
        // Interior vertices moved to the middle; the ends stay.
        let mid = w.line[w.line.len() / 2];
        assert!(mid.y.abs() < 0.2, "{mid}");
        assert!(w.width.iter().skip(2).take(w.width.len() - 4).all(|x| (x - 3.0).abs() < 0.4), "{:?}", w.width);

    }

    #[test]
    fn narrow_car_streets_become_shared() {
        let lib = ProfileLibrary::builtin();
        let rect = |x0: f32, z0: f32, x1: f32, z1: f32| vec![Vec2::new(x0, z0), Vec2::new(x1, z0), Vec2::new(x1, z1), Vec2::new(x0, z1)];
        let mut d = Draft { snap: 0.3, ..Default::default() };
        d.footprints.push(Footprint { ring: rect(0.0, -20.0, 60.0, -2.4), holes: vec![], levels: None, roof_levels: None, osm: 1, zone: 1 });
        d.footprints.push(Footprint { ring: rect(0.0, 2.4, 60.0, 20.0), holes: vec![], levels: None, roof_levels: None, osm: 2, zone: 1 });
        d.ways.push(Way::new(vec![Vec2::new(-5.0, 0.0), Vec2::new(65.0, 0.0)], "osm:residential"));
        measure(&mut d, &lib);
        let w = &d.ways[0];
        assert_eq!(w.profile, "callejuela_aceras");
        assert_eq!(w.carriageway, Some(2.8));
    }
}
