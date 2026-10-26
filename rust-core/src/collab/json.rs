//! JSON model with JavaScript semantics. Strings are UTF-16 code units so that
//! ordering, hashing and escaping match the original JS byte for byte, including
//! lone surrogates. Object entries are stored sorted by code unit (every
//! consumer sorts keys), duplicates keep the last value like JSON.parse.
use std::io::Write;

#[derive(Default)]
pub struct Pool { data: Vec<u16> }
#[derive(Clone, Copy)]
pub struct Sp { start: u32, len: u32 }
impl Pool {
    pub fn get(&self, s: Sp) -> &[u16] { &self.data[s.start as usize..(s.start + s.len) as usize] }
}

/// A number token that is already its own ECMAScript spelling (what JSON.stringify wrote):
/// at most 15 digits, no exponent, no trailing fraction zero, not -0, and not below 1e-6.
/// Such a token is the unique shortest form of its double, so it is copied through unchanged.
#[derive(Clone, Copy)]
pub struct Lit { bytes: [u8; 17], len: u8 }
impl Lit {
    pub fn as_bytes(&self) -> &[u8] { &self.bytes[..usize::from(self.len)] }
    pub fn value(&self) -> f64 { std::str::from_utf8(self.as_bytes()).ok().and_then(|t| t.parse().ok()).unwrap_or(f64::NAN) }
}

pub enum J { Null, Bool(bool), Num(f64), Lit(Lit), Str(Sp), Arr(Vec<J>), Obj(Vec<(Sp, J)>) }

const MAX_DEPTH: u32 = 1500;

struct Parser<'a> { s: &'a str, b: &'a [u8], i: usize, pool: &'a mut Pool, depth: u32 }

/// Parse JSON text (as produced by JSON.stringify) into the model.
pub fn parse(pool: &mut Pool, text: &[u8]) -> Result<J, String> {
    let s = std::str::from_utf8(text).map_err(|_| "Input is not UTF-8".to_string())?;
    if s.len() > u32::MAX as usize / 2 { return Err("Input too large".into()); }
    let mut p = Parser { s, b: text, i: 0, pool, depth: 0 };
    p.ws();
    let v = p.value()?;
    p.ws();
    if p.i != p.b.len() { return Err(format!("Unexpected data at {}", p.i)); }
    Ok(v)
}

