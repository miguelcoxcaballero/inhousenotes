//! DOM op builder (ARCHITECTURE §3.3 family 1, §3.6). Every method appends one record that `host/dom.js` applies
//! in order. Ops on a handle whose node is missing (never bound, removed in app mode) are ignored by the host, like
//! 5.18.0's `element?.…`.
//!
//! Numbers that end up in DOM strings must be formatted with `ihn_core::jsfmt` by the caller, so `outerHTML`
//! matches 5.18.0.

use crate::abi::{dom_op as op, fam, Node, OutBuf, LONG_STYLE, SAME_LONG_STYLE};

/// A value for [`OutBuf::prop`].
#[derive(Clone, Copy, Debug)]
pub enum Prop<'a> {
    Str(&'a str),
    Num(f64),
    Bool(bool),
}

/// `scroll_to` behaviour.
#[derive(Clone, Copy, Debug)]
#[repr(u8)]
pub enum Scroll {
    /// No `behavior` member (the CSS `scroll-behavior` decides).
    Auto = 0,
    Smooth = 1,
    Instant = 2,
}

/// `dialog` actions.
#[derive(Clone, Copy, Debug)]
#[repr(u8)]
pub enum Dialog {
    Show = 0,
    ShowModal = 1,
    Close = 2,
}

/// Record layouts for `abi::dump` (h/u = u32, i = i32, b = u8, f = f64, s = str, o = nullable str, v = prop value,
/// x = bytes).
#[cfg(test)]
pub const LAYOUT: &[(u8, &str, &str)] = &[
    (op::BIND, "bind", "hs"),
    (op::QUERY, "query", "hhs"),
    (op::CREATE, "create", "hs"),
    (op::INSERT_HTML, "insert_html", "hhhs"),
    (op::CHILD, "child", "hhx"),
    (op::INSERT, "insert", "hhh"),
    (op::REMOVE, "remove", "h"),
    (op::RELEASE, "release", "h"),
    (op::ATTR, "attr", "hso"),
    (op::PROP, "prop", "hsv"),
    (op::TEXT, "text", "hs"),
    (op::CLASS, "class", "hsb"),
    (op::CLASS_SET, "class_set", "hs"),
    (op::STYLE, "style", "hso"),
    (op::LISTEN, "listen", "hsub"),
    (op::UNLISTEN, "unlisten", "u"),
    (op::FOCUS, "focus", "hb"),
    (op::BLUR, "blur", "h"),
    (op::SCROLL_TO, "scroll_to", "hffb"),
    (op::ANIMATE, "animate", "hhss"),
    (op::ANIM_CANCEL, "anim_cancel", "h"),
    (op::ANIM_FINISH, "anim_finish", "h"),
    (op::POINTER_CAPTURE, "pointer_capture", "hib"),
    (op::DIALOG, "dialog", "hb"),
    (op::CLICK, "click", "h"),
    (op::HTML, "html", "hs"),
    (op::SELECT, "select", "h"),
    (op::OBSERVE, "observe", "hubs"),
    (op::UNOBSERVE, "unobserve", "u"),
];

impl OutBuf {
    fn dom(&mut self, o: u8, h: Node) -> usize {
        let m = self.w.rec(fam::DOM, o);
        self.w.u32(h.0);
        m
    }
    fn dom_h(&mut self, o: u8, h: Node) {
        let m = self.dom(o, h);
        self.w.end(m);
    }
    fn dom_s(&mut self, o: u8, h: Node, s: &str) {
        let m = self.dom(o, h);
        self.w.str(s);
        self.w.end(m);
    }

