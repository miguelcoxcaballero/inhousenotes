import { expect, test } from '@playwright/test';

// Regression coverage for a data-integrity bug: saving ink over a PDF could
// silently fall back to a raster pipeline that burns strokes into the page's
// pixels. Once that happens, the eraser can only paint an opaque white cover
// over the burnt pixels instead of truly removing the stroke — visible,
// sometimes-irreversible residue. See app-v5.js buildPdfBlob /
// buildPdfBlobWithPdfLib / markDocumentLegacyBakedAfterRasterFallback.

async function primeSyntheticPdfPage(page) {
  await page.goto('/?e2e=1', { waitUntil: 'domcontentloaded' });
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  await page.evaluate(() => window.__IHN_TEST_API__.ready());
  const pdfBytes = await page.evaluate(async () => {
    const doc = await window.PDFLib.PDFDocument.create();
    doc.addPage([200, 300]);
    const bytes = await doc.save();
    return Array.from(bytes);
  });
  await page.evaluate(bytes => window.__IHN_TEST_API__.primePdfPageForTest(bytes), pdfBytes);
  return pdfBytes.length;
}

test('saving a PDF with the clean original available never falls back to legacy baking', async ({ page }) => {
  await primeSyntheticPdfPage(page);
  const result = await page.evaluate(() => window.__IHN_TEST_API__.buildPdfBlobForTest());
  expect(result.ok).toBe(true);
  expect(result.size).toBeGreaterThan(0);
  expect(result.hasLegacyBakedOverlay).toBe(false);
  expect(result.legacyCoverPageCount).toBe(0);
});

test('a transient pdf-lib failure is retried instead of immediately burning strokes into pixels', async ({ page }) => {
  await primeSyntheticPdfPage(page);
  await page.evaluate(() => {
    const original = window.PDFLib.PDFDocument.load;
    let calls = 0;
    window.PDFLib.PDFDocument.load = function (...args) {
      calls += 1;
      if (calls === 1) return Promise.reject(new Error('simulated transient pdf-lib failure'));
      return original.apply(window.PDFLib.PDFDocument, args);
    };
    window.__ihnRestorePdfLibLoad = () => { window.PDFLib.PDFDocument.load = original; };
  });
  const result = await page.evaluate(() => window.__IHN_TEST_API__.buildPdfBlobForTest());
  await page.evaluate(() => window.__ihnRestorePdfLibLoad());

  expect(result.ok).toBe(true);
  expect(result.hasLegacyBakedOverlay).toBe(false);
  expect(result.legacyCoverPageCount).toBe(0);
});

test('a persistent pdf-lib failure is marked honestly, still carries the clean original, and self-heals', async ({ page }) => {
  const originalLength = await primeSyntheticPdfPage(page);
  // Realistic simulation: pdf-lib can never parse THIS specific original PDF
  // (a real-world malformed/unusual source file), every time it's tried — but
  // still works fine for any other PDF, including the raster fallback's own
  // simple jsPDF-produced output. A blanket "PDFDocument.load always throws"
  // stub would be unrealistic (it would also break the attach step that's
  // supposed to save this document from permanent damage) and duck-test
  // nothing that happens in production.
  await page.evaluate((originalLength) => {
    const original = window.PDFLib.PDFDocument.load;
    window.PDFLib.PDFDocument.load = function (data, ...rest) {
      const len = data?.byteLength ?? data?.length;
      if (len === originalLength) {
        return Promise.reject(new Error('simulated persistent pdf-lib failure on the original PDF'));
      }
      return original.call(window.PDFLib.PDFDocument, data, ...rest);
    };
    window.__ihnRestorePdfLibLoad = () => { window.PDFLib.PDFDocument.load = original; };
  }, originalLength);
  const marked = await page.evaluate(() => window.__IHN_TEST_API__.buildPdfBlobForTest());
  expect(marked.hasLegacyBakedOverlay).toBe(true);
  expect(marked.legacyCoverPageCount).toBe(1);
  // Closing the recovery gap: even though this save was forced through the
  // destructive raster pipeline (strokes flattened into page pixels), the
  // clean original must still travel with the file it wrote. Otherwise a
  // user who closes right after this save — before any later save gets a
  // chance to self-heal — loses reversible editing permanently, which is
  // exactly the gap that was found and is being closed here.
  expect(marked.ok).toBe(true);
  expect(marked.hasCleanOriginalAttachment).toBe(true);

  // Self-heal: once the safe path works again, the next successful save must
  // drop the legacy covers — a document should never be stuck believing it
  // can't erase cleanly once it demonstrably can again. This save doesn't
  // need to re-embed the attachment (needsCleanOriginalAttachmentForSave()
  // correctly skips it when page structure is unchanged, since the safe path
  // already preserves the true original pages by copying them directly —
  // that's a different, equally valid non-destructive mechanism, not a gap).
  await page.evaluate(() => window.__ihnRestorePdfLibLoad());
  const healed = await page.evaluate(() => window.__IHN_TEST_API__.buildPdfBlobForTest());
  expect(healed.ok).toBe(true);
  expect(healed.hasLegacyBakedOverlay).toBe(false);
  expect(healed.legacyCoverPageCount).toBe(0);
});
