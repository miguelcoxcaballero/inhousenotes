// Differential test: the Rust canonical hash / stable stringify / structure and field normalizers
// against the original JavaScript (collaboration-core-v5.js at the port base commit).
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import test from 'node:test';
import './helpers/collab-rust.mjs';
import { makeGenerator } from './helpers/collab-gen.mjs';
import { loadLegacy } from './helpers/legacy.mjs';

const require = createRequire(import.meta.url);
const rust = require('../collaboration-core-v5.js');
const legacy = loadLegacy('collaboration-core-v5.js');

const same = (actual, expected, label) => assert.equal(JSON.stringify(actual), JSON.stringify(expected), label);

test('hash string matches the original for random and surrogate strings', () => {
    const { random, int, string } = makeGenerator(11);
    const fixed = ['', 'a', 'hello', '\ud83d', '\ude00', '😀', 'a\ud83db', 'é中', '\u0000', 'x'.repeat(5000), `${'ab\ud83d'.repeat(333)}z`];
    for (const text of fixed) assert.equal(rust.ihnHashString(text), legacy.ihnHashString(text), JSON.stringify(text.slice(0, 20)));
    for (const value of [undefined, null, 0, false, NaN, 5, 1.5, true, [], ['a'], {}, 'x']) assert.equal(rust.ihnHashString(value), legacy.ihnHashString(value));
    for (let n = 0; n < 4000; n += 1) {
        let text = string(40);
        if (random() < .3) text += String.fromCharCode(int(65536));
        if (random() < .1) text = text.repeat(1 + int(60));
        assert.equal(rust.ihnHashString(text), legacy.ihnHashString(text), JSON.stringify(text.slice(0, 40)));
    }
});

test('stable stringify matches the original on JSON data', () => {
    const { plain } = makeGenerator(21);
    const fixed = [null, undefined, 0, -0, 1.5, 'x', '', true, [], {}, [undefined, null], { a: undefined, b: 1 }, { b: 1, a: 2, 10: 'x', 2: 'y', c: [3, { z: 1, y: 2 }] },
        [NaN, Infinity, -Infinity], { n: NaN }, 1e21, 1e-7, 123456789012345680000, { '\ud83d': 1, 'ﬁ': 2, '': 3 }, { __proto__: null, k: 1 }, '\ud800', { '\ud800': 1 }];
    for (const value of fixed) assert.equal(rust.ihnStableStringify(value), legacy.ihnStableStringify(value), JSON.stringify(value));
    assert.equal(rust.ihnStableStringify(() => 1), legacy.ihnStableStringify(() => 1));
    assert.equal(rust.ihnStableStringify(Symbol('s')), legacy.ihnStableStringify(Symbol('s')));
    for (let n = 0; n < 5000; n += 1) {
        const value = plain(0, true);
        assert.equal(rust.ihnStableStringify(value), legacy.ihnStableStringify(value), JSON.stringify(value));
    }
});

test('number formatting matches JSON.stringify on millions of doubles and exact decimal ties', () => {
    const { random, int } = makeGenerator(71);
    const f64 = new Float64Array(1), u32 = new Uint32Array(f64.buffer);
    const batch = [];
    const flush = () => { const text = JSON.stringify(batch); assert.equal(rust.ihnStableStringify(batch), text); batch.length = 0; };
    const add = value => { if (Number.isFinite(value)) batch.push(value); if (batch.length >= 4000) flush(); };
    for (let n = 0; n < 300000; n += 1) { u32[0] = int(2 ** 32); u32[1] = int(2 ** 32); add(f64[0]); }
    // Dyadic fractions: the exact value is a decimal that ends in 5, so shortest digits can tie (ES rounds to even).
    for (let n = 0; n < 300000; n += 1) {
        const bits = 40 + int(14);
        add((random() < .5 ? -1 : 1) * Math.floor(random() * 2 ** bits) / 2 ** (1 + int(30)));
    }
    for (let n = 0; n < 100000; n += 1) add(Math.round(random() * 1e7) / 10 ** int(9));
    for (let n = 0; n < 100000; n += 1) add(random() * 10 ** (int(60) - 30));
    for (let n = 0; n < 50000; n += 1) add((int(2 ** 30) * 2 ** 30 + int(2 ** 30)) * 2 ** (int(40) - 25));
    flush();
});

test('structure meta normalizer and hash match the original', () => {
    const gen = makeGenerator(31);
    same(rust.ihnNormalizeStructureMeta(), legacy.ihnNormalizeStructureMeta());
    for (const meta of [null, undefined, {}, 3, 'x', []]) {
        same(rust.ihnNormalizeStructureMeta(meta), legacy.ihnNormalizeStructureMeta(meta));
        assert.equal(rust.ihnStructureHash(meta), legacy.ihnStructureHash(meta));
    }
    for (let n = 0; n < 3000; n += 1) {
        const ids = gen.pageIdSet();
        const meta = gen.structure(ids);
        const given = gen.random() < .5 ? ids : gen.pick([null, undefined, [], 'abc', 5, ['p1', 'p1', '', null, 7]]);
        same(rust.ihnNormalizeStructureMeta(meta, given), legacy.ihnNormalizeStructureMeta(meta, given), `normalize ${n}`);
        assert.equal(rust.ihnStructureHash(meta, given), legacy.ihnStructureHash(meta, given), `hash ${n}`);
        assert.equal(rust.ihnStructureHash(meta), legacy.ihnStructureHash(meta), `hash-null ${n}`);
        // Results stay usable as inputs of the (still JavaScript) merge code.
        const normalized = rust.ihnNormalizeStructureMeta(meta, given);
        assert.equal(rust.ihnStructureHash(normalized), legacy.ihnStructureHash(normalized));
    }
});

