//! W2.7 acceptance (BUILD-PLAN, SYNC §19.1 item 4 for the relay codecs and §19.2): the real relay and export parts
//! with W2.8's peer parts, through W2.6's simulator. The test binding [`serve`] plays the host side of the export
//! desk: a build is `IHN6:` + the snapshot, a 5.x revision is `STROKES_` + one import batch, anything else is plain.

use super::*;
use crate::export::{classify, push_name, Class};
use crate::sim::{sha256, Sim, SimReplica, PROP_BYTES, PROP_KEYS, T0};
use ihn_core::doc::{ReceiveResult, Tx};
use ihn_core::snapshot::{encode_batch, EncodedSnapshot};
use ihn_core::stamp::Stamp;
use ihn_core::testutil::Rng;

const ANN: &str = "ann@example.org";
const BOB: &str = "bob@example.org";
const EVE: &str = "eve@example.org";
const VIC: &str = "vic@example.org";

fn all() -> Vec<Box<dyn Part>> { crate::parts() }

// ------------------------------------------------------------------------------------------------ codecs

fn rand_file(rng: &mut Rng) -> RelayFile {
    let range = |rng: &mut Rng| {
        let first = 1 + rng.below(50) as u32;
        Range { actor: ActorId(rng.next() & !0xff), first, last: first + rng.below(9) as u32 }
    };
    let mut vv = VersionVector::default();
    vv.raise(ActorId(rng.next()), 1 + rng.below(9) as u32);
    let mut replica = SimReplica::new("d", ActorId::tab(0x5500, 0));
    let batches = (0..rng.below(4)).map(|_| replica.edit(1000, rng)).collect();
    RelayFile {
        doc_id: format!("doc{}", rng.below(1000)),
        inst: rng.next() & !0xff,
        gen: rng.next() >> rng.below(64),
        written_at: rng.next() >> 20,
        profile: Profile { email: "a@b.c".into(), name: "Ann".into(), photo_url: "https://x/y".into(), device_type: b'm', label: "Tab".into(), save_phase: 2 },
        mail: (0..rng.below(3)).map(|i| Mail { to: rng.next(), sid: rng.next(), kind: 1 + (i % 2) as u8, ttl_ms: 60_000, sdp: "v=0".into() }).collect(),
        export: rng.chance(0.5).then(|| ("rev-9".to_string(), vv)),
        assets: (0..rng.below(3)).map(|_| AssetRef { hash: [rng.below(256) as u8; 32], file_id: "gen1".into(), size: rng.next() >> 30 }).collect(),
        archives: (0..rng.below(3)).map(|_| ArchiveRef { file_id: "gen2".into(), digest: [7; 12], ranges: vec![range(rng), range(rng)] }).collect(),
        ranges: (0..rng.below(3)).map(|_| range(rng)).collect(),
        batches,
    }
}

