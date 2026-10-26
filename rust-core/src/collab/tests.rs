//! Golden values below were produced by the original JavaScript (collaboration-core-v5.js at a2dea10).
use super::call;

fn run(method: &str, input: &str) -> String { String::from_utf8(call(method, input.as_bytes()).unwrap().unwrap()).unwrap() }
fn run_bytes(method: &str, input: &[u8]) -> Result<Vec<u8>, String> { call(method, input).unwrap() }

const META: &str = r#"{"clock":7,"entries":{"2":{},"10":{"rank":null},"b":{"rank":2048,"orderStamp":{"clock":5,"actor":"a"},"createdStamp":{"clock":2,"actor":"b"}},"a":{"rank":"0","orderStamp":{"clock":3,"actor":"z"}}},"tombstones":{"b":{"clock":9,"actor":"q"},"gone":{"clock":1,"actor":"x"}}}"#;
const FIELDS: &str = r#"{"clock":3,"fields":{"3":{"value":"x","stamp":{"clock":1}},"title":{"value":{"b":[1,2.5],"a":null},"stamp":{"clock":4,"actor":"w"}},"bad":{"stamp":{}}}}"#;
const PAGES: &str = r##"[{"pageId":"p1","strokes":[{"id":"s2","preview":"x","points":[{"x":1.5,"y":-0.25}]},{"id":"s1","needsRedraw":true,"color":"#000","n":[1e+21,1e-7]},"str",[5,6]],"images":[{"id":"i1","src":"data:"}],"deletedStrokeIds":["s9","s9","","z"],"deletedStrokeStamps":{"s9":{"clock":4,"actor":"a"},"s7":{"stamp":{"clock":2,"actor":"b"}}},"backgroundSource":"","pageWidth":"210","pageHeight":null,"sidePanel":{"title":"T","dateKeys":["a",5,null]}},null]"##;

#[test] fn hash_string_golden_values() {
    assert_eq!(run("collab.hash_utf8", "hello"), "4f9f2cab7db12245");
    assert_eq!(run("collab.hash_utf8", ""), "811c9dc59e3779b9");
    assert_eq!(run("collab.hash_utf8", "x\u{1F600}"), "71c60a4af0f4f1d7");
    // 'a', lone high surrogate, 'b' can only be passed as raw UTF-16 units.
    let units: Vec<u8> = [0x61u16, 0xd83d, 0x62].iter().flat_map(|u| u.to_le_bytes()).collect();
    assert_eq!(String::from_utf8(run_bytes("collab.hash_utf16", &units).unwrap()).unwrap(), "fa52c473d107fe92");
    assert!(run_bytes("collab.hash_utf16", &[1, 2, 3]).is_err());
    assert!(run_bytes("collab.hash_utf8", &[0xff]).is_err());
}

#[test] fn utf16_and_utf8_paths_agree_for_well_formed_text() {
    let text = "Notes \u{1F600} é中";
    let units: Vec<u8> = text.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    assert_eq!(run("collab.hash_utf8", text).into_bytes(), run_bytes("collab.hash_utf16", &units).unwrap());
}

