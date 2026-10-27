//! Public space as surface classes, and its meshes: roadway, medians,
//! shared cobbled surfaces, plazas and sidewalks, kerbs where heights
//! differ, gutters and road markings.
//!
//! Every class is a region expression over the fabric's public space, so
//! any street layout gets the same treatment:
//!
//! ```text
//! J        = discs at nodes with two or more car arms (kerb radius)
//! carriage = car roadways, with kerb fillets only inside J
//! median   = boulevard medians (outside J)
//! shared   = streets without a roadway, minus car streets (whose
//!            sidewalk runs straight across alley mouths)
//! plaza    = given plazas and every open area wider than 16 m
//! walk     = the rest
//! ```

use std::collections::BTreeMap;

use rayon::prelude::*;

use forja_math::{Vec2, Vec3};

use crate::geom::Parts;
use crate::kit::{Mat, hex};
use crate::plan::line::left;
use crate::plan::{End, Join, Located, NodeId, Polyline, Region, StreetGraph, StreetId};
use crate::profile::{Class, Gutter, Markings, Parking, ProfileLibrary};

/// A junction of car streets: centre, kerb-fillet radius and the disc
/// radius that bounds markings and furniture.
#[derive(Clone, Copy, Debug)]
pub struct Junction {
    pub node: NodeId,
    pub c: Vec2,
    pub kerb_radius: f32,
    pub r: f32,
}

/// One piece of ground: a region at a height with a material.
#[derive(Clone, Debug)]
pub struct Surface {
    pub region: Region,
    pub h: f32,
    pub mat: Mat,
}

#[derive(Clone, Debug, Default)]
pub struct Ground {
    pub carriage: Region,
    pub median: Region,
    pub shared: Region,
    pub plaza: Region,
    pub walk: Region,
    pub junctions: Vec<Junction>,
    /// The height partition of public space, for meshes and kerbs. Heights
    /// are above the relief.
    pub surfaces: Vec<Surface>,
    /// Ground level under everything (street grades, stairs, terrain).
    pub relief: crate::plan::Relief,
    /// Public space by tile (for clipping risers and the like).
    pub public_tiles: crate::plan::Tiled,
    loc: Vec<Located>,
    carriage_loc: Located,
    plaza_loc: Located,
    plaza_poles: Vec<(Located, f32)>,
}

fn disc(c: Vec2, r: f32, n: usize) -> Vec<Vec2> {
    (0..n).map(|i| c + Vec2::from_angle(i as f32 / n as f32 * std::f32::consts::TAU) * r).collect()
}

/// Car arms: streets with vehicles and a roadway.
fn car(g: &StreetGraph, lib: &ProfileLibrary, s: StreetId) -> bool {
    let st = g.street(s);
    lib.get(st.profile).vehicles && st.carriageway > 0.0
}

/// Whether end `e` of street `s` has a zebra crossing (and a stop line):
/// a car street whose profile has crossings, at a real junction, with room
/// for both before the other end's junction.
pub fn has_zebra(g: &StreetGraph, lib: &ProfileLibrary, s: StreetId, e: End) -> bool {
    let n = g.street(s).node(e);
    car(g, lib, s) && lib.get(g.street(s).profile).crossings && (g.star(n).iter().filter(|a| car(g, lib, a.0)).count() >= 3 || g.degree(n) >= 3) && g.street(s).len() >= 2.0 * g.mouth(s, e, lib) + 12.0
}