impl<'a> Parser<'a> {
    fn ws(&mut self) { while self.i < self.b.len() && matches!(self.b[self.i], b' ' | b'\t' | b'\n' | b'\r') { self.i += 1; } }
    fn err<T>(&self, what: &str) -> Result<T, String> { Err(format!("Invalid JSON ({what}) at {}", self.i)) }
    fn lit(&mut self, word: &str, v: J) -> Result<J, String> {
        if self.b[self.i..].starts_with(word.as_bytes()) { self.i += word.len(); Ok(v) } else { self.err("literal") }
    }
    fn value(&mut self) -> Result<J, String> {
        if self.i >= self.b.len() { return self.err("end"); }
        match self.b[self.i] {
            b'n' => self.lit("null", J::Null),
            b't' => self.lit("true", J::Bool(true)),
            b'f' => self.lit("false", J::Bool(false)),
            b'"' => Ok(J::Str(self.string()?)),
            b'[' => {
                self.depth += 1;
                if self.depth > MAX_DEPTH { return self.err("depth"); }
                self.i += 1; self.ws();
                let mut items = Vec::new();
                if self.i < self.b.len() && self.b[self.i] == b']' { self.i += 1; self.depth -= 1; return Ok(J::Arr(items)); }
                loop {
                    self.ws();
                    items.push(self.value()?);
                    self.ws();
                    match self.b.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b']') => { self.i += 1; break; }
                        _ => return self.err("array"),
                    }
                }
                self.depth -= 1;
                Ok(J::Arr(items))
            }
            b'{' => {
                self.depth += 1;
                if self.depth > MAX_DEPTH { return self.err("depth"); }
                self.i += 1; self.ws();
                let mut items: Vec<(Sp, J)> = Vec::new();
                if self.i < self.b.len() && self.b[self.i] == b'}' { self.i += 1; self.depth -= 1; return Ok(J::Obj(items)); }
                loop {
                    self.ws();
                    if self.b.get(self.i) != Some(&b'"') { return self.err("key"); }
                    let key = self.string()?;
                    self.ws();
                    if self.b.get(self.i) != Some(&b':') { return self.err("colon"); }
                    self.i += 1; self.ws();
                    let v = self.value()?;
                    items.push((key, v));
                    self.ws();
                    match self.b.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b'}') => { self.i += 1; break; }
                        _ => return self.err("object"),
                    }
                }
                self.depth -= 1;
                finish_object(self.pool, &mut items);
                Ok(J::Obj(items))
            }
            b'-' | b'0'..=b'9' => self.number(),
            _ => self.err("value"),
        }
    }
    fn number(&mut self) -> Result<J, String> {
        let start = self.i;
        let negative = self.b[self.i] == b'-';
        if negative { self.i += 1; }
        let int_start = self.i;
        match self.b.get(self.i) {
            Some(b'0') => self.i += 1,
            Some(b'1'..=b'9') => { while matches!(self.b.get(self.i), Some(b'0'..=b'9')) { self.i += 1; } }
            _ => return self.err("number"),
        }
        let int_digits = self.i - int_start;
        let mut frac_digits = 0;
        let mut exponent = false;
        if self.b.get(self.i) == Some(&b'.') {
            self.i += 1;
            let frac_start = self.i;
            while matches!(self.b.get(self.i), Some(b'0'..=b'9')) { self.i += 1; }
            frac_digits = self.i - frac_start;
            if frac_digits == 0 { return self.err("fraction"); }
        }
        if matches!(self.b.get(self.i), Some(b'e' | b'E')) {
            exponent = true; self.i += 1;
            if matches!(self.b.get(self.i), Some(b'+' | b'-')) { self.i += 1; }
            if !matches!(self.b.get(self.i), Some(b'0'..=b'9')) { return self.err("exponent"); }
            while matches!(self.b.get(self.i), Some(b'0'..=b'9')) { self.i += 1; }
        }
        let token = &self.b[start..self.i];
        if !exponent && int_digits + frac_digits <= 15 {
            let int_is_zero = self.b[int_start] == b'0';
            let canonical = if frac_digits == 0 { !(negative && int_is_zero) } else {
                let frac = &token[token.len() - frac_digits..];
                frac[frac_digits - 1] != b'0' && (!int_is_zero || frac.iter().take_while(|&&c| c == b'0').count() <= 5)
            };
            if canonical {
                let mut lit = Lit { bytes: [0; 17], len: token.len() as u8 };
                lit.bytes[..token.len()].copy_from_slice(token);
                return Ok(J::Lit(lit));
            }
        }
        match self.s[start..self.i].parse::<f64>() { Ok(v) => Ok(J::Num(v)), Err(_) => self.err("number") }
    }
    fn string(&mut self) -> Result<Sp, String> {
        self.i += 1;
        let start = self.pool.data.len();
        let mut seg = self.i;
        loop {
            let Some(&c) = self.b.get(self.i) else { return self.err("string"); };
            match c {
                b'"' => {
                    self.pool.data.extend(self.s[seg..self.i].encode_utf16());
                    self.i += 1;
                    break;
                }
                b'\\' => {
                    self.pool.data.extend(self.s[seg..self.i].encode_utf16());
                    self.i += 1;
                    let Some(&e) = self.b.get(self.i) else { return self.err("escape"); };
                    self.i += 1;
                    let unit = match e {
                        b'"' => 0x22, b'\\' => 0x5c, b'/' => 0x2f, b'b' => 8, b'f' => 12, b'n' => 10, b'r' => 13, b't' => 9,
                        b'u' => {
                            if self.i + 4 > self.b.len() { return self.err("unicode"); }
                            let mut v: u16 = 0;
                            for k in 0..4 {
                                let d = match self.b[self.i + k] {
                                    h @ b'0'..=b'9' => h - b'0', h @ b'a'..=b'f' => h - b'a' + 10, h @ b'A'..=b'F' => h - b'A' + 10,
                                    _ => return self.err("unicode"),
                                };
                                v = v << 4 | u16::from(d);
                            }
                            self.i += 4;
                            v
                        }
                        _ => return self.err("escape"),
                    };
                    self.pool.data.push(unit);
                    seg = self.i;
                }
                0..=0x1f => return self.err("control"),
                _ => self.i += 1,
            }
        }
        Ok(Sp { start: start as u32, len: (self.pool.data.len() - start) as u32 })
    }
}

