// Differential test for unit eh-1: item fingerprints, simpleHash and the stroke delta kernels.
// The pre-port JavaScript (tests/helpers/legacy.mjs) is the oracle; every output must match byte for byte.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import vm from 'node:vm';
import { legacySource, rng } from './helpers/legacy.mjs';
import './helpers/collab-rust.mjs'; // collaboration-core-v5.js hashes in the Rust core (collab area); install the adapter first

const require = createRequire(import.meta.url);
const { ihnStableStringify, ihnNormalizeDeletionStamps } = require('../collaboration-core-v5.js');
const wasm = await readFile(new URL('../wasm/inhouse_notes_core.wasm', import.meta.url));

function extractFunction(source, name) {
  const start = source.indexOf(`function ${name}(`);
  assert.ok(start >= 0, `${name} must exist`);
  let depth = 0, index = source.indexOf('{', source.indexOf(')', start));
  const begin = start;
  for (; index < source.length; index++) {
    if (source[index] === '{') depth++;
    else if (source[index] === '}' && --depth === 0) return source.slice(begin, index + 1);
  }
  throw new Error(`Unterminated ${name}`);
}

const NAMES = ['simpleHash', 'pointFingerprint', 'strokeSyncFingerprint', 'imageSyncFingerprint', 'itemSyncStampFingerprint',
  'strokeCollabFingerprint', 'imageCollabFingerprint', 'collaborativeItemLocationFingerprint', 'remoteStrokeDataLooksDifferent',
  'applyHistoryStrokeSnapshotDelta'];
const SHARED_STUBS = `
  const trackLog = [];
  function cloneStroke(stroke) { return stroke ? JSON.parse(JSON.stringify(stroke)) : null; }
  function trackDeletedStrokeIds(page, items) { trackLog.push(['del', items.map(item => item.id)]); }
  function untrackDeletedStrokeIds(page, items) { trackLog.push(['undel', items.map(item => item.id)]); }
  function normalizeTemplateKind(kind) { return kind || 'blank'; }
  const state = { pages: [] };`;

function makeContext(source, names, extra = {}) {
  const scope = { console, ihnStableStringify, ihnNormalizeDeletionStamps, TextEncoder, TextDecoder, ...extra };
  scope.window = scope.self = scope.globalThis = scope;
  vm.createContext(scope);
  const code = `${SHARED_STUBS}\n${names.map(name => extractFunction(source, name)).join('\n')}
    ({ ${names.join(', ')}, state, trackLog })`;
  return { scope, api: vm.runInContext(code, scope) };
}

// Legacy: original JS, no Rust. Current: the app's new glue plus the real rust-core-v5.js adapter.
const legacy = makeContext(legacySource('app-v5.js'), NAMES).api;
const rustScope = { WebAssembly, Uint8Array, Uint16Array, Uint32Array, Float32Array, Float64Array, URL, location: new URL('https://notes.test/'),
  fetch: async () => new Response(wasm), console, AbortSignal, AbortController, setTimeout, clearTimeout, TextEncoder, TextDecoder };
rustScope.globalThis = rustScope;
vm.createContext(rustScope);
vm.runInContext(await readFile(new URL('../rust-core-v5.js', import.meta.url), 'utf8'), rustScope);
assert.equal(await rustScope.InhouseRust.ready, true, 'wasm must load (run node scripts/build-rust-core.mjs)');
const appSource = await readFile(new URL('../app-v5.js', import.meta.url), 'utf8');
const current = makeContext(appSource, [...NAMES.filter(name => !['pointFingerprint', 'itemSyncStampFingerprint'].includes(name)), 'rustHistory'],
  { InhouseRust: rustScope.InhouseRust }).api;

