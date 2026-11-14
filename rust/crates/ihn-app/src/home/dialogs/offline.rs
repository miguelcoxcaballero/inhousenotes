//! The Offline documents dialog (`dialog#offline-settings`), the upload-error dialog (`dialog#offline-sync-error`)
//! (spec/home.md §6.7, ARCHITECTURE §6.6) and the Download-app modal (§6.6).
// v5: offline-library-v5.js:showSettings / showSyncFailure

use super::{id, on_overlay};
use crate::abi::{ety, DomEvent, Io};
use crate::dom::{Dialog, Prop};
use crate::home::badges::{self, SyncError};
use crate::home::cards::esc;
use crate::shell::{self, fx};
use crate::store::sv::Value;
use crate::store::{self, schema, DocRecord, Reply, Tx};
use crate::{App, Host, Mod, Node, Ui};

const L_SETTINGS: u32 = id(80); // ..=81
const L_SETTINGS_CLICK: u32 = id(82);
const L_SETTINGS_INPUT: u32 = id(83);
const L_SETTINGS_CANCEL: u32 = id(84);
const L_SETTINGS_CLOSE: u32 = id(85);
const L_ERROR_CLICK: u32 = id(86);
const L_ERROR_CANCEL: u32 = id(87);
const L_ERROR_CLOSE: u32 = id(88);
const L_APP_OPEN: u32 = id(90);
const L_APP_CLOSE: u32 = id(91);
const L_APP_OVERLAY: u32 = id(92);
const L_APP_LINK: u32 = id(93);
const T_APP_CLOSE: u32 = id(90);

/// One 5.x recovery (`kv.recoveries`, written by the migration): `{name, createdAt, localBlob, remoteBlob}`.
struct Recovery {
    buttons: Vec<(Node, u32, String)>,
}

struct Settings {
    dialog: Node,
    save: Node,
    days: String,
    /// The last form button clicked: `save` closes the dialog only when the form is valid.
    returning: &'static str,
    recoveries: Vec<Recovery>,
    blobs: Vec<u32>,
}

struct ErrorDialog {
    dialog: Node,
    copy: Node,
    retry: Node,
    returning: bool,
    id: String,
}

enum Job {
    Load,
    Save,
    Prune,
    Copied,
}

#[derive(Default)]
pub struct State {
    settings: Option<Settings>,
    error: Option<ErrorDialog>,
    jobs: Vec<(u32, Job)>,
    loading: bool,
}

impl State {
    pub fn settings_open(&self) -> bool { self.settings.is_some() }
    pub fn error_open(&self) -> bool { self.error.is_some() }
}

pub fn init(_app: &mut App, host: &mut dyn Host) {
    let out = host.out();
    for (i, menu) in ["#drive-profile-menu", "#drive-profile-menu-editor"].iter().enumerate() {
        let button = out.query(Node::DOCUMENT, &format!("{menu} [data-offline-settings]"));
        out.listen(button, "click", L_SETTINGS + i as u32, 0);
    }
    out.listen(Ui::BTN_DOWNLOAD_APP, "click", L_APP_OPEN, 0);
    out.listen(Ui::DOWNLOAD_APP_CLOSE, "click", L_APP_CLOSE, 0);
    out.listen(Ui::MODAL_DOWNLOAD_APP, "click", L_APP_OVERLAY, 0);
    out.listen(Ui::DOWNLOAD_APP_LINK, "click", L_APP_LINK, 0);
}

fn st(app: &mut App) -> &mut State { &mut app.st.home_dialogs.offline }

fn tx(app: &mut App, host: &mut dyn Host, t: Tx, job: Job) {
    let req = app.req(Mod::HomeDialogs);
    store::tx(app, host, req, t);
    st(app).jobs.push((req, job));
}

// ---- Offline documents ------------------------------------------------------------------------------------------

fn show_settings(app: &mut App, host: &mut dyn Host) {
    let account = app.st.home.account.clone();
    if account.is_empty() || app.st.home_dialogs.offline.loading {
        return;
    }
    st(app).loading = true;
    let mut t = Tx::new();
    t.kv_get(&account, schema::kv::PREFERENCES);
    t.kv_get(&account, schema::kv::RECOVERIES);
    tx(app, host, t, Job::Load);
}

fn remove_settings(app: &mut App, host: &mut dyn Host) {
    let Some(s) = st(app).settings.take() else { return };
    let out = host.out();
    for l in [L_SETTINGS_CLICK, L_SETTINGS_INPUT, L_SETTINGS_CANCEL, L_SETTINGS_CLOSE] {
        out.unlisten(l);
    }
    s.recoveries.iter().flat_map(|r| r.buttons.iter()).for_each(|b| out.release(b.0));
    s.blobs.iter().for_each(|b| out.obj_release(*b));
    out.remove(s.dialog);
    out.release(s.save);
    out.release(s.dialog);
}

