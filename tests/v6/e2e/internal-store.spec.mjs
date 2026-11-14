// internal-store (BUILD-PLAN W2.4, e2e-internal): `store.selftest` runs the Rust store against the real
// host/idb.js — IndexedDB transactions, the degraded signal (blocked / timed-out open), Web Locks and the read-only
// 5.x dumper over the W1.2 storage fixtures. Rust issues the requests and returns the raw completions; this file
// decodes the structured values (rust/crates/ihn-app/src/store/sv.rs) and holds the expectations.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { expect, test } from '@playwright/test';

// The 5.x storage dumps and their restore helper belong to W1.2 (tests/v6/fixtures, tests/v6/mock). Until both
// branches are merged, IHN_W12_ROOT points at a checkout that has them.
const root = process.env.IHN_W12_ROOT || fileURLToPath(new URL('../../../', import.meta.url));
const fixturesModule = path.join(root, 'tests/v6/mock/fixtures.mjs');
const fixtures = fs.existsSync(fixturesModule) ? await import(pathToFileURL(fixturesModule).href) : null;

const DOC = 'selftest-doc', ACCOUNT = 'selftest@example.test', ACTOR = '5e1f7e5700000100', REMOTE = '0badcafe00000200';
const KINDS = ['ab', 'i8', 'u8c', 'i16', 'u16', 'i32', 'u32', 'f32', 'f64', 'i64', 'u64', 'dv'];
const UNDEF = { $: 'undef' };
const u8 = (...bytes) => ({ $: 'u8', b: Buffer.from(bytes).toString('base64') });
const text = string => ({ $: 'u8', b: Buffer.from(string).toString('base64') });

// Structured values → the JSON notation of the W1.2 dumps (a Blob becomes {$:'blob', type, size}).
function decodeAll(base64) {
  const bytes = Buffer.from(base64, 'base64');
  let p = 0;
  const u32 = () => { p += 4; return bytes.readUInt32LE(p - 4); };
  const f64 = () => { p += 8; return bytes.readDoubleLE(p - 8); };
  const raw = () => { const n = u32(); p += n; return bytes.subarray(p - n, p); };
  const str = () => raw().toString('utf8');
  const num = v => (Number.isFinite(v) && !Object.is(v, -0) ? v : { $: 'num', v: Object.is(v, -0) ? '-0' : String(v) });
  const list = () => Array.from({ length: u32() }, value);
  function value() {
    switch (bytes[p++]) {
      case 0: return UNDEF;
      case 1: return null;
      case 2: return false;
      case 3: return true;
      case 4: return num(f64());
      case 5: return str();
      case 6: return { $: 'u8', b: raw().toString('base64') };
      case 7: return list();
      case 8: { const out = {}; for (let n = u32(); n > 0; n--) { const key = str(); out[key === '$' ? '$$' : key] = value(); } return out; }
      case 9: { u32(); const size = f64(); return { $: 'blob', type: str(), size }; }
      case 10: return { $: 'date', ms: f64() };
      case 11: { const kind = KINDS[bytes[p++]]; return { $: kind, b: raw().toString('base64') }; }
      case 12: return { $: 'map', v: Array.from({ length: u32() }, () => [value(), value()]) };
      case 13: return { $: 'set', v: list() };
      case 14: return { $: 'big', v: str() };
      default: throw new Error(`unknown tag ${bytes[p - 1]} at ${p - 1}`);
    }
  }
  const values = [];
  while (p < bytes.length) values.push(value());
  return values;
}

async function open(page, url = '/?e2e=1') {
  const problems = [];
  page.on('console', message => { if (message.type() === 'error') problems.push(`console: ${message.text()}`); });
  page.on('pageerror', error => problems.push(`pageerror: ${error.message}`));
  await page.route('https://accounts.google.com/**', route => route.fulfill({ contentType: 'text/javascript', body: '' }));
  // The signed-out connectivity probe (shell) must not reach the real Drive, whose 401 logs a console error.
  await page.route('https://www.googleapis.com/**', route => route.fulfill({ headers: { 'access-control-allow-origin': '*' }, json: { kind: 'drive#about' } }));
  await page.goto(url);
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  expect(await page.evaluate(() => window.__IHN_TEST_API__.ready())).toBe(true);
  return problems;
}

const cmd = (page, name, args = null) => page.evaluate(([n, a]) => window.__IHN_TEST_API__.cmd(n, a), [name, args]);