fn finish_object(pool: &Pool, items: &mut Vec<(Sp, J)>) {
    if items.len() < 2 { return; }
    items.sort_by(|a, b| pool.get(a.0).cmp(pool.get(b.0)));
    if items.windows(2).any(|w| pool.get(w[0].0) == pool.get(w[1].0)) {
        let mut out: Vec<(Sp, J)> = Vec::with_capacity(items.len());
        for item in items.drain(..) {
            if out.last().is_some_and(|last| pool.get(last.0) == pool.get(item.0)) { out.pop(); }
            out.push(item);
        }
        *items = out;
    }
}

// ---------------------------------------------------------------------------
// Output

pub trait Sink { fn put(&mut self, unit: u16); }
impl Sink for Vec<u16> { #[inline] fn put(&mut self, unit: u16) { self.push(unit); } }

#[inline] pub fn put_ascii<S: Sink>(s: &mut S, text: &str) { for b in text.bytes() { s.put(u16::from(b)); } }

fn put_hex4<S: Sink>(s: &mut S, v: u16) {
    for shift in [12, 8, 4, 0] { s.put(u16::from(b"0123456789abcdef"[usize::from((v >> shift) & 15)])); }
}

/// JSON.stringify string escaping (well-formed: lone surrogates become \udXXX).
pub fn write_str<S: Sink>(s: &mut S, units: &[u16]) {
    s.put(0x22);
    let mut i = 0;
    while i < units.len() {
        let c = units[i];
        match c {
            0x22 => put_ascii(s, "\\\""),
            0x5c => put_ascii(s, "\\\\"),
            8 => put_ascii(s, "\\b"), 9 => put_ascii(s, "\\t"), 10 => put_ascii(s, "\\n"),
            12 => put_ascii(s, "\\f"), 13 => put_ascii(s, "\\r"),
            0..=0x1f => { put_ascii(s, "\\u"); put_hex4(s, c); }
            0xd800..=0xdbff => {
                if i + 1 < units.len() && (0xdc00..=0xdfff).contains(&units[i + 1]) { s.put(c); s.put(units[i + 1]); i += 1; }
                else { put_ascii(s, "\\u"); put_hex4(s, c); }
            }
            0xdc00..=0xdfff => { put_ascii(s, "\\u"); put_hex4(s, c); }
            _ => s.put(c),
        }
        i += 1;
    }
    s.put(0x22);
}

/// Fixed-size ASCII buffer for one formatted number.
pub struct NumBuf { bytes: [u8; 40], len: usize }
impl NumBuf {
    fn new() -> Self { NumBuf { bytes: [0; 40], len: 0 } }
    #[inline] fn push(&mut self, b: u8) { self.bytes[self.len] = b; self.len += 1; }
    #[inline] fn extend(&mut self, bs: &[u8]) { self.bytes[self.len..self.len + bs.len()].copy_from_slice(bs); self.len += bs.len(); }
    pub fn as_bytes(&self) -> &[u8] { &self.bytes[..self.len] }
}

/// ECMAScript Number::toString for finite values, written as ASCII.
pub fn es_number_bytes(x: f64, out: &mut NumBuf) {
    if x == 0.0 { out.push(b'0'); return; }
    if x < 0.0 { out.push(b'-'); }
    let a = x.abs();
    if a.fract() == 0.0 && a < 1e15 {
        let mut n = a as u64;
        let mut tmp = [0u8; 20]; let mut at = 20;
        while n > 0 { at -= 1; tmp[at] = b'0' + (n % 10) as u8; n /= 10; }
        out.extend(&tmp[at..]);
        return;
    }
    let mut buf = [0u8; 48];
    let used = { let mut w = &mut buf[..]; let _ = write!(w, "{:e}", a); 48 - w.len() };
    let text = &buf[..used];
    let epos = text.iter().position(|&c| c == b'e').unwrap_or(used);
    let mut digits = [0u8; 24]; let mut k = 0usize;
    for &c in &text[..epos] { if c != b'.' { digits[k] = c; k += 1; } }
    let exp: i32 = std::str::from_utf8(&text[epos + 1..]).ok().and_then(|t| t.parse().ok()).unwrap_or(0);
    let mut tied = [0u8; 24];
    let digits = if k >= 15 && tie_candidate(a, k, exp) { match even_tie(a, &digits[..k], exp, &mut tied) { Some(n) => &tied[..n], None => &digits[..k] } } else { &digits[..k] };
    let k = digits.len();
    let kk = k as i32; let n = exp + 1;
    if kk <= n && n <= 21 {
        out.extend(digits);
        for _ in 0..(n - kk) { out.push(b'0'); }
    } else if 0 < n && n <= 21 {
        out.extend(&digits[..n as usize]); out.push(b'.'); out.extend(&digits[n as usize..]);
    } else if -6 < n && n <= 0 {
        out.extend(b"0.");
        for _ in 0..(-n) { out.push(b'0'); }
        out.extend(digits);
    } else {
        out.push(digits[0]);
        if k > 1 { out.push(b'.'); out.extend(&digits[1..]); }
        out.push(b'e'); out.push(if n - 1 < 0 { b'-' } else { b'+' });
        out.extend((n - 1).abs().to_string().as_bytes());
    }
}

/// A shortest-digit tie needs x * 2^(s+1) to be an integer (s = digits after the point); cheap pre-filter.
fn tie_candidate(x: f64, k: usize, exp: i32) -> bool {
    let bits = x.to_bits();
    let biased = ((bits >> 52) & 0x7ff) as i32;
    let (mantissa, e2) = if biased == 0 { (bits & ((1 << 52) - 1), -1074) } else { ((bits & ((1 << 52) - 1)) | (1 << 52), biased - 1075) };
    if mantissa == 0 { return false; }
    let fractional_bits = (-(e2 + mantissa.trailing_zeros() as i32)).max(0);
    fractional_bits <= k as i32 - exp
}

/// ECMAScript picks the even digit when the value sits exactly between two shortest candidates;
/// Rust's shortest formatter rounds such ties up. Returns the digit count written to `out`.
fn even_tie(x: f64, digits: &[u8], exp: i32, out: &mut [u8; 24]) -> Option<usize> {
    let k = digits.len();
    let mut buf = [0u8; 400];
    let used = { let mut w = &mut buf[..]; write!(w, "{:.*e}", k + 40, x).ok()?; 400 - w.len() };
    let text = &buf[..used];
    let epos = text.iter().position(|&c| c == b'e')?;
    let exact: Vec<u8> = text[..epos].iter().copied().filter(|&c| c != b'.').collect();
    if exact.len() < k + 2 || exact[k] != b'5' || exact[k + 1..].iter().any(|&c| c != b'0') { return None; }
    let mut candidate = [0u8; 24];
    candidate[..k].copy_from_slice(&exact[..k]);
    if (candidate[k - 1] - b'0') % 2 == 1 {
        if candidate[k - 1] == b'9' { return None; }
        candidate[k - 1] += 1;
    }
    let source = format!("{}e{}", std::str::from_utf8(&candidate[..k]).ok()?, exp - (k as i32 - 1));
    if source.parse::<f64>().ok()? != x { return None; }
    out[..k].copy_from_slice(&candidate[..k]);
    Some(k)
}

/// String(number) with the NaN / Infinity spellings.
pub fn es_number_string(x: f64) -> Vec<u8> {
    if x.is_nan() { return b"NaN".to_vec(); }
    if x.is_infinite() { return if x > 0.0 { b"Infinity".to_vec() } else { b"-Infinity".to_vec() }; }
    let mut out = NumBuf::new(); es_number_bytes(x, &mut out); out.as_bytes().to_vec()
}

/// JSON.stringify number: non-finite values become null.
pub fn write_number<S: Sink>(s: &mut S, x: f64) {
    if !x.is_finite() { put_ascii(s, "null"); return; }
    let mut out = NumBuf::new();
    es_number_bytes(x, &mut out);
    for &b in out.as_bytes() { s.put(u16::from(b)); }
}

#[inline] pub fn is_proto(key: &[u16]) -> bool {
    key.len() == 9 && key.iter().zip(b"__proto__").all(|(&a, &b)| a == u16::from(b))
}

