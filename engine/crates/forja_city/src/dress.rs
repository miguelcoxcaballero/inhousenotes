//! Street furniture as 2D-planned items: lamps, trees, bins, containers,
//! benches, bollards, traffic control, street plates, plaza dressing,
//! catenaries and bar terraces.
//!
//! Candidates are sampled along kerbs and building lines by arc length at
//! the profile's pitch and kept only where they stand on paving with room
//! around them, away from junctions and shop fronts. Nothing can land on a
//! facade, in a junction or in an alley too narrow for it, on any layout.

use forja_math::{Vec2, Vec3};

use crate::building::Frontage;
use crate::fabric::{Fabric, stage};
use crate::furniture::Aspect;
use crate::generators::CityPlan;
use crate::geom::{Rng, point_key};
use crate::lot::EdgeKind;
use crate::plan::line::left;
use crate::plan::{End, Located, NodeId, PlazaKind, Region, SegIndex, StreetId};
use crate::profile::{Bollards, Class, Control, Mount, Parking, ProfileLibrary};
use crate::texgen::SignKind;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ItemKind {
    /// Cast-iron lamp post: fernandina or candelabra.
    Lamp { candelabra: bool },
    RoadLight,
    TrafficLight { aspect: Aspect, ped_walk: bool, arm: f32 },
    Sign(SignKind),
    Plate(u8),
    Bollard { valencia: bool },
    Bin,
    Container(u8),
    Tree,
    Palm,
    Bench,
    Fountain(f32),
    Parterre(f32),
    FlowerBed(f32),
    Catenary { span: f32, laundry: bool },
    Terrace(u8),
}

/// One item: base position (y is the ground) and yaw (+Z of the module
/// faces `(sin yaw, cos yaw)`).
#[derive(Clone, Copy, Debug)]
pub struct Item {
    pub kind: ItemKind,
    pub pos: Vec3,
    pub yaw: f32,
}

fn yaw_of(face: Vec2) -> f32 {
    face.x.atan2(face.y)
}

/// Right of a direction in (x, z).
fn right(d: Vec2) -> Vec2 {
    Vec2::new(d.y, -d.x)
}

struct Env<'a> {
    f: &'a Fabric,
    lib: &'a ProfileLibrary,
    walk: Located,
    paved: Located,
    lots: SegIndex,
    fronts: &'a [Frontage],
}

impl Env<'_> {
    fn junction_clear(&self, p: Vec2, margin: f32) -> bool {
        self.f.ground.junctions.iter().all(|j| j.c.distance(p) > j.r + margin)
    }

    fn shop_clear(&self, p: Vec2) -> bool {
        self.fronts.iter().all(|fr| {
            let (o, r) = (Vec2::new(fr.out.x, fr.out.z), Vec2::new(fr.right.x, fr.right.z));
            let d = p - Vec2::new(fr.pos.x, fr.pos.z);
            !(d.dot(o) > -0.2 && d.dot(o) < 1.5 && d.dot(r).abs() < fr.width * 0.5 + 0.3)
        })
    }

    fn facade_clear(&self, p: Vec2, r: f32) -> bool {
        self.lots.nearest(p, r).is_none()
    }

    fn y(&self, p: Vec2) -> f32 {
        self.f.ground.height_at(p)
    }

    fn at(&self, kind: ItemKind, p: Vec2, face: Vec2) -> Item {
        Item { kind, pos: Vec3::new(p.x, self.y(p), p.y), yaw: yaw_of(face) }
    }
}

fn zone_valencia(plan: &CityPlan, z: u16) -> bool {
    plan.zone(z).map(|z| z.valencia()).unwrap_or(false)
}

