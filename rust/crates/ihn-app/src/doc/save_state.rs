//! Status mapping (ARCHITECTURE §6.6): one presentation derived from the sync status and the local commit state —
//! the save indicator, its labels, the Home badge text and the save-details popup. Owned by W3.1.
//!
//! The indicator follows 5.18.0 `refreshSaveIndicatorState`: whatever `shell::status` painted, the class, the
//! trigger label and the short label are re-derived here after every `sig::STATUS` and after every change of an
//! input ([`session::sig::STATE`], network, auth, view).
// v5: app-v5.js:getSavePresentation / refreshSaveIndicatorState / renderSaveDebugPopup

use super::session::{self, Local};
use crate::abi::{ptr, ret, tag, Event, Node};
use crate::app::Mod;
use crate::dom::Prop;
use crate::testapi::json_str;
use crate::ui_ids::Ui;
use crate::{auth, shell, App, Host};
use ihn_core::doc::{ExportPhase, SyncStatus};

/// The save indicator's states (5.18.0 `SAVE_STATES`).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Phase {
    Error,
    Saving,
    Building,
    Uploading,
    Syncing,
    Paused,
    #[default]
    Saved,
}

impl Phase {
    pub const ALL: [Phase; 7] = [Phase::Paused, Phase::Building, Phase::Uploading, Phase::Syncing, Phase::Saving, Phase::Saved, Phase::Error];
    /// The class on `#save-indicator`.
    pub fn class(self) -> &'static str {
        match self {
            Phase::Error => "error",
            Phase::Saving => "saving",
            Phase::Building => "building",
            Phase::Uploading => "uploading",
            Phase::Syncing => "syncing",
            Phase::Paused => "paused",
            Phase::Saved => "saved",
        }
    }
    /// SYNC §10.5 `Presence.save_phase` (remote save rings use the same class names).
    pub fn code(self) -> u8 { self as u8 }
}

/// Everything the mapping reads.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Inputs {
    pub local: Local,
    pub status: SyncStatus,
    /// Catch-up reads after open or reconnect are running (SYNC §8.8).
    pub catching_up: bool,
    /// A writable Drive document is open (5.18.0 `driveAutosave && driveFileId`).
    pub connected: bool,
}

/// The §6.6 table, first match wins: the indicator phase and the Home badge `(text, data-phase)`.
pub fn derive(i: &Inputs) -> (Phase, Option<(&'static str, &'static str)>) {
    let s = &i.status;
    let exporting = !matches!(s.export, ExportPhase::Idle | ExportPhase::Failed { .. });
    let unsynced = s.unsealed_local > 0 || s.not_created;
    if i.local.failed || (s.last_error.is_some() && !s.relay_writing && !exporting) {
        return (Phase::Error, Some(("Retry upload", "error")));
    }
    if i.local.pending {
        return (Phase::Saving, None);
    }
    match s.export {
        ExportPhase::Building => return (Phase::Building, Some(("Preparing…", "building"))),
        ExportPhase::Checking => return (Phase::Uploading, Some(("Checking…", "checking"))),
        ExportPhase::Merging => return (Phase::Uploading, Some(("Merging…", "merging"))),
        ExportPhase::Preserving => return (Phase::Uploading, Some(("Keeping both versions…", "preserving"))),
        ExportPhase::Uploading => return (Phase::Uploading, Some(("Uploading…", "uploading"))),
        ExportPhase::Idle | ExportPhase::Failed { .. } => {}
    }
    if s.relay_writing && unsynced {
        return (Phase::Uploading, Some(("Uploading…", "uploading")));
    }
    if i.catching_up {
        return (Phase::Syncing, Some(("Syncing…", "syncing")));
    }
    if unsynced {
        let offline = !s.online && !s.auth_paused;
        return (Phase::Paused, Some((if offline { "Saved offline" } else { "Waiting to upload" }, if offline { "offline" } else { "queued" })));
    }
    (Phase::Saved, None)
}

/// The texts of the indicator and of the save-details popup.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct View {
    pub phase: Phase,
    pub title: &'static str,
    pub short_label: &'static str,
    pub local_label: &'static str,
    pub local_class: &'static str,
    pub drive_label: &'static str,
    pub drive_class: &'static str,
    pub notice: String,
    /// The popup's button, or "" when it is hidden.
    pub action: &'static str,
    pub action_disabled: bool,
}

