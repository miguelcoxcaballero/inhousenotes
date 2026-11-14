// internal-doc (BUILD-PLAN W3.1, e2e-internal "doc.selftest"): a v6 container opened through the shared link
// `/?drive=<id>` against the real host — Drive download, host/pdf.js (pdf.js open, Keywords, attachments, page
// sizes, page raster), inflate, snapshot merge, IndexedDB persistence — then a reload that must give the same state
// from the local copy alone. Also: the viewer banner, the editor menu's statistics, the save indicator states, the
// write path, a new document with a reserved id, and a failing open.
//
// The fixture is built here: `doc.session.fixture` returns the snapshot of a small deterministic document, which
// this file wraps into a PDF by hand (Keywords `STROKES_Z:!;IHN6:<base64 zlib snapshot>` and one attachment
// `IHN6-<hash>`), exactly the container of ARCHITECTURE §8.2. The PDF export itself is W5.4's.
import zlib from 'node:zlib';
import { test, expect, seedSession, ANA, BEN } from '../mock/fixtures.mjs';

test.use({ serviceWorkers: 'block' });

const ASSET = '6a'.repeat(32);
// A 1×1 PNG: the bytes of the photo asset.
const PNG = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==', 'base64');

/** A three-page PDF with `/Keywords` and embedded files, written object by object. */
function containerPdf({ keywords, attachments = [], pages = 3 }) {
  const chunks = [Buffer.from('%PDF-1.7\n%\xe2\xe3\xcf\xd3\n', 'latin1')], offsets = [];
  let size = chunks[0].length;
  const push = buffer => { chunks.push(buffer); size += buffer.length; };
  const object = (number, body, stream) => {
    offsets[number] = size;
    push(Buffer.from(`${number} 0 obj\n${body}\n`, 'latin1'));
    if (stream) { push(Buffer.from('stream\n', 'latin1')); push(stream); push(Buffer.from('\nendstream\n', 'latin1')); }
    push(Buffer.from('endobj\n', 'latin1'));
  };
  const first = 4, files = first + pages;
  const names = attachments.map((a, i) => `(${a.name}) ${files + i * 2} 0 R`).join(' ');
  object(1, `<< /Type /Catalog /Pages 2 0 R${attachments.length ? ` /Names << /EmbeddedFiles << /Names [${names}] >> >>` : ''} >>`);
  object(2, `<< /Type /Pages /Kids [${Array.from({ length: pages }, (_, i) => `${first + i} 0 R`).join(' ')}] /Count ${pages} >>`);
  object(3, `<< /Title (Cuaderno Digital) /Subject (Notas con S-Pen) /Creator (Cuaderno Digital App) /Keywords (${keywords}) >>`);
  for (let i = 0; i < pages; i++) object(first + i, '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595.5 842.25] >>');
  attachments.forEach((a, i) => {
    object(files + i * 2, `<< /Type /Filespec /F (${a.name}) /UF (${a.name}) /EF << /F ${files + i * 2 + 1} 0 R >> >>`);
    object(files + i * 2 + 1, `<< /Type /EmbeddedFile /Length ${a.data.length} >>`, a.data);
  });
  const count = files + attachments.length * 2, xref = size;
  let table = `xref\n0 ${count}\n0000000000 65535 f \n`;
  for (let n = 1; n < count; n++) table += `${String(offsets[n]).padStart(10, '0')} 00000 n \n`;
  push(Buffer.from(`${table}trailer\n<< /Size ${count} /Root 1 0 R /Info 3 0 R >>\nstartxref\n${xref}\n%%EOF\n`, 'latin1'));
  return Buffer.concat(chunks);
}

