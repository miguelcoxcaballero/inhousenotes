//! Presence avatars and the `Actividad del documento` popup (SYNC §9.1 mapping onto 5.18.0
//! `renderPresenceAvatars` / `renderPresencePopup`, app-v5.js:24791-25105). Owned by W4.5.
//!
//! The avatars show every present remote device the engine reports (`Sync::peers`): `p2p-connected` with the peer
//! badge for an open channel, `p2p-connecting` during a handshake, no class for relay-only devices; every shown
//! device has the `live` device badge. Save rings follow the device's `save_phase` (ARCHITECTURE §6.6 code) with
//! 5.18.0's timing: `uploading` reads `processing` for its first 900 ms and `saved` lasts 2.2 s after a change.

use super::{L_AVATARS, L_DOC_CLICK, L_DOC_KEY, L_POPUP_CLOSE, T_RING};
use crate::abi::{ety, ret, DomEvent};
use crate::doc::save_state::{self, Phase};
use crate::{auth, App, Host, Ui};
use ihn_core::doc::{PeerLink, SyncEngine};
use ihn_core::jsfmt;

const AVATAR_FALLBACK: &str = "data:image/svg+xml;utf8,<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"%23999\"><path d=\"M12 12c2.21 0 4-1.79 4-4s-1.79-4-4-4-4 1.79-4 4 1.79 4 4 4zm0 2c-2.67 0-8 1.34-8 4v2h16v-2c0-2.66-5.33-4-8-4z\"/></svg>";
const SAVE_DONE_MS: f64 = 2200.0;
const PROCESSING_MIN_VISIBLE_MS: f64 = 900.0;
const RING_SPIN_MS: f64 = 1080.0;

/// One device as the popup and the avatars see it.
#[derive(Clone, PartialEq, Debug)]
struct User {
    inst: u64,
    name: String,
    email: String,
    photo: String,
    dtype: &'static str,
    label: String,
    link: PeerLink,
    ring: &'static str,
    own: bool,
    main: bool,
}

#[derive(Default)]
pub struct State {
    users: Vec<User>,
    /// What the avatar row shows (to skip identical renders).
    shown: Vec<User>,
    open: bool,
    /// Per device (own = 0): last save-phase code and the time it was first seen.
    phases: Vec<(u64, u8, f64)>,
}

impl State {
    pub fn shown_count(&self) -> usize { self.shown.len() }
}

pub fn init(app: &mut App, host: &mut dyn Host) {
    if app.cfg.has(crate::abi::mode::HEADLESS) {
        return;
    }
    let out = host.out();
    out.listen(Ui::PRESENCE_AVATARS, "click", L_AVATARS, 0);
    out.listen(Ui::BTN_CLOSE_PRESENCE_POPUP, "click", L_POPUP_CLOSE, 0);
    out.listen(crate::abi::Node::DOCUMENT, "click", L_DOC_CLICK, 0);
    out.listen(crate::abi::Node::DOCUMENT, "keydown", L_DOC_KEY, 0);
}

fn dtype(code: u8) -> &'static str {
    match code {
        b'm' => "mobile",
        b't' => "tablet",
        _ => "pc",
    }
}

/// v5 `getPresenceVisibleSavePhase` with the phase's local first sighting; `Some(ms)` = re-render then.
fn ring(st: &mut State, inst: u64, code: u8, now: f64) -> (&'static str, Option<f64>) {
    let since = match st.phases.iter_mut().find(|p| p.0 == inst) {
        Some(p) if p.1 == code => p.2,
        Some(p) => {
            (p.1, p.2) = (code, now);
            now
        }
        None => {
            st.phases.push((inst, code, f64::NEG_INFINITY));
            f64::NEG_INFINITY
        }
    };
    let age = now - since;
    match code {
        c if c == Phase::Saved.code() && age <= SAVE_DONE_MS => ("saved", Some(since + SAVE_DONE_MS + 1.0)),
        c if c == Phase::Uploading.code() && age < PROCESSING_MIN_VISIBLE_MS => ("processing", Some(since + PROCESSING_MIN_VISIBLE_MS)),
        c if c == Phase::Uploading.code() => ("uploading", None),
        c if c == Phase::Saving.code() || c == Phase::Building.code() => ("processing", None),
        c if c == Phase::Syncing.code() => ("syncing", None),
        _ => ("", None),
    }
}

fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

/// v5 `getDeviceBadgeSvg`.
fn device_svg(t: &str) -> &'static str {
    match t {
        "mobile" => "<svg viewBox=\"0 0 24 24\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><rect x=\"7\" y=\"2\" width=\"10\" height=\"20\" rx=\"2\"/><line x1=\"12\" y1=\"18\" x2=\"12\" y2=\"18.01\"/></svg>",
        "tablet" => "<svg viewBox=\"0 0 24 24\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><rect x=\"4\" y=\"2\" width=\"16\" height=\"20\" rx=\"2\"/><line x1=\"12\" y1=\"18\" x2=\"12\" y2=\"18.01\"/></svg>",
        _ => "<svg viewBox=\"0 0 24 24\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><rect x=\"2\" y=\"3\" width=\"20\" height=\"14\" rx=\"2\"/><line x1=\"8\" y1=\"21\" x2=\"16\" y2=\"21\"/><line x1=\"12\" y1=\"17\" x2=\"12\" y2=\"21\"/></svg>",
    }
}
const PEER_SVG: &str = "<svg viewBox=\"0 0 24 24\" aria-hidden=\"true\"><path d=\"M9.6 14.4l4.8-4.8\"/><path d=\"M7.2 16.8l-1.1 1.1a3.4 3.4 0 0 1-4.8-4.8l3.1-3.1a3.4 3.4 0 0 1 4.8 0\"/><path d=\"M16.8 7.2l1.1-1.1a3.4 3.4 0 0 1 4.8 4.8L19.6 14a3.4 3.4 0 0 1-4.8 0\"/></svg>";

fn who(u: &User) -> &str {
    if !u.name.is_empty() {
        &u.name
    } else if !u.email.is_empty() {
        &u.email
    } else {
        "Unknown"
    }
}

/// v5 `renderPresenceAvatars(users)`.
fn avatars_html(users: &[User], now: f64) -> String {
    let mut html = String::new();
    if users.len() > 3 {
        let names: Vec<&str> = users[3..].iter().map(who).collect();
        html += &format!("<div class=\"presence-overflow\" title=\"{}\" style=\"z-index: 0;\">+{}</div>", esc(&names.join(", ")), users.len() - 3);
    }
    let visible = &users[..users.len().min(3)];
    for (visual, u) in visible.iter().rev().enumerate() {
        let (connected, connecting) = (u.link == PeerLink::Connected, u.link == PeerLink::Connecting);
        let class = if connected { " p2p-connected" } else if connecting { " p2p-connecting" } else { "" };
        let suffix = if connected { " · Peer-to-peer connected" } else if connecting { " · Connecting peer-to-peer" } else { "" };
        html += &format!("<div class=\"presence-avatar-wrapper{class}\" style=\"--presence-z: {};\" title=\"{} ({}){suffix}\">", visual + 1, esc(who(u)), u.dtype);
        if !u.ring.is_empty() {
            if u.ring == "saved" {
                html += "<span class=\"presence-save-ring saved\"></span>";
            } else {
                html += &format!("<span class=\"presence-save-ring {}\" style=\"--presence-ring-delay: {}ms;\"></span>", u.ring, jsfmt::num(-(now % RING_SPIN_MS)));
            }
        }
        let photo = if u.photo.is_empty() { AVATAR_FALLBACK } else { &u.photo };
        html += &format!("<img class=\"presence-avatar-img\" src=\"{}\" alt=\"{}\">", esc(photo), esc(if u.name.is_empty() { "User" } else { &u.name }));
        html += &format!("<div class=\"presence-device-badge live\">{}</div>", device_svg(u.dtype));
        if connected {
            html += &format!("<span class=\"presence-peer-badge\" aria-hidden=\"true\">{PEER_SVG}</span>");
        }
        html += "</div>";
    }
    html
}