#[test] fn structure_hash_and_normalizer_golden_values() {
    assert_eq!(run("collab.structure_hash", &format!(r#"{{"meta":{META}}}"#)), "b1dbdb5881054657");
    assert_eq!(run("collab.structure_hash", &format!(r#"{{"meta":{META},"ids":["a","b","c","a"]}}"#)), "120ce9a454c0ff35");
    // Text is in insertion (code unit) order; the JS caller's JSON.parse lists "2" before "10" like the original object.
    assert_eq!(run("collab.normalize_structure", &format!(r#"{{"meta":{META},"ids":["c","a"]}}"#)),
        r#"{"v":1,"clock":9,"entries":{"10":{"rank":0,"orderStamp":{"clock":0,"actor":""},"createdStamp":{"clock":0,"actor":""}},"2":{"rank":1024,"orderStamp":{"clock":0,"actor":""},"createdStamp":{"clock":0,"actor":""}},"a":{"rank":0,"orderStamp":{"clock":3,"actor":"z"},"createdStamp":{"clock":0,"actor":""}},"b":{"rank":2048,"orderStamp":{"clock":5,"actor":"a"},"createdStamp":{"clock":2,"actor":"b"}},"c":{"rank":0,"orderStamp":{"clock":0,"actor":""},"createdStamp":{"clock":0,"actor":""}}},"tombstones":{"b":{"clock":9,"actor":"q"},"gone":{"clock":1,"actor":"x"}}}"#);
    assert_eq!(run("collab.normalize_structure", r#"{"meta":null,"ids":["p1","p2"]}"#),
        r#"{"v":1,"clock":0,"entries":{"p1":{"rank":0,"orderStamp":{"clock":0,"actor":""},"createdStamp":{"clock":0,"actor":""}},"p2":{"rank":1024,"orderStamp":{"clock":0,"actor":""},"createdStamp":{"clock":0,"actor":""}}},"tombstones":{}}"#);
}

#[test] fn field_meta_golden_values() {
    assert_eq!(run("collab.field_hash", FIELDS), "5ebe9b5436ddfa56");
    assert_eq!(run("collab.normalize_fields", FIELDS),
        r#"{"v":1,"clock":4,"fields":{"3":{"value":"x","stamp":{"clock":1,"actor":""}},"title":{"value":{"a":null,"b":[1,2.5]},"stamp":{"clock":4,"actor":"w"}}}}"#);
}

#[test] fn document_hash_golden_value() {
    let input = format!(r#"{{"pages":{PAGES},"structure":{META},"calendar":{{"rows":3}},"name":"Name 😀","fields":{FIELDS}}}"#);
    assert_eq!(run("collab.document_hash", &input), "13d43dac9c78d9fd");
}

#[test] fn document_hash_ignores_volatile_state_and_item_order() {
    let a = r#"{"pages":[{"pageId":"p","strokes":[{"id":"b","x":1,"preview":"zzz"},{"id":"a","unloaded":true}]}],"structure":null,"calendar":null,"name":"","fields":null}"#;
    let b = r#"{"pages":[{"pageId":"p","strokes":[{"x":1,"id":"b"},{"id":"a","needsRedraw":1}]}],"structure":null,"calendar":null,"name":"","fields":null}"#;
    let c = r#"{"pages":[{"pageId":"p","strokes":[{"id":"b","x":2},{"id":"a"}]}],"structure":null,"calendar":null,"name":"","fields":null}"#;
    assert_eq!(run("collab.document_hash", a), run("collab.document_hash", b));
    assert_ne!(run("collab.document_hash", a), run("collab.document_hash", c));
}

#[test] fn items_without_id_sort_by_their_stable_text_and_equal_keys_keep_input_order() {
    let a = r#"{"pages":[{"strokes":[{"id":"k","v":1},{"id":"k","v":2},null,5,"s"]}],"structure":null,"calendar":null,"name":"","fields":null}"#;
    let b = r#"{"pages":[{"strokes":["s",{"id":"k","v":1},5,null,{"id":"k","v":2}]}],"structure":null,"calendar":null,"name":"","fields":null}"#;
    let swapped = r#"{"pages":[{"strokes":[{"id":"k","v":2},{"id":"k","v":1},null,5,"s"]}],"structure":null,"calendar":null,"name":"","fields":null}"#;
    assert_eq!(run("collab.document_hash", a), run("collab.document_hash", b));
    assert_ne!(run("collab.document_hash", a), run("collab.document_hash", swapped)); // stable sort keeps duplicate ids in input order
}

#[test] fn stable_method_and_errors() {
    assert_eq!(run("collab.stable", r#"{"b":[1,{"z":null,"a":1e21}],"a":"é"}"#), r#"{"a":"é","b":[1,{"a":1e+21,"z":null}]}"#);
    assert!(run_bytes("collab.stable", b"{").is_err());
    assert!(run_bytes("collab.document_hash", b"[]").map(|v| v.len() == 16).unwrap_or(false));
    assert!(call("collab.nope", b"").is_none());
}

#[test] #[ignore] fn bench_document_hash() {
    let mut seed = 12345u64;
    let mut rnd = move || { seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407); (seed >> 33) as f64 / (1u64 << 31) as f64 };
    let mut pages = String::from("[");
    for p in 0..3 {
        if p > 0 { pages.push(','); }
        pages.push_str(&format!(r#"{{"pageId":"p{p}","strokes":["#));
        for s in 0..100 {
            if s > 0 { pages.push(','); }
            pages.push_str(&format!(r##"{{"id":"p{p}-s{s}","color":"#1a1a1a","width":2.25,"points":["##));
            for q in 0..60 {
                if q > 0 { pages.push(','); }
                pages.push_str(&format!(r#"{{"x":{},"y":{},"p":{}}}"#, (rnd() * 59500.0).round() / 100.0, (rnd() * 84200.0).round() / 100.0, (rnd() * 1000.0).round() / 1000.0));
            }
            pages.push_str("]}");
        }
        pages.push_str(r#"],"images":[],"pageWidth":595.28,"pageHeight":841.89}"#);
    }
    pages.push(']');
    let input = format!(r#"{{"pages":{pages},"structure":null,"calendar":null,"name":"","fields":null}}"#);
    let started = std::time::Instant::now();
    for _ in 0..20 { run("collab.document_hash", &input); }
    println!("{} KB: {:?} per hash", input.len() / 1024, started.elapsed() / 20);
    let started = std::time::Instant::now();
    for _ in 0..20 { let mut pool = super::json::Pool::default(); let v = super::json::parse(&mut pool, input.as_bytes()).unwrap(); std::hint::black_box(&v); }
    println!("parse only: {:?}", started.elapsed() / 20);
    let mut pool = super::json::Pool::default(); let v = super::json::parse(&mut pool, input.as_bytes()).unwrap();
    let started = std::time::Instant::now();
    for _ in 0..20 { let mut h = super::canon::Hasher::new(); super::json::write_stable(&mut h, &pool, &v); std::hint::black_box(h.finish()); }
    println!("write+hash only: {:?}", started.elapsed() / 20);
}

// ---- live snapshot cache and delta planner (collab/snapshot.rs); the cache is thread-local, so each test has its own.

fn snap(hash: &str, pages: &[(&str, usize, char)]) -> String {
    let list: Vec<String> = pages.iter().map(|(id, n, c)| format!(r#"{{"pageId":"{id}","text":"{}"}}"#, c.to_string().repeat(*n))).collect();
    format!(r#"{{"contentHash":"{hash}","pages":[{}]}}"#, list.join(","))
}
fn remember(limit: usize, max: usize, body: &str) -> String { run("collab.snap_remember", &format!("{limit},{max}\n{body}")) }
fn plan(hash: &str, ids: &[&str], bases: &[&str], force: bool) -> String {
    let q = |l: &[&str]| l.iter().map(|x| format!("\"{x}\"")).collect::<Vec<_>>().join(",");
    run("collab.snap_delta", &format!(r#"{{"hash":"{hash}","ids":[{}],"bases":[{}],"force":{force}}}"#, q(ids), q(bases)))
}

#[test] fn snapshot_delta_lists_only_changed_pages() {
    let ids = ["p0", "p1", "p2", "p3", "p4"];
    let a: Vec<(&str, usize, char)> = ids.iter().map(|id| (*id, 1500, 'a')).collect();
    let mut b = a.clone(); b[2].2 = 'b';
    assert_eq!(remember(3, 16_000_000, &snap("a", &a)), "1");
    assert_eq!(plan("b", &ids, &["a"], false), r#"["missing"]"#);
    assert_eq!(remember(3, 16_000_000, &snap("b", &b)), "1");
    assert_eq!(plan("b", &ids, &["a"], false), r#"["delta","a",[2]]"#);
    assert_eq!(plan("b", &ids, &["a"], true), r#"["full"]"#);
    assert_eq!(plan("b", &ids, &["zz", "a"], false), r#"["delta","a",[2]]"#);
    assert_eq!(plan("b", &ids, &["zz"], false), r#"["full"]"#);
    assert_eq!(plan("b", &ids, &[], false), r#"["full"]"#);
    assert_eq!(plan("b", &["p0", "p1"], &["a"], false), r#"["full"]"#); // page list differs from the cached one
    run("collab.snap_reset", "");
    assert_eq!(plan("b", &ids, &["a"], false), r#"["missing"]"#);
}

#[test] fn snapshot_delta_threshold_and_all_changed_fall_back_to_full() {
    let ids = ["p0", "p1"];
    let a = [("p0", 100usize, 'a'), ("p1", 100, 'a')];
    remember(3, 16_000_000, &snap("a", &a));
    remember(3, 16_000_000, &snap("b", &[("p0", 100, 'b'), ("p1", 100, 'a')]));
    assert_eq!(plan("b", &ids, &["a"], false), r#"["full"]"#); // changed + 2048 >= 80% of the total
    remember(3, 16_000_000, &snap("c", &[("p0", 5000, 'c'), ("p1", 5000, 'c')]));
    assert_eq!(plan("c", &ids, &["a"], false), r#"["full"]"#); // every page changed
}

#[test] fn snapshot_cache_evicts_oldest_by_count_then_by_characters() {
    let page = [("p0", 1000usize, 'x')];
    for h in ["h1", "h2", "h3"] { remember(2, 16_000_000, &snap(h, &page)); }
    assert_eq!(plan("h1", &["p0"], &[], false), r#"["missing"]"#); // limit 2 evicted h1
    assert_eq!(plan("h3", &["p0"], &[], false), r#"["full"]"#);
    // Characters: one page record is ~1000+ chars; with a 2500 cap only the newest two fit, the newest stays alone at worst.
    run("collab.snap_reset", "");
    for h in ["a", "b", "c"] { remember(9, 2500, &snap(h, &page)); }
    assert_eq!(plan("a", &["p0"], &[], false), r#"["missing"]"#);
    assert_eq!(plan("c", &["p0"], &[], false), r#"["full"]"#);
    remember(9, 10, &snap("huge", &[("p0", 5000, 'y')]));
    assert_eq!(plan("c", &["p0"], &[], false), r#"["missing"]"#);
    assert_eq!(plan("huge", &["p0"], &[], false), r#"["full"]"#); // a single record is never evicted by size
}

#[test] fn snapshot_remember_rejects_snapshots_without_hash_or_pages() {
    assert_eq!(remember(3, 100, r#"{"contentHash":"","pages":[]}"#), "0");
    assert_eq!(remember(3, 100, r#"{"contentHash":"h","pages":"x"}"#), "0");
    assert_eq!(remember(3, 100, r#"{"pages":[]}"#), "0");
    assert!(call("collab.snap_remember", b"no header").unwrap().is_err());
    assert!(call("collab.snap_delta", b"{").unwrap().is_err());
}

#[test] fn snapshot_duplicate_and_missing_page_ids_follow_the_map_semantics() {
    // The second "d" page overwrites the first in the map, so base and current compare the last one.
    let a = r#"{"contentHash":"a","pages":[{"pageId":"d","t":"1"},{"pageId":"d","t":"2"},{"x":"y"},{"pageId":"e","t":""}]}"#;
    let b = r#"{"contentHash":"b","pages":[{"pageId":"d","t":"9"},{"pageId":"d","t":"2"},{"x":"z"},{"pageId":"e","t":""}]}"#;
    remember(3, 16_000_000, a); remember(3, 16_000_000, b);
    // Tiny pages stay under the 2048 character floor, so the plan is full; the differential test covers real deltas.
    assert_eq!(plan("b", &["d", "d", "legacy-page-3", "e"], &["a"], false), r#"["full"]"#);
}

#[test] #[ignore] fn bench_snapshot_remember() {
    let mut seed = 99u64;
    let mut rnd = move || { seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407); (seed >> 33) as f64 / (1u64 << 31) as f64 };
    for decimals in [None, Some(2usize)] {
        let mut text = String::from(r#"{"contentHash":"h","pages":["#);
        for p in 0..3 {
            if p > 0 { text.push(','); }
            text.push_str(&format!(r#"{{"pageId":"p{p}","strokes":["#));
            for s in 0..40 {
                if s > 0 { text.push(','); }
                text.push_str(&format!(r#"{{"id":"s{s}","points":["#));
                for q in 0..60 {
                    if q > 0 { text.push(','); }
                    let (x, y, pr) = (rnd() * 800.0, rnd() * 1100.0, rnd());
                    match decimals { None => text.push_str(&format!(r#"{{"x":{x},"y":{y},"p":{pr}}}"#)), Some(d) => text.push_str(&format!(r#"{{"x":{x:.d$},"y":{y:.d$},"p":{pr:.d$}}}"#)) }
                }
                text.push_str("]}");
            }
            text.push_str("]}");
        }
        text.push_str("]}");
        let input = format!("3,16000000\n{text}");
        let start = std::time::Instant::now();
        for _ in 0..20 { run("collab.snap_remember", &input); }
        println!("decimals {decimals:?}: {} KB, {:?} per remember", text.len() / 1024, start.elapsed() / 20);
        use super::json::{parse, write_stable, Pool};
        let t = std::time::Instant::now();
        for _ in 0..20 { let mut pool = Pool::default(); let _ = parse(&mut pool, text.as_bytes()).unwrap(); }
        println!("  parse only {:?}", t.elapsed() / 20);
        let mut pool = Pool::default(); let root = parse(&mut pool, text.as_bytes()).unwrap();
        let t = std::time::Instant::now();
        for _ in 0..20 { let mut out: Vec<u16> = Vec::new(); write_stable(&mut out, &pool, &root); std::hint::black_box(&out); }
        println!("  write only {:?}", t.elapsed() / 20);
    }
}
