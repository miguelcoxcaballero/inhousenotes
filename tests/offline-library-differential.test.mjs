// Differential test for the library catalogue index ported to Rust (offline unit U2).
// The pre-port offline-library-v5.js is loaded from git (tests/helpers/legacy.mjs) and compared with the current file,
// backed by the real WASM core, on hand written fixtures and thousands of seeded random catalogues.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import { legacySource, rng } from './helpers/legacy.mjs';

const currentSource = readFileSync(new URL('../offline-library-v5.js', import.meta.url), 'utf8');
const rustSource = readFileSync(new URL('../rust-core-v5.js', import.meta.url), 'utf8');
const wasm = readFileSync(new URL('../wasm/inhouse_notes_core.wasm', import.meta.url));
const folderType = 'application/vnd.google-apps.folder';
const pdf = 'application/pdf';

async function load(source, rust) {
    const state = { entries: [], docs: [], views: {}, driveRoot: null, quick: new Set() };
    const store = {
        async list(kind) { return kind === 'entries' ? structuredClone(state.entries) : kind === 'document-index' ? structuredClone(state.docs) : []; },
        async read(kind, account, key) { return structuredClone(key === 'drive-root' ? state.driveRoot : key === 'home-views' ? state.views : null); }
    };
    const sandbox = {
        URLSearchParams, location: { search: '?offline-worker=1', pathname: '/', href: 'https://notes.test/' }, navigator: { onLine: false },
        WebAssembly, URL, TextEncoder, TextDecoder, AbortSignal, fetch: async () => new Response(wasm),
        localStorage: { getItem: () => 'index@example.test' }, sessionStorage: { getItem: () => null },
        document: { querySelectorAll: () => [] }, InhouseOfflineStore: store, InhouseRuntimeCore: { CoalescingQueue: class {} }
    };
    sandbox.window = sandbox;
    vm.createContext(sandbox);
    if (rust) { vm.runInContext(rustSource, sandbox); assert.equal(await sandbox.InhouseRust.ready, true); }
    vm.runInContext(source, sandbox);
    const api = sandbox.InhouseOfflineLibrary;
    await api.init({ quickAccess: entry => state.quick.has(entry.id) });
    return { api, state };
}
const [legacy, current] = [await load(legacySource('offline-library-v5.js'), false), await load(currentSource, true)];

// Plain copies so deepStrictEqual compares data, not realms.
const plain = value => JSON.parse(JSON.stringify(value ?? null));
function snapshot(api, queries) {
    return {
        home: plain(api.home()),
        folders: queries.folders.map(([id, search]) => plain(api.folder(id, search))),
        combined: queries.combine.map(([remote, parent, search]) => plain(api.combine(structuredClone(remote), parent, search))),
        recent: queries.recent.map(list => plain(api.recentItems(structuredClone(list)))),
        records: queries.records.map(id => plain(api.record(id)))
    };
}
async function agree(state, queries, label) {
    for (const side of [legacy, current]) { Object.assign(side.state, structuredClone({ ...state, quick: undefined })); side.state.quick = new Set(state.quick); }
    await Promise.all([legacy.api.reload(), current.api.reload()]);
    assert.deepStrictEqual(snapshot(current.api, queries), snapshot(legacy.api, queries), label);
}

const names = ['Alpha', 'alpha', 'ALPHA', 'beta', 'Beta', 'ä', 'a', 'z', 'Z', 'é', 'e', 'E', 'ñ', 'n', '_x', '-x', '10', '2', '1', 'a b', 'ab', 'a-b', 'ǆ', 'Dž', 'ß', 'ss',
    '日本', '中文', '한국', 'ÅÄÖ', 'Ωmega', 'σίσυφος', 'ΣΊΣΥΦΟΣ', 'İstanbul', 'ı', 'I', 'i', '😀 smile', '😀', 'a­b', 'ab', 'a​b', 'é', 'é', '', 'Ａ', 'A', 'ǅ', 'Ⅻ', 'xii', '\ud83d', 'tail\ud83d'];