/// Array-index key (canonical uint32 below 2^32-1), the ones a JS object lists first in numeric order.
pub fn index_key(key: &[u16]) -> Option<u32> {
    if key.is_empty() || key.len() > 10 || (key.len() > 1 && key[0] == 0x30) { return None; }
    let mut n: u64 = 0;
    for &c in key { if !(0x30..=0x39).contains(&c) { return None; } n = n * 10 + u64::from(c - 0x30); }
    if n < 4_294_967_295 { Some(n as u32) } else { None }
}

/// Positions in JS own-property order for keys that are already sorted by code unit.
pub fn output_order(keys: &[&[u16]]) -> Vec<usize> {
    let mut ints: Vec<(u32, usize)> = Vec::new();
    let mut rest: Vec<usize> = Vec::with_capacity(keys.len());
    for (i, k) in keys.iter().enumerate() { if let Some(n) = index_key(k) { ints.push((n, i)); } else { rest.push(i); } }
    if ints.is_empty() { return rest; }
    ints.sort_unstable();
    let mut order: Vec<usize> = ints.into_iter().map(|p| p.1).collect();
    order.extend(rest);
    order
}

/// Write `{...}` for keys sorted by code unit, honouring JS key order and the `__proto__` drop of ihnStableValue.
pub fn write_map<S: Sink>(s: &mut S, keys: &[&[u16]], mut value: impl FnMut(&mut S, usize)) {
    s.put(0x7b);
    let mut first = true;
    let any_index = keys.iter().any(|k| k.first().is_some_and(|c| (0x30..=0x39).contains(c)));
    let mut emit = |s: &mut S, i: usize| {
        if is_proto(keys[i]) { return; }
        if !first { s.put(0x2c); }
        first = false;
        write_str(s, keys[i]); s.put(0x3a); value(s, i);
    };
    if any_index { for i in output_order(keys) { emit(s, i); } } else { for i in 0..keys.len() { emit(s, i); } }
    s.put(0x7d);
}

