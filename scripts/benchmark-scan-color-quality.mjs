// Micro-benchmark: capture-quality readiness, original JavaScript vs the Rust core, on a realistic
// 1100 px frozen photograph. Run: node scripts/benchmark-scan-color-quality.mjs [runs]
import {readFile} from 'node:fs/promises';
import vm from 'node:vm';
import {loadLegacy, rng} from '../tests/helpers/legacy.mjs';

const wasm = await readFile(new URL('../wasm/inhouse_notes_core.wasm', import.meta.url));
const rustScope = {WebAssembly, Uint8Array, Uint16Array, Float32Array, URL, location: new URL('https://notes.test/scanner/'),
    fetch: async () => new Response(wasm), console, AbortSignal, AbortController, setTimeout, clearTimeout, TextEncoder, TextDecoder};
vm.createContext(rustScope);
vm.runInContext(await readFile(new URL('../rust-core-v5.js', import.meta.url), 'utf8'), rustScope);
await rustScope.InhouseRust.ready;

const width = 1100, height = 825, random = rng(7);
const rgba = new Uint8ClampedArray(width * height * 4);
for (let i = 0; i < width * height; i++) { const v = 225 + random() * 6; rgba[i * 4] = v; rgba[i * 4 + 1] = v; rgba[i * 4 + 2] = v - 8; rgba[i * 4 + 3] = 255; }
const quad = [{x: .08, y: .05}, {x: .92, y: .05}, {x: .92, y: .95}, {x: .08, y: .95}];
const frameEdges = quad.map((a, side) => { const b = quad[(side + 1) % 4]; return Array.from({length: 33}, (_, i) => ({x: a.x + (b.x - a.x) * i / 32, y: a.y + (b.y - a.y) * i / 32})); });
const dot = (cx, cy) => { for (let y = Math.round(cy) - 2; y <= Math.round(cy) + 2; y++) for (let x = Math.round(cx) - 2; x <= Math.round(cx) + 2; x++) {
    const k = Math.max(0, 1 - Math.hypot(x - cx, y - cy) / 2.2) * .55, at = (y * width + x) * 4; for (let c = 0; c < 3; c++) rgba[at + c] *= 1 - k; } };
for (let r = 1; r < 54; r++) for (let c = 1; c < 36; c++) dot(width * .08 + width * .84 * c / 36 + random() * .4, height * .05 + height * .9 * r / 54 + random() * .4);
for (let i = 0; i < 4; i++) { const a = quad[i], b = quad[(i + 1) % 4]; for (let s = 0; s <= 1200; s++) { const x = Math.round((a.x + (b.x - a.x) * s / 1200) * width), y = Math.round((a.y + (b.y - a.y) * s / 1200) * height); const at = (y * width + x) * 4; rgba[at] = rgba[at + 1] = rgba[at + 2] = 60; } }
const stubDocument = {createElement: () => ({width: 0, height: 0, getContext: () => ({drawImage() {}, getImageData: () => ({data: rgba})})})};
const source = {width, height};
const detection = {quad, frameEdges};
const legacy = loadLegacy('scanner/processing/capture-quality.js', {document: stubDocument}).ScannerPro.CaptureQuality;
const scope = {console, setTimeout, clearTimeout, URL, Float32Array, Float64Array, Uint8Array, Math, JSON, Map, Set, Promise, document: stubDocument, InhouseRust: rustScope.InhouseRust};
scope.window = scope.self = scope.globalThis = scope;
vm.createContext(scope);
vm.runInContext(await readFile(new URL('../scanner/processing/capture-quality.js', import.meta.url), 'utf8'), scope);
const rust = scope.ScannerPro.CaptureQuality;

const runs = Number(process.argv[2]) || 15;
const median = values => values.slice().sort((a, b) => a - b)[Math.floor(values.length / 2)];
function time(label, fn) {
    for (let i = 0; i < 3; i++) fn();
    const samples = [];
    for (let i = 0; i < runs; i++) { const t = performance.now(); fn(); samples.push(performance.now() - t); }
    console.log(`${label}: median ${median(samples).toFixed(2)} ms  min ${Math.min(...samples).toFixed(2)} ms`);
    return median(samples);
}
const expected = legacy.inspectReadiness(source, detection);
console.log(`result: ${expected.reason}, matched ${expected.grid?.matched}, cells ${expected.grid?.cells}`);
if (JSON.stringify(rust.inspectReadiness(source, detection)) !== JSON.stringify(expected)) throw new Error('Rust and JavaScript disagree');
const raster = (() => { const gray = new Float32Array(width * height); for (let i = 0; i < gray.length; i++) gray[i] = rgba[i * 4] * .2126 + rgba[i * 4 + 1] * .7152 + rgba[i * 4 + 2] * .0722; return {width, height, gray}; })();
const a = time('acutance   JS  ', () => legacy.inspect(source, detection, raster)), b = time('acutance   Rust', () => rust.inspect(source, detection, raster));
const c = time('lattice    JS  ', () => legacy.inspectGrid(source, detection, raster)), d = time('lattice    Rust', () => rust.inspectGrid(source, detection, raster));
const e = time('readiness  JS  ', () => legacy.inspectReadiness(source, detection)), f = time('readiness  Rust', () => rust.inspectReadiness(source, detection));
console.log(`speedup: acutance x${(a / b).toFixed(1)}, lattice x${(c / d).toFixed(1)}, readiness x${(e / f).toFixed(1)}`);
