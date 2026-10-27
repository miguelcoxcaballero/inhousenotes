//! Rule tests on a single junction: red lights, amber, STOP, give way,
//! pedestrians and the junction box.

use forja_ai::network::LaneKind;
use forja_ai::spec::{EndSpec, KMH, LaneSpec, NodeSpec, RoadGraphSpec, StreetSpec};
use forja_ai::{Control, PedSignal, PedState, RoadNetwork, SimConfig, TrafficSim};
use forja_math::Vec2;

const ARM: f32 = 120.0;
const BOX: f32 = 7.0;

/// A 4-way junction at the origin; arms N, S, E, W (streets 0..4) with
/// one lane each way, `ns` / `ew` controlling the traffic arriving from
/// the north-south and east-west arms.
fn cross(ns: Control, ew: Control) -> RoadGraphSpec {
    let dirs = [Vec2::Y, -Vec2::Y, Vec2::X, -Vec2::X];
    let mut spec = RoadGraphSpec { nodes: vec![NodeSpec { pos: Vec2::ZERO, signal_offset: 0.0 }], ..Default::default() };
    for (k, d) in dirs.iter().enumerate() {
        spec.nodes.push(NodeSpec { pos: *d * (ARM + BOX), signal_offset: 0.0 });
        let control = if k < 2 { ns } else { ew };
        spec.streets.push(StreetSpec {
            nodes: [0, k + 1],
            points: vec![*d * BOX, *d * (ARM + BOX)],
            road_half: 3.5,
            sidewalk: 3.0,
            median_half: 0.0,
            lanes: vec![LaneSpec { offset: -1.75, forward: true, width: 3.2 }, LaneSpec { offset: 1.75, forward: false, width: 3.2 }],
            parking: vec![],
            speed_limit: 50.0 * KMH,
            ends: [EndSpec { control, crosswalk: Some(3.0), crosswalk_width: 4.0, stop_line: 6.0, ..Default::default() }, EndSpec::default()],
        });
    }
    spec
}

/// Lane of street `k` arriving at the junction / leaving it.
fn inbound(net: &RoadNetwork, k: usize) -> u32 {
    net.streets[k].lanes.iter().copied().find(|&l| matches!(net.lanes[l as usize].kind, LaneKind::Street { forward: false, .. })).expect("inbound")
}

fn outbound(net: &RoadNetwork, k: usize) -> u32 {
    net.streets[k].lanes.iter().copied().find(|&l| matches!(net.lanes[l as usize].kind, LaneKind::Street { forward: true, .. })).expect("outbound")
}

fn sim(spec: &RoadGraphSpec) -> TrafficSim {
    let mut s = TrafficSim::new(RoadNetwork::build(spec), SimConfig { parked_fill: 0.0, ..Default::default() });
    s.manual = true;
    s.focus = Vec2::ZERO;
    s
}

fn phase_of(sim: &TrafficSim, lane: u32) -> usize {
    let a = sim.net.lanes[lane as usize].approach.expect("approach");
    sim.net.approaches[a as usize].phase.expect("phase") as usize
}

fn run(sim: &mut TrafficSim, secs: f32, mut each: impl FnMut(&TrafficSim)) {
    let n = (secs / sim.cfg.dt) as usize;
    for _ in 0..n {
        sim.step();
        each(sim);
    }
}

#[test]
fn idm_stops_before_a_red_light_stop_line() {
    let mut s = sim(&cross(Control::Signal, Control::Signal));
    let (from, to) = (inbound(&s.net, 1), outbound(&s.net, 0));
    let p = phase_of(&s, from);
    s.net.signals[0].forced = Some(1 - p);
    let id = s.spawn_vehicle_to(from, 10.0, 13.9, to).expect("spawn");
    run(&mut s, 30.0, |_| {});
    let v = &s.vehicles[s.vehicle_index(id).expect("alive")];
    let stop = s.net.lanes[from as usize].stop_s;
    assert_eq!(v.lane, from);
    assert!(v.v < 0.05, "still moving at {}", v.v);
    assert!(v.s <= stop && v.s > stop - 2.5, "front at {} for a line at {stop}", v.s);
    // Green: it goes, and nobody ran the red.
    s.net.signals[0].forced = Some(p);
    run(&mut s, 15.0, |_| {});
    assert!(s.vehicle_index(id).is_none_or(|i| s.vehicles[i].lane != from));
    assert_eq!(s.stats.red_light_violations, 0);
    assert_eq!(s.stats.vehicle_collisions, 0);
}