/// ihnStableValue + JSON.stringify of a parsed value.
pub fn write_stable<S: Sink>(s: &mut S, pool: &Pool, v: &J) { write_stable_skip(s, pool, v, &[]) }

/// Same, dropping the listed top-level keys of an object (`{...item}` then `delete`).
pub fn write_stable_skip<S: Sink>(s: &mut S, pool: &Pool, v: &J, skip: &[&str]) {
    match v {
        J::Null => put_ascii(s, "null"),
        J::Bool(true) => put_ascii(s, "true"),
        J::Bool(false) => put_ascii(s, "false"),
        J::Num(n) => write_number(s, *n),
        J::Lit(lit) => for &b in lit.as_bytes() { s.put(u16::from(b)); },
        J::Str(sp) => write_str(s, pool.get(*sp)),
        J::Arr(items) => {
            if skip.is_empty() {
                s.put(0x5b);
                for (i, item) in items.iter().enumerate() { if i > 0 { s.put(0x2c); } write_stable(s, pool, item); }
                s.put(0x5d);
            } else {
                // An array spread into an object becomes index keys 0..n.
                s.put(0x7b);
                for (i, item) in items.iter().enumerate() {
                    if i > 0 { s.put(0x2c); }
                    put_ascii(s, &format!("\"{i}\":")); write_stable(s, pool, item);
                }
                s.put(0x7d);
            }
        }
        J::Obj(entries) => {
            if skip.is_empty() {
                if entries.iter().any(|e| pool.get(e.0).first().is_some_and(|c| (0x30..=0x39).contains(c))) {
                    let keys: Vec<&[u16]> = entries.iter().map(|e| pool.get(e.0)).collect();
                    write_map(s, &keys, |s, i| write_stable(s, pool, &entries[i].1));
                } else {
                    // Common case: keys are already in output order, no allocation.
                    s.put(0x7b);
                    let mut first = true;
                    for e in entries {
                        let key = pool.get(e.0);
                        if is_proto(key) { continue; }
                        if !first { s.put(0x2c); }
                        first = false;
                        write_str(s, key); s.put(0x3a); write_stable(s, pool, &e.1);
                    }
                    s.put(0x7d);
                }
            } else {
                let kept: Vec<&(Sp, J)> = entries.iter().filter(|e| {
                    let k = pool.get(e.0);
                    !skip.iter().any(|name| eq_ascii(k, name))
                }).collect();
                let keys: Vec<&[u16]> = kept.iter().map(|e| pool.get(e.0)).collect();
                write_map(s, &keys, |s, i| write_stable(s, pool, &kept[i].1));
            }
        }
    }
}

