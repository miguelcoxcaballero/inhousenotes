//! Document state, apply, queries, the `Tx` builder and undo inverses (ARCHITECTURE §4.1–4.3), and the
//! core↔sync interface types (§4.4).
//!
//! Every field is an LWW register, so `apply` is idempotent and commutative: any delivery order, duplicates and gaps
//! converge. The derived indexes (page order, per-page draw lists with bounds, per-page membership, dirty pages)
//! are kept in step by `apply`/`merge`; nothing else may write the registers.

use crate::geom::{self, Affine, SelectionBounds, Sweep};
use crate::id::{ActorId, DocId, Hash, Id, IdGen, IdMap, IdSet};
use crate::op::{Background, Batch, Body, Color, ImageRect, Op, PageProp, Template, Tool, VersionMeta, MIN_PAGE_SIDE, Z};
use crate::pos::Pos;
use crate::snapshot::{EncodedBatch, EncodedSnapshot};
use crate::stamp::{Clock, Reg, Stamp, VersionVector};
use std::collections::{BTreeMap, BTreeSet, HashSet};

// ------------------------------------------------------------------------------------------------ state

#[derive(Clone, PartialEq, Default, Debug)]
pub struct Page {
    pub pos: Reg<Pos>,
    pub deleted: Reg<bool>,
    pub template: Reg<Template>,
    pub size: Reg<(u32, u32)>,
    pub background: Reg<Background>,
    pub side_panel: Reg<Option<Vec<u8>>>,
}

impl Page {
    /// Whether the page's `PageNew` arrived. Ops that name a page before that are stored and the page stays
    /// hidden until it does.
    pub fn is_born(&self) -> bool { self.size.at != Stamp::ZERO }
}

#[derive(Clone, PartialEq, Debug)]
pub struct Item {
    pub page: Reg<Id>,
    pub z: Reg<Z>,
    pub body: Reg<Body>,
    pub deleted: Reg<bool>,
}

impl Default for Item {
    /// An item known only by an `ItemDel`: registers at `Stamp::ZERO`, no body yet.
    fn default() -> Self { Item { page: Reg::default(), z: Reg::default(), body: Reg::new(Body::Dropped, Stamp::ZERO), deleted: Reg::default() } }
}

impl Item {
    /// Drawn, hit-tested and selectable.
    pub fn is_live(&self) -> bool { !self.deleted.v && !self.body.v.is_dropped() }
    /// Whether a snapshot omits the body: it is already gone, or the item was deleted after its body was last
    /// written. Depends on register stamps only, so every converged replica encodes the same bytes. An undo of the
    /// deletion carries the body with a stamp above the deletion (SYNC §5), which revives it everywhere.
    pub fn body_dropped_in_snapshot(&self) -> bool { self.body.v.is_dropped() || (self.deleted.v && self.deleted.at > self.body.at) }
}

/// A timeline entry: the opaque `VersionMeta` record and its prune tombstone.
#[derive(Clone, PartialEq, Default, Debug)]
pub struct Version {
    pub data: Reg<Vec<u8>>,
    pub deleted: Reg<bool>,
}

impl Version {
    /// Listed in the timeline: its `VersionAdd` arrived and it was not pruned.
    pub fn is_listed(&self) -> bool { self.data.at != Stamp::ZERO && !self.deleted.v }
}

/// One entry of a page's draw list. `bounds` = point bounds of a stroke (v5 `getStrokeBounds`) or the corner
/// bounds of an image; `width` = stroke width (0 for images). They make hit tests a contiguous scan.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct DrawRef {
    pub z: Z,
    pub id: Id,
    pub bounds: [f64; 4],
    pub width: f64,
}

/// v5 `redrawPage` order: legacy cover strokes, then all images, then all other strokes — each in `(Z, Id)` order.
#[derive(Clone, Copy, Default, Debug)]
pub struct DrawList<'a> {
    pub covers: &'a [DrawRef],
    pub images: &'a [DrawRef],
    pub strokes: &'a [DrawRef],
}

impl<'a> DrawList<'a> {
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &'a DrawRef> + 'a { self.covers.iter().chain(self.images).chain(self.strokes) }
    pub fn len(&self) -> usize { self.covers.len() + self.images.len() + self.strokes.len() }
    pub fn is_empty(&self) -> bool { self.len() == 0 }
}

#[derive(Clone, Default, Debug)]
struct PageIndex {
    /// covers, images, strokes
    runs: [Vec<DrawRef>; 3],
}

/// What a batch or snapshot changed (ids ascending).
#[derive(Clone, PartialEq, Eq, Default, Debug)]
pub struct Changes {
    /// Pages whose items changed: repaint them and rewrite their chunks.
    pub pages: Vec<Id>,
    /// Items with a written register.
    pub items: Vec<Id>,
    /// Pages with a written property (template, size, background, side panel, position, deleted).
    pub page_props: Vec<Id>,
    /// `order()` may have changed (page created, moved, deleted or restored).
    pub order: bool,
    pub name: bool,
    pub versions: bool,
    /// `false` when the batch was a duplicate and nothing was done.
    pub applied: bool,
}

impl Changes {
    /// Page add/move/delete or a page property: v5 clears the undo stack on a remote structural change.
    pub fn is_structural(&self) -> bool { self.order || !self.page_props.is_empty() }
}

#[derive(Default)]
struct Track {
    /// item → (was live on page, page register before (None = unknown item), a register was written)
    items: IdMap<(Option<Id>, Option<Id>, bool)>,
    page_props: BTreeSet<Id>,
    order: bool,
    name: bool,
    versions: bool,
}

#[derive(Clone, Default, Debug)]
pub struct Doc {
    /// doc:exportName
    pub name: Reg<String>,
    /// Includes deleted (tombstoned) and not-yet-born pages.
    pub pages: IdMap<Page>,
    /// Strokes and images of all pages, including deleted ones.
    pub items: IdMap<Item>,
    /// Timeline (grow-only; prune = tombstone register).
    pub versions: BTreeMap<Id, Version>,
    pub vv: VersionVector,
    // Derived, not serialized.
    order: Vec<Id>,
    index: IdMap<PageIndex>,
    members: IdMap<IdSet>,
    dirty: BTreeSet<Id>,
    header_dirty: bool,
}

fn merge_reg<T: Clone>(mine: &mut Reg<T>, other: &Reg<T>) -> bool {
    let wins = other.at > mine.at;
    if wins {
        *mine = other.clone();
    }
    wins
}

/// Body register write: LWW, plus "same write, but mine lost its body" (a re-delivered batch or a fuller snapshot
/// gives the content back; the encoded state does not depend on it).
fn body_wins(mine: &Reg<Body>, body: &Body, at: Stamp) -> bool { at > mine.at || (at == mine.at && mine.v.is_dropped() && !body.is_dropped()) }

fn merge_body(mine: &mut Reg<Body>, body: &Body, at: Stamp) -> bool {
    let wins = body_wins(mine, body, at);
    if wins {
        *mine = Reg::new(body.clone(), at);
    }
    wins
}

/// → (order may change, another property was written)
fn merge_page(p: &mut Page, o: &Page) -> (bool, bool) {
    let order = merge_reg(&mut p.pos, &o.pos) | merge_reg(&mut p.deleted, &o.deleted) | merge_reg(&mut p.size, &o.size);
    (order, merge_reg(&mut p.template, &o.template) | merge_reg(&mut p.background, &o.background) | merge_reg(&mut p.side_panel, &o.side_panel))
}

fn merge_item(it: &mut Item, o: &Item) -> bool {
    merge_reg(&mut it.page, &o.page) | merge_reg(&mut it.z, &o.z) | merge_body(&mut it.body, &o.body.v, o.body.at) | merge_reg(&mut it.deleted, &o.deleted)
}

fn merge_version(v: &mut Version, o: &Version) -> bool { merge_reg(&mut v.data, &o.data) | merge_reg(&mut v.deleted, &o.deleted) }

fn run_of(body: &Body) -> usize {
    match body {
        Body::Stroke { tool: Tool::LegacyCover, .. } => 0,
        Body::Image { .. } => 1,
        _ => 2,
    }
}

fn draw_ref(id: Id, item: &Item) -> DrawRef {
    let (bounds, width) = match &item.body.v {
        Body::Stroke { xy, width, .. } => (geom::bounds(xy), *width),
        Body::Image { cx, cy, w, h, rotation, .. } => (geom::bounds(&geom::image_corners(*cx, *cy, *w, *h, *rotation)), 0.0),
        Body::Dropped => ([f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY], 0.0),
    };
    DrawRef { z: item.z.v, id, bounds, width }
}

impl Doc {
    // -------------------------------------------------------------------------------------------- clock

    /// The hybrid logical clock a writer `actor` resumes with after loading this document: `observed` = the
    /// greatest clock of any register or z stamp not beyond `wall_ms + 24 h` (the same horizon as
    /// `Clock::observe`, so a corrupt far-future stamp stored in the document is not adopted on reload either),
    /// `last` = the greatest clock of the stamps written by `actor` itself, horizon or not (a wall clock that
    /// stepped back must not let the writer fall below its own earlier writes). One pass over the registers.
    pub fn clock(&self, actor: ActorId, wall_ms: u64) -> Clock {
        let horizon = wall_ms.saturating_add(crate::stamp::CLOCK_HORIZON_MS);
        let mut c = Clock::default();
        let mut see = |at: Stamp| {
            if at.clock <= horizon {
                c.observed = c.observed.max(at.clock);
            }
            if at.actor == actor && at.clock != u64::MAX {
                c.last = c.last.max(at.clock);
            }
        };
        see(self.name.at);
        for p in self.pages.values() {
            [p.pos.at, p.deleted.at, p.template.at, p.size.at, p.background.at, p.side_panel.at].into_iter().for_each(&mut see);
        }
        for it in self.items.values() {
            [it.page.at, it.z.at, it.z.v.at, it.body.at, it.deleted.at].into_iter().for_each(&mut see);
        }
        for v in self.versions.values() {
            see(v.data.at);
            see(v.deleted.at);
        }
        c
    }

    /// Greatest clock the document holds within the horizon of `wall_ms` (see [`Doc::clock`]).
    pub fn max_clock(&self, wall_ms: u64) -> u64 { self.clock(ActorId(0), wall_ms).observed }

    // -------------------------------------------------------------------------------------------- apply

    /// Applies a local or remote batch atomically. A batch already in `vv` is skipped (`Changes::applied == false`).
    /// The caller observes `b.max_clock()` with its `Clock`.
    pub fn apply(&mut self, b: &Batch) -> Changes {
        if !self.vv.add(b.actor, b.seq) {
            return Changes::default();
        }
        let mut t = Track::default();
        for (i, op) in b.ops.iter().enumerate() {
            self.apply_op(op, b.stamp(i), &mut t, None);
        }
        self.finish(t)
    }

    /// `apply` for a batch the caller no longer needs (remote batches): bodies are moved, not copied.
    pub fn apply_owned(&mut self, mut b: Batch) -> Changes {
        let all = 0..b.ops.len();
        self.apply_part(&mut b, all)
    }

    /// Applies ops `ops` of a large remote batch, so that the session can spread it over idle slices (remote-apply
    /// budget, ARCHITECTURE §4.4). Bodies in that range are moved out of `b`. The batch enters `vv` with the part
    /// that reaches its last op; until then the document holds a prefix of it, which is an ordinary consistent
    /// state (every op is an independent register write), and a restart simply applies the journal entry again.
    pub fn apply_part(&mut self, b: &mut Batch, ops: std::ops::Range<usize>) -> Changes {
        if b.seq == 0 || self.vv.contains(b.actor, b.seq) {
            return Changes::default();
        }
        let mut t = Track::default();
        let end = ops.end.min(b.ops.len());
        for i in ops.start.min(end)..end {
            let at = b.stamp(i);
            let moved = match &mut b.ops[i] {
                Op::ItemPut { body, .. } => Some(std::mem::replace(body, Body::Dropped)),
                _ => None,
            };
            self.apply_op(&b.ops[i], at, &mut t, moved);
        }
        if end == b.ops.len() {
            self.vv.add(b.actor, b.seq);
        }
        self.finish(t)
    }

