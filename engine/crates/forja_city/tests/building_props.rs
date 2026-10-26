//! Building guarantees (B1-B9 of the lot contract) on hand-made lots of
//! every shape the layout can hand out: alleys, curves, sharp and rounded
//! corners, re-entrant fronts, through-lots, plazas, OSM footprints and
//! the smallest plots. No layout code is involved.

use forja_city::building::facade::{self, end_clearance};
use forja_city::building::roof::{Roof, Slope};
use forja_city::building::{self, CityOut, EdgeKind, Lot, ModuleLib, point_in};
use forja_city::geom::Parts;
use forja_city::kit::{Kit, Mat};
use forja_city::lot::SKIRT;
use forja_city::style::{Style, StyleLibrary};
use forja_city::{Front, FrontRules, Rhythm};
use forja_math::{Mat4, Vec2, Vec3};

use EdgeKind::*;

// ------------------------------------------------------------- fixtures

fn v(x: f32, z: f32) -> Vec2 {
    Vec2::new(x, z)
}

/// Front on a street `clear` metres wide with `walk` metres to walk on.
fn front(clear: f32, walk: f32) -> Front {
    FrontRules::default().front(clear, walk)
}

/// A lot whose street-facing edges all face `fr` (with `along` filled
/// from `along`, a function of the point) and whose other edges are
/// unconstrained.
fn lot_with(ring: Vec<Vec2>, edges: Vec<EdgeKind>, fr: Front, along: impl Fn(Vec2) -> f32) -> Lot {
    let n = ring.len();
    let fronts = (0..n)
        .map(|i| {
            if edges[i].is_street() {
                Front { along: [along(ring[i]), along(ring[(i + 1) % n])], ..fr }
            } else {
                Front::UNCONSTRAINED
            }
        })
        .collect();
    let lot = Lot { ring, edges, base: 0.15, fronts, levels: None, roof_levels: None };
    lot.check().unwrap_or_else(|e| panic!("fixture lot fails the contract: {e}: {lot:?}"));
    lot
}

/// Two rows of five houses (4-7 m wide) facing across a 3 m alley along
/// x, lit from the facades every 14 m, the rows' lanterns staggered.
fn alley_pair() -> Vec<Lot> {
    let widths = [4.5, 6.2, 4.0, 7.0, 5.3];
    let mut lots = Vec::new();
    for (row, (depth, phase)) in [(12.0f32, 0.0f32), (10.5, 7.0)].into_iter().enumerate() {
        let mut x = 0.0;
        for (k, w) in widths.iter().enumerate() {
            let w = if row == 0 { *w } else { widths[(k + 2) % 5] };
            let (x0, x1) = (x, x + w);
            let fr = Front { lamps: Some(Rhythm { pitch: 14.0, phase }), depth, way: 1, ..front(3.0, 1.5) };
            let lot = if row == 0 {
                // South row: street edge at z = 0, facing +z.
                lot_with(vec![v(x0, -depth), v(x1, -depth), v(x1, 0.0), v(x0, 0.0)], vec![Courtyard, Party, Street, Party], fr, |p| p.x)
            } else {
                // North row: street edge at z = 3, facing -z.
                lot_with(vec![v(x0, 3.0), v(x1, 3.0), v(x1, 3.0 + depth), v(x0, 3.0 + depth)], vec![Street, Party, Courtyard, Party], fr, |p| p.x)
            };
            lots.push(lot);
            x = x1;
        }
    }
    lots
}

/// Lots on both sides of a lane curving at R = 25 m (centreline), 6 m
/// wide, frontages given as chords of at most 3 m: the inner row has a
/// convex front, the outer row a concave one.
fn curved_rows() -> Vec<Lot> {
    let (rc, half) = (25.0f32, 3.0f32);
    let at = |a: f32, r: f32| v(a.sin(), -a.cos()) * r;
    let along = |p: Vec2| 100.0 + rc * p.x.atan2(-p.y);
    let spans = [(-0.6f32, -0.38f32), (-0.38, -0.1), (-0.1, 0.12), (0.12, 0.4)];
    let mut lots = Vec::new();
    for (k, &(a0, a1)) in spans.iter().enumerate() {
        // Inner row: front on r = 22, back on r = 10.
        let r = rc - half;
        let m = ((a1 - a0) * r / 2.8).ceil() as usize;
        let mut ring: Vec<Vec2> = (0..=m).map(|i| at(a0 + (a1 - a0) * i as f32 / m as f32, r)).collect();
        let mut edges = vec![Street; m];
        ring.extend([at(a1, 10.0), at(a0, 10.0)]);
        edges.extend([Party, Courtyard, Party]);
        let fr = Front { lamps: Some(Rhythm { pitch: 14.0, phase: 0.0 }), depth: 12.0, ..front(6.0, 1.6) };
        lots.push(lot_with(ring, edges, fr, along));
        // Outer row: front on r = 28 (concave), back on r = 40; shifted.
        let (b0, b1) = (a0 + 0.05 * k as f32, a1 + 0.05 * k as f32);
        let r = rc + half;
        let m = ((b1 - b0) * r / 2.8).ceil() as usize;
        let mut ring: Vec<Vec2> = (0..=m).map(|i| at(b1 - (b1 - b0) * i as f32 / m as f32, r)).collect();
        let mut edges = vec![Street; m];
        ring.extend([at(b0, 40.0), at(b1, 40.0)]);
        edges.extend([Party, Courtyard, Party]);
        let fr = Front { lamps: Some(Rhythm { pitch: 14.0, phase: 7.0 }), depth: 12.0, ..front(6.0, 1.6) };
        lots.push(lot_with(ring, edges, fr, along));
    }
    lots
}

