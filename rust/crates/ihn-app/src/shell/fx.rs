//! Effect builders for the families W1.3 provides (ARCHITECTURE §3.3): 3.3 fetch and blobs (`host/fetch.js`),
//! 3.8 SHA-256, 3.10 deflate, 3.6 platform (`host/platform.js`). Each function appends one record; an asynchronous
//! effect takes a request id made with `app.req(Mod::X)` and completes as `Event::Io` at `X::event`.
//!
//! Completion status: `>= 0` success (the HTTP status of a fetch, the file count of `files_take`), else one of
//! [`status`]. On failure `Io::data` is the error message.
//!
//! JS objects (Blobs, Files) are addressed by u32 handles. Handles made here with [`new_obj`] are chosen by Rust,
//! so creating, slicing and joining blobs needs no round trip; handles received in events come from the host.
//! Both kinds are released with `OutBuf::obj_release`.

use crate::abi::{fam, OutBuf};
use crate::shell::json::Json;
use crate::testapi::json_str;
use crate::{App, Host, Mod};
use ihn_core::codec::Reader;

/// Record ops (`host/fetch.js`, `host/platform.js`).
pub mod op {
    pub const FETCH: u8 = 0x30;
    pub const FETCH_ABORT: u8 = 0x31;
    pub const BLOB_NEW: u8 = 0x38;
    pub const BLOB_SLICE: u8 = 0x39;
    pub const BLOB_CONCAT: u8 = 0x3a;
    pub const BLOB_READ: u8 = 0x3b;
    pub const BLOB_URL: u8 = 0x3c;
    pub const URL_REVOKE: u8 = 0x3d;
    pub const BLOB_INFO: u8 = 0x3e;
    pub const DIGEST: u8 = 0x78;
    pub const DEFLATE: u8 = 0x90;
    pub const GIS_INIT: u8 = 0xa0;
    pub const GIS_REQUEST: u8 = 0xa1;
    pub const GIS_REVOKE: u8 = 0xa2;
    pub const NATIVE_CALL: u8 = 0xa4;
    pub const NATIVE_BLOB: u8 = 0xa5;
    pub const UPDATER_CREATE: u8 = 0xa8;
    pub const UPDATER_SETTINGS: u8 = 0xa9;
    pub const BOOT_SIGNED_IN: u8 = 0xaa;
    pub const BACKGROUND: u8 = 0xab;
    pub const FILES_TAKE: u8 = 0xb0;
    pub const CLIPBOARD_WRITE: u8 = 0xb4;
    pub const SHARE: u8 = 0xb8;
    pub const DOWNLOAD: u8 = 0xb9;
    pub const FULLSCREEN: u8 = 0xbc;
    pub const DISPATCH_RESIZE: u8 = 0xbd;
    pub const NAVIGATE: u8 = 0xc0;
}

/// Failure codes of `Io::status`.
pub mod status {
    /// Network error or exception (fetch family); "not available here" (platform family).
    pub const FAILED: i32 = -1;
    /// Fetch: the deadline passed before the response headers. Platform: the call threw.
    pub const TIMEOUT: i32 = -2;
    /// Fetch: aborted with [`super::fetch_abort`]. Platform: cancelled by the user.
    pub const ABORTED: i32 = -3;
    /// Fetch: the deadline or the idle timeout passed while the body was being read.
    pub const BODY_TIMEOUT: i32 = -5;
}

/// [`Fetch::flags`].
pub mod fetch_flag {
    /// `keepalive: true` (exit flushes; the browser limits these bodies to 64 KiB per page).
    pub const KEEPALIVE: u32 = 1;
    /// Deliver the body as a Blob handle (`Io::obj`) instead of bytes: downloads that go to pdf.js or storage.
    pub const BLOB: u32 = 2;
    /// `cache: 'no-store'`.
    pub const NO_STORE: u32 = 4;
    /// `credentials: 'omit'`.
    pub const OMIT_CREDENTIALS: u32 = 8;
    /// Status and headers only; the body is cancelled.
    pub const HEAD_ONLY: u32 = 16;
}

/// `Msg::chan` values sent by `host/platform.js` (besides `abi::chan::OAUTH` and `PDF_EXPORT`).
pub mod chan {
    /// `popstate`; data = `location.href`.
    pub const POPSTATE: u8 = 0x10;
}

