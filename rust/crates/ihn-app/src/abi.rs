//! ABI v6 between `ihn.wasm` and `host/*.js` (ARCHITECTURE §3.2–3.5). This file is the frozen wire contract:
//! `host/main.js` and `host/dom.js` mirror the numbers below, and e2e `00-boot` runs `app.selftest` against them.
//!
//! # Out-buffer (Rust → JS)
//! One ordered stream of records `[u8 family][u8 op][u32 len][payload]`, little-endian. Strings are UTF-8 with a
//! u32 length (`u32::MAX` = null where a null is allowed), numbers are f64 unless noted, handles are u32.
//! Operations that produce a handle carry the new handle (chosen by Rust) as their first field.
//!
//! | Family | Ops | Executor |
//! |---|---|---|
//! | 1 DOM | [`dom_op`] | `host/dom.js` |
//! | 2 Canvas | owned by `ihn-core::render::cmd` (W2.3) | `host/canvas.js` |
//! | 3 Effects | [`fx`]: `0x00..0x10` core, `0x10..0x20` timers/rAF/idle (3.1), `0x20..0x30` IndexedDB (3.2), `0x30..0x50` fetch/blob (3.3), `0x50..0x60` pdf (3.4), `0x60..0x70` peer (3.5), `0x70..0x78` worker (3.7), `0x78..0x80` crypto (3.8), `0x80..0x90` Web Locks (3.9), `0x90..0xa0` deflate (3.10), `0xa0..0xe0` platform (3.6) | `host/main.js` and the module of that range |
//!
//! Asynchronous effects carry a u32 request id made with [`tag`]; the completion arrives as an [`Event::Io`] routed
//! to the module encoded in the id.
//!
//! # Events (JS → Rust)
//! `ihn_event(kind, ptr, len) -> u32`; the result is a set of [`ret`] bits. Payload layouts are the `decode_*`
//! functions below (`encodeDom`/`encodePointer` in `host/dom.js`).
//!
//! # Synchronous reads
//! The nine imports are listed in `ihn-wasm`. Layout numbers go through `host_metric` ([`crate::host::Metric`]);
//! computed style, media queries, hit testing, the focused element and pointer capture go through the text channel
//! of `host_intl` ([`crate::host::Ask`], ops `16..`).

use crate::app::Mod;
use ihn_core::codec::{Reader, Writer};

pub const IHN_ABI: u32 = 6;

/// Out-buffer families.
pub mod fam {
    pub const DOM: u8 = 1;
    pub const CANVAS: u8 = 2;
    pub const FX: u8 = 3;
}

/// DOM family ops. Payloads are documented on the builder methods in `dom.rs`.
pub mod dom_op {
    pub const BIND: u8 = 1;
    pub const QUERY: u8 = 2;
    pub const CREATE: u8 = 3;
    pub const INSERT_HTML: u8 = 4;
    pub const CHILD: u8 = 5;
    pub const INSERT: u8 = 6;
    pub const REMOVE: u8 = 7;
    pub const RELEASE: u8 = 8;
    pub const ATTR: u8 = 9;
    pub const PROP: u8 = 10;
    pub const TEXT: u8 = 11;
    pub const CLASS: u8 = 12;
    pub const CLASS_SET: u8 = 13;
    pub const STYLE: u8 = 14;
    pub const LISTEN: u8 = 15;
    pub const UNLISTEN: u8 = 16;
    pub const FOCUS: u8 = 17;
    pub const BLUR: u8 = 18;
    pub const SCROLL_TO: u8 = 19;
    pub const ANIMATE: u8 = 20;
    pub const ANIM_CANCEL: u8 = 21;
    pub const ANIM_FINISH: u8 = 22;
    pub const POINTER_CAPTURE: u8 = 23;
    pub const DIALOG: u8 = 24;
    pub const CLICK: u8 = 25;
    pub const HTML: u8 = 26;
    pub const SELECT: u8 = 27;
    pub const OBSERVE: u8 = 28;
    pub const UNOBSERVE: u8 = 29;
    pub const ANIM_SET: u8 = 30;
}

/// `observe` kinds: [`observe::RESIZE`] alone, or a set of the `MutationObserver` options.
pub mod observe {
    /// `ResizeObserver`.
    pub const RESIZE: u8 = 0;
    pub const ATTRIBUTES: u8 = 1;
    pub const CHILD_LIST: u8 = 2;
    pub const SUBTREE: u8 = 4;
    pub const CHARACTER_DATA: u8 = 8;
}

macro_rules! event_types {
    ($($name:ident $js:literal $code:literal;)*) => {
        /// [`DomEvent::ty`]: the `event.type` code (`DOM_TYPE` in `host/dom.js`; a cargo test compares the two
        /// tables). A type that is not listed arrives as [`ety::OTHER`]; `pointer*` types have [`ptr`] codes.
        pub mod ety {
            pub const OTHER: u8 = 0;
            $(pub const $name: u8 = $code;)*
            /// A `ResizeObserver` callback ([`crate::abi::OutBuf::observe`]).
            pub const RESIZE_OBSERVED: u8 = 64;
            /// A `MutationObserver` callback.
            pub const MUTATION_OBSERVED: u8 = 65;
            #[cfg(test)]
            pub const NAMES: &[(&str, u8)] = &[$(($js, $code)),*];
        }
    };
}

