// Differential test: the Rust live snapshot cache and delta planner (live-collaboration-v5.js glue +
// rust-core/src/collab/snapshot.rs) against the original JavaScript (ihnSerializeSnapshotPage,
// ihnRememberLiveSnapshot, ihnBuildPeerSnapshot at the port base commit).
import assert from 'node:assert/strict';
import fs from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';
import './helpers/collab-rust.mjs';
import { makeGenerator } from './helpers/collab-gen.mjs';
import { legacySource, loadLegacy } from './helpers/legacy.mjs';

const legacyCore = loadLegacy('collaboration-core-v5.js');

function sliceSnapshotCode(source) {
    const from = source.indexOf(source.includes('function ihnSerializeSnapshotPage(') ? 'function ihnSerializeSnapshotPage(' : 'function ihnSnapshotRust(');
    const to = source.indexOf('async function ihnBuildLiveSnapshot(');
    assert.ok(from > 0 && to > from, 'snapshot code not found');
    return source.slice(from, to);
}

function load(source, limit, maxChars, extra = {}) {
    const scope = vm.createContext({ JSON, Map, Set, String, Array, Math, Number, Boolean, Error, ...extra });
    vm.runInContext(`const IHN_LIVE_SNAPSHOT_CACHE_LIMIT = ${limit}; const IHN_LIVE_SNAPSHOT_CACHE_MAX_CHARS = ${maxChars};
        const ihnLiveSnapshotCache = new Map();\n${sliceSnapshotCode(source)}
        globalThis.api = { remember: ihnRememberLiveSnapshot, build: ihnBuildPeerSnapshot, cache: ihnLiveSnapshotCache };`, scope);
    return scope.api;
}

const currentSource = fs.readFileSync(new URL('../live-collaboration-v5.js', import.meta.url), 'utf8');
const pair = (limit = 3, maxChars = 16_000_000) => ({
    old: load(legacySource('live-collaboration-v5.js'), limit, maxChars, { ihnStableStringify: legacyCore.ihnStableStringify }),
    now: load(currentSource, limit, maxChars, { InhouseRust: globalThis.InhouseRust })
});
const reset = api => api.now.__reset?.();
const clearRust = () => globalThis.InhouseRust.collab.call('collab.snap_reset', '');

function page(g, id, bulk) {
    const { plain, string, int, random } = g;
    const value = { strokes: Array.from({ length: bulk }, (_, n) => ({ id: `s${n}`, points: Array.from({ length: 6 }, () => ({ x: int(500) + random(), y: int(500) })), w: plain(2) })), note: string(12) };
    if (id !== undefined) value.pageId = id;
    if (random() < 0.3) value.extra = plain(0);
    return value;
}

test('snapshot cache and delta planner match the original on random sessions', () => {
    let deltas = 0, fulls = 0;
    for (let session = 0; session < 40; session += 1) {
        const g = makeGenerator(900 + session);
        const { int, random, pick, string } = g;
        const small = session % 4 === 0; // exercises the character eviction as well
        const { old, now } = pair(1 + int(4), small ? 4000 + int(8000) : 16_000_000);
        clearRust();
        const ids = ['p1', 'p2', 'p3', 'p4', 'p5', 'p6', 'a'.repeat(3), 7, undefined, '', 'p1', '\ud83d', string(4), { x: 1 }, ['q']];
        const bulk = 6 + int(14);
        let pages = Array.from({ length: 3 + int(5) }, () => page(g, pick(ids), bulk));
        const seen = [];
        for (let step = 0; step < 30; step += 1) {
            // Mutate: keep most pages, change one, sometimes add/remove/reorder.
            pages = pages.map(p => p);
            const roll = random();
            if (roll < 0.45 && pages.length) pages[int(pages.length)] = page(g, pick(ids), bulk);
            else if (roll < 0.55) pages.push(page(g, pick(ids), bulk));
            else if (roll < 0.62 && pages.length > 1) pages.splice(int(pages.length), 1);
            else if (roll < 0.68 && pages.length > 1) pages.reverse();
            else if (roll < 0.72) pages = pages.map(() => page(g, pick(ids), bulk));
            if (random() < 0.05) pages[0] = pick([null, 5, 'str', []]);
            const contentHash = random() < 0.04 ? pick(['', undefined, null]) : legacyCore.ihnHashString(JSON.stringify(pages));
            const snapshot = { contentHash, kind: 'snapshot', pages, structure: { v: 1 }, name: 'doc' };
            if (random() < 0.03) snapshot.pages = pick([undefined, null, 'x']);
            seen.push(contentHash);
            if (random() < 0.8) assert.equal(old.remember(snapshot) === null, now.remember(snapshot) === null, 'remember null');
            const peer = random() < 0.1 ? pick([undefined, null, {}])
                : { lastAckedHash: pick(seen.concat(['', undefined, 'zz'])), remoteCurrentHash: pick(seen.concat(['', undefined, 0])) };
            const force = random() < 0.15;
            const expected = old.build(snapshot, peer, force);
            const actual = now.build(snapshot, peer, force);
            assert.equal(JSON.stringify(actual), JSON.stringify(expected), `session ${session} step ${step}`);
            assert.equal(actual === snapshot, expected === snapshot, 'full snapshot is returned by reference');
            if (actual.partial) deltas += 1; else fulls += 1;
        }
    }
    assert.ok(deltas > 50 && fulls > 50, `coverage: ${deltas} deltas, ${fulls} full`);
});