// v5: app-v5.js:getSavePresentation — the Drive lines that named PDF build/upload phases name the export phase,
// and "Strokes saved" means "sealed in the relay" (BEHAVIOUR-CHANGES N-2).
pub fn present(i: &Inputs) -> View {
    let (phase, _) = derive(i);
    let (s, local) = (&i.status, i.local);
    let (offline, reconnect) = (!s.online, i.connected && s.auth_paused);
    let unsynced = s.unsealed_local > 0 || s.not_created;
    let exporting = !matches!(s.export, ExportPhase::Idle | ExportPhase::Failed { .. });
    let title = match phase {
        Phase::Error => "Save needs attention",
        Phase::Saving => "Saving on this device",
        Phase::Building => "Preparing Drive copy",
        Phase::Uploading => match s.export {
            ExportPhase::Checking => "Checking Google Drive",
            ExportPhase::Merging | ExportPhase::Preserving => "Preparing Drive copy",
            _ => "Saving to Google Drive",
        },
        Phase::Syncing => "Receiving changes",
        Phase::Paused if reconnect => "Reconnect to Google Drive",
        Phase::Paused if offline => "Waiting for connection",
        Phase::Paused => "Waiting to save to Drive",
        Phase::Saved => match (i.connected, local.saved_at > 0.0) {
            (true, true) => "All changes saved",
            (true, false) => "Saved in Google Drive",
            (false, true) => "Saved on this device",
            (false, false) => "No pending changes",
        },
    };
    let short_label = match phase {
        Phase::Error => "Check save",
        Phase::Saving => "Saving",
        Phase::Building | Phase::Uploading | Phase::Syncing => "Syncing",
        Phase::Paused if reconnect => "Sign in",
        Phase::Paused if offline => "Offline",
        Phase::Paused => "Pending",
        Phase::Saved if i.connected => "Saved",
        Phase::Saved => "Local",
    };
    let (local_label, local_class) = match () {
        _ if local.failed => ("Needs attention", "error"),
        _ if local.pending => ("Saving", "syncing"),
        _ if local.saved_at > 0.0 => ("Saved", ""),
        _ => ("No pending changes", ""),
    };
    let (drive_label, drive_class) = match () {
        _ if !i.connected => ("Not connected", "idle"),
        _ if reconnect => ("Sign in needed", "active"),
        _ if offline && unsynced => ("Offline", "active"),
        _ if exporting => (
            match s.export {
                ExportPhase::Checking => "Checking Drive",
                ExportPhase::Building => "Preparing",
                ExportPhase::Merging => "Merging changes",
                ExportPhase::Preserving => "Keeping both versions",
                _ => "Uploading",
            },
            "syncing",
        ),
        _ if s.relay_writing && unsynced => ("Uploading", "syncing"),
        _ if unsynced && s.last_error.is_some() => ("Upload failed", "active"),
        _ if unsynced => ("Waiting", "active"),
        _ if i.catching_up => ("Receiving changes", "syncing"),
        _ if s.export_pending => ("Strokes saved", ""),
        _ => ("Saved", ""),
    };
    let notice = match &s.last_error {
        _ if local.failed => "Keep this document open and retry.".to_string(),
        Some(e) if unsynced => e.detail.clone(),
        _ => String::new(),
    };
    let running = local.pending || s.relay_writing || exporting;
    let actionable = !running && (local.failed || (!offline && (unsynced || reconnect)));
    let action = match () {
        _ if !actionable => "",
        _ if local.failed || phase == Phase::Error => "Retry save",
        _ if reconnect => "Reconnect Drive",
        _ => "Save now",
    };
    View { phase, title, short_label, local_label, local_class, drive_label, drive_class, notice, action, action_disabled: running || (!local.failed && offline) }
}

/// `sig` sent when the popup's button asks for a flush (the sync binding and the export follow it).
pub const SIG_SAVE_NOW: u8 = 0x46;

const L_TRIGGER: u32 = tag(Mod::DocSaveState, 1);
const L_CLOSE: u32 = tag(Mod::DocSaveState, 2);
const L_ACTION: u32 = tag(Mod::DocSaveState, 3);
const L_DOWN: u32 = tag(Mod::DocSaveState, 4);
const L_KEY: u32 = tag(Mod::DocSaveState, 5);
const T_RENDER: u32 = tag(Mod::DocSaveState, 6);

