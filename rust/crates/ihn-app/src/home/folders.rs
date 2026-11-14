//! Folders: the navigation stack, breadcrumbs, the opened folder view with its deferred cards, and the zoom that
//! opens and closes a folder (5.18.0's scene, keyframes from `ihn_core::motion`).

use super::cards::{self, esc};
use super::{lists, nav, scroll, Card, Slot};
use crate::abi::{mode, observe};
use crate::dom::{Prop, Scroll};
use crate::host::Metric;
use crate::{shell, App, Host, Mod, Node, Rect, Ui};
use ihn_core::catalog::Entry;
use ihn_core::jsfmt::num;
use ihn_core::motion::{self, Box4, Motion};
use ihn_core::palette;
use ihn_core::rows::{px_list, Rows};

/// Where a folder was opened from, to zoom back to it.
#[derive(Clone, Debug, PartialEq)]
pub struct Origin {
    pub rect: Box4,
    pub viewport: (f64, f64),
    /// `scrollTop` of the folder view the card was in.
    pub scroll_top: f64,
}

/// One level of the folder path.
#[derive(Clone, Debug, PartialEq)]
pub struct Crumb {
    pub id: String,
    pub name: String,
    pub color: String,
    pub origin: Option<Origin>,
    /// The folder as its card showed it (the ghost is drawn from it).
    pub entry: Entry,
}

impl Crumb {
    pub fn root() -> Crumb { Crumb { id: "root".into(), name: "My Drive".into(), color: String::new(), origin: None, entry: Entry::default() } }
}

/// What the scene swaps in underneath the turning lid.
#[derive(Clone, Debug)]
enum Swap {
    Open(Crumb),
    CloseToRoot,
    Parent { entry: Crumb, scroll_top: f64 },
}

/// A running folder zoom (5.18.0 `driveFolderAnimation`).
pub struct Anim {
    backwards: bool,
    swap: Option<Swap>,
    entry: Crumb,
    layer: Node,
    ghost: Node,
    surface: Node,
    zoom: Node,
    zoom_listener: u32,
    anims: Vec<Node>,
    nodes: Vec<Node>,
    rect: Box4,
    bounds: Box4,
    motion: Motion,
    /// The card's art, hidden while its ghost moves.
    art: Node,
    dest_art: Node,
    /// The snapshot of the outgoing interior (closing), mounted in the ghost.
    previous: Node,
    started: f64,
    prepared: bool,
    pub(super) preparing: bool,
    live_moved: bool,
    focus_before: Node,
    pub(super) focus_target: Node,
    pub(super) refresh: Option<bool>,
    pub(super) pending_contents: Option<(Vec<Entry>, Vec<Entry>)>,
}

/// Work left for after the finished scene has painted.
pub struct Settle {
    nav: u32,
    pending_contents: Option<(Vec<Entry>, Vec<Entry>)>,
    refresh: Option<bool>,
}

/// The measured tracks of one grid of the folder view.
pub struct Plan {
    key: String,
    rows: Rows,
    heights: Vec<f64>,
    columns: usize,
    inset: f64,
    box_width: f64,
}

const VIEW_SLOTS: [Slot; 2] = [Slot::ViewFolders, Slot::ViewFiles];
const VIEW_PROPS: [&str; 9] = ["top", "left", "right", "bottom", "width", "height", "transform", "transform-origin", "opacity"];
fn px(v: f64) -> String { format!("{}px", num(v)) }

pub(super) fn view_open(app: &App) -> bool { app.st.home.view_open }
fn reduced(app: &App) -> bool { app.cfg.has(mode::REDUCED_MOTION) }
fn box_of(r: Rect) -> Box4 { Box4 { left: r.x, top: r.y, width: r.w, height: r.h } }

// v5: app-v5.js:renderDriveBreadcrumbs
pub(super) fn render_breadcrumbs(app: &mut App, host: &mut dyn Host) {
    let last = app.st.home.stack.len() - 1;
    let mut html = String::new();
    for (i, crumb) in app.st.home.stack.iter().enumerate() {
        let label = if crumb.id == "root" && last == 0 { format!("{}/", crumb.name) } else { crumb.name.clone() };
        html.push_str(&format!("<button class=\"drive-breadcrumb{}\">{}</button>", if i == last { " active" } else { "" }, esc(&label)));
        if i < last {
            html.push_str("<span>/</span>");
        }
    }
    let title = (last > 0).then(|| if app.st.home.stack[last].name.is_empty() { "Folder".to_string() } else { app.st.home.stack[last].name.clone() });
    let out = host.out();
    for node in std::mem::take(&mut app.st.home.crumb_nodes) {
        out.release(node);
    }
    out.html(Ui::DRIVE_BREADCRUMBS, &html);
    // Crumb `i` is child `2 i` (a separator follows each but the last).
    app.st.home.crumb_nodes = (0..last).map(|i| out.child(Ui::DRIVE_BREADCRUMBS, &[(i * 2).min(255) as u8])).collect();
    out.prop(Ui::BTN_DRIVE_BACK, "disabled", Prop::Bool(last == 0));
    if let Some(title) = title {
        out.text(Ui::DRIVE_FOLDER_VIEW_TITLE, &title);
    }
}

/// A click inside the breadcrumbs: every crumb but the last navigates to its folder.
pub(super) fn breadcrumb_clicked(app: &mut App, host: &mut dyn Host, path: &crate::abi::Handles) {
    if let Some(index) = app.st.home.crumb_nodes.iter().position(|n| path.contains(*n)) {
        navigate_to(app, host, index);
    }
}

// v5: app-v5.js:positionDriveFolderView
pub(super) fn position_view(app: &mut App, host: &mut dyn Host) -> f64 {
    let (topbar, home) = (host.rect(app.st.home.topbar), host.rect(Ui::DRIVE_HOME));
    let top = ihn_core::jsmath::round(topbar.bottom() - home.y).max(0.0);
    if app.st.home.view_top != top {
        app.st.home.view_top = top;
        host.out().style(Ui::DRIVE_FOLDER_VIEW, "--drive-folder-view-top", Some(&px(top)));
    }
    top
}

