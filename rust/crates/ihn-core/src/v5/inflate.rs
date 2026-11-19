//! zlib (RFC 1950) inflate with an output limit: what `DecompressionStream('deflate')` accepts, for the
//! `STROKES_Z` and `IH_VERSIONS` sections. Pure, so the importer is one deterministic function; a host that has
//! already inflated a section passes the text instead (`import::Sections`).

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum InflateError {
    /// Not a zlib stream, damaged data, truncated input or a wrong Adler-32.
    Corrupt,
    /// The output would exceed the limit.
    TooLarge,
}

const FAST: u32 = 10;

struct Huff {
    /// `len << 9 | symbol` for every `FAST`-bit prefix (0 = longer code).
    fast: Vec<u16>,
    count: [u16; 16],
    symbol: Vec<u16>,
}

impl Huff {
    fn new(lengths: &[u8]) -> Option<Huff> {
        let mut count = [0u16; 16];
        for &l in lengths {
            count[l as usize] += 1;
        }
        count[0] = 0;
        // Over-subscribed sets are invalid; incomplete ones are allowed (a single distance code).
        let mut left = 1i32;
        for &c in &count[1..] {
            left = (left << 1) - c as i32;
            if left < 0 {
                return None;
            }
        }
        let mut offs = [0u16; 16];
        for l in 1..15 {
            offs[l + 1] = offs[l] + count[l];
        }
        let mut symbol = vec![0u16; lengths.len()];
        let mut next = [0u32; 16];
        let mut code = 0u32;
        for l in 1..16 {
            code = (code + count[l - 1] as u32) << 1;
            next[l] = code;
        }
        let mut fast = vec![0u16; 1 << FAST];
        for (sym, &l) in lengths.iter().enumerate() {
            if l == 0 {
                continue;
            }
            symbol[offs[l as usize] as usize] = sym as u16;
            offs[l as usize] += 1;
            let c = next[l as usize];
            next[l as usize] += 1;
            if l as u32 <= FAST {
                let rev = c.reverse_bits() >> (32 - l as u32);
                let mut i = rev as usize;
                while i < 1 << FAST {
                    fast[i] = (l as u16) << 9 | sym as u16;
                    i += 1 << l;
                }
            }
        }
        Some(Huff { fast, count, symbol })
    }
}

struct Bits<'a> {
    d: &'a [u8],
    pos: usize,
    buf: u64,
    cnt: u32,
    /// Zero bits appended after the end of the input (reading one of them is an error).
    pad: u32,
}

impl Bits<'_> {
    fn fill(&mut self) {
        while self.cnt <= 56 {
            let b = match self.d.get(self.pos) {
                Some(&b) => b,
                None => {
                    self.pad += 8;
                    0
                }
            };
            self.pos += 1;
            self.buf |= (b as u64) << self.cnt;
            self.cnt += 8;
        }
    }
    fn drop(&mut self, n: u32) -> Result<(), InflateError> {
        self.buf >>= n;
        self.cnt -= n;
        if self.cnt < self.pad {
            Err(InflateError::Corrupt)
        } else {
            Ok(())
        }
    }
    fn bits(&mut self, n: u32) -> Result<u32, InflateError> {
        if self.cnt < n {
            self.fill();
        }
        let v = (self.buf & ((1u64 << n) - 1)) as u32;
        self.drop(n)?;
        Ok(v)
    }
    fn sym(&mut self, h: &Huff) -> Result<u32, InflateError> {
        if self.cnt < 15 {
            self.fill();
        }
        let e = h.fast[(self.buf & ((1 << FAST) - 1)) as usize];
        if e != 0 {
            self.drop((e >> 9) as u32)?;
            return Ok((e & 0x1ff) as u32);
        }
        let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
        for len in 1..16 {
            code |= ((self.buf >> (len - 1)) & 1) as i32;
            let count = h.count[len] as i32;
            if code - count < first {
                self.drop(len as u32)?;
                return Ok(h.symbol[(index + code - first) as usize] as u32);
            }
            index += count;
            first = (first + count) << 1;
            code <<= 1;
        }
        Err(InflateError::Corrupt)
    }
}