test('deltas are produced for realistic documents and match the original', () => {
    const g = makeGenerator(5);
    const { old, now } = pair(20);
    clearRust();
    let deltas = 0;
    const pages = Array.from({ length: 12 }, (_, n) => page(g, `page-${n}`, 40));
    const snap = list => ({ contentHash: legacyCore.ihnHashString(JSON.stringify(list)), pages: list, structure: {} });
    const base = snap(pages);
    old.remember(base); now.remember(base);
    for (let n = 0; n < 12; n += 1) {
        const next = pages.map((p, i) => (i === n || i === (n + 5) % 12 ? page(g, `page-${i}`, 40) : p));
        const current = snap(next);
        const peer = { lastAckedHash: base.contentHash };
        const expected = old.build(current, peer, false), actual = now.build(current, peer, false);
        assert.equal(JSON.stringify(actual), JSON.stringify(expected));
        if (actual.partial) { deltas += 1; assert.equal(actual.pages.length, 2); assert.equal(actual.baseHash, base.contentHash); }
    }
    assert.equal(deltas, 12);
});

test('unicode, lone surrogates and index-like keys count and compare like the original', () => {
    const { old, now } = pair();
    clearRust();
    const filler = 'é中😀\ud83d x'.repeat(400);
    const mk = (tweak) => ({ pageId: 'u', 10: 1, 2: 2, b: filler, a: [filler, '\ude00'], tweak });
    const list = [mk(0), { pageId: 'v', text: filler }, { pageId: 'w', text: filler }, { pageId: 'x', text: filler }];
    const a = { contentHash: 'h1', pages: list };
    const b = { contentHash: 'h2', pages: [mk(1), ...list.slice(1)] };
    for (const api of [old, now]) { api.remember(a); api.remember(b); }
    const peer = { lastAckedHash: 'h1' };
    assert.equal(JSON.stringify(now.build(b, peer)), JSON.stringify(old.build(b, peer)));
    assert.equal(now.build(b, peer).partial, true);
});

test('missing snapshots are remembered on demand and reset clears the cache', () => {
    const { now } = pair();
    clearRust();
    const list = Array.from({ length: 6 }, (_, n) => ({ pageId: `p${n}`, text: `${n}`.repeat(1500) }));
    const a = { contentHash: 'a', pages: list };
    const b = { contentHash: 'b', pages: [{ ...list[0], text: 'changed' }, ...list.slice(1)] };
    now.remember(a);
    const delta = now.build(b, { lastAckedHash: 'a' }); // b was never remembered explicitly
    assert.equal(delta.partial, true);
    assert.deepEqual(delta.pages.map(p => p.pageId), ['p0']);
    clearRust();
    assert.equal(now.build(b, { lastAckedHash: 'a' }), b, 'base is gone after reset, so the full snapshot is sent');
});

test('without the Rust core the snapshot helpers fail loudly', () => {
    const { now } = pair();
    const broken = load(currentSource, 3, 16_000_000, { InhouseRust: undefined });
    assert.throws(() => broken.remember({ contentHash: 'x', pages: [] }), /Rust core is not loaded/);
    assert.ok(now);
});

test('a snapshot with the same hash but a different page list is sent whole (documented tolerance)', () => {
    const { now } = pair();
    clearRust();
    const list = Array.from({ length: 6 }, (_, n) => ({ pageId: `p${n}`, text: `${n}`.repeat(1500) }));
    now.remember({ contentHash: 'a', pages: list });
    now.remember({ contentHash: 'b', pages: list });
    const other = { contentHash: 'b', pages: list.slice(0, 5) };
    assert.equal(now.build(other, { lastAckedHash: 'a' }), other);
});
