// Micro-benchmark for unit eh-1: legacy JavaScript fingerprints vs the Rust core (best of N runs). Run: node tests/editor-history-fingerprint.bench.mjs
import { readFile } from 'node:fs/promises';
import vm from 'node:vm';
import { legacySource, rng } from './helpers/legacy.mjs';

const wasm = await readFile(new URL('../wasm/inhouse_notes_core.wasm', import.meta.url));
function extractFunction(source, name) {
  const start = source.indexOf(`function ${name}(`);
  let depth = 0;
  for (let index = source.indexOf('{', source.indexOf(')', start)); index < source.length; index++) {
    if (source[index] === '{') depth++; else if (source[index] === '}' && --depth === 0) return source.slice(start, index + 1);
  }
}
const names = ['simpleHash', 'pointFingerprint', 'strokeSyncFingerprint', 'imageSyncFingerprint', 'itemSyncStampFingerprint', 'strokeCollabFingerprint', 'imageCollabFingerprint'];
// Both sides run in the main context: globals inside a vm context go through slow interceptors and would skew the comparison.
const legacy = vm.runInThisContext(`(function () {\n${names.map(name => extractFunction(legacySource('app-v5.js'), name)).join('\n')}\nreturn { ${names.join(',')} };\n})()`);
globalThis.location = new URL('https://n.test/');
const realFetch = globalThis.fetch;
globalThis.fetch = async () => new Response(wasm);
vm.runInThisContext(await readFile(new URL('../rust-core-v5.js', import.meta.url), 'utf8'));
await globalThis.InhouseRust.ready;
globalThis.fetch = realFetch;
const rust = globalThis.InhouseRust.history;

const r = rng(5);
const makeStroke = (id, count) => ({ id: `s${id}`, tool: 'pen', color: '#1a1a1a', width: 2.5, syncStamp: { clock: id, actor: 'a:b' },
  points: Array.from({ length: count }, (_, i) => ({ x: 100 + i * 1.37 + r(), y: 200 + Math.sin(i / 7) * 40 + r(), p: Math.round(r() * 100) / 100 })) });
const clone = value => JSON.parse(JSON.stringify(value));
function time(label, legacyFn, rustFn, runs) {
  for (let i = 0; i < 2; i++) { legacyFn(); rustFn(); }
  const measure = fn => { const times = []; for (let i = 0; i < runs; i++) { const t = process.hrtime.bigint(); fn(); times.push(Number(process.hrtime.bigint() - t) / 1e6); } return Math.min(...times); };
  const a = measure(legacyFn), b = measure(rustFn);
  console.log(`${label.padEnd(58)} legacy ${a.toFixed(3).padStart(9)} ms   rust ${b.toFixed(3).padStart(9)} ms   x${(a / b).toFixed(1)}`);
}
const single = makeStroke(1, 200), page = Array.from({ length: 2000 }, (_, i) => makeStroke(i, 60)), pageCopy = clone(page);
const pageHeavy = Array.from({ length: 300 }, (_, i) => makeStroke(i, 800));
time('one stroke, 200 points (collab fingerprint)', () => legacy.strokeCollabFingerprint(single), () => rust.strokeCollabFingerprint(single), 400);
time('one stroke, 12 points', (s => () => legacy.strokeSyncFingerprint(s))(makeStroke(2, 12)), (s => () => rust.strokeFingerprint(s))(makeStroke(2, 12)), 2000);
time('page: 2000 strokes x 60 points, sync fingerprints', () => page.map(s => legacy.strokeSyncFingerprint(s)), () => rust.strokeFingerprints(page, false), 20);
time('page: 300 strokes x 800 points (240k points)', () => pageHeavy.map(s => legacy.strokeCollabFingerprint(s)), () => rust.strokeFingerprints(pageHeavy, true), 8);
time('remote vs local diff, 2000 equal strokes (no change found)', () => { for (let i = 0; i < 2000; i++) if (legacy.strokeCollabFingerprint(page[i]) !== legacy.strokeCollabFingerprint(pageCopy[i])) return true; },
  () => rust.collabItemsDiffer('stroke', page, pageCopy), 10);
const sourceMap = new Map(page.map(s => [s.id, s])), targetMap = new Map(pageCopy.map(s => [s.id, s]));
time('history delta touched ids, 2000 strokes (cloned snapshots)', () => {
  const touched = new Set();
  sourceMap.forEach((s, id) => { const t = targetMap.get(id); if (!t || legacy.strokeSyncFingerprint(s) !== legacy.strokeSyncFingerprint(t)) touched.add(id); });
  targetMap.forEach((t, id) => { const s = sourceMap.get(id); if (!s || legacy.strokeSyncFingerprint(s) !== legacy.strokeSyncFingerprint(t)) touched.add(id); });
  return touched;
}, () => rust.touchedStrokeIds(sourceMap, targetMap), 10);
const dataUrl = 'data:image/png;base64,' + 'QUJDREVGR0hJSktMTU5PUFFSU1RVVldYWVo'.repeat(30000);
time('simpleHash, 20 chars', () => legacy.simpleHash('page-1700000000000-abc'), () => rust.simpleHash('page-1700000000000-abc'), 20000);
time(`simpleHash, ${(dataUrl.length / 1e6).toFixed(1)} MB data URL`, () => legacy.simpleHash(dataUrl), () => rust.simpleHash(dataUrl), 12);
const image = { id: 'i', src: dataUrl, x: 10.123, y: 20.456, width: 300.2, height: 200.7, rotation: 0.25, syncStamp: { clock: 3, actor: 'a' } };
time('image collab fingerprint with 1 MB data URL', () => legacy.imageCollabFingerprint(image), () => rust.imageCollabFingerprint(image), 12);