event_types! {
    CLICK "click" 1;
    DBLCLICK "dblclick" 2;
    KEYDOWN "keydown" 3;
    KEYUP "keyup" 4;
    INPUT "input" 5;
    CHANGE "change" 6;
    SUBMIT "submit" 7;
    FOCUS "focus" 8;
    BLUR "blur" 9;
    FOCUSIN "focusin" 10;
    FOCUSOUT "focusout" 11;
    SCROLL "scroll" 12;
    WHEEL "wheel" 13;
    TOUCHSTART "touchstart" 14;
    TOUCHMOVE "touchmove" 15;
    TOUCHEND "touchend" 16;
    TOUCHCANCEL "touchcancel" 17;
    DRAGENTER "dragenter" 18;
    DRAGOVER "dragover" 19;
    DRAGLEAVE "dragleave" 20;
    DROP "drop" 21;
    PASTE "paste" 22;
    COPY "copy" 23;
    CUT "cut" 24;
    ANIMATIONEND "animationend" 25;
    TRANSITIONEND "transitionend" 26;
    LOAD "load" 27;
    ERROR "error" 28;
    ABORT "abort" 29;
    FINISH "finish" 30;
    CANCEL "cancel" 31;
    CLOSE "close" 32;
    CONTEXTMENU "contextmenu" 33;
    MOUSEDOWN "mousedown" 34;
    MOUSEUP "mouseup" 35;
    MOUSEMOVE "mousemove" 36;
    MOUSEENTER "mouseenter" 37;
    MOUSELEAVE "mouseleave" 38;
    RESIZE "resize" 39;
    SCROLLEND "scrollend" 40;
    TOGGLE "toggle" 41;
    SELECTIONCHANGE "selectionchange" 42;
    BEFOREINPUT "beforeinput" 43;
    COMPOSITIONEND "compositionend" 44;
    READY "ready" 45; // an Animation's `ready` promise (listen with "ready" on an animation handle; delivered once)
}

/// Effect family ops implemented by `host/main.js` (core, 3.1 and 3.7). Other ranges belong to their executors.
pub mod fx {
    /// `obj_release(u32 obj)`: drop a JS object handle received in an event (Blob, File list, …).
    pub const OBJ_RELEASE: u8 = 0x01;
    /// `test_result(str json)`: the answer to the `TEST` event being handled.
    pub const TEST_RESULT: u8 = 0x02;
    /// `ready()`: boot finished; `__IHN_TEST_API__.ready()` resolves.
    pub const READY: u8 = 0x03;
    /// `timer_set(u32 id, f64 ms, u8 repeat)`: `setTimeout`/`setInterval`; setting a live id replaces it.
    pub const TIMER_SET: u8 = 0x10;
    /// `timer_clear(u32 id)`.
    pub const TIMER_CLEAR: u8 = 0x11;
    /// `raf_request()`: one `requestAnimationFrame` → `ihn_frame(t)`. Idempotent until the frame runs.
    pub const RAF_REQUEST: u8 = 0x12;
    /// `raf_cancel()`.
    pub const RAF_CANCEL: u8 = 0x13;
    /// `idle_request(u32 id, f64 timeout_ms)`: `requestIdleCallback` (`timeout <= 0`: none) → `IDLE` event.
    pub const IDLE_REQUEST: u8 = 0x14;
    /// `idle_cancel(u32 id)`.
    pub const IDLE_CANCEL: u8 = 0x15;
    /// `worker_start()`: create the export worker (`host/export-worker.js`) and hand it the compiled module.
    pub const WORKER_START: u8 = 0x70;
    /// `worker_post(u32 obj, bytes)`: post `{bytes, obj}` (obj = a JS object handle or 0); bytes are transferred.
    pub const WORKER_POST: u8 = 0x71;
    /// `worker_stop()`: terminate the worker.
    pub const WORKER_STOP: u8 = 0x72;
}

/// `ihn_event` kinds.
pub mod kind {
    pub const DOM: u32 = 1;
    pub const POINTER: u32 = 2;
    pub const TIMER: u32 = 4;
    pub const IDLE: u32 = 5;
    pub const IO: u32 = 6;
    pub const LIFECYCLE: u32 = 7;
    pub const MESSAGE: u32 = 8;
    pub const TEST: u32 = 9;
}

/// `ihn_event` result bits.
pub mod ret {
    pub const PREVENT_DEFAULT: u32 = 1;
    pub const STOP_PROPAGATION: u32 = 2;
    pub const STOP_IMMEDIATE: u32 = 4;
    /// `event.dataTransfer.dropEffect = 'copy'` (drag events).
    pub const DROP_COPY: u32 = 8;
}

/// `listen` flags.
pub mod listen {
    pub const CAPTURE: u8 = 1;
    pub const PASSIVE: u8 = 2;
    pub const ONCE: u8 = 4;
    /// `{passive:false}` given explicitly (wheel/touch listeners that call `preventDefault`).
    pub const ACTIVE: u8 = 8;
}

/// [`Config::flags`] (`ihn_init`).
pub mod mode {
    pub const APP: u32 = 1;
    pub const HEADLESS: u32 = 2;
    pub const E2E: u32 = 4;
    pub const WORKER: u32 = 8;
    pub const ONLINE: u32 = 16;
    pub const DARK_SCHEME: u32 = 32;
    pub const HIDDEN: u32 = 64;
    /// `prefers-reduced-motion: reduce` matches.
    pub const REDUCED_MOTION: u32 = 128;
}