/// The engine's present devices (+ timers for ring changes); renders when the row changed.
pub fn update(app: &mut App, host: &mut dyn Host) {
    let now = host.now();
    let Some(run) = app.st.collab.run.as_ref().filter(|r| !r.leaving) else { return clear(app, host) };
    let views = run.sync.peers().to_vec();
    let own_inst = run.sync.core.inst;
    let main = run.sync.core.presence.scribe(own_inst, run.sync.core.now);
    let st = &mut app.st.collab.presence;
    let mut users = Vec::new();
    let mut wake: Option<f64> = None;
    for v in views {
        if v.name.trim().is_empty() && v.email.trim().is_empty() {
            continue;
        }
        let (r, at) = ring(st, v.inst, v.save_phase, now);
        wake = match (wake, at) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        users.push(User { inst: v.inst, name: v.name.trim().into(), email: v.email.trim().into(), photo: v.photo_url, dtype: dtype(v.device_type), label: v.label, link: v.link, ring: r, own: false, main: v.inst == main });
    }
    st.phases.retain(|p| p.0 == 0 || users.iter().any(|u| u.inst == p.0));
    st.users = users.clone();
    if let Some(at) = wake {
        app.sched.timer(host.out(), T_RING, (at - now).max(0.0));
    }
    if app.st.collab.presence.shown != users {
        let out = host.out();
        out.html(Ui::PRESENCE_AVATARS, &avatars_html(&users, now));
        if users.is_empty() {
            out.attr(Ui::PRESENCE_AVATARS, "aria-label", None);
            out.class(Ui::PRESENCE_AVATARS, "popup-open", false);
        } else {
            out.attr(Ui::PRESENCE_AVATARS, "aria-label", Some(&format!("Actividad reciente: {} usuario{}", users.len(), if users.len() == 1 { "" } else { "s" })));
        }
        app.st.collab.presence.shown = users;
    }
    if app.st.collab.presence.open {
        render_popup(app, host);
    }
}

/// No engine: no avatars, popup closed.
pub fn clear(app: &mut App, host: &mut dyn Host) {
    let st = &mut app.st.collab.presence;
    st.users.clear();
    st.phases.clear();
    if !st.shown.is_empty() {
        st.shown.clear();
        let out = host.out();
        out.html(Ui::PRESENCE_AVATARS, "");
        out.attr(Ui::PRESENCE_AVATARS, "aria-label", None);
        out.class(Ui::PRESENCE_AVATARS, "popup-open", false);
    }
    close(app, host);
}

/// v5 `formatPresenceLastActivity` for an activity `ms` before `now`.
fn last_activity(age_ms: f64) -> String {
    let sec = (age_ms / 1000.0).floor().max(0.0);
    if sec < 60.0 {
        return "hace menos de 1 min".into();
    }
    let min = (sec / 60.0).floor();
    if min < 60.0 {
        return format!("hace {min} min");
    }
    let hour = (min / 60.0).floor();
    if hour < 24.0 {
        return format!("hace {hour} h");
    }
    format!("hace {} d", (hour / 24.0).floor())
}

