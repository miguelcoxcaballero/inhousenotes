//! Item fingerprints for stroke/image sync comparison (unit eh-1).
//!
//! These reproduce the legacy JavaScript byte for byte: `Math.round(v * 100) / 100`
//! (ties toward +infinity), `Number.prototype.toString`, `-0` printing as `0`, and the
//! 32-bit `(h << 5) - h + code | 0` string hash over UTF-16 code units. The host packs
//! numbers as `f64` (non-finite already mapped to 0) and strings stay on the JS side;
//! only the numeric tails, which are the expensive part, are produced here.

/// `Math.round`: nearest integer, ties toward +infinity, exact for every finite double.
pub fn round_half_up(x: f64) -> f64 {
    if !x.is_finite() { return x; }
    let floor = x.floor();
    if x - floor >= 0.5 { floor + 1.0 } else { floor }
}

/// Shortest round-tripping digits of a positive finite `v` and its decimal exponent (`d.ddd` x 10^exponent).
/// Rust already picks the shortest digit string closest to `v`, but when two candidates are exactly equally
/// close (possible once 16 or 17 digits are needed) it rounds up, while ECMAScript chooses the even digit.
fn shortest_digits(v: f64) -> (Vec<u8>, i32) {
    let text = format!("{v:e}");
    let (mantissa, exponent) = text.split_once('e').unwrap_or((&text, "0"));
    let exponent: i32 = exponent.parse().unwrap_or(0);
    let mut digits: Vec<u8> = mantissa.bytes().filter(|b| *b != b'.').collect();
    if digits.len() >= 16 {
        // 1100 fractional digits exceed the longest exact expansion of any double (767 significant digits).
        let exact = format!("{v:.1100e}");
        let (long_mantissa, _) = exact.split_once('e').unwrap_or((&exact, "0"));
        let long: Vec<u8> = long_mantissa.bytes().filter(|b| *b != b'.').collect();
        let k = digits.len();
        if long.len() > k && long[k] == b'5' && long[k + 1..].iter().all(|d| *d == b'0') {
            let lower = long[..k].to_vec();
            let mut upper = lower.clone();
            let mut index = k;
            let mut carried = true;
            while carried && index > 0 { index -= 1; if upper[index] == b'9' { upper[index] = b'0'; } else { upper[index] += 1; carried = false; } }
            let round_trips = |candidate: &[u8]| -> bool {
                let text = format!("{}e{}", String::from_utf8_lossy(candidate), exponent - (k as i32 - 1));
                text.parse::<f64>().map(|parsed| parsed == v).unwrap_or(false)
            };
            let (lower_ok, upper_ok) = (round_trips(&lower), !carried && round_trips(&upper));
            digits = if lower_ok && upper_ok { if (lower[k - 1] - b'0') % 2 == 0 { lower } else { upper } }
                else if upper_ok { upper } else { lower };
        }
    }
    (digits, exponent)
}