#[test]
fn codecs_round_trip_and_never_panic() {
    let mut rng = Rng::new(11);
    for _ in 0..300 {
        let f = rand_file(&mut rng);
        let bytes = f.payload();
        assert_eq!(RelayFile::decode_payload(&bytes).as_ref(), Some(&f));
        let wrapped = wrap(false, rng.chance(0.5), &bytes);
        assert_eq!(unwrap(false, &wrapped).map(|x| x.1), Some(&bytes[..]));
        assert!(unwrap(true, &wrapped).is_none());
        // Truncated and bit-flipped inputs are rejected or decode to something else, never panic.
        for cut in [0, 1, bytes.len() / 2, bytes.len() - 1] {
            assert!(RelayFile::decode_payload(&bytes[..cut]).is_none());
        }
        for _ in 0..20 {
            let mut b = bytes.clone();
            let i = rng.below(b.len() as u64) as usize;
            b[i] ^= 1 << rng.below(8);
            let _ = RelayFile::decode_payload(&b);
            let _ = decode_archive(&b);
        }
        let mut extra = bytes.clone();
        extra.push(0);
        assert!(RelayFile::decode_payload(&extra).is_none());
        let arch = archive_payload(&f.doc_id, f.inst, &f.batches);
        assert_eq!(decode_archive(&arch), Some((f.doc_id.clone(), f.inst, f.batches.clone())));
        assert!(decode_archive(&arch[..arch.len() - 1]).is_none());
    }
    for _ in 0..2000 {
        let junk: Vec<u8> = (0..rng.below(64)).map(|_| rng.next() as u8).collect();
        let _ = RelayFile::decode_payload(&junk);
        let _ = decode_archive(&junk);
        let _ = Record::parse(&String::from_utf8_lossy(&junk));
    }
    // Records: the longest shape fits Drive's 124 bytes; anything malformed or with an unsafe id is refused.
    let rec = Record { gen: u64::MAX, nonce: u64::MAX, file_id: "x".repeat(MAX_FILE_ID), d16: d16(&[0xff; 32]), flags: "clx".into() };
    let key = record_key(u64::MAX & !0xff);
    assert_eq!(key.len(), 18);
    assert!(key.len() + rec.value().len() <= PROP_BYTES, "{}", rec.value().len());
    assert_eq!(Record::parse(&rec.value()), Some(rec.clone()));
    for bad in ["1|1|1|../x|AAAAAAAAAAAAAAAA|", "1|1|1|a b|AAAAAAAAAAAAAAAA|", "1|1|1|a|AAAAAAAAAAAAAAA|", "2|1|1|a|AAAAAAAAAAAAAAAA|"] {
        assert_eq!(Record::parse(bad), None, "{bad}");
    }
    assert_eq!(d16(&sha256(b"abc")).len(), 16);
    // Ranges of a batch list; 5.x keys; classification and the name rule.
    let a = ActorId(0x100);
    let mut r = SimReplica::new("d", a);
    let bs: Vec<_> = (0..4).map(|_| r.edit(5, &mut rng)).collect();
    assert_eq!(ranges_of(&[bs[3].clone(), bs[0].clone(), bs[1].clone()]), [Range { actor: a, first: 1, last: 2 }, Range { actor: a, first: 4, last: 4 }]);
    let meta = DriveMeta { props: vec![("p_x".into(), "1".into()), ("ihn_live_key_v1".into(), "k".into()), ("ihn6_x".into(), "1".into()), ("name".into(), "n".into())], app: vec![("ihn_lm1_a".into(), "1".into()), ("ihn6".into(), "1".into())], ..Default::default() };
    assert_eq!(legacy_keys(&meta), (vec!["p_x".to_string(), "ihn_live_key_v1".to_string()], vec!["ihn_lm1_a".to_string()]));
    assert_eq!([classify("STROKES_Z:!;IHN6:abc"), classify("STROKES_v1:xyz"), classify("")], [Class::V6, Class::V5, Class::Plain]);
    let (old, new) = (Stamp::new(10, a, 0), Stamp::new(20, a, 0));
    assert!(!push_name("Home name", old, &Some(("Home name".into(), new))));
    assert!(!push_name("Editor name", old, &Some(("Home name".into(), new))));
    assert!(push_name("Editor name", Stamp::new(30, a, 0), &Some(("Home name".into(), new))));
    assert_eq!(crate::export::recovered_name("Notes.pdf"), "Notes (recovered Drive version).pdf");
    assert!(relayed_by(ActorId::tab(0x1200, 3), 0x1200, "d") && !relayed_by(ActorId::tab(0x1300, 0), 0x1200, "d"));
    assert!(relayed_by(ActorId::import(&migration_source(0x1200, "d")), 0x1200, "d") && !relayed_by(ActorId::import("rev-1"), 0x1200, "d"));
    let files = [("f1".to_string(), "d1".to_string()), ("f2".to_string(), "d2".to_string()), ("f3".to_string(), "d3".to_string())];
    assert_eq!(stale_files(&files, &["d1".into()], &|d| d == "d3"), ["f1", "f3"]);
}

// ------------------------------------------------------------------------------------------------ world

/// The host side of the export desk and of renames (what W4.5/W5.4 will do), then a tick.
fn serve(sim: &mut Sim) {
    for d in 0..sim.devs.len() {
        let wall = sim.local(d) as u64;
        let dev = &mut sim.devs[d];
        let Some(sync) = dev.sync.as_mut() else { continue };
        let rep = &mut dev.replica;
        let desk = &mut sync.core.export;
        let mut touched = false;
        if let (Some(job), None) = (desk.build, &desk.built) {
            desk.built = Some((job, Some(([b"IHN6:".to_vec(), rep.snapshot().0].concat(), rep.doc.vv.clone()))));
            touched = true;
        }
        if let (Some((job, _, _, bytes)), None) = (&desk.merge, &desk.merged) {
            let vv = if let Some(snap) = bytes.strip_prefix(b"IHN6:") {
                let snap = EncodedSnapshot(snap.to_vec());
                let vv = snap.decode().ok().map(|d| d.vv);
                rep.load_snapshot(&snap, PeerRef::Export);
                vv
            } else if let Some(batch) = bytes.strip_prefix(b"STROKES_") {
                let b = EncodedBatch(batch.to_vec());
                let mut vv = VersionVector::default();
                if let Some((a, s, _)) = b.header() {
                    vv.add(a, s);
                }
                let _: ReceiveResult = rep.receive(&[b], PeerRef::Export);
                Some(vv)
            } else {
                Some(VersionVector::default())
            };
            desk.merged = Some((*job, vv));
            touched = true;
        }
        if let Some(name) = desk.rename.take() {
            if rep.doc.name.v != name {
                let mut tx = Tx::new(&rep.doc, &mut rep.author, wall);
                tx.set_name(&name);
                let b = tx.commit();
                let bytes = encode_batch(&b);
                rep.doc.apply(&b);
                rep.journal.insert((b.actor, b.seq), bytes.clone());
                sync.local_batch(&bytes);
            }
            touched = true;
        }
        if touched {
            sim.tick(d);
        }
    }
}