const dates = ['2026-10-01T00:00:00Z', '2026-10-01T00:00:00.000Z', '2026-10-03T12:00:00Z', '2025-01-01T00:00:00Z', 'Wed, 03 Oct 2026 00:00:00 GMT', '2026-13-45', 'not a date', '', undefined, null, '2026-10-01', '1970-01-01T00:00:00Z', '2030-01-01T00:00:00Z'];

function scenario(seed) {
    const next = rng(seed), pick = list => list[Math.floor(next() * list.length)], chance = p => next() < p;
    const count = 1 + Math.floor(next() * (seed % 40 === 0 ? 400 : 40));
    const ids = Array.from({ length: count }, (_, i) => chance(0.25) ? `local-${i}` : chance(0.1) ? `😀${i}` : chance(0.05) ? `￮${i}` : `id${i}`);
    const remoteIds = [...ids.map(id => `r-${id}`), 'drive-root', 'shared-remote', 'root'];
    const pool = [...ids, ...remoteIds, 'root', 'root', 'ghost'];
    const entries = ids.map(id => {
        const kind = next();
        const entry = { id, name: chance(0.03) ? undefined : pick(names), mimeType: kind < 0.3 ? folderType : kind < 0.9 ? pdf : 'text/plain' };
        if (chance(0.85)) entry.parents = Array.from({ length: Math.floor(next() * 4) }, () => chance(0.45) ? 'root' : pick(pool));
        else if (chance(0.5)) entry.parents = undefined;
        if (chance(0.7)) entry.modifiedTime = pick(dates);
        if (chance(0.35)) entry.remoteId = chance(0.8) ? `r-${id}` : pick(pool);
        if (chance(0.12)) entry.remoteId = id;
        if (chance(0.2)) entry.pending = true;
        if (chance(0.12)) entry.trashed = true;
        if (chance(0.15)) entry.starred = true;
        if (chance(0.25)) entry.ownedByMe = chance(0.5);
        return entry;
    });
    // remote copies of local records, some newer, some older, some with their own parents
    for (const entry of [...entries]) if (entry.remoteId && chance(0.6) && entry.remoteId !== entry.id && !entries.some(other => other.id === entry.remoteId)) {
        const copy = { id: entry.remoteId, name: pick(names), mimeType: entry.mimeType, modifiedTime: pick(dates) };
        if (chance(0.5)) copy.parents = [pick(pool)];
        if (chance(0.2)) copy.parents = undefined;
        if (chance(0.3)) copy.starred = true;
        entries.push(copy);
    }
    const unique = entries.filter((entry, at) => entries.findIndex(other => other.id === entry.id) === at); // store keys are unique
    entries.splice(0, entries.length, ...unique);
    const docs = Array.from({ length: Math.floor(next() * count) }, (_, i) => {
        const doc = { id: chance(0.7) ? pick(ids) : `doc${i}` };
        if (chance(0.5)) doc.remoteId = pick(remoteIds.concat(ids));
        if (chance(0.7)) doc.lastOpenedAt = pick([0, 50, 100, 100, 200, 5000, undefined, null, '7']);
        return doc;
    }).filter((doc, at, all) => all.findIndex(other => other.id === doc.id) === at);
    const viewOf = (limit) => chance(0.35) ? Array.from({ length: Math.floor(next() * limit) }, () => pick([...pool, 'unknown', 5, null])) : undefined;
    const views = {}; for (const name of ['recents', 'starred', 'shared']) { const list = viewOf(10); if (list) views[name] = list; if (chance(0.03)) views[name] = 'not-an-array'; }
    const driveRoot = chance(0.5) ? { remoteId: chance(0.8) ? 'drive-root' : pick(remoteIds) } : chance(0.5) ? {} : null;
    const quick = new Set(entries.filter(() => chance(0.2)).map(entry => entry.id));
    const parents = [...new Set(['root', ...entries.flatMap(entry => entry.parents || []), 'nowhere'])];
    const searches = ['', '', 'a', 'A', 'alpha', 'é', 'σ', 'ς', 'İ', 'i', 'ǆ', 'ss', '😀', 'x', ' ', 'zzz'];
    const foreign = (n) => Array.from({ length: n }, (_, i) => {
        const base = chance(0.6) && entries.length ? { ...pick(entries) } : {};
        return { ...base, id: base.id && chance(0.7) ? base.id : `net${i}`, name: pick(names), mimeType: chance(0.4) ? folderType : pdf,
            modifiedTime: pick(dates), ...(chance(0.3) ? { remoteId: pick(pool) } : {}), ...(chance(0.2) ? { pending: true } : {}) };
    });
    const queries = {
        folders: Array.from({ length: 6 }, () => [chance(0.1) ? undefined : pick(parents), pick(searches)]),
        combine: Array.from({ length: 4 }, () => [foreign(Math.floor(next() * 12)), chance(0.1) ? undefined : pick(parents), pick(searches)]),
        recent: Array.from({ length: 4 }, () => foreign(Math.floor(next() * 30))),
        records: [...ids.slice(0, 5), ...remoteIds.slice(0, 5), 'none']
    };
    return { state: { entries, docs, views, driveRoot, quick }, queries };
}

