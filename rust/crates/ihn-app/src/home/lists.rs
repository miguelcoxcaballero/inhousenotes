//! Home's lists: the Drive refreshes (full, quiet, periodic), the cached first paint, keyed rendering of the card
//! grids, skeletons and thumbnails.

use super::badges::{self, Deco};
use super::cards::{self, Sheet, Today};
use super::{folders, pull, Card, Slot, State};
use crate::abi::{ety, DomEvent};
use crate::dom::Prop;
use crate::drive::home::{self as drive, List, Media};
use crate::host::Metric;
use crate::shell::fx::Response;
use crate::{auth, shell, App, Host, IntlOp, Io, Mod, Node, Ui};
use ihn_core::catalog::{Entry, View};
use ihn_core::{jsfmt, palette};
use std::cmp::Ordering;

/// Which refresh a group of list requests belongs to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Kind {
    /// `refreshDriveFiles`.
    Files,
    /// `refreshDriveHomeQuiet`.
    Quiet,
    /// `refreshDriveHomeSnapshot` (the 30 s refresh).
    Snapshot,
}

/// One `Promise.all` of list refreshes.
pub(super) struct Batch {
    id: u32,
    kind: Kind,
    left: u8,
    settled: bool,
    pull: bool,
}

/// One `refreshDriveFolderContents`: two requests that render together.
pub(super) struct FolderJob {
    id: u32,
    nav: u32,
    folder: String,
    silent: bool,
    folders: Option<Vec<Entry>>,
    files: Option<Vec<Entry>>,
    batch: Option<u32>,
}

pub(crate) enum Pending {
    List { batch: u32, list: List },
    Folder { job: u32, files: bool },
    Thumb { slot: Slot, id: String, ticket: u32 },
}

/// A thumbnail being decoded off screen before it is shown.
pub(super) struct Decode {
    listener: u32,
    img: Node,
    slot: Slot,
    id: String,
    ticket: u32,
    url: String,
}

/// A decoded thumbnail waiting for the folder zoom to end.
pub(super) struct Decoded {
    slot: Slot,
    id: String,
    ticket: u32,
    url: String,
    width: f64,
    height: f64,
}

pub(super) fn library_ready(app: &App) -> bool { app.st.home.catalog.loaded && !app.st.home.account.is_empty() }
fn search(app: &App) -> String { app.st.home.search.trim().to_string() }

/// The cached directory `id` in the catalogue's order, filtered like 5.18.0's `lib.folder(id, search)`.
// v5: `e.name?.toLowerCase().includes(needle)` — lower-cased by the browser, which knows every script.
fn cached(app: &mut App, host: &mut dyn Host, id: &str, remote: Option<Vec<Entry>>) -> Vec<Entry> {
    let term = search(app);
    let needle = if term.is_empty() { String::new() } else { host.intl(IntlOp::Lower, &term) };
    // Two host questions are asked while sorting: names are compared, and searched names lower-cased.
    let host = std::cell::RefCell::new(host);
    let mut matches = |name: &str| host.borrow_mut().intl(IntlOp::Lower, name).contains(&needle);
    let mut order = |a: &str, b: &str| match host.borrow_mut().intl(IntlOp::Compare, &format!("{a}\0{b}\0\0")).as_str() {
        "-1" => Ordering::Less,
        "1" => Ordering::Greater,
        _ => Ordering::Equal,
    };
    let filter: ihn_core::catalog::NameFilter = if needle.is_empty() { None } else { Some(&mut matches) };
    match remote {
        Some(remote) => app.st.home.catalog.combine(remote, id, filter, &mut order),
        None => app.st.home.catalog.folder(id, filter, &mut order),
    }
}
fn folders_only(list: &[Entry]) -> Vec<Entry> { list.iter().filter(|e| e.is_folder()).cloned().collect() }
fn files_only(list: &[Entry]) -> Vec<Entry> { list.iter().filter(|e| e.mime == ihn_core::catalog::PDF).cloned().collect() }

// ---- refreshes --------------------------------------------------------------------------------------------------

