//! Timeline versions (ARCHITECTURE §4.6): the `VersionMeta` record, version-snapshot assets with per-actor delta
//! chains, 5.18.0's capture / coalesce / prune rules, restore planning, the change summary and the preview diff
//! highlights. Owned by W2.9. Pure: asset bytes come in through a lookup closure; the caller hashes new assets
//! (SHA-256, effect family 3.8) and stores them.
//!
//! Flow of a capture (W5.3 `doc/versions.rs`): `plan_capture` → hash `Planned::asset` → store the asset →
//! `Planned::commit(hash, doc, tx)` adds the `VersionAdd` (a coalesced entry rewrites the last entry's register) and
//! the `VersionDel`s of pruning → commit the `Tx`. Restore: capture the `pre-restore` milestone, then push
//! `plan_restore(doc, &materialize(..))` into one `Tx` and capture again with the `restore` hint.
//!
//! Snapshot asset: `u8 1 ‖ u8 has_base [‖ hash base] ‖ str name ‖ uv pages ‖ page*`, page =
//! `op PageNew{id, Pos::rank(i), template, size, background} ‖ u8 has_panel [‖ bytes panel] ‖ uv items ‖ item*`,
//! item = `u8 tag ‖ id [‖ z if tag ≠ 0] [‖ body if tag = 1]`: tag 0 reuses the base snapshot's z and body for that
//! id, tag 2 its body only, tag 1 is self-contained. Bodies are deduplicated by `(id, body)` against the author's
//! previous version, and every
//! [`CHECKPOINT_EVERY`]th version of an actor (or one whose base cannot be read) is self-contained.

use std::collections::{HashMap, HashSet};

use crate::codec::{Reader, Writer};
use crate::doc::{Doc, Tx};
use crate::id::{fnv1a64, ActorId, Hash, Id, IdMap};
use crate::jsmath;
use crate::op::{Background, Body, Op, PageProp, Template, Tool, VersionMeta, Z};
use crate::pos::Pos;
use crate::snapshot::{get_body, get_bytes, get_hash, get_id, get_op, get_str, get_z, put_body, put_bytes, put_hash, put_id, put_op, put_str, put_z};

/// v5: app-v5.js `VERSION_HISTORY_MAX`.
pub const MAX_ENTRIES: usize = 80;
/// v5: app-v5.js `VERSION_HISTORY_MILESTONE_MAX`.
pub const MAX_MILESTONES: usize = 30;
/// v5: app-v5.js `VERSION_HISTORY_COALESCE_MS`.
pub const COALESCE_MS: f64 = 300_000.0;
/// A self-contained snapshot every 8 versions of one actor (5.x `checkpointInterval`).
pub const CHECKPOINT_EVERY: u8 = 8;
/// v5: app-v5.js:normalizeTimelineEntry `milestoneKinds`.
pub const MILESTONE_KINDS: [&str; 5] = ["baseline", "restore", "revert-original", "pre-restore", "manual"];

const RECORD_FORMAT: u8 = 1;
const ASSET_FORMAT: u8 = 1;
/// Longest chain `materialize` follows (native chains are ≤ 8; this only bounds damaged or hostile data).
const MAX_CHAIN: usize = 64;

// ------------------------------------------------------------------------------------------------ record

/// The `VersionMeta.data` record (5.x entry fields; the pages live in the snapshot asset).
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Record {
    /// Capture time, ms since the epoch.
    pub ts: f64,
    pub email: String,
    pub name: String,
    pub photo: String,
    /// The capturing device (5.x `deviceId`); coalescing requires the same one.
    pub device: String,
    /// `autosave` or one of [`MILESTONE_KINDS`] (5.x `kind`).
    pub kind: String,
    pub milestone: bool,
    pub summary: String,
    /// FNV-1a 64 of the self-contained encoding of the snapshot (5.x `contentHash`): skip and dedupe key.
    pub content: u64,
    /// SHA-256 of the snapshot asset.
    pub snapshot: Hash,
    /// Snapshot asset this one is a delta of (`None` = checkpoint).
    pub base: Option<Hash>,
    /// Position in the actor's chain since its checkpoint (0 = checkpoint).
    pub depth: u8,
}

impl Record {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::default();
        w.u8(RECORD_FORMAT);
        w.f64(self.ts);
        for s in [&self.email, &self.name, &self.photo, &self.device, &self.kind, &self.summary] {
            put_str(&mut w, s);
        }
        w.u8(self.milestone as u8);
        w.u64(self.content);
        put_hash(&mut w, &self.snapshot);
        put_opt_hash(&mut w, self.base.as_ref());
        w.u8(self.depth);
        w.buf
    }

    /// `None` for damaged or unknown records (they are not listed). Trailing bytes are allowed (later fields).
    pub fn decode(bytes: &[u8]) -> Option<Record> {
        let r = &mut Reader::new(bytes);
        if r.u8()? != RECORD_FORMAT {
            return None;
        }
        let ts = r.f64()?;
        let mut s = || get_str(r).map(str::to_string);
        let (email, name, photo, device, kind, summary) = (s()?, s()?, s()?, s()?, s()?, s()?);
        Some(Record {
            ts,
            email,
            name,
            photo,
            device,
            kind,
            summary,
            milestone: r.u8()? != 0,
            content: r.u64()?,
            snapshot: get_hash(r)?,
            base: get_opt_hash(r)?,
            depth: r.u8()?,
        })
    }
}

fn put_opt_hash(w: &mut Writer, h: Option<&Hash>) {
    match h {
        Some(h) => {
            w.u8(1);
            put_hash(w, h);
        }
        None => w.u8(0),
    }
}
fn get_opt_hash(r: &mut Reader) -> Option<Option<Hash>> {
    match r.u8()? {
        0 => Some(None),
        1 => Some(Some(get_hash(r)?)),
        _ => None,
    }
}

/// Listed timeline entries (added, not pruned, decodable), oldest first: 5.18.0 order `(ts, id)`.
pub fn entries(doc: &Doc) -> Vec<(Id, Record)> {
    let mut list: Vec<(Id, Record)> =
        doc.versions.iter().filter(|(_, v)| v.is_listed()).filter_map(|(id, v)| Some((*id, Record::decode(&v.data.v)?))).collect();
    list.sort_by(|a, b| a.1.ts.total_cmp(&b.1.ts).then(a.0.cmp(&b.0)));
    list
}

fn actor_of(id: Id) -> u64 { (id.0 >> 64) as u64 }

/// Newest version (listed or pruned) of `actor`'s own chain.
fn chain_tail(doc: &Doc, actor: ActorId) -> Option<Record> {
    let lo = Id((actor.0 as u128) << 64);
    doc.versions.range(lo..=Id(lo.0 | u64::MAX as u128)).rev().find_map(|(_, v)| Record::decode(&v.data.v))
}

/// Snapshot assets still needed: those of listed versions and, transitively, the bases of their delta chains
/// (a pruned version's asset stays while a later version of its chain needs it).
pub fn needed_assets(doc: &Doc) -> Vec<Hash> {
    let records: Vec<(bool, Record)> =
        doc.versions.values().filter_map(|v| Some((v.is_listed(), Record::decode(&v.data.v)?))).collect();
    let bases: HashMap<Hash, Hash> = records.iter().filter_map(|(_, r)| Some((r.snapshot, r.base?))).collect();
    let mut seen = HashSet::new();
    let mut need = Vec::new();
    for (_, r) in records.iter().filter(|(listed, _)| *listed) {
        let mut h = Some(r.snapshot);
        while let Some(x) = h.filter(|x| seen.insert(*x)) {
            need.push(x);
            h = bases.get(&x).copied();
        }
    }
    need
}

// ------------------------------------------------------------------------------------------------ snapshot

/// A materialised version: the document as the timeline shows and restores it.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Snapshot {
    /// `exportName` at capture time.
    pub name: String,
    /// Live pages in document order.
    pub pages: Vec<SnapPage>,
}

