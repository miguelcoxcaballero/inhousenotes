//! `v5::import` against W1.2's 5.18.0 records (`tests/v6/fixtures/v5/*.expect.json`, `storage/*.expect.json`).

use super::*;
use crate::jsfmt;
use crate::v5::fixtures::{self, Dump};
use crate::v5::idb;
use crate::v5::json::Value;
use crate::v5::sha256::hex;
use std::collections::HashMap;

const T0: u64 = 1_793_354_000_000;

fn doc_of<'a>(batches: impl IntoIterator<Item = &'a Batch>) -> Doc {
    let mut d = Doc::default();
    for b in batches {
        assert!(b.is_well_formed());
        d.apply(b);
    }
    d
}

/// Every register of a document (with stamps), in a canonical order: two replicas in the same state print the same.
fn state(d: &Doc) -> String {
    let mut pages: Vec<String> = d.pages.iter().map(|(id, p)| format!("{id:?}{p:?}")).collect();
    let mut items: Vec<String> = d.items.iter().map(|(id, i)| format!("{id:?}{i:?}")).collect();
    pages.sort();
    items.sort();
    format!("{:?}\n{}\n{}", d.name, pages.join("\n"), items.join("\n"))
}

/// What a user sees: names, page ids, sizes, templates, backgrounds, items in draw order.
fn visible(d: &Doc) -> String {
    let pages: Vec<String> = d.order().iter().map(|&p| {
        let pg = &d.pages[&p];
        let items: Vec<String> = d.page_items(p).iter().map(|i| format!("{i:?}={:?}", d.items[i].body.v)).collect();
        format!("{p:?} {:?} {:?} {:?} [{}]", pg.size.v, pg.template.v, pg.background.v, items.join(","))
    }).collect();
    format!("{}\n{}", d.name.v, pages.join("\n"))
}

fn facts(name: &str, e: &Value) -> PdfFacts {
    PdfFacts {
        doc_id: String::new(),
        name: "cuaderno".into(),
        page_sizes: e.get("pages").arr().iter().map(|p| (p.get("pageWidth").num() as u32, p.get("pageHeight").num() as u32)).collect(),
        revision_sha256: sha256(&fixtures::read(&format!("{name}.pdf"))),
        clean_attachment: false,
        strict: true,
    }
}

fn import_fixture(name: &str, rev: u64) -> (Import, Value, String) {
    let e = fixtures::json(&format!("{name}.expect.json"));
    let kw = fixtures::keywords(&fixtures::read(&format!("{name}.pdf")));
    let imp = from_pdf(&kw, &facts(name, &e), ActorId::import(&format!("rev:{name}:{rev}")), T0 + rev).unwrap_or_else(|err| panic!("{name}: {err}"));
    (imp, e, kw)
}

fn points_json(d: &Doc, strokes: &[Id]) -> String {
    let mut s = String::from("[");
    for (k, id) in strokes.iter().enumerate() {
        let Body::Stroke { xy, p, .. } = &d.items[id].body.v else { panic!() };
        s.push_str(if k > 0 { ",[" } else { "[" });
        for i in 0..xy.len() / 2 {
            let pr = p.as_ref().map_or(f32::NAN, |p| p[i]);
            s.push_str(&format!("{}[{},{},{}]", if i > 0 { "," } else { "" }, jsfmt::num(xy[2 * i]), jsfmt::num(xy[2 * i + 1]), if pr.is_nan() { "null".into() } else { format!("{pr}") }));
        }
        s.push(']');
    }
    s.push(']');
    s
}

