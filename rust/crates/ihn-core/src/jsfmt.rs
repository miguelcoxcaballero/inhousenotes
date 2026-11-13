//! ES `Number::toString` (ECMA-262 §6.1.6.1.20, radix 10) and `Number.prototype.toFixed`: the exact text
//! JavaScript produces for a number, so numbers written into DOM strings (inline styles, attributes,
//! `aria-valuenow`) match 5.18.0's `outerHTML` (ARCHITECTURE §3.3). Verified against Chromium on
//! `tests/v6/fixtures/math/{tostring,tofixed}.json`.

use std::fmt::Write;

/// `String(x)` / `` `${x}` `` / `x.toString()`.
pub fn num(x: f64) -> String {
    let mut out = String::new();
    push_num(&mut out, x);
    out
}

/// Appends `String(x)` to `out`.
pub fn push_num(out: &mut String, x: f64) {
    if x.is_nan() {
        return out.push_str("NaN");
    }
    if x == 0.0 {
        return out.push('0'); // also -0
    }
    if x < 0.0 {
        out.push('-');
    }
    let x = x.abs();
    if x.is_infinite() {
        return out.push_str("Infinity");
    }
    if x < 9.0e15 && x == x.trunc() {
        let _ = write!(out, "{}", x as u64); // exact: every integer below 2^53 prints all its digits
        return;
    }
    // Rust's `{:e}` gives the shortest digit string that round-trips (the `s`, `k` of the spec) as `d.ddde<n-1>`.
    let mut sci = String::with_capacity(24);
    let _ = write!(sci, "{x:e}");
    let (mantissa, exponent) = sci.split_once('e').unwrap_or((&sci, "0"));
    let n = exponent.parse::<i32>().unwrap_or(0) + 1;
    let (first, rest) = mantissa.split_at(1);
    let rest = rest.strip_prefix('.').unwrap_or(rest);
    let k = mantissa.len() as i32 - i32::from(!rest.is_empty());
    let even = if k >= 16 { even_tie(x, first, rest, n - k) } else { None };
    let (first, rest) = even.as_ref().map_or((first, rest), |digits| digits.split_at(1));
    let zeros = |out: &mut String, count: i32| out.extend(std::iter::repeat_n('0', count as usize));
    if k <= n && n <= 21 {
        out.push_str(first);
        out.push_str(rest);
        zeros(out, n - k);
    } else if 0 < n && n <= 21 {
        out.push_str(first);
        let (int, frac) = rest.split_at(n as usize - 1);
        out.push_str(int);
        out.push('.');
        out.push_str(frac);
    } else if -6 < n && n <= 0 {
        out.push_str("0.");
        zeros(out, -n);
        out.push_str(first);
        out.push_str(rest);
    } else {
        out.push_str(first);
        if k > 1 {
            out.push('.');
            out.push_str(rest);
        }
        out.push('e');
        out.push(if n > 0 { '+' } else { '-' });
        let _ = write!(out, "{}", (n - 1).abs());
    }
}

/// The spec picks, among the shortest digit strings that round-trip, the one closest to `x`, and the even one when
/// two are equally close; Rust's formatter may return the odd one. Returns the even neighbour of the digits
/// `first rest` (value `digits × 10^q`) when `x` is exactly half-way between the two and both round-trip.
/// Integer arithmetic only: no float parsing, which would pull 10 KB of tables into the wasm.
fn even_tie(x: f64, first: &str, rest: &str, q: i32) -> Option<String> {
    let digits: u64 = format!("{first}{rest}").parse().ok()?;
    if digits % 2 == 0 {
        return None;
    }
    let (m, e) = odd_mantissa(x);
    // x = m·2^e is the midpoint iff 2x = (digits + neighbour)·10^q; both sides are an odd number times a power of two.
    if e + 1 != q {
        return None;
    }
    let pow5 = 5u128.checked_pow(q.unsigned_abs())?;
    [digits - 1, digits + 1].into_iter().find(|&neighbour| {
        let sum = u128::from(digits + neighbour);
        let midpoint = if q >= 0 { pow5.checked_mul(sum) == Some(u128::from(m)) } else { pow5.checked_mul(u128::from(m)) == Some(sum) };
        // `digits` round-trips, and the neighbour is as far from `x` on the other side, so it round-trips too —
        // except below a power of two, where doubles are twice as dense: there it needs 10^q <= ulp(x) / 2.
        midpoint && (m != 1 || neighbour > digits || sum >= 1 << 54)
    })
    .map(|neighbour| neighbour.to_string())
    .filter(|text| text.len() == first.len() + rest.len())
}

/// `x = m · 2^e` with `m` odd (`x` finite, non-zero).
fn odd_mantissa(x: f64) -> (u64, i32) {
    let bits = x.to_bits();
    let (exponent, fraction) = ((bits >> 52 & 0x7FF) as i32, bits & ((1 << 52) - 1));
    let (m, e) = if exponent == 0 { (fraction, -1074) } else { (fraction | 1 << 52, exponent - 1075) };
    (m >> m.trailing_zeros(), e + m.trailing_zeros() as i32)
}