fn apply_view_palette(app: &mut App, host: &mut dyn Host, color: &str) {
    let p = palette::view_palette(color, shell::dark(app));
    let out = host.out();
    for (name, value) in p.vars() {
        out.style(Ui::DRIVE_FOLDER_VIEW, name, Some(value));
    }
    out.attr(Ui::DRIVE_FOLDER_VIEW, "data-folder-color", Some(color));
    app.st.home.view_color = color.to_string();
}

/// Re-shades every folder on screen after a theme switch.
// v5: app-v5.js:repaintDriveFolders
pub(super) fn repaint(app: &mut App, host: &mut dyn Host) {
    let night = shell::dark(app);
    for slot in Slot::ALL {
        for card in &app.st.home.slots[slot as usize].cards {
            if !card.folder || card.deferred {
                continue;
            }
            let p = palette::palette(&palette::resolve_color(&card.entry.folder_color), night, false);
            let out = host.out();
            let art = out.child(card.node, cards::ART_PATH);
            for (name, value) in p.vars() {
                out.style(art, name, Some(value));
            }
            out.release(art);
        }
    }
    if view_open(app) && !app.st.home.view_color.is_empty() {
        let color = app.st.home.view_color.clone();
        apply_view_palette(app, host, &color);
    }
}

// v5: app-v5.js:revealDriveFolderView
fn reveal_view(app: &mut App, host: &mut dyn Host, entry: &Crumb) {
    let color = if entry.color.is_empty() { palette::DEFAULT_FOLDER_COLOR.to_string() } else { entry.color.clone() };
    apply_view_palette(app, host, &color);
    let out = host.out();
    out.class(Ui::DRIVE_FOLDER_VIEW, "hidden", false);
    out.class(Ui::DRIVE_FOLDER_VIEW, "is-switching", false);
    out.class(Ui::DRIVE_FOLDER_VIEW, "is-leaving", false);
    out.prop(Ui::DRIVE_FOLDER_VIEW, "inert", Prop::Bool(false));
    out.class(Ui::DRIVE_HOME, "folder-open", true);
    app.st.home.view_open = true;
    let active = host.active();
    let main = app.st.home.main;
    if active.first().is_none_or(|n| *n == Node::BODY) || active.contains(&main) || active.contains(&Ui::DRIVE_FOLDER_VIEW) {
        match &mut app.st.home.anim {
            Some(anim) => anim.focus_target = Ui::BTN_FOLDER_VIEW_BACK,
            None => host.out().focus(Ui::BTN_FOLDER_VIEW_BACK, true),
        }
    }
}

// v5: app-v5.js:hideDriveFolderView
pub(super) fn hide_view(app: &mut App, host: &mut dyn Host, finish: bool) {
    if finish {
        finish_animation(app, host);
    }
    let out = host.out();
    out.class(Ui::DRIVE_FOLDER_VIEW, "hidden", true);
    out.class(Ui::DRIVE_FOLDER_VIEW, "is-switching", false);
    out.class(Ui::DRIVE_FOLDER_VIEW, "is-leaving", false);
    out.prop(Ui::DRIVE_FOLDER_VIEW, "inert", Prop::Bool(false));
    out.class(Ui::DRIVE_HOME, "folder-open", false);
    app.st.home.view_open = false;
}

/// Title, cached directory (or skeletons) and scroll position of the folder being shown.
// v5: app-v5.js:prepareDriveFolderViewContent
fn prepare_view_content(app: &mut App, host: &mut dyn Host, entry: &Crumb, scroll_top: f64) {
    let name = if entry.name.is_empty() { "Folder" } else { &entry.name };
    host.out().text(Ui::DRIVE_FOLDER_VIEW_TITLE, name);
    host.out().attr(Ui::DRIVE_FOLDER_VIEW, "aria-label", Some(&format!("{name} folder")));
    // Paint the saved directory immediately, even online: Drive reconciles it in the background.
    let (folder_list, file_list) = lists::cached_folder(app, host, &entry.id);
    invalidate_plans(app);
    for slot in VIEW_SLOTS {
        lists::clear_slot(app, host, slot);
    }
    if !folder_list.is_empty() || !file_list.is_empty() || shell::offline(app) {
        app.st.home.folders = folder_list.clone();
        app.st.home.files = file_list.clone();
        lists::render_folder_contents(app, host, folder_list, file_list);
    } else {
        host.out().html(Ui::DRIVE_VIEW_FILES, &cards::CARD_SKELETON.repeat(4));
        app.st.home.slots[Slot::ViewFiles as usize].skeletons = true;
        host.out().class(Ui::DRIVE_VIEW_EMPTY, "hidden", true);
    }
    host.out().prop(Ui::DRIVE_FOLDER_VIEW_SCROLL, "scrollTop", Prop::Num(scroll_top));
    if app.st.home.anim.is_none() {
        hydrate_visible(app, host);
    }
}

// ---- deferred cards ---------------------------------------------------------------------------------------------

pub(super) fn invalidate_plans(app: &mut App) { app.st.home.plans = [None, None]; }

/// Builds the real card inside a deferred slot.
// v5: app-v5.js:createDeferredDriveFolderCard `hydrate`
pub(super) fn hydrate(app: &mut App, host: &mut dyn Host, slot: Slot, index: usize) {
    let night = shell::dark(app);
    let Some(card) = app.st.home.slots[slot as usize].cards.get_mut(index).filter(|c| c.deferred) else { return };
    card.deferred = false;
    let (node, folder, arrive) = (card.node, card.folder, card.arriving);
    let entry = card.entry.clone();
    let mut today = cards::Today::new(host);
    let sub = if folder { String::new() } else { cards::describe(host, &mut today, &entry) };
    let card = &mut app.st.home.slots[slot as usize].cards[index];
    let inner = if folder { cards::folder_art(&entry, night, true, "", "", "", false) } else { cards::file_inner(&entry, &sub, &cards::Sheet::default(), &card.deco) };
    card.sub = sub;
    let mut class = String::from(if folder { "drive-folder-card" } else { "drive-card" });
    if card.deco.unavailable {
        class.push_str(" offline-unavailable");
    }
    if arrive {
        class.push_str(" drive-folder-item-arriving");
    }
    let had_badge = !card.badge.is_none();
    let out = host.out();
    if had_badge {
        out.release(card.badge);
    }
    out.attr(node, "aria-label", None);
    out.style(node, "aspect-ratio", None);
    out.class_set(node, &class);
    out.html(node, &inner);
    card.menu = out.child(node, if folder { cards::FOLDER_MENU_PATH } else { cards::FILE_MENU_PATH });
    card.badge = if had_badge { out.query(node, ".document-sync-status") } else { Node::NONE };
    card.fp = entry.fingerprint();
    if !folder {
        lists::refresh_thumb(app, host, slot, index);
    }
}

