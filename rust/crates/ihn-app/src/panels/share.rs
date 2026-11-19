//! The Share modal (spec/home.md §9.1, 5.18.0 §4.21a): people with access, invite, row role change and removal,
//! General access (Restricted / Anyone with the link, Viewer / Editor) and Copy link. Drive calls are
//! `drive::perms`'s. The editor header's `#btn-drive-share` is shown while a Drive document is open.
// v5: app-v5.js:setupShareModalEvents (refreshShareModalData, renderOwnerOnlyRow, the listeners)

use crate::abi::{ev, ret, tag, Event, Io, Node};
use crate::doc::{public, session};
use crate::dom::Prop;
use crate::drive::perms::{self, OwnerInfo, Perm};
use crate::home::cards::esc;
use crate::shell::fx::{self, Response};
use crate::shell::{Status, StatusOpts};
use crate::testapi::json_str;
use crate::ui_ids::Ui;
use crate::{auth, shell, App, Host, Mod};

const fn id(n: u32) -> u32 { tag(Mod::PanelsShare, n) }
const L_OPEN: u32 = id(1);
const L_CLOSE: u32 = id(2);
const L_OVERLAY: u32 = id(3);
const L_SUBMIT: u32 = id(4);
const L_EMAIL: u32 = id(5);
const L_INVITE_ROLE: u32 = id(6);
const L_ACCESS: u32 = id(7);
const L_LINK_ROLE: u32 = id(8);
const L_COPY: u32 = id(9);
/// `+ i` for the role select / Remove button of row `i`.
const L_ROW_ROLE: u32 = id(0x100);
const L_ROW_REMOVE: u32 = id(0x200);
const MAX_ROWS: usize = 0x100;

const AVATAR_FALLBACK: &str = "data:image/svg+xml;utf8,<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"%23ccc\"><path d=\"M12 12c2.21 0 4-1.79 4-4s-1.79-4-4-4-4 1.79-4 4 1.79 4 4 4zm0 2c-2.67 0-8 1.34-8 4v2h16v-2c0-2.66-5.33-4-8-4z\"/></svg>";
/// `#btn-copy-share-link`'s markup, restored after `Preparing public link...` (5.18.0 keeps `innerHTML`).
const COPY_HTML: &str = "\n                        <svg viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\">\n                            <path d=\"M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71\"></path>\n                            <path d=\"M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71\"></path>\n                        </svg>\n                        Copy link\n                    ";
const ROW_STYLE: &str = "display: flex; align-items: center; gap: 12px;";
const AVATAR_STYLE: &str = "width: 36px; height: 36px; border-radius: 50%; object-fit: cover;";

/// One rendered row with controls.
struct Row {
    node: Node,
    select: Node,
    remove: Node,
    perm: String,
    role: &'static str,
}

/// Copy link: the steps after the click (5.18.0 awaits them in order).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum Copy {
    #[default]
    Idle,
    SetLink,
    List,
    Meta,
    Clipboard,
}

/// This module's state, stored in `App::st`.
#[derive(Default)]
pub struct State {
    open: bool,
    loading: bool,
    general: Node,
    copy_section: Node,
    container: Node,
    /// The document the modal works on (5.18.0 `state.driveFileId`) and its Drive facts.
    file: String,
    resource_key: String,
    file_name: String,
    owned_by_me: Option<bool>,
    owner: Perm,
    /// Refresh chain: owner info, then the permission list.
    owner_req: u32,
    list_req: u32,
    rows: Vec<Row>,
    /// Form values as the listeners saw them.
    email: String,
    invite_role: String,
    anyone: bool,
    link_role: String,
    invite_req: u32,
    access_req: u32,
    link_role_req: u32,
    row_req: u32,
    row_at: usize,
    row_next: &'static str,
    remove_req: u32,
    copy: Copy,
    copy_req: u32,
    copy_role: &'static str,
    copied: String,
}

fn role_label(role: &str) -> &'static str {
    match role {
        "owner" => "Owner",
        "writer" => "Editor",
        _ => "Viewer",
    }
}

