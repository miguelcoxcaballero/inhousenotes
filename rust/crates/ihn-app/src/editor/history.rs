//! Editor: Undo/redo (ARCHITECTURE §4.3).
//!
//! Every local action is one batch; its step keeps the ops that reverse it (`Doc::inverse`, taken at commit).
//! Undo applies them as a *new* batch (new stamps, 5.18.0's re-stamp semantics) and keeps that batch's own inverse
//! as the redo ops, and the other way round, so a step always holds the ops of its next move. Limits are 5.18.0's:
//! 10 steps, about 25,000 points (halved while hidden), the newest step always kept. The stack is cleared when a
//! document opens or closes and after a remote structural change (5.18.0's clearing rules); other units call
//! [`clear`] for theirs (import, restore, photo page, page delete).

use super::{pages, view};
use crate::abi::{ety, ev, life, ret};
use crate::doc::session;
use crate::{tag, App, Event, Host, Mod, Node, Prop, Ui};
use ihn_core::id::Id;
use ihn_core::op::{Body, Op};

const L_UNDO: u32 = tag(Mod::EditorHistory, 1);
const L_REDO: u32 = tag(Mod::EditorHistory, 2);
const L_KEY: u32 = tag(Mod::EditorHistory, 3);

/// v5: `state.maxHistory`, `MAX_HISTORY_MEMORY`.
pub const MAX_STEPS: usize = 10;
pub const MAX_POINTS: usize = 25_000;

struct Step {
    page: Id,
    /// The ops of the next undo (when applied) or redo (when undone).
    ops: Vec<Op>,
    /// v5 `estimateHistoryActionMemory`: points held by the action.
    points: usize,
}

#[derive(Default)]
pub struct State {
    steps: Vec<Step>,
    /// Steps applied (v5 `historyIndex + 1`).
    done: usize,
    /// The page order last seen (a remote change of it clears the stack).
    order: Vec<Id>,
    /// The disabled state written to the buttons.
    buttons: Option<(bool, bool)>,
}

fn body_points(ops: &[Op]) -> usize {
    ops.iter().map(|op| match op {
        Op::ItemPut { body: Body::Stroke { xy, .. }, .. } => xy.len() / 2,
        _ => 0,
    }).sum()
}

/// v5: `updateHistoryButtons()`.
pub fn update_buttons(app: &mut App, host: &mut dyn Host) {
    let ro = view::read_only(app);
    let st = &mut app.st.editor_history;
    let next = (ro || st.done == 0, ro || st.done >= st.steps.len());
    if st.buttons != Some(next) {
        st.buttons = Some(next);
        let out = host.out();
        out.prop(Ui::BTN_UNDO, "disabled", Prop::Bool(next.0));
        out.prop(Ui::BTN_REDO, "disabled", Prop::Bool(next.1));
    }
}

fn snapshot_order(app: &mut App) {
    let order = session::get(app).map(|s| s.doc.order().to_vec()).unwrap_or_default();
    app.st.editor_history.order = order;
}

/// Empties the stack (v5 `state.history = []; state.historyIndex = -1; updateHistoryButtons()`).
pub fn clear(app: &mut App, host: &mut dyn Host) {
    let st = &mut app.st.editor_history;
    st.steps.clear();
    st.done = 0;
    snapshot_order(app);
    update_buttons(app, host);
}

/// v5: `trimHistoryByMemory(target)`: drops the oldest steps over `target` points, keeping the newest.
fn trim(st: &mut State, target: usize) {
    let mut total: usize = st.steps.iter().map(|s| s.points).sum();
    let mut removed = 0;
    while st.steps.len() - removed > 1 && total > target {
        total -= st.steps[removed].points;
        removed += 1;
    }
    if removed > 0 {
        st.steps.drain(..removed);
        st.done = st.done.saturating_sub(removed);
    }
}

/// v5: `saveToHistory(action)`: one local action on `page`, `inverse` = the ops that undo it, `points` = the
/// points it added.
pub fn record(app: &mut App, host: &mut dyn Host, page: Id, inverse: Vec<Op>, points: usize) {
    if view::read_only(app) {
        return;
    }
    let st = &mut app.st.editor_history;
    st.steps.truncate(st.done);
    let points = points + body_points(&inverse);
    st.steps.push(Step { page, ops: inverse, points });
    if st.steps.len() > MAX_STEPS {
        st.steps.remove(0);
    } else {
        st.done += 1;
    }
    trim(st, MAX_POINTS);
    snapshot_order(app);
    update_buttons(app, host);
}

