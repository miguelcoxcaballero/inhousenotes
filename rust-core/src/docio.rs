//! PDF background, import and export helpers.
//!
//! Unit export-meta: the pure computation behind `buildPdfBlobWithPdfLib` and
//! `buildPdfBlob` (stroke de-duplication, point compaction, the per-page
//! metadata entry, overlay content bounds, the page hash, the export profile,
//! the copyPages plan and page geometry). pdf-lib, pdf.js, jsPDF and every
//! canvas call stay in JavaScript.

mod export;
mod jv;
mod strokes;

pub fn call(method: &str, input: &[u8]) -> Option<Result<Vec<u8>, String>> {
    Some(match method {
        "docio.page_export" => export::page_export(input),
        "docio.page_hash" => export::page_hash(input),
        "docio.overlay_bounds" => export::bounds_only(input),
        "docio.export_plan" => export::export_plan(input),
        _ => return None,
    })
}

/// Writes `[width, height, mmWidth, mmHeight, landscape]` (5 doubles) for a stored page size.
#[no_mangle]
pub unsafe extern "C" fn docio_page_geometry(out: *mut f64, page_w: f64, page_h: f64, a4_w: f64, a4_h: f64) -> u32 {
    if out.is_null() || !a4_w.is_finite() || !a4_h.is_finite() { return 0; }
    let mut values = [0.0; 5];
    export::page_geometry(&mut values, page_w, page_h, a4_w, a4_h);
    std::ptr::copy_nonoverlapping(values.as_ptr(), out, 5);
    1
}

/// Writes `[logicalW, logicalH, pxW, pxH, ptW, ptH]` (6 doubles); pass NaN for a page that was not copied from a source PDF.
#[no_mangle]
pub unsafe extern "C" fn docio_pdflib_geometry(out: *mut f64, page_w: f64, page_h: f64, a4_w: f64, a4_h: f64, copied_w_pt: f64, copied_h_pt: f64) -> u32 {
    if out.is_null() || !a4_w.is_finite() || !a4_h.is_finite() { return 0; }
    let mut values = [0.0; 6];
    export::pdflib_geometry(&mut values, page_w, page_h, a4_w, a4_h, copied_w_pt, copied_h_pt);
    std::ptr::copy_nonoverlapping(values.as_ptr(), out, 6);
    1
}

/// Writes `[x, y, width, height]` (4 doubles) in PDF points for the cropped overlay image.
#[no_mangle]
pub unsafe extern "C" fn docio_overlay_placement(out: *mut f64, bx: f64, by: f64, bw: f64, bh: f64, pt_w: f64, pt_h: f64, logical_w: f64, logical_h: f64) -> u32 {
    if out.is_null() || ![bx, by, bw, bh, pt_w, pt_h].iter().all(|v| v.is_finite()) || !(logical_w.is_finite() && logical_w > 0.0 && logical_h.is_finite() && logical_h > 0.0) { return 0; }
    let mut values = [0.0; 4];
    export::overlay_placement(&mut values, bx, by, bw, bh, pt_w, pt_h, logical_w, logical_h);
    std::ptr::copy_nonoverlapping(values.as_ptr(), out, 4);
    1
}
