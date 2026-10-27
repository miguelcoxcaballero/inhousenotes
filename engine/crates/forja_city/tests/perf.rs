//! Performance budgets (release only, ignored by default):
//! `cargo test -p forja_city --release --test perf -- --ignored --nocapture`

use std::time::Instant;

use forja_city::fabric::Fabric;
use forja_city::generators::{self, CityPlan};
use forja_city::kit::Kit;
use forja_city::layout::realize;
use forja_city::profile::ProfileLibrary;
use forja_city::style::StyleLibrary;

fn time(name: &str) -> (f32, f32, f32) {
    let lib = ProfileLibrary::builtin();
    let plan = CityPlan::builtin(name).unwrap();
    let t = Instant::now();
    let d = generators::draft(&plan, &lib, 7);
    let td = t.elapsed().as_secs_f32();
    let t = Instant::now();
    let f = Fabric::build(&d, &plan, &lib, 7);
    let tf = t.elapsed().as_secs_f32();
    let (kit, styles) = (Kit::load(None), StyleLibrary::builtin());
    let t = Instant::now();
    let city = realize(&kit, &styles, f, &plan, &lib, 7);
    let tr = t.elapsed().as_secs_f32();
    println!("{name}: draft {td:.2}s, fabric {tf:.2}s, realize {tr:.2}s; {:?}", city.stats);
    (td, tf, tr)
}

#[test]
#[ignore]
fn budgets() {
    let names = std::env::var("FORJA_PERF_PRESETS").unwrap_or("classic,carmen,austrias,organic,default".into());
    for name in names.split(',') {
        time(name);
    }
}