fn norm_role(role: &str) -> &'static str { if role == "writer" { "writer" } else { "reader" } }

/// The open document's Drive facts (the session's `driveMeta`, or the public document).
fn seed(app: &mut App) {
    let s = session::get(app);
    let meta = s.map(|s| &s.record.drive_meta);
    let text = |k: &str| meta.and_then(|m| m.get(k).as_str()).unwrap_or("").to_string();
    let mut file = s.map(|s| s.doc_id.to_string()).unwrap_or_default();
    let name = s.map(|s| s.record.name.clone()).unwrap_or_default();
    let mut key = text("resourceKey");
    let mut owned = meta.and_then(|m| m.get("ownedByMe").as_bool());
    let owner = Perm { name: text("ownerName"), email: text("ownerEmail"), photo: text("ownerPhoto"), ..Perm::default() };
    if let Some((id, k)) = public::file(app) {
        if file.is_empty() || file == id {
            (file, key, owned) = (id.into(), k.into(), Some(false));
        }
    }
    let st = &mut app.st.panels_share;
    (st.file, st.file_name, st.resource_key, st.owned_by_me, st.owner) = (file, name, key, owned, owner);
}

fn set_loading(app: &mut App, host: &mut dyn Host, on: bool) {
    app.st.panels_share.loading = on;
    let out = host.out();
    out.class(Ui::SHARE_MODAL_OVERLAY, "loading", on);
    out.attr(Ui::SHARE_MODAL_LOADING, "aria-hidden", Some(if on { "false" } else { "true" }));
}

fn sections(app: &mut App, host: &mut dyn Host, owner: bool) {
    let st = &app.st.panels_share;
    let display = Some(if owner { "" } else { "none" });
    let out = host.out();
    out.style(Ui::SHARE_INVITE_FORM, "display", display);
    out.style(st.general, "display", display);
    out.style(st.copy_section, "display", display);
}

fn clear_rows(app: &mut App, host: &mut dyn Host) {
    let out = host.out();
    for (i, r) in app.st.panels_share.rows.drain(..).enumerate() {
        out.unlisten(L_ROW_ROLE + i as u32);
        out.unlisten(L_ROW_REMOVE + i as u32);
        out.release(r.select);
        out.release(r.remove);
        out.release(r.node);
    }
    out.html(Ui::SHARE_ACCESS_LIST, "");
}

fn set_access(app: &mut App, host: &mut dyn Host, anyone: bool, role: &str) {
    let st = &mut app.st.panels_share;
    st.anyone = anyone;
    let out = host.out();
    out.prop(Ui::SHARE_LINK_ACCESS_SELECT, "value", Prop::Str(if anyone { "anyone" } else { "restricted" }));
    if anyone {
        st.link_role = role.into();
        out.prop(Ui::SHARE_LINK_ROLE_SELECT, "value", Prop::Str(role));
    }
    out.class(Ui::SHARE_LINK_ROLE_SELECT, "hidden", !anyone);
}

// v5: refreshShareModalData
fn refresh(app: &mut App, host: &mut dyn Host) {
    if app.st.panels_share.file.is_empty() {
        return finish_loading(app, host);
    }
    // v5: resetShareModalState
    host.out().class(Ui::SHARE_OWNER_INFO, "hidden", true);
    clear_rows(app, host);
    sections(app, host, false);
    set_access(app, host, false, "");
    let (file, key) = (app.st.panels_share.file.clone(), app.st.panels_share.resource_key.clone());
    app.st.panels_share.list_req = 0;
    app.st.panels_share.owner_req = perms::owner_info(app, host, Mod::PanelsShare, &file, &key);
}

fn finish_loading(app: &mut App, host: &mut dyn Host) {
    if app.st.panels_share.loading {
        set_loading(app, host, false);
    }
}

