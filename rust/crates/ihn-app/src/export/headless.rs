//! `?offline-worker` headless runs (ARCHITECTURE §12) and the same work in the foreground while Home is shown.
//! Owned by W5.4.
//!
//! The runner lists the account's documents with `relay_pending`, `export_pending` or not created, and for each one
//! loads a quiet session (the engine starts on `sig::OPENED` and takes `ihn6-net`), lets the relay flush, creates the
//! Drive file when needed (`export::upload`) and forces an export (SYNC §14.1 (b)). Headless (Android WorkManager,
//! `?inhouse_app=1&offline-worker=1`): then `InhouseBackground.complete({busy, pending, errors})`. Foreground: when
//! the connection comes back while Home is shown, documents left pending by earlier sessions are finished the same
//! way (an open from Home replaces the runner's session and stops it).

use crate::abi::{mode, Event};
use crate::app::Mod;
use crate::doc::session::{self, LOADED_EMPTY};
use crate::shell::fx::{self, Background};
use crate::shell::View;
use crate::store::schema::DocRecord;
use crate::store::{self, Reply, Tx as StoreTx};
use crate::testapi::json_str;
use crate::{auth, collab, shell, tag, App, Host};
use ihn_core::doc::{ExportPhase, SyncEngine};

const T_POLL: u32 = tag(Mod::ExportHeadless, 1);
const T_START: u32 = tag(Mod::ExportHeadless, 2);
const POLL_MS: f64 = 250.0;
/// One document may take this long before the runner gives up on it.
const DOC_LIMIT_MS: f64 = 90_000.0;
/// Foreground: the connection must have been back for this long first.
const FOREGROUND_DELAY_MS: f64 = 3_000.0;

struct Current {
    doc: String,
    started: f64,
    load_req: u32,
    loaded: bool,
    forced: bool,
}

/// This module's state, stored in `App::st`.
#[derive(Default)]
pub struct State {
    running: bool,
    account: String,
    scan_req: u32,
    queue: Vec<String>,
    cur: Option<Current>,
    errors: Vec<String>,
    pending: u32,
    /// Test: documents finished, in order.
    log: Vec<String>,
    report: Option<String>,
    /// Test: runs started.
    runs: u32,
}

fn headless(app: &App) -> bool { app.cfg.has(mode::HEADLESS) }

/// Lists the documents that still need work and starts on them.
fn start(app: &mut App, host: &mut dyn Host) {
    let account = auth::known_account(host);
    if app.st.export_headless.running || account.is_empty() {
        if headless(app) && account.is_empty() {
            report(app, host);
        }
        return;
    }
    let st = &mut app.st.export_headless;
    (st.running, st.account, st.errors, st.pending) = (true, account, Vec::new(), 0);
    st.runs += 1;
    let mut t = StoreTx::new();
    t.docs_scan();
    let req = app.req(Mod::ExportHeadless);
    app.st.export_headless.scan_req = req;
    store::tx(app, host, req, t);
}

fn needs_work(r: &DocRecord) -> bool { r.flags.relay_pending || r.flags.export_pending || !r.flags.created }

fn scanned(app: &mut App, host: &mut dyn Host, io: &crate::abi::Io) {
    let mut reply = Reply::new(io);
    let account = app.st.export_headless.account.clone();
    let rows = if reply.ok() { reply.rows() } else { Vec::new() };
    let mut queue = Vec::new();
    for (key, value) in &rows {
        let mut handles = Vec::new();
        value.handles(&mut handles);
        handles.iter().for_each(|h| host.out().obj_release(*h));
        let r = DocRecord::from_value(value);
        if r.account == account && needs_work(&r) {
            queue.push(key.text());
        }
    }
    app.st.export_headless.queue = queue;
    next(app, host);
}

fn next(app: &mut App, host: &mut dyn Host) {
    app.sched.clear(host.out(), T_POLL);
    let st = &mut app.st.export_headless;
    st.cur = None;
    if !headless(app) && (shell::view(app) != View::Home || shell::offline(app)) {
        return stop(app, host);
    }
    let st = &mut app.st.export_headless;
    let Some(doc) = (!st.queue.is_empty()).then(|| st.queue.remove(0)) else { return stop(app, host) };
    let account = st.account.clone();
    let now = host.now();
    let req = app.req(Mod::ExportHeadless);
    app.st.export_headless.cur = Some(Current { doc: doc.clone(), started: now, load_req: req, loaded: false, forced: false });
    session::load(app, host, &account, &doc, req);
}

fn stop(app: &mut App, host: &mut dyn Host) {
    let st = &mut app.st.export_headless;
    st.running = false;
    st.cur = None;
    st.pending += st.queue.len() as u32;
    st.queue.clear();
    app.sched.clear(host.out(), T_POLL);
    if headless(app) {
        report(app, host);
    } else if shell::view(app) == View::Home {
        // The cards' badges follow the flags the runner cleared.
        crate::home::refresh(app, host);
    }
}