#[derive(Clone, PartialEq, Debug)]
pub struct SnapPage {
    pub id: Id,
    pub template: Template,
    pub size: (u32, u32),
    pub background: Background,
    pub side_panel: Option<Vec<u8>>,
    /// Live items in draw order (covers, images, strokes; each by `(Z, Id)`).
    pub items: Vec<SnapItem>,
}

#[derive(Clone, PartialEq, Debug)]
pub struct SnapItem {
    pub id: Id,
    pub z: Z,
    pub body: Body,
}

impl Snapshot {
    /// The current visible state of `doc`.
    pub fn of(doc: &Doc) -> Snapshot {
        let pages = doc
            .order()
            .iter()
            .filter_map(|&id| {
                let p = doc.pages.get(&id)?;
                let items = doc
                    .page_items(id)
                    .into_iter()
                    .filter_map(|i| doc.live(i).map(|it| SnapItem { id: i, z: it.z.v, body: it.body.v.clone() }))
                    .collect();
                Some(SnapPage {
                    id,
                    template: p.template.v,
                    size: p.size.v,
                    background: p.background.v,
                    side_panel: p.side_panel.v.clone(),
                    items,
                })
            })
            .collect();
        Snapshot { name: doc.name.v.clone(), pages }
    }

    /// Asset bytes; with a base, bodies equal to the base's for the same id are not repeated.
    pub fn encode(&self, base: Option<(&Hash, &Snapshot)>) -> Vec<u8> {
        let mut w = Writer::default();
        w.u8(ASSET_FORMAT);
        put_opt_hash(&mut w, base.map(|b| b.0));
        put_str(&mut w, &self.name);
        let known = base.map(|b| b.1.bodies()).unwrap_or_default();
        w.leb(self.pages.len() as u64);
        for (i, p) in self.pages.iter().enumerate() {
            let (template, size, background) = (p.template, p.size, p.background);
            put_op(&mut w, &Op::PageNew { page: p.id, pos: Pos::rank(i as u32), template, size, background });
            match &p.side_panel {
                Some(b) => {
                    w.u8(1);
                    put_bytes(&mut w, b);
                }
                None => w.u8(0),
            }
            w.leb(p.items.len() as u64);
            for it in &p.items {
                let tag = match known.get(&it.id) {
                    Some(b) if b.1 == &it.body => if b.0 == it.z { 0 } else { 2 },
                    _ => 1,
                };
                w.u8(tag);
                put_id(&mut w, it.id);
                if tag != 0 {
                    put_z(&mut w, it.z);
                }
                if tag == 1 {
                    put_body(&mut w, &it.body);
                }
            }
        }
        w.buf
    }

    /// 5.x `contentHash` analogue: equal for equal visible content and name.
    pub fn content_hash(&self) -> u64 { fnv1a64(&self.encode(None)) }

    fn bodies(&self) -> IdMap<(Z, &Body)> { self.pages.iter().flat_map(|p| &p.items).map(|i| (i.id, (i.z, &i.body))).collect() }

    fn decode(bytes: &[u8], base: Option<&Snapshot>) -> Option<Snapshot> {
        let r = &mut Reader::new(bytes);
        if r.u8()? != ASSET_FORMAT || get_opt_hash(r)?.is_some() != base.is_some() {
            return None;
        }
        let name = get_str(r)?.to_string();
        let known = base.map(Snapshot::bodies).unwrap_or_default();
        let n = usize::try_from(r.leb()?).ok().filter(|&n| n <= r.buf.len())?;
        let mut pages = Vec::with_capacity(n);
        for _ in 0..n {
            let Op::PageNew { page: id, template, size, background, .. } = get_op(r)? else { return None };
            let side_panel = match r.u8()? {
                0 => None,
                1 => Some(get_bytes(r)?.to_vec()),
                _ => return None,
            };
            let m = usize::try_from(r.leb()?).ok().filter(|&m| m <= r.buf.len())?;
            let mut items = Vec::with_capacity(m);
            for _ in 0..m {
                let (tag, id) = (r.u8()?, get_id(r)?);
                let item = match (tag, known.get(&id)) {
                    (0, Some(b)) => SnapItem { id, z: b.0, body: b.1.clone() },
                    (1, _) => SnapItem { id, z: get_z(r)?, body: get_body(r)? },
                    (2, Some(b)) => SnapItem { id, z: get_z(r)?, body: b.1.clone() },
                    _ => return None,
                };
                items.push(item);
            }
            pages.push(SnapPage { id, template, size, background, side_panel, items });
        }
        r.is_empty().then_some(Snapshot { name, pages })
    }
}

/// The snapshot stored under `hash`, following its delta chain. `None` when an asset of the chain is missing or
/// damaged (fail closed).
pub fn materialize<'a>(hash: &Hash, assets: &dyn Fn(&Hash) -> Option<&'a [u8]>) -> Option<Snapshot> {
    let mut chain = vec![assets(hash)?];
    while let Some(base) = get_opt_hash(&mut Reader::new(chain.last()?.get(1..)?))? {
        if chain.len() >= MAX_CHAIN {
            return None;
        }
        chain.push(assets(&base)?);
    }
    let mut snap: Option<Snapshot> = None;
    for bytes in chain.iter().rev() {
        snap = Some(Snapshot::decode(bytes, snap.as_ref())?);
    }
    snap
}

// ------------------------------------------------------------------------------------------------ capture

/// Who captures: the signed-in profile (5.x `getCurrentVersionAuthor`, name fallback email | `You`) and the device.
pub struct Who<'a> {
    pub email: &'a str,
    pub name: &'a str,
    pub photo: &'a str,
    pub device: &'a str,
}

/// 5.x `pendingVersionHint`: marks the next capture (restore, revert, pre-restore, manual).
#[derive(Clone, Debug, Default)]
pub struct Hint {
    pub milestone: bool,
    pub kind: String,
    pub summary: String,
}

/// A capture waiting for its asset hash.
#[derive(Debug)]
pub struct Planned {
    /// The coalesced entry this one replaces (same id), or `None` for a new entry.
    pub replace: Option<Id>,
    /// `snapshot` is filled by [`Planned::commit`].
    pub record: Record,
    /// Snapshot asset bytes to hash and store.
    pub asset: Vec<u8>,
}

/// v5: app-v5.js:captureVersionSnapshot. `None` = nothing to capture (no pages, or content identical to the newest
/// entry and no milestone). Coalesces into the newest entry when neither is a milestone, it has the same author
/// (email and name) and device, it is less than 5 min old and it belongs to this actor's chain.
pub fn plan_capture<'a>(
    doc: &Doc,
    actor: ActorId,
    who: &Who,
    ts: f64,
    hint: Option<&Hint>,
    assets: &dyn Fn(&Hash) -> Option<&'a [u8]>,
) -> Option<Planned> {
    let snap = Snapshot::of(doc);
    if snap.pages.is_empty() {
        return None;
    }
    let milestone = hint.is_some_and(|h| h.milestone);
    let content = snap.content_hash();
    let list = entries(doc);
    let last = list.last();
    if !milestone && last.is_some_and(|(_, l)| l.content == content) {
        return None;
    }
    let coalesce = !milestone
        && last.is_some_and(|(id, l)| {
            !l.milestone
                && l.email == who.email
                && l.name == who.name
                && !l.device.is_empty()
                && l.device == who.device
                && ts - l.ts < COALESCE_MS
                && actor_of(*id) == actor.0
        });
    let prior = match (coalesce, last) {
        (true, Some((_, l))) => l.base.map(|h| (h, l.depth)),
        _ => chain_tail(doc, actor).map(|t| (t.snapshot, t.depth.saturating_add(1))),
    };
    let base = prior.filter(|p| p.1 < CHECKPOINT_EVERY).and_then(|(h, d)| Some((h, d, materialize(&h, assets)?)));
    // The newest entry's snapshot (for the summary) is usually the base just materialised.
    let reuse = |l: &Record| base.as_ref().filter(|b| b.0 == l.snapshot).map(|b| &b.2);
    let other = last.filter(|(_, l)| reuse(l).is_none()).and_then(|(_, l)| materialize(&l.snapshot, assets));
    let prev = last.map(|(_, l)| reuse(l).or(other.as_ref()));
    let summary = match hint.filter(|h| !h.summary.is_empty()) {
        Some(h) => h.summary.clone(),
        None => match prev {
            None => summarize(&Snapshot::default(), &snap),
            Some(Some(p)) => summarize(p, &snap),
            Some(None) => "Saved".to_string(),
        },
    };
    let kind = hint.map(|h| h.kind.as_str()).filter(|k| !k.is_empty()).unwrap_or("autosave").to_string();
    let record = Record {
        ts,
        email: who.email.to_string(),
        name: who.name.to_string(),
        photo: who.photo.to_string(),
        device: who.device.to_string(),
        milestone: milestone || MILESTONE_KINDS.contains(&kind.as_str()),
        kind,
        summary,
        content,
        snapshot: [0; 32],
        base: base.as_ref().map(|b| b.0),
        depth: base.as_ref().map_or(0, |b| b.1),
    };
    let asset = snap.encode(base.as_ref().map(|(h, _, s)| (h, s)));
    Some(Planned { replace: coalesce.then(|| last.map(|l| l.0)).flatten(), record, asset })
}

