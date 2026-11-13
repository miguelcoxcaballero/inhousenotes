//! The fast-scroll thumb (`#document-fast-scroll`) for Home's lists and the opened folder.
// v5: app-v5.js:installDocumentFastScroller (the editor's canvas is the editor unit's target)

use super::folders;
use crate::abi::{listen, observe, ptr, ret, DomEvent, PointerBatch};
use crate::dom::Prop;
use crate::host::Metric;
use crate::{shell, App, Host, Mod, Node, Ui};
use ihn_core::jsfmt::num;
use ihn_core::jsmath::round;

#[derive(Clone, Copy, Debug)]
pub(super) struct Metrics {
    target: Node,
    max: f64,
    top: f64,
    travel: f64,
    offset: f64,
}

#[derive(Clone, Copy, Debug)]
struct Drag {
    id: i32,
    y: f64,
    ratio: f64,
    travel: f64,
    target: Node,
}

#[derive(Default)]
pub struct State {
    thumb: Node,
    hidden: bool,
    scheduled: bool,
    drag: Option<Drag>,
    main_named: bool,
}

const THUMB: &str = "<div id=\"document-fast-scroll\" tabindex=\"0\" hidden=\"\" role=\"scrollbar\" aria-label=\"Fast scroll\" aria-orientation=\"vertical\" aria-valuemin=\"0\" aria-valuemax=\"100\"><svg width=\"24\" height=\"28\" viewBox=\"0 0 24 28\" aria-hidden=\"true\"><path d=\"M6 12h12L12 4z M6 16h12l-6 8z\" fill=\"currentColor\"/></svg></div>";

pub(super) fn init(app: &mut App, host: &mut dyn Host) {
    let (main, inner) = (app.st.home.main, app.st.home.view_inner);
    let out = host.out();
    let thumb = out.insert_html(Node::BODY, Node::NONE, THUMB);
    for (event, id) in [("pointerdown", super::L_THUMB_DOWN), ("pointermove", super::L_THUMB_MOVE), ("pointerup", super::L_THUMB_UP), ("pointercancel", super::L_THUMB_CANCEL), ("lostpointercapture", super::L_THUMB_LOST)] {
        out.listen(thumb, event, id, 0);
    }
    out.listen(thumb, "keydown", super::L_THUMB_KEY, 0);
    out.listen(main, "scroll", super::L_MAIN_SCROLL, listen::PASSIVE);
    for node in [main, Ui::DRIVE_FOLDER_VIEW_SCROLL, Ui::DRIVE_RECENTS, inner] {
        out.observe(node, super::L_SCROLL_RESIZE, observe::RESIZE, "");
    }
    for node in [Ui::DRIVE_HOME, Ui::EDITOR_VIEW, Ui::DRIVE_FOLDER_VIEW] {
        out.observe(node, super::L_SCROLL_VISIBILITY, observe::ATTRIBUTES, "class,data-home-tab");
    }
    for node in [main, Ui::DRIVE_FOLDER_VIEW_SCROLL] {
        out.observe(node, super::L_SCROLL_CONTENTS, observe::CHILD_LIST | observe::SUBTREE, "");
    }
    app.st.home.scroll = State { thumb, hidden: true, ..State::default() };
    schedule(app, host);
}

/// Repositions the thumb in the next frame.
pub fn schedule(app: &mut App, host: &mut dyn Host) {
    if !app.st.home.on || std::mem::replace(&mut app.st.home.scroll.scheduled, true) {
        return;
    }
    app.st.home.frames |= super::F_SCROLL;
    app.sched.frame(host.out(), Mod::Home);
}

fn measure(app: &mut App, host: &mut dyn Host) -> Option<Metrics> {
    if shell::view(app) != shell::View::Home {
        return None;
    }
    let target = if folders::view_open(app) { Ui::DRIVE_FOLDER_VIEW_SCROLL } else { app.st.home.main };
    let rect = host.rect(target);
    let inner_height = host.metric(Node::WINDOW, Metric::InnerHeight);
    Some(Metrics {
        target,
        max: host.metric(target, Metric::ScrollHeight) - host.metric(target, Metric::ClientHeight),
        top: rect.y.max(0.0) + 12.0,
        travel: (inner_height.min(rect.bottom()) - rect.y.max(0.0) - 72.0).max(1.0),
        offset: host.metric(target, Metric::ScrollTop),
    })
}