/// [`Config::flags`](crate::Config) bit set by `host/platform.js`: boot.js painted the signed-in skeleton
/// (`window.__INHOUSE_BOOT_SIGNED_IN__ === true`).
pub const BOOT_SIGNED_IN: u32 = 256;

/// A byte source for a request body, a digest or a deflate.
#[derive(Clone, Copy, Debug)]
pub enum Src<'a> {
    None,
    Bytes(&'a [u8]),
    /// Bytes `start..end` of a blob (`end < 0`: to its end).
    Blob { obj: u32, start: f64, end: f64 },
}

impl Src<'_> {
    pub fn blob(obj: u32) -> Src<'static> { Src::Blob { obj, start: 0.0, end: -1.0 } }
    fn write(&self, out: &mut OutBuf) {
        match *self {
            Src::None => out.w.u8(0),
            Src::Bytes(b) => {
                out.w.u8(1);
                out.room(b.len() + 4);
                out.w.bytes(b);
            }
            Src::Blob { obj, start, end } => {
                out.w.u8(2);
                out.w.u32(obj);
                out.w.f64(start);
                out.w.f64(end);
            }
        }
    }
}

/// One `fetch()`. A resumable upload is a sequence of these driven by Rust: the session POST, then `PUT`s whose
/// body is a `Src::Blob` range with a `Content-Range` header, reading `Range` from each 308 answer.
#[derive(Clone, Copy, Debug)]
pub struct Fetch<'a> {
    pub method: &'a str,
    pub url: &'a str,
    /// `"Name: value\n"` lines.
    pub headers: &'a str,
    /// [`fetch_flag`] bits.
    pub flags: u32,
    /// Total deadline in ms, body included (0 = none).
    pub deadline_ms: f64,
    /// Longest silence while reading the body in ms (0 = none).
    pub idle_ms: f64,
    pub body: Src<'a>,
}

impl Default for Fetch<'_> {
    fn default() -> Self {
        Fetch { method: "GET", url: "", headers: "", flags: 0, deadline_ms: 0.0, idle_ms: 0.0, body: Src::None }
    }
}

/// The payload of a completed fetch: `str headers, body…` (with [`fetch_flag::BLOB`]: `str headers, f64 size`).
#[derive(Clone, Copy, Debug, Default)]
pub struct Response<'a> {
    /// `"name: value\n"` lines with lower-case names.
    pub headers: &'a str,
    pub body: &'a [u8],
}

impl<'a> Response<'a> {
    pub fn parse(data: &'a [u8]) -> Response<'a> {
        let mut r = Reader::new(data);
        match r.str() {
            Some(headers) => Response { headers, body: r.buf },
            None => Response::default(),
        }
    }
    /// `response.headers.get(name)` (`name` in lower case).
    pub fn header(&self, name: &str) -> Option<&'a str> {
        self.headers.lines().find_map(|l| l.split_once(": ").filter(|(n, _)| *n == name).map(|(_, v)| v))
    }
    pub fn text(&self) -> &'a str { std::str::from_utf8(self.body).unwrap_or("") }
    /// The size of the Blob of a [`fetch_flag::BLOB`] response.
    pub fn blob_size(&self) -> f64 { Reader::new(self.body).f64().unwrap_or(0.0) }
}

/// A fresh object handle chosen by Rust (`0x8000_0000..`).
pub fn new_obj(app: &mut App) -> u32 {
    let st = &mut app.st.shell;
    st.next_obj = st.next_obj.wrapping_add(1) & 0x7fff_ffff;
    0x8000_0000 | st.next_obj
}

fn rec(out: &mut OutBuf, o: u8, id: u32) -> usize {
    let m = out.w.rec(fam::FX, o);
    out.w.u32(id);
    m
}

pub fn fetch(out: &mut OutBuf, req: u32, f: &Fetch) {
    let m = rec(out, op::FETCH, req);
    out.w.str(f.method);
    out.w.str(f.url);
    out.w.str(f.headers);
    out.w.u32(f.flags);
    out.w.f64(f.deadline_ms);
    out.w.f64(f.idle_ms);
    f.body.write(out);
    out.w.end(m);
}
/// Aborts a running fetch; it completes with [`status::ABORTED`].
pub fn fetch_abort(out: &mut OutBuf, req: u32) { out.fx_id(op::FETCH_ABORT, req); }