pub fn eq_ascii(units: &[u16], text: &str) -> bool {
    units.len() == text.len() && units.iter().zip(text.bytes()).all(|(&a, b)| a == u16::from(b))
}

// ---------------------------------------------------------------------------
// JavaScript coercions

impl J {
    /// Own property of a plain object (arrays and primitives have none that matter here).
    pub fn get<'a>(&'a self, pool: &Pool, key: &str) -> Option<&'a J> {
        let J::Obj(entries) = self else { return None; };
        let target: Vec<u16> = key.encode_utf16().collect();
        entries.binary_search_by(|e| pool.get(e.0).cmp(&target[..])).ok().map(|i| &entries[i].1)
    }
    pub fn truthy(&self, pool: &Pool) -> bool {
        match self { J::Null => false, J::Bool(b) => *b, J::Num(n) => *n != 0.0 && !n.is_nan(), J::Lit(lit) => lit.as_bytes() != b"0", J::Str(sp) => sp.len > 0 && !pool.get(*sp).is_empty(), _ => true }
    }
    pub fn is_object(&self) -> bool { matches!(self, J::Arr(_) | J::Obj(_)) }
}

pub fn truthy(pool: &Pool, v: Option<&J>) -> bool { v.is_some_and(|v| v.truthy(pool)) }

/// Object.keys(value) for objects and arrays, in code-unit sorted order, with the values.
pub fn sorted_pairs<'a>(pool: &Pool, v: &'a J) -> Vec<(Vec<u16>, &'a J)> {
    match v {
        J::Obj(entries) => entries.iter().map(|e| (pool.get(e.0).to_vec(), &e.1)).collect(),
        J::Arr(items) => {
            let mut pairs: Vec<(Vec<u16>, &J)> = items.iter().enumerate().map(|(i, x)| (i.to_string().encode_utf16().collect(), x)).collect();
            pairs.sort_by(|a, b| a.0.cmp(&b.0));
            pairs
        }
        _ => Vec::new(),
    }
}

fn is_js_space(c: u16) -> bool {
    matches!(c, 9..=13 | 0x20 | 0xa0 | 0x1680 | 0x2000..=0x200a | 0x2028 | 0x2029 | 0x202f | 0x205f | 0x3000 | 0xfeff)
}

