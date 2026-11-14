// 08-save-reopen (TESTING §2 row 08, BUILD-PLAN W5.4): write, Home → the device alone exports; a fresh device
// (no local data) opens the export → identical snapshot; the uploaded PDF is a v6 container (fence + IHN6 Keywords,
// image attachments `IHN6-<hex>`, no source-PDF attachment, `/IHN6Ink` streams); a PDF-background page's source is
// rebuilt from the export; a 412 with an unchanged head re-sends the same bytes, a 412 with a new head merges (a
// foreign plain PDF is saved as the recovered sibling copy first); export to the user: web `<a download>` with
// `sanitizeFilename`, Android bridge v2 (chunks ≤ 240,000 chars, `handleInhousePdfExportResult`), `navigator.share`.
import fs from 'node:fs';
import vm from 'node:vm';
import { test, expect, seedSession, addScene, sceneId, blankPdf, ANA } from '../mock/fixtures.mjs';

test.use({ serviceWorkers: 'block', viewport: { width: 1280, height: 800 } });
test.describe.configure({ timeout: 120_000 });

const realm = vm.createContext({});
vm.runInContext(fs.readFileSync(new URL('../../../vendor/pdf-lib/pdf-lib.min.js', import.meta.url), 'utf8'), realm);
const pdfLib = realm.PDFLib;

const cmd = (page, name, args) => page.evaluate(([n, a]) => window.__IHN_TEST_API__.cmd(n, a), [name, args]);
async function boot(page, url) {
  await page.goto(url);
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  expect(await page.evaluate(() => window.__IHN_TEST_API__.ready())).toBe(true);
}
const session = page => cmd(page, 'doc.session.state');
const collab = page => cmd(page, 'collab.state');

async function device(browser, mock, { app = false, init } = {}) {
  const context = await browser.newContext({ serviceWorkers: 'block', viewport: { width: 1280, height: 800 }, acceptDownloads: true });
  await mock.install(context);
  await seedSession(context, mock, ANA);
  if (init) await context.addInitScript(init);
  const page = await context.newPage();
  const problems = [];
  page.on('pageerror', e => problems.push(`pageerror: ${e.message}`));
  return { context, page, problems, app };
}

async function openDoc(page, id, { edit = true, app = false } = {}) {
  await boot(page, `/?drive=${id}&e2e=1${app ? '&inhouse_app=1' : ''}`);
  await expect.poll(async () => (await cmd(page, 'doc.open.state')).log.some(e => e[0] === id && e[1]), { timeout: 30_000 }).toBe(true);
  await expect.poll(async () => (await collab(page)).active, { timeout: 10_000 }).toBe(true);
  if (edit) {
    await page.click('#mode-toggle-btn');
    await expect(page.locator('#editor-view')).not.toHaveClass(/read-only/);
  }
}

async function stroke(page, points) {
  const cdp = await page.context().newCDPSession(page);
  const box = await page.locator('canvas.page-canvas[data-page="0"]').boundingBox();
  const at = p => [box.x + p[0] * box.width / 794, box.y + p[1] * box.height / 1123];
  const send = (type, p, buttons) => cdp.send('Input.dispatchMouseEvent', { type, x: at(p)[0], y: at(p)[1], button: 'left', buttons, clickCount: 1, pointerType: 'pen', force: 0.5 });
  await send('mousePressed', points[0], 1);
  for (const p of points.slice(1)) await send('mouseMoved', p, 1);
  await send('mouseReleased', points.at(-1), 0);
  await cdp.detach();
}
const line = (x, y, n = 8) => Array.from({ length: n }, (_, i) => [x + i * 14, y + (i % 2) * 7]);
const revisions = (mock, id) => mock.drive.file(id).revisions.length;
const uploads = (mock, id) => mock.calls.filter(c => c.method === 'PATCH' && c.path.includes(`/upload/drive/v3/files/${id}`) && /uploadType=multipart/.test(c.url));

