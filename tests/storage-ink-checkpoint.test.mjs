// Differential test: ink page checkpoint packing/unpacking, original JavaScript (git base) vs the Rust-backed glue.
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import vm from 'node:vm';
import {legacySource, rng} from './helpers/legacy.mjs';

const wasm = await readFile(new URL('../wasm/inhouse_notes_core.wasm', import.meta.url));
const rustScope = {WebAssembly, Uint8Array, Uint16Array, Uint32Array, Float32Array, Float64Array, URL, location: new URL('https://notes.test/'),
    fetch: async () => new Response(wasm), console, AbortSignal, AbortController, setTimeout, clearTimeout};
vm.createContext(rustScope);
vm.runInContext(await readFile(new URL('../rust-core-v5.js', import.meta.url), 'utf8'), rustScope);
const rust = rustScope.InhouseRust;
await rust.ready;

// Cut the two functions out of an app-v5.js text and evaluate them with host stubs.
function build(source, {rustApi = rust} = {}) {
    const start = source.indexOf('async function packInkPageForStorage(');
    const end = source.indexOf('async function cloneSanitizedPageForStorage(');
    assert.ok(start > 0 && end > start, 'ink functions not found');
    const code = source.slice(start, end);
    let valid = true;
    const scope = {structuredClone, performance, Number, Object, Array, Float64Array, Uint8Array, Uint32Array, Error, JSON,
        PAGE_CHECKPOINT_IDLE_MS: 1200,
        yieldProcessingForUi: async () => {}, waitForHotInteractionToSettle: async () => true,
        sanitizePageForStorage: page => page,
        cloneSanitizedPageForStorage: async () => { throw new Error('lossless fallback'); },
        getDocumentSessionToken: () => 1, isDocumentSessionTokenValid: () => valid, createAbortError: m => new Error(m),
        stripStoredPageSession: record => { const next = {...record}; delete next.__sessionId; return next; }};
    scope.window = {InhouseRust: rustApi, InhouseDocumentProcessing: {run: async (type, data) => {
        if (type === 'packInkCheckpoint') return {inkCheckpoint: 1, codec: 'raw-v1', rawLength: data.bytes.byteLength, bytes: new Uint8Array(data.bytes)};
        if (type === 'unpackInkCheckpoint') return rustApi.unpackCheckpoint(data);
        throw new Error(type);
    }}};
    vm.createContext(scope);
    vm.runInContext(`${code};this.pack=packInkPageForStorage;this.unpack=unpackInkPageFromStorage;`, scope);
    return {pack: scope.pack, unpack: scope.unpack, invalidate: () => { valid = false; }};
}
const legacy = build(legacySource('app-v5.js'));
const current = build(await readFile(new URL('../app-v5.js', import.meta.url), 'utf8'));

// Order-sensitive, realm-independent encoding that tells -0 from 0 and keeps NaN, undefined and key order.
function canon(value) {
    if (value === undefined) return 'undefined';
    if (value === null) return 'null';
    if (typeof value === 'number') return Object.is(value, -0) ? '-0' : String(value);
    if (typeof value === 'string') return JSON.stringify(value);
    if (typeof value === 'boolean') return String(value);
    if (value instanceof ArrayBuffer) return `buf(${Buffer.from(value).toString('hex')})`;
    if (ArrayBuffer.isView(value)) return `${value.constructor.name}(${Buffer.from(value.buffer, value.byteOffset, value.byteLength).toString('hex')})`;
    if (Array.isArray(value)) return `[${Array.from(value, canon).join(',')}]`;
    return `{${Object.keys(value).map(key => `${JSON.stringify(key)}:${canon(value[key])}`).join(',')}}`;
}
const outcome = async work => { try { return 'ok ' + canon(await work()); } catch (error) { return 'error ' + error.message; } };