fn start_batch(app: &mut App, host: &mut dyn Host, kind: Kind, lists: &[List], folder_silent: Option<bool>, pull: bool) {
    let st = &mut app.st.home;
    st.next_batch += 1;
    let id = st.next_batch;
    st.batches.push(Batch { id, kind, left: lists.len() as u8 + folder_silent.is_some() as u8, settled: false, pull });
    let term = search(app);
    for list in lists {
        let req = drive::request(app, host, Mod::Home, *list, &term, "root");
        app.st.home.reqs.push((req, Pending::List { batch: id, list: *list }));
    }
    if let Some(silent) = folder_silent {
        if !refresh_folder_contents(app, host, silent, Some(id)) {
            member_done(app, host, id, true);
        }
    }
}

/// A member of a batch resolved (`ok`) or rejected.
fn member_done(app: &mut App, host: &mut dyn Host, batch: u32, ok: bool) {
    let Some(at) = app.st.home.batches.iter().position(|b| b.id == batch) else { return };
    let b = &mut app.st.home.batches[at];
    b.left = b.left.saturating_sub(1);
    let (kind, left, pull) = (b.kind, b.left, b.pull);
    let settle = !b.settled && (!ok || left == 0);
    b.settled |= settle;
    if left == 0 {
        app.st.home.batches.remove(at);
    }
    if !settle {
        return;
    }
    // v5: the `try { await Promise.all([...]); updateDriveStatusCounts(); } catch { … }` of each refresh
    match (kind, ok) {
        (_, true) => update_status_counts(app, host),
        (Kind::Files, false) => shell::home_status(host, "Unable to load Drive files."),
        // A failed quiet refresh leaves the cards on screen; the periodic one is always silent.
        (Kind::Quiet | Kind::Snapshot, false) => {}
    }
    if kind == Kind::Snapshot {
        app.st.home.auto_busy = false;
    }
    if kind == Kind::Files && ok && !app.st.home.crawled {
        // v5: InhouseOfflineLibrary.catalogue() — once per session, after Drive answered.
        app.st.home.crawled = true;
        let account = app.st.home.account.clone();
        drive::crawl(app, host, &account);
    }
    if pull {
        pull::refreshed(app, host);
    }
}

/// Lists everything again, painting the cached library (or skeletons) first.
// v5: app-v5.js:refreshDriveFiles
pub fn refresh_files(app: &mut App, host: &mut dyn Host, pull: bool) {
    if !app.st.home.on {
        return;
    }
    if shell::offline(app) && library_ready(app) {
        render_home(app, host);
    } else if auth::token(app).is_none() {
        auth::update_ui(app, host);
    } else {
        let out = host.out();
        out.html(Ui::DRIVE_STATUS, "<span class=\"drive-status-skeleton\" aria-hidden=\"true\"></span>");
        out.attr(Ui::DRIVE_STATUS, "aria-label", Some("Loading your Drive"));
        folders::render_breadcrumbs(app, host);
        if library_ready(app) {
            render_home(app, host);
        } else {
            show_skeletons(app, host);
        }
        return start_batch(app, host, Kind::Files, &[List::Recents, List::Starred, List::Shared], Some(false), pull);
    }
    if pull {
        pull::refreshed(app, host);
    }
}

/// The same lists without skeletons, after a small change (star, colour, move, bin, upload) and on reconnect.
// v5: app-v5.js:refreshDriveHomeQuiet
pub fn refresh_quiet(app: &mut App, host: &mut dyn Host) {
    if app.st.home.on && auth::token(app).is_some() {
        start_batch(app, host, Kind::Quiet, &[List::Recents, List::Starred, List::Shared], Some(true), false);
    }
}

// v5: app-v5.js:isDriveHomeAutoRefreshBlocked
fn auto_blocked(app: &mut App, host: &mut dyn Host) -> bool {
    if shell::view(app) != shell::View::Home || auth::token(app).is_none() || auth::signed_out(app) || auth::reconnecting(app) {
        return true;
    }
    if app.cfg.has(crate::abi::mode::HIDDEN) || shell::offline(app) || app.st.home.anim.is_some() || app.st.home.opening {
        return true;
    }
    // An open card menu or colour popup, the folder picker, the bin and the Home profile menu keep Home still.
    let out = host.out();
    let open = out.query(Node::DOCUMENT, ".drive-card-dropdown:not(.hidden), .drive-color-popup, #modal-folder-picker.visible, #modal-drive-bin.visible, #drive-profile-menu:not(.hidden)");
    let blocked = host.metric(open, Metric::Connected) != 0.0;
    host.out().release(open);
    blocked
}

