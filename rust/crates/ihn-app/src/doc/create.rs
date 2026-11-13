//! New documents: an id reserved from the pool at creation (ARCHITECTURE §7.1), a session that is writable at
//! once and a Drive file that does not exist until the first export (`flags.created = false`). Owned by W3.1.
// v5: app-v5.js:createNewDriveDocumentWithFolder

use super::session;
use crate::app::Mod;
use crate::drive::client::{self, Req};
use crate::shell::json::Json;
use crate::shell::{Status, StatusOpts, View};
use crate::store::schema::kv;
use crate::store::sv::Value;
use crate::store::{self, Reply, Tx as StoreTx};
use crate::{auth, shell, App, Event, Host};
use ihn_core::op::{Background, Template};

/// 5.18.0 `A4_WIDTH` × `A4_HEIGHT`.
pub const A4: (u32, u32) = (794, 1123);
/// Ids asked for at once, and the level below which the pool is refilled (SYNC §8.2).
pub const POOL_SIZE: usize = 20;
pub const POOL_LOW: usize = 10;

#[derive(Default)]
struct Pending {
    folder: String,
    name: String,
    account: String,
    id: String,
}

/// This module's state, stored in `App::st`.
#[derive(Default)]
pub struct State {
    pending: Option<Pending>,
    pool_req: u32,
    ids_req: u32,
    load_req: u32,
    /// The pool was read for a creation (else: a refill check).
    creating: bool,
    /// The pool as last read or written.
    pool: Vec<String>,
}

// v5: app-v5.js:normalizeDocTitleInput / sanitizeFilename
pub fn normalize_title(value: &str) -> String {
    let cleaned = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if cleaned.is_empty() {
        return "Untitled".into();
    }
    let lower = cleaned.to_ascii_lowercase();
    let no_ext = if lower.ends_with(".pdf") { cleaned[..cleaned.len() - 4].trim() } else { &cleaned[..] };
    let mut out = String::new();
    let mut run = false;
    for c in no_ext.chars() {
        let bad = "\\/:*?\"<>|".contains(c);
        if !bad {
            out.push(c);
        } else if !run {
            out.push('_');
        }
        run = bad;
    }
    let out = out.trim();
    if out.is_empty() { "cuaderno".into() } else { out.into() }
}

/// Creates a document in `folder` ("" = My Drive) and opens it in the editor. False when another creation runs
/// or nobody is signed in.
pub fn new_document(app: &mut App, host: &mut dyn Host, folder: &str, base_name: &str) -> bool {
    let account = auth::known_account(host);
    if app.st.doc_create.pending.is_some() || account.is_empty() {
        return false;
    }
    app.st.doc_create.pending = Some(Pending { folder: folder.into(), name: normalize_title(base_name), account: account.clone(), id: String::new() });
    shell::show_view(app, host, View::Editor);
    shell::loading(app, host, "Creating PDF...");
    read_pool(app, host, &account, true);
    true
}

/// Refills the id pool of the signed-in account when it is low (called when a token appears and after a creation).
pub fn refill(app: &mut App, host: &mut dyn Host) {
    let account = auth::known_account(host);
    if !account.is_empty() && app.st.doc_create.pool_req == 0 && app.st.doc_create.pending.is_none() {
        read_pool(app, host, &account, false);
    }
}

fn read_pool(app: &mut App, host: &mut dyn Host, account: &str, creating: bool) {
    let mut t = StoreTx::new();
    t.kv_get(account, kv::IDPOOL);
    let req = app.req(Mod::DocCreate);
    app.st.doc_create.pool_req = req;
    app.st.doc_create.creating = creating;
    store::tx(app, host, req, t);
}

fn write_pool(app: &mut App, host: &mut dyn Host, account: &str, ids: &[String]) {
    let mut t = StoreTx::new();
    t.kv_put(account, kv::IDPOOL, &Value::from(ids.iter().map(|id| Value::from(&id[..])).collect::<Vec<_>>()));
    let req = app.req(Mod::DocCreate);
    store::tx(app, host, req, t);
}

fn fail(app: &mut App, host: &mut dyn Host) {
    app.st.doc_create.pending = None;
    // v5: the catch of createNewDriveDocumentWithFolder (BEHAVIOUR-CHANGES N-12: an empty pool while offline).
    shell::status(app, host, "Failed to create PDF", StatusOpts::from(Status::Info).preserve());
    shell::hide_loading(app, host);
    shell::show_view(app, host, View::Home);
}

fn pool_read(app: &mut App, host: &mut dyn Host, mut ids: Vec<String>) {
    let creating = app.st.doc_create.creating;
    let account = auth::known_account(host);
    if ids.len() < POOL_LOW && !shell::offline(app) && auth::token(app).is_some() && app.st.doc_create.ids_req == 0 {
        let url = format!("https://www.googleapis.com/drive/v3/files/generateIds?count={POOL_SIZE}&space=drive&type=files");
        app.st.doc_create.ids_req = client::request(app, host, Mod::DocCreate, Req::get(&url));
    }
    if !creating {
        app.st.doc_create.pool = ids;
        return;
    }
    if ids.is_empty() && app.st.doc_create.ids_req != 0 {
        return; // the creation continues with the answer
    }
    let Some(p) = app.st.doc_create.pending.as_mut() else { return };
    if ids.is_empty() {
        return fail(app, host);
    }
    p.id = ids.remove(0);
    let (id, owner) = (p.id.clone(), p.account.clone());
    write_pool(app, host, &account, &ids);
    app.st.doc_create.pool = ids;
    let req = app.req(Mod::DocCreate);
    app.st.doc_create.load_req = req;
    session::begin_new(app, host, &owner, &id, req);
}

