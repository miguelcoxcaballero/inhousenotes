import { expect, test } from '@playwright/test';
import fs from 'node:fs';

const fixtureSource = fs.readFileSync(new URL('./fixtures/scanner-synthetic.js', import.meta.url), 'utf8');
const physicalFixtureSource = fs.readFileSync(new URL('./fixtures/scanner-developable.js', import.meta.url), 'utf8');

function installCaptureCamera(options = {}) {
  window.__captureRegression = { streams: [], invalidOutputs: 0 };
  Object.defineProperty(navigator.mediaDevices, 'getUserMedia', {
    configurable: true,
    value: async () => {
      if (options.denied) throw new DOMException('Permission denied by fixture', 'NotAllowedError');
      // Camera capture now requires an independently validated physical grid.
      // The legacy Coons fixture bends two axes without a developable paper
      // surface; retain it below for ordinary photo-import compatibility only.
      const canvas = ScannerDevelopableFixtures.render({ radius: 0, axis: 0, pitch: .1, yaw: .1 }).source;
      const stream = canvas.captureStream(20);
      const track = stream.getVideoTracks()[0];
      const context = canvas.getContext('2d');
      const refresh = setInterval(() => {
        context.fillStyle = '#8a674a';
        context.fillRect(0, 0, 2, 2);
      }, 50);
      const stop = track.stop.bind(track);
      track.stop = () => { clearInterval(refresh); stop(); };
      track.applyConstraints = async () => {};
      window.__captureRegression.streams.push(stream);
      return stream;
    }
  });
}

async function installFixture(page, options = {}) {
  await page.addInitScript({ content: `${fixtureSource}\n${physicalFixtureSource}\n;(${installCaptureCamera.toString()})(${JSON.stringify(options)});` });
}

async function holdFirstBatch(frame) {
  await frame.evaluate(() => {
    const original = ScannerPro.Lightweight.processPageAsync;
    const probe = window.__captureRegression;
    probe.processingCalls = 0;
    probe.pendingResults = 0;
    probe.completedResults = [];
    const gate = new Promise(resolve => { probe.releaseResults = resolve; });
    ScannerPro.Lightweight.processPageAsync = async function (...args) {
      const call = ++probe.processingCalls;
      try {
        const processed = await original.apply(this, args);
        if (call <= 3) {
          // Hold worker results at the asynchronous boundary. Deliberately
          // ignore cancellation here to exercise stale-result protection too.
          probe.pendingResults++;
          await gate;
        }
        return processed;
      } finally { probe.completedResults.push(call); }
    };
  });
}

async function invalidateFirstBatch(frame, failure) {
  await frame.evaluate(failure => {
    const original = ScannerPro.Lightweight.processPageAsync;
    let calls = 0;
    ScannerPro.Lightweight.processPageAsync = async function (...args) {
      const call = ++calls;
      const processed = await original.apply(this, args);
      if (call > 3) return processed;
      __captureRegression.invalidOutputs++;
      if (failure === 'unverified') return { ...processed, corrected: false,
        failureReason: 'The page dots could not be verified. Try more light.' };
      return { ...processed, blob: new Blob(['invalid encoded image'], { type: 'image/jpeg' }) };
    };
  }, failure);
}

async function expectDecodedPreview(frame) {
  await expect(frame.locator('#livePreview')).toBeVisible();
  await expect.poll(() => frame.locator('#livePreview').evaluate(image =>
    image.complete && image.naturalWidth > 100 && image.naturalHeight > 100)).toBe(true);
}

async function openStandalone(page, options = {}) {
  await installFixture(page, options);
  await page.goto('/scanner/index.html?embed=1&session=capture-regression', { waitUntil: 'domcontentloaded' });
  await expect(page.locator('#liveScanner')).toBeVisible();
}