/// Every item that does not depend on the buildings' eaves.
pub fn street_items(f: &Fabric, plan: &CityPlan, lib: &ProfileLibrary, fronts: &[Frontage], seed: u64) -> Vec<Item> {
    let mut lots = SegIndex::new(8.0);
    for (i, l) in f.lots.iter().enumerate() {
        for k in 0..l.ring.len() {
            lots.push(l.ring[k], l.ring[(k + 1) % l.ring.len()], i as u32);
        }
    }
    let g = &f.ground;
    let env = Env { f, lib, walk: Located::new(g.walk.clone()), paved: Located::new(Region::union_all([&g.walk, &g.plaza, &g.shared, &g.median])), lots, fronts };
    let mut out = Vec::new();
    for si in 0..f.graph.streets.len() {
        along_street(&env, plan, si as StreetId, seed, &mut out);
    }
    junction_control(&env, &mut out);
    one_way_signs(&env, plan, &mut out);
    street_plates(&env, &mut out);
    mouth_bollards(&env, plan, &mut out);
    plazas(&env, plan, seed, &mut out);
    isletas(&env, seed, &mut out);
    out
}

/// Lamps, trees, bins, containers, benches and kerb bollards of one street.
fn along_street(env: &Env, plan: &CityPlan, si: StreetId, seed: u64, out: &mut Vec<Item>) {
    let f = env.f;
    let s = f.graph.street(si);
    let p = env.lib.get(s.profile);
    if p.class == Class::Limit || s.len() < 4.0 {
        return;
    }
    let valencia = zone_valencia(plan, s.zone) || p.surface.walk == "SidewalkHex";
    let (m0, m1) = (f.graph.mouth(si, End::A, env.lib), s.len() - f.graph.mouth(si, End::B, env.lib));
    let road = s.carriageway * 0.5;
    let line = &s.line;
    let mut rng = Rng::keyed(seed, stage::DRESS, s.key);
    // Points `o` metres from the kerb into the walk on `side`, every
    // `pitch`, starting `phase` in.
    let row = |off: f32, pitch: f32, phase: f32, side: f32| -> Vec<(Vec2, Vec2, f32)> {
        let mut v = Vec::new();
        let mut t = m0 + phase;
        while t < m1 - 2.0 {
            let n = line.normal(t);
            let half = if s.carriageway > 0.0 { road } else { s.width_at(t) * 0.5 - off * 2.0 };
            let q = line.at(t) + n * side * (half + off);
            v.push((q, -n * side, t));
            t += pitch;
        }
        v
    };
    // The walk between kerb and facade must be wide enough for the item.
    let walk_width = |t: f32| s.width_at(t) * 0.5 - road;
    if p.vehicles && s.carriageway > 0.0 {
        match p.lamps.mount {
            Mount::Pole | Mount::Candelabra => {
                let cand = p.lamps.mount == Mount::Candelabra;
                for (k, side) in [1.0f32, -1.0].into_iter().enumerate() {
                    for (q, face, t) in row(0.6, p.lamps.pitch * 2.0, 10.0 + k as f32 * p.lamps.pitch, side) {
                        if walk_width(t) >= 1.4 && env.walk.contains_disc(q, 0.25) && env.junction_clear(q, 1.0) && env.facade_clear(q, 0.8) {
                            out.push(env.at(ItemKind::Lamp { candelabra: cand }, q, face));
                        }
                    }
                }
            }
            Mount::Road => {
                if p.median > 0.0 {
                    let mut t = m0 + 10.0;
                    while t < m1 - 5.0 {
                        for side in [-1.0f32, 1.0] {
                            let n = line.normal(t);
                            let q = line.at(t) + n * side * 0.9;
                            if env.f.ground.median.contains(q) {
                                out.push(env.at(ItemKind::RoadLight, q, n * side));
                            }
                        }
                        t += p.lamps.pitch;
                    }
                }
            }
            _ => {}
        }
        if p.trees > 0.0 {
            for side in [1.0f32, -1.0] {
                for (q, face, t) in row(1.0, p.trees, 5.5 + rng.range(0.0, 3.0), side) {
                    if walk_width(t) >= 3.5 && env.walk.contains_disc(q, 0.7) && env.junction_clear(q, 2.0) && env.shop_clear(q) && env.facade_clear(q, 1.8) && !out.iter().rev().take(64).any(|it| Vec2::new(it.pos.x, it.pos.z).distance(q) < 2.5) {
                        out.push(env.at(ItemKind::Tree, q, face));
                    }
                }
            }
        }
        if p.bollards == Bollards::Kerb {
            for side in [1.0f32, -1.0] {
                for (q, face, t) in row(0.3, 2.2, 6.0, side) {
                    if walk_width(t) >= 0.8 && env.walk.contains_disc(q, 0.12) && env.junction_clear(q, 0.5) && env.facade_clear(q, 0.5) {
                        out.push(env.at(ItemKind::Bollard { valencia }, q, face));
                    }
                }
            }
        }
        if p.containers && p.parking != Parking::None && s.len() > 40.0 {
            let t = s.len() * 0.5 + rng.range(-8.0, 8.0);
            for k in 0..4u8 {
                let tt = t + k as f32 * 2.0;
                let n = line.normal(tt);
                let q = line.at(tt) - n * (road - 1.1);
                if env.f.ground.in_carriage(q) && env.junction_clear(q, 3.0) {
                    out.push(Item { kind: ItemKind::Container(k), pos: Vec3::new(q.x, 0.0, q.y), yaw: yaw_of(n) });
                }
            }
        }
    }
    if p.bins > 0.0 {
        let mut k = 0;
        let off: f32 = if s.carriageway > 0.0 { 0.5 } else { 0.0 };
        for side in [1.0f32, -1.0] {
            for (q, face, t) in row(off.max(0.4), p.bins * 2.0, 20.0 + k as f32 * p.bins, side) {
                let wide = if s.carriageway > 0.0 { walk_width(t) >= 1.6 } else { s.width_at(t) >= 4.5 };
                let q = if s.carriageway > 0.0 { q } else { line.at(t) + (q - line.at(t)).normalize_or(Vec2::X) * (s.width_at(t) * 0.5 - 0.35) };
                if wide && env.paved.contains_disc(q, 0.22) && env.junction_clear(q, 1.0) && env.shop_clear(q) {
                    out.push(env.at(ItemKind::Bin, q, if s.carriageway > 0.0 { face } else { -face }));
                }
            }
            k += 1;
        }
    }
    if p.benches > 0.0 && (s.carriageway <= 0.0 || p.median > 0.0) {
        let mut t = m0 + p.benches * 0.5;
        while t < m1 - 3.0 {
            let n = line.normal(t);
            let q = if p.median > 0.0 { line.at(t) - n * 3.2 } else { line.at(t) };
            let ok = if p.median > 0.0 { env.f.ground.median.contains(q) } else { s.width_at(t) >= 7.0 && env.paved.contains_disc(q, 1.2) };
            if ok && env.junction_clear(q, 2.0) {
                out.push(env.at(ItemKind::Bench, q, line.tangent(t)));
            }
            t += p.benches;
        }
    }
    if p.palms && p.median > 0.0 {
        let mut t = m0 + 6.0;
        while t < m1 - 4.0 {
            let q = line.at(t);
            if env.f.ground.median.contains(q) {
                out.push(Item { kind: ItemKind::Palm, pos: Vec3::new(q.x, 0.17, q.y), yaw: t * 0.37 });
            }
            t += 11.0;
        }
    }
    // Pedestrian streets with pole lamps (plazas handle their own).
    if !p.vehicles && p.lamps.mount == Mount::Pole && p.class != Class::Plaza {
        let mut t = m0 + 6.0;
        let mut side = 1.0f32;
        while t < m1 - 3.0 {
            let n = line.normal(t);
            let q = line.at(t) + n * side * (s.width_at(t) * 0.5 - 1.2);
            if s.width_at(t) >= 6.0 && env.paved.contains_disc(q, 0.3) && env.facade_clear(q, 0.9) {
                out.push(env.at(ItemKind::Lamp { candelabra: valencia }, q, -n * side));
            }
            side = -side;
            t += p.lamps.pitch;
        }
    }
}

