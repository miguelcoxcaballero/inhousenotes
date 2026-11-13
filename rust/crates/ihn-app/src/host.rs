//! `trait Host`: the Rust side of the browser boundary (ARCHITECTURE §3.4). `ihn-app` never calls `extern`
//! functions; `ihn-wasm` implements this trait with the wasm imports and `FakeHost` implements it for cargo tests.

use crate::abi::{Node, OutBuf};

/// `getBoundingClientRect()`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn right(&self) -> f64 { self.x + self.w }
    pub fn bottom(&self) -> f64 { self.y + self.h }
}

/// Synchronous layout reads (`host_metric`). The node is ignored for the window-level metrics.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum Metric {
    ScrollTop = 0,
    ScrollLeft = 1,
    ScrollHeight = 2,
    ScrollWidth = 3,
    ClientWidth = 4,
    ClientHeight = 5,
    OffsetTop = 6,
    OffsetLeft = 7,
    OffsetWidth = 8,
    OffsetHeight = 9,
    InnerWidth = 10,
    InnerHeight = 11,
    DevicePixelRatio = 12,
    /// `visualViewport.width` … (window values when there is no visual viewport).
    VisualWidth = 13,
    VisualHeight = 14,
    VisualOffsetLeft = 15,
    VisualOffsetTop = 16,
    VisualScale = 17,
    NaturalWidth = 18,
    NaturalHeight = 19,
    /// 1 when the node is connected to the document, 0 otherwise (also for unknown handles).
    Connected = 20,
    ScrollX = 21,
    ScrollY = 22,
}

/// `host_ls` operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum LsOp {
    Get = 0,
    Set = 1,
    Remove = 2,
    SessionGet = 4,
    SessionSet = 5,
    SessionRemove = 6,
}

/// `host_intl` operations. The input is a list of fields separated by `'\0'`; JSON fields may be empty
/// (= `undefined`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum IntlOp {
    /// `epoch_ms \0 method \0 locales_json \0 options_json` → `new Date(ms).toLocale{Time,Date,}String(…)`
    /// (`method` = `time` | `date` | `string`).
    Date = 0,
    /// `a \0 b \0 locales_json \0 options_json` → `Intl.Collator(…).compare(a, b)` as `"-1"`, `"0"` or `"1"`.
    Compare = 1,
    /// `text` → `text.toLocaleLowerCase()`.
    Lower = 2,
    /// `text` → `text.toLocaleUpperCase()`.
    Upper = 3,
    /// `number \0 locales_json \0 options_json` → `Number(number).toLocaleString(…)`.
    Number = 4,
}

/// Synchronous platform questions answered as text through the `host_intl` import (ops `16..`); each implies a
/// flush. Use the typed methods of [`Host`] (`style`, `media`, `hit`, `active`, `has_capture`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum Ask {
    /// `handle \0 property` → `getComputedStyle(node)[property]` (`getPropertyValue` for names containing `-`).
    Style = 16,
    /// `query` → `matchMedia(query).matches` as `"1"` / `"0"`.
    Media = 17,
    /// `x \0 y` → the known handles from `document.elementFromPoint(x, y)` upwards, comma-separated.
    Hit = 18,
    /// The known handles from `document.activeElement` upwards, comma-separated.
    Active = 19,
    /// `handle \0 pointerId` → `node.hasPointerCapture(pointerId)` as `"1"` / `"0"`.
    Capture = 20,
}

fn nodes(list: &str) -> Vec<Node> { list.split(',').filter_map(|h| h.parse().ok()).map(Node).collect() }

pub trait Host {
    /// DOM / canvas / effect records.
    fn out(&mut self) -> &mut OutBuf;
    /// Applies the records written so far (before depending on their effect outside a measurement).
    fn flush(&mut self);
    /// `getBoundingClientRect` (implies flush).
    fn rect(&mut self, h: Node) -> Rect;
    /// A layout number (implies flush).
    fn metric(&mut self, h: Node, m: Metric) -> f64;
    /// `performance.now()`.
    fn now(&self) -> f64;
    /// `Date.now()`.
    fn time(&self) -> f64;
    /// `crypto.getRandomValues`.
    fn random(&mut self, buf: &mut [u8]);
    /// `localStorage` / `sessionStorage`. `Get` returns the value; `Set`/`Remove` return `Some("")` on success and
    /// `None` when storage threw (quota, disabled), which callers may ignore as 5.18.0's `try {…} catch {}` does.
    fn ls(&mut self, op: LsOp, key: &str, val: Option<&str>) -> Option<String>;
    /// Locale-dependent formatting computed by the browser.
    fn intl(&mut self, op: IntlOp, input: &str) -> String;
    /// `console.debug/info/warn/error` (`level` 0..=3). Not part of the §3.4 sketch; maps to the `host_log` import.
    fn log(&mut self, level: u32, msg: &str);
    /// The text answer to a platform question (implies flush); empty when the host cannot answer.
    fn ask(&mut self, q: Ask, input: &str) -> String;