impl Planned {
    /// Adds the entry (`VersionAdd`; a coalesced one rewrites the replaced id's register) and the prune deletions
    /// to `tx`. `snapshot` = SHA-256 of `self.asset`, which the caller has stored. Returns the version id.
    pub fn commit(mut self, snapshot: Hash, doc: &Doc, tx: &mut Tx) -> Id {
        self.record.snapshot = snapshot;
        let data = self.record.encode();
        let id = match self.replace {
            Some(id) => {
                tx.push(Op::VersionAdd { meta: VersionMeta { id, data } });
                id
            }
            None => tx.version_add(data),
        };
        let mut list = entries(doc);
        list.retain(|e| e.0 != id);
        list.push((id, self.record));
        for dropped in plan_prune(&list) {
            tx.version_del(dropped);
        }
        id
    }
}

/// v5: app-v5.js:pruneTimelineHistory. Returns the ids to delete: non-milestone duplicates of the same author and
/// content (the newest stays), milestones beyond the newest 30 (the first `baseline` stays), then the oldest
/// non-milestones beyond 80 entries. (5.x branch-id regrouping does not apply: v6 ids never collide.)
pub fn plan_prune(list: &[(Id, Record)]) -> Vec<Id> {
    let newer = |a: &(Id, Record), b: &(Id, Record)| a.1.ts.total_cmp(&b.1.ts).then(a.0.cmp(&b.0)).is_gt();
    let mut winners: HashMap<(&str, u64), usize> = HashMap::new();
    for (i, e) in list.iter().enumerate().filter(|(_, e)| !e.1.milestone) {
        let who = if e.1.email.is_empty() { &e.1.name } else { &e.1.email };
        let w = winners.entry((who.as_str(), e.1.content)).or_insert(i);
        if newer(e, &list[*w]) {
            *w = i;
        }
    }
    let mut keep: Vec<&(Id, Record)> = list.iter().filter(|e| e.1.milestone).chain(winners.values().map(|&i| &list[i])).collect();
    keep.sort_by(|a, b| a.1.ts.total_cmp(&b.1.ts).then(a.0.cmp(&b.0)));
    let milestones: Vec<Id> = keep.iter().filter(|e| e.1.milestone).map(|e| e.0).collect();
    if milestones.len() > MAX_MILESTONES {
        let mut protect: HashSet<Id> = milestones[milestones.len() - MAX_MILESTONES..].iter().copied().collect();
        protect.extend(keep.iter().find(|e| e.1.milestone && e.1.kind == "baseline").map(|e| e.0));
        keep.retain(|e| !e.1.milestone || protect.contains(&e.0));
    }
    while keep.len() > MAX_ENTRIES {
        let at = keep.iter().position(|e| !e.1.milestone).unwrap_or(0);
        keep.remove(at);
    }
    let kept: HashSet<Id> = keep.iter().map(|e| e.0).collect();
    list.iter().map(|e| e.0).filter(|id| !kept.contains(id)).collect()
}

// ------------------------------------------------------------------------------------------------ restore

/// Ops that make the visible document equal `target` (v5: app-v5.js:applyRestoredPages + addRestoreTombstones):
/// live pages absent from the target are deleted, target pages are revived/created with their properties and
/// order, items of target pages absent from the target are erased, and target items whose page, z or body differ
/// (or that are not live) are put back — each undelete preceded by its put. The name is restored when the target
/// has one (5.18.0 `if (entry.exportName)`). Push the ops into one `Tx`: the restore is a new causal write.
pub fn plan_restore(doc: &Doc, target: &Snapshot) -> Vec<Op> {
    let mut ops = Vec::new();
    let wanted: HashSet<Id> = target.pages.iter().map(|p| p.id).collect();
    // Items move between pages: one erased from its current page must not be in the target anywhere.
    let keep: HashSet<Id> = target.pages.iter().flat_map(|p| &p.items).map(|i| i.id).collect();
    for &page in doc.order() {
        if !wanted.contains(&page) && doc.pages.get(&page).is_some_and(|p| !p.deleted.v) {
            ops.push(Op::PageDel { page, deleted: true });
        }
    }
    let born = |id: &Id| doc.pages.get(id).filter(|p| p.is_born());
    // Re-key the order only when the existing keys of the target pages would not sort them as the target does.
    let rekey = target.pages.iter().any(|p| born(&p.id).is_none())
        || target.pages.windows(2).any(|w| {
            let key = |id: &Id| born(id).map(|p| (p.pos.v.clone(), *id));
            key(&w[0].id) >= key(&w[1].id)
        });
    for (i, t) in target.pages.iter().enumerate() {
        let page = t.id;
        match born(&page) {
            None => {
                let (template, size, background) = (t.template, t.size, t.background);
                ops.push(Op::PageNew { page, pos: Pos::rank(i as u32), template, size, background });
                if t.side_panel.is_some() {
                    ops.push(Op::PageSet { page, prop: PageProp::SidePanel(t.side_panel.clone()) });
                }
            }
            Some(p) => {
                if p.deleted.v {
                    ops.push(Op::PageDel { page, deleted: false });
                }
                if rekey && p.pos.v != Pos::rank(i as u32) {
                    ops.push(Op::PageMove { page, pos: Pos::rank(i as u32) });
                }
                let props = [
                    (p.template.v != t.template, PageProp::Template(t.template)),
                    (p.size.v != t.size, PageProp::Size(t.size)),
                    (p.background.v != t.background, PageProp::Background(t.background)),
                    (p.side_panel.v != t.side_panel, PageProp::SidePanel(t.side_panel.clone())),
                ];
                ops.extend(props.into_iter().filter(|c| c.0).map(|(_, prop)| Op::PageSet { page, prop }));
            }
        }
        ops.extend(doc.page_items(page).into_iter().filter(|i| !keep.contains(i)).map(|item| Op::ItemDel { item, deleted: true }));
        for it in &t.items {
            let cur = doc.items.get(&it.id);
            let live = cur.is_some_and(|c| c.is_live());
            if live && cur.is_some_and(|c| c.page.v == page && c.z.v == it.z && c.body.v == it.body) {
                continue;
            }
            ops.push(Op::ItemPut { item: it.id, page, z: it.z, body: it.body.clone() });
            if cur.is_some_and(|c| c.deleted.v) {
                ops.push(Op::ItemDel { item: it.id, deleted: false });
            }
        }
    }
    if !target.name.is_empty() && doc.name.v != target.name {
        ops.push(Op::DocName { name: target.name.clone() });
    }
    ops
}

// ------------------------------------------------------------------------------------------------ summary

fn strokes(p: &SnapPage) -> impl Iterator<Item = &SnapItem> {
    p.items.iter().filter(|i| matches!(i.body, Body::Stroke { tool, .. } if tool != Tool::LegacyCover))
}
fn images(p: &SnapPage) -> impl Iterator<Item = &SnapItem> { p.items.iter().filter(|i| matches!(i.body, Body::Image { .. })) }

fn plural(n: usize, sign: &str, word: &str) -> String { format!("{sign}{n} {word}{}", if n == 1 { "" } else { "s" }) }

