//! Home view: tabs, lists, cards, folders, breadcrumbs, search, pull-to-refresh, fast scroller, badges
//! (ARCHITECTURE §13). Implemented by W2.5 (BUILD-PLAN.md); `dialogs` is W5.5's.
//!
//! For other units:
//! - [`show`] / [`refresh`] / [`refresh_quiet`]: 5.18.0 `showDriveHome`, `refreshDriveFiles`, `refreshDriveHomeQuiet`.
//! - [`sig`]: what Home broadcasts (a card was activated, a badge was pressed).
//! - [`card_at`]: the item of the card an event happened in (for the card ⋮ menu).
//! - [`doc_changed`], [`badges::set_sync`], [`badges::set_errors`]: what the document, sync and export units report.
//! - [`current_folder`], [`search`], [`catalog`]: Home's navigation and catalogue state.

use crate::abi::{ety, ev, life, mode, ret, DomEvent, Handles};
use crate::host::LsOp;
use crate::shell::json::Json;
use crate::shell::sig as shell_sig;
use crate::store::{self, schema, DocRecord, Reply, Store, Tx, Value};
use crate::testapi::json_str;
use crate::{auth, shell, tag, App, Event, Host, Io, Mod, Node, Ui};
use ihn_core::catalog::{Catalog, DocInfo, Entry, Views};

pub mod badges;
pub mod cards;
pub mod dialogs;
pub mod folders;
pub mod lists;
pub mod nav;
pub mod pull;
pub mod scroll;

/// Broadcasts of Home (`Event::Msg`, sent with `shell::signal`).
pub mod sig {
    /// A document card was activated (5.18.0 `openDriveFile(card.driveFile)`): data = the file as JSON
    /// `{id, name, mimeType, modifiedTime, resourceKey, thumbnailLink, parents, ownedByMe, pending}`.
    pub const OPEN_FILE: u8 = 0x30;
    /// The badge of a document whose upload failed was pressed: data = the document id (`#offline-sync-error`).
    pub const SYNC_ERROR: u8 = 0x31;
    /// A waiting badge was pressed while Drive is reachable: data = the document id (5.18.0 `retrySync`).
    pub const RETRY_SYNC: u8 = 0x32;
}

/// The seven card grids.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Slot {
    Recents,
    Starred,
    Shared,
    Folders,
    Files,
    ViewFolders,
    ViewFiles,
}

impl Slot {
    pub const ALL: [Slot; 7] = [Slot::Recents, Slot::Starred, Slot::Shared, Slot::Folders, Slot::Files, Slot::ViewFolders, Slot::ViewFiles];
    pub fn node(self) -> Node {
        [Ui::DRIVE_RECENTS, Ui::DRIVE_STARRED, Ui::DRIVE_SHARED, Ui::DRIVE_FOLDERS, Ui::DRIVE_FOLDER_FILES, Ui::DRIVE_VIEW_FOLDERS, Ui::DRIVE_VIEW_FILES][self as usize]
    }
    /// Inside the opened folder view.
    pub fn in_view(self) -> bool { matches!(self, Slot::ViewFolders | Slot::ViewFiles) }
    pub fn name(self) -> &'static str { ["recents", "starred", "shared", "folders", "files", "view-folders", "view-files"][self as usize] }
}

/// One card on screen.
pub struct Card {
    pub id: String,
    pub fp: String,
    pub node: Node,
    /// Its `.drive-card-menu` (`Node::NONE` while deferred).
    pub menu: Node,
    /// Its sync badge, when it has one.
    pub badge: Node,
    pub entry: Entry,
    pub deco: badges::Deco,
    pub sub: String,
    pub folder: bool,
    /// Still the empty slot of an opened folder.
    pub deferred: bool,
    pub arriving: bool,
    pub sheet: cards::Sheet,
    rev: Option<String>,
    local: String,
    ticket: u32,
}

#[derive(Default)]
pub struct SlotState {
    pub cards: Vec<Card>,
    pub skeletons: bool,
}