/// [`DomEvent::flags`] and [`PointerBatch::flags`].
pub mod ev {
    pub const CTRL: u32 = 1;
    pub const SHIFT: u32 = 2;
    pub const ALT: u32 = 4;
    pub const META: u32 = 8;
    pub const COMPOSING: u32 = 16;
    pub const REPEAT: u32 = 32;
    pub const CHECKED: u32 = 64;
    pub const TRUSTED: u32 = 128;
    pub const PRIMARY: u32 = 256;
    /// `event.target` is itself `path[0]` (not merely a descendant of it).
    pub const EXACT: u32 = 512;
    /// `event.dataTransfer.types` (or `clipboardData.types` of a paste) includes `'Files'`.
    pub const DT_FILES: u32 = 1024;
}

/// [`PointerBatch::kind`].
pub mod ptr {
    pub const DOWN: u8 = 1;
    pub const MOVE: u8 = 2;
    pub const UP: u8 = 3;
    pub const CANCEL: u8 = 4;
    pub const LEAVE: u8 = 5;
    pub const ENTER: u8 = 6;
    pub const OVER: u8 = 7;
    pub const OUT: u8 = 8;
    pub const RAW_UPDATE: u8 = 9;
    pub const LOST_CAPTURE: u8 = 10;
    pub const GOT_CAPTURE: u8 = 11;
    /// [`super::PointerBatch::ptype`] / [`super::DomEvent::ptype`].
    pub const MOUSE: u8 = 1;
    pub const PEN: u8 = 2;
    pub const TOUCH: u8 = 3;
}

/// [`Life::code`].
pub mod life {
    /// `a` = 1 when hidden.
    pub const VISIBILITY: u8 = 1;
    /// `a` = `persisted`.
    pub const PAGEHIDE: u8 = 2;
    /// Result bit 0 ⇒ the host sets `returnValue = ''`.
    pub const BEFOREUNLOAD: u8 = 3;
    pub const ONLINE: u8 = 4;
    pub const OFFLINE: u8 = 5;
    /// `a`, `b` = `innerWidth`, `innerHeight`; `c` = `devicePixelRatio`.
    pub const RESIZE: u8 = 6;
    /// `a` = 1 while an element is fullscreen.
    pub const FULLSCREEN: u8 = 7;
    /// `a` = 1 when `prefers-color-scheme: dark` matches.
    pub const SCHEME: u8 = 8;
    pub const BLUR: u8 = 9;
    pub const FOCUS: u8 = 10;
    /// `a` = `persisted`.
    pub const PAGESHOW: u8 = 11;
    /// `s` = the changed `localStorage` key (another tab wrote it).
    pub const STORAGE: u8 = 12;
    /// Android `resume` document event.
    pub const RESUME: u8 = 13;
    /// `a` = 1 when `prefers-reduced-motion: reduce` matches.
    pub const MOTION: u8 = 14;
}

/// [`Msg::chan`].
pub mod chan {
    /// `window.handleInhouseNativeOAuth(payload)`.
    pub const OAUTH: u8 = 1;
    /// `window.handleInhousePdfExportResult(result)` (JSON).
    pub const PDF_EXPORT: u8 = 2;
    /// A message from the export worker (`{bytes, obj}`).
    pub const WORKER: u8 = 3;
    /// The export worker failed; data = the error message.
    pub const WORKER_ERROR: u8 = 4;
    /// A message from the service worker (JSON).
    pub const SERVICE_WORKER: u8 = 5;
    /// Peer / BroadcastChannel traffic (SYNC.md, `host/peer.js`).
    pub const PEER: u8 = 6;
    /// Free for `host/platform.js` (paste data, GIS callbacks, …): `0x10..`.
    pub const PLATFORM: u8 = 0x10;
}

/// First handle given out by [`OutBuf::node`]; lower handles are fixed (`Node::DOCUMENT`…, `Ui::*`).
pub const FIRST_DYNAMIC_NODE: u32 = 1024;

/// Capacity of the scratch buffer of `host_intl` (input and result share it).
pub const INTL_CAP: usize = 1024;

/// A DOM node (or other `EventTarget`, e.g. an `Animation`) held by `host/dom.js`. `0` is "none".
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Node(pub u32);

impl Node {
    pub const NONE: Node = Node(0);
    pub const DOCUMENT: Node = Node(1);
    pub const WINDOW: Node = Node(2);
    pub const HTML: Node = Node(3);
    pub const BODY: Node = Node(4);
    pub fn is_none(self) -> bool { self.0 == 0 }
}

/// Builds a listener, timer or request id owned by module `m`: high byte = module, low 24 bits = `local`.
pub const fn tag(m: Mod, local: u32) -> u32 { (m as u32) << 24 | (local & 0x00ff_ffff) }
/// The module index encoded in an id made by [`tag`].
pub const fn tag_mod(id: u32) -> usize { (id >> 24) as usize }
/// The module-local part of an id made by [`tag`].
pub const fn tag_local(id: u32) -> u32 { id & 0x00ff_ffff }

/// The out-buffer: records for `host/main.js` plus the DOM handle allocator.
pub struct OutBuf {
    pub w: Writer,
    next_node: u32,
    free_nodes: Vec<u32>,
    /// The last style value of at least [`LONG_STYLE`] bytes written (`dom::style`); an identical one is sent as
    /// [`SAME_LONG_STYLE`] instead of its bytes.
    pub(crate) long_style: Vec<u8>,
}