/// Number(string)
pub fn str_to_number(units: &[u16]) -> f64 {
    let mut a = 0; let mut b = units.len();
    while a < b && is_js_space(units[a]) { a += 1; }
    while b > a && is_js_space(units[b - 1]) { b -= 1; }
    let t = &units[a..b];
    if t.is_empty() { return 0.0; }
    if t.iter().any(|&c| c > 0x7f) { return f64::NAN; }
    let text: String = t.iter().map(|&c| c as u8 as char).collect();
    let bytes = text.as_bytes();
    if bytes.len() > 2 && bytes[0] == b'0' && matches!(bytes[1], b'x' | b'X' | b'o' | b'O' | b'b' | b'B') {
        let radix = match bytes[1] { b'x' | b'X' => 16, b'o' | b'O' => 8, _ => 2 };
        let mut value = 0f64;
        for &c in &bytes[2..] {
            let Some(d) = (c as char).to_digit(radix) else { return f64::NAN; };
            value = value * f64::from(radix) + f64::from(d);
        }
        return value;
    }
    let (sign, body) = match bytes[0] { b'+' => (1.0, &text[1..]), b'-' => (-1.0, &text[1..]), _ => (1.0, &text[..]) };
    if body == "Infinity" { return sign * f64::INFINITY; }
    // StrDecimalLiteral: digits [. digits] [e[+-]digits] | . digits [exp]
    let bb = body.as_bytes(); let mut i = 0; let mut mantissa_digits = 0;
    while i < bb.len() && bb[i].is_ascii_digit() { i += 1; mantissa_digits += 1; }
    if i < bb.len() && bb[i] == b'.' { i += 1; while i < bb.len() && bb[i].is_ascii_digit() { i += 1; mantissa_digits += 1; } }
    if mantissa_digits == 0 { return f64::NAN; }
    if i < bb.len() && matches!(bb[i], b'e' | b'E') {
        i += 1;
        if i < bb.len() && matches!(bb[i], b'+' | b'-') { i += 1; }
        let start = i;
        while i < bb.len() && bb[i].is_ascii_digit() { i += 1; }
        if i == start { return f64::NAN; }
    }
    if i != bb.len() { return f64::NAN; }
    body.parse::<f64>().map(|v| sign * v).unwrap_or(f64::NAN)
}

/// String(value) for parsed JSON values.
pub fn to_js_string(pool: &Pool, v: &J) -> Vec<u16> {
    match v {
        J::Null => "null".encode_utf16().collect(),
        J::Bool(true) => "true".encode_utf16().collect(),
        J::Bool(false) => "false".encode_utf16().collect(),
        J::Num(n) => es_number_string(*n).into_iter().map(u16::from).collect(),
        J::Lit(lit) => lit.as_bytes().iter().map(|&b| u16::from(b)).collect(),
        J::Str(sp) => pool.get(*sp).to_vec(),
        J::Obj(_) => "[object Object]".encode_utf16().collect(),
        J::Arr(items) => {
            let mut out = Vec::new();
            for (i, item) in items.iter().enumerate() {
                if i > 0 { out.push(0x2c); }
                if !matches!(item, J::Null) { out.extend(to_js_string(pool, item)); }
            }
            out
        }
    }
}

/// Number(value); a missing property is undefined, hence NaN.
pub fn to_number(pool: &Pool, v: Option<&J>) -> f64 {
    match v {
        None => f64::NAN,
        Some(J::Null) => 0.0,
        Some(J::Bool(b)) => f64::from(u8::from(*b)),
        Some(J::Num(n)) => *n,
        Some(J::Lit(lit)) => lit.value(),
        Some(J::Str(sp)) => str_to_number(pool.get(*sp)),
        Some(J::Obj(_)) => f64::NAN,
        Some(j @ J::Arr(_)) => str_to_number(&to_js_string(pool, j)),
    }
}

/// String(value || '')
pub fn str_or_empty(pool: &Pool, v: Option<&J>) -> Vec<u16> {
    match v { Some(j) if j.truthy(pool) => to_js_string(pool, j), _ => Vec::new() }
}

/// Math.max(0, Math.floor(Number(value) || 0))
pub fn clamp_clock(pool: &Pool, v: Option<&J>) -> f64 {
    let n = to_number(pool, v);
    let n = if n.is_nan() || n == 0.0 { 0.0 } else { n.floor() };
    if n > 0.0 { n } else { 0.0 }
}