test('fixture catalogue matches the legacy implementation', async () => {
    const { state, queries } = scenario(7);
    await agree(state, queries, 'fixture');
    for (const parent of ['root', 'ghost']) for (const search of ['', 'a']) {
        assert.deepStrictEqual(plain(current.api.folder(parent, search)), plain(legacy.api.folder(parent, search)));
    }
});

test('folder names order exactly like localeCompare, including collation ties and ignorables', async () => {
    const all = names.filter((name, at) => names.indexOf(name) === at);
    const state = { entries: all.flatMap((name, at) => [{ id: `f${at}`, name, mimeType: folderType, parents: ['root'] }, { id: `g${at}`, name, mimeType: folderType, parents: ['root'] }]), docs: [], views: {}, driveRoot: null, quick: new Set() };
    await agree(state, { folders: [['root', '']], combine: [], recent: [], records: [] }, 'names');
    assert.equal(current.api.folder('root').length, all.length * 2);
});

test('canonical ISO timestamps order exactly like Date.parse, across every year and leap day', async () => {
    const next = rng(99), pad = (value, size) => String(value).padStart(size, '0');
    const iso = (year, month, day, hour, minute, second, millis, withMillis) =>
        `${pad(year, 4)}-${pad(month, 2)}-${pad(day, 2)}T${pad(hour, 2)}:${pad(minute, 2)}:${pad(second, 2)}${withMillis ? `.${pad(millis, 3)}` : ''}Z`;
    const stamps = [];
    for (let i = 0; i < 3000; i++) stamps.push(iso(Math.floor(next() * 10000), 1 + Math.floor(next() * 12), 1 + Math.floor(next() * 28), Math.floor(next() * 24), Math.floor(next() * 60), Math.floor(next() * 60), Math.floor(next() * 1000), next() < 0.7));
    // leap days, month ends, the epoch, century rules and one dense millisecond window
    for (const year of [0, 4, 100, 400, 1600, 1900, 1969, 1970, 1999, 2000, 2024, 2100, 2400, 9999]) for (const [month, day] of [[1, 1], [2, 28], [2, 29], [3, 1], [4, 30], [12, 31]]) stamps.push(iso(year, month, day, 23, 59, 59, 999, true));
    for (let i = 0; i < 400; i++) stamps.push(iso(2026, 10, 1, 12, 0, 0, i, true));
    // text the fast path does not take (and invalid dates JavaScript rejects or bends)
    stamps.push('2026-02-30T00:00:00Z', '2026-02-29T00:00:00Z', '2026-10-01', '2026-10-01T00:00:00', '2026-10-01T00:00:00+05:30', '2026-10-01T00:00:00.5Z', '2026-10-01T24:00:00Z', '2026-10-01T00:00:60Z', 'junk', '', ' 2026-10-01T00:00:00Z');
    const state = { entries: stamps.map((modifiedTime, at) => ({ id: `t${at}`, name: `t${at}`, mimeType: pdf, parents: ['root'], modifiedTime })), docs: [], views: {}, driveRoot: null, quick: new Set() };
    await agree(state, { folders: [['root', ''], ['root', 't1']], combine: [], recent: [], records: [] }, 'timestamps');
    assert.equal(current.api.folder('root').length, stamps.length);
});

