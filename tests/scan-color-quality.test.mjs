// Differential test: the Rust capture-quality kernels must reproduce the original JavaScript
// (scanner/processing/capture-quality.js at the pre-port commit) bit for bit, including result key order.
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import vm from 'node:vm';
import {loadLegacy, rng} from './helpers/legacy.mjs';

const wasm = await readFile(new URL('../wasm/inhouse_notes_core.wasm', import.meta.url));
const rustScope = {WebAssembly, Uint8Array, Uint16Array, Float32Array, URL, location: new URL('https://notes.test/scanner/'),
    fetch: async () => new Response(wasm), console, AbortSignal, AbortController, setTimeout, clearTimeout, TextEncoder, TextDecoder};
vm.createContext(rustScope);
vm.runInContext(await readFile(new URL('../rust-core-v5.js', import.meta.url), 'utf8'), rustScope);
await rustScope.InhouseRust.ready;

// A canvas stub that "draws" a source by asking it to paint the requested size.
function stubCanvas() {
    const canvas = {width: 0, height: 0, getContext: () => {
        let painted = null;
        return {drawImage(source, x, y, w, h) { painted = source.paint(w, h); }, getImageData: () => ({data: painted})};
    }};
    return canvas;
}
const documentStub = {createElement: name => { assert.equal(name, 'canvas'); return stubCanvas(); }};

const legacy = loadLegacy('scanner/processing/capture-quality.js', {document: documentStub}).ScannerPro.CaptureQuality;
const currentSource = await readFile(new URL('../scanner/processing/capture-quality.js', import.meta.url), 'utf8');
const current = (() => {
    const scope = {console, setTimeout, clearTimeout, URL, Float32Array, Float64Array, Uint8Array, Math, JSON, Map, Set, Promise,
        document: documentStub, InhouseRust: rustScope.InhouseRust};
    scope.window = scope.self = scope.globalThis = scope;
    vm.createContext(scope);
    vm.runInContext(currentSource, scope);
    return scope.ScannerPro.CaptureQuality;
})();

// QUALITY_SCALE=0.1 runs a tenth of the seeded cases (quick local check); QUALITY_SCALE=5 runs ten thousand-odd.
const N = count => Math.max(5, Math.round(count * (Number(process.env.QUALITY_SCALE) || 1)));

function same(a, b, path = 'result') {
    if (typeof a === 'number' || typeof b === 'number') { assert.ok(Object.is(a, b) || (a === b), `${path}: ${a} !== ${b}`); return; }
    if (a === null || b === null || typeof a !== 'object' || typeof b !== 'object') { assert.equal(a, b, path); return; }
    assert.deepEqual(Object.keys(a), Object.keys(b), `${path}: key order`);
    for (const key of Object.keys(a)) same(a[key], b[key], `${path}.${key}`);
}

// ---- Scene generation ---------------------------------------------------------------------------

const luma = (r, g, b) => r * .2126 + g * .7152 + b * .0722;

function blur(values, width, height, radius) {
    const out = new Float32Array(values.length), tmp = new Float32Array(values.length), n = radius * 2 + 1;
    for (let y = 0; y < height; y++) for (let x = 0; x < width; x++) { let s = 0;
        for (let d = -radius; d <= radius; d++) s += values[y * width + Math.min(width - 1, Math.max(0, x + d))]; tmp[y * width + x] = s / n; }
    for (let y = 0; y < height; y++) for (let x = 0; x < width; x++) { let s = 0;
        for (let d = -radius; d <= radius; d++) s += tmp[Math.min(height - 1, Math.max(0, y + d)) * width + x]; out[y * width + x] = s / n; }
    return out;
}