/// A 60° street corner bevelled to a 2.5 m tip (the chamfer meets both
/// fronts at 120°).
fn acute_corner() -> Vec<Lot> {
    let u2 = v(0.5, 3f32.sqrt() * 0.5);
    let inward2 = v(u2.y, -u2.x);
    let (p1, q1, r1) = (v(2.5, 0.0), v(16.0, 0.0), v(16.0, 7.0));
    let (p2, q2) = (u2 * 2.5, u2 * 16.0);
    let r2 = q2 + inward2 * 7.0;
    vec![lot_with(vec![p1, q1, r1, r2, q2, p2], vec![Street, Party, Courtyard, Party, Street, Corner], front(11.0, 2.0), |p| p.length())]
}

/// A 90° Ensanche corner rounded by six 15° chords of radius 9, wrapped
/// by one lot.
fn rounded_corner() -> Vec<Lot> {
    let mut ring = vec![v(0.0, 0.0), v(20.0, 0.0), v(20.0, 11.0)];
    let mut edges = vec![Party, Street];
    let c = v(11.0, 11.0);
    for k in 0..6 {
        let a = (k as f32 + 1.0) * 15f32.to_radians();
        ring.push(c + v(a.cos(), a.sin()) * 9.0);
        edges.push(Corner);
    }
    edges.push(Street);
    ring.push(v(0.0, 20.0));
    edges.push(Party);
    vec![lot_with(ring, edges, front(20.0, 4.5), |p| p.x + p.y)]
}

/// A lot whose frontage turns 270°: an inside corner on a small square.
fn reentrant_l() -> Vec<Lot> {
    vec![lot_with(
        vec![v(0.0, 0.0), v(16.0, 0.0), v(16.0, 8.0), v(8.0, 8.0), v(8.0, 16.0), v(0.0, 16.0)],
        vec![Party, Party, Street, Street, Party, Courtyard],
        Front { depth: 8.0, ..front(8.0, 2.0) },
        |p| p.x + p.y,
    )]
}

/// 5 m x 9 m with an alley at both ends.
fn through_lot() -> Vec<Lot> {
    vec![lot_with(vec![v(0.0, 0.0), v(5.0, 0.0), v(5.0, 9.0), v(0.0, 9.0)], vec![Street, Party, Street, Party], Front { depth: 9.0, ..front(4.0, 2.0) }, |p| p.x)]
}

/// A house on a 24 m plaza.
fn plaza_lot() -> Vec<Lot> {
    vec![lot_with(vec![v(0.0, 0.0), v(12.0, 0.0), v(12.0, 14.0), v(0.0, 14.0)], vec![Courtyard, Party, Street, Party], Front { open: true, ..front(24.0, 6.0) }, |p| p.x)]
}

/// An irregular OSM footprint with known storeys and a roof storey.
fn osm_like() -> Vec<Lot> {
    let ring = vec![v(0.0, 0.0), v(6.0, 0.4), v(12.5, 1.5), v(12.0, 7.5), v(13.5, 12.0), v(8.0, 12.5), v(5.0, 9.5), v(1.0, 11.0), v(-0.5, 5.0)];
    let edges = vec![Street, Street, Party, Party, Courtyard, Courtyard, Party, Party, Party];
    let mut lot = lot_with(ring, edges, front(9.0, 2.0), |p| p.x);
    lot.levels = Some(5);
    lot.roof_levels = Some(1);
    lot.check().unwrap();
    vec![lot]
}

/// The smallest plot: a 3.5 m front, 30 m².
fn minimal() -> Vec<Lot> {
    vec![lot_with(vec![v(0.0, 0.0), v(3.5, 0.0), v(3.5, 8.6), v(0.0, 8.6)], vec![Courtyard, Party, Street, Party], front(3.0, 1.5), |p| p.x)]
}

fn fixtures() -> Vec<(&'static str, Vec<Lot>)> {
    vec![
        ("alley_pair", alley_pair()),
        ("curved_rows", curved_rows()),
        ("acute_corner", acute_corner()),
        ("rounded_corner", rounded_corner()),
        ("reentrant_l", reentrant_l()),
        ("through_lot", through_lot()),
        ("plaza_lot", plaza_lot()),
        ("osm_like", osm_like()),
        ("minimal", minimal()),
    ]
}

// ---------------------------------------------------------------- build