/// Payloads from this size on are reserved exactly ([`OutBuf::room`]).
pub const BIG_PAYLOAD: usize = 64 << 10;
/// Style values this long (template backgrounds: ~350 KB JPEG data URLs, set on every page) are sent once.
pub const LONG_STYLE: usize = 16 << 10;
/// In place of a STYLE record's value length: "the last long value again" (`host/dom.js` keeps it).
pub const SAME_LONG_STYLE: u32 = 0xffff_fffe;

impl Default for OutBuf {
    fn default() -> Self { OutBuf { w: Writer::default(), next_node: FIRST_DYNAMIC_NODE, free_nodes: Vec::new(), long_style: Vec::new() } }
}

impl OutBuf {
    /// A fresh handle for an op that creates or looks up a node.
    pub fn node(&mut self) -> Node {
        Node(self.free_nodes.pop().unwrap_or_else(|| {
            self.next_node += 1;
            self.next_node - 1
        }))
    }
    pub(crate) fn recycle(&mut self, h: Node) {
        if h.0 >= FIRST_DYNAMIC_NODE {
            self.free_nodes.push(h.0);
        }
    }
    /// An effect record without payload.
    pub fn fx0(&mut self, op: u8) {
        let m = self.w.rec(fam::FX, op);
        self.w.end(m);
    }
    /// An effect record whose payload is one u32 (an id or handle).
    pub fn fx_id(&mut self, op: u8, id: u32) {
        let m = self.w.rec(fam::FX, op);
        self.w.u32(id);
        self.w.end(m);
    }
    /// Makes room for a payload of `n` more bytes. A large one (≥ [`BIG_PAYLOAD`]: a store transaction, a blob, a
    /// worker message) gets exactly that room plus a little slack instead of `Vec`'s doubling, which would keep up to
    /// twice the payload reserved in wasm memory (it never shrinks).
    pub fn room(&mut self, n: usize) {
        let buf = &mut self.w.buf;
        if n >= BIG_PAYLOAD && buf.capacity() - buf.len() < n + 64 {
            buf.reserve_exact(n + (16 << 10));
        }
    }
    /// Gives back the capacity a large payload left behind once the host applied it (the out-buffer is empty at the
    /// start of every entry): wasm memory then reuses it instead of keeping it reserved for the out-buffer.
    pub fn trim(&mut self) {
        let buf = &mut self.w.buf;
        if buf.is_empty() && buf.capacity() > BIG_PAYLOAD * 4 {
            buf.shrink_to(BIG_PAYLOAD);
        }
    }
    /// Releases a JS object handle received in an event.
    pub fn obj_release(&mut self, obj: u32) {
        if obj != 0 {
            self.fx_id(fx::OBJ_RELEASE, obj);
        }
    }
}

/// `ihn_init` payload: `str version, str url, str ua, str lang, u32 flags, f64 vw, f64 vh, f64 dpr, u32 touch`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Config {
    /// `APP_VERSION` from the inert block of `index.html`.
    pub version: String,
    pub url: String,
    pub user_agent: String,
    pub language: String,
    /// [`mode`] bits.
    pub flags: u32,
    pub viewport: (f64, f64),
    pub dpr: f64,
    pub max_touch_points: u32,
}

impl Config {
    pub fn decode(bytes: &[u8]) -> Option<Config> {
        let mut r = Reader::new(bytes);
        Some(Config {
            version: r.str()?.into(),
            url: r.str()?.into(),
            user_agent: r.str()?.into(),
            language: r.str()?.into(),
            flags: r.u32()?,
            viewport: (r.f64()?, r.f64()?),
            dpr: r.f64()?,
            max_touch_points: r.u32()?,
        })
    }
    pub fn has(&self, flag: u32) -> bool { self.flags & flag != 0 }
}

/// A list of u32 handles borrowed from an event payload.
#[derive(Clone, Copy, Default)]
pub struct Handles<'a>(&'a [u8]);

impl<'a> Handles<'a> {
    pub fn len(&self) -> usize { self.0.len() / 4 }
    pub fn is_empty(&self) -> bool { self.0.is_empty() }
    pub fn iter(&self) -> impl Iterator<Item = Node> + 'a {
        self.0.chunks_exact(4).map(|c| Node(u32::from_le_bytes([c[0], c[1], c[2], c[3]])))
    }
    /// Nearest known ancestor-or-self of the event target.
    pub fn first(&self) -> Node { self.iter().next().unwrap_or(Node::NONE) }
    /// `node.contains(event.target)` for a node Rust holds a handle to.
    pub fn contains(&self, node: Node) -> bool { self.iter().any(|h| h == node) }
}

fn handles<'a>(r: &mut Reader<'a>) -> Option<Handles<'a>> {
    let n = r.u32()? as usize;
    r.take(n.checked_mul(4)?).map(Handles)
}

/// One touch point of a touch event.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Touch {
    pub id: i32,
    pub x: f64,
    pub y: f64,
}

/// `event.touches` borrowed from the payload (20 bytes each: `i32 id, f64 clientX, f64 clientY`).
#[derive(Clone, Copy, Default)]
pub struct Touches<'a>(&'a [u8]);

impl<'a> Touches<'a> {
    pub fn len(&self) -> usize { self.0.len() / 20 }
    pub fn is_empty(&self) -> bool { self.0.is_empty() }
    pub fn iter(&self) -> impl Iterator<Item = Touch> + 'a {
        self.0.chunks_exact(20).filter_map(|c| {
            let mut r = Reader::new(c);
            Some(Touch { id: r.i32()?, x: r.f64()?, y: r.f64()? })
        })
    }
}