async function selftest(page, args, timeout = 20_000) {
  expect(await cmd(page, 'store.selftest', args)).toBe(true);
  await page.waitForFunction(() => window.__IHN_TEST_API__.cmd('store.selftest_done'), null, { timeout });
  const result = await cmd(page, 'store.selftest_result');
  result.steps = result.steps.map(step => ({ status: step.status, results: step.status >= 0 ? decodeAll(step.data) : [] }));
  return result;
}

const databases = page => page.evaluate(async () => (await indexedDB.databases()).map(db => `${db.name}@${db.version}`).sort());

test('transactions, the frozen schema, locks and the actor slot against the real host', async ({ page }) => {
  const problems = await open(page);
  // The database is opened by the first use, not at boot.
  expect(await databases(page)).toEqual([]);
  expect(await cmd(page, 'store.health')).toEqual({ health: 'closed', slot: null });

  const { steps, health } = await selftest(page, { run: 'basic', db: 'ihn6-selftest-basic' });
  expect(health).toBe('closed');
  expect(steps.map(step => step.status)).toEqual([0, -7, 0, 0, 0, 0, 1, 0, 0, 1, 1, 0, 0]);

  // 0: kv first-writer-wins and counters, document + first batch, chunks, catalog merge.
  expect(steps[0].results).toEqual([UNDEF, 'aaaa', 65536, 131072, UNDEF, UNDEF, true, UNDEF, UNDEF, UNDEF, UNDEF, true, { starred: true }]);
  // 1 aborted (seq 1 exists) → 2: neither seq 2 nor its batch stayed; chunks come back in numeric order.
  const record = {
    account: ACCOUNT, name: 'Self-test', driveMeta: {}, vv: u8(1, 2, 3), seq: { [ACTOR]: 1 },
    relay: { file_id: '', epoch_day: 0, gen: 0, archives: [], assets: [], sealed: {}, perm: u8(), applied: {} },
    exports: [], flags: { created: true, relay_pending: false, export_pending: false, migrated: false }, lastOpenedAt: 1, pinned: false
  };
  expect(steps[2].results).toEqual([
    record,
    [[DOC, ACTOR, 1], text('one')],
    [[DOC, 0], u8(0, 0, 0, 0, 0), [DOC, 2], u8(2, 2, 2, 2, 2), [DOC, 10], u8(10, 10, 10, 10, 10)]
  ]);
  // 3: SyncRecord writes + a received batch + own seq 2 + trim, one strict transaction; then a late seq 1 (the
  // stored seq stays 2) and an asset that is referenced before its bytes arrive → 4: what is stored.
  expect(steps[3].results).toEqual([true, true, true, UNDEF, UNDEF, UNDEF, true, UNDEF, true, 1, { refs: 1 }]);
  expect(steps[4].results).toEqual([
    {
      ...record,
      seq: { [ACTOR]: 2 },
      relay: {
        ...record.relay,
        sealed: { [ACTOR]: 1 },
        archives: [{ file_id: 'old', digest: u8(1, 1), ranges: u8(2, 3) }],
        assets: [{ hash: u8(...new Array(32).fill(4)), file_id: 'asset', size: 9 }],
        perm: u8(5, 5),
        applied: { [REMOTE]: [2, u8(...new Array(16).fill(6))] }
      },
      last_export: { headRevisionId: 'rev-1', vv: u8(7), etag: 'e', at: 2 },
      flags: { ...record.flags, relay_pending: true }
    },
    [[DOC, REMOTE, 5], text('remote'), [DOC, ACTOR, 2], text('two')],
    ['id-1'],
    [[DOC, ACTOR, 2], text('two')],
    { refs: 1, blob: u8(9), mime: 'x/y', size: 1 }
  ]);
  // 5: the document, its journal and chunks are gone; the asset is collected with its last reference.
  expect(steps[5].results).toEqual([UNDEF, UNDEF, UNDEF, 0, 0, 0, 0, 0]);
  // 6..10: granted, busy for a second ifAvailable request, a waiting request is pending and takes over on release;
  // released again, the next request gets it.
  const lock = 'ihn6-selftest:ihn6-selftest-basic';
  expect(steps[8].results).toEqual([{ held: [lock], pending: [lock] }]);
  // 11: the first free actor slot; 12: the scratch database deleted.
  expect(steps[11].status).toBe(0);
  expect(await databases(page)).toEqual([]);

  // The application database: version 1, the six stores of ARCHITECTURE §7.1, out-of-line keys, no indexes.
  await cmd(page, 'store.open');
  await page.waitForFunction(() => window.__IHN_TEST_API__.cmd('store.health').health === 'ready');
  expect(await databases(page)).toEqual(['inhouse-notes-v6@1']);
  const schema = await page.evaluate(() => new Promise(resolve => {
    const request = indexedDB.open('inhouse-notes-v6');
    request.onsuccess = () => {
      const db = request.result;
      const tx = db.transaction([...db.objectStoreNames]);
      resolve([...db.objectStoreNames].map(name => { const s = tx.objectStore(name); return [name, s.keyPath, s.autoIncrement, s.indexNames.length]; }));
      db.close();
    };
  }));
  expect(schema).toEqual(['assets', 'catalog', 'chunks', 'docs', 'kv', 'ops'].map(name => [name, null, false, 0]));
  expect(problems).toEqual([]);
});