/// v5: app-v5.js:summarizeVersionDelta (`'+2 pages, -1 stroke, 1 image change'`, nothing → `'Saved'`).
pub fn summarize(prev: &Snapshot, next: &Snapshot) -> String {
    let before: IdMap<&SnapPage> = prev.pages.iter().map(|p| (p.id, p)).collect();
    let after: IdMap<&SnapPage> = next.pages.iter().map(|p| (p.id, p)).collect();
    let added = after.keys().filter(|k| !before.contains_key(k)).count();
    let removed = before.keys().filter(|k| !after.contains_key(k)).count();
    let (mut s_add, mut s_del, mut img, mut layout) = (0, 0, 0, 0);
    for p in &next.pages {
        let ns = strokes(p).count();
        let Some(b) = before.get(&p.id) else {
            s_add += ns;
            img += images(p).count();
            continue;
        };
        let ps = strokes(b).count();
        s_add += ns.saturating_sub(ps);
        s_del += ps.saturating_sub(ns);
        if !images(b).map(|i| (i.id, &i.body)).eq(images(p).map(|i| (i.id, &i.body))) {
            img += 1;
        }
        let kind = |bg: &Background| std::mem::discriminant(bg);
        if (b.size, kind(&b.background), b.template, &b.side_panel) != (p.size, kind(&p.background), p.template, &p.side_panel) {
            layout += 1;
        }
    }
    s_del += prev.pages.iter().filter(|p| !after.contains_key(&p.id)).map(|p| strokes(p).count()).sum::<usize>();
    let mut parts = Vec::new();
    if added > 0 {
        parts.push(plural(added, "+", "page"));
    }
    if removed > 0 {
        parts.push(plural(removed, "-", "page"));
    }
    if added == 0 && removed == 0 && !prev.pages.iter().map(|p| p.id).eq(next.pages.iter().map(|p| p.id)) {
        parts.push("Pages reordered".to_string());
    }
    if s_add > 0 {
        parts.push(plural(s_add, "+", "stroke"));
    }
    if s_del > 0 {
        parts.push(plural(s_del, "-", "stroke"));
    }
    if img > 0 {
        parts.push(plural(img, "", "image change"));
    }
    if layout > 0 {
        parts.push(format!("{} changed", plural(layout, "", "page setting")));
    }
    if parts.is_empty() { "Saved".to_string() } else { parts.join(", ") }
}

// ------------------------------------------------------------------------------------------------ diff

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mark {
    Added,
    Changed,
    Removed,
}

/// A preview highlight in page coordinates (drawn by `drawTimelineDiffHighlights`).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Highlight {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub mark: Mark,
}

fn jmin(a: f64, b: f64) -> f64 { if a.is_nan() || b.is_nan() { f64::NAN } else { a.min(b) } }
fn jmax(a: f64, b: f64) -> f64 { if a.is_nan() || b.is_nan() { f64::NAN } else { a.max(b) } }
/// `Number(v) || 0`.
fn or0(v: f64) -> f64 { if v.is_nan() { 0.0 } else { v } }
/// A JSON-stringified `Math.round(v * k) / k`: -0 prints as 0, ±Infinity as null.
fn sig(v: f64, k: f64) -> f64 {
    let r = jsmath::round(or0(v) * k) / k;
    if r.is_finite() { r + 0.0 } else { f64::INFINITY }
}

/// v5: app-v5.js:getTimelineItemSignature (equality of the signatures).
fn same_signature(a: &Body, b: &Body) -> bool {
    match (a, b) {
        (Body::Stroke { tool: t1, color: c1, width: w1, xy: p1, .. }, Body::Stroke { tool: t2, color: c2, width: w2, xy: p2, .. }) => {
            t1 == t2
                && c1 == c2
                && sig(*w1, 10.0) == sig(*w2, 10.0)
                && p1.len() / 2 == p2.len() / 2
                && p1.iter().zip(p2).all(|(x, y)| sig(*x, 10.0) == sig(*y, 10.0))
        }
        (Body::Image { asset: a1, cx: x1, cy: y1, w: w1, h: h1, rotation: r1 }, Body::Image { asset: a2, cx: x2, cy: y2, w: w2, h: h2, rotation: r2 }) => {
            a1 == a2
                && [(x1, x2), (y1, y2), (w1, w2), (h1, h2)].iter().all(|(u, v)| sig(**u, 10.0) == sig(**v, 10.0))
                && sig(*r1, 100.0) == sig(*r2, 100.0)
        }
        _ => false,
    }
}

/// v5: app-v5.js:getTimelineStrokeBounds / getTimelineImageBounds → `[x, y, w, h]`.
fn bounds(body: &Body) -> Option<[f64; 4]> {
    match body {
        Body::Stroke { width, xy, .. } => {
            let (mut x0, mut y0, mut x1, mut y1) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
            for p in xy.chunks_exact(2).filter(|p| p[0].is_finite() && p[1].is_finite()) {
                (x0, y0, x1, y1) = (x0.min(p[0]), y0.min(p[1]), x1.max(p[0]), y1.max(p[1]));
            }
            if !x0.is_finite() {
                return None;
            }
            let w = if *width == 0.0 || width.is_nan() { 2.0 } else { *width };
            let pad = jmax(10.0, w * 1.4);
            Some([x0 - pad, y0 - pad, (x1 - x0) + pad * 2.0, (y1 - y0) + pad * 2.0])
        }
        Body::Image { cx, cy, w, h, rotation, .. } => {
            if ![cx, cy, w, h].iter().all(|v| v.is_finite()) || *w <= 0.0 || *h <= 0.0 {
                return None;
            }
            let a = or0(*rotation);
            let (cos, sin) = (jsmath::cos(a).abs(), jsmath::sin(a).abs());
            let (bw, bh) = (w * cos + h * sin, w * sin + h * cos);
            Some([cx - bw / 2.0 - 8.0, cy - bh / 2.0 - 8.0, bw + 16.0, bh + 16.0])
        }
        Body::Dropped => None,
    }
}

/// v5: app-v5.js:clampTimelineBounds.
fn clamp(b: [f64; 4], pw: f64, ph: f64, mark: Mark) -> Option<Highlight> {
    let x1 = jmax(0.0, jmin(pw, b[0]));
    let y1 = jmax(0.0, jmin(ph, b[1]));
    let x2 = jmax(0.0, jmin(pw, b[0] + b[2]));
    let y2 = jmax(0.0, jmin(ph, b[1] + b[3]));
    if x2 <= x1 || y2 <= y1 {
        return None;
    }
    Some(Highlight { x: x1, y: y1, w: x2 - x1, h: y2 - y1, mark })
}

/// v5: app-v5.js:collectTimelineDiffHighlights — strokes then images; within each, the next page's added/changed
/// items in order, then the previous page's removed items in order. `prev = None`: every item is added.
pub fn diff_page(prev: Option<&SnapPage>, next: &SnapPage) -> Vec<Highlight> {
    let (pw, ph) = (next.size.0 as f64, next.size.1 as f64);
    let mut out = Vec::new();
    let kinds: [fn(&SnapPage) -> Vec<&SnapItem>; 2] = [|p| strokes(p).collect(), |p| images(p).collect()];
    for kind in kinds {
        let before = prev.map(kind).unwrap_or_default();
        let mut gone: IdMap<&Body> = before.iter().map(|i| (i.id, &i.body)).collect();
        for it in kind(next) {
            let mark = match gone.remove(&it.id) {
                None => Mark::Added,
                Some(b) if !same_signature(b, &it.body) => Mark::Changed,
                Some(_) => continue,
            };
            out.extend(bounds(&it.body).and_then(|b| clamp(b, pw, ph, mark)));
        }
        for it in before.iter().filter(|i| gone.contains_key(&i.id)) {
            out.extend(bounds(&it.body).and_then(|b| clamp(b, pw, ph, Mark::Removed)));
        }
    }
    out
}

