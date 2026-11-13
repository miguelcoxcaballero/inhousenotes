//! Drive HTTP client (ARCHITECTURE §2.3): 5.18.0's `driveFetch` — a deadline that includes getting the token, the
//! `Authorization` header, one retry after a 401 with a fresh token, and its error mapping. Typed Drive calls
//! (`drive::home`, `manage`, `perms`, `revisions`, `relay`) build a [`Req`] and call [`request`].
//!
//! The caller's module receives one `Event::Io { req, status, obj, data }` for the id [`request`] returned:
//! - `status >= 0`: the HTTP status (2xx, or one of `Req::accept`); `data` is `fx::Response` (`obj` = the Blob
//!   for `Req::blob`).
//! - `status < 0`: [`err`] or `-(HTTP status)`; `data` is the error message 5.18.0 throws.

use crate::auth::{self, Token};
use crate::shell::{self, fx};
use crate::{App, Event, Host, Mod};

/// Failure statuses (besides `-(HTTP status)` for `Drive API error <status>: <body>`).
pub mod err {
    /// The network request failed.
    pub const NETWORK: i32 = -1;
    /// `Drive request timed out` / `Drive download timed out before the PDF finished`.
    pub const TIMEOUT: i32 = -2;
    /// No token: signed out, sign-in needed, or the refresh failed.
    pub const AUTH: i32 = -4;
}

/// A request body.
#[derive(Clone, Debug, Default)]
pub enum Body {
    #[default]
    None,
    Bytes(Vec<u8>),
    /// Bytes `start..end` of a blob handle (`end < 0`: to its end) — upload chunks.
    Blob { obj: u32, start: f64, end: f64 },
}

/// One Drive request (5.18.0 `driveFetch(url, options)`).
#[derive(Clone, Debug)]
pub struct Req {
    pub method: &'static str,
    pub url: String,
    /// `"Name: value\n"` lines; `Authorization` is added here.
    pub headers: String,
    pub body: Body,
    /// `options.timeout` (default 60 s): the whole request, token included.
    pub timeout_ms: f64,
    /// `options.acceptStatuses`: non-2xx statuses the caller handles itself (308, 404, 412 …).
    pub accept: &'static [i32],
    pub keepalive: bool,
    /// Deliver the body as a Blob handle (5.18.0's streamed media downloads).
    pub blob: bool,
    /// `options.bodyIdleTimeout` for `blob` downloads (default 15 s).
    pub body_idle_ms: f64,
}

impl Default for Req {
    fn default() -> Self {
        Req { method: "GET", url: String::new(), headers: String::new(), body: Body::None, timeout_ms: 60000.0, accept: &[], keepalive: false, blob: false, body_idle_ms: 15000.0 }
    }
}

impl Req {
    pub fn get(url: &str) -> Req { Req { url: url.into(), ..Default::default() } }
}

struct Pending {
    /// The caller's id.
    id: u32,
    /// This module's id for the token wait, the deadline timer and the fetch.
    key: u32,
    req: Req,
    started: f64,
    retried: bool,
    fetching: bool,
}

#[derive(Default)]
pub struct State {
    pending: Vec<Pending>,
}

pub fn init(_app: &mut App, _host: &mut dyn Host) {}

/// Starts a Drive request for module `owner`; returns the id of the `Event::Io` that completes it.
// v5: app-v5.js:driveFetch
pub fn request(app: &mut App, host: &mut dyn Host, owner: Mod, req: Req) -> u32 {
    let (id, key) = (app.req(owner), app.req(Mod::DriveClient));
    app.sched.timer(host.out(), key, req.timeout_ms);
    app.st.drive_client.pending.push(Pending { id, key, req, started: host.now(), retried: false, fetching: false });
    acquire(app, host, key);
    id
}