fn grid_node(app: &App, slot: Slot) -> Node { if slot == Slot::ViewFiles { app.st.home.sheet_grid } else { slot.node() } }

/// The item ranges of the folder view's grids that touch the viewport ± `overscan`.
// v5: app-v5.js:readDriveFolderGridPlans
fn read_plans(app: &mut App, host: &mut dyn Host, overscan: f64) -> Vec<(Slot, usize, usize)> {
    let viewport = host.rect(Ui::DRIVE_FOLDER_VIEW_SCROLL);
    let mut out = Vec::new();
    if viewport.w == 0.0 || viewport.h == 0.0 {
        return out;
    }
    for (i, slot) in VIEW_SLOTS.into_iter().enumerate() {
        let count = app.st.home.slots[slot as usize].cards.len();
        if count == 0 {
            continue;
        }
        // Files are `display: contents`: their tracks belong to the sheet grid, New PDF's row included.
        let grid = grid_node(app, slot);
        let rect = host.rect(grid);
        let columns_css = host.style(grid, "gridTemplateColumns");
        let columns = px_list(&columns_css).len();
        let width_css = host.style(grid, "width");
        let width = px_list(&width_css).first().copied().unwrap_or(0.0);
        if columns == 0 || width == 0.0 || rect.w == 0.0 {
            continue;
        }
        let gap_css = host.style(grid, "rowGap");
        let key = format!("{count}|{columns_css}|{width_css}|{gap_css}");
        if app.st.home.plans[i].as_ref().is_none_or(|p| p.key != key) {
            app.st.home.plans[i] = None;
            let heights = px_list(&host.style(grid, "gridTemplateRows"));
            let gap = px_list(&gap_css).first().copied().unwrap_or(0.0);
            let Some(rows) = Rows::new(&heights, gap, columns, count) else { continue };
            let mut read = |prop: &str| px_list(&host.style(grid, prop)).first().copied().unwrap_or(0.0);
            let inset = read("paddingTop") + read("borderTopWidth");
            let extra = read("paddingLeft") + read("paddingRight") + read("borderLeftWidth") + read("borderRightWidth");
            let border_box = host.style(grid, "boxSizing") == "border-box";
            app.st.home.plans[i] = Some(Plan { key, rows, heights, columns, inset, box_width: width + if border_box { 0.0 } else { extra } });
            if !std::mem::replace(&mut app.st.home.observing, true) {
                // A thumbnail, a font or the viewport can change track sizes.
                let sheet_grid = app.st.home.sheet_grid;
                host.out().observe(Ui::DRIVE_VIEW_FOLDERS, super::L_GRID_RESIZE, observe::RESIZE, "");
                host.out().observe(sheet_grid, super::L_GRID_RESIZE, observe::RESIZE, "");
            }
        }
        let Some(plan) = &app.st.home.plans[i] else { continue };
        let scale = rect.w / plan.box_width;
        if let Some((start, end)) = plan.rows.range((viewport.y - rect.y) / scale - plan.inset, (viewport.bottom() - rect.y) / scale - plan.inset, overscan) {
            out.push((slot, start, end));
        }
    }
    out
}

// v5: app-v5.js:hydrateVisibleDriveFolderCards
pub(super) fn hydrate_visible(app: &mut App, host: &mut dyn Host) {
    let any = |app: &App| VIEW_SLOTS.iter().any(|s| app.st.home.slots[*s as usize].cards.iter().any(|c| c.deferred));
    if !view_open(app) || !any(app) {
        return;
    }
    let plans = read_plans(app, host, 240.0);
    let mut visible: Vec<(Slot, usize)> = Vec::new();
    for (slot, start, end) in &plans {
        visible.extend((*start..*end).filter(|i| app.st.home.slots[*slot as usize].cards.get(*i).is_some_and(|c| c.deferred)).map(|i| (*slot, i)));
    }
    for slot in VIEW_SLOTS {
        if plans.iter().any(|p| p.0 == slot) || app.st.home.slots[slot as usize].cards.is_empty() {
            continue;
        }
        // A grid whose tracks could not be read: ask each slot where it is.
        let viewport = host.rect(Ui::DRIVE_FOLDER_VIEW_SCROLL);
        if viewport.w == 0.0 || viewport.h == 0.0 {
            return;
        }
        for i in 0..app.st.home.slots[slot as usize].cards.len() {
            let card = &app.st.home.slots[slot as usize].cards[i];
            if card.deferred {
                let rect = host.rect(card.node);
                if rect.bottom() >= viewport.y && rect.y <= viewport.bottom() + 240.0 {
                    visible.push((slot, i));
                }
            }
        }
    }
    let hydrated = !visible.is_empty();
    for (slot, i) in visible {
        hydrate(app, host, slot, i);
    }
    if hydrated {
        invalidate_plans(app);
    }
}

// ---- navigation -------------------------------------------------------------------------------------------------

// v5: app-v5.js:captureDriveFolderOrigin
fn capture_origin(app: &mut App, host: &mut dyn Host, art: Node) -> Option<Origin> {
    if art.is_none() {
        return None;
    }
    let rect = host.rect(art);
    if rect.w == 0.0 || rect.h == 0.0 {
        return None;
    }
    let scroll_top = host.metric(Ui::DRIVE_FOLDER_VIEW_SCROLL, Metric::ScrollTop);
    let _ = app;
    Some(Origin { rect: box_of(rect), viewport: (host.metric(Node::WINDOW, Metric::InnerWidth), host.metric(Node::WINDOW, Metric::InnerHeight)), scroll_top })
}