const sample = (random, list) => list[Math.floor(random() * list.length)];
const SPECIAL = [-0, 0, NaN, Infinity, -Infinity, 5e-324, 1.7976931348623157e308, -1.5, 123456789.123456789];
function number(random) { return random() < 0.1 ? sample(random, SPECIAL) : (random() - 0.5) * 2000 * (random() < 0.5 ? 1 : 1e-6); }
function randomPoint(random, hostile) {
    const roll = random();
    if (hostile && roll < 0.04) return sample(random, [null, undefined, 7, 'x', [], [1, 2], () => 1]);
    const point = {x: number(random), y: number(random)};
    if (random() < 0.6) point.p = random() < 0.1 ? sample(random, SPECIAL) : random();
    if (hostile) {
        const extra = random();
        if (extra < 0.03) point.tiltX = random();
        else if (extra < 0.05) point.p = undefined;
        else if (extra < 0.07) point.x = String(point.x);
        else if (extra < 0.09) point.y = null;
        else if (extra < 0.10) point.p = '0.5';
        else if (extra < 0.11) delete point.x;
        else if (extra < 0.12) { const {x, y, p} = point; return p === undefined ? {y, x} : {p, y, x}; }
    }
    return point;
}
function randomStroke(random, hostile, id) {
    const stroke = {id: `s${id}`, tool: sample(random, ['pen', 'highlighter']), color: '#123456', width: random() * 5};
    if (random() < 0.3) stroke.nested = {a: [1, 2, {b: random()}], when: null};
    const kind = random();
    const length = Math.floor(random() * (random() < 0.1 ? 600 : 30));
    if (hostile && kind < 0.04) stroke.points = sample(random, [undefined, null, 'abc', {inkRange: [0, 0]}, 5]);
    else if (hostile && kind < 0.06) { stroke.points = new Array(length + 2); for (let i = 0; i < length; i++) stroke.points[i] = randomPoint(random, hostile); }
    else stroke.points = Array.from({length}, () => randomPoint(random, hostile));
    return stroke;
}
function randomPage(seed, hostile) {
    const random = rng(seed);
    const page = {pageId: `p${seed}`, images: [], width: 800, templateKind: 'lines', sidePanel: random() < 0.2 ? {kind: 'x'} : null};
    page.strokes = Array.from({length: Math.floor(random() * 12)}, (_, i) => randomStroke(random, hostile, i));
    page.legacyCoverStrokes = random() < 0.5 ? null : Array.from({length: Math.floor(random() * 4)}, (_, i) => randomStroke(random, hostile, 100 + i));
    if (random() < 0.05) page.strokes = 'not an array';
    return page;
}
const never = () => false;
const packOutcome = (impl, page) => outcome(async () => impl.pack(structuredClone(page), 1, never, true));

test('pack: fixtures match the original, including -0, NaN and fallback strokes', async () => {
    const fixtures = [
        {strokes: [], legacyCoverStrokes: null},
        {strokes: [{id: 'a', points: []}], legacyCoverStrokes: []},
        {strokes: [{id: 'a', points: [{x: -0, y: 2, p: 0}, {x: 1.123456789012345, y: 400}, {x: -15, y: 22, p: 0.123456789}]},
            {id: 'b', points: [{x: 1, y: 2, p: 0.6, tiltX: 7}]}, {id: 'c', points: []}],
         legacyCoverStrokes: [{id: 'cover', points: [{x: 2, y: 3, p: 0.7}]}]},
        {strokes: [{id: 'n', points: [{x: NaN, y: Infinity, p: -Infinity}, {x: 5e-324, y: -0}]}], legacyCoverStrokes: null},
        {strokes: [{id: 'u', points: [{x: 1, y: 2, p: undefined}]}, {id: 'k', points: [{x: 1, y: 2}]}], legacyCoverStrokes: null},
        {strokes: [{id: 's', points: 'abc'}, {id: 'o', points: {inkRange: [4, 4]}}, {id: 'z', points: undefined}], legacyCoverStrokes: null},
    ];
    for (const [index, page] of fixtures.entries()) {
        const expected = await packOutcome(legacy, page);
        assert.equal(await packOutcome(current, page), expected, `fixture ${index}`);
    }
});
test('pack: seeded random pages match the original byte for byte', async () => {
    for (let seed = 1; seed <= 1500; seed++) {
        const page = randomPage(seed, seed % 2 === 0);
        assert.equal(await packOutcome(current, page), await packOutcome(legacy, page), `seed ${seed}`);
    }
});
test('pack: a stale session returns null at every check of the original', async () => {
    const page = {strokes: Array.from({length: 4}, (_, i) => ({id: `s${i}`, points: [{x: i, y: 1, p: 0.5}, {x: 2, y: 3}]})), legacyCoverStrokes: [{id: 'c', points: [{x: 1, y: 1}]}]};
    // isStale is consulted once per stroke (5), once after the loops and once before returning.
    for (let limit = 0; limit <= 7; limit++) for (const impl of [legacy, current]) {
        let calls = 0;
        const result = await impl.pack(structuredClone(page), 1, () => ++calls > limit, true);
        assert.equal(result === null, limit < 7, `${impl === legacy ? 'legacy' : 'current'} limit ${limit}`);
    }
});

