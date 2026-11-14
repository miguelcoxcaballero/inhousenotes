//! Pull-to-refresh on touch devices.
// v5: app-v5.js:setupMobileHomePullToRefresh

use super::lists;
use crate::abi::{ety, listen, ret, DomEvent};
use crate::host::Metric;
use crate::{App, Host, Mod, Ui};
use ihn_core::jsfmt::num;
use ihn_core::jsmath::{exp, pow};

const THRESHOLD: f64 = 96.0;
const MAX_OFFSET: f64 = 82.0;
const AXIS_LOCK: f64 = 8.0;
const CAROUSEL_VERTICAL_INTENT: f64 = 1.25;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Axis {
    #[default]
    None,
    Pending,
    Vertical,
    Horizontal,
}

#[derive(Default)]
pub struct State {
    enabled: bool,
    start: Option<(f64, f64)>,
    axis: Axis,
    in_carousel: bool,
    distance: f64,
    refreshing: bool,
    started_at: f64,
}

pub(super) fn init(app: &mut App, host: &mut dyn Host) {
    // v5: `navigator.maxTouchPoints <= 0 && !('ontouchstart' in window)` — no width condition.
    if app.cfg.max_touch_points == 0 {
        return;
    }
    app.st.home.pull.enabled = true;
    update_top(app, host);
    app.st.home.frames |= super::F_PULL_TOP;
    app.sched.frame(host.out(), Mod::Home);
    let (out, main) = (host.out(), app.st.home.main);
    out.listen(main, "touchstart", super::L_TOUCH_START, listen::PASSIVE);
    out.listen(main, "touchmove", super::L_TOUCH_MOVE, listen::ACTIVE);
    out.listen(main, "touchend", super::L_TOUCH_END, listen::PASSIVE);
    out.listen(main, "touchcancel", super::L_TOUCH_CANCEL, listen::PASSIVE);
}

pub(super) fn update_top(app: &mut App, host: &mut dyn Host) {
    let height = host.metric(app.st.home.topbar, Metric::OffsetHeight);
    host.out().style(Ui::DRIVE_HOME, "--home-pull-top", Some(&format!("{}px", num(height))));
}

fn vars(host: &mut dyn Host, distance: &str, scale: &str, opacity: &str) {
    let out = host.out();
    out.style(Ui::DRIVE_HOME, "--home-pull-distance", Some(distance));
    out.style(Ui::DRIVE_HOME, "--home-pull-scale", Some(scale));
    out.style(Ui::DRIVE_HOME, "--home-pull-opacity", Some(opacity));
}

fn set_pull(app: &mut App, host: &mut dyn Host, distance: f64) {
    let d = distance.max(0.0);
    app.st.home.pull.distance = d;
    let progress = (d / THRESHOLD).min(1.0);
    let eased = 1.0 - pow(1.0 - progress, 3.0);
    let offset = MAX_OFFSET * (1.0 - exp(-d / 78.0));
    vars(host, &format!("{}px", num(offset)), &num(0.84 + eased * 0.16), &num((progress * 2.0).min(1.0)));
    let out = host.out();
    out.class(Ui::DRIVE_HOME, "home-is-pulling", d > 0.0);
    out.attr(Ui::HOME_PULL_REFRESH, "aria-label", Some(if progress >= 1.0 { "Release to refresh" } else { "Pull to refresh" }));
}

fn reset(app: &mut App, host: &mut dyn Host) {
    let enabled = app.st.home.pull.enabled;
    app.st.home.pull = State { enabled, ..State::default() };
    let out = host.out();
    out.class(Ui::DRIVE_HOME, "home-pull-resetting", true);
    out.class(Ui::DRIVE_HOME, "home-is-pulling", false);
    vars(host, "0px", "0.84", "0");
    host.out().attr(Ui::HOME_PULL_REFRESH, "aria-label", Some("Pull to refresh"));
    app.sched.timer(host.out(), super::T_PULL_RESET, 220.0);
}