fn owner_answer(app: &mut App, host: &mut dyn Host, io: &Io) {
    app.st.panels_share.owner_req = 0;
    // v5: fetchDriveOwnerInfo — a failure is only logged.
    if let Some(info) = (io.status >= 0).then(|| perms::parse_owner(Response::parse(io.data).text())).flatten() {
        let OwnerInfo { owned_by_me, resource_key, owner, .. } = info;
        let st = &mut app.st.panels_share;
        st.owned_by_me = owned_by_me.or(st.owned_by_me);
        if !resource_key.is_empty() {
            st.resource_key = resource_key;
        }
        if let Some(o) = owner {
            for (mine, theirs) in [(&mut st.owner.name, o.name), (&mut st.owner.email, o.email), (&mut st.owner.photo, o.photo)] {
                if !theirs.is_empty() {
                    *mine = theirs;
                }
            }
        }
    }
    let file = app.st.panels_share.file.clone();
    app.st.panels_share.list_req = perms::list(app, host, Mod::PanelsShare, &file);
}

fn list_answer(app: &mut App, host: &mut dyn Host, io: &Io) {
    app.st.panels_share.list_req = 0;
    let list = (io.status >= 0).then(|| perms::parse_list(Response::parse(io.data).text())).flatten();
    let Some(list) = list else {
        // v5: the permission load failed — sections by `driveOwnedByMe !== false`.
        let owner = app.st.panels_share.owned_by_me != Some(false);
        sections(app, host, owner);
        if !owner {
            owner_only_row(app, host, None);
        }
        return finish_loading(app, host);
    };
    let me = auth::profile(app).map(|p| (p.name.clone(), p.email.to_lowercase(), p.photo.clone())).unwrap_or_default();
    let st = &mut app.st.panels_share;
    let owner_perm = list.iter().find(|p| p.role == "owner").cloned();
    let mut is_owner = st.owned_by_me != Some(false);
    if !me.1.is_empty() {
        if let Some(mine) = list.iter().find(|p| p.email.to_lowercase() == me.1) {
            is_owner = mine.role == "owner";
            st.owned_by_me = Some(is_owner);
        }
    }
    if let Some(o) = &owner_perm {
        for (mine, theirs) in [(&mut st.owner.name, &o.name), (&mut st.owner.email, &o.email), (&mut st.owner.photo, &o.photo)] {
            if !theirs.is_empty() {
                *mine = theirs.clone();
            }
        }
    }
    sections(app, host, is_owner);
    if !is_owner {
        owner_only_row(app, host, owner_perm.as_ref());
        set_access(app, host, false, "");
        return finish_loading(app, host);
    }
    let mut visible: Vec<Perm> = list.iter().filter(|p| p.kind != "anyone").cloned().collect();
    if !visible.iter().any(|p| p.role == "owner") {
        let o = &app.st.panels_share.owner;
        let pick = |a: &str, b: &str, c: &'static str| if !a.is_empty() { a.to_string() } else if !b.is_empty() { b.to_string() } else { c.to_string() };
        visible.insert(0, Perm { id: String::new(), kind: "user".into(), role: "owner".into(), name: pick(&o.name, &me.0, "Owner"), email: pick(&o.email, &me.1, ""), photo: pick(&o.photo, &me.2, "") });
    }
    clear_rows(app, host);
    for (i, p) in visible.iter().take(MAX_ROWS).enumerate() {
        row(app, host, i, p);
    }
    host.out().class(Ui::SHARE_OWNER_INFO, "hidden", false);
    match list.iter().find(|p| p.kind == "anyone") {
        Some(a) => set_access(app, host, true, norm_role(&a.role)),
        None => set_access(app, host, false, ""),
    }
    finish_loading(app, host);
}