    /// `document.getElementById(id)` into the fixed handle `h` (used for the `Ui` table).
    pub fn bind_at(&mut self, h: Node, id: &str) { self.dom_s(op::BIND, h, id); }
    /// `document.getElementById(id)`.
    pub fn bind(&mut self, id: &str) -> Node {
        let h = self.node();
        self.bind_at(h, id);
        h
    }
    /// `root.querySelector(selector)` (`Node::DOCUMENT` for the whole document).
    pub fn query(&mut self, root: Node, selector: &str) -> Node {
        let h = self.node();
        let m = self.dom(op::QUERY, h);
        self.w.u32(root.0);
        self.w.str(selector);
        self.w.end(m);
        h
    }
    /// `document.createElement(tag)`.
    pub fn create(&mut self, tag: &str) -> Node {
        let h = self.node();
        self.dom_s(op::CREATE, h, tag);
        h
    }
    /// Parses `html` and inserts its nodes into `parent` before `before` (`Node::NONE` = append). Returns a handle
    /// to the first inserted element.
    pub fn insert_html(&mut self, parent: Node, before: Node, html: &str) -> Node {
        let h = self.node();
        self.insert_html_at(h, parent, before, html);
        h
    }
    /// [`OutBuf::insert_html`] without keeping a handle.
    pub fn append_html(&mut self, parent: Node, html: &str) { self.insert_html_at(Node::NONE, parent, Node::NONE, html); }
    fn insert_html_at(&mut self, h: Node, parent: Node, before: Node, html: &str) {
        let m = self.dom(op::INSERT_HTML, h);
        self.w.u32(parent.0);
        self.w.u32(before.0);
        self.w.str(html);
        self.w.end(m);
    }
    /// The descendant reached from `root` by following `children[i]` for each index of `path`.
    pub fn child(&mut self, root: Node, path: &[u8]) -> Node {
        let h = self.node();
        let m = self.dom(op::CHILD, h);
        self.w.u32(root.0);
        self.w.bytes(path);
        self.w.end(m);
        h
    }
    /// `parent.insertBefore(h, before)` (`Node::NONE` = append).
    pub fn insert(&mut self, parent: Node, before: Node, h: Node) {
        let m = self.dom(op::INSERT, parent);
        self.w.u32(before.0);
        self.w.u32(h.0);
        self.w.end(m);
    }
    /// `h.remove()`; the handle stays valid (the node can be inserted again).
    pub fn remove(&mut self, h: Node) { self.dom_h(op::REMOVE, h); }
    /// Forgets the handle (the node itself is untouched); its listeners stay attached.
    pub fn release(&mut self, h: Node) {
        self.dom_h(op::RELEASE, h);
        self.recycle(h);
    }
    /// `setAttribute(name, value)`, or `removeAttribute(name)` for `None`.
    pub fn attr(&mut self, h: Node, name: &str, value: Option<&str>) {
        let m = self.dom(op::ATTR, h);
        self.w.str(name);
        self.w.opt_str(value);
        self.w.end(m);
    }
    /// `h[name] = value` (`value`, `checked`, `disabled`, `hidden`, `scrollTop`, `src`, …).
    pub fn prop(&mut self, h: Node, name: &str, value: Prop) {
        let m = self.dom(op::PROP, h);
        self.w.str(name);
        match value {
            Prop::Str(s) => {
                self.w.u8(0);
                self.w.str(s);
            }
            Prop::Num(n) => {
                self.w.u8(1);
                self.w.f64(n);
            }
            Prop::Bool(b) => {
                self.w.u8(2);
                self.w.u8(b as u8);
            }
        }
        self.w.end(m);
    }
    /// `h.textContent = s`.
    pub fn text(&mut self, h: Node, s: &str) { self.dom_s(op::TEXT, h, s); }
    /// `h.innerHTML = html`.
    pub fn html(&mut self, h: Node, html: &str) { self.dom_s(op::HTML, h, html); }
    /// `h.classList.toggle(name, on)`.
    pub fn class(&mut self, h: Node, name: &str, on: bool) {
        let m = self.dom(op::CLASS, h);
        self.w.str(name);
        self.w.u8(on as u8);
        self.w.end(m);
    }
    /// `h.className = s`.
    pub fn class_set(&mut self, h: Node, s: &str) { self.dom_s(op::CLASS_SET, h, s); }
    /// `h.style[prop] = value` for camelCase names, `h.style.setProperty(prop, value)` for names containing `-`
    /// (custom properties included); `None` clears the property.
    pub fn style(&mut self, h: Node, prop: &str, value: Option<&str>) {
        let m = self.dom(op::STYLE, h);
        self.w.str(prop);
        match value {
            Some(v) if v.len() >= LONG_STYLE && self.long_style == v.as_bytes() => self.w.u32(SAME_LONG_STYLE),
            Some(v) if v.len() >= LONG_STYLE => {
                self.long_style.clear();
                self.long_style.extend_from_slice(v.as_bytes());
                self.w.str(v);
            }
            _ => self.w.opt_str(value),
        }
        self.w.end(m);
    }
    /// `h.addEventListener(event, …, flags)`; events arrive as `Event::Dom` (or `Event::Pointer` for `pointer*`
    /// types) carrying `listener`, which must be made with `abi::tag(module, n)` and be unique while attached.
    pub fn listen(&mut self, h: Node, event: &str, listener: u32, flags: u8) {
        let m = self.dom(op::LISTEN, h);
        self.w.str(event);
        self.w.u32(listener);
        self.w.u8(flags);
        self.w.end(m);
    }
    /// Removes the listener registered under `listener`.
    pub fn unlisten(&mut self, listener: u32) {
        let m = self.w.rec(fam::DOM, op::UNLISTEN);
        self.w.u32(listener);
        self.w.end(m);
    }
    /// `h.focus({preventScroll})`.
    pub fn focus(&mut self, h: Node, prevent_scroll: bool) {
        let m = self.dom(op::FOCUS, h);
        self.w.u8(prevent_scroll as u8);
        self.w.end(m);
    }
    /// `h.blur()`.
    pub fn blur(&mut self, h: Node) { self.dom_h(op::BLUR, h); }
    /// `h.select()` (text inputs).
    pub fn select(&mut self, h: Node) { self.dom_h(op::SELECT, h); }
    /// `h.scrollTo({left: x, top: y, behavior})`; a NaN coordinate is left out.
    pub fn scroll_to(&mut self, h: Node, x: f64, y: f64, behavior: Scroll) {
        let m = self.dom(op::SCROLL_TO, h);
        self.w.f64(x);
        self.w.f64(y);
        self.w.u8(behavior as u8);
        self.w.end(m);
    }
    /// `h.animate(JSON.parse(keyframes), JSON.parse(options))`. The returned handle is the `Animation`: it accepts
    /// `listen(.., "finish", ..)`, [`OutBuf::anim_cancel`], [`OutBuf::anim_finish`] and [`OutBuf::release`].
    pub fn animate(&mut self, h: Node, keyframes_json: &str, options_json: &str) -> Node {
        let a = self.node();
        let m = self.dom(op::ANIMATE, a);
        self.w.u32(h.0);
        self.w.str(keyframes_json);
        self.w.str(options_json);
        self.w.end(m);
        a
    }
    pub fn anim_cancel(&mut self, anim: Node) { self.dom_h(op::ANIM_CANCEL, anim); }
    pub fn anim_finish(&mut self, anim: Node) { self.dom_h(op::ANIM_FINISH, anim); }
    /// `h.setPointerCapture(id)` / `h.releasePointerCapture(id)`; failures are swallowed as in 5.18.0.
    pub fn pointer_capture(&mut self, h: Node, pointer_id: i32, on: bool) {
        let m = self.dom(op::POINTER_CAPTURE, h);
        self.w.i32(pointer_id);
        self.w.u8(on as u8);
        self.w.end(m);
    }
    /// `<dialog>` `show()` / `showModal()` / `close()`.
    pub fn dialog(&mut self, h: Node, action: Dialog) {
        let m = self.dom(op::DIALOG, h);
        self.w.u8(action as u8);
        self.w.end(m);
    }
    /// `h.click()` (file inputs, download anchors).
    pub fn click(&mut self, h: Node) { self.dom_h(op::CLICK, h); }
    /// Adds `h` to the observer registered under `listener` (created on first use): a `ResizeObserver` for
    /// `what == abi::observe::RESIZE`, else a `MutationObserver` with those `abi::observe` options and the
    /// comma-separated `attribute_filter` (empty = none). Each callback arrives as one `Event::Dom` with
    /// `ty == ety::RESIZE_OBSERVED` / `MUTATION_OBSERVED`, `node` = the first node observed and `path` = the
    /// target of the first entry. `listener` follows the rules of [`OutBuf::listen`].
    pub fn observe(&mut self, h: Node, listener: u32, what: u8, attribute_filter: &str) {
        let m = self.dom(op::OBSERVE, h);
        self.w.u32(listener);
        self.w.u8(what);
        self.w.str(attribute_filter);
        self.w.end(m);
    }
    /// Disconnects the observer registered under `listener`.
    pub fn unobserve(&mut self, listener: u32) {
        let m = self.w.rec(fam::DOM, op::UNOBSERVE);
        self.w.u32(listener);
        self.w.end(m);
    }
}