fn session_ready(app: &mut App, host: &mut dyn Host) {
    let Some(p) = app.st.doc_create.pending.take() else { return };
    let profile = auth::profile(app).cloned();
    if let Some(s) = session::get_mut(app).filter(|s| s.doc_id == p.id) {
        s.can_edit = true;
        s.record.name = format!("{}.pdf", p.name);
        s.record.flags.created = false;
        s.record.last_opened_at = host.time();
        s.record.drive_meta = Value::obj([
            ("parents", Value::from(if p.folder.is_empty() { Vec::new() } else { vec![Value::from(&p.folder[..])] })),
            ("canEdit", Value::from(true)),
            ("ownedByMe", Value::from(true)),
            ("ownerName", Value::from(profile.as_ref().map_or("", |p| &p.name[..]))),
            ("ownerEmail", Value::from(profile.as_ref().map_or("", |p| &p.email[..]))),
            ("ownerPhoto", Value::from(profile.as_ref().map_or("", |p| &p.photo[..]))),
        ]);
    }
    let made = session::edit(app, host, |tx| {
        tx.page_new(None, Template::Default, A4, Background::Template);
        tx.set_name(&p.name);
    });
    if made.is_none() {
        return fail(app, host);
    }
    shell::signal(app, host, session::sig::OPENED, &p.id);
    shell::status(app, host, "Created on this device", StatusOpts::from(Status::Info).preserve());
    shell::hide_loading(app, host);
}

/// Called once at boot, in `app::MODULES` order, after the `Ui` table is bound.
pub fn init(_app: &mut App, _host: &mut dyn Host) {}

/// Events addressed to this module (ids made with `abi::tag`) and broadcasts (`Life`, `Msg`). Returns `abi::ret` bits.
pub fn event(app: &mut App, host: &mut dyn Host, event: &Event) -> u32 {
    match *event {
        Event::Io(io) if io.req == app.st.doc_create.pool_req => {
            app.st.doc_create.pool_req = 0;
            let mut reply = Reply::new(&io);
            let ids = reply.value().items().iter().filter_map(|v| v.as_str().map(String::from)).collect();
            pool_read(app, host, ids);
        }
        Event::Io(io) if io.req == app.st.doc_create.ids_req => {
            app.st.doc_create.ids_req = 0;
            let body = if io.status >= 0 { crate::shell::fx::Response::parse(io.data).text() } else { "" };
            let fresh: Vec<String> = Json::parse(body).as_ref().and_then(|j| j.get("ids")).map(|ids| ids.items().iter().filter_map(|i| i.as_str().map(String::from)).collect()).unwrap_or_default();
            let account = auth::known_account(host);
            if fresh.is_empty() || account.is_empty() {
                if app.st.doc_create.pending.as_ref().is_some_and(|p| p.id.is_empty()) && app.st.doc_create.load_req == 0 {
                    fail(app, host);
                }
            } else if app.st.doc_create.pending.as_ref().is_some_and(|p| p.id.is_empty()) {
                // The pool was empty: the creation was waiting for these ids.
                app.st.doc_create.creating = true;
                pool_read(app, host, fresh);
            } else {
                // A top-up: the fresh ids go behind what the pool still holds.
                let mut ids = std::mem::take(&mut app.st.doc_create.pool);
                ids.extend(fresh);
                write_pool(app, host, &account, &ids);
                app.st.doc_create.pool = ids;
            }
        }
        Event::Io(io) if io.req == app.st.doc_create.load_req => {
            app.st.doc_create.load_req = 0;
            session_ready(app, host);
        }
        Event::Msg(m) if m.chan == shell::sig::ACCOUNT => refill(app, host),
        _ => {}
    }
    0
}

/// `__IHN_TEST_API__.cmd('doc.create.new', {folder, name})` → whether the creation started.
pub fn test_cmd(app: &mut App, host: &mut dyn Host, name: &str, args: &str) -> Option<String> {
    (name == "new").then(|| {
        let json = Json::parse(args);
        let (folder, title) = json.as_ref().map_or(("", ""), |j| (j.text("folder"), j.text("name")));
        new_document(app, host, folder, title).to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_are_normalised_like_5_18_0() {
        assert_eq!(normalize_title(""), "Untitled");
        assert_eq!(normalize_title("  My   notes .PDF "), "My notes");
        assert_eq!(normalize_title("a/b\\\\c:d"), "a_b_c_d");
        assert_eq!(normalize_title("???"), "_");
        assert_eq!(normalize_title(".pdf"), "cuaderno");
    }
}
