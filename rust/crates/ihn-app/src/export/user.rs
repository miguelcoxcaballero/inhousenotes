//! Export to the user: the export modal, the forced export, the download of that head revision, and the delivery
//! through the Android export bridge v2 / `navigator.share` / `<a download>` (ARCHITECTURE §8.5). Owned by W5.4.
// v5: app-v5.js:sanitizeFilename / openExportModal / closeExportModal / confirmExportModal / exportPDF /
// exportDrivePdf / deliverExportedPdf / waitForNativePdfExportResult

use super::pdf::{self, Built, Purpose};
use crate::abi::{chan, ety, ret, Event, Io};
use crate::app::Mod;
use crate::dom::Prop;
use crate::doc::session;
use crate::drive::revisions;
use crate::editor::view;
use crate::shell::fx::{self, Response};
use crate::shell::json::Json;
use crate::shell::{Status, StatusOpts};
use crate::ui_ids::Ui;
use crate::{abi, collab, shell, tag, App, Host};

const L_OPEN: u32 = tag(Mod::ExportUser, 1);
const L_CANCEL: u32 = tag(Mod::ExportUser, 2);
const L_CONFIRM: u32 = tag(Mod::ExportUser, 3);
const L_OVERLAY: u32 = tag(Mod::ExportUser, 4);
const L_INPUT: u32 = tag(Mod::ExportUser, 5);
const T_NATIVE: u32 = tag(Mod::ExportUser, 6);
const T_FORCE: u32 = tag(Mod::ExportUser, 7);
const L_INPUT_KEY: u32 = tag(Mod::ExportUser, 8);
/// `appendPdfExportChunk` size (5.18.0 `chunkSize`).
const CHUNK_CHARS: u32 = 240_000;
/// 5.18.0 waits 30 s for `handleInhousePdfExportResult`.
const NATIVE_WAIT_MS: f64 = 30_000.0;
/// The forced export must finish within this time (the loop retries meanwhile).
const FORCE_WAIT_MS: f64 = 120_000.0;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Stage {
    /// Waiting for the forced export (`desk.force` cleared).
    Forcing,
    Downloading(u32),
    Building,
    /// `beginPdfExport` → chunks → `finishPdfExport` → the result.
    NativeBegin(u32),
    NativeChunks(u32),
    NativeFinish(u32),
    NativeWait,
    Sharing(u32),
}

struct Run {
    name: String,
    blob: u32,
    stage: Stage,
}

/// This module's state, stored in `App::st`.
#[derive(Default)]
pub struct State {
    value: String,
    run: Option<Run>,
    /// Test: delivered `(path, name)` pairs and errors.
    log: Vec<String>,
}

// v5: app-v5.js:sanitizeFilename
pub fn sanitize_filename(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut run = false;
    for c in name.chars() {
        let bad = "\\/:*?\"<>|".contains(c);
        if !bad {
            out.push(c);
        } else if !run {
            out.push('_');
        }
        run = bad;
    }
    let cleaned = out.trim();
    if cleaned.is_empty() { "cuaderno".into() } else { cleaned.into() }
}

/// The modal's name: `input.value.trim()`, a trailing `.pdf` removed, then sanitized.
pub fn confirmed_name(value: &str) -> String {
    let mut name = value.trim();
    if name.len() >= 4 && name.is_char_boundary(name.len() - 4) && name[name.len() - 4..].eq_ignore_ascii_case(".pdf") {
        name = &name[..name.len() - 4];
    }
    sanitize_filename(name)
}

fn open_modal(app: &mut App, host: &mut dyn Host) {
    // v5: input.value = state.exportName || 'cuaderno'
    let name = session::get(app).map(|s| s.doc.name.v.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| "cuaderno".into());
    app.st.export_user.value = name.clone();
    let out = host.out();
    out.prop(Ui::EXPORT_FILENAME, "value", Prop::Str(&name));
    out.class(Ui::MODAL_EXPORT_PDF, "visible", true);
    // v5: requestAnimationFrame(() => { input.focus(); input.select(); })
    app.sched.frame(host.out(), Mod::ExportUser);
}

fn close_modal(host: &mut dyn Host) { host.out().class(Ui::MODAL_EXPORT_PDF, "visible", false); }