/// v5: app-v5.js:renderTimelinePreview diff step: page `i` of `next` against page `i` of the previous entry (by
/// index, as 5.18.0); no previous entry → no highlights.
pub fn diff(prev: Option<&Snapshot>, next: &Snapshot) -> Vec<Vec<Highlight>> {
    next.pages.iter().enumerate().map(|(i, p)| prev.map_or_else(Vec::new, |s| diff_page(s.pages.get(i), p))).collect()
}

/// Test command hook: `__IHN_TEST_API__.cmd('core.history.<name>', args)`. `constants` → the 5.18.0 limits.
pub fn test_cmd(name: &str, _args: &str) -> Option<String> {
    match name {
        "constants" => Some(format!(
            "{{\"max\":{MAX_ENTRIES},\"milestones\":{MAX_MILESTONES},\"coalesceMs\":{COALESCE_MS},\"checkpoint\":{CHECKPOINT_EVERY}}}"
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::Author;
    use crate::id::{fnv1a128, IdGen};
    use crate::op::{Batch, Color, ImageRect};
    use crate::testutil::Rng;

    const A4: (u32, u32) = (595, 842);
    const MIN: f64 = 60_000.0;
    type Store = HashMap<Hash, Vec<u8>>;

    fn sha(bytes: &[u8]) -> Hash {
        let mut h = [0u8; 32];
        h[..16].copy_from_slice(&fnv1a128(bytes).to_le_bytes());
        h[16..24].copy_from_slice(&fnv1a64(bytes).to_le_bytes());
        h
    }
    fn author(n: u64) -> Author { Author::new(IdGen::new(ActorId::tab(0x1000_0000 * n, n as u8), 0), 0) }
    fn who<'a>(name: &'a str, device: &'a str) -> Who<'a> { Who { email: "a@x", name, photo: "", device } }
    fn pen(rng: &mut Rng) -> Body {
        let xy = (0..2 + rng.below(6) * 2).map(|_| rng.range(0.0, 600.0)).collect();
        Body::Stroke { tool: Tool::Pen, color: Color::Hex(0x002fd9), width: 3.0, xy, p: None }
    }
    fn edit(doc: &mut Doc, au: &mut Author, f: impl FnOnce(&mut Tx)) {
        let mut tx = Tx::new(doc, au, 1);
        f(&mut tx);
        if !tx.is_empty() {
            let b = tx.commit();
            doc.apply(&b);
        }
    }
    fn capture(doc: &mut Doc, au: &mut Author, store: &mut Store, w: &Who, ts: f64, hint: Option<&Hint>) -> Option<Id> {
        let p = plan_capture(doc, au.actor, w, ts, hint, &|h| store.get(h).map(Vec::as_slice))?;
        let h = sha(&p.asset);
        store.insert(h, p.asset.clone());
        let mut tx = Tx::new(doc, au, 1);
        let id = p.commit(h, doc, &mut tx);
        let b = tx.commit();
        doc.apply(&b);
        Some(id)
    }
    fn record(doc: &Doc, id: Id) -> Record { Record::decode(&doc.versions[&id].data.v).unwrap() }

    #[test]
    fn capture_skips_coalesces_and_keeps_milestones() {
        let (mut doc, mut a, mut st, mut rng) = (Doc::default(), author(1), Store::new(), Rng::new(1));
        edit(&mut doc, &mut a, |tx| {
            let p = tx.page_new(None, Template::Default, A4, Background::Template);
            tx.add_stroke(p, pen(&mut rng));
        });
        let page = doc.order()[0];
        let d1 = who("Ann", "d1");
        let v1 = capture(&mut doc, &mut a, &mut st, &d1, 0.0, None).unwrap();
        let r = record(&doc, v1);
        assert_eq!((r.summary.as_str(), r.kind.as_str(), r.milestone, r.base, r.depth), ("+1 page, +1 stroke", "autosave", false, None, 0));
        assert!(capture(&mut doc, &mut a, &mut st, &d1, MIN, None).is_none(), "identical content is skipped");
        let mut add = |doc: &mut Doc, a: &mut Author| edit(doc, a, |tx| { tx.add_stroke(page, pen(&mut rng)); });
        add(&mut doc, &mut a);
        assert_eq!(capture(&mut doc, &mut a, &mut st, &d1, 4.0 * MIN, None), Some(v1), "coalesced within 5 min");
        // 5.18.0 quirk: the summary is computed against the replaced entry.
        assert_eq!((entries(&doc).len(), record(&doc, v1).summary.as_str()), (1, "+1 stroke"));
        add(&mut doc, &mut a);
        let v2 = capture(&mut doc, &mut a, &mut st, &d1, 9.0 * MIN, None).unwrap();
        assert_ne!(v2, v1, "5 min after the last entry: a new one");
        add(&mut doc, &mut a);
        let v3 = capture(&mut doc, &mut a, &mut st, &who("Ann", "d2"), 9.5 * MIN, None).unwrap();
        add(&mut doc, &mut a);
        let v4 = capture(&mut doc, &mut a, &mut st, &who("Bob", "d2"), 9.6 * MIN, None).unwrap();
        let hint = Hint { milestone: true, kind: "manual".into(), summary: "Pinned".into() };
        let v5 = capture(&mut doc, &mut a, &mut st, &who("Bob", "d2"), 9.7 * MIN, Some(&hint)).unwrap();
        add(&mut doc, &mut a);
        let v6 = capture(&mut doc, &mut a, &mut st, &who("Bob", "d2"), 9.8 * MIN, None).unwrap();
        let ids: Vec<Id> = entries(&doc).iter().map(|e| e.0).collect();
        assert_eq!(ids, [v1, v2, v3, v4, v5, v6]);
        let r5 = record(&doc, v5);
        assert_eq!((r5.milestone, r5.kind.as_str(), r5.summary.as_str()), (true, "manual", "Pinned"));
        // Another actor with the same profile and device never rewrites this actor's entry.
        let mut b = Author::resume(IdGen::new(ActorId::tab(0x7000_0000, 7), 0), 0, &doc, 1);
        edit(&mut doc, &mut b, |tx| { tx.add_stroke(page, pen(&mut rng)); });
        let v7 = capture(&mut doc, &mut b, &mut st, &who("Bob", "d2"), 9.9 * MIN, None).unwrap();
        assert!(v7 != v6 && record(&doc, v7).depth == 0 && record(&doc, v7).base.is_none());
        // An empty document has nothing to capture.
        assert!(plan_capture(&Doc::default(), a.actor, &d1, 0.0, None, &|_| None).is_none());
    }

    #[test]
    fn chains_checkpoint_every_8_and_survive_pruning() {
        let (mut doc, mut a, mut b, mut st, mut rng) = (Doc::default(), author(1), author(2), Store::new(), Rng::new(7));
        edit(&mut doc, &mut a, |tx| { tx.page_new(None, Template::Default, A4, Background::Template); });
        let mut seen: Vec<(Id, Snapshot)> = Vec::new();
        let (mut own_a, mut own_b) = (0u32, 0u32);
        for i in 0..130 {
            let use_b = i % 10 == 9;
            let au = if use_b { &mut b } else { &mut a };
            let pages = doc.order().to_vec();
            let items: Vec<Id> = pages.iter().flat_map(|p| doc.page_items(*p)).collect();
            edit(&mut doc, au, |tx| {
                let p = *rng.pick(&pages);
                for _ in 0..1 + rng.below(30) {
                    tx.add_stroke(p, pen(&mut rng));
                }
                if !items.is_empty() && rng.chance(0.3) {
                    tx.erase(&[*rng.pick(&items)]);
                }
                if rng.chance(0.1) {
                    tx.page_new(Some(p), Template::Default, A4, Background::Template);
                }
            });
            let dev = if use_b { "dB" } else { "dA" };
            let id = capture(&mut doc, au, &mut st, &who("Ann", dev), i as f64 * 6.0 * MIN, None).unwrap();
            let n = if use_b { &mut own_b } else { &mut own_a };
            let r = record(&doc, id);
            assert_eq!((r.depth as u32, r.base.is_none()), (*n % 8, *n % 8 == 0), "capture {i}");
            *n += 1;
            seen.push((id, Snapshot::of(&doc)));
        }
        let list = entries(&doc);
        assert_eq!(list.len(), MAX_ENTRIES);
        let need: HashSet<Hash> = needed_assets(&doc).into_iter().collect();
        assert!(need.len() > list.len(), "pruned bases are still needed");
        let kept: Store = st.iter().filter(|(h, _)| need.contains(*h)).map(|(h, v)| (*h, v.clone())).collect();
        for (id, r) in &list {
            let want = &seen.iter().find(|s| s.0 == *id).unwrap().1;
            assert_eq!(materialize(&r.snapshot, &|h| kept.get(h).map(Vec::as_slice)).as_ref(), Some(want));
        }
        // Deltas carry only new bodies.
        let r = record(&doc, list.last().unwrap().0);
        let full = seen.last().unwrap().1.encode(None).len();
        assert!(r.base.is_some() && st[&r.snapshot].len() * 4 < full, "delta {} vs full {full}", st[&r.snapshot].len());
        // A missing chain asset fails closed.
        let base = r.base.unwrap();
        assert!(materialize(&r.snapshot, &|h| (h != &base).then(|| st.get(h).map(Vec::as_slice)).flatten()).is_none());
    }

    #[test]
    fn prune_dedupes_and_caps_like_5_18_0() {
        let rec = |ts: f64, kind: &str, content: u64| Record {
            ts,
            kind: kind.into(),
            milestone: kind != "autosave",
            content,
            email: "a@x".into(),
            ..Record::default()
        };
        let mut list = vec![(Id(0), rec(0.0, "baseline", 0))];
        list.extend((1..=40).map(|i| (Id(i), rec(i as f64, "manual", i as u64))));
        list.extend((41..=100).map(|i| (Id(i), rec(i as f64, "autosave", i as u64))));
        list.push((Id(101), rec(101.0, "autosave", 100)));
        let mut dropped = plan_prune(&list);
        dropped.sort();
        let want: Vec<Id> = (1..=10).chain(41..=51).chain([100]).map(Id).collect();
        assert_eq!(dropped, want);
        assert!(plan_prune(&list[..30]).is_empty());
    }

    #[test]
    fn restore_reaches_the_target_on_every_replica() {
        let (mut doc, mut a, mut st, mut rng) = (Doc::default(), author(3), Store::new(), Rng::new(11));
        let mut other = Doc::default();
        let mut log: Vec<Batch> = Vec::new();
        let run = |doc: &mut Doc, a: &mut Author, log: &mut Vec<Batch>, ops: Vec<Op>| {
            let mut tx = Tx::new(doc, a, 1);
            ops.into_iter().for_each(|o| tx.push(o));
            if !tx.is_empty() {
                let b = tx.commit();
                doc.apply(&b);
                log.push(b);
            }
        };
        edit(&mut doc, &mut a, |tx| { tx.page_new(None, Template::Default, A4, Background::Template); });
        let mut ts = 0.0;
        for step in 0..400 {
            let pages = doc.order().to_vec();
            let items: Vec<Id> = pages.iter().flat_map(|p| doc.page_items(*p)).collect();
            let mut tx = Tx::new(&doc, &mut a, 1);
            let p = *rng.pick(&pages);
            match rng.below(10) {
                0..=2 => { tx.add_stroke(p, pen(&mut rng)); }
                3 if !items.is_empty() => tx.erase(&[*rng.pick(&items)]),
                4 if !items.is_empty() => tx.recolor(&[*rng.pick(&items)], Color::Hex(rng.below(1 << 24) as u32)),
                5 if !items.is_empty() => tx.move_to_page(&[*rng.pick(&items)], *rng.pick(&pages), 5.0),
                6 => { tx.page_new(Some(p), Template::Default, A4, Background::Template); }
                7 if pages.len() > 1 => tx.page_delete(p),
                8 => tx.page_move(p, Some(pages[0])),
                _ => {
                    tx.set_name(&format!("Doc {}", rng.below(5)));
                    tx.page_set(p, PageProp::Size((500 + rng.below(3) as u32, 700)));
                    tx.add_image(p, [rng.below(3) as u8; 32], ImageRect { cx: 100.0, cy: 90.0, w: 50.0, h: 40.0, rotation: 0.5 });
                }
            }
            if !tx.is_empty() {
                let b = tx.commit();
                doc.apply(&b);
                log.push(b);
            }
            if step % 7 == 0 {
                ts += 6.0 * MIN;
                capture(&mut doc, &mut a, &mut st, &who("Ann", "d"), ts, None);
            }
            if step % 37 == 36 {
                let list = entries(&doc);
                let (_, r) = rng.pick(&list).clone();
                let target = materialize(&r.snapshot, &|h| st.get(h).map(Vec::as_slice)).unwrap();
                let ops = plan_restore(&doc, &target);
                run(&mut doc, &mut a, &mut log, ops);
                let now = Snapshot::of(&doc);
                assert_eq!(now.pages, target.pages, "step {step}");
                assert_eq!(now.name, if target.name.is_empty() { now.name.clone() } else { target.name.clone() });
                assert!(plan_restore(&doc, &target).is_empty(), "a reached target needs no ops");
            }
        }
        for b in log.iter().rev() {
            other.apply(b);
        }
        assert_eq!(Snapshot::of(&other), Snapshot::of(&doc));
    }

    #[test]
    fn records_and_assets_fail_closed() {
        let r = Record { ts: 5.5, email: "e".into(), name: "n".into(), kind: "restore".into(), milestone: true, base: Some([3; 32]), depth: 4, ..Record::default() };
        let bytes = r.encode();
        assert_eq!(Record::decode(&bytes), Some(r));
        for n in 0..bytes.len() {
            assert!(Record::decode(&bytes[..n]).is_none());
        }
        let (mut doc, mut a, mut rng) = (Doc::default(), author(4), Rng::new(3));
        edit(&mut doc, &mut a, |tx| {
            let p = tx.page_new(None, Template::Agenda, A4, Background::Template);
            tx.page_set(p, PageProp::SidePanel(Some(vec![1, 2])));
            tx.add_stroke(p, pen(&mut rng));
        });
        let s = Snapshot::of(&doc);
        let full = s.encode(None);
        let h = sha(&full);
        assert_eq!(materialize(&h, &|x| (x == &h).then_some(&full[..])), Some(s.clone()));
        for n in 0..full.len() {
            let cut = &full[..n];
            assert!(materialize(&h, &|_| Some(cut)).is_none());
        }
        // A delta whose base is another snapshot is rejected.
        let delta = s.encode(Some((&h, &s)));
        let other = Snapshot::default().encode(None);
        assert!(materialize(&[9; 32], &|x| Some(if x == &h { &other[..] } else { &delta[..] })).is_none());
        assert_eq!(test_cmd("constants", ""), Some("{\"max\":80,\"milestones\":30,\"coalesceMs\":300000,\"checkpoint\":8}".into()));
    }

    /// Page pairs and outputs recorded from 5.18.0 `collectTimelineDiffHighlights` and `summarizeVersionDelta`
    /// (scratchpad generator `w29tools/gen-diff.mjs`, seeded).
    const FIXTURE: &str = r"P 612 842
A s s0 pen #5e45fd 3.04 422.034 118.882 440.946 640.266 499.85 493.5 102.085 761.602 35.71 632.826
A s s1 pen #002fd9 3.04 77.45 252.441 303.058 390.132 113.205 763.562 169.351 738.846 35.525 44.755
A s s2 highlighter #111111 3.06 252.75 191.7 413.45 223.35 244.1 699.041 326.004 524.248 -25.852 372.787
A s s3 pen #740686 2 374.5 548.634 231.328 590.7 499.057 413.4 346.891 753.391 431.25 819.45
A s s4 highlighter #111111 3.04 238.1 -31.766 441.7 690.713 362.25 242.7 132.847 555.242 416.958 563.272
A i i5 b 213.327 206.307 40.028 211.676 2.609
B s s0 pen #5e45fd 3.04 422.034 118.882 440.946 640.266 499.85 493.5 102.085 761.602 35.71 632.826
B s s1 pen #002fd9 3.04 77.45 252.441 303.058 390.132 113.205 763.562 169.351 738.846 35.525 44.755
B s s3 pen #740686 2 374.5 548.634 231.328 590.7 499.057 413.4 346.891 753.391 431.25 819.45
B s s4 highlighter #111111 3.04 238.16 -31.766 441.7 690.713 362.25 242.7 132.847 555.242 416.958 563.272
B i i5 b 213.331 206.307 40.028 211.676 2.609
B i i6 b 426.234 297.75 0 33.119 0
E changed 122.84700000000001 0 328.85299999999995 700.713
E removed 0 181.7 423.44999999999993 527.3410000000001
S -1 stroke, 1 image change
P 595 792
A s s7 highlighter #002fd9 0 457.862 713.45 316.572 443.565
A s s8 highlighter #66db5f 3.04 274.864 602.59 352.357 359.12 361.535 818.341
A s s9 highlighter #b7b7f3 18 597.418 -10.442 217.1 786.25
A s s10 highlighter #002fd9 18 168.952 510.566 340.65 797.501 -22.905 225.073 456.034 443.529 97.3 166.513
A s s11 pen #4091b0 3.06 95.212 717.341 113.35 38.3 194.513 415.05 181.499 793.15
A s s12 pen #111111 3.04 510.33 548.334
A s s13 pen #ab8252 12.793 634.105 695.5
A s s14 highlighter #9620ff 18 284.246 446.472 436.898 224.45 -6.8 745.079 181.65 53.972
A i i15 b 401.85 667.047 201.837 141.95 0
A i i16 b 242.111 45.915 134.462 237.419 1.004
A i i17 a 15.514 347.569 0 30.083 0
B s s7 highlighter #002fd9 0 457.872 713.45 316.572 443.565
B s s10 highlighter #002fd9 18 168.952 510.566 340.65 797.501 -22.905 225.073 456.034 443.529 97.3 166.513 52.393 458.1
B s s11 pen #4091b0 3.06 95.212 717.341 113.35 38.3 194.513 415.05 181.499 793.15
B s s12 pen #111111 3.04 510.33 548.334
B s s13 pen #ab8252 12.793 634.115 695.5
B i i15 b 401.85 667.047 201.837 141.95 0
B i i16 b 242.111 45.915 134.462 237.419 1.004
B i i17 a 15.514 347.569 0 30.083 0
E changed 0 141.31300000000002 481.2339999999999 650.687
E removed 264.864 349.12 106.67100000000005 442.88
E removed 191.9 0 403.1 792
E removed 0 28.772000000000002 462.098 741.507
S -3 strokes
P 595 400
A i i18 a 229.487 358.65 0 158.914 0
B s s20 pen #002fd9 0 518.844 803.504 614.25 507.741 158.639 83.377
B s s19 highlighter #111111 3.04 164.4 290.072 292.687 395.687 324.05 477.601 474.684 293.397 57.197 250.654
B s s21 highlighter #79e5e1 20.332 570.6 842.361 111.35 149.58
B i i18 b 229.487 358.65 0 158.914 0
E added 148.639 73.377 446.361 326.623
E added 47.197 240.654 437.487 159.346
E added 82.8852 121.11520000000002 512.1148000000001 278.8848
S +3 strokes, 1 image change
P 595 400
A i i22 a 265.85 805.972 0 223.684 0
A i i23 a 226.027 91.677 143.318 162.5 1.004
B i i23 a 226.027 91.677 143.322 162.5 1.004
B i i24 b 350.339 351.474 0 134.732 2.348
S 1 image change
P 612 842
A s s25 pen #002fd9 0 336.35 254.564 535.35 126.797 506.033 542.343
A s s26 highlighter #002fd9 2 242.659 522.8 545.55 790.664 540.5 -21.021
A s s27 pen #002fd9 18 569.932 611.6 150.038 508.244
A s s28 highlighter #e54767 3.04 40.474 721.727 445.8 243.511
A s s29 highlighter #a29d2d 3.04 632.519 810.8 612.45 567.413 560.669 286.6
A i i30 b 375.844 359.903 270.072 53.254 1.095
A i i31 a 404.778 769.95 0 85.446 0
B s s25 pen #002fd9 0 336.35 254.564 535.35 126.797 506.033 542.343
B s s26 highlighter #35befd 2 242.659 522.8 545.55 790.664 540.5 -21.021
B s s27 pen #002fd9 18 569.932 611.6 150.038 508.244
B s s28 highlighter #e54767 3.04 40.474 721.727 445.8 243.511
B i i30 b 375.844 359.903 270.072 53.254 1.095
B i i31 a 404.778 769.95 0 85.446 0
E changed 232.659 0 322.89099999999996 800.664
E removed 550.669 276.6 61.33100000000002 544.1999999999999
S -1 stroke
P 300 400
A s s32 highlighter #111111 23.391 193.072 289.448 531.3 809.4
A s s33 highlighter #111111 27.492 212.8 37.5 51.797 231.564 210.055 214.713 363.875 159.594 286.8 -15.758
A s s34 pen #111111 18 318.759 588.584 566.2 -38.171 294.75 462.35 36.116 213.655
A s s35 highlighter #002fd9 3.06 111.989 196.663
A s s36 pen #002fd9 9.05 174.4 231.524 560.85 511.859 324.634 54.959
A s s37 highlighter #2bd425 3.04 178.013 15.316
A s s38 highlighter #a76ae5 0 -22.41 427.256 454.55 4.894 532.65 481.889
A i i39 a 8.45 396.9 0 24.3 1.004
A i i40 b 298.389 208.8 0 37.714 0
B s s33 highlighter #111111 27.492 212.8 37.5 51.797 231.564 210.055 214.713 363.875 159.594 286.8 -15.758 237.35 211.751
B s s34 pen #111111 18 318.759 588.584 566.2 -38.171 294.75 462.35 36.116 213.655
B s s35 highlighter #002fd9 3.06 111.989 196.663 198.9 137.35
B s s36 pen #0bdca5 9.05 174.4 231.524 560.85 511.859 324.634 54.959
B s s37 highlighter #2bd425 3.04 178.013 15.316
B s s38 highlighter #a76ae5 0 -22.41 427.256 454.55 4.894 532.65 481.889
B i i39 a 8.45 396.9 0 26.3 1.004
B i i40 b 298.389 208.86 0 37.714 0
E changed 13.3082 0 286.6918 270.0528
E changed 101.989 127.35 106.911 79.31300000000002
E changed 161.73000000000002 42.289 138.26999999999998 357.711
E removed 160.3246 256.7006 139.6754 143.2994
S -1 stroke, 1 image change
P 612 792
A s s41 pen #111111 0 542.4 120.6 271.549 820.259 585.967 140.2 485.674 733.151
A i i42 b 347.505 779.45 0 234.45 1.006
A i i43 b 239.6 96.15 57.25 126.377 1.004
B s s41 pen #111111 0 542.4 120.6 271.549 820.259 585.967 140.2 485.674 733.151
B s s44 pen #111111 13.2 559 742.75
E added 540.52 724.27 36.960000000000036 36.960000000000036
E removed 162.92289123578462 30.07328676998192 153.3542175284307 132.1534264600362
S +1 stroke, 1 image change
P 300 400
A s s45 highlighter #9577e1 2 464.988 683.45 302.65 521.25 347.95 827.75 345.921 613.688 567.276 383.915
A s s46 highlighter #002fd9 3.06 394 660.15 618.592 536.5
A s s47 highlighter #111111 3.06 272.877 62.2 102.578 135.627
A s s48 highlighter #111111 2 399.164 367.841
A s s49 pen #002fd9 3.06 520.651 -15.027 -34.189 800.426
A i i50 a 387.105 496.726 141.8 108.35 0
A i i51 a 471.272 477.124 158.118 237.416 0.722
B s s45 highlighter #9577e1 2 465.028 683.45 302.65 521.25 347.95 827.75 345.921 613.688 567.276 383.915
B s s53 highlighter #088901 0 22.4 727.721
B s s46 highlighter #002fd9 3.06 394 660.15 618.592 536.5
B s s52 pen #2e1745 18 330.589 500.323
B s s54 highlighter #002fd9 3.06 212.8 90.181
B s s47 highlighter #111111 3.06 272.877 62.2 102.578 135.627
B s s48 highlighter #111111 2 402.164 367.841
B s s49 pen #002fd9 3.06 523.651 -15.027 -34.189 800.426
B i i50 a 387.105 496.726 141.8 108.35 0
B i i51 a 471.272 477.124 158.118 237.416 0.722
E added 202.8 80.181 20 20
E changed 0 0 300 400
S +3 strokes
P 595 400
A s s55 pen #9e531d 9.332 283.25 175.55
A s s56 highlighter #002fd9 3.04 254.45 645.646 325.912 429.409 208.65 72.65 449.45 625.412 -6.041 385.739
A s s57 highlighter #e57777 18 614.547 299.397 287.386 843.013 236.371 121.264 69.781 811.85
A s s58 highlighter #f3283d 0 182.6 214.7 117.8 445.1 176.095 441.036 201.7 442
A s s59 pen #002fd9 18 14.4 347.076
A s s60 pen #002fd9 3.06 -34.6 340.786 432.418 226.565 -9.148 511.7 586 202.245
A i i61 a 547.374 265.878 0 265.4 1.039
A i i62 a 365.25 777.05 217.663 283.65 1.004
B s s55 pen #9e531d 9.332 283.25 175.55 311.284 244.25
B s s56 highlighter #002fd9 3.04 254.45 645.646 325.912 429.409 208.65 72.65 449.45 625.412 -6.041 385.739
B s s58 highlighter #f3283d 0 182.6 214.7 117.8 445.1 176.095 441.036 201.7 442
B s s59 pen #002fd9 18 14.4 347.076
B s s64 pen #002fd9 1.927 401.863 13.081
B s s60 pen #002fd9 3.06 -34.6 340.786 432.418 226.565 -9.148 511.7 586 202.245
B s s63 highlighter #002fd9 0 526.202 676.1 -10.2 -6.613 65.434 119.75 -15.494 724.95
B i i61 a 547.374 265.878 0 265.4 1.043
E changed 270.1852 162.48520000000002 54.163599999999974 94.82959999999997
E added 391.863 3.0809999999999995 20 20
E added 0 0 536.202 400
E removed 44.581 96.064 550.419 303.93600000000004
S +1 stroke, 1 image change
P 612 792
A s s65 pen #002fd9 2 347.45 121.564 200.65 152.263 289.4 405.217
A s s66 highlighter #bc1c87 2 162.471 86.4 479.511 459.439 527.128 136.8 594.914 627.05 629.31 209.615
A s s67 pen #002fd9 0 604.9 459.85 272.411 227.997
A i i68 a 345.704 89.864 88.713 59.7 1.006
A i i69 a 72.027 67.9 0 267.744 0
B s s66 highlighter #bc1c87 2 162.471 86.4 479.511 459.439 527.128 136.8 594.914 627.05 629.31 209.615
B s s67 pen #002fd9 0 604.9 459.85 272.411 227.997
B i i68 a 345.704 89.864 88.717 59.7 1.006
B i i69 a 72.027 67.9 0 267.744 0
E removed 190.65 111.564 166.79999999999998 303.653
S -1 stroke, 1 image change
P 300 400
A s s70 highlighter #b5f765 18 -38.901 179.809 -14.373 569.226 230.45 495.208
A s s71 highlighter #111111 14.292 50.623 660.04
A s s72 highlighter #5e861f 3 483.542 235.856 466.363 360.5 109.579 140.301 345.643 -18.302
A i i73 a 457.461 219.3 0 83.2 1.006
A i i74 b 65.343 828.15 140.75 196.478 1.006
B s s70 highlighter #b5f765 18 -38.901 179.809 -14.373 569.226 230.45 495.208
B s s71 highlighter #111111 14.292 50.623 660.04
B s s75 highlighter #002fd9 2 384.95 469.142 346.39 225.644 215.263 691.576
B s s72 highlighter #5e861f 3 483.542 235.856 466.363 360.5 109.579 140.301 345.643 -18.302
B i i73 a 457.461 219.3 0 83.2 1.006
B i i74 b 65.343 828.15 140.75 196.478 1.006
B i i76 b 198.05 623.989 0 275.85 0.94
E added 205.263 215.644 94.737 184.356
S +1 stroke, 1 image change
";

    #[test]
    fn diff_and_summary_equal_5_18_0_for_recorded_pairs() {
        let page = |items: Vec<SnapItem>, w: f64, h: f64| SnapPage {
            id: Id(1),
            template: Template::Default,
            size: (w as u32, h as u32),
            background: Background::Template,
            side_panel: None,
            items,
        };
        let mut cases = 0;
        for case in FIXTURE.split("P ").filter(|c| !c.is_empty()) {
            let mut lines = case.lines();
            let wh: Vec<f64> = lines.next().unwrap().split(' ').map(|v| v.parse().unwrap()).collect();
            let (mut prev, mut next, mut want, mut summary) = (vec![], vec![], vec![], "");
            for line in lines {
                let f: Vec<&str> = line.split(' ').collect();
                let num = |i: usize| f[i].parse::<f64>().unwrap();
                match f[0] {
                    "A" | "B" => {
                        let body = if f[1] == "s" {
                            let tool = if f[3] == "pen" { Tool::Pen } else { Tool::Highlighter };
                            let xy = f[6..].iter().map(|v| v.parse().unwrap()).collect();
                            Body::Stroke { tool, color: Color::parse(f[4]), width: num(5), xy, p: None }
                        } else {
                            Body::Image { asset: [f[3].as_bytes()[0]; 32], cx: num(4), cy: num(5), w: num(6), h: num(7), rotation: num(8) }
                        };
                        let it = SnapItem { id: Id::legacy(f[2]), z: Z::default(), body };
                        if f[0] == "A" { prev.push(it) } else { next.push(it) }
                    }
                    "E" => {
                        let mark = match f[1] {
                            "added" => Mark::Added,
                            "changed" => Mark::Changed,
                            _ => Mark::Removed,
                        };
                        want.push(Highlight { x: num(2), y: num(3), w: num(4), h: num(5), mark });
                    }
                    _ => summary = &line[2..],
                }
            }
            let (p, n) = (page(prev, wh[0], wh[1]), page(next, wh[0], wh[1]));
            let got = diff_page(Some(&p), &n);
            assert_eq!(got.len(), want.len(), "case {cases}");
            for (g, w) in got.iter().zip(&want) {
                assert!(g.mark == w.mark && [g.x, g.y, g.w, g.h].map(f64::to_bits) == [w.x, w.y, w.w, w.h].map(f64::to_bits), "case {cases}: {g:?} != {w:?}");
            }
            let snap = |p: SnapPage| Snapshot { name: String::new(), pages: vec![p] };
            assert_eq!(summarize(&snap(p.clone()), &snap(n.clone())), summary, "case {cases}");
            // By index, no previous entry → nothing; previous entry without the page → everything added.
            assert!(diff(None, &snap(n.clone()))[0].is_empty());
            let fresh = diff(Some(&Snapshot::default()), &snap(n.clone()));
            assert!(fresh[0].iter().all(|h| h.mark == Mark::Added));
            cases += 1;
        }
        assert_eq!(cases, 11);
        let two = |ids: [u128; 2]| Snapshot { name: String::new(), pages: ids.map(|_| page(vec![], 595.0, 842.0)).into_iter().zip(ids).map(|(mut p, i)| { p.id = Id(i); p }).collect() };
        assert_eq!(summarize(&two([1, 2]), &two([2, 1])), "Pages reordered");
        assert_eq!(summarize(&two([1, 2]), &two([1, 2])), "Saved");
        assert_eq!(summarize(&two([1, 2]), &two([1, 3])), "+1 page, -1 page");
        let mut wide = two([1, 2]);
        wide.pages[0].size.0 = 600;
        wide.pages[1].background = Background::Custom { asset: [0; 32] };
        assert_eq!(summarize(&two([1, 2]), &wide), "2 page settings changed");
    }
}