// v5: renderOwnerOnlyRow
fn owner_only_row(app: &mut App, host: &mut dyn Host, perm: Option<&Perm>) {
    clear_rows(app, host);
    let o = &app.st.panels_share.owner;
    let photo = perm.map(|p| &p.photo[..]).filter(|s| !s.is_empty()).or(Some(&o.photo[..]).filter(|s| !s.is_empty())).unwrap_or(AVATAR_FALLBACK).to_string();
    let name = [perm.map_or("", |p| &p.name[..]), &o.name, &o.email].into_iter().find(|s| !s.is_empty()).unwrap_or("Owner").to_string();
    let out = host.out();
    let row = out.insert_html(Ui::SHARE_ACCESS_LIST, Node::NONE, &format!(
        "<div class=\"share-person-row\" style=\"{ROW_STYLE}\"><img class=\"share-person-avatar\" src=\"\" style=\"{AVATAR_STYLE}\"><div class=\"share-person-primary\" style=\"font-weight: 500; font-size: 0.95rem; color: var(--text-primary);\">{}</div></div>",
        esc(&name)
    ));
    let img = out.child(row, &[0]);
    out.prop(img, "src", Prop::Str(&photo));
    out.release(img);
    out.release(row);
    out.class(Ui::SHARE_OWNER_INFO, "hidden", false);
}

fn row(app: &mut App, host: &mut dyn Host, i: usize, p: &Perm) {
    let st = &app.st.panels_share;
    let owner = p.role == "owner";
    let photo = [&p.photo[..], if owner { &st.owner.photo } else { "" }].into_iter().find(|s| !s.is_empty()).unwrap_or(AVATAR_FALLBACK).to_string();
    let name = [&p.name[..], if owner { &st.owner.name } else { "" }, &p.email].into_iter().find(|s| !s.is_empty()).unwrap_or("Unknown user").to_string();
    let email = if p.email.is_empty() && owner { st.owner.email.clone() } else { p.email.clone() };
    let mut html = format!(
        "<div class=\"share-person-row\" style=\"{ROW_STYLE}\"><img class=\"share-person-avatar\" src=\"\" style=\"{AVATAR_STYLE}\"><div class=\"share-person-details\" style=\"flex: 1 1 0%; display: flex; flex-direction: column; line-height: 1.3;\"><div class=\"share-person-primary\" style=\"font-weight: 500; font-size: 0.9rem; color: var(--text-primary);\">{}</div>",
        esc(&name)
    );
    if !email.is_empty() {
        html.push_str(&format!("<div class=\"share-person-secondary\" style=\"font-size: 0.8rem; color: var(--text-secondary);\">{}</div>", esc(&email)));
    }
    html.push_str("</div>");
    let controls = !owner && !p.id.is_empty();
    if controls {
        html.push_str(&format!(
            "<div class=\"share-person-actions\" style=\"display: flex; align-items: center; gap: 8px;\"><select class=\"share-person-role-select\" aria-label=\"Role for {}\" style=\"padding: 8px 28px 8px 10px; border: 1px solid rgba(0, 0, 0, 0.15); border-radius: 8px; background-color: var(--bg-primary); color: var(--text-primary); font-size: 0.85rem;\"><option value=\"reader\">Viewer</option><option value=\"writer\">Editor</option></select><button type=\"button\" class=\"btn btn-secondary share-person-remove\" style=\"padding: 6px 10px; font-size: 0.8rem;\">Remove</button></div>",
            esc(&name)
        ));
    } else {
        html.push_str(&format!("<div class=\"share-person-role\" style=\"font-size: 0.85rem; color: var(--text-secondary);\">{}</div>", role_label(&p.role)));
    }
    html.push_str("</div>");
    let out = host.out();
    let node = out.insert_html(Ui::SHARE_ACCESS_LIST, Node::NONE, &html);
    let img = out.child(node, &[0]);
    out.prop(img, "src", Prop::Str(&photo));
    out.release(img);
    let (mut select, mut remove) = (Node::NONE, Node::NONE);
    let role = norm_role(&p.role);
    if controls {
        (select, remove) = (out.child(node, &[2, 0]), out.child(node, &[2, 1]));
        out.prop(select, "value", Prop::Str(role));
        out.listen(select, "change", L_ROW_ROLE + i as u32, 0);
        out.listen(remove, "click", L_ROW_REMOVE + i as u32, 0);
    }
    app.st.panels_share.rows.push(Row { node, select, remove, perm: p.id.clone(), role });
}

