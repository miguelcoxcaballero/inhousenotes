//! Slide-to-confirm modal (`#modal-confirm-slider`, spec/pages.md §6.1). Owned by W5.1.
//!
//! [`show`] is 5.18.0's `showConfirmSlider({title, message, sliderText})`: instead of a `Promise<boolean>` the caller
//! passes a function that receives the answer. A second `show` answers the pending one `false` first. The handle
//! follows the pointer under pointer capture; a release within 2 px of the end completes (`.complete`, snap to the
//! end, answer `true` after 120 ms), anything else animates back. Cancel or a click on the backdrop answers `false`.
// v5: app-v5.js:showConfirmSlider / _confirmSliderBindEvents / _confirmSliderSetPos / _confirmSliderResolve

use crate::abi::{ev, ptr};
use crate::{tag, App, Event, Host, Mod, Ui};
use ihn_core::jsfmt::num;

const L_DOWN: u32 = tag(Mod::PanelsSlider, 1);
const L_MOVE: u32 = tag(Mod::PanelsSlider, 2);
const L_UP: u32 = tag(Mod::PanelsSlider, 3);
const L_CANCEL: u32 = tag(Mod::PanelsSlider, 4);
const L_OVERLAY: u32 = tag(Mod::PanelsSlider, 5);
/// `pointercancel` / `lostpointercapture` (one listener id per registration).
const L_UP2: u32 = tag(Mod::PanelsSlider, 6);
const L_UP3: u32 = tag(Mod::PanelsSlider, 7);
const T_DONE: u32 = tag(Mod::PanelsSlider, 1);
/// v5 `_confirmSliderState.padding`.
const PAD: f64 = 4.0;

/// Receives the answer of [`show`].
pub type Done = fn(&mut App, &mut dyn Host, bool);

pub struct State {
    done: Option<Done>,
    bound: bool,
    dragging: bool,
    x: f64,
    max_x: f64,
    handle_w: f64,
    offset: f64,
}

impl Default for State {
    fn default() -> Self { State { done: None, bound: false, dragging: false, x: 0.0, max_x: 0.0, handle_w: 44.0, offset: 0.0 } }
}

/// v5 `_confirmSliderUpdateMetrics`: returns the slider's left edge.
fn metrics(app: &mut App, host: &mut dyn Host) -> f64 {
    let r = host.rect(Ui::CONFIRM_SLIDER);
    let h = host.rect(Ui::CONFIRM_SLIDER_HANDLE);
    let st = &mut app.st.panels_slider;
    st.handle_w = if h.w != 0.0 { h.w } else { 44.0 };
    st.max_x = (r.w - st.handle_w - PAD * 2.0).max(0.0);
    r.x
}

/// v5 `_confirmSliderSetPos(x, animate)` (metrics already read).
fn set_pos(app: &mut App, host: &mut dyn Host, x: f64, animate: bool) {
    let st = &mut app.st.panels_slider;
    let x = x.min(st.max_x).max(0.0);
    st.x = x;
    let fill = x + st.handle_w / 2.0 + PAD;
    let out = host.out();
    out.class(Ui::CONFIRM_SLIDER, "dragging", !animate);
    out.style(Ui::CONFIRM_SLIDER_HANDLE, "transform", Some(&format!("translateX({}px)", num(x))));
    out.style(Ui::CONFIRM_SLIDER_FILL, "width", Some(&format!("{}px", num(fill))));
}

/// v5 `_confirmSliderReset`.
fn reset(app: &mut App, host: &mut dyn Host) {
    host.out().class(Ui::CONFIRM_SLIDER, "complete", false);
    metrics(app, host);
    set_pos(app, host, 0.0, true);
}

/// v5 `_confirmSliderResolve(value)`.
fn resolve(app: &mut App, host: &mut dyn Host, value: bool) {
    let out = host.out();
    out.class(Ui::CONFIRM_SLIDER, "dragging", false);
    out.class(Ui::MODAL_CONFIRM_SLIDER, "visible", false);
    app.sched.clear(out, T_DONE);
    let done = app.st.panels_slider.done.take();
    app.st.panels_slider.dragging = false;
    reset(app, host);
    if let Some(done) = done {
        done(app, host, value);
    }
}

/// v5 `showConfirmSlider({title, message, sliderText})`: empty texts take the defaults.
pub fn show(app: &mut App, host: &mut dyn Host, title: &str, message: &str, text: &str, done: Done) {
    let out = host.out();
    if !std::mem::replace(&mut app.st.panels_slider.bound, true) {
        out.listen(Ui::CONFIRM_SLIDER, "pointerdown", L_DOWN, 0);
        out.listen(Ui::CONFIRM_SLIDER, "pointermove", L_MOVE, 0);
        out.listen(Ui::CONFIRM_SLIDER, "pointerup", L_UP, 0);
        out.listen(Ui::CONFIRM_SLIDER, "pointercancel", L_UP2, 0);
        out.listen(Ui::CONFIRM_SLIDER, "lostpointercapture", L_UP3, 0);
        out.listen(Ui::CONFIRM_SLIDER_CANCEL, "click", L_CANCEL, 0);
        out.listen(Ui::MODAL_CONFIRM_SLIDER, "click", L_OVERLAY, 0);
    }
    out.text(Ui::CONFIRM_SLIDER_TITLE, if title.is_empty() { "Are you sure?" } else { title });
    out.text(Ui::CONFIRM_SLIDER_MESSAGE, message);
    out.text(Ui::CONFIRM_SLIDER_TEXT, if text.is_empty() { "Slide to confirm" } else { text });
    if let Some(prev) = app.st.panels_slider.done.take() {
        prev(app, host, false);
    }
    app.st.panels_slider.done = Some(done);
    app.sched.clear(host.out(), T_DONE);
    reset(app, host);
    host.out().class(Ui::MODAL_CONFIRM_SLIDER, "visible", true);
}

