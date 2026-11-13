//! Structured values: the byte form of IndexedDB keys and values on both sides of the boundary (`Enc`/`val` in
//! `host/idb.js`). One tag byte, then:
//!
//! | Tag | Value | Payload |
//! |---|---|---|
//! | 0 | `undefined` | |
//! | 1 | `null` | |
//! | 2 / 3 | `false` / `true` | |
//! | 4 | number | f64 |
//! | 5 | string | u32 length + UTF-8 |
//! | 6 | `Uint8Array` | u32 length + bytes |
//! | 7 | array | u32 n + n values |
//! | 8 | plain object | u32 n + n × (string key, value) |
//! | 9 | `Blob` | u32 JS object handle, f64 size, string type |
//! | 10 | `Date` | f64 ms |
//! | 11 | other binary | u8 kind ([`TYPED`]) + u32 length + bytes |
//! | 12 | `Map` | u32 n + n × (key value, value) |
//! | 13 | `Set` | u32 n + n values |
//! | 14 | `BigInt` | decimal string |
//!
//! Rust writes tags 0..=9 only; 10..=14 occur in 5.x dumps and in keys read back. A `Blob` never crosses as bytes:
//! Rust holds its handle (release it with `OutBuf::obj_release`) and reads it through the blob effects.
//! 64-bit ids (actors, installs) are written as 16 lower-case hex digits, hashes as 64.

use ihn_core::codec::Reader;

pub const UNDEF: u8 = 0;
pub const NULL: u8 = 1;
pub const FALSE: u8 = 2;
pub const TRUE: u8 = 3;
pub const NUM: u8 = 4;
pub const STR: u8 = 5;
pub const BYTES: u8 = 6;
pub const ARR: u8 = 7;
pub const OBJ: u8 = 8;
pub const BLOB: u8 = 9;
pub const DATE: u8 = 10;
pub const TYPED_TAG: u8 = 11;
pub const MAP: u8 = 12;
pub const SET: u8 = 13;
pub const BIG: u8 = 14;

/// Kinds of tag 11, in the order of `VIEWS` in `host/idb.js` (the names are those of the W1.2 dump format).
#[allow(dead_code)]
pub const TYPED: [&str; 12] = ["ab", "i8", "u8c", "i16", "u16", "i32", "u32", "f32", "f64", "i64", "u64", "dv"];

/// Nesting limit of [`R::value`] and [`R::skip`].
const MAX_DEPTH: u32 = 96;

/// A decoded structured value.
#[derive(Clone, Debug, PartialEq, Default)]
pub enum Value {
    #[default]
    Undef,
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Bytes(Vec<u8>),
    Arr(Vec<Value>),
    /// In insertion order (the order `Object.keys` reports).
    Obj(Vec<(String, Value)>),
    Blob { obj: u32, size: f64, mime: String },
    Date(f64),
    Typed(u8, Vec<u8>),
    Map(Vec<(Value, Value)>),
    Set(Vec<Value>),
    Big(String),
}

static UNDEFINED: Value = Value::Undef;

impl Value {
    pub fn obj<const N: usize>(fields: [(&str, Value); N]) -> Value {
        Value::Obj(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }
    /// A field of an object; `Undef` when missing or not an object.
    pub fn get(&self, key: &str) -> &Value {
        match self {
            Value::Obj(fields) => fields.iter().find(|f| f.0 == key).map_or(&UNDEFINED, |f| &f.1),
            _ => &UNDEFINED,
        }
    }
    /// Sets (or with `Undef` removes) a field; turns a non-object into an object first.
    pub fn set(&mut self, key: &str, value: Value) {
        if !matches!(self, Value::Obj(_)) {
            *self = Value::Obj(Vec::new());
        }
        let Value::Obj(fields) = self else { return };
        match (fields.iter().position(|f| f.0 == key), value) {
            (Some(i), Value::Undef) => drop(fields.remove(i)),
            (Some(i), value) => fields[i].1 = value,
            (None, Value::Undef) => {}
            (None, value) => fields.push((key.to_string(), value)),
        }
    }
    pub fn is_undef(&self) -> bool { matches!(self, Value::Undef) }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Num(n) => Some(*n),
            _ => None,
        }
    }
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }
    /// The bytes of a `Uint8Array`, `ArrayBuffer` or any view.
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Value::Bytes(b) | Value::Typed(_, b) => Some(b),
            _ => None,
        }
    }
    pub fn items(&self) -> &[Value] {
        match self {
            Value::Arr(items) | Value::Set(items) => items,
            _ => &[],
        }
    }
    pub fn fields(&self) -> &[(String, Value)] {
        match self {
            Value::Obj(fields) => fields,
            _ => &[],
        }
    }
    /// Text of a string, `u32` of a non-negative integer number: the two forms ids take in records.
    pub fn text(&self) -> String { self.as_str().unwrap_or("").to_string() }
    pub fn u32(&self) -> u32 { self.as_f64().map_or(0, |n| n as u32) }
    pub fn flag(&self) -> bool { self.as_bool().unwrap_or(false) }
    pub fn blob(&self) -> Vec<u8> { self.as_bytes().unwrap_or(&[]).to_vec() }
    /// The JS object handles of every `Blob` inside (to release them).
    pub fn handles(&self, out: &mut Vec<u32>) {
        match self {
            Value::Blob { obj, .. } => out.push(*obj),
            Value::Arr(items) | Value::Set(items) => items.iter().for_each(|v| v.handles(out)),
            Value::Obj(fields) => fields.iter().for_each(|f| f.1.handles(out)),
            Value::Map(pairs) => pairs.iter().for_each(|p| {
                p.0.handles(out);
                p.1.handles(out);
            }),
            _ => {}
        }
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        W(&mut buf).value(self);
        buf
    }
    pub fn decode(bytes: &[u8]) -> Option<Value> { R::new(bytes).value() }
}

