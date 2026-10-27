//! Adapter for `forja_city`'s exported [`TrafficInfo`]: its street
//! pieces, sign plates and traffic lights become a [`RoadGraphSpec`], with
//! every sign bound to the street end it faces.

use forja_city::texgen::SignKind;
use forja_city::traffic::{TrafficEnd, TrafficInfo};
use forja_math::Vec2;

use crate::adapter::{SegmentIn, SignIn, SignRule, SignUse, apply_signal_heads, apply_signs, build_graph};
use crate::spec::{Control, EndSpec, KMH, LaneSpec, NodeSpec, ParkingSpec, RoadGraphSpec, RoadSource, StreetSpec};

/// The road graph and what was bound to what.
pub struct CityRoads {
    pub spec: RoadGraphSpec,
    /// Signal heads `(position, plate normal)`, in `TrafficInfo::signals`
    /// order, for [`crate::RoadNetwork::bind_heads`].
    pub heads: Vec<(Vec2, Vec2)>,
    pub signs: Vec<SignUse>,
}

pub fn rule(kind: SignKind) -> SignRule {
    match kind {
        SignKind::Stop => SignRule::Stop,
        SignKind::Yield => SignRule::Yield,
        SignKind::Speed50 => SignRule::Speed(50),
        SignKind::Speed30 => SignRule::Speed(30),
        SignKind::Crossing => SignRule::Crossing,
        SignKind::NoEntry => SignRule::NoEntry,
        SignKind::AheadOnly => SignRule::AheadOnly,
        SignKind::NoParking => SignRule::NoParking,
        SignKind::Parking => SignRule::Parking,
    }
}

/// The road graph a generator described with its own topology: its
/// junctions and the streets between them as given (curved centrelines
/// included), zebras and stop lines where it painted them.
fn given_graph(info: &TrafficInfo) -> RoadGraphSpec {
    let streets = info
        .streets
        .iter()
        .map(|s| {
            let nodes = s.nodes.expect("topology");
            let given = |e: usize| s.ends.map_or(TrafficEnd { crosswalk: s.crosswalk, stop_line: s.stop_line }, |x| x[e]);
            let end = |e: usize| EndSpec {
                control: Control::None,
                crosswalk: given(e).crosswalk,
                crosswalk_width: 4.0,
                stop_line: given(e).stop_line,
                ahead_only: false,
                median_open: s.median_half > 0.0 && info.median_openings.iter().any(|o| o.distance(info.nodes[nodes[e]]) < 12.0),
            };
            StreetSpec {
                nodes,
                points: s.centreline(),
                road_half: s.road_half,
                sidewalk: s.sidewalk,
                median_half: s.median_half,
                lanes: s.lanes.iter().map(|l| LaneSpec { offset: l.offset, forward: l.forward, width: l.width }).collect(),
                parking: s.parking.iter().map(|&offset| ParkingSpec { offset, allowed: true }).collect(),
                speed_limit: s.speed_kmh * KMH,
                ends: [end(0), end(1)],
            }
        })
        .collect();
    RoadGraphSpec { nodes: info.nodes.iter().map(|&pos| NodeSpec { pos, signal_offset: 0.0 }).collect(), streets, ..Default::default() }
}

pub fn roads(info: &TrafficInfo) -> CityRoads {
    let signs: Vec<SignIn> = info.signs.iter().map(|s| SignIn { rule: rule(s.kind), pos: s.pos, facing: s.facing }).collect();
    let (mut spec, uses) = if info.has_topology() {
        let mut spec = given_graph(info);
        let uses = apply_signs(&mut spec, &signs);
        (spec, uses)
    } else {
        build_graph(&soup(info), &signs)
    };
    let heads: Vec<(Vec2, Vec2)> = info.signals.iter().map(|p| (p.pos, p.facing)).collect();
    apply_signal_heads(&mut spec, &heads);
    // A green wave along z at about 50 km/h.
    for n in &mut spec.nodes {
        n.signal_offset = n.pos.y / 13.9 + n.pos.x.abs() / 40.0;
    }
    spec.plazas = info.plazas.clone();
    spec.benches = info.benches.clone();
    CityRoads { spec, heads, signs: uses }
}

/// The streets as straight pieces for [`build_graph`].
fn soup(info: &TrafficInfo) -> Vec<SegmentIn> {
    info
        .streets
        .iter()
        .map(|s| SegmentIn {
            a: s.a,
            b: s.b,
            half_width: s.half_width,
            road_half: s.road_half,
            sidewalk: s.sidewalk,
            median_half: s.median_half,
            lanes: s.lanes.iter().map(|l| LaneSpec { offset: l.offset, forward: l.forward, width: l.width }).collect(),
            parking: s.parking.iter().map(|&offset| ParkingSpec { offset, allowed: true }).collect(),
            speed_limit: s.speed_kmh * KMH,
            crosswalk: s.crosswalk,
            stop_line: s.stop_line,
            median_openings: if s.median_half > 0.0 { info.median_openings.clone() } else { Vec::new() },
        })
        .collect()
}