/// The modal is waiting for an answer.
pub fn is_open(app: &App) -> bool { app.st.panels_slider.done.is_some() }

pub fn init(_app: &mut App, _host: &mut dyn Host) {}

pub fn event(app: &mut App, host: &mut dyn Host, event: &Event) -> u32 {
    match event {
        Event::Pointer(p) => match p.listener {
            L_DOWN if p.kind == ptr::DOWN => {
                if p.button > 0 {
                    return 0;
                }
                app.st.panels_slider.dragging = true;
                let out = host.out();
                out.class(Ui::CONFIRM_SLIDER, "dragging", true);
                out.class(Ui::CONFIRM_SLIDER, "complete", false);
                out.pointer_capture(Ui::CONFIRM_SLIDER, p.pointer_id, true);
                let left = metrics(app, host);
                let h = host.rect(Ui::CONFIRM_SLIDER_HANDLE);
                let offset = (p.raw.x - h.x).max(0.0).min(h.w);
                app.st.panels_slider.offset = offset;
                set_pos(app, host, p.raw.x - left - PAD - offset, false);
                0
            }
            L_MOVE if app.st.panels_slider.dragging => {
                let left = metrics(app, host);
                let x = p.raw.x - left - PAD - app.st.panels_slider.offset;
                set_pos(app, host, x, false);
                0
            }
            L_UP | L_UP2 | L_UP3 if app.st.panels_slider.dragging => {
                app.st.panels_slider.dragging = false;
                let out = host.out();
                out.pointer_capture(Ui::CONFIRM_SLIDER, p.pointer_id, false);
                out.class(Ui::CONFIRM_SLIDER, "dragging", false);
                metrics(app, host);
                let st = &app.st.panels_slider;
                if st.x >= st.max_x - 2.0 {
                    let max = st.max_x;
                    host.out().class(Ui::CONFIRM_SLIDER, "complete", true);
                    set_pos(app, host, max, true);
                    app.sched.timer(host.out(), T_DONE, 120.0);
                } else {
                    set_pos(app, host, 0.0, true);
                }
                0
            }
            _ => 0,
        },
        Event::Dom(d) => match d.listener {
            L_CANCEL => {
                resolve(app, host, false);
                0
            }
            L_OVERLAY if d.flags & ev::EXACT != 0 && d.path.first() == Ui::MODAL_CONFIRM_SLIDER => {
                resolve(app, host, false);
                0
            }
            _ => 0,
        },
        Event::Timer(T_DONE) => {
            resolve(app, host, true);
            0
        }
        _ => 0,
    }
}

/// `panels.slider.state` → `{open, x, maxX, dragging}`.
pub fn test_cmd(app: &mut App, _host: &mut dyn Host, name: &str, _args: &str) -> Option<String> {
    let st = &app.st.panels_slider;
    match name {
        "state" => Some(format!("{{\"open\":{},\"x\":{},\"maxX\":{},\"dragging\":{}}}", st.done.is_some(), num(st.x), num(st.max_x), st.dragging)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::FakeHost;
    use crate::Rect;

    fn answer(app: &mut App, _h: &mut dyn Host, v: bool) { app.st.panels_slider.offset = if v { 1.0 } else { -1.0 }; }

    #[test]
    fn completes_only_at_the_end_and_cancel_answers_false() {
        let (mut app, mut host) = crate::shell::testkit::boot(0, |h| {
            h.rects.insert(Ui::CONFIRM_SLIDER, Rect { x: 100.0, y: 0.0, w: 352.0, h: 52.0 });
            h.rects.insert(Ui::CONFIRM_SLIDER_HANDLE, Rect { x: 104.0, y: 4.0, w: 44.0, h: 44.0 });
        });
        show(&mut app, &mut host, "Delete page?", "m", "Slide to delete", answer);
        assert!(is_open(&app));
        assert_eq!(app.st.panels_slider.max_x, 300.0);
        // A second show answers the first one `false`.
        show(&mut app, &mut host, "", "", "", answer);
        assert_eq!(app.st.panels_slider.offset, -1.0);
        resolve(&mut app, &mut host, true);
        assert_eq!(app.st.panels_slider.offset, 1.0);
        assert!(!is_open(&app));
        let text = host.take();
        assert!(text.contains("Slide to confirm") && text.contains("translateX(0px)"), "{text}");
    }
}