fn row_busy(app: &mut App, host: &mut dyn Host, at: usize, on: bool) {
    let Some(r) = app.st.panels_share.rows.get(at) else { return };
    let out = host.out();
    out.prop(r.select, "disabled", Prop::Bool(on));
    out.prop(r.remove, "disabled", Prop::Bool(on));
}

fn ok_status(app: &mut App, host: &mut dyn Host, text: &str) { shell::status(app, host, text, Status::Info); }
fn err_status(app: &mut App, host: &mut dyn Host, text: &str) { shell::status(app, host, text, StatusOpts::from(Status::Error)); }

fn is_owner(app: &App) -> bool { app.st.panels_share.owned_by_me != Some(false) && !app.st.panels_share.file.is_empty() }

/// The page URL without query and hash (`location.origin + location.pathname`).
fn page_base(url: &str) -> &str { url.split(['?', '#']).next().unwrap_or(url) }

/// The §8.1 link Copy link builds (URLSearchParams order: drive, public, role, name, mode, resourcekey).
pub fn share_link(page_url: &str, file: &str, role: &str, name: &str, resource_key: &str) -> String {
    let mut pairs = vec![("drive", file), ("public", "1"), ("role", role), ("name", name)];
    if role == "writer" {
        pairs.push(("mode", "edit"));
    }
    if !resource_key.is_empty() {
        pairs.push(("resourcekey", resource_key));
    }
    format!("{}?{}", page_base(page_url), fx::form(&pairs))
}

// v5: the copy-link listener
fn copy_step(app: &mut App, host: &mut dyn Host, io: &Io) {
    let st = &mut app.st.panels_share;
    st.copy_req = 0;
    let file = st.file.clone();
    let fail = |app: &mut App, host: &mut dyn Host, message: &str| {
        app.st.panels_share.copy = Copy::Idle;
        let text = if message.is_empty() { "Could not create a public link." } else { message };
        err_status(app, host, text);
        copy_done(host);
    };
    if io.status < 0 {
        let message = String::from_utf8_lossy(io.data).into_owned();
        return fail(app, host, &message);
    }
    match st.copy {
        Copy::SetLink => {
            st.copy = Copy::List;
            app.st.panels_share.copy_req = perms::list(app, host, Mod::PanelsShare, &file);
        }
        Copy::List => {
            let list = perms::parse_list(Response::parse(io.data).text()).unwrap_or_default();
            let public = list.into_iter().find(|p| p.kind == "anyone" && ["reader", "commenter", "writer"].contains(&&p.role[..]));
            let Some(public) = public else { return fail(app, host, "Google Drive did not confirm public access. Your organisation may block public sharing.") };
            if app.st.panels_share.copy_role == "writer" && public.role != "writer" {
                return fail(app, host, "Google Drive did not confirm editor access for this link.");
            }
            app.st.panels_share.copy = Copy::Meta;
            app.st.panels_share.copy_req = perms::file_info(app, host, Mod::PanelsShare, &file);
        }
        Copy::Meta => {
            let j = crate::shell::json::Json::parse(Response::parse(io.data).text());
            let (name, key) = j.as_ref().map_or(("", ""), |j| (j.text("name"), j.text("resourceKey")));
            if !key.is_empty() {
                st.resource_key = key.into();
            }
            let name = if !name.is_empty() {
                name.to_string()
            } else if !st.file_name.is_empty() {
                st.file_name.clone()
            } else {
                "Shared document.pdf".into()
            };
            let link = share_link(&app.cfg.url, &file, app.st.panels_share.copy_role, &name, &app.st.panels_share.resource_key);
            let req = app.req(Mod::PanelsShare);
            let st = &mut app.st.panels_share;
            (st.copy, st.copy_req, st.copied) = (Copy::Clipboard, req, link);
            fx::clipboard_write(host.out(), req, &st.copied);
        }
        Copy::Clipboard => {
            let role = st.copy_role;
            st.copy = Copy::Idle;
            set_access(app, host, true, role);
            ok_status(app, host, if role == "writer" { "Editor link copied. Recipients sign in to save changes to Drive." } else { "Public viewer link copied — no sign-in required." });
            copy_done(host);
        }
        Copy::Idle => {}
    }
}