impl Ground {
    /// Splits `public` into surface classes.
    pub fn classes(g: &StreetGraph, lib: &ProfileLibrary, public: &Region, plazas: &Region, relief: crate::plan::Relief) -> Ground {
        let prof = |s: &crate::plan::Street| lib.get(s.profile);
        let mut laps = crate::fabric::Laps::new();
        // Junction discs.
        let mut junctions = Vec::new();
        for n in 0..g.nodes.len() as NodeId {
            let arms: Vec<(StreetId, End)> = g.star(n).iter().filter(|a| car(g, lib, a.0)).map(|a| (a.0, a.1)).collect();
            if arms.len() < 2 {
                continue;
            }
            let half = arms.iter().map(|a| g.street(a.0).carriageway * 0.5).fold(0.0, f32::max);
            let kr = arms.iter().map(|a| prof(g.street(a.0)).kerb_radius).fold(0.0, f32::max);
            let shortest = arms.iter().map(|a| g.street(a.0).len()).fold(f32::INFINITY, f32::min);
            let mouth = arms.iter().map(|a| g.mouth(a.0, a.1, lib)).fold(0.0, f32::max);
            let r = (half + kr).max(mouth).min(shortest * 0.5);
            junctions.push(Junction { node: n, c: g.nodes[n as usize].p, kerb_radius: kr, r });
        }
        laps.lap("ground: junction discs");
        let carriage0 = g.corridor(|s, _| if prof(s).vehicles { s.carriageway } else { 0.0 });
        // Kerb fillets: each junction disc only sees the roadway tiles under it.
        let tiled = carriage0.tiles(TILE);
        // Independent per junction: in parallel, in junction order.
        let fillets: Vec<Region> = junctions
            .par_iter()
            .filter(|j| j.kerb_radius > 0.0)
            .map(|j| {
                let d = Region::ring(&disc(j.c, j.r, 32));
                let (k0, k1) = (tiled.key(j.c - Vec2::splat(j.r)), tiled.key(j.c + Vec2::splat(j.r)));
                let near: Vec<&Region> = (k0.0..=k1.0).flat_map(|x| (k0.1..=k1.1).map(move |z| (x, z))).filter_map(|k| tiled.get(k)).collect();
                Region::union_all(near).and(&d).closing(j.kerb_radius).and(&d)
            })
            .collect();
        laps.lap("ground: fillets");
        let carriage_all = carriage0.union(&Region::union_all(fillets.iter())).and(public);
        let jdiscs = Region::rings(junctions.iter().map(|j| disc(j.c, j.r, 32)).collect::<Vec<_>>().iter().map(|r| r.as_slice()));
        let median = g.corridor(|s, _| prof(s).median).minus(&jdiscs).and(&carriage_all);
        let carriage = carriage_all.minus(&median);
        laps.lap("ground: carriage and median");
        let carroads = g.corridor(|s, i| if prof(s).vehicles && s.carriageway > 0.0 { s.width[i] } else { 0.0 });
        // Shared streets yield to car streets' sidewalks at their mouths and
        // to squares: an alley opens onto a plaza's paving, it does not cross it.
        let shared = g.corridor(|s, i| if s.carriageway <= 0.0 && prof(s).class != Class::Limit { s.width[i] } else { 0.0 }).and(public).minus(&carroads).minus(&carriage).minus(plazas);
        laps.lap("ground: shared");
        // `public - carriage - median - shared`, computed once (the same
        // operations in the same order as before).
        let rest = public.minus(&carriage).minus(&median).minus(&shared);
        let open = rest.opening(8.0);
        let plaza = plazas.union(&open).and(public).minus(&carriage).minus(&median).minus(&shared);
        let walk = rest.minus(&plaza);
        laps.lap("ground: plaza and walk");
        let plaza_poles = plaza.components().into_iter().map(|c| {
            let r = c.pole().map(|p| p.1).unwrap_or(0.0);
            (Located::new(c), r)
        }).collect();
        laps.lap("ground: poles");
        let mut gr = Ground { carriage, median, shared, plaza, walk, junctions, relief, public_tiles: public.tiles(TILE), ..Default::default() };
        gr.surfaces = surfaces(&gr, g, lib);
        laps.lap("ground: surfaces");
        gr.loc = gr.surfaces.iter().map(|s| Located::new(s.region.clone())).collect();
        gr.carriage_loc = Located::new(gr.carriage.clone());
        gr.plaza_loc = Located::new(gr.plaza.clone());
        gr.plaza_poles = plaza_poles;
        gr
    }

    /// Ground height at a point: its surface's height above the relief
    /// (0 off public space), plus the relief.
    pub fn height_at(&self, p: Vec2) -> f32 {
        self.surface_at(p).map(|s| s.h).unwrap_or(0.0) + self.relief.elev(p)
    }

    /// Height of the surface class at `p` above the relief.
    pub fn class_height(&self, p: Vec2) -> f32 {
        self.surface_at(p).map(|s| s.h).unwrap_or(0.0)
    }

    pub fn surface_at(&self, p: Vec2) -> Option<&Surface> {
        self.loc.iter().position(|l| l.contains(p)).map(|i| &self.surfaces[i])
    }

    /// Distance along `dir` to the roadway, if within `max`.
    pub fn to_carriage(&self, p: Vec2, dir: Vec2, max: f32) -> Option<f32> {
        if self.carriage_loc.contains(p) {
            return Some(0.0);
        }
        self.carriage_loc.ray(p, dir, max)
    }

    pub fn in_carriage(&self, p: Vec2) -> bool {
        self.carriage_loc.contains(p)
    }

    pub fn in_plaza(&self, p: Vec2) -> bool {
        self.plaza_loc.contains(p)
    }

    /// Inscribed radius of the plaza containing `p`.
    pub fn plaza_radius(&self, p: Vec2) -> Option<f32> {
        self.plaza_poles.iter().find(|(l, _)| l.contains(p)).map(|(_, r)| *r)
    }
}