// ---- random value generators -------------------------------------------------------------------------------------
function numberFactory(r) {
  const pick = list => list[Math.floor(r() * list.length)];
  return () => {
    const k = r();
    if (k < 0.2) return Math.round((r() - 0.5) * 2000 * 100) / 100;
    if (k < 0.3) return (Math.floor(r() * 100000) + 0.5) / 100;           // exact .xx5 ties after scaling
    if (k < 0.4) return Math.floor((r() - 0.5) * 4000);
    if (k < 0.5) return (r() - 0.5) * 20;
    if (k < 0.55) return pick([1.005, 2.675, 0.005, -0.005, 0.015, 8.345, 1.255, 0.285, 1e-9, -1e-9, 0.49999999999999994, -0.49999999999999994]);
    if (k < 0.6) return pick([-0, 0, 1e15, -1e15, 1e21, 1e22, 1.5e21, 123456789012345680000, 4503599627370495.5, 9007199254740993, 1e300, -1e300, 5e-324, 1.7976931348623157e308]);
    if (k < 0.7) return (r() - 0.5) * Math.pow(10, Math.floor(r() * 24) - 4);
    if (k < 0.8) return pick([NaN, Infinity, -Infinity, undefined, null, '5', '', 'abc', true, {}, []]);
    return Math.round(r() * 1000) / 10;
  };
}
function textFactory(r) {
  const alphabet = ['a', 'b', 'Z', '0', '9', '-', '_', ':', ' ', '|', 'é', 'ñ', '日', '本', '\u{1F600}', '\u{1D11E}', '\ud800', '\udc00', '\u0000', '�'];
  return (max = 12) => {
    const length = Math.floor(r() * max);
    let text = '';
    for (let index = 0; index < length; index++) text += alphabet[Math.floor(r() * alphabet.length)];
    return text;
  };
}
function makeGenerators(seed) {
  const r = rng(seed), num = numberFactory(r), text = textFactory(r);
  const pick = list => list[Math.floor(r() * list.length)];
  const finiteish = () => { const v = num(); return typeof v === 'number' && Number.isFinite(v) ? v : Math.round(r() * 500) / 10; };
  const point = () => {
    const k = r();
    if (k < 0.04) return null; if (k < 0.06) return undefined; if (k < 0.07) return 0; if (k < 0.08) return 5; if (k < 0.09) return 'x';
    const p = { x: r() < 0.9 ? finiteish() : num(), y: r() < 0.9 ? finiteish() : num() };
    if (r() < 0.8) p.p = r() < 0.9 ? Math.round(r() * 100) / 100 : num();
    return p;
  };
  const points = () => {
    const k = r();
    if (k < 0.05) return pick([undefined, null, 'abc', {}, 7]);
    const length = r() < 0.1 ? Math.floor(r() * 400) : Math.floor(r() * 12);
    const list = Array.from({ length }, point);
    if (r() < 0.05 && length) { delete list[Math.floor(r() * length)]; }           // sparse array hole
    if (r() < 0.02) { const sparse = new Array(Math.floor(r() * 4)); if (r() < 0.5 && sparse.length) sparse[0] = point(); return sparse; }
    return list;
  };
  const stamp = () => {
    const k = r();
    if (k < 0.3) return {};
    const make = () => ({ clock: pick([0, 1, 5, 99, 1e21, 1e22, -4, 3.7, NaN, Infinity, '12', '', null, undefined, 4294967296, 2 ** 53, 1.5e300, '7.9', true, -0]), actor: pick(['peer:tab', '', undefined, null, 0, 'é', 'a|b', 42]) });
    if (k < 0.5) return { syncStamp: make() };
    if (k < 0.6) return { _syncStamp: make() };
    if (k < 0.7) return { syncStamp: null, _syncStamp: make() };
    if (k < 0.85) { const s = make(); return { updatedClock: s.clock, updatedBy: s.actor }; }
    return {};
  };
  const stroke = () => {
    const k = r();
    if (k < 0.04) return pick([null, undefined, 0, '', false]);
    return { id: r() < 0.9 ? text(10) || 'id' : pick([0, 5, '', null, undefined, {}]), tool: pick(['pen', 'highlighter', 'eraser', '', undefined, 3, null]),
      color: r() < 0.8 ? pick(['#000', '#ff00aa', 'rgb(1,2,3)', '']) : text(6), width: r() < 0.9 ? finiteish() : num(), points: points(), ...stamp() };
  };
  const src = () => {
    const k = r();
    if (k < 0.1) return pick([undefined, null, '', 0, 12, {}, []]);
    if (k < 0.5) return text(40);
    const length = 200 + Math.floor(r() * 3000);
    let out = 'data:image/png;base64,';
    for (let index = 0; index < length; index++) out += 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/'[Math.floor(r() * 64)];
    if (r() < 0.25) out += text(5) + 'é' + '\u{1F600}';
    if (r() < 0.05) out = '\ud83d' + out;
    return out;
  };
  const image = () => {
    if (r() < 0.04) return pick([null, undefined, 0, '']);
    return { id: r() < 0.9 ? text(10) || 'im' : pick([0, null, 7]), src: src(), x: num(), y: num(), width: num(), height: num(),
      rotation: r() < 0.5 ? r() * 6.2832 : num(), ...stamp() };
  };
  return { r, num, text, pick, point, points, stroke, image, stamp };
}
// Legacy functions throw on null sources etc.; both sides must then throw the same way.
function outcome(fn) { try { return { value: fn() }; } catch (error) { return { error: error?.constructor?.name === 'TypeError' ? 'TypeError' : String(error) }; } }
function bothEqual(label, a, b) { assert.deepEqual(JSON.parse(JSON.stringify(a)), JSON.parse(JSON.stringify(b)), label); }
function same(label, legacyFn, currentFn) {
  const expected = outcome(legacyFn), actual = outcome(currentFn);
  if (expected.error) assert.equal(actual.error, expected.error, label);
  else assert.deepEqual(actual, expected, label);
}