struct Built {
    out: CityOut,
    /// Per lot: its own geometry, its placements and its eave.
    parts: Vec<Parts<Mat>>,
    placements: Vec<std::ops::Range<usize>>,
    eaves: Vec<f32>,
    party: Vec<std::ops::Range<usize>>,
}

fn build(kit: &Kit, lots: &[Lot], style: &Style, seed: u64) -> Built {
    let mut lib = ModuleLib::new(kit);
    let mut out = CityOut::default();
    let mut b = Built { out: CityOut::default(), parts: Vec::new(), placements: Vec::new(), eaves: Vec::new(), party: Vec::new() };
    for (k, lot) in lots.iter().enumerate() {
        let mut parts = Parts::new();
        let (p0, w0) = (out.placements.len(), out.party_walls.len());
        let eave = building::build_into(&mut lib, &mut out, &mut parts, lot, style, seed * 1000 + k as u64);
        b.placements.push(p0..out.placements.len());
        b.party.push(w0..out.party_walls.len());
        b.parts.push(parts);
        b.eaves.push(eave);
    }
    b.out = out;
    b
}

/// World-space corners of a placed module's local bounds.
fn corners(out: &CityOut, si: usize, m: &Mat4) -> Vec<Vec3> {
    let bb = out.scenes[si].bounds();
    let mut c = Vec::with_capacity(8);
    for i in 0..8 {
        let p = Vec3::new(if i & 1 == 0 { bb.min.x } else { bb.max.x }, if i & 2 == 0 { bb.min.y } else { bb.max.y }, if i & 4 == 0 { bb.min.z } else { bb.max.z });
        c.push(m.transform_point3(p));
    }
    c
}

fn seg_dist(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let d = b - a;
    let t = ((p - a).dot(d) / d.length_squared().max(1e-12)).clamp(0.0, 1.0);
    p.distance(a + d * t)
}

/// If `p` is outside the lot: the nearest edge and how far out it is.
fn outside(lot: &Lot, p: Vec2) -> Option<(usize, f32)> {
    if point_in(&lot.ring, p) {
        return None;
    }
    // On a tie (a point beyond a corner) the street edge counts, then the
    // patio: a point on a party wall's line does not cross it.
    let key = |i: usize| (seg_dist(p, lot.edge(i).0, lot.edge(i).1), match lot.edges[i] { Street | Corner => 0, Courtyard => 1, Party => 2 });
    (0..lot.ring.len()).min_by(|&a, &b| {
        let (ka, kb) = (key(a), key(b));
        if (ka.0 - kb.0).abs() < 1e-4 { ka.1.cmp(&kb.1) } else { ka.0.total_cmp(&kb.0) }
    }).map(|i| (i, key(i).0))
}

fn has_mat(out: &CityOut, si: usize, name: &str) -> bool {
    out.scenes[si].materials.iter().any(|m| m.name.contains(name))
}

/// A glazed mirador (window glass standing out of the wall).
fn is_mirador(out: &CityOut, si: usize) -> bool {
    let sc = &out.scenes[si];
    sc.name == "window" && has_mat(out, si, "glass") && sc.bounds().max.z > 0.5
}

/// A shop or balcony awning (laundry uses the same striped cloth).
fn is_awning(out: &CityOut, si: usize) -> bool {
    matches!(out.scenes[si].name.as_str(), "shop" | "window") && has_mat(out, si, "awning")
}

/// The front the nearest street edge of a point outside faces.
fn allowance(fr: &Front, ground: bool) -> f32 {
    let up = fr.reach + 0.15;
    if ground && fr.walk >= 1.5 { up.max(0.75) } else { up }
}

// ------------------------------------------------------------ guarantees

