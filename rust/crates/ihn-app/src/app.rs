//! `App`: root state, module table, event dispatch and the scheduler (ARCHITECTURE §2.3).
//!
//! # Module contract
//! Every UI/logic module of `ihn-app` listed in [`MODULES`] provides
//! `State: Default`, `init(&mut App, &mut dyn Host)`, `event(&mut App, &mut dyn Host, &Event) -> u32` and
//! `test_cmd(&mut App, &mut dyn Host, name, args) -> Option<String>`. Its state lives in [`App::st`]
//! (`app.st.shell`, `app.st.drive_client`, …). Ids it hands to the host — listeners, timers, idle slots, deferred
//! jobs, effect requests — are made with [`abi::tag`]`(Mod::X, n)`, and the matching events come back to `X::event`.
//! `Life` and `Msg` events are broadcast to every module in table order. Modules call each other directly through
//! their public functions; nothing else needs `app.rs` to change.

use crate::abi::{self, fx, life, mode, ptr, tag, tag_mod, Config, Event, Node, OutBuf};
use crate::host::Host;
use crate::ui_ids::Ui;
use std::collections::VecDeque;

use crate::collab;
use crate::doc::{
    backgrounds as doc_backgrounds, create as doc_create, import as doc_import, links as doc_links, open as doc_open,
    public as doc_public, save_state as doc_save_state, session as doc_session, stats as doc_stats, v5open as doc_v5open,
    versions as doc_versions,
};
use crate::drive::{
    client as drive_client, home as drive_home, manage as drive_manage, perms as drive_perms, relay as drive_relay,
    revisions as drive_revisions,
};
use crate::editor::{
    eraser as editor_eraser, history as editor_history, images as editor_images, input as editor_input,
    layout as editor_layout, pages as editor_pages, pen as editor_pen, selection as editor_selection,
    toolbar as editor_toolbar, view as editor_view, viewport as editor_viewport,
};
use crate::export::{
    headless as export_headless, lifecycle as export_lifecycle, pdf as export_pdf, upload as export_upload,
    user as export_user,
};
use crate::home::dialogs as home_dialogs;
use crate::panels::{
    pages_panel as panels_pages_panel, photo_page as panels_photo_page, share as panels_share, slider as panels_slider,
    timeline as panels_timeline,
};
use crate::testapi as app;
use crate::{auth, home, migrate, shell, store, updater};

/// One entry of the module table.
pub struct Module {
    /// Test-command prefix: `__IHN_TEST_API__.cmd("<name>.<command>", args)`.
    pub name: &'static str,
    pub init: fn(&mut App, &mut dyn Host),
    pub event: fn(&mut App, &mut dyn Host, &Event) -> u32,
    pub test_cmd: fn(&mut App, &mut dyn Host, &str, &str) -> Option<String>,
}

macro_rules! modules {
    ($($variant:ident $module:ident $name:literal;)*) => {
        /// Module ids (the high byte of every id made with [`abi::tag`]).
        #[repr(u8)]
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        pub enum Mod { $($variant),* }

        /// The state of every module.
        #[derive(Default)]
        pub struct St { $(pub $module: $module::State),* }

        /// The module table, in `Mod` order. `init` runs in this order: infrastructure first, the shell (boot
        /// routing) last, so that it can call into every other module.
        pub static MODULES: &[Module] = &[
            $(Module { name: $name, init: $module::init, event: $module::event, test_cmd: $module::test_cmd }),*
        ];
    };
}