fn copy_done(host: &mut dyn Host) {
    let out = host.out();
    out.prop(Ui::BTN_COPY_SHARE_LINK, "disabled", Prop::Bool(false));
    out.html(Ui::BTN_COPY_SHARE_LINK, COPY_HTML);
}

fn close(app: &mut App, host: &mut dyn Host) {
    set_loading(app, host, false);
    app.st.panels_share.open = false;
    host.out().class(Ui::SHARE_MODAL_OVERLAY, "visible", false);
}

fn share_button(app: &mut App, host: &mut dyn Host) {
    seed(app);
    let shown = !app.st.panels_share.file.is_empty();
    host.out().class(Ui::BTN_DRIVE_SHARE, "hidden", !shown);
}

/// Called once at boot, in `app::MODULES` order, after the `Ui` table is bound.
pub fn init(app: &mut App, host: &mut dyn Host) {
    if app.cfg.has(crate::abi::mode::HEADLESS) {
        return;
    }
    let out = host.out();
    let st = &mut app.st.panels_share;
    st.general = out.query(Node::DOCUMENT, "#share-invite-form + .share-section");
    st.copy_section = out.query(Node::DOCUMENT, ".copy-link-section");
    st.container = out.query(Ui::SHARE_MODAL_OVERLAY, ".modal-container");
    (st.invite_role, st.link_role) = ("reader".into(), "reader".into());
    for (node, event, l) in [
        (Ui::BTN_DRIVE_SHARE, "click", L_OPEN),
        (Ui::BTN_CLOSE_SHARE_MODAL, "click", L_CLOSE),
        (Ui::SHARE_MODAL_OVERLAY, "click", L_OVERLAY),
        (Ui::SHARE_INVITE_FORM, "submit", L_SUBMIT),
        (Ui::SHARE_EMAIL_INPUT, "input", L_EMAIL),
        (Ui::SHARE_ROLE_SELECT, "change", L_INVITE_ROLE),
        (Ui::SHARE_LINK_ACCESS_SELECT, "change", L_ACCESS),
        (Ui::SHARE_LINK_ROLE_SELECT, "change", L_LINK_ROLE),
        (Ui::BTN_COPY_SHARE_LINK, "click", L_COPY),
    ] {
        out.listen(node, event, l, 0);
    }
}

