//! Seeded property harness over random street layouts: segment soups with
//! crossings at any angle, junctions of high valence, dead ends, arcs and
//! wiggles of every profile, star-shaped plazas. Every case must become a
//! valid fabric; a failing case is written as a Draft TOML fixture.
//!
//! `FORJA_PROP_CASES=2000 cargo test -p forja_city --release --test random_drafts -- --ignored`

use forja_city::fabric::Fabric;
use forja_city::generators::CityPlan;
use forja_city::geom::Rng;
use forja_city::plan::{Draft, Mask, Plaza, PlazaKind, Way};
use forja_city::profile::ProfileLibrary;
use forja_math::Vec2;

const PROFILES: [&str; 10] = ["callejon", "callejuela", "callejuela_aceras", "calle_vieja", "calle_peatonal", "madrid_calle", "ensanche", "calle_ancha", "avenida", "ronda"];

/// A random draft in a 220 m square.
fn random_draft(seed: u64) -> Draft {
    let mut rng = Rng::new(seed.wrapping_mul(7919) + 17);
    let half = 110.0;
    let mut d = Draft { snap: 0.05, ..Default::default() };
    let ring = vec![Vec2::new(-half, -half), Vec2::new(half, -half), Vec2::new(half, half), Vec2::new(-half, half)];
    d.masks.push(Mask { zone: 1, ring: ring.clone() });
    // A boundary street around (most cases), so blocks close.
    if rng.chance(0.8) {
        let mut l = ring.clone();
        l.push(ring[0]);
        d.ways.push(Way::new(l, "madrid_calle").zone(1));
    }
    let n = 3 + rng.below(6);
    let narrow = rng.chance(0.5);
    let pick = |rng: &mut Rng| if narrow { PROFILES[rng.below(5)] } else { PROFILES[rng.below(PROFILES.len())] };
    for _ in 0..n {
        let kind = rng.below(4);
        let a = Vec2::new(rng.range(-half, half), rng.range(-half, half));
        let ang = rng.range(0.0, std::f32::consts::TAU);
        let len = rng.range(60.0, 260.0);
        let dir = Vec2::from_angle(ang);
        let line: Vec<Vec2> = match kind {
            // Straight through.
            0 => vec![a - dir * len * 0.5, a + dir * len * 0.5],
            // Arc of radius 15–200 m.
            1 => {
                let r = rng.range(15.0, 200.0);
                let sweep = (len / r).min(3.5);
                let c = a - Vec2::new(-dir.y, dir.x) * r;
                (0..=24).map(|i| c + Vec2::from_angle(ang + std::f32::consts::FRAC_PI_2 + sweep * (i as f32 / 24.0 - 0.5)) * r).collect()
            }
            // Sine wiggle.
            2 => {
                let amp = rng.range(2.0, 15.0);
                let wl = rng.range(30.0, 90.0);
                (0..=30).map(|i| {
                    let t = (i as f32 / 30.0 - 0.5) * len;
                    a + dir * t + Vec2::new(-dir.y, dir.x) * amp * (t / wl * std::f32::consts::TAU).sin()
                }).collect()
            }
            // A star of dead-end spokes from a hub (valence up to 7).
            _ => {
                let k = 3 + rng.below(5);
                for s in 0..k {
                    let d2 = Vec2::from_angle(ang + s as f32 * std::f32::consts::TAU / k as f32 + rng.range(-0.2, 0.2));
                    d.ways.push(Way::new(vec![a, a + d2 * rng.range(18.0, 70.0)], pick(&mut rng)).zone(1));
                }
                continue;
            }
        };
        let mut w = Way::new(line, pick(&mut rng)).zone(1);
        if rng.chance(0.3) {
            w.oneway = 1;
        }
        d.ways.push(w);
    }
    if rng.chance(0.4) {
        let c = Vec2::new(rng.range(-60.0, 60.0), rng.range(-60.0, 60.0));
        let r = rng.range(8.0, 20.0);
        let k = 5 + rng.below(4);
        d.plazas.push(Plaza { ring: (0..k).map(|i| c + Vec2::from_angle(i as f32 / k as f32 * std::f32::consts::TAU) * r * if i % 2 == 0 { 1.0 } else { 0.75 }).collect(), zone: 1, kind: PlazaKind::Paved });
    }
    d
}

fn run(cases: u64) {
    let lib = ProfileLibrary::builtin();
    let mut failures = Vec::new();
    for seed in 0..cases {
        let d = random_draft(seed);
        let plan = CityPlan::from_draft(d.clone(), if seed % 2 == 0 { "valencia" } else { "madrid" });
        let f = Fabric::build(&d, &plan, &lib, seed);
        if let Err(v) = f.validate() {
            let path = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("random_draft_{seed}.toml"));
            std::fs::write(&path, d.to_toml()).ok();
            println!("seed {seed}: {} violations, first {:?}; draft at {}", v.len(), v[0], path.display());
            failures.push(seed);
        }
    }
    assert!(failures.is_empty(), "failing seeds {failures:?}");
}

#[test]
fn random_drafts_become_valid_fabrics() {
    run(std::env::var("FORJA_PROP_CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(24));
}

#[test]
#[ignore]
fn random_drafts_stress() {
    run(std::env::var("FORJA_PROP_CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(2000));
}

#[test]
fn shuffled_and_reversed_ways_give_the_same_fabric() {
    let lib = ProfileLibrary::builtin();
    for seed in [3u64, 11, 20] {
        let d = random_draft(seed);
        let plan = CityPlan::from_draft(d.clone(), "valencia");
        let a = Fabric::build(&d, &plan, &lib, 5);
        let mut e = d.clone();
        e.ways.reverse();
        for w in e.ways.iter_mut().step_by(2) {
            w.line.reverse();
            w.oneway = -w.oneway;
        }
        let b = Fabric::build(&e, &plan, &lib, 5);
        assert_eq!(a.graph.hash64(), b.graph.hash64(), "seed {seed}: graphs differ");
        assert_eq!(a.hash64(), b.hash64(), "seed {seed}: fabrics differ");
        // Deterministic, and the seed matters.
        assert_eq!(Fabric::build(&d, &plan, &lib, 5).hash64(), a.hash64());
    }
}

#[test]
fn committed_fixtures_replay() {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/drafts");
    let lib = ProfileLibrary::builtin();
    let Ok(entries) = std::fs::read_dir(&dir) else { return };
    let mut paths: Vec<_> = entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "toml")).collect();
    paths.sort();
    for p in paths {
        let d = Draft::from_toml(&std::fs::read_to_string(&p).unwrap()).unwrap();
        let plan = CityPlan::from_draft(d.clone(), "valencia");
        Fabric::build(&d, &plan, &lib, 1).validate().unwrap_or_else(|v| panic!("{}: {:?}", p.display(), &v[..v.len().min(5)]));
    }
}
