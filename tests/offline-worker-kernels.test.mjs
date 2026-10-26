// Differential test for the document-processing worker kernels ported to Rust (offline unit U1):
// base64 (compress / decodeTimeline), timelineHash, the prepared-PNG cache eviction plan and the RGBA split.
// The pre-port worker is loaded from git (tests/helpers/legacy.mjs) and driven through its real onmessage router.
import test from 'node:test';
import assert from 'node:assert/strict';
import vm from 'node:vm';
import {readFile} from 'node:fs/promises';
import {legacySource, rng} from './helpers/legacy.mjs';

const wasm = await readFile(new URL('../wasm/inhouse_notes_core.wasm', import.meta.url));
const rustSource = await readFile(new URL('../rust-core-v5.js', import.meta.url), 'utf8');
const currentSource = await readFile(new URL('../document-processing-worker-v5.js', import.meta.url), 'utf8');
const CACHE_LIMIT = /const PNG_RASTER_CACHE_LIMIT = 32 \* 1024 \* 1024;/;
const SMALL_LIMIT = 'const PNG_RASTER_CACHE_LIMIT = 200000;'; // lets a few thousand operations exercise eviction

/** A worker scope with real CompressionStream/Blob/crypto, a stub pdf-lib and a stub canvas. */
function workerScope({rust, canvas = null}) {
    const scope = {console, setTimeout, clearTimeout, URL, TextEncoder, TextDecoder, Uint8Array, Uint8ClampedArray, Uint16Array,
        Uint32Array, Float32Array, Float64Array, Math, JSON, Map, Set, Promise, WebAssembly, AbortSignal, AbortController,
        Blob, Response, CompressionStream, DecompressionStream, DOMException, crypto: globalThis.crypto, btoa, atob,
        location: {search: '', href: 'https://notes.test/', pathname: '/'}, fetch: async () => new Response(wasm),
        InhouseTimelineCore: {materializeArchive: value => value}, ihnCanonicalDocumentHash: () => 'hash',
        decodeCalls: 0, messages: []};
    scope.window = scope.self = scope.globalThis = scope;
    scope.postMessage = (message, transfers) => scope.messages.push(message);
    scope.importScripts = name => { if (rust && name.startsWith('rust-core-v5.js')) vm.runInContext(rustSource, scope, {filename: 'rust-core-v5.js'}); };
    // pdf-lib stand-in: the decoder yields deterministic samples keyed by the first byte pair of the "PNG".
    scope.PDFLib = {PngEmbedder: {for: async bytes => {
        scope.decodeCalls++;
        const seed = bytes[0] * 256 + bytes[1], next = rng(seed), size = 2000 + Math.floor(next() * 9000);
        const rgbChannel = Uint8Array.from({length: size}, () => next() * 256);
        const alphaChannel = seed % 3 ? Uint8Array.from({length: size / 3 | 0}, () => next() * 256) : undefined;
        return {image: {width: size, height: 1, rgbChannel, alphaChannel}};
    }}};
    if (canvas) scope.OffscreenCanvas = class {
        constructor(width, height) { this.width = width; this.height = height; }
        getContext() { return {setTransform() {}, save() {}, restore() {}, getImageData: (x, y, w, h) => ({data: canvas(w, h)})}; }
    };
    return scope;
}
function loadWorker(source, options) {
    const scope = workerScope(options);
    vm.createContext(scope);
    vm.runInContext(source, scope, {filename: 'worker'});
    return scope;
}
async function call(scope, type, data) {
    scope.messages.length = 0;
    await scope.onmessage({data: {id: 1, type, data}});
    const [message] = scope.messages;
    return message.error !== undefined ? {error: message.error, errorName: message.errorName} : {value: plain(message.value)};
}
// vm objects have their own Object.prototype; re-create plain records so deepStrictEqual compares data only.
const plain = value => value && typeof value === 'object' && !ArrayBuffer.isView(value) ? {...value} : value;
// Engines word the InvalidCharacterError differently (Node: "Invalid character", Chromium: "...not correctly encoded").
// Callers only log it, so the exception name is the contract.
const sameFailure = (actual, expected) => actual.error === undefined ? assert.deepEqual(actual, expected)
    : (assert.equal(typeof expected.error, 'string'), assert.equal(actual.errorName, expected.errorName));
const legacy = options => loadWorker(legacySource('document-processing-worker-v5.js'), {...options, rust: false});
const current = async options => { const scope = loadWorker(currentSource, {...options, rust: true}); assert.equal(await scope.InhouseRust.ready, true); return scope; };

function randomText(next, max) {
    const pieces = ['a', 'b', '{"x":1}', ' ', '\n', '"', 'é', '€', '\u{1F600}', '中', '\u0000', '\\u00e9', 'zzzz', '0123456789'];
    let text = ''; const target = Math.floor(next() * max);
    while (text.length < target) text += pieces[Math.floor(next() * pieces.length)];
    return text;
}
function randomBytes(next, length) { return Uint8Array.from({length}, () => Math.floor(next() * 256)); }

