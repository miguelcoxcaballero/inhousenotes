// Fixture state for the mock: accounts, Drive libraries, the 5.x documents of tests/v6/fixtures/v5 (TESTING §5) and
// the 5.x storage dumps, plus the Playwright `test` used by the smoke specs (a mock per test, installed on the context).
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { test as base, expect } from '@playwright/test';
import { createMock } from './google.mjs';
import { FOLDER } from './drive.mjs';

const HERE = path.dirname(fileURLToPath(import.meta.url));
export const V5_DIR = path.resolve(HERE, '..', 'fixtures', 'v5');

/** The three people of every scenario: the owner, a second editor, a viewer of another account. */
export const ACCOUNTS = {
  ana: { email: 'ana@example.test', displayName: 'Ana Owner', photoLink: 'https://lh3.googleusercontent.com/a/mock-ana=s96-c' },
  ben: { email: 'ben@example.test', displayName: 'Ben Writer', photoLink: 'https://lh3.googleusercontent.com/a/mock-ben=s96-c' },
  cho: { email: 'cho@example.test', displayName: 'Cho Viewer', photoLink: '' }
};
export const ANA = ACCOUNTS.ana.email, BEN = ACCOUNTS.ben.email, CHO = ACCOUNTS.cho.email;

export function addAccounts(mock) {
  for (const account of Object.values(ACCOUNTS)) if (!mock.drive.accounts.has(account.email)) mock.drive.addAccount(account);
  return mock;
}

// ---- v5 documents ----
/** Names of the committed 5.x Drive fixtures (without extension). */
export function v5Fixtures() {
  return fs.existsSync(V5_DIR) ? fs.readdirSync(V5_DIR).filter(f => f.endsWith('.pdf')).map(f => f.slice(0, -4)).sort() : [];
}
/** `{pdf, expect}` of a 5.x Drive fixture; `expect` is what 5.18.0 reported after importing it. */
export function v5Fixture(name) {
  const pdf = fs.readFileSync(path.join(V5_DIR, `${name}.pdf`));
  const expectFile = path.join(V5_DIR, `${name}.expect.json`);
  return { pdf, expect: fs.existsSync(expectFile) ? JSON.parse(fs.readFileSync(expectFile, 'utf8')) : null };
}
/** A PDF larger than the 64 MiB safety limit, generated on the fly (never committed). */
export function oversizedPdf(sizeBytes = 65 * 1024 * 1024) {
  const head = Buffer.from('%PDF-1.4\n%\xe2\xe3\xcf\xd3\n', 'latin1'), out = Buffer.alloc(sizeBytes, 0x20);
  head.copy(out); Buffer.from('\n%%EOF\n').copy(out, sizeBytes - 7);
  return out;
}
/** Smallest well-formed one-page PDF (A4, empty); for library entries whose content is never opened. */
export function blankPdf() {
  const objects = ['<< /Type /Catalog /Pages 2 0 R >>', '<< /Type /Pages /Kids [3 0 R] /Count 1 >>', '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595.28 841.89] >>'];
  let body = '%PDF-1.4\n'; const offsets = [];
  objects.forEach((object, index) => { offsets.push(body.length); body += `${index + 1} 0 obj\n${object}\nendobj\n`; });
  const xref = body.length;
  body += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n${offsets.map(o => `${String(o).padStart(10, '0')} 00000 n \n`).join('')}trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
  return Buffer.from(body, 'latin1');
}

export const sceneId = name => `scene-${name}`;
/**
 * Puts a 5.x fixture document into the mock Drive (owner: Ana) and returns the file; states open it with
 * `/?drive=<id>&e2e=1` (the scene loader of TESTING §3.1). `permissions` adds collaborators or a public link.
 */
export function addScene(mock, name, { id = sceneId(name), owner = ANA, title, permissions = [], parents, revisions = [] } = {}) {
  addAccounts(mock);
  const existing = mock.drive.file(id);
  if (existing) return existing;
  const file = mock.drive.addFile({ id, name: title || `${name}.pdf`, owner, parents, permissions, data: v5Fixture(name).pdf, modifiedTime: '2026-10-29T09:00:00.000Z' });
  for (const later of revisions) mock.drive.handle({ method: 'PATCH', url: `https://www.googleapis.com/upload/drive/v3/files/${id}?uploadType=media`, headers: { authorization: `Bearer ${mock.drive.accounts.get(owner).token}`, 'content-type': 'application/pdf' }, body: v5Fixture(later).pdf });
  return file;
}