/// v5 `renderPresencePopup(users)` (own device first; accounts grouped by e-mail, then name).
fn render_popup(app: &mut App, host: &mut dyn Host) {
    let now = host.now();
    let time = host.time();
    let mut users = app.st.collab.presence.users.clone();
    if let Some(p) = auth::profile(app).cloned() {
        let code = save_state::phase(app).code();
        let (ring, _) = ring(&mut app.st.collab.presence, 0, code, now);
        let run = app.st.collab.run.as_ref();
        let own_inst = run.map_or(0, |r| r.sync.core.inst);
        let main = run.is_some_and(|r| r.sync.core.presence.scribe(own_inst, r.sync.core.now) == own_inst);
        let name = if !p.name.is_empty() { p.name.clone() } else if !p.email.is_empty() { p.email.clone() } else { "You".into() };
        users.insert(0, User { inst: 0, name, email: p.email, photo: p.photo, dtype: dtype(super::device_type(app)), label: super::device_label(app).into(), link: PeerLink::Relay, ring, own: true, main });
    }
    let leader = app.st.collab.run.as_ref().is_some_and(|r| r.sync.core.leader);
    // Group by account (all shown devices are online; order of appearance, own first).
    let mut groups: Vec<(String, Vec<&User>)> = Vec::new();
    for (i, u) in users.iter().enumerate() {
        let key = if u.email.is_empty() { format!("n:{}:{i}", u.name.to_lowercase()) } else { format!("e:{}", u.email.to_lowercase()) };
        match groups.iter_mut().find(|g| g.0 == key) {
            Some(g) => g.1.push(u),
            None => groups.push((key, vec![u])),
        }
    }
    let mut html = String::new();
    if groups.is_empty() {
        html += "<div class=\"presence-popup-empty\">No hay actividad reciente.</div>";
    }
    let absolute = host.intl(crate::host::IntlOp::Date, &format!("{}\0string\0\0", jsfmt::num(time.floor())));
    for (_, devices) in &groups {
        let first = devices[0];
        let name = devices.iter().map(|u| u.name.as_str()).find(|n| !n.is_empty()).unwrap_or("");
        let email = devices.iter().map(|u| u.email.as_str()).find(|n| !n.is_empty()).unwrap_or("");
        let photo = devices.iter().map(|u| u.photo.as_str()).find(|n| !n.is_empty()).unwrap_or(AVATAR_FALLBACK);
        let shown_name = if !name.is_empty() { name } else if !email.is_empty() { email } else { "Unknown" };
        html += &format!(
            "<div class=\"presence-account\"><div class=\"presence-account-head\"><img class=\"presence-account-photo\" src=\"{}\" alt=\"{}\"><div><div class=\"presence-account-name\">{}</div><div class=\"presence-account-email\">{}</div></div></div><div class=\"presence-device-list\">",
            esc(photo),
            esc(if first.name.is_empty() { "User" } else { shown_name }),
            esc(shown_name),
            esc(if email.is_empty() { "No email" } else { email })
        );
        for u in devices {
            let device = if !u.label.is_empty() {
                u.label.clone()
            } else {
                match u.dtype {
                    "mobile" => "Mobile".into(),
                    "tablet" => "Tablet".into(),
                    _ => "PC".into(),
                }
            };
            let transport = if u.own {
                if leader { "This device · coordination leader" } else { "This device · local tab" }
            } else if u.link == PeerLink::Connected {
                "Peer-to-peer"
            } else {
                "Drive fallback"
            };
            let phase = match u.ring {
                "processing" => "preparing changes",
                "uploading" => "uploading to Drive",
                "syncing" => "receiving changes",
                "saved" => "saved",
                _ => "idle",
            };
            html += &format!(
                "<div class=\"presence-device-row\"><div class=\"presence-device-icon\">{}</div><div><div class=\"presence-device-main\">{}{}</div><div class=\"presence-device-meta\">Ubicación desconocida · Última actividad {} ({})</div><div class=\"presence-device-connection{}\">{transport} · {phase}{}</div></div></div>",
                device_svg(u.dtype),
                esc(&device),
                if u.own { " · This device" } else { "" },
                last_activity(0.0),
                esc(&absolute),
                if u.main { " main" } else { "" },
                if u.main { " · Main device" } else { "" }
            );
        }
        html += "</div></div>";
    }
    host.out().html(Ui::PRESENCE_POPUP_LIST, &html);
}

/// v5 `positionPresencePopup` (run right after opening; the popup is laid out synchronously).
fn position(app: &mut App, host: &mut dyn Host) {
    let margin = 12.0;
    let (vw, vh) = app.cfg.viewport;
    let anchor = host.rect(Ui::PRESENCE_AVATARS);
    let popup = host.rect(Ui::PRESENCE_POPUP);
    let mut left = anchor.x;
    let mut top = anchor.y + anchor.h + 10.0;
    if left + popup.w > vw - margin {
        left = vw - popup.w - margin;
    }
    if left < margin {
        left = margin;
    }
    if top + popup.h > vh - margin {
        let above = anchor.y - popup.h - 10.0;
        top = if above >= margin { above } else { margin.max(vh - popup.h - margin) };
    }
    let out = host.out();
    out.style(Ui::PRESENCE_POPUP, "left", Some(&format!("{}px", jsfmt::num(left.round()))));
    out.style(Ui::PRESENCE_POPUP, "top", Some(&format!("{}px", jsfmt::num(top.round()))));
}