pub fn units_to_string(units: &[u16]) -> Result<String, String> {
    String::from_utf16(units).map_err(|_| "Unpaired surrogate in output".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn num(x: f64) -> String { String::from_utf8(es_number_string(x)).unwrap() }
    fn stable(text: &str) -> String {
        let mut pool = Pool::default();
        let v = parse(&mut pool, text.as_bytes()).unwrap();
        let mut out: Vec<u16> = Vec::new();
        write_stable(&mut out, &pool, &v);
        String::from_utf16(&out).unwrap()
    }
    #[test] fn number_formatting_matches_ecmascript() {
        assert_eq!(num(1e21), "1e+21"); assert_eq!(num(1e-7), "1e-7"); assert_eq!(num(0.000001), "0.000001");
        assert_eq!(num(123456789012345680000.0), "123456789012345680000"); assert_eq!(num(1.5e-7), "1.5e-7");
        assert_eq!(num(-0.0), "0"); assert_eq!(num(100.0), "100"); assert_eq!(num(5e-324), "5e-324");
        assert_eq!(num(1.7976931348623157e308), "1.7976931348623157e+308"); assert_eq!(num(0.1 + 0.2), "0.30000000000000004");
        assert_eq!(num(-12.5), "-12.5"); assert_eq!(num(1e15), "1000000000000000"); assert_eq!(num(2f64.powi(60)), "1152921504606847000");
        assert_eq!(num(-116376807667179.62), "-116376807667179.62"); assert_eq!(num(f64::NAN), "NaN"); assert_eq!(num(f64::NEG_INFINITY), "-Infinity"); assert_eq!(num(4.35), "4.35");
    }
    #[test] fn stable_sorts_keys_and_lists_index_keys_first() {
        assert_eq!(stable(r#"{"b":1,"10":2,"2":3,"a":[{"z":1,"y":null}]}"#), r#"{"2":3,"10":2,"a":[{"y":null,"z":1}],"b":1}"#);
        assert_eq!(stable(r#"{"b":1,"b":2,"__proto__":{"x":1}}"#), r#"{"b":2}"#);
        assert_eq!(stable(r#"{"4294967295":1,"4294967294":2,"01":3}"#), r#"{"4294967294":2,"01":3,"4294967295":1}"#);
        assert_eq!(stable("1e999"), "null"); assert_eq!(stable("-0"), "0");
    }
    #[test] fn strings_escape_like_json_stringify() {
        assert_eq!(stable(r#""a\u0001\"\\\n\ud800 \ud83d\ude00 \u2028""#), "\"a\\u0001\\\"\\\\\\n\\ud800 \u{1F600} \u{2028}\"");
        assert_eq!(stable(r#""\udc00x""#), "\"\\udc00x\"");
    }
    #[test] fn code_unit_order_not_code_point_order() {
        // U+FF5E sorts after the astral U+1F600 in UTF-16 order (surrogates 0xD83D < 0xFF5E).
        assert_eq!(stable("{\"\u{FF5E}\":1,\"\u{1F600}\":2}"), "{\"\u{1F600}\":2,\"\u{FF5E}\":1}");
    }
    #[test] fn coercions() {
        let u = |t: &str| t.encode_utf16().collect::<Vec<u16>>();
        assert_eq!(str_to_number(&u("  12 ")), 12.0); assert_eq!(str_to_number(&u("")), 0.0); assert_eq!(str_to_number(&u("0x10")), 16.0);
        assert!(str_to_number(&u("1_0")).is_nan()); assert!(str_to_number(&u("inf")).is_nan()); assert_eq!(str_to_number(&u("-Infinity")), f64::NEG_INFINITY);
        assert_eq!(str_to_number(&u(".5e1")), 5.0); assert!(str_to_number(&u("5e")).is_nan()); assert!(str_to_number(&u("+0x10")).is_nan());
    }
    #[test] fn rejects_bad_json() {
        let mut pool = Pool::default();
        for bad in ["", "{", "[1,]", "01", "1.", "\"\\x\"", "{\"a\" 1}", "nul", "[1] 2", "\"\u{1}\""] { assert!(parse(&mut pool, bad.as_bytes()).is_err(), "{bad:?}"); }
        assert!(parse(&mut pool, &[b'"', 0xff, b'"']).is_err());
        let deep = "[".repeat(2000); assert!(parse(&mut pool, deep.as_bytes()).is_err());
    }
}
