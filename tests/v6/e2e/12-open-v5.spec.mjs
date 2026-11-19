// 12-open-v5 (BUILD-PLAN W3.4): every 5.x Drive fixture opens through `/?drive=` and gives the document 5.18.0
// reported after importing it (`*.expect.json`: name, pages, sizes, templates, backgrounds, stroke ids in draw
// order, images, cover strokes); a plain PDF becomes PDF pages; a PDF over the 64 MB limit fails like 5.18.0; a
// later 5.x revision is merged (R1 deletes strokes without tombstones); a plain PDF replacing an open v6 document
// keeps the document and records the recovered copy; large Keywords are parsed in the export worker and the main
// instance's memory does not grow by the legacy JSON.
import { test, expect, seedSession, addScene, sceneId, v5Fixture, v5Fixtures, oversizedPdf, ANA } from '../mock/fixtures.mjs';

test.use({ serviceWorkers: 'block' });

const cmd = (page, name, args) => page.evaluate(([n, a]) => window.__IHN_TEST_API__.cmd(n, a), [name, args]);
async function load(page, url) {
  await page.goto(url);
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  expect(await page.evaluate(() => window.__IHN_TEST_API__.ready())).toBe(true);
}
const outcome = (page, id) => expect.poll(async () => {
  const log = (await cmd(page, 'doc.open.state')).log.filter(e => e[0] === id);
  return log.length ? log.at(-1)[1] : null;
}, { timeout: 20_000 });
const settled = page => expect.poll(async () => !(await cmd(page, 'doc.open.state')).running && !(await cmd(page, 'doc.v5open.state')).busy, { timeout: 20_000 }).toBe(true);
const legacy = async (page, ids) => ids.length ? cmd(page, 'doc.v5open.legacy', ids) : [];

/** The open document compared with what 5.18.0 reported. `exactIds` = false for a document whose ids 5.18.0 made up
 * (no `IH_NORM`, no page ids: random or keyed by the import, not by the Drive file). */
async function expectDocument(page, expected, { exactIds = true, label = expected.exportName } = {}) {
  const doc = await cmd(page, 'doc.v5open.doc');
  expect(doc.name, label).toBe(expected.exportName);
  expect(doc.pages.length, label).toBe(expected.pageCount);
  expect(doc.strokes, label).toBe(expected.pages.reduce((n, p) => n + p.strokes.length, 0) + expected.pages.reduce((n, p) => n + (p.legacyCoverStrokes?.length || 0), 0));
  expect(doc.pages.reduce((n, p) => n + p.images, 0)).toBe(expected.imageCount);
  for (const [i, want] of expected.pages.entries()) {
    const got = doc.pages[i], at = `${label} page ${i}`;
    if (exactIds) expect(got.id, at).toBe((await legacy(page, [want.pageId]))[0]);
    expect([got.w, got.h], at).toEqual([want.pageWidth, want.pageHeight]);
    expect(got.template.toLowerCase(), at).toBe(want.templateKind || 'default');
    expect(got.bg, at).toBe(want.backgroundSource === 'template' ? 'template' : `pdf:${want.pdfPageIndex}`);
    expect(got.asset, at).toBe(true);
    expect(got.images, at).toBe(want.images.length);
    expect(got.covers, at).toBe(want.legacyCoverStrokes?.length || 0);
    if (exactIds) expect(got.strokes, at).toEqual(await legacy(page, want.strokes.map(s => s[0])));
    else expect(got.strokes.length, at).toBe(want.strokes.length);
  }
}

test('every 5.x fixture opens with the document 5.18.0 imported', async ({ page, context, mock, problems }) => {
  test.setTimeout(180_000);
  await seedSession(context, mock);
  for (const name of v5Fixtures()) {
    const { expect: expected } = v5Fixture(name);
    // Without `IH_FIELDS` the name is the Drive file's, as when 5.18.0 imported the fixture.
    addScene(mock, name, expected.keywordSections.includes('IH_FIELDS') ? {} : { title: `${expected.exportName}.pdf` });
    await load(page, `/?drive=${sceneId(name)}&e2e=1`);
    await outcome(page, sceneId(name)).toBe(true);
    await settled(page);
    await expect(page.locator('#editor-view')).toBeVisible();
    await expectDocument(page, expected, { exactIds: !['legacy-no-norm', 'plain'].includes(name), label: name });
    const kind = (await cmd(page, 'doc.v5open.state')).log.filter(e => e[0] === sceneId(name)).map(e => e[1]);
    expect(kind.find(k => /^(v5|plain):/.test(k))).toBe(`${name === 'plain' ? 'plain' : 'v5'}:${expected.pageCount}`);
    // The pages show the clean original: overlays stripped by pdf-lib in the worker, or the attachment.
    const source = { 'imported-overlays': 'source:Stripped', 'photo-page': 'source:Stripped', 'structure-changed': 'source:Saved', 'raster-cover': 'source:Saved', plain: 'source:Saved' }[name];
    if (source) expect(kind.at(-1), name).toBe(source);
  }
  expect(problems).toEqual([]);
});