/// B1-B9 for every lot of a build.
fn check(name: &str, lots: &[Lot], style: &Style, seed: u64, b: &Built) {
    let ctx = |k: usize| format!("{name}[{k}] {} seed {seed}", style.id);
    let tol = 0.02;
    for (k, layout_lot) in lots.iter().enumerate() {
        // Measured on the lot actually built (its fitted fronts stray from
        // the layout's by at most FIT_OUTWARD over the street, tested apart).
        let fitted = building::plan(layout_lot, style, seed * 1000 + k as u64).lot;
        let lot = &fitted;
        let parts = &b.parts[k];
        let base = lot.base;
        let eave = b.eaves[k];
        // B1: finite everything.
        assert!(eave.is_finite() && eave > base, "{}: eave {eave}", ctx(k));
        for p in parts.prims.values() {
            assert!(p.positions.iter().chain(&p.normals).all(|q| q.is_finite()), "{}: non-finite vertex", ctx(k));
            assert!(p.uvs.iter().all(|q| q.is_finite()), "{}: non-finite uv", ctx(k));
        }
        for (_, m) in &b.out.placements[b.placements[k].clone()] {
            assert!(m.is_finite(), "{}: non-finite placement", ctx(k));
        }
        // B2 and B3: what stands outside the lot stays within the street's
        // reach (or on its own side of a party wall).
        let ground_top = base + style.ground_height + 0.01;
        let test = |p: Vec3, ground: bool, what: &str| {
            let q = Vec2::new(p.x, p.z);
            let Some((i, d)) = outside(lot, q) else { return };
            if lot.edges[i].is_street() {
                let limit = allowance(&lot.front(i), ground);
                assert!(d <= limit + tol, "{}: {what} at {p:?} is {d:.2} m out of edge {i} (limit {limit:.2})", ctx(k));
            }
            // How far it stands beyond any party wall, alongside it.
            for j in (0..lot.ring.len()).filter(|&j| lot.edges[j] == Party) {
                let (a, b) = lot.edge(j);
                let dir = (b - a).normalize();
                let t = (q - a).dot(dir) / a.distance(b);
                let beyond = (q - a).perp_dot(dir);
                assert!(!(t > 0.0 && t < 1.0 && beyond > 0.05 + tol && beyond < 1.0), "{}: {what} at {p:?} crosses party edge {j} by {beyond:.2} m", ctx(k));
            }
        };
        for (mat, p) in &parts.prims {
            for q in &p.positions {
                test(*q, q.y <= ground_top, &format!("{mat:?} vertex"));
            }
        }
        for (si, m) in &b.out.placements[b.placements[k].clone()] {
            let awning = is_awning(&b.out, *si);
            let ground = m.w_axis.y <= ground_top;
            if awning {
                // Awnings only where the street takes them.
                let p = m.w_axis.truncate();
                let i = (0..lot.ring.len()).filter(|&i| lot.edges[i].is_street()).min_by(|&a, &c| seg_dist(Vec2::new(p.x, p.z), lot.edge(a).0, lot.edge(a).1).total_cmp(&seg_dist(Vec2::new(p.x, p.z), lot.edge(c).0, lot.edge(c).1)));
                assert!(i.is_some_and(|i| lot.front(i).awnings), "{}: awning on a front without room for it", ctx(k));
                continue;
            }
            for c in corners(&b.out, *si, m) {
                test(c, ground, &b.out.scenes[*si].name);
            }
        }
        // B4: walls start below the base, colliders at it.
        let low = parts.prims.values().flat_map(|p| p.positions.iter()).map(|q| q.y).fold(f32::INFINITY, f32::min);
        assert!((low - (base - SKIRT)).abs() < 1e-3, "{}: lowest wall at {low}", ctx(k));
        // B5: storeys.
        let floors = b.out.party_walls[b.party[k].clone()].first().map(|w| w.floors.len());
        if let (Some(l), Some(f)) = (lot.levels, floors) {
            assert_eq!(f as u32, l - 1, "{}: storeys", ctx(k));
        } else if let Some(f) = floors {
            let max_h = (0..lot.ring.len()).filter(|&i| lot.edges[i].is_street()).map(|i| lot.front(i).max_height).fold(0.0f32, f32::max);
            assert!(f == 1 || eave - base <= max_h + 1e-3, "{}: {f} floors, {:.1} m on a street allowing {max_h:.1}", ctx(k), eave - base);
        }
        if lot.roof_levels.is_some_and(|r| r > 0) {
            let mansard = parts.prims.get(&Mat::RoofSlate).is_some_and(|p| !p.positions.is_empty());
            let dormers = b.out.placements[b.placements[k].clone()].iter().any(|(si, _)| b.out.scenes[*si].name.starts_with("dormer_"));
            assert!(mansard || dormers, "{}: roof storey without mansard or dormers", ctx(k));
        }
        // B6: no mirador where the street has no room for one; the street
        // door no wider than it allows.
        for (si, m) in &b.out.placements[b.placements[k].clone()] {
            let sc = &b.out.scenes[*si];
            let p = m.w_axis.truncate();
            let near = (0..lot.ring.len()).filter(|&i| lot.edges[i].is_street()).min_by(|&a, &c| seg_dist(Vec2::new(p.x, p.z), lot.edge(a).0, lot.edge(a).1).total_cmp(&seg_dist(Vec2::new(p.x, p.z), lot.edge(c).0, lot.edge(c).1)));
            let Some(i) = near else { continue };
            let fr = lot.front(i);
            if is_mirador(&b.out, *si) {
                assert!(fr.miradores || fr.reach >= 0.75, "{}: mirador on a {:.1} m street", ctx(k), fr.clear);
            }
            if sc.name == "portal" {
                let w = sc.bounds().max.x - sc.bounds().min.x - 2.0 * 0.27;
                assert!(w <= fr.portal + 0.01, "{}: portal {w:.2} m on a front allowing {:.2}", ctx(k), fr.portal);
            }
        }
        // B8: above the cornice, everything (towers, roofs, ático) stands
        // inside the lot (eaves may overhang a patio).
        for (mat, p) in &parts.prims {
            for q in p.positions.iter().filter(|q| q.y > eave + 0.9) {
                if let Some((i, d)) = outside(lot, Vec2::new(q.x, q.z)) {
                    // (eaves may overhang a patio)
                    let limit = if lot.edges[i] == Courtyard { 0.5 } else { 0.05 };
                    assert!(d <= limit, "{}: {mat:?} at {q:?} above the eave is {d:.2} m out over edge {i}", ctx(k));
                }
            }
        }
    }
}