fn dom(app: &mut App, host: &mut dyn Host, d: &crate::DomEvent) -> u32 {
    match d.listener {
        L_OPEN => {
            seed(app);
            if app.st.panels_share.file.is_empty() {
                return 0;
            }
            set_loading(app, host, true);
            app.st.panels_share.open = true;
            host.out().class(Ui::SHARE_MODAL_OVERLAY, "visible", true);
            refresh(app, host);
        }
        L_CLOSE => close(app, host),
        L_OVERLAY if d.flags & ev::EXACT != 0 && d.path.first() == Ui::SHARE_MODAL_OVERLAY => close(app, host),
        L_EMAIL => app.st.panels_share.email = d.value.into(),
        L_INVITE_ROLE => app.st.panels_share.invite_role = d.value.into(),
        L_SUBMIT => {
            let st = &app.st.panels_share;
            let email = st.email.trim().to_string();
            if st.owned_by_me != Some(false) && !email.is_empty() && !st.file.is_empty() && st.invite_req == 0 {
                let (file, role) = (st.file.clone(), st.invite_role.clone());
                let out = host.out();
                out.prop(Ui::BTN_SEND_SHARE, "disabled", Prop::Bool(true));
                out.text(Ui::BTN_SEND_SHARE, "Sending...");
                app.st.panels_share.invite_req = perms::add(app, host, Mod::PanelsShare, &file, &email, &role);
            }
            return ret::PREVENT_DEFAULT;
        }
        L_ACCESS => {
            if !is_owner(app) {
                return 0;
            }
            let anyone = d.value == "anyone";
            let st = &mut app.st.panels_share;
            st.anyone = anyone;
            host.out().class(Ui::SHARE_LINK_ROLE_SELECT, "hidden", !anyone);
            let role = if anyone { st.link_role.clone() } else { "restricted".into() };
            let file = st.file.clone();
            app.st.panels_share.access_req = perms::set_link(app, host, Mod::PanelsShare, &file, &role);
        }
        L_LINK_ROLE => {
            app.st.panels_share.link_role = d.value.into();
            if !is_owner(app) || !app.st.panels_share.anyone {
                return 0;
            }
            let file = app.st.panels_share.file.clone();
            app.st.panels_share.link_role_req = perms::set_link(app, host, Mod::PanelsShare, &file, d.value);
        }
        L_COPY => {
            if !is_owner(app) || app.st.panels_share.copy != Copy::Idle {
                return 0;
            }
            let out = host.out();
            out.prop(Ui::BTN_COPY_SHARE_LINK, "disabled", Prop::Bool(true));
            out.text(Ui::BTN_COPY_SHARE_LINK, "Preparing public link...");
            // v5: the role chosen in the dialog, never downgraded.
            let role = norm_role(&app.st.panels_share.link_role);
            let file = app.st.panels_share.file.clone();
            let req = perms::set_link(app, host, Mod::PanelsShare, &file, role);
            let st = &mut app.st.panels_share;
            (st.copy, st.copy_req, st.copy_role) = (Copy::SetLink, req, role);
        }
        l if (L_ROW_ROLE..L_ROW_ROLE + MAX_ROWS as u32).contains(&l) => {
            let at = (l - L_ROW_ROLE) as usize;
            let Some(r) = app.st.panels_share.rows.get(at) else { return 0 };
            let next = norm_role(d.value);
            if next == r.role || app.st.panels_share.row_req != 0 {
                return 0;
            }
            let (file, perm) = (app.st.panels_share.file.clone(), r.perm.clone());
            row_busy(app, host, at, true);
            let req = perms::update_role(app, host, Mod::PanelsShare, &file, &perm, next);
            let st = &mut app.st.panels_share;
            (st.row_req, st.row_at, st.row_next) = (req, at, next);
        }
        l if (L_ROW_REMOVE..L_ROW_REMOVE + MAX_ROWS as u32).contains(&l) => {
            let at = (l - L_ROW_REMOVE) as usize;
            let Some(r) = app.st.panels_share.rows.get(at) else { return 0 };
            if app.st.panels_share.remove_req != 0 {
                return 0;
            }
            let (file, perm) = (app.st.panels_share.file.clone(), r.perm.clone());
            row_busy(app, host, at, true);
            let req = perms::revoke(app, host, Mod::PanelsShare, &file, &perm);
            let st = &mut app.st.panels_share;
            (st.remove_req, st.row_at) = (req, at);
        }
        _ => {}
    }
    0
}