/// Height partition: each class split by the street that owns it (higher
/// rank first), with that street's material and height.
fn surfaces(gr: &Ground, g: &StreetGraph, lib: &ProfileLibrary) -> Vec<Surface> {
    // Group streets by their ground signature so there are few regions.
    #[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
    struct Sig {
        rank: std::cmp::Reverse<u8>,
        carriage: String,
        walk: String,
        shared: String,
        kerb: i32,
        shared_h: i32,
    }
    let mut groups: BTreeMap<Sig, Vec<StreetId>> = BTreeMap::new();
    for (i, s) in g.streets.iter().enumerate() {
        let p = lib.get(s.profile);
        let shared_h = if p.vehicles { 0.02 } else { p.shared_height.max(0.02) };
        let sig = Sig { rank: std::cmp::Reverse(p.rank), carriage: p.surface.carriage.clone(), walk: p.surface.walk.clone(), shared: p.surface.shared.clone(), kerb: (p.kerb * 1000.0) as i32, shared_h: (shared_h * 1000.0) as i32 };
        groups.entry(sig).or_default().push(i as StreetId);
    }
    let classes = [(&gr.carriage, 0usize), (&gr.walk, 1), (&gr.shared, 2)];
    // A class is owned only by streets that have it: an alley's shared
    // paving that reaches into a car street's corridor keeps the alley's
    // height, it does not take the car street's.
    let has = |k: usize, s: &crate::plan::Street| match k {
        0 => lib.get(s.profile).vehicles && s.carriageway > 0.0,
        2 => s.carriageway <= 0.0,
        _ => true,
    };
    // The classes are independent, and so is each group's corridor within
    // a class; only the `taken` chain is sequential.
    let groups: Vec<(&Sig, &Vec<StreetId>)> = groups.iter().collect();
    let per_class: Vec<Vec<Surface>> = classes.par_iter().map(|&(class, k)| {
        let mut out = Vec::new();
        let corridors: Vec<Option<(Sig, Region)>> = groups
            .par_iter()
            .map(|(sig, list)| {
                let set: std::collections::BTreeSet<u64> = list.iter().filter(|&&i| has(k, g.street(i))).map(|&i| g.street(i).key).collect();
                if set.is_empty() {
                    return None;
                }
                Some(((*sig).clone(), g.corridor(|s, i| if set.contains(&s.key) { s.width[i] + 0.5 } else { 0.0 }).and(class)))
            })
            .collect();
        let mut taken = Region::empty();
        let mut cells = Vec::new();
        for (sig, c) in corridors.into_iter().flatten() {
            let own = c.minus(&taken);
            taken = taken.union(&own);
            cells.push((sig, own));
        }
        let mut rest = class.clone();
        for (sig, own) in &cells {
            let r = class.and(own);
            rest = rest.minus(own);
            if r.is_empty() {
                continue;
            }
            let (h, m) = match k {
                0 => (if sig.carriage == "Asphalt" { 0.0 } else { 0.002 }, crate::profile::mat(&sig.carriage).unwrap_or(Mat::Asphalt)),
                1 => (sig.kerb as f32 / 1000.0, crate::profile::mat(&sig.walk).unwrap_or(Mat::SidewalkSlab)),
                _ => (sig.shared_h as f32 / 1000.0, crate::profile::mat(&sig.shared).unwrap_or(Mat::Cobble)),
            };
            out.push(Surface { region: r, h, mat: m });
        }
        if !rest.is_empty() {
            let (h, m) = match k {
                0 => (0.0, Mat::Asphalt),
                1 => (0.15, Mat::SidewalkSlab),
                _ => (0.15, Mat::Cobble),
            };
            out.push(Surface { region: rest, h, mat: m });
        }
        out
    }).collect();
    let mut out: Vec<Surface> = per_class.into_iter().flatten().collect();
    if !gr.plaza.is_empty() {
        out.push(Surface { region: gr.plaza.clone(), h: 0.15, mat: Mat::Cobble });
    }
    if !gr.median.is_empty() {
        out.push(Surface { region: gr.median.clone(), h: 0.15, mat: Mat::SidewalkHex });
    }
    // Merge surfaces with the same height and material.
    let mut merged: Vec<Surface> = Vec::new();
    for s in out {
        if let Some(m) = merged.iter_mut().find(|m| m.mat == s.mat && (m.h - s.h).abs() < 1e-4) {
            m.region = m.region.union(&s.region);
        } else {
            merged.push(s);
        }
    }
    merged
}

// ------------------------------------------------------------------ meshes

/// A kerb face: a polyline along the boundary of a raised surface, from
/// `lo` up to `hi`.
#[derive(Clone, Debug)]
pub struct Kerb {
    pub line: Vec<Vec2>,
    pub lo: f32,
    pub hi: f32,
}

