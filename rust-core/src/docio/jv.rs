//! Ordered JSON values with JavaScript semantics.
//!
//! The PDF export metadata is a JSON string whose bytes feed hashes and the
//! keyword payload, so key order, number formatting and string escaping must
//! match `JSON.parse` / `JSON.stringify` exactly. `serde_json` would need the
//! `preserve_order` feature (extra crate), so this module is a small ordered
//! reader/writer. Strings are kept as WTF-8 so lone UTF-16 surrogates survive.

use std::collections::HashMap;

pub type Bytes = Vec<u8>;

#[derive(Clone, Debug)]
pub enum V {
    Null,
    Bool(bool),
    Num(f64),
    Str(Bytes),
    Arr(Vec<V>),
    Obj(Vec<(Bytes, V)>),
}

const MAX_DEPTH: usize = 256;

impl V {
    /// `value.key` for plain objects; every other receiver yields `undefined`.
    pub fn get(&self, key: &[u8]) -> Option<&V> {
        match self {
            V::Obj(entries) => entries.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    pub fn truthy(&self) -> bool {
        match self {
            V::Null => false,
            V::Bool(b) => *b,
            V::Num(n) => *n != 0.0 && !n.is_nan(),
            V::Str(s) => !s.is_empty(),
            V::Arr(_) | V::Obj(_) => true,
        }
    }
    pub fn is_str(&self, expected: &str) -> bool { matches!(self, V::Str(s) if s == expected.as_bytes()) }
}

pub fn truthy(v: Option<&V>) -> bool { v.map_or(false, V::truthy) }

/// `Number.isFinite(value)`: true only for finite number-typed values.
pub fn finite_number(v: Option<&V>) -> Option<f64> {
    match v { Some(V::Num(n)) if n.is_finite() => Some(*n), _ => None }
}

// ---------------------------------------------------------------- numbers

/// `Math.round`: ties toward +infinity, `-0` kept, non-finite untouched.
pub fn js_round(x: f64) -> f64 {
    if !x.is_finite() { return x; }
    let f = x.floor();
    if x - f >= 0.5 { f + 1.0 } else { f }
}

/// `Number.prototype.toString()` (radix 10).
///
/// Rust prints the shortest round-trip digits, but when two shortest candidates are equally close
/// (an exact decimal tie) it takes the upper one while ECMAScript takes the even one. Those ties only
/// exist for values with a short binary fraction, so they are detected cheaply and settled with an
/// exactly rounded (ties-to-even) rendering of the same digit count.
pub fn fmt_num(x: f64, out: &mut Bytes) {
    if x.is_nan() { out.extend_from_slice(b"NaN"); return; }
    if x == 0.0 { out.push(b'0'); return; }
    if x.is_infinite() { out.extend_from_slice(if x < 0.0 { b"-Infinity" } else { b"Infinity" }); return; }
    let a = x.abs();
    if x.fract() == 0.0 && a < 1e15 {
        let mut value = x as i64;
        if value < 0 { out.push(b'-'); value = -value; }
        let mut digits = [0u8; 20];
        let mut n = 0;
        while value > 0 { digits[n] = b'0' + (value % 10) as u8; value /= 10; n += 1; }
        while n > 0 { n -= 1; out.push(digits[n]); }
        return;
    }
    let mut text = format!("{:e}", a);
    if (a.fract() * 1073741824.0).fract() == 0.0 || a.fract() == 0.0 {
        let digits = text.split('e').next().map_or(1, |m| m.bytes().filter(u8::is_ascii_digit).count());
        if digits > 1 {
            let exact = format!("{:.*e}", digits - 1, a);
            if exact != text && exact.parse::<f64>() == Ok(a) { text = exact; }
        }
    }
    if x < 0.0 { out.push(b'-'); }
    let (mantissa, exponent) = text.split_once('e').unwrap_or((&text, "0"));
    let exponent: i32 = exponent.parse().unwrap_or(0);
    let digits: Vec<u8> = mantissa.bytes().filter(u8::is_ascii_digit).collect();
    let k = digits.len() as i32;
    let n = exponent + 1; // decimal point position relative to the first digit
    if k <= n && n <= 21 {
        out.extend_from_slice(&digits);
        out.extend(std::iter::repeat(b'0').take((n - k) as usize));
    } else if 0 < n && n <= 21 {
        out.extend_from_slice(&digits[..n as usize]);
        out.push(b'.');
        out.extend_from_slice(&digits[n as usize..]);
    } else if -6 < n && n <= 0 {
        out.extend_from_slice(b"0.");
        out.extend(std::iter::repeat(b'0').take((-n) as usize));
        out.extend_from_slice(&digits);
    } else {
        out.push(digits[0]);
        if k > 1 { out.push(b'.'); out.extend_from_slice(&digits[1..]); }
        out.push(b'e');
        out.push(if n - 1 < 0 { b'-' } else { b'+' });
        out.extend_from_slice((n - 1).abs().to_string().as_bytes());
    }
}

/// Plain decimal text of `r / 100` for an integer-valued `r` with `|r| < 1e13`, written into `buf`.
/// This is what `Math.round(v * 100) / 100` prints: the shortest round-trip text of `r / 100.0` is the
/// plain decimal `r / 100`, because every decimal of at most 15 significant digits maps to a distinct double.
#[inline]
pub fn r2_bytes(r: f64, buf: &mut [u8; 24]) -> usize {
    let n = r as i64;
    if n == 0 { buf[0] = b'0'; return 1; }
    let magnitude = n.unsigned_abs();
    let (mut whole, fraction) = (magnitude / 100, (magnitude % 100) as u8);
    let mut digits = [0u8; 16];
    let mut count = 0;
    if whole == 0 { digits[0] = b'0'; count = 1; }
    while whole > 0 { digits[count] = b'0' + (whole % 10) as u8; whole /= 10; count += 1; }
    let mut at = 0;
    if n < 0 { buf[0] = b'-'; at = 1; }
    while count > 0 { count -= 1; buf[at] = digits[count]; at += 1; }
    if fraction != 0 {
        buf[at] = b'.'; buf[at + 1] = b'0' + fraction / 10; at += 2;
        if fraction % 10 != 0 { buf[at] = b'0' + fraction % 10; at += 1; }
    }
    at
}

/// `x = Math.round(v * 100) / 100` given the integer-valued `r = Math.round(v * 100)`.
pub fn fmt_r2(r: f64, out: &mut Bytes) {
    if !(r.abs() < 1e13) { return fmt_num(r / 100.0, out); }
    let mut buf = [0u8; 24];
    let n = r2_bytes(r, &mut buf);
    out.extend_from_slice(&buf[..n]);
}

pub fn num_string(x: f64) -> Bytes { let mut out = Vec::new(); fmt_num(x, &mut out); out }

fn is_js_space(c: char) -> bool {
    matches!(c, '\u{9}' | '\u{a}' | '\u{b}' | '\u{c}' | '\u{d}' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}'
        | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}')
}

/// `Number(string)`.
pub fn str_to_number(bytes: &[u8]) -> f64 {
    let Ok(text) = std::str::from_utf8(bytes) else { return f64::NAN };
    let text = text.trim_matches(is_js_space);
    if text.is_empty() { return 0.0; }
    let radix_digits = |digits: &str, radix: u32| -> f64 {
        if digits.is_empty() { return f64::NAN; }
        let mut value = 0.0f64;
        for c in digits.chars() {
            let Some(d) = c.to_digit(radix) else { return f64::NAN };
            value = value * radix as f64 + d as f64;
        }
        value
    };
    if let Some(rest) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) { return radix_digits(rest, 16); }
    if let Some(rest) = text.strip_prefix("0o").or_else(|| text.strip_prefix("0O")) { return radix_digits(rest, 8); }
    if let Some(rest) = text.strip_prefix("0b").or_else(|| text.strip_prefix("0B")) { return radix_digits(rest, 2); }
    let (negative, body) = match text.as_bytes()[0] { b'-' => (true, &text[1..]), b'+' => (false, &text[1..]), _ => (false, text) };
    if body == "Infinity" { return if negative { f64::NEG_INFINITY } else { f64::INFINITY }; }
    let b = body.as_bytes();
    let mut i = 0;
    let mut int_digits = 0;
    while i < b.len() && b[i].is_ascii_digit() { i += 1; int_digits += 1; }
    let mut frac_digits = 0;
    if i < b.len() && b[i] == b'.' { i += 1; while i < b.len() && b[i].is_ascii_digit() { i += 1; frac_digits += 1; } }
    if int_digits + frac_digits == 0 { return f64::NAN; }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        i += 1;
        if i < b.len() && (b[i] == b'+' || b[i] == b'-') { i += 1; }
        let start = i;
        while i < b.len() && b[i].is_ascii_digit() { i += 1; }
        if i == start { return f64::NAN; }
    }
    if i != b.len() { return f64::NAN; }
    match body.parse::<f64>() { Ok(v) => if negative { -v } else { v }, Err(_) => f64::NAN }
}