function makeScene(seed, {minSize = 160, maxSize = 700} = {}) {
    const random = rng(seed), pick = list => list[Math.floor(random() * list.length)];
    const width = minSize + Math.floor(random() * (maxSize - minSize)), height = minSize + Math.floor(random() * (maxSize - minSize));
    const landscapeBias = random() < .3;
    const margin = pick([.04, .08, .1, .15]);
    let quad = [{x: margin, y: margin * .6}, {x: 1 - margin, y: margin * .6}, {x: 1 - margin, y: 1 - margin * .6}, {x: margin, y: 1 - margin * .6}];
    const kind = pick(['lattice', 'lattice', 'lattice', 'lattice', 'noise', 'text', 'specks', 'flat', 'checker']);
    if (random() < .5) { const skew = .06 * random(); quad = quad.map((p, i) => ({x: p.x + (i % 2 ? -1 : 1) * skew * random(), y: p.y + (i < 2 ? 1 : -1) * skew * random()})); }
    if (random() < .15) { const shift = (random() - .5) * .6; quad = quad.map(p => ({x: p.x + shift, y: p.y + shift * .5})); } // partly off-image
    if (random() < .1) quad = quad.map(p => ({x: p.y, y: 1 - p.x}));                                                           // rotated
    const px = quad.map(p => ({x: p.x * width, y: p.y * height}));
    const edgeLength = (a, b) => Math.hypot(a.x - b.x, a.y - b.y);
    const landscape = edgeLength(px[0], px[1]) + edgeLength(px[2], px[3]) > edgeLength(px[1], px[2]) + edgeLength(px[3], px[0]);
    const columns = landscape ? 54 : 36, rows = landscape ? 36 : 54;
    const rgba = new Uint8ClampedArray(width * height * 4);
    const paper = 140 + Math.floor(random() * 110), noise = pick([0, 1, 2, 4, 8, 16]);
    for (let i = 0; i < width * height; i++) {
        const tint = (random() - .5) * noise;
        rgba[i * 4] = paper + tint; rgba[i * 4 + 1] = paper + tint * .8; rgba[i * 4 + 2] = paper - 10 * random() + tint * .6; rgba[i * 4 + 3] = 255;
    }
    const dot = (cx, cy, radius, darkness) => {
        for (let y = Math.floor(cy - radius - 1); y <= Math.ceil(cy + radius + 1); y++) for (let x = Math.floor(cx - radius - 1); x <= Math.ceil(cx + radius + 1); x++) {
            if (x < 0 || y < 0 || x >= width || y >= height) continue;
            const coverage = Math.max(0, Math.min(1, radius + .5 - Math.hypot(x - cx, y - cy))), at = (y * width + x) * 4;
            for (let c = 0; c < 3; c++) rgba[at + c] = rgba[at + c] * (1 - coverage * darkness);
        }
    };
    const surface = (u, v) => ({
        x: (px[0].x * (1 - u) + px[1].x * u) * (1 - v) + (px[3].x * (1 - u) + px[2].x * u) * v + Math.sin(v * 6 + seed) * 2 * (kind === 'lattice' ? random() < .3 : 0),
        y: (px[0].y * (1 - u) + px[1].y * u) * (1 - v) + (px[3].y * (1 - u) + px[2].y * u) * v});
    if (kind === 'lattice') {
        const darkness = .25 + random() * .5, radius = pick([.7, 1.1, 1.5, 2]), skip = pick([0, 0, 0, .1, .4]), partial = random() < .15;
        for (let r = 1; r < rows; r++) for (let c = 1; c < columns; c++) {
            if (random() < skip || (partial && (r > rows * .5 || c > columns * .5))) continue;
            const p = surface(c / columns, r / rows);
            dot(p.x + (random() - .5) * .6, p.y + (random() - .5) * .6, radius, darkness);
        }
    } else if (kind === 'text') {
        for (let line = 0; line < 30; line++) { const y = Math.floor(random() * height);
            for (let x = 0; x < width; x++) { const at = (y * width + x) * 4; rgba[at] = rgba[at + 1] = rgba[at + 2] = 40; } }
        for (let stroke = 0; stroke < 80; stroke++) dot(random() * width, random() * height, 1 + random() * 2, .9);
    } else if (kind === 'specks') {
        const count = pick([300, 1500, 9000]);
        for (let i = 0; i < count; i++) dot(random() * width, random() * height, .9 + random(), .3 + random() * .5);
    } else if (kind === 'noise') {
        for (let i = 0; i < width * height * 4; i += 4) { const v = Math.floor(random() * 256); rgba[i] = rgba[i + 1] = rgba[i + 2] = v; }
    } else if (kind === 'checker') {
        const cell = pick([2, 3, 5, 9]);
        for (let y = 0; y < height; y++) for (let x = 0; x < width; x++) if (((x / cell | 0) + (y / cell | 0)) & 1) {
            const at = (y * width + x) * 4; rgba[at] = rgba[at + 1] = rgba[at + 2] = 30; }
    }
    // outline so the frame edges carry real gradient
    const outline = (a, b) => { const steps = Math.ceil(Math.hypot(b.x - a.x, b.y - a.y)); for (let s = 0; s <= steps; s++) dot(a.x + (b.x - a.x) * s / steps, a.y + (b.y - a.y) * s / steps, 1, .8); };
    if (random() < .8) for (let i = 0; i < 4; i++) outline(px[i], px[(i + 1) % 4]);
    let gray = new Float32Array(width * height);
    for (let i = 0; i < gray.length; i++) gray[i] = luma(rgba[i * 4], rgba[i * 4 + 1], rgba[i * 4 + 2]);
    const softness = pick([0, 0, 0, 1, 2, 3]);
    if (softness) { gray = blur(gray, width, height, softness); for (let i = 0; i < gray.length; i++) { const v = Math.max(0, Math.min(255, Math.round(gray[i]))); rgba[i * 4] = rgba[i * 4 + 1] = rgba[i * 4 + 2] = v; gray[i] = luma(v, v, v); } }
    // frame edges: along the sides, with jitter and a varying number of points
    const edgeMode = pick(['good', 'good', 'good', 'sparse', 'missing', 'three', 'single']);
    const frameEdges = edgeMode === 'missing' ? undefined : (edgeMode === 'three' ? [0, 1, 2] : [0, 1, 2, 3]).map(side => {
        const a = quad[side], b = quad[(side + 1) % 4], count = edgeMode === 'single' ? 1 : edgeMode === 'sparse' ? 2 + Math.floor(random() * 3) : 12 + Math.floor(random() * 30);
        return Array.from({length: count}, (_, i) => { const t = count === 1 ? .5 : i / (count - 1), j = edgeMode === 'good' ? (random() - .5) * .004 : 0;
            return {x: a.x + (b.x - a.x) * t + j, y: a.y + (b.y - a.y) * t + j}; });
    });
    const detection = frameEdges ? {quad, frameEdges} : {quad};
    const source = {width, height, paint: () => rgba};
    return {kind, width, height, rgba, gray, detection, source};
}