/// ECMAScript `Number::toString(10)`.
pub fn js_number_to_string(value: f64, out: &mut Vec<u8>) {
    if value.is_nan() { out.extend_from_slice(b"NaN"); return; }
    if value == 0.0 { out.push(b'0'); return; }
    let mut v = value;
    if v < 0.0 { out.push(b'-'); v = -v; }
    if v.is_infinite() { out.extend_from_slice(b"Infinity"); return; }
    let (digits, exponent) = shortest_digits(v);
    let k = digits.len() as i32;
    let n = exponent + 1;
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

/// `Number.isFinite(v) ? Math.round(v * scale) / scale : 0`, printed as JavaScript prints it.
/// `scale` is 100 (or 10000 for rotation); `decimals` is its digit count.
pub fn write_rounded(out: &mut Vec<u8>, value: f64, scale: u64, decimals: usize) {
    if !value.is_finite() { out.push(b'0'); return; }
    let rounded = round_half_up(value * scale as f64);
    if rounded.abs() < 1e15 {
        // n / scale for an integer n below 1e15 has at most 15 significant digits, so its
        // shortest round-trip form is exactly the decimal expansion (DBL_DIG guarantee).
        if rounded == 0.0 { out.push(b'0'); return; }
        if rounded < 0.0 { out.push(b'-'); }
        let magnitude = rounded.abs() as u64;
        let (whole, fraction) = (magnitude / scale, magnitude % scale);
        let mut digits = [0u8; 20];
        let mut length = 0;
        let mut rest = whole;
        loop { digits[length] = b'0' + (rest % 10) as u8; length += 1; rest /= 10; if rest == 0 { break; } }
        while length > 0 { length -= 1; out.push(digits[length]); }
        if fraction != 0 {
            let mut text = [b'0'; 8];
            let mut rest = fraction;
            for slot in (0..decimals).rev() { text[slot] = b'0' + (rest % 10) as u8; rest /= 10; }
            let mut end = decimals;
            while text[end - 1] == b'0' { end -= 1; }
            out.push(b'.');
            out.extend_from_slice(&text[..end]);
        }
    } else {
        js_number_to_string(rounded / scale as f64, out);
    }
}

#[inline] pub fn hash_bytes(mut h: i32, bytes: &[u8]) -> i32 {
    for &byte in bytes { h = h.wrapping_mul(31).wrapping_add(byte as i32); }
    h
}
#[inline] pub fn hash_units(mut h: i32, units: &[u16]) -> i32 {
    for &unit in units { h = h.wrapping_mul(31).wrapping_add(unit as i32); }
    h
}

/// `Math.abs(h).toString(36).slice(0, 8)`.
pub fn write_hash_text(out: &mut Vec<u8>, h: i32) {
    const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut rest = (h as i64).unsigned_abs();
    let mut buffer = [0u8; 16];
    let mut length = 0;
    loop { buffer[length] = DIGITS[(rest % 36) as usize]; length += 1; rest /= 36; if rest == 0 { break; } }
    for index in (0..length).rev().take(8) { out.push(buffer[index]); }
}

/// Hash of the legacy point string `x,y,p;x,y,p;...` for packed triplets. A triplet whose x is
/// NaN stands for a hole in a sparse array, which `Array.prototype.join` renders as empty text.
pub fn points_hash(points: &[f64]) -> i32 {
    let mut h = 0i32;
    let mut text = Vec::with_capacity(96);
    for (index, point) in points.chunks_exact(3).enumerate() {
        if index > 0 { h = hash_bytes(h, b";"); }
        if point[0].is_nan() { continue; }
        text.clear();
        write_rounded(&mut text, point[0], 100, 2);
        text.push(b',');
        write_rounded(&mut text, point[1], 100, 2);
        text.push(b',');
        write_rounded(&mut text, point[2], 100, 2);
        h = hash_bytes(h, &text);
    }
    h
}

/// `width|pointCount|pointHash` followed by a tab and the stamp clock (`max(0, floor(clock))`).
pub fn write_stroke_record(out: &mut Vec<u8>, width: f64, clock: f64, points: &[f64]) {
    write_rounded(out, width, 100, 2);
    out.push(b'|');
    out.extend_from_slice((points.len() / 3).to_string().as_bytes());
    out.push(b'|');
    write_hash_text(out, points_hash(points));
    out.push(b'\t');
    write_clock(out, clock);
}

/// `srcHash|x|y|width|height|rotation` for an image whose source text hash is already known,
/// followed by a tab and the stamp clock.
pub fn write_image_tail(out: &mut Vec<u8>, src_hash: i32, geometry: [f64; 5], clock: f64) {
    write_hash_text(out, src_hash);
    for (index, value) in geometry.iter().enumerate() {
        out.push(b'|');
        if index == 4 { write_rounded(out, *value, 10000, 4); } else { write_rounded(out, *value, 100, 2); }
    }
    out.push(b'\t');
    write_clock(out, clock);
}

