// 13-migrate (TESTING §2 row 13 and §5, ARCHITECTURE §10, BUILD-PLAN W5.6): every 5.18.0 storage dump is restored
// into a fresh context and v6 boots on it. Every unsynced stroke must be in the v6 document and reach Drive (a fresh
// device opens it), `local-` documents are created with a reserved id, pending folders are created, and the 5.x
// databases are deleted only once every migration batch is sealed and no 5.x window is open. Also: a crash in the
// middle resumes; a 5.x page that keeps writing after the migration gets its strokes migrated before the deletion;
// a deletion blocked by an open 5.x connection is retried at the next boot.
import { test, expect, seedSession, addScene, storageDumps, storageDump, storageExpect } from '../mock/fixtures.mjs';
import { restoreStorage } from '../mock/fixtures.mjs';

test.use({ viewport: { width: 1280, height: 800 } });
test.describe.configure({ timeout: 180_000 });

const V5_DATABASES = ['inhouse-offline-v1', 'notebook-data-db-v1', 'background-notebook-data-db-v1', 'inhouse-original-pdfs-v1'];
const V5_KEYS = ['drive-session-v1', 'drive-pending-exit-upload-v1', 'notebook-data-v3', 'background-notebook-data-v3'];
const cmd = (page, name, args) => page.evaluate(([n, a]) => window.__IHN_TEST_API__.cmd(n, a), [name, args]);
const migration = page => cmd(page, 'migrate.state');
const databases = page => page.evaluate(async () => (await indexedDB.databases()).map(d => d.name).sort());

/** Restores a dump (its databases and its 5.x localStorage keys) into the origin, from a page that runs no app. */
async function seed(page, name) {
  const dump = storageDump(name);
  await page.goto('/icons/inhouse-notes-32.png');
  const localStorage = Object.fromEntries(Object.entries(dump.localStorage || {}).filter(([k]) => V5_KEYS.includes(k)));
  await restoreStorage(page, { indexedDB: dump.indexedDB, localStorage });
}

function scene(mock, e) {
  if (e.document.driveFixture) addScene(mock, e.document.driveFixture, { id: e.document.id, title: e.document.id === 'scene-r0' ? 'r0.pdf' : undefined });
}

async function boot(page) {
  await page.goto('/?e2e=1');
  await page.waitForFunction(() => window.__IHN_TEST_API__);
}

const named = (mock, name) => [...mock.drive.files.values()].filter(f => f.name === name && !f.trashed).length;
const v6Id = (state, e) => (state.docs.find(d => d[0] === e.document.id) || [])[1];

/** Opens a document in the editor and returns the expected stroke ids it lacks. */
async function missing(page, id, e) {
  await page.goto(`/?drive=${id}&e2e=1`);
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  await expect.poll(async () => (await cmd(page, 'doc.open.state')).log.some(l => l[0] === id && l[1]), { timeout: 30_000 }).toBe(true);
  // A fresh device may get the newest strokes from the relay a moment after the open.
  const ids = e.document.pages.flatMap(p => p.strokeIds);
  let left = [];
  for (let i = 0; i < 60; i++) {
    left = await cmd(page, 'migrate.has', { ids });
    if (!left.length) break;
    await page.waitForTimeout(500);
  }
  return left;
}

for (const name of storageDumps()) {
  test(`${name}: migrated, sealed, readable on a fresh device, then the 5.x data is deleted`, async ({ page, context, browser, mock, problems }) => {
    const e = storageExpect(name);
    scene(mock, e);
    await seedSession(context, mock);
    await seed(page, name);
    await boot(page);
    await expect.poll(async () => (await migration(page)).deleted, { timeout: 120_000 }).toBe(true);
    const state = await migration(page);
    expect(state.errors).toEqual([]);
    expect(await databases(page)).toEqual(expect.not.arrayContaining(V5_DATABASES));
    expect(await page.evaluate(keys => keys.filter(k => localStorage.getItem(k) !== null), V5_KEYS)).toEqual([]);

    if (!e.document.id) {
      // A pending local folder: created in Drive once.
      expect(named(mock, 'Offline folder')).toBe(1);
      expect(state.folders.length).toBe(1);
      expect(mock.drive.file(state.folders[0][1]).name).toBe('Offline folder');
      expect(problems).toEqual([]);
      return;
    }
    const id = v6Id(state, e);
    expect(id).toBeTruthy();
    if (e.document.id.startsWith('local-')) {
      // A reserved id, created once in Drive.
      expect(id).not.toBe(e.document.id);
      expect(mock.drive.file(id)?.name).toBe(`${e.document.name}.pdf`);
      expect(named(mock, `${e.document.name}.pdf`)).toBe(1);
    } else {
      expect(id).toBe(e.document.id);
    }
    expect(await missing(page, id, e)).toEqual([]);

    // A fresh device: what this one migrated reached Drive.
    const fresh = await browser.newContext({ serviceWorkers: 'block', viewport: { width: 1280, height: 800 } });
    await mock.install(fresh);
    await seedSession(fresh, mock);
    const other = await fresh.newPage();
    expect(await missing(other, id, e)).toEqual([]);
    await fresh.close();
    expect(problems).toEqual([]);
  });
}