test('an open blocked for 1.5 s or longer than the timeout is the degraded signal; reopen recovers', async ({ page }) => {
  const problems = await open(page);
  // Another connection holds version 1 and ignores versionchange, as a stuck 5.x-style tab would.
  const hold = name => page.evaluate(n => new Promise(resolve => {
    const request = indexedDB.open(n, 1);
    request.onsuccess = () => { (window.__held ||= {})[n] = request.result; resolve(); };
  }), name);
  const letGo = name => page.evaluate(n => window.__held[n].close(), name);

  // Blocked, with the real constants (BLOCKED_MS = 1500, OPEN_TIMEOUT_MS = 5000).
  await hold('ihn6-selftest-blocked');
  const started = Date.now();
  let result = await selftest(page, { run: 'degraded', db: 'ihn6-selftest-blocked', version: 2 });
  const elapsed = Date.now() - started;
  expect(result.health).toBe('degraded:blocked');
  expect(elapsed).toBeGreaterThanOrEqual(1500);
  expect(elapsed).toBeLessThan(4500);
  // The write queued behind the open and the one issued afterwards are both refused, not lost silently.
  expect(result.steps.map(step => step.status)).toEqual([-16, -16]);
  // The blocker goes away: the abandoned open must not leave a connection behind, and reopen works.
  await letGo('ihn6-selftest-blocked');
  result = await selftest(page, { run: 'reopen' });
  expect(result.steps.map(step => step.status)).toEqual([0, 0]);

  // Timeout: still blocked when the open timeout expires first.
  await hold('ihn6-selftest-timeout');
  result = await selftest(page, { run: 'degraded', db: 'ihn6-selftest-timeout', version: 2, blockedMs: 60000, timeoutMs: 400 });
  expect(result.health).toBe('degraded:timeout');
  expect(result.steps.map(step => step.status)).toEqual([-16, -16]);
  await letGo('ihn6-selftest-timeout');
  result = await selftest(page, { run: 'reopen' });
  expect(result.steps.map(step => step.status)).toEqual([0, 0]);
  expect(await databases(page)).toEqual([]);
  // The only console output is the store's own warning per degraded open.
  expect(problems).toEqual([]);
});

test('each tab holds its own actor slot and a closed tab frees it', async ({ context, page }) => {
  await open(page);
  const slot = async tab => {
    await cmd(tab, 'store.slot', { account: 'ana@example.test' });
    await tab.waitForFunction(() => window.__IHN_TEST_API__.cmd('store.health').slot !== null);
    return (await cmd(tab, 'store.health')).slot;
  };
  expect(await slot(page)).toBe(0);
  const second = await context.newPage();
  await open(second);
  expect(await slot(second)).toBe(1);
  expect(await slot(second)).toBe(1);
  await page.close();
  const third = await context.newPage();
  await open(third);
  expect(await slot(third)).toBe(0);
});

// The dump in the notation the Rust side reports: Blobs and Files without their bytes.
function expected(value) {
  if (value === null || typeof value !== 'object') return value;
  if (Array.isArray(value)) return value.map(expected);
  if (value.$ === 'blob' || value.$ === 'file') return { $: 'blob', type: value.type, size: Buffer.from(value.b, 'base64').length };
  return Object.fromEntries(Object.entries(value).map(([key, item]) => [key, expected(item)]));
}

const V5_DATABASES = ['inhouse-offline-v1', 'notebook-data-db-v1', 'background-notebook-data-db-v1', 'inhouse-original-pdfs-v1'];