/// `Math.max(0, Math.floor(clock))` printed as JavaScript prints it.
fn write_clock(out: &mut Vec<u8>, clock: f64) { js_number_to_string(clock.floor().max(0.0) + 0.0, out); }

const MAX_STROKES: usize = 1 << 22;
/// A record is at most about 90 bytes; the host reserves this much per stroke.
pub const RECORD_RESERVE: usize = 128;

/// Hash `len` bytes (ASCII text) or UTF-16 units (`wide`), continuing from `seed`.
/// Returns the new hash, or `i64::MIN` for invalid arguments.
#[no_mangle]
pub unsafe extern "C" fn history_hash_update(seed: i32, ptr: *const u8, len: usize, wide: u32) -> i64 {
    if len == 0 { return seed as i64; }
    if ptr.is_null() || len > (1 << 28) { return i64::MIN; }
    if wide != 0 {
        if (ptr as usize) % 2 != 0 { return i64::MIN; }
        hash_units(seed, std::slice::from_raw_parts(ptr as *const u16, len)) as i64
    } else {
        hash_bytes(seed, std::slice::from_raw_parts(ptr, len)) as i64
    }
}

/// Write `Math.abs(h).toString(36).slice(0, 8)`. Returns its length or -1.
#[no_mangle]
pub unsafe extern "C" fn history_hash_text(h: i32, out: *mut u8, cap: usize) -> i32 {
    if out.is_null() || cap < 8 { return -1; }
    let mut text = Vec::with_capacity(8);
    write_hash_text(&mut text, h);
    std::ptr::copy_nonoverlapping(text.as_ptr(), out, text.len());
    text.len() as i32
}

/// Image tail from the source hash, geometry (x, y, width, height, rotation) and stamp clock.
/// Returns its length or -1.
#[no_mangle]
pub unsafe extern "C" fn history_image_tail(src_hash: i32, x: f64, y: f64, width: f64, height: f64, rotation: f64, clock: f64, out: *mut u8, cap: usize) -> i32 {
    if out.is_null() || cap < 160 { return -1; }
    let mut text = Vec::with_capacity(96);
    write_image_tail(&mut text, src_hash, [x, y, width, height, rotation], clock);
    if text.len() > cap { return -1; }
    std::ptr::copy_nonoverlapping(text.as_ptr(), out, text.len());
    text.len() as i32
}

