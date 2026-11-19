//! The one byte codec of v6 (ARCHITECTURE §2.2): little-endian fixed-width numbers, LEB128, u32-length-prefixed
//! UTF-8 strings, the out-buffer record framing of §3.3 and base64. Readers never panic: truncated or corrupt input
//! yields `None`.

/// Append-only byte writer. Also the body of the ABI out-buffer (`[u8 family][u8 op][u32 len][payload]`).
#[derive(Default)]
pub struct Writer {
    pub buf: Vec<u8>,
}

impl Writer {
    pub fn u8(&mut self, v: u8) { self.buf.push(v); }
    pub fn u32(&mut self, v: u32) { self.buf.extend_from_slice(&v.to_le_bytes()); }
    pub fn i32(&mut self, v: i32) { self.buf.extend_from_slice(&v.to_le_bytes()); }
    pub fn u64(&mut self, v: u64) { self.buf.extend_from_slice(&v.to_le_bytes()); }
    pub fn f32(&mut self, v: f32) { self.buf.extend_from_slice(&v.to_le_bytes()); }
    pub fn f64(&mut self, v: f64) { self.buf.extend_from_slice(&v.to_le_bytes()); }
    pub fn raw(&mut self, v: &[u8]) { self.buf.extend_from_slice(v); }
    /// Unsigned LEB128.
    pub fn leb(&mut self, mut v: u64) {
        while v >= 0x80 {
            self.buf.push(v as u8 | 0x80);
            v >>= 7;
        }
        self.buf.push(v as u8);
    }
    /// u32 length + bytes.
    pub fn bytes(&mut self, v: &[u8]) {
        self.u32(v.len() as u32);
        self.raw(v);
    }
    pub fn str(&mut self, v: &str) { self.bytes(v.as_bytes()); }
    /// Nullable string: length `u32::MAX` means null.
    pub fn opt_str(&mut self, v: Option<&str>) {
        match v {
            Some(s) => self.str(s),
            None => self.u32(u32::MAX),
        }
    }
    /// Opens an out-buffer record; pass the result to [`Writer::end`].
    pub fn rec(&mut self, family: u8, op: u8) -> usize {
        self.buf.push(family);
        self.buf.push(op);
        self.u32(0);
        self.buf.len()
    }
    /// Closes the record opened at `mark` by patching its payload length.
    pub fn end(&mut self, mark: usize) {
        let len = (self.buf.len() - mark) as u32;
        self.buf[mark - 4..mark].copy_from_slice(&len.to_le_bytes());
    }
}

/// Cursor over borrowed bytes.
#[derive(Clone, Copy)]
pub struct Reader<'a> {
    pub buf: &'a [u8],
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Self { Reader { buf } }
    pub fn is_empty(&self) -> bool { self.buf.is_empty() }
    pub fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if n > self.buf.len() {
            return None;
        }
        let (head, tail) = self.buf.split_at(n);
        self.buf = tail;
        Some(head)
    }
    fn arr<const N: usize>(&mut self) -> Option<[u8; N]> { self.take(N)?.try_into().ok() }
    pub fn u8(&mut self) -> Option<u8> { Some(self.arr::<1>()?[0]) }
    pub fn u32(&mut self) -> Option<u32> { self.arr().map(u32::from_le_bytes) }
    pub fn i32(&mut self) -> Option<i32> { self.arr().map(i32::from_le_bytes) }
    pub fn u64(&mut self) -> Option<u64> { self.arr().map(u64::from_le_bytes) }
    pub fn f32(&mut self) -> Option<f32> { self.arr().map(f32::from_le_bytes) }
    pub fn f64(&mut self) -> Option<f64> { self.arr().map(f64::from_le_bytes) }
    /// Unsigned LEB128; rejects encodings longer than 10 bytes or overflowing 64 bits.
    pub fn leb(&mut self) -> Option<u64> {
        let mut v = 0u64;
        for shift in (0..70).step_by(7) {
            let b = self.u8()?;
            if shift == 63 && b > 1 {
                return None;
            }
            v |= ((b & 0x7f) as u64) << shift;
            if b < 0x80 {
                return Some(v);
            }
        }
        None
    }
    pub fn bytes(&mut self) -> Option<&'a [u8]> {
        let n = self.u32()? as usize;
        self.take(n)
    }
    pub fn str(&mut self) -> Option<&'a str> { std::str::from_utf8(self.bytes()?).ok() }
    pub fn opt_str(&mut self) -> Option<Option<&'a str>> {
        if self.buf.len() >= 4 && self.buf[..4] == [0xff; 4] {
            self.buf = &self.buf[4..];
            return Some(None);
        }
        self.str().map(Some)
    }
}

const STD: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const URL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// Base64. `url == false`: the `btoa` alphabet with `=` padding; `url == true`: base64url without padding (PKCE).
pub fn b64_encode(src: &[u8], url: bool) -> String {
    let abc = if url { URL } else { STD };
    let mut out = Vec::with_capacity(src.len().div_ceil(3) * 4);
    for c in src.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                out.push(abc[(n >> (18 - 6 * i)) as usize & 63]);
            } else if !url {
                out.push(b'=');
            }
        }
    }
    // Only alphabet bytes were pushed.
    String::from_utf8(out).unwrap_or_default()
}