/// `Number(value)` for a JSON-derived value (`undefined` is `None`).
pub fn to_number(v: Option<&V>) -> f64 {
    match v {
        None => f64::NAN,
        Some(V::Null) => 0.0,
        Some(V::Bool(b)) => if *b { 1.0 } else { 0.0 },
        Some(V::Num(n)) => *n,
        Some(V::Str(s)) => str_to_number(s),
        Some(V::Obj(_)) => f64::NAN,
        Some(array @ V::Arr(_)) => str_to_number(&to_js_string(array)),
    }
}

/// `String(value)` for a JSON-derived value.
pub fn to_js_string(v: &V) -> Bytes {
    match v {
        V::Null => b"null".to_vec(),
        V::Bool(b) => if *b { b"true".to_vec() } else { b"false".to_vec() },
        V::Num(n) => num_string(*n),
        V::Str(s) => s.clone(),
        V::Obj(_) => b"[object Object]".to_vec(),
        V::Arr(items) => {
            let mut out = Vec::new();
            for (index, item) in items.iter().enumerate() {
                if index > 0 { out.push(b','); }
                if !matches!(item, V::Null) { out.extend_from_slice(&to_js_string(item)); }
            }
            out
        }
    }
}

// ---------------------------------------------------------------- strings (WTF-8)