const cmd = (page, name, args) => page.evaluate(([n, a]) => window.__IHN_TEST_API__.cmd(n, a), [name, args]);
async function open(page, url) {
  await page.goto(url);
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  expect(await page.evaluate(() => window.__IHN_TEST_API__.ready())).toBe(true);
}
const opened = (page, id, ok = true) => expect.poll(async () => (await cmd(page, 'doc.open.state')).log.some(e => e[0] === id && e[1] === ok), { timeout: 15_000 }).toBe(true);
const settled = page => expect.poll(async () => { const s = await cmd(page, 'doc.open.state'); return !s.running; }, { timeout: 15_000 }).toBe(true);
const media = mock => mock.calls.filter(call => /alt=media/.test(call.url)).length;
/** The fixture document as a v6 container in Ana's Drive. */
async function seedContainer(page, mock, { id = 'v6-doc', permissions = [] } = {}) {
  await open(page, '/?e2e=1');
  const snapshot = Buffer.from(await cmd(page, 'doc.session.fixture'), 'base64');
  const data = containerPdf({ keywords: `STROKES_Z:!;IHN6:${zlib.deflateSync(snapshot).toString('base64')}`, attachments: [{ name: `IHN6-${ASSET}`, data: PNG }] });
  mock.drive.addFile({ id, name: 'Fixture.pdf', owner: ANA, data, permissions, modifiedTime: '2026-10-29T09:00:00.000Z' });
  return { snapshot, size: data.length };
}

test('doc.selftest: a v6 container opens through the shared link, and a reload restores the same state locally', async ({ page, context, mock, problems }) => {
  await seedSession(context, mock);
  const { snapshot, size } = await seedContainer(page, mock);

  await open(page, '/?drive=v6-doc&e2e=1');
  await opened(page, 'v6-doc');
  await settled(page);
  await expect(page.locator('#editor-view')).toBeVisible();
  await expect(page.locator('#loading-overlay')).not.toHaveClass(/visible/);
  const first = await cmd(page, 'doc.session.state');
  expect(first).toMatchObject({ docId: 'v6-doc', account: ANA, pages: 3, items: 3, live: 3, name: 'Fixture', canEdit: true, writable: true, persistent: true, netHolder: true, assets: 1, missing: 0, failed: false });
  expect(first.snapshotBytes).toBe(snapshot.length);
  expect(media(mock)).toBe(1);
  expect((await cmd(page, 'doc.links.state')).pending).toBe('');
  await expect(page.locator('#readonly-banner')).toHaveClass(/hidden/);
  // Everything is in Drive and nothing was edited: saved.
  expect(await cmd(page, 'doc.save_state.state')).toMatchObject({ phase: 'saved', title: 'Saved in Google Drive', shortLabel: 'Saved', badge: '' });
  await expect(page.locator('#save-indicator')).toHaveClass(/saved/);
  // The editor menu shows the statistics of the open document, with the size of the Drive file.
  await page.click('#btn-drive-profile-editor');
  const kb = (size / 1024).toFixed(1);
  await expect(page.locator('#doc-stats')).toHaveText(`3 pages · 2 strokes · ${kb} KB`);
  await page.keyboard.press('Escape');

  // The executor's remaining ops against the real pdf.js: page sizes, a page raster, the attachment bytes.
  const probe = await page.evaluate(async png => {
    const host = await new Promise(resolve => { const t = setInterval(() => { if (window.pdfjsLib) { clearInterval(t); resolve(true); } }, 10); });
    const stored = await new Promise((resolve, reject) => {
      const request = indexedDB.open('inhouse-notes-v6');
      request.onerror = () => reject(request.error);
      request.onsuccess = () => {
        const db = request.result, tx = db.transaction(['assets', 'chunks', 'docs', 'ops']);
        const out = {};
        tx.objectStore('assets').getAll().onsuccess = e => { out.assets = e.target.result; };
        tx.objectStore('chunks').getAllKeys().onsuccess = e => { out.chunks = e.target.result; };
        tx.objectStore('docs').getAll().onsuccess = e => { out.docs = e.target.result; };
        tx.objectStore('ops').count().onsuccess = e => { out.ops = e.target.result; };
        tx.oncomplete = () => { db.close(); resolve(out); };
      };
    });
    const asset = stored.assets[0];
    const bytes = Array.from(new Uint8Array(await asset.blob.arrayBuffer()));
    return { host, bytes, expected: Array.from(atob(png), c => c.charCodeAt(0)), refs: asset.refs, chunks: stored.chunks, doc: stored.docs[0], ops: stored.ops };
  }, PNG.toString('base64'));
  expect(probe.bytes).toEqual(probe.expected);
  expect(probe.refs).toBe(1);
  expect(probe.chunks.map(key => key[1])).toEqual([0, 1, 2]);   // header + the two pages that have items
  expect(probe.ops).toBe(0);
  expect(probe.doc).toMatchObject({ account: ANA, name: 'Fixture.pdf', flags: { created: true }, driveMeta: { canEdit: true, ownedByMe: true, ownerEmail: ANA } });
  expect(probe.doc.exports).toHaveLength(1);

  // Reload: the same canonical state from IndexedDB, shown without downloading the PDF again.
  await open(page, '/?drive=v6-doc&e2e=1');
  await opened(page, 'v6-doc');
  await settled(page);
  const second = await cmd(page, 'doc.session.state');
  expect(second).toEqual(first);
  expect(media(mock)).toBe(1);
  expect(await cmd(page, 'doc.save_state.state')).toMatchObject({ phase: 'saved' });

  // The write path against the real IndexedDB: journalled, then "waiting to upload" (nothing seals it yet).
  expect(await cmd(page, 'doc.session.stroke')).toBe(true);
  await expect.poll(async () => (await cmd(page, 'doc.session.state')).uncommitted).toBe(0);
  const edited = await cmd(page, 'doc.session.state');
  expect(edited).toMatchObject({ items: 4, journal: 1, failed: false });
  expect(await cmd(page, 'doc.save_state.state')).toMatchObject({ phase: 'paused', title: 'Waiting to save to Drive', badge: 'Waiting to upload', local: 'Saved', drive: 'Waiting' });
  await expect(page.locator('#save-indicator')).toHaveClass(/paused/);
  // The save-details popup: 5.18.0's markup, rows and button.
  await page.click('.mode-toggle-status');
  await expect(page.locator('.save-debug-backdrop')).toHaveClass(/visible/);
  await expect(page.locator('.save-device-status-label')).toHaveText(['Saved', 'Waiting']);
  await expect(page.locator('.save-debug-action')).toHaveText('Save now');
  await page.keyboard.press('Escape');
  await expect(page.locator('.save-debug-backdrop')).not.toHaveClass(/visible/);
  // A third load replays the journal over the chunks.
  await open(page, '/?drive=v6-doc&e2e=1');
  await opened(page, 'v6-doc');
  await settled(page);
  const third = await cmd(page, 'doc.session.state');
  expect(third).toMatchObject({ items: 4, journal: 1, snapshot: edited.snapshot, vv: edited.vv });
  expect(media(mock)).toBe(1);
  expect(problems).toEqual([]);
});