// ---- Drive libraries (Home) ----
export const fixtures = {
  /** Three accounts, empty Drives. */
  empty(mock) { return addAccounts(mock); },
  /**
   * A small library for Home: folders (one nested, coloured), documents in root and in folders, a starred document,
   * a document and a folder shared by Ben, one item in the bin. Times are fixed, a day apart.
   */
  library(mock) {
    addAccounts(mock);
    const d = mock.drive, pdf = blankPdf(), at = day => `2026-10-${String(day).padStart(2, '0')}T08:00:00.000Z`;
    const school = d.addFile({ id: 'lib-folder-school', name: 'School', mimeType: FOLDER, owner: ANA, folderColorRgb: '#4986e7', modifiedTime: at(10) });
    const maths = d.addFile({ id: 'lib-folder-maths', name: 'Maths', mimeType: FOLDER, owner: ANA, parents: [school.id], modifiedTime: at(11) });
    d.addFile({ id: 'lib-folder-travel', name: 'Travel', mimeType: FOLDER, owner: ANA, folderColorRgb: '#16a765', modifiedTime: at(12) });
    d.addFile({ id: 'lib-doc-ideas', name: 'Ideas.pdf', owner: ANA, data: pdf, modifiedTime: at(20) });
    d.addFile({ id: 'lib-doc-journal', name: 'Journal.pdf', owner: ANA, data: pdf, starred: true, modifiedTime: at(21) });
    d.addFile({ id: 'lib-doc-algebra', name: 'Algebra notes.pdf', owner: ANA, parents: [maths.id], data: pdf, modifiedTime: at(22) });
    d.addFile({ id: 'lib-doc-syllabus', name: 'Syllabus.pdf', owner: ANA, parents: [school.id], data: pdf, modifiedTime: at(23) });
    d.addFile({ id: 'lib-doc-shared', name: 'Shared plan.pdf', owner: BEN, data: pdf, permissions: [{ type: 'user', role: 'writer', emailAddress: ANA }], modifiedTime: at(24) });
    d.addFile({ id: 'lib-folder-shared', name: 'Team', mimeType: FOLDER, owner: BEN, permissions: [{ type: 'user', role: 'reader', emailAddress: ANA }], modifiedTime: at(13) });
    d.addFile({ id: 'lib-doc-binned', name: 'Old draft.pdf', owner: ANA, data: pdf, trashed: true, modifiedTime: at(5) });
    return mock;
  },
  /** `count` documents and `folders` folders in root, for long lists and fast scroll. */
  long(mock, { count = 120, folders = 12 } = {}) {
    addAccounts(mock);
    const pdf = blankPdf();
    for (let i = 0; i < folders; i++) mock.drive.addFile({ id: `long-folder-${i}`, name: `Folder ${String(i + 1).padStart(2, '0')}`, mimeType: FOLDER, owner: ANA, modifiedTime: T(1, i) });
    for (let i = 0; i < count; i++) mock.drive.addFile({ id: `long-doc-${i}`, name: `Document ${String(i + 1).padStart(3, '0')}.pdf`, owner: ANA, data: pdf, starred: i % 9 === 0, modifiedTime: T(2, i) });
    return mock;
  }
};
const T = (day, minute) => new Date(Date.UTC(2026, 9, day, 8, 0, 0) + minute * 60_000).toISOString();

// ---- 5.x storage dumps (TESTING §5) ----
export const STORAGE_DIR = path.join(V5_DIR, 'storage');
export const storageDumps = () => fs.existsSync(STORAGE_DIR) ? fs.readdirSync(STORAGE_DIR).filter(f => f.endsWith('.dump.json')).map(f => f.slice(0, -10)).sort() : [];
export const storageDump = name => JSON.parse(fs.readFileSync(path.join(STORAGE_DIR, `${name}.dump.json`), 'utf8'));
export const storageExpect = name => JSON.parse(fs.readFileSync(path.join(STORAGE_DIR, `${name}.expect.json`), 'utf8'));