/// This module's state, stored in `App::st`.
pub struct State {
    /// Home runs (not the headless page).
    on: bool,
    main: Node,
    topbar: Node,
    sheet_grid: Node,
    view_inner: Node,
    tabs: [Node; 4],
    links: [Node; 2],
    // The catalogue of the known account.
    account: String,
    catalog: Catalog,
    load_req: u32,
    sync: badges::Sync,
    // Lists.
    slots: [SlotState; 7],
    recents: Vec<Entry>,
    starred: Vec<Entry>,
    shared: Vec<Entry>,
    folders: Vec<Entry>,
    files: Vec<Entry>,
    search: String,
    reqs: Vec<(u32, lists::Pending)>,
    batches: Vec<lists::Batch>,
    folder_jobs: Vec<lists::FolderJob>,
    next_batch: u32,
    auto_on: bool,
    auto_busy: bool,
    crawled: bool,
    /// A document is being opened (5.18.0 `driveOpenInProgress`): the periodic refresh waits.
    opening: bool,
    decodes: Vec<lists::Decode>,
    motion_waiters: Vec<lists::Decoded>,
    next_dyn: u32,
    // Navigation.
    tab: u8,
    stack: Vec<folders::Crumb>,
    current: String,
    origin_id: Option<String>,
    nav_id: u32,
    crumb_nodes: Vec<Node>,
    view_open: bool,
    view_top: f64,
    view_color: String,
    anim: Option<Box<folders::Anim>>,
    settle: Option<folders::Settle>,
    settling_labels: u8,
    plans: [Option<folders::Plan>; 2],
    observing: bool,
    frames: u32,
    pull: pull::State,
    scroll: scroll::State,
}

impl Default for State {
    fn default() -> State {
        State {
            on: false, main: Node::NONE, topbar: Node::NONE, sheet_grid: Node::NONE, view_inner: Node::NONE, tabs: [Node::NONE; 4], links: [Node::NONE; 2],
            account: String::new(), catalog: Catalog::default(), load_req: 0, sync: badges::Sync::default(),
            slots: Default::default(), recents: Vec::new(), starred: Vec::new(), shared: Vec::new(), folders: Vec::new(), files: Vec::new(), search: String::new(),
            reqs: Vec::new(), batches: Vec::new(), folder_jobs: Vec::new(), next_batch: 0, auto_on: false, auto_busy: false, crawled: false, opening: false,
            decodes: Vec::new(), motion_waiters: Vec::new(), next_dyn: 0,
            tab: 0, stack: vec![folders::Crumb::root()], current: "root".into(), origin_id: None, nav_id: 0, crumb_nodes: Vec::new(), view_open: false, view_top: -1.0,
            view_color: String::new(), anim: None, settle: None, settling_labels: 0, plans: [None, None], observing: false, frames: 0,
            pull: pull::State::default(), scroll: scroll::State::default(),
        }
    }
}

const fn id(n: u32) -> u32 { tag(Mod::Home, n) }
const L_NAV: u32 = id(1);
const L_SEE_ALL: u32 = id(2); // ..=3
const L_BACK: u32 = id(4);
const L_VIEW_BACK: u32 = id(5);
const L_SEARCH: u32 = id(6);
const L_VIEW_SCROLL: u32 = id(7);
const L_DOC_KEY: u32 = id(8);
const L_CRUMBS: u32 = id(9);
const L_TOUCH_START: u32 = id(10);
const L_TOUCH_MOVE: u32 = id(11);
const L_TOUCH_END: u32 = id(12);
const L_TOUCH_CANCEL: u32 = id(13);
const L_THUMB_DOWN: u32 = id(14);
const L_THUMB_MOVE: u32 = id(15);
const L_THUMB_UP: u32 = id(16);
const L_THUMB_CANCEL: u32 = id(17);
const L_THUMB_LOST: u32 = id(18);
const L_THUMB_KEY: u32 = id(19);
const L_MAIN_SCROLL: u32 = id(20);
const L_SCROLL_RESIZE: u32 = id(21);
const L_SCROLL_VISIBILITY: u32 = id(22);
const L_SCROLL_CONTENTS: u32 = id(23);
const L_GRID_RESIZE: u32 = id(24);
const L_SLOT_CLICK: u32 = id(32); // + slot
const L_SLOT_KEY: u32 = id(40);
const L_SLOT_FOCUS: u32 = id(48);
const L_SLOT_ANIM: u32 = id(56);
/// Local ids of listeners made at run time (thumbnail decodes, the zoom's `finish`).
const L_DYNAMIC: u32 = 0x10_0000;
const T_SEARCH: u32 = id(100);
const T_AUTO: u32 = id(101);
const T_PULL_RESET: u32 = id(102);
const T_PULL_HOLD: u32 = id(103);
const T_PREPARE: u32 = id(104);
const T_SETTLE: u32 = id(105);
const F_PULL_TOP: u32 = 1;
const F_SCROLL: u32 = 2;
const F_PREPARE: u32 = 4;
const F_SETTLE: u32 = 8;
const F_LABELS: u32 = 16;