/// Car arms of a node: (street, end, heading into the street).
fn car_arms(env: &Env, n: NodeId) -> Vec<(StreetId, End, Vec2)> {
    env.f
        .graph
        .star(n)
        .iter()
        .filter(|a| {
            let s = env.f.graph.street(a.0);
            env.lib.get(s.profile).vehicles && s.carriageway > 0.0
        })
        .map(|a| (a.0, a.1, env.f.graph.street(a.0).heading(a.1, 6.0)))
        .collect()
}

/// Whether traffic on arm (s, e) drives towards the node.
fn arriving(env: &Env, s: StreetId, e: End) -> bool {
    match env.f.graph.street(s).oneway {
        1 => e == End::B,
        -1 => e == End::A,
        _ => true,
    }
}

/// Traffic lights or STOP / yield signs at every junction of three or
/// more car arms, one per arriving approach.
fn junction_control(env: &Env, out: &mut Vec<Item>) {
    let g = &env.f.graph;
    for n in 0..g.nodes.len() as NodeId {
        let arms = car_arms(env, n);
        if arms.len() < 3 {
            continue;
        }
        let signals = g.nodes[n as usize].signals || arms.iter().any(|a| env.lib.get(g.street(a.0).profile).control == Control::Signals);
        let rank = |a: &(StreetId, End, Vec2)| env.lib.get(g.street(a.0).profile).rank;
        // The straightest pair: green, or priority.
        let mut best = (0usize, 1usize, f32::INFINITY);
        for i in 0..arms.len() {
            for j in i + 1..arms.len() {
                let d = arms[i].2.dot(arms[j].2);
                let score = d - (rank(&arms[i]) + rank(&arms[j])) as f32 * 0.01;
                if score < best.2 {
                    best = (i, j, score);
                }
            }
        }
        let straight = arms[best.0].2.dot(arms[best.1].2) < -0.866;
        let top = arms.iter().map(rank).max().unwrap_or(0);
        for (k, &(s, e, d)) in arms.iter().enumerate() {
            if !arriving(env, s, e) {
                continue;
            }
            let st = g.street(s);
            let m = g.mouth(s, e, env.lib);
            let travel = -d;
            let p0 = g.nodes[n as usize].p;
            let q = p0 + d * (m + 0.5) + right(travel) * (st.carriageway * 0.5 + 0.45);
            if !env.paved.contains_disc(q, 0.15) {
                continue;
            }
            let green = k == best.0 || k == best.1;
            if signals {
                let avenue = env.lib.get(st.profile).class == Class::Avenue && env.lib.get(st.profile).median > 0.0;
                out.push(env.at(ItemKind::TrafficLight { aspect: if green { Aspect::Green } else { Aspect::Red }, ped_walk: !green, arm: if avenue { 5.0 } else { 0.0 } }, q, d));
            } else if !(straight && green) {
                let kind = if rank(&arms[k]) < top || straight { SignKind::Stop } else { SignKind::Yield };
                out.push(env.at(ItemKind::Sign(kind), q, d));
            }
            // A crossing sign a little further back on the other kerb.
            if env.lib.get(st.profile).crossings && st.len() > 2.0 * m + 24.0 {
                let q2 = p0 + d * (m + 9.0) - right(travel) * (st.carriageway * 0.5 + 0.4);
                if env.paved.contains_disc(q2, 0.15) && env.facade_clear(q2, 0.5) {
                    out.push(env.at(ItemKind::Sign(SignKind::Crossing), q2, d));
                }
            }
        }
    }
}