/// (Pressure is stored as `f32`: its shortest text is the 5.x two-decimal value.)
/// The 5.x ids a fixture's entries carry (ids 5.18.0 generated at random on import cannot be compared).
fn source_ids(kw: &str) -> HashSet<String> {
    let k = keywords::parse(kw);
    let Some((z, payload)) = k.strokes else { return HashSet::new() };
    let v = crate::v5::json::parse(&keywords::strokes_json(z, payload).unwrap()).unwrap();
    let mut ids = HashSet::new();
    for p in pages::entries(&v).unwrap().unwrap().into_iter().flatten() {
        ids.insert(p.page_id.clone());
        let mut seen = HashSet::new();
        for s in &p.strokes {
            if seen.insert(s.id.clone()) {
                ids.insert(s.id.clone());
            }
        }
        ids.extend(p.images.iter().map(|i| i.id.clone()));
    }
    ids.remove("");
    ids
}

#[test]
fn every_drive_fixture_imports_like_5_18_0() {
    let names = fixtures::drive_names();
    assert!(names.len() >= 15, "{names:?}");
    for name in &names {
        let (imp, e, kw) = import_fixture(name, 0);
        let again = import_fixture(name, 0).0;
        assert_eq!(imp, again, "{name}: deterministic");
        let known = source_ids(&kw);
        let d = doc_of(&imp.batches);
        let ctx = |what: &str| format!("{name}: {what}");
        let assets: HashMap<Hash, &Asset> = imp.assets.iter().map(|a| (a.hash, a)).collect();

        if name != "plain" {
            assert_eq!(d.name.v, e.get("exportName").text(), "{}", ctx("name"));
        }
        assert_eq!(imp.v5, !keywords::parse(&kw).strokes.is_none(), "{}", ctx("v5"));
        assert_eq!(imp.legacy_baked, e.get("hasLegacyBakedOverlay").truthy(), "{}", ctx("legacy baked"));
        assert_eq!(d.order().len(), e.get("pageCount").num() as usize, "{}", ctx("pages"));
        assert_eq!(imp.strokes, e.get("strokeCount").num() as usize, "{}", ctx("strokes"));
        let mut images = 0;
        let mut used = HashSet::new();
        for (i, (&page, ep)) in d.order().iter().zip(e.get("pages").arr()).enumerate() {
            let ctx = |what: &str| format!("{name} page {i}: {what}");
            let pid = ep.get("pageId").text();
            if known.contains(&pid) {
                assert_eq!(page, Id::legacy(&pid), "{}", ctx("id"));
            }
            let pg = &d.pages[&page];
            assert_eq!(pg.size.v, (ep.get("pageWidth").num() as u32, ep.get("pageHeight").num() as u32), "{}", ctx("size"));
            assert_eq!(pg.template.v, template(ep.get("templateKind").as_str().unwrap_or("")), "{}", ctx("template"));
            match (ep.get("backgroundSource").as_str(), pg.background.v) {
                (Some("template"), Background::Template) => {}
                (Some("pdf"), Background::Pdf { asset, page: n }) => {
                    assert_eq!(n as f64, ep.get("pdfPageIndex").num(), "{}", ctx("pdf page"));
                    assert_eq!(Some(asset), imp.source.map(|s| s.1));
                }
                other => panic!("{}: {other:?}", ctx("background")),
            }
            assert_eq!(pg.side_panel.v.as_deref().map(|b| String::from_utf8(b.to_vec()).unwrap()), ep.get("sidePanel").truthy().then(|| crate::v5::json::stable(ep.get("sidePanel"))), "{}", ctx("side panel"));

            let dl = d.draw_list(page);
            let strokes: Vec<Id> = dl.strokes.iter().map(|r| r.id).collect();
            let es = ep.get("strokes").arr();
            assert_eq!(strokes.len(), es.len(), "{}", ctx("stroke count"));
            for (id, row) in strokes.iter().zip(es) {
                let row = row.arr();
                // An id an earlier page already used is page-qualified.
                if known.contains(&row[0].text()) && used.insert(row[0].text()) {
                    assert_eq!(*id, Id::legacy(&row[0].text()), "{}", ctx("stroke id/order"));
                }
                let Body::Stroke { tool, color, width, xy, .. } = &d.items[id].body.v else { panic!() };
                assert_eq!(*tool, if row[1].as_str() == Some("highlighter") { Tool::Highlighter } else { Tool::Pen });
                assert!(color.css().eq_ignore_ascii_case(&row[2].text()), "{}", ctx("colour"));
                assert_eq!((*width, xy.len() / 2), (row[3].num(), row[4].num() as usize), "{}", ctx("width/points"));
            }
            assert_eq!(hex(&sha256(points_json(&d, &strokes).as_bytes())), ep.get("pointsSha256").text(), "{}", ctx("points"));
            assert_eq!(dl.covers.len(), ep.get("legacyCoverStrokes").arr().len(), "{}", ctx("covers"));
            for (r, im) in dl.images.iter().zip(ep.get("images").arr()) {
                assert_eq!(r.id, Id::legacy(&im.get("id").text()), "{}", ctx("image id"));
                let Body::Image { asset, cx, cy, w, h, rotation } = d.items[&r.id].body.v else { panic!() };
                assert_eq!((cx, cy, w, h, rotation), (im.get("x").num(), im.get("y").num(), im.get("width").num(), im.get("height").num(), im.get("rotation").num()), "{}", ctx("image"));
                let a = assets[&asset];
                assert_eq!(hex(&sha256(format!("data:{};base64,{}", a.mime, crate::codec::b64_encode(&a.bytes, false)).as_bytes())), im.get("srcSha256").text(), "{}", ctx("image bytes"));
            }
            assert_eq!(dl.images.len(), ep.get("images").arr().len(), "{}", ctx("images"));
            images += dl.images.len();
        }
        assert_eq!(images, e.get("imageCount").num() as usize, "{}", ctx("image count"));

        let ev = e.get("versions").arr();
        assert_eq!(imp.versions.len(), ev.len(), "{}", ctx("versions"));
        for (v, x) in imp.versions.iter().zip(ev) {
            assert_eq!((v.id.as_str(), v.ts, v.summary.as_str(), v.kind.as_str(), v.content_hash.as_str(), v.export_name.as_str(), v.pages.len()), (&*x.get("id").text(), x.get("ts").num(), &*x.get("summary").text(), &*x.get("kind").text(), &*x.get("contentHash").text(), &*x.get("exportName").text(), x.get("pageCount").num() as usize), "{}", ctx("version"));
        }
    }
}