test('Rust core exposes the worker kernels with no host imports', async () => {
    const scope = await current();
    assert.equal(WebAssembly.Module.imports(await WebAssembly.compile(wasm)).length, 0);
    assert.equal(typeof scope.InhouseRust.b64Encode, 'function');
});

test('compress: deflate + base64 output is byte-identical to the former btoa loop', async () => {
    const old = legacy(), now = await current(), next = rng(11);
    const fixtures = ['', '{}', 'x', '{"pages":[]}', 'café \u{1F600}', 'a'.repeat(100000)];
    for (let i = 0; i < 400; i++) fixtures.push(randomText(next, i < 380 ? 400 : 60000));
    for (const json of fixtures) assert.deepEqual(await call(now, 'compress', {json}), await call(old, 'compress', {json}));
    // Incompressible multi-chunk input: the old loop sliced 16 KB, the new one 24 MB.
    const noise = Array.from(randomBytes(next, 300000), c => String.fromCharCode(c)).join('');
    assert.deepEqual(await call(now, 'compress', {json: noise}), await call(old, 'compress', {json: noise}));
});

test('b64Encode equals btoa for every length and for thousands of random buffers', async () => {
    const {InhouseRust} = await current(), next = rng(5);
    for (let length = 0; length < 300; length++) {
        const bytes = randomBytes(next, length);
        assert.equal(InhouseRust.b64Encode(bytes), btoa(String.fromCharCode(...bytes)), `length ${length}`);
    }
    for (let i = 0; i < 2000; i++) {
        const bytes = randomBytes(next, Math.floor(next() * 5000));
        assert.equal(InhouseRust.b64Encode(bytes), Buffer.from(bytes).toString('base64'));
    }
    const big = randomBytes(next, 5 * 1024 * 1024 + 1);
    assert.equal(InhouseRust.b64Encode(big), Buffer.from(big).toString('base64'));
    // A subarray view (non-zero byteOffset) must encode only its own bytes.
    assert.equal(InhouseRust.b64Encode(big.subarray(7, 1000)), Buffer.from(big.subarray(7, 1000)).toString('base64'));
});

function atobOutcome(text) {
    try { return {bytes: Uint8Array.from(atob(text), c => c.charCodeAt(0))}; } catch (error) { return {name: error.name}; }
}
function rustOutcome(rust, text) {
    try { return {bytes: rust.b64Decode(text)}; } catch (error) { return {name: error.name}; }
}

test('b64Decode accepts and rejects exactly what atob does (whitespace, padding, bad characters)', async () => {
    const {InhouseRust} = await current(), next = rng(23);
    const fixtures = ['', 'Zg==', 'Zg=', 'Zg', 'Z', 'Zm9v', ' Zm 9v\r\n', 'Zg == ', 'Zh==', 'Zm9v====', '====', 'Zm9v!', 'Zm-v', 'Zm_v',
        'éabc', '中abc', '\u{1F600}abc', '\ud800abc', 'Zm9v\u000b', 'Zm9v ', 'Z=g=', '=Zg=', 'Zg===', 'Zg=\n=', 'Zg\n==\n', 'A'.repeat(1001), 'A'.repeat(1000)];
    for (const text of fixtures) assert.deepEqual(rustOutcome(InhouseRust, text), atobOutcome(text), JSON.stringify(text));
    const alphabet = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
    const noise = ['=', ' ', '\n', '\t', '\r', '\f', '\v', '-', '_', '!', 'é', '\u{1F600}', '\u0000', '\ud800'];
    for (let i = 0; i < 6000; i++) {
        let text = Buffer.from(randomBytes(next, Math.floor(next() * 90))).toString('base64');
        const edits = Math.floor(next() * 4);
        for (let e = 0; e < edits; e++) {
            const at = Math.floor(next() * (text.length + 1)), roll = next();
            if (roll < .35) text = text.slice(0, at) + noise[Math.floor(next() * noise.length)] + text.slice(at);
            else if (roll < .6) text = text.slice(0, at) + text.slice(at + 1);
            else if (roll < .8) text = text.replace(/=+$/, '');
            else text = text.slice(0, at) + alphabet[Math.floor(next() * 64)] + text.slice(at + 1);
        }
        assert.deepEqual(rustOutcome(InhouseRust, text), atobOutcome(text), JSON.stringify(text));
    }
    const big = Buffer.from(randomBytes(next, 4 * 1024 * 1024)).toString('base64');
    assert.deepEqual(rustOutcome(InhouseRust, big), atobOutcome(big));
});

test('timelineHash is byte-exact against the former 31-multiplier loop, including lone surrogates', async () => {
    const old = legacy(), now = await current(), next = rng(77);
    const fixtures = ['', 'a', 'ab', 'abc', 'abcd', 'abcde', '{"pages":[]}', 'é', '\u{1F600}', '\ud800', '\udc00x', 'x\ud83d', '😀\ud83d', 'a'.repeat(100003),
        '￿'.repeat(5), '{"pages":[{"strokes":[]}],"exportName":"über \u{1F4D3}"}'];
    for (let i = 0; i < 3000; i++) {
        let text = randomText(next, i < 2900 ? 300 : 40000);
        if (i % 7 === 0 && text.length) { const at = Math.floor(next() * text.length); text = text.slice(0, at) + String.fromCharCode(0xD800 + Math.floor(next() * 0x800)) + text.slice(at); }
        fixtures.push(text);
    }
    for (const json of fixtures) assert.deepEqual(await call(now, 'timelineHash', {json}), await call(old, 'timelineHash', {json}), JSON.stringify(json).slice(0, 80));
    const huge = randomText(next, 4e6) + 'z'.repeat(2e6);
    assert.deepEqual(await call(now, 'timelineHash', {json: huge}), await call(old, 'timelineHash', {json: huge}));
});

