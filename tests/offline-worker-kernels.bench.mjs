// Micro-benchmark: former worker loops (copied verbatim from the pre-port worker) vs the Rust kernels.
// Run: node tests/offline-worker-kernels.bench.mjs
import vm from 'node:vm';
import {readFile} from 'node:fs/promises';
import {rng} from './helpers/legacy.mjs';
const wasm = await readFile(new URL('../wasm/inhouse_notes_core.wasm', import.meta.url));
const scope = {WebAssembly, Uint8Array, Uint16Array, Float32Array, URL, TextEncoder, TextDecoder, DOMException, console, AbortSignal, AbortController, setTimeout, clearTimeout,
    location: new URL('https://notes.test/'), fetch: async () => new Response(wasm)};
vm.createContext(scope);
vm.runInContext(await readFile(new URL('../rust-core-v5.js', import.meta.url), 'utf8'), scope);
await scope.InhouseRust.ready;
const R = scope.InhouseRust;
const legacyEncode = bytes => { let binary = ''; for (let o = 0; o < bytes.length; o += 16384) binary += String.fromCharCode(...bytes.subarray(o, o + 16384)); return btoa(binary); };
const legacyDecode = payload => Uint8Array.from(atob(payload), c => c.charCodeAt(0));
const legacyHash = json => { let hash = 0; for (let i = 0; i < json.length; i++) hash = ((hash << 5) - hash + json.charCodeAt(i)) | 0;
    return `${Math.abs(hash).toString(36).slice(0, 8)}:${json.length.toString(36)}`; };
function time(fn, runs = 7) { fn(); const t = []; for (let i = 0; i < runs; i++) { const s = performance.now(); fn(); t.push(performance.now() - s); } t.sort((a, b) => a - b); return t[t.length >> 1]; }
const next = rng(1), mb = n => n * 1024 * 1024;
const rows = [];
for (const size of [mb(1), mb(8), mb(32)]) {
    const bytes = Uint8Array.from({length: size}, () => next() * 256);
    const text = Buffer.from(bytes).toString('base64');
    const json = JSON.stringify({pages: Array.from({length: size / 400 | 0}, (_, i) => ({i, strokes: [{x: i * 1.5, y: i % 97, color: '#ffde00'}]}))});
    rows.push([`b64 encode ${size >> 20} MB`, time(() => legacyEncode(bytes)), time(() => R.b64Encode(bytes))]);
    rows.push([`b64 decode ${size >> 20} MB`, time(() => legacyDecode(text)), time(() => R.b64Decode(text))]);
    rows.push([`timelineHash ${(json.length / 1048576).toFixed(1)} MB json`, time(() => legacyHash(json)), time(() => R.timelineHash(json))]);
}
console.log('kernel'.padEnd(30), 'JS ms'.padStart(9), 'Rust ms'.padStart(9), 'speedup'.padStart(8));
for (const [name, js, rust] of rows) console.log(name.padEnd(30), js.toFixed(2).padStart(9), rust.toFixed(2).padStart(9), `${(js / rust).toFixed(1)}x`.padStart(8));