/// Recents and the current folder, every 30 s while Home is quietly visible.
// v5: app-v5.js:refreshDriveHomeSnapshot
pub(super) fn refresh_snapshot(app: &mut App, host: &mut dyn Host) {
    if app.st.home.auto_busy || auto_blocked(app, host) {
        return;
    }
    app.st.home.auto_busy = true;
    start_batch(app, host, Kind::Snapshot, &[List::Recents], Some(true), false);
}

/// Lists the current folder. Returns `false` when nothing was requested (the caller's promise is resolved).
// v5: app-v5.js:refreshDriveFolderContents
pub fn refresh_folder_contents(app: &mut App, host: &mut dyn Host, silent: bool, batch: Option<u32>) -> bool {
    if let Some(anim) = &mut app.st.home.anim {
        anim.refresh = Some(silent);
        return false;
    }
    if shell::offline(app) && library_ready(app) {
        render_folder(app, host);
        return false;
    }
    let st = &mut app.st.home;
    st.next_batch += 1;
    let (id, folder, nav) = (st.next_batch, st.current.clone(), st.nav_id);
    st.folder_jobs.push(FolderJob { id, nav, folder: folder.clone(), silent, folders: None, files: None, batch });
    let term = search(app);
    for (list, files) in [(List::Folders, false), (List::Files, true)] {
        let req = drive::request(app, host, Mod::Home, list, &term, &folder);
        app.st.home.reqs.push((req, Pending::Folder { job: id, files }));
    }
    true
}

fn folder_answer(app: &mut App, host: &mut dyn Host, job: u32, files: bool, listed: Option<Vec<Entry>>) {
    let Some(at) = app.st.home.folder_jobs.iter().position(|j| j.id == job) else { return };
    let stale = |st: &State, j: &FolderJob| j.folder != st.current || j.nav != st.nav_id;
    let Some(listed) = listed else {
        // One of the two requests failed: the other answer is dropped.
        let j = app.st.home.folder_jobs.remove(at);
        if !stale(&app.st.home, &j) && !j.silent {
            shell::home_status(host, "Unable to load this folder.");
        }
        if let Some(batch) = j.batch {
            member_done(app, host, batch, true); // v5: refreshDriveFolderContents never rejects
        }
        return;
    };
    let j = &mut app.st.home.folder_jobs[at];
    *(if files { &mut j.files } else { &mut j.folders }) = Some(listed);
    if j.folders.is_none() || j.files.is_none() {
        return;
    }
    let j = app.st.home.folder_jobs.remove(at);
    if !stale(&app.st.home, &j) {
        let (mut folder_list, mut file_list) = (j.folders.unwrap_or_default(), j.files.unwrap_or_default());
        let all: Vec<Entry> = folder_list.iter().chain(file_list.iter()).cloned().collect();
        observed(app, host, &all, Some(&j.folder), None);
        if library_ready(app) {
            let combined = cached(app, host, &j.folder, Some(all));
            (folder_list, file_list) = (folders_only(&combined), files_only(&combined));
        }
        app.st.home.folders = folder_list.clone();
        app.st.home.files = file_list.clone();
        render_folder_contents(app, host, folder_list, file_list);
        update_status_counts(app, host);
    }
    if let Some(batch) = j.batch {
        member_done(app, host, batch, true);
    }
}

/// A Drive listing reached Home: remember it for the next cached paint.
// v5: offline-library-v5.js:observed
fn observed(app: &mut App, host: &mut dyn Host, items: &[Entry], parent: Option<&str>, view: Option<View>) {
    if app.st.home.account.is_empty() {
        return;
    }
    if let Some(seen) = app.st.home.catalog.observe(items, parent, view) {
        super::persist(app, host, &seen.entries, seen.views);
        badges::refresh(app, host);
    }
}

