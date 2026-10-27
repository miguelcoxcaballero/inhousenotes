//! Diagnostics: runs the grid city for 5 minutes and prints every rule
//! violation and collision with the state of the vehicles involved, and
//! why vehicles wait (`cargo run --release -p forja_ai --example diag`).
use forja_ai::spec::{GridOptions, grid_city};
use forja_ai::network::LaneKind;
use forja_ai::{RoadNetwork, SimConfig, TrafficSim, SimEvent};
use forja_math::Vec2;
fn desc(sim: &TrafficSim, id: u64) -> String {
    let Some(i) = sim.vehicle_index(id) else { return format!("{id} gone") };
    let v = &sim.vehicles[i];
    let l = &sim.net.lanes[v.lane as usize];
    let k = match l.kind { LaneKind::Connector { node, turn, from, .. } => format!("conn n{node} {turn:?} from {from} ctl {:?}", sim.net.lanes[from as usize].approach.map(|a| sim.net.approaches[a as usize].control)), LaneKind::Street { street, .. } => format!("street {street}") };
    format!("v{} lane {} ({k}) s {:.1}/{:.1} v {:.1} acc {:.1} pos {:?} gate {} passed {} blocked {:?} route {:?} len {}", v.id, v.lane, v.s, l.len(), v.v, v.acc, v.pos, v.gate_open, v.passed_line, v.blocked_by, v.route.iter().take(3).collect::<Vec<_>>(), v.len)
}
fn main() {
    let net = RoadNetwork::build(&grid_city(&GridOptions::default()));
    let cfg = SimConfig { seed: 7, max_vehicles: 140, vehicle_density: 18.0, max_peds: 220, ped_density: 45.0, spawn_min: 0.0, spawn_max: 1000.0, despawn: 2000.0, near: 120.0, ..Default::default() };
    let mut sim = TrafficSim::new(net, cfg);
    let mut nv = 0;
    for step in 0..6000 {
        sim.step();
        for e in sim.events.drain(..).collect::<Vec<_>>() {
            if let SimEvent::Collision { a, b, pos } = e {
                println!("t {:.2} COLLISION {a:?} {b:?} at {pos:?}", sim.t);
                for r in [a, b] { if let forja_ai::sim::AgentRef::Vehicle(id) = r { println!("   {}", desc(&sim, id)); } }
            }
        }
        while nv < sim.violations.len() {
            let (t, k, id, lane) = sim.violations[nv];
            println!("t {t:.2} {k} v{id} lane {lane}: {}", desc(&sim, id));
            if let LaneKind::Connector { .. } = sim.net.lanes[lane as usize].kind {
                for cf in &sim.net.lanes[lane as usize].conflicts {
                    for v in sim.vehicles.iter().filter(|v| v.lane == cf.other) { println!("    other {} conf o_in {:.1} o_out {:.1} s_in {:.1}", desc(&sim, v.id), cf.o_in, cf.o_out, cf.s_in); }
                }
            }
            nv += 1;
        }
        if step % 3000 == 2999 {
            let mut hist: std::collections::BTreeMap<String, usize> = Default::default();
            for v in &sim.vehicles {
                if v.wait_t > 20.0 {
                    let k = format!("{:?} / {:?}", v.why, v.blocked_by.map(|b| match b { forja_ai::sim::AgentRef::Vehicle(o) => format!("veh(wait {:.0})", sim.vehicle_index(o).map_or(-1.0, |i| sim.vehicles[i].wait_t)), x => format!("{x:?}") }));
                    *hist.entry(k).or_default() += 1;
                }
            }
            println!("t {:.0}: {} vehicles, stuck>20s: {:?}", sim.t, sim.vehicles.len(), hist);
            let stuck: Vec<_> = sim.vehicles.iter().filter(|v| v.wait_t > 20.0 && v.why == forja_ai::sim::Why::Held).take(4).map(|v| v.id).collect();
            for id in stuck {
                println!("   {}", desc(&sim, id));
                let v = &sim.vehicles[sim.vehicle_index(id).unwrap()];
                let c = *v.route.front().unwrap();
                for cf in &sim.net.lanes[c as usize].conflicts {
                    for h in sim.vehicles.iter().filter(|o| o.holds == Some(cf.other)) {
                        println!("      held by {} (sibling {})", desc(&sim, h.id), cf.sibling);
                    }
                }
            }
        }
    }
    let _ = Vec2::ZERO;
    println!("{:?}", sim.stats);
}