async function dump(page) {
  const result = await selftest(page, { run: 'dump', db: 'ihn6-selftest-assets', dbs: V5_DATABASES.join(',') }, 60_000);
  const out = {};
  for (const entry of result.dump) {
    if (entry.store === undefined) {
      const [info] = entry.status === 0 ? decodeAll(entry.data) : [null];
      out[entry.db] = { status: entry.status, fingerprint: entry.fingerprint, version: info?.version, stores: info?.stores, records: {}, pageSizes: [] };
    } else {
      const [[done, ...flat]] = decodeAll(entry.data);
      const rows = (out[entry.db].records[entry.store] ||= []);
      for (let i = 0; i < flat.length; i += 2) rows.push({ key: flat[i], value: flat[i + 1] });
      out[entry.db].pageSizes.push(flat.length / 2);
      out[entry.db].lastDone = done;
    }
  }
  return { ...result, dbs: out };
}

test.describe('read-only dump of the 5.x storage fixtures', () => {
  test.skip(!fixtures || !fixtures.storageDumps().length, 'the W1.2 storage fixtures are not in this checkout (set IHN_W12_ROOT)');
  for (const name of fixtures ? fixtures.storageDumps() : []) {
    test(`${name}: every record, unchanged databases, stable fingerprint`, async ({ page }) => {
      test.setTimeout(120_000);
      const source = fixtures.storageDump(name);
      // Seed the 5.x databases before the app loads, from a page of the same origin that runs no script.
      await page.goto('/icons/inhouse-notes-32.png');
      await fixtures.restoreStorage(page, { indexedDB: source.indexedDB });
      const before = await databases(page);
      const problems = await open(page);

      const first = await dump(page);
      for (const db of V5_DATABASES) {
        const want = source.indexedDB.find(record => record.name === db);
        const got = first.dbs[db];
        if (!want) { expect(got.status, db).toBe(1); continue; }
        expect(got.status, db).toBe(0);
        expect(got.version, db).toBe(want.version);
        expect(got.stores.map(s => [s.name, s.count]), db).toEqual(want.stores.map(s => [s.name, s.records.length]));
        for (const store of want.stores) {
          expect(got.records[store.name], `${db}/${store.name}`).toEqual(store.records.map(expected));
        }
        expect(Math.max(...got.pageSizes), db).toBeLessThanOrEqual(3);
      }
      const counted = fixtures.storageExpect(name).stores;
      for (const [db, stores] of Object.entries(counted)) {
        expect(Object.fromEntries(first.dbs[db].stores.map(s => [s.name, s.count])), db).toEqual(stores);
      }
      // A Blob read from 5.x was stored as a v6 asset and read back (same size and type), where the dump has one.
      const blobs = JSON.stringify(source.indexedDB).includes('"$":"blob"');
      expect(first.steps.length).toBe(blobs ? 2 : 0);
      if (blobs) {
        const [absent, refs, asset] = first.steps[0].results;
        expect([absent, refs, asset.refs, asset.size, asset.mime]).toEqual([UNDEF, 1, 1, asset.blob.size, asset.blob.type]);
        expect(asset.blob.size).toBeGreaterThan(0);
      }

      // Read-only: same databases at the same versions, nothing created for the missing ones, same content.
      expect(await databases(page)).toEqual(before);
      const again = await dump(page);
      for (const db of V5_DATABASES) expect(again.dbs[db].fingerprint, db).toEqual(first.dbs[db].fingerprint);
      expect(again.dbs).toEqual(first.dbs);

      // A 5.x tab writing afterwards changes the fingerprint of that database only (ARCHITECTURE §10.2 step 3).
      await page.evaluate(() => new Promise((resolve, reject) => {
        const request = indexedDB.open('notebook-data-db-v1');
        request.onsuccess = () => {
          const db = request.result, tx = db.transaction('stroke-ops', 'readwrite');
          tx.objectStore('stroke-ops').add({ pageIndex: 0, type: 'add-stroke', stroke: { id: 'late' }, savedAt: Date.now(), sessionId: 1 });
          tx.oncomplete = () => { db.close(); resolve(); };
          tx.onerror = () => reject(tx.error);
        };
      }));
      const later = await dump(page);
      expect(later.dbs['notebook-data-db-v1'].fingerprint).not.toEqual(first.dbs['notebook-data-db-v1'].fingerprint);
      expect(later.dbs['inhouse-offline-v1'].fingerprint).toEqual(first.dbs['inhouse-offline-v1'].fingerprint);
      expect(problems).toEqual([]);
    });
  }
});