test('observed timestamp and star changes are honoured before the next reload', async () => {
    const { state, queries } = scenario(11);
    await agree(state, queries, 'before');
    for (const side of [legacy, current]) {
        const rows = side.api.folder('root');
        for (const [at, row] of rows.entries()) if (at % 2 === 0) { row.modifiedTime = '2031-01-01T00:00:00Z'; row.starred = !row.starred; }
    }
    assert.deepStrictEqual(snapshot(current.api, queries), snapshot(legacy.api, queries), 'after mutation');
});

test('logout clears every index and a failed Rust core is reported loudly', async () => {
    const { state, queries } = scenario(13);
    await agree(state, queries, 'before logout');
    legacy.api.logout(); current.api.logout();
    assert.deepStrictEqual(snapshot(current.api, queries), snapshot(legacy.api, queries), 'after logout');
    await assert.rejects(() => load(currentSource, false), /Rust core is unavailable/);
    await Promise.all([legacy.api.useAccount('index@example.test'), current.api.useAccount('index@example.test')]); // sign back in for the next tests
});

test('three thousand random catalogues agree with the legacy implementation', async () => {
    const seeds = Number(process.env.LIBRARY_SEEDS || 3000);
    const seen = { home: 0, folders: 0, combined: 0, recent: 0, overlaid: 0, mixedFolderRows: 0 };
    const first = Number(process.env.LIBRARY_FIRST_SEED || 1); // LIBRARY_FIRST_SEED=3001 LIBRARY_SEEDS=9000 explores further
    for (let seed = first; seed <= seeds; seed++) {
        const { state, queries } = scenario(seed);
        await agree(state, queries, `seed ${seed}`);
        const view = snapshot(current.api, queries);
        seen.home += view.home.recents.length + view.home.starred.length + view.home.shared.length;
        seen.folders += view.folders.reduce((sum, list) => sum + list.length, 0);
        seen.combined += view.combined.reduce((sum, list) => sum + list.length, 0);
        seen.recent += view.recent.reduce((sum, list) => sum + list.length, 0);
        seen.overlaid += state.entries.filter(entry => entry.remoteId && state.entries.some(other => other.id === entry.remoteId)).length;
        seen.mixedFolderRows += view.folders.filter(list => list.filter(row => row.mimeType === folderType).length > 1).length;
        if (seed % 7 === 0) {
            // mutate observed records on both sides identically, then compare again without a reload
            for (const side of [legacy, current]) side.api.folder('root').forEach((row, at) => { if (at % 3 === 0) { row.modifiedTime = ['2030-05-05T00:00:00Z', undefined, 'junk'][at % 3 + seed % 3]; row.starred = at % 2 === 0; } });
            assert.deepStrictEqual(snapshot(current.api, queries), snapshot(legacy.api, queries), `seed ${seed} after mutation`);
        }
    }
    // the random catalogues must exercise every query, otherwise agreement proves nothing
    for (const [name, total] of Object.entries(seen)) assert.ok(total > (seeds - first + 1) / (name === 'mixedFolderRows' ? 10 : 1), `${name} coverage too low: ${total}`);
});