/// Runs `ms` in 10 ms steps, serving the desks.
fn go(sim: &mut Sim, ms: f64) {
    let end = sim.now + ms;
    while sim.now < end {
        sim.run(10.0f64.min(end - sim.now));
        serve(sim);
    }
}
fn until(sim: &mut Sim, max: f64, mut done: impl FnMut(&mut Sim) -> bool) -> bool {
    let end = sim.now + max;
    while !done(sim) {
        if sim.now >= end {
            return false;
        }
        go(sim, 10.0);
    }
    true
}

/// Editors `accounts` (first owns the document) with all parts, P2P blocked unless `p2p`.
fn world(seed: u64, accounts: &[&str], p2p: bool) -> (Sim, String, Vec<usize>) {
    let mut sim = Sim::new(seed);
    let doc = sim.drive.create_doc(accounts[0], "Notes");
    let devs = accounts
        .iter()
        .enumerate()
        .map(|(i, acc)| {
            if i > 0 {
                sim.drive.share(&doc, acc, true);
            }
            sim.add_dev(acc, 0x1000 * (i as u64 + 1), 0.0, all)
        })
        .collect();
    if !p2p {
        sim.block_all_p2p();
    }
    (sim, doc, devs)
}

fn has(sim: &Sim, d: usize, b: &EncodedBatch) -> bool {
    let (a, s, _) = b.header().unwrap();
    sim.devs[d].replica.doc.vv.contains(a, s)
}

fn record(sim: &Sim, doc: &str, d: usize) -> Option<Record> { sim.drive.files[doc].props.get(&record_key(sim.devs[d].inst)).and_then(|v| Record::parse(v)) }

/// Both devices open, one edit each, until each sees the other's record (company).
fn meet(sim: &mut Sim, doc: &str, devs: &[usize]) {
    for &d in devs {
        sim.open(d, doc);
    }
    go(sim, 500.0);
    for &d in devs {
        sim.edit(d);
    }
    assert!(until(sim, 90_000.0, |s| devs.iter().all(|&d| s.devs[d].sync.as_ref().is_some_and(|x| matches!(x.phase(), Phase::Live(Mode::Company | Mode::RelayOnly | Mode::Linked))))));
    assert!(until(sim, 30_000.0, |s| s.converged(devs)));
}

fn p95(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[(v.len() * 95).div_ceil(100) - 1]
}

// ------------------------------------------------------------------------------------------------ §19.2

#[test]
fn p2p_blocked_relay_only_latency_and_request_budget() {
    let (mut sim, doc, d) = world(21, &[ANN, BOB], false);
    let (a, b) = (d[0], d[1]);
    meet(&mut sim, &doc, &[a, b]);
    // Steady relay-only company: committed batches reach the other editor in < 4 s P95.
    let mut lat = Vec::new();
    let t0 = sim.now - T0;
    for i in 0..60 {
        let wait = sim.rng.range(200.0, 3000.0);
        go(&mut sim, wait);
        let (from, to) = if i % 3 == 0 { (b, a) } else { (a, b) };
        let batch = sim.edit(from);
        let t = sim.now;
        assert!(until(&mut sim, 20_000.0, |s| has(s, to, &batch)), "batch {i} never arrived");
        lat.push(sim.now - t);
    }
    let p = p95(lat.clone());
    assert!(p < 4000.0, "relay P95 {p} ms: {lat:?}");
    assert!(until(&mut sim, 20_000.0, |s| s.converged(&[a, b])));
    assert!(matches!(sim.phase(a), Phase::Live(Mode::RelayOnly | Mode::Company)));
    let t1 = sim.now - T0;
    // Budget: never two relay writes of one device within 2 s; ≤ 2 GET/s; records within Drive's limits.
    for dev in [a, b] {
        let writes: Vec<f64> = sim.drive.log.iter().filter(|l| l.dev == dev && l.kind == "props" && !l.keepalive).map(|l| l.at).collect();
        assert!(writes.windows(2).all(|w| w[1] - w[0] >= 1600.0), "{writes:?}");
        let gets = sim.drive.log.iter().filter(|l| l.dev == dev && l.at >= t0 && matches!(l.kind, "meta" | "media")).count() as f64;
        assert!(gets / ((t1 - t0) / 1000.0) <= 2.0);
    }
    let f = &sim.drive.files[&doc];
    assert!(f.props.len() <= PROP_KEYS && f.props.iter().all(|(k, v)| k.len() + v.len() <= PROP_BYTES));
    assert!(sim.drive.log.iter().all(|l| l.status != 400));
    // Only the scribe (smallest installation) exported, once things were idle.
    go(&mut sim, 75_000.0);
    assert!(sim.drive.count(a, "put", 0.0, f64::MAX) >= 1);
    assert_eq!(sim.drive.count(b, "put", t0, f64::MAX), 0);
    assert!(sim.devs[b].stored.exports.iter().any(|(h, _)| *h == sim.drive.files[&doc].head_id()));
}

