//! Property tests of the city plan: any draft becomes a valid fabric.

use forja_city::fabric::Fabric;
use forja_city::generators::{self, CityPlan};
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
    let d = generators::draft(&plan, &lib, 7);
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
    preset("carmen", 400);
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
    preset("default", 1000);
}
