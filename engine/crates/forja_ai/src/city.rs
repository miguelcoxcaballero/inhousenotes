//! Adapter for `forja_city`'s exported [`TrafficInfo`]: its street
//! pieces, sign plates and traffic lights become a [`RoadGraphSpec`], with
//! every sign bound to the street end it faces.

use forja_city::texgen::SignKind;
use forja_city::traffic::TrafficInfo;
use forja_math::Vec2;

use crate::adapter::{SegmentIn, SignIn, SignRule, SignUse, apply_signal_heads, build_graph};
use crate::spec::{KMH, LaneSpec, ParkingSpec, RoadGraphSpec, RoadSource};

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

pub fn roads(info: &TrafficInfo) -> CityRoads {
    let segs: Vec<SegmentIn> = info
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
        .collect();
    let signs: Vec<SignIn> = info.signs.iter().map(|s| SignIn { rule: rule(s.kind), pos: s.pos, facing: s.facing }).collect();
    let (mut spec, uses) = build_graph(&segs, &signs);
    let heads: Vec<(Vec2, Vec2)> = info.signals.iter().map(|p| (p.pos, p.facing)).collect();
    apply_signal_heads(&mut spec, &heads);
    // A green wave along the boulevard (z) at about 50 km/h.
    for n in &mut spec.nodes {
        n.signal_offset = n.pos.y / 13.9 + n.pos.x.abs() / 40.0;
    }
    spec.plazas = info.plazas.clone();
    spec.benches = info.benches.clone();
    CityRoads { spec, heads, signs: uses }
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
}