#[test]
fn lone_device_budget_and_hidden_silence() {
    let (mut sim, doc, d) = world(22, &[ANN], false);
    let a = d[0];
    sim.open(a, &doc);
    go(&mut sim, 10_000.0);
    // Opening: one poll, the id pool, the sync folder and (after its grace) one look at the unknown head.
    let t = sim.now - T0;
    go(&mut sim, 120_000.0);
    assert!(sim.drive.count(a, "meta", t, f64::MAX) <= 7, "{:?}", sim.drive.log);
    assert!(sim.drive.log.iter().filter(|l| l.at >= t).all(|l| l.kind == "meta"), "an idle lone device only polls {:?}", sim.drive.log);
    // Writing alone: ≤ 4 relay writes/min; then an export when idle.
    let t = sim.now - T0;
    for _ in 0..60 {
        sim.edit(a);
        go(&mut sim, 1000.0);
    }
    assert!(sim.drive.count(a, "props", t, t + 60_000.0) <= 4);
    assert!(until(&mut sim, 120_000.0, |s| s.devs[a].stored.export_vv().is_some_and(|vv| vv.covers(&s.devs[a].replica.doc.vv))));
    assert_eq!(sim.sync(a).status().unsealed_local, 0);
    // Hidden: one keepalive flush, then nothing.
    sim.edit(a);
    sim.io(a, SyncIo::Lifecycle(Lifecycle::Hidden));
    go(&mut sim, 1000.0);
    assert!(sim.drive.log.iter().rev().take(3).any(|l| l.kind == "props" && l.keepalive));
    let t = sim.now - T0;
    go(&mut sim, 600_000.0);
    // Suspended after 60 s: record `l` (keepalive) and, alone with an unexported edit, the leaving export; then
    // nothing at all.
    let at_suspend: Vec<&str> = sim.drive.log.iter().filter(|l| l.at >= t).map(|l| l.kind).collect();
    assert!(at_suspend.contains(&"put") && sim.drive.log.iter().all(|l| l.at < t + 65_000.0), "{at_suspend:?}");
    assert!(sim.drive.files[&doc].props[&record_key(sim.devs[a].inst)].ends_with('l'));
    assert!(sim.devs[a].stored.export_vv().is_some_and(|vv| vv.covers(&sim.devs[a].replica.doc.vv)));
    assert_eq!(sim.devs[a].stored.relay_pending, false);
}

#[test]
fn viewer_receives_everything_and_writes_nothing() {
    let (mut sim, doc, d) = world(23, &[ANN, BOB], true);
    sim.drive.share(&doc, VIC, false);
    let v = sim.add_dev(VIC, 0x9000, 0.0, all);
    meet(&mut sim, &doc, &[d[0], d[1]]);
    sim.open(v, &doc);
    for i in 0..20 {
        sim.edit(d[i % 2]);
        go(&mut sim, 700.0);
    }
    assert!(until(&mut sim, 30_000.0, |s| s.converged(&[d[0], d[1], v])));
    assert_eq!(sim.phase(v), Phase::Live(Mode::Viewer));
    assert!(sim.drive.log.iter().filter(|l| l.dev == v).all(|l| matches!(l.kind, "meta" | "media")));
    assert!(!sim.devs[v].effects.iter().any(|e| matches!(e, SyncEffect::Rtc(_) | SyncEffect::Send { to: Dest::Peer(_), .. })));
    assert!(sim.drive.files[&doc].props.keys().all(|k| *k != record_key(sim.devs[v].inst)));
    assert!(sim.devs[v].replica.received.iter().any(|r| matches!(r, PeerRef::Relay(_))));
}