/// A visible art of folder `id` to land the closing zoom on (a new handle the caller releases).
// v5: app-v5.js:findVisibleDriveFolderArt
fn find_visible_art(app: &mut App, host: &mut dyn Host, id: &str) -> Node {
    if id.is_empty() {
        return Node::NONE;
    }
    let main = app.st.home.main;
    let mut containers = vec![(main, vec![Slot::Starred, Slot::Shared, Slot::Folders])];
    if view_open(app) {
        containers.insert(0, (Ui::DRIVE_FOLDER_VIEW_SCROLL, VIEW_SLOTS.to_vec()));
    }
    for (container, slots) in containers {
        let bounds = host.rect(container);
        for slot in slots {
            let Some(card) = app.st.home.slots[slot as usize].cards.iter().find(|c| c.id == id && c.folder && !c.deferred) else { continue };
            let art = host.out().child(card.node, cards::ART_PATH);
            let r = host.rect(art);
            if r.w > 0.0 && r.right() > bounds.x && r.x < bounds.right() && r.bottom() > bounds.y + 8.0 && r.y < bounds.bottom() - 8.0 {
                return art;
            }
            host.out().release(art);
        }
    }
    Node::NONE
}

/// Opens the folder of a card with the zoom.
// v5: app-v5.js:openDriveFolderFromCard
pub(super) fn open_from_card(app: &mut App, host: &mut dyn Host, slot: Slot, index: usize) {
    let Some(card) = app.st.home.slots[slot as usize].cards.get(index) else { return };
    if card.id.is_empty() {
        return;
    }
    let (folder, node) = (card.entry.clone(), card.node);
    nav::close_card_menu(app, host);
    finish_animation(app, host);
    let art = host.out().child(node, cards::ART_PATH);
    let origin = capture_origin(app, host, art);
    let entry = Crumb { id: folder.id.clone(), name: if folder.name.is_empty() { "Folder".into() } else { folder.name.clone() }, color: palette::resolve_color(&folder.folder_color), origin, entry: folder };
    let st = &mut app.st.home;
    if st.view_open && slot.in_view() {
        if st.stack.last().is_some_and(|c| c.id == entry.id) {
            host.out().release(art);
            return;
        }
        st.stack.push(entry.clone());
    } else {
        st.stack = vec![Crumb::root(), entry.clone()];
        st.origin_id = Some(entry.id.clone());
    }
    st.current = entry.id.clone();
    st.nav_id += 1;
    position_view(app, host);
    play(app, host, art, entry.clone(), Swap::Open(entry), false);
}

// v5: app-v5.js:closeDriveFolderView
pub(super) fn close_view(app: &mut App, host: &mut dyn Host, animate: bool) {
    if !view_open(app) && app.st.home.stack.len() <= 1 {
        return;
    }
    finish_animation(app, host);
    let st = &mut app.st.home;
    let closing = st.stack.get(1).or(st.stack.last()).cloned().unwrap_or_else(Crumb::root);
    let origin_id = st.origin_id.take().unwrap_or_else(|| closing.id.clone());
    let animate = animate && st.view_open && !app.cfg.has(mode::REDUCED_MOTION);
    st.shown_stack = Some(std::mem::replace(&mut st.stack, vec![Crumb::root()]));
    st.current = "root".into();
    st.nav_id += 1;
    let focus_in_view = host.active().contains(&Ui::DRIVE_FOLDER_VIEW);
    let art = if animate { find_visible_art(app, host, &origin_id) } else { Node::NONE };
    if animate {
        play(app, host, art, closing, Swap::CloseToRoot, true);
    } else {
        swap(app, host, Swap::CloseToRoot);
    }
    if focus_in_view {
        // The card the folder was opened from gets the focus back.
        let card = [Slot::Starred, Slot::Shared, Slot::Folders].iter().flat_map(|s| app.st.home.slots[*s as usize].cards.iter()).find(|c| c.id == origin_id && c.folder).map(|c| c.node);
        let visible = card.filter(|n| host.rect(*n).w > 0.0);
        if let Some(node) = visible {
            match &mut app.st.home.anim {
                Some(anim) => anim.focus_target = node,
                None => host.out().focus(node, true),
            }
        }
    }
    lists::refresh_folder_contents(app, host, true, None);
}

// v5: app-v5.js:navigateDriveFolderUp
pub(super) fn navigate_up(app: &mut App, host: &mut dyn Host) {
    let depth = app.st.home.stack.len();
    if depth <= 2 {
        close_view(app, host, true);
    } else {
        navigate_to(app, host, depth - 2);
    }
}

// v5: app-v5.js:navigateDriveFolderTo
pub(super) fn navigate_to(app: &mut App, host: &mut dyn Host, index: usize) {
    if index == 0 {
        return close_view(app, host, true);
    }
    if index >= app.st.home.stack.len() - 1 {
        return;
    }
    finish_animation(app, host);
    let st = &mut app.st.home;
    let closing = st.stack[index + 1].clone();
    st.shown_stack = Some(st.stack.clone());
    st.stack.truncate(index + 1);
    let entry = st.stack[index].clone();
    st.current = entry.id.clone();
    st.nav_id += 1;
    let scroll_top = closing.origin.as_ref().map_or(0.0, |o| o.scroll_top);
    play(app, host, Node::NONE, closing, Swap::Parent { entry, scroll_top }, true);
}

fn swap(app: &mut App, host: &mut dyn Host, what: Swap) {
    app.st.home.shown_stack = None;
    match what {
        Swap::Open(entry) => {
            reveal_view(app, host, &entry);
            prepare_view_content(app, host, &entry, 0.0);
            render_breadcrumbs(app, host);
            shell::home_status(host, "Loading folder...");
            lists::refresh_folder_contents(app, host, false, None);
        }
        Swap::CloseToRoot => {
            render_breadcrumbs(app, host);
            hide_view(app, host, false);
        }
        Swap::Parent { entry, scroll_top } => {
            reveal_view(app, host, &entry);
            prepare_view_content(app, host, &entry, scroll_top);
            render_breadcrumbs(app, host);
            host.out().prop(Ui::DRIVE_FOLDER_VIEW_SCROLL, "scrollTop", Prop::Num(scroll_top));
            shell::home_status(host, "Loading folder...");
            lists::refresh_folder_contents(app, host, false, None);
        }
    }
    scroll::schedule(app, host);
}