pub(super) fn list_answer(app: &mut App, host: &mut dyn Host, pending: Pending, io: &Io) {
    let listed = (io.status >= 0).then(|| {
        let (files, _) = drive::parse_files(Response::parse(io.data).text());
        app.st.home.catalog.visible(files)
    });
    match pending {
        Pending::Folder { job, files } => folder_answer(app, host, job, files, listed),
        Pending::List { batch, list } => {
            let ok = listed.is_some();
            if let Some(mut files) = listed {
                match list {
                    List::Recents => {
                        observed(app, host, &files, None, Some(View::Recents));
                        app.st.home.recents = files.clone();
                        render_recents(app, host, files);
                    }
                    List::Starred => {
                        observed(app, host, &files, None, Some(View::Starred));
                        app.st.home.starred = files.clone();
                        render_starred(app, host, files);
                    }
                    _ => {
                        files.retain(Entry::shared);
                        observed(app, host, &files, None, Some(View::Shared));
                        app.st.home.shared = files.clone();
                        render_shared(app, host, files);
                    }
                }
            }
            member_done(app, host, batch, ok);
        }
        Pending::Thumb { slot, id, ticket } => thumb_url(app, host, slot, &id, ticket, (io.status >= 0).then(|| String::from_utf8_lossy(io.data).into_owned())),
    }
}

// v5: app-v5.js:updateDriveStatusCounts
pub(super) fn update_status_counts(app: &mut App, host: &mut dyn Host) {
    if auth::token(app).is_none() {
        return;
    }
    let st = &app.st.home;
    let n = st.files.len();
    let text = format!("Recents: {} | Starred: {} | Shared: {} | This folder: {n} PDF{}.", st.recents.len(), st.starred.len(), st.shared.len(), if n != 1 { "s" } else { "" });
    host.out().attr(Ui::DRIVE_STATUS, "aria-label", None);
    host.out().text(Ui::DRIVE_STATUS, &text);
}

// ---- rendering --------------------------------------------------------------------------------------------------

fn fill_skeletons(app: &mut App, host: &mut dyn Host, slot: Slot, count: usize, html: &str) {
    clear_slot(app, host, slot);
    host.out().html(slot.node(), &html.repeat(count));
    app.st.home.slots[slot as usize].skeletons = count > 0;
}

/// Empties a grid and forgets its cards.
pub(super) fn clear_slot(app: &mut App, host: &mut dyn Host, slot: Slot) {
    let out = host.out();
    for card in app.st.home.slots[slot as usize].cards.drain(..) {
        release_card(out, &card);
    }
    out.html(slot.node(), "");
    app.st.home.slots[slot as usize].skeletons = false;
}

fn release_card(out: &mut crate::OutBuf, card: &Card) {
    for node in [card.menu, card.badge, card.node] {
        if !node.is_none() {
            out.release(node);
        }
    }
}

// v5: app-v5.js:showDriveSkeletons
pub(super) fn show_skeletons(app: &mut App, host: &mut dyn Host) {
    for slot in [Slot::Recents, Slot::Starred, Slot::Shared] {
        fill_skeletons(app, host, slot, 6, cards::CARD_SKELETON);
    }
    if app.st.home.stack.len() > 1 {
        fill_skeletons(app, host, Slot::ViewFolders, 0, cards::FOLDER_SKELETON);
        fill_skeletons(app, host, Slot::ViewFiles, 4, cards::CARD_SKELETON);
    } else {
        fill_skeletons(app, host, Slot::Folders, 4, cards::FOLDER_SKELETON);
        fill_skeletons(app, host, Slot::Files, 8, cards::CARD_SKELETON);
    }
    for empty in [Ui::DRIVE_STARRED_EMPTY, Ui::DRIVE_SHARED_EMPTY, Ui::DRIVE_FOLDER_EMPTY] {
        host.out().class(empty, "hidden", true);
    }
}

// v5: app-v5.js:renderDriveRecents
pub(super) fn render_recents(app: &mut App, host: &mut dyn Host, files: Vec<Entry>) {
    let list = if library_ready(app) { app.st.home.catalog.recent_items(files) } else { files };
    diff_render(app, host, Slot::Recents, &list, false);
}