const LEN_BASE: [u16; 29] = [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258];
const LEN_EXTRA: [u8; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
const DIST_BASE: [u16; 30] = [1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577];
const DIST_EXTRA: [u8; 30] = [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];
const ORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];

fn block(b: &mut Bits, out: &mut Vec<u8>, lit: &Huff, dist: &Huff, limit: usize) -> Result<(), InflateError> {
    loop {
        let s = b.sym(lit)?;
        if s < 256 {
            if out.len() >= limit {
                return Err(InflateError::TooLarge);
            }
            out.push(s as u8);
        } else if s == 256 {
            return Ok(());
        } else {
            let s = s as usize - 257;
            if s >= 29 {
                return Err(InflateError::Corrupt);
            }
            let len = LEN_BASE[s] as usize + b.bits(LEN_EXTRA[s] as u32)? as usize;
            let d = b.sym(dist)? as usize;
            if d >= 30 {
                return Err(InflateError::Corrupt);
            }
            let d = DIST_BASE[d] as usize + b.bits(DIST_EXTRA[d] as u32)? as usize;
            if d > out.len() {
                return Err(InflateError::Corrupt);
            }
            if out.len() + len > limit {
                return Err(InflateError::TooLarge);
            }
            let from = out.len() - d;
            if d >= len {
                out.extend_from_within(from..from + len);
            } else {
                for i in 0..len {
                    out.push(out[from + i]);
                }
            }
        }
    }
}

/// Raw DEFLATE (RFC 1951). Returns the output and the number of input bytes used.
pub fn inflate_raw(data: &[u8], limit: usize) -> Result<(Vec<u8>, usize), InflateError> {
    let mut b = Bits { d: data, pos: 0, buf: 0, cnt: 0, pad: 0 };
    let mut out = Vec::with_capacity((data.len() * 4).min(limit));
    let mut fixed: Option<(Huff, Huff)> = None;
    loop {
        let last = b.bits(1)?;
        match b.bits(2)? {
            0 => {
                b.drop(b.cnt % 8)?;
                let len = b.bits(16)? as usize;
                if b.bits(16)? as usize != len ^ 0xffff {
                    return Err(InflateError::Corrupt);
                }
                if out.len() + len > limit {
                    return Err(InflateError::TooLarge);
                }
                for _ in 0..len {
                    out.push(b.bits(8)? as u8);
                }
            }
            1 => {
                let (lit, dist) = fixed.get_or_insert_with(|| {
                    let mut l = [8u8; 288];
                    l[144..256].fill(9);
                    l[256..280].fill(7);
                    // Both sets are valid by construction.
                    (Huff::new(&l).unwrap_or_else(|| unreachable!()), Huff::new(&[5u8; 30]).unwrap_or_else(|| unreachable!()))
                });
                block(&mut b, &mut out, lit, dist, limit)?;
            }
            2 => {
                let (nlen, ndist, ncode) = (b.bits(5)? as usize + 257, b.bits(5)? as usize + 1, b.bits(4)? as usize + 4);
                if nlen > 286 || ndist > 30 {
                    return Err(InflateError::Corrupt);
                }
                let mut lengths = [0u8; 320];
                for &o in &ORDER[..ncode] {
                    lengths[o] = b.bits(3)? as u8;
                }
                let code = Huff::new(&lengths[..19]).ok_or(InflateError::Corrupt)?;
                lengths = [0; 320];
                let mut i = 0;
                while i < nlen + ndist {
                    let s = b.sym(&code)?;
                    let (value, repeat) = match s {
                        0..=15 => (s as u8, 1),
                        16 if i > 0 => (lengths[i - 1], 3 + b.bits(2)? as usize),
                        17 => (0, 3 + b.bits(3)? as usize),
                        18 => (0, 11 + b.bits(7)? as usize),
                        _ => return Err(InflateError::Corrupt),
                    };
                    if i + repeat > nlen + ndist {
                        return Err(InflateError::Corrupt);
                    }
                    lengths[i..i + repeat].fill(value);
                    i += repeat;
                }
                if lengths[256] == 0 {
                    return Err(InflateError::Corrupt);
                }
                let lit = Huff::new(&lengths[..nlen]).ok_or(InflateError::Corrupt)?;
                let dist = Huff::new(&lengths[nlen..nlen + ndist]).ok_or(InflateError::Corrupt)?;
                block(&mut b, &mut out, &lit, &dist, limit)?;
            }
            _ => return Err(InflateError::Corrupt),
        }
        if last == 1 {
            b.drop(b.cnt % 8)?;
            let unread = ((b.cnt - b.pad) / 8) as usize;
            return Ok((out, b.pos.min(data.len() + (b.pad / 8) as usize) - (b.pad / 8) as usize - unread));
        }
    }
}

