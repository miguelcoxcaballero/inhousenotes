//! `ihn.wasm`: the `extern "C"` boundary between `ihn-app` and `host/main.js` (ARCHITECTURE §3.2).
//!
//! Exports: `ihn_abi`, `ihn_alloc`, `ihn_free`, `ihn_init`, `ihn_event`, `ihn_frame`, `ihn_out_ptr`, `ihn_out_len`,
//! `ihn_out_reset`, `ihn_worker`. Imports (module `env`): exactly the nine `host_*` functions below; the build
//! (`scripts/build-wasm.mjs`) fails on any other import.
//!
//! The module is single-threaded and the host never re-enters an export while another one runs (events that fire
//! during a call are queued by `host/main.js`), except for `ihn_out_ptr`/`ihn_out_len`/`ihn_out_reset`, which the
//! host calls from inside `host_flush`/`host_rect`/`host_metric` and the `Ask` ops of `host_intl`; those touch only
//! the out-buffer.
#![cfg(target_arch = "wasm32")]

use ihn_app::abi::{mode, Config, Node, OutBuf, IHN_ABI, INTL_CAP};
use ihn_app::{App, Ask, Host, IntlOp, LsOp, Metric, Rect};
use std::ptr::addr_of_mut;

#[link(wasm_import_module = "env")]
extern "C" {
    fn host_flush();
    fn host_rect(handle: u32, out: *mut f64);
    fn host_metric(handle: u32, which: u32) -> f64;
    fn host_now() -> f64;
    fn host_time() -> f64;
    fn host_random(ptr: *mut u8, len: usize);
    /// Get: writes at most `vlen` bytes of the value to `vptr` and returns its full length, or -1 when missing.
    /// Set (value = `vptr..vlen`) and remove: 0, or -1 when storage threw.
    fn host_ls(op: u32, kptr: *const u8, klen: usize, vptr: *mut u8, vlen: usize) -> i32;
    /// Input at `ptr..len` in a buffer of `max(len, INTL_CAP)` bytes; the result (truncated to that capacity) is
    /// written at `ptr` and its length returned, or -1. Ops `0..16` are `IntlOp`; ops `16..` are `Ask` (the host
    /// applies the out-buffer first).
    fn host_intl(op: u32, ptr: *mut u8, len: usize) -> i32;
    fn host_log(level: u32, ptr: *const u8, len: usize);
}

static mut OUT: Option<OutBuf> = None;
static mut APP: Option<App> = None;

fn out() -> &'static mut OutBuf {
    // SAFETY: single-threaded; no reference to OUT is held across a call that could create another one.
    unsafe { (*addr_of_mut!(OUT)).get_or_insert_with(OutBuf::default) }
}

struct Wasm;

impl Host for Wasm {
    fn out(&mut self) -> &mut OutBuf { out() }
    fn flush(&mut self) {
        if !out().w.buf.is_empty() {
            unsafe { host_flush() }
        }
    }
    fn rect(&mut self, h: Node) -> Rect {
        let mut r = [0f64; 4];
        unsafe { host_rect(h.0, r.as_mut_ptr()) }
        Rect { x: r[0], y: r[1], w: r[2], h: r[3] }
    }
    fn metric(&mut self, h: Node, m: Metric) -> f64 { unsafe { host_metric(h.0, m as u32) } }
    fn now(&self) -> f64 { unsafe { host_now() } }
    fn time(&self) -> f64 { unsafe { host_time() } }
    fn random(&mut self, buf: &mut [u8]) { unsafe { host_random(buf.as_mut_ptr(), buf.len()) } }
    fn ls(&mut self, op: LsOp, key: &str, val: Option<&str>) -> Option<String> {
        let call = |ptr: *mut u8, len: usize| unsafe { host_ls(op as u32, key.as_ptr(), key.len(), ptr, len) };
        if !matches!(op, LsOp::Get | LsOp::SessionGet) {
            let v = val.unwrap_or("");
            // The host only reads the value.
            return (call(v.as_ptr() as *mut u8, v.len()) >= 0).then(String::new);
        }
        let mut buf = vec![0u8; 256];
        loop {
            let n = usize::try_from(call(buf.as_mut_ptr(), buf.len())).ok()?;
            if n <= buf.len() {
                buf.truncate(n);
                return String::from_utf8(buf).ok();
            }
            buf.resize(n, 0);
        }
    }
    fn intl(&mut self, op: IntlOp, input: &str) -> String { text(op as u32, input) }
    fn ask(&mut self, q: Ask, input: &str) -> String { text(q as u32, input) }
    fn log(&mut self, level: u32, msg: &str) { unsafe { host_log(level, msg.as_ptr(), msg.len()) } }
}