/// This module's state, stored in `App::st`.
#[derive(Default)]
pub struct State {
    /// The class and the `title` the indicator carries now (`shell::status` paints them too).
    class: Option<&'static str>,
    title: String,
    trigger: Node,
    /// What the trigger's labels show.
    shown: Option<View>,
    current: View,
    catching_up: bool,
    backdrop: Node,
    panel: Node,
    summary: Node,
    notice: Node,
    action: Node,
    close: Node,
    open: bool,
    popup: Option<View>,
}

/// The inputs as they are now.
pub fn inputs(app: &App) -> Inputs {
    let mut status = session::sync_status(app);
    if session::get(app).is_none() {
        status.online = !shell::offline(app);
        status.auth_paused = auth::reconnecting(app);
    }
    Inputs { local: session::local(app), status, catching_up: app.st.doc_save_state.catching_up, connected: session::get(app).is_some_and(|s| s.writable()) }
}

/// The current phase (presence `save_phase`, Home's active-job badge).
pub fn phase(app: &App) -> Phase { derive(&inputs(app)).0 }

/// Catch-up reads started or ended (the sync binding reports them).
pub fn set_catching_up(app: &mut App, host: &mut dyn Host, on: bool) {
    if std::mem::replace(&mut app.st.doc_save_state.catching_up, on) != on {
        refresh(app, host);
    }
}

/// Re-derives the presentation and writes what changed.
// v5: app-v5.js:refreshSaveIndicatorState
pub fn refresh(app: &mut App, host: &mut dyn Host) {
    let view = present(&inputs(app));
    let st = &mut app.st.doc_save_state;
    let out = host.out();
    if st.class.unwrap_or("saved") != view.phase.class() {
        for p in Phase::ALL {
            out.class(Ui::SAVE_INDICATOR, p.class(), p == view.phase);
        }
        st.class = Some(view.phase.class());
    }
    // The trigger exists once the editor was shown; until then its labels stay as the markup has them.
    if !st.trigger.is_none() {
        let old = st.shown.as_ref();
        if old.map(|o| o.title) != Some(view.title) {
            let label = format!("{}. Open save details", view.title);
            out.attr(st.trigger, "aria-label", Some(&label));
            out.prop(st.trigger, "title", Prop::Str(&label));
        }
        if old.map(|o| o.short_label) != Some(view.short_label) {
            out.text(Ui::SAVE_STATUS_LABEL, view.short_label);
        }
        st.shown = Some(view.clone());
    }
    if st.title != view.title {
        out.prop(Ui::SAVE_INDICATOR, "title", Prop::Str(view.title));
        st.title = view.title.into();
    }
    st.current = view;
    render_popup(app, host);
}