/// Kerb faces wherever neighbouring public surfaces differ in height.
pub fn kerbs(gr: &Ground) -> Vec<Kerb> {
    let mut out: Vec<Kerb> = Vec::new();
    for s in &gr.surfaces {
        if s.h < 0.02 {
            continue;
        }
        for sh in s.region.shapes() {
            for ring in sh.rings() {
                let n = ring.len();
                let mut cur: Option<Kerb> = None;
                for i in 0..n {
                    let (a, b) = (ring[i], ring[(i + 1) % n]);
                    let d = (b - a).normalize_or(Vec2::X);
                    let probe = (a + b) * 0.5 + Vec2::new(d.y, -d.x) * 0.05;
                    let lo = gr.surface_at(probe).map(|o| o.h);
                    let face = lo.filter(|lo| s.h - lo > 0.01);
                    match (face, cur.as_mut()) {
                        (Some(lo), Some(k)) if (k.lo - lo).abs() < 1e-3 && k.line.last() == Some(&a) => k.line.push(b),
                        (Some(lo), _) => {
                            if let Some(k) = cur.take() {
                                out.push(k);
                            }
                            cur = Some(Kerb { line: vec![a, b], lo, hi: s.h });
                        }
                        (None, _) => {
                            if let Some(k) = cur.take() {
                                out.push(k);
                            }
                        }
                    }
                }
                if let Some(k) = cur.take() {
                    out.push(k);
                }
            }
        }
    }
    out
}

/// Ground meshes per 64 m tile: every surface triangulated at its height,
/// kerb faces with a granite top band, gutters. Colliders come from the
/// same triangles. Returns tiles of parts plus collider triangles.
/// Side of the ground, marking and building tiles (m).
pub const TILE: f32 = 64.0;