// ---- what other units use ---------------------------------------------------------------------------------------

/// 5.18.0 `showDriveHome()`.
pub fn show(app: &mut App, host: &mut dyn Host) { shell::show_view(app, host, shell::View::Home); }
/// 5.18.0 `refreshDriveFiles()`.
pub fn refresh(app: &mut App, host: &mut dyn Host) { lists::refresh_files(app, host, false); }
/// 5.18.0 `refreshDriveHomeQuiet()`.
pub fn refresh_quiet(app: &mut App, host: &mut dyn Host) { lists::refresh_quiet(app, host); }
/// The account whose catalogue Home shows (lower-case e-mail; empty when none is known).
pub fn account(app: &App) -> &str { &app.st.home.account }
pub fn catalog(app: &App) -> &Catalog { &app.st.home.catalog }
/// `(id, name)` of the folder Home is in (`root`, `My Drive` at the top).
pub fn current_folder(app: &App) -> (&str, &str) {
    let crumb = app.st.home.stack.last();
    (&app.st.home.current, crumb.map_or("My Drive", |c| c.name.as_str()))
}
/// The search box's text, trimmed.
pub fn search(app: &App) -> &str { app.st.home.search.trim() }
/// List requests in flight (tests of other modules count Drive requests).
pub fn pending_requests(app: &App) -> usize { app.st.home.reqs.iter().filter(|r| !matches!(r.1, lists::Pending::Thumb { .. })).count() }
/// A document is being opened or stopped being opened (the periodic refresh waits meanwhile).
pub fn set_opening(app: &mut App, on: bool) { app.st.home.opening = on; }

/// The card an event's path goes through: `(slot, item, the click was inside its ⋮ menu)`.
pub fn card_at<'a>(app: &'a App, path: &Handles) -> Option<(Slot, &'a Entry, bool)> {
    Slot::ALL.into_iter().find_map(|slot| {
        let card = app.st.home.slots[slot as usize].cards.iter().find(|c| path.contains(c.node))?;
        Some((slot, &card.entry, !card.menu.is_none() && path.contains(card.menu)))
    })
}

/// A document's local state changed (opened, pinned, flags written, deleted): its cards follow.
pub fn doc_changed(app: &mut App, host: &mut dyn Host, id: &str, info: Option<DocInfo>) {
    app.st.home.catalog.set_doc(id, info);
    badges::refresh(app, host);
}

/// A catalogue entry written by this device (a document created offline, a new local preview).
pub fn entry_changed(app: &mut App, host: &mut dyn Host, entry: &Entry) {
    if app.st.home.account.is_empty() {
        return;
    }
    if let Some(seen) = app.st.home.catalog.observe(std::slice::from_ref(entry), None, None) {
        persist(app, host, &seen.entries, false);
    }
}

// ---- the catalogue on this device -------------------------------------------------------------------------------

fn load(app: &mut App, host: &mut dyn Host) {
    let account = app.st.home.account.clone();
    if account.is_empty() || !app.st.home.on {
        return;
    }
    let req = app.req(Mod::Home);
    app.st.home.load_req = req;
    let mut tx = Tx::new();
    tx.catalog_scan(&account);
    tx.kv_get(&account, schema::kv::HOME_VIEWS);
    tx.kv_get(&account, schema::kv::RELAY_FOLDER);
    tx.kv_get(&account, schema::kv::DRIVE_ROOT);
    tx.docs_scan();
    tx.scan(Store::Chunks, store::Range::All, 0, true);
    store::tx(app, host, req, tx);
}

fn strings(v: &Value) -> Option<Vec<String>> {
    match v {
        Value::Arr(items) => Some(items.iter().map(Value::text).collect()),
        _ => None,
    }
}