modules! {
    App app "app";
    Updater updater "updater";
    Store store "store";
    Auth auth "auth";
    DriveClient drive_client "drive.client";
    DriveHome drive_home "drive.home";
    DriveManage drive_manage "drive.manage";
    DrivePerms drive_perms "drive.perms";
    DriveRevisions drive_revisions "drive.revisions";
    DriveRelay drive_relay "drive.relay";
    DocSession doc_session "doc.session";
    DocOpen doc_open "doc.open";
    DocLinks doc_links "doc.links";
    DocPublic doc_public "doc.public";
    DocV5open doc_v5open "doc.v5open";
    DocCreate doc_create "doc.create";
    DocImport doc_import "doc.import";
    DocBackgrounds doc_backgrounds "doc.backgrounds";
    DocSaveState doc_save_state "doc.save_state";
    DocStats doc_stats "doc.stats";
    DocVersions doc_versions "doc.versions";
    EditorView editor_view "editor.view";
    EditorLayout editor_layout "editor.layout";
    EditorViewport editor_viewport "editor.viewport";
    EditorPages editor_pages "editor.pages";
    EditorInput editor_input "editor.input";
    EditorPen editor_pen "editor.pen";
    EditorEraser editor_eraser "editor.eraser";
    EditorSelection editor_selection "editor.selection";
    EditorImages editor_images "editor.images";
    EditorHistory editor_history "editor.history";
    EditorToolbar editor_toolbar "editor.toolbar";
    Collab collab "collab";
    PanelsPagesPanel panels_pages_panel "panels.pages_panel";
    PanelsPhotoPage panels_photo_page "panels.photo_page";
    PanelsSlider panels_slider "panels.slider";
    PanelsTimeline panels_timeline "panels.timeline";
    PanelsShare panels_share "panels.share";
    ExportPdf export_pdf "export.pdf";
    ExportUpload export_upload "export.upload";
    ExportUser export_user "export.user";
    ExportLifecycle export_lifecycle "export.lifecycle";
    ExportHeadless export_headless "export.headless";
    Home home "home";
    HomeDialogs home_dialogs "home.dialogs";
    Migrate migrate "migrate";
    Shell shell "shell";
}

/// Roles of the export worker: the first byte of every `ihn_worker` payload.
pub mod worker_role {
    /// Large 5.x imports and migration parsing → `doc::v5open::worker`.
    pub const IMPORT: u8 = 0;
    /// PDF assembly → `export::pdf::worker`.
    pub const EXPORT: u8 = 1;
}

/// Timer id of the scheduler's deferred-work poll (module byte `0xff` is never a table index).
const WORK_TIMER: u32 = 0xff00_0000;

struct Job {
    id: u32,
    session: u32,
    quiet_ms: f64,
}

/// Timers, frames, idle slots and input-priority gating of background work.
///
/// The gating is the 5.18.0 `document_work.rs` policy: a deferred job runs once input has been quiet for its
/// `quiet_ms` and nothing is busy; hidden/exiting pages bypass the wait; at most eight jobs resume per turn; a job
/// bound to a session that ended is handed back as stale.
///
/// One host timer (`WORK_TIMER`) polls the queue. Input never touches it (ARCHITECTURE §0.6): activity and
/// `set_busy(.., true)` only postpone work, so the pending timer is left alone and the delay is recomputed when it
/// fires; while something is busy no timer runs at all (5.18.0 re-polled every 100 ms) and the poll is armed when
/// the last module stops being busy. The timer is set again only when work could start earlier than it is armed for.
// v5: rust-core/src/document_work.rs:WorkQueue::poll
#[derive(Default)]
pub struct Sched {
    frame_mods: u64,
    raf: bool,
    last_activity: f64,
    busy: u64,
    hidden: bool,
    exiting: bool,
    session: u32,
    jobs: VecDeque<Job>,
    /// Work may be able to start earlier than the timer is armed for: `App::after` checks.
    rearm: bool,
    /// `Host::now` time at which the pending `WORK_TIMER` fires.
    armed: Option<f64>,
}

impl Sched {
    /// `setTimeout`: one `Event::Timer(id)` after `ms`. Setting a pending id again replaces it.
    pub fn timer(&mut self, out: &mut OutBuf, id: u32, ms: f64) { timer_set(out, id, ms, false); }
    /// `setInterval`: `Event::Timer(id)` every `ms` until [`Sched::clear`].
    pub fn interval(&mut self, out: &mut OutBuf, id: u32, ms: f64) { timer_set(out, id, ms, true); }
    /// `clearTimeout` / `clearInterval`.
    pub fn clear(&mut self, out: &mut OutBuf, id: u32) { out.fx_id(fx::TIMER_CLEAR, id); }
    /// Asks for one `Event::Frame` in the next animation frame (`requestAnimationFrame`); idempotent per frame.
    pub fn frame(&mut self, out: &mut OutBuf, m: Mod) {
        self.frame_mods |= 1 << m as u32;
        if !self.raf {
            self.raf = true;
            out.fx0(fx::RAF_REQUEST);
        }
    }
    /// `requestIdleCallback`: one `Event::Idle` (`timeout_ms <= 0`: no timeout).
    pub fn idle(&mut self, out: &mut OutBuf, id: u32, timeout_ms: f64) {
        let m = out.w.rec(abi::fam::FX, fx::IDLE_REQUEST);
        out.w.u32(id);
        out.w.f64(timeout_ms);
        out.w.end(m);
    }
    pub fn idle_cancel(&mut self, out: &mut OutBuf, id: u32) { out.fx_id(fx::IDLE_CANCEL, id); }

