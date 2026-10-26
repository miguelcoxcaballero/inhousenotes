//! Persistence logic ported from the notebook storage layer. Pure functions only: IndexedDB,
//! fetch, timers and token storage stay with the host. Every entry point validates its lengths
//! and returns a status instead of panicking.
pub fn call(_method: &str, _input: &[u8]) -> Option<Result<Vec<u8>, String>> { None }

mod ink_words {
    //! Ink page checkpoint words: four f64 per point (x, y, pressure, has-pressure flag), the
    //! layout already stored in user databases and fed to the lossless codec in `checkpoint.rs`.

    /// Per-point shape tags written by the host while it reads the live point objects.
    pub const TAG_X: u8 = 1;
    pub const TAG_Y: u8 = 2;
    pub const TAG_HAS_P: u8 = 4;
    pub const TAG_P: u8 = 8;
    /// The point is not an object, or it has keys other than x, y and p.
    pub const TAG_BAD: u8 = 16;
    /// Stroke whose `points` is not an array; the host keeps its structured clone.
    pub const NOT_ARRAY: u32 = u32::MAX;
    /// Stroke that cannot use the binary representation.
    pub const NOT_PACKED: f64 = -1.0;

    pub const OK: isize = 0;
    pub const INVALID_RANGE: isize = 1;
    pub const INVALID_FLAG: isize = 2;

    fn portable(tag: u8) -> bool {
        tag & (TAG_BAD | TAG_X | TAG_Y) == TAG_X | TAG_Y && (tag & TAG_HAS_P == 0 || tag & TAG_P != 0)
    }

    /// Decide per stroke whether all points are portable, then lay the portable ones out as words.
    /// `offsets[i]` receives the word offset of stroke `i` or `NOT_PACKED`. Returns the used word count.
    pub fn pack(raw: &[f64], tags: &[u8], counts: &[u32], words: &mut [f64], offsets: &mut [f64]) -> Option<usize> {
        if raw.len() != tags.len() * 3 || words.len() < tags.len() * 4 || offsets.len() != counts.len() { return None; }
        let (mut cursor, mut used) = (0usize, 0usize);
        for (stroke, &count) in counts.iter().enumerate() {
            if count == NOT_ARRAY { offsets[stroke] = NOT_PACKED; continue; }
            let count = count as usize;
            let end = cursor.checked_add(count).filter(|&end| end <= tags.len())?;
            if !tags[cursor..end].iter().all(|&tag| portable(tag)) { offsets[stroke] = NOT_PACKED; cursor = end; continue; }
            offsets[stroke] = used as f64;
            for point in cursor..end {
                let tag = tags[point];
                let target = &mut words[used..used + 4];
                target[0] = raw[point * 3];
                target[1] = raw[point * 3 + 1];
                // A missing pressure is stored as +0 with flag 0; an explicit one keeps its value.
                let has_p = tag & TAG_HAS_P != 0;
                target[2] = if has_p { raw[point * 3 + 2] } else { 0.0 };
                target[3] = if has_p { 1.0 } else { 0.0 };
                used += 4;
            }
            cursor = end;
        }
        Some(used)
    }

    fn safe_integer(value: f64) -> bool { value.is_finite() && value.fract() == 0.0 && value.abs() <= 9007199254740991.0 }

