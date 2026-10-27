//! Headless city simulation: minutes of traffic and pedestrians on a grid
//! with lights, STOP and give-way junctions, an avenue with permissive
//! left turns and zebras everywhere. Zero collisions and zero violations.

use forja_ai::debug;
use forja_ai::spec::{GridOptions, grid_city};
use forja_ai::{RoadNetwork, SimConfig, TrafficSim};
use forja_math::Vec2;

fn city_sim(seed: u64) -> TrafficSim {
    let net = RoadNetwork::build(&grid_city(&GridOptions::default()));
    let cfg = SimConfig { seed, max_vehicles: 140, vehicle_density: 18.0, max_peds: 220, ped_density: 45.0, spawn_min: 0.0, spawn_max: 1000.0, despawn: 2000.0, near: 120.0, ..Default::default() };
    let mut sim = TrafficSim::new(net, cfg);
    sim.focus = Vec2::ZERO;
    sim
}

#[test]
fn minutes_of_city_traffic_without_collisions_or_violations() {
    let minutes: f32 = std::env::var("FORJA_AI_MINUTES").ok().and_then(|m| m.parse().ok()).unwrap_or(5.0);
    let mut sim = city_sim(7);
    let steps = (minutes * 60.0 / sim.cfg.dt) as usize;
    let mut peak = (0usize, 0usize);
    for _ in 0..steps {
        sim.step();
        peak = (peak.0.max(sim.vehicles.len()), peak.1.max(sim.peds.len()));
    }
    let st = sim.stats;
    println!(
        "{minutes} min: {} vehicles / {} peds (peak {} / {}), {:.1} km driven, {} crossings, {} jaywalks, {} stuck removed, step mean {:.0} us max {:.0} us",
        sim.vehicles.len(),
        sim.peds.len(),
        peak.0,
        peak.1,
        st.distance / 1000.0,
        st.crossings,
        st.jaywalks,
        st.stuck_removed,
        st.mean_step_us(),
        st.max_step_us
    );
    println!("{st:?}");
    if let Ok(dir) = std::env::var("CARGO_TARGET_TMPDIR") {
        let (lo, hi) = sim.net.bounds;
        let c = debug::render(&sim, lo - Vec2::splat(10.0), hi + Vec2::splat(10.0), 3.0);
        let path = std::path::Path::new(&dir).join("traffic_headless.png");
        image::RgbImage::from_raw(c.width, c.height, c.rgb).expect("image").save(&path).expect("png");
        println!("debug view: {}", path.display());
    }
    assert!(peak.0 >= 80 && peak.1 >= 150, "{peak:?}");
    assert_eq!(st.vehicle_collisions, 0, "vehicle collisions");
    assert_eq!(st.ped_collisions, 0, "pedestrians hit");
    assert_eq!(st.red_light_violations, 0, "red lights run");
    assert_eq!(st.stop_sign_violations, 0, "STOP signs run");
    assert_eq!(st.box_violations, 0, "junction box entered while occupied");
    // Traffic flows (no gridlock): a mean speed above 0.8 m/s including
    // every wait at lights, signs and zebras.
    assert!(st.distance > (minutes * 60.0) as f64 * peak.0 as f64 * 0.8, "{} m", st.distance);
    assert!(st.crossings as f32 > minutes * 20.0, "{} crossings", st.crossings);
}

#[test]
fn same_seed_same_traffic() {
    let mut a = city_sim(11);
    let mut b = city_sim(11);
    for _ in 0..600 {
        a.step();
        b.step();
    }
    assert_eq!(a.digest(), b.digest());
    assert!(!a.vehicles.is_empty());
}