// v5: offline-library-v5.js:reload → host.refreshed
fn loaded(app: &mut App, host: &mut dyn Host, io: &Io) {
    let mut reply = Reply::new(io);
    if !reply.ok() {
        return; // degraded storage: no local library, Home lists from Drive only
    }
    let account = app.st.home.account.clone();
    let rows = reply.rows();
    let mut handles = Vec::new();
    rows.iter().for_each(|(_, v)| v.handles(&mut handles));
    let entries: Vec<Entry> = rows.iter().map(|(_, v)| crate::drive::home::entry_from_value(v)).filter(|e| !e.id.is_empty()).collect();
    let views = reply.value();
    let views = Views { recents: strings(views.get("recents")), starred: strings(views.get("starred")), shared: strings(views.get("shared")) };
    let sync_folder = reply.value().text();
    let root = reply.value().get("remoteId").text();
    let docs = reply.rows();
    let mut with_chunks: Vec<String> = reply.keys().iter().filter_map(|k| k.items().first().map(Value::text)).collect();
    with_chunks.dedup();
    let mut infos = Vec::new();
    for (key, value) in &docs {
        value.handles(&mut handles);
        let record = DocRecord::from_value(value);
        if record.account == account {
            let id = key.text();
            let info = DocInfo { last_opened_at: record.last_opened_at, pinned: record.pinned, available: with_chunks.binary_search(&id).is_ok(), pending: record.flags.relay_pending || !record.flags.created };
            infos.push((id, info));
        }
    }
    handles.into_iter().for_each(|h| host.out().obj_release(h));
    // Drive answered before the stored catalogue arrived (a first run opens the database meanwhile): its lists are
    // on screen and newer, so the cached paint is skipped. v5: the library is read before the first list request.
    let c = &app.st.home.catalog;
    let early = !c.loaded && (!c.is_empty() || c.views.recents.is_some() || c.views.starred.is_some() || c.views.shared.is_some());
    app.st.home.catalog.load(entries, infos, views, sync_folder, root);
    auth::set_local_library(app, host, true);
    badges::refresh(app, host);
    if early {
        auth::update_ui(app, host);
    } else {
        lists::render_home(app, host);
    }
}

/// Writes observed entries (field by field, so fields other units own survive) and the saved views.
fn persist(app: &mut App, host: &mut dyn Host, entries: &[Entry], views: bool) {
    let account = app.st.home.account.clone();
    if account.is_empty() || (entries.is_empty() && !views) {
        return;
    }
    let mut tx = Tx::new();
    for entry in entries {
        let Value::Obj(mut fields) = crate::drive::home::entry_value(entry) else { continue };
        if entry.local_preview.is_empty() {
            fields.push(("localPreview".into(), Value::Null));
        }
        let patch: Vec<(Vec<&str>, Value)> = fields.iter().map(|(k, v)| (vec![k.as_str()], v.clone())).collect();
        let patch: Vec<(&[&str], Value)> = patch.iter().map(|(k, v)| (k.as_slice(), v.clone())).collect();
        tx.catalog_patch(&account, &entry.id, &patch);
    }
    if views {
        let v = &app.st.home.catalog.views;
        let list = |ids: &Option<Vec<String>>| ids.as_ref().map(|ids| Value::Arr(ids.iter().map(|i| Value::Str(i.clone())).collect()));
        let fields: Vec<(String, Value)> = [("recents", list(&v.recents)), ("starred", list(&v.starred)), ("shared", list(&v.shared))].into_iter().filter_map(|(k, v)| Some((k.to_string(), v?))).collect();
        tx.kv_put(&account, schema::kv::HOME_VIEWS, &Value::Obj(fields));
    }
    let req = app.req(Mod::Home);
    store::tx(app, host, req, tx);
}

/// The Drive id of My Drive is known (the enumeration's first answer).
pub(crate) fn root_known(app: &mut App, host: &mut dyn Host, root: &str) {
    let account = app.st.home.account.clone();
    if account.is_empty() || app.st.home.catalog.root_id() == root {
        return;
    }
    app.st.home.catalog.set_root(root);
    let mut tx = Tx::new();
    tx.kv_put(&account, schema::kv::DRIVE_ROOT, &Value::obj([("remoteId", Value::Str(root.into()))]));
    let req = app.req(Mod::Home);
    store::tx(app, host, req, tx);
}