fn settings_loaded(app: &mut App, host: &mut dyn Host, io: &Io) {
    st(app).loading = false;
    let mut reply = Reply::new(io);
    let (prefs, recoveries) = if reply.ok() { (reply.value(), reply.value()) } else { (Value::Undef, Value::Undef) };
    let mut blobs = Vec::new();
    recoveries.handles(&mut blobs);
    remove_settings(app, host);
    let days = schema::retention_days(&prefs);
    let pending = app.st.home.catalog.pending_count();
    let queue = if pending > 0 { format!("{pending} pending · {}", if shell::offline(app) { "Waiting for connection" } else { "Will sync automatically" }) } else { "Up to date".into() };
    let mut list: Vec<&Value> = recoveries.items().iter().collect();
    list.sort_by(|a, b| b.get("createdAt").as_f64().unwrap_or(0.0).partial_cmp(&a.get("createdAt").as_f64().unwrap_or(0.0)).unwrap_or(std::cmp::Ordering::Equal));
    let mut rows = String::new();
    let mut plan = Vec::new();
    for r in &list {
        let name = r.get("name").as_str().filter(|n| !n.is_empty()).unwrap_or("Document").to_string();
        let mut buttons = String::new();
        let mut kept = Vec::new();
        for (label, key) in [("This device", "localBlob"), ("Other device", "remoteBlob")] {
            if let Value::Blob { obj, .. } = r.get(key) {
                buttons.push_str(&format!("<button type=\"button\">{label}</button>"));
                kept.push((*obj, format!("{name} ({label}).pdf")));
            }
        }
        rows.push_str(&format!("<p>{} · versions before merge {buttons}</p>", esc(&name)));
        plan.push(kept);
    }
    let html = format!(
        "<dialog id=\"offline-settings\" class=\"offline-settings\"><form method=\"dialog\"><h2>Offline documents</h2><label>Keep recently opened documents for <input name=\"days\" type=\"number\" min=\"1\" max=\"3650\" required> days</label><p>Pinned documents and unsynced changes are kept until you remove them.</p><p class=\"offline-queue-status\" role=\"status\">{}</p><div class=\"offline-recoveries\">{rows}</div><div class=\"offline-settings-actions\"><button value=\"cancel\" formnovalidate>Cancel</button><button value=\"save\">Save</button></div></form></dialog>",
        esc(&queue)
    );
    let days = ihn_core::jsfmt::num(days);
    let out = host.out();
    let dialog = out.insert_html(Node::BODY, Node::NONE, &html);
    let input = out.child(dialog, &[0, 1, 0]);
    out.prop(input, "value", Prop::Str(&days));
    out.listen(input, "input", L_SETTINGS_INPUT, 0);
    out.release(input);
    let save = out.child(dialog, &[0, 5, 1]);
    let recoveries = plan
        .into_iter()
        .enumerate()
        .map(|(i, kept)| Recovery { buttons: kept.into_iter().enumerate().map(|(j, (obj, name))| (out.child(dialog, &[0, 4, i as u8, j as u8]), obj, name)).collect() })
        .collect();
    out.listen(dialog, "click", L_SETTINGS_CLICK, 0);
    out.listen(dialog, "cancel", L_SETTINGS_CANCEL, 0);
    out.listen(dialog, "close", L_SETTINGS_CLOSE, 0);
    out.dialog(dialog, Dialog::ShowModal);
    st(app).settings = Some(Settings { dialog, save, days, returning: "", recoveries, blobs });
}

/// Save: the retention, then 5.18.0's prune (`S.prune(account, now, [open document])`), then Home again.
fn save_settings(app: &mut App, host: &mut dyn Host, days: f64) {
    let account = app.st.home.account.clone();
    let mut t = Tx::new();
    t.kv_put(&account, schema::kv::PREFERENCES, &Value::obj([("retentionDays", Value::Num(days))]));
    t.docs_scan();
    tx(app, host, t, Job::Save);
}

fn prune(app: &mut App, host: &mut dyn Host, io: &Io) {
    let mut reply = Reply::new(io);
    if !reply.ok() {
        return;
    }
    reply.skip();
    let account = app.st.home.account.clone();
    let open = crate::doc::session::doc_id(app).to_string();
    let rows = reply.rows();
    let days = app.st.home_dialogs.offline.settings.as_ref().map_or(schema::RETENTION_DAYS, |s| s.days.parse().unwrap_or(schema::RETENTION_DAYS));
    let now = host.time();
    let mut t = Tx::new();
    let mut handles = Vec::new();
    for (key, value) in &rows {
        value.handles(&mut handles);
        let id = key.text();
        let record = DocRecord::from_value(value);
        if record.account == account && record.prunable(id == open, now, days) {
            t.doc_delete(&id);
        }
    }
    handles.into_iter().for_each(|h| host.out().obj_release(h));
    tx(app, host, t, Job::Prune);
}