/// `outside` is the ground around the city (to the edge of the map): on
/// sloping ground it is meshed on the terrain too, so the city does not
/// stand on a ledge above a flat plane.
pub fn mesh(gr: &Ground, g: &StreetGraph, lib: &ProfileLibrary, courts: &Region, outside: &Region) -> (BTreeMap<(i32, i32), Parts<Mat>>, Vec<[Vec3; 3]>) {
    let mut tiles: BTreeMap<(i32, i32), Parts<Mat>> = BTreeMap::new();
    let mut col: Vec<[Vec3; 3]> = Vec::new();
    let tile_of = |p: Vec2| ((p.x / TILE).floor() as i32, (p.y / TILE).floor() as i32);
    let relief = &gr.relief;
    let flat = relief.is_flat();
    let risers = riser_lines(gr, g);
    // On sloping ground the flat asphalt plane no longer carries the roadway
    // and courts need their own paving.
    let court = Surface { region: if flat { Region::empty() } else { courts.clone() }, h: 0.0, mat: Mat::SidewalkSlab };
    let around = Surface { region: if flat { Region::empty() } else { outside.clone() }, h: 0.0, mat: Mat::Asphalt };
    for s in gr.surfaces.iter().chain([&court, &around]) {
        if flat && s.h <= 0.0 && s.mat == Mat::Asphalt {
            continue;
        }
        for (&(tx, tz), piece) in s.region.tiles(TILE).iter() {
            // Pieces small enough for the relief to be linear on them, and
            // split at every riser so each tread is its own flat piece.
            let pieces = if flat {
                vec![piece.clone()]
            } else {
                let mut cuts = grid_cuts(tx, tz, RELIEF_CELL);
                cuts.extend(risers.iter().filter(|r| r.0.iter().any(|p| tile_of(*p) == (tx, tz))).map(|r| r.0.clone()));
                // Terrace edges: the plaza and the ground around it are
                // separate pieces at their own levels.
                for t in &relief.terraces {
                    let mut ring = t.ring.clone();
                    ring.push(ring[0]);
                    cuts.push(ring);
                }
                piece.slice(&cuts).into_iter().map(|sh| Region::from_shapes(&[sh])).collect()
            };
            for sub in pieces {
                let (pts, tris) = sub.triangulate();
                if pts.is_empty() {
                    continue;
                }
                let c = pts.iter().copied().sum::<Vec2>() / pts.len() as f32;
                // Each vertex reads the relief 1 cm inside its own piece.
                let y = |p: Vec2| s.h + relief.elev(p + (c - p).normalize_or_zero() * 0.01);
                let prim = tiles.entry((tx, tz)).or_insert_with(Parts::new).get(s.mat);
                let scale = s.mat.scale();
                let base = prim.positions.len() as u32;
                let ys: Vec<f32> = pts.iter().map(|p| y(*p)).collect();
                for (p, h) in pts.iter().zip(&ys) {
                    prim.vertex(Vec3::new(p.x, *h, p.y), Vec3::Y, *p / scale);
                }
                for t in &tris {
                    // CCW in (x, z) faces -Y: flip to face up.
                    prim.indices.extend([base + t[0], base + t[2], base + t[1]]);
                    let v = |i: u32| Vec3::new(pts[i as usize].x, ys[i as usize], pts[i as usize].y);
                    col.push([v(t[0]), v(t[2]), v(t[1])]);
                }
                if !flat {
                    prim.recompute_normals_from(base);
                }
            }
        }
    }
    // Risers of stair streets: granite faces between treads, each facing
    // its lower tread.
    for (line, s, t) in &risers {
        let across = line[line.len() - 1] - line[0];
        for piece in gr.public_tiles.clip(std::slice::from_ref(line)) {
            let (Some(mut a), Some(mut b)) = (piece.first().copied(), piece.last().copied()) else { continue };
            if (b - a).dot(across) < 0.0 {
                std::mem::swap(&mut a, &mut b);
            }
            let m = (a + b) * 0.5;
            let h = gr.class_height(m);
            let (lo, hi) = (relief.street_z(*s, t - 0.05), relief.street_z(*s, t + 0.05));
            let (lo, hi) = (lo.min(hi) + h, lo.max(hi) + h);
            // a -> b runs to the street's left, so quad(a, b, ..) faces back
            // along the street (as the kerb faces do).
            let up = relief.street_z(*s, t + 0.05) > relief.street_z(*s, t - 0.05);
            let (pa, pb) = if up { (a, b) } else { (b, a) };
            let (qa, qb) = (Vec3::new(pa.x, lo, pa.y), Vec3::new(pb.x, lo, pb.y));
            let parts = tiles.entry(tile_of(m)).or_insert_with(Parts::new);
            parts.get(Mat::Granite).quad(qa, qb, qb + Vec3::Y * (hi - lo), qa + Vec3::Y * (hi - lo), Mat::Granite.scale());
            col.push([qa, qb, qb + Vec3::Y * (hi - lo)]);
            col.push([qa, qb + Vec3::Y * (hi - lo), qa + Vec3::Y * (hi - lo)]);
        }
    }
    // Terrace walls: granite faces where a level plaza meets lower or
    // higher public ground, each facing the lower side.
    for (a, b, lo, hi, inner_high) in terrace_walls(gr) {
        let m = (a + b) * 0.5;
        let parts = tiles.entry(tile_of(m)).or_insert_with(Parts::new);
        let (pa, pb) = if inner_high { (b, a) } else { (a, b) };
        let (qa, qb) = (Vec3::new(pa.x, lo, pa.y), Vec3::new(pb.x, lo, pb.y));
        let up = Vec3::Y * (hi - lo);
        parts.get(Mat::Granite).quad(qa, qb, qb + up, qa + up, Mat::Granite.scale());
        col.push([qa, qb, qb + up]);
        col.push([qa, qb + up, qa + up]);
    }
    // Kerb faces with a 0.3 m granite top band on the raised side.
    for k in kerbs(gr) {
        let mid = k.line[k.line.len() / 2];
        let parts = tiles.entry(tile_of(mid)).or_insert_with(Parts::new);
        let h = k.hi - k.lo;
        // Kerbs follow the relief: chords of at most 2 m, lifted per vertex.
        let line = if flat { k.line.clone() } else { Polyline::new(k.line.clone()).resample(2.0).pts().to_vec() };
        let lift = |p: Vec2| relief.elev(p);
        for w in line.windows(2) {
            let (a, b) = (w[0], w[1]);
            let (pa, pb) = (Vec3::new(a.x, k.lo + lift(a), a.y), Vec3::new(b.x, k.lo + lift(b), b.y));
            // The raised surface is on the left of a -> b (CCW ring): the
            // face looks right (outward).
            parts.get(Mat::Curb).quad(pb, pa, pa + Vec3::Y * (h + 0.003), pb + Vec3::Y * (h + 0.003), Mat::Curb.scale());
            col.push([pb, pa, pa + Vec3::Y * h]);
            col.push([pb, pa + Vec3::Y * h, pb + Vec3::Y * h]);
        }
        if h > 0.05 {
            let path: Vec<Vec3> = line.iter().map(|p| Vec3::new(p.x, k.hi + 0.003 + lift(*p), p.y)).collect();
            let nrm = crate::geom::polyline_normals(&path, false, -1.0);
            parts.get(Mat::Curb).sweep(&path, &nrm, &[(0.3, 0.0), (0.0, 0.0)], false, false, 1.0);
        }
    }
    // Gutters: a granite strip along the centre of shared streets.
    let shared = gr.shared.tiles(TILE);
    for (si, s) in g.streets.iter().enumerate() {
        let p = lib.get(s.profile);
        // Stair streets drain down their steps.
        if p.gutter == Gutter::None || s.carriageway > 0.0 || relief.steps(si as StreetId).is_some() {
            continue;
        }
        let lines: Vec<Vec<Vec2>> = match p.gutter {
            Gutter::Centre => vec![s.line.pts().to_vec()],
            _ => [1.0f32, -1.0].iter().map(|side| s.line.offset(side * (s.min_width() * 0.5 - 0.45).max(0.3)).pts().to_vec()).collect(),
        };
        for piece in shared.clip(&lines) {
            let pl = Polyline::new(piece);
            if pl.len() < 0.5 {
                continue;
            }
            let h = gr.class_height(pl.at(pl.len() * 0.5)) + 0.004;
            let parts = tiles.entry(tile_of(pl.at(pl.len() * 0.5))).or_insert_with(Parts::new);
            let pl = if flat { pl } else { pl.resample(2.0) };
            ribbon(parts.get(Mat::Granite), &pl, 0.35, h, 1.0, &|q| relief.elev(q));
        }
    }
    (tiles, col)
}