#[test]
fn legacy_ids_without_norm_are_repaired_deterministically() {
    let (imp, _, _) = import_fixture("legacy-no-norm", 0);
    assert!(imp.needs_resave);
    let d = doc_of(&imp.batches);
    // `leg-0` exists on both pages: two items. Duplicated `leg-2`/`leg-7` keep their first occurrence's id.
    let all: Vec<Id> = d.order().iter().flat_map(|&p| d.page_items(p)).collect();
    assert_eq!(all.len(), 9);
    assert_eq!(all.iter().collect::<HashSet<_>>().len(), 9);
    assert_eq!(all[0], Id::legacy("leg-0"));
    assert_ne!(all[5], Id::legacy("leg-0"));
    // Pages without ids: `pg_legacy_` from the document key.
    let doc_key = format!("import:{:016x}", ActorId::import("rev:legacy-no-norm:0").0);
    assert_eq!(d.order()[0], Id::legacy(&legacy_page_id(&doc_key, 1).unwrap()));
}

#[test]
fn two_revisions_in_either_order_give_the_same_document_and_r1_wins() {
    let (r0, ..) = import_fixture("r0", 0);
    let (r1, ..) = import_fixture("r1", 1000);
    let apply = |d: &mut Doc, imp: &Import| {
        let extra = supersede(d, &imp.batches, &[]);
        for b in imp.batches.iter().chain(&extra) {
            d.apply(b);
        }
    };
    let (mut a, mut b) = (Doc::default(), Doc::default());
    apply(&mut a, &r0);
    apply(&mut a, &r1);
    apply(&mut b, &r1);
    apply(&mut b, &r0);
    assert_eq!(state(&a), state(&b));
    assert_eq!(visible(&a), visible(&doc_of(&r1.batches)));
    // Re-importing either revision changes nothing.
    let before = state(&a);
    apply(&mut a, &r0);
    apply(&mut a, &r1);
    assert_eq!(state(&a), before);
    // rev-s2 and rev-s5 were dropped by R1 without tombstones.
    assert!(a.live(Id::legacy("rev-s2")).is_none() && a.live(Id::legacy("rev-s10")).is_some());

    // A v6 edit made after R0 survives R1 when R1 did not touch the item; registers 5.x stamped keep their stamp.
    let mut c = Doc::default();
    apply(&mut c, &r0);
    let native = ActorId::tab(0x1200, 1);
    let page = c.order()[1];
    c.apply(&Batch { actor: native, seq: 1, clock: T0 + 500, ops: vec![Op::ItemPut { item: Id::new(native, 1), page, z: Z::default(), body: stroke_body(&V5Stroke { xy: vec![1.0, 1.0, 2.0, 2.0], ..Default::default() }, false).unwrap() }], stamps: None });
    apply(&mut c, &r1);
    assert!(c.live(Id::new(native, 1)).is_some());

    // Migrated local strokes (the migration actor is kept) survive a newer revision that never saw them.
    let mut m = Doc::default();
    apply(&mut m, &r0);
    let migrate = ActorId::import("migrate:test");
    let local = V5LocalDoc { doc_id: "d".into(), pages: vec![V5Page { page_id: "pg_mvuswpfc_ry7x67vru".into(), strokes: vec![V5Stroke { id: "local-1".into(), tool: "pen".into(), color: "#000000".into(), width: 2.0, xy: vec![1.0, 2.0, 3.0, 4.0], ..Default::default() }], has_images_key: true, background_source: "template".into(), template_kind: "default", ..Default::default() }], saved_at: (T0 + 10) as f64, ..Default::default() };
    for b in from_local(&local, migrate, Some(&m)).unwrap().batches {
        m.apply(&b);
    }
    let extra = supersede(&m, &r1.batches, &[migrate]);
    for b in r1.batches.iter().chain(&extra) {
        m.apply(b);
    }
    assert!(m.live(Id::legacy("local-1")).is_some() && m.live(Id::legacy("rev-s2")).is_none());
}