test('capture shows a decoded photo while correction is pending, then inserts only the decoded result', async ({ page }) => {
  test.setTimeout(60_000);
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await installFixture(page);
  await page.goto('/?e2e=1', { waitUntil: 'domcontentloaded' });
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  await page.evaluate(async () => {
    await __IHN_TEST_API__.ready();
    await __IHN_TEST_API__.resetLocalDocument(1, 'Captured preview regression');
    await __IHN_TEST_API__.showEditorForTest();
  });
  await page.locator('#btn-edit-pages').click();
  await page.locator('#btn-scan-page').click();
  const iframe = page.frameLocator('#scanner-editor-frame');
  const frame = await page.locator('#scanner-editor-frame').elementHandle().then(handle => handle.contentFrame());
  await frame.waitForFunction(() => !!window.ScannerPro?.Lightweight);
  await holdFirstBatch(frame);
  await frame.waitForFunction(() => window.__captureRegression.pendingResults > 0, null, { timeout: 20_000 });
  await expect(iframe.locator('#liveScanner')).toHaveAttribute('data-state', 'processing');
  await expectDecodedPreview(frame);
  await expect(iframe.locator('#liveAddBtn')).toBeDisabled();
  expect(await page.evaluate(() => __IHN_TEST_API__.snapshot().pages.length)).toBe(1);
  expect(await frame.evaluate(() => __captureRegression.streams.flatMap(stream => stream.getTracks().map(track => track.readyState))))
    .toEqual(['ended']);
  await frame.evaluate(() => __captureRegression.releaseResults());
  await expect(iframe.locator('#liveScanner')).toHaveAttribute('data-state', 'review', { timeout: 20_000 });
  await expectDecodedPreview(frame);
  await expect(iframe.locator('#liveAddBtn')).toBeEnabled();
  expect(await iframe.locator('#livePreview').evaluate(image => image.naturalWidth / image.naturalHeight)).toBeCloseTo(21 / 29.7, 2);
  await iframe.locator('#liveAddBtn').click();
  await expect(page.locator('#scanner-editor-overlay')).not.toHaveClass(/visible/);
  const persisted = await page.evaluate(async () => {
    const saved = (await __IHN_TEST_API__.checkpointBeforeLeaving()).snapshot;
    const scannedPage = saved.pages[1];
    return { pages: saved.pages.length, strokes: scannedPage.strokes, images: scannedPage.images,
      backgroundSource: scannedPage.backgroundSource, backgroundImage: scannedPage.backgroundImage,
      templateKind: scannedPage.templateKind, dirtyPages: saved.dirtyPages };
  });
  expect(persisted.pages).toBe(2);
  expect(persisted.images).toEqual([]);
  expect(persisted.strokes.length).toBeGreaterThan(0);
  expect(persisted.strokes.every(stroke=>stroke.tool==='pen')).toBe(true);
  expect(persisted.backgroundSource).toBe('template');
  expect(persisted.templateKind).toBe('default');
  expect(persisted.backgroundImage).toBeNull();
  expect(persisted.dirtyPages).toEqual([]);
  expect(errors).toEqual([]);
});

for (const failure of ['undecodable', 'unverified']) test(`${failure} captures stay available until an explicit retake`, async ({ page }) => {
  test.setTimeout(60000);
  await openStandalone(page);
  await invalidateFirstBatch(page, failure);
  await page.waitForFunction(() => __captureRegression.invalidOutputs === 3, null, {timeout:25000});
  await expect(page.locator('#liveAddBtn')).toBeDisabled();
  await expect(page.locator('#liveScanner')).toHaveAttribute('data-state','review',{timeout:25000});
  await expectDecodedPreview(page);
  const retained = await page.locator('#livePreview').getAttribute('src');
  if (failure === 'unverified') await expect(page.locator('#liveReviewStatus')).toContainText('Try more light');
  // The old 350 ms restart discarded the captured image and its error.
  await page.waitForTimeout(900);
  await expect(page.locator('#liveScanner')).toHaveAttribute('data-state','review');
  await expect(page.locator('#livePreview')).toHaveAttribute('src', retained);
  await expect(page.locator('#liveAddBtn')).toBeDisabled();
  expect(await page.evaluate(() => __captureRegression.streams.length)).toBe(1);
  await page.locator('#liveRetakeBtn').click();
  await expect(page.locator('#liveAddBtn')).toBeEnabled({timeout:20000});
  await expectDecodedPreview(page);
  await expect(page.locator('#liveReviewStatus')).not.toContainText('Original photo ready');
  expect(await page.evaluate(() => __captureRegression.streams.length)).toBeGreaterThanOrEqual(2);
});