/// Wall pieces along the terrace edges: (a, b, low, high, plaza side
/// higher), with the plaza on the left of a -> b. Only where both sides are
/// public and differ by more than a few centimetres (a facade or a plinth
/// closes the rest).
pub fn terrace_walls(gr: &Ground) -> Vec<(Vec2, Vec2, f32, f32, bool)> {
    let mut out = Vec::new();
    for t in &gr.relief.terraces {
        let mut ring = t.ring.clone();
        if crate::geom::signed_area(&ring) < 0.0 {
            ring.reverse();
        }
        ring.push(ring[0]);
        let pts = Polyline::new(ring).resample(1.0).pts().to_vec();
        for w in pts.windows(2) {
            let (a, b) = (w[0], w[1]);
            let d = (b - a).normalize_or(Vec2::X);
            let out_n = Vec2::new(d.y, -d.x);
            let m = (a + b) * 0.5;
            let (pi, po) = (m - out_n * 0.05, m + out_n * 0.05);
            if !gr.public_tiles.contains(pi) || !gr.public_tiles.contains(po) {
                continue;
            }
            let (zi, zo) = (gr.height_at(pi), gr.height_at(po));
            if (zi - zo).abs() < 0.04 {
                continue;
            }
            out.push((a, b, zi.min(zo), zi.max(zo), zi > zo));
        }
    }
    out
}

/// Side of the cells public space is cut into on sloping ground (m).
pub const RELIEF_CELL: f32 = 6.0;

/// Cut lines of a `cell` grid over tile `(tx, tz)`.
fn grid_cuts(tx: i32, tz: i32, cell: f32) -> Vec<Vec<Vec2>> {
    let (x0, z0) = (tx as f32 * TILE, tz as f32 * TILE);
    let k = (TILE / cell).round() as i32;
    (1..k)
        .flat_map(|i| {
            let o = i as f32 * TILE / k as f32;
            [vec![Vec2::new(x0 + o, z0 - 1.0), Vec2::new(x0 + o, z0 + TILE + 1.0)], vec![Vec2::new(x0 - 1.0, z0 + o), Vec2::new(x0 + TILE + 1.0, z0 + o)]]
        })
        .collect()
}

/// Every riser of every stair street, as a line across the street from its
/// right to its left building line: (line, street, arc length).
pub fn riser_lines(gr: &Ground, g: &StreetGraph) -> Vec<(Vec<Vec2>, StreetId, f32)> {
    let mut out = Vec::new();
    for si in 0..g.streets.len() as StreetId {
        let s = g.street(si);
        for t in gr.relief.risers(si) {
            let (p, n, w) = (s.line.at(t), s.line.normal(t), s.width_at(t) * 0.5 + 0.05);
            out.push((vec![p - n * w, p + n * w], si, t));
        }
    }
    out
}

/// Flat strip of half width `w/2` along a polyline at height `y`.
/// Heights are `y` above `lift` (the relief) at each vertex.
pub fn ribbon(p: &mut crate::geom::Prim, line: &Polyline, w: f32, y: f32, scale: f32, lift: &dyn Fn(Vec2) -> f32) {
    let pts = line.pts();
    let n = pts.len();
    if n < 2 {
        return;
    }
    let base = p.positions.len() as u32;
    let mut s = 0.0;
    for i in 0..n {
        if i > 0 {
            s += pts[i].distance(pts[i - 1]);
        }
        let nl = line.vertex_normal(i) * (w * 0.5);
        let (l, r) = (pts[i] + nl, pts[i] - nl);
        p.vertex(Vec3::new(l.x, y + lift(l), l.y), Vec3::Y, Vec2::new(0.0, s) / scale);
        p.vertex(Vec3::new(r.x, y + lift(r), r.y), Vec3::Y, Vec2::new(w, s) / scale);
    }
    for i in 0..n as u32 - 1 {
        let k = base + i * 2;
        // (left, right) pairs: wind to face up.
        p.indices.extend([k, k + 2, k + 1, k + 1, k + 2, k + 3]);
    }
}

