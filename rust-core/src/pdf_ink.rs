//! PDF pen paths. Preserve every sample; no smoothing or simplification.
use std::fmt::Write;

fn encode(values: &[f64], x: f64, bottom: f64) -> Option<String> {
    if ![x, bottom].iter().all(|v| v.is_finite() && v.abs() <= 1e7) {
        return None;
    }
    let mut text = String::with_capacity(values.len().saturating_mul(20));
    writeln!(text, "q\n1 0 0 -1 {:.6} {:.6} cm\n1 J 1 j", -x, bottom).ok()?;
    let mut i = 0;
    while i < values.len() {
        if i + 5 > values.len() {
            return None;
        }
        let [width, r, g, b, count] = <[f64; 5]>::try_from(&values[i..i + 5]).ok()?;
        if !width.is_finite()
            || width <= 0.0
            || width > 1e5
            || ![r, g, b]
                .iter()
                .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
            || !count.is_finite()
            || count < 2.0
            || count > 1e6
            || count.fract() != 0.0
        {
            return None;
        }
        i += 5;
        let n = count as usize;
        if i + n * 2 > values.len() {
            return None;
        }
        writeln!(text, "{width:.6} w\n{r:.6} {g:.6} {b:.6} RG").ok()?;
        for (index, pair) in values[i..i + n * 2].chunks_exact(2).enumerate() {
            if !pair.iter().all(|v| v.is_finite() && v.abs() <= 1e7) {
                return None;
            }
            writeln!(
                text,
                "{:.6} {:.6} {}",
                pair[0],
                pair[1],
                if index == 0 { "m" } else { "l" }
            )
            .ok()?;
        }
        text.push_str("S\n");
        i += n * 2;
    }
    text.push_str("Q\n");
    Some(text)
}

/// All pointers are private aligned allocations supplied by the JS bridge.
#[no_mangle]
pub unsafe extern "C" fn pdf_ink_path(
    values: *const f64,
    len: usize,
    x: f64,
    bottom: f64,
    out: *mut u8,
    capacity: usize,
) -> usize {
    if values.is_null() || out.is_null() || len > 3_000_000 || capacity > 128 * 1024 * 1024 {
        return 0;
    }
    let Some(text) = encode(std::slice::from_raw_parts(values, len), x, bottom) else {
        return 0;
    };
    if text.len() > capacity {
        return 0;
    }
    std::ptr::copy_nonoverlapping(text.as_ptr(), out, text.len());
    text.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn samples_and_style_are_retained() {
        let text = encode(
            &[2.0, 0.0, 0.0, 1.0, 3.0, 10.0, 20.0, 10.0, 20.0, 15.0, 24.0],
            8.0,
            30.0,
        )
        .unwrap();
        assert!(text.contains("-8.000000 30.000000 cm"));
        assert!(
            text.contains("10.000000 20.000000 m\n10.000000 20.000000 l\n15.000000 24.000000 l")
        );
        assert!(encode(
            &[2.0, 0.0, 0.0, 1.0, 2.0, 0.0, f64::NAN, 1.0, 2.0],
            0.0,
            100.0
        )
        .is_none());
        assert!(encode(&[2.0, 0.0, 0.0, 1.0, 3.0, 1.0, 2.0], 0.0, 100.0).is_none());
    }
}
