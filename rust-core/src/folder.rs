//! Folder scene planning. The host measures layout once and passes these plans
//! to the compositor; this module never drives a per-frame rendering loop.

#[no_mangle]
pub extern "C" fn folder_plan_version() -> u32 { 1 }

/// The private adapter supplies an aligned buffer of at least 29 f64 values.
#[no_mangle]
pub unsafe extern "C" fn folder_motion(
    bounds_left: f64, bounds_top: f64, bounds_width: f64, bounds_height: f64,
    left: f64, top: f64, width: f64, height: f64, backwards: u32, out: *mut f64,
) -> u32 {
    if out.is_null() || ![bounds_left, bounds_top, bounds_width, bounds_height,
        left, top, width, height].iter().all(|value| value.is_finite())
        || bounds_width <= 0.0 || bounds_height <= 0.0 || width <= 0.0 || height <= 0.0 {
        return 0;
    }
    let scale = ((bounds_width + 4.0) / width).max((bounds_height + 4.0) / (height * 0.94));
    let final_width = width * scale;
    let final_height = height * scale;
    let final_left = (bounds_width - final_width) / 2.0;
    let final_top = (bounds_height - height * 0.94 * scale) / 2.0 - height * 0.06 * scale;
    let dx = final_left - (left - bounds_left);
    let dy = final_top - (top - bounds_top);
    let closing = backwards != 0;
    let values = [
        scale, final_left, final_top, dx, dy, final_width, final_height, 1100.0 * scale,
        if closing { 560.0 } else { 600.0 },
        if closing { -112.0 } else { 0.0 }, if closing { 0.0 } else { -112.0 },
        0.55, 0.0, 0.15, 1.0,
        if closing { 0.4 } else { 0.2 }, 0.0, 0.2, 1.0,
        if closing { 0.0 } else { 1.0 }, 0.0,
        if closing { 0.6 } else { 0.4 }, if closing { 1.0 } else { 0.0 },
        if closing { 1.0 } else { 1.0 / scale }, if closing { 1.0 / scale } else { 1.0 },
        if closing { dx } else { 0.0 }, if closing { dy } else { 0.0 },
        if closing { 0.0 } else { dx }, if closing { 0.0 } else { dy },
    ];
    if scale <= 0.0 || !values.iter().all(|value| value.is_finite()) { return 0; }
    std::slice::from_raw_parts_mut(out, values.len()).copy_from_slice(&values);
    1
}

/// Opacity keyframes as (offset, opacity) pairs. Returns the number of pairs.
#[no_mangle]
pub unsafe extern "C" fn folder_fade(elapsed_fraction: f64, backwards: u32, out: *mut f64) -> u32 {
    if out.is_null() || !elapsed_fraction.is_finite() { return 0; }
    if backwards != 0 {
        std::slice::from_raw_parts_mut(out, 6).copy_from_slice(&[0.0, 1.0, 0.3, 1.0, 1.0, 0.0]);
        return 3;
    }
    let from = (elapsed_fraction.clamp(0.0, 1.0) + 0.03).min(0.9);
    let to = (from + 0.15).min(1.0).max(0.7);
    std::slice::from_raw_parts_mut(out, 8).copy_from_slice(&[0.0, 0.0, from, 0.0, to, 1.0, 1.0, 1.0]);
    4
}

pub struct FolderRows {
    tops: Vec<f64>,
    bottoms: Vec<f64>,
    columns: u32,
    count: u32,
}

/// Cache variable CSS grid tracks once. Extra trailing tracks (the New PDF
/// button) do not invent document indices beyond count.
#[no_mangle]
pub unsafe extern "C" fn folder_rows_new(
    heights: *const f64, length: u32, gap: f64, columns: u32, count: u32,
) -> *mut FolderRows {
    if columns == 0 || length > 1_000_000 || !gap.is_finite() || gap < 0.0
        || (length > 0 && heights.is_null()) { return std::ptr::null_mut(); }
    let required = (count as u64).div_ceil(columns as u64) as usize;
    if required > length as usize { return std::ptr::null_mut(); }
    let input = if length == 0 { &[] } else { std::slice::from_raw_parts(heights, length as usize) };
    if input.iter().any(|height| !height.is_finite() || *height <= 0.0) { return std::ptr::null_mut(); }
    let mut rows = FolderRows { tops: Vec::new(), bottoms: Vec::new(), columns, count };
    if rows.tops.try_reserve_exact(required).is_err() || rows.bottoms.try_reserve_exact(required).is_err() {
        return std::ptr::null_mut();
    }
    let mut top = 0.0;
    for height in input.iter().take(required) {
        let bottom = top + height;
        if !bottom.is_finite() || !(bottom + gap).is_finite() { return std::ptr::null_mut(); }
        rows.tops.push(top);
        rows.bottoms.push(bottom);
        top = bottom + gap;
    }
    Box::into_raw(Box::new(rows))
}

/// Binary-search complete visible rows; no allocation or pixel/DOM copying.
#[no_mangle]
pub unsafe extern "C" fn folder_rows_range(
    context: *const FolderRows, view_top: f64, view_bottom: f64, overscan: f64, out: *mut f64,
) -> u32 {
    if context.is_null() || out.is_null() || ![view_top, view_bottom, overscan].iter().all(|v| v.is_finite())
        || view_bottom < view_top || overscan < 0.0 { return 0; }
    let first = view_top - overscan;
    let last = view_bottom + overscan;
    if !first.is_finite() || !last.is_finite() { return 0; }
    let rows = &*context;
    let start_row = rows.bottoms.partition_point(|bottom| *bottom < first);
    let end_row = rows.tops.partition_point(|top| *top <= last).max(start_row);
    let start = ((start_row as u64) * rows.columns as u64).min(rows.count as u64);
    let end = ((end_row as u64) * rows.columns as u64).min(rows.count as u64);
    std::slice::from_raw_parts_mut(out, 2).copy_from_slice(&[start as f64, end as f64]);
    1
}

/// Physical height of a consecutive set of rows, without outside gaps.
#[no_mangle]
pub unsafe extern "C" fn folder_rows_span(context: *const FolderRows, first: u32, end: u32) -> f64 {
    if context.is_null() { return f64::NAN; }
    let rows = &*context;
    if first > end || end as usize > rows.tops.len() { return f64::NAN; }
    if first == end { return 0.0; }
    rows.bottoms[end as usize - 1] - rows.tops[first as usize]
}

#[no_mangle]
pub unsafe extern "C" fn folder_rows_drop(context: *mut FolderRows) {
    if !context.is_null() { drop(Box::from_raw(context)); }
}