#[test]
fn every_fixture_every_style_keeps_the_guarantees() {
    let kit = Kit::load(None);
    let styles = StyleLibrary::builtin();
    for (name, lots) in fixtures() {
        for style in &styles.styles {
            for seed in 0..3 {
                let b = build(&kit, &lots, style, seed);
                check(name, &lots, style, seed, &b);
            }
        }
    }
}

#[test]
fn buildings_are_deterministic() {
    // B9.
    let kit = Kit::load(None);
    let styles = StyleLibrary::builtin();
    for (name, lots) in fixtures() {
        for style in styles.styles.iter().step_by(2) {
            let (a, b) = (build(&kit, &lots, style, 7), build(&kit, &lots, style, 7));
            assert_eq!(a.eaves, b.eaves, "{name} {}", style.id);
            let pa: Vec<_> = a.out.placements.iter().map(|(s, m)| (*s, m.to_cols_array())).collect();
            let pb: Vec<_> = b.out.placements.iter().map(|(s, m)| (*s, m.to_cols_array())).collect();
            assert_eq!(pa, pb, "{name} {}", style.id);
            for (x, y) in a.parts.iter().zip(&b.parts) {
                let mut kx: Vec<_> = x.prims.iter().map(|(m, p)| (format!("{m:?}"), p.positions.clone())).collect();
                let mut ky: Vec<_> = y.prims.iter().map(|(m, p)| (format!("{m:?}"), p.positions.clone())).collect();
                kx.sort_by(|a, b| a.0.cmp(&b.0));
                ky.sort_by(|a, b| a.0.cmp(&b.0));
                assert_eq!(kx, ky, "{name} {}", style.id);
            }
        }
    }
}

#[test]
fn neighbours_never_interpenetrate_across_party_walls() {
    // B3 on a whole street: no wall vertex or module of one house inside
    // its neighbour below the neighbour's eave, medianeras included.
    let kit = Kit::load(None);
    let styles = StyleLibrary::builtin();
    let lots = alley_pair();
    for style in &styles.styles {
        let mut b = build(&kit, &lots, style, 1);
        let mut lib = ModuleLib::new(&kit);
        let shrink = |ring: &[Vec2]| building::inset(ring, 0.05).unwrap();
        // Each lot's medianeras on their own, so we know whose they are.
        let all = b.out.party_walls.clone();
        let mut walls: Vec<Parts<Mat>> = Vec::new();
        for k in 0..lots.len() {
            let mine: Vec<_> = all[b.party[k].clone()].to_vec();
            let rest: Vec<_> = all.iter().enumerate().filter(|(i, _)| !b.party[k].contains(i)).map(|(_, w)| w.clone()).collect();
            b.out.party_walls = rest.into_iter().chain(mine.iter().cloned()).collect();
            let mut parts = Parts::new();
            building::medianeras(&mut lib, &mut b.out, &mut parts, all.len() - mine.len());
            walls.push(parts);
        }
        for k in 0..lots.len() {
            for (j, other) in lots.iter().enumerate() {
                if j == k {
                    continue;
                }
                let inner = shrink(&other.ring);
                let inside = |p: Vec3| p.y < b.eaves[j] - 0.01 && point_in(&inner, Vec2::new(p.x, p.z));
                for p in b.parts[k].prims.values().chain(walls[k].prims.values()) {
                    for q in &p.positions {
                        assert!(!inside(*q), "{}: lot {k} vertex {q:?} inside lot {j}", style.id);
                    }
                }
                for (_, m) in &b.out.placements[b.placements[k].clone()] {
                    assert!(!inside(m.w_axis.truncate()), "{}: lot {k} module at {:?} inside lot {j}", style.id, m.w_axis);
                }
            }
        }
    }
}

// --------------------------------------------------------- limits shape

/// A plain 14 m house facing a street of the given width.
fn plain(clear: f32, walk: f32) -> Lot {
    lot_with(vec![v(0.0, 0.0), v(14.0, 0.0), v(14.0, 16.0), v(0.0, 16.0)], vec![Courtyard, Party, Street, Party], front(clear, walk), |p| p.x)
}