// v5: app-v5.js:renderDriveStarred
pub(super) fn render_starred(app: &mut App, host: &mut dyn Host, files: Vec<Entry>) {
    diff_render(app, host, Slot::Starred, &files, false);
    host.out().class(Ui::DRIVE_STARRED_EMPTY, "hidden", !files.is_empty());
}

// v5: app-v5.js:renderDriveShared
pub(super) fn render_shared(app: &mut App, host: &mut dyn Host, files: Vec<Entry>) {
    diff_render(app, host, Slot::Shared, &files, false);
    host.out().class(Ui::DRIVE_SHARED_EMPTY, "hidden", !files.is_empty());
}

/// Root contents go to the Files tab; an opened folder's go to the folder view, as deferred slots.
// v5: app-v5.js:renderDriveFolderContents
pub(super) fn render_folder_contents(app: &mut App, host: &mut dyn Host, folder_list: Vec<Entry>, file_list: Vec<Entry>) {
    if let Some(anim) = &mut app.st.home.anim {
        if !anim.preparing {
            anim.pending_contents = Some((folder_list, file_list));
            return;
        }
    }
    let nested = app.st.home.stack.len() > 1;
    if nested {
        folders::invalidate_plans(app);
    }
    let (folders_slot, files_slot, empty) = if nested { (Slot::ViewFolders, Slot::ViewFiles, Ui::DRIVE_VIEW_EMPTY) } else { (Slot::Folders, Slot::Files, Ui::DRIVE_FOLDER_EMPTY) };
    diff_render(app, host, folders_slot, &folder_list, nested);
    diff_render(app, host, files_slot, &file_list, nested);
    host.out().class(empty, "hidden", !folder_list.is_empty() || !file_list.is_empty());
    if app.st.home.anim.is_none() {
        folders::hydrate_visible(app, host);
    }
}

/// The cached library, before (or instead of) Drive's answer.
// v5: offline-app-v5.js:renderHome
pub fn render_home(app: &mut App, host: &mut dyn Host) {
    if !library_ready(app) {
        return;
    }
    let home = app.st.home.catalog.home();
    let st = &mut app.st.home;
    (st.recents, st.starred, st.shared) = (home.recents.clone(), home.starred.clone(), home.shared.clone());
    render_recents(app, host, home.recents);
    render_starred(app, host, home.starred);
    render_shared(app, host, home.shared);
    render_folder(app, host);
    shell::home_status(host, if shell::offline(app) { "Offline · changes stay on this device" } else { "Connecting to Drive…" });
    auth::update_ui(app, host);
}

// v5: offline-app-v5.js:renderFolder
pub(super) fn render_folder(app: &mut App, host: &mut dyn Host) {
    let current = app.st.home.current.clone();
    let items = cached(app, host, &current, None);
    let (folder_list, file_list) = (folders_only(&items), files_only(&items));
    app.st.home.folders = folder_list.clone();
    app.st.home.files = file_list.clone();
    render_folder_contents(app, host, folder_list, file_list);
    folders::render_breadcrumbs(app, host);
}

/// The cached directory `id` split into folders and files (the folder view's first paint).
pub(super) fn cached_folder(app: &mut App, host: &mut dyn Host, id: &str) -> (Vec<Entry>, Vec<Entry>) {
    if !library_ready(app) {
        return (Vec::new(), Vec::new());
    }
    let items = cached(app, host, id, None);
    (folders_only(&items), files_only(&items))
}

