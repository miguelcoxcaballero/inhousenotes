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
    for _ in 0..(warm / sim.cfg.dt) as usize {
        sim.step();
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
        }
    }
    let st = sim.stats;
    println!("{} vehicles, {} peds, {:.1} km, step mean {:.0} us (vehicles {:.0}, peds {:.0}), max {:.0} us", sim.vehicles.len(), sim.peds.len(), st.distance / 1000.0, st.mean_step_us(), st.vehicles_us / st.steps as f64, st.peds_us / st.steps as f64, st.max_step_us);
    println!("{st:?}");
}