/** The container facts of a PDF (ARCHITECTURE §8.2). */
async function container(bytes) {
  const { PDFDocument, PDFName, PDFArray } = pdfLib;
  const doc = await PDFDocument.load(vm.runInContext('Uint8Array', realm).from(bytes), { updateMetadata: false });
  const names = doc.catalog.lookup(PDFName.of('Names'))?.lookup(PDFName.of('EmbeddedFiles'))?.lookup(PDFName.of('Names'));
  const attachments = [];
  if (names instanceof PDFArray) for (let i = 0; i < names.size(); i += 2) attachments.push(names.lookup(i).decodeText());
  const inked = doc.getPages().map(page => {
    const contents = page.node.Contents();
    const last = contents instanceof PDFArray ? doc.context.lookup(contents.get(contents.size() - 1)) : contents;
    const text = last?.getContents ? Buffer.from(last.getContents()).toString('latin1').trim() : '';
    return text === 'q /IHN6Ink Do Q' && !!page.node.Resources().lookup(PDFName.of('XObject'))?.get(PDFName.of('IHN6Ink'));
  });
  return { keywords: doc.getKeywords() || '', title: doc.getTitle(), attachments, inked, pages: doc.getPageCount() };
}

/** Writes, goes Home and waits for the device's exit export (it is alone, SYNC §14.1). */
async function writeAndLeave(page, mock, id, points = [line(120, 240), line(140, 420)]) {
  const before = revisions(mock, id);
  for (const p of points) await stroke(page, p);
  await expect.poll(async () => (await collab(page)).unsealed, { timeout: 20_000 }).toBe(0);
  await page.click('#btn-drive-home svg');
  await expect(page.locator('#drive-home')).toBeVisible();
  await expect.poll(() => revisions(mock, id), { timeout: 30_000 }).toBeGreaterThan(before);
}

test('write, Home, reopen on a fresh device: the export is a v6 container with the same snapshot', async ({ browser, mock, problems }) => {
  addScene(mock, 'inline-images');
  const id = sceneId('inline-images');
  const A = await device(browser, mock);
  await openDoc(A.page, id);
  await writeAndLeave(A.page, mock, id);
  const snapshot = (await session(A.page)).snapshot;
  expect((await cmd(A.page, 'export.lifecycle.state')).log[0][1]).toBe('All changes saved.');
  const pdf = await container(mock.drive.content(id));
  expect(pdf.keywords.startsWith('STROKES_Z:!;IHN6:')).toBe(true);
  expect(pdf.title).toBe('Cuaderno Digital');
  expect(pdf.attachments.length).toBeGreaterThan(0);
  expect(pdf.attachments.every(n => /^IHN6-[0-9a-f]{64}$/.test(n))).toBe(true);
  expect(pdf.inked[0]).toBe(true);
  expect(mock.drive.file(id).appProperties.ihn6).toBe('1');

  // A fresh device (clear local DB) opens the export: the same document.
  const B = await device(browser, mock);
  await openDoc(B.page, id, { edit: false });
  await expect.poll(async () => (await session(B.page)).snapshot, { timeout: 20_000 }).toBe(snapshot);
  expect((await session(B.page)).missing).toBe(0);
  // The export holds the whole document: re-importing it alone (no relay) gives the same snapshot.
  const C = await device(browser, mock);
  await C.context.route(/\/files\/[^/?]+\?.*alt=media/, route => route.request().url().includes(id) ? route.fallback() : route.abort());
  await openDoc(C.page, id, { edit: false });
  await expect.poll(async () => (await session(C.page)).snapshot, { timeout: 20_000 }).toBe(snapshot);
  expect([...A.problems, ...B.problems, ...C.problems, ...problems]).toEqual([]);
  for (const d of [A, B, C]) await d.context.close();
});