// ------------------------------------------------------------------ markings

const WHITE: u32 = 0xf2f2ec;
const YELLOW: u32 = 0xe8b51c;

/// Road markings on any layout: lines along offsets of each centreline,
/// clipped to the roadway shrunk by 0.3 m minus the junction discs, so
/// paint never reaches a sidewalk or a junction; zebras and stop lines at
/// the mouths.
pub fn markings(gr: &Ground, g: &StreetGraph, lib: &ProfileLibrary) -> Parts<Mat> {
    let mut p: Parts<Mat> = Parts::new();
    if gr.carriage.is_empty() {
        return p;
    }
    let jd = Region::rings(gr.junctions.iter().map(|j| disc(j.c, j.r, 32)).collect::<Vec<_>>().iter().map(|r| r.as_slice()));
    // Paint stays 0.3 m off the kerbs and out of junctions; zebras reach to
    // 0.2 m. Tiled once, so each street clips against a few tiles only.
    let paintable = gr.carriage.offset(-0.3, Join::Miter(0.5)).minus(&jd).tiles(TILE);
    let zebra = gr.carriage.offset(-0.2, Join::Miter(0.5)).tiles(TILE);
    let white = Mat::Paint(hex(WHITE));
    let yellow = Mat::Paint(hex(YELLOW));
    let lift = |q: Vec2| gr.relief.elev(q);
    for (si, s) in g.streets.iter().enumerate() {
        let prof = lib.get(s.profile);
        if !prof.vehicles || s.carriageway <= 0.0 || prof.markings == Markings::None {
            continue;
        }
        let road = s.carriageway * 0.5;
        let si = si as StreetId;
        let (m0, m1) = (g.mouth(si, End::A, lib), s.len() - g.mouth(si, End::B, lib));
        let line = &s.line;
        // Solid and dashed lines along offsets, clipped.
        let solid = |o: f32, w: f32, m: Mat, p: &mut Parts<Mat>| {
            for piece in paintable.clip(&[line.offset(o).pts().to_vec()]) {
                let pl = Polyline::new(piece);
                ribbon(p.get(m), &pl, w, 0.008, 1.0, &lift);
            }
        };
        let dashed = |o: f32, dash: f32, gap: f32, p: &mut Parts<Mat>| {
            let off = line.offset(o);
            let mut t = m0 + 6.5;
            let mut segs = Vec::new();
            while t < m1 - 6.5 {
                segs.push(off.sub(t, (t + dash).min(m1 - 6.5)).pts().to_vec());
                t += dash + gap;
            }
            for piece in paintable.clip(&segs) {
                ribbon(p.get(white), &Polyline::new(piece), 0.12, 0.008, 1.0, &lift);
            }
        };
        match prof.markings {
            Markings::Avenue => {
                let inner = prof.median * 0.5 + 0.3;
                for side in [-1.0f32, 1.0] {
                    solid(side * inner, 0.15, white, &mut p);
                    solid(side * (road - 0.3), 0.15, white, &mut p);
                    let lanes = prof.lanes.max(1) as f32;
                    let lane = (road - 0.3 - inner) / lanes;
                    for k in 1..prof.lanes.max(1) {
                        dashed(side * (inner + lane * k as f32), 4.5, 7.5, &mut p);
                    }
                    if prof.bike {
                        // Red bike lane beside the median.
                        for piece in paintable.clip(&[line.offset(side * (inner + 1.0)).pts().to_vec()]) {
                            ribbon(p.get(Mat::Paint(hex(0xa23b2c))), &Polyline::new(piece), 1.6, 0.007, 2.0, &lift);
                        }
                    }
                }
            }
            Markings::Centre => dashed(0.0, 3.0, 4.5, &mut p),
            Markings::ParkingOneWay | Markings::OneWay => {
                if prof.parking != Parking::None && prof.markings == Markings::ParkingOneWay {
                    let sides: &[f32] = if prof.parking == Parking::Both { &[-1.0, 1.0] } else { &[-1.0] };
                    for &side in sides {
                        let o = side * (road - 2.2);
                        solid(o, 0.1, white, &mut p);
                        // Bay ticks where the street is straight enough.
                        let mut t = m0 + 8.0;
                        let mut ticks = Vec::new();
                        while t < m1 - 8.0 {
                            if curvature(line, t) < 1.0 / 60.0 {
                                let a = line.at(t) + line.normal(t) * o;
                                let b = line.at(t) + line.normal(t) * side * road;
                                ticks.push(vec![a, b]);
                            }
                            t += 4.8;
                        }
                        for piece in paintable.clip(&ticks) {
                            ribbon(p.get(white), &Polyline::new(piece), 0.1, 0.008, 1.0, &lift);
                        }
                    }
                }
                if prof.lanes >= 2 {
                    dashed(0.0, 3.0, 4.5, &mut p);
                }
            }
            Markings::KerbLine => {
                // One lane, parking on one side, a yellow line on the other.
                if prof.parking != Parking::None && road > 2.5 {
                    solid(-(road - 2.0), 0.1, white, &mut p);
                }
                solid(road - 0.25, 0.12, yellow, &mut p);
            }
            Markings::None => {}
        }
        // Arrows along one-way and avenue streets.
        if s.oneway != 0 || prof.markings == Markings::Avenue {
            let mut t = m0 + 14.0;
            while t < m1 - 10.0 {
                let fwd = line.tangent(t);
                if curvature(line, t) < 1.0 / 40.0 {
                    let offs: Vec<f32> = if prof.markings == Markings::Avenue {
                        let inner = prof.median * 0.5 + 0.3;
                        let lane = (road - 0.3 - inner) / prof.lanes.max(1) as f32;
                        (0..prof.lanes.max(1)).flat_map(|k| [inner + lane * (k as f32 + 0.5), -(inner + lane * (k as f32 + 0.5))]).collect()
                    } else if prof.lanes >= 2 {
                        vec![-1.4, 1.4]
                    } else {
                        vec![if prof.parking == Parking::One { 1.0 } else { 0.0 }]
                    };
                    for o in offs {
                        let c = line.at(t) + line.normal(t) * o;
                        let dir = if prof.markings == Markings::Avenue { if o > 0.0 { -fwd } else { fwd } } else { fwd * s.oneway as f32 };
                        if paintable.contains(c) {
                            arrow(&mut p, c, dir, &lift);
                        }
                    }
                }
                t += if prof.markings == Markings::Avenue { 60.0 } else { 35.0 };
            }
        }
        // Zebra crossings and stop lines at the mouths of real junctions.
        if prof.crossings {
            for e in [End::A, End::B] {
                if !has_zebra(g, lib, si, e) {
                    continue;
                }
                let m = g.mouth(si, e, lib);
                let sign = if e == End::A { 1.0 } else { -1.0 };
                let at = |d: f32| if e == End::A { d } else { s.len() - d };
                let mut stripes = Vec::new();
                let (c0, c1) = (at(m + 1.0), at(m + 5.0));
                let mut o = -road + 0.3;
                while o < road - 0.3 {
                    let c = o + 0.25;
                    stripes.push(vec![line.at(c0) + line.normal(c0) * c, line.at(c1) + line.normal(c1) * c]);
                    o += 1.0;
                }
                for piece in zebra.clip(&stripes) {
                    ribbon(p.get(white), &Polyline::new(piece), 0.5, 0.008, 1.0, &lift);
                }
                // Stop line across the lanes arriving here.
                let arriving = match s.oneway {
                    1 => e == End::B,
                    -1 => e == End::A,
                    _ => true,
                };
                if arriving {
                    let ts = at(m + 6.0);
                    let (oa, ob) = if s.oneway == 0 { if sign > 0.0 { (0.0, road) } else { (-road, 0.0) } } else { (-road, road) };
                    let seg = vec![line.at(ts) + line.normal(ts) * (oa + 0.2), line.at(ts) + line.normal(ts) * (ob - 0.2)];
                    for piece in paintable.clip(&[seg]) {
                        ribbon(p.get(white), &Polyline::new(piece), 0.4, 0.008, 1.0, &lift);
                    }
                }
            }
        }
    }
    p
}