/// `x.toFixed(digits)` (`digits` 0..=100): exact decimal rounding, exact ties away from zero.
pub fn fixed(x: f64, digits: usize) -> String {
    if !x.is_finite() || x.abs() >= 1e21 {
        return num(x);
    }
    let a = x.abs();
    let mut out = format!("{a:.digits$}");
    // Rust rounds exact ties to even. `a` is a tie iff its binary fraction ends exactly one decimal place further.
    if a != 0.0 && odd_mantissa(a).1 == -(digits as i32 + 1) {
        let exact = format!("{a:.0$}", digits + 1);
        let mut bytes = exact[..exact.len() - 1].trim_end_matches('.').as_bytes().to_vec();
        let mut i = bytes.len();
        loop {
            if i == 0 {
                bytes.insert(0, b'1');
                break;
            }
            i -= 1;
            match bytes[i] {
                b'.' => {}
                b'9' => bytes[i] = b'0',
                _ => {
                    bytes[i] += 1;
                    break;
                }
            }
        }
        out = String::from_utf8(bytes).unwrap_or(out);
    }
    if x < 0.0 {
        out.insert(0, '-');
    }
    out
}

/// `core.jsfmt.num`: args = JSON array of 16-digit hex bit patterns; returns a JSON array of the strings.
pub fn test_cmd(name: &str, args: &str) -> Option<String> {
    if name != "num" {
        return None;
    }
    let cells: Vec<String> = crate::jsmath::hex_args(args).iter().map(|&x| format!("\"{}\"", num(x))).collect();
    Some(format!("[{}]", cells.join(",")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jsmath::tests::{bits, corpus};

    #[test]
    fn matches_chromium_number_to_string() {
        let rows = corpus("tostring");
        let bad: Vec<String> = rows
            .iter()
            .filter(|row| num(bits(&row[0])) != row[1])
            .map(|row| format!("{} -> {:?}, Chromium {:?}", row[0], num(bits(&row[0])), row[1]))
            .collect();
        assert!(bad.is_empty(), "{} of {} differ:\n{}", bad.len(), rows.len(), bad[..bad.len().min(12)].join("\n"));
    }

    #[test]
    fn matches_chromium_to_fixed() {
        let rows = corpus("tofixed");
        let bad: Vec<String> = rows
            .iter()
            .map(|row| (row, fixed(bits(&row[0]), row[1].parse().expect("digits"))))
            .filter(|(row, got)| *got != row[2])
            .map(|(row, got)| format!("{:?}.toFixed({}) -> {got:?}, Chromium {:?}", bits(&row[0]), row[1], row[2]))
            .collect();
        assert!(bad.is_empty(), "{} of {} differ:\n{}", bad.len(), rows.len(), bad[..bad.len().min(12)].join("\n"));
    }

    #[test]
    fn spec_branches() {
        for (x, want) in [
            (0.0, "0"), (-0.0, "0"), (f64::NAN, "NaN"), (f64::INFINITY, "Infinity"), (f64::NEG_INFINITY, "-Infinity"),
            (1.0, "1"), (-12.5, "-12.5"), (0.1 + 0.2, "0.30000000000000004"), (1e21, "1e+21"), (1e20, "100000000000000000000"),
            (123456789012345680000.0, "123456789012345680000"), (1.5e21, "1.5e+21"), (1e-6, "0.000001"), (1e-7, "1e-7"),
            (1.25e-7, "1.25e-7"), (0.000123, "0.000123"), (5e-324, "5e-324"), (f64::MAX, "1.7976931348623157e+308"),
            (9007199254740993.0, "9007199254740992"), (4.35, "4.35"), (100.0, "100"), (1.0e15 + 0.5, "1000000000000000.5"),
        ] {
            assert_eq!(num(x), want);
        }
        assert_eq!(num(1368354512933715.25), "1368354512933715.2"); // tie: even digit, not Rust's ".3"
        assert_eq!(num(f64::from_bits(0x3E60000000000000)), "2.9802322387695312e-8"); // 2^-25: tie at a power of two, both round-trip
        assert_eq!(num(f64::from_bits(0x3E70000000000000)), "5.960464477539063e-8"); // 2^-24: tie, but the even neighbour is another double
        for (x, digits, want) in [(2.5, 0, "3"), (-2.5, 0, "-3"), (0.5, 0, "1"), (1.005, 2, "1.00"), (0.125, 2, "0.13"), (9.5, 0, "10"),
            (99.995, 2, "100.00"), (-0.0, 2, "0.00"), (-0.001, 2, "-0.00"), (1e21, 2, "1e+21"), (f64::NAN, 1, "NaN"), (12.0, 3, "12.000")] {
            assert_eq!(fixed(x, digits), want);
        }
        assert_eq!(test_cmd("num", r#"["3ff8000000000000","c000000000000000"]"#).unwrap(), r#"["1.5","-2"]"#);
        assert!(test_cmd("x", "[]").is_none());
    }
}