test('simpleHash matches legacy on fixtures, unicode, lone surrogates and long strings', () => {
  const fixtures = ['', 'a', 'hello world', 'é', '日本語', '\u{1F600}', '\ud800', '\udc00x', 'a'.repeat(255), 'a'.repeat(256), 'a'.repeat(257),
    'é'.repeat(300), 'x'.repeat(1000) + '\u{1F600}', '\u0000\u0000', 'zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz'];
  for (const text of fixtures) assert.equal(current.simpleHash(text), legacy.simpleHash(text), JSON.stringify(text.slice(0, 20)));
  // INT32_MIN: a string whose hash lands on -2147483648 prints via abs() as "zik0zk".
  assert.equal(legacy.simpleHash('\u0000'), '0');
  const { text } = makeGenerators(77);
  for (let index = 0; index < 4000; index++) {
    const value = text(index % 9 === 0 ? 600 : 40);
    assert.equal(current.simpleHash(value), legacy.simpleHash(value), JSON.stringify(value));
  }
  for (const value of [null, undefined]) same(`hash ${value}`, () => legacy.simpleHash(value), () => current.simpleHash(value));
  for (const value of [12, true, {}, [], ['a'], { length: 3 }]) same(`hash ${JSON.stringify(value)}`, () => legacy.simpleHash(value), () => current.simpleHash(value));
});

test('simpleHash is exact across the 4M-unit chunk boundary (ASCII, wide and split surrogate pairs)', () => {
  const CHUNK = 1 << 22;
  const ascii = 'A'.repeat(CHUNK * 2 + 17);
  assert.equal(current.simpleHash(ascii), legacy.simpleHash(ascii));
  const pair = 'x'.repeat(CHUNK - 1) + '\u{1F600}' + 'y'.repeat(50);
  assert.equal(current.simpleHash(pair), legacy.simpleHash(pair));
  const wide = 'é'.repeat(CHUNK + 5) + 'z';
  assert.equal(current.simpleHash(wide), legacy.simpleHash(wide));
});

test('stroke and image fingerprints match legacy on seeded random items', () => {
  for (let seed = 1; seed <= 6; seed++) {
    const g = makeGenerators(seed * 1009);
    for (let index = 0; index < 700; index++) {
      const stroke = g.stroke(), image = g.image();
      same(`stroke sync seed ${seed}#${index}`, () => legacy.strokeSyncFingerprint(stroke), () => current.strokeSyncFingerprint(stroke));
      same(`stroke collab seed ${seed}#${index}`, () => legacy.strokeCollabFingerprint(stroke), () => current.strokeCollabFingerprint(stroke));
      same(`image sync seed ${seed}#${index}`, () => legacy.imageSyncFingerprint(image), () => current.imageSyncFingerprint(image));
      same(`image collab seed ${seed}#${index}`, () => legacy.imageCollabFingerprint(image), () => current.imageCollabFingerprint(image));
      const occurrence = { kind: g.pick(['stroke', 'image', 'stroke']), item: g.pick([stroke, image]), pageId: g.pick(['p1', '', undefined]), pageIndex: 3 };
      occurrence.kind === 'stroke' && image && (occurrence.item = stroke);
      same(`location seed ${seed}#${index}`, () => legacy.collaborativeItemLocationFingerprint(occurrence), () => current.collaborativeItemLocationFingerprint(occurrence));
    }
  }
  same('null location', () => legacy.collaborativeItemLocationFingerprint(null), () => current.collaborativeItemLocationFingerprint(null));
});