fn new_card(app: &mut App, host: &mut dyn Host, slot: Slot, item: &Entry, today: &mut Today, deferred: bool, before: Node) -> Card {
    let folder = item.is_folder();
    let deco = if folder { Deco::default() } else { badges::deco(app, &item.id) };
    let reduced = app.cfg.has(crate::abi::mode::REDUCED_MOTION);
    let sub = if folder { String::new() } else { cards::describe(host, today, item) };
    let night = shell::dark(app);
    let (html, arrive) = if deferred {
        // v5: createDeferredDriveFolderCard — the arrival fade is decided when the slot is made.
        let arrive = app.st.home.anim.is_none() && !reduced;
        (cards::deferred_card(item, arrive, (!folder).then_some(&deco)), arrive)
    } else {
        // v5: diffRenderDriveList — cards built straight into the folder view fade in.
        let arrive = slot.in_view() && !reduced;
        (if folder { cards::folder_card(item, night, arrive) } else { cards::file_card(item, &sub, &Sheet::default(), &deco, arrive) }, arrive)
    };
    let out = host.out();
    let node = out.insert_html(slot.node(), before, &html);
    let menu = if deferred { Node::NONE } else { out.child(node, if folder { cards::FOLDER_MENU_PATH } else { cards::FILE_MENU_PATH }) };
    let badge = if deco.badge.is_some() { out.query(node, ".document-sync-status") } else { Node::NONE };
    Card { id: item.id.clone(), fp: item.fingerprint(), node, menu, badge, entry: item.clone(), deco, sub, folder, deferred, arriving: arrive, sheet: Sheet::default(), rev: None, local: String::new(), ticket: 0 }
}

/// Keyed rendering: a card whose fingerprint did not change keeps its element (and its loaded image).
// v5: app-v5.js:diffRenderDriveList
pub(super) fn diff_render(app: &mut App, host: &mut dyn Host, slot: Slot, items: &[Entry], deferred: bool) {
    if app.st.home.slots[slot as usize].skeletons {
        clear_slot(app, host, slot);
    }
    let mut old: Vec<Option<Card>> = std::mem::take(&mut app.st.home.slots[slot as usize].cards).into_iter().map(Some).collect();
    // The children of the grid, in DOM order.
    let mut order: Vec<Node> = old.iter().flatten().map(|c| c.node).collect();
    let mut cursor = 0usize;
    let mut next: Vec<Card> = Vec::with_capacity(items.len());
    let mut today = Today::new(host);
    let mut fresh: Vec<usize> = Vec::new();
    for item in items {
        if item.id.is_empty() || next.iter().any(|c| c.id == item.id) {
            continue;
        }
        let found = old.iter().position(|c| c.as_ref().is_some_and(|c| c.id == item.id));
        let fp = item.fingerprint();
        let before = |order: &[Node], cursor: usize| order.get(cursor).copied().unwrap_or(Node::NONE);
        match found.filter(|i| old[*i].as_ref().is_some_and(|c| c.fp == fp)) {
            Some(i) => {
                let mut card = old[i].take().unwrap_or_else(|| unreachable!());
                card.entry = item.clone();
                if !card.folder && !card.deferred {
                    let sub = cards::describe(host, &mut today, item);
                    if sub != card.sub {
                        let node = host.out().query(card.node, ".drive-card-sub[data-file-sub]");
                        host.out().text(node, &sub);
                        host.out().release(node);
                        card.sub = sub;
                    }
                }
                if !item.is_folder() {
                    let deco = badges::deco(app, &item.id);
                    if deco != card.deco {
                        card.badge = badges::apply(host, card.node, card.badge, &card.deco, &deco);
                        card.deco = deco;
                    }
                }
                if before(&order, cursor) != card.node {
                    order.retain(|n| *n != card.node);
                    host.out().insert(slot.node(), before(&order, cursor), card.node);
                    order.insert(cursor, card.node);
                }
                cursor += 1;
                if !card.folder && !card.deferred {
                    fresh.push(next.len());
                }
                next.push(card);
            }
            None => {
                // A changed card takes the place of the one it replaces.
                let replaced = found.and_then(|i| old[i].take());
                let at = replaced.as_ref().and_then(|c| order.iter().position(|n| *n == c.node));
                let anchor = match (at, &replaced) {
                    (Some(_), Some(c)) => c.node,
                    _ => before(&order, cursor),
                };
                let card = new_card(app, host, slot, item, &mut today, deferred, anchor);
                match (at, replaced) {
                    (Some(at), Some(gone)) => {
                        host.out().remove(gone.node);
                        release_card(host.out(), &gone);
                        order[at] = card.node;
                        cursor = at + 1;
                    }
                    _ => {
                        order.insert(cursor, card.node);
                        cursor += 1;
                    }
                }
                if !card.folder && !card.deferred {
                    fresh.push(next.len());
                }
                next.push(card);
            }
        }
    }
    for gone in old.into_iter().flatten() {
        host.out().remove(gone.node);
        release_card(host.out(), &gone);
    }
    app.st.home.slots[slot as usize].cards = next;
    for i in fresh {
        refresh_thumb(app, host, slot, i);
    }
}