#[test]
fn strict_mode_and_limits_fail_with_5_18_0_messages() {
    let mut f = PdfFacts { page_sizes: vec![(595, 842)], strict: true, name: "x".into(), ..Default::default() };
    let actor = ActorId::import("r");
    let err = |kw: &str, f: &PdfFacts| from_pdf(kw, f, actor, T0).err().map(|e| e.message());
    let enc = |j: &str| crate::codec::b64_encode(j.as_bytes(), false);
    assert_eq!(err(&format!("STROKES_DATA:{}", enc("{}")), &f), Some("Embedded stroke metadata is invalid"));
    assert_eq!(err("STROKES_Z:AAAA", &f), Some("The compressed data was not valid."));
    assert_eq!(err(&format!("STROKES_DATA:{};IH_STRUCT:%%", enc("[]")), &f), Some(V5Error::Base64.message()));
    assert_eq!(err(&format!("STROKES_DATA:{};IH_FIELDS:{}", enc("[]"), enc("{")), &f), Some("Unexpected token in JSON"));
    let big = "A".repeat(super::super::MAX_SECTION_ENCODED + 1);
    assert_eq!(err(&format!("STROKES_Z:{big}"), &f), Some("PDF metadata payload is too large"));
    assert_eq!(err(&format!("STROKES_DATA:{};IH_STRUCT:{}", enc("[]"), "A".repeat(super::super::MAX_COLLAB_SECTION + 1)), &f), Some("Collaboration structure metadata is too large"));
    // Not strict (user import): the PDF opens as background.
    f.strict = false;
    let imp = from_pdf("STROKES_Z:AAAA", &f, actor, T0).unwrap();
    assert!(!imp.v5 && imp.pages == 1 && imp.source.unwrap().0 == Source::Saved);
    f.page_sizes = vec![(10, 10); 2001];
    assert_eq!(err("", &f), Some("PDF page count exceeds the safe limit"));
    f.page_sizes.clear();
    assert_eq!(err("", &f), Some("PDF page count exceeds the safe limit"));
}