/// A page of the full enumeration, or with `found` (its ids, sorted) its end: entries Drive no longer has are marked removed.
pub(crate) fn crawled(app: &mut App, host: &mut dyn Host, page: Vec<Entry>, found: Option<Vec<String>>) {
    let page = app.st.home.catalog.visible(page);
    if let Some(seen) = app.st.home.catalog.observe(&page, None, None) {
        persist(app, host, &seen.entries, false);
    }
    if let Some(found) = found {
        let gone = app.st.home.catalog.mark_missing(&found);
        persist(app, host, &gone, false);
        badges::refresh(app, host);
    }
}

// v5: app-v5.js:signOutDrive (Home's part) / offline-library-v5.js:logout
fn signed_out(app: &mut App, host: &mut dyn Host) {
    folders::reset(app, host);
    let st = &mut app.st.home;
    st.catalog.clear();
    st.account.clear();
    st.sync = badges::Sync::default();
    (st.reqs, st.batches, st.folder_jobs) = (Vec::new(), Vec::new(), Vec::new());
    (st.auto_busy, st.crawled) = (false, false);
    for list in [&mut st.recents, &mut st.starred, &mut st.shared, &mut st.folders, &mut st.files] {
        list.clear();
    }
    for slot in Slot::ALL {
        lists::clear_slot(app, host, slot);
    }
    folders::render_breadcrumbs(app, host);
    crate::drive::home::clear_media(app, host);
    auth::set_local_library(app, host, false);
}

fn use_account(app: &mut App, host: &mut dyn Host, email: &str) {
    let next = email.trim().to_ascii_lowercase();
    if next == app.st.home.account {
        return;
    }
    app.st.home.catalog.clear();
    app.st.home.account = next;
    app.st.home.crawled = false;
    load(app, host);
}

// ---- events -----------------------------------------------------------------------------------------------------

pub fn init(app: &mut App, host: &mut dyn Host) {
    // v5: the offline worker page runs no UI.
    if app.cfg.has(mode::HEADLESS) {
        return;
    }
    let out = host.out();
    let st = &mut app.st.home;
    st.on = true;
    st.main = out.query(Ui::DRIVE_HOME, ".drive-main");
    st.topbar = out.query(Ui::DRIVE_HOME, ".drive-topbar");
    st.sheet_grid = out.query(Ui::DRIVE_FOLDER_VIEW, ".drive-sheet-grid");
    st.view_inner = out.query(Ui::DRIVE_FOLDER_VIEW_SCROLL, ".drive-folder-view-inner");
    for (i, name) in nav::TABS.iter().enumerate() {
        st.tabs[i] = out.query(Ui::HOME_NAV, &format!("[data-home-tab=\"{name}\"]"));
    }
    for (i, name) in ["starred", "shared"].iter().enumerate() {
        st.links[i] = out.query(Ui::DRIVE_HOME, &format!("[data-home-tab-link=\"{name}\"]"));
        out.listen(st.links[i], "click", L_SEE_ALL + i as u32, 0);
    }
    for (node, listener) in [(Ui::HOME_NAV, L_NAV), (Ui::BTN_DRIVE_BACK, L_BACK), (Ui::BTN_FOLDER_VIEW_BACK, L_VIEW_BACK), (Ui::DRIVE_BREADCRUMBS, L_CRUMBS)] {
        out.listen(node, "click", listener, 0);
    }
    out.listen(Ui::DRIVE_SEARCH, "input", L_SEARCH, 0);
    out.listen(Ui::DRIVE_FOLDER_VIEW_SCROLL, "scroll", L_VIEW_SCROLL, crate::abi::listen::PASSIVE);
    out.listen(Node::DOCUMENT, "keydown", L_DOC_KEY, 0);
    // One listener per grid: the card is found from the event's path.
    for slot in Slot::ALL {
        out.listen(slot.node(), "click", L_SLOT_CLICK + slot as u32, 0);
        out.listen(slot.node(), "keydown", L_SLOT_KEY + slot as u32, 0);
        if slot.in_view() {
            out.listen(slot.node(), "focusin", L_SLOT_FOCUS + slot as u32, 0);
            out.listen(slot.node(), "animationend", L_SLOT_ANIM + slot as u32, 0);
        }
    }
    nav::restore_tab(app, host);
    pull::init(app, host);
    scroll::init(app, host);
    // v5: InhouseOfflineLibrary.init — the library of the account remembered on this device.
    app.st.home.account = auth::known_account(host);
    load(app, host);
    let _ = LsOp::Get;
}

