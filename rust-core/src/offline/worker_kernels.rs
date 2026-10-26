//! Document-processing worker kernels: base64 (btoa/atob semantics, byte for byte),
//! the timeline content hash and the prepared-PNG cache eviction planner.
//! Deflate/inflate stay on the browser's native CompressionStream.

use serde::{Deserialize, Serialize};

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const INVALID: u8 = 0xFF;
const SPACE: u8 = 0xFE;
/// ASCII whitespace as defined by the WHATWG forgiving-base64 decoder (what `atob` accepts).
const DECODE: [u8; 256] = {
    let mut table = [INVALID; 256];
    let mut i = 0;
    while i < 64 { table[ALPHABET[i] as usize] = i as u8; i += 1; }
    table[b'\t' as usize] = SPACE; table[b'\n' as usize] = SPACE; table[0x0C] = SPACE;
    table[b'\r' as usize] = SPACE; table[b' ' as usize] = SPACE;
    table
};

pub fn base64_len(bytes: usize) -> usize { bytes.div_ceil(3) * 4 }

/// Standard padded base64, identical to `btoa` over the same octets. `dst` must hold `base64_len(src.len())` bytes.
pub fn base64_encode(src: &[u8], dst: &mut [u8]) -> usize {
    let mut out = 0;
    let mut chunks = src.chunks_exact(3);
    for c in &mut chunks {
        let v = (c[0] as u32) << 16 | (c[1] as u32) << 8 | c[2] as u32;
        dst[out] = ALPHABET[(v >> 18) as usize]; dst[out + 1] = ALPHABET[(v >> 12 & 63) as usize];
        dst[out + 2] = ALPHABET[(v >> 6 & 63) as usize]; dst[out + 3] = ALPHABET[(v & 63) as usize];
        out += 4;
    }
    match *chunks.remainder() {
        [a] => {
            let v = (a as u32) << 16;
            dst[out] = ALPHABET[(v >> 18) as usize]; dst[out + 1] = ALPHABET[(v >> 12 & 63) as usize];
            dst[out + 2] = b'='; dst[out + 3] = b'='; out += 4;
        }
        [a, b] => {
            let v = (a as u32) << 16 | (b as u32) << 8;
            dst[out] = ALPHABET[(v >> 18) as usize]; dst[out + 1] = ALPHABET[(v >> 12 & 63) as usize];
            dst[out + 2] = ALPHABET[(v >> 6 & 63) as usize]; dst[out + 3] = b'='; out += 4;
        }
        _ => {}
    }
    out
}

/// Upper bound of decoded bytes for `len` input characters.
pub fn base64_decoded_capacity(len: usize) -> usize { len / 4 * 3 + 3 }

/// `atob` semantics (forgiving-base64): ASCII whitespace ignored, up to two trailing `=` when the
/// cleaned length divides by 4, missing padding allowed, leftover bits dropped. `Err` where `atob` throws.
/// `dst` must hold `base64_decoded_capacity(src.len())` bytes.
pub fn base64_decode(src: &[u8], dst: &mut [u8]) -> Result<usize, ()> {
    let mut count = src.iter().filter(|&&b| DECODE[b as usize] != SPACE).count();
    let mut end = src.len();
    if count % 4 == 0 {
        for _ in 0..2 {
            while end > 0 && DECODE[src[end - 1] as usize] == SPACE { end -= 1; }
            if end > 0 && src[end - 1] == b'=' { end -= 1; count -= 1; } else { break; }
        }
    }
    if count % 4 == 1 { return Err(()); }
    let (mut acc, mut bits, mut out) = (0u32, 0u32, 0usize);
    for &byte in &src[..end] {
        let value = DECODE[byte as usize];
        if value >= SPACE { if value == SPACE { continue; } return Err(()); }
        acc = acc << 6 | value as u32; bits += 6;
        if bits >= 8 { bits -= 8; dst[out] = (acc >> bits) as u8; out += 1; acc &= (1 << bits) - 1; }
    }
    Ok(out)
}