fn confirm(app: &mut App, host: &mut dyn Host) {
    let name = confirmed_name(&app.st.export_user.value);
    if !view::read_only(app) {
        if session::get(app).is_some_and(|s| s.doc.name.v != name) {
            session::edit(app, host, |tx| tx.set_name(&name));
        }
        view::update_title(app, host);
    }
    close_modal(host);
    export(app, host, format!("{name}.pdf"));
}

fn status(app: &mut App, host: &mut dyn Host, text: &str, error: bool) {
    shell::status(app, host, text, StatusOpts::from(if error { Status::Error } else { Status::Info }).preserve());
}

fn fail(app: &mut App, host: &mut dyn Host, message: &str) {
    if let Some(run) = app.st.export_user.run.take() {
        host.out().obj_release(run.blob);
    }
    app.sched.clear(host.out(), T_NATIVE);
    app.sched.clear(host.out(), T_FORCE);
    app.st.export_user.log.push(format!("error:{message}"));
    status(app, host, message, true);
    shell::hide_loading(app, host);
}

fn success(app: &mut App, host: &mut dyn Host, how: &str) {
    let Some(run) = app.st.export_user.run.take() else { return };
    host.out().obj_release(run.blob);
    app.sched.clear(host.out(), T_NATIVE);
    app.st.export_user.log.push(format!("{how}:{}", run.name));
    status(app, host, "PDF exported", false);
    shell::hide_loading(app, host);
}

// v5: app-v5.js:exportPDF
fn export(app: &mut App, host: &mut dyn Host, name: String) {
    if app.st.export_user.run.is_some() {
        return status(app, host, "A PDF export is already in progress", true);
    }
    let Some(s) = session::get(app) else { return };
    let (created, writable, doc) = (s.record.flags.created && !s.account.is_empty(), s.writable(), s.doc_id.clone());
    app.st.export_user.run = Some(Run { name, blob: 0, stage: Stage::Building });
    if !created {
        shell::loading(app, host, "Generando PDF...");
        return pdf::start(app, host, Purpose::User);
    }
    if !writable {
        return download(app, host, &doc);
    }
    // v5: exportDrivePdf — save the latest changes in Drive first (a forced export, SYNC §14.1).
    shell::loading(app, host, "Guardando últimos cambios en Drive...");
    let forced = collab::with_sync(app, |s| s.core.export.force = true).is_some();
    if !forced {
        return fail(app, host, "Drive autosave is not available");
    }
    if let Some(run) = app.st.export_user.run.as_mut() {
        run.stage = Stage::Forcing;
    }
    app.sched.timer(host.out(), T_FORCE, FORCE_WAIT_MS);
    collab::kick(app, host);
}

/// After every engine call (`collab` → `export::upload::desk`): the forced export ended.
pub fn desk(app: &mut App, host: &mut dyn Host) {
    if !app.st.export_user.run.as_ref().is_some_and(|r| r.stage == Stage::Forcing) {
        return;
    }
    let Some((force, done)) = collab::with_sync(app, |s| (s.core.export.force, s.core.export.done.clone())) else { return };
    if force || done.is_none() {
        return;
    }
    app.sched.clear(host.out(), T_FORCE);
    let doc = session::doc_id(app).to_string();
    download(app, host, &doc);
}

fn download(app: &mut App, host: &mut dyn Host, doc: &str) {
    shell::loading(app, host, "Descargando PDF de Drive...");
    let key = session::get(app).map_or(String::new(), |s| s.record.drive_meta.get("resourceKey").text());
    let req = revisions::download(app, host, Mod::ExportUser, doc, &key);
    if let Some(run) = app.st.export_user.run.as_mut() {
        run.stage = Stage::Downloading(req);
    }
}

/// The locally built PDF of a document that is not in Drive (`export::pdf`, high quality).
pub fn built(app: &mut App, host: &mut dyn Host, out: Built) {
    if !app.st.export_user.run.as_ref().is_some_and(|r| r.stage == Stage::Building) {
        return;
    }
    let Some((bytes, _)) = out else { return fail(app, host, "PDF export failed") };
    let blob = fx::new_obj(app);
    fx::blob_new(host.out(), blob, "application/pdf", &bytes);
    deliver(app, host, blob);
}