pub(super) fn render(app: &mut App, host: &mut dyn Host) -> Option<Metrics> {
    app.st.home.scroll.scheduled = false;
    let metrics = measure(app, host);
    let hidden = metrics.is_none_or(|m| m.max < 2.0 || m.travel < 30.0);
    let st = &mut app.st.home.scroll;
    let thumb = st.thumb;
    if std::mem::replace(&mut st.hidden, hidden) != hidden {
        host.out().prop(thumb, "hidden", Prop::Bool(hidden));
    }
    let m = metrics.filter(|_| !hidden)?;
    let ratio = (m.offset / m.max).clamp(0.0, 1.0);
    let out = host.out();
    out.style(thumb, "transform", Some(&format!("translateY({}px)", num(m.top + ratio * m.travel))));
    out.attr(thumb, "aria-valuenow", Some(&num(round(ratio * 100.0))));
    if m.target != Ui::DRIVE_FOLDER_VIEW_SCROLL && !std::mem::replace(&mut app.st.home.scroll.main_named, true) {
        out.prop(m.target, "id", Prop::Str("drive-main-scroll"));
    }
    out.attr(thumb, "aria-controls", Some(if m.target == Ui::DRIVE_FOLDER_VIEW_SCROLL { "drive-folder-view-scroll" } else { "drive-main-scroll" }));
    Some(m)
}

fn move_to(app: &mut App, host: &mut dyn Host, ratio: f64) {
    let Some(m) = measure(app, host) else { return };
    host.out().prop(m.target, "scrollTop", Prop::Num(ratio.clamp(0.0, 1.0) * m.max));
    schedule(app, host);
}

pub(super) fn pointer(app: &mut App, host: &mut dyn Host, p: &PointerBatch) -> u32 {
    let thumb = app.st.home.scroll.thumb;
    match p.kind {
        ptr::DOWN => {
            if p.button != 0 {
                return 0;
            }
            if let Some(m) = render(app, host) {
                app.st.home.scroll.drag = Some(Drag { id: p.pointer_id, y: p.raw.y, ratio: (m.offset / m.max).clamp(0.0, 1.0), travel: m.travel, target: m.target });
                host.out().pointer_capture(thumb, p.pointer_id, true);
                host.out().class(thumb, "is-dragging", true);
            } else {
                app.st.home.scroll.drag = None;
            }
            ret::PREVENT_DEFAULT | ret::STOP_PROPAGATION
        }
        ptr::MOVE => {
            let Some(drag) = app.st.home.scroll.drag.filter(|d| d.id == p.pointer_id) else { return 0 };
            let target = if folders::view_open(app) { Ui::DRIVE_FOLDER_VIEW_SCROLL } else { app.st.home.main };
            if shell::view(app) != shell::View::Home || target != drag.target {
                app.st.home.scroll.drag = None;
            } else {
                move_to(app, host, drag.ratio + (p.raw.y - drag.y) / drag.travel);
            }
            ret::PREVENT_DEFAULT | ret::STOP_PROPAGATION
        }
        _ => {
            app.st.home.scroll.drag = None;
            host.out().class(thumb, "is-dragging", false);
            schedule(app, host);
            0
        }
    }
}

pub(super) fn key(app: &mut App, host: &mut dyn Host, e: &DomEvent) -> u32 {
    let Some(m) = measure(app, host) else { return 0 };
    let step = match e.key {
        "ArrowDown" => 0.04,
        "ArrowUp" => -0.04,
        "PageDown" => 0.2,
        "PageUp" => -0.2,
        "Home" | "End" => 0.0,
        _ => return 0,
    };
    move_to(app, host, match e.key {
        "Home" => 0.0,
        "End" => 1.0,
        _ => m.offset / m.max + step,
    });
    ret::PREVENT_DEFAULT | ret::STOP_PROPAGATION
}