fn file_json(e: &Entry) -> String {
    let parents: Vec<String> = e.parents.iter().flatten().map(|p| json_str(p)).collect();
    format!(
        "{{\"id\":{},\"name\":{},\"mimeType\":{},\"modifiedTime\":{},\"resourceKey\":{},\"thumbnailLink\":{},\"parents\":[{}],\"ownedByMe\":{},\"pending\":{}}}",
        json_str(&e.id), json_str(&e.name), json_str(&e.mime), json_str(&e.modified), json_str(&e.resource_key), json_str(&e.thumbnail), parents.join(","),
        e.owned_by_me.map_or("null".into(), |b| b.to_string()), e.pending
    )
}

/// A card was clicked, or Enter / Space pressed on it.
// v5: app-v5.js:bindDriveCardActivation
fn activate(app: &mut App, host: &mut dyn Host, slot: Slot, index: usize) {
    if app.st.home.slots[slot as usize].cards[index].deferred {
        // v5: the slot hydrates and forwards the click to the real card.
        folders::hydrate(app, host, slot, index);
        folders::invalidate_plans(app);
    }
    let card = &app.st.home.slots[slot as usize].cards[index];
    if card.folder {
        folders::open_from_card(app, host, slot, index);
    } else {
        let json = file_json(&card.entry);
        shell::signal(app, host, sig::OPEN_FILE, &json);
    }
}

fn slot_event(app: &mut App, host: &mut dyn Host, slot: Slot, e: &DomEvent) -> u32 {
    let Some(index) = app.st.home.slots[slot as usize].cards.iter().position(|c| e.path.contains(c.node)) else { return 0 };
    let card = &app.st.home.slots[slot as usize].cards[index];
    match e.ty {
        ety::CLICK => {
            if !card.badge.is_none() && e.path.contains(card.badge) {
                let (id, deco) = (card.id.clone(), card.deco.clone());
                badges::clicked(app, host, &id, &deco);
                return ret::PREVENT_DEFAULT | ret::STOP_PROPAGATION;
            }
            if !card.menu.is_none() && e.path.contains(card.menu) {
                return 0;
            }
            activate(app, host, slot, index);
        }
        ety::KEYDOWN => {
            if e.path.first() != card.node || !e.has(ev::EXACT) || !matches!(e.key, "Enter" | " ") {
                return 0;
            }
            activate(app, host, slot, index);
            return ret::PREVENT_DEFAULT;
        }
        ety::FOCUSIN => {
            if card.deferred && e.path.first() == card.node && e.has(ev::EXACT) {
                folders::hydrate(app, host, slot, index);
                folders::invalidate_plans(app);
            }
        }
        ety::ANIMATIONEND => {
            if card.arriving && e.path.first() == card.node && e.has(ev::EXACT) {
                app.st.home.slots[slot as usize].cards[index].arriving = false;
                host.out().class(e.path.first(), "drive-folder-item-arriving", false);
            }
        }
        _ => {}
    }
    0
}

fn on_dom(app: &mut App, host: &mut dyn Host, e: &DomEvent) -> u32 {
    match e.listener {
        L_NAV => nav::nav_clicked(app, host, e),
        l if l == L_SEE_ALL || l == L_SEE_ALL + 1 => nav::set_tab(app, host, if l == L_SEE_ALL { "starred" } else { "shared" }, false),
        L_BACK => {
            if app.st.home.stack.len() > 1 {
                folders::navigate_up(app, host);
            }
        }
        L_VIEW_BACK => folders::navigate_up(app, host),
        L_CRUMBS => folders::breadcrumb_clicked(app, host, &e.path),
        L_SEARCH => nav::search_input(app, host, e),
        L_VIEW_SCROLL => {
            if app.st.home.anim.is_none() {
                folders::hydrate_visible(app, host);
            }
            scroll::schedule(app, host);
        }
        L_DOC_KEY if e.key == "Escape" => nav::escape(app, host),
        L_TOUCH_START | L_TOUCH_MOVE | L_TOUCH_END | L_TOUCH_CANCEL => return pull::touch(app, host, e),
        L_THUMB_KEY => return scroll::key(app, host, e),
        L_MAIN_SCROLL | L_SCROLL_RESIZE | L_SCROLL_VISIBILITY | L_SCROLL_CONTENTS => scroll::schedule(app, host),
        L_GRID_RESIZE => folders::invalidate_plans(app),
        l if (L_SLOT_CLICK..L_SLOT_ANIM + 8).contains(&l) => return slot_event(app, host, Slot::ALL[((l - L_SLOT_CLICK) % 8) as usize % 7], e),
        _ => {
            if app.st.home.anim.as_ref().is_some_and(|a| a.zoom_listener() == e.listener) {
                folders::cleanup(app, host);
            } else {
                lists::decode_event(app, host, e);
            }
        }
    }
    0
}