/// No-entry at the exits of one-way streets, speed limits at entries.
fn one_way_signs(env: &Env, plan: &CityPlan, out: &mut Vec<Item>) {
    let g = &env.f.graph;
    for (si, s) in g.streets.iter().enumerate() {
        let p = env.lib.get(s.profile);
        if !p.vehicles || s.carriageway <= 0.0 || s.len() < 30.0 {
            continue;
        }
        let si = si as StreetId;
        let road = s.carriageway * 0.5;
        for e in [End::A, End::B] {
            if g.degree(s.node(e)) < 2 {
                continue;
            }
            let d = s.heading(e, 6.0);
            let p0 = g.nodes[s.node(e) as usize].p;
            let m = g.mouth(si, e, env.lib);
            let entry = match s.oneway {
                1 => e == End::A,
                -1 => e == End::B,
                _ => e == End::A,
            };
            if s.oneway != 0 && !entry {
                let q = p0 + d * (m + 1.5) + right(d) * (road + 0.4);
                if env.paved.contains_disc(q, 0.15) {
                    out.push(env.at(ItemKind::Sign(SignKind::NoEntry), q, -d));
                }
            }
            if entry {
                let speed = p.speed.unwrap_or(if zone_valencia(plan, s.zone) { 50 } else { 30 });
                let kind = if speed <= 30 { SignKind::Speed30 } else { SignKind::Speed50 };
                let q = p0 + d * (m + 9.0) + right(d) * (road + 0.4);
                if env.paved.contains_disc(q, 0.15) && env.facade_clear(q, 0.4) {
                    out.push(env.at(ItemKind::Sign(kind), q, -d));
                }
            }
        }
    }
}