test('host/pdf.js: every op against the real pdf.js, and the limits', async ({ page, context, mock, problems }) => {
  await seedSession(context, mock);
  await seedContainer(page, mock);
  const probe = async id => {
    expect(await cmd(page, 'doc.open.pdf_probe', { id })).toBe(true);
    await expect.poll(() => cmd(page, 'doc.open.pdf_probe_result'), { timeout: 15_000 }).not.toBeNull();
    return cmd(page, 'doc.open.pdf_probe_result');
  };
  // open → pages → render → attachment → close, on the container of the first test.
  expect(await probe('v6-doc')).toEqual({ pages: 3, kind: 0, attachments: 1, sizes: [595.5, 842.25, 595.5, 842.25, 595.5, 842.25], raster: [true, 297, 421], attachmentSize: PNG.length });
  expect(await page.evaluate(() => window.pdfjsLib.GlobalWorkerOptions.workerSrc)).toContain('vendor/pdfjs/pdf.worker.min.js');
  // Not a PDF: pdf.js refuses it and the executor reports the failure.
  mock.drive.addFile({ id: 'not-pdf', name: 'x.pdf', owner: ANA, data: Buffer.from('hello') });
  expect(await probe('not-pdf')).toMatchObject({ stage: 2, error: -1 });
  // An unexpected content type is refused before pdf.js sees it (the security-core blob rule).
  mock.drive.addFile({ id: 'an-image', name: 'x.png', mimeType: 'image/png', owner: ANA, data: PNG });
  expect(await probe('an-image')).toMatchObject({ stage: 2, error: -2, message: 'This PDF is larger than the 64 MB safety limit.' });
  expect(problems).toEqual([]);
});