    fn item_mut<'t>(&'t mut self, id: Id, t: &'t mut Track) -> (&'t mut Item, &'t mut bool) {
        let items = &self.items;
        let entry = t.items.entry(id).or_insert_with(|| {
            let old = items.get(&id);
            (old.filter(|i| i.is_live()).map(|i| i.page.v), old.map(|i| i.page.v), false)
        });
        (self.items.entry(id).or_default(), &mut entry.2)
    }

    /// `moved` = the body taken out of an `ItemPut` whose `body` field is then hollow.
    fn apply_op(&mut self, op: &Op, at: Stamp, t: &mut Track, moved: Option<Body>) {
        match op {
            Op::PageNew { page, pos, template, size, background } => {
                let p = self.pages.entry(*page).or_default();
                // Not short-circuited: each register is written independently.
                let wrote = p.pos.set(pos.clone(), at) | p.template.set(*template, at) | p.size.set(*size, at) | p.background.set(*background, at);
                if wrote {
                    t.order = true;
                    t.page_props.insert(*page);
                }
            }
            Op::PageMove { page, pos } => {
                if self.pages.entry(*page).or_default().pos.set(pos.clone(), at) {
                    t.order = true;
                }
            }
            Op::PageDel { page, deleted } => {
                if self.pages.entry(*page).or_default().deleted.set(*deleted, at) {
                    t.order = true;
                }
            }
            Op::PageSet { page, prop } => {
                let p = self.pages.entry(*page).or_default();
                let wrote = match prop {
                    PageProp::Template(v) => p.template.set(*v, at),
                    PageProp::Size(v) => p.size.set(*v, at),
                    PageProp::Background(v) => p.background.set(*v, at),
                    PageProp::SidePanel(v) => p.side_panel.set(v.clone(), at),
                };
                if wrote {
                    t.page_props.insert(*page);
                    t.order |= matches!(prop, PageProp::Size(_)); // a size write can be the page's birth
                }
            }
            Op::ItemPut { item, page, z, body } => {
                let (it, wrote) = self.item_mut(*item, t);
                *wrote |= it.page.set(*page, at) | it.z.set(*z, at);
                if body_wins(&it.body, moved.as_ref().unwrap_or(body), at) {
                    it.body = Reg::new(moved.unwrap_or_else(|| body.clone()), at);
                    *wrote = true;
                }
            }
            Op::ItemDel { item, deleted } => {
                let (it, wrote) = self.item_mut(*item, t);
                *wrote |= it.deleted.set(*deleted, at);
            }
            Op::DocName { name } => t.name |= self.name.set(name.clone(), at),
            Op::VersionAdd { meta } => t.versions |= self.versions.entry(meta.id).or_default().data.set(meta.data.clone(), at),
            Op::VersionDel { version } => t.versions |= self.versions.entry(*version).or_default().deleted.set(true, at),
        }
    }

    /// Merges another replica's state register by register and VV pointwise (snapshots never replace, SYNC §3.7).
    pub fn merge(&mut self, other: &Doc) -> Changes {
        let mut t = Track::default();
        t.name = merge_reg(&mut self.name, &other.name);
        for (id, o) in &other.pages {
            let (order, props) = merge_page(self.pages.entry(*id).or_default(), o);
            t.order |= order;
            if order | props {
                t.page_props.insert(*id);
            }
        }
        for (id, o) in &other.items {
            let (it, wrote) = self.item_mut(*id, &mut t);
            *wrote |= merge_item(it, o);
        }
        for (id, o) in &other.versions {
            t.versions |= merge_version(self.versions.entry(*id).or_default(), o);
        }
        self.vv.merge(&other.vv);
        self.finish(t)
    }

    fn finish(&mut self, t: Track) -> Changes {
        if t.order {
            self.rebuild_order();
        }
        // page → (ids to drop from its draw list, entries to add)
        let mut edits: IdMap<(IdSet, Vec<(usize, DrawRef)>)> = IdMap::default();
        let mut pages = BTreeSet::new();
        let mut items = Vec::new();
        for (id, (was_live_on, old_page, wrote)) in t.items {
            if !wrote {
                continue;
            }
            let Some(item) = self.items.get(&id) else { continue };
            let page = item.page.v;
            items.push(id);
            pages.insert(page);
            if old_page != Some(page) {
                if let Some(old) = old_page {
                    pages.insert(old);
                    if let Some(set) = self.members.get_mut(&old) {
                        set.remove(&id);
                    }
                }
                self.members.entry(page).or_default().insert(id);
            }
            if let Some(old) = was_live_on {
                edits.entry(old).or_default().0.insert(id);
            }
            if item.is_live() {
                edits.entry(page).or_default().1.push((run_of(&item.body.v), draw_ref(id, item)));
            }
        }
        for (page, (removed, mut added)) in edits {
            let runs = &mut self.index.entry(page).or_default().runs;
            if !removed.is_empty() {
                for run in runs.iter_mut() {
                    run.retain(|r| !removed.contains(&r.id));
                }
            }
            added.sort_unstable_by_key(|(_, r)| (r.z, r.id));
            // Stable sort below: a sorted run plus a sorted tail merges in linear time.
            let mut unsorted = [false; 3];
            for (k, r) in added {
                unsorted[k] |= runs[k].last().is_some_and(|l| (l.z, l.id) > (r.z, r.id));
                runs[k].push(r);
            }
            for k in 0..3 {
                if unsorted[k] {
                    runs[k].sort_by_key(|r| (r.z, r.id));
                }
            }
        }
        items.sort_unstable();
        self.dirty.extend(pages.iter().copied());
        self.header_dirty = true; // the VV at least
        Changes {
            pages: pages.into_iter().collect(),
            items,
            page_props: t.page_props.into_iter().collect(),
            order: t.order,
            name: t.name,
            versions: t.versions,
            applied: true,
        }
    }

    fn rebuild_order(&mut self) {
        let mut live: Vec<(&Pos, Id)> = self.pages.iter().filter(|(_, p)| p.is_born() && !p.deleted.v).map(|(id, p)| (&p.pos.v, *id)).collect();
        live.sort_unstable();
        self.order = live.into_iter().map(|(_, id)| id).collect();
        if self.order.is_empty() {
            // Every page deleted: show the one deleted last (projection only, ARCHITECTURE §4.1).
            self.order.extend(self.pages.iter().filter(|(_, p)| p.is_born()).max_by_key(|(id, p)| (p.deleted.at, **id)).map(|(id, _)| *id));
        }
    }

    /// Bulk load without index maintenance (`snapshot::Loader`; `reindex` follows).
    pub(crate) fn load_header(&mut self, vv: &VersionVector, name: Reg<String>, pages: Vec<(Id, Page)>, versions: Vec<(Id, Version)>) {
        self.vv.merge(vv);
        merge_reg(&mut self.name, &name);
        for (id, o) in &pages {
            merge_page(self.pages.entry(*id).or_default(), o);
        }
        for (id, o) in &versions {
            merge_version(self.versions.entry(*id).or_default(), o);
        }
    }

    pub(crate) fn load_items(&mut self, items: Vec<(Id, Item)>) {
        for (id, o) in items {
            match self.items.entry(id) {
                std::collections::hash_map::Entry::Vacant(e) => {
                    e.insert(o);
                }
                std::collections::hash_map::Entry::Occupied(mut e) => {
                    merge_item(e.get_mut(), &o);
                }
            }
        }
    }

    /// Rebuilds every derived index from the registers (after a bulk load, `snapshot::Loader`).
    pub fn reindex(&mut self) {
        self.rebuild_order();
        self.index.clear();
        self.members.clear();
        for (id, item) in &self.items {
            self.members.entry(item.page.v).or_default().insert(*id);
            if item.is_live() {
                self.index.entry(item.page.v).or_default().runs[run_of(&item.body.v)].push(draw_ref(*id, item));
            }
        }
        for run in self.index.values_mut().flat_map(|p| p.runs.iter_mut()) {
            run.sort_unstable_by_key(|r| (r.z, r.id));
        }
    }

    // -------------------------------------------------------------------------------------------- queries

    /// Live pages in `(pos, id)` order; if every page is deleted, the page with the greatest `deleted` stamp.
    pub fn order(&self) -> &[Id] { &self.order }

    /// Covers, images, strokes of a page — each in `(Z, Id)` order (ARCHITECTURE §6.2).
    pub fn draw_list(&self, page: Id) -> DrawList<'_> {
        match self.index.get(&page) {
            Some(p) => DrawList { covers: &p.runs[0], images: &p.runs[1], strokes: &p.runs[2] },
            None => DrawList::default(),
        }
    }

    /// Live items of a page in draw order.
    pub fn page_items(&self, page: Id) -> Vec<Id> { self.draw_list(page).iter().map(|r| r.id).collect() }

    /// Every item whose page register names `page` (live or deleted), ascending: the content of its chunk.
    pub fn chunk_items(&self, page: Id) -> Vec<Id> {
        let mut ids: Vec<Id> = self.members.get(&page).into_iter().flatten().copied().collect();
        ids.sort_unstable();
        ids
    }

    /// Pages that have a chunk (at least one item names them), ascending.
    pub fn chunk_pages(&self) -> Vec<Id> {
        let mut ids: Vec<Id> = self.members.iter().filter(|(_, set)| !set.is_empty()).map(|(id, _)| *id).collect();
        ids.sort_unstable();
        ids
    }

    /// Pages whose chunks changed since the last call, and whether the header did (dirty tracking for the store).
    pub fn take_dirty(&mut self) -> (Vec<Id>, bool) {
        (std::mem::take(&mut self.dirty).into_iter().collect(), std::mem::take(&mut self.header_dirty))
    }

    /// A live item.
    pub fn live(&self, id: Id) -> Option<&Item> { self.items.get(&id).filter(|i| i.is_live()) }

    fn stroke(&self, r: &DrawRef) -> Option<&[f64]> {
        match &self.items.get(&r.id)?.body.v {
            Body::Stroke { tool, xy, .. } if *tool != Tool::LegacyEraseArea => Some(xy),
            _ => None,
        }
    }

    /// Stroke eraser (v5 `eraseStrokeAtPoint` + `strokeHitsPoint`): strokes of `page` whose centre-line passes
    /// within `r` of the segment swept from `(x0,y0)` to `(x1,y1)` (BEHAVIOUR-CHANGES B-E6; equal points = one
    /// sample, exactly v5). Topmost first, as v5 iterates. Covers, images and legacy eraser strokes never hit.
    pub fn hit_strokes(&self, page: Id, x0: f64, y0: f64, x1: f64, y1: f64, r: f64) -> SmallVec<Id> {
        let sweep = Sweep { x0, y0, x1, y1 };
        let hit = |d: &&DrawRef| sweep.may_touch(&d.bounds, geom::eraser_pad(r, d.width)) && self.stroke(d).is_some_and(|xy| geom::polyline_hit(xy, &sweep, r));
        self.draw_list(page).strokes.iter().rev().filter(hit).map(|d| d.id).collect()
    }

    /// Area eraser (v5 `eraseAreaAtPoint`): for each stroke of `page` that the sweep changes, the surviving
    /// half-open point ranges (possibly none), in draw order. Strokes with fewer than two points are kept.
    pub fn split_strokes(&self, page: Id, sweep: &Sweep, r: f64) -> Vec<(Id, Vec<(u32, u32)>)> {
        let mut out = Vec::new();
        for d in self.draw_list(page).strokes {
            let Some(xy) = self.stroke(d).filter(|xy| xy.len() >= 4 && sweep.may_touch(&d.bounds, geom::eraser_pad(r, d.width))) else { continue };
            let mut ranges = Vec::new();
            if geom::split_ranges(xy, sweep, r, &mut ranges) {
                out.push((d.id, ranges));
            }
        }
        out
    }

    /// Lasso (v5 `selectStrokesWithLasso`): images whose rotated rectangle intersects the polygon, then strokes
    /// with any point inside it, in draw order. A polygon needs more than two points.
    pub fn lasso(&self, page: Id, poly: &[f64]) -> Vec<Id> {
        if poly.len() / 2 <= 2 {
            return Vec::new();
        }
        let list = self.draw_list(page);
        let pb = geom::bounds(poly);
        let image = |d: &&DrawRef| match self.items.get(&d.id).map(|i| &i.body.v) {
            Some(Body::Image { cx, cy, w, h, rotation, .. }) => geom::polygon_intersects_rect(poly, &geom::image_corners(*cx, *cy, *w, *h, *rotation)),
            _ => false,
        };
        // A stroke whose bounds miss the polygon's bounds has no point inside it.
        let stroke = |d: &&DrawRef| {
            !(d.bounds[2] < pb[0] || d.bounds[0] > pb[2] || d.bounds[3] < pb[1] || d.bounds[1] > pb[3])
                && self.stroke(d).is_some_and(|xy| geom::polyline_in_polygon(xy, poly))
        };
        list.images.iter().filter(image).chain(list.strokes.iter().filter(stroke)).map(|d| d.id).collect()
    }

    /// v5 `getSelectionBounds`: min/max of the selected strokes' points and images' corners (stroke width not
    /// included) as `[minX, minY, maxX, maxY]`.
    pub fn selection_bounds(&self, items: &[Id]) -> Option<[f64; 4]> {
        let mut acc = SelectionBounds::default();
        for item in items.iter().filter_map(|id| self.live(*id)) {
            match &item.body.v {
                Body::Stroke { xy, .. } => acc.add(xy),
                Body::Image { cx, cy, w, h, rotation, .. } => acc.add(&geom::image_corners(*cx, *cy, *w, *h, *rotation)),
                Body::Dropped => {}
            }
        }
        acc.get()
    }

    // -------------------------------------------------------------------------------------------- undo

    /// Inverse ops of `b` against the pre-state (call before `apply`), per SYNC §5. Undo applies them as a new
    /// batch (new stamps); the redo ops are the inverse of that undo batch, computed the same way.
    pub fn inverse(&self, b: &Batch) -> Vec<Op> {
        let mut groups: Vec<Vec<Op>> = Vec::new();
        let mut seen: HashSet<(u8, Id)> = HashSet::new();
        let mut name_seen = false;
        for op in &b.ops {
            let group = match op {
                Op::ItemPut { item, .. } if seen.insert((0, *item)) => match self.items.get(item) {
                    None => vec![Op::ItemDel { item: *item, deleted: true }],
                    Some(old) if old.body.v.is_dropped() => vec![],
                    Some(old) => vec![Op::ItemPut { item: *item, page: old.page.v, z: old.z.v, body: old.body.v.clone() }],
                },
                Op::ItemDel { item, deleted } if seen.insert((1, *item)) => match self.items.get(item) {
                    Some(old) if *deleted && old.is_live() => {
                        // The body travels, so a replica that dropped it can revive it.
                        vec![Op::ItemPut { item: *item, page: old.page.v, z: old.z.v, body: old.body.v.clone() }, Op::ItemDel { item: *item, deleted: false }]
                    }
                    Some(old) if !*deleted && old.deleted.v => vec![Op::ItemDel { item: *item, deleted: true }],
                    _ => vec![],
                },
                Op::PageSet { page, prop } => {
                    let kind = match prop {
                        PageProp::Template(_) => 2,
                        PageProp::Size(_) => 3,
                        PageProp::Background(_) => 4,
                        PageProp::SidePanel(_) => 5,
                    };
                    match self.pages.get(page).filter(|p| p.is_born() && seen.insert((kind, *page))) {
                        Some(p) => {
                            let prop = match prop {
                                PageProp::Template(_) => PageProp::Template(p.template.v),
                                PageProp::Size(_) => PageProp::Size(p.size.v),
                                PageProp::Background(_) => PageProp::Background(p.background.v),
                                PageProp::SidePanel(_) => PageProp::SidePanel(p.side_panel.v.clone()),
                            };
                            vec![Op::PageSet { page: *page, prop }]
                        }
                        None => vec![],
                    }
                }
                Op::DocName { .. } if !std::mem::replace(&mut name_seen, true) => vec![Op::DocName { name: self.name.v.clone() }],
                // Page operations and timeline entries are not undoable (5.18.0 records no history for them).
                _ => vec![],
            };
            groups.push(group);
        }
        // Reverse op order; inside a group the order is kept (put before undelete).
        groups.into_iter().rev().flatten().collect()
    }
}

/// The plan's `SmallVec`: no external crates (ARCHITECTURE §2.4), hits are few.
pub type SmallVec<T> = Vec<T>;

// ------------------------------------------------------------------------------------------------ Tx

/// The writer identity of one document session: actor, id generator, HLC and the last batch seq.
#[derive(Clone, Debug)]
pub struct Author {
    pub actor: ActorId,
    pub ids: IdGen,
    pub clock: Clock,
    /// Seq of the last committed batch of this actor in this document (`docs.seq[actor]`).
    pub seq: u32,
}

impl Author {
    /// A writer for a document it has never seen: an empty new document, or tests. For a document loaded from the
    /// store use [`Author::resume`]: a writer whose clock restarts at zero stamps below what the document already
    /// holds, and LWW then drops its edits on every replica.
    pub fn new(ids: IdGen, seq: u32) -> Author { Author { actor: ids.actor, ids, clock: Clock::default(), seq } }

