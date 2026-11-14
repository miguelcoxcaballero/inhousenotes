//! 5.x packed ink (`__inkCheckpoint`, spec/persistence.md §1.1.3): the lossless four-lane XOR codec of
//! `rust-core/src/checkpoint.rs` and the `raw-v1` form, with 5.18.0's validation.

use super::json::Value;
use super::V5Error;

pub const MAX_RAW: usize = 64 * 1024 * 1024;

/// `xor-v1`: for word *i*, `delta = bits ^ previous[i % 4]`; one length byte (0..8) and the low bytes of `delta`.
pub fn xor_encode(words: &[u64]) -> Vec<u8> {
    let mut out = Vec::with_capacity(words.len() * 3);
    let mut previous = [0u64; 4];
    for (i, &bits) in words.iter().enumerate() {
        let delta = bits ^ previous[i % 4];
        previous[i % 4] = bits;
        let length = (64 - delta.leading_zeros() as usize).div_ceil(8);
        out.push(length as u8);
        out.extend_from_slice(&delta.to_le_bytes()[..length]);
    }
    out
}

/// Exactly `words` words; truncated input, a length byte above 8 or trailing bytes are errors.
pub fn xor_decode(input: &[u8], words: usize) -> Option<Vec<u64>> {
    let mut out = Vec::with_capacity(words);
    let mut previous = [0u64; 4];
    let mut read = 0;
    for i in 0..words {
        let length = *input.get(read)? as usize;
        read += 1;
        if length > 8 {
            return None;
        }
        let mut bytes = [0u8; 8];
        bytes[..length].copy_from_slice(input.get(read..read + length)?);
        read += length;
        let bits = u64::from_le_bytes(bytes) ^ previous[i % 4];
        previous[i % 4] = bits;
        out.push(bits);
    }
    (read == input.len()).then_some(out)
}

/// The Float64 words (as bits) of an `__inkCheckpoint` record: `{inkCheckpoint: 1, codec, bytes, rawLength}`.
pub fn unpack(checkpoint: &Value) -> Result<Vec<u64>, V5Error> {
    let Value::Bytes(bytes) = checkpoint.get("bytes") else { return Err(V5Error::Checkpoint("Unknown ink checkpoint format")) };
    if checkpoint.get("inkCheckpoint").finite() != Some(1.0) {
        return Err(V5Error::Checkpoint("Unknown ink checkpoint format"));
    }
    let raw = checkpoint.get("rawLength").num();
    if !(raw >= 0.0 && raw <= MAX_RAW as f64 && raw.fract() == 0.0 && raw % 32.0 == 0.0) {
        return Err(V5Error::Checkpoint("Invalid ink checkpoint length"));
    }
    let raw = raw as usize;
    match checkpoint.get("codec").as_str() {
        Some("raw-v1") if bytes.len() == raw => Ok(bytes.chunks_exact(8).map(|c| u64::from_le_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]])).collect()),
        Some("raw-v1") => Err(V5Error::Checkpoint("Truncated ink checkpoint")),
        Some("xor-v1") => xor_decode(bytes, raw / 8).ok_or(V5Error::Checkpoint("Invalid ink checkpoint")),
        _ => Err(V5Error::Checkpoint("Unknown ink checkpoint codec")),
    }
}