/// `new Blob([bytes], {type})` as object `obj`.
pub fn blob_new(out: &mut OutBuf, obj: u32, mime: &str, bytes: &[u8]) {
    let m = rec(out, op::BLOB_NEW, obj);
    out.w.str(mime);
    out.room(bytes.len() + 4);
    out.w.bytes(bytes);
    out.w.end(m);
}
/// `src.slice(start, end, type)` as object `obj` (`end < 0`: to the end; empty `mime`: keep the type).
pub fn blob_slice(out: &mut OutBuf, obj: u32, src: u32, start: f64, end: f64, mime: &str) {
    let m = rec(out, op::BLOB_SLICE, obj);
    out.w.u32(src);
    out.w.f64(start);
    out.w.f64(end);
    out.w.str(mime);
    out.w.end(m);
}
/// `new Blob(parts, {type})` as object `obj`.
pub fn blob_concat(out: &mut OutBuf, obj: u32, mime: &str, parts: &[u32]) {
    let m = rec(out, op::BLOB_CONCAT, obj);
    out.w.str(mime);
    out.w.u32(parts.len() as u32);
    parts.iter().for_each(|p| out.w.u32(*p));
    out.w.end(m);
}
/// Bytes `start..end` of a blob → `Io::data`.
pub fn blob_read(out: &mut OutBuf, req: u32, src: u32, start: f64, end: f64) {
    let m = rec(out, op::BLOB_READ, req);
    out.w.u32(src);
    out.w.f64(start);
    out.w.f64(end);
    out.w.end(m);
}
/// `URL.createObjectURL(blob)` → `Io::data` = the URL.
pub fn blob_url(out: &mut OutBuf, req: u32, src: u32) {
    let m = rec(out, op::BLOB_URL, req);
    out.w.u32(src);
    out.w.end(m);
}
pub fn url_revoke(out: &mut OutBuf, url: &str) {
    let m = out.w.rec(fam::FX, op::URL_REVOKE);
    out.w.str(url);
    out.w.end(m);
}
/// → `Io::data` = `f64 size, str type, str name, f64 lastModified` ([`BlobInfo::parse`]).
pub fn blob_info(out: &mut OutBuf, req: u32, src: u32) {
    let m = rec(out, op::BLOB_INFO, req);
    out.w.u32(src);
    out.w.end(m);
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BlobInfo {
    pub size: f64,
    pub mime: String,
    pub name: String,
    pub last_modified: f64,
}

impl BlobInfo {
    pub fn parse(data: &[u8]) -> Option<BlobInfo> {
        let mut r = Reader::new(data);
        Some(BlobInfo { size: r.f64()?, mime: r.str()?.into(), name: r.str()?.into(), last_modified: r.f64()? })
    }
}

/// SHA-256 → `Io::data` = 32 bytes.
pub fn digest(out: &mut OutBuf, req: u32, src: Src) {
    let m = rec(out, op::DIGEST, req);
    src.write(out);
    out.w.end(m);
}

/// [`deflate`] flags.
pub mod deflate_flag {
    pub const INFLATE: u8 = 1;
    /// zlib wrapper (`deflate`); default is `deflate-raw`.
    pub const ZLIB: u8 = 2;
    /// Result as a Blob handle (`Io::obj`, data = `f64 size`).
    pub const BLOB: u8 = 4;
}
/// Compression/DecompressionStream → `Io::data` = the result.
pub fn deflate(out: &mut OutBuf, req: u32, flags: u8, src: Src) {
    let m = rec(out, op::DEFLATE, req);
    out.w.u8(flags);
    src.write(out);
    out.w.end(m);
}

/// `google.accounts.oauth2.initTokenClient` → 0, or -1 while the GIS script has not loaded.
pub fn gis_init(out: &mut OutBuf, req: u32, client_id: &str, scope: &str) {
    let m = rec(out, op::GIS_INIT, req);
    out.w.str(client_id);
    out.w.str(scope);
    out.w.end(m);
}
/// `tokenClient.requestAccessToken` → 0 and the token response as JSON.
pub fn gis_request(out: &mut OutBuf, req: u32, interactive: bool, scope: &str, login_hint: &str) {
    let m = rec(out, op::GIS_REQUEST, req);
    out.w.u8(interactive as u8);
    out.w.str(scope);
    out.w.str(login_hint);
    out.w.end(m);
}
pub fn gis_revoke(out: &mut OutBuf, token: &str) {
    let m = out.w.rec(fam::FX, op::GIS_REVOKE);
    out.w.str(token);
    out.w.end(m);
}
/// `InhouseNative[method](...args)` → 0 and `String(result)`; -1 when the bridge has no such method. `req` 0:
/// no answer. Covers `openAuthUrl`, `getPdfExportApiVersion`, `beginPdfExport`, `finishPdfExport`, `savePdf`.
pub fn native_call(out: &mut OutBuf, req: u32, method: &str, args: &[&str]) {
    let m = rec(out, op::NATIVE_CALL, req);
    out.w.str(method);
    out.w.u32(args.len() as u32);
    args.iter().for_each(|a| out.w.str(a));
    out.w.end(m);
}
/// A blob as base64 through `InhouseNative[method](chunk)`, `chunk_chars` characters per call (a multiple of 4;
/// 5.18.0's export bridge uses 240000 with `appendPdfExportChunk`).
pub fn native_blob(out: &mut OutBuf, req: u32, blob: u32, method: &str, chunk_chars: u32) {
    let m = rec(out, op::NATIVE_BLOB, req);
    out.w.u32(blob);
    out.w.str(method);
    out.w.u32(chunk_chars);
    out.w.end(m);
}
/// `InhouseServicesUpdater.create(…)`, `window.InhouseUpdater`, `start()` → 0, or -1 without the updater script.
pub fn updater_create(out: &mut OutBuf, req: u32) { out.fx_id(op::UPDATER_CREATE, req); }
/// `updates.settings()`.
pub fn updater_settings(out: &mut OutBuf) { out.fx0(op::UPDATER_SETTINGS); }
/// `window.__INHOUSE_BOOT_SIGNED_IN__ = value`.
pub fn boot_signed_in(out: &mut OutBuf, value: bool) {
    let m = out.w.rec(fam::FX, op::BOOT_SIGNED_IN);
    out.w.u8(value as u8);
    out.w.end(m);
}

/// [`background`] operations (`InhouseBackground`, the Android WorkManager bridge of boot.js).
#[derive(Clone, Copy, Debug)]
#[repr(u8)]
pub enum Background {
    Schedule = 0,
    Cancel = 1,
    /// `complete(json)` with `{busy, pending, errors}`.
    Complete = 2,
}
pub fn background(out: &mut OutBuf, req: u32, what: Background, json: &str) {
    let m = rec(out, op::BACKGROUND, req);
    out.w.u8(what as u8);
    out.w.str(json);
    out.w.end(m);
}
/// Takes the files of an event object (`DomEvent::obj` of a `change`, `drop` or `paste`) as objects `base`,
/// `base + 1`, … → status = their count, data = JSON `[{name, type, size, lastModified}]`. Reserve the handles
/// with [`new_obj`] (call it once per file you accept; the host stores at most what the event carries). For
/// `paste` and `drop` call this while handling the event. `images_only` applies 5.18.0's paste rule.
pub fn files_take(out: &mut OutBuf, req: u32, src: u32, base: u32, images_only: bool) {
    let m = rec(out, op::FILES_TAKE, req);
    out.w.u32(src);
    out.w.u32(base);
    out.w.u8(images_only as u8);
    out.w.end(m);
}
pub fn clipboard_write(out: &mut OutBuf, req: u32, text: &str) {
    let m = rec(out, op::CLIPBOARD_WRITE, req);
    out.w.str(text);
    out.w.end(m);
}
/// `navigator.share({files: [new File([blob], name, {type})], title})` → -1 when it cannot be shared here.
pub fn share(out: &mut OutBuf, req: u32, blob: u32, name: &str, mime: &str, title: &str) {
    let m = rec(out, op::SHARE, req);
    out.w.u32(blob);
    out.w.str(name);
    out.w.str(mime);
    out.w.str(title);
    out.w.end(m);
}
/// `<a download=name>` click on an object URL revoked after `revoke_ms`; `open_ms > 0` also opens the URL in a
/// new window after that delay (5.18.0: app 120 / 60000, web 0 / 1200).
pub fn download(out: &mut OutBuf, blob: u32, name: &str, open_ms: f64, revoke_ms: f64) {
    let m = rec(out, op::DOWNLOAD, blob);
    out.w.str(name);
    out.w.f64(open_ms);
    out.w.f64(revoke_ms);
    out.w.end(m);
}
/// `documentElement.requestFullscreen()` / `document.exitFullscreen()` (webkit fallbacks).
pub fn fullscreen(out: &mut OutBuf, on: bool) {
    let m = out.w.rec(fam::FX, op::FULLSCREEN);
    out.w.u8(on as u8);
    out.w.end(m);
}
/// `window.dispatchEvent(new Event('resize'))`.
pub fn dispatch_resize(out: &mut OutBuf) { out.fx0(op::DISPATCH_RESIZE); }

/// [`navigate`] operations.
#[derive(Clone, Copy, Debug)]
#[repr(u8)]
pub enum Nav {
    PushState = 0,
    ReplaceState = 1,
    Back = 2,
    Assign = 3,
    Replace = 4,
    Reload = 5,
    /// `window.open(url, '_blank')`.
    Open = 6,
}
pub fn navigate(out: &mut OutBuf, what: Nav, url: &str) {
    let m = out.w.rec(fam::FX, op::NAVIGATE);
    out.w.u8(what as u8);
    out.w.str(url);
    out.w.end(m);
}

fn percent(s: &str, keep: &[u8], plus: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        if b.is_ascii_alphanumeric() || keep.contains(&b) {
            out.push(b as char);
        } else if plus && b == b' ' {
            out.push('+');
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}
/// `encodeURIComponent(s)`.
pub fn url_encode(s: &str) -> String { percent(s, b"-_.!~*'()", false) }
/// One name or value as `URLSearchParams.toString()` writes it (`application/x-www-form-urlencoded`).
pub fn form_encode(s: &str) -> String { percent(s, b"*-._", true) }
/// `new URLSearchParams(pairs).toString()`.
pub fn form(pairs: &[(&str, &str)]) -> String {
    pairs.iter().map(|(k, v)| format!("{}={}", form_encode(k), form_encode(v))).collect::<Vec<_>>().join("&")
}

/// `new URLSearchParams(query).get(name)`.
pub fn query_param(query: &str, name: &str) -> Option<String> {
    query.trim_start_matches(['?', '#']).split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        (url_decode(k) == name).then(|| url_decode(v))
    })
}