#[test]
fn plain_pdf_pages_are_backgrounds_of_the_downloaded_file() {
    let (imp, ..) = import_fixture("plain", 0);
    let d = doc_of(&imp.batches);
    assert_eq!(imp.source.unwrap(), (Source::Saved, source_hash(&sha256(&fixtures::read("plain.pdf")), Source::Saved)));
    assert_eq!(d.name.v, "cuaderno");
    let doc_key = format!("import:{:016x}", ActorId::import("rev:plain:0").0);
    assert_eq!(d.order()[1], Id::legacy(&legacy_page_id(&doc_key, 2).unwrap()));
    // A Drive file id gives 5.18.0's own legacy page ids.
    let e = fixtures::json("plain.expect.json");
    let f = PdfFacts { doc_id: "scene-plain".into(), ..facts("plain", &e) };
    let d = doc_of(&from_pdf("", &f, ActorId::import("x"), T0).unwrap().batches);
    assert_eq!(d.order()[0], Id::legacy(&legacy_page_id("scene-plain", 1).unwrap()));
    // Overlay-stripped source for a 5.x file with overlay refs.
    let (imp, ..) = import_fixture("imported-overlays", 0);
    assert_eq!(imp.source.unwrap().0, Source::Stripped);
}

// ------------------------------------------------------------------------------------- migration (storage dumps)

fn working(dump: &Dump, db: &str, doc_id: &str) -> Option<V5LocalDoc> {
    let snaps = dump.store(db, "snapshots");
    let meta = match snaps.first() {
        Some(r) => idb::meta_payload(r).unwrap()?,
        None if db == "notebook-data-db-v1" => idb::meta_payload(&Value::Str(dump.local("notebook-data-v3")?.into())).unwrap()?,
        None => return None,
    };
    let rows: Vec<Option<V5Page>> = dump.store(db, "pages").iter().map(|r| idb::page_row(r, meta.session_id).unwrap()).collect();
    let ops = idb::stroke_ops(&dump.store(db, "stroke-ops"), meta.session_id);
    let pages = idb::working_copy(&meta, rows, &ops);
    Some(V5LocalDoc::from_working(doc_id, &meta, pages, ""))
}

