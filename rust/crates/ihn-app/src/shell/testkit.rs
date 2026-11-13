//! Helpers for cargo tests that drive the app through `FakeHost`: boot, feed events, read the records written.

use crate::abi::{self, ety, mode, Config, Node};
use crate::host::FakeHost;
use crate::shell::fx;
use crate::App;
use ihn_core::codec::Writer;

pub const TOKEN_JSON: &str = r#"{"access_token":"tok-1","expires_in":3600,"scope":"https://www.googleapis.com/auth/drive","token_type":"Bearer"}"#;

pub fn config(flags: u32, url: &str) -> Config {
    Config {
        version: "6.0.0".into(),
        url: url.into(),
        user_agent: "test".into(),
        language: "en".into(),
        flags: flags | mode::E2E | mode::ONLINE,
        viewport: (1280.0, 800.0),
        dpr: 1.0,
        max_touch_points: 0,
    }
}

/// Boots the app with `flags` after `setup` has prepared storage.
pub fn boot_at(flags: u32, url: &str, setup: impl FnOnce(&mut FakeHost)) -> (App, FakeHost) {
    let (mut app, mut host) = (App::new(config(flags, url)), FakeHost::default());
    setup(&mut host);
    app.init(&mut host);
    (app, host)
}
pub fn boot(flags: u32, setup: impl FnOnce(&mut FakeHost)) -> (App, FakeHost) { boot_at(flags, "http://127.0.0.1:4400/?e2e=1", setup) }

/// The records written since the last call, one per line.
pub fn out(host: &mut FakeHost) -> String {
    let text = fx::dump(&host.out.w.buf);
    host.out.w.buf.clear();
    text
}

/// The request id (first field) of the last `op` record in `text`.
pub fn req_of(text: &str, op: &str) -> u32 {
    let line = text.lines().rev().find(|l| l.split(' ').next() == Some(op)).unwrap_or_else(|| panic!("no {op} in:\n{text}"));
    line.split(' ').nth(1).and_then(|n| n.parse().ok()).expect("request id")
}
pub fn line_of<'a>(text: &'a str, op: &str) -> &'a str {
    text.lines().rev().find(|l| l.split(' ').next() == Some(op)).unwrap_or_else(|| panic!("no {op} in:\n{text}"))
}

pub fn io(app: &mut App, host: &mut FakeHost, req: u32, status: i32, data: &[u8]) {
    let mut w = Writer::default();
    w.u32(req);
    w.i32(status);
    w.u32(0);
    w.raw(data);
    app.event(host, abi::kind::IO, &w.buf);
}
/// A fetch completion payload.
pub fn response(headers: &str, body: &str) -> Vec<u8> {
    let mut w = Writer::default();
    w.str(headers);
    w.raw(body.as_bytes());
    w.buf
}
pub fn timer(app: &mut App, host: &mut FakeHost, id: u32) {
    let mut w = Writer::default();
    w.u32(id);
    app.event(host, abi::kind::TIMER, &w.buf);
}
pub fn msg(app: &mut App, host: &mut FakeHost, chan: u8, data: &str) -> u32 {
    let mut w = Writer::default();
    w.u8(chan);
    w.u32(0);
    w.raw(data.as_bytes());
    app.event(host, abi::kind::MESSAGE, &w.buf)
}
pub fn life(app: &mut App, host: &mut FakeHost, code: u8, a: f64) {
    let mut w = Writer::default();
    w.u8(code);
    for v in [a, 0.0, 0.0] {
        w.f64(v);
    }
    w.str("");
    app.event(host, abi::kind::LIFECYCLE, &w.buf);
}
/// A DOM event of type `ty` on `node` for `listener`; `path` = the known nodes from the target upwards.
pub fn dom(app: &mut App, host: &mut FakeHost, listener: u32, node: Node, ty: u8, flags: u32, key: &str, path: &[Node]) -> u32 {
    let mut w = Writer::default();
    w.u32(listener);
    w.u32(node.0);
    w.u8(ty);
    w.u32(flags);
    w.f64(host.now);
    w.i32(0);
    w.u32(0);
    for _ in 0..4 {
        w.f64(0.0);
    }
    w.u32(0);
    w.i32(0);
    w.u8(0);
    w.u32(0);
    w.str(key);
    w.str("");
    w.str("");
    w.u32(path.len() as u32);
    path.iter().for_each(|n| w.u32(n.0));
    for _ in 0..3 {
        w.u32(0);
    }
    app.event(host, abi::kind::DOM, &w.buf)
}
pub fn click(app: &mut App, host: &mut FakeHost, listener: u32, node: Node) -> u32 {
    dom(app, host, listener, node, ety::CLICK, 0, "", &[node, Node::BODY, Node::HTML, Node::DOCUMENT])
}

/// The signals broadcast since boot or the last call, as `(sig, data)`.
pub fn signals(app: &mut App) -> Vec<(u8, String)> { std::mem::take(&mut app.st.shell.probe_log) }