fn text(op: u32, input: &str) -> String {
    let mut buf = vec![0u8; input.len().max(INTL_CAP)];
    buf[..input.len()].copy_from_slice(input.as_bytes());
    let n = unsafe { host_intl(op, buf.as_mut_ptr(), input.len()) };
    buf.truncate(usize::try_from(n).unwrap_or(0).min(buf.len()));
    String::from_utf8_lossy(&buf).into_owned()
}

unsafe fn bytes<'a>(ptr: *const u8, len: usize) -> &'a [u8] {
    if len == 0 {
        &[]
    } else {
        std::slice::from_raw_parts(ptr, len)
    }
}

#[no_mangle]
pub extern "C" fn ihn_abi() -> u32 { IHN_ABI }

/// A buffer of `len` bytes for an event payload; give it back with `ihn_free(ptr, len)`.
#[no_mangle]
pub extern "C" fn ihn_alloc(len: usize) -> *mut u8 {
    let mut v = Vec::<u8>::with_capacity(len.max(1));
    let ptr = v.as_mut_ptr();
    std::mem::forget(v);
    ptr
}

/// # Safety
/// `ptr`/`len` must come from `ihn_alloc(len)` and not have been freed.
#[no_mangle]
pub unsafe extern "C" fn ihn_free(ptr: *mut u8, len: usize) { drop(Vec::from_raw_parts(ptr, 0, len.max(1))); }

/// Boot with the encoded `Config`. In the export worker (`mode::WORKER`) no `App` is created.
///
/// # Safety
/// `ptr..ptr+len` must be readable.
#[no_mangle]
pub unsafe extern "C" fn ihn_init(ptr: *const u8, len: usize) {
    std::panic::set_hook(Box::new(|info| {
        let msg = info.to_string();
        unsafe { host_log(3, msg.as_ptr(), msg.len()) }
    }));
    let Some(cfg) = Config::decode(bytes(ptr, len)) else {
        Wasm.log(3, "ihn: bad config");
        return;
    };
    if cfg.has(mode::WORKER) {
        return;
    }
    let app = (*addr_of_mut!(APP)).insert(App::new(cfg));
    app.init(&mut Wasm);
}

/// # Safety
/// `ptr..ptr+len` must be readable.
#[no_mangle]
pub unsafe extern "C" fn ihn_event(kind: u32, ptr: *const u8, len: usize) -> u32 {
    match (*addr_of_mut!(APP)).as_mut() {
        Some(app) => app.event(&mut Wasm, kind, bytes(ptr, len)),
        None => 0,
    }
}

#[no_mangle]
pub extern "C" fn ihn_frame(t: f64) {
    // SAFETY: see the module comment.
    if let Some(app) = unsafe { (*addr_of_mut!(APP)).as_mut() } {
        app.frame(&mut Wasm, t);
    }
}

#[no_mangle]
pub extern "C" fn ihn_out_ptr() -> *const u8 { out().w.buf.as_ptr() }

#[no_mangle]
pub extern "C" fn ihn_out_len() -> usize { out().w.buf.len() }

/// The host applied everything up to `ihn_out_len()`.
#[no_mangle]
pub extern "C" fn ihn_out_reset() {
    let buf = &mut out().w.buf;
    buf.clear();
    // One large flush (a store transaction, the pages of a document) must not keep megabytes reserved for good.
    if buf.capacity() > 1 << 20 {
        buf.shrink_to(256 << 10);
    }
}

/// Export-worker entry: one message (`[u8 role][payload]`).
///
/// # Safety
/// `ptr..ptr+len` must be readable.
#[no_mangle]
pub unsafe extern "C" fn ihn_worker(ptr: *const u8, len: usize) { ihn_app::app::worker(&mut Wasm, bytes(ptr, len)); }