impl From<&str> for Value {
    fn from(s: &str) -> Value { Value::Str(s.to_string()) }
}
impl From<String> for Value {
    fn from(s: String) -> Value { Value::Str(s) }
}
impl From<f64> for Value {
    fn from(n: f64) -> Value { Value::Num(n) }
}
impl From<u32> for Value {
    fn from(n: u32) -> Value { Value::Num(n as f64) }
}
impl From<bool> for Value {
    fn from(b: bool) -> Value { Value::Bool(b) }
}
impl From<&[u8]> for Value {
    fn from(b: &[u8]) -> Value { Value::Bytes(b.to_vec()) }
}
impl From<Vec<Value>> for Value {
    fn from(items: Vec<Value>) -> Value { Value::Arr(items) }
}

/// 16 lower-case hex digits of a 64-bit id (keys and object keys sort like the number).
pub fn hex16(v: u64) -> String { hex(&v.to_be_bytes()) }
pub fn unhex16(s: &str) -> Option<u64> { if s.len() == 16 { u64::from_str_radix(s, 16).ok() } else { None } }
pub fn hex(bytes: &[u8]) -> String {
    let digit = |n: u8| (if n < 10 { b'0' + n } else { b'a' + n - 10 }) as char;
    bytes.iter().flat_map(|b| [digit(b >> 4), digit(b & 15)]).collect()
}
pub fn unhex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 || !s.is_ascii() {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok()).collect()
}

/// Writer of structured values over a byte vector.
pub struct W<'a>(pub &'a mut Vec<u8>);

impl W<'_> {
    fn u32(&mut self, v: u32) { self.0.extend_from_slice(&v.to_le_bytes()); }
    fn raw_str(&mut self, s: &str) {
        self.u32(s.len() as u32);
        self.0.extend_from_slice(s.as_bytes());
    }
    pub fn undef(&mut self) { self.0.push(UNDEF); }
    pub fn null(&mut self) { self.0.push(NULL); }
    pub fn bool(&mut self, v: bool) { self.0.push(if v { TRUE } else { FALSE }); }
    pub fn num(&mut self, v: f64) {
        self.0.push(NUM);
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub fn str(&mut self, s: &str) {
        self.0.push(STR);
        self.raw_str(s);
    }
    pub fn bytes(&mut self, b: &[u8]) {
        self.0.push(BYTES);
        self.u32(b.len() as u32);
        self.0.extend_from_slice(b);
    }
    /// Header of an array of `n` values, which follow.
    pub fn arr(&mut self, n: usize) {
        self.0.push(ARR);
        self.u32(n as u32);
    }
    /// Header of an object of `n` fields; each is [`W::key`] + one value.
    pub fn obj(&mut self, n: usize) {
        self.0.push(OBJ);
        self.u32(n as u32);
    }
    pub fn key(&mut self, k: &str) { self.raw_str(k); }
    /// A `Blob` the host holds under `obj`.
    pub fn blob(&mut self, obj: u32, size: f64, mime: &str) {
        self.0.push(BLOB);
        self.u32(obj);
        self.0.extend_from_slice(&size.to_le_bytes());
        self.raw_str(mime);
    }
    pub fn hex16(&mut self, v: u64) { self.str(&hex16(v)); }
    /// A path of a `patch`/`add` op: u32 n + n strings (not a value).
    pub fn path(&mut self, path: &[&str]) {
        self.u32(path.len() as u32);
        path.iter().for_each(|p| self.raw_str(p));
    }
    pub fn value(&mut self, v: &Value) {
        match v {
            Value::Undef => self.undef(),
            Value::Null => self.null(),
            Value::Bool(b) => self.bool(*b),
            Value::Num(n) => self.num(*n),
            Value::Str(s) => self.str(s),
            Value::Bytes(b) => self.bytes(b),
            Value::Arr(items) => {
                self.arr(items.len());
                items.iter().for_each(|i| self.value(i));
            }
            Value::Obj(fields) => {
                self.obj(fields.len());
                for (k, v) in fields {
                    self.key(k);
                    self.value(v);
                }
            }
            Value::Blob { obj, size, mime } => self.blob(*obj, *size, mime),
            Value::Date(ms) => {
                self.0.push(DATE);
                self.0.extend_from_slice(&ms.to_le_bytes());
            }
            Value::Typed(kind, b) => {
                self.0.extend_from_slice(&[TYPED_TAG, *kind]);
                self.u32(b.len() as u32);
                self.0.extend_from_slice(b);
            }
            Value::Map(pairs) => {
                self.0.push(MAP);
                self.u32(pairs.len() as u32);
                for (k, v) in pairs {
                    self.value(k);
                    self.value(v);
                }
            }
            Value::Set(items) => {
                self.0.push(SET);
                self.u32(items.len() as u32);
                items.iter().for_each(|i| self.value(i));
            }
            Value::Big(s) => {
                self.0.push(BIG);
                self.raw_str(s);
            }
        }
    }
}