#[test]
fn facing_limits_shape_facades() {
    let kit = Kit::load(None);
    let styles = StyleLibrary::builtin();
    let style = styles.get("madrid_centro").unwrap();
    let mut prev: Option<(f32, f32)> = None;
    for (clear, walk) in [(3.0, 1.5), (6.0, 1.6), (11.0, 2.0), (20.0, 4.5)] {
        let lot = plain(clear, walk);
        let (mut balcony, mut cornice) = (0.0f32, 0.0f32);
        for seed in 0..4 {
            let b = build(&kit, std::slice::from_ref(&lot), style, seed);
            // Deepest window module (balconies) and deepest moulding at the eave.
            for (si, _) in &b.out.placements {
                if b.out.scenes[*si].name == "window" {
                    balcony = balcony.max(b.out.scenes[*si].bounds().max.z);
                }
            }
            for (mat, p) in &b.parts[0].prims {
                if matches!(mat, Mat::Moulding(_) | Mat::Wood(_) | Mat::Stucco(_) | Mat::Plaster(_) | Mat::Brick(_)) {
                    for q in p.positions.iter().filter(|q| (q.y - (b.eaves[0] + 0.15)).abs() < 0.2) {
                        cornice = cornice.max(q.z - 16.0);
                    }
                }
            }
        }
        if let Some((pb, pc)) = prev {
            assert!(balcony >= pb - 1e-4 && cornice >= pc - 1e-4, "clear {clear}: balcony {balcony} (was {pb}), cornice {cornice} (was {pc})");
        }
        // Projections stay within reach.
        let fr = front(clear, walk);
        assert!(balcony <= fr.reach + 0.15 + 1e-3, "clear {clear}: balcony {balcony}");
        assert!(cornice <= fr.reach + 0.02, "clear {clear}: cornice {cornice}");
        prev = Some((balcony, cornice));
    }
    let (b, c) = prev.unwrap();
    assert!(b > 0.5 && c > 0.5, "wide street: balcony {b}, cornice {c}");
}

#[test]
fn alley_fronts_have_no_awnings_or_miradores() {
    let kit = Kit::load(None);
    let styles = StyleLibrary::builtin();
    let lots = alley_pair();
    for style in &styles.styles {
        for seed in 0..3 {
            let b = build(&kit, &lots, style, seed);
            for (si, _) in &b.out.placements {
                let sc = &b.out.scenes[*si];
                assert!(!is_awning(&b.out, *si), "{}: awning in a 3 m alley ({})", style.id, sc.name);
                assert!(!is_mirador(&b.out, *si), "{}: mirador in a 3 m alley ({})", style.id, sc.name);
            }
        }
    }
}

#[test]
fn lanterns_follow_the_rhythm_across_lots() {
    let kit = Kit::load(None);
    let styles = StyleLibrary::builtin();
    let lots = alley_pair();
    for style in &styles.styles {
        let b = build(&kit, &lots, style, 2);
        for (k, lot) in lots.iter().enumerate() {
            let i = (0..4).find(|&i| lot.edges[i] == Street).unwrap();
            let fr = lot.front(i);
            let r = fr.lamps.unwrap();
            let (lo, hi) = (fr.along[0].min(fr.along[1]), fr.along[0].max(fr.along[1]));
            let marks: Vec<f32> = (0..20).map(|m| r.phase + m as f32 * r.pitch).filter(|m| *m >= lo && *m < hi).collect();
            let lanterns: Vec<Vec3> = b.out.placements[b.placements[k].clone()].iter().filter(|(s, _)| b.out.scenes[*s].name.starts_with("wall_lantern")).map(|(_, m)| m.w_axis.truncate()).collect();
            assert_eq!(lanterns.len(), marks.len(), "{} lot {k}: lanterns {lanterns:?} for marks {marks:?}", style.id);
            for m in &marks {
                // On the pier nearest the mark: within half a bay.
                assert!(lanterns.iter().any(|p| (p.x - m).abs() <= 3.0), "{} lot {k}: no lantern near mark {m} ({lanterns:?})", style.id);
            }
        }
        // Every mark of the street got exactly one lantern, whichever lot it fell on.
        let total = b.out.placements.iter().filter(|(s, _)| b.out.scenes[*s].name.starts_with("wall_lantern")).count();
        let expected = [(0.0f32, 27.0f32), (7.0, 27.0)].iter().map(|(ph, len)| (0..20).filter(|m| { let x = ph + *m as f32 * 14.0; x < *len }).count()).sum::<usize>();
        assert_eq!(total, expected, "{}", style.id);
    }
}

#[test]
fn fit_keeps_endpoints_and_shares_party_vertices() {
    let styles = StyleLibrary::builtin();
    let lots = curved_rows();
    for style in &styles.styles {
        let fitted: Vec<Lot> = lots.iter().map(|l| facade::plan(l, style, &mut forja_city::geom::Rng::new(5)).lot).collect();
        for (l, f) in lots.iter().zip(&fitted) {
            f.check().unwrap();
            // Every party vertex survives the fit, bit for bit.
            let n = l.ring.len();
            for i in 0..n {
                if !(l.edges[i].is_street() && l.edges[(i + n - 1) % n].is_street()) {
                    assert!(f.ring.contains(&l.ring[i]), "{}: vertex {i} moved", style.id);
                }
            }
            // The fitted front stays on the curve: the chords stray at most
            // FIT_OUTWARD over the street.
            for p in &f.ring {
                let r = p.length();
                assert!(r > 9.9 && r < 40.1, "{p:?}");
            }
            for k in 0..f.ring.len() {
                if !f.edges[k].is_street() {
                    continue;
                }
                let mid = (f.edge(k).0 + f.edge(k).1) * 0.5;
                if !point_in(&l.ring, mid) {
                    let d = (0..n).map(|i| seg_dist(mid, l.edge(i).0, l.edge(i).1)).fold(f32::INFINITY, f32::min);
                    assert!(d <= facade::FIT_OUTWARD + 1e-4, "{}: chord {k} {d:.3} m over the street", style.id);
                }
            }
        }
    }
}

