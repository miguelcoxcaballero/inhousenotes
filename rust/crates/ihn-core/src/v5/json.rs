//! Minimal JSON reader and writer for 5.x payloads, and the value tree the 5.x readers consume.
//!
//! `Value` is also the input type of `v5::idb`: `ihn-app::store::sv::Value` (structured clones of IndexedDB records)
//! maps onto it one to one (`Bytes` = typed arrays, `Blob` = a host object handle).
//! `stringify` and `stable` write what `JSON.stringify` writes (number text from `jsfmt`), because 5.x hashes such
//! text (timeline integrity, `contentHash`).

use crate::jsfmt;
use std::cmp::Ordering;

#[derive(Clone, Debug, PartialEq, Default)]
pub enum Value {
    #[default]
    Undef,
    Null,
    Bool(bool),
    Num(f64),
    Str(Box<str>),
    Arr(Vec<Value>),
    /// In source order. A repeated key keeps every entry; readers take the last one, as `JSON.parse` does.
    Obj(Vec<(Box<str>, Value)>),
    /// A stroke point `{"x":…,"y":…[,"p":…]}` (`p` = NaN when absent): 24 bytes instead of an object.
    Pt { x: f64, y: f64, p: f64 },
    Bytes(Vec<u8>),
    Blob { obj: u32, size: f64 },
}

static UNDEF: Value = Value::Undef;

impl Value {
    /// `value[key]`; `Undef` when missing or not an object.
    pub fn get(&self, key: &str) -> &Value {
        match self {
            Value::Obj(fields) => fields.iter().rev().find(|(k, _)| &**k == key).map_or(&UNDEF, |(_, v)| v),
            _ => &UNDEF,
        }
    }
    /// `Object.prototype.hasOwnProperty.call(value, key)`.
    pub fn has(&self, key: &str) -> bool { matches!(self, Value::Obj(f) if f.iter().any(|(k, _)| &**k == key)) }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }
    /// The elements of an array; empty for anything else (`Array.isArray(v) ? v : []`).
    pub fn arr(&self) -> &[Value] {
        match self {
            Value::Arr(a) => a,
            _ => &[],
        }
    }
    pub fn is_arr(&self) -> bool { matches!(self, Value::Arr(_)) }
    /// `value && typeof value === 'object'` (arrays included).
    pub fn is_object(&self) -> bool { matches!(self, Value::Obj(_) | Value::Arr(_) | Value::Pt { .. } | Value::Bytes(_) | Value::Blob { .. }) }
    /// Own entries of a plain object in source order (a repeated key appears more than once; the last one counts).
    pub fn entries(&self) -> impl Iterator<Item = (&str, &Value)> {
        let fields: &[(Box<str>, Value)] = match self {
            Value::Obj(f) => f,
            _ => &[],
        };
        fields.iter().map(|(k, v)| (&**k, v))
    }
    /// JavaScript truthiness.
    pub fn truthy(&self) -> bool {
        match self {
            Value::Undef | Value::Null => false,
            Value::Bool(b) => *b,
            Value::Num(n) => *n != 0.0 && !n.is_nan(),
            Value::Str(s) => !s.is_empty(),
            _ => true,
        }
    }
    /// `Number(value)` for the types 5.x data holds (objects → NaN).
    pub fn num(&self) -> f64 {
        match self {
            Value::Num(n) => *n,
            Value::Null => 0.0,
            Value::Bool(b) => *b as u8 as f64,
            Value::Str(s) => {
                let t = s.trim();
                if t.is_empty() {
                    0.0
                } else if t.bytes().all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'-' | b'+' | b'e' | b'E')) {
                    t.parse().unwrap_or(f64::NAN)
                } else {
                    f64::NAN
                }
            }
            _ => f64::NAN,
        }
    }
    /// The number when `typeof value === 'number'` and finite (`Number.isFinite(value)`).
    pub fn finite(&self) -> Option<f64> {
        match self {
            Value::Num(n) if n.is_finite() => Some(*n),
            _ => None,
        }
    }
    /// `String(value || '')` for strings and numbers (other truthy values → empty: 5.x never stores them as ids).
    pub fn text(&self) -> String {
        match self {
            Value::Str(s) => s.to_string(),
            Value::Num(n) if self.truthy() => jsfmt::num(*n),
            Value::Bool(true) => "true".into(),
            _ => String::new(),
        }
    }
}