pub fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for chunk in data.chunks(5552) {
        for &x in chunk {
            a += x as u32;
            b += a;
        }
        a %= 65521;
        b %= 65521;
    }
    b << 16 | a
}

/// zlib stream → bytes, at most `limit` of them. Trailing bytes after the checksum are an error, as in the browser.
pub fn inflate(data: &[u8], limit: usize) -> Result<Vec<u8>, InflateError> {
    if data.len() < 6 || data[0] & 0x0f != 8 || data[0] >> 4 > 7 || (((data[0] as u32) << 8) | data[1] as u32) % 31 != 0 || data[1] & 0x20 != 0 {
        return Err(InflateError::Corrupt);
    }
    let (out, used) = inflate_raw(&data[2..], limit)?;
    let tail = &data[2 + used..];
    if tail.len() != 4 || u32::from_be_bytes([tail[0], tail[1], tail[2], tail[3]]) != adler32(&out) {
        return Err(InflateError::Corrupt);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    // python3: zlib.compress(b"hello hello hello hello, inflate!\n" * 3, 9) / level 0 / a fixed-Huffman stream.
    const DYN: &[u8] = &[120, 218, 203, 72, 205, 201, 201, 87, 200, 64, 39, 117, 20, 50, 243, 210, 114, 18, 75, 82, 21, 185, 50, 168, 160, 2, 0, 97, 148, 36, 31];
    const STORED: &[u8] = &[120, 1, 1, 5, 0, 250, 255, 104, 101, 108, 108, 111, 6, 44, 2, 21];

    #[test]
    fn inflates_and_checks() {
        let text = b"hello hello hello hello, inflate!\n".repeat(3);
        assert_eq!(inflate(DYN, 1 << 20).as_deref(), Ok(&text[..]));
        assert_eq!(inflate(STORED, 5).as_deref(), Ok(&b"hello"[..]));
        assert_eq!(inflate(DYN, text.len() - 1), Err(InflateError::TooLarge));
        assert_eq!(inflate(STORED, 4), Err(InflateError::TooLarge));
        for end in 0..DYN.len() {
            assert_eq!(inflate(&DYN[..end], 1 << 20), Err(InflateError::Corrupt), "{end}");
        }
        let mut bad = DYN.to_vec();
        *bad.last_mut().unwrap() ^= 1;
        assert_eq!(inflate(&bad, 1 << 20), Err(InflateError::Corrupt));
        let mut long = DYN.to_vec();
        long.push(0);
        assert_eq!(inflate(&long, 1 << 20), Err(InflateError::Corrupt));
        // Raw deflate is not accepted (the browser's 'deflate' is zlib-wrapped).
        assert_eq!(inflate(&DYN[2..], 1 << 20), Err(InflateError::Corrupt));
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
    }
}