#[test]
fn openings_never_straddle_vertices() {
    // Every bay of a fitted front is one planar chord: on curved and
    // rounded fronts, each opening lies within its edge.
    let styles = StyleLibrary::builtin();
    for lots in [curved_rows(), rounded_corner()] {
        for style in &styles.styles {
            for lot in &lots {
                let p = facade::plan(lot, style, &mut forja_city::geom::Rng::new(9));
                for (i, eb) in p.bays.iter().enumerate() {
                    if eb.n == 0 {
                        continue;
                    }
                    let (a, b) = p.lot.edge(i);
                    let len = a.distance(b);
                    assert!(eb.m0 >= -1e-4 && eb.m0 + eb.n as f32 * eb.w <= len + 1e-3, "{}: bays overrun edge {i}: {eb:?} on {len}", style.id);
                    // Openings are at most a bay wide minus their piers.
                    assert!(eb.w <= len + 1e-3);
                }
            }
        }
    }
}

#[test]
fn end_clearance_by_corner_angle() {
    let deg = |d: f32| d.to_radians();
    let table = [(20.0, 0.3, 0.3 / deg(20.0).tan()), (45.0, 0.2, 0.55), (90.0, 0.5, 0.55), (135.0, 0.5, 0.55), (270.0, 0.5, 0.9)];
    for (a, p, want) in table {
        let got = end_clearance(deg(a), p);
        assert!((got - want).abs() < 1e-4, "{a}°: {got} vs {want}");
    }
}

#[test]
fn skirt_closes_mixed_ground() {
    // Ground up to SKIRT below the base in front of a facade never shows a
    // gap: the street wall covers the band under the base all along it,
    // under doors and shopfronts too.
    let kit = Kit::load(None);
    let styles = StyleLibrary::builtin();
    for (name, lots) in [("minimal", minimal()), ("acute_corner", acute_corner()), ("curved_rows", curved_rows())] {
        for style in &styles.styles {
            let b = build(&kit, &lots, style, 4);
            for (k, lot) in lots.iter().enumerate() {
                // Triangles of the walls touching the skirt band.
                let mut tris: Vec<[Vec3; 3]> = Vec::new();
                for p in b.parts[k].prims.values() {
                    for t in p.indices.chunks_exact(3) {
                        let q = [p.positions[t[0] as usize], p.positions[t[1] as usize], p.positions[t[2] as usize]];
                        if q.iter().any(|x| x.y < lot.base - 0.1) {
                            tris.push(q);
                        }
                    }
                }
                for i in 0..lot.ring.len() {
                    if !lot.edges[i].is_street() {
                        continue;
                    }
                    let (a, c) = lot.edge(i);
                    let len = a.distance(c);
                    let mut s = 0.1;
                    while s < len - 0.1 {

                        let y = lot.base - SKIRT * 0.5;
                        // Some skirt triangle on the facade (fitted chords stray a
                        // few centimetres from the original edge) covers (p2, y).
                        let p2 = a.lerp(c, s / len);
                        let covered = tris.iter().any(|t| {
                            let ys = [t[0].y, t[1].y, t[2].y];
                            let (lo, hi) = (ys.iter().copied().fold(f32::INFINITY, f32::min), ys.iter().copied().fold(f32::NEG_INFINITY, f32::max));
                            if y < lo - 1e-4 || y > hi + 1e-4 {
                                return false;
                            }
                            // A wall triangle stands on a segment of ground:
                            // its two farthest-apart footprint points.
                            let xs: Vec<Vec2> = t.iter().map(|q| Vec2::new(q.x, q.z)).collect();
                            let (mut s0, mut s1, mut best) = (xs[0], xs[0], -1.0f32);
                            for i in 0..3 {
                                for j in i + 1..3 {
                                    if xs[i].distance(xs[j]) > best {
                                        (s0, s1, best) = (xs[i], xs[j], xs[i].distance(xs[j]));
                                    }
                                }
                            }
                            seg_dist(p2, s0, s1) < 0.15
                        });
                        assert!(covered, "{name}[{k}] {}: no skirt at {s:.2} m along edge {i}", style.id);
                        s += 0.25;
                    }
                }
            }
        }
    }
}