/// The points of `{inkRange: [offset, count]}`: `(xy, p)` with `p` = NaN where the flag word is 0.
pub fn range(words: &[u64], ink_range: &Value) -> Result<(Vec<f64>, Vec<f64>), V5Error> {
    let bad = V5Error::Checkpoint("Invalid ink point range");
    let r = ink_range.arr();
    let int = |v: Option<&Value>| v.and_then(Value::finite).filter(|n| *n >= 0.0 && n.fract() == 0.0 && *n <= 9_007_199_254_740_991.0);
    let (Some(offset), Some(count)) = (int(r.first()), int(r.get(1))) else { return Err(bad) };
    if offset % 4.0 != 0.0 || offset + count * 4.0 > words.len() as f64 {
        return Err(bad);
    }
    let (offset, count) = (offset as usize, count as usize);
    let mut xy = Vec::with_capacity(count * 2);
    let mut p = Vec::new();
    for (i, w) in words[offset..offset + count * 4].chunks_exact(4).enumerate() {
        xy.push(f64::from_bits(w[0]));
        xy.push(f64::from_bits(w[1]));
        match f64::from_bits(w[3]) {
            f if f == 1.0 => {
                if p.is_empty() {
                    p = vec![f64::NAN; count];
                }
                p[i] = f64::from_bits(w[2]);
            }
            f if f == 0.0 => {}
            _ => return Err(V5Error::Checkpoint("Invalid ink pressure flag")),
        }
    }
    Ok((xy, p))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(codec: &str, bytes: Vec<u8>, raw: f64) -> Value {
        Value::Obj(vec![("inkCheckpoint".into(), Value::Num(1.0)), ("codec".into(), Value::Str(codec.into())), ("bytes".into(), Value::Bytes(bytes)), ("rawLength".into(), Value::Num(raw))])
    }

    #[test]
    fn xor_v1_is_bit_exact_and_strict() {
        let words = [0u64, (-0.0f64).to_bits(), f64::INFINITY.to_bits(), f64::NAN.to_bits(), 1, u64::MAX, 0x8000_0000_0000_0001, 42];
        let packed = xor_encode(&words);
        // The byte stream of rust-core/src/checkpoint.rs for the same words.
        assert_eq!(&packed[..12], &[0, 8, 0, 0, 0, 0, 0, 0, 0, 0x80, 8, 0]);
        assert_eq!(xor_decode(&packed, 8).as_deref(), Some(&words[..]));
        for end in 0..packed.len() {
            assert_eq!(xor_decode(&packed[..end], 8), None);
        }
        assert_eq!(xor_decode(&[9], 1), None);
        assert_eq!(xor_decode(&[0, 0], 1), None);
        let ones: Vec<u64> = vec![1f64.to_bits(); 400];
        assert!(xor_encode(&ones).len() < 400 * 8 / 4);

        assert_eq!(unpack(&rec("xor-v1", packed.clone(), 64.0)).as_deref(), Ok(&words[..]));
        let raw: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        assert_eq!(unpack(&rec("raw-v1", raw.clone(), 64.0)).as_deref(), Ok(&words[..]));
        let err = |v: Value| match unpack(&v) {
            Err(e) => e.message(),
            Ok(_) => "",
        };
        assert_eq!(err(rec("raw-v1", raw.clone(), 32.0)), "Truncated ink checkpoint");
        assert_eq!(err(rec("xor-v1", packed.clone(), 32.0)), "Invalid ink checkpoint");
        assert_eq!(err(rec("xor-v1", packed.clone(), 60.0)), "Invalid ink checkpoint length");
        assert_eq!(err(rec("xor-v1", packed, -32.0)), "Invalid ink checkpoint length");
        assert_eq!(err(rec("zip", raw, 64.0)), "Unknown ink checkpoint codec");
        assert_eq!(unpack(&Value::Null), Err(V5Error::Checkpoint("Unknown ink checkpoint format")));
    }

    #[test]
    fn ranges_rebuild_points() {
        let w: Vec<u64> = [1.5, 2.5, 0.25, 1.0, -0.0, 4.0, 0.0, 0.0].iter().map(|f: &f64| f.to_bits()).collect();
        let r = |a: f64, b: f64| Value::Arr(vec![Value::Num(a), Value::Num(b)]);
        let (xy, p) = range(&w, &r(0.0, 2.0)).unwrap();
        assert_eq!(xy, [1.5, 2.5, -0.0, 4.0]);
        assert!(xy[2].is_sign_negative() && p[0] == 0.25 && p[1].is_nan());
        assert_eq!(range(&w, &r(4.0, 1.0)).unwrap().1, Vec::<f64>::new());
        for bad in [r(1.0, 1.0), r(0.0, 3.0), r(-4.0, 1.0), r(0.5, 1.0), Value::Null] {
            assert!(range(&w, &bad).is_err());
        }
        let mut flag = w.clone();
        flag[3] = 2f64.to_bits();
        assert_eq!(range(&flag, &r(0.0, 1.0)), Err(V5Error::Checkpoint("Invalid ink pressure flag")));
    }
}