/// A street name plate on the corner walls of every junction sector.
fn street_plates(env: &Env, out: &mut Vec<Item>) {
    for l in &env.f.lots {
        let n = l.ring.len();
        for i in 0..n {
            // A corner of the lot between two street-facing edges of
            // different ways (a junction sector).
            let (e0, e1) = ((i + n - 1) % n, i);
            if !(l.edges[e0].is_street() && l.edges[e1].is_street()) {
                continue;
            }
            let (w0, w1) = (l.front(e0).way, l.front(e1).way);
            if w0 == w1 || w0 == u32::MAX || w1 == u32::MAX {
                continue;
            }
            for (e, toward_start) in [(e0, false), (e1, true)] {
                let (a, b) = l.edge(e);
                let len = a.distance(b);
                if len < 1.4 {
                    continue;
                }
                let d = (b - a) / len;
                let q = if toward_start { a + d * 0.6 } else { b - d * 0.6 };
                let outn = right(d);
                let name = (l.front(e).way % 8) as u8;
                out.push(Item { kind: ItemKind::Plate(name), pos: Vec3::new(q.x + outn.x * 0.01, l.base + 3.2, q.y + outn.y * 0.01), yaw: yaw_of(outn) });
            }
        }
        // Chamfered corners: the plate on the chaflán.
        let _ = EdgeKind::Corner;
    }
}

/// Bollards across the mouth where a vehicle-free street meets a car street.
fn mouth_bollards(env: &Env, plan: &CityPlan, out: &mut Vec<Item>) {
    let g = &env.f.graph;
    for n in 0..g.nodes.len() as NodeId {
        let star = g.star(n);
        let car: Vec<StreetId> = star.iter().filter(|a| env.lib.get(g.street(a.0).profile).vehicles && g.street(a.0).carriageway > 0.0).map(|a| a.0).collect();
        if car.is_empty() {
            continue;
        }
        let half_car = car.iter().map(|&s| g.street(s).width_at(0.0).max(g.street(s).min_width()) * 0.5).fold(0.0, f32::max);
        for &(s, e, _) in star {
            let st = g.street(s);
            let p = env.lib.get(st.profile);
            if p.vehicles || p.bollards != Bollards::Mouth {
                continue;
            }
            let d = st.heading(e, 4.0);
            let w = st.width_at(st.end_s(e));
            let c = g.nodes[n as usize].p + d * (half_car - 0.45);
            let k = ((w - 0.6) / 1.4).floor().max(1.0) as i32;
            for j in 0..=k {
                let o = -w * 0.5 + 0.3 + (w - 0.6) * j as f32 / k as f32;
                let q = c + left(d) * o;
                if env.paved.contains_disc(q, 0.1) && env.facade_clear(q, 0.25) {
                    out.push(env.at(ItemKind::Bollard { valencia: zone_valencia(plan, st.zone) }, q, d));
                }
            }
        }
    }
}