test('large Keywords are parsed in the export worker; the main instance does not hold the legacy JSON', async ({ page, context, mock, problems }) => {
  await seedSession(context, mock);
  addScene(mock, 'dense-20');
  addScene(mock, 'r0');
  // A small document first: the worker is used only for the large one.
  await load(page, `/?drive=${sceneId('r0')}&e2e=1`);
  await outcome(page, sceneId('r0')).toBe(true);
  await settled(page);
  await cmd(page, 'doc.open.open', { id: sceneId('dense-20'), name: 'dense-20.pdf' });
  await outcome(page, sceneId('dense-20')).toBe(true);
  await settled(page);
  const state = await cmd(page, 'doc.v5open.state');
  expect(state.log.filter(e => e[0] === sceneId('dense-20')).map(e => e[1])).toEqual(['worker', 'v5:20']);
  await expectDocument(page, v5Fixture('dense-20').expect);
  const after = await page.evaluate(() => window.__IHN_TEST_API__.wasmMemoryBytes());
  // The same document reopened from the local copy (no import): the import's own transient memory is what is left.
  await load(page, `/?drive=${sceneId('dense-20')}&e2e=1`);
  await outcome(page, sceneId('dense-20')).toBe(true);
  await settled(page);
  const local = await page.evaluate(() => window.__IHN_TEST_API__.wasmMemoryBytes());
  expect(after - local).toBeLessThan(16 << 20);
  expect(problems).toEqual([]);
});

test('a later 5.x revision is merged: R1 wins and its deletions apply', async ({ page, context, mock, problems }) => {
  await seedSession(context, mock);
  addScene(mock, 'r0');
  await load(page, `/?drive=${sceneId('r0')}&e2e=1`);
  await outcome(page, sceneId('r0')).toBe(true);
  await settled(page);
  await expectDocument(page, v5Fixture('r0').expect);
  // A 5.x device saves R1; the next open revalidates the local copy, downloads the new head and merges it.
  mock.drive.handle({ method: 'PATCH', url: `https://www.googleapis.com/upload/drive/v3/files/${sceneId('r0')}?uploadType=media`, headers: { authorization: `Bearer ${mock.drive.accounts.get(ANA).token}`, 'content-type': 'application/pdf' }, body: v5Fixture('r1').pdf });
  await load(page, `/?drive=${sceneId('r0')}&e2e=1`);
  await expect.poll(async () => (await cmd(page, 'doc.v5open.state')).log.filter(e => e[0] === sceneId('r0')).length, { timeout: 20_000 }).toBe(1);
  await settled(page);
  await expectDocument(page, v5Fixture('r1').expect);
  expect(problems).toEqual([]);
});

test('a plain PDF replacing an open document keeps it and records the recovered copy', async ({ page, context, mock, problems }) => {
  await seedSession(context, mock);
  addScene(mock, 'r0', { title: 'Revisions.pdf' });
  await load(page, `/?drive=${sceneId('r0')}&e2e=1`);
  await outcome(page, sceneId('r0')).toBe(true);
  await settled(page);
  mock.drive.handle({ method: 'PATCH', url: `https://www.googleapis.com/upload/drive/v3/files/${sceneId('r0')}?uploadType=media`, headers: { authorization: `Bearer ${mock.drive.accounts.get(ANA).token}`, 'content-type': 'application/pdf' }, body: v5Fixture('plain').pdf });
  await load(page, `/?drive=${sceneId('r0')}&e2e=1`);
  await expect.poll(async () => (await cmd(page, 'doc.v5open.state')).recoveries, { timeout: 20_000 }).toEqual([[sceneId('r0'), 'Revisions (recovered Drive version).pdf', mock.drive.file(sceneId('r0')).revisions.at(-1).id]]);
  await settled(page);
  await expectDocument(page, v5Fixture('r0').expect);
  expect(problems).toEqual([]);
});

test('a PDF over the 64 MB safety limit fails like 5.18.0', async ({ page, context, mock, problems }) => {
  test.setTimeout(60_000);
  await seedSession(context, mock);
  mock.drive.addFile({ id: 'huge', name: 'Huge.pdf', owner: ANA, data: oversizedPdf(), modifiedTime: '2026-10-29T09:00:00.000Z' });
  await load(page, '/?drive=huge&e2e=1');
  await outcome(page, 'huge').toBe(false);
  await expect(page.locator('#drive-status')).toHaveText('Could not open this document. Try again.');
  await expect(page.locator('#drive-status')).toBeVisible();
  expect(problems).toEqual([]);
});