    /// `getComputedStyle(h)[prop]` (`"gridTemplateColumns"`, `"transform"`, `"display"`, `"--custom"`, …); empty
    /// for a missing node. Read it when 5.18.0 does, never per pointer sample.
    fn style(&mut self, h: Node, prop: &str) -> String { self.ask(Ask::Style, &format!("{}\0{prop}", h.0)) }
    /// `matchMedia(query).matches`. The colour scheme and reduced-motion preferences are also kept current in
    /// `Config::flags`.
    fn media(&mut self, query: &str) -> bool { self.ask(Ask::Media, query) == "1" }
    /// `document.elementFromPoint(x, y)` as the known nodes from it upwards (nearest first).
    fn hit(&mut self, x: f64, y: f64) -> Vec<Node> { nodes(&self.ask(Ask::Hit, &format!("{x}\0{y}"))) }
    /// `document.activeElement` as the known nodes from it upwards (nearest first): `[0] == Node::BODY` when
    /// nothing is focused, `.contains(&h)` for `h.contains(document.activeElement)`.
    fn active(&mut self) -> Vec<Node> { nodes(&self.ask(Ask::Active, "")) }
    /// `h.hasPointerCapture(pointer_id)`.
    fn has_capture(&mut self, h: Node, pointer_id: i32) -> bool {
        self.ask(Ask::Capture, &format!("{}\0{pointer_id}", h.0)) == "1"
    }
}

/// Scripted host for cargo tests: records the out-buffer, answers measurements from tables, has a manual clock,
/// an in-memory storage and a deterministic random source.
#[cfg(test)]
pub struct FakeHost {
    pub out: OutBuf,
    pub now: f64,
    pub time: f64,
    pub rects: std::collections::HashMap<Node, Rect>,
    pub metrics: std::collections::HashMap<(Node, Metric), f64>,
    /// Scripted answers of [`Host::ask`] by question and input (missing = empty).
    pub answers: std::collections::HashMap<(Ask, String), String>,
    pub local: std::collections::BTreeMap<String, String>,
    pub session: std::collections::BTreeMap<String, String>,
    pub flushes: u32,
    pub logs: Vec<(u32, String)>,
    rng: ihn_core::testutil::Rng,
}

#[cfg(test)]
impl Default for FakeHost {
    fn default() -> Self {
        FakeHost {
            out: OutBuf::default(),
            now: 0.0,
            time: 1_793_354_400_000.0, // 2026-10-30T10:00:00Z, the parity clock (TESTING §3.1)
            rects: Default::default(),
            metrics: Default::default(),
            answers: Default::default(),
            local: Default::default(),
            session: Default::default(),
            flushes: 0,
            logs: Vec::new(),
            rng: ihn_core::testutil::Rng::new(6),
        }
    }
}

#[cfg(test)]
impl FakeHost {
    /// The records written since the last call, as text (`abi::dump`), and clears them.
    pub fn take(&mut self) -> String {
        let text = crate::abi::dump(&self.out.w.buf);
        self.out.w.buf.clear();
        text
    }
    /// Advances both clocks.
    pub fn advance(&mut self, ms: f64) {
        self.now += ms;
        self.time += ms;
    }
}

#[cfg(test)]
impl Host for FakeHost {
    fn out(&mut self) -> &mut OutBuf { &mut self.out }
    fn flush(&mut self) { self.flushes += 1; }
    fn rect(&mut self, h: Node) -> Rect {
        self.flushes += 1;
        self.rects.get(&h).copied().unwrap_or_default()
    }
    fn metric(&mut self, h: Node, m: Metric) -> f64 {
        self.flushes += 1;
        self.metrics.get(&(h, m)).copied().unwrap_or(0.0)
    }
    fn now(&self) -> f64 { self.now }
    fn time(&self) -> f64 { self.time }
    fn random(&mut self, buf: &mut [u8]) { buf.iter_mut().for_each(|b| *b = self.rng.next() as u8); }
    fn ls(&mut self, op: LsOp, key: &str, val: Option<&str>) -> Option<String> {
        let map = if op as u32 >= 4 { &mut self.session } else { &mut self.local };
        match op as u32 & 3 {
            0 => map.get(key).cloned(),
            1 => {
                map.insert(key.into(), val.unwrap_or("").into());
                Some(String::new())
            }
            _ => {
                map.remove(key);
                Some(String::new())
            }
        }
    }
    fn intl(&mut self, op: IntlOp, input: &str) -> String {
        let mut f = input.split('\0');
        match op {
            IntlOp::Lower => input.to_lowercase(),
            IntlOp::Upper => input.to_uppercase(),
            IntlOp::Compare => (f.next().cmp(&f.next()) as i32).to_string(),
            IntlOp::Date | IntlOp::Number => f.next().unwrap_or("").into(),
        }
    }
    fn log(&mut self, level: u32, msg: &str) { self.logs.push((level, msg.into())); }
    fn ask(&mut self, q: Ask, input: &str) -> String {
        self.flushes += 1;
        self.answers.get(&(q, input.to_string())).cloned().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_questions_encode_their_input_and_parse_the_answer() {
        let mut host = FakeHost::default();
        host.answers.insert((Ask::Style, "20\0transform".into()), "matrix(1, 0, 0, 1, 0, 0)".into());
        host.answers.insert((Ask::Media, "(prefers-reduced-motion: reduce)".into()), "1".into());
        host.answers.insert((Ask::Hit, "10.5\0-2".into()), "1500,20,4,3,1".into());
        host.answers.insert((Ask::Active, String::new()), "4,3,1".into());
        host.answers.insert((Ask::Capture, "20\0-7".into()), "1".into());
        assert_eq!(host.style(Node(20), "transform"), "matrix(1, 0, 0, 1, 0, 0)");
        assert_eq!(host.style(Node(21), "transform"), "");
        assert!(host.media("(prefers-reduced-motion: reduce)") && !host.media("(max-width: 720px)"));
        assert_eq!(host.hit(10.5, -2.0), [Node(1500), Node(20), Node::BODY, Node::HTML, Node::DOCUMENT]);
        assert_eq!(host.hit(0.0, 0.0), []);
        assert_eq!(host.active()[0], Node::BODY);
        assert!(host.has_capture(Node(20), -7) && !host.has_capture(Node(20), 1));
        assert_eq!(host.flushes, 9);
    }
}