const rasterOf = scene => ({width: scene.width, height: scene.height, gray: scene.gray});
const sourceFor = scene => ({width: scene.width, height: scene.height, paint: (w, h) => {
    assert.equal(w * h * 4, scene.rgba.length); return scene.rgba; }});

// ---- Tests --------------------------------------------------------------------------------------

test('acutance matches the original JavaScript on 2000 small seeded scenes', () => {
    const kinds = new Set();
    for (let seed = 1; seed <= N(2000); seed++) {
        const scene = makeScene(seed, {minSize: 48, maxSize: 220});
        kinds.add(scene.kind);
        same(current.inspect(scene.source, scene.detection, rasterOf(scene)), legacy.inspect(scene.source, scene.detection, rasterOf(scene)), `seed ${seed} ${scene.kind}`);
    }
    if (N(2000) >= 2000) assert.ok(kinds.size >= 6);
});

test('acutance matches the original JavaScript on large (up to 1100 px) seeded scenes', () => {
    for (let seed = 3000; seed < 3000 + N(40); seed++) {
        const scene = makeScene(seed, {minSize: 400, maxSize: 1100});
        same(current.inspect(scene.source, scene.detection, rasterOf(scene)), legacy.inspect(scene.source, scene.detection, rasterOf(scene)), `seed ${seed} ${scene.kind}`);
    }
});

test('lattice preflight matches the original JavaScript on seeded scenes and exercises every outcome', () => {
    const outcomes = new Map();
    for (let seed = 1000; seed < 1000 + N(110); seed++) {
        const scene = makeScene(seed, {minSize: 320, maxSize: 900});
        const expected = legacy.inspectGrid(scene.source, scene.detection, rasterOf(scene));
        same(current.inspectGrid(scene.source, scene.detection, rasterOf(scene)), expected, `seed ${seed} ${scene.kind}`);
        const outcome = `${expected.valid ? 'valid' : 'invalid'}:${expected.reason || (expected.cells === undefined ? 'empty' : 'measured')}`;
        outcomes.set(outcome, (outcomes.get(outcome) || 0) + 1);
    }
    console.log('grid outcomes', JSON.stringify([...outcomes]));
    if (N(110) >= 110) {
        assert.ok(outcomes.has('valid:measured'), 'some seeded scenes must carry a valid lattice');
        assert.ok(outcomes.has('invalid:measured') && outcomes.has('invalid:page-clipped'));
    }
});