test('decodeTimeline: same output, same size guard and same errors as before', async () => {
    const old = legacy(), now = await current(), next = rng(3);
    for (let i = 0; i < 60; i++) {
        const json = JSON.stringify({entries: Array.from({length: Math.floor(next() * 30)}, (_, k) => ({k, text: randomText(next, 200)}))});
        const {value: payload} = await call(old, 'compress', {json});
        const expected = await call(old, 'decodeTimeline', {payload});
        assert.equal(expected.value, JSON.stringify(JSON.parse(json)));
        assert.deepEqual(await call(now, 'decodeTimeline', {payload}), expected);
        // whitespace inside the payload and a dropped pad are tolerated by atob; so must Rust
        const loose = payload.replace(/=+$/, '').replace(/(.{40})/g, '$1\n');
        sameFailure(await call(now, 'decodeTimeline', {payload: loose}), await call(old, 'decodeTimeline', {payload: loose}));
    }
    for (const payload of ['', '!!!!', 'Zg=', 'AAAA', 'éééé', 'x'.repeat(48 * 1024 * 1024 + 1)]) {
        sameFailure(await call(now, 'decodeTimeline', {payload}), await call(old, 'decodeTimeline', {payload}));
    }
});

test('prepared-PNG cache: identical hit/miss sequence, rasters and evictions over thousands of operations', async () => {
    const old = loadWorker(legacySource('document-processing-worker-v5.js').replace(CACHE_LIMIT, SMALL_LIMIT), {rust: false});
    const now = loadWorker(currentSource.replace(CACHE_LIMIT, SMALL_LIMIT), {rust: true});
    assert.equal(await now.InhouseRust.ready, true);
    const next = rng(2024), pngs = Array.from({length: 40}, (_, i) => Uint8Array.of(i >> 8, i & 255, 7, 7));
    let misses = 0;
    for (let op = 0; op < 3000; op++) {
        // Zipf-ish: a few hot images, a long cold tail, so both hits and evictions occur.
        const png = pngs[Math.floor(next() ** 2 * pngs.length)];
        const [a, b] = [await old.preparedPng(png), await now.preparedPng(png)];
        assert.deepEqual(plain(b), plain(a), `operation ${op}`);
        assert.equal(now.decodeCalls, old.decodeCalls, `operation ${op}: cache hit/miss diverged`);
        misses = old.decodeCalls;
    }
    assert.ok(misses > 100 && misses < 2900, `eviction must be exercised (${misses} misses of 3000)`);
    // Oversized raster (larger than the whole budget) is returned but never cached, in both.
    const huge = loadWorker(currentSource.replace(CACHE_LIMIT, 'const PNG_RASTER_CACHE_LIMIT = 10;'), {rust: true});
    await huge.InhouseRust.ready;
    await huge.preparedPng(pngs[0]); await huge.preparedPng(pngs[0]);
    assert.equal(huge.decodeCalls, 2);
});

test('PDF raster split: Rust output equals the former JS loop, and a missing core fails loudly', async () => {
    const next = rng(9);
    for (let i = 0; i < 40; i++) {
        const width = 1 + Math.floor(next() * 60), height = 1 + Math.floor(next() * 40), opaque = i % 2 === 0;
        const pixels = Uint8ClampedArray.from({length: width * height * 4}, (_, k) => (k % 4 === 3 && opaque) ? 255 : Math.floor(next() * 256));
        const options = {canvas: () => pixels};
        const expected = await call(legacy(options), 'renderOverlay', {json: '{}', bounds: {x: 0, y: 0, width, height}, scale: 1, pdfRaster: true});
        const actual = await call(await current(options), 'renderOverlay', {json: '{}', bounds: {x: 0, y: 0, width, height}, scale: 1, pdfRaster: true});
        assert.deepEqual(actual, expected);
        assert.equal(actual.value.alphaBytes === null, opaque);
    }
    const broken = loadWorker(currentSource, {rust: false, canvas: () => new Uint8ClampedArray(4)});
    broken.InhouseRust = {ready: Promise.resolve(false), available: () => false};
    const failure = await call(broken, 'renderOverlay', {json: '{}', bounds: {x: 0, y: 0, width: 1, height: 1}, scale: 1, pdfRaster: true});
    assert.match(failure.error, /Rust core is unavailable/);
    for (const [type, data] of [['compress', {json: '{}'}], ['timelineHash', {json: '{}'}], ['decodeTimeline', {payload: 'AAAA'}]])
        assert.match((await call(broken, type, data)).error, /Rust core is unavailable/, type);
});