test('hand-written edge fixtures: rounding ties, -0, exponent forms, holes, non-finite points', () => {
  const points = [{ x: 1.005, y: 2.675, p: 0.285 }, { x: -0, y: -0.004, p: -0.005 }, { x: 1e21, y: 1e22, p: 1.5e21 }, { x: 1e300, y: -1e300 },
    { x: NaN, y: Infinity, p: -Infinity }, null, undefined, 3, { x: '5', y: '6', p: '7' }, { x: 0.49999999999999994, y: -0.49999999999999994, p: 4503599627370495.5 },
    { x: 12345678901234.567, y: 0.1 + 0.2, p: 8.345 }, { x: 5e-324, y: -5e-324, p: 1e-7 }];
  const sparse = [...points]; delete sparse[2]; sparse.length = 20;
  for (const list of [points, sparse, [], [null], new Array(3), points.slice(0, 1)]) {
    const stroke = { id: 'é\u{1F600}', tool: 'pen', color: '#fff', width: 2.345, points: list, syncStamp: { clock: 1e21, actor: 'a' } };
    assert.equal(current.strokeSyncFingerprint(stroke), legacy.strokeSyncFingerprint(stroke));
    assert.equal(current.strokeCollabFingerprint(stroke), legacy.strokeCollabFingerprint(stroke));
  }
  for (const width of [0, -0, NaN, 1e21, 1e22, 0.005, 1.005, '3', -1.239, 1e300, -1e-9]) {
    const stroke = { id: 'w', width, points: [] };
    assert.equal(current.strokeSyncFingerprint(stroke), legacy.strokeSyncFingerprint(stroke), String(width));
  }
  for (const rotation of [0, -0, 0.00004, 0.00005, -0.00005, 6.28318, 1e21, 1e22, NaN, 3.14159265, -3.14159265, 12345678.123456]) {
    const image = { id: 'i', src: 'abc', x: 1, y: 2, width: 3, height: 4, rotation };
    assert.equal(current.imageSyncFingerprint(image), legacy.imageSyncFingerprint(image), String(rotation));
  }
  assert.equal(current.strokeSyncFingerprint(null), '');
  assert.equal(current.strokeCollabFingerprint(null), legacy.strokeCollabFingerprint(null));
  assert.equal(current.imageCollabFingerprint(undefined), legacy.imageCollabFingerprint(undefined));
});

test('very long strokes and group boundaries (points and item counts) stay exact', () => {
  const g = makeGenerators(4242);
  const big = { id: 'big', tool: 'pen', color: '#000', width: 3, points: Array.from({ length: 120000 }, (_, index) => ({ x: index * 0.137, y: Math.sin(index) * 100.005, p: (index % 100) / 100 })) };
  assert.equal(current.strokeSyncFingerprint(big), legacy.strokeSyncFingerprint(big));
  const many = Array.from({ length: 4300 }, (_, index) => ({ id: `s${index}`, tool: 'pen', color: '#111', width: 2, points: Array.from({ length: 1 + (index % 5) * 30 }, () => g.point()) }));
  const heavy = Array.from({ length: 6 }, (_, index) => ({ id: `h${index}`, width: 1, points: Array.from({ length: 90000 }, () => ({ x: g.r() * 1000, y: g.r() * 1000, p: g.r() })) }));
  const history = current.rustHistory();
  const batch = history.strokeFingerprints([...many, ...heavy, null, ...many.slice(0, 5)], false);
  const expected = [...many, ...heavy, null, ...many.slice(0, 5)].map(item => legacy.strokeSyncFingerprint(item));
  assert.equal(batch.length, expected.length);
  for (let index = 0; index < batch.length; index++) if (batch[index] !== expected[index]) assert.fail(`batch fingerprint ${index}`);
  const collab = history.strokeFingerprints(heavy, true);
  heavy.forEach((item, index) => assert.equal(collab[index], legacy.strokeCollabFingerprint(item)));
});