/// A DOM event other than `pointer*` (kind `DOM`).
///
/// Payload: `u32 listener, u32 node, u8 type, u32 flags, f64 t, i32 button, u32 buttons, f64 x, f64 y, f64 dx,
/// f64 dy, u32 delta_mode, i32 detail, u8 ptype, u32 obj, str key, str code, str value, u32 n + n×u32 path,
/// u32 n + n×u32 related path, u32 n + n×20 touches, u32 n + n×20 changed touches`.
#[derive(Clone, Copy)]
pub struct DomEvent<'a> {
    /// The id given to `listen`.
    pub listener: u32,
    /// The node the listener is attached to (`currentTarget`); for observer callbacks, the first node observed.
    pub node: Node,
    /// [`ety`] code of `event.type`.
    pub ty: u8,
    /// [`ev`] bits.
    pub flags: u32,
    /// `event.timeStamp` (the clock of `Host::now`).
    pub t: f64,
    pub button: i32,
    pub buttons: u32,
    /// `clientX` / `clientY`.
    pub x: f64,
    pub y: f64,
    /// `deltaX` / `deltaY` (wheel).
    pub dx: f64,
    pub dy: f64,
    pub delta_mode: u32,
    pub detail: i32,
    /// [`ptr::MOUSE`]… for clicks that are pointer events, else 0.
    pub ptype: u8,
    /// JS object handle: `files` of a file input or drop, `clipboardData` of a paste, else 0. Release it with
    /// [`OutBuf::obj_release`].
    pub obj: u32,
    pub key: &'a str,
    pub code: &'a str,
    /// `value` of the target (inputs, selects, textareas); `animationName` / `propertyName` for animation and
    /// transition events.
    pub value: &'a str,
    /// Known nodes from `event.target` upwards (nearest first).
    pub path: Handles<'a>,
    /// Known nodes from `event.relatedTarget` upwards (empty when there is none): `related.contains(node)` is
    /// `node.contains(event.relatedTarget)`.
    pub related: Handles<'a>,
    pub touches: Touches<'a>,
    pub changed_touches: Touches<'a>,
}

impl<'a> DomEvent<'a> {
    pub fn decode(bytes: &'a [u8]) -> Option<Self> {
        let mut r = Reader::new(bytes);
        Some(DomEvent {
            listener: r.u32()?,
            node: Node(r.u32()?),
            ty: r.u8()?,
            flags: r.u32()?,
            t: r.f64()?,
            button: r.i32()?,
            buttons: r.u32()?,
            x: r.f64()?,
            y: r.f64()?,
            dx: r.f64()?,
            dy: r.f64()?,
            delta_mode: r.u32()?,
            detail: r.i32()?,
            ptype: r.u8()?,
            obj: r.u32()?,
            key: r.str()?,
            code: r.str()?,
            value: r.str()?,
            path: handles(&mut r)?,
            related: handles(&mut r)?,
            touches: {
                let n = r.u32()? as usize;
                Touches(r.take(n.checked_mul(20)?)?)
            },
            changed_touches: {
                let n = r.u32()? as usize;
                Touches(r.take(n.checked_mul(20)?)?)
            },
        })
    }
    pub fn has(&self, flag: u32) -> bool { self.flags & flag != 0 }
}

/// One pointer sample (40 bytes: `f64 x, f64 y, f64 t, f32 pressure, f32 tiltX, f32 tiltY, u32 buttons`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Sample {
    /// `clientX` / `clientY`.
    pub x: f64,
    pub y: f64,
    /// `event.timeStamp`.
    pub t: f64,
    pub pressure: f32,
    pub tilt_x: f32,
    pub tilt_y: f32,
    pub buttons: u32,
}

fn sample(r: &mut Reader) -> Option<Sample> {
    Some(Sample { x: r.f64()?, y: r.f64()?, t: r.f64()?, pressure: r.f32()?, tilt_x: r.f32()?, tilt_y: r.f32()?, buttons: r.u32()? })
}

/// A `pointer*` event (kind `POINTER`).
///
/// Payload: `u32 listener, u32 node, u8 kind, u8 ptype, i32 pointer_id, i32 button, u32 flags, sample raw,
/// u32 n + n×sample coalesced, u32 n + n×u32 path` (path is empty for `pointermove`/`pointerrawupdate`).
#[derive(Clone, Copy)]
pub struct PointerBatch<'a> {
    pub listener: u32,
    pub node: Node,
    /// [`ptr::DOWN`]…
    pub kind: u8,
    /// [`ptr::MOUSE`], [`ptr::PEN`] or [`ptr::TOUCH`].
    pub ptype: u8,
    pub pointer_id: i32,
    pub button: i32,
    /// [`ev`] bits (modifiers, `PRIMARY`, `TRUSTED`, `EXACT`).
    pub flags: u32,
    /// The dispatched event itself.
    pub raw: Sample,
    coalesced: &'a [u8],
    pub path: Handles<'a>,
}