/// Fingerprint records for `count` strokes. `meta` holds (width, clock) per stroke, `offsets`
/// holds `count + 1` point indices into `points` (x, y, p triplets). Records are
/// `width|count|hash<TAB>clock<LF>`. Returns the bytes written or -1 for invalid arguments.
#[no_mangle]
pub unsafe extern "C" fn history_stroke_batch(meta: *const f64, offsets: *const u32, points: *const f64, count: usize,
    total_points: usize, out: *mut u8, cap: usize) -> i64 {
    if count == 0 { return 0; }
    if count > MAX_STROKES || meta.is_null() || offsets.is_null() || out.is_null() || (total_points > 0 && points.is_null())
        || total_points > (1 << 28) || cap < count * RECORD_RESERVE { return -1; }
    if (meta as usize) % 8 != 0 || (points as usize) % 8 != 0 || (offsets as usize) % 4 != 0 { return -1; }
    let meta = std::slice::from_raw_parts(meta, count * 2);
    let offsets = std::slice::from_raw_parts(offsets, count + 1);
    let points: &[f64] = if total_points == 0 { &[] } else { std::slice::from_raw_parts(points, total_points * 3) };
    let mut text: Vec<u8> = Vec::with_capacity(count * 48);
    for index in 0..count {
        let (start, end) = (offsets[index] as usize, offsets[index + 1] as usize);
        if start > end || end > total_points { return -1; }
        write_stroke_record(&mut text, meta[index * 2], meta[index * 2 + 1], &points[start * 3..end * 3]);
        text.push(b'\n');
    }
    if text.len() > cap { return -1; }
    std::ptr::copy_nonoverlapping(text.as_ptr(), out, text.len());
    text.len() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn exact_ties_choose_the_even_digit() {
        // Both -1590602947399020.2 and ...0.3 are 0.05 away from the double ...020.25; ECMAScript prints ...0.2.
        assert_eq!(num(-1590602947399020.2), "-1590602947399020.2");
        assert_eq!(num(1590602947399020.2), "1590602947399020.2");
        assert_eq!(num(2f64.powi(60) + 256.0), "1152921504606847200");
        assert_eq!(num(9007199254740993.0), "9007199254740992");
    }
    fn num(v: f64) -> String { let mut out = Vec::new(); js_number_to_string(v, &mut out); String::from_utf8(out).unwrap() }
    fn rounded(v: f64, scale: u64, decimals: usize) -> String { let mut out = Vec::new(); write_rounded(&mut out, v, scale, decimals); String::from_utf8(out).unwrap() }
    fn hash_text(h: i32) -> String { let mut out = Vec::new(); write_hash_text(&mut out, h); String::from_utf8(out).unwrap() }
    fn simple_hash(text: &str) -> String { hash_text(hash_units(0, &text.encode_utf16().collect::<Vec<_>>())) }

    #[test] fn math_round_matches_javascript() {
        for (input, expected) in [(0.5, 1.0), (1.5, 2.0), (2.5, 3.0), (-0.5, 0.0), (-1.5, -1.0), (-2.5, -2.0), (0.49999999999999994, 0.0),
            (-0.49999999999999994, 0.0), (4503599627370495.5, 4503599627370496.0), (1e300, 1e300), (-3.7, -4.0), (3.2, 3.0)] {
            assert_eq!(round_half_up(input), expected, "{input}");
        }
        assert!(round_half_up(f64::NAN).is_nan());
        assert_eq!(round_half_up(f64::INFINITY), f64::INFINITY);
    }
    #[test] fn number_to_string_follows_ecmascript() {
        for (value, text) in [(1.0, "1"), (-1.0, "-1"), (0.0, "0"), (-0.0, "0"), (1.5, "1.5"), (0.1 + 0.2, "0.30000000000000004"),
            (1e21, "1e+21"), (1e20, "100000000000000000000"), (123456789012345680000.0, "123456789012345680000"), (1e-6, "0.000001"),
            (1e-7, "1e-7"), (1.5e-7, "1.5e-7"), (5e-324, "5e-324"), (1.7976931348623157e308, "1.7976931348623157e+308"),
            (f64::INFINITY, "Infinity"), (f64::NEG_INFINITY, "-Infinity"), (f64::NAN, "NaN"), (123.456, "123.456"),
            (1.2345e25, "1.2345e+25"), (-2.5e-10, "-2.5e-10"), (4294967296.0, "4294967296")] {
            assert_eq!(num(value), text);
        }
    }
    #[test] fn rounded_values_print_like_javascript() {
        assert_eq!(rounded(1.005, 100, 2), "1"); // 1.005 * 100 = 100.49999999999999
        assert_eq!(rounded(2.675, 100, 2), "2.68");
        assert_eq!(rounded(-0.004, 100, 2), "0");
        assert_eq!(rounded(-0.005, 100, 2), "0");
        assert_eq!(rounded(-0.006, 100, 2), "-0.01");
        assert_eq!(rounded(12.3456, 100, 2), "12.35");
        assert_eq!(rounded(100.0, 100, 2), "100");
        assert_eq!(rounded(0.1, 100, 2), "0.1");
        assert_eq!(rounded(f64::NAN, 100, 2), "0");
        assert_eq!(rounded(f64::INFINITY, 100, 2), "0");
        assert_eq!(rounded(1e307, 100, 2), "Infinity");
        assert_eq!(rounded(1e22, 100, 2), "1e+22");
        assert_eq!(rounded(0.12345, 10000, 4), "0.1235");
        assert_eq!(rounded(-6.2832, 10000, 4), "-6.2832");
        assert_eq!(rounded(0.0001, 10000, 4), "0.0001");
    }
    #[test] fn fast_path_equals_general_path() {
        let mut state = 0x9e3779b97f4a7c15u64;
        for _ in 0..200_000 {
            state ^= state << 13; state ^= state >> 7; state ^= state << 17;
            let v = ((state >> 11) as f64 / (1u64 << 53) as f64 - 0.5) * 10f64.powi((state % 14) as i32);
            for (scale, decimals) in [(100u64, 2usize), (10000, 4)] {
                let expected = { let r = round_half_up(v * scale as f64) / scale as f64; num(r) };
                assert_eq!(rounded(v, scale, decimals), expected, "{v}");
            }
        }
    }
    #[test] fn simple_hash_matches_javascript() {
        assert_eq!(simple_hash(""), "0");
        assert_eq!(simple_hash("a"), "2p");
        assert_eq!(simple_hash("hello world"), "to5x38");
        // INT32_MIN: Math.abs(-2147483648).toString(36) === "zik0zk"
        assert_eq!(hash_text(i32::MIN), "zik0zk");
        assert_eq!(hash_text(-1), "1");
        let astral = "\u{1F600}"; // two UTF-16 code units
        assert_eq!(hash_units(0, &astral.encode_utf16().collect::<Vec<_>>()), 55357 * 31 + 56832);
    }
    #[test] fn points_and_records() {
        assert_eq!(points_hash(&[]), 0);
        let mut out = Vec::new();
        write_stroke_record(&mut out, 2.345, 7.9, &[1.0, 2.0, 0.5, 3.0, 4.0, 0.0]);
        let text = String::from_utf8(out).unwrap();
        let expected_hash = simple_hash("1,2,0.5;3,4,0");
        assert_eq!(text, format!("2.35|2|{expected_hash}\t7"));
        // A hole renders as empty text, not as 0,0,0.
        assert_eq!(hash_text(points_hash(&[f64::NAN, 0.0, 0.0, 1.0, 1.0, 1.0])), simple_hash(";1,1,1"));
    }
    #[test] fn batch_kernel_validates_input() {
        let meta = [1.0, 0.0, 2.0, 5.0];
        let offsets = [0u32, 1, 2];
        let points = [0.0, 0.0, 0.0, 1.0, 1.0, 1.0];
        let mut out = vec![0u8; 512];
        let written = unsafe { history_stroke_batch(meta.as_ptr(), offsets.as_ptr(), points.as_ptr(), 2, 2, out.as_mut_ptr(), out.len()) };
        assert!(written > 0);
        let text = String::from_utf8(out[..written as usize].to_vec()).unwrap();
        assert_eq!(text, format!("1|1|{}\t0\n2|1|{}\t5\n", simple_hash("0,0,0"), simple_hash("1,1,1")));
        let bad = [0u32, 3, 2];
        assert_eq!(unsafe { history_stroke_batch(meta.as_ptr(), bad.as_ptr(), points.as_ptr(), 2, 2, out.as_mut_ptr(), out.len()) }, -1);
        assert_eq!(unsafe { history_stroke_batch(meta.as_ptr(), offsets.as_ptr(), points.as_ptr(), 2, 2, out.as_mut_ptr(), 100) }, -1);
        assert_eq!(unsafe { history_stroke_batch(std::ptr::null(), offsets.as_ptr(), points.as_ptr(), 2, 2, out.as_mut_ptr(), out.len()) }, -1);
    }
    #[test] fn image_tail_and_hash_entry_points() {
        let mut out = Vec::new();
        write_image_tail(&mut out, -1, [1.234, f64::NAN, 3.0, 4.0, 0.12345], 12.9);
        assert_eq!(String::from_utf8(out).unwrap(), "1|1.23|0|3|4|0.1235\t12");
        let text = "abc";
        assert_eq!(unsafe { history_hash_update(0, text.as_ptr(), 3, 0) } as i32, hash_bytes(0, b"abc"));
        assert_eq!(unsafe { history_hash_update(0, std::ptr::null(), 3, 0) }, i64::MIN);
    }
}
