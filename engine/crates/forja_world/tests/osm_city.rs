//! Real OpenStreetMap tiles through the city fabric (when downloaded).
//! `cargo test -p forja_world --test osm_city -- --ignored --nocapture`

use forja_city::fabric::Fabric;
use forja_city::generators::CityPlan;
use forja_city::profile::ProfileLibrary;
use forja_world::osm::parse_any;
use forja_world::osm_city::{OsmOptions, city_of, draft};

#[test]
#[ignore]
fn osm_real_tiles() {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/downloaded/osm");
    let lib = ProfileLibrary::builtin();
    let mut any = false;
    for tile in ["carmen", "austrias", "ruzafa"] {
        let Ok(text) = std::fs::read_to_string(dir.join(format!("{tile}.json"))) else { continue };
        any = true;
        let t = std::time::Instant::now();
        let data = parse_any(&text).unwrap();
        let d = draft(&data, &lib, &OsmOptions::default()).unwrap();
        let plan = CityPlan::from_draft(d.clone(), city_of(&data));
        let f = Fabric::build(&d, &plan, &lib, 7);
        let mut widths: Vec<f32> = f.graph.streets.iter().map(|s| s.min_width()).collect();
        widths.sort_by(|a, b| a.total_cmp(b));
        let levels = f.lots.iter().filter(|l| l.levels.is_some()).count();
        println!(
            "{tile}: {} ways, {} footprints -> {:?}; width median {:.1} p10 {:.1}; levels on {levels}/{} lots; {:.2}s",
            d.ways.len(),
            d.footprints.len(),
            f.stats,
            widths.get(widths.len() / 2).unwrap_or(&0.0),
            widths.get(widths.len() / 10).unwrap_or(&0.0),
            f.lots.len(),
            t.elapsed().as_secs_f32()
        );
        let out = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("forja_city_svg");
        std::fs::create_dir_all(&out).ok();
        std::fs::write(out.join(format!("osm_{tile}.svg")), f.to_svg(2.0, None)).unwrap();
        if let Err(v) = f.validate() {
            for x in v.iter().take(10) {
                println!("  {x:?}");
            }
            panic!("{tile}: {} violations", v.len());
        }
        assert!(f.lots.len() > 100, "{tile}: {} lots", f.lots.len());
    }
    if !any {
        println!("no tiles in {}", dir.display());
    }
}