const P1: u32 = 31;
const P2: u32 = P1.wrapping_mul(P1);
const P3: u32 = P2.wrapping_mul(P1);
const P4: u32 = P3.wrapping_mul(P1);

/// `hash = (hash << 5) - hash + unit | 0` over UTF-16 code units is `hash * 31 + unit` modulo 2^32.
/// ASCII text is hashed four units at a time, which removes the serial multiply chain.
fn hash_ascii(bytes: &[u8]) -> u32 {
    let mut hash = 0u32;
    let mut chunks = bytes.chunks_exact(4);
    for c in &mut chunks {
        hash = hash.wrapping_mul(P4).wrapping_add((c[0] as u32).wrapping_mul(P3))
            .wrapping_add((c[1] as u32).wrapping_mul(P2)).wrapping_add((c[2] as u32).wrapping_mul(P1)).wrapping_add(c[3] as u32);
    }
    for &b in chunks.remainder() { hash = hash.wrapping_mul(P1).wrapping_add(b as u32); }
    hash
}

fn hash_units(units: impl Iterator<Item = u16>) -> (u32, u32) {
    let (mut hash, mut count) = (0u32, 0u32);
    for unit in units { hash = hash.wrapping_mul(P1).wrapping_add(unit as u32); count = count.wrapping_add(1); }
    (hash, count)
}

/// JSON text as UTF-8 (a well-formed JS string) or raw UTF-16LE code units (lone surrogates survive).
/// Returns `<abs(hash) base36>:<UTF-16 length base36>` exactly as the former worker loop did.
pub fn timeline_hash(bytes: &[u8], utf16: bool) -> Result<String, String> {
    let (hash, units) = if utf16 {
        if bytes.len() % 2 != 0 { return Err("UTF-16 input has an odd byte length".into()); }
        hash_units(bytes.chunks_exact(2).map(|p| u16::from_le_bytes([p[0], p[1]])))
    } else if bytes.is_ascii() {
        (hash_ascii(bytes), bytes.len() as u32)
    } else {
        let text = std::str::from_utf8(bytes).map_err(|_| "Input is not valid UTF-8".to_string())?;
        hash_units(text.encode_utf16())
    };
    let magnitude = (hash as i32 as i64).unsigned_abs();
    let mut text = base36(magnitude);
    text.truncate(8);
    Ok(format!("{text}:{}", base36(units as u64)))
}