/// Aborts a request (5.18.0 `options.signal`); the caller gets no event for it.
pub fn abort(app: &mut App, host: &mut dyn Host, id: u32) {
    let pending = &mut app.st.drive_client.pending;
    if let Some(at) = pending.iter().position(|p| p.id == id) {
        let p = pending.swap_remove(at);
        app.sched.clear(host.out(), p.key);
        if p.fetching {
            fx::fetch_abort(host.out(), p.key);
        }
    }
}

/// Requests in flight (used by exit and test code).
pub fn busy(app: &App) -> usize { app.st.drive_client.pending.len() }

fn acquire(app: &mut App, host: &mut dyn Host, key: u32) {
    match auth::ensure_token(app, host, false, key) {
        Token::Ready(token) => send(app, host, key, &token),
        Token::Failed(message) => fail(app, host, key, err::AUTH, &message),
        Token::Pending => {}
    }
}

fn send(app: &mut App, host: &mut dyn Host, key: u32, token: &str) {
    let now = host.now();
    let Some(p) = app.st.drive_client.pending.iter_mut().find(|p| p.key == key) else { return };
    p.fetching = true;
    let r = &p.req;
    let remaining = (r.timeout_ms - (now - p.started)).max(1.0);
    let headers = format!("{}Authorization: Bearer {token}\n", r.headers);
    let out = host.out();
    // From here the deadline is enforced where the request runs, so a body read is covered too.
    app.sched.clear(out, key);
    fx::fetch(out, key, &fx::Fetch {
        method: r.method,
        url: &r.url,
        headers: &headers,
        flags: if r.keepalive { fx::fetch_flag::KEEPALIVE } else { 0 } | if r.blob { fx::fetch_flag::BLOB } else { 0 },
        deadline_ms: remaining,
        idle_ms: if r.blob { r.timeout_ms.min(if r.body_idle_ms > 0.0 { r.body_idle_ms } else { 15000.0 }) } else { 0.0 },
        body: match &r.body {
            Body::None => fx::Src::None,
            Body::Bytes(b) => fx::Src::Bytes(b),
            Body::Blob { obj, start, end } => fx::Src::Blob { obj: *obj, start: *start, end: *end },
        },
    });
}

/// 5.18.0: `/auth|token|sign.?in|refresh|No refresh token|Silent token/i.test(message)`.
fn is_auth_error(message: &str) -> bool {
    let m = message.to_ascii_lowercase();
    let b = m.as_bytes();
    let sign_in = m.match_indices("sign").any(|(at, _)| {
        let rest = &b[at + 4..];
        rest.starts_with(b"in") || (rest.len() >= 3 && rest[0] != b'\n' && &rest[1..3] == b"in")
    });
    m.contains("auth") || m.contains("token") || m.contains("refresh") || sign_in
}

fn take(app: &mut App, host: &mut dyn Host, key: u32) -> Option<Pending> {
    let pending = &mut app.st.drive_client.pending;
    let p = pending.swap_remove(pending.iter().position(|p| p.key == key)?);
    app.sched.clear(host.out(), key);
    Some(p)
}

fn fail(app: &mut App, host: &mut dyn Host, key: u32, status: i32, message: &str) {
    let Some(p) = take(app, host, key) else { return };
    // A failed retry after a 401, or any error that speaks of auth, pauses Drive until the user signs in again.
    if !auth::signed_out(app) && (p.retried || is_auth_error(message)) {
        auth::mark_needs_reconnect(app, host, message);
    }
    shell::deliver(app, host, p.id, status, 0, message.as_bytes());
}