fn refresh(app: &mut App, host: &mut dyn Host) {
    let enabled = app.st.home.pull.enabled;
    app.st.home.pull = State { enabled, refreshing: true, started_at: host.now(), ..State::default() };
    app.sched.clear(host.out(), super::T_PULL_RESET);
    let out = host.out();
    out.class(Ui::DRIVE_HOME, "home-is-pulling", true);
    out.class(Ui::DRIVE_HOME, "home-is-refreshing", true);
    vars(host, "52px", "1", "1");
    host.out().attr(Ui::HOME_PULL_REFRESH, "aria-label", Some("Refreshing"));
    lists::refresh_files(app, host, true);
}

/// The refresh a pull started has settled: the indicator stays for at least 420 ms.
pub(super) fn refreshed(app: &mut App, host: &mut dyn Host) {
    if !app.st.home.pull.refreshing {
        return;
    }
    let remaining = (420.0 - (host.now() - app.st.home.pull.started_at)).max(0.0);
    if remaining > 0.0 {
        app.sched.timer(host.out(), super::T_PULL_HOLD, remaining);
    } else {
        finish(app, host);
    }
}

pub(super) fn finish(app: &mut App, host: &mut dyn Host) {
    if std::mem::take(&mut app.st.home.pull.refreshing) {
        host.out().class(Ui::DRIVE_HOME, "home-is-refreshing", false);
        reset(app, host);
    }
}

pub(super) fn reset_done(host: &mut dyn Host) { host.out().class(Ui::DRIVE_HOME, "home-pull-resetting", false); }

pub(super) fn touch(app: &mut App, host: &mut dyn Host, e: &DomEvent) -> u32 {
    let main = app.st.home.main;
    if app.st.home.pull.refreshing {
        return 0;
    }
    match e.ty {
        ety::TOUCHSTART => {
            if e.touches.len() != 1 || host.metric(main, Metric::ScrollTop) > 1.0 {
                return 0;
            }
            let Some(t) = e.touches.iter().next() else { return 0 };
            let pull = &mut app.st.home.pull;
            pull.start = Some((t.x, t.y));
            pull.in_carousel = e.path.contains(Ui::DRIVE_RECENTS);
            pull.axis = Axis::Pending;
            pull.distance = 0.0;
            app.sched.clear(host.out(), super::T_PULL_RESET);
            host.out().class(Ui::DRIVE_HOME, "home-pull-resetting", false);
        }
        ety::TOUCHMOVE => {
            let Some((sx, sy)) = app.st.home.pull.start.filter(|_| e.touches.len() == 1) else { return 0 };
            if host.metric(main, Metric::ScrollTop) > 1.0 {
                reset(app, host);
                return 0;
            }
            let Some(t) = e.touches.iter().next() else { return 0 };
            let (dx, dy) = (t.x - sx, t.y - sy);
            let pull = &mut app.st.home.pull;
            if pull.axis == Axis::Pending {
                let (ax, ay) = (dx.abs(), dy.abs());
                if ax.max(ay) < AXIS_LOCK {
                    return 0;
                }
                let vertical = if pull.in_carousel { ay > ax * CAROUSEL_VERTICAL_INTENT } else { ax <= ay };
                pull.axis = if vertical { Axis::Vertical } else { Axis::Horizontal };
            }
            if pull.axis == Axis::Horizontal {
                if pull.distance > 0.0 {
                    set_pull(app, host, 0.0);
                }
                return 0;
            }
            if dy <= 0.0 {
                set_pull(app, host, 0.0);
                return 0;
            }
            set_pull(app, host, dy);
            return ret::PREVENT_DEFAULT;
        }
        ety::TOUCHEND => {
            if app.st.home.pull.start.is_none() {
                return 0;
            }
            if app.st.home.pull.axis == Axis::Vertical && app.st.home.pull.distance >= THRESHOLD {
                refresh(app, host);
            } else {
                reset(app, host);
            }
        }
        _ => reset(app, host),
    }
    0
}