fn base36(mut value: u64) -> String {
    if value == 0 { return "0".into(); }
    let mut digits = Vec::new();
    while value > 0 { digits.push(b"0123456789abcdefghijklmnopqrstuvwxyz"[(value % 36) as usize]); value /= 36; }
    digits.reverse();
    String::from_utf8(digits).unwrap_or_default()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CachePlanInput { limit: f64, sizes: Vec<f64>, incoming: f64 }
#[derive(Serialize, Debug, PartialEq)]
pub struct CachePlan { pub store: bool, pub evict: usize }

/// Byte-budget LRU: `sizes` lists the cached entries oldest first. An entry larger than the budget is never
/// stored; otherwise the oldest entries go until the newcomer fits.
pub fn png_cache_plan(limit: f64, sizes: &[f64], incoming: f64) -> Result<CachePlan, String> {
    if !(limit.is_finite() && limit >= 0.0 && incoming.is_finite() && incoming >= 0.0 && sizes.iter().all(|s| s.is_finite() && *s >= 0.0)) {
        return Err("Invalid cache sizes".into());
    }
    if incoming > limit { return Ok(CachePlan { store: false, evict: 0 }); }
    let mut total: f64 = sizes.iter().sum();
    let mut evict = 0;
    while total + incoming > limit && evict < sizes.len() { total -= sizes[evict]; evict += 1; }
    Ok(CachePlan { store: true, evict })
}

pub fn call(method: &str, input: &[u8]) -> Option<Result<Vec<u8>, String>> {
    match method {
        "offline.png_cache_plan" => Some(crate::bridge::json(input, |v: CachePlanInput| png_cache_plan(v.limit, &v.sizes, v.incoming))),
        _ => None,
    }
}

/// Writes `base64_len(len)` ASCII bytes at `dst` and returns that count; 0 when the arguments are invalid.
#[no_mangle]
pub unsafe extern "C" fn offline_b64_encode(src: *const u8, len: usize, dst: *mut u8) -> usize {
    if len == 0 || src.is_null() || dst.is_null() || len > 96 * 1024 * 1024 { return 0; }
    base64_encode(std::slice::from_raw_parts(src, len), std::slice::from_raw_parts_mut(dst, base64_len(len)))
}

/// Decodes like `atob`. Returns the byte count, -1 for text `atob` rejects, -2 for invalid arguments.
#[no_mangle]
pub unsafe extern "C" fn offline_b64_decode(src: *const u8, len: usize, dst: *mut u8, capacity: usize) -> isize {
    if len > 96 * 1024 * 1024 || (len > 0 && src.is_null()) || dst.is_null() || capacity < base64_decoded_capacity(len) { return -2; }
    let input: &[u8] = if len == 0 { &[] } else { std::slice::from_raw_parts(src, len) };
    match base64_decode(input, std::slice::from_raw_parts_mut(dst, capacity)) { Ok(count) => count as isize, Err(()) => -1 }
}

/// Writes the hash text at `out` (capacity >= 32) and returns its length, or 0 when the input is invalid.
#[no_mangle]
pub unsafe extern "C" fn offline_timeline_hash(src: *const u8, len: usize, utf16: u32, out: *mut u8, capacity: usize) -> usize {
    if (len > 0 && src.is_null()) || out.is_null() || capacity < 32 || len > 96 * 1024 * 1024 { return 0; }
    let input: &[u8] = if len == 0 { &[] } else { std::slice::from_raw_parts(src, len) };
    let Ok(text) = timeline_hash(input, utf16 == 1) else { return 0; };
    std::ptr::copy_nonoverlapping(text.as_ptr(), out, text.len());
    text.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn encode(src: &[u8]) -> String { let mut out = vec![0; base64_len(src.len())]; let n = base64_encode(src, &mut out); String::from_utf8(out[..n].to_vec()).unwrap() }
    fn decode(text: &str) -> Result<Vec<u8>, ()> { let mut out = vec![0; base64_decoded_capacity(text.len())]; base64_decode(text.as_bytes(), &mut out).map(|n| out[..n].to_vec()) }
    #[test] fn rfc4648_vectors() {
        for (plain, coded) in [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foob", "Zm9vYg=="), ("fooba", "Zm9vYmE="), ("foobar", "Zm9vYmFy")] {
            assert_eq!(encode(plain.as_bytes()), coded);
            assert_eq!(decode(coded).unwrap(), plain.as_bytes());
        }
    }
    #[test] fn every_byte_round_trips() {
        let all: Vec<u8> = (0..=255).collect();
        assert_eq!(decode(&encode(&all)).unwrap(), all);
    }
    #[test] fn forgiving_decoder_matches_atob() {
        assert_eq!(decode("Zg").unwrap(), b"f");               // padding optional
        assert_eq!(decode(" Zm9v\r\nYmFy\t").unwrap(), b"foobar"); // ASCII whitespace ignored
        assert_eq!(decode("Zg == ").unwrap(), b"f");           // whitespace before padding
        assert_eq!(decode("Zh==").unwrap(), b"f");             // leftover bits dropped
        assert!(decode("Z").is_err()); assert!(decode("Zg=").is_err()); assert!(decode("Zg===").is_err());
        assert!(decode("Z=g=").is_err()); assert!(decode("Zm9v!").is_err()); assert!(decode("Zm-v").is_err());
        assert!(decode("====").is_err()); assert_eq!(decode("").unwrap(), b"");
        assert!(decode("\u{e9}abc").is_err()); assert!(decode("\u{b}Zg==").is_err()); // vertical tab is not whitespace
    }
    #[test] fn hash_matches_the_former_loop() {
        fn reference(text: &str) -> String {
            let mut hash = 0i32; let mut len = 0u64;
            for unit in text.encode_utf16() { hash = hash.wrapping_shl(5).wrapping_sub(hash).wrapping_add(unit as i32); len += 1; }
            format!("{}:{}", base36((hash as i64).unsigned_abs()), base36(len))
        }
        for text in ["", "a", "ab", "abc", "abcd", "abcde", "{\"pages\":[]}", "h\u{e9}llo \u{1F600} \u{20AC}", &"x".repeat(1001), &"\u{1F600}".repeat(33)] {
            assert_eq!(timeline_hash(text.as_bytes(), false).unwrap(), reference(text), "{text}");
            let units: Vec<u8> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
            assert_eq!(timeline_hash(&units, true).unwrap(), reference(text));
        }
        assert_eq!(timeline_hash(b"", false).unwrap(), "0:0");
        // i32::MIN magnitude, as Math.abs gives 2147483648
        assert_eq!(base36((i32::MIN as i64).unsigned_abs()), "zik0zk");
        assert!(timeline_hash(&[0xff, 0xfe], false).is_err()); assert!(timeline_hash(&[1], true).is_err());
    }
    #[test] fn lone_surrogates_hash_as_utf16_units() {
        let units: Vec<u8> = [0x61u16, 0xD800, 0x62].iter().flat_map(|u| u.to_le_bytes()).collect();
        let mut hash = 0i32; for u in [0x61i32, 0xD800, 0x62] { hash = hash.wrapping_mul(31).wrapping_add(u); }
        assert_eq!(timeline_hash(&units, true).unwrap(), format!("{}:3", base36((hash as i64).unsigned_abs())));
    }
    #[test] fn cache_plan_evicts_oldest_until_fit() {
        let limit = 100.0;
        assert_eq!(png_cache_plan(limit, &[], 40.0).unwrap(), CachePlan { store: true, evict: 0 });
        assert_eq!(png_cache_plan(limit, &[30.0, 30.0, 30.0], 10.0).unwrap(), CachePlan { store: true, evict: 0 });
        assert_eq!(png_cache_plan(limit, &[30.0, 30.0, 30.0], 11.0).unwrap(), CachePlan { store: true, evict: 1 });
        assert_eq!(png_cache_plan(limit, &[30.0, 30.0, 30.0], 100.0).unwrap(), CachePlan { store: true, evict: 3 });
        assert_eq!(png_cache_plan(limit, &[30.0], 101.0).unwrap(), CachePlan { store: false, evict: 0 });
        assert!(png_cache_plan(limit, &[f64::NAN], 1.0).is_err());
    }
    #[test] fn bridge_plan_round_trip() {
        let out = crate::bridge::dispatch("offline.png_cache_plan", br#"{"limit":10,"sizes":[6,6],"incoming":5}"#).unwrap();
        assert_eq!(out, br#"{"store":true,"evict":2}"#);
        assert!(crate::bridge::dispatch("offline.png_cache_plan", b"{}").is_err());
    }
    #[test] fn exports_reject_bad_arguments() {
        let mut out = [0u8; 64];
        unsafe {
            assert_eq!(offline_b64_encode(std::ptr::null(), 3, out.as_mut_ptr()), 0);
            assert_eq!(offline_b64_decode(b"Zg==".as_ptr(), 4, out.as_mut_ptr(), 1), -2);
            assert_eq!(offline_b64_decode(b"Z".as_ptr(), 1, out.as_mut_ptr(), 64), -1);
            assert_eq!(offline_b64_decode(b"Zg==".as_ptr(), 4, out.as_mut_ptr(), 64), 1);
            assert_eq!(offline_timeline_hash(b"abc".as_ptr(), 3, 0, out.as_mut_ptr(), 8), 0);
            let n = offline_timeline_hash(b"abc".as_ptr(), 3, 0, out.as_mut_ptr(), 64);
            assert_eq!(&out[..n], b"22ci:3");
        }
    }
}