#[test]
fn hip_roof_watertight() {
    // Faces and top tile the eave outline, inner slope edges are shared by
    // exactly two faces, and no face is inverted, on every fixture lot.
    for (name, lots) in fixtures() {
        for lot in &lots {
            let n = lot.ring.len();
            for (pitch, cap, eave) in [(0.42f32, 0.42f32 * 6.25, 0.45f32), (2.9 / 0.9, 2.9, 0.1)] {
                let slopes: Vec<Slope> = (0..n).map(|i| if lot.edges[i] == Party { Slope::WALL } else { Slope { pitch, eave: eave.min(lot.front(i).reach) } }).collect();
                let r = [1.0, 0.6, 0.35].iter().find_map(|k| Roof::new(&lot.ring, &slopes, cap * k)).unwrap_or_else(|| panic!("{name}: no roof"));
                let area = |p: &[Vec2]| forja_city::geom::signed_area(p);
                let total: f32 = r.faces.iter().map(|(_, f)| area(f)).sum::<f32>() + r.top.iter().map(|t| area(t)).sum::<f32>();
                let outer = area(&r.outer);
                assert!((total - outer).abs() <= 1e-3 * outer.max(1.0), "{name}: faces {total} vs outline {outer}");
                for (_, f) in &r.faces {
                    assert!(area(f) > 0.0, "{name}: inverted face");
                }
                // 3D edges: each inner edge (not on the outline, not on a
                // party wall) appears in exactly two faces, reversed.
                let mut edges: Vec<(Vec3, Vec3)> = Vec::new();
                let h = |i: usize, p: Vec2| Vec3::new(p.x, r.face_height(i, p), p.y);
                for (i, f) in &r.faces {
                    for k in 0..f.len() {
                        edges.push((h(*i, f[k]), h(*i, f[(k + 1) % f.len()])));
                    }
                }
                for t in &r.top {
                    for k in 0..t.len() {
                        edges.push((Vec3::new(t[k].x, r.cap, t[k].y), Vec3::new(t[(k + 1) % t.len()].x, r.cap, t[(k + 1) % t.len()].y)));
                    }
                }
                let on_outline = |a: Vec3, b: Vec3| {
                    let m = Vec2::new((a.x + b.x) * 0.5, (a.z + b.z) * 0.5);
                    let no = r.outer.len();
                    // The outline, or a party wall's line (closed by gables).
                    let on_line = |i: usize| {
                        let (p, q) = lot.edge(i);
                        (m - p).perp_dot((q - p).normalize()).abs() < 1e-3
                    };
                    (0..no).any(|i| seg_dist(m, r.outer[i], r.outer[(i + 1) % no]) < 1e-3) || (0..n).any(|i| lot.edges[i] == Party && on_line(i))
                };
                // Split edges at every vertex lying on them (T-junctions
                // between a face and the top are fine), then match.
                let verts: Vec<Vec3> = edges.iter().flat_map(|e| [e.0, e.1]).collect();
                let mut pieces: Vec<(Vec3, Vec3)> = Vec::new();
                for &(a, b) in &edges {
                    let d = b - a;
                    let l2 = d.length_squared();
                    if l2 < 1e-8 {
                        continue;
                    }
                    let mut ts: Vec<f32> = verts.iter().filter_map(|p| {
                        let t = (*p - a).dot(d) / l2;
                        (t > 1e-4 && t < 1.0 - 1e-4 && (a + d * t).distance(*p) < 2e-3).then_some(t)
                    }).collect();
                    ts.push(0.0);
                    ts.push(1.0);
                    ts.sort_by(f32::total_cmp);
                    ts.dedup_by(|x, y| (*x - *y).abs() < 1e-4);
                    for w in ts.windows(2) {
                        pieces.push((a + d * w[0], a + d * w[1]));
                    }
                }
                for &(a, b) in &pieces {
                    if on_outline(a, b) {
                        continue;
                    }
                    let twins = pieces.iter().filter(|(c, d)| c.distance(b) < 3e-3 && d.distance(a) < 3e-3).count();
                    assert_eq!(twins, 1, "{name}: slope edge {a:?} -> {b:?} has {twins} twins");
                }
            }
        }
    }
}

#[test]
fn shallow_lots_get_shallow_rooms() {
    let kit = Kit::load(None);
    let styles = StyleLibrary::builtin();
    let lot = lot_with(vec![v(0.0, 0.0), v(10.0, 0.0), v(10.0, 3.5), v(0.0, 3.5)], vec![Party, Party, Street, Party], Front { depth: 3.5, ..front(8.0, 2.0) }, |p| p.x);
    for style in &styles.styles {
        let b = build(&kit, std::slice::from_ref(&lot), style, 1);
        for (si, m) in &b.out.placements {
            // Window modules on the street front reach no deeper than a
            // shallow room behind the reveal.
            if b.out.scenes[*si].name == "window" && (m.w_axis.z - 3.5).abs() < 1e-3 {
                let depth = -b.out.scenes[*si].bounds().min.z;
                assert!(depth <= 1.0, "{}: room {depth:.2} m deep in a 3.5 m lot", style.id);
            }
        }
    }
}

#[test]
fn osm_levels_are_honoured() {
    let kit = Kit::load(None);
    let styles = StyleLibrary::builtin();
    for levels in [2u32, 5, 9] {
        let mut lot = plain(11.0, 2.0);
        lot.levels = Some(levels);
        for style in &styles.styles {
            let b = build(&kit, std::slice::from_ref(&lot), style, 3);
            assert_eq!(b.out.party_walls[0].floors.len() as u32, levels - 1, "{}", style.id);
        }
    }
}