    /// The writer of a document that was loaded (snapshot chunks decoded, journal applied): the hybrid logical
    /// clock is rebuilt from the stamps in `doc` ([`Doc::clock`]), so the next batch is stamped above everything
    /// the document holds within the horizon and above every surviving write of this actor, whatever the wall
    /// clock says now. Nothing about the clock is stored; call this again after every reload.
    pub fn resume(ids: IdGen, seq: u32, doc: &Doc, wall_ms: u64) -> Author { Author { actor: ids.actor, clock: doc.clock(ids.actor, wall_ms), ids, seq } }
}

/// Builder used by tools: reads the pre-state, collects ops, `commit` assigns clock, seq and stamps.
pub struct Tx<'a> {
    doc: &'a Doc,
    author: &'a mut Author,
    wall_ms: u64,
    ops: Vec<Op>,
    /// Page order including this transaction's own page ops (several pages can be chained in one batch).
    pages: Option<Vec<(Pos, Id)>>,
}

impl<'a> Tx<'a> {
    pub fn new(doc: &'a Doc, author: &'a mut Author, wall_ms: u64) -> Tx<'a> { Tx { doc, author, wall_ms, ops: Vec::new(), pages: None } }
    pub fn is_empty(&self) -> bool { self.ops.is_empty() }
    pub fn ops(&self) -> &[Op] { &self.ops }
    /// The document this transaction is built against (W5.3: `history::Planned::commit` inside `session::edit`).
    pub fn doc(&self) -> &'a Doc { self.doc }
    /// Adds raw ops (undo/redo inverses, timeline restore plans, imports of the editor).
    pub fn push(&mut self, op: Op) { self.ops.push(op) }

    /// The first local edit on the page shown while every page is deleted restores it (ARCHITECTURE §4.1).
    fn wake(&mut self, page: Id) {
        let projected = self.doc.order() == [page] && self.doc.pages.get(&page).is_some_and(|p| p.deleted.v);
        if projected && !self.ops.iter().any(|op| matches!(op, Op::PageDel { page: p, deleted: false } if *p == page)) {
            self.ops.push(Op::PageDel { page, deleted: false });
        }
    }

    fn put(&mut self, item: Id, page: Id, z: Z, body: Body) {
        self.wake(page);
        self.ops.push(Op::ItemPut { item, page, z, body });
    }

    pub fn add_stroke(&mut self, page: Id, body: Body) -> Id {
        let id = self.author.ids.take();
        self.put(id, page, Z::TOP, body);
        id
    }

    pub fn add_image(&mut self, page: Id, asset: Hash, rect: ImageRect) -> Id {
        self.add_stroke(page, Body::Image { asset, cx: rect.cx, cy: rect.cy, w: rect.w, h: rect.h, rotation: rect.rotation })
    }

    pub fn erase(&mut self, items: &[Id]) {
        for &item in items {
            if let Some(it) = self.doc.live(item) {
                self.wake(it.page.v);
                self.ops.push(Op::ItemDel { item, deleted: true });
            }
        }
    }

    /// Area eraser: deletes `item` and puts the surviving point runs as new items at the original z (B-E4).
    /// Pressure is recovered by locating each fragment in the original; prefer [`Tx::split_ranges`].
    pub fn split(&mut self, item: Id, fragments: Vec<Vec<f64>>) -> Vec<Id> {
        let Some(Body::Stroke { xy, .. }) = self.doc.live(item).map(|i| &i.body.v) else { return Vec::new() };
        let same = |a: &[f64], b: &[f64]| a.iter().zip(b).all(|(x, y)| x.to_bits() == y.to_bits());
        let mut from = 0;
        let ranges: Vec<Option<(u32, u32)>> = fragments
            .iter()
            .map(|f| {
                let n = f.len() / 2;
                let total = xy.len() / 2;
                let found = (from..total).chain(0..from).find(|&s| s + n <= total && same(&xy[s * 2..(s + n) * 2], f))?;
                from = found + n;
                Some((found as u32, (found + n) as u32))
            })
            .collect();
        self.split_with(item, |k| match ranges[k] {
            Some(r) => Fragment::Range(r),
            None => Fragment::Points(fragments[k].clone()),
        }, fragments.len())
    }

    /// Area eraser with the half-open point ranges of `geom::split_ranges` (no copies, pressure kept).
    pub fn split_ranges(&mut self, item: Id, ranges: &[(u32, u32)]) -> Vec<Id> { self.split_with(item, |k| Fragment::Range(ranges[k]), ranges.len()) }

    fn split_with(&mut self, item: Id, fragment: impl Fn(usize) -> Fragment, count: usize) -> Vec<Id> {
        let Some(it) = self.doc.live(item) else { return Vec::new() };
        let Body::Stroke { tool, color, width, xy, p } = &it.body.v else { return Vec::new() };
        let (page, z) = (it.page.v, it.z.v);
        self.wake(page);
        self.ops.push(Op::ItemDel { item, deleted: true });
        let mut ids = Vec::with_capacity(count);
        for k in 0..count {
            let (points, pressure) = match fragment(k) {
                Fragment::Range((a, b)) => {
                    let (a, b) = ((a as usize).min(xy.len() / 2), (b as usize).min(xy.len() / 2));
                    if a >= b {
                        continue;
                    }
                    (xy[a * 2..b * 2].to_vec(), p.as_ref().and_then(|p| p.get(a..b)).map(<[f32]>::to_vec))
                }
                Fragment::Points(points) => (points, None),
            };
            let id = self.author.ids.take();
            let body = Body::Stroke { tool: *tool, color: color.clone(), width: *width, xy: points, p: pressure };
            self.ops.push(Op::ItemPut { item: id, page, z: z.fragment(k as u32), body });
            ids.push(id);
        }
        ids
    }

    /// Selection move/scale/rotate, computed from the originals (z kept).
    pub fn transform(&mut self, items: &[Id], m: Affine) {
        for &item in items {
            let Some(it) = self.doc.live(item) else { continue };
            let body = match &it.body.v {
                Body::Stroke { tool, color, width, xy, p } => Body::Stroke { tool: *tool, color: color.clone(), width: *width, xy: m.points(xy), p: p.clone() },
                Body::Image { asset, cx, cy, w, h, rotation } => {
                    let (cx, cy, w, h, rotation) = m.image(*cx, *cy, *w, *h, *rotation);
                    Body::Image { asset: *asset, cx, cy, w, h, rotation }
                }
                Body::Dropped => continue,
            };
            self.put(item, it.page.v, it.z.v, body);
        }
    }

    /// v5 `changeSelectionColor`: strokes with a truthy colour (pens and highlighters); images untouched.
    pub fn recolor(&mut self, items: &[Id], c: Color) {
        for &item in items {
            let Some(it) = self.doc.live(item) else { continue };
            if let Body::Stroke { tool, color, width, xy, p } = &it.body.v {
                if *color != Color::Css("".into()) && *color != c {
                    let body = Body::Stroke { tool: *tool, color: c.clone(), width: *width, xy: xy.clone(), p: p.clone() };
                    self.put(item, it.page.v, it.z.v, body);
                }
            }
        }
    }

    /// v5: `x + dx, y + dy` on every point (images: on the centre).
    fn shifted(body: &Body, dx: Option<f64>, dy: f64) -> Body {
        let sx = |x: f64| dx.map_or(x, |dx| x + dx);
        match body {
            Body::Stroke { tool, color, width, xy, p } => {
                Body::Stroke { tool: *tool, color: color.clone(), width: *width, xy: xy.chunks_exact(2).flat_map(|q| [sx(q[0]), q[1] + dy]).collect(), p: p.clone() }
            }
            Body::Image { asset, cx, cy, w, h, rotation } => Body::Image { asset: *asset, cx: sx(*cx), cy: cy + dy, w: *w, h: *h, rotation: *rotation },
            Body::Dropped => Body::Dropped,
        }
    }

    /// Cross-page move (v5 `moveSelectionToPage`): y shifted by `dy`, new z = top of the target page, in the
    /// order given.
    pub fn move_to_page(&mut self, items: &[Id], page: Id, dy: f64) {
        for &item in items {
            if let Some(it) = self.doc.live(item) {
                self.put(item, page, Z::TOP, Self::shifted(&it.body.v, None, dy));
            }
        }
    }

    /// v5 `duplicateSelection`: copies offset by `(dx, dy)` with new ids, on top, in the order given.
    pub fn duplicate(&mut self, items: &[Id], dx: f64, dy: f64) -> Vec<Id> {
        let mut ids = Vec::new();
        for &item in items {
            if let Some(it) = self.doc.live(item) {
                ids.push(self.add_stroke(it.page.v, Self::shifted(&it.body.v, Some(dx), dy)));
            }
        }
        ids
    }

    fn order_mut(&mut self) -> &mut Vec<(Pos, Id)> {
        let doc = self.doc;
        self.pages.get_or_insert_with(|| doc.order().iter().filter_map(|id| Some((doc.pages.get(id).filter(|p| !p.deleted.v)?.pos.v.clone(), *id))).collect())
    }

    /// A key for a page inserted at index `at` of the working order. If its neighbours share a key (concurrent
    /// inserts at the same place), the pages that must follow are re-keyed too.
    fn key_at(&mut self, at: usize) -> Pos {
        let order = self.order_mut();
        let lo = at.checked_sub(1).map(|i| order[i].0.clone());
        if lo.is_none() || order.get(at).is_none_or(|hi| lo.as_ref().is_some_and(|lo| lo < &hi.0)) {
            return Pos::between(lo.as_ref(), order.get(at).map(|e| &e.0));
        }
        let end = (at..order.len()).find(|&j| Some(&order[j].0) > lo.as_ref()).unwrap_or(order.len());
        let hi = order.get(end).map(|e| e.0.clone());
        let key = Pos::between(lo.as_ref(), hi.as_ref());
        let mut cur = key.clone();
        let mut moves = Vec::new();
        for entry in &mut order[at..end] {
            cur = Pos::between(Some(&cur), hi.as_ref());
            entry.0 = cur.clone();
            moves.push(Op::PageMove { page: entry.1, pos: cur.clone() });
        }
        self.ops.extend(moves);
        key
    }

    /// New page after `after` (`None` = first; an unknown page = last).
    pub fn page_new(&mut self, after: Option<Id>, tpl: Template, size: (u32, u32), bg: Background) -> Id {
        let page = self.author.ids.take();
        let order = self.order_mut();
        let at = match after {
            None => 0,
            Some(a) => order.iter().position(|e| e.1 == a).map_or(order.len(), |i| i + 1),
        };
        let pos = self.key_at(at);
        self.order_mut().insert(at, (pos.clone(), page));
        self.ops.push(Op::PageNew { page, pos, template: tpl, size: (size.0.max(MIN_PAGE_SIDE), size.1.max(MIN_PAGE_SIDE)), background: bg });
        page
    }

    /// Moves `page` before `before` (`None` or an unknown page = to the end).
    pub fn page_move(&mut self, page: Id, before: Option<Id>) {
        let order = self.order_mut();
        let Some(from) = order.iter().position(|e| e.1 == page) else { return };
        let to = before.and_then(|b| order.iter().position(|e| e.1 == b)).unwrap_or(order.len());
        if to == from || to == from + 1 {
            return; // already there
        }
        order.remove(from);
        let at = if to > from { to - 1 } else { to };
        let pos = self.key_at(at);
        self.order_mut().insert(at, (pos.clone(), page));
        self.ops.push(Op::PageMove { page, pos });
    }

    pub fn page_delete(&mut self, page: Id) {
        self.order_mut().retain(|e| e.1 != page);
        self.ops.push(Op::PageDel { page, deleted: true });
    }

    pub fn page_restore(&mut self, page: Id) { self.ops.push(Op::PageDel { page, deleted: false }) }

    pub fn page_set(&mut self, page: Id, prop: PageProp) {
        let prop = match prop {
            PageProp::Size((w, h)) => PageProp::Size((w.max(MIN_PAGE_SIDE), h.max(MIN_PAGE_SIDE))),
            other => other,
        };
        self.ops.push(Op::PageSet { page, prop });
    }

    pub fn set_name(&mut self, name: &str) {
        if self.doc.name.v != name {
            self.ops.push(Op::DocName { name: name.to_string() });
        }
    }

    pub fn version_add(&mut self, data: Vec<u8>) -> Id {
        let id = self.author.ids.take();
        self.ops.push(Op::VersionAdd { meta: VersionMeta { id, data } });
        id
    }

    pub fn version_del(&mut self, version: Id) { self.ops.push(Op::VersionDel { version }) }

    /// Assigns the batch clock (HLC), the next seq and the stamps of items put "on top".
    pub fn commit(self) -> Batch {
        let clock = self.author.clock.next(self.wall_ms);
        self.author.seq += 1;
        let actor = self.author.actor;
        let mut ops = self.ops;
        for (i, op) in ops.iter_mut().enumerate() {
            if let Op::ItemPut { z, .. } = op {
                if *z == Z::TOP {
                    *z = Z { at: Stamp { clock, actor, idx: i as u32 }, sub: 0 };
                }
            }
        }
        Batch { actor, seq: self.author.seq, clock, ops, stamps: None }
    }
}

enum Fragment {
    Range((u32, u32)),
    Points(Vec<f64>),
}

// ------------------------------------------------------------------------------------------------ §4.4

/// Where batches, snapshots and assets came from (persistence and forwarding decisions).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum PeerRef {
    /// A direct P2P peer (installation id).
    Peer(u64),
    /// Another tab of this browser.
    Tab,
    /// The relay file of an installation.
    Relay(u64),
    /// A v6 export downloaded from Drive.
    Export,
    /// A 5.x import or the migration.
    Import,
}

/// Host blob handle (assets never cross the ABI as bytes inside the model).
pub type BlobHandle = u32;

/// `batches_since`: the requested range has left the journal; the peer needs a snapshot (SYNC §7.3).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Compacted;

#[derive(Clone, PartialEq, Eq, Default, Debug)]
pub struct ReceiveResult {
    pub applied: u32,
    pub duplicate: u32,
    pub queued: u32,
    pub changed_pages: Vec<Id>,
}