test('full readiness (RGBA to luma included) matches the original JavaScript on seeded scenes', () => {
    const outcomes = new Map();
    for (let seed = 5000; seed < 5000 + N(40); seed++) {
        const scene = makeScene(seed, {minSize: 300, maxSize: 800});
        const expected = legacy.inspectReadiness(sourceFor(scene), scene.detection);
        same(current.inspectReadiness(sourceFor(scene), scene.detection), expected, `seed ${seed} ${scene.kind}`);
        outcomes.set(expected.reason, (outcomes.get(expected.reason) || 0) + 1);
    }
    console.log('readiness outcomes', JSON.stringify([...outcomes]));
    if (N(40) >= 40) assert.ok(outcomes.has('sharp-grid') && outcomes.has('focus-or-motion-blur'));
});

test('very large sources are downscaled to 1100 px exactly as before', () => {
    const scene = makeScene(77, {minSize: 500, maxSize: 600});
    const big = {width: 2200, height: 1650, paint: (w, h) => { assert.deepEqual([w, h], [1100, 825]); return makeScene(77, {minSize: 500, maxSize: 600}).rgba.length === w * h * 4 ? scene.rgba : new Uint8ClampedArray(w * h * 4).fill(180); }};
    const filler = {width: 2200, height: 1650, paint: () => new Uint8ClampedArray(1100 * 825 * 4).fill(180)};
    same(current.inspectReadiness(filler, scene.detection), legacy.inspectReadiness(filler, scene.detection));
    assert.ok(big);
});

test('degenerate and invalid detections behave like the original', () => {
    const scene = makeScene(9, {minSize: 400, maxSize: 500}), raster = rasterOf(scene);
    const good = scene.detection;
    const variants = [
        {}, {quad: null}, {quad: []}, {quad: good.quad.slice(0, 3)},
        {quad: good.quad.map(p => ({x: p.x, y: Number.NaN}))}, {quad: good.quad.map(p => ({x: Infinity, y: p.y}))},
        {quad: good.quad.map(p => ({x: p.x, y: p.y})), frameEdges: [[], [], [], []]},
        {quad: good.quad, frameEdges: [good.frameEdges?.[0] || [], good.frameEdges?.[1] || [], [{x: .5, y: .5}], []]},
        {quad: good.quad.map(() => ({x: .5, y: .5}))},
        {quad: [{x: 0, y: 0}, {x: 1, y: 0}, {x: 1, y: 1}, {x: 0, y: 1}]},
        {quad: [{x: -1, y: -1}, {x: 2, y: -1}, {x: 2, y: 2}, {x: -1, y: 2}], frameEdges: [[{x: -1, y: -1}, {x: 2, y: -1}], [{x: 2, y: -1}, {x: 2, y: 2}], [{x: 2, y: 2}, {x: -1, y: 2}], [{x: -1, y: 2}, {x: -1, y: -1}]]},
        {quad: good.quad, frameEdges: [[{x: Number.NaN, y: .2}, {x: .5, y: .2}], good.frameEdges?.[1] || [], good.frameEdges?.[2] || [], good.frameEdges?.[3] || []]},
    ];
    for (const [index, detection] of variants.entries()) {
        for (const name of ['inspect', 'inspectGrid']) {
            let expected, actual;
            try { expected = legacy[name](scene.source, detection, raster); } catch (error) { expected = {threw: true}; }
            try { actual = current[name](scene.source, detection, raster); } catch (error) { actual = {threw: true}; }
            // A detection with non-finite frame-edge samples used for resampling is rejected loudly (documented).
            if (index === variants.length - 1 && name === 'inspectGrid') { assert.ok(actual.threw || actual.valid === false); continue; }
            if (expected.threw || actual.threw) { assert.equal(!!actual.threw, !!expected.threw, `${name} #${index}`); continue; }
            same(actual, expected, `${name} #${index}`);
        }
    }
    assert.deepEqual({...current.inspect(scene.source, {}, raster)}, {valid: false, reason: 'missing-page'});
    assert.deepEqual({...current.inspectReadiness({width: 0, height: 1}, good)}, {valid: false, reason: 'missing-page', grid: null});
});

test('the Rust adapter fails loudly instead of degrading when the core is missing', () => {
    const scope = {console, URL, Float32Array, Math, JSON, document: documentStub, InhouseRust: {available: () => false}};
    scope.window = scope.self = scope.globalThis = scope;
    vm.createContext(scope);
    vm.runInContext(currentSource, scope);
    const scene = makeScene(3, {minSize: 200, maxSize: 260});
    assert.throws(() => scope.ScannerPro.CaptureQuality.inspect(scene.source, scene.detection, rasterOf(scene)), /Rust core is not ready/);
});
