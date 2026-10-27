//! Writes top-down debug frames of a simulated grid city:
//!
//! ```text
//! cargo run --release -p forja_ai --example traffic_review -- OUT_DIR [SECONDS]
//! ```
//! `city.png` (whole map), and a sequence `junction_NN.png` zoomed on a
//! signalled junction through one signal cycle.

use forja_ai::debug;
use forja_ai::spec::{GridOptions, grid_city};
use forja_ai::{RoadNetwork, SimConfig, TrafficSim};
use forja_math::Vec2;

fn save(c: debug::Canvas, path: &std::path::Path) {
    image::RgbImage::from_raw(c.width, c.height, c.rgb).expect("image").save(path).expect("png");
    println!("{}", path.display());
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let out = std::path::PathBuf::from(args.first().cloned().unwrap_or_else(|| "traffic_review".into()));
    let warm: f32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(120.0);
    std::fs::create_dir_all(&out).expect("out dir");
    let net = RoadNetwork::build(&grid_city(&GridOptions::default()));
    let cfg = SimConfig { seed: 7, max_vehicles: 140, vehicle_density: 18.0, max_peds: 220, ped_density: 45.0, spawn_min: 0.0, spawn_max: 1000.0, despawn: 2000.0, near: 120.0, ..Default::default() };
    let mut sim = TrafficSim::new(net, cfg);
    for _ in 0..(warm / sim.cfg.dt) as usize {
        sim.step();
    }
    let (lo, hi) = sim.net.bounds;
    save(debug::render(&sim, lo - Vec2::splat(10.0), hi + Vec2::splat(10.0), 3.0), &out.join("city.png"));
    // A signalled four-way junction.
    let sig = sim.net.signals.iter().find(|s| sim.net.nodes[s.node as usize].approaches.len() >= 4).expect("4-way signal");
    let c = sim.net.nodes[sig.node as usize].pos;
    let cycle = sig.cycle();
    let frames = 12;
    for k in 0..frames {
        save(debug::render(&sim, c - Vec2::splat(45.0), c + Vec2::splat(45.0), 9.0), &out.join(format!("junction_{k:02}.png")));
        for _ in 0..((cycle / frames as f32) / sim.cfg.dt) as usize {
            sim.step();
        }
    }
    println!("{:?}", sim.stats);
}