fn completed(app: &mut App, host: &mut dyn Host, io: &crate::Io) {
    let key = io.req;
    let Some(p) = app.st.drive_client.pending.iter().find(|p| p.key == key) else { return };
    if !p.fetching {
        // The token arrived (or did not).
        return match io.status {
            0 => send(app, host, key, &String::from_utf8_lossy(io.data)),
            _ => fail(app, host, key, err::AUTH, &String::from_utf8_lossy(io.data)),
        };
    }
    let (blob, retried, accepted) = (p.req.blob, p.retried, p.req.accept.contains(&io.status));
    if io.status < 0 {
        let message = match io.status {
            fx::status::TIMEOUT => "Drive request timed out".into(),
            fx::status::BODY_TIMEOUT if blob => "Drive download timed out before the PDF finished".into(),
            fx::status::BODY_TIMEOUT => "Drive request timed out".into(),
            _ => String::from_utf8_lossy(io.data).into_owned(),
        };
        let status = if io.status == fx::status::FAILED { err::NETWORK } else { err::TIMEOUT };
        return fail(app, host, key, status, &message);
    }
    shell::network_available(app, host);
    if io.status == 401 && !retried {
        host.out().obj_release(io.obj);
        if auth::signed_out(app) {
            return fail(app, host, key, -401, "Drive API error 401: signed out");
        }
        // Token expired server-side — force a refresh and retry once.
        auth::drop_token(app);
        let fresh = app.req(Mod::DriveClient);
        let now = host.now();
        if let Some(p) = app.st.drive_client.pending.iter_mut().find(|p| p.key == key) {
            p.key = fresh;
            p.retried = true;
            p.fetching = false;
            p.started = now;
            app.sched.timer(host.out(), fresh, p.req.timeout_ms);
        }
        return acquire(app, host, fresh);
    }
    if !(200..300).contains(&io.status) && !accepted {
        host.out().obj_release(io.obj);
        let body = if blob { "" } else { fx::Response::parse(io.data).text() };
        let message = format!("Drive API error {}: {body}", io.status);
        return fail(app, host, key, -io.status, &message);
    }
    if let Some(p) = take(app, host, key) {
        shell::deliver(app, host, p.id, io.status, io.obj, io.data);
    }
}

pub fn event(app: &mut App, host: &mut dyn Host, event: &Event) -> u32 {
    match *event {
        // The deadline passed while waiting for the token.
        Event::Timer(key) => fail(app, host, key, err::TIMEOUT, "Drive request timed out"),
        Event::Io(io) => completed(app, host, &io),
        _ => {}
    }
    0
}