// ---- thumbnails -------------------------------------------------------------------------------------------------

fn card_index(app: &App, slot: Slot, id: &str) -> Option<usize> { app.st.home.slots[slot as usize].cards.iter().position(|c| c.id == id) }

fn preview_class(host: &mut dyn Host, card: Node, name: &str, on: bool) {
    let out = host.out();
    let preview = out.child(card, &[0]);
    out.class(preview, name, on);
    out.release(preview);
}

/// Loads the thumbnail of a card when its document or its local preview changed.
// v5: app-v5.js:createDrivePreview `refreshFile`
pub(super) fn refresh_thumb(app: &mut App, host: &mut dyn Host, slot: Slot, index: usize) {
    let account = app.st.home.account.clone();
    let Some(card) = app.st.home.slots[slot as usize].cards.get_mut(index) else { return };
    // Drive thumbnail URLs are temporary and rotate even without edits.
    if card.rev.as_deref() == Some(card.entry.modified.as_str()) && card.local == card.entry.local_preview {
        return;
    }
    card.rev = Some(card.entry.modified.clone());
    card.local = card.entry.local_preview.clone();
    card.ticket += 1;
    let (ticket, id, node, revision) = (card.ticket, card.id.clone(), card.node, card.entry.modified.clone());
    let source = if card.local.is_empty() { drive::normalize_thumbnail(&card.entry.thumbnail) } else { card.local.clone() };
    if card.sheet.url.is_empty() && !card.sheet.loading {
        card.sheet.loading = true;
        preview_class(host, node, "is-loading", true);
    }
    let waiter = app.req(Mod::Home);
    match drive::media_url(app, host, &account, &format!("preview:{id}"), &source, &revision, waiter) {
        Media::Ready(url) => thumb_url(app, host, slot, &id, ticket, Some(url)),
        Media::Pending => app.st.home.reqs.push((waiter, Pending::Thumb { slot, id, ticket })),
    }
}

fn thumb_failed(app: &mut App, host: &mut dyn Host, slot: Slot, id: &str, ticket: u32, retry: bool) {
    let Some(i) = card_index(app, slot, id) else { return };
    let card = &mut app.st.home.slots[slot as usize].cards[i];
    if card.ticket != ticket {
        return;
    }
    if retry {
        card.rev = None; // retry at the next refresh, keeping the last good image
    }
    if std::mem::take(&mut card.sheet.loading) {
        let node = card.node;
        preview_class(host, node, "is-loading", false);
    }
}

fn thumb_url(app: &mut App, host: &mut dyn Host, slot: Slot, id: &str, ticket: u32, url: Option<String>) {
    let Some(url) = url.filter(|u| !u.is_empty()) else { return thumb_failed(app, host, slot, id, ticket, true) };
    if card_index(app, slot, id).is_none_or(|i| app.st.home.slots[slot as usize].cards[i].ticket != ticket) {
        return;
    }
    // Decode off screen; the displayed image is never removed while its replacement loads.
    let st = &mut app.st.home;
    // Two ids: a listener id names one listener (`listener` for `load`, `listener + 1` for `error`).
    st.next_dyn = (st.next_dyn + 2) & 0x0f_fffe;
    let listener = crate::tag(Mod::Home, super::L_DYNAMIC + st.next_dyn);
    let out = host.out();
    let img = out.create("img");
    out.prop(img, "decoding", Prop::Str("async"));
    out.listen(img, "load", listener, 0);
    out.listen(img, "error", listener + 1, 0);
    out.prop(img, "src", Prop::Str(&url));
    app.st.home.decodes.push(Decode { listener, img, slot, id: id.into(), ticket, url });
}