    /// Validate every `[offset, count]` pair against the decoded words, in stroke order, exactly as the
    /// original decoder did: range first, then the pressure flags of that stroke's points.
    pub fn validate(words: &[f64], ranges: &[f64]) -> isize {
        let length = words.len() as f64;
        for pair in ranges.chunks_exact(2) {
            let (offset, count) = (pair[0], pair[1]);
            if !safe_integer(offset) || !safe_integer(count) || offset < 0.0 || count < 0.0
                || offset % 4.0 != 0.0 || offset + count * 4.0 > length { return INVALID_RANGE; }
            let (start, count) = (offset as usize, count as usize);
            for point in 0..count {
                let flag = words[start + point * 4 + 3];
                if flag != 1.0 && flag != 0.0 { return INVALID_FLAG; }
            }
        }
        OK
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        fn point(raw: &mut Vec<f64>, tags: &mut Vec<u8>, x: f64, y: f64, p: Option<f64>) {
            raw.extend([x, y, p.unwrap_or(0.0)]);
            tags.push(TAG_X | TAG_Y | if p.is_some() { TAG_HAS_P | TAG_P } else { 0 });
        }
        #[test] fn packs_words_in_stroke_order_and_keeps_special_values() {
            let (mut raw, mut tags) = (vec![], vec![]);
            point(&mut raw, &mut tags, -0.0, 2.0, Some(0.0));
            point(&mut raw, &mut tags, 1.5, f64::NAN, None);
            point(&mut raw, &mut tags, f64::INFINITY, -3.0, Some(0.25));
            let (mut words, mut offsets) = (vec![9.0; 12], vec![0.0; 3]);
            assert_eq!(pack(&raw, &tags, &[2, NOT_ARRAY, 1], &mut words, &mut offsets), Some(12));
            assert_eq!(offsets, [0.0, NOT_PACKED, 8.0]);
            assert!(words[0].is_sign_negative() && words[0] == 0.0 && words[2] == 0.0 && words[3] == 1.0);
            assert_eq!(words[4], 1.5); assert!(words[5].is_nan()); assert!(words[6] == 0.0 && words[6].is_sign_positive() && words[7] == 0.0);
            assert_eq!(&words[8..12], &[f64::INFINITY, -3.0, 0.25, 1.0]);
        }
        #[test] fn a_bad_point_rewinds_only_its_own_stroke() {
            let (mut raw, mut tags) = (vec![], vec![]);
            for i in 0..5 { point(&mut raw, &mut tags, i as f64, 0.0, None); }
            for bad in [TAG_X | TAG_Y | TAG_BAD, TAG_X, TAG_Y, TAG_X | TAG_Y | TAG_HAS_P, TAG_BAD, 0] {
                let mut tags = tags.clone(); tags[3] = bad;
                let (mut words, mut offsets) = (vec![0.0; 20], vec![0.0; 3]);
                assert_eq!(pack(&raw, &tags, &[2, 2, 1], &mut words, &mut offsets), Some(12));
                assert_eq!(offsets, [0.0, NOT_PACKED, 8.0], "tag {bad}");
                assert_eq!(words[8], 4.0);
            }
        }
        #[test] fn empty_arrays_are_packed_with_no_words() {
            let (mut words, mut offsets) = (vec![], vec![7.0; 2]);
            assert_eq!(pack(&[], &[], &[0, NOT_ARRAY], &mut words, &mut offsets), Some(0));
            assert_eq!(offsets, [0.0, NOT_PACKED]);
        }
        #[test] fn inconsistent_buffers_are_rejected() {
            let mut offsets = vec![0.0; 1]; let mut words = vec![0.0; 4];
            assert!(pack(&[0.0; 3], &[3], &[2], &mut words, &mut offsets).is_none());
            assert!(pack(&[0.0; 3], &[3], &[1], &mut vec![0.0; 3], &mut offsets).is_none());
            assert!(pack(&[0.0; 3], &[3], &[1], &mut words, &mut vec![0.0; 2]).is_none());
            assert!(pack(&[0.0; 2], &[3], &[1], &mut words, &mut offsets).is_none());
            assert!(pack(&[], &[], &[u32::MAX - 1], &mut vec![], &mut offsets).is_none());
        }
        #[test] fn validates_ranges_and_flags_in_stroke_order() {
            let words = [1.0, 2.0, 0.5, 1.0, 3.0, 4.0, 0.0, 0.0, 5.0, 6.0, 0.0, 2.0];
            assert_eq!(validate(&words, &[0.0, 2.0]), OK);
            assert_eq!(validate(&words, &[]), OK);
            assert_eq!(validate(&words, &[8.0, 0.0, 4.0, 1.0]), OK);
            assert_eq!(validate(&words, &[0.0, 3.0]), INVALID_FLAG);
            assert_eq!(validate(&words, &[0.0, 3.0, 99.0, 1.0]), INVALID_FLAG);
            assert_eq!(validate(&words, &[99.0, 1.0, 0.0, 3.0]), INVALID_RANGE);
            for bad in [[0.0, 4.0], [1.0, 1.0], [-4.0, 1.0], [0.0, -1.0], [f64::NAN, 1.0], [0.0, f64::NAN], [0.5, 1.0],
                [0.0, 1.5], [f64::INFINITY, 0.0], [9007199254740992.0, 0.0], [0.0, 9007199254740992.0], [8.0, 2.0]] {
                assert_eq!(validate(&words, &bad), INVALID_RANGE, "{bad:?}");
            }
            assert_eq!(validate(&words, &[-0.0, 1.0]), OK);
            assert_eq!(validate(&words, &[12.0, 0.0]), OK);
            assert_eq!(validate(&[0.0, 0.0, 0.0, f64::NAN], &[0.0, 1.0]), INVALID_FLAG);
        }
    }
}

/// Lay out the ink points of one page as checkpoint words.
/// `raw` holds x, y, p per point and `tags` the host's shape tags; `counts` lists the points of each stroke
/// (`u32::MAX` for a stroke whose points are not an array). Writes `strokes` offsets (word index, or -1 when
/// the stroke keeps its lossless structured clone) and returns the number of words used, or -1 on bad input.
/// `words` must have room for `4 * points` entries.
#[no_mangle]
pub unsafe extern "C" fn ink_pack_words(raw: *const f64, tags: *const u8, points: usize, counts: *const u32, strokes: usize,
    words: *mut f64, offsets: *mut f64) -> isize {
    const MAX: usize = 16 * 1024 * 1024;
    if points > MAX || strokes > MAX || raw.is_null() || tags.is_null() || counts.is_null() || words.is_null() || offsets.is_null() { return -1; }
    match ink_words::pack(std::slice::from_raw_parts(raw, points * 3), std::slice::from_raw_parts(tags, points),
        std::slice::from_raw_parts(counts, strokes), std::slice::from_raw_parts_mut(words, points * 4),
        std::slice::from_raw_parts_mut(offsets, strokes)) {
        Some(used) => used as isize,
        None => -1,
    }
}

/// Check the `[offset, count]` pairs (2 f64 each) of a page against decoded checkpoint words.
/// Returns 0 when valid, 1 for an invalid range, 2 for an invalid pressure flag, -1 for bad arguments.
#[no_mangle]
pub unsafe extern "C" fn ink_validate_words(words: *const f64, length: usize, ranges: *const f64, strokes: usize) -> isize {
    const MAX: usize = 16 * 1024 * 1024;
    if length > MAX * 4 || strokes > MAX || words.is_null() || ranges.is_null() { return -1; }
    ink_words::validate(std::slice::from_raw_parts(words, length), std::slice::from_raw_parts(ranges, strokes * 2))
}