fn settings_click(app: &mut App, host: &mut dyn Host, e: &DomEvent) {
    let Some(s) = app.st.home_dialogs.offline.settings.as_ref() else { return };
    if let Some((_, obj, name)) = s.recoveries.iter().flat_map(|r| r.buttons.iter()).find(|b| e.path.contains(b.0)) {
        // v5: host.downloadBlob(blob, `${name} (${label}).pdf`)
        fx::download(host.out(), *obj, name, 0.0, 1200.0);
        return;
    }
    let returning = if e.path.contains(s.save) { "save" } else { "" };
    if let Some(s) = st(app).settings.as_mut() {
        s.returning = returning;
    }
}

fn settings_closed(app: &mut App, host: &mut dyn Host) {
    let Some(s) = app.st.home_dialogs.offline.settings.as_ref() else { return };
    let days: f64 = s.days.trim().parse().unwrap_or(f64::NAN);
    if s.returning == "save" && days.is_finite() {
        save_settings(app, host, days);
        return; // removed once the prune ran
    }
    remove_settings(app, host);
}

// ---- upload error -----------------------------------------------------------------------------------------------

/// `#offline-sync-error` for document `id` (Home's badge sent `home::sig::SYNC_ERROR`).
pub fn show_error(app: &mut App, host: &mut dyn Host, id: &str) {
    let home = &app.st.home;
    let error: SyncError = home.sync.errors.iter().find(|e| e.id.is_empty() || e.id == id).cloned().unwrap_or_default();
    let name = home.catalog.entry(id).map(|e| e.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| "Document".into());
    remove_error(app, host);
    let report = report(app, &error);
    let html = format!(
        "<dialog id=\"offline-sync-error\" class=\"offline-settings\"><form method=\"dialog\"><h2>Upload pending</h2><p class=\"sync-file-name\">{}</p><p class=\"sync-error-summary\">{}</p><pre class=\"sync-error-detail\">{}</pre><div class=\"offline-settings-actions\"><button value=\"close\">Close</button><button type=\"button\" class=\"copy-sync-error\">Copy error</button><button value=\"retry\">Retry</button></div></form></dialog>",
        esc(&name),
        esc(&badges::error_text(&error)),
        esc(&report)
    );
    let out = host.out();
    let dialog = out.insert_html(Node::BODY, Node::NONE, &html);
    let copy = out.child(dialog, &[0, 4, 1]);
    let retry = out.child(dialog, &[0, 4, 2]);
    out.listen(dialog, "click", L_ERROR_CLICK, 0);
    out.listen(dialog, "cancel", L_ERROR_CANCEL, 0);
    out.listen(dialog, "close", L_ERROR_CLOSE, 0);
    out.dialog(dialog, Dialog::ShowModal);
    st(app).error = Some(ErrorDialog { dialog, copy, retry, returning: false, id: id.into() });
}

/// `Inhouse Notes <version>\n<phase> · <code>\n<detail>` with the token redacted and the detail cut at 1200.
fn report(app: &App, error: &SyncError) -> String {
    let message = if error.message.is_empty() { "Unknown error" } else { &error.message };
    let detail = String::from_utf16_lossy(&redact(message).encode_utf16().take(1200).collect::<Vec<_>>());
    let or = |s: &str, d: &'static str| if s.is_empty() { d.to_string() } else { s.to_string() };
    format!("Inhouse Notes {}\n{} · {}\n{detail}", app.cfg.version, or(&error.phase, "sync"), or(&error.code, "unknown"))
}

/// `/Bearer\s+[^\s"']+/gi` → `Bearer [redacted]`.
fn redact(text: &str) -> String {
    let lower = text.to_ascii_lowercase();
    let (mut out, mut at) = (String::new(), 0);
    while let Some(i) = lower[at..].find("bearer").map(|i| i + at) {
        let rest = &text[i + 6..];
        let ws = rest.len() - rest.trim_start().len();
        let token = rest[ws..].find(|c: char| c.is_whitespace() || c == '"' || c == '\'').unwrap_or(rest.len() - ws);
        if ws == 0 || token == 0 {
            out.push_str(&text[at..i + 6]);
            at = i + 6;
            continue;
        }
        out.push_str(&text[at..i]);
        out.push_str("Bearer [redacted]");
        at = i + 6 + ws + token;
    }
    out.push_str(&text[at..]);
    out
}