fn escape(s: &str) -> String { s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;") }

// v5: app-v5.js:renderSaveDebugPopup — a closed panel costs nothing.
fn render_popup(app: &mut App, host: &mut dyn Host) {
    let st = &mut app.st.doc_save_state;
    if !st.open {
        return;
    }
    let view = st.current.clone();
    let out = host.out();
    let old = st.popup.as_ref();
    if old.map(|o| (o.local_label, o.local_class, o.drive_label, o.drive_class)) != Some((view.local_label, view.local_class, view.drive_label, view.drive_class)) {
        let row = |name: &str, label: &str, style: &str| {
            format!("\n                <div class=\"save-device-status\">\n                    <span class=\"save-device-status-title\">{}</span>\n                    <span class=\"save-device-status-label {style}\">{}</span>\n                </div>", escape(name), escape(label))
        };
        out.html(st.summary, &(row("This device", view.local_label, view.local_class) + &row("Google Drive", view.drive_label, view.drive_class)));
    }
    if old.map(|o| o.action.is_empty()) != Some(view.action.is_empty()) {
        out.prop(st.action, "hidden", Prop::Bool(view.action.is_empty()));
    }
    if old.map(|o| o.action_disabled) != Some(view.action_disabled) {
        out.prop(st.action, "disabled", Prop::Bool(view.action_disabled));
    }
    let label = if view.action.is_empty() { "Save now" } else { view.action };
    if old.map(|o| if o.action.is_empty() { "Save now" } else { o.action }) != Some(label) {
        out.text(st.action, label);
    }
    if old.map(|o| &o.notice) != Some(&view.notice) {
        out.text(st.notice, &view.notice);
        out.prop(st.notice, "hidden", Prop::Bool(view.notice.is_empty()));
    }
    st.popup = Some(view);
}

// v5: app-v5.js:ensureSaveDebugPopup / openSaveDebugPopup
fn open_popup(app: &mut App, host: &mut dyn Host) {
    let st = &mut app.st.doc_save_state;
    let out = host.out();
    if st.backdrop.is_none() {
        st.backdrop = out.insert_html(Node::BODY, Node::NONE, "<div class=\"save-debug-backdrop\">\n                <div class=\"save-debug-panel\" id=\"save-details-panel\" role=\"dialog\" aria-modal=\"false\" aria-labelledby=\"save-debug-title\">\n                    <div class=\"save-debug-header\">\n                        <h2 class=\"save-debug-title\" id=\"save-debug-title\">Save status</h2>\n                        <button type=\"button\" class=\"save-debug-close\" aria-label=\"Close save details\">&times;</button>\n                    </div>\n                    <div class=\"save-debug-summary\" role=\"status\" aria-live=\"polite\" aria-label=\"Saved copies\"></div>\n                    <p class=\"save-debug-notice\" role=\"status\" hidden></p>\n                    <button type=\"button\" class=\"btn btn-secondary save-debug-action\" hidden>Save now</button>\n                </div></div>");
        st.panel = out.query(st.backdrop, ".save-debug-panel");
        st.close = out.query(st.backdrop, ".save-debug-close");
        st.summary = out.query(st.backdrop, ".save-debug-summary");
        st.notice = out.query(st.backdrop, ".save-debug-notice");
        st.action = out.query(st.backdrop, ".save-debug-action");
        out.listen(st.close, "click", L_CLOSE, 0);
        out.listen(st.action, "click", L_ACTION, 0);
        out.listen(Node::DOCUMENT, "pointerdown", L_DOWN, 0);
        out.listen(Node::DOCUMENT, "keydown", L_KEY, 0);
        // The first render compares against the markup: button hidden and enabled, no notice, no rows.
        st.popup = Some(View::default());
    }
    out.class(st.backdrop, "visible", true);
    out.attr(st.trigger, "aria-expanded", Some("true"));
    st.open = true;
    render_popup(app, host);
    let st = &mut app.st.doc_save_state;
    host.out().focus(st.close, true);
    app.sched.interval(host.out(), T_RENDER, 1000.0);
}

// v5: app-v5.js:closeSaveDebugPopup
fn close_popup(app: &mut App, host: &mut dyn Host, restore_focus: bool) {
    let st = &mut app.st.doc_save_state;
    if st.backdrop.is_none() {
        return;
    }
    let out = host.out();
    out.class(st.backdrop, "visible", false);
    out.attr(st.trigger, "aria-expanded", Some("false"));
    if restore_focus {
        out.focus(st.trigger, true);
    }
    if std::mem::take(&mut st.open) {
        app.sched.clear(out, T_RENDER);
    }
}

/// Called once at boot, in `app::MODULES` order, after the `Ui` table is bound.
pub fn init(_app: &mut App, _host: &mut dyn Host) {}

// v5: initSaveIndicatorDetails — bound when the editor is first shown.
fn bind_trigger(app: &mut App, host: &mut dyn Host) {
    if app.st.doc_save_state.trigger.is_none() {
        let out = host.out();
        let trigger = out.query(Ui::EDITOR_VIEW, ".mode-toggle-status");
        out.attr(Ui::SAVE_INDICATOR, "data-debug-bound", Some("true"));
        out.listen(trigger, "click", L_TRIGGER, 0);
        app.st.doc_save_state.trigger = trigger;
    }
}

/// Events addressed to this module (ids made with `abi::tag`) and broadcasts (`Life`, `Msg`). Returns `abi::ret` bits.
pub fn event(app: &mut App, host: &mut dyn Host, event: &Event) -> u32 {
    match *event {
        Event::Dom(e) if e.listener == L_TRIGGER => {
            if app.st.doc_save_state.open {
                close_popup(app, host, true);
            } else {
                refresh(app, host);
                open_popup(app, host);
            }
            return ret::PREVENT_DEFAULT | ret::STOP_PROPAGATION;
        }
        Event::Dom(e) if e.listener == L_CLOSE => close_popup(app, host, true),
        Event::Dom(e) if e.listener == L_KEY && e.key == "Escape" && app.st.doc_save_state.open => {
            close_popup(app, host, true);
            return ret::PREVENT_DEFAULT;
        }
        Event::Dom(e) if e.listener == L_ACTION => {
            let view = app.st.doc_save_state.current.clone();
            if view.action == "Reconnect Drive" {
                auth::sign_in(app, host);
            } else {
                shell::signal(app, host, SIG_SAVE_NOW, "");
            }
            refresh(app, host);
        }
        Event::Pointer(p) if p.listener == L_DOWN && p.kind == ptr::DOWN => {
            let st = &app.st.doc_save_state;
            if st.open && !p.path.contains(st.panel) && !p.path.contains(st.trigger) {
                close_popup(app, host, false);
            }
        }
        Event::Timer(T_RENDER) => refresh(app, host),
        Event::Msg(m) => match m.chan {
            shell::sig::VIEW => {
                if m.data == b"editor" {
                    bind_trigger(app, host);
                    refresh(app, host);
                } else {
                    close_popup(app, host, false);
                }
            }
            // v5: showStatus paints the class and the tooltip, then refreshSaveIndicatorState corrects them.
            shell::sig::STATUS => {
                if let Some(msg) = shell::StatusMsg::parse(m.data) {
                    let st = &mut app.st.doc_save_state;
                    if !msg.preserve_state {
                        st.class = Phase::ALL.iter().map(|p| p.class()).find(|c| *c == msg.semantic_state());
                    }
                    st.title = msg.text.into();
                }
                refresh(app, host);
            }
            shell::sig::NETWORK | shell::sig::AUTH | shell::sig::RECONNECT if shell::view(app) == shell::View::Editor => refresh(app, host),
            session::sig::STATE | session::sig::OPENED | session::sig::CLOSED | session::sig::COMMITTED => refresh(app, host),
            _ => {}
        },
        _ => {}
    }
    0
}

/// `__IHN_TEST_API__.cmd('doc.save_state.<name>', args)`: `state` → `{phase, title, shortLabel, local, drive,
/// badge, open}`.
pub fn test_cmd(app: &mut App, _host: &mut dyn Host, name: &str, _args: &str) -> Option<String> {
    (name == "state").then(|| {
        let i = inputs(app);
        let (v, badge) = (present(&i), derive(&i).1);
        format!(
            "{{\"phase\":{},\"title\":{},\"shortLabel\":{},\"local\":{},\"drive\":{},\"badge\":{},\"open\":{}}}",
            json_str(v.phase.class()), json_str(v.title), json_str(v.short_label), json_str(v.local_label), json_str(v.drive_label), json_str(badge.map_or("", |b| b.0)), app.st.doc_save_state.open
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ihn_core::doc::SyncError;

    fn base() -> Inputs { Inputs { status: SyncStatus { online: true, ..SyncStatus::default() }, connected: true, ..Inputs::default() } }
    fn with(f: impl FnOnce(&mut Inputs)) -> (Phase, Option<&'static str>) {
        let mut i = base();
        f(&mut i);
        let (phase, badge) = derive(&i);
        (phase, badge.map(|b| b.0))
    }

    #[test]
    fn status_mapping_follows_the_table_in_order() {
        // One row at a time.
        assert_eq!(with(|_| {}), (Phase::Saved, None));
        assert_eq!(with(|i| i.status.export_pending = true), (Phase::Saved, None), "an export may still be pending");
        assert_eq!(with(|i| i.status.unsealed_local = 2), (Phase::Paused, Some("Waiting to upload")));
        assert_eq!(with(|i| i.status.not_created = true), (Phase::Paused, Some("Waiting to upload")));
        assert_eq!(with(|i| (i.status.unsealed_local, i.status.online) = (1, false)), (Phase::Paused, Some("Saved offline")));
        assert_eq!(with(|i| (i.status.unsealed_local, i.status.auth_paused) = (1, true)), (Phase::Paused, Some("Waiting to upload")));
        assert_eq!(with(|i| i.catching_up = true), (Phase::Syncing, Some("Syncing…")));
        assert_eq!(with(|i| (i.status.unsealed_local, i.status.relay_writing) = (1, true)), (Phase::Uploading, Some("Uploading…")));
        assert_eq!(with(|i| i.status.relay_writing = true), (Phase::Saved, None), "a relay write without own batches");
        assert_eq!(with(|i| i.status.export = ExportPhase::Uploading), (Phase::Uploading, Some("Uploading…")));
        assert_eq!(with(|i| i.status.export = ExportPhase::Checking), (Phase::Uploading, Some("Checking…")));
        assert_eq!(with(|i| i.status.export = ExportPhase::Merging), (Phase::Uploading, Some("Merging…")));
        assert_eq!(with(|i| i.status.export = ExportPhase::Preserving), (Phase::Uploading, Some("Keeping both versions…")));
        assert_eq!(with(|i| i.status.export = ExportPhase::Building), (Phase::Building, Some("Preparing…")));
        assert_eq!(with(|i| i.local.pending = true), (Phase::Saving, None));
        assert_eq!(with(|i| i.local.failed = true), (Phase::Error, Some("Retry upload")));
        let error = || Some(SyncError { phase: "relay".into(), code: "403".into(), detail: "Drive refused the write.".into() });
        assert_eq!(with(|i| i.status.last_error = error()), (Phase::Error, Some("Retry upload")));
        // Precedence: each row wins over every row below it.
        assert_eq!(with(|i| (i.status.last_error, i.status.relay_writing, i.status.unsealed_local) = (error(), true, 1)).0, Phase::Uploading, "an error with a retry running");
        assert_eq!(with(|i| (i.status.last_error, i.status.export) = (error(), ExportPhase::Building)).0, Phase::Building);
        assert_eq!(with(|i| (i.local.failed, i.local.pending, i.status.export) = (true, true, ExportPhase::Building)).0, Phase::Error);
        assert_eq!(with(|i| (i.local.pending, i.status.export, i.catching_up) = (true, ExportPhase::Building, true)).0, Phase::Saving);
        assert_eq!(with(|i| (i.status.export, i.status.relay_writing, i.status.unsealed_local) = (ExportPhase::Building, true, 1)).0, Phase::Building);
        assert_eq!(with(|i| (i.status.export, i.catching_up) = (ExportPhase::Checking, true)).0, Phase::Uploading);
        assert_eq!(with(|i| (i.catching_up, i.status.unsealed_local, i.status.online) = (true, 1, false)).0, Phase::Syncing);
        assert_eq!(with(|i| (i.status.export, i.status.unsealed_local) = (ExportPhase::Failed { code: "x".into(), detail: String::new() }, 1)).0, Phase::Paused);
        // Classes are 5.18.0's SAVE_STATES.
        let mut classes: Vec<&str> = Phase::ALL.iter().map(|p| p.class()).collect();
        ihn_core::sort::ord(&mut classes);
        assert_eq!(classes, ["building", "error", "paused", "saved", "saving", "syncing", "uploading"]);
    }

    #[test]
    fn presentation_texts_are_5_18_0s() {
        let v = present(&base());
        assert_eq!((v.title, v.short_label, v.local_label, v.drive_label, v.action), ("Saved in Google Drive", "Saved", "No pending changes", "Saved", ""));
        let v = present(&Inputs { connected: false, ..base() });
        assert_eq!((v.title, v.short_label, v.drive_label, v.drive_class), ("No pending changes", "Local", "Not connected", "idle"));
        let mut i = base();
        (i.status.unsealed_local, i.status.online) = (1, false);
        let v = present(&i);
        assert_eq!((v.title, v.short_label, v.drive_label, v.action), ("Waiting for connection", "Offline", "Offline", ""));
        i.status.online = true;
        i.status.auth_paused = true;
        let v = present(&i);
        assert_eq!((v.title, v.short_label, v.drive_label, v.action), ("Reconnect to Google Drive", "Sign in", "Sign in needed", "Reconnect Drive"));
        i.status.auth_paused = false;
        i.local.saved_at = 5.0;
        let v = present(&i);
        assert_eq!((v.title, v.local_label, v.drive_label, v.action, v.action_disabled), ("Waiting to save to Drive", "Saved", "Waiting", "Save now", false));
        i.local.failed = true;
        let v = present(&i);
        assert_eq!((v.title, v.short_label, v.local_label, v.local_class, v.action, v.notice.as_str()), ("Save needs attention", "Check save", "Needs attention", "error", "Retry save", "Keep this document open and retry."));
    }
}
