// Micro-benchmark: ink page checkpoint packing/unpacking, original JavaScript vs the Rust-backed glue.
// Run: node tests/storage-ink-bench.mjs [points]
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
const never = () => false;
const legacy = build(legacySource("app-v5.js"));
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


function page(points) {
    let state = 12345; const random = () => { state = (Math.imul(state, 1664525) + 1013904223) >>> 0; return state / 4294967296; };
    const strokes = []; let left = points;
    for (let id = 0; left > 0; id++) {
        const length = Math.min(left, 20 + Math.floor(random() * 180)); left -= length;
        strokes.push({id: `s${id}`, tool: 'pen', color: '#123456', width: 1.5, points: Array.from({length}, () => random() < 0.8
            ? {x: random() * 800, y: random() * 1100, p: random()} : {x: random() * 800, y: random() * 1100})});
    }
    return {pageId: 'bench', strokes, legacyCoverStrokes: null, images: []};
}
const median = values => values.sort((a, b) => a - b)[values.length >> 1];
const stable = process.argv[2] ? [Number(process.argv[2])] : [5000, 50000, 250000];
for (const points of stable) {
    const source = page(points), runs = points > 100000 ? 9 : 25;
    const timings = {legacyPack: [], rustPack: [], legacyUnpack: [], rustUnpack: []};
    for (let run = 0; run < runs + 3; run++) {
        const record = run < 3 ? null : timings;
        const copies = [structuredClone(source), structuredClone(source)];
        let started = performance.now();
        const a = await legacy.pack(copies[0], 1, never, true);
        const t1 = performance.now() - started; started = performance.now();
        const b = await current.pack(copies[1], 1, never, true);
        const t2 = performance.now() - started;
        assert.equal(canon(a), canon(b));
        started = performance.now(); await legacy.unpack({...a, __sessionId: 1}); const t3 = performance.now() - started;
        started = performance.now(); await current.unpack({...b, __sessionId: 1}); const t4 = performance.now() - started;
        if (record) { timings.legacyPack.push(t1); timings.rustPack.push(t2); timings.legacyUnpack.push(t3); timings.rustUnpack.push(t4); }
    }
    const m = Object.fromEntries(Object.entries(timings).map(([k, v]) => [k, median(v)]));
    console.log(`${points} points: pack ${m.legacyPack.toFixed(2)} -> ${m.rustPack.toFixed(2)} ms (${(m.legacyPack / m.rustPack).toFixed(2)}x), `
        + `unpack ${m.legacyUnpack.toFixed(2)} -> ${m.rustUnpack.toFixed(2)} ms (${(m.legacyUnpack / m.rustUnpack).toFixed(2)}x)`);
}