impl<'a> PointerBatch<'a> {
    pub fn decode(bytes: &'a [u8]) -> Option<Self> {
        let mut r = Reader::new(bytes);
        Some(PointerBatch {
            listener: r.u32()?,
            node: Node(r.u32()?),
            kind: r.u8()?,
            ptype: r.u8()?,
            pointer_id: r.i32()?,
            button: r.i32()?,
            flags: r.u32()?,
            raw: sample(&mut r)?,
            coalesced: {
                let n = r.u32()? as usize;
                r.take(n.checked_mul(40)?)?
            },
            path: handles(&mut r)?,
        })
    }
    /// `event.getCoalescedEvents()` (empty for every type but `pointermove`, and on WebViews that report none).
    pub fn coalesced(&self) -> impl Iterator<Item = Sample> + 'a {
        self.coalesced.chunks_exact(40).filter_map(|c| sample(&mut Reader::new(c)))
    }
    /// The samples 5.18.0 consumes for this event, without allocating: the coalesced list, then the dispatched
    /// event unless the list already ends with it.
    // v5: app-v5.js:samplesForEvent / appendPenSamples
    pub fn samples(&self) -> impl Iterator<Item = Sample> + 'a {
        let tail = self.coalesced.rchunks_exact(40).next().and_then(|c| sample(&mut Reader::new(c)));
        let raw = self.raw;
        let include = tail.is_none_or(|l| l.x != raw.x || l.y != raw.y || l.t < raw.t);
        self.coalesced().chain(include.then_some(raw))
    }
}

/// Completion of an asynchronous effect (kind `IO`). Payload: `u32 req, i32 status, u32 obj, bytes…`.
#[derive(Clone, Copy)]
pub struct Io<'a> {
    /// The request id of the effect (made with [`tag`]).
    pub req: u32,
    /// Executor-defined: ≥ 0 success (HTTP status for fetches), < 0 failure.
    pub status: i32,
    /// JS object handle (Blob, ImageBitmap, PDF document…) or 0.
    pub obj: u32,
    pub data: &'a [u8],
}

/// A window/document lifecycle notification (kind `LIFECYCLE`). Payload: `u8 code, f64 a, f64 b, f64 c, str s`.
#[derive(Clone, Copy)]
pub struct Life<'a> {
    /// [`life`] code.
    pub code: u8,
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub s: &'a str,
}

/// A message from outside the page (kind `MESSAGE`). Payload: `u8 chan, u32 obj, bytes…`.
#[derive(Clone, Copy)]
pub struct Msg<'a> {
    /// [`chan`] code.
    pub chan: u8,
    pub obj: u32,
    pub data: &'a [u8],
}