test('number formatting is exact for tie-prone magnitudes and random bit patterns', () => {
  const g = makeGenerators(2024), bits = new DataView(new ArrayBuffer(8));
  const strokes = [];
  for (let index = 0; index < 6000; index++) {
    const k = index % 4;
    let value;
    if (k === 0) value = (Math.floor(g.r() * 2 ** 20) + 0.25 * Math.floor(g.r() * 4)) * 2 ** Math.floor(g.r() * 40);   // quarter steps near 1e15..1e17 give exact ties
    else if (k === 1) value = (g.r() - 0.5) * 10 ** (13 + Math.floor(g.r() * 6));
    else if (k === 2) { bits.setUint32(0, Math.floor(g.r() * 2 ** 32)); bits.setUint32(4, Math.floor(g.r() * 2 ** 32)); value = bits.getFloat64(0); }
    else value = Math.round(g.r() * 1e6) / 100 + 2 ** Math.floor(g.r() * 60);
    strokes.push({ id: `n${index}`, width: value, points: [{ x: value, y: -value, p: value / 3 }], updatedClock: Math.abs(value) });
  }
  const history = current.rustHistory();
  const plain = history.strokeFingerprints(strokes, false), collab = history.strokeFingerprints(strokes, true);
  strokes.forEach((stroke, index) => {
    assert.equal(plain[index], legacy.strokeSyncFingerprint(stroke), `sync ${stroke.width}`);
    assert.equal(collab[index], legacy.strokeCollabFingerprint(stroke), `collab ${stroke.width}`);
  });
  const images = strokes.slice(0, 2000).map((stroke, index) => ({ id: `i${index}`, src: 'x', x: stroke.width, y: -stroke.width, width: stroke.width * 3, height: 1 / (stroke.width || 1), rotation: stroke.width / 1e12, syncStamp: { clock: Math.abs(stroke.width), actor: 'a' } }));
  images.forEach(image => assert.equal(current.imageCollabFingerprint(image), legacy.imageCollabFingerprint(image), `image ${image.x}`));
  // BigInt inputs are not finite numbers for isFinite but convert for Number(): legacy semantics must hold.
  const bigint = { id: 'b', width: 12n, points: [{ x: 1n, y: 2, p: 3 }], syncStamp: { clock: 12n, actor: 'q' } };
  assert.equal(current.strokeCollabFingerprint(bigint), legacy.strokeCollabFingerprint(bigint));
});

function pageWith(g, strokes, images, extra = {}) { return { pageId: 'p', strokes, images, backgroundSource: 'template', templateKind: 'blank', ...extra }; }
test('remoteStrokeDataLooksDifferent matches legacy on random local/remote documents', () => {
  for (let seed = 1; seed <= 8; seed++) {
    const g = makeGenerators(seed * 31337);
    for (let round = 0; round < 120; round++) {
      const pageCount = 1 + Math.floor(g.r() * 3);
      const localPages = [], remotePages = [];
      for (let index = 0; index < pageCount; index++) {
        const strokes = Array.from({ length: Math.floor(g.r() * 6) }, () => ({ ...g.stroke() || { id: 'z', points: [] } }));
        const images = Array.from({ length: Math.floor(g.r() * 3) }, () => ({ ...g.image() || { id: 'z', src: '' } }));
        const clone = value => JSON.parse(JSON.stringify(value));
        const local = pageWith(g, strokes.map(clone), images.map(clone), { pageId: `p${index}`, deletedStrokeIds: g.r() < 0.2 ? ['a'] : undefined });
        const remote = pageWith(g, strokes.map(clone), images.map(clone), { pageId: `p${index}`, deletedStrokeIds: local.deletedStrokeIds });
        const k = g.r();
        if (k < 0.15 && remote.strokes.length) remote.strokes[0].width = (remote.strokes[0].width || 0) + 0.01;
        else if (k < 0.25 && remote.strokes.length) remote.strokes[remote.strokes.length - 1].points?.push?.({ x: 1, y: 2, p: 3 });
        else if (k < 0.3 && remote.images.length) remote.images[0].src = (remote.images[0].src || '') + 'x';
        else if (k < 0.35 && remote.strokes.length) remote.strokes[0].syncStamp = { clock: 9, actor: 'zz' };
        else if (k < 0.4) remote.strokes.push({ id: 'extra', points: [] });
        else if (k < 0.43) remote.deletedStrokeIds = ['b'];
        else if (k < 0.46) remote.templateKind = 'grid';
        else if (k < 0.48) remote.backgroundSource = 'pdf';
        else if (k < 0.5) remote.pageId = 'other';
        else if (k < 0.52) delete local.images;
        else if (k < 0.55 && remote.images.length) remote.images[0].rotation = 1.25;
        localPages.push(local); remotePages.push(remote);
      }
      legacy.state.pages = localPages; current.state.pages = localPages;
      const remoteData = g.r() < 0.05 ? null : remotePages;
      same(`remote diff seed ${seed} round ${round}`, () => legacy.remoteStrokeDataLooksDifferent(remoteData), () => current.remoteStrokeDataLooksDifferent(remoteData));
    }
  }
  // identical object references (same strokes in both documents) are never different
  const shared = { id: 's', points: [{ x: 1, y: 1, p: 1 }] };
  legacy.state.pages = current.state.pages = [pageWith(null, [shared], [])];
  const remote = [pageWith(null, [shared], [])];
  assert.equal(current.remoteStrokeDataLooksDifferent(remote), legacy.remoteStrokeDataLooksDifferent(remote));
  assert.equal(current.remoteStrokeDataLooksDifferent(remote), false);
});