/// Iterate UTF-16 code units of a WTF-8 byte string.
pub fn utf16_units(bytes: &[u8]) -> Utf16Units<'_> { Utf16Units { bytes, i: 0, low: 0 } }
pub struct Utf16Units<'a> { bytes: &'a [u8], i: usize, low: u16 }
impl Iterator for Utf16Units<'_> {
    type Item = u16;
    fn next(&mut self) -> Option<u16> {
        if self.low != 0 { let unit = self.low; self.low = 0; return Some(unit); }
        let b = self.bytes;
        let i = self.i;
        let first = *b.get(i)?;
        if first < 0x80 { self.i += 1; return Some(first as u16); }
        let tail = |k: usize| -> u32 { b.get(i + k).map_or(0, |x| (x & 0x3f) as u32) };
        let (code, len) = if first < 0xe0 { ((((first & 0x1f) as u32) << 6) | tail(1), 2) }
            else if first < 0xf0 { ((((first & 0x0f) as u32) << 12) | (tail(1) << 6) | tail(2), 3) }
            else { ((((first & 0x07) as u32) << 18) | (tail(1) << 12) | (tail(2) << 6) | tail(3), 4) };
        self.i += len;
        if code >= 0x10000 {
            let c = code - 0x10000;
            self.low = 0xdc00 | (c & 0x3ff) as u16;
            Some(0xd800 | (c >> 10) as u16)
        } else { Some(code as u16) }
    }
}

pub fn utf16_len(bytes: &[u8]) -> usize {
    if bytes.is_ascii() { bytes.len() } else { utf16_units(bytes).count() }
}

/// JavaScript string comparison (`<`): by UTF-16 code unit.
pub fn cmp_utf16(a: &[u8], b: &[u8]) -> std::cmp::Ordering {
    if a.is_ascii() && b.is_ascii() { return a.cmp(b); }
    utf16_units(a).cmp(utf16_units(b))
}

fn push_unit(out: &mut Bytes, unit: u16) {
    let u = unit as u32;
    if u < 0x80 { out.push(u as u8); }
    else if u < 0x800 { out.push(0xc0 | (u >> 6) as u8); out.push(0x80 | (u & 0x3f) as u8); }
    else { out.push(0xe0 | (u >> 12) as u8); out.push(0x80 | ((u >> 6) & 0x3f) as u8); out.push(0x80 | (u & 0x3f) as u8); }
}

// ---------------------------------------------------------------- parser

pub struct P<'a> { pub s: &'a [u8], pub i: usize, depth: usize }