fn io(app: &mut App, host: &mut dyn Host, io: &Io) {
    let st = &mut app.st.panels_share;
    let ok = io.status >= 0;
    if io.req == st.owner_req {
        owner_answer(app, host, io);
    } else if io.req == st.list_req {
        list_answer(app, host, io);
    } else if io.req == st.copy_req && st.copy != Copy::Idle {
        copy_step(app, host, io);
    } else if io.req == st.invite_req {
        st.invite_req = 0;
        let out = host.out();
        out.prop(Ui::BTN_SEND_SHARE, "disabled", Prop::Bool(false));
        out.text(Ui::BTN_SEND_SHARE, "Send");
        if ok {
            ok_status(app, host, "Invitation sent");
            app.st.panels_share.email.clear();
            host.out().prop(Ui::SHARE_EMAIL_INPUT, "value", Prop::Str(""));
            refresh(app, host);
        } else {
            err_status(app, host, "Failed to share");
        }
    } else if io.req == st.access_req {
        st.access_req = 0;
        if ok {
            ok_status(app, host, "Link access updated");
            refresh(app, host);
        } else {
            err_status(app, host, "Failed to update access");
        }
    } else if io.req == st.link_role_req {
        st.link_role_req = 0;
        if ok {
            ok_status(app, host, "Link role updated");
            refresh(app, host);
        } else {
            err_status(app, host, "Failed to update role");
        }
    } else if io.req == st.row_req {
        st.row_req = 0;
        let at = st.row_at;
        row_busy(app, host, at, false);
        if ok {
            if let Some(r) = app.st.panels_share.rows.get_mut(at) {
                r.role = app.st.panels_share.row_next;
            }
            ok_status(app, host, "Access updated");
            refresh(app, host);
        } else {
            if let Some(r) = app.st.panels_share.rows.get(at) {
                host.out().prop(r.select, "value", Prop::Str(r.role));
            }
            err_status(app, host, "Failed to update access");
        }
    } else if io.req == st.remove_req {
        st.remove_req = 0;
        let at = st.row_at;
        row_busy(app, host, at, false);
        if ok {
            ok_status(app, host, "Access revoked");
            refresh(app, host);
        } else {
            err_status(app, host, "Failed to revoke access");
        }
    }
}

/// Events addressed to this module (ids made with `abi::tag`) and broadcasts (`Life`, `Msg`). Returns `abi::ret` bits.
pub fn event(app: &mut App, host: &mut dyn Host, event: &Event) -> u32 {
    match *event {
        Event::Dom(ref d) => return dom(app, host, d),
        Event::Io(ref i) => io(app, host, i),
        Event::Msg(m) if m.chan == session::sig::OPENED || m.chan == session::sig::CLOSED || m.chan == shell::sig::VIEW => share_button(app, host),
        Event::Msg(m) if m.chan == shell::sig::SIGNED_OUT && app.st.panels_share.open => close(app, host),
        _ => {}
    }
    0
}

/// `__IHN_TEST_API__.cmd('panels.share.state')` → `{open, loading, file, ownedByMe, rows: [[permId, role]…],
/// anyone, linkRole, copied}`.
pub fn test_cmd(app: &mut App, _host: &mut dyn Host, name: &str, _args: &str) -> Option<String> {
    (name == "state").then(|| {
        let st = &app.st.panels_share;
        let rows: Vec<String> = st.rows.iter().map(|r| format!("[{},{}]", json_str(&r.perm), json_str(r.role))).collect();
        format!(
            "{{\"open\":{},\"loading\":{},\"file\":{},\"ownedByMe\":{},\"rows\":[{}],\"anyone\":{},\"linkRole\":{},\"copied\":{}}}",
            st.open,
            st.loading,
            json_str(&st.file),
            st.owned_by_me.map_or("null".into(), |b| b.to_string()),
            rows.join(","),
            st.anyone,
            json_str(&st.link_role),
            json_str(&st.copied)
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copied_links_have_5_18_0_format() {
        assert_eq!(share_link("https://inhousenotes.com/?drive=x&e2e=1#a", "abc", "reader", "My notes.pdf", ""), "https://inhousenotes.com/?drive=abc&public=1&role=reader&name=My+notes.pdf");
        assert_eq!(share_link("http://127.0.0.1:4400/index.html", "abc", "writer", "a&b", "0-k"), "http://127.0.0.1:4400/index.html?drive=abc&public=1&role=writer&name=a%26b&mode=edit&resourcekey=0-k");
        let link = crate::doc::links::parse(&share_link("https://x.test/", "id1", "writer", "N", "rk")).unwrap();
        assert_eq!((link.id.as_str(), link.writer, link.resource_key.as_str(), link.name.as_str()), ("id1", true, "rk", "N"));
    }
}