test('applyHistoryStrokeSnapshotDelta matches legacy (page result, written strokes and tracking calls)', () => {
  for (let seed = 1; seed <= 8; seed++) {
    const g = makeGenerators(seed * 7919);
    for (let round = 0; round < 150; round++) {
      const universe = Array.from({ length: 2 + Math.floor(g.r() * 8) }, (_, index) => ({ id: `s${index}`, tool: 'pen', color: '#000', width: 2, points: Array.from({ length: Math.floor(g.r() * 8) }, () => g.point()) }));
      const clone = value => JSON.parse(JSON.stringify(value));
      const snapshot = () => universe.filter(() => g.r() < 0.7).map(stroke => {
        const copy = clone(stroke);
        const k = g.r();
        if (k < 0.2) copy.width += 1; else if (k < 0.3) copy.points.push({ x: 5, y: 5, p: 1 });
        else if (k < 0.35) copy.color = '#fff'; else if (k < 0.38) copy.id = '';
        return copy;
      });
      const source = g.r() < 0.05 ? null : snapshot(), target = g.r() < 0.05 ? undefined : snapshot();
      if (g.r() < 0.2 && Array.isArray(source) && Array.isArray(target) && source.length) target.push(source[0]);          // same object in both
      const pageBefore = { strokes: universe.filter(() => g.r() < 0.6).map(clone) };
      const pageA = clone(pageBefore), pageB = clone(pageBefore);
      legacy.trackLog.length = 0; current.trackLog.length = 0;
      const expected = outcome(() => legacy.applyHistoryStrokeSnapshotDelta(pageA, source, target));
      const actual = outcome(() => current.applyHistoryStrokeSnapshotDelta(pageB, source, target));
      assert.deepEqual(JSON.parse(JSON.stringify(actual)), JSON.parse(JSON.stringify(expected)), `result ${seed}/${round}`);
      bothEqual(`page ${seed}/${round}`, pageB, pageA);
      bothEqual(`tracking ${seed}/${round}`, current.trackLog, legacy.trackLog);
    }
  }
  const invalid = outcome(() => current.applyHistoryStrokeSnapshotDelta(null, [], []));
  assert.deepEqual(JSON.parse(JSON.stringify(invalid)), { value: [] });
});

test('the adapter fails loudly when the WASM core is not loaded', async () => {
  const scope = { WebAssembly, Uint8Array, URL, location: new URL('https://notes.test/'), fetch: async () => { throw new Error('offline'); },
    console, AbortSignal, AbortController, setTimeout, clearTimeout, TextEncoder, TextDecoder };
  scope.globalThis = scope;
  vm.createContext(scope);
  vm.runInContext(await readFile(new URL('../rust-core-v5.js', import.meta.url), 'utf8'), scope);
  assert.equal(await scope.InhouseRust.ready, false);
  assert.throws(() => scope.InhouseRust.history.simpleHash('a'), /Rust core \(WASM\) is required/);
  assert.throws(() => scope.InhouseRust.history.strokeFingerprint({ id: 'a' }), /Rust core \(WASM\) is required/);
  const { api } = makeContext(appSource, ['simpleHash', 'rustHistory'], { InhouseRust: scope.InhouseRust });
  assert.throws(() => api.simpleHash('a'), /Rust core \(WASM\) is not loaded/);
  const none = makeContext(appSource, ['simpleHash', 'rustHistory'], {}).api;
  assert.throws(() => none.simpleHash('a'), /Rust core \(WASM\) is not loaded/);
});