#[test]
fn amber_close_goes_far_stops() {
    for (dist, expect_stop) in [(9.0f32, false), (55.0, true)] {
        let mut s = sim(&cross(Control::Signal, Control::Signal));
        let (from, to) = (inbound(&s.net, 1), outbound(&s.net, 0));
        let p = phase_of(&s, from);
        let ctl = &s.net.signals[0];
        // Just before this approach turns amber.
        let t_amber = ctl.phase_start(p) + ctl.phases[p].green - ctl.offset;
        s.t = t_amber as f64 - 0.05;
        let stop = s.net.lanes[from as usize].stop_s;
        let id = s.spawn_vehicle_to(from, stop - dist, 13.0, to).expect("spawn");
        let mut min_v: f32 = 99.0;
        run(&mut s, 12.0, |s| {
            if let Some(i) = s.vehicle_index(id)
                && s.vehicles[i].lane == from
            {
                min_v = min_v.min(s.vehicles[i].v);
            }
        });
        if expect_stop {
            assert!(min_v < 0.1, "should have stopped (min {min_v})");
        } else {
            assert!(min_v > 8.0, "should have gone through (min {min_v})");
        }
        assert_eq!(s.stats.red_light_violations, 0, "{dist} m");
    }
}

#[test]
fn stop_sign_means_a_full_stop_then_go() {
    let mut s = sim(&cross(Control::Stop, Control::None));
    let (from, to) = (inbound(&s.net, 1), outbound(&s.net, 0));
    let id = s.spawn_vehicle_to(from, 30.0, 11.0, to).expect("spawn");
    let stop = s.net.lanes[from as usize].stop_s;
    let mut min_near = 99.0f32;
    let mut passed = false;
    run(&mut s, 25.0, |s| match s.vehicle_index(id) {
        Some(i) => {
            let v = &s.vehicles[i];
            if v.lane == from && (stop - v.s) < 3.0 {
                min_near = min_near.min(v.v);
            }
            passed |= v.lane != from;
        }
        None => passed = true,
    });
    assert!(min_near < 0.15, "did not stop: {min_near}");
    assert!(passed, "never went on");
    assert_eq!(s.stats.stop_sign_violations, 0);
}

#[test]
fn give_way_to_the_priority_road() {
    let spec = cross(Control::Yield, Control::None);
    // Alone: slows but does not need to stop.
    let mut s = sim(&spec);
    let (from, to) = (inbound(&s.net, 1), outbound(&s.net, 0));
    let id = s.spawn_vehicle_to(from, 60.0, 8.0, to).expect("spawn");
    let mut min_v = 99.0f32;
    run(&mut s, 20.0, |s| {
        if let Some(i) = s.vehicle_index(id)
            && s.vehicles[i].lane == from
        {
            min_v = min_v.min(s.vehicles[i].v);
        }
    });
    assert!(min_v > 1.0, "stopped without traffic ({min_v})");
    // With a car on the priority road: waits for it.
    let mut s = sim(&spec);
    let main = s.spawn_vehicle_to(inbound(&s.net, 2), ARM - 45.0, 11.0, outbound(&s.net, 3)).expect("main");
    let minor = s.spawn_vehicle_to(from, ARM - 25.0, 8.0, to).expect("minor");
    let mut minor_entered_at = None;
    let mut main_entered_at = None;
    let mut step = 0;
    run(&mut s, 25.0, |s| {
        step += 1;
        for (id, slot) in [(main, &mut main_entered_at), (minor, &mut minor_entered_at)] {
            if slot.is_none() && s.vehicle_index(id).is_some_and(|i| s.net.lanes[s.vehicles[i].lane as usize].is_connector()) {
                *slot = Some(step);
            }
        }
    });
    let (ma, mi) = (main_entered_at.expect("main crossed"), minor_entered_at.expect("minor crossed"));
    assert!(mi > ma, "minor entered at {mi}, main at {ma}");
    assert_eq!(s.stats.vehicle_collisions, 0);
    assert_eq!(s.stats.box_violations, 0);
}