fn frame(app: &mut App, host: &mut dyn Host) {
    let work = std::mem::take(&mut app.st.home.frames);
    if work & F_PULL_TOP != 0 {
        pull::update_top(app, host);
    }
    if work & F_PREPARE != 0 {
        folders::prepare_frame(app, host);
    }
    if work & F_SETTLE != 0 {
        folders::settle_frame(app, host);
    }
    if work & F_LABELS != 0 {
        folders::labels_frame(app, host);
    }
    if work & F_SCROLL != 0 {
        scroll::render(app, host);
    }
}

fn on_msg(app: &mut App, host: &mut dyn Host, chan: u8, data: &str) {
    match chan {
        shell_sig::REFRESH => refresh(app, host),
        shell_sig::VIEW => nav::auto_refresh(app, host, data == "home"),
        shell_sig::ACCOUNT => use_account(app, host, data),
        shell_sig::SIGNED_OUT => signed_out(app, host),
        // v5: updateDriveAuthUI leaves Back to the folder stack.
        shell_sig::AUTH => host.out().prop(Ui::BTN_DRIVE_BACK, "disabled", crate::Prop::Bool(app.st.home.stack.len() <= 1)),
        shell_sig::THEME => folders::repaint(app, host),
        // v5: connectivity → reload → refreshed → renderHome; back online, the lists are fetched again.
        shell_sig::NETWORK => {
            badges::refresh(app, host);
            lists::render_home(app, host);
            if data == "1" {
                let account = app.st.home.account.clone();
                crate::drive::home::crawl(app, host, &account);
                lists::refresh_quiet(app, host);
            }
        }
        _ => {}
    }
}

pub fn event(app: &mut App, host: &mut dyn Host, event: &Event) -> u32 {
    if !app.st.home.on {
        return 0;
    }
    match *event {
        Event::Dom(e) => return on_dom(app, host, &e),
        Event::Pointer(p) => return scroll::pointer(app, host, &p),
        Event::Frame(_) => frame(app, host),
        Event::Timer(T_SEARCH) => refresh(app, host),
        Event::Timer(T_AUTO) => nav::auto_tick(app, host),
        Event::Timer(T_PULL_RESET) => pull::reset_done(host),
        Event::Timer(T_PULL_HOLD) => pull::finish(app, host),
        Event::Timer(T_PREPARE) => folders::prepare_scene(app, host),
        Event::Timer(T_SETTLE) => folders::settle(app, host),
        Event::Io(io) if io.req == app.st.home.load_req && io.req != 0 => {
            app.st.home.load_req = 0;
            loaded(app, host, &io);
        }
        Event::Io(io) => {
            if let Some(at) = app.st.home.reqs.iter().position(|r| r.0 == io.req) {
                let (_, pending) = app.st.home.reqs.swap_remove(at);
                lists::list_answer(app, host, pending, &io);
            }
        }
        Event::Msg(m) => on_msg(app, host, m.chan, std::str::from_utf8(m.data).unwrap_or("")),
        Event::Life(l) => match l.code {
            life::RESIZE => {
                folders::resized(app, host);
                scroll::schedule(app, host);
            }
            life::VISIBILITY if l.a != 0.0 => folders::finish_animation(app, host),
            _ => {}
        },
        _ => {}
    }
    0
}