fn remove_error(app: &mut App, host: &mut dyn Host) {
    let Some(d) = st(app).error.take() else { return };
    let out = host.out();
    for l in [L_ERROR_CLICK, L_ERROR_CANCEL, L_ERROR_CLOSE] {
        out.unlisten(l);
    }
    out.remove(d.dialog);
    [d.copy, d.retry, d.dialog].into_iter().for_each(|n| out.release(n));
}

fn error_click(app: &mut App, host: &mut dyn Host, e: &DomEvent) {
    let Some(d) = app.st.home_dialogs.offline.error.as_ref() else { return };
    if e.path.contains(d.copy) {
        let id = d.id.clone();
        let error: SyncError = app.st.home.sync.errors.iter().find(|x| x.id.is_empty() || x.id == id).cloned().unwrap_or_default();
        let report = report(app, &error);
        let req = app.req(Mod::HomeDialogs);
        fx::clipboard_write(host.out(), req, &report);
        st(app).jobs.push((req, Job::Copied));
        return;
    }
    let retry = e.path.contains(d.retry);
    if let Some(d) = st(app).error.as_mut() {
        d.returning = retry;
    }
}

fn error_closed(app: &mut App, host: &mut dyn Host) {
    let Some(d) = app.st.home_dialogs.offline.error.as_ref() else { return };
    let (retry, id) = (d.returning, d.id.clone());
    remove_error(app, host);
    // v5: `if(!offline())host.retrySync()` (offline: after a successful probe).
    if retry && !shell::offline(app) {
        shell::signal(app, host, crate::home::sig::RETRY_SYNC, &id);
    }
}

// ---- events -----------------------------------------------------------------------------------------------------

pub fn dom(app: &mut App, host: &mut dyn Host, e: &DomEvent) -> u32 {
    match e.listener {
        l if l == L_SETTINGS || l == L_SETTINGS + 1 => show_settings(app, host),
        L_SETTINGS_CLICK => settings_click(app, host, e),
        L_SETTINGS_INPUT => {
            if let Some(s) = st(app).settings.as_mut() {
                s.days = e.value.into();
            }
        }
        L_SETTINGS_CANCEL | L_ERROR_CANCEL if e.ty == ety::CANCEL => {
            if let Some(s) = st(app).settings.as_mut().filter(|_| e.listener == L_SETTINGS_CANCEL) {
                s.returning = "";
            }
            if let Some(d) = st(app).error.as_mut().filter(|_| e.listener == L_ERROR_CANCEL) {
                d.returning = false;
            }
        }
        L_SETTINGS_CLOSE => settings_closed(app, host),
        L_ERROR_CLICK => error_click(app, host, e),
        L_ERROR_CLOSE => error_closed(app, host),
        L_APP_OPEN => host.out().class(Ui::MODAL_DOWNLOAD_APP, "visible", true),
        L_APP_CLOSE => host.out().class(Ui::MODAL_DOWNLOAD_APP, "visible", false),
        L_APP_OVERLAY if on_overlay(e, Ui::MODAL_DOWNLOAD_APP) => host.out().class(Ui::MODAL_DOWNLOAD_APP, "visible", false),
        L_APP_LINK => app.sched.timer(host.out(), T_APP_CLOSE, 100.0),
        _ => {}
    }
    0
}

pub fn timer(_app: &mut App, host: &mut dyn Host, t: u32) {
    if t == T_APP_CLOSE {
        host.out().class(Ui::MODAL_DOWNLOAD_APP, "visible", false);
    }
}

pub fn io(app: &mut App, host: &mut dyn Host, io: &Io) -> bool {
    let Some(at) = st(app).jobs.iter().position(|j| j.0 == io.req) else { return false };
    let (_, job) = st(app).jobs.swap_remove(at);
    match job {
        Job::Load => settings_loaded(app, host, io),
        Job::Save => prune(app, host, io),
        Job::Prune => {
            remove_settings(app, host);
            // v5: `await reload(); host.refreshed?.()` — the library is read again and Home rendered from it.
            crate::home::load(app, host);
        }
        Job::Copied => {
            if let Some(d) = app.st.home_dialogs.offline.error.as_ref() {
                host.out().text(d.copy, if io.status >= 0 { "Copied" } else { "Select the error text" });
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::redact;

    #[test]
    fn tokens_are_redacted_like_5_18_0() {
        assert_eq!(redact("Authorization: Bearer abc.def \"x\""), "Authorization: Bearer [redacted] \"x\"");
        assert_eq!(redact("bearer\ttok'"), "Bearer [redacted]'");
        assert_eq!(redact("no token Bearer"), "no token Bearer");
    }
}