    /// Records user input at `t` (an event `timeStamp` or `host.now()`). Contact pointer events do this
    /// automatically (see `App::dispatch`); modules call it for the other 5.18.0 activity sources (`touchstart`,
    /// `touchmove`, `wheel`, `keydown`) with `DomEvent::t`.
    // v5: app-v5.js:noteRecentUserActivity
    pub fn activity(&mut self, t: f64) { self.last_activity = self.last_activity.max(t); }
    /// Milliseconds since the last recorded input.
    pub fn quiet_for(&self, now: f64) -> f64 { (now - self.last_activity).max(0.0) }
    /// Marks module `m` as interacting (pen in contact, gesture running, parent busy): deferred work waits.
    pub fn set_busy(&mut self, m: Mod, on: bool) {
        let bit = 1 << m as u32;
        self.busy = if on { self.busy | bit } else { self.busy & !bit };
        self.rearm |= self.busy == 0;
    }
    pub fn is_busy(&self) -> bool { self.busy != 0 }
    /// Explicit exit (Home button, export on exit): deferred work runs at once, like a hidden page.
    pub fn set_exiting(&mut self, on: bool) {
        self.exiting = on;
        self.rearm |= on;
    }
    /// The current document session; jobs deferred with `bound == true` are dropped as stale when it changes.
    pub fn session(&self) -> u32 { self.session }
    /// Starts a new session (document opened or closed).
    pub fn new_session(&mut self) -> u32 {
        self.session += 1;
        self.rearm = true;
        self.session
    }
    /// Queues background work: `Event::Work { id, stale }` is delivered to the module of `id` once input has been
    /// quiet for `quiet_ms` (0..=60000). Returns false when the id is already queued or the queue is full.
    pub fn defer(&mut self, id: u32, quiet_ms: f64, bound: bool) -> bool {
        if !(0.0..=60000.0).contains(&quiet_ms) || self.jobs.len() >= 4096 || self.jobs.iter().any(|j| j.id == id) {
            return false;
        }
        self.jobs.push_back(Job { id, session: if bound { self.session } else { u32::MAX }, quiet_ms });
        self.rearm = true;
        true
    }
    /// Removes a deferred job.
    pub fn cancel(&mut self, id: u32) { self.jobs.retain(|j| j.id != id); }

    /// Takes the jobs that may run now (at most eight) and returns the delay until the next poll, if any job waits.
    fn poll(&mut self, now: f64, due: &mut Vec<(u32, bool)>) -> Option<f64> {
        let (bypass, session, elapsed) = (self.hidden || self.exiting, self.session, self.quiet_for(now));
        let busy = self.busy != 0 && !bypass;
        self.jobs.retain(|job| {
            let stale = job.session != u32::MAX && job.session != session;
            let runnable = stale || (!busy && (bypass || job.quiet_ms <= elapsed));
            if runnable && due.len() < 8 {
                due.push((job.id, stale));
            }
            !(runnable && due.last().is_some_and(|d| d.0 == job.id))
        });
        self.next_delay(now)
    }

    /// Delay until the earliest waiting job could run: 1 ms for stale or already-runnable jobs; jobs that wait for
    /// a busy module have none (`set_busy(.., false)` asks again).
    fn next_delay(&self, now: f64) -> Option<f64> {
        let (bypass, elapsed) = (self.hidden || self.exiting, self.quiet_for(now));
        self.jobs.iter().filter_map(|job| {
            if (job.session != u32::MAX && job.session != self.session) || bypass {
                Some(1.0)
            } else if self.busy != 0 {
                None
            } else {
                Some((job.quiet_ms - elapsed).max(1.0))
            }
        }).reduce(f64::min)
    }

