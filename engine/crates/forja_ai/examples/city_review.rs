//! Debug frames of traffic on the procedural Spanish city itself (the
//! sandbox's `spain` map: Madrid barrio, boulevard, Valencian Ensanche):
//!
//! ```text
//! cargo run --release -p forja_ai --features city --example city_review -- OUT_DIR [SECONDS]
//! ```
//! Writes `city_map.png` (whole map) and `city_junction_NN.png` (a
//! signalled junction through one cycle), and prints the statistics.

use forja_ai::{RoadNetwork, SimConfig, TrafficSim, debug};
use forja_city::layout::{CityConfig, build_city};
use forja_city::{Kit, StyleLibrary};
use forja_math::Vec2;

fn save(c: debug::Canvas, path: &std::path::Path) {
    image::RgbImage::from_raw(c.width, c.height, c.rgb).expect("image").save(path).expect("png");
    println!("{}", path.display());
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let out = std::path::PathBuf::from(args.first().cloned().unwrap_or_else(|| "city_review".into()));
    let warm: f32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(150.0);
    std::fs::create_dir_all(&out).expect("out dir");
    let t = std::time::Instant::now();
    let config = if std::env::var_os("FORJA_SMALL_CITY").is_some() { CityConfig::small() } else { CityConfig::default() };
    let city = build_city(&Kit::load(None), &StyleLibrary::builtin(), &config);
    println!("city built in {:.1?}: {} streets, {} signs, {} lights", t.elapsed(), city.traffic.streets.len(), city.traffic.signs.len(), city.traffic.signals.len());
    let roads = forja_ai::city::roads(&city.traffic);
    let net = RoadNetwork::build(&roads.spec);
    let bound = net.bind_heads(&roads.heads);
    println!(
        "network: {} lanes, {} junctions, {} signal controllers, {} crosswalks, heads bound {}/{}",
        net.lanes.len(),
        net.nodes.len(),
        net.signals.len(),
        net.crosswalks.len(),
        bound.iter().filter(|b| b.approach.is_some()).count(),
        bound.len()
    );
    if std::env::var_os("FORJA_DUMP").is_some() {
        for (i, n) in net.nodes.iter().enumerate() {
            let ends: Vec<String> = n.ends.iter().map(|(s, e)| {
                let st = &net.streets[*s as usize];
                format!("s{s}/{e} {:?}->{:?} lanes {}", st.center.start(), st.center.end(), st.lanes.len())
            }).collect();
            println!("node {i} {:?} approaches {} connectors {} signal {:?}: {}", n.pos, n.approaches.len(), n.connectors.len(), n.signal, ends.join(" | "));
        }
    }
    println!("strongly connected lanes: {} of {}", net.strong.iter().filter(|x| **x).count(), net.lanes.len());
    let cfg = SimConfig { seed: 3, max_vehicles: 220, vehicle_density: 14.0, max_peds: 400, ped_density: 40.0, spawn_min: 0.0, spawn_max: 5000.0, despawn: 9000.0, near: 150.0, ..Default::default() };
    let mut sim = TrafficSim::new(net, cfg);
    let diag = std::env::var_os("FORJA_DIAG").is_some();
    let mut seen = 0;
    for _ in 0..(warm / sim.cfg.dt) as usize {
        sim.step();
        if diag {
            report(&sim, &mut seen);
        }
        sim.events.clear();
    }
    let (lo, hi) = sim.net.bounds;
    save(debug::render(&sim, lo - Vec2::splat(8.0), hi + Vec2::splat(8.0), 1.6), &out.join("city_map.png"));
    // The boulevard junction with the most approaches under lights.
    let sig = sim.net.signals.iter().max_by_key(|s| (sim.net.nodes[s.node as usize].approaches.len(), sim.net.nodes[s.node as usize].connectors.len())).expect("signals");
    let c = sim.net.nodes[sig.node as usize].pos;
    let cycle = sig.cycle();
    for k in 0..10 {
        save(debug::render(&sim, c - Vec2::splat(42.0), c + Vec2::splat(42.0), 9.0), &out.join(format!("city_junction_{k:02}.png")));
        for _ in 0..((cycle / 10.0) / sim.cfg.dt) as usize {
            sim.step();
            if diag {
                report(&sim, &mut seen);
            }
            sim.events.clear();
        }
    }
    let st = sim.stats;
    println!("{} vehicles, {} peds, {:.1} km, step mean {:.0} us (vehicles {:.0}, peds {:.0}), max {:.0} us", sim.vehicles.len(), sim.peds.len(), st.distance / 1000.0, st.mean_step_us(), st.vehicles_us / st.steps as f64, st.peds_us / st.steps as f64, st.max_step_us);
    println!("{st:?}");
}