/// Plazas of any polygon: a fountain (Valencia) or parterre (Madrid) at
/// the pole, lamps and benches around.
fn plazas(env: &Env, plan: &CityPlan, seed: u64, out: &mut Vec<Item>) {
    // The plaza ground class: drafted squares and wide open paving, never the
    // roadway or a shared street that crosses them.
    for c in env.f.ground.plaza.components() {
        let Some((pole, r)) = c.pole() else { continue };
        if r < 4.0 {
            continue;
        }
        let zone = env.f.graph.near(pole, 200.0).first().map(|h| env.f.graph.street(h.0).zone).unwrap_or(0);
        let kind = plan.zone(zone).map(|z| z.plaza_kind).unwrap_or(PlazaKind::Paved);
        let valencia = zone_valencia(plan, zone);
        let y = env.y(pole);
        match kind {
            PlazaKind::Fountain => {
                let fr = (r * 0.3).clamp(2.0, 9.0);
                out.push(Item { kind: ItemKind::Fountain(fr), pos: Vec3::new(pole.x, y, pole.y), yaw: 0.0 });
                if r > fr + 9.0 {
                    for k in 0..4 {
                        let a = std::f32::consts::FRAC_PI_2 * k as f32 + std::f32::consts::FRAC_PI_4;
                        let q = pole + Vec2::from_angle(a) * (fr + 6.0);
                        if c.contains_disc(q, 3.5) {
                            out.push(Item { kind: ItemKind::FlowerBed(3.0), pos: Vec3::new(q.x, y, q.y), yaw: 0.0 });
                            out.push(Item { kind: ItemKind::Palm, pos: Vec3::new(q.x, y + 0.05, q.y), yaw: a });
                        }
                    }
                }
            }
            PlazaKind::Parterre | PlazaKind::Garden => {
                let s = (r * 0.35).clamp(2.5, 12.0);
                out.push(Item { kind: ItemKind::Parterre(s), pos: Vec3::new(pole.x, y, pole.y), yaw: 0.0 });
            }
            PlazaKind::Paved => {
                if r > 7.0 {
                    out.push(Item { kind: ItemKind::Fountain((r * 0.15).clamp(1.2, 2.5)), pos: Vec3::new(pole.x, y, pole.y), yaw: 0.0 });
                }
            }
        }
        // Lamps and benches along the inside of the edge.
        let inner = c.offset(-2.5, crate::plan::Join::Miter(0.5));
        let mut rng = Rng::keyed(seed, stage::DRESS, point_key(pole));
        for sh in inner.shapes() {
            let mut ring = sh.outer.clone();
            ring.push(ring[0]);
            let pl = crate::plan::Polyline::new(ring);
            let mut t = rng.range(3.0, 8.0);
            while t < pl.len() - 4.0 {
                let q = pl.at(t);
                let inward = pl.normal(t);
                if env.paved.contains_disc(q, 0.3) {
                    out.push(env.at(ItemKind::Lamp { candelabra: valencia }, q, inward));
                }
                let b = pl.at(t + 6.0) + pl.normal(t + 6.0) * 1.0;
                if t + 6.0 < pl.len() && env.paved.contains_disc(b, 1.0) {
                    out.push(env.at(ItemKind::Bench, b, -pl.normal(t + 6.0)));
                }
                t += 14.0;
            }
        }
    }
}

/// Isletas (paved islands touching no lot) over 15 m² get a tree or bench.
fn isletas(env: &Env, seed: u64, out: &mut Vec<Item>) {
    for c in env.f.ground.walk.components() {
        let a = c.area();
        if a < 15.0 || a > 600.0 {
            continue;
        }
        let Some((p, r)) = c.pole() else { continue };
        if env.lots.nearest(p, r + 0.3).is_some() {
            continue;
        }
        // Touching a lot anywhere disqualifies it.
        let touches = c.shapes().iter().flat_map(|s| s.outer.clone()).any(|q| env.lots.nearest(q, 0.05).is_some());
        if touches {
            continue;
        }
        let mut rng = Rng::keyed(seed, stage::DRESS, point_key(p));
        if r >= 1.0 {
            out.push(env.at(ItemKind::Tree, p, Vec2::from_angle(rng.range(0.0, 6.28))));
        } else if r >= 0.6 {
            out.push(env.at(ItemKind::Bench, p, Vec2::from_angle(rng.range(0.0, 6.28))));
        }
    }
}