function checkpointRecord(words, strokes, cover) {
    const bytes = new Uint8Array(words.buffer.slice(words.byteOffset, words.byteOffset + words.byteLength));
    return {pageId: 'u', __sessionId: 1, strokes, legacyCoverStrokes: cover, __inkCheckpoint: {inkCheckpoint: 1, codec: 'raw-v1', rawLength: bytes.length, bytes}};
}
const unpackOutcome = (impl, record) => outcome(() => impl.unpack(record));

test('unpack: fixtures and error messages match the original', async () => {
    const words = Float64Array.from([-0, 2, 0, 1, 1.5, 400, 0, 0, NaN, Infinity, 0.25, 1, 7, 8, 9, 2]);
    const cases = [
        [[{id: 'a', points: {inkRange: [0, 3]}}], null],
        [[{id: 'a', points: {inkRange: [0, 0]}}, {id: 'b', points: {inkRange: [12, 0]}}], [{points: {inkRange: [4, 2]}}]],
        [[{id: 'a', points: {inkRange: [0, 4]}}], null],
        [[{id: 'a', points: {inkRange: [0, 5]}}], null],
        [[{id: 'a', points: {inkRange: [2, 1]}}], null],
        [[{id: 'a', points: {inkRange: [-4, 1]}}], null],
        [[{id: 'a', points: {inkRange: [0, 1.5]}}], null],
        [[{id: 'a', points: {inkRange: ['0', 1]}}], null],
        [[{id: 'a', points: {inkRange: [NaN, 1]}}], null],
        [[{id: 'a', points: {inkRange: [0]}}], null],
        [[{id: 'a', points: {inkRange: []}}], null],
        [[{id: 'a', points: {inkRange: [0, 1]}}, {id: 'b', points: {inkRange: [0, 9]}}], null],
        [[{id: 'a', points: {inkRange: [0, 4]}}, {id: 'b', points: {inkRange: [0, 9]}}], null],
        [[{id: 'a', points: [{x: 1, y: 2}]}, {id: 'b'}, {id: 'c', points: null}], 'x'],
        [[{id: 'a', points: {inkRange: [0, 2]}, extra: {k: 1}}], undefined],
    ];
    for (const [index, [strokes, cover]] of cases.entries()) {
        const make = () => checkpointRecord(words, structuredClone(strokes), cover === 'x' ? undefined : cover && structuredClone(cover));
        assert.equal(await unpackOutcome(current, make()), await unpackOutcome(legacy, make()), `case ${index}`);
    }
    for (const record of [null, undefined, {strokes: []}, {__inkCheckpoint: {inkCheckpoint: 2, bytes: new Uint8Array(8)}}, {__inkCheckpoint: {inkCheckpoint: 1, bytes: [1]}}]) {
        assert.equal(await unpackOutcome(current, structuredClone(record)), await unpackOutcome(legacy, structuredClone(record)));
    }
});
test('unpack: seeded random pages and corrupt ranges match the original', async () => {
    for (let seed = 1; seed <= 3000; seed++) {
        const random = rng(seed * 7919);
        const total = Math.floor(random() * 80);
        const words = new Float64Array(total * 4);
        for (let i = 0; i < words.length; i++) words[i] = i % 4 === 3 ? sample(random, [0, 0, 0, 1, 1, 1, 1, random() < 0.02 ? 2 : 0, -0]) : number(random);
        const strokes = [], cover = [];
        let cursor = 0;
        for (let remaining = Math.floor(random() * 10); remaining > 0; remaining--) {
            const count = Math.min(Math.floor(random() * 12), total - cursor / 4);
            let range = [cursor, count]; cursor += count * 4;
            if (random() < 0.08) range = sample(random, [[cursor + 4, 1], [-4, 1], [1, 1], [0, total + 1], [0.5, 1], [0, -1], [NaN, 0], [Infinity, 0],
                ['0', 1], [0, '1'], [null, 1], [9007199254740992, 0], [0, 9007199254740992], [-0, 0], [total * 4, 0]]);
            (random() < 0.3 ? cover : strokes).push({id: `s${remaining}`, color: 'c', points: {inkRange: range}});
        }
        const make = () => checkpointRecord(words, structuredClone(strokes), cover.length ? structuredClone(cover) : null);
        assert.equal(await unpackOutcome(current, make()), await unpackOutcome(legacy, make()), `seed ${seed}`);
    }
});
test('unpack: a document change while decoding aborts after the codec step', async () => {
    const words = Float64Array.from({length: 4096 * 4}, (_, i) => i % 4 === 3 ? 0 : i);
    const record = () => checkpointRecord(words, [{id: 'a', points: {inkRange: [0, 4096]}}], null);
    for (const impl of [legacy, current]) {
        const target = build(impl === legacy ? legacySource('app-v5.js') : await readFile(new URL('../app-v5.js', import.meta.url), 'utf8'));
        target.invalidate();
        assert.equal(await unpackOutcome(target, record()), 'error Document changed while decoding ink');
    }
});

