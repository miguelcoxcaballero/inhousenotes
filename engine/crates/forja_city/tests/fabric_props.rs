//! Property tests of the city plan: any draft becomes a valid fabric.

use forja_city::fabric::Fabric;
use forja_city::generators::{self, CityPlan};
use forja_city::plan::{Join, Region};
use forja_city::profile::ProfileLibrary;

fn svg_dir() -> std::path::PathBuf {
    let d = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("forja_city_svg");
    std::fs::create_dir_all(&d).ok();
    d
}

fn preset(name: &str, min_lots: usize) -> Fabric {
    let lib = ProfileLibrary::builtin();
    let plan = CityPlan::builtin(name).unwrap();
    let t = std::time::Instant::now();
    let (d, carved) = generators::draft_report(&plan, &lib, 7);
    let td = t.elapsed().as_secs_f32();
    std::fs::write(svg_dir().join(format!("{name}_draft.toml")), d.to_toml()).unwrap();
    let f = Fabric::build(&d, &plan, &lib, 7);
    println!("{name}: {} ways -> {:?}; draft {td:.2}s, total {:.2}s", d.ways.len(), f.stats, t.elapsed().as_secs_f32());
    std::fs::write(svg_dir().join(format!("{name}.svg")), f.to_svg(1.5, None)).unwrap();
    if let Err(v) = f.validate() {
        for x in v.iter().take(30) {
            println!("{x:?}");
        }
        panic!("{name}: {} violations", v.len());
    }
    assert!(f.lots.len() >= min_lots, "{name}: {} lots", f.lots.len());
    // Accessibility: carving ended by its rule, not its safety cap, and no
    // land of an organic zone is deeper than the last tier's d_max.
    for c in &carved {
        assert!(!c.capped, "{name}: zone {} hit the carve cap after {} iterations", c.zone, c.iterations);
        let ring = |z: u16| d.masks.iter().filter(move |m| m.zone == z).map(|m| m.ring.as_slice());
        let earlier = Region::rings(d.masks.iter().filter(|m| m.zone < c.zone).map(|m| m.ring.as_slice()));
        let land = Region::rings(ring(c.zone)).minus(&earlier).minus(&f.corridors).minus(&f.plazas);
        let deep = land.offset(-c.d_max, Join::Round).area();
        println!("{name}: zone {} carved in {} iterations, land deeper than {} m: {deep:.2} m²", c.zone, c.iterations, c.d_max);
        assert!(deep < 1.0, "{name}: zone {}: {deep:.1} m² of land deeper than {} m from any street", c.zone, c.d_max);
    }
    f
}

#[test]
fn classic_plan_builds_a_valid_fabric() {
    let f = preset("classic", 250);
    // Parity with the legacy map: 27 built blocks, two squares, chaflanes.
    assert_eq!(f.stats.iter().find(|s| s.0 == "blocks").unwrap().1, 27);
    assert_eq!(f.plazas.components().len(), 2);
    let legacy_lots = 309.0;
    assert!((f.lots.len() as f32 - legacy_lots).abs() / legacy_lots < 0.15, "{} lots vs {legacy_lots}", f.lots.len());
}

#[test]
fn carmen_plan_builds_a_valid_fabric() {
    let f = preset("carmen", 400);
    // Shared paving keeps the height of a shared street everywhere, also
    // where it reaches into a car street's corridor at an alley's mouth (no
    // trench of the car street's shared height between alley and sidewalk).
    let lib = ProfileLibrary::builtin();
    let heights: Vec<f32> = f.graph.streets.iter().filter(|s| s.carriageway <= 0.0).map(|s| lib.get(s.profile)).map(|p| if p.vehicles { 0.02 } else { p.shared_height.max(0.02) }).collect();
    let mut bad = 0;
    for c in f.ground.shared.components() {
        for sh in c.shapes() {
            for &p in &sh.outer {
                let q = p + (c.pole().unwrap().0 - p).normalize_or_zero() * 0.05;
                if let Some(s) = f.ground.surface_at(q).filter(|_| f.ground.shared.contains(q) && f.ground.shared.boundary_distance(q) > 0.02) {
                    if !heights.iter().any(|h| (h - s.h).abs() < 1e-3) {
                        if bad < 5 {
                            println!("shared at {q} has h {} ({:?})", s.h, s.mat);
                        }
                        bad += 1;
                    }
                }
            }
        }
    }
    assert_eq!(bad, 0, "{bad} shared points at a height no shared street has");
}

#[test]
fn austrias_plan_builds_a_valid_fabric() {
    preset("austrias", 300);
}

#[test]
fn organic_plan_builds_a_valid_fabric() {
    preset("organic", 150);
}

#[test]
fn default_plan_builds_a_valid_fabric() {
    let f = preset("default", 1000);
    // The playable map: old towns on sloping ground, with stair streets
    // and ramps, crossed by large avenues that stay gentle.
    let (lib, r, g) = (ProfileLibrary::builtin(), &f.ground.relief, &f.graph);
    let (mut stairs, mut ramps, mut avenue_m) = (0, 0, 0.0);
    for (i, s) in g.streets.iter().enumerate() {
        let (p, i) = (lib.get(s.profile), i as u32);
        let max = if r.steps(i).is_some() { forja_city::plan::STAIR_GRADE } else { p.max_grade() };
        assert!(r.grade(i) <= max + 1e-3, "{} at {:.3} > {max}", p.id, r.grade(i));
        stairs += (r.steps(i).is_some_and(|st| st.n >= 6)) as usize;
        ramps += (r.steps(i).is_none() && r.grade(i) > 0.06 && s.len() > 20.0) as usize;
        if matches!(p.id.as_str(), "bulevar" | "avenida" | "ronda" | "calle_ancha") {
            avenue_m += s.len();
        }
    }
    let z: Vec<f32> = r.nodes.clone();
    let range = z.iter().cloned().fold(f32::MIN, f32::max) - z.iter().cloned().fold(f32::MAX, f32::min);
    println!("default: {stairs} stair streets, {ramps} steep ramps, {avenue_m:.0} m of avenue, relief {range:.1} m");
    assert!(stairs >= 2, "{stairs} stair streets");
    assert!(ramps >= 10, "{ramps} sloping streets");
    assert!(avenue_m > 1500.0, "{avenue_m} m of avenue");
    assert!(range > 12.0, "relief of {range} m");
}