test('a PDF-background page: no source attachment, the source is rebuilt from the export and renders', async ({ browser, mock, problems }) => {
  addScene(mock, 'plain');
  const id = sceneId('plain');
  const A = await device(browser, mock);
  await openDoc(A.page, id);
  await writeAndLeave(A.page, mock, id, [line(100, 300)]);
  const pdf = await container(mock.drive.content(id));
  expect(pdf.attachments.filter(n => !/^IHN6-/.test(n))).toEqual([]);
  expect(pdf.inked[0]).toBe(true);
  const B = await device(browser, mock);
  await openDoc(B.page, id, { edit: false });
  await expect.poll(async () => (await session(B.page)).snapshot, { timeout: 20_000 }).toBe((await session(A.page)).snapshot);
  // The source is not in the export as a file: B rebuilds it from the export's pages and the page renders.
  await expect.poll(async () => (await session(B.page)).missing, { timeout: 30_000 }).toBe(0);
  await expect.poll(async () => B.page.locator('canvas.page-canvas[data-page="0"]').evaluate(w => getComputedStyle(w).backgroundImage), { timeout: 20_000 }).toMatch(/^url\("?blob:/);
  expect([...A.problems, ...B.problems, ...problems]).toEqual([]);
  await A.context.close();
  await B.context.close();
});

test('412 with an unchanged head re-sends the same bytes; 412 with a new head merges and keeps the foreign copy', async ({ browser, mock, problems }) => {
  addScene(mock, 'inline-images');
  const id = sceneId('inline-images');
  const A = await device(browser, mock);
  await openDoc(A.page, id);
  // Metadata churn: the first export PUT fails its If-Match; the head is unchanged.
  mock.inject({ method: 'PATCH', path: `/upload/drive/v3/files/${id}`, query: /uploadType=multipart/, status: 412, reason: 'conditionNotMet', times: 1 });
  await writeAndLeave(A.page, mock, id);
  const puts = uploads(mock, id);
  expect(puts.length).toBe(2);
  expect(puts[0].status).toBe(412);
  expect((await cmd(A.page, 'export.pdf.state')).log.filter(l => l[0].startsWith('Desk')).length).toBe(1);

  // A new head (a foreign app wrote a plain PDF) arrives with the 412: merged = saved as the sibling copy first.
  await openDoc(A.page, id);
  mock.inject({
    method: 'PATCH', path: `/upload/drive/v3/files/${id}`, query: /uploadType=multipart/, times: 1,
    respond: () => {
      mock.drive.handle({ method: 'PATCH', url: `https://www.googleapis.com/upload/drive/v3/files/${id}?uploadType=media`, headers: { authorization: `Bearer ${mock.drive.accounts.get(ANA).token}`, 'content-type': 'application/pdf' }, body: blankPdf() });
      return { status: 412, headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ error: { code: 412, errors: [{ reason: 'conditionNotMet' }] } }) };
    }
  });
  await writeAndLeave(A.page, mock, id, [line(300, 600)]);
  await expect.poll(() => [...mock.drive.files.values()].filter(f => f.name === 'inline-images (recovered Drive version).pdf').length, { timeout: 20_000 }).toBe(1);
  const copy = [...mock.drive.files.values()].find(f => f.name.endsWith('(recovered Drive version).pdf'));
  expect(Buffer.compare(copy.revisions.at(-1).data, blankPdf())).toBe(0);
  await expect.poll(async () => (await container(mock.drive.content(id))).keywords.startsWith('STROKES_Z:!;IHN6:'), { timeout: 30_000 }).toBe(true);
  expect((await cmd(A.page, 'export.upload.state')).log).toEqual(expect.arrayContaining(['merge:Plain', 'copy:200']));

  // A foreign plain PDF found when the document opens (`doc::v5open` keeps the v6 state and records it): the copy
  // is written before the next export.
  mock.drive.handle({ method: 'PATCH', url: `https://www.googleapis.com/upload/drive/v3/files/${id}?uploadType=media`, headers: { authorization: `Bearer ${mock.drive.accounts.get(ANA).token}`, 'content-type': 'application/pdf' }, body: blankPdf() });
  await openDoc(A.page, id);
  // The cached copy shows first ("Restoring document..."); the Drive check that finds the foreign head follows.
  await expect.poll(async () => (await cmd(A.page, 'doc.v5open.state')).recoveries.length, { timeout: 30_000 }).toBe(1);
  await writeAndLeave(A.page, mock, id, [line(320, 640)]);
  await expect.poll(() => [...mock.drive.files.values()].filter(f => f.name === 'inline-images (recovered Drive version).pdf').length, { timeout: 20_000 }).toBe(2);
  await expect.poll(async () => (await container(mock.drive.content(id))).keywords.startsWith('STROKES_Z:!;IHN6:'), { timeout: 30_000 }).toBe(true);
  expect([...A.problems, ...problems]).toEqual([]);
  await A.context.close();
});