/// `FORJA_DIAG`: every collision and rule violation, with the agents.
fn report(sim: &TrafficSim, seen: &mut usize) {
    use forja_ai::network::LaneKind;
    use forja_ai::sim::AgentRef;
    let desc = |id: u64| -> String {
        let Some(i) = sim.vehicle_index(id) else { return format!("v{id} gone") };
        let v = &sim.vehicles[i];
        let l = &sim.net.lanes[v.lane as usize];
        let k = match l.kind {
            LaneKind::Connector { node, turn, from, .. } => format!("conn n{node} {turn:?} from {from}"),
            LaneKind::Street { street, .. } => format!("street {street} stop_s {:.1}", l.stop_s),
        };
        let who = match v.blocked_by {
            Some(AgentRef::Ped(pid)) => sim.peds.iter().find(|p| p.id == pid).map(|p| {
                let mut nodes = vec![p.from];
                nodes.extend(p.path.iter().take(2));
                let pts: Vec<_> = nodes.iter().map(|&n| sim.net.ped.nodes[n as usize].pos).collect();
                let edges: Vec<_> = nodes.windows(2).filter_map(|w| sim.net.ped.edges.iter().find(|e| (e.a == w[0] && e.b == w[1]) || (e.a == w[1] && e.b == w[0])).map(|e| (e.kind.clone(), e.half))).collect();
                format!(" [ped {pid} {:?} pos {:?} vel {:?} from {} nodes {pts:?} edges {edges:?}]", p.state, p.pos, p.vel, p.from)
            }).unwrap_or_default(),
            _ => String::new(),
        };
        format!("{who} v{id} lane {} ({k}) s {:.1}/{:.1} v {:.1} pos {:?} gate {} passed {} why {:?} blocked {:?} wait {:.0} route {:?} cw {:?}", v.lane, v.s, l.len(), v.v, v.pos, v.gate_open, v.passed_line, v.why, v.blocked_by, v.wait_t, v.route.iter().take(3).collect::<Vec<_>>(), l.crosswalks)
    };
    for e in &sim.events {
        if let forja_ai::SimEvent::Collision { a, b, pos } = e {
            println!("t {:.2} COLLISION {a:?} {b:?} at {pos:?}", sim.t);
            for r in [a, b] {
                match r {
                    AgentRef::Vehicle(id) => println!("   {}", desc(*id)),
                    AgentRef::Ped(id) => {
                        if let Some(p) = sim.peds.iter().find(|p| p.id == *id) {
                            println!("   ped {id} {:?} pos {:?} vel {:?} jay {}", p.state, p.pos, p.vel, p.jaywalker);
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    while *seen < sim.violations.len() {
        let (t, k, id, lane) = sim.violations[*seen];
        let ap = sim.net.lanes[lane as usize].approach.map(|a| (a, sim.net.approach_aspect(a, t - sim.cfg.dt as f64)));
        println!("t {t:.2} {k} lane {lane} approach/aspect-before {ap:?}: {}", desc(id));
        *seen += 1;
    }
}