// v5: app-v5.js:deliverExportedPdf
fn deliver(app: &mut App, host: &mut dyn Host, blob: u32) {
    let Some(run) = app.st.export_user.run.as_mut() else { return host.out().obj_release(blob) };
    run.blob = blob;
    let name = run.name.clone();
    if app.cfg.has(abi::mode::APP) {
        let req = app.req(Mod::ExportUser);
        fx::native_call(host.out(), req, "beginPdfExport", &[&name]);
        return set(app, Stage::NativeBegin(req));
    }
    web_download(app, host);
}

fn set(app: &mut App, stage: Stage) {
    if let Some(run) = app.st.export_user.run.as_mut() {
        run.stage = stage;
    }
}

fn share(app: &mut App, host: &mut dyn Host) {
    let Some(run) = app.st.export_user.run.as_ref() else { return };
    let (blob, name) = (run.blob, run.name.clone());
    let req = app.req(Mod::ExportUser);
    fx::share(host.out(), req, blob, &name, "application/pdf", &name);
    set(app, Stage::Sharing(req));
}

fn web_download(app: &mut App, host: &mut dyn Host) {
    let Some(run) = app.st.export_user.run.as_ref() else { return };
    let app_mode = app.cfg.has(abi::mode::APP);
    fx::download(host.out(), run.blob, &run.name, if app_mode { 120.0 } else { 0.0 }, if app_mode { 60_000.0 } else { 1_200.0 });
    success(app, host, "download");
}

fn io(app: &mut App, host: &mut dyn Host, io: &Io) {
    let Some(stage) = app.st.export_user.run.as_ref().map(|r| r.stage) else { return host.out().obj_release(io.obj) };
    match stage {
        Stage::Downloading(req) if req == io.req => {
            if io.status < 0 || io.obj == 0 {
                host.out().obj_release(io.obj);
                let message = std::str::from_utf8(io.data).unwrap_or("").to_string();
                return fail(app, host, if message.is_empty() { "PDF export failed" } else { &message });
            }
            if Response::parse(io.data).blob_size() <= 0.0 {
                host.out().obj_release(io.obj);
                return fail(app, host, "Drive returned an empty PDF");
            }
            deliver(app, host, io.obj);
        }
        Stage::NativeBegin(req) if req == io.req => {
            // The bridge lacks beginPdfExport: navigator.share, then <a download>.
            if io.status < 0 {
                return share(app, host);
            }
            let Some(run) = app.st.export_user.run.as_ref() else { return };
            let blob = run.blob;
            let req = app.req(Mod::ExportUser);
            fx::native_blob(host.out(), req, blob, "appendPdfExportChunk", CHUNK_CHARS);
            set(app, Stage::NativeChunks(req));
        }
        Stage::NativeChunks(req) if req == io.req => {
            if io.status < 0 {
                return fail(app, host, "Android could not save the PDF");
            }
            app.sched.timer(host.out(), T_NATIVE, NATIVE_WAIT_MS);
            let req = app.req(Mod::ExportUser);
            fx::native_call(host.out(), req, "finishPdfExport", &[]);
            set(app, Stage::NativeFinish(req));
        }
        Stage::NativeFinish(req) if req == io.req => {
            if io.status < 0 {
                return fail(app, host, "Android could not save the PDF");
            }
            // The result may already have arrived (`handleInhousePdfExportResult` during finishPdfExport).
            if app.st.export_user.run.as_ref().is_some_and(|r| r.stage == Stage::NativeFinish(req)) {
                set(app, Stage::NativeWait);
            }
        }
        Stage::Sharing(req) if req == io.req => {
            if io.status == -1 && io.data.is_empty() {
                return web_download(app, host);
            }
            if io.status < 0 {
                let message = std::str::from_utf8(io.data).unwrap_or("").to_string();
                return fail(app, host, if message.is_empty() { "PDF export failed" } else { &message });
            }
            success(app, host, "share");
        }
        _ => host.out().obj_release(io.obj),
    }
}