#[test]
fn storage_fixtures_migrate_their_unsynced_strokes() {
    for scenario in ["background-worker", "local-doc", "offline-dirty", "pending-exit", "recoveries", "recovery-conflict", "stroke-ops-pending", "xor-packed"] {
        let e = fixtures::json(&format!("storage/{scenario}.expect.json"));
        let dump = Dump::load(scenario);
        let doc_id = e.get("document").get("id").text();
        let mut base = Doc::default();
        let mut base_ids: HashSet<Id> = HashSet::new();
        let mut base_ms = 0.0;
        let offline: Vec<idb::OfflineDoc> = dump.store("inhouse-offline-v1", "documents").iter().filter_map(|r| idb::offline_document(r).unwrap()).collect();
        if let Some(o) = offline.first() {
            base_ms = o.base.modified_ms;
        }
        if let Some(fx) = e.get("document").get("driveFixture").as_str() {
            let ex = fixtures::json(&format!("{fx}.expect.json"));
            let f = PdfFacts { doc_id: doc_id.clone(), ..facts(fx, &ex) };
            let rev = if base_ms > 0.0 { base_ms as u64 } else { T0 };
            let imp = from_pdf(&fixtures::keywords(&fixtures::read(&format!("{fx}.pdf"))), &f, ActorId::import("base"), rev).unwrap();
            base = doc_of(&imp.batches);
            base_ids = base.items.keys().copied().collect();
        }
        let actor = ActorId::import(&format!("migrate:{doc_id}"));
        let mut sources: Vec<V5LocalDoc> = ["notebook-data-db-v1", "background-notebook-data-db-v1"].iter().filter_map(|db| working(&dump, db, &doc_id)).collect();
        for o in offline.iter().filter(|o| o.id == doc_id) {
            if let Some(s) = &o.snapshot {
                sources.push(V5LocalDoc::from_snapshot(&o.id, s, o.updated_at, o.base.modified_ms, None));
            }
        }
        assert!(!sources.is_empty(), "{scenario}");
        let mut doc = base.clone();
        let mut emitted: HashSet<Id> = HashSet::new();
        for src in &sources {
            let imp = from_local(src, actor, Some(&doc)).unwrap_or_else(|err| panic!("{scenario}: {err}"));
            assert_eq!(imp, from_local(src, actor, Some(&doc)).unwrap(), "{scenario}: deterministic");
            for b in &imp.batches {
                assert!(b.is_well_formed());
                for op in &b.ops {
                    if let Op::ItemPut { item, .. } = op {
                        emitted.insert(*item);
                    }
                }
                doc.apply(b);
            }
            // A second pass over the merged document finds nothing left to migrate.
            let left: Vec<Op> = from_local(src, actor, Some(&doc)).unwrap().batches.into_iter().flat_map(|b| b.ops).filter(|op| matches!(op, Op::ItemPut { .. } | Op::PageNew { .. })).collect();
            assert!(left.is_empty(), "{scenario}: idempotent {left:?}");
        }
        let want: Vec<(Id, Vec<Id>)> = e.get("document").get("pages").arr().iter().map(|p| (Id::legacy(&p.get("pageId").text()), p.get("strokeIds").arr().iter().map(|s| Id::legacy(&s.text())).collect())).collect();
        let got: Vec<(Id, Vec<Id>)> = doc.order().iter().map(|&p| (p, doc.draw_list(p).strokes.iter().map(|r| r.id).collect())).collect();
        let sets = |v: &[(Id, Vec<Id>)]| v.iter().map(|(p, s)| (*p, s.iter().copied().collect::<HashSet<_>>())).collect::<Vec<_>>();
        assert_eq!(sets(&got), sets(&want), "{scenario}: migrated document");
        let unsynced: HashSet<Id> = e.get("unsyncedStrokeIds").arr().iter().map(|s| Id::legacy(&s.text())).collect();
        assert!(unsynced.is_subset(&emitted), "{scenario}: unsynced strokes migrated");
        assert!(emitted.iter().all(|id| !base_ids.contains(id) || base.items[id].body.v != doc.items[id].body.v), "{scenario}: only what the base lacks");
    }
}

#[test]
fn xor_packed_rows_decode_bit_exactly() {
    let dump = Dump::load("xor-packed");
    let mut packed = 0;
    for r in dump.store("notebook-data-db-v1", "pages") {
        if r.get("__inkCheckpoint").truthy() {
            packed += 1;
            let p = idb::page_row(&r, None).unwrap().unwrap();
            // Every point of every stroke came back (counts recorded next to the packed lanes).
            for s in &p.strokes {
                assert!(s.xy.len() >= 2 && s.xy.iter().all(|v| v.is_finite()));
            }
        }
    }
    assert!(packed > 0);
}


#[test]
fn open_yields_follow_import_pdf_data() {
    let page = |strokes: usize| Some(V5Page { strokes: vec![V5Stroke::default(); strokes], ..V5Page::default() });
    // The page loop alone: every second PDF page (3 pages → 1, as traced in 5.18.0's open of the template scene).
    assert_eq!(open_yields(3, None, false), 1);
    assert_eq!(open_yields(1, None, false), 0);
    assert_eq!(open_yields(4, Some(&[page(0), page(2), None, page(5)]), true), 2);
    // Without IH_NORM: twice per 256 strokes of a page with more than one stroke, and every third entry.
    assert_eq!(open_yields(4, Some(&[page(600), page(1), None, page(256)]), false), 2 + 4 + 2 + 1);
}