/// Implemented by `ihn-app::doc::session`; called by `ihn-sync`.
pub trait Replica {
    /// Always the Drive file id.
    fn doc_id(&self) -> &DocId;
    /// Contiguous + extras.
    fn version_vector(&self) -> &VersionVector;
    /// `Err` only when the range left the journal.
    fn batches_since(&self, have: &VersionVector, max_bytes: usize) -> Result<Vec<EncodedBatch>, Compacted>;
    /// Dedupe, apply (budgeted), persist.
    fn receive(&mut self, b: &[EncodedBatch], from: PeerRef) -> ReceiveResult;
    fn snapshot(&self) -> EncodedSnapshot;
    /// Merge, never replace.
    fn load_snapshot(&mut self, s: &EncodedSnapshot, from: PeerRef) -> ReceiveResult;
    fn missing_assets(&self) -> Vec<Hash>;
    fn put_asset(&mut self, h: Hash, bytes: BlobHandle);
    /// Serve `AssetWant` / relay asset uploads.
    fn asset(&self, h: &Hash) -> Option<BlobHandle>;
    /// Drive capability; viewers never send ops.
    fn can_edit(&self) -> bool;
    /// Journal entries covered by peers/tabs/seal/export.
    fn retain(&mut self, covered: &VersionVector);
}

/// Live ink and live erase previews (SYNC §10.5–10.6), one call per animation frame.
#[derive(Clone, PartialEq, Debug)]
pub enum LivePreview {
    Begin { live: u64, page: Id, tool: Tool, color: Color, width: f64, has_p: bool },
    /// Samples added since the previous frame; `from` = index of the first one.
    Points { live: u64, from: u32, xy: Vec<f64>, p: Option<Vec<f32>> },
    /// Pen up: the batch putting `item` follows.
    End { live: u64, item: Id },
    Cancel { live: u64 },
    /// The complete current hidden set and fragments of an eraser gesture (at most every 50 ms).
    Erase { gesture: u64, page: Id, hidden: Vec<Id>, fragments: Vec<(u64, Body)> },
    EraseEnd { gesture: u64 },
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum PresenceMode {
    Editing,
    #[default]
    Viewing,
    Reading,
}

/// SYNC §10.5 `Presence` frame.
#[derive(Clone, PartialEq, Debug)]
pub struct PresenceState {
    pub mode: PresenceMode,
    pub page: Id,
    pub tool_colour: Color,
    /// ARCHITECTURE §6.6 phase, for remote save rings.
    pub save_phase: u8,
}

/// Transport state of a present remote device (SYNC §9.1 avatar mapping).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PeerLink {
    /// Peer `Open`: `p2p-connected`.
    Connected,
    /// `Offering`/`Answering`/`Connecting`/`Backoff`: `p2p-connecting`.
    Connecting,
    /// `Idle`/`Unreachable`, record present: relay only.
    Relay,
}

/// A present remote device: its relay `Profile` (SYNC §8.3) and latest presence.
#[derive(Clone, PartialEq, Debug)]
pub struct PeerView {
    pub inst: u64,
    pub email: String,
    pub name: String,
    pub photo_url: String,
    /// `b'm'` | `b't'` | `b'p'`.
    pub device_type: u8,
    pub label: String,
    pub link: PeerLink,
    pub presence: Option<PresenceState>,
    pub save_phase: u8,
}

#[derive(Clone, PartialEq, Eq, Default, Debug)]
pub enum ExportPhase {
    #[default]
    Idle,
    Building,
    Checking,
    Merging,
    Uploading,
    Preserving,
    Failed { code: String, detail: String },
}

/// Report shown by `#offline-sync-error`: `<phase> · <code>\n<detail>` (ARCHITECTURE §6.6).
#[derive(Clone, PartialEq, Eq, Default, Debug)]
pub struct SyncError {
    pub phase: String,
    pub code: String,
    pub detail: String,
}

#[derive(Clone, PartialEq, Eq, Default, Debug)]
pub struct SyncStatus {
    /// Own batches not yet sealed in the relay.
    pub unsealed_local: u32,
    pub relay_durable_through: VersionVector,
    pub relay_writing: bool,
    pub not_created: bool,
    pub export: ExportPhase,
    pub export_pending: bool,
    pub online: bool,
    pub auth_paused: bool,
    pub last_error: Option<SyncError>,
    pub peers_connected: u32,
}

/// Implemented by `ihn-sync::Sync`; called by the session. `Effect`/`Io` are `ihn-sync`'s `SyncEffect`/`SyncIo`
/// (SYNC §18.2), which live in that crate.
pub trait SyncEngine {
    type Effect;
    type Io;
    /// After the IDB commit (degraded mode: at once).
    fn local_batch(&mut self, b: &EncodedBatch);
    fn live(&mut self, p: LivePreview);
    fn presence(&mut self, p: PresenceState);
    fn tick(&mut self, now: f64, r: &mut dyn Replica) -> Vec<Self::Effect>;
    fn on_io(&mut self, ev: Self::Io, r: &mut dyn Replica) -> Vec<Self::Effect>;
    fn status(&self) -> SyncStatus;
    fn peers(&self) -> &[PeerView];
}

/// Test command hook: `__IHN_TEST_API__.cmd('core.doc.<name>', args)`.
pub fn test_cmd(_name: &str, _args: &str) -> Option<String> { None }