/// `load` / `error` of an off-screen thumbnail.
pub(super) fn decode_event(app: &mut App, host: &mut dyn Host, e: &DomEvent) -> bool {
    let Some(at) = app.st.home.decodes.iter().position(|d| d.listener == e.listener & !1) else { return false };
    let d = app.st.home.decodes.remove(at);
    let (width, height) = if e.ty == ety::LOAD { (host.metric(d.img, Metric::NaturalWidth), host.metric(d.img, Metric::NaturalHeight)) } else { (0.0, 0.0) };
    host.out().unlisten(d.listener);
    host.out().unlisten(d.listener + 1);
    host.out().release(d.img);
    let decoded = Decoded { slot: d.slot, id: d.id, ticket: d.ticket, url: d.url, width, height };
    // v5: applyDrivePreviewTone samples the sheet and then never applies the result (it asks whether the preview
    // contains the off-screen image, which it never does), so there is no label tone to port.
    if e.ty != ety::LOAD {
        thumb_failed(app, host, decoded.slot, &decoded.id, decoded.ticket, true);
    } else if app.st.home.anim.is_some() {
        // v5: whenDriveFolderMotionSettled — a repaint of the zooming scene would cost a frame.
        app.st.home.motion_waiters.push(decoded);
    } else {
        show_thumb(app, host, decoded);
    }
    true
}

/// The thumbnails that waited for the folder zoom.
pub(super) fn release_motion_waiters(app: &mut App, host: &mut dyn Host) {
    if app.st.home.anim.is_some() {
        return;
    }
    for decoded in std::mem::take(&mut app.st.home.motion_waiters) {
        show_thumb(app, host, decoded);
    }
}

fn show_thumb(app: &mut App, host: &mut dyn Host, d: Decoded) {
    let Some(i) = card_index(app, d.slot, &d.id).filter(|i| app.st.home.slots[d.slot as usize].cards[*i].ticket == d.ticket) else { return };
    if !(d.width > 0.0) || !(d.height > 0.0) {
        return thumb_failed(app, host, d.slot, &d.id, d.ticket, false);
    }
    let card = &mut app.st.home.slots[d.slot as usize].cards[i];
    let (node, first) = (card.node, card.sheet.url.is_empty());
    let ratio = palette::sheet_ratio(d.width, d.height).map_or(String::new(), |r| jsfmt::fixed(r, 4));
    let landscape = d.width > d.height;
    let (was_loading, ratio_changed, landscape_changed) = (card.sheet.loading, card.sheet.ratio != ratio, card.sheet.landscape != landscape);
    card.sheet = Sheet { url: d.url.clone(), loading: false, landscape, ratio: ratio.clone() };
    let name = card.entry.name.clone();
    let html = cards::image(&card.entry, &d.url);
    let out = host.out();
    let preview = out.child(node, &[0]);
    if first {
        let icon = out.child(preview, &[0]);
        out.remove(icon);
        out.release(icon);
        let info = out.child(preview, &[0]);
        let img = out.insert_html(preview, info, &html);
        out.release(img);
        out.release(info);
        out.class(preview, "is-loaded", true);
    } else {
        let img = out.child(preview, &[0]);
        out.prop(img, "alt", Prop::Str(&format!("{name} preview")));
        out.prop(img, "src", Prop::Str(&d.url));
        out.release(img);
    }
    if ratio_changed {
        out.style(node, "--sheet-ratio", (!ratio.is_empty()).then_some(ratio.as_str()));
    }
    if landscape_changed {
        out.class(preview, "is-landscape", landscape);
    }
    if was_loading {
        out.class(preview, "is-loading", false);
    }
    out.release(preview);
    // Two rows may exchange height without resizing the grid.
    if d.slot == Slot::ViewFiles {
        folders::invalidate_plans(app);
    }
}

/// What the lists hold, for tests.
pub(super) fn slot_ids(st: &State, slot: Slot) -> Vec<String> { st.slots[slot as usize].cards.iter().map(|c| c.id.clone()).collect() }

/// The cached directory `id` as Home lists it (search included), for tests and other units.
pub fn folder_listing(app: &mut App, host: &mut dyn Host, id: &str) -> Vec<Entry> { cached(app, host, id, None) }