// ---- the zoom ---------------------------------------------------------------------------------------------------

const BACK_ICON: &str = "<svg viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\" aria-hidden=\"true\"><path d=\"m15 6-6 6 6 6\"></path></svg>";
const NEW_FOLDER_ICON: &str = "<svg viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\" aria-hidden=\"true\"><path d=\"M4 20h16a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13c0 1.1.9 2 2 2Z\"></path><line x1=\"12\" y1=\"10\" x2=\"12\" y2=\"16\"></line><line x1=\"9\" y1=\"13\" x2=\"15\" y2=\"13\"></line></svg>";
const NEW_PDF_ICON: &str = "<svg viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\" stroke-linecap=\"round\" aria-hidden=\"true\"><path d=\"M12 5v14M5 12h14\"></path></svg>";

fn card_html(card: &Card, night: bool) -> String {
    if card.deferred {
        cards::deferred_card(&card.entry, false, (!card.folder).then_some(&card.deco))
    } else if card.folder {
        cards::folder_card(&card.entry, night, false)
    } else {
        cards::file_card(&card.entry, &card.sub, &card.sheet, &card.deco, false)
    }
}

/// A still copy of the open folder view: whole off-screen rows become one empty track each, so its cost depends
/// on the visible rows while its scroll position stays exact. Returns the HTML and the view's `scrollTop`.
// v5: app-v5.js:snapshotDriveFolderView
fn snapshot(app: &mut App, host: &mut dyn Host) -> Option<(String, f64)> {
    let shown = app.st.home.shown_stack.take();
    if !view_open(app) {
        return None;
    }
    let plans = read_plans(app, host, 0.0);
    let night = shell::dark(app);
    let st = &app.st.home;
    let grid = |slot: Slot| -> String {
        let list = &st.slots[slot as usize].cards;
        let i = (slot == Slot::ViewFiles) as usize;
        let (Some(&(_, start, end)), Some(plan)) = (plans.iter().find(|p| p.0 == slot), st.plans[i].as_ref()) else {
            if st.slots[slot as usize].skeletons {
                return cards::CARD_SKELETON.repeat(4);
            }
            return list.iter().map(|c| card_html(c, night)).collect();
        };
        let (columns, count) = (plan.columns, list.len());
        let whole = count / columns;
        let spacer = |from: usize, to: usize| if to <= from { String::new() } else {
            format!("<div class=\"drive-folder-snapshot-spacer\" aria-hidden=\"true\" style=\"grid-column: 1 / -1; height: {};\"></div>", px(plan.rows.span(from, to)))
        };
        let mut html = spacer(0, whole.min(start / columns));
        html.extend(list[start..end].iter().map(|c| card_html(c, night)));
        html.push_str(&spacer(end.div_ceil(columns), whole));
        // The incomplete last row keeps its cells: New PDF shares that row.
        if count % columns != 0 && (end <= whole * columns || start == count) {
            for card in &list[whole * columns..] {
                html.push_str(&format!("<div class=\"{}\" style=\"height: {};\"></div>", if card.folder { "drive-folder-card" } else { "drive-card" }, px(plan.heights.get(whole).copied().unwrap_or(0.0))));
            }
        }
        html
    };
    let stack = shown.as_ref().unwrap_or(&st.stack);
    let last = stack.len() - 1;
    let crumbs: String = stack.iter().enumerate().map(|(i, c)| {
        format!("<button class=\"drive-breadcrumb{}\">{}</button>{}", if i == last { " active" } else { "" }, esc(&c.name), if i < last { "<span>/</span>" } else { "" })
    }).collect();
    let name = stack.last().map_or("Folder", |c| if c.name.is_empty() { "Folder" } else { &c.name });
    let empty = st.slots[Slot::ViewFolders as usize].cards.is_empty() && st.slots[Slot::ViewFiles as usize].cards.is_empty() && !st.slots[Slot::ViewFiles as usize].skeletons;
    let style = format!("--drive-folder-view-top: {}; {}", px(st.view_top), cards::palette_style(&palette::view_palette(&st.view_color, night)));
    let html = format!(
        "<section class=\"drive-folder-view{{CLASS}}\" aria-label=\"{} folder\" style=\"{style}{{STYLE}}\" data-folder-color=\"{}\" inert aria-hidden=\"true\"><div class=\"drive-folder-view-scroll\"><div class=\"drive-folder-view-inner\"><div class=\"drive-folder-view-head\"><button class=\"drive-folder-view-btn\" type=\"button\" title=\"Close folder\" aria-label=\"Close folder\">{BACK_ICON}</button><div class=\"drive-folder-view-heading\"><h2>{}</h2><nav class=\"drive-breadcrumbs\" aria-label=\"Folder path\">{crumbs}</nav></div><button class=\"drive-folder-view-btn\" type=\"button\" title=\"New folder\" aria-label=\"New folder\">{NEW_FOLDER_ICON}</button></div><div class=\"drive-folder-grid\">{}</div><div class=\"drive-sheet-grid\"><div class=\"drive-sheet-grid-items\">{}</div><button class=\"drive-new-sheet\" type=\"button\">{NEW_PDF_ICON}<span>New PDF here</span></button></div><div class=\"drive-empty{}\">This folder is empty.</div></div></div></section>",
        esc(name), esc(&st.view_color), esc(name), grid(Slot::ViewFolders), grid(Slot::ViewFiles), if empty { "" } else { " hidden" },
    );
    Some((html, host.metric(Ui::DRIVE_FOLDER_VIEW_SCROLL, Metric::ScrollTop)))
}

fn interior_style(m: &Motion, bounds: &Box4, hidden: bool) -> String {
    format!(
        " top: {}; left: {}; right: auto; bottom: auto; width: {}; height: {}; transform-origin: 0px 0px; transform: none;{}",
        px(-m.final_top()), px(-m.final_left()), px(bounds.width), px(bounds.height), if hidden { " opacity: 0;" } else { "" }
    )
}