/// Items that need the buildings: catenaries between facing eaves in
/// narrow streets, and bar terraces on wide sidewalks.
pub fn after_buildings(f: &Fabric, lib: &ProfileLibrary, eaves: &[f32], fronts: &[Frontage], seed: u64) -> Vec<Item> {
    let mut out = Vec::new();
    let mut lots = SegIndex::new(8.0);
    for (i, l) in f.lots.iter().enumerate() {
        for k in 0..l.ring.len() {
            lots.push(l.ring[k], l.ring[(k + 1) % l.ring.len()], i as u32);
        }
    }
    for s in &f.graph.streets {
        let p = lib.get(s.profile);
        if !p.catenary {
            continue;
        }
        let mut rng = Rng::keyed(seed, stage::DRESS ^ 0x55, s.key);
        let mut t = rng.range(6.0, 12.0);
        while t < s.len() - 6.0 {
            let c = s.line.at(t);
            let n = s.line.normal(t);
            let hl = lots.ray(c, n, 6.0);
            let hr = lots.ray(c, -n, 6.0);
            if let (Some((dl, il)), Some((dr, ir))) = (hl, hr) {
                let span = dl + dr;
                let (li, ri) = (lots.seg(il).2 as usize, lots.seg(ir).2 as usize);
                if span < 5.0 && span > 1.5 && li != ri {
                    let eave = eaves.get(li).copied().unwrap_or(9.0).min(eaves.get(ri).copied().unwrap_or(9.0));
                    let base = f.lots[li].base.max(f.lots[ri].base);
                    let h = (eave - 1.5 - base).clamp(4.5, 7.0) + base;
                    let mid = c + n * (dl - dr) * 0.5;
                    let laundry = rng.chance(0.45);
                    out.push(Item { kind: ItemKind::Catenary { span, laundry }, pos: Vec3::new(mid.x, h, mid.y), yaw: yaw_of(right(n)) });
                }
            }
            t += rng.range(12.0, 20.0);
        }
    }
    // Bar terraces where the walk in front is wide enough.
    let g = &f.ground;
    for fr in fronts.iter().filter(|fr| fr.bar && !fr.closed) {
        let p = Vec2::new(fr.pos.x, fr.pos.z);
        let o = Vec2::new(fr.out.x, fr.out.z);
        let Some((si, _, _)) = f.graph.near(p, 30.0).into_iter().min_by(|a, b| a.2.abs().total_cmp(&b.2.abs())) else { continue };
        let prof = lib.get(f.graph.street(si).profile);
        if !prof.terraces {
            continue;
        }
        let walk = if f.graph.street(si).carriageway > 0.0 { g.to_carriage(p + o * 0.05, o, 12.0).unwrap_or(12.0) } else { lots.ray(p + o * 0.05, o, 30.0).map(|h| h.0 * 0.5).unwrap_or(6.0) };
        let mut rng = Rng::keyed(seed, stage::DRESS ^ 0x77, point_key(p));
        if walk >= 3.5 && rng.chance(0.85) {
            terrace(fr, walk, &mut rng, &mut out);
        }
    }
    out
}

/// Table sets (some under parasols) in front of an open bar, between the
/// pedestrian strip along the facade and the trees at the kerb.
pub fn terrace(fr: &Frontage, sidewalk: f32, rng: &mut Rng, out: &mut Vec<Item>) {
    let parasols = rng.chance(0.5);
    let pitch = if parasols { 2.1 } else { 1.75 };
    let n = ((fr.width + 1.2) / pitch).floor().clamp(1.0, 3.0) as i32;
    let dist = (sidewalk * 0.52).clamp(1.6, 2.5);
    let yaw0 = fr.out.x.atan2(fr.out.z);
    for k in 0..n {
        let off = (k as f32 - (n - 1) as f32 * 0.5) * pitch;
        let v = rng.below(3) as u8 * 2 + parasols as u8;
        let p = fr.pos + fr.right * off + fr.out * dist;
        out.push(Item { kind: ItemKind::Terrace(v), pos: p, yaw: yaw0 + rng.range(-0.25, 0.25) });
    }
}