/// Forgiving base64 decode with `atob` semantics (ASCII whitespace ignored, optional `=` padding, a remainder of one
/// character rejected), accepting both alphabets.
pub fn b64_decode(src: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(src.len() / 4 * 3 + 3);
    let (mut bits, mut have, mut data, mut pad) = (0u32, 0u32, 0usize, 0usize);
    for &b in src {
        let v = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => {
                pad += 1;
                continue;
            }
            b'\t' | b'\n' | 0x0c | b'\r' | b' ' => continue,
            _ => return None,
        };
        if pad > 0 {
            return None;
        }
        bits = bits << 6 | v as u32;
        have += 6;
        data += 1;
        if have >= 8 {
            have -= 8;
            out.push((bits >> have) as u8);
            bits &= (1 << have) - 1;
        }
    }
    let bad = if pad > 0 { pad > 2 || (data + pad) % 4 != 0 } else { data % 4 == 1 };
    (!bad).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::Rng;

    #[test]
    fn numbers_strings_and_records_round_trip() {
        let mut w = Writer::default();
        let m = w.rec(1, 9);
        w.u8(7);
        w.u32(0xdead_beef);
        w.i32(-5);
        w.u64(u64::MAX - 1);
        w.f32(1.5);
        w.f64(-0.0);
        w.str("añ€😀");
        w.opt_str(None);
        w.opt_str(Some(""));
        w.end(m);
        let mut r = Reader::new(&w.buf);
        assert_eq!((r.u8(), r.u8()), (Some(1), Some(9)));
        assert_eq!(r.u32(), Some(w.buf.len() as u32 - 6));
        assert_eq!(r.u8(), Some(7));
        assert_eq!(r.u32(), Some(0xdead_beef));
        assert_eq!(r.i32(), Some(-5));
        assert_eq!(r.u64(), Some(u64::MAX - 1));
        assert_eq!(r.f32(), Some(1.5));
        assert_eq!(r.f64().map(f64::to_bits), Some((-0.0f64).to_bits()));
        assert_eq!(r.str(), Some("añ€😀"));
        assert_eq!(r.opt_str(), Some(None));
        assert_eq!(r.opt_str(), Some(Some("")));
        assert!(r.is_empty());
        assert_eq!(r.u8(), None);
    }

    #[test]
    fn leb128_round_trips_and_rejects_bad_input() {
        let mut rng = Rng::new(1);
        let mut values = vec![0, 1, 127, 128, 16383, 16384, u32::MAX as u64, u64::MAX];
        values.extend((0..2000).map(|_| rng.next() >> (rng.below(64) as u32)));
        for v in values {
            let mut w = Writer::default();
            w.leb(v);
            assert_eq!(Reader::new(&w.buf).leb(), Some(v));
            assert_eq!(Reader::new(&w.buf[..w.buf.len() - 1]).leb(), None, "truncated {v}");
        }
        assert_eq!(Reader::new(&[0xff; 11]).leb(), None);
        assert_eq!(Reader::new(&[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x02]).leb(), None);
    }

    #[test]
    fn truncated_input_is_none_never_a_panic() {
        let mut w = Writer::default();
        w.str("hello");
        w.f64(3.0);
        for cut in 0..w.buf.len() {
            let mut r = Reader::new(&w.buf[..cut]);
            assert!(r.str().and_then(|_| r.f64()).is_none());
        }
        assert_eq!(Reader::new(&[2, 0, 0, 0, 0xff, 0xfe]).str(), None); // invalid UTF-8
    }

    #[test]
    fn base64_matches_btoa_and_atob() {
        assert_eq!(b64_encode(b"", false), "");
        assert_eq!(b64_encode(b"f", false), "Zg==");
        assert_eq!(b64_encode(b"fo", false), "Zm8=");
        assert_eq!(b64_encode(b"foo", false), "Zm9v");
        assert_eq!(b64_encode(&[0xfb, 0xff], false), "+/8=");
        assert_eq!(b64_encode(&[0xfb, 0xff], true), "-_8");
        assert_eq!(b64_decode(b" Zm9v\nYg= = "), Some(b"foob".to_vec()));
        assert_eq!(b64_decode(b"Zm9vYg"), Some(b"foob".to_vec()));
        assert_eq!(b64_decode(b"-_8"), Some(vec![0xfb, 0xff]));
        for bad in [&b"Z"[..], b"Zg=", b"Zg===", b"Z=g=", b"Zm9v!", b"Zm9vY"] {
            assert_eq!(b64_decode(bad), None, "{bad:?}");
        }
        let mut rng = Rng::new(7);
        for len in 0..200 {
            let data: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
            for url in [false, true] {
                assert_eq!(b64_decode(b64_encode(&data, url).as_bytes()), Some(data.clone()));
            }
        }
    }
}