// Structured-clone values ⇄ JSON. Binary values are tagged: {"$":"u8","b":"<base64>"}, {"$":"blob","type":…,"b":…},
// {"$":"date","ms":…}, {"$":"map"|"set", "v":[…]}, {"$":"undef"}, {"$":"num","v":"NaN"|"Infinity"|"-Infinity"|"-0"}.
const CODEC = `
  const b64 = bytes => { let s = ''; for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode.apply(null, bytes.subarray(i, i + 0x8000)); return btoa(s); };
  const unb64 = text => { const s = atob(text), out = new Uint8Array(s.length); for (let i = 0; i < s.length; i++) out[i] = s.charCodeAt(i); return out; };
  const VIEWS = { i8: Int8Array, u8: Uint8Array, u8c: Uint8ClampedArray, i16: Int16Array, u16: Uint16Array, i32: Int32Array, u32: Uint32Array, f32: Float32Array, f64: Float64Array, i64: BigInt64Array, u64: BigUint64Array };
  const encode = async v => {
    if (v === undefined) return { $: 'undef' };
    if (typeof v === 'number' && (!Number.isFinite(v) || Object.is(v, -0))) return { $: 'num', v: Object.is(v, -0) ? '-0' : String(v) };
    if (typeof v === 'bigint') return { $: 'big', v: String(v) };
    if (v === null || typeof v !== 'object') return v;
    if (v instanceof Date) return { $: 'date', ms: v.getTime() };
    if (v instanceof Blob) return { $: v instanceof File ? 'file' : 'blob', type: v.type, name: v.name, lastModified: v instanceof File ? v.lastModified : undefined, b: b64(new Uint8Array(await v.arrayBuffer())) };
    if (v instanceof ArrayBuffer) return { $: 'ab', b: b64(new Uint8Array(v)) };
    if (ArrayBuffer.isView(v)) { const kind = v instanceof DataView ? 'dv' : Object.keys(VIEWS).find(k => v.constructor === VIEWS[k]); return { $: kind, b: b64(new Uint8Array(v.buffer, v.byteOffset, v.byteLength)) }; }
    if (v instanceof Map) return { $: 'map', v: await Promise.all([...v].map(async ([k, x]) => [await encode(k), await encode(x)])) };
    if (v instanceof Set) return { $: 'set', v: await Promise.all([...v].map(encode)) };
    if (Array.isArray(v)) return Promise.all(v.map(encode));
    const out = {};
    for (const key of Object.keys(v)) out[key === '$' ? '$$' : key] = await encode(v[key]);
    return out;
  };
  const decode = v => {
    if (v === null || typeof v !== 'object') return v;
    if (Array.isArray(v)) return v.map(decode);
    if (typeof v.$ === 'string') {
      if (v.$ === 'undef') return undefined;
      if (v.$ === 'num') return v.v === '-0' ? -0 : Number(v.v);
      if (v.$ === 'big') return BigInt(v.v);
      if (v.$ === 'date') return new Date(v.ms);
      if (v.$ === 'blob') return new Blob([unb64(v.b)], { type: v.type });
      if (v.$ === 'file') return new File([unb64(v.b)], v.name, { type: v.type, lastModified: v.lastModified });
      if (v.$ === 'ab') return unb64(v.b).buffer;
      if (v.$ === 'dv') return new DataView(unb64(v.b).buffer);
      if (v.$ === 'map') return new Map(v.v.map(([k, x]) => [decode(k), decode(x)]));
      if (v.$ === 'set') return new Set(v.v.map(decode));
      if (VIEWS[v.$]) { const bytes = unb64(v.b); return new VIEWS[v.$](bytes.buffer, 0, bytes.byteLength / VIEWS[v.$].BYTES_PER_ELEMENT); }
    }
    const out = {};
    for (const key of Object.keys(v)) out[key === '$$' ? '$' : key] = decode(v[key]);
    return out;
  };
  const done = request => new Promise((resolve, reject) => { request.onsuccess = () => resolve(request.result); request.onerror = () => reject(request.error); });
`;

/**
 * Dumps the origin's localStorage, sessionStorage and every IndexedDB database of the page (schema + records, in key
 * order). The result is plain JSON and deterministic for a deterministic session.
 */