test('invalid imported image leaves processing and a subsequent valid photo can be reviewed', async ({ page }) => {
  test.setTimeout(35_000);
  await openStandalone(page, { denied: true });
  await page.locator('#liveFileInput').setInputFiles({ name: 'damaged.jpg', mimeType: 'image/jpeg', buffer: Buffer.from('not a JPEG') });
  await expect(page.locator('#liveScanner')).not.toHaveAttribute('data-state', 'processing', { timeout: 10_000 });
  await expect(page.locator('#liveAddBtn')).toBeDisabled();
  await expect(page.locator('#liveReviewStatus')).toContainText(/could not|couldn't|decode|open|failed|try again/i);
  await expect(page.locator('#livePreview')).toBeHidden();
  const valid = await page.evaluate(() => ScannerSyntheticFixtures.renderCase(ScannerSyntheticFixtures.CASES[0]).source.toDataURL('image/jpeg', .9).split(',')[1]);
  await page.locator('#liveFileInput').setInputFiles({ name: 'valid.jpg', mimeType: 'image/jpeg', buffer: Buffer.from(valid, 'base64') });
  await expect(page.locator('#liveScanner')).toHaveAttribute('data-state', 'review', { timeout: 20_000 });
  await expectDecodedPreview(page);
  await expect(page.locator('#liveAddBtn')).toBeEnabled();
});

test('retaking during correction cancels the old result and keeps the new preview', async ({ page }) => {
  test.setTimeout(50_000);
  await openStandalone(page);
  await holdFirstBatch(page);
  await page.waitForFunction(() => __captureRegression.pendingResults > 0, null, { timeout: 20_000 });
  await expectDecodedPreview(page);
  await page.locator('#liveRetakeBtn').click();
  await expect(page.locator('#liveCameraView')).toBeVisible();
  await expect(page.locator('#liveScanner')).toHaveAttribute('data-state', 'review', { timeout: 20_000 });
  await expectDecodedPreview(page);
  // Review now becomes available after the first verified photo. Let the
  // current backups finish before checking that stale prior results do not
  // change it; a better current backup may legitimately become the preview.
  await expect.poll(() => page.evaluate(() => __captureRegression.completedResults.filter(call => call > 3).length),
    {timeout:20000}).toBe(3);
  const before = await page.locator('#livePreview').getAttribute('src');
  await page.evaluate(() => __captureRegression.releaseResults());
  await expect.poll(() => page.evaluate(() => __captureRegression.completedResults.length)).toBe(6);
  await expect(page.locator('#liveScanner')).toHaveAttribute('data-state', 'review');
  await expect(page.locator('#livePreview')).toHaveAttribute('src', before);
  await expect(page.locator('#liveAddBtn')).toBeEnabled();
  expect(await page.evaluate(() => __captureRegression.streams.flatMap(stream => stream.getTracks().map(track => track.readyState))))
    .toEqual(['ended', 'ended']);
});


test('a verified scan can be added while slower backup photos are still processing', async ({ page }) => {
  test.setTimeout(60000);
  await openStandalone(page);
  await page.evaluate(() => {
    const original=ScannerPro.Lightweight.processPageAsync;
    let calls=0;
    const gate=new Promise(resolve=>{__captureRegression.releaseBackups=resolve;});
    ScannerPro.Lightweight.processPageAsync=async function(...args) {
      const index=++calls, result=await original.apply(this,args);
      if(index>1) await gate;
      return result;
    };
  });
  await expect(page.locator('#liveAddBtn')).toBeEnabled({timeout:20000});
  await expect(page.locator('#liveReviewStatus')).toHaveText('Ready');
  await expectDecodedPreview(page);
  await page.evaluate(()=>__captureRegression.releaseBackups());
  await expect(page.locator('#liveReviewStatus')).toHaveText('Ready',{timeout:10000});
});