    /// Arms the poll timer for `now + delay` unless it is already due by then.
    fn arm(&mut self, out: &mut OutBuf, now: f64, delay: f64) {
        if self.armed.is_none_or(|at| now + delay < at) {
            self.armed = Some(now + delay);
            timer_set(out, WORK_TIMER, delay, false);
        }
    }
}

fn timer_set(out: &mut OutBuf, id: u32, ms: f64, repeat: bool) {
    let m = out.w.rec(abi::fam::FX, fx::TIMER_SET);
    out.w.u32(id);
    out.w.f64(ms);
    out.w.u8(repeat as u8);
    out.w.end(m);
}

/// The application.
pub struct App {
    /// Boot configuration, kept current for viewport, DPR and the `ONLINE` / `HIDDEN` / `DARK_SCHEME` /
    /// `REDUCED_MOTION` flags.
    pub cfg: Config,
    pub sched: Sched,
    /// Module states.
    pub st: St,
    req_counter: u32,
    ready_holds: u32,
    booted: bool,
    ready: bool,
}

impl App {
    pub fn new(cfg: Config) -> App {
        App { cfg, sched: Sched::default(), st: St::default(), req_counter: 0, ready_holds: 0, booted: false, ready: false }
    }

    /// Boot: binds the static nodes, writes the version labels, initialises every module, then reports ready.
    pub fn init(&mut self, host: &mut dyn Host) {
        self.sched.hidden = self.cfg.has(mode::HIDDEN);
        let out = host.out();
        for (i, id) in Ui::IDS.iter().enumerate() {
            out.bind_at(Node(Ui::FIRST + i as u32), id);
        }
        // v5: app-v5.js § 4.2 — document.querySelectorAll('[data-app-version]') → `v${APP_VERSION}`. The two labels
        // are the Home and editor profile menus.
        let label = format!("v{}", self.cfg.version);
        for root in [Ui::DRIVE_HOME, Ui::EDITOR_VIEW] {
            let node = out.query(root, "[data-app-version]");
            out.text(node, &label);
            out.release(node);
        }
        for module in MODULES {
            (module.init)(self, host);
        }
        self.booted = true;
        self.after(host);
    }

    /// A new effect request id owned by `m`; its completion arrives at `m`'s `event` as `Event::Io`.
    pub fn req(&mut self, m: Mod) -> u32 {
        self.req_counter = self.req_counter.wrapping_add(1) & 0x00ff_ffff;
        tag(m, self.req_counter)
    }

    /// Delays the `ready` signal (`__IHN_TEST_API__.ready()`) until [`App::release_ready`]: call from `init` when
    /// boot continues asynchronously (routing, first list from cache).
    pub fn hold_ready(&mut self) { self.ready_holds += 1; }
    pub fn release_ready(&mut self) { self.ready_holds = self.ready_holds.saturating_sub(1); }
    pub fn is_ready(&self) -> bool { self.ready }

    /// `ihn_event`: decodes and routes one event; returns `abi::ret` bits.
    pub fn event(&mut self, host: &mut dyn Host, kind: u32, bytes: &[u8]) -> u32 {
        let Some(event) = Event::decode(kind, bytes) else {
            host.log(2, "ihn: malformed event ignored");
            return 0;
        };
        let bits = self.dispatch(host, &event);
        self.after(host);
        bits
    }

    /// `ihn_frame`: delivers `Event::Frame` to the modules that asked for this frame.
    pub fn frame(&mut self, host: &mut dyn Host, t: f64) {
        let mods = std::mem::take(&mut self.sched.frame_mods);
        self.sched.raf = false;
        for (i, module) in MODULES.iter().enumerate() {
            if mods & 1 << i != 0 {
                (module.event)(self, host, &Event::Frame(t));
            }
        }
        self.after(host);
    }