pub fn test_cmd(app: &mut App, host: &mut dyn Host, name: &str, args: &str) -> Option<String> {
    let arg = shell::json::Json::parse(args)?;
    Some(match name {
        // A request owned by the shell: its completion shows up in `shell.fx_results`.
        "request" => {
            let method = match arg.text("method") {
                "POST" => "POST",
                "PATCH" => "PATCH",
                "PUT" => "PUT",
                "DELETE" => "DELETE",
                _ => "GET",
            };
            let body = match arg.get("body") {
                Some(b) => Body::Bytes(b.as_str().unwrap_or("").as_bytes().to_vec()),
                None => Body::None,
            };
            let timeout_ms = arg.get("timeout").and_then(shell::json::Json::as_f64).unwrap_or(60000.0);
            let accept: &'static [i32] = if arg.get("accept404").is_some() { &[404] } else { &[] };
            let req = Req { method, url: arg.text("url").into(), headers: arg.text("headers").into(), body, timeout_ms, accept, blob: arg.get("blob").is_some(), ..Default::default() };
            request(app, host, Mod::Shell, req).to_string()
        }
        "abort" => {
            abort(app, host, arg.get("req").and_then(shell::json::Json::as_f64).unwrap_or(0.0) as u32);
            "true".into()
        }
        "busy" => busy(app).to_string(),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_errors_match_the_v5_pattern() {
        for yes in ["No refresh token; user must sign in", "Silent token refresh timed out", "Drive auth not ready", "Refresh failed: 400", "sign-in needed", "Sign in", "SIGNIN", "Drive API error 403: insufficient authentication"] {
            assert!(is_auth_error(yes), "{yes}");
        }
        for no in ["Drive request timed out", "Failed to fetch", "Drive API error 404: File not found", "[object Object]", "design", "sign", "signal", "Signed out"] {
            assert!(!is_auth_error(no), "{no}");
        }
    }
}

#[cfg(test)]
mod flow_tests {
    use super::*;
    use crate::shell::testkit::*;

    fn signed_in() -> (App, crate::host::FakeHost) {
        let (mut app, mut host) = boot(fx::BOOT_SIGNED_IN, |h| {
            let payload = format!("{{\"token\":\"tok\",\"expiry\":{},\"grantedScopes\":\"{}\"}}", h.time + 3_600_000.0, auth::DRIVE_SCOPE);
            h.local.insert("drive-token-v1".into(), payload);
        });
        // Answer the boot requests (connectivity probe, profile) so only the test's own remain.
        let text = out(&mut host);
        for line in text.lines().filter(|l| l.starts_with("fetch ")) {
            let req = line.split(' ').nth(1).unwrap().parse().unwrap();
            io(&mut app, &mut host, req, 200, &response("", "{}"));
        }
        out(&mut host);
        (app, host)
    }
    fn results(app: &mut App) -> Vec<(i32, u32, String)> {
        std::mem::take(&mut app.st.shell.probe.results).into_iter().map(|r| (r.1, r.2, String::from_utf8_lossy(&r.3).into_owned())).collect()
    }

    #[test]
    fn requests_carry_the_token_and_map_errors_like_v5() {
        let (mut app, mut host) = signed_in();
        let req = Req { method: "PATCH", url: "https://www.googleapis.com/upload/drive/v3/files/f?uploadType=media".into(), headers: "If-Match: \"7\"\n".into(), body: Body::Blob { obj: 9, start: 0.0, end: 100.0 }, timeout_ms: 25000.0, accept: &[412], keepalive: true, ..Default::default() };
        let id = request(&mut app, &mut host, Mod::Shell, req);
        let text = out(&mut host);
        let key = req_of(&text, "fetch");
        assert_eq!(
            text,
            format!("timer_set {key} 25000 0\ntimer_clear {key}\nfetch {key} \"PATCH\" \"https://www.googleapis.com/upload/drive/v3/files/f?uploadType=media\" \"If-Match: \\\"7\\\"\\nAuthorization: Bearer tok\\n\" 1 25000 0 blob 9 0..100\n")
        );
        // An accepted status is a result, not an error.
        io(&mut app, &mut host, key, 412, &response("etag: \"8\"\n", "{}"));
        let got = results(&mut app);
        assert_eq!(got.len(), 1);
        assert_eq!((got[0].0, fx::Response::parse(got[0].2.as_bytes()).header("etag")), (412, Some("\"8\"")));
        assert!(id != key && busy(&app) == 0);

        // Other non-2xx answers: `Drive API error <status>: <body>`, status = -(HTTP status).
        request(&mut app, &mut host, Mod::Shell, Req::get("https://www.googleapis.com/drive/v3/files/nope"));
        let key = req_of(&out(&mut host), "fetch");
        io(&mut app, &mut host, key, 404, &response("", "{\"error\":\"notFound\"}"));
        assert_eq!(results(&mut app), [(-404, 0, "Drive API error 404: {\"error\":\"notFound\"}".to_string())]);
        assert!(!auth::reconnecting(&app));
        // A body that speaks of auth pauses Drive (5.18.0's pattern test).
        request(&mut app, &mut host, Mod::Shell, Req::get("https://www.googleapis.com/drive/v3/files/x"));
        let key = req_of(&out(&mut host), "fetch");
        io(&mut app, &mut host, key, 403, &response("", "Insufficient authentication scopes"));
        assert_eq!(results(&mut app)[0].0, -403);
        assert!(auth::reconnecting(&app));
    }

    #[test]
    fn timeouts_network_errors_downloads_and_abort() {
        let (mut app, mut host) = signed_in();
        let get = |app: &mut App, host: &mut crate::host::FakeHost, blob: bool| {
            request(app, host, Mod::Shell, Req { url: "https://www.googleapis.com/drive/v3/files/f?alt=media".into(), blob, timeout_ms: 120000.0, ..Default::default() });
            let text = out(host);
            (req_of(&text, "fetch"), text)
        };
        let (key, text) = get(&mut app, &mut host, true);
        assert!(text.ends_with("\"Authorization: Bearer tok\\n\" 2 120000 15000 -\n"), "{text}");
        io(&mut app, &mut host, key, fx::status::BODY_TIMEOUT, b"");
        assert_eq!(results(&mut app), [(err::TIMEOUT, 0, "Drive download timed out before the PDF finished".to_string())]);
        let (key, _) = get(&mut app, &mut host, false);
        io(&mut app, &mut host, key, fx::status::BODY_TIMEOUT, b"");
        let (key2, _) = get(&mut app, &mut host, false);
        io(&mut app, &mut host, key2, fx::status::TIMEOUT, b"");
        assert_eq!(results(&mut app), [(err::TIMEOUT, 0, "Drive request timed out".to_string()), (err::TIMEOUT, 0, "Drive request timed out".to_string())]);
        // A network failure marks nothing; the next answer from Drive proves the network again.
        let (key, _) = get(&mut app, &mut host, false);
        io(&mut app, &mut host, key, fx::status::FAILED, b"Failed to fetch");
        assert_eq!(results(&mut app), [(err::NETWORK, 0, "Failed to fetch".to_string())]);
        assert!(!auth::reconnecting(&app));
        // A successful download hands over the Blob handle.
        let (key, _) = get(&mut app, &mut host, true);
        let mut w = ihn_core::codec::Writer::default();
        w.u32(key);
        w.i32(200);
        w.u32(55);
        w.raw(&response("content-type: application/pdf\n", ""));
        app.event(&mut host, crate::abi::kind::IO, &w.buf);
        let got = results(&mut app);
        assert_eq!((got[0].0, got[0].1), (200, 55));
        // Abort: the fetch is cancelled and the caller hears nothing more.
        let id = request(&mut app, &mut host, Mod::Shell, Req::get("https://www.googleapis.com/drive/v3/files"));
        let key = req_of(&out(&mut host), "fetch");
        abort(&mut app, &mut host, id);
        assert_eq!(out(&mut host), format!("timer_clear {key}\nfetch_abort {key}\n"));
        io(&mut app, &mut host, key, fx::status::ABORTED, b"");
        assert!(results(&mut app).is_empty() && busy(&app) == 0);
    }

    #[test]
    fn the_deadline_covers_waiting_for_the_token() {
        let (mut app, mut host) = boot(fx::BOOT_SIGNED_IN, |h| {
            h.local.insert("drive-remember-v1".into(), "1".into());
        });
        let text = out(&mut host);
        io(&mut app, &mut host, req_of(&text, "gis_init"), 0, b"");
        let silent = req_of(&out(&mut host), "gis_request");
        request(&mut app, &mut host, Mod::Shell, Req { url: "https://www.googleapis.com/drive/v3/files".into(), timeout_ms: 5000.0, ..Default::default() });
        let text = out(&mut host);
        assert!(!text.contains("fetch") && !text.contains("gis_request"), "it joins the running token request: {text}");
        let key = req_of(&text, "timer_set");
        timer(&mut app, &mut host, key);
        assert_eq!(std::mem::take(&mut app.st.shell.probe.results)[0].1, err::TIMEOUT);
        // The token arriving later starts nothing for the request that already failed.
        io(&mut app, &mut host, silent, 0, TOKEN_JSON.as_bytes());
        assert!(!out(&mut host).contains("drive/v3/files\""));
        // With time already spent on the token, the fetch gets what is left of the deadline.
        auth::drop_token(&mut app);
        request(&mut app, &mut host, Mod::Shell, Req { url: "https://www.googleapis.com/drive/v3/files".into(), timeout_ms: 5000.0, ..Default::default() });
        let silent = req_of(&out(&mut host), "gis_request");
        host.advance(2000.0);
        io(&mut app, &mut host, silent, 0, TOKEN_JSON.as_bytes());
        assert!(line_of(&out(&mut host), "fetch").ends_with("\"Authorization: Bearer tok-1\\n\" 0 3000 0 -"));
    }
}