// v5: app-v5.js:waitForNativePdfExportResult — `{ok}` resolves; otherwise its message or the generic text.
fn native_result(app: &mut App, host: &mut dyn Host, payload: &[u8]) {
    if !app.st.export_user.run.as_ref().is_some_and(|r| matches!(r.stage, Stage::NativeWait | Stage::NativeFinish(_))) {
        return;
    }
    let text = std::str::from_utf8(payload).unwrap_or("");
    let json = Json::parse(text).or_else(|| Json::parse(Json::parse(text)?.as_str()?));
    let ok = json.as_ref().and_then(|j| j.get("ok")).is_some_and(Json::truthy);
    if ok {
        return success(app, host, "native");
    }
    let message = json.as_ref().map(|j| j.text("message").to_string()).filter(|m| !m.is_empty()).unwrap_or_else(|| "Android could not save the PDF".into());
    fail(app, host, &message);
}

/// Called once at boot, in `app::MODULES` order, after the `Ui` table is bound.
pub fn init(app: &mut App, host: &mut dyn Host) {
    if app.cfg.has(abi::mode::HEADLESS) {
        return;
    }
    let out = host.out();
    out.listen(Ui::BTN_EXPORT_PDF, "click", L_OPEN, 0);
    out.listen(Ui::MODAL_EXPORT_CANCEL, "click", L_CANCEL, 0);
    out.listen(Ui::MODAL_EXPORT_CONFIRM, "click", L_CONFIRM, 0);
    out.listen(Ui::MODAL_EXPORT_PDF, "click", L_OVERLAY, 0);
    out.listen(Ui::EXPORT_FILENAME, "keydown", L_INPUT_KEY, 0);
    out.listen(Ui::EXPORT_FILENAME, "input", L_INPUT, 0);
}

/// Events addressed to this module (ids made with `abi::tag`) and broadcasts (`Life`, `Msg`). Returns `abi::ret` bits.
pub fn event(app: &mut App, host: &mut dyn Host, event: &Event) -> u32 {
    match *event {
        Event::Dom(e) => match e.listener {
            L_OPEN => open_modal(app, host),
            L_CANCEL => close_modal(host),
            L_CONFIRM => confirm(app, host),
            // v5: `if (e.target.id === 'modal-export-pdf') closeExportModal()`.
            L_OVERLAY if e.path.first() == Ui::MODAL_EXPORT_PDF => close_modal(host),
            L_INPUT if e.ty == ety::INPUT => app.st.export_user.value = e.value.to_string(),
            L_INPUT_KEY if e.ty == ety::KEYDOWN => {
                app.st.export_user.value = e.value.to_string();
                match e.key {
                    "Enter" => {
                        confirm(app, host);
                        return ret::PREVENT_DEFAULT;
                    }
                    "Escape" => {
                        close_modal(host);
                        return ret::PREVENT_DEFAULT;
                    }
                    _ => {}
                }
            }
            _ => {}
        },
        Event::Frame(_) => {
            host.out().focus(Ui::EXPORT_FILENAME, false);
            host.out().select(Ui::EXPORT_FILENAME);
        }
        Event::Io(io) => self::io(app, host, &io),
        Event::Msg(m) if m.chan == chan::PDF_EXPORT => native_result(app, host, m.data),
        Event::Timer(T_NATIVE) => fail(app, host, "Android did not confirm the PDF download"),
        Event::Timer(T_FORCE) => fail(app, host, "Drive did not confirm the latest document version"),
        _ => {}
    }
    0
}

/// `__IHN_TEST_API__.cmd('export.user.<name>', args)`: `state` → `{running, log}`; `export` `{name}` runs the
/// modal's confirm with that name.
pub fn test_cmd(app: &mut App, host: &mut dyn Host, name: &str, args: &str) -> Option<String> {
    match name {
        "state" => {
            let st = &app.st.export_user;
            let log: Vec<String> = st.log.iter().map(|l| crate::testapi::json_str(l)).collect();
            Some(format!("{{\"running\":{},\"log\":[{}]}}", st.run.is_some(), log.join(",")))
        }
        "export" => {
            app.st.export_user.value = Json::parse(args).map_or(String::new(), |j| j.text("name").to_string());
            confirm(app, host);
            Some("true".into())
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_follow_5_18_0() {
        assert_eq!(sanitize_filename("  a/b:c  "), "a_b_c");
        assert_eq!(sanitize_filename("   "), "cuaderno");
        assert_eq!(confirmed_name(" Notes.PDF "), "Notes");
        assert_eq!(confirmed_name("x<>y.pdf"), "x_y");
        assert_eq!(confirmed_name(".pdf"), "cuaderno");
    }
}