/// `__IHN_TEST_API__.cmd('home.<name>', args)`.
pub fn test_cmd(app: &mut App, host: &mut dyn Host, name: &str, args: &str) -> Option<String> {
    let arg = Json::parse(args).unwrap_or(Json::Null);
    let list = |ids: Vec<String>| format!("[{}]", ids.iter().map(|i| json_str(i)).collect::<Vec<_>>().join(","));
    Some(match name {
        "state" => {
            let st = &app.st.home;
            let slots: Vec<String> = Slot::ALL.iter().map(|s| format!("{}:{}", json_str(s.name()), list(lists::slot_ids(st, *s)))).collect();
            let deferred = Slot::ALL.iter().map(|s| st.slots[*s as usize].cards.iter().filter(|c| c.deferred).count()).sum::<usize>();
            format!(
                "{{\"tab\":{},\"account\":{},\"library\":{},\"entries\":{},\"folder\":{},\"stack\":{},\"viewOpen\":{},\"animating\":{},\"search\":{},\"requests\":{},\"deferred\":{deferred},\"pending\":{},\"slots\":{{{}}}}}",
                json_str(nav::tab(app)), json_str(&st.account), st.catalog.loaded, st.catalog.len(), json_str(&st.current),
                list(st.stack.iter().map(|c| c.name.clone()).collect()), st.view_open, st.anim.is_some(), json_str(search(app)),
                st.reqs.len(), st.catalog.pending_count(), slots.join(",")
            )
        }
        "refresh" => {
            refresh(app, host);
            "true".into()
        }
        "refresh_quiet" => {
            refresh_quiet(app, host);
            "true".into()
        }
        "tab" => {
            nav::set_tab(app, host, arg.as_str().unwrap_or("home"), false);
            json_str(nav::tab(app))
        }
        // {"phase":"uploading","id":"…"} and/or {"errors":[{"id","code","message","phase"}]}
        "sync" => {
            if let Some(errors) = arg.get("errors") {
                let errors = errors.items().iter().map(|e| badges::SyncError { id: e.text("id").into(), code: e.text("code").into(), message: e.text("message").into(), phase: e.text("phase").into() }).collect();
                badges::set_errors(app, host, errors);
            }
            if arg.get("phase").is_some() {
                badges::set_sync(app, host, arg.text("phase"), arg.text("id"));
            }
            "true".into()
        }
        // {"id":"…","pending":true,"available":true,"pinned":false,"lastOpenedAt":0} — or {"id":"…","remove":true}
        "doc" => {
            let flag = |k: &str| arg.get(k).is_some_and(Json::truthy);
            let info = (!flag("remove")).then(|| DocInfo { last_opened_at: arg.get("lastOpenedAt").and_then(Json::as_f64).unwrap_or(0.0), pinned: flag("pinned"), available: flag("available"), pending: flag("pending") });
            doc_changed(app, host, arg.text("id"), info);
            "true".into()
        }
        // Seeds `docs` records as the document unit writes them: [{"id","created","relayPending","pinned","lastOpenedAt","chunks"}]
        "seed_docs" => {
            let account = app.st.home.account.clone();
            let mut tx = Tx::new();
            for doc in arg.items() {
                let flag = |k: &str| doc.get(k).is_some_and(Json::truthy);
                let record = DocRecord {
                    account: account.clone(), name: doc.text("name").into(), pinned: flag("pinned"), last_opened_at: doc.get("lastOpenedAt").and_then(Json::as_f64).unwrap_or(0.0),
                    flags: schema::Flags { created: flag("created"), relay_pending: flag("relayPending"), ..Default::default() }, ..DocRecord::default()
                };
                tx.doc_put(doc.text("id"), &record);
                if flag("chunks") {
                    tx.chunk_put(doc.text("id"), 0, &[0]);
                }
            }
            let req = app.req(Mod::Home);
            store::tx(app, host, req, tx);
            load(app, host);
            "true".into()
        }
        "reload" => {
            load(app, host);
            "true".into()
        }
        "folder" => list(lists::folder_listing(app, host, arg.as_str().unwrap_or("root")).into_iter().map(|e| e.id).collect()),
        "scroll_view" => {
            folders::scroll_view(host, arg.as_f64().unwrap_or(0.0));
            "true".into()
        }
        _ => return None,
    })
}