impl<'a> P<'a> {
    pub fn new(s: &'a [u8]) -> Self { P { s, i: 0, depth: 0 } }
    pub fn err<T>(&self, message: &str) -> Result<T, String> { Err(format!("Invalid JSON at {}: {}", self.i, message)) }
    #[inline] pub fn ws(&mut self) { while matches!(self.s.get(self.i), Some(b' ' | b'\t' | b'\n' | b'\r')) { self.i += 1; } }
    #[inline] pub fn peek(&self) -> Option<u8> { self.s.get(self.i).copied() }
    pub fn expect(&mut self, c: u8) -> Result<(), String> {
        self.ws();
        if self.peek() == Some(c) { self.i += 1; Ok(()) } else { self.err(&format!("expected '{}'", c as char)) }
    }
    pub fn end(&mut self) -> Result<(), String> {
        self.ws();
        if self.i == self.s.len() { Ok(()) } else { self.err("trailing characters") }
    }

    pub fn document(s: &'a [u8]) -> Result<V, String> {
        let mut p = P::new(s);
        let v = p.value()?;
        p.end()?;
        Ok(v)
    }

    fn literal(&mut self, word: &[u8]) -> Result<(), String> {
        if self.s[self.i..].starts_with(word) { self.i += word.len(); Ok(()) } else { self.err("bad literal") }
    }

    pub fn number(&mut self) -> Result<f64, String> {
        let s = self.s;
        let start = self.i;
        let mut i = start;
        if s.get(i) == Some(&b'-') { i += 1; }
        match s.get(i) {
            Some(b'0') => i += 1,
            Some(b'1'..=b'9') => { while matches!(s.get(i), Some(b'0'..=b'9')) { i += 1; } }
            _ => return self.err("bad number"),
        }
        if s.get(i) == Some(&b'.') {
            i += 1;
            let digits = i;
            while matches!(s.get(i), Some(b'0'..=b'9')) { i += 1; }
            if i == digits { return self.err("bad fraction"); }
        }
        if matches!(s.get(i), Some(b'e' | b'E')) {
            i += 1;
            if matches!(s.get(i), Some(b'+' | b'-')) { i += 1; }
            let digits = i;
            while matches!(s.get(i), Some(b'0'..=b'9')) { i += 1; }
            if i == digits { return self.err("bad exponent"); }
        }
        let text = std::str::from_utf8(&s[start..i]).map_err(|_| "Invalid JSON number".to_string())?;
        let value = text.parse::<f64>().map_err(|_| "Invalid JSON number".to_string())?;
        self.i = i;
        Ok(value)
    }

    fn hex4(&mut self) -> Result<u16, String> {
        let mut value = 0u16;
        for _ in 0..4 {
            let Some(c) = self.peek() else { return self.err("bad unicode escape") };
            let d = match c { b'0'..=b'9' => c - b'0', b'a'..=b'f' => c - b'a' + 10, b'A'..=b'F' => c - b'A' + 10, _ => return self.err("bad unicode escape") };
            value = (value << 4) | d as u16;
            self.i += 1;
        }
        Ok(value)
    }

    /// Parse a string starting at the opening quote; `out` collects WTF-8 when given.
    pub fn string(&mut self, mut out: Option<&mut Bytes>) -> Result<(), String> {
        if self.peek() != Some(b'"') { return self.err("expected string"); }
        self.i += 1;
        let s = self.s;
        loop {
            let start = self.i;
            let mut i = start;
            while i < s.len() && s[i] != b'"' && s[i] != b'\\' && s[i] >= 0x20 { i += 1; }
            if let Some(o) = out.as_deref_mut() { o.extend_from_slice(&s[start..i]); }
            self.i = i;
            match s.get(i) {
                None => return self.err("unterminated string"),
                Some(b'"') => { self.i += 1; return Ok(()); }
                Some(b'\\') => {
                    self.i += 1;
                    let Some(&e) = s.get(self.i) else { return self.err("unterminated escape") };
                    self.i += 1;
                    let plain = match e {
                        b'"' => b'"', b'\\' => b'\\', b'/' => b'/', b'b' => 8, b'f' => 12, b'n' => b'\n', b'r' => b'\r', b't' => b'\t',
                        b'u' => {
                            let unit = self.hex4()?;
                            let mut pair = None;
                            if (0xd800..0xdc00).contains(&unit) && s[self.i..].starts_with(b"\\u") {
                                let save = self.i;
                                self.i += 2;
                                match self.hex4() {
                                    Ok(low) if (0xdc00..0xe000).contains(&low) => pair = Some(low),
                                    _ => self.i = save,
                                }
                            }
                            if let Some(o) = out.as_deref_mut() {
                                if let Some(low) = pair {
                                    let code = 0x10000 + (((unit as u32) - 0xd800) << 10) + ((low as u32) - 0xdc00);
                                    o.push(0xf0 | (code >> 18) as u8);
                                    o.push(0x80 | ((code >> 12) & 0x3f) as u8);
                                    o.push(0x80 | ((code >> 6) & 0x3f) as u8);
                                    o.push(0x80 | (code & 0x3f) as u8);
                                } else { push_unit(o, unit); }
                            }
                            continue;
                        }
                        _ => return self.err("bad escape"),
                    };
                    if let Some(o) = out.as_deref_mut() { o.push(plain); }
                }
                _ => return self.err("control character in string"),
            }
        }
    }

    pub fn skip_value(&mut self) -> Result<(), String> {
        self.ws();
        match self.peek() {
            Some(b'{') | Some(b'[') => {
                let open = self.s[self.i];
                if self.depth >= MAX_DEPTH { return self.err("nesting too deep"); }
                self.depth += 1;
                self.i += 1;
                self.ws();
                let close = if open == b'{' { b'}' } else { b']' };
                if self.peek() == Some(close) { self.i += 1; self.depth -= 1; return Ok(()); }
                loop {
                    if open == b'{' { self.ws(); self.string(None)?; self.expect(b':')?; }
                    self.skip_value()?;
                    self.ws();
                    match self.peek() {
                        Some(b',') => self.i += 1,
                        Some(c) if c == close => { self.i += 1; break; }
                        _ => return self.err("expected ',' or closing bracket"),
                    }
                }
                self.depth -= 1;
                Ok(())
            }
            Some(b'"') => self.string(None),
            Some(b't') => self.literal(b"true"),
            Some(b'f') => self.literal(b"false"),
            Some(b'n') => self.literal(b"null"),
            Some(_) => self.number().map(|_| ()),
            None => self.err("unexpected end"),
        }
    }

    pub fn value(&mut self) -> Result<V, String> {
        self.ws();
        match self.peek() {
            Some(b'{') => {
                if self.depth >= MAX_DEPTH { return self.err("nesting too deep"); }
                self.depth += 1;
                self.i += 1;
                let mut entries: Vec<(Bytes, V)> = Vec::new();
                let mut index: Option<HashMap<Bytes, usize>> = None;
                self.ws();
                if self.peek() == Some(b'}') { self.i += 1; self.depth -= 1; return Ok(V::Obj(entries)); }
                loop {
                    self.ws();
                    let mut key = Vec::new();
                    self.string(Some(&mut key))?;
                    self.expect(b':')?;
                    let value = self.value()?;
                    let existing = if entries.len() > 24 {
                        let map = index.get_or_insert_with(|| entries.iter().enumerate().map(|(n, (k, _))| (k.clone(), n)).collect());
                        map.get(&key).copied()
                    } else { entries.iter().position(|(k, _)| *k == key) };
                    match existing {
                        Some(n) => entries[n].1 = value,
                        None => {
                            if let Some(map) = index.as_mut() { map.insert(key.clone(), entries.len()); }
                            entries.push((key, value));
                        }
                    }
                    self.ws();
                    match self.peek() {
                        Some(b',') => self.i += 1,
                        Some(b'}') => { self.i += 1; break; }
                        _ => return self.err("expected ',' or '}'"),
                    }
                }
                self.depth -= 1;
                js_property_order(&mut entries);
                Ok(V::Obj(entries))
            }
            Some(b'[') => {
                if self.depth >= MAX_DEPTH { return self.err("nesting too deep"); }
                self.depth += 1;
                self.i += 1;
                let mut items = Vec::new();
                self.ws();
                if self.peek() == Some(b']') { self.i += 1; self.depth -= 1; return Ok(V::Arr(items)); }
                loop {
                    items.push(self.value()?);
                    self.ws();
                    match self.peek() {
                        Some(b',') => self.i += 1,
                        Some(b']') => { self.i += 1; break; }
                        _ => return self.err("expected ',' or ']'"),
                    }
                }
                self.depth -= 1;
                Ok(V::Arr(items))
            }
            Some(b'"') => { let mut s = Vec::new(); self.string(Some(&mut s))?; Ok(V::Str(s)) }
            Some(b't') => { self.literal(b"true")?; Ok(V::Bool(true)) }
            Some(b'f') => { self.literal(b"false")?; Ok(V::Bool(false)) }
            Some(b'n') => { self.literal(b"null")?; Ok(V::Null) }
            Some(_) => Ok(V::Num(self.number()?)),
            None => self.err("unexpected end"),
        }
    }
}

/// Canonical array index (`"0"`, `"17"`; not `"01"`, `"-1"`, not above 2^32-2).
fn array_index(key: &[u8]) -> Option<u32> {
    if key.is_empty() || key.len() > 10 || !key.iter().all(u8::is_ascii_digit) || (key[0] == b'0' && key.len() > 1) { return None; }
    let value: u64 = std::str::from_utf8(key).ok()?.parse().ok()?;
    if value < 4294967295 { Some(value as u32) } else { None }
}

/// JavaScript objects enumerate integer-like keys first, ascending, then the others in insertion order.
pub fn js_property_order<T>(entries: &mut Vec<(Bytes, T)>) {
    if !entries.iter().any(|(k, _)| k.first().is_some_and(u8::is_ascii_digit)) { return; }
    if !entries.iter().any(|(k, _)| array_index(k).is_some()) { return; }
    let mut indexed: Vec<(u32, (Bytes, T))> = Vec::new();
    let mut rest = Vec::new();
    for entry in entries.drain(..) {
        match array_index(&entry.0) { Some(n) => indexed.push((n, entry)), None => rest.push(entry) }
    }
    indexed.sort_by_key(|(n, _)| *n);
    entries.extend(indexed.into_iter().map(|(_, e)| e));
    entries.extend(rest);
}

// ---------------------------------------------------------------- writer

pub fn write_str(s: &[u8], out: &mut Bytes) {
    out.push(b'"');
    let mut i = 0;
    let mut run = 0;
    while i < s.len() {
        let b = s[i];
        let escape: Option<&[u8]> = match b {
            b'"' => Some(b"\\\""), b'\\' => Some(b"\\\\"), 8 => Some(b"\\b"), 12 => Some(b"\\f"),
            b'\n' => Some(b"\\n"), b'\r' => Some(b"\\r"), b'\t' => Some(b"\\t"), _ => None,
        };
        if let Some(e) = escape { out.extend_from_slice(&s[run..i]); out.extend_from_slice(e); i += 1; run = i; }
        else if b < 0x20 {
            out.extend_from_slice(&s[run..i]);
            out.extend_from_slice(format!("\\u{:04x}", b).as_bytes());
            i += 1; run = i;
        } else if b == 0xed && s.get(i + 1).is_some_and(|n| *n >= 0xa0) && i + 2 < s.len() {
            out.extend_from_slice(&s[run..i]);
            let unit = (((b & 0x0f) as u32) << 12) | (((s[i + 1] & 0x3f) as u32) << 6) | (s[i + 2] & 0x3f) as u32;
            out.extend_from_slice(format!("\\u{:04x}", unit).as_bytes());
            i += 3; run = i;
        } else { i += 1; }
    }
    out.extend_from_slice(&s[run..]);
    out.push(b'"');
}

/// `JSON.stringify` of a number (non-finite becomes `null`).
pub fn write_num(x: f64, out: &mut Bytes) { if x.is_finite() { fmt_num(x, out) } else { out.extend_from_slice(b"null") } }

pub fn write(v: &V, out: &mut Bytes) {
    match v {
        V::Null => out.extend_from_slice(b"null"),
        V::Bool(b) => out.extend_from_slice(if *b { b"true" } else { b"false" }),
        V::Num(n) => write_num(*n, out),
        V::Str(s) => write_str(s, out),
        V::Arr(items) => {
            out.push(b'[');
            for (n, item) in items.iter().enumerate() { if n > 0 { out.push(b','); } write(item, out); }
            out.push(b']');
        }
        V::Obj(entries) => {
            out.push(b'{');
            for (n, (k, item)) in entries.iter().enumerate() {
                if n > 0 { out.push(b','); }
                write_str(k, out);
                out.push(b':');
                write(item, out);
            }
            out.push(b'}');
        }
    }
}

#[cfg(test)]
pub fn to_json(v: &V) -> Bytes { let mut out = Vec::new(); write(v, &mut out); out }

#[cfg(test)]
mod tests {
    use super::*;
    fn num(x: f64) -> String { String::from_utf8(num_string(x)).unwrap() }
    #[test] fn number_formatting_matches_javascript() {
        assert_eq!(num(0.0), "0"); assert_eq!(num(-0.0), "0"); assert_eq!(num(1.5), "1.5"); assert_eq!(num(-12.25), "-12.25");
        assert_eq!(num(1e21), "1e+21"); assert_eq!(num(1.5e300), "1.5e+300"); assert_eq!(num(123456789012345680000.0), "123456789012345680000");
        assert_eq!(num(1e-7), "1e-7"); assert_eq!(num(1.5e-7), "1.5e-7"); assert_eq!(num(0.000001), "0.000001");
        assert_eq!(num(0.1 + 0.2), "0.30000000000000004"); assert_eq!(num(f64::NAN), "NaN"); assert_eq!(num(-f64::INFINITY), "-Infinity");
        assert_eq!(num(4294967296.0), "4294967296"); assert_eq!(num(5e-324), "5e-324");
        assert_eq!(num(1e20), "100000000000000000000"); assert_eq!(num(-1.5e-6), "-0.0000015"); assert_eq!(num(123.456), "123.456");
        // Exact decimal tie: ECMAScript prints the even candidate (verified in Node), Rust alone prints ...313.
        assert_eq!(num(88.245391845703125), "88.24539184570312");
        assert_eq!(num(204.11453247070312), "204.11453247070312");
    }
    #[test] fn math_round_ties_up() {
        assert_eq!(js_round(0.5), 1.0); assert_eq!(js_round(-0.5), 0.0); assert_eq!(js_round(2.5), 3.0); assert_eq!(js_round(-2.5), -2.0);
        assert_eq!(js_round(0.49999999999999994), 0.0); assert!(js_round(f64::NAN).is_nan());
        assert!(js_round(-0.0).is_sign_negative());
    }
    #[test] fn string_to_number() {
        assert_eq!(str_to_number(b"  12 "), 12.0); assert_eq!(str_to_number(b""), 0.0); assert!(str_to_number(b"abc").is_nan());
        assert_eq!(str_to_number(b"0x10"), 16.0); assert_eq!(str_to_number(b"1e3"), 1000.0); assert_eq!(str_to_number(b".5"), 0.5);
        assert_eq!(str_to_number(b"5."), 5.0); assert!(str_to_number(b".").is_nan()); assert!(str_to_number(b"1_0").is_nan());
        assert!(str_to_number(b"inf").is_nan()); assert!(str_to_number(b"NaN").is_nan()); assert_eq!(str_to_number(b"-Infinity"), f64::NEG_INFINITY);
        assert!(str_to_number(b"-0x10").is_nan());
    }
    #[test] fn parse_and_write_round_trip_with_surrogates() {
        let text = br#"{"a":[1,2.5,"x\ud83d\ude00y","\ud800","\u0007"],"1":true,"b":null}"#;
        let v = P::document(text).unwrap();
        assert_eq!(String::from_utf8(to_json(&v)).unwrap(), r#"{"1":true,"a":[1,2.5,"x😀y","\ud800","\u0007"],"b":null}"#);
    }
    #[test] fn integer_keys_come_first() {
        let v = P::document(br#"{"b":1,"10":2,"9":3,"a":4,"01":5}"#).unwrap();
        assert_eq!(String::from_utf8(to_json(&v)).unwrap(), r#"{"9":3,"10":2,"b":1,"a":4,"01":5}"#);
    }
    #[test] fn rejects_invalid_json() {
        for bad in ["", "{", "[1,]", "{\"a\":01}", "\"\u{1}\"", "nul", "[1] x", "{\"a\" 1}", "1.", "\"\\x\""] {
            assert!(P::document(bad.as_bytes()).is_err(), "{bad:?}");
        }
        assert!(P::document("[".repeat(300).as_bytes()).is_err());
    }
    #[test] fn utf16_helpers() {
        let v = P::document(br#""a\ud83d\ude00\ud800""#).unwrap();
        let V::Str(s) = v else { panic!() };
        assert_eq!(utf16_len(&s), 4);
        assert_eq!(utf16_units(&s).collect::<Vec<_>>(), vec![0x61, 0xd83d, 0xde00, 0xd800]);
    }
    #[test] fn duplicate_keys_keep_first_position_last_value() {
        let v = P::document(br#"{"a":1,"b":2,"a":3}"#).unwrap();
        assert_eq!(String::from_utf8(to_json(&v)).unwrap(), r#"{"a":3,"b":2}"#);
    }
}
