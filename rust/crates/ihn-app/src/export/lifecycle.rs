//! Home exit, pagehide, visibility (ARCHITECTURE §12, SYNC §14.5). Owned by W5.4.
//!
//! The Home button waits only for local durability: `Saving on this device…` while a local batch is not committed
//! (in degraded storage, until every own batch is sealed), then Home is shown and sync/export continue detached
//! (`collab` leaves: `Bye`, record `l`, relay flush, export if alone). `pagehide` / hidden fold the dirty pages into
//! chunks (the relay keepalive flush is the engine's); `beforeunload` prompts only while an exit export runs.
// v5: app-v5.js:goHome (the driveHomeBtn handler) / beforeunload handler

use crate::abi::{ety, life, ret, Event};
use ihn_core::doc::{ExportPhase, SyncEngine};
use crate::app::Mod;
use crate::doc::session;
use crate::shell::Status;
use crate::ui_ids::Ui;
use crate::{abi, home, shell, store, tag, App, Host};

const L_HOME: u32 = tag(Mod::ExportLifecycle, 1);
const T_CHECK: u32 = tag(Mod::ExportLifecycle, 2);
/// How long Home waits for the local commit (or, in degraded storage, the seal) before giving up.
const WAIT_LIMIT_MS: f64 = 15_000.0;

/// This module's state, stored in `App::st`.
#[derive(Default)]
pub struct State {
    /// `goHomeInProgress`: the time the wait started.
    going: Option<f64>,
    /// Test: Home exits `(waited, status text)`.
    log: Vec<(bool, String)>,
}

/// What Home still waits for: a local batch not committed, or (degraded storage) own batches not sealed.
fn must_wait(app: &App) -> bool {
    let local = session::local(app);
    local.pending || (store::degraded(app) && session::sync_status(app).unsealed_local > 0)
}

// v5: app-v5.js:goHome
fn go_home(app: &mut App, host: &mut dyn Host) {
    if app.st.export_lifecycle.going.is_some() {
        return;
    }
    app.sched.set_exiting(true);
    if must_wait(app) {
        app.st.export_lifecycle.going = Some(host.now());
        shell::loading_home_save(app, host, "Saving on this device…");
        app.sched.timer(host.out(), T_CHECK, 100.0);
        return;
    }
    app.st.export_lifecycle.going = Some(host.now());
    finish(app, host, false);
}

fn finish(app: &mut App, host: &mut dyn Host, waited: bool) {
    app.st.export_lifecycle.going = None;
    app.sched.clear(host.out(), T_CHECK);
    app.sched.set_exiting(false);
    if session::local(app).failed && session::sync_status(app).unsealed_local > 0 && store::degraded(app) || waited && must_wait(app) {
        shell::hide_loading(app, host);
        return shell::status(app, host, "Could not finish saving. The document is still open; please try Home again.", Status::Error);
    }
    let status = session::sync_status(app);
    let pending = status.unsealed_local > 0 || status.not_created;
    // Compaction folds the dirty pages before the document leaves the screen.
    session::compact(app, host, false);
    home::show(app, host);
    let text = if pending { "Saved on this device · waiting to sync" } else { "All changes saved." };
    shell::home_status(host, text);
    app.st.export_lifecycle.log.push((waited, text.into()));
    home::refresh(app, host);
    shell::hide_loading(app, host);
}

fn check(app: &mut App, host: &mut dyn Host) {
    let Some(start) = app.st.export_lifecycle.going else { return };
    if !must_wait(app) || host.now() - start > WAIT_LIMIT_MS || session::local(app).failed && !store::degraded(app) {
        return finish(app, host, true);
    }
    app.sched.timer(host.out(), T_CHECK, 100.0);
}

/// An export started by a leaving engine is still running (5.18.0 prompted while its exit save ran).
fn exit_export_running(app: &App) -> bool {
    app.st.collab.run.as_ref().is_some_and(|r| r.leaving() && r.sync.core.export.build.is_some() || r.leaving() && !matches!(r.sync.status().export, ExportPhase::Idle | ExportPhase::Failed { .. }))
}

/// Called once at boot, in `app::MODULES` order, after the `Ui` table is bound.
pub fn init(app: &mut App, host: &mut dyn Host) {
    if app.cfg.has(abi::mode::HEADLESS) {
        return;
    }
    host.out().listen(Ui::BTN_DRIVE_HOME, "click", L_HOME, 0);
    host.out().listen(Ui::BTN_DRIVE_HOME, "keydown", L_HOME, 0);
}

/// Events addressed to this module (ids made with `abi::tag`) and broadcasts (`Life`, `Msg`). Returns `abi::ret` bits.
pub fn event(app: &mut App, host: &mut dyn Host, event: &Event) -> u32 {
    match *event {
        // v5: `if (docTitleEl && docTitleEl.contains(e.target)) return;`
        Event::Dom(e) if e.listener == L_HOME && e.ty == ety::CLICK && !e.path.contains(Ui::DOC_TITLE) => go_home(app, host),
        Event::Dom(e) if e.listener == L_HOME && e.ty == ety::KEYDOWN && !e.path.contains(Ui::DOC_TITLE) && (e.key == "Enter" || e.key == " ") => {
            go_home(app, host);
            return ret::PREVENT_DEFAULT;
        }
        Event::Timer(T_CHECK) => check(app, host),
        Event::Msg(m) if m.chan == session::sig::COMMITTED || m.chan == session::sig::STATE => {
            if app.st.export_lifecycle.going.is_some() && !must_wait(app) {
                finish(app, host, true);
            }
        }
        Event::Life(l) => match l.code {
            life::PAGEHIDE => session::compact(app, host, false),
            life::VISIBILITY if l.a != 0.0 => session::compact(app, host, false),
            life::BEFOREUNLOAD if exit_export_running(app) => return ret::PREVENT_DEFAULT,
            _ => {}
        },
        _ => {}
    }
    0
}

/// `__IHN_TEST_API__.cmd('export.lifecycle.<name>', args)`: `state` → `{going, log: [[waited, text]…]}`.
pub fn test_cmd(app: &mut App, _host: &mut dyn Host, name: &str, _args: &str) -> Option<String> {
    match name {
        "state" => {
            let st = &app.st.export_lifecycle;
            let log: Vec<String> = st.log.iter().map(|(w, t)| format!("[{w},{}]", crate::testapi::json_str(t))).collect();
            Some(format!("{{\"going\":{},\"log\":[{}]}}", st.going.is_some(), log.join(",")))
        }
        _ => None,
    }
}