#[test]
fn pedestrians_wait_for_the_walk_signal() {
    let mut s = sim(&cross(Control::Signal, Control::Signal));
    // The crosswalk over the south arm walks with the east-west phase.
    let cw = s.net.crosswalks.iter().position(|c| c.street == 1).expect("crosswalk") as u32;
    let cwp = s.net.crosswalks[cw as usize].phase.expect("phase") as usize;
    s.net.signals[0].forced = Some(1 - cwp);
    // Kerb nodes of that crosswalk.
    let edge = s.net.ped.edges.iter().find(|e| e.kind == forja_ai::network::WalkKind::Crosswalk { crosswalk: cw }).expect("edge").clone();
    let (near, far) = (edge.a, edge.b);
    let start = s.net.ped.nodes[near as usize].pos;
    let id = s.spawn_ped(start, near, far).expect("ped");
    {
        let p = s.peds.iter_mut().find(|p| p.id == id).expect("ped");
        p.path = vec![far];
        p.from = near;
        p.jaywalker = false;
        p.state = PedState::Wait { cw };
    }
    assert_eq!(s.net.crosswalk_signal(cw, s.t), Some(PedSignal::DontWalk));
    let c = s.net.crosswalks[cw as usize].clone();
    let mut stepped_in = false;
    run(&mut s, 20.0, |s| {
        let p = s.peds.iter().find(|p| p.id == id).expect("ped");
        stepped_in |= c.contains(p.pos, -0.3);
    });
    assert!(!stepped_in, "crossed on DON'T WALK");
    s.net.signals[0].forced = Some(cwp);
    run(&mut s, 20.0, |_| {});
    assert!(s.stats.crossings >= 1, "never crossed on WALK");
    assert_eq!(s.stats.ped_collisions, 0);
}

#[test]
fn turning_cars_give_way_to_pedestrians() {
    let mut s = sim(&cross(Control::Signal, Control::Signal));
    // North-south green; a car from the south turns right into the east
    // arm while people cross that arm on their WALK.
    let from = inbound(&s.net, 1);
    let to = outbound(&s.net, 2);
    let p = phase_of(&s, from);
    s.net.signals[0].forced = Some(p);
    let cw = s.net.crosswalks.iter().position(|c| c.street == 2).expect("crosswalk") as u32;
    assert_eq!(s.net.crosswalk_signal(cw, s.t), Some(PedSignal::Walk));
    let edge = s.net.ped.edges.iter().find(|e| e.kind == forja_ai::network::WalkKind::Crosswalk { crosswalk: cw }).expect("edge").clone();
    for k in 0..3 {
        let (a, b) = if k % 2 == 0 { (edge.a, edge.b) } else { (edge.b, edge.a) };
        let pos = s.net.ped.nodes[a as usize].pos;
        let id = s.spawn_ped(pos + Vec2::new(0.0, k as f32 * 0.5), a, b).expect("ped");
        let pd = s.peds.iter_mut().find(|p| p.id == id).expect("ped");
        pd.path = vec![b];
        pd.from = a;
        pd.state = PedState::Wait { cw };
    }
    s.spawn_vehicle_to(from, ARM - 30.0, 9.0, to).expect("car");
    run(&mut s, 30.0, |_| {});
    assert!(s.stats.crossings >= 3, "{} crossings", s.stats.crossings);
    assert_eq!(s.stats.ped_collisions, 0);
}

#[test]
fn nobody_enters_an_occupied_junction_box() {
    let mut s = sim(&cross(Control::Signal, Control::Signal));
    // A broken-down car in the box on the east-west straight connector.
    let ew_in = inbound(&s.net, 2);
    let conn = s.net.lanes[ew_in as usize].next.iter().copied().find(|&c| s.net.lanes[c as usize].turn() == forja_ai::network::Turn::Straight).expect("straight");
    let dead = s.spawn_vehicle_to(conn, 9.0, 0.0, outbound(&s.net, 3)).expect("dead");
    let i = s.vehicle_index(dead).expect("dead");
    s.vehicles[i].params.a_max = 1e-4;
    s.vehicles[i].params.speed_factor = 1e-4;
    // North-south green, a car arriving straight across its path.
    let from = inbound(&s.net, 1);
    let p = phase_of(&s, from);
    s.net.signals[0].forced = Some(p);
    let id = s.spawn_vehicle_to(from, 40.0, 10.0, outbound(&s.net, 0)).expect("car");
    run(&mut s, 20.0, |_| {});
    let v = &s.vehicles[s.vehicle_index(id).expect("waiting")];
    assert_eq!(v.lane, from, "entered the box");
    assert!(v.v < 0.1);
    // Cleared: it goes.
    s.vehicles.retain(|v| v.id != dead);
    run(&mut s, 15.0, |_| {});
    assert!(s.vehicle_index(id).is_none_or(|i| s.vehicles[i].lane != from));
    assert_eq!(s.stats.box_violations, 0);
    assert_eq!(s.stats.vehicle_collisions, 0);
}