fn url_decode(s: &str) -> String {
    let (b, mut out, mut i) = (s.as_bytes(), Vec::with_capacity(s.len()), 0);
    while i < b.len() {
        let hex = |c: u8| (c as char).to_digit(16);
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() && hex(b[i + 1]).is_some() && hex(b[i + 2]).is_some() => {
                out.push((hex(b[i + 1]).unwrap_or(0) * 16 + hex(b[i + 2]).unwrap_or(0)) as u8);
                i += 2;
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The query string of a URL (without `?` and without the fragment).
pub fn url_query(url: &str) -> &str {
    let url = url.split('#').next().unwrap_or("");
    url.split_once('?').map_or("", |(_, q)| q)
}

// ---------------------------------------------------------------------------------------------------------------
// Executor self-test (BUILD-PLAN W1.3 acceptance): `shell.fx` writes one effect record from JSON against the real
// host and `shell.fx_results` returns what came back, so e2e `01-signin` exercises every op of both executors.
// It is deliberately generic (a record is an op and a list of typed fields) to cost the production binary little.

/// Results collected for `shell.fx_results`: `(req, status, obj, data)`.
#[derive(Default)]
pub struct Probe {
    pub results: Vec<(u32, i32, u32, Vec<u8>)>,
    pub messages: Vec<(u8, String)>,
    pub file_listener: bool,
}

/// Listener id of the self-test's `change` listener on `#pdf-input`.
pub const PROBE_FILES: u32 = crate::tag(Mod::Shell, 0x00f0_0001);

/// `{"op": <number>, "fields": [[type, value], …]}` with types `r` (a fresh request id), `o` (a fresh object
/// handle), `u` u32, `b` u8, `f` f64, `s` string, `x` bytes (given as text). `op` -1 listens for `change` on
/// `#pdf-input` instead. Returns `{"req", "obj"}`.
pub fn probe(app: &mut App, host: &mut dyn Host, args: &str) -> Option<String> {
    let a = Json::parse(args)?;
    let (req, obj) = (app.req(Mod::Shell), new_obj(app));
    let out = host.out();
    let code = a.get("op")?.as_f64()?;
    if code < 0.0 {
        if !std::mem::replace(&mut app.st.shell.probe.file_listener, true) {
            // Capture: it runs before `doc::import`'s listener, which clears the input (and so its FileList).
            out.listen(crate::Ui::PDF_INPUT, "change", PROBE_FILES, crate::abi::listen::CAPTURE);
        }
        return Some("{}".into());
    }
    let m = out.w.rec(fam::FX, code as u8);
    for field in a.get("fields")?.items() {
        let (kind, value) = (field.items().first()?.as_str()?, field.items().get(1).unwrap_or(&Json::Null));
        let n = value.as_f64().unwrap_or(0.0);
        match kind {
            "r" => out.w.u32(req),
            "o" => out.w.u32(obj),
            "u" => out.w.u32(n as u32),
            "b" => out.w.u8(n as u8),
            "f" => out.w.f64(n),
            "s" => out.w.str(value.as_str()?),
            _ => out.w.bytes(value.as_str()?.as_bytes()),
        }
    }
    out.w.end(m);
    Some(format!("{{\"req\":{req},\"obj\":{obj}}}"))
}

/// The self-test's `change` listener: take the chosen files.
pub fn probe_files(app: &mut App, host: &mut dyn Host, files: u32) {
    let (req, base) = (app.req(Mod::Shell), new_obj(app));
    files_take(host.out(), req, files, base, false);
    host.out().obj_release(files);
    app.st.shell.probe.results.push((req, i32::MIN, base, Vec::new()));
}

pub fn probe_results(app: &mut App) -> String {
    let p = std::mem::take(&mut app.st.shell.probe);
    app.st.shell.probe.file_listener = p.file_listener;
    let results: Vec<String> = p.results.iter().filter(|r| r.1 != i32::MIN).map(|(req, status, obj, data)| {
        let hex: String = data.iter().take(64).map(|b| format!("{b:02x}")).collect();
        let base = p.results.iter().find(|r| r.0 == *req && r.1 == i32::MIN).map_or(0, |r| r.2);
        format!(
            "{{\"req\":{req},\"status\":{status},\"obj\":{obj},\"base\":{base},\"len\":{},\"text\":{},\"hex\":\"{hex}\"}}",
            data.len(),
            json_str(&String::from_utf8_lossy(data))
        )
    }).collect();
    let messages: Vec<String> = p.messages.iter().map(|(c, d)| format!("{{\"chan\":{c},\"data\":{}}}", json_str(d))).collect();
    format!("{{\"results\":[{}],\"messages\":[{}]}}", results.join(","), messages.join(","))
}

/// Text form of an out-buffer for golden tests: this module's effect records by name, everything else through
/// `abi::dump`.
#[cfg(test)]
pub fn dump(bytes: &[u8]) -> String {
    use std::fmt::Write;
    const LAYOUT: &[(u8, &str, &str)] = &[
        (op::FETCH, "fetch", "usssuffS"),
        (op::FETCH_ABORT, "fetch_abort", "u"),
        (op::GIS_INIT, "gis_init", "uss"),
        (op::GIS_REQUEST, "gis_request", "ubss"),
        (op::GIS_REVOKE, "gis_revoke", "s"),
        (op::NATIVE_CALL, "native_call", "usA"),
        (op::UPDATER_CREATE, "updater_create", "u"),
        (op::UPDATER_SETTINGS, "updater_settings", ""),
        (op::BOOT_SIGNED_IN, "boot_signed_in", "b"),
        (op::DIGEST, "digest", "uS"),
        (op::FULLSCREEN, "fullscreen", "b"),
        (op::DISPATCH_RESIZE, "dispatch_resize", ""),
        (op::NAVIGATE, "navigate", "bs"),
    ];
    let (mut out, mut r) = (String::new(), Reader::new(bytes));
    while !r.is_empty() {
        let start = r.buf;
        let (family, o, payload) = (r.u8().unwrap(), r.u8().unwrap(), r.bytes().unwrap());
        let Some(&(_, name, layout)) = LAYOUT.iter().find(|l| family == fam::FX && l.0 == o) else {
            out.push_str(&crate::abi::dump(&start[..start.len() - r.buf.len()]));
            continue;
        };
        let mut p = Reader::new(payload);
        out.push_str(name);
        for c in layout.chars() {
            out.push(' ');
            match c {
                'u' => write!(out, "{}", p.u32().unwrap()).unwrap(),
                'b' => write!(out, "{}", p.u8().unwrap()).unwrap(),
                'f' => write!(out, "{}", p.f64().unwrap()).unwrap(),
                's' => write!(out, "{:?}", p.str().unwrap()).unwrap(),
                'A' => {
                    let list: Vec<&str> = (0..p.u32().unwrap()).map(|_| p.str().unwrap()).collect();
                    write!(out, "{list:?}").unwrap()
                }
                _ => match p.u8().unwrap() {
                    0 => out.push('-'),
                    1 => write!(out, "{:?}", String::from_utf8_lossy(p.bytes().unwrap())).unwrap(),
                    _ => write!(out, "blob {} {}..{}", p.u32().unwrap(), p.f64().unwrap(), p.f64().unwrap()).unwrap(),
                },
            }
        }
        assert!(p.is_empty(), "{name}: trailing bytes");
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_and_responses() {
        assert_eq!(url_encode("a b&c=d/é:~'()*!"), "a%20b%26c%3Dd%2F%C3%A9%3A~'()*!");
        assert_eq!(form(&[("redirect_uri", "com.local:/o2"), ("scope", "a b"), ("e", "x@y~z")]), "redirect_uri=com.local%3A%2Fo2&scope=a+b&e=x%40y%7Ez");
        assert_eq!(query_param("?code=4%2FabC&state=inhouse_notes_pkce", "code").as_deref(), Some("4/abC"));
        assert_eq!(query_param("#a=1&name=My+file%20x&flag", "name").as_deref(), Some("My file x"));
        assert_eq!(query_param("a=1&flag", "flag").as_deref(), Some(""));
        assert_eq!(query_param("a=1", "b"), None);
        assert_eq!(query_param("a=100%", "a").as_deref(), Some("100%"));
        assert_eq!(url_query("http://h/p?x=1&y=2#frag?z"), "x=1&y=2");
        assert_eq!(url_query("http://h/p"), "");
        let mut w = ihn_core::codec::Writer::default();
        w.str("content-type: application/json\netag: \"7\"\n");
        w.raw(b"{\"a\":1}");
        let r = Response::parse(&w.buf);
        assert_eq!((r.header("etag"), r.header("range"), r.text()), (Some("\"7\""), None, "{\"a\":1}"));
        assert_eq!(Response::parse(&[1, 2]).text(), "");
    }

    #[test]
    fn records_are_readable_in_dumps() {
        let mut out = OutBuf::default();
        fetch(&mut out, 9, &Fetch { method: "PUT", url: "https://x/u", headers: "Content-Range: bytes 0-3/8\n", flags: fetch_flag::KEEPALIVE, deadline_ms: 5000.0, idle_ms: 0.0, body: Src::Blob { obj: 7, start: 0.0, end: 4.0 } });
        native_call(&mut out, 0, "openAuthUrl", &["https://a"]);
        digest(&mut out, 3, Src::Bytes(b"abc"));
        out.fx0(crate::abi::fx::READY);
        assert_eq!(
            dump(&out.w.buf),
            "fetch 9 \"PUT\" \"https://x/u\" \"Content-Range: bytes 0-3/8\\n\" 1 5000 0 blob 7 0..4\n\
             native_call 0 \"openAuthUrl\" [\"https://a\"]\ndigest 3 \"abc\"\nready\n"
        );
    }
}