test('a crash in the middle of the migration resumes to the same result', async ({ page, context, mock, problems }) => {
  const e = storageExpect('offline-dirty');
  scene(mock, e);
  await seedSession(context, mock);
  await seed(page, 'offline-dirty');
  await boot(page);
  await expect.poll(async () => { const s = await migration(page); return s.busy || s.docs.length > 0; }, { timeout: 30_000 }).toBe(true);
  await page.close({ runBeforeUnload: false });
  const again = await context.newPage();
  await boot(again);
  await expect.poll(async () => (await migration(again)).deleted, { timeout: 120_000 }).toBe(true);
  expect(await missing(again, e.document.id, e)).toEqual([]);
  // Nothing was imported twice: the live strokes are exactly the expected ones.
  expect((await cmd(again, 'doc.session.state')).live).toBe(e.document.pages.reduce((n, p) => n + p.strokeIds.length, 0));
  expect(problems).toEqual([]);
});

test('a 5.x page still open keeps writing: its strokes are migrated before the 5.x data is deleted', async ({ page, context, mock, problems }) => {
  const e = storageExpect('stroke-ops-pending');
  scene(mock, e);
  await seedSession(context, mock);
  await seed(page, 'stroke-ops-pending');
  // A window that never announces itself as v6 (as a 5.18.0 tab).
  const old = await context.newPage();
  await old.goto('/icons/inhouse-notes-32.png');
  await boot(page);
  await expect.poll(async () => { const s = await migration(page); return s.written && s.phase; }, { timeout: 90_000 }).toMatch(/Check|Census/);
  await page.waitForTimeout(6_000);
  expect((await migration(page)).deleted).toBe(false);
  // The 5.x page writes one more stroke into its working cache.
  await old.evaluate(() => new Promise((resolve, reject) => {
    const open = indexedDB.open('notebook-data-db-v1');
    open.onsuccess = () => {
      const db = open.result, tx = db.transaction('stroke-ops', 'readwrite'), store = tx.objectStore('stroke-ops');
      const all = store.getAll();
      all.onsuccess = () => {
        const last = all.result.at(-1);
        const row = { ...last, savedAt: Date.now(), stroke: { ...last.stroke, id: 'late-stroke-1', points: last.stroke.points.map(p => ({ ...p, x: p.x + 30 })) } };
        delete row.id;
        store.add(row);
      };
      tx.oncomplete = () => { db.close(); resolve(); };
      tx.onerror = () => reject(tx.error);
    };
    open.onerror = () => reject(open.error);
  }));
  await old.close();
  await expect.poll(async () => (await migration(page)).deleted, { timeout: 120_000 }).toBe(true);
  const want = { ...e, document: { ...e.document, pages: [{ strokeIds: ['late-stroke-1', ...e.unsyncedStrokeIds] }] } };
  expect(await missing(page, e.document.id, want)).toEqual([]);
  expect(problems).toEqual([]);
});

test('a deletion blocked by an open 5.x connection is retried at the next boot', async ({ page, context, mock, problems }) => {
  const e = storageExpect('pending-exit');
  scene(mock, e);
  await seedSession(context, mock);
  await seed(page, 'pending-exit');
  // The first boot keeps a 5.x connection open (it ignores `versionchange`, like a busy 5.x tab).
  await context.addInitScript(() => {
    if (location.pathname !== '/' || sessionStorage.getItem('held')) return;
    sessionStorage.setItem('held', '1');
    const open = indexedDB.open('inhouse-original-pdfs-v1');
    open.onsuccess = () => { window.__held = open.result; };
  });
  await boot(page);
  await expect.poll(async () => (await migration(page)).phase, { timeout: 120_000 }).toBe('Blocked');
  expect((await migration(page)).deleted).toBe(false);
  await boot(page);
  await expect.poll(async () => (await migration(page)).deleted, { timeout: 60_000 }).toBe(true);
  expect(await databases(page)).toEqual(expect.not.arrayContaining(V5_DATABASES));
  expect(await missing(page, e.document.id, e)).toEqual([]);
  expect(problems).toEqual([]);
});