/// `a < b` as JavaScript compares strings (UTF-16 code units).
pub fn cmp_utf16(a: &str, b: &str) -> Ordering {
    if a.is_ascii() && b.is_ascii() {
        a.cmp(b)
    } else {
        a.encode_utf16().cmp(b.encode_utf16())
    }
}

/// `s.slice(0, n)` in UTF-16 code units (a split surrogate pair is dropped).
pub fn js_slice(s: &str, n: usize) -> &str {
    let mut units = 0;
    for (i, c) in s.char_indices() {
        units += c.len_utf16();
        if units > n {
            return &s[..i];
        }
    }
    s
}

// ---------------------------------------------------------------------------------------------------- reader

const MAX_DEPTH: u32 = 256;

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

/// `JSON.parse(text)`; `None` = SyntaxError.
pub fn parse(text: &[u8]) -> Option<Value> {
    let mut p = Parser { s: text, i: 0 };
    let v = p.value(0)?;
    p.ws();
    (p.i == p.s.len()).then_some(v)
}

impl Parser<'_> {
    fn ws(&mut self) {
        while let Some(b' ' | b'\n' | b'\r' | b'\t') = self.s.get(self.i) {
            self.i += 1;
        }
    }
    fn eat(&mut self, lit: &[u8]) -> Option<()> {
        self.s[self.i..].starts_with(lit).then(|| self.i += lit.len())
    }
    fn value(&mut self, depth: u32) -> Option<Value> {
        self.ws();
        match *self.s.get(self.i)? {
            b'{' => self.object(depth),
            b'[' => {
                if depth >= MAX_DEPTH {
                    return None;
                }
                self.i += 1;
                let mut out = Vec::new();
                self.ws();
                if self.s.get(self.i) == Some(&b']') {
                    self.i += 1;
                    return Some(Value::Arr(out));
                }
                loop {
                    out.push(self.value(depth + 1)?);
                    self.ws();
                    match *self.s.get(self.i)? {
                        b',' => self.i += 1,
                        b']' => {
                            self.i += 1;
                            out.shrink_to_fit();
                            return Some(Value::Arr(out));
                        }
                        _ => return None,
                    }
                }
            }
            b'"' => self.string().map(Value::Str),
            b't' => self.eat(b"true").map(|_| Value::Bool(true)),
            b'f' => self.eat(b"false").map(|_| Value::Bool(false)),
            b'n' => self.eat(b"null").map(|_| Value::Null),
            _ => self.number().map(Value::Num),
        }
    }
    /// The form 5.x writes for points, without allocating: `{"x":n,"y":n}` or `{"x":n,"y":n,"p":n}`.
    fn point(&mut self) -> Option<Value> {
        let start = self.i;
        let mut go = || {
            self.eat(b"{\"x\":")?;
            let x = self.number()?;
            self.eat(b",\"y\":")?;
            let y = self.number()?;
            let p = if self.eat(b",\"p\":").is_some() { self.number()? } else { f64::NAN };
            self.eat(b"}")?;
            Some(Value::Pt { x, y, p })
        };
        let v = go();
        if v.is_none() {
            self.i = start;
        }
        v
    }
    fn object(&mut self, depth: u32) -> Option<Value> {
        if let Some(pt) = self.point() {
            return Some(pt);
        }
        if depth >= MAX_DEPTH {
            return None;
        }
        self.i += 1;
        let mut out = Vec::new();
        self.ws();
        if self.s.get(self.i) == Some(&b'}') {
            self.i += 1;
            return Some(Value::Obj(out));
        }
        loop {
            self.ws();
            let key = self.string()?;
            self.ws();
            self.eat(b":")?;
            let v = self.value(depth + 1)?;
            out.push((key, v));
            self.ws();
            match *self.s.get(self.i)? {
                b',' => self.i += 1,
                b'}' => {
                    self.i += 1;
                    out.shrink_to_fit();
                    return Some(Value::Obj(out));
                }
                _ => return None,
            }
        }
    }
    fn number(&mut self) -> Option<f64> {
        let start = self.i;
        let digits = |p: &mut Self| {
            let from = p.i;
            while p.s.get(p.i).is_some_and(u8::is_ascii_digit) {
                p.i += 1;
            }
            p.i - from
        };
        if self.s.get(self.i) == Some(&b'-') {
            self.i += 1;
        }
        let int = digits(self);
        if int == 0 || (int > 1 && self.s[self.i - int] == b'0') {
            return None;
        }
        if self.s.get(self.i) == Some(&b'.') {
            self.i += 1;
            if digits(self) == 0 {
                return None;
            }
        }
        if let Some(b'e' | b'E') = self.s.get(self.i) {
            self.i += 1;
            if let Some(b'+' | b'-') = self.s.get(self.i) {
                self.i += 1;
            }
            if digits(self) == 0 {
                return None;
            }
        }
        // The slice is ASCII digits, sign, dot and exponent; Rust's parser is correctly rounded, as V8's is.
        std::str::from_utf8(&self.s[start..self.i]).ok()?.parse().ok()
    }
    fn hex4(&mut self) -> Option<u32> {
        let h = self.s.get(self.i..self.i + 4)?;
        self.i += 4;
        u32::from_str_radix(std::str::from_utf8(h).ok()?, 16).ok().filter(|_| h.iter().all(u8::is_ascii_hexdigit))
    }
    fn string(&mut self) -> Option<Box<str>> {
        if self.s.get(self.i) != Some(&b'"') {
            return None;
        }
        self.i += 1;
        let start = self.i;
        // Fast path: no escapes.
        loop {
            match *self.s.get(self.i)? {
                b'"' => {
                    let s = std::str::from_utf8(&self.s[start..self.i]).ok()?;
                    self.i += 1;
                    return Some(s.into());
                }
                b'\\' => break,
                0..=0x1f => return None,
                _ => self.i += 1,
            }
        }
        let mut out = Vec::from(&self.s[start..self.i]);
        loop {
            match *self.s.get(self.i)? {
                b'"' => {
                    self.i += 1;
                    return String::from_utf8(out).ok().map(String::into_boxed_str);
                }
                b'\\' => {
                    self.i += 1;
                    let c = *self.s.get(self.i)?;
                    self.i += 1;
                    let ch = match c {
                        b'"' | b'\\' | b'/' => c as char,
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => {
                            let hi = self.hex4()?;
                            let code = if (0xd800..0xdc00).contains(&hi) && self.s[self.i..].starts_with(b"\\u") {
                                let back = self.i;
                                self.i += 2;
                                let lo = self.hex4()?;
                                if (0xdc00..0xe000).contains(&lo) {
                                    0x10000 + ((hi - 0xd800) << 10) + (lo - 0xdc00)
                                } else {
                                    self.i = back;
                                    hi
                                }
                            } else {
                                hi
                            };
                            // A lone surrogate cannot live in a Rust string: U+FFFD, as `TextEncoder` writes it.
                            char::from_u32(code).unwrap_or('\u{fffd}')
                        }
                        _ => return None,
                    };
                    out.extend_from_slice(ch.encode_utf8(&mut [0; 4]).as_bytes());
                }
                0..=0x1f => return None,
                b => {
                    out.push(b);
                    self.i += 1;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------------- writer

pub fn push_str(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

fn push_num(out: &mut String, n: f64) {
    if n.is_finite() {
        jsfmt::push_num(out, n);
    } else {
        out.push_str("null");
    }
}

fn write(out: &mut String, v: &Value, sorted: bool) {
    match v {
        Value::Undef | Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Num(n) => push_num(out, *n),
        Value::Str(s) => push_str(out, s),
        Value::Arr(a) => {
            out.push('[');
            for (i, e) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write(out, e, sorted);
            }
            out.push(']');
        }
        Value::Pt { x, y, p } => {
            // Source order is x, y, p; sorted order is p, x, y.
            let mut field = |out: &mut String, name: &str, n: f64, first: bool| {
                out.push_str(if first { "{\"" } else { ",\"" });
                out.push_str(name);
                out.push_str("\":");
                push_num(out, n);
            };
            if sorted && !p.is_nan() {
                field(out, "p", *p, true);
                field(out, "x", *x, false);
                field(out, "y", *y, false);
            } else {
                field(out, "x", *x, true);
                field(out, "y", *y, false);
                if !p.is_nan() {
                    field(out, "p", *p, false);
                }
            }
            out.push('}');
        }
        Value::Obj(_) => {
            let mut fields: Vec<(&str, &Value)> = v.entries().filter(|(_, v)| !matches!(v, Value::Undef)).collect();
            if sorted {
                fields.retain(|(k, _)| !matches!(*k, "__proto__" | "prototype" | "constructor"));
                fields.reverse(); // stable sort + dedup then keeps the last of a repeated key
                fields.sort_by(|a, b| cmp_utf16(a.0, b.0));
                fields.dedup_by(|b, a| a.0 == b.0);
            }
            out.push('{');
            for (i, (k, e)) in fields.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                push_str(out, k);
                out.push(':');
                write(out, e, sorted);
            }
            out.push('}');
        }
        Value::Bytes(_) | Value::Blob { .. } => out.push_str("{}"),
    }
}

/// `JSON.stringify(value)` (keys in source order).
pub fn stringify(v: &Value) -> String {
    let mut out = String::new();
    write(&mut out, v, false);
    out
}

/// 5.x `stableStringify` (timeline-core) / `ihnStableStringify`: keys sorted as JavaScript sorts strings.
pub fn stable(v: &Value) -> String {
    let mut out = String::new();
    write(&mut out, v, true);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_what_json_parse_parses() {
        let v = parse(br#" {"a":[1,-2.5e3,true,null,"x\n\u00e9\ud83d\ude00"],"p":{"x":1,"y":2},"q":{"x":1.5,"y":-0,"p":0.25},"a":7} "#).unwrap();
        assert_eq!(v.get("a"), &Value::Num(7.0));
        assert!(matches!(v.get("p"), Value::Pt { x, y, p } if *x == 1.0 && *y == 2.0 && p.is_nan()));
        assert_eq!(v.get("q"), &Value::Pt { x: 1.5, y: -0.0, p: 0.25 });
        assert_eq!(v.entries().count(), 4);
        assert_eq!(stable(&v).matches("\"a\":").count(), 1);
        let first = match &v {
            Value::Obj(f) => &f[0].1,
            _ => unreachable!(),
        };
        assert_eq!(first.arr()[4].as_str(), Some("x\né😀"));
        for bad in ["", "{", "[1,]", "{\"a\":1,}", "01", "1.", "+1", "\"\t\"", "nul", "[1] x", "{\"x\":1,\"y\":}", "\"\\x\"", ".5", "-"] {
            assert!(parse(bad.as_bytes()).is_none(), "{bad}");
        }
        // A point-like object in another key order or with spaces is an ordinary object.
        assert!(matches!(parse(br#"{"y":1,"x":2}"#).unwrap(), Value::Obj(_)));
        assert!(matches!(parse(br#"{"x": 1,"y":2}"#).unwrap(), Value::Obj(_)));
        assert!(parse(&[b'['; 300]).is_none());
    }


    #[test]
    fn writes_what_json_stringify_writes() {
        let text = r#"{"b":[1,0.1,1e+21,-0.5,"a\"\\\n\u0001é"],"a":{"z":null,"k":true},"pt":{"x":1.25,"y":2,"p":0.5},"pt2":{"x":0,"y":-3}}"#;
        let v = parse(text.as_bytes()).unwrap();
        assert_eq!(stringify(&v), text);
        assert_eq!(stable(&v), r#"{"a":{"k":true,"z":null},"b":[1,0.1,1e+21,-0.5,"a\"\\\n\u0001é"],"pt":{"p":0.5,"x":1.25,"y":2},"pt2":{"x":0,"y":-3}}"#);
        let odd = Value::Obj(vec![("u".into(), Value::Undef), ("n".into(), Value::Num(f64::NAN)), ("__proto__".into(), Value::Null), ("a".into(), Value::Arr(vec![Value::Undef]))]);
        assert_eq!(stringify(&odd), r#"{"n":null,"__proto__":null,"a":[null]}"#);
        assert_eq!(stable(&odd), r#"{"a":[null],"n":null}"#);
        assert_eq!(cmp_utf16("\u{ff5e}", "\u{1f600}"), Ordering::Greater); // UTF-16 order, not code point order
        assert_eq!(js_slice("a😀b", 2), "a");
        assert_eq!(js_slice("a😀b", 3), "a😀");
        assert_eq!((Value::Str(" 12 ".into()).num(), Value::Null.num(), Value::Str("".into()).num()), (12.0, 0.0, 0.0));
        assert!(Value::Str("1x".into()).num().is_nan() && Value::Undef.num().is_nan());
    }
}
