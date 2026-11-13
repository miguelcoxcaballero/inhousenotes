//! Document state, apply, queries, the `Tx` builder and undo inverses (ARCHITECTURE §4.1–4.3), and the
//! core↔sync interface types (§4.4).
//!
//! Every field is an LWW register, so `apply` is idempotent and commutative: any delivery order, duplicates and gaps
//! converge. The derived indexes (page order, per-page draw lists with bounds, per-page membership, dirty pages)
//! are kept in step by `apply`/`merge`; nothing else may write the registers.

use crate::geom::{self, Affine, SelectionBounds, Sweep};
use crate::id::{ActorId, DocId, Hash, Id, IdGen};
use crate::op::{Background, Batch, Body, Color, ImageRect, Op, PageProp, Template, Tool, VersionMeta, MIN_PAGE_SIDE, Z};
use crate::pos::Pos;
use crate::snapshot::{EncodedBatch, EncodedSnapshot};
use crate::stamp::{Clock, Reg, Stamp, VersionVector};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

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
    items: HashMap<Id, (Option<Id>, Option<Id>, bool)>,
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
    pub pages: HashMap<Id, Page>,
    /// Strokes and images of all pages, including deleted ones.
    pub items: HashMap<Id, Item>,
    /// Timeline (grow-only; prune = tombstone register).
    pub versions: BTreeMap<Id, Version>,
    pub vv: VersionVector,
    // Derived, not serialized.
    order: Vec<Id>,
    index: HashMap<Id, PageIndex>,
    members: HashMap<Id, HashSet<Id>>,
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
fn merge_body(mine: &mut Reg<Body>, body: &Body, at: Stamp) -> bool {
    let wins = at > mine.at || (at == mine.at && mine.v.is_dropped() && !body.is_dropped());
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
    // -------------------------------------------------------------------------------------------- apply

    /// Applies a local or remote batch atomically. A batch already in `vv` is skipped (`Changes::applied == false`).
    /// The caller observes `b.max_clock()` with its `Clock`.
    pub fn apply(&mut self, b: &Batch) -> Changes {
        if !self.vv.add(b.actor, b.seq) {
            return Changes::default();
        }
        let mut t = Track::default();
        for (i, op) in b.ops.iter().enumerate() {
            self.apply_op(op, b.stamp(i), &mut t);
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

    fn apply_op(&mut self, op: &Op, at: Stamp, t: &mut Track) {
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
                *wrote |= it.page.set(*page, at) | it.z.set(*z, at) | merge_body(&mut it.body, body, at);
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
        let mut edits: HashMap<Id, (HashSet<Id>, Vec<(usize, DrawRef)>)> = HashMap::new();
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
            let mut unsorted = [false; 3];
            for (k, r) in added {
                unsorted[k] |= runs[k].last().is_some_and(|l| (l.z, l.id) > (r.z, r.id));
                runs[k].push(r);
            }
            for k in 0..3 {
                if unsorted[k] {
                    runs[k].sort_unstable_by_key(|r| (r.z, r.id));
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
    pub fn new(ids: IdGen, seq: u32) -> Author { Author { actor: ids.actor, ids, clock: Clock::default(), seq } }
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