/// Everything a module can receive in its `event` function.
#[derive(Clone, Copy)]
pub enum Event<'a> {
    Dom(DomEvent<'a>),
    Pointer(PointerBatch<'a>),
    /// A frame requested with `Sched::frame` (`requestAnimationFrame` timestamp).
    Frame(f64),
    /// A timer set with `Sched::timer` fired (the id).
    Timer(u32),
    /// An idle slot requested with `Sched::idle`.
    Idle { id: u32, remaining_ms: f64, timed_out: bool },
    /// A deferred job queued with `Sched::defer` may run now (`stale`: its session ended; drop it).
    Work { id: u32, stale: bool },
    Io(Io<'a>),
    Life(Life<'a>),
    Msg(Msg<'a>),
    /// `__IHN_TEST_API__.cmd(name, args)`; handled by `testapi`, never delivered to modules.
    Test { name: &'a str, args: &'a str },
}

impl<'a> Event<'a> {
    /// Decodes an `ihn_event` payload. `None` for an unknown kind or a malformed payload (ignored by the app).
    pub fn decode(kind: u32, bytes: &'a [u8]) -> Option<Self> {
        let mut r = Reader::new(bytes);
        Some(match kind {
            kind::DOM => Event::Dom(DomEvent::decode(bytes)?),
            kind::POINTER => Event::Pointer(PointerBatch::decode(bytes)?),
            kind::TIMER => Event::Timer(r.u32()?),
            kind::IDLE => Event::Idle { id: r.u32()?, remaining_ms: r.f64()?, timed_out: r.u8()? != 0 },
            kind::IO => Event::Io(Io { req: r.u32()?, status: r.i32()?, obj: r.u32()?, data: r.buf }),
            kind::LIFECYCLE => Event::Life(Life { code: r.u8()?, a: r.f64()?, b: r.f64()?, c: r.f64()?, s: r.str()? }),
            kind::MESSAGE => Event::Msg(Msg { chan: r.u8()?, obj: r.u32()?, data: r.buf }),
            kind::TEST => Event::Test { name: r.str()?, args: r.str()? },
            _ => return None,
        })
    }
}

/// Text form of an out-buffer for golden tests, one line per record: DOM and core/timer/worker effect records are
/// decoded by name and field, anything else is shown as hex.
#[cfg(test)]
pub fn dump(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let mut r = Reader::new(bytes);
    let mut long: Option<String> = None;
    while !r.is_empty() {
        let (family, op, payload) = (r.u8().unwrap(), r.u8().unwrap(), r.bytes().expect("record length"));
        let mut p = Reader::new(payload);
        let table: &[(u8, &str, &str)] = match family {
            fam::DOM => crate::dom::LAYOUT,
            fam::FX => FX_LAYOUT,
            _ => &[],
        };
        let Some(&(_, name, layout)) = table.iter().find(|l| l.0 == op) else {
            writeln!(out, "{family}.{op:#04x} {payload:02x?}").unwrap();
            continue;
        };
        out.push_str(name);
        for c in layout.chars() {
            out.push(' ');
            match c {
                'h' | 'u' => write!(out, "{}", p.u32().unwrap()).unwrap(),
                'i' => write!(out, "{}", p.i32().unwrap()).unwrap(),
                'b' => write!(out, "{}", p.u8().unwrap()).unwrap(),
                'f' => write!(out, "{}", p.f64().unwrap()).unwrap(),
                's' => write!(out, "{:?}", p.str().unwrap()).unwrap(),
                'o' if p.clone().u32() == Some(SAME_LONG_STYLE) => {
                    p.u32();
                    write!(out, "{:?}", long.as_deref().expect("a long style before its repeat")).unwrap()
                }
                'o' => match p.opt_str().unwrap() {
                    Some(s) => {
                        if s.len() >= LONG_STYLE {
                            long = Some(s.to_string());
                        }
                        write!(out, "{s:?}").unwrap()
                    }
                    None => out.push_str("null"),
                },
                // prop value: tag + value
                'v' => match p.u8().unwrap() {
                    0 => write!(out, "{:?}", p.str().unwrap()).unwrap(),
                    1 => write!(out, "{}", p.f64().unwrap()).unwrap(),
                    _ => write!(out, "{}", p.u8().unwrap() != 0).unwrap(),
                },
                'x' => write!(out, "{:?}", p.bytes().unwrap()).unwrap(),
                _ => unreachable!(),
            }
        }
        assert!(p.is_empty(), "{name}: {} trailing bytes", p.buf.len());
        out.push('\n');
    }
    out
}

#[cfg(test)]
const FX_LAYOUT: &[(u8, &str, &str)] = &[
    (fx::OBJ_RELEASE, "obj_release", "u"),
    (fx::TEST_RESULT, "test_result", "s"),
    (fx::READY, "ready", ""),
    (fx::TIMER_SET, "timer_set", "ufb"),
    (fx::TIMER_CLEAR, "timer_clear", "u"),
    (fx::RAF_REQUEST, "raf_request", ""),
    (fx::RAF_CANCEL, "raf_cancel", ""),
    (fx::IDLE_REQUEST, "idle_request", "uf"),
    (fx::IDLE_CANCEL, "idle_cancel", "u"),
    (fx::WORKER_START, "worker_start", ""),
    (fx::WORKER_POST, "worker_post", "ux"),
    (fx::WORKER_STOP, "worker_stop", ""),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_repeated_long_style_value_is_sent_once() {
        let (long, other) = ("a".repeat(LONG_STYLE), "b".repeat(LONG_STYLE));
        let mut out = OutBuf::default();
        out.style(Node(1024), "background-image", Some(&long));
        let first = out.w.buf.len();
        out.style(Node(1025), "background-image", Some(&long));
        assert!(out.w.buf.len() - first < 64, "the repeat carries no value");
        out.style(Node(1026), "background-image", Some(&other));
        out.style(Node(1027), "color", Some("red"));
        out.style(Node(1028), "background-image", Some(&other));
        // The dump resolves repeats: the same text as the host applies, without the node handle.
        let d: Vec<String> = dump(&out.w.buf).lines().map(|l| l.splitn(3, ' ').nth(2).unwrap().to_string()).collect();
        assert!(d[0].contains(&long) && d[2].contains(&other));
        assert_eq!((&d[1], &d[4]), (&d[0], &d[2]));
    }

    fn sample_bytes(w: &mut Writer, x: f64, y: f64, t: f64) {
        w.f64(x);
        w.f64(y);
        w.f64(t);
        w.f32(0.5);
        w.f32(1.0);
        w.f32(-2.0);
        w.u32(1);
    }

    #[test]
    fn config_and_tags() {
        let mut w = Writer::default();
        for s in ["6.0.0", "http://127.0.0.1/?e2e=1", "UA", "en-GB"] {
            w.str(s);
        }
        w.u32(mode::E2E | mode::ONLINE);
        for v in [1280.0, 800.0, 2.0] {
            w.f64(v);
        }
        w.u32(5);
        let c = Config::decode(&w.buf).unwrap();
        assert_eq!((c.version.as_str(), c.viewport, c.dpr, c.max_touch_points), ("6.0.0", (1280.0, 800.0), 2.0, 5));
        assert!(c.has(mode::E2E) && !c.has(mode::APP));
        assert_eq!(Config::decode(&w.buf[..w.buf.len() - 1]), None);
        let id = tag(Mod::Shell, 0x0123_4567);
        assert_eq!((tag_mod(id), tag_local(id)), (Mod::Shell as usize, 0x23_4567));
    }

    #[test]
    fn dom_event_decodes_every_field() {
        let mut w = Writer::default();
        w.u32(7);
        w.u32(20);
        w.u8(ety::DRAGLEAVE);
        w.u32(ev::SHIFT | ev::EXACT | ev::DT_FILES);
        w.f64(1234.5);
        w.i32(-1);
        w.u32(2);
        for v in [10.5, 20.5, 1.0, -120.0] {
            w.f64(v);
        }
        w.u32(1);
        w.i32(3);
        w.u8(ptr::TOUCH);
        w.u32(99);
        w.str("Enter");
        w.str("NumpadEnter");
        w.str("típed");
        w.u32(2);
        w.u32(1500);
        w.u32(20);
        w.u32(1);
        w.u32(21);
        for n in [1u32, 0] {
            w.u32(n);
            for _ in 0..n {
                w.i32(4);
                w.f64(1.0);
                w.f64(2.0);
            }
        }
        let Some(Event::Dom(e)) = Event::decode(kind::DOM, &w.buf) else { panic!() };
        assert_eq!((e.listener, e.node, e.button, e.buttons, e.delta_mode, e.detail, e.ptype, e.obj), (7, Node(20), -1, 2, 1, 3, 3, 99));
        assert_eq!((e.x, e.y, e.dx, e.dy), (10.5, 20.5, 1.0, -120.0));
        assert_eq!((e.key, e.code, e.value), ("Enter", "NumpadEnter", "típed"));
        assert!(e.has(ev::SHIFT) && e.has(ev::EXACT) && e.has(ev::DT_FILES) && !e.has(ev::CTRL));
        assert_eq!((e.ty, e.t), (ety::DRAGLEAVE, 1234.5));
        assert!(e.related.contains(Node(21)) && !e.related.contains(Node(20)));
        assert_eq!(e.path.iter().collect::<Vec<_>>(), [Node(1500), Node(20)]);
        assert!(e.path.contains(Node(20)) && !e.path.contains(Node(21)) && e.path.first() == Node(1500));
        assert_eq!(e.touches.iter().collect::<Vec<_>>(), [Touch { id: 4, x: 1.0, y: 2.0 }]);
        assert!(e.changed_touches.is_empty());
        for cut in 0..w.buf.len() {
            assert!(Event::decode(kind::DOM, &w.buf[..cut]).is_none());
        }
    }

    /// `host/dom.js` mirrors the event type codes; a drifted table would silently misroute events.
    #[test]
    fn event_type_codes_match_the_host() {
        let js = include_str!("../../../../host/dom.js");
        let table = js.split("const DOM_TYPE = {").nth(1).and_then(|s| s.split("};").next()).expect("DOM_TYPE in host/dom.js");
        let mut host: Vec<(String, u8)> = table
            .split(',')
            .map(|entry| {
                let (name, code) = entry.trim().split_once(':').expect("name: code");
                (name.trim().trim_matches('\'').to_string(), code.trim().parse().expect("code"))
            })
            .collect();
        let mut rust: Vec<(String, u8)> = ety::NAMES.iter().map(|&(n, c)| (n.to_string(), c)).collect();
        rust.push(("ihn-resize".into(), ety::RESIZE_OBSERVED));
        rust.push(("ihn-mutation".into(), ety::MUTATION_OBSERVED));
        host.sort();
        rust.sort();
        assert_eq!(host, rust);
        let mut codes: Vec<u8> = rust.iter().map(|e| e.1).collect();
        codes.dedup();
        assert_eq!(codes.len(), rust.len(), "duplicate event type code");
        assert!(!codes.contains(&ety::OTHER));
    }

    #[test]
    fn pointer_samples_follow_v5_rule() {
        // v5: the dispatched event is appended unless the coalesced list already ends with it.
        let batch = |coalesced: &[(f64, f64, f64)], raw: (f64, f64, f64)| {
            let mut w = Writer::default();
            w.u32(tag(Mod::EditorInput, 1));
            w.u32(30);
            w.u8(ptr::MOVE);
            w.u8(ptr::PEN);
            w.i32(5);
            w.i32(-1);
            w.u32(ev::PRIMARY);
            sample_bytes(&mut w, raw.0, raw.1, raw.2);
            w.u32(coalesced.len() as u32);
            for c in coalesced {
                sample_bytes(&mut w, c.0, c.1, c.2);
            }
            w.u32(0);
            w.buf
        };
        let xs = |bytes: &[u8]| {
            let Some(Event::Pointer(p)) = Event::decode(kind::POINTER, bytes) else { panic!() };
            assert_eq!((p.kind, p.ptype, p.pointer_id, p.button, p.raw.pressure, p.raw.tilt_y), (ptr::MOVE, ptr::PEN, 5, -1, 0.5, -2.0));
            p.samples().map(|s| s.x).collect::<Vec<_>>()
        };
        assert_eq!(xs(&batch(&[], (3.0, 0.0, 10.0))), [3.0]);
        assert_eq!(xs(&batch(&[(1.0, 0.0, 8.0), (3.0, 0.0, 10.0)], (3.0, 0.0, 10.0))), [1.0, 3.0]);
        assert_eq!(xs(&batch(&[(1.0, 0.0, 8.0), (2.0, 0.0, 9.0)], (3.0, 0.0, 10.0))), [1.0, 2.0, 3.0]);
        assert_eq!(xs(&batch(&[(3.0, 0.0, 9.0)], (3.0, 0.0, 10.0))), [3.0, 3.0]);
    }

    #[test]
    fn small_events_and_unknown_kinds() {
        let mut w = Writer::default();
        w.u32(9);
        w.i32(-3);
        w.u32(4);
        w.raw(b"abc");
        let Some(Event::Io(io)) = Event::decode(kind::IO, &w.buf) else { panic!() };
        assert_eq!((io.req, io.status, io.obj, io.data), (9, -3, 4, &b"abc"[..]));
        let mut w = Writer::default();
        w.u8(life::RESIZE);
        for v in [360.0, 800.0, 3.0] {
            w.f64(v);
        }
        w.str("");
        let Some(Event::Life(l)) = Event::decode(kind::LIFECYCLE, &w.buf) else { panic!() };
        assert_eq!((l.code, l.a, l.b, l.c, l.s), (life::RESIZE, 360.0, 800.0, 3.0, ""));
        assert!(Event::decode(3, &[]).is_none() && Event::decode(kind::TIMER, &[1, 2]).is_none());
        let mut w = Writer::default();
        w.u32(5);
        w.f64(12.0);
        w.u8(1);
        assert!(matches!(Event::decode(kind::IDLE, &w.buf), Some(Event::Idle { id: 5, remaining_ms, timed_out: true }) if remaining_ms == 12.0));
    }
}