/// Applies step `i`'s ops as a new batch; the batch's inverse becomes the step's next ops.
fn apply(app: &mut App, host: &mut dyn Host, i: usize) {
    let page = app.st.editor_history.steps[i].page;
    if let Some(at) = pages::order(app).iter().position(|&id| id == page) {
        pages::set_active_page(app, at);
    }
    let ops = std::mem::take(&mut app.st.editor_history.steps[i].ops);
    let Some(e) = session::edit(app, host, |tx| ops.iter().for_each(|op| tx.push(op.clone()))) else {
        app.st.editor_history.steps[i].ops = ops;
        return;
    };
    if let Some(step) = app.st.editor_history.steps.get_mut(i) {
        step.ops = e.inverse;
    }
    let mut touched = e.changes.pages.clone();
    touched.extend(e.changes.page_props.iter().copied());
    ihn_core::sort::ord(&mut touched);
    touched.dedup();
    for id in touched {
        pages::touch(app, id);
        pages::request_repaint(app, host, id);
    }
    snapshot_order(app);
}

fn page_alive(app: &App, page: Id) -> bool { session::get(app).is_some_and(|s| s.doc.order().contains(&page)) }

/// v5: `undo()`.
pub fn undo(app: &mut App, host: &mut dyn Host) {
    if view::read_only(app) || app.st.editor_history.done == 0 {
        return;
    }
    app.st.editor_history.done -= 1;
    let i = app.st.editor_history.done;
    if page_alive(app, app.st.editor_history.steps[i].page) {
        apply(app, host, i);
    }
    update_buttons(app, host);
}

/// v5: `redo()`.
pub fn redo(app: &mut App, host: &mut dyn Host) {
    let st = &app.st.editor_history;
    if view::read_only(app) || st.done >= st.steps.len() {
        return;
    }
    let i = st.done;
    app.st.editor_history.done += 1;
    if page_alive(app, app.st.editor_history.steps[i].page) {
        apply(app, host, i);
    }
    update_buttons(app, host);
}

/// Steps and position (`(steps, applied)`).
pub fn depth(app: &App) -> (usize, usize) { (app.st.editor_history.steps.len(), app.st.editor_history.done) }

/// Called once at boot, in `app::MODULES` order, after the `Ui` table is bound.
pub fn init(app: &mut App, host: &mut dyn Host) {
    if app.cfg.has(crate::abi::mode::HEADLESS) {
        return;
    }
    let out = host.out();
    out.listen(Ui::BTN_UNDO, "click", L_UNDO, 0);
    out.listen(Ui::BTN_REDO, "click", L_REDO, 0);
    out.listen(Node::DOCUMENT, "keydown", L_KEY, 0);
}

/// Events addressed to this module (ids made with `abi::tag`) and broadcasts (`Life`, `Msg`). Returns `abi::ret` bits.
pub fn event(app: &mut App, host: &mut dyn Host, event: &Event) -> u32 {
    match *event {
        Event::Dom(e) if e.listener == L_UNDO && e.ty == ety::CLICK => {
            undo(app, host);
            0
        }
        Event::Dom(e) if e.listener == L_REDO && e.ty == ety::CLICK => {
            redo(app, host);
            0
        }
        // v5: Ctrl+z / Ctrl+y (lowercase keys, Ctrl only), not in Read mode.
        Event::Dom(e) if e.listener == L_KEY => {
            if view::read_only(app) || !e.has(ev::CTRL) {
                return 0;
            }
            match e.key {
                "z" => {
                    undo(app, host);
                    ret::PREVENT_DEFAULT
                }
                "y" => {
                    redo(app, host);
                    ret::PREVENT_DEFAULT
                }
                _ => 0,
            }
        }
        // v5: releaseMemoryOnHide trims the stack to half its budget.
        Event::Life(l) if l.code == life::VISIBILITY && l.a != 0.0 => {
            trim(&mut app.st.editor_history, MAX_POINTS / 2);
            update_buttons(app, host);
            0
        }
        Event::Msg(m) => {
            let data = std::str::from_utf8(m.data).unwrap_or("");
            match m.chan {
                session::sig::OPENED if !data.is_empty() && data == session::doc_id(app) => clear(app, host),
                session::sig::CLOSED => clear(app, host),
                // A local edit (any module): the page order it left is the reference.
                session::sig::STATE => snapshot_order(app),
                session::sig::CHANGED => {
                    let same = session::get(app).is_none_or(|s| s.doc.order() == app.st.editor_history.order.as_slice());
                    if !same {
                        clear(app, host);
                    }
                }
                super::view::sig::MODE => {
                    app.st.editor_history.buttons = None;
                    update_buttons(app, host);
                }
                _ => {}
            }
            0
        }
        _ => 0,
    }
}

/// `__IHN_TEST_API__.cmd('editor.history.state')` → `{steps, done, canUndo, canRedo}`.
pub fn test_cmd(app: &mut App, _host: &mut dyn Host, name: &str, _args: &str) -> Option<String> {
    match name {
        "state" => {
            let st = &app.st.editor_history;
            Some(format!("{{\"steps\":{},\"done\":{},\"canUndo\":{},\"canRedo\":{}}}", st.steps.len(), st.done, st.done > 0, st.done < st.steps.len()))
        }
        _ => None,
    }
}