test('pack and unpack round-trip arbitrary pages through the Rust-backed glue', async () => {
    for (let seed = 5000; seed < 5200; seed++) {
        const page = randomPage(seed, false);
        if (!Array.isArray(page.strokes)) page.strokes = [];
        const packed = await current.pack(structuredClone(page), 1, never, true);
        const restored = await current.unpack({...packed, __sessionId: 1});
        assert.equal(canon(restored.strokes), canon(page.strokes), `seed ${seed}`);
        assert.equal(canon(restored.legacyCoverStrokes), canon(page.legacyCoverStrokes), `seed ${seed}`);
    }
});

// Without WebAssembly an existing offline file must still be readable (the checkpoint codec keeps a JS decoder for the same reason).
const noWasmScope = {WebAssembly, Uint8Array, Uint16Array, Uint32Array, Float32Array, Float64Array, URL, location: new URL('https://notes.test/'),
    fetch: async () => { throw new Error('blocked'); }, console, AbortSignal, AbortController, setTimeout, clearTimeout};
vm.createContext(noWasmScope);
vm.runInContext(await readFile(new URL('../rust-core-v5.js', import.meta.url), 'utf8'), noWasmScope);
test('unpack without WebAssembly still decodes and rejects like the original; packing refuses loudly', async () => {
    assert.equal(await noWasmScope.InhouseRust.ready, false);
    const offline = build(await readFile(new URL('../app-v5.js', import.meta.url), 'utf8'), {rustApi: noWasmScope.InhouseRust});
    for (let seed = 1; seed <= 400; seed++) {
        const random = rng(seed * 104729);
        const words = new Float64Array(Math.floor(random() * 20) * 4);
        for (let i = 0; i < words.length; i++) words[i] = i % 4 === 3 ? sample(random, [0, 1, 1, random() < 0.1 ? 3 : 0]) : number(random);
        const strokes = Array.from({length: 1 + Math.floor(random() * 4)}, (_, i) => ({id: i, points: {inkRange:
            random() < 0.15 ? sample(random, [[1, 1], [0, 99], [-4, 0], [NaN, 1], ['0', 1]]) : [Math.floor(random() * 4) * 4, Math.floor(random() * 3)]}}));
        const make = () => checkpointRecord(words, structuredClone(strokes), null);
        assert.equal(await unpackOutcome(offline, make()), await unpackOutcome(legacy, make()), `seed ${seed}`);
    }
    const message = await offline.pack({strokes: [{id: 'a', points: [{x: 1, y: 2}]}], legacyCoverStrokes: null}, 1, never, true).then(() => null, error => error.message);
    assert.match(message, /Rust core is required/);
});