/// `InhouseBackground.complete({busy, pending, errors})`: WorkManager succeeds iff nothing is pending.
fn report(app: &mut App, host: &mut dyn Host) {
    let st = &app.st.export_headless;
    let errors: Vec<String> = st.errors.iter().map(|e| json_str(e)).collect();
    let json = format!("{{\"busy\":false,\"pending\":{},\"errors\":[{}]}}", st.pending, errors.join(","));
    app.st.export_headless.report = Some(json.clone());
    let req = app.req(Mod::ExportHeadless);
    fx::background(host.out(), req, Background::Complete, &json);
}

fn loaded(app: &mut App, host: &mut dyn Host, status: i32) {
    let Some(cur) = app.st.export_headless.cur.as_mut() else { return };
    cur.loaded = true;
    let doc = cur.doc.clone();
    if status == LOADED_EMPTY || session::doc_id(app) != doc {
        app.st.export_headless.errors.push(format!("{doc}: not stored on this device"));
        return next(app, host);
    }
    // The engine starts on this signal (collab), the create on it too (export::upload).
    shell::signal(app, host, session::sig::OPENED, &doc);
    app.sched.timer(host.out(), T_POLL, POLL_MS);
}

/// Waits for the document's engine to settle: everything sealed, the Drive file created and an export done.
fn poll(app: &mut App, host: &mut dyn Host) {
    let Some(cur) = app.st.export_headless.cur.as_ref() else { return };
    let (doc, started, forced) = (cur.doc.clone(), cur.started, cur.forced);
    if session::doc_id(app) != doc {
        // Something else opened a document (a tap on a card): the runner stops.
        app.st.export_headless.pending += 1;
        return stop(app, host);
    }
    let created = session::get(app).is_some_and(|s| s.record.flags.created);
    let state = collab::with_sync(app, |s| {
        let st = s.status();
        let settled = st.unsealed_local == 0 && !st.not_created && !st.relay_writing && matches!(st.export, ExportPhase::Idle);
        (s.core.export.force, settled, s.core.polled)
    });
    if let (Some((force, settled, polled)), true) = (state, created) {
        if !forced && polled && session::get(app).is_some_and(|s| s.writable()) {
            collab::with_sync(app, |s| s.core.export.force = true);
            if let Some(c) = app.st.export_headless.cur.as_mut() {
                c.forced = true;
            }
            collab::kick(app, host);
        } else if forced && !force && settled {
            app.st.export_headless.log.push(doc);
            super::lifecycle::note_home(app, host);
            session::close(app, host);
            return next(app, host);
        }
    }
    if host.now() - started > DOC_LIMIT_MS {
        let st = &mut app.st.export_headless;
        st.pending += 1;
        st.errors.push(format!("{doc}: timed out"));
        session::close(app, host);
        return next(app, host);
    }
    app.sched.timer(host.out(), T_POLL, POLL_MS);
}

/// The connection came back: documents left pending are finished from Home a little later.
fn back_online(app: &mut App, host: &mut dyn Host) {
    if !headless(app) && auth::token(app).is_some() && !app.st.export_headless.running {
        app.sched.timer(host.out(), T_START, FOREGROUND_DELAY_MS);
    }
}

/// Called once at boot, in `app::MODULES` order, after the `Ui` table is bound.
pub fn init(app: &mut App, host: &mut dyn Host) {
    if headless(app) {
        auth::init_drive_auth(app, host);
        app.sched.timer(host.out(), T_START, 0.0);
    }
}

/// Events addressed to this module (ids made with `abi::tag`) and broadcasts (`Life`, `Msg`). Returns `abi::ret` bits.
pub fn event(app: &mut App, host: &mut dyn Host, event: &Event) -> u32 {
    match *event {
        Event::Timer(T_START) => {
            if headless(app) || shell::view(app) == View::Home && !shell::offline(app) {
                start(app, host);
            }
        }
        Event::Timer(T_POLL) => poll(app, host),
        Event::Io(io) if io.req == app.st.export_headless.scan_req => {
            app.st.export_headless.scan_req = 0;
            scanned(app, host, &io);
        }
        Event::Io(io) if app.st.export_headless.cur.as_ref().is_some_and(|c| c.load_req == io.req && !c.loaded) => loaded(app, host, io.status),
        Event::Msg(m) if m.chan == shell::sig::NETWORK && m.data == b"1" => back_online(app, host),
        Event::Life(l) if l.code == crate::abi::life::ONLINE => back_online(app, host),
        _ => {}
    }
    0
}

/// `__IHN_TEST_API__.cmd('export.headless.<name>', args)`: `state` → `{running, queue, done, report}`; `run`
/// starts the foreground runner now.
pub fn test_cmd(app: &mut App, host: &mut dyn Host, name: &str, _args: &str) -> Option<String> {
    match name {
        "state" => {
            let st = &app.st.export_headless;
            let done: Vec<String> = st.log.iter().map(|d| json_str(d)).collect();
            let errors: Vec<String> = st.errors.iter().map(|d| json_str(d)).collect();
            Some(format!("{{\"running\":{},\"runs\":{},\"queue\":{},\"done\":[{}],\"errors\":[{}],\"report\":{}}}", st.running, st.runs, st.queue.len(), done.join(","), errors.join(","), st.report.as_deref().unwrap_or("null")))
        }
        "run" => {
            start(app, host);
            Some("true".into())
        }
        _ => None,
    }
}