fn timing(m: &Motion, easing: &str, delay: f64) -> String {
    format!("{{\"duration\":{},\"fill\":\"both\",\"easing\":\"{easing}\"{}}}", num(m.duration()), if delay != 0.0 { format!(",\"delay\":{}", num(delay)) } else { String::new() })
}

// v5: app-v5.js:createDriveFolderGhost `sizeGhost`
fn ghost_size(m: &Motion, rect: &Box4, bounds: &Box4) -> [(&'static str, String); 6] {
    let s = m.scale();
    [
        ("left", px(rect.left - bounds.left)), ("top", px(rect.top - bounds.top)), ("width", px(rect.width * s)), ("height", px(rect.height * s)),
        ("perspective", px(1100.0 * s)), ("--ghost-scale", num(s)),
    ]
}

/// Plays the zoom between a folder card and the full folder view; `what` runs underneath the turning lid.
// v5: app-v5.js:playDriveFolderTransition / createDriveFolderGhost
fn play(app: &mut App, host: &mut dyn Host, art: Node, entry: Crumb, what: Swap, backwards: bool) {
    let instant = |app: &mut App, host: &mut dyn Host, art: Node, what: Swap| {
        if !art.is_none() {
            host.out().release(art);
        }
        swap(app, host, what);
    };
    if reduced(app) {
        return instant(app, host, art, what);
    }
    // The outgoing viewport is read before new surfaces are attached.
    let previous_html = if backwards { snapshot(app, host) } else { None };
    let home = host.rect(Ui::DRIVE_HOME);
    let top = home.y + position_view(app, host);
    let bounds = Box4 { left: home.x, top, width: home.w, height: (home.bottom() - top).max(1.0) };
    let live = capture_origin(app, host, art);
    let from_origin = live.is_none();
    let source = live.or_else(|| entry.origin.clone());
    let viewport = (host.metric(Node::WINDOW, Metric::InnerWidth), host.metric(Node::WINDOW, Metric::InnerHeight));
    let rect = match &source {
        None => {
            let width = 180f64.min(bounds.width * 0.36).min(bounds.height * 0.4);
            Box4 { width, height: width * 297.0 / 210.0, left: bounds.left + (bounds.width - width) / 2.0, top: bounds.top + (bounds.height - width * 297.0 / 210.0) / 2.0 }
        }
        Some(o) => {
            let r = o.rect;
            let outside = r.left + r.width <= bounds.left || r.left >= bounds.left + bounds.width || r.top + r.height <= bounds.top || r.top >= bounds.top + bounds.height;
            if from_origin && (o.viewport != viewport || outside) {
                // A saved card after a resize: bring its place back inside the view.
                let fit = 1f64.min((bounds.width - 24.0) / r.width).min((bounds.height - 24.0) / r.height);
                let (width, height) = (r.width * fit, r.height * fit);
                Box4 { width, height, left: (bounds.left + 12.0).max(r.left.min(bounds.left + bounds.width - width - 12.0)), top: (bounds.top + 12.0).max(r.top.min(bounds.top + bounds.height - height - 12.0)) }
            } else {
                r
            }
        }
    };
    let Some(m) = motion::folder_motion(bounds, rect, backwards) else { return instant(app, host, art, what) };
    let resting = if art.is_none() { String::from("none") } else {
        let cover = host.out().child(art, cards::COVER_PATH);
        let t = host.style(cover, "transform");
        host.out().release(cover);
        t
    };
    let closed = if resting.is_empty() || resting == "none" { "rotateY(0deg)".to_string() } else { resting };
    let night = shell::dark(app);
    // The printed name and the ⋮ ride on the lid; the ghost is inert, so its copy can never be pressed.
    let mut ghost_entry = entry.entry.clone();
    if source.is_none() {
        ghost_entry = Entry { name: entry.name.clone(), folder_color: entry.color.clone(), ..ghost_entry };
    }
    let size: String = ghost_size(&m, &rect, &bounds).iter().map(|(k, v)| format!(" {k}: {v};")).collect();
    let ghost_html = cards::folder_art(&ghost_entry, night, source.is_some(), " drive-folder-ghost", &format!(" aria-hidden=\"true\" inert data-direction=\"{}\"", if backwards { "close" } else { "open" }), &size, true);
    let layer_html = format!(
        "<div class=\"drive-folder-transition\" aria-hidden=\"true\" inert style=\"left: {}; top: {}; width: {}; height: {};\">{ghost_html}</div>",
        px(bounds.left), px(bounds.top), px(bounds.width), px(bounds.height)
    );
    let focus_before = host.active().first().copied().unwrap_or(Node::BODY);
    let out = host.out();
    let layer = out.insert_html(Ui::DRIVE_HOME, Node::NONE, &layer_html);
    let ghost = out.child(layer, &[0]);
    let surface = out.child(ghost, &[2]);
    let cover = out.child(ghost, &[3]);
    if !art.is_none() {
        out.style(art, "visibility", Some("hidden"));
    }
    out.prop(Ui::DRIVE_FOLDER_VIEW, "inert", Prop::Bool(true));
    // Closing starts with the outgoing interior already in the scene.
    let mut previous = Node::NONE;
    if let Some((html, scroll_top)) = previous_html.as_ref().filter(|_| backwards) {
        previous = out.insert_html(surface, Node::NONE, &html.replace("{CLASS}", "").replace("{STYLE}", &interior_style(&m, &bounds, false)));
        if *scroll_top != 0.0 {
            let scroller = out.child(previous, &[0]);
            out.prop(scroller, "scrollTop", Prop::Num(*scroll_top));
            out.release(scroller);
        }
    }
    // The lid and the zoom are one movement on one timeline, created together in the tap's task.
    let zoom = out.animate(ghost, &m.zoom_frames(), &timing(&m, &m.easing(), 0.0));
    let mut anims = vec![out.animate(cover, &m.hinge_frames(&closed), &timing(&m, &m.hinge_easing(), 0.0))];
    let print = out.child(cover, &[0, 0]);
    anims.push(out.animate(print, &m.label_frames(), &timing(&m, "linear", 0.0)));
    let mut nodes = vec![cover, print];
    if source.is_some() {
        let menu = out.child(cover, &[0, 1]);
        anims.push(out.animate(menu, &m.label_frames(), &timing(&m, "linear", 0.0)));
        nodes.push(menu);
    }
    if !previous.is_none() {
        let frames = motion::folder_fade(0.0, true).unwrap_or_default();
        anims.push(out.animate(previous, &motion::fade_frames(&frames), &timing(&m, "linear", 0.0)));
    }
    let st = &mut app.st.home;
    st.next_dyn = (st.next_dyn + 2) & 0x0f_fffe;
    let zoom_listener = crate::tag(Mod::Home, super::L_DYNAMIC + st.next_dyn);
    out.listen(zoom, "finish", zoom_listener, 0);
    st.anim = Some(Box::new(Anim {
        backwards, swap: Some(what), entry, layer, ghost, surface, zoom, zoom_listener, anims, nodes, rect, bounds, motion: m, art, dest_art: Node::NONE,
        previous, started: host.now(), prepared: false, preparing: false, live_moved: false,
        focus_before, focus_target: Node::NONE, refresh: None, pending_contents: None,
    }));
    // Right after the first frame, the next screen is built underneath the turning lid.
    st.frames |= super::F_PREPARE;
    app.sched.frame(host.out(), Mod::Home);
}

/// The first frame of the zoom was painted.
pub(super) fn prepare_frame(app: &mut App, host: &mut dyn Host) {
    if app.st.home.anim.is_some() {
        app.sched.timer(host.out(), super::T_PREPARE, 0.0);
    }
}

fn mount_previous(host: &mut dyn Host, anim: &Anim, html: &str, scroll_top: f64, parent: Node, before: Node, class: &str, style: &str) -> Node {
    let out = host.out();
    let node = out.insert_html(parent, before, &html.replace("{CLASS}", class).replace("{STYLE}", style));
    if scroll_top != 0.0 {
        let scroller = out.child(node, &[0]);
        out.prop(scroller, "scrollTop", Prop::Num(scroll_top));
        out.release(scroller);
    }
    let _ = anim;
    node
}

/// Builds the destination under the lid and lets its fade join the running movement.
// v5: app-v5.js:playDriveFolderTransition `prepareScene` / `joinMotion`
pub(super) fn prepare_scene(app: &mut App, host: &mut dyn Host) {
    let Some(anim) = app.st.home.anim.as_mut().filter(|a| !a.prepared) else { return };
    anim.prepared = true;
    let backwards = anim.backwards;
    if !backwards {
        // A nested opening keeps the old parent behind the scene.
        if let Some((html, scroll_top)) = snapshot(app, host) {
            let Some(anim) = app.st.home.anim.as_mut() else { return };
            let node = mount_previous(host, anim, &html, scroll_top, anim.layer, anim.ghost, " drive-folder-transition-backdrop", " top: 0px; left: 0px; right: 0px; bottom: 0px;");
            anim.nodes.push(node);
        }
    }
    let Some(anim) = app.st.home.anim.as_mut() else { return };
    anim.preparing = true;
    let what = anim.swap.take();
    if let Some(what) = what {
        swap(app, host, what);
    }
    let Some(anim) = app.st.home.anim.as_mut() else { return };
    anim.preparing = false;
    let elapsed = (host.now() - anim.started).max(0.0);
    if backwards {
        // The landing card may still be a deferred slot.
        hydrate_visible(app, host);
        let id = app.st.home.anim.as_ref().map_or(String::new(), |a| a.entry.id.clone());
        let dest = find_visible_art(app, host, &id);
        if let Some(origin) = capture_origin(app, host, dest) {
            let Some(anim) = app.st.home.anim.as_mut() else { return };
            let (r, d) = (anim.rect, origin.rect);
            if [(r.left, d.left), (r.top, d.top), (r.width, d.width), (r.height, d.height)].iter().any(|(a, b)| (a - b).abs() > 0.5) {
                if let Some(m) = motion::folder_motion(anim.bounds, d, true) {
                    // The ghost is retargeted: its zoom restarts from where the movement is.
                    anim.rect = d;
                    anim.motion = m;
                    let out = host.out();
                    for (name, value) in ghost_size(&anim.motion, &anim.rect, &anim.bounds) {
                        out.style(anim.ghost, name, Some(&value));
                    }
                    if !anim.previous.is_none() {
                        let m = &anim.motion;
                        for (name, value) in [("top", px(-m.final_top())), ("left", px(-m.final_left()))] {
                            out.style(anim.previous, name, Some(&value));
                        }
                    }
                    out.unlisten(anim.zoom_listener);
                    out.anim_cancel(anim.zoom);
                    out.release(anim.zoom);
                    anim.zoom = out.animate(anim.ghost, &anim.motion.zoom_frames(), &timing(&anim.motion, &anim.motion.easing(), -elapsed));
                    out.listen(anim.zoom, "finish", anim.zoom_listener, 0);
                }
            }
            anim.dest_art = dest;
            host.out().style(dest, "visibility", Some("hidden"));
        } else if !dest.is_none() {
            host.out().release(dest);
        }
    }
    let Some(anim) = app.st.home.anim.as_mut() else { return };
    let out = host.out();
    out.prop(Ui::DRIVE_FOLDER_VIEW, "inert", Prop::Bool(true));
    out.class(Ui::DRIVE_FOLDER_VIEW, "is-leaving", true);
    if !backwards {
        // The live view keeps its final layout inside the ghost; only the outer surface shrinks.
        let scroll_top = host.metric(Ui::DRIVE_FOLDER_VIEW_SCROLL, Metric::ScrollTop);
        let m = &anim.motion;
        let out = host.out();
        for (name, value) in [("top", px(-m.final_top())), ("left", px(-m.final_left())), ("right", "auto".into()), ("bottom", "auto".into()), ("width", px(anim.bounds.width)), ("height", px(anim.bounds.height)), ("transform-origin", "0 0".into()), ("transform", "none".into()), ("opacity", "0".into())] {
            out.style(Ui::DRIVE_FOLDER_VIEW, name, Some(&value));
        }
        out.insert(anim.surface, Node::NONE, Ui::DRIVE_FOLDER_VIEW);
        if scroll_top != 0.0 {
            out.prop(Ui::DRIVE_FOLDER_VIEW_SCROLL, "scrollTop", Prop::Num(scroll_top));
        }
        anim.live_moved = true;
        hydrate_visible(app, host);
        // Invisible where it arrives and fully in by 70 % of the zoom.
        let Some(anim) = app.st.home.anim.as_mut() else { return };
        let frames = motion::folder_fade((elapsed / anim.motion.duration()).clamp(0.0, 1.0), false).unwrap_or_default();
        let fade = host.out().animate(Ui::DRIVE_FOLDER_VIEW, &motion::fade_frames(&frames), &timing(&anim.motion, "linear", -elapsed));
        anim.anims.push(fade);
    }
}

// v5: app-v5.js:finishDriveFolderAnimation
pub(super) fn finish_animation(app: &mut App, host: &mut dyn Host) {
    if app.st.home.anim.is_some() {
        cleanup(app, host);
    }
}

/// Ends the scene: the live view goes back to its place, the layer disappears, focus returns.
// v5: app-v5.js:playDriveFolderTransition `cleanup`
pub(super) fn cleanup(app: &mut App, host: &mut dyn Host) {
    prepare_scene(app, host);
    app.sched.clear(host.out(), super::T_PREPARE);
    app.st.home.frames &= !super::F_PREPARE;
    let Some(anim) = app.st.home.anim.take() else { return };
    if anim.live_moved {
        let scroll_top = host.metric(Ui::DRIVE_FOLDER_VIEW_SCROLL, Metric::ScrollTop);
        // These cards were already visible inside the zoom: their arrival fade must not restart.
        for slot in VIEW_SLOTS {
            for card in app.st.home.slots[slot as usize].cards.iter_mut().filter(|c| c.arriving) {
                card.arriving = false;
                host.out().class(card.node, "drive-folder-item-arriving", false);
            }
        }
        let out = host.out();
        // Labels move without their backdrop blur; it eases back in once the landed view is styled.
        out.class(Ui::DRIVE_FOLDER_VIEW, "is-settling-labels", true);
        out.insert(Ui::DRIVE_HOME, Ui::HOME_NAV, Ui::DRIVE_FOLDER_VIEW);
        for name in VIEW_PROPS {
            out.style(Ui::DRIVE_FOLDER_VIEW, name, None);
        }
        if scroll_top != 0.0 {
            out.prop(Ui::DRIVE_FOLDER_VIEW_SCROLL, "scrollTop", Prop::Num(scroll_top));
        }
        app.st.home.settling_labels = 2;
        app.st.home.frames |= super::F_LABELS;
    }
    let out = host.out();
    out.unlisten(anim.zoom_listener);
    for a in anim.anims.iter().chain([&anim.zoom]) {
        out.anim_cancel(*a);
        out.release(*a);
    }
    out.remove(anim.layer);
    for node in anim.nodes.iter().chain([&anim.surface, &anim.ghost, &anim.layer, &anim.previous]) {
        if !node.is_none() {
            out.release(*node);
        }
    }
    for art in [anim.art, anim.dest_art] {
        if !art.is_none() {
            out.style(art, "visibility", None);
            out.release(art);
        }
    }
    out.prop(Ui::DRIVE_FOLDER_VIEW, "inert", Prop::Bool(false));
    out.class(Ui::DRIVE_FOLDER_VIEW, "is-leaving", false);
    if !anim.focus_target.is_none() {
        let active = host.active().first().copied().unwrap_or(Node::BODY);
        if active == Node::BODY || active == anim.focus_before {
            host.out().focus(anim.focus_target, true);
        }
    }
    // Let the completed scene paint before reconciliation or thumbnail work takes the main thread.
    app.st.home.settle = Some(Settle { nav: app.st.home.nav_id, pending_contents: anim.pending_contents, refresh: anim.refresh });
    app.st.home.frames |= super::F_SETTLE;
    app.sched.frame(host.out(), Mod::Home);
    scroll::schedule(app, host);
}

/// One frame after the scene ended.
pub(super) fn settle_frame(app: &mut App, host: &mut dyn Host) {
    if app.st.home.settle.is_some() {
        app.sched.timer(host.out(), super::T_SETTLE, 0.0);
    }
}

pub(super) fn settle(app: &mut App, host: &mut dyn Host) {
    let Some(work) = app.st.home.settle.take() else { return };
    lists::release_motion_waiters(app, host);
    if work.nav != app.st.home.nav_id || app.st.home.anim.is_some() {
        return;
    }
    if let Some((folder_list, file_list)) = work.pending_contents {
        lists::render_folder_contents(app, host, folder_list, file_list);
    }
    if let Some(silent) = work.refresh {
        lists::refresh_folder_contents(app, host, silent, None);
    }
    hydrate_visible(app, host);
}

/// The two frames after which the label blur returns.
pub(super) fn labels_frame(app: &mut App, host: &mut dyn Host) {
    let st = &mut app.st.home;
    st.settling_labels = st.settling_labels.saturating_sub(1);
    if st.settling_labels == 0 {
        host.out().class(Ui::DRIVE_FOLDER_VIEW, "is-settling-labels", false);
    } else {
        st.frames |= super::F_LABELS;
        app.sched.frame(host.out(), Mod::Home);
    }
}

/// The window was resized.
pub(super) fn resized(app: &mut App, host: &mut dyn Host) {
    if app.st.home.anim.is_some() {
        cleanup(app, host);
        position_view(app, host);
    }
    if view_open(app) {
        position_view(app, host);
        if app.st.home.anim.is_none() {
            hydrate_visible(app, host);
        }
    }
}

/// Sign-out: back to an empty root.
pub(super) fn reset(app: &mut App, host: &mut dyn Host) {
    finish_animation(app, host);
    let st = &mut app.st.home;
    st.stack = vec![Crumb::root()];
    st.current = "root".into();
    st.origin_id = None;
    st.nav_id += 1;
    hide_view(app, host, false);
    let _ = Scroll::Auto;
}

/// Scrolls the folder view (tests).
pub(super) fn scroll_view(host: &mut dyn Host, top: f64) { host.out().prop(Ui::DRIVE_FOLDER_VIEW_SCROLL, "scrollTop", Prop::Num(top)); }

impl Anim {
    pub(super) fn zoom_listener(&self) -> u32 { self.zoom_listener }
}