test('export to the user: web download, Android bridge v2 and navigator.share', async ({ browser, mock, problems }) => {
  addScene(mock, 'inline-images');
  const id = sceneId('inline-images');
  // Web: the modal prefills the name; the confirmed name is sanitized; the head revision is downloaded as is.
  const A = await device(browser, mock);
  await openDoc(A.page, id);
  await stroke(A.page, line(120, 700));
  // The button lives in the pages panel (its own unit); its click is what matters here.
  await A.page.locator('#btn-export-pdf').dispatchEvent('click');
  await expect(A.page.locator('#modal-export-pdf')).toHaveClass(/visible/);
  await expect(A.page.locator('#export-filename')).toHaveValue((await session(A.page)).name);
  await A.page.fill('#export-filename', 'My/Notes: final.pdf');
  const download = A.page.waitForEvent('download', { timeout: 60_000 });
  await A.page.click('#modal-export-confirm');
  const file = await download;
  expect(file.suggestedFilename()).toBe('My_Notes_ final.pdf');
  const bytes = fs.readFileSync(await file.path());
  expect(Buffer.compare(bytes, mock.drive.content(id))).toBe(0);
  expect((await container(bytes)).inked.some(Boolean)).toBe(true);
  expect((await session(A.page)).name).toBe('My_Notes_ final');
  await expect(A.page.locator('#loading-overlay')).not.toHaveClass(/visible/);
  await A.context.close();

  // Android: beginPdfExport → chunks ≤ 240,000 characters → finishPdfExport → handleInhousePdfExportResult.
  const native = () => {
    window.__chunks = [];
    window.InhouseNative = {
      getAppVersion: () => '1.0.28', getAppVersionCode: () => 29, getUpdateState: () => '{}', saveUpdateState() {},
      beginPdfExport: name => { window.__begun = name; },
      appendPdfExportChunk: chunk => { window.__chunks.push(chunk); },
      finishPdfExport: () => setTimeout(() => window.handleInhousePdfExportResult(JSON.stringify({ ok: true })), 10)
    };
  };
  const B = await device(browser, mock, { init: native });
  await openDoc(B.page, id, { app: true });
  await cmd(B.page, 'export.user.export', { name: 'Android copy' });
  await expect.poll(async () => (await cmd(B.page, 'export.user.state')).log, { timeout: 60_000 }).toContain('native:Android copy.pdf');
  const got = await B.page.evaluate(() => ({ name: window.__begun, sizes: window.__chunks.map(c => c.length), data: window.__chunks.join('') }));
  expect(got.name).toBe('Android copy.pdf');
  expect(Math.max(...got.sizes)).toBeLessThanOrEqual(240000);
  expect(Buffer.compare(Buffer.from(got.data, 'base64'), mock.drive.content(id))).toBe(0);
  await B.context.close();

  // Android without the bridge: navigator.share with the PDF file.
  const share = () => {
    window.InhouseNative = { getAppVersion: () => '1.0.28', getAppVersionCode: () => 29, getUpdateState: () => '{}', saveUpdateState() {} };
    navigator.canShare = () => true;
    navigator.share = async data => { window.__shared = { name: data.files[0].name, type: data.files[0].type, title: data.title }; };
  };
  const C = await device(browser, mock, { init: share });
  await openDoc(C.page, id, { app: true });
  await cmd(C.page, 'export.user.export', { name: 'Shared' });
  await expect.poll(() => C.page.evaluate(() => window.__shared), { timeout: 60_000 }).toEqual({ name: 'Shared.pdf', type: 'application/pdf', title: 'Shared.pdf' });
  expect([...B.problems, ...C.problems, ...problems]).toEqual([]);
  await C.context.close();
});