    fn dispatch(&mut self, host: &mut dyn Host, event: &Event) -> u32 {
        let owner = match *event {
            Event::Dom(e) => e.listener,
            Event::Pointer(p) => {
                // v5: app-v5.js § init "Global hard interrupt" — pointerdown, and pointermove only in contact
                // (buttons, pressure or touch); hovering is not activity.
                let contact = p.raw.buttons != 0 || p.raw.pressure > 0.0 || p.ptype == ptr::TOUCH;
                if p.kind == ptr::DOWN || (contact && matches!(p.kind, ptr::MOVE | ptr::RAW_UPDATE)) {
                    self.sched.activity(p.raw.t);
                }
                p.listener
            }
            Event::Timer(WORK_TIMER) => return self.run_work(host),
            Event::Timer(id) | Event::Idle { id, .. } | Event::Work { id, .. } => id,
            Event::Io(io) => io.req,
            Event::Test { name, args } => {
                // The host only sends these with `?e2e=1` on localhost; refuse them anywhere else.
                if self.cfg.has(mode::E2E) {
                    crate::testapi::dispatch(self, host, name, args);
                }
                return 0;
            }
            Event::Life(l) => {
                self.life(l);
                return self.broadcast(host, event);
            }
            Event::Msg(_) => return self.broadcast(host, event),
            Event::Frame(_) => return 0, // frames enter through `App::frame`
        };
        match MODULES.get(tag_mod(owner)) {
            Some(module) => (module.event)(self, host, event),
            None => 0,
        }
    }

    fn broadcast(&mut self, host: &mut dyn Host, event: &Event) -> u32 {
        MODULES.iter().fold(0, |bits, module| bits | (module.event)(self, host, event))
    }

    fn life(&mut self, l: abi::Life) {
        let mut flag = |bit: u32, on: bool| self.cfg.flags = if on { self.cfg.flags | bit } else { self.cfg.flags & !bit };
        match l.code {
            life::VISIBILITY => flag(mode::HIDDEN, l.a != 0.0),
            life::PAGEHIDE => flag(mode::HIDDEN, true),
            life::PAGESHOW => flag(mode::HIDDEN, false),
            life::ONLINE => flag(mode::ONLINE, true),
            life::OFFLINE => flag(mode::ONLINE, false),
            life::SCHEME => flag(mode::DARK_SCHEME, l.a != 0.0),
            life::MOTION => flag(mode::REDUCED_MOTION, l.a != 0.0),
            life::RESIZE => {
                self.cfg.viewport = (l.a, l.b);
                self.cfg.dpr = l.c;
            }
            _ => {}
        }
        let hidden = self.cfg.has(mode::HIDDEN);
        if hidden != self.sched.hidden {
            self.sched.hidden = hidden;
            self.sched.rearm |= hidden;
        }
    }

    fn run_work(&mut self, host: &mut dyn Host) -> u32 {
        let mut due = Vec::new();
        self.sched.armed = None;
        let delay = self.sched.poll(host.now(), &mut due);
        for (id, stale) in due {
            if let Some(module) = MODULES.get(tag_mod(id)) {
                (module.event)(self, host, &Event::Work { id, stale });
            }
        }
        // Work delivered above may have queued more; `after` re-arms in that case.
        self.sched.rearm |= delay.is_some();
        0
    }

    /// Bookkeeping after every entry from the host: (re)arm the deferred-work poll and report readiness.
    fn after(&mut self, host: &mut dyn Host) {
        if std::mem::take(&mut self.sched.rearm) && !self.sched.jobs.is_empty() {
            let now = host.now();
            if let Some(delay) = self.sched.next_delay(now) {
                self.sched.arm(host.out(), now, delay);
            }
        }
        if self.booted && !self.ready && self.ready_holds == 0 {
            self.ready = true;
            host.out().fx0(fx::READY);
        }
    }
}