test('a viewer gets the read-only banner; a broken container fails the open with 5.18.0\'s message', async ({ page, context, mock, problems }) => {
  await seedSession(context, mock, BEN);
  await seedContainer(page, mock, { permissions: [{ type: 'user', role: 'reader', emailAddress: BEN }] });
  mock.drive.addFile({ id: 'broken', name: 'Broken.pdf', owner: ANA, permissions: [{ type: 'user', role: 'writer', emailAddress: BEN }], data: containerPdf({ keywords: 'STROKES_Z:!;IHN6:AAAA' }) });

  await open(page, '/?drive=v6-doc&e2e=1');
  await opened(page, 'v6-doc');
  await settled(page);
  expect(await cmd(page, 'doc.session.state')).toMatchObject({ canEdit: false, writable: false, pages: 3 });
  await expect(page.locator('#readonly-banner')).not.toHaveClass(/hidden/);
  await expect(page.locator('#readonly-banner-message')).toHaveText("View Only You don't have permission to edit this file.");
  await expect(page.locator('#mode-toggle-btn')).toBeHidden();
  expect(await cmd(page, 'doc.session.stroke')).toBe(false);
  expect(await cmd(page, 'doc.save_state.state')).toMatchObject({ phase: 'saved', shortLabel: 'Local', drive: 'Not connected' });

  await open(page, '/?drive=broken&e2e=1');
  await opened(page, 'broken', false);
  await expect(page.locator('#drive-home')).toBeVisible();
  await expect(page.locator('#drive-status')).toHaveText('Could not open this document. Try again.');
  expect((await cmd(page, 'doc.session.state')).docId).toBe('');
  expect(problems).toEqual([]);
});

test('a new document takes a reserved Drive id, is writable at once and survives a reload before it exists in Drive', async ({ page, context, mock, problems }) => {
  await seedSession(context, mock);
  await open(page, '/?e2e=1');
  expect(await cmd(page, 'doc.create.new', { folder: '', name: '  Plan  de viaje.pdf ' })).toBe(true);
  await expect.poll(async () => (await cmd(page, 'doc.session.state')).pages).toBe(1);
  await expect.poll(async () => (await cmd(page, 'doc.session.state')).uncommitted).toBe(0);
  const created = await cmd(page, 'doc.session.state');
  expect(created).toMatchObject({ name: 'Plan de viaje', writable: true, persistent: true, journal: 1 });
  expect(created.docId).toMatch(/^1MockGen/);
  expect(mock.calls.filter(call => /generateIds/.test(call.url))).toHaveLength(1);
  expect(mock.drive.file(created.docId)).toBeUndefined();
  await expect(page.locator('#editor-view')).toBeVisible();
  // Not in Drive yet: waiting, never "saved".
  expect(await cmd(page, 'doc.save_state.state')).toMatchObject({ phase: 'paused', badge: 'Waiting to upload' });
  // The rest of the pool is kept for the next creations.
  const pool = await page.evaluate(account => new Promise(resolve => {
    const request = indexedDB.open('inhouse-notes-v6');
    request.onsuccess = () => { const get = request.result.transaction('kv').objectStore('kv').get([account, 'idpool']); get.onsuccess = () => { request.result.close(); resolve(get.result); }; };
  }), ANA);
  expect(pool).toHaveLength(19);
  expect(pool).not.toContain(created.docId);

  // Reopened from the local copy (Drive answers 404 for the id).
  await open(page, '/?e2e=1');
  expect(await cmd(page, 'doc.open.open', { id: created.docId, name: 'Plan de viaje.pdf' })).toBe(true);
  await opened(page, created.docId);
  await settled(page);
  expect(await cmd(page, 'doc.session.state')).toMatchObject({ docId: created.docId, pages: 1, name: 'Plan de viaje', snapshot: created.snapshot, writable: true });
  expect(problems).toEqual([]);
});