/// Cursor over structured values. Typed getters consume one value; on a type mismatch they skip it and return
/// `None`, so the cursor stays aligned. Corrupt input ends the cursor (`None` from then on).
#[derive(Clone, Copy)]
pub struct R<'a>(pub Reader<'a>);

impl<'a> R<'a> {
    pub fn new(bytes: &'a [u8]) -> Self { R(Reader::new(bytes)) }
    pub fn is_empty(&self) -> bool { self.0.is_empty() }
    /// The tag of the next value.
    pub fn tag(&self) -> Option<u8> { self.0.buf.first().copied() }
    fn fail<T>(&mut self) -> Option<T> {
        self.0.buf = &[];
        None
    }
    fn expect(&mut self, tag: u8) -> bool {
        if self.tag() == Some(tag) {
            self.0.buf = &self.0.buf[1..];
            return true;
        }
        self.skip();
        false
    }
    pub fn num(&mut self) -> Option<f64> { if self.expect(NUM) { self.0.f64() } else { None } }
    pub fn str(&mut self) -> Option<&'a str> { if self.expect(STR) { self.0.str() } else { None } }
    pub fn bytes(&mut self) -> Option<&'a [u8]> { if self.expect(BYTES) { self.0.bytes() } else { None } }
    pub fn bool(&mut self) -> Option<bool> {
        match self.tag() {
            Some(t @ (TRUE | FALSE)) => {
                self.0.buf = &self.0.buf[1..];
                Some(t == TRUE)
            }
            _ => {
                self.skip();
                None
            }
        }
    }
    /// Enters an array: the number of values that follow.
    pub fn arr(&mut self) -> Option<u32> { if self.expect(ARR) { self.0.u32() } else { None } }
    /// Enters an object: the number of fields that follow, each [`R::key`] + one value.
    pub fn obj(&mut self) -> Option<u32> { if self.expect(OBJ) { self.0.u32() } else { None } }
    pub fn key(&mut self) -> Option<&'a str> { self.0.str() }
    /// Skips one value.
    pub fn skip(&mut self) -> Option<()> { self.walk(0, false).map(|_| ()) }
    /// Decodes one value into a tree.
    pub fn value(&mut self) -> Option<Value> { self.walk(0, true) }

    fn walk(&mut self, depth: u32, build: bool) -> Option<Value> {
        if depth > MAX_DEPTH {
            return self.fail();
        }
        let Some(tag) = self.0.u8() else { return self.fail() };
        let r = &mut self.0;
        let value = match tag {
            UNDEF => Some(Value::Undef),
            NULL => Some(Value::Null),
            FALSE => Some(Value::Bool(false)),
            TRUE => Some(Value::Bool(true)),
            NUM => r.f64().map(Value::Num),
            DATE => r.f64().map(Value::Date),
            STR => r.str().map(|s| if build { Value::Str(s.into()) } else { Value::Undef }),
            BIG => r.str().map(|s| if build { Value::Big(s.into()) } else { Value::Undef }),
            BYTES => r.bytes().map(|b| if build { Value::Bytes(b.into()) } else { Value::Undef }),
            TYPED_TAG => (|| {
                let kind = r.u8().filter(|k| (*k as usize) < TYPED.len())?;
                r.bytes().map(|b| if build { Value::Typed(kind, b.into()) } else { Value::Undef })
            })(),
            BLOB => (|| Some(Value::Blob { obj: r.u32()?, size: r.f64()?, mime: r.str()?.into() }))(),
            ARR | SET | MAP | OBJ => {
                let Some(n) = r.u32() else { return self.fail() };
                // Every value takes at least one byte: a count beyond the input is corrupt, not a reason to allocate.
                if n as usize > self.0.buf.len() {
                    return self.fail();
                }
                let cap = if build { n as usize } else { 0 };
                let (mut items, mut fields, mut pairs) = (Vec::with_capacity(cap), Vec::new(), Vec::new());
                for _ in 0..n {
                    match tag {
                        OBJ => {
                            let Some(key) = self.0.str() else { return self.fail() };
                            let value = self.walk(depth + 1, build)?;
                            if build {
                                fields.push((key.to_string(), value));
                            }
                        }
                        MAP => {
                            let pair = (self.walk(depth + 1, build)?, self.walk(depth + 1, build)?);
                            if build {
                                pairs.push(pair);
                            }
                        }
                        _ => {
                            let value = self.walk(depth + 1, build)?;
                            if build {
                                items.push(value);
                            }
                        }
                    }
                }
                Some(match tag {
                    ARR => Value::Arr(items),
                    SET => Value::Set(items),
                    MAP => Value::Map(pairs),
                    _ => Value::Obj(fields),
                })
            }
            _ => None,
        };
        if value.is_none() {
            return self.fail();
        }
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Value {
        Value::obj([
            ("s", "héllo".into()),
            ("n", Value::Num(-0.0)),
            ("b", Value::Bytes(vec![1, 2, 255])),
            ("a", vec![Value::Null, true.into(), Value::Undef, Value::Date(5.0)].into()),
            ("blob", Value::Blob { obj: 7, size: 12.0, mime: "application/pdf".into() }),
            ("t", Value::Typed(8, vec![0; 8])),
            ("m", Value::Map(vec![("k".into(), Value::Set(vec![1u32.into()]))])),
            ("big", Value::Big("123456789012345678901234567890".into())),
            ("$", Value::Num(f64::INFINITY)),
        ])
    }

    #[test]
    fn values_round_trip_and_skip() {
        let v = sample();
        let mut bytes = v.encode();
        W(&mut bytes).num(9.0);
        let mut r = R::new(&bytes);
        assert_eq!(r.value(), Some(v.clone()));
        assert_eq!(r.num(), Some(9.0));
        assert!(r.is_empty());
        let mut r = R::new(&bytes);
        assert_eq!(r.str(), None, "a mismatch skips the whole value");
        assert_eq!(r.num(), Some(9.0));
        let mut handles = Vec::new();
        v.handles(&mut handles);
        assert_eq!(handles, [7]);
        assert_eq!(v.get("s").as_str(), Some("héllo"));
        assert!(v.get("missing").is_undef() && v.get("s").get("x").is_undef());
    }

    #[test]
    fn corrupt_input_never_panics() {
        let bytes = sample().encode();
        for end in 0..bytes.len() {
            assert_eq!(R::new(&bytes[..end]).value(), None, "truncated at {end}");
        }
        // A count far beyond the input, an unknown tag, nesting beyond the limit.
        assert_eq!(R::new(&[ARR, 255, 255, 255, 255]).value(), None);
        assert_eq!(R::new(&[99]).value(), None);
        let deep: Vec<u8> = (0..200).flat_map(|_| [ARR, 1, 0, 0, 0]).chain([NULL]).collect();
        assert_eq!(R::new(&deep).value(), None);
        let mut rng = ihn_core::testutil::Rng::new(24);
        for _ in 0..2000 {
            let mut fuzzed = bytes.clone();
            for _ in 0..3 {
                let i = rng.next() as usize % fuzzed.len();
                fuzzed[i] = rng.next() as u8;
            }
            let mut r = R::new(&fuzzed);
            while !r.is_empty() && r.value().is_some() {}
        }
    }

    #[test]
    fn set_and_ids() {
        let mut v = Value::Null;
        v.set("a", 1u32.into());
        v.set("b", "x".into());
        v.set("a", Value::Undef);
        assert_eq!(v, Value::obj([("b", "x".into())]));
        assert_eq!(hex16(0x1ff), "00000000000001ff");
        assert_eq!(unhex16("00000000000001ff"), Some(0x1ff));
        assert_eq!(unhex16("1ff"), None);
        assert_eq!(unhex(&hex(&[0, 171, 255])), Some(vec![0, 171, 255]));
        assert_eq!(unhex("abc"), None);
    }
}