export function dumpStorage(page) {
  return page.evaluate(new Function(`return (async () => { ${CODEC}
    const entries = storage => Object.fromEntries(Object.keys(storage).sort().map(key => [key, storage.getItem(key)]));
    const out = { localStorage: entries(localStorage), sessionStorage: entries(sessionStorage), indexedDB: [] };
    for (const info of (await indexedDB.databases()).sort((a, b) => a.name < b.name ? -1 : 1)) {
      const db = await done(indexedDB.open(info.name));
      const record = { name: db.name, version: db.version, stores: [] };
      for (const name of [...db.objectStoreNames].sort()) {
        const tx = db.transaction(name, 'readonly'), store = tx.objectStore(name);
        const keys = await done(store.getAllKeys()), values = await done(store.getAll());
        record.stores.push({ name, keyPath: store.keyPath, autoIncrement: store.autoIncrement,
          indexes: [...store.indexNames].sort().map(i => { const index = store.index(i); return { name: i, keyPath: index.keyPath, unique: index.unique, multiEntry: index.multiEntry }; }),
          records: await Promise.all(keys.map(async (key, i) => ({ key: await encode(key), value: await encode(values[i]) }))) });
      }
      db.close();
      out.indexedDB.push(record);
    }
    return out;
  })()`));
}

/** Restores a dump into the page's origin (databases are recreated with their schema). Call before loading the app. */
export function restoreStorage(page, dump) {
  return page.evaluate(new Function('dump', `return (async () => { ${CODEC}
    for (const [key, value] of Object.entries(dump.localStorage || {})) localStorage.setItem(key, value);
    for (const [key, value] of Object.entries(dump.sessionStorage || {})) sessionStorage.setItem(key, value);
    for (const record of dump.indexedDB || []) {
      await done(indexedDB.deleteDatabase(record.name));
      const open = indexedDB.open(record.name, record.version);
      open.onupgradeneeded = () => {
        for (const s of record.stores) {
          const store = open.result.createObjectStore(s.name, { keyPath: s.keyPath ?? undefined, autoIncrement: s.autoIncrement });
          for (const index of s.indexes) store.createIndex(index.name, index.keyPath, { unique: index.unique, multiEntry: index.multiEntry });
        }
      };
      const db = await done(open);
      for (const s of record.stores) {
        if (!s.records.length) continue;
        const tx = db.transaction(s.name, 'readwrite'), store = tx.objectStore(s.name);
        for (const row of s.records) { if (s.keyPath != null) store.put(decode(row.value)); else store.put(decode(row.value), decode(row.key)); }
        await new Promise((resolve, reject) => { tx.oncomplete = resolve; tx.onerror = tx.onabort = () => reject(tx.error); });
      }
      db.close();
    }
  })()`), dump);
}

// ---- Playwright fixtures for the smoke specs (tests/v6/e2e) ----
/**
 * `test` with a `mock` (accounts added, real-time Drive clock, installed on the context before the page exists) and
 * `problems` (page errors and CSP violations; every smoke spec ends with `expect(problems).toEqual([])`).
 */
export const test = base.extend({
  mock: async ({ context }, use) => {
    const mock = createMock({ drive: { now: () => Date.now() } });
    addAccounts(mock);
    await mock.install(context);
    await use(mock);
  },
  problems: async ({ page, mock }, use) => {
    const problems = [];
    page.on('pageerror', error => problems.push(`pageerror: ${error.message}`));
    page.on('console', message => { if (message.type() === 'error' && /Content Security Policy|Refused to/.test(message.text())) problems.push(`csp: ${message.text()}`); });
    await use(problems);
    for (const request of mock.unexpected) problems.push(`unmocked: ${request}`);
  }
});
export { expect };

/** Signs the mock account in before the app loads (stored session, like a returning user). */
export async function seedSession(context, mock, email = ANA) {
  await context.addInitScript(entries => {
    if (localStorage.getItem('ihn-test-seeded')) return;   // once per context: a later sign-out must stick
    localStorage.setItem('ihn-test-seeded', '1');
    for (const [key, value] of Object.entries(entries)) localStorage.setItem(key, value);
  }, mock.session(email));
}