/// Turning rate (1/m) around arc length `t`.
pub fn curvature(l: &Polyline, t: f32) -> f32 {
    let (a, b) = (l.tangent((t - 3.0).max(0.0)), l.tangent((t + 3.0).min(l.len())));
    a.perp_dot(b).atan2(a.dot(b)).abs() / 6.0
}

/// Straight-ahead lane arrow (5 m long) at `c` pointing along `fwd`.
pub fn arrow(p: &mut Parts<Mat>, c: Vec2, fwd: Vec2, lift: &dyn Fn(Vec2) -> f32) {
    let side = left(fwd);
    let m = Mat::Paint(hex(WHITE));
    let (a, b) = (c - fwd * 2.5, c + fwd * 1.0);
    let n = side * 0.09;
    let first = p.get(m).positions.len();
    p.get(m).polygon(&[a - n, b - n, b + n, a + n], 0.008, true, 1.0);
    let tip = c + fwd * 2.5;
    let base = c + fwd * 0.9;
    p.get(m).polygon(&[base - side * 0.45, base + side * 0.45, tip], 0.008, true, 1.0);
    // Laid on the relief, vertex by vertex.
    for v in &mut p.get(m).positions[first..] {
        v.y += lift(Vec2::new(v.x, v.z));
    }
}