/// `ihn_worker`: one message for the export-worker instance; the first byte selects the role.
pub fn worker(host: &mut dyn Host, msg: &[u8]) {
    match msg.first() {
        Some(&worker_role::IMPORT) => doc_v5open::worker(host, &msg[1..]),
        Some(&worker_role::EXPORT) => export_pdf::worker(host, &msg[1..]),
        _ => host.log(3, "ihn: unknown worker role"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::FakeHost;
    use ihn_core::codec::Writer;

    pub(crate) fn config() -> Config {
        Config {
            version: "6.0.0".into(),
            url: "http://127.0.0.1:4300/?e2e=1".into(),
            user_agent: "test".into(),
            language: "en".into(),
            flags: mode::E2E | mode::ONLINE,
            viewport: (1280.0, 800.0),
            dpr: 1.0,
            max_touch_points: 0,
        }
    }

    fn timer(app: &mut App, host: &mut FakeHost, id: u32) {
        let mut w = Writer::default();
        w.u32(id);
        app.event(host, abi::kind::TIMER, &w.buf);
    }

    #[test]
    fn module_table_matches_the_mod_enum() {
        assert!(MODULES.len() <= 64, "frame and busy sets are u64 bitsets");
        assert_eq!(MODULES[Mod::App as usize].name, "app");
        assert_eq!(MODULES[Mod::DriveClient as usize].name, "drive.client");
        assert_eq!(MODULES[Mod::HomeDialogs as usize].name, "home.dialogs");
        assert_eq!(MODULES[Mod::Shell as usize].name, "shell");
        assert_eq!(Mod::Shell as usize, MODULES.len() - 1, "the shell initialises last");
        let mut names: Vec<_> = MODULES.iter().map(|m| m.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), MODULES.len());
    }

    /// The DOM-op golden test of the boot sequence: bind every static node, write both version labels, ready.
    #[test]
    fn boot_binds_static_nodes_writes_version_labels_and_reports_ready() {
        let (mut app, mut host) = (App::new(config()), FakeHost::default());
        app.init(&mut host);
        let text = host.take();
        let lines: Vec<&str> = text.lines().collect();
        for (i, id) in Ui::IDS.iter().enumerate() {
            assert_eq!(lines[i], format!("bind {} {id:?}", Ui::FIRST + i as u32));
        }
        let golden = format!(
            "query 1024 {home} \"[data-app-version]\"\ntext 1024 \"v6.0.0\"\nrelease 1024\n\
             query 1024 {editor} \"[data-app-version]\"\ntext 1024 \"v6.0.0\"\nrelease 1024\nready\n",
            home = Ui::DRIVE_HOME.0,
            editor = Ui::EDITOR_VIEW.0
        );
        // The modules' own boot records (shell, auth, …) follow the version labels; `ready` is always last.
        assert_eq!(lines[Ui::IDS.len()..Ui::IDS.len() + 6].join("\n") + "\nready\n", golden);
        assert_eq!(lines.last(), Some(&"ready"));
        assert!(app.is_ready());
        // Nothing more is written for an event nobody handles.
        timer(&mut app, &mut host, tag(Mod::Shell, 1));
        assert_eq!(host.take(), "");
    }

    #[test]
    fn ready_waits_for_holds() {
        let (mut app, mut host) = (App::new(config()), FakeHost::default());
        app.hold_ready();
        app.init(&mut host);
        assert!(!host.take().lines().any(|l| l == "ready"));
        app.release_ready();
        timer(&mut app, &mut host, tag(Mod::Shell, 1));
        assert_eq!(host.take(), "ready\n");
        timer(&mut app, &mut host, tag(Mod::Shell, 1));
        assert_eq!(host.take(), "");
    }

    #[test]
    fn scheduler_records() {
        let (mut s, mut out) = (Sched::default(), OutBuf::default());
        s.timer(&mut out, 7, 250.0);
        s.interval(&mut out, 8, 30000.0);
        s.clear(&mut out, 7);
        s.frame(&mut out, Mod::EditorInput);
        s.frame(&mut out, Mod::Home);
        s.idle(&mut out, 9, 2000.0);
        s.idle_cancel(&mut out, 9);
        assert_eq!(
            abi::dump(&out.w.buf),
            "timer_set 7 250 0\ntimer_set 8 30000 1\ntimer_clear 7\nraf_request\nidle_request 9 2000\nidle_cancel 9\n"
        );
        assert_eq!(s.frame_mods, 1 << Mod::EditorInput as u32 | 1 << Mod::Home as u32);
    }

    // v5: rust-core/src/document_work.rs test `held_pen_stale_sessions_and_bounded_resume`
    #[test]
    fn held_pen_stale_sessions_and_bounded_resume() {
        let mut s = Sched::default();
        s.session = 3;
        for id in 1..=20 {
            assert!(s.defer(id, 1200.0, true));
        }
        assert!(!s.defer(1, 1200.0, true) && !s.defer(99, -1.0, true) && !s.defer(99, 60001.0, true));
        let mut due = Vec::new();
        s.set_busy(Mod::EditorInput, true);
        assert_eq!((s.poll(5000.0, &mut due), due.len()), (None, 0)); // v5 re-polled after 100 ms; v6 waits for set_busy(false)
        s.set_busy(Mod::EditorInput, false);
        s.activity(4900.0);
        assert_eq!((s.poll(5000.0, &mut due), due.len()), (Some(1100.0), 0));
        assert_eq!(s.poll(6100.0, &mut due), Some(1.0));
        assert_eq!(due, (1..=8).map(|id| (id, false)).collect::<Vec<_>>());
        due.clear();
        s.new_session();
        s.set_busy(Mod::EditorInput, true);
        s.poll(6101.0, &mut due);
        assert_eq!(due, (9..=16).map(|id| (id, true)).collect::<Vec<_>>());
        due.clear();
        s.session = 3;
        s.hidden = true;
        assert_eq!(s.poll(6102.0, &mut due), None);
        assert_eq!(due, (17..=20).map(|id| (id, false)).collect::<Vec<_>>());
        // Unbound jobs survive session changes; cancel removes.
        assert!(s.defer(50, 0.0, false) && s.defer(51, 0.0, false));
        s.cancel(51);
        s.new_session();
        due.clear();
        s.hidden = false;
        s.set_busy(Mod::EditorInput, false);
        assert_eq!((s.poll(9000.0, &mut due), &due[..]), (None, &[(50, false)][..]));
    }

    #[test]
    fn deferred_work_is_polled_by_the_scheduler_timer() {
        let (mut app, mut host) = (App::new(config()), FakeHost::default());
        app.init(&mut host);
        host.take();
        host.now = 1000.0;
        app.sched.activity(1000.0);
        assert!(app.sched.defer(tag(Mod::Store, 1), 180.0, false));
        timer(&mut app, &mut host, tag(Mod::Shell, 1)); // any entry re-arms
        assert_eq!(host.take(), format!("timer_set {WORK_TIMER} 180 0\n"));
        host.now = 1100.0;
        timer(&mut app, &mut host, WORK_TIMER);
        assert_eq!(host.take(), format!("timer_set {WORK_TIMER} 80 0\n"));
        host.now = 1180.0;
        timer(&mut app, &mut host, WORK_TIMER);
        assert_eq!(host.take(), "");
        assert!(app.sched.jobs.is_empty());
    }

    fn pointer(app: &mut App, host: &mut FakeHost, kind: u8, ptype: u8, t: f64, pressure: f32, buttons: u32) {
        let mut w = Writer::default();
        w.u32(tag(Mod::EditorInput, 1));
        w.u32(30);
        w.u8(kind);
        w.u8(ptype);
        w.i32(1);
        w.i32(0);
        w.u32(0);
        w.f64(10.0);
        w.f64(10.0);
        w.f64(t);
        w.f32(pressure);
        w.f32(0.0);
        w.f32(0.0);
        w.u32(buttons);
        w.u32(0);
        w.u32(0);
        app.event(host, abi::kind::POINTER, &w.buf);
    }

    /// ARCHITECTURE §0.6 / §14: a pen stream writes nothing to the out-buffer, however much work is queued.
    #[test]
    fn pointer_samples_never_touch_the_work_timer() {
        let (mut app, mut host) = (App::new(config()), FakeHost::default());
        app.init(&mut host);
        host.take();
        host.now = 1000.0;
        app.sched.activity(1000.0);
        assert!(app.sched.defer(tag(Mod::Store, 1), 180.0, false));
        timer(&mut app, &mut host, tag(Mod::Shell, 1));
        assert_eq!(host.take(), format!("timer_set {WORK_TIMER} 180 0\n"));

        // Pen down: the editor is busy for the whole stroke.
        pointer(&mut app, &mut host, ptr::DOWN, ptr::PEN, 1010.0, 0.5, 1);
        app.sched.set_busy(Mod::EditorInput, true);
        for i in 0..50 {
            pointer(&mut app, &mut host, ptr::MOVE, ptr::PEN, 1014.0 + 4.0 * i as f64, 0.5, 1);
            pointer(&mut app, &mut host, ptr::RAW_UPDATE, ptr::PEN, 1015.0 + 4.0 * i as f64, 0.5, 1);
        }
        assert_eq!(host.take(), "");
        assert_eq!(app.sched.last_activity, 1211.0);
        // The timer armed before the stroke fires once and is not renewed while the pen is down.
        host.now = 1180.0;
        timer(&mut app, &mut host, WORK_TIMER);
        assert_eq!(host.take(), "");
        // Pen up: the poll is armed for the quiet time that is left.
        host.now = 1300.0;
        pointer(&mut app, &mut host, ptr::UP, ptr::PEN, 1300.0, 0.0, 0);
        app.sched.set_busy(Mod::EditorInput, false);
        timer(&mut app, &mut host, tag(Mod::Shell, 1));
        assert_eq!(host.take(), format!("timer_set {WORK_TIMER} 91 0\n"));
        // Later entries do not set it again, and an earlier deadline replaces it.
        timer(&mut app, &mut host, tag(Mod::Shell, 1));
        assert_eq!(host.take(), "");
        assert!(app.sched.defer(tag(Mod::Store, 2), 0.0, false));
        timer(&mut app, &mut host, tag(Mod::Shell, 1));
        assert_eq!(host.take(), format!("timer_set {WORK_TIMER} 1 0\n"));
        host.now = 1391.0;
        timer(&mut app, &mut host, WORK_TIMER);
        assert_eq!(host.take(), "");
        assert!(app.sched.jobs.is_empty() && app.sched.armed.is_none());
    }

    // v5: app-v5.js § init onMoveInterrupt — a pointermove without buttons, pressure or touch is not activity.
    #[test]
    fn only_contact_pointer_events_count_as_activity() {
        let (mut app, mut host) = (App::new(config()), FakeHost::default());
        app.init(&mut host);
        let mut last = |kind, ptype, t, pressure, buttons| {
            pointer(&mut app, &mut host, kind, ptype, t, pressure, buttons);
            app.sched.last_activity
        };
        assert_eq!(last(ptr::MOVE, ptr::MOUSE, 100.0, 0.0, 0), 0.0);
        assert_eq!(last(ptr::MOVE, ptr::PEN, 110.0, 0.0, 0), 0.0); // hovering pen
        assert_eq!(last(ptr::ENTER, ptr::MOUSE, 120.0, 0.0, 0), 0.0);
        assert_eq!(last(ptr::UP, ptr::MOUSE, 130.0, 0.0, 0), 0.0);
        assert_eq!(last(ptr::MOVE, ptr::MOUSE, 140.0, 0.5, 1), 140.0);
        assert_eq!(last(ptr::MOVE, ptr::PEN, 150.0, 0.2, 0), 150.0);
        assert_eq!(last(ptr::MOVE, ptr::TOUCH, 160.0, 0.0, 0), 160.0);
        assert_eq!(last(ptr::DOWN, ptr::MOUSE, 170.0, 0.0, 0), 170.0);
        assert_eq!(last(ptr::MOVE, ptr::MOUSE, 165.0, 0.5, 1), 170.0); // never moves backwards
    }

    #[test]
    fn lifecycle_updates_config() {
        let (mut app, mut host) = (App::new(config()), FakeHost::default());
        app.init(&mut host);
        let life = |app: &mut App, host: &mut FakeHost, code: u8, a: f64, b: f64, c: f64| {
            let mut w = Writer::default();
            w.u8(code);
            w.f64(a);
            w.f64(b);
            w.f64(c);
            w.str("");
            app.event(host, abi::kind::LIFECYCLE, &w.buf)
        };
        life(&mut app, &mut host, life::RESIZE, 360.0, 800.0, 3.0);
        assert_eq!((app.cfg.viewport, app.cfg.dpr), ((360.0, 800.0), 3.0));
        life(&mut app, &mut host, life::OFFLINE, 0.0, 0.0, 0.0);
        assert!(!app.cfg.has(mode::ONLINE));
        life(&mut app, &mut host, life::MOTION, 1.0, 0.0, 0.0);
        assert!(app.cfg.has(mode::REDUCED_MOTION));
        life(&mut app, &mut host, life::MOTION, 0.0, 0.0, 0.0);
        assert!(!app.cfg.has(mode::REDUCED_MOTION));
        life(&mut app, &mut host, life::VISIBILITY, 1.0, 0.0, 0.0);
        assert!(app.cfg.has(mode::HIDDEN) && app.sched.hidden);
        life(&mut app, &mut host, life::VISIBILITY, 0.0, 0.0, 0.0);
        assert!(!app.sched.hidden);
        assert_eq!(app.event(&mut host, 77, &[]), 0);
        assert_eq!(host.logs.len(), 1);
    }
}