impl RoadSource for TrafficInfo {
    fn road_graph(&self) -> RoadGraphSpec {
        roads(self).spec
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RoadNetwork, SimConfig, TrafficSim};
    use forja_city::layout::{CityConfig, build_city};
    use forja_city::{Kit, StyleLibrary};

    #[test]
    fn the_generated_city_drives_by_its_own_signs_and_lights() {
        let kit = Kit::load(None);
        let city = build_city(&kit, &StyleLibrary::builtin(), &CityConfig::small());
        let info = &city.traffic;
        assert!(!info.streets.is_empty() && !info.signals.is_empty() && !info.signs.is_empty());
        let r = roads(info);
        let net = RoadNetwork::build(&r.spec);
        // Every STOP / give-way plate governs a street end.
        for (s, u) in info.signs.iter().zip(&r.signs) {
            if matches!(s.kind, SignKind::Stop | SignKind::Yield) {
                assert!(matches!(u, SignUse::End(..)), "{:?} at {:?} unbound", s.kind, s.pos);
            }
        }
        // Every light faces an approach it controls, except those at the
        // boulevard's ends where it leaves the map (no junction there).
        let bound = net.bind_heads(&r.heads);
        let dead_ends: Vec<Vec2> = net.nodes.iter().filter(|n| n.ends.len() <= 1).map(|n| n.pos).collect();
        let unbound: Vec<_> = bound.iter().zip(&r.heads).filter(|(b, _)| b.approach.is_none()).map(|(_, h)| h.0).filter(|p| !dead_ends.iter().any(|d| d.distance(*p) < 25.0)).collect();
        assert!(unbound.is_empty(), "unbound heads {unbound:?}");
        // Pedestrian heads serve the zebra across their street (the
        // boulevard has none at its side junctions: a steady red man).
        assert!(bound.iter().filter_map(|b| b.approach.map(|a| (a, b.crosswalk))).all(|(a, c)| c.is_some() || net.approaches[a as usize].lanes.len() >= 3));
        let strong = net.strong.iter().filter(|x| **x).count();
        assert!(strong * 10 > net.lanes.len() * 7, "{strong} of {}", net.lanes.len());
        let mut sim = TrafficSim::new(net, SimConfig { spawn_min: 0.0, spawn_max: 2000.0, despawn: 4000.0, max_vehicles: 60, max_peds: 80, ..Default::default() });
        for _ in 0..(120.0 / sim.cfg.dt) as usize {
            sim.step();
        }
        let st = sim.stats;
        println!("{st:?}");
        assert!(sim.vehicles.len() >= 15, "{}", sim.vehicles.len());
        assert_eq!((st.vehicle_collisions, st.ped_collisions, st.red_light_violations, st.stop_sign_violations, st.box_violations), (0, 0, 0, 0, 0), "{:?}", sim.violations);
        // Moving at well over 1 m/s on average, waits included.
        assert!(st.distance > sim.vehicles.len() as f64 * 120.0 * 1.5, "{}", st.distance);
    }

    /// The full map at review density: staggered boulevard junctions with
    /// 5.5 m links, L-junctions at the map edge, many pedestrians.
    #[test]
    fn the_full_city_at_review_density_stays_clean() {
        let city = build_city(&Kit::load(None), &StyleLibrary::builtin(), &CityConfig::default());
        let net = RoadNetwork::build(&roads(&city.traffic).spec);
        let cfg = SimConfig { seed: 3, max_vehicles: 220, vehicle_density: 14.0, max_peds: 400, ped_density: 40.0, spawn_min: 0.0, spawn_max: 5000.0, despawn: 9000.0, near: 150.0, ..Default::default() };
        let mut sim = TrafficSim::new(net, cfg);
        for _ in 0..(150.0 / sim.cfg.dt) as usize {
            sim.step();
            sim.events.clear();
        }
        let st = sim.stats;
        assert!(sim.vehicles.len() >= 60 && sim.peds.len() >= 200, "{} {}", sim.vehicles.len(), sim.peds.len());
        assert_eq!((st.vehicle_collisions, st.ped_collisions, st.red_light_violations, st.stop_sign_violations, st.box_violations), (0, 0, 0, 0, 0), "{:?}", sim.violations);
    }
}