#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::Writer;
    use crate::snapshot::{self, gen};
    use crate::stamp::CLOCK_HORIZON_MS;
    use crate::testutil::Rng;

    const T0: u64 = 1_700_000_000_000;
    const A4: (u32, u32) = (794, 1123);

    /// One replica: document, writer identity, wall clock and the undo/redo stacks of `editor/history.rs`.
    struct Rep {
        doc: Doc,
        au: Author,
        wall: u64,
        undo: Vec<Vec<Op>>,
        redo: Vec<Vec<Op>>,
        /// The persisted id high-water mark (`kv['ctr:'+actor]`).
        mark: u64,
    }

    impl Rep {
        fn new(n: u64) -> Rep {
            let au = Author::new(IdGen::new(ActorId::tab(0x1000_0000 * (n + 1), n as u8), 0), 0);
            Rep { doc: Doc::default(), au, wall: T0, undo: vec![], redo: vec![], mark: 0 }
        }
        /// Compaction followed by a reload, with only what the store keeps (ARCHITECTURE §7.1): the snapshot
        /// chunks, the actor's seq and the id mark. The clock is not stored; `Author::resume` rebuilds it.
        fn reload(&mut self) {
            self.mark = self.au.ids.persist_mark().unwrap_or(self.mark);
            self.doc = snapshot::encode(&self.doc).decode().expect("own snapshot decodes");
            self.au = Author::resume(IdGen::new(self.au.actor, self.mark), self.au.seq, &self.doc, self.wall);
        }
        /// Merges a snapshot received from a peer; its stamps are observed like those of a batch.
        fn merge(&mut self, other: &Doc) -> Changes {
            self.au.clock.observe(other.max_clock(self.wall), self.wall);
            self.doc.merge(other)
        }
        /// Commits a transaction: inverse against the pre-state, then apply. `None` if it had no ops.
        fn tx(&mut self, f: impl FnOnce(&mut Tx)) -> Option<(Batch, Vec<Op>)> {
            let mut tx = Tx::new(&self.doc, &mut self.au, self.wall);
            f(&mut tx);
            if tx.is_empty() {
                return None;
            }
            let b = tx.commit();
            let inverse = self.doc.inverse(&b);
            assert!(self.doc.apply(&b).applied);
            Some((b, inverse))
        }
        /// A user action: recorded for undo, clears redo.
        fn act(&mut self, f: impl FnOnce(&mut Tx)) -> Option<Batch> {
            let (b, inverse) = self.tx(f)?;
            self.undo.push(inverse);
            self.redo.clear();
            Some(b)
        }
        /// Applies stored inverse ops as a new batch; a step with nothing to do still moves between the stacks.
        fn replay(&mut self, ops: Vec<Op>) -> (Option<Batch>, Vec<Op>) {
            self.tx(|tx| ops.into_iter().for_each(|op| tx.push(op))).map_or((None, vec![]), |(b, inverse)| (Some(b), inverse))
        }
        fn undo(&mut self) -> Option<Batch> {
            let ops = self.undo.pop()?;
            let (b, inverse) = self.replay(ops);
            self.redo.push(inverse);
            b
        }
        fn redo(&mut self) -> Option<Batch> {
            let ops = self.redo.pop()?;
            let (b, inverse) = self.replay(ops);
            self.undo.push(inverse);
            b
        }
        fn recv(&mut self, b: &Batch) -> Changes {
            self.au.clock.observe(b.max_clock(), self.wall);
            self.doc.apply(b)
        }
        fn page(&mut self) -> Id {
            let last = self.doc.order().last().copied();
            let mut id = Id::NONE;
            self.act(|tx| id = tx.page_new(last, Template::Default, A4, Background::Template));
            id
        }
        fn stroke(&mut self, page: Id, xy: &[f64]) -> Id {
            let mut id = Id::NONE;
            self.act(|tx| id = tx.add_stroke(page, pen(xy)));
            id
        }
    }

    fn pen(xy: &[f64]) -> Body { Body::Stroke { tool: Tool::Pen, color: Color::Hex(0x1a1a1a), width: 2.0, xy: xy.to_vec(), p: Some(vec![0.5; xy.len() / 2]) } }
    fn tool(t: Tool, xy: &[f64]) -> Body { Body::Stroke { tool: t, color: Color::Hex(0xffe600), width: 16.0, xy: xy.to_vec(), p: None } }
    fn snap(doc: &Doc) -> Vec<u8> { snapshot::encode(doc).0 }
    /// Ignores a returned id.
    fn ig<T>(_: T) {}

    /// What the user sees: name, page order and properties, and every page's draw list with z and bodies.
    fn visible(doc: &Doc) -> Vec<u8> {
        let mut w = Writer::default();
        w.str(&doc.name.v);
        for page in doc.order() {
            let p = &doc.pages[page];
            w.raw(&page.0.to_le_bytes());
            w.str(&format!("{:?}{:?}{:?}{:?}", p.template.v, p.size.v, p.background.v, p.side_panel.v));
            for d in doc.draw_list(*page).iter() {
                w.raw(&d.id.0.to_le_bytes());
                snapshot::put_z(&mut w, d.z);
                snapshot::put_body(&mut w, &doc.items[&d.id].body.v);
            }
        }
        w.buf
    }

    // ------------------------------------------------------------------------------------ convergence

    /// Two revisions of an imported 5.x document: explicit stamps, legacy ids, R1 newer than R0.
    fn import(rev: u32) -> Vec<Batch> {
        let actor = ActorId::import(&format!("revision-{rev}"));
        let time = T0 - 5_000_000 + rev as u64 * 1000;
        let page = Id::legacy("page_1");
        let line = |y: f64| pen(&[10.0, y, 200.0, y + rev as f64]);
        let mut ops = vec![
            Op::PageNew { page, pos: Pos::rank(0), template: Template::Default, size: A4, background: Background::Template },
            // 5.x-stamped items keep their own stamp in both revisions; stampless ones take the revision's.
            Op::ItemPut { item: Id::legacy("st_a"), page, z: Z { at: Stamp::new(1000, ActorId(77), 0), sub: 0 }, body: pen(&[1.0, 1.0, 2.0, 2.0]) },
            Op::ItemPut { item: Id::legacy("st_b"), page, z: Z { at: Stamp::new(time, actor, 2), sub: 0 }, body: line(50.0) },
            Op::DocName { name: format!("Imported r{rev}") },
        ];
        let mut stamps = vec![Stamp::new(time, actor, 0), Stamp::new(1000, ActorId(77), 0), Stamp::new(time, actor, 2), Stamp::new(time, actor, 3)];
        if rev == 1 {
            ops.push(Op::ItemPut { item: Id::legacy("st_c"), page, z: Z { at: Stamp::new(time, actor, 4), sub: 0 }, body: line(90.0) });
            ops.push(Op::ItemDel { item: Id::legacy("st_a"), deleted: true });
            stamps.extend([Stamp::new(time, actor, 4), Stamp::new(time, actor, 5)]);
        }
        vec![Batch { actor, seq: 1, clock: time, ops, stamps: Some(stamps) }]
    }

    fn pick(rng: &mut Rng, ids: &[Id], max: u64) -> Vec<Id> {
        let mut out: Vec<Id> = (0..1 + rng.below(max)).filter_map(|_| (!ids.is_empty()).then(|| *rng.pick(ids))).collect();
        out.dedup();
        out
    }

    /// One random user action (every `Tx` method, undo, redo) on `rep`.
    fn random_action(rng: &mut Rng, rep: &mut Rep) -> Option<Batch> {
        let pages = rep.doc.order().to_vec();
        if pages.is_empty() {
            return rep.act(|tx| ig(tx.page_new(None, Template::Default, A4, Background::Template)));
        }
        let page = *rng.pick(&pages);
        let items = rep.doc.page_items(page);
        let xy: Vec<f64> = (0..2 * (1 + rng.below(9))).map(|_| rng.range(0.0, 790.0)).collect();
        match rng.below(26) {
            0..=6 => rep.act(|tx| ig(tx.add_stroke(page, if rng.chance(0.15) { gen::stroke(rng, 6) } else { pen(&xy) }))),
            7 => rep.act(|tx| ig(tx.add_image(page, [rng.below(4) as u8; 32], ImageRect { cx: xy[0], cy: xy[1], w: 80.0, h: 60.0, rotation: rng.unit() }))),
            8 | 9 => rep.act(|tx| tx.erase(&pick(rng, &items, 3))),
            10 | 11 => {
                let sweep = Sweep { x0: xy[0], y0: xy[1], x1: rng.range(0.0, 790.0), y1: rng.range(0.0, 790.0) };
                let cuts = rep.doc.split_strokes(page, &sweep, 12.0);
                rep.act(|tx| cuts.iter().for_each(|(id, ranges)| ig(tx.split_ranges(*id, ranges))))
            }
            12 | 13 => {
                let sel = pick(rng, &items, 4);
                let centre = rep.doc.selection_bounds(&sel).map_or((0.0, 0.0), |b| ((b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0));
                let m = Affine::new(centre, rng.range(0.5, 1.5), rng.range(0.5, 1.5), rng.range(-1.0, 1.0), rng.range(-20.0, 20.0), rng.range(-20.0, 20.0));
                rep.act(|tx| tx.transform(&sel, m))
            }
            14 => rep.act(|tx| tx.recolor(&pick(rng, &items, 4), Color::Hex(rng.below(4) as u32 * 0x40_4040))),
            15 | 16 => rep.act(|tx| tx.move_to_page(&pick(rng, &items, 3), *rng.pick(&pages), if rng.chance(0.5) { 1123.0 } else { -1123.0 })),
            17 => rep.act(|tx| ig(tx.duplicate(&pick(rng, &items, 3), 18.0, 18.0))),
            18 => rep.act(|tx| ig(tx.page_new(rng.chance(0.8).then_some(page), gen::template(rng), gen::size(rng), gen::background(rng)))),
            19 => rep.act(|tx| tx.page_move(page, rng.chance(0.7).then(|| *rng.pick(&pages)))),
            20 => rep.act(|tx| {
                let mut dead: Vec<Id> = tx.doc.pages.iter().filter(|(_, p)| p.deleted.v).map(|(id, _)| *id).collect();
                dead.sort_unstable();
                if !dead.is_empty() && rng.chance(0.3) {
                    tx.page_restore(*rng.pick(&dead));
                } else if rng.chance(0.4) || pages.len() > 3 {
                    tx.page_delete(page);
                }
            }),
            21 => rep.act(|tx| match rng.below(4) {
                0 => tx.set_name(["Notes", "Física", "Untitled"][rng.below(3) as usize]),
                1 => tx.page_set(page, PageProp::Template(gen::template(rng))),
                2 => tx.page_set(page, PageProp::Background(gen::background(rng))),
                _ => tx.page_set(page, PageProp::SidePanel(rng.chance(0.5).then(|| vec![rng.next() as u8; 3]))),
            }),
            22 => rep.act(|tx| {
                let versions: Vec<Id> = tx.doc.versions.keys().copied().collect();
                if versions.len() > 2 && rng.chance(0.5) {
                    tx.version_del(*rng.pick(&versions));
                } else {
                    tx.version_add(vec![rng.next() as u8; 5]);
                }
            }),
            23 | 24 => rep.undo(),
            _ => rep.redo(),
        }
    }

    fn converge(seed: u64, replicas: usize, ops_each: usize) {
        let mut rng = Rng::new(seed);
        let mut reps: Vec<Rep> = (0..replicas as u64).map(Rep::new).collect();
        for (i, rep) in reps.iter_mut().enumerate() {
            rep.wall = (T0 as i64 + rng.range(-600_000.0, 600_000.0) as i64) as u64; // ±10 min of skew
            if seed % 3 == 0 && i == 1 {
                rep.wall += 10 * CLOCK_HORIZON_MS; // a device whose clock is 10 days ahead
            }
        }
        let mut all: Vec<Batch> = Vec::new();
        let mut inbox: Vec<Vec<Batch>> = vec![Vec::new(); replicas];
        let send = |b: Batch, from: Option<usize>, all: &mut Vec<Batch>, inbox: &mut Vec<Vec<Batch>>| {
            let wire = snapshot::encode_batch(&b).decode().expect("own batch decodes"); // through the codec
            for (i, q) in inbox.iter_mut().enumerate() {
                if Some(i) != from {
                    q.push(wire.clone());
                }
            }
            all.push(b);
        };
        let imports = [import(0), import(1)];
        for step in 0..replicas * ops_each {
            let r = rng.below(replicas as u64) as usize;
            reps[r].wall += rng.below(3); // several batches within the same millisecond
            if let Some(b) = random_action(&mut rng, &mut reps[r]) {
                send(b, Some(r), &mut all, &mut inbox);
            }
            if step % 400 == 37 {
                // Some replica opens a 5.x revision: every importer derives the identical batch.
                let rev = &imports[rng.below(2) as usize];
                for b in rev {
                    reps[r].recv(b);
                    if !all.contains(b) {
                        send(b.clone(), Some(r), &mut all, &mut inbox);
                    }
                }
            }
            // Delivery: random replica, random message (reordering), sometimes duplicated or lost.
            for _ in 0..rng.below(4) {
                let to = rng.below(replicas as u64) as usize;
                if inbox[to].is_empty() || (seed % 2 == 1 && to == 0 && (step / 500) % 2 == 1) {
                    continue; // replica 0 is partitioned half of the time on odd seeds
                }
                let at = rng.below(inbox[to].len() as u64) as usize;
                match rng.below(10) {
                    0 => drop(inbox[to].swap_remove(at)), // lost; redelivered at the end
                    1 => drop(reps[to].recv(&inbox[to][at].clone())), // delivered, and stays for a duplicate
                    _ => {
                        let b = inbox[to].swap_remove(at);
                        reps[to].recv(&b);
                        assert!(!reps[to].doc.apply(&b).applied, "apply twice = apply once");
                    }
                }
            }
            if step % 250 == 100 {
                // Compaction (bodies of erased items dropped) and a snapshot merged into another replica.
                let (from, to) = (rng.below(replicas as u64) as usize, rng.below(replicas as u64) as usize);
                // The compacting replica also reloads: its writer is rebuilt from the store, clock included.
                reps[from].reload();
                if from != to {
                    let other = snapshot::encode(&reps[from].doc).decode().expect("snapshot decodes");
                    reps[to].merge(&other);
                }
            }
        }
        if ops_each >= 1000 {
            // The generator reaches every op kind, undo-made revivals and import batches.
            let has = |f: &dyn Fn(&Op) -> bool| all.iter().flat_map(|b| &b.ops).any(f);
            assert!(has(&|op| matches!(op, Op::PageNew { .. })) && has(&|op| matches!(op, Op::PageMove { .. })));
            assert!(has(&|op| matches!(op, Op::PageDel { deleted: true, .. })) && has(&|op| matches!(op, Op::PageDel { deleted: false, .. })));
            assert!(has(&|op| matches!(op, Op::PageSet { .. })) && has(&|op| matches!(op, Op::DocName { .. })));
            assert!(has(&|op| matches!(op, Op::ItemPut { .. })) && has(&|op| matches!(op, Op::ItemDel { deleted: true, .. })));
            assert!(has(&|op| matches!(op, Op::ItemDel { deleted: false, .. })) && has(&|op| matches!(op, Op::ItemPut { z, .. } if z.sub > 0)));
            assert!(has(&|op| matches!(op, Op::VersionAdd { .. })) && has(&|op| matches!(op, Op::VersionDel { .. })));
            assert!(all.iter().any(|b| b.stamps.is_some()) && all.len() > replicas * ops_each / 2);
        }
        // Loss with redelivery: everything, in a different random order per replica.
        for rep in &mut reps {
            let mut order: Vec<usize> = (0..all.len()).collect();
            rng.shuffle(&mut order);
            for i in order {
                rep.recv(&all[i]);
            }
        }
        let first = snap(&reps[0].doc);
        for (i, rep) in reps.iter().enumerate() {
            assert!(snap(&rep.doc) == first, "seed {seed}: replica {i} differs from replica 0");
            assert_eq!(rep.doc.order(), reps[0].doc.order(), "seed {seed}");
            assert_eq!(rep.doc.vv.extras().count(), 0, "seed {seed}");
            assert!(rep.doc.vv == reps[0].doc.vv, "seed {seed}");
            for page in rep.doc.pages.keys() {
                assert_eq!(rep.doc.page_items(*page), reps[0].doc.page_items(*page), "seed {seed}");
            }
            // The incrementally maintained indexes equal a rebuild from the registers.
            let mut fresh = rep.doc.clone();
            fresh.reindex();
            assert_eq!(visible(&fresh), visible(&rep.doc), "seed {seed}: replica {i} index drift");
            assert_eq!(fresh.chunk_pages(), rep.doc.chunk_pages());
            if seed % 3 == 0 && i != 1 {
                assert!(rep.au.clock.observed < T0 + 2 * CLOCK_HORIZON_MS, "seed {seed}: far-future clock was observed");
            }
        }
        // R1 is newer than R0 whatever the import order.
        if let Some(b) = reps[0].doc.items.get(&Id::legacy("st_b")).filter(|_| all.contains(&imports[1][0])) {
            assert!(b.body.at >= Stamp::new(T0 - 5_000_000 + 1000, ActorId::import("revision-1"), 2));
        }
    }

    #[test]
    fn replicas_converge_under_reordering_duplication_loss_and_snapshots() {
        for (seed, replicas) in [(1, 2), (2, 3), (3, 4), (4, 5)] {
            converge(seed, replicas, 2000);
        }
        // `IHN_CONVERGE_SEEDS=5000 cargo test -p ihn-core converge` soaks more seeds.
        let extra = std::env::var("IHN_CONVERGE_SEEDS").ok().and_then(|v| v.parse().ok()).unwrap_or(30);
        for seed in 10..10 + extra {
            converge(seed, 2 + (seed % 4) as usize, 120);
        }
    }

    #[test]
    fn imports_of_two_revisions_commute_and_the_newer_wins() {
        let (r0, r1) = (import(0), import(1));
        let (mut x, mut y, mut z) = (Doc::default(), Doc::default(), Doc::default());
        for b in r0.iter().chain(&r1) {
            x.apply(b);
        }
        for b in r1.iter().chain(&r0).chain(&r1) {
            y.apply(b);
        }
        z.apply(&r1[0]);
        assert_eq!(snap(&x), snap(&y));
        assert_eq!(x.name.v, "Imported r1");
        assert_eq!(x.page_items(Id::legacy("page_1")), z.page_items(Id::legacy("page_1")));
        assert_eq!(x.items[&Id::legacy("st_b")].body.v, pen(&[10.0, 50.0, 200.0, 51.0]));
        assert!(x.items[&Id::legacy("st_a")].deleted.v && x.live(Id::legacy("st_c")).is_some());
        assert_eq!(import(1), r1); // deterministic
    }

    // ------------------------------------------------------------------------------------ stamps

    #[test]
    fn same_millisecond_batches_delivered_in_reverse_give_the_same_state() {
        let (mut src, mut b, mut c) = (Rep::new(0), Rep::new(1), Rep::new(2));
        let (mut page, mut item) = (Id::NONE, Id::NONE);
        let mut log = vec![
            src.act(|tx| page = tx.page_new(None, Template::Default, A4, Background::Template)).unwrap(),
            src.act(|tx| item = tx.add_stroke(page, pen(&[0.0, 0.0, 10.0, 10.0]))).unwrap(),
        ];
        // Ten writes to the same two registers, all within one millisecond (the wall clock does not move).
        for i in 0..5 {
            let m = Affine::new((0.0, 0.0), 1.0, 1.0, 0.0, i as f64, 0.0);
            log.push(src.act(|tx| tx.transform(&[item], m)).unwrap());
            log.push(src.act(|tx| tx.set_name(&format!("name {i}"))).unwrap());
        }
        assert_eq!(log[0].clock, T0);
        assert!(log.windows(2).all(|w| w[1].clock == w[0].clock + 1), "the HLC advances past the stuck wall clock");
        for batch in &log {
            b.recv(batch);
        }
        for batch in log.iter().rev() {
            c.recv(batch);
        }
        assert_eq!(snap(&b.doc), snap(&src.doc));
        assert_eq!(snap(&c.doc), snap(&src.doc));
        assert_eq!(visible(&c.doc), visible(&src.doc));
        assert_eq!(c.doc.name.v, "name 4");
        assert_eq!(c.doc.items[&item].body.v.points(), &[10.0, 0.0, 20.0, 10.0]); // 0 + 1 + 2 + 3 + 4
        assert_eq!((c.doc.vv.get(src.au.actor), c.doc.vv.extras().count()), (12, 0));
        // The receivers observed the clocks: their next batch is stamped after everything they saw.
        assert_eq!(c.act(|tx| tx.set_name("later")).unwrap().clock, T0 + 12);
    }

    /// A reload keeps nothing of the writer but seq and id mark. The resumed clock must still stamp above what the
    /// document holds, or the next local edit loses to an older write on every replica.
    #[test]
    fn reload_resumes_the_clock() {
        let (mut fast, mut me) = (Rep::new(0), Rep::new(1));
        fast.wall = T0 + 5 * 60_000; // five minutes ahead: inside the horizon
        let page = fast.page();
        let s = fast.stroke(page, &[0.0, 0.0, 10.0, 10.0]);
        me.merge(&snapshot::encode(&fast.doc).decode().unwrap());
        let shift = |dx: f64| Affine::new((0.0, 0.0), 1.0, 1.0, 0.0, dx, 0.0);
        let first = me.act(|tx| tx.transform(&[s], shift(5.0))).unwrap();
        assert_eq!(first.clock, fast.wall + 2); // after the page and the stroke
        assert_eq!(me.doc.max_clock(me.wall), first.clock);

        me.wall = T0 + 2000;
        me.reload();
        assert_eq!((me.au.clock.last, me.au.clock.observed, me.au.seq), (first.clock, first.clock, 1));
        let mut new = Id::NONE;
        let second = me.act(|tx| {
            tx.transform(&[s], shift(50.0));
            new = tx.add_stroke(page, pen(&[1.0, 1.0, 2.0, 2.0]));
        });
        let second = second.unwrap();
        assert!(second.clock > first.clock && second.seq == 2);
        assert_eq!(me.doc.items[&s].body.v.points(), &[55.0, 0.0, 65.0, 10.0], "the move after the reload was applied");
        assert_eq!(me.doc.page_items(page).last(), Some(&new), "the new stroke is on top");
        assert!(new > s && new.0 >> 64 == me.au.actor.0 as u128);
        // Every replica agrees, in either delivery order.
        fast.recv(&second);
        fast.recv(&first);
        assert_eq!(snap(&fast.doc), snap(&me.doc));

        // The wall clock steps back two days: the device's own writes are beyond the horizon now, and its next
        // batch must still be stamped above them.
        me.wall = T0 - 2 * CLOCK_HORIZON_MS;
        me.reload();
        assert_eq!(me.au.clock.last, second.clock);
        assert!(me.au.clock.observed < second.clock, "stamps beyond the horizon are not observed, own or not");
        let third = me.act(|tx| tx.transform(&[s], shift(100.0))).unwrap();
        assert_eq!(third.clock, second.clock + 1);
        assert_eq!(me.doc.items[&s].body.v.points(), &[155.0, 0.0, 165.0, 10.0]);

        // A far-future stamp of another device stored in the document is not adopted by a reload either.
        let mut ahead = Rep::new(2);
        ahead.wall = T0 + 10 * CLOCK_HORIZON_MS;
        let far = ahead.act(|tx| tx.set_name("from the future")).unwrap();
        me.wall = T0 + 3000;
        me.recv(&far);
        me.reload();
        assert_eq!(me.doc.name.v, "from the future");
        assert_eq!((me.au.clock.last, me.au.clock.observed), (third.clock, third.clock));
        // A new writer on a loaded document (another tab, a second device) starts above it too.
        let other = Author::resume(IdGen::new(ActorId::tab(0x7700_0000, 4), 0), 0, &me.doc, me.wall);
        assert_eq!((other.clock.last, other.clock.observed), (0, third.clock));
        assert_eq!(Doc::default().clock(me.au.actor, T0).next(T0), T0);
        // A saturated clock never panics.
        let mut c = Clock { last: u64::MAX, observed: u64::MAX };
        assert_eq!(c.next(5), u64::MAX);
    }

    #[test]
    fn far_future_clock_is_applied_but_not_observed() {
        let (mut sane, mut ahead) = (Rep::new(0), Rep::new(1));
        ahead.wall = T0 + 10 * CLOCK_HORIZON_MS; // wall + 10 days
        let page = sane.page();
        let created = sane.undo.len();
        let from_future = ahead.act(|tx| tx.set_name("from the future")).unwrap();
        assert_eq!(from_future.clock, T0 + 10 * CLOCK_HORIZON_MS);
        assert!(sane.recv(&from_future).name);
        assert_eq!(sane.doc.name.v, "from the future"); // applied
        assert_eq!(sane.au.clock.observed, 0); // not observed
        let next = sane.act(|tx| ig(tx.add_stroke(page, pen(&[1.0, 1.0])))).unwrap();
        assert!(next.clock < T0 + CLOCK_HORIZON_MS, "the sane device was not dragged forward");
        assert_eq!(sane.undo.len(), created + 1);
        // A clock inside the horizon is observed: the next local batch is stamped after it.
        let mut near = Rep::new(2);
        near.wall = T0 + 60_000;
        let b = near.act(|tx| tx.set_name("a minute ahead")).unwrap();
        sane.recv(&b);
        let after = sane.act(|tx| tx.set_name("mine")).unwrap();
        assert_eq!(after.clock, T0 + 60_001);
        near.recv(&after);
        assert_eq!(near.doc.name.v, "mine");
    }

    // ------------------------------------------------------------------------------------ algebra

    /// Ops of every kind aimed at the same few pages, items and registers.
    fn op_universe() -> Vec<Op> {
        let (p1, p2, i1, i2, v1) = (Id(1), Id(2), Id(11), Id(12), Id(21));
        let z = |c| Z { at: Stamp::new(c, ActorId(5), 0), sub: 0 };
        vec![
            Op::PageNew { page: p1, pos: Pos(vec![10]), template: Template::Default, size: A4, background: Background::Template },
            Op::PageNew { page: p1, pos: Pos(vec![30]), template: Template::Diary, size: (559, 794), background: Background::Custom { asset: [1; 32] } },
            Op::PageNew { page: p2, pos: Pos(vec![20]), template: Template::Agenda, size: (559, 794), background: Background::Template },
            Op::PageMove { page: p1, pos: Pos(vec![25]) },
            Op::PageMove { page: p2, pos: Pos(vec![5]) },
            Op::PageDel { page: p1, deleted: true },
            Op::PageDel { page: p1, deleted: false },
            Op::PageDel { page: p2, deleted: true },
            Op::PageSet { page: p1, prop: PageProp::Template(Template::Agenda) },
            Op::PageSet { page: p1, prop: PageProp::Size((600, 800)) },
            Op::PageSet { page: p1, prop: PageProp::Background(Background::Pdf { asset: [2; 32], page: 3 }) },
            Op::PageSet { page: p2, prop: PageProp::SidePanel(Some(vec![1, 2])) },
            Op::ItemPut { item: i1, page: p1, z: z(3), body: pen(&[1.0, 2.0, 3.0, 4.0]) },
            Op::ItemPut { item: i1, page: p2, z: z(9), body: tool(Tool::Highlighter, &[5.0, 6.0]) },
            Op::ItemPut { item: i2, page: p1, z: z(1), body: Body::Image { asset: [3; 32], cx: 1.0, cy: 2.0, w: 3.0, h: 4.0, rotation: 0.5 } },
            Op::ItemPut { item: i2, page: p1, z: z(4), body: tool(Tool::LegacyCover, &[7.0, 8.0, 9.0, 1.0]) },
            Op::ItemDel { item: i1, deleted: true },
            Op::ItemDel { item: i1, deleted: false },
            Op::ItemDel { item: i2, deleted: true },
            Op::DocName { name: "one".into() },
            Op::DocName { name: "two".into() },
            Op::VersionAdd { meta: VersionMeta { id: v1, data: vec![1] } },
            Op::VersionAdd { meta: VersionMeta { id: v1, data: vec![2, 2] } },
            Op::VersionDel { version: v1 },
        ]
    }

    #[test]
    fn every_pair_of_ops_commutes_and_apply_is_idempotent() {
        let ops = op_universe();
        let batch = |op: &Op, actor: u64, clock: u64| Batch { actor: ActorId(actor << 8), seq: 1, clock, ops: vec![op.clone()], stamps: None };
        // Pre-states: empty, and one where everything already exists.
        let mut full = Doc::default();
        full.apply(&Batch { actor: ActorId(0x900), seq: 1, clock: 50, ops: ops.clone(), stamps: None });
        for base in [Doc::default(), full] {
            for (i, a) in ops.iter().enumerate() {
                for b in &ops[i..] {
                    // Different clocks both ways, and the same clock (the actor breaks the tie).
                    for (ca, cb) in [(100, 200), (200, 100), (100, 100)] {
                        let (ba, bb) = (batch(a, 1, ca), batch(b, 2, cb));
                        let (mut x, mut y) = (base.clone(), base.clone());
                        x.apply(&ba);
                        x.apply(&bb);
                        y.apply(&bb);
                        y.apply(&ba);
                        assert!(snap(&x) == snap(&y), "{a:?} / {b:?} at {ca}/{cb}");
                        assert_eq!(visible(&x), visible(&y), "{a:?} / {b:?} at {ca}/{cb}");
                        let once = snap(&x);
                        assert!(!x.apply(&ba).applied && !x.apply(&bb).applied && snap(&x) == once);
                        // The same ops under new batch ids with the same stamps (a re-import) change nothing either.
                        let stamps = Some(vec![ba.stamp(0)]);
                        let again = Batch { actor: ActorId::import("again"), seq: 1, clock: 0, ops: ba.ops.clone(), stamps };
                        x.apply(&again);
                        let mut expect = Doc::default();
                        expect.merge(&x);
                        assert!(snap(&x)[..] == snap(&expect)[..], "merge of self is identity");
                        assert_eq!(visible(&y), visible(&x), "{a:?} re-applied with the same stamp");
                    }
                }
            }
        }
    }

    #[test]
    fn tombstones_never_resurrect_and_unknown_targets_are_stored() {
        let (page, item) = (Id(1), Id(2));
        let at = |c| Stamp::new(c, ActorId(0x100), 0);
        let one = |seq: u32, clock: u64, op: Op| Batch { actor: ActorId(0x100), seq, clock, ops: vec![op], stamps: None };
        let put = |c: u64| Op::ItemPut { item, page, z: Z { at: at(c), sub: 0 }, body: pen(&[c as f64, 0.0, 1.0, 1.0]) };
        let mut doc = Doc::default();
        // The deletion arrives first, for an item and a page nobody has heard of.
        let ch = doc.apply(&one(3, 30, Op::ItemDel { item, deleted: true }));
        assert_eq!((ch.items.clone(), ch.pages.clone()), (vec![item], vec![Id::NONE]));
        assert!(doc.items[&item].body.v.is_dropped() && doc.order().is_empty());
        // The older put arrives late: stored, still deleted, not drawn.
        doc.apply(&one(1, 10, put(10)));
        assert!(doc.items[&item].deleted.v && doc.live(item).is_none() && doc.page_items(page).is_empty());
        assert_eq!(doc.chunk_items(page), vec![item]);
        assert!(doc.chunk_items(Id::NONE).is_empty());
        // A newer put does not undelete either: `deleted` is its own register.
        doc.apply(&one(4, 40, put(40)));
        assert!(doc.live(item).is_none());
        // The page arrives: it shows, without the deleted item.
        doc.apply(&one(2, 20, Op::PageNew { page, pos: Pos(vec![128]), template: Template::Default, size: A4, background: Background::Template }));
        assert_eq!((doc.order(), doc.page_items(page).len()), (&[page][..], 0));
        // Only a newer undelete revives it; an older one loses.
        doc.apply(&one(5, 25, Op::ItemDel { item, deleted: false }));
        assert!(doc.live(item).is_none());
        doc.apply(&one(6, 50, Op::ItemDel { item, deleted: false }));
        assert_eq!(doc.page_items(page), vec![item]);
        assert_eq!(doc.items[&item].body.v.points()[0], 40.0);
        assert_eq!((doc.vv.get(ActorId(0x100)), doc.take_dirty()), (6, (vec![Id::NONE, page], true)));
        assert_eq!(doc.take_dirty(), (vec![], false));
        // Page ops before PageNew are stored and win or lose by stamp once the page is born.
        let late = Id(9);
        doc.apply(&one(8, 80, Op::PageMove { page: late, pos: Pos(vec![1]) }));
        doc.apply(&one(9, 90, Op::PageSet { page: late, prop: PageProp::Template(Template::Diary) }));
        assert_eq!(doc.order(), &[page]);
        doc.apply(&one(7, 70, Op::PageNew { page: late, pos: Pos(vec![200]), template: Template::Agenda, size: A4, background: Background::Template }));
        assert_eq!(doc.order(), &[late, page]);
        assert_eq!(doc.pages[&late].template.v, Template::Diary);
    }

    // ------------------------------------------------------------------------------------ undo

    #[test]
    fn inverse_of_every_tx_restores_the_visible_state_and_redo_brings_it_back() {
        let mut rng = Rng::new(77);
        let mut rep = Rep::new(0);
        let (p1, p2) = (rep.page(), rep.page());
        let a = rep.stroke(p1, &[10.0, 10.0, 20.0, 20.0, 30.0, 10.0, 40.0, 20.0, 50.0, 10.0]);
        let b = rep.stroke(p1, &[100.0, 100.0, 120.0, 130.0]);
        let mut img = Id::NONE;
        rep.act(|tx| img = tx.add_image(p1, [4; 32], ImageRect { cx: 300.0, cy: 300.0, w: 100.0, h: 50.0, rotation: 0.2 }));
        let hl = rep.act(|tx| ig(tx.add_stroke(p1, tool(Tool::Highlighter, &[5.0, 5.0, 60.0, 60.0])))).map(|b| match &b.ops[0] {
            Op::ItemPut { item, .. } => *item,
            _ => Id::NONE,
        });
        let hl = hl.unwrap();
        let m = Affine::new((30.0, 15.0), 1.5, 0.75, 0.4, 12.0, -7.0);
        type Action<'x> = Box<dyn Fn(&mut Tx) + 'x>;
        let actions: Vec<(&str, Action)> = vec![
            ("add stroke", Box::new(|tx| ig(tx.add_stroke(p1, pen(&[1.0, 2.0, 3.0, 4.0]))))),
            ("add image", Box::new(|tx| ig(tx.add_image(p2, [5; 32], ImageRect { cx: 1.0, cy: 2.0, w: 3.0, h: 4.0, rotation: 0.0 })))),
            ("erase", Box::new(|tx| tx.erase(&[a, img]))),
            ("area erase", Box::new(|tx| ig(tx.split_ranges(a, &[(0, 2), (3, 5)])))),
            ("area erase (points)", Box::new(|tx| ig(tx.split(a, vec![vec![10.0, 10.0, 20.0, 20.0], vec![40.0, 20.0, 50.0, 10.0]])))),
            ("area erase to nothing", Box::new(|tx| ig(tx.split_ranges(b, &[])))),
            ("transform", Box::new(move |tx| tx.transform(&[a, img], m))),
            ("recolour", Box::new(|tx| tx.recolor(&[a, hl, img], Color::Hex(0x002fd9)))),
            ("cross-page move", Box::new(|tx| tx.move_to_page(&[a, img], p2, -1123.0))),
            ("duplicate", Box::new(|tx| ig(tx.duplicate(&[a, img], 18.0, 18.0)))),
            ("rename", Box::new(|tx| tx.set_name("Renamed"))),
            ("page props", Box::new(|tx| {
                tx.page_set(p1, PageProp::Template(Template::Agenda));
                tx.page_set(p1, PageProp::Size((559, 794)));
                tx.page_set(p1, PageProp::Background(Background::Custom { asset: [8; 32] }));
                tx.page_set(p1, PageProp::SidePanel(Some(vec![1])));
            })),
            ("two writes to one item", Box::new(move |tx| {
                tx.transform(&[a], m);
                tx.recolor(&[a], Color::Hex(0xff0000));
                tx.erase(&[a]);
            })),
            ("create and delete", Box::new(|tx| {
                let id = tx.add_stroke(p1, pen(&[0.0, 0.0]));
                tx.push(Op::ItemDel { item: id, deleted: true });
            })),
        ];
        for (name, action) in &actions {
            let before = visible(&rep.doc);
            assert!(rep.act(|tx| action(tx)).is_some(), "{name}: no ops");
            let after = visible(&rep.doc);
            if *name != "create and delete" {
                assert!(after != before, "{name} changed nothing");
            }
            // Sometimes a compaction happens in between: undo must not depend on bodies the snapshot dropped.
            if rng.chance(0.5) {
                rep.reload();
            }
            assert!(rep.undo().is_some(), "{name}");
            assert!(visible(&rep.doc) == before, "{name}: undo did not restore the pre-state");
            rep.redo();
            assert!(visible(&rep.doc) == after, "{name}: redo did not restore the post-state");
            rep.undo();
            assert!(visible(&rep.doc) == before, "{name}: second undo");
        }
        // Page operations and timeline entries are not undoable (5.18.0).
        let page_ops = rep.tx(|tx| {
            let p = tx.page_new(Some(p2), Template::Default, A4, Background::Template);
            tx.page_move(p, Some(p1));
            tx.page_delete(p2);
            tx.version_add(vec![1]);
        });
        assert_eq!(page_ops.unwrap().1, vec![]);
    }

    #[test]
    fn inverse_ops_follow_the_sync_table() {
        let mut rep = Rep::new(0);
        let page = rep.page();
        let s = rep.stroke(page, &[1.0, 1.0, 2.0, 2.0]);
        let old = rep.doc.items[&s].clone();
        let au = rep.au.actor;
        let batch = |ops: Vec<Op>| Batch { actor: au, seq: 99, clock: T0 + 99, ops, stamps: None };
        let put_old = Op::ItemPut { item: s, page, z: old.z.v, body: old.body.v.clone() };
        let put_new = Op::ItemPut { item: s, page: Id(5), z: Z::default(), body: pen(&[9.0, 9.0]) };
        // Creation → ItemDel{true}; replace → the old page, z and body.
        assert_eq!(rep.doc.inverse(&batch(vec![Op::ItemPut { item: Id(404), page, z: Z::default(), body: pen(&[]) }])), vec![Op::ItemDel { item: Id(404), deleted: true }]);
        assert_eq!(rep.doc.inverse(&batch(vec![put_new.clone()])), vec![put_old.clone()]);
        // Deletion → ItemPut (the body travels) THEN ItemDel{false}.
        assert_eq!(rep.doc.inverse(&batch(vec![Op::ItemDel { item: s, deleted: true }])), vec![put_old.clone(), Op::ItemDel { item: s, deleted: false }]);
        // Nothing to invert: deleting the unknown, undeleting the live.
        assert_eq!(rep.doc.inverse(&batch(vec![Op::ItemDel { item: Id(404), deleted: true }, Op::ItemDel { item: s, deleted: false }])), vec![]);
        // Reverse order across ops; one inverse per register however often the batch writes it.
        let inv = rep.doc.inverse(&batch(vec![Op::DocName { name: "x".into() }, put_new.clone(), put_new, Op::DocName { name: "y".into() }]));
        assert_eq!(inv, vec![put_old.clone(), Op::DocName { name: String::new() }]);
        // ItemDel{false} on a deleted item → ItemDel{true}; its ItemPut companion restores the old body.
        rep.act(|tx| tx.erase(&[s]));
        let inv = rep.doc.inverse(&batch(vec![put_old.clone(), Op::ItemDel { item: s, deleted: false }]));
        assert_eq!(inv, vec![Op::ItemDel { item: s, deleted: true }, put_old]);
        // PageSet → the old value, for a born page only.
        let inv = rep.doc.inverse(&batch(vec![Op::PageSet { page, prop: PageProp::Size((100, 100)) }, Op::PageSet { page: Id(404), prop: PageProp::Size((100, 100)) }]));
        assert_eq!(inv, vec![Op::PageSet { page, prop: PageProp::Size(A4) }]);
    }

    #[test]
    fn undo_of_an_erase_revives_the_body_on_a_replica_that_dropped_it() {
        let (mut a, mut b) = (Rep::new(0), Rep::new(1));
        let page = a.page();
        let s = a.stroke(page, &[10.0, 10.0, 90.0, 90.0]);
        let erase = a.act(|tx| tx.erase(&[s])).unwrap();
        // b joins from a's snapshot: the erased stroke is a tombstone without a body.
        b.doc.merge(&snapshot::encode(&a.doc).decode().unwrap());
        assert!(b.doc.items[&s].body.v.is_dropped() && b.doc.items[&s].deleted.v);
        assert!(b.doc.page_items(page).is_empty());
        // The original batches arriving again (anti-entropy overlap) are duplicates by VV.
        assert!(!b.recv(&erase).applied);
        // a undoes the erase: ItemPut (with the body) then ItemDel{false}.
        let undo = a.undo().unwrap();
        assert!(matches!(&undo.ops[..], [Op::ItemPut { body: Body::Stroke { .. }, .. }, Op::ItemDel { deleted: false, .. }]));
        let ch = b.recv(&snapshot::encode_batch(&undo).decode().unwrap());
        assert_eq!((ch.pages, ch.items), (vec![page], vec![s]));
        assert_eq!(b.doc.page_items(page), vec![s]);
        assert_eq!(b.doc.items[&s].body.v, pen(&[10.0, 10.0, 90.0, 90.0]));
        assert_eq!(visible(&b.doc), visible(&a.doc));
        assert_eq!(snap(&b.doc), snap(&a.doc));
        // And a's redo erases it everywhere again.
        let redo = a.redo().unwrap();
        b.recv(&redo);
        assert!(b.doc.page_items(page).is_empty() && snap(&b.doc) == snap(&a.doc));
    }

    #[test]
    fn undo_under_concurrent_remote_edits_keeps_the_remote_edits() {
        let (mut a, mut b) = (Rep::new(0), Rep::new(1));
        let page = a.page();
        let mine = a.stroke(page, &[10.0, 10.0, 20.0, 20.0]);
        let seed: Vec<Batch> = snapshot::encode(&a.doc).decode().map(|d| b.doc.merge(&d)).map(|_| vec![]).unwrap();
        assert!(seed.is_empty());
        // a recolours its stroke; meanwhile b draws, renames and transforms a's stroke.
        let recolour = a.act(|tx| tx.recolor(&[mine], Color::Hex(0xff0000))).unwrap();
        b.wall += 5;
        let theirs = b.act(|tx| ig(tx.add_stroke(page, pen(&[50.0, 50.0, 60.0, 60.0])))).unwrap();
        let rename = b.act(|tx| tx.set_name("B's title")).unwrap();
        for batch in [&theirs, &rename] {
            a.recv(batch);
        }
        b.recv(&recolour);
        // a undoes the recolour: a new write, re-stamped, that restores the colour and nothing else.
        let undo = a.undo().unwrap();
        b.recv(&undo);
        for doc in [&a.doc, &b.doc] {
            assert_eq!(doc.page_items(page).len(), 2);
            assert_eq!(doc.name.v, "B's title");
            assert!(matches!(&doc.items[&mine].body.v, Body::Stroke { color: Color::Hex(0x1a1a1a), .. }));
        }
        assert_eq!(snap(&a.doc), snap(&b.doc));
        // Undo re-stamps, so it wins over a concurrent remote write to the same register made before it.
        let m = Affine::new((0.0, 0.0), 2.0, 2.0, 0.0, 0.0, 0.0);
        let erase = a.act(|tx| tx.erase(&[mine])).unwrap();
        let concurrent = b.act(|tx| tx.transform(&[mine], m)).unwrap(); // b has not seen the erase
        a.recv(&concurrent);
        a.wall += 50;
        let undo = a.undo().unwrap();
        b.recv(&erase);
        b.recv(&undo);
        assert_eq!(snap(&a.doc), snap(&b.doc));
        assert_eq!(a.doc.items[&mine].body.v.points(), &[10.0, 10.0, 20.0, 20.0], "the undo's body is the newest write");
        assert_eq!(a.doc.page_items(page).len(), 2);
    }

    // ------------------------------------------------------------------------------------ z rules, queries

    #[test]
    fn z_rules_area_erase_keeps_position_and_cross_page_move_goes_on_top() {
        let mut rep = Rep::new(0);
        let (p1, p2) = (rep.page(), rep.page());
        let ink = rep.stroke(p1, &[0.0, 0.0, 10.0, 0.0, 20.0, 0.0, 30.0, 0.0, 40.0, 0.0]);
        let mut hl = Id::NONE;
        rep.act(|tx| hl = tx.add_stroke(p1, tool(Tool::Highlighter, &[0.0, 5.0, 40.0, 5.0])));
        let top = rep.stroke(p1, &[0.0, 9.0, 40.0, 9.0]);
        let on_p2 = rep.stroke(p2, &[1.0, 1.0, 2.0, 2.0]);
        assert_eq!(rep.doc.page_items(p1), vec![ink, hl, top]);
        // Area-erase the bottom stroke: its fragments stay below the highlighter (B-E4), in run order.
        let mut frags = vec![];
        rep.act(|tx| frags = tx.split_ranges(ink, &[(0, 2), (3, 5)]));
        assert_eq!(rep.doc.page_items(p1), vec![frags[0], frags[1], hl, top]);
        let z0 = rep.doc.items[&ink].z.v;
        assert_eq!((rep.doc.items[&frags[0]].z.v, rep.doc.items[&frags[1]].z.v), (z0.fragment(0), z0.fragment(1)));
        assert_eq!(rep.doc.items[&frags[1]].body.v, pen(&[30.0, 0.0, 40.0, 0.0]));
        // A fragment of a fragment still sits at the original position.
        let mut again = vec![];
        rep.act(|tx| again = tx.split(frags[1], vec![vec![30.0, 0.0, 40.0, 0.0]]));
        assert_eq!(rep.doc.page_items(p1), vec![frags[0], again[0], hl, top]);
        assert!(matches!(&rep.doc.items[&again[0]].body.v, Body::Stroke { p: Some(p), .. } if p.len() == 2), "pressure travels with the points");
        rep.undo();
        // Undo of the area erase: the original returns to its original z, not to the top.
        rep.undo();
        assert_eq!(rep.doc.page_items(p1), vec![ink, hl, top]);
        assert_eq!(rep.doc.items[&ink].z.v, z0);
        rep.redo();
        assert_eq!(rep.doc.page_items(p1), vec![frags[0], frags[1], hl, top]);
        rep.undo();
        // Cross-page move: on top of the target page, in the order given, y shifted; z = the move's stamp.
        let moved = rep.act(|tx| tx.move_to_page(&[ink, hl], p2, -1123.0)).unwrap();
        assert_eq!(rep.doc.page_items(p2), vec![on_p2, ink, hl]);
        assert_eq!(rep.doc.page_items(p1), vec![top]);
        assert_eq!(rep.doc.items[&ink].z.v, Z { at: moved.stamp(0), sub: 0 });
        assert_eq!(rep.doc.items[&hl].body.v.points(), &[0.0, -1118.0, 40.0, -1118.0]);
        // Transform and recolour within a page keep z; a duplicate goes on top.
        let z_before = rep.doc.items[&ink].z.v;
        rep.act(|tx| tx.transform(&[ink], Affine::new((0.0, 0.0), 1.0, 1.0, 0.0, 3.0, 3.0)));
        rep.act(|tx| tx.recolor(&[ink], Color::Hex(0x00ff00)));
        assert_eq!(rep.doc.items[&ink].z.v, z_before);
        let mut dup = vec![];
        rep.act(|tx| dup = tx.duplicate(&[on_p2], 18.0, 18.0));
        assert_eq!(rep.doc.page_items(p2), vec![on_p2, ink, hl, dup[0]]);
        assert_eq!(rep.doc.items[&dup[0]].body.v.points(), &[19.0, 19.0, 20.0, 20.0]);
        // Undo of the cross-page move puts both back at their old z on the old page.
        for _ in 0..4 {
            rep.undo();
        }
        assert_eq!(rep.doc.page_items(p1), vec![ink, hl, top]);
        assert_eq!(rep.doc.page_items(p2), vec![on_p2]);
    }

    #[test]
    fn draw_list_is_covers_then_images_then_strokes_each_by_z() {
        let mut rep = Rep::new(0);
        let page = rep.page();
        let mut ids = vec![];
        rep.act(|tx| {
            ids.push(tx.add_stroke(page, pen(&[0.0, 0.0, 5.0, 5.0])));
            ids.push(tx.add_image(page, [1; 32], ImageRect { cx: 50.0, cy: 50.0, w: 20.0, h: 10.0, rotation: 0.0 }));
            ids.push(tx.add_stroke(page, tool(Tool::LegacyCover, &[0.0, 0.0, 9.0, 9.0])));
            ids.push(tx.add_stroke(page, tool(Tool::LegacyEraseArea, &[1.0, 1.0, 2.0, 2.0])));
            ids.push(tx.add_image(page, [2; 32], ImageRect { cx: 10.0, cy: 10.0, w: 4.0, h: 4.0, rotation: 0.0 }));
            ids.push(tx.add_stroke(page, tool(Tool::LegacyCover, &[3.0, 3.0])));
        });
        let list = rep.doc.draw_list(page);
        let run = |r: &[DrawRef]| r.iter().map(|d| d.id).collect::<Vec<_>>();
        assert_eq!((run(list.covers), run(list.images), run(list.strokes)), (vec![ids[2], ids[5]], vec![ids[1], ids[4]], vec![ids[0], ids[3]]));
        assert_eq!(rep.doc.page_items(page), vec![ids[2], ids[5], ids[1], ids[4], ids[0], ids[3]]);
        assert_eq!((list.len(), list.is_empty(), rep.doc.draw_list(Id(999)).len()), (6, false, 0));
        assert_eq!(list.images[0].bounds, [40.0, 45.0, 60.0, 55.0]);
        assert_eq!((list.strokes[0].bounds, list.strokes[0].width), ([0.0, 0.0, 5.0, 5.0], 2.0));
        // Same clock, lower op index = lower z; ties never happen, but (Z, Id) keeps the order total.
        assert!(list.covers[0].z < list.covers[1].z);
        // A remote item with an older z lands below, wherever it arrives.
        let below = Batch { actor: ActorId(0x100), seq: 1, clock: 5, ops: vec![Op::ItemPut { item: Id(7), page, z: Z { at: Stamp::new(5, ActorId(0x100), 0), sub: 0 }, body: pen(&[8.0, 8.0]) }], stamps: None };
        rep.recv(&below);
        assert_eq!(run(rep.doc.draw_list(page).strokes), vec![Id(7), ids[0], ids[3]]);
        // A transform updates the cached bounds.
        rep.act(|tx| tx.transform(&[ids[0]], Affine::new((0.0, 0.0), 2.0, 2.0, 0.0, 100.0, 0.0)));
        assert_eq!(rep.doc.draw_list(page).strokes[1].bounds, [100.0, 0.0, 110.0, 10.0]);
    }

    #[test]
    fn hit_lasso_and_selection_queries_follow_5_18() {
        let mut rep = Rep::new(0);
        let page = rep.page();
        let low = rep.stroke(page, &[0.0, 100.0, 200.0, 100.0]);
        let high = rep.stroke(page, &[100.0, 0.0, 100.0, 200.0]);
        let dot = rep.stroke(page, &[300.0, 300.0]);
        let mut extra = vec![];
        rep.act(|tx| {
            extra.push(tx.add_stroke(page, tool(Tool::LegacyEraseArea, &[0.0, 100.0, 200.0, 100.0])));
            extra.push(tx.add_stroke(page, tool(Tool::LegacyCover, &[0.0, 100.0, 200.0, 100.0])));
            extra.push(tx.add_image(page, [1; 32], ImageRect { cx: 100.0, cy: 100.0, w: 40.0, h: 20.0, rotation: 0.0 }));
        });
        let hit = |x0, y0, x1, y1, r| rep.doc.hit_strokes(page, x0, y0, x1, y1, r);
        // One sample (v5): centre-line within r; topmost first; legacy eraser strokes, covers and images never hit.
        assert_eq!(hit(100.0, 100.0, 100.0, 100.0, 5.0), vec![high, low]);
        assert_eq!(hit(50.0, 105.0, 50.0, 105.0, 5.0), vec![low]);
        assert_eq!(hit(50.0, 105.01, 50.0, 105.01, 5.0), vec![]); // stroke width is not added
        assert_eq!(hit(303.0, 304.0, 303.0, 304.0, 5.0), vec![dot]);
        assert_eq!(hit(303.0, 304.1, 303.0, 304.1, 5.0), vec![]);
        // Swept between two samples (B-E6): a fast swipe no longer skips the thin strokes between them.
        assert_eq!(hit(50.0, 60.0, 50.0, 140.0, 2.0), vec![low]);
        assert_eq!(hit(60.0, 60.0, 140.0, 140.0, 2.0), vec![high, low]);
        assert_eq!(hit(250.0, 250.0, 350.0, 350.0, 1.0), vec![dot]);
        assert_eq!(hit(250.0, 250.0, 260.0, 260.0, 1.0), vec![]);
        assert_eq!(rep.doc.hit_strokes(Id(404), 0.0, 0.0, 9.0, 9.0, 99.0), vec![]);
        // Area eraser candidates: ranges for changed strokes only, dots and legacy strokes untouched.
        let cuts = rep.doc.split_strokes(page, &Sweep::point(100.0, 100.0), 250.0);
        assert_eq!(cuts, vec![(low, vec![]), (high, vec![])]);
        assert_eq!(rep.doc.split_strokes(page, &Sweep::point(100.0, 50.0), 5.0), vec![(high, vec![])]);
        assert_eq!(rep.doc.split_strokes(page, &Sweep::point(500.0, 500.0), 5.0), vec![]);
        // Lasso: images first, then strokes with any point inside; needs more than two points.
        let around_dot = [290.0, 290.0, 310.0, 290.0, 310.0, 310.0, 290.0, 310.0];
        assert_eq!(rep.doc.lasso(page, &around_dot), vec![dot]);
        assert_eq!(rep.doc.lasso(page, &around_dot[..4]), vec![]);
        let centre = [90.0, 90.0, 110.0, 90.0, 110.0, 110.0, 90.0, 110.0];
        assert_eq!(rep.doc.lasso(page, &centre), vec![extra[2]]); // strokes cross it but have no sample inside
        let left = [-10.0, 90.0, 10.0, 90.0, 10.0, 110.0, -10.0, 110.0];
        assert_eq!(rep.doc.lasso(page, &left), vec![low]);
        let all = [-10.0, -10.0, 400.0, -10.0, 400.0, 400.0, -10.0, 400.0];
        assert_eq!(rep.doc.lasso(page, &all), vec![extra[2], low, high, dot]);
        // Selection bounds: stroke points and image corners; nothing selected → None.
        assert_eq!(rep.doc.selection_bounds(&[low, dot]), Some([0.0, 100.0, 300.0, 300.0]));
        assert_eq!(rep.doc.selection_bounds(&[extra[2]]), Some([80.0, 90.0, 120.0, 110.0]));
        assert_eq!(rep.doc.selection_bounds(&[Id(404)]), None);
        // Erased strokes leave every query.
        rep.act(|tx| tx.erase(&[low, dot]));
        assert_eq!(rep.doc.hit_strokes(page, 100.0, 100.0, 100.0, 100.0, 5.0), vec![high]);
        assert_eq!(rep.doc.lasso(page, &all), vec![extra[2], high]);
    }

    // ------------------------------------------------------------------------------------ pages

    #[test]
    fn page_order_new_move_delete_and_the_all_deleted_projection() {
        let mut rep = Rep::new(0);
        let mut p = vec![];
        // Several pages chained in one batch (import PDF): each after the previous.
        let batch = rep
            .act(|tx| {
                let mut last = None;
                for _ in 0..4 {
                    last = Some(tx.page_new(last, Template::Default, (10, 5000), Background::Template));
                    p.push(last.unwrap());
                }
            })
            .unwrap();
        assert_eq!(rep.doc.order(), &p[..]);
        assert_eq!(rep.doc.pages[&p[0]].size.v, (64, 5000), "sizes are integers ≥ 64");
        assert_eq!(batch.ops.len(), 4);
        // New page first, in the middle, after an unknown page (= last).
        let (mut first, mut mid, mut end) = (Id::NONE, Id::NONE, Id::NONE);
        rep.act(|tx| {
            first = tx.page_new(None, Template::Agenda, A4, Background::Template);
            mid = tx.page_new(Some(p[1]), Template::Default, A4, Background::Template);
            end = tx.page_new(Some(Id(404)), Template::Default, A4, Background::Template);
        });
        assert_eq!(rep.doc.order(), &[first, p[0], p[1], mid, p[2], p[3], end]);
        // Moves: before a page, to the end, no-ops.
        rep.act(|tx| tx.page_move(end, Some(p[0])));
        assert_eq!(rep.doc.order(), &[first, end, p[0], p[1], mid, p[2], p[3]]);
        rep.act(|tx| tx.page_move(first, None));
        assert_eq!(rep.doc.order(), &[end, p[0], p[1], mid, p[2], p[3], first]);
        assert!(rep.act(|tx| tx.page_move(p[0], Some(p[1]))).is_none() && rep.act(|tx| tx.page_move(first, None)).is_none());
        assert!(rep.act(|tx| tx.page_move(Id(404), None)).is_none());
        // Delete and restore.
        rep.act(|tx| tx.page_delete(mid));
        assert_eq!(rep.doc.order(), &[end, p[0], p[1], p[2], p[3], first]);
        rep.act(|tx| tx.page_restore(mid));
        assert_eq!(rep.doc.order(), &[end, p[0], p[1], mid, p[2], p[3], first]);
        // Every page deleted: the page deleted last is shown (projection only).
        let all = rep.doc.order().to_vec();
        for page in all.iter().filter(|id| **id != p[2]) {
            rep.act(|tx| tx.page_delete(*page));
        }
        rep.act(|tx| tx.page_delete(p[2]));
        assert_eq!(rep.doc.order(), &[p[2]]);
        assert!(rep.doc.pages[&p[2]].deleted.v);
        // The first local edit on it restores it.
        let b = rep.act(|tx| ig(tx.add_stroke(p[2], pen(&[1.0, 1.0])))).unwrap();
        assert!(matches!(&b.ops[..], [Op::PageDel { deleted: false, .. }, Op::ItemPut { .. }]));
        assert!(!rep.doc.pages[&p[2]].deleted.v && rep.doc.order() == [p[2]]);
        // Not undoable as a page operation: undo removes the stroke, the page stays.
        rep.undo();
        assert!(rep.doc.order() == [p[2]] && rep.doc.page_items(p[2]).is_empty() && !rep.doc.pages[&p[2]].deleted.v);
    }

    #[test]
    fn concurrent_inserts_at_one_place_share_a_key_and_later_inserts_still_land_between() {
        let (mut a, mut b) = (Rep::new(0), Rep::new(1));
        let p1 = a.page();
        let p2 = a.page();
        b.doc.merge(&snapshot::encode(&a.doc).decode().unwrap());
        // Both insert after p1 without seeing each other: same key, order decided by id.
        let (mut x, mut y) = (Id::NONE, Id::NONE);
        let bx = a.act(|tx| x = tx.page_new(Some(p1), Template::Default, A4, Background::Template)).unwrap();
        let by = b.act(|tx| y = tx.page_new(Some(p1), Template::Default, A4, Background::Template)).unwrap();
        a.recv(&by);
        b.recv(&bx);
        assert_eq!(a.doc.pages[&x].pos.v, a.doc.pages[&y].pos.v);
        assert_eq!(a.doc.order(), &[p1, x, y, p2]);
        assert_eq!(b.doc.order(), a.doc.order());
        // A page inserted between the twins re-keys the one that must follow.
        let mut between = Id::NONE;
        let batch = a.act(|tx| between = tx.page_new(Some(x), Template::Default, A4, Background::Template)).unwrap();
        assert!(batch.ops.iter().any(|op| matches!(op, Op::PageMove { page, .. } if *page == y)));
        b.recv(&batch);
        assert_eq!(a.doc.order(), &[p1, x, between, y, p2]);
        assert_eq!(b.doc.order(), a.doc.order());
        // Moving p2 between x and `between` works with ordinary keys again.
        let mv = b.act(|tx| tx.page_move(p2, Some(between))).unwrap();
        a.recv(&mv);
        assert_eq!(a.doc.order(), &[p1, x, p2, between, y]);
        assert_eq!(snap(&a.doc), snap(&b.doc));
        // Concurrent move and delete of the same page: both registers apply, the page stays deleted.
        let mv = a.act(|tx| tx.page_move(y, Some(p1))).unwrap();
        let del = b.act(|tx| tx.page_delete(y)).unwrap();
        a.recv(&del);
        b.recv(&mv);
        assert_eq!(a.doc.order(), &[p1, x, p2, between]);
        assert_eq!(snap(&a.doc), snap(&b.doc));
    }

    #[test]
    fn changes_report_what_a_batch_touched() {
        let mut rep = Rep::new(0);
        let (p1, p2) = (rep.page(), rep.page());
        let s = rep.stroke(p1, &[1.0, 1.0, 2.0, 2.0]);
        rep.doc.take_dirty();
        let (mut tx_author, doc) = (rep.au.clone(), rep.doc.clone());
        let mut tx = Tx::new(&doc, &mut tx_author, rep.wall);
        tx.move_to_page(&[s], p2, 0.0);
        tx.set_name("n");
        tx.page_set(p1, PageProp::Template(Template::Diary));
        let b = tx.commit();
        let ch = rep.doc.apply(&b);
        assert_eq!(ch, Changes { pages: vec![p1, p2], items: vec![s], page_props: vec![p1], order: false, name: true, versions: false, applied: true });
        assert!(ch.is_structural());
        assert_eq!(rep.doc.take_dirty(), (vec![p1, p2], true));
        assert_eq!((rep.doc.chunk_items(p1), rep.doc.chunk_items(p2), rep.doc.chunk_pages()), (vec![], vec![s], vec![p2]));
        assert_eq!(rep.doc.apply(&b), Changes::default());
        // A pen stroke is not structural (the undo stack survives remote ink).
        let ink = Batch { actor: ActorId(0x300), seq: 1, clock: T0, ops: vec![Op::ItemPut { item: Id(1), page: p1, z: Z::default(), body: pen(&[0.0, 0.0]) }], stamps: None };
        assert!(!rep.doc.apply(&ink).is_structural());
        let del = Batch { actor: ActorId(0x300), seq: 2, clock: T0 + 1, ops: vec![Op::PageDel { page: p2, deleted: true }], stamps: None };
        let ch = rep.doc.apply(&del);
        assert!(ch.order && ch.is_structural() && ch.pages.is_empty());
        // A large remote batch applied in slices, or moved in, ends in the same state as `apply`.
        let mut rng = Rng::new(4);
        let big = Batch { actor: ActorId(0x400), seq: 1, clock: T0 + 9, ops: (0..300).map(|_| gen::op(&mut rng)).collect(), stamps: None };
        let (mut whole, mut sliced, mut owned) = (rep.doc.clone(), rep.doc.clone(), rep.doc.clone());
        whole.apply(&big);
        let mut hollow = big.clone();
        let mut touched = BTreeSet::new();
        for start in (0..300).step_by(64) {
            assert!(!sliced.vv.contains(big.actor, 1));
            let ch = sliced.apply_part(&mut hollow, start..start + 64);
            assert!(ch.applied);
            touched.extend(ch.items);
            let mut fresh = sliced.clone();
            fresh.reindex();
            assert_eq!(visible(&fresh), visible(&sliced), "indexes are consistent after every slice");
        }
        assert!(sliced.vv.contains(big.actor, 1) && !sliced.apply_part(&mut hollow, 0..300).applied);
        assert!(owned.apply_owned(big.clone()).applied && !owned.apply_owned(big).applied);
        assert_eq!(snap(&sliced), snap(&whole));
        assert_eq!(snap(&owned), snap(&whole));
        assert_eq!(visible(&sliced), visible(&whole));
        assert!(!touched.is_empty());
        // Gaps: a batch above a gap applies at once and sits in the extras until the gap fills.
        let late = Batch { actor: ActorId(0x300), seq: 4, clock: T0 + 1000, ops: vec![Op::DocName { name: "late".into() }], stamps: None };
        assert!(rep.doc.apply(&late).name);
        assert_eq!((rep.doc.vv.get(ActorId(0x300)), rep.doc.vv.contains(ActorId(0x300), 4)), (2, true));
        let fill = Batch { actor: ActorId(0x300), seq: 3, clock: T0 + 500, ops: vec![Op::DocName { name: "early".into() }], stamps: None };
        assert!(!rep.doc.apply(&fill).name);
        assert_eq!((rep.doc.vv.get(ActorId(0x300)), rep.doc.name.v.as_str()), (4, "late"));
    }

    /// Timing probe, not a gate: `cargo test --release -p ihn-core perf_probe -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn perf_probe() {
        use std::time::Instant;
        let mut rng = Rng::new(1);
        let mut rep = Rep::new(0);
        let page = rep.page();
        let stroke = |rng: &mut Rng| {
            let (x, y) = (rng.range(0.0, 780.0), rng.range(0.0, 1100.0));
            pen(&(0..60).flat_map(|i| [x + i as f64 * 0.5, y + (i % 7) as f64]).collect::<Vec<_>>())
        };
        // A remote batch of 20,000 ops arriving at once (import), in shuffled z order.
        let mut ops: Vec<Op> = (0..20_000u32)
            .map(|i| Op::ItemPut { item: Id(1000 + i as u128), page, z: Z { at: Stamp::new(i as u64, ActorId(0x300), 0), sub: 0 }, body: stroke(&mut rng) })
            .collect();
        rng.shuffle(&mut ops);
        let big = Batch { actor: ActorId(0x300), seq: 1, clock: 1, ops, stamps: None };
        let t = Instant::now();
        let enc = snapshot::encode_batch(&big);
        let dec = enc.decode().unwrap();
        println!("20k-op batch: encode+decode {:?} ({} bytes)", t.elapsed(), enc.0.len());
        let t = Instant::now();
        rep.doc.clone().apply(&dec);
        println!("20k-op batch: clone of an empty doc + apply (copying) {:?}", t.elapsed());
        let t = Instant::now();
        let mut dec = dec;
        for start in (0..20_000).step_by(500) {
            rep.doc.apply_part(&mut dec, start..start + 500);
        }
        println!("20k-op batch: apply in 40 slices (moving) {:?}", t.elapsed());
        let t = Instant::now();
        for _ in 0..1000 {
            rep.act(|tx| ig(tx.add_stroke(page, stroke(&mut rng))));
        }
        println!("pen stroke on a 20k-stroke page (tx + inverse + apply): {:?} each", t.elapsed() / 1000);
        let t = Instant::now();
        let mut hits = 0;
        for i in 0..1000 {
            let x = (i % 780) as f64;
            hits += rep.doc.hit_strokes(page, x, 500.0, x + 3.0, 503.0, 8.0).len();
        }
        println!("hit_strokes over 21k strokes: {:?} each ({hits} hits)", t.elapsed() / 1000);
        let t = Instant::now();
        let bytes = snapshot::encode(&rep.doc);
        println!("snapshot encode {:?} ({} bytes)", t.elapsed(), bytes.0.len());
        let t = Instant::now();
        let back = bytes.decode().unwrap();
        println!("snapshot decode + index {:?} ({} items)", t.elapsed(), back.items.len());
        let t = Instant::now();
        let chunk = snapshot::encode_chunk(&rep.doc, page);
        println!("one page chunk {:?} ({} bytes)", t.elapsed(), chunk.len());
    }
}