#[test]
fn a_car_brakes_for_the_player_standing_in_the_road() {
    let mut s = sim(&cross(Control::None, Control::None));
    let from = inbound(&s.net, 1);
    let lane = &s.net.lanes[from as usize];
    let at = lane.path.point(70.0);
    s.externals = vec![forja_ai::External { pos: at, vel: Vec2::ZERO, fwd: Vec2::X, half: Vec2::new(0.3, 0.3), kind: forja_ai::ExternalKind::Player }];
    let id = s.spawn_vehicle_to(from, 10.0, 13.0, outbound(&s.net, 0)).expect("car");
    let mut honked = false;
    run(&mut s, 20.0, |s| honked |= s.events.iter().any(|e| matches!(e, forja_ai::SimEvent::Honk { .. })));
    let v = &s.vehicles[s.vehicle_index(id).expect("car")];
    assert!(v.v < 0.1 && v.s < 70.0 - 0.3, "s {} v {}", v.s, v.v);
    assert!(honked, "no honk");
    assert_eq!(s.stats.player_collisions, 0);
}

/// Step at which vehicle `id` first enters a junction connector.
fn entry_step(s: &mut TrafficSim, ids: &[u64], secs: f32) -> Vec<Option<usize>> {
    let mut out = vec![None; ids.len()];
    let mut step = 0;
    run(s, secs, |s| {
        step += 1;
        for (k, id) in ids.iter().enumerate() {
            if out[k].is_none() && s.vehicle_index(*id).is_some_and(|i| s.net.lanes[s.vehicles[i].lane as usize].is_connector()) {
                out[k] = Some(step);
            }
        }
    });
    out
}

#[test]
fn all_way_stop_goes_in_order_of_arrival() {
    let mut s = sim(&cross(Control::Stop, Control::Stop));
    // The east car arrives first, the south one a few seconds later.
    let first = s.spawn_vehicle_to(inbound(&s.net, 2), ARM - 30.0, 8.0, outbound(&s.net, 3)).expect("east");
    let second = s.spawn_vehicle_to(inbound(&s.net, 1), ARM - 60.0, 8.0, outbound(&s.net, 0)).expect("south");
    let e = entry_step(&mut s, &[first, second], 30.0);
    let (a, b) = (e[0].expect("first crossed"), e[1].expect("second crossed"));
    assert!(a < b, "{a} {b}");
    assert_eq!(s.stats.stop_sign_violations, 0);
    assert_eq!(s.stats.vehicle_collisions, 0);
}

#[test]
fn permissive_left_turn_waits_for_a_gap_in_oncoming_traffic() {
    let mut s = sim(&cross(Control::Signal, Control::Signal));
    let from = inbound(&s.net, 1);
    let p = phase_of(&s, from);
    s.net.signals[0].forced = Some(p);
    // Left from the south into the west arm, oncoming straight from the north.
    let left = s.spawn_vehicle_to(from, ARM - 20.0, 6.0, outbound(&s.net, 3)).expect("left");
    let oncoming = s.spawn_vehicle_to(inbound(&s.net, 0), ARM - 50.0, 12.0, outbound(&s.net, 1)).expect("oncoming");
    let e = entry_step(&mut s, &[left, oncoming], 30.0);
    let (l, o) = (e[0].expect("left turned"), e[1].expect("oncoming passed"));
    assert!(l > o, "turned at {l} before the oncoming car at {o}");
    assert_eq!(s.stats.vehicle_collisions, 0);
    assert_eq!(s.stats.box_violations, 0);
}