test('field meta normalizer and hash match the original', () => {
    const gen = makeGenerator(41);
    for (const meta of [null, undefined, {}, 3, 'x', [], { fields: { a: { value: undefined, stamp: { clock: 1 } } } }].slice(0, 6)) {
        same(rust.ihnNormalizeFieldMeta(meta), legacy.ihnNormalizeFieldMeta(meta));
        assert.equal(rust.ihnFieldMetaHash(meta), legacy.ihnFieldMetaHash(meta));
    }
    for (let n = 0; n < 4000; n += 1) {
        const meta = gen.fields();
        same(rust.ihnNormalizeFieldMeta(meta), legacy.ihnNormalizeFieldMeta(meta), `normalize ${n}`);
        assert.equal(rust.ihnFieldMetaHash(meta), legacy.ihnFieldMetaHash(meta), `hash ${n}`);
    }
});

test('canonical document hash matches the original on fixtures', () => {
    const page = { pageId: 'p1', strokes: [{ id: 's2', points: [{ x: 1.5, y: 2 }] }, { id: 's1', preview: 'drop', points: [] }], images: [], deletedStrokeIds: ['s9', 's9', ''],
        deletedStrokeStamps: { s9: { clock: 4, actor: 'a' } }, backgroundSource: 'template', pageWidth: 210, pageHeight: 297 };
    const structure = legacy.ihnNormalizeStructureMeta(null, ['p1']);
    const fields = legacy.ihnRecordFieldValue(null, 'title', { b: 1, a: [1, 2] }, 'actor', 5);
    for (const args of [
        [[], null, null, '', null], [undefined, undefined, undefined, undefined, undefined], [[page], structure, { rows: 3 }, 'Doc', fields],
        [[page, null, 4, 'x'], JSON.parse(JSON.stringify(structure)), 0, 'é😀', null], ['not an array', structure, null, null, fields],
        [[{ ...page, sidePanel: { title: 'T', dateKeys: ['2024-01-01', 5] } }], structure, {}, 'N', fields]
    ]) assert.equal(rust.ihnCanonicalDocumentHash(...args), legacy.ihnCanonicalDocumentHash(...args));
    const json = JSON.stringify([page]);
    assert.equal(rust.ihnCanonicalDocumentHashJson(json, structure, null, 'Doc', fields), legacy.ihnCanonicalDocumentHash([page], structure, null, 'Doc', fields));
});

test('canonical document hash matches the original on seeded random documents', () => {
    const gen = makeGenerator(51);
    for (let n = 0; n < 4000; n += 1) {
        const ids = gen.pageIdSet().slice(0, 1 + gen.int(5));
        const pages = gen.random() < .05 ? gen.pick([null, undefined, 'x', {}, 4]) : ids.map(id => gen.page(id));
        const structure = gen.structure(ids);
        const calendar = gen.random() < .5 ? null : gen.plain(0, true);
        const name = gen.pick(['', 'Doc', undefined, null, 0, 'é😀', gen.string(10), 5, ['a']]);
        const fieldMeta = gen.random() < .2 ? undefined : gen.fields();
        const rustHash = rust.ihnCanonicalDocumentHash(pages, structure, calendar, name, fieldMeta);
        const legacyHash = legacy.ihnCanonicalDocumentHash(pages, structure, calendar, name, fieldMeta);
        assert.equal(rustHash, legacyHash, `document ${n}`);
    }
});

test('canonical document hash matches the original on realistic large pages', () => {
    const gen = makeGenerator(61);
    const strokes = Array.from({ length: 400 }, (_, i) => ({ id: `stroke-${(i * 7919) % 400}`, color: '#1a1a1a', width: 2.25, tool: 'pen',
        updatedClock: i, updatedBy: 'peer-a', points: Array.from({ length: 80 }, () => ({ x: gen.random() * 595, y: gen.random() * 842, p: gen.random() })) }));
    const pages = ['p1', 'p2', 'p3'].map(pageId => ({ pageId, strokes: strokes.map(s => ({ ...s, id: `${pageId}-${s.id}` })), images: [], deletedStrokeIds: [], deletedStrokeStamps: {},
        backgroundSource: 'template', templateKind: 'lined', pageWidth: 595.28, pageHeight: 841.89, sidePanel: null }));
    const structure = legacy.ihnNormalizeStructureMeta(null, ['p1', 'p2', 'p3']);
    assert.equal(rust.ihnCanonicalDocumentHash(pages, structure, null, 'Big', null), legacy.ihnCanonicalDocumentHash(pages, structure, null, 'Big', null));
});

test('the Rust-backed core refuses to run without the WASM module', () => {
    const real = globalThis.InhouseRust;
    try {
        globalThis.InhouseRust = undefined;
        assert.throws(() => rust.ihnHashString('x'), /Rust core is not loaded/);
        assert.throws(() => rust.ihnCanonicalDocumentHash([], null, null, '', null), /Rust core is not loaded/);
        globalThis.InhouseRust = { available: () => false, collab: real.collab };
        assert.throws(() => rust.ihnStableStringify({}), /Rust core is not loaded/);
    } finally { globalThis.InhouseRust = real; }
});