fn open(app: &mut App, host: &mut dyn Host) {
    render_popup(app, host);
    app.st.collab.presence.open = true;
    let out = host.out();
    out.class(Ui::PRESENCE_POPUP, "hidden", false);
    out.class(Ui::PRESENCE_AVATARS, "popup-open", true);
    position(app, host);
}

/// v5 `closePresencePopup`.
pub fn close(app: &mut App, host: &mut dyn Host) {
    app.st.collab.presence.open = false;
    let out = host.out();
    out.class(Ui::PRESENCE_POPUP, "hidden", true);
    out.class(Ui::PRESENCE_AVATARS, "popup-open", false);
}

pub fn dom(app: &mut App, host: &mut dyn Host, e: &DomEvent) -> u32 {
    let open_now = app.st.collab.presence.open;
    match e.listener {
        L_AVATARS => {
            if !app.st.collab.presence.users.is_empty() {
                if open_now {
                    close(app, host);
                } else {
                    open(app, host);
                }
            }
            ret::STOP_PROPAGATION
        }
        L_POPUP_CLOSE => {
            close(app, host);
            ret::STOP_PROPAGATION
        }
        L_DOC_CLICK if open_now && !e.path.contains(Ui::PRESENCE_POPUP) && !e.path.contains(Ui::PRESENCE_AVATARS) => {
            close(app, host);
            0
        }
        L_DOC_KEY if e.ty == ety::KEYDOWN && e.key == "Escape" && open_now => {
            close(app, host);
            0
        }
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(name: &str, link: PeerLink, ring: &'static str) -> User {
        User { inst: 1, name: name.into(), email: format!("{name}@x"), photo: String::new(), dtype: "pc", label: "Linux PC".into(), link, ring, own: false, main: false }
    }

    #[test]
    fn avatar_markup_follows_5_18() {
        let html = avatars_html(&[user("Ana", PeerLink::Connected, ""), user("Ben", PeerLink::Connecting, "processing")], 2000.0);
        // Rendered last-first: Ben (z 1) then Ana (z 2).
        let ben = html.find("title=\"Ben (pc) · Connecting peer-to-peer\"").unwrap();
        let ana = html.find("title=\"Ana (pc) · Peer-to-peer connected\"").unwrap();
        assert!(ben < ana);
        assert!(html.contains("<div class=\"presence-avatar-wrapper p2p-connecting\" style=\"--presence-z: 1;\""));
        assert!(html.contains("<div class=\"presence-avatar-wrapper p2p-connected\" style=\"--presence-z: 2;\""));
        assert!(html.contains("presence-save-ring processing\" style=\"--presence-ring-delay: -920ms;\""));
        assert_eq!(html.matches("presence-peer-badge").count(), 1);
        assert_eq!(html.matches("presence-device-badge live").count(), 2);
        let relay = avatars_html(&[user("Cho", PeerLink::Relay, "")], 0.0);
        assert!(relay.contains("<div class=\"presence-avatar-wrapper\" style=\"--presence-z: 1;\" title=\"Cho (pc)\">"));
        let many: Vec<User> = ["A", "B", "C", "D", "E"].iter().map(|n| user(n, PeerLink::Relay, "")).collect();
        assert!(avatars_html(&many, 0.0).starts_with("<div class=\"presence-overflow\" title=\"D, E\" style=\"z-index: 0;\">+2</div>"));
    }

    #[test]
    fn ring_timing() {
        let mut st = State::default();
        assert_eq!(ring(&mut st, 7, Phase::Saved.code(), 0.0).0, ""); // first sighting: no completion flash
        assert_eq!(ring(&mut st, 7, Phase::Uploading.code(), 100.0), ("processing", Some(1000.0)));
        assert_eq!(ring(&mut st, 7, Phase::Uploading.code(), 1000.0).0, "uploading");
        assert_eq!(ring(&mut st, 7, Phase::Saved.code(), 1500.0).0, "saved");
        assert_eq!(ring(&mut st, 7, Phase::Saved.code(), 3800.0).0, "");
        assert_eq!(last_activity(59_000.0), "hace menos de 1 min");
        assert_eq!(last_activity(3_600_000.0), "hace 1 h");
    }
}
