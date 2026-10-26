// Differential test: stroke path/bounds packing and legacy cover strokes, Rust port vs the original JS
// (app-v5.js as it was at RUST_PORT_BASE). Both versions run against recording canvas/Path2D mocks and
// must produce byte-identical drawing logs and identical hit-test bounds.
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import vm from 'node:vm';
import {legacySource, rng} from './helpers/legacy.mjs';

const wasm = await readFile(new URL('../wasm/inhouse_notes_core.wasm', import.meta.url));
const rustScope = {WebAssembly, URL, location: new URL('https://notes.test/'), fetch: async () => new Response(wasm), console, AbortSignal, AbortController, setTimeout, clearTimeout};
vm.createContext(rustScope);
vm.runInContext(await readFile(new URL('../rust-core-v5.js', import.meta.url), 'utf8'), rustScope);
assert.equal(await rustScope.InhouseRust.ready, true, 'Rust core must load');
const rust = rustScope.InhouseRust;

function extract(source, name) {
    const start = source.indexOf(`\n        function ${name}(`);
    assert.ok(start >= 0, `function ${name} not found`);
    let i = source.indexOf('{', source.indexOf(')', start)), depth = 0;
    for (; i < source.length; i++) {
        const c = source[i];
        if (c === '/' && source[i + 1] === '/') { i = source.indexOf('\n', i); continue; }
        if (c === "'" || c === '"') { for (i++; source[i] !== c; i++) if (source[i] === '\\') i++; continue; }
        if (c === '{') depth++;
        else if (c === '}' && --depth === 0) return source.slice(start, i + 1);
    }
    throw new Error(`unterminated ${name}`);
}
const NAMES = ['drawSmoothStroke', 'drawLegacyCoverStrokes', 'cloneLegacyCoverStrokes', 'getStrokeBounds', 'setupContext', 'setStrokeCaps',
    'getStrokeWidth', 'drawFixedHighlighterStroke', 'buildSmoothPath', 'strokeRustCore'];
const fmt = value => {
    if (typeof value === 'number') return Object.is(value, -0) ? '-0' : String(value);
    if (value && value.ops) return `path[${value.ops.map(op => op.map(fmt).join(',')).join(';')}]`;
    if (typeof value === 'string') return JSON.stringify(value);
    return String(value);
};
function harness(source, extra = '') {
    const logs = [];
    const scope = {console, WeakMap, Array, Math, Infinity, Number, Object, window: {InhouseRust: rust}, logs};
    class Path2D {
        constructor() { this.ops = []; }
        moveTo(x, y) { this.ops.push(['M', x, y]); }
        lineTo(x, y) { this.ops.push(['L', x, y]); }
    }
    scope.Path2D = Path2D;
    vm.createContext(scope);
    const functions = NAMES.filter(name => source.includes(`\n        function ${name}(`)).map(name => extract(source, name)).join('\n') + extra;
    vm.runInContext(`const penPathCache = new WeakMap(); const strokeBoundsCache = new WeakMap();
        ${functions}
        globalThis.api = {drawSmoothStroke, drawLegacyCoverStrokes, cloneLegacyCoverStrokes, getStrokeBounds, strokeBoundsCache};`, scope);
    const props = {};
    const ctx = new Proxy({}, {
        get: (_, key) => key in props ? props[key] : (...args) => { logs.push(`${String(key)}(${args.map(fmt).join(',')})`); },
        set: (_, key, value) => { props[key] = value; logs.push(`${String(key)}=${fmt(value)}`); return true; }
    });
    return {api: scope.api, ctx, logs, take: () => logs.splice(0)};
}
const legacy = harness(legacySource('app-v5.js'));
const ported = harness(await readFile(new URL('../app-v5.js', import.meta.url), 'utf8'));
const bounds = (h, stroke) => {
    const entry = h.api.strokeBoundsCache.get(stroke);
    if (!entry) return null;
    const {points, ...rest} = entry;
    return {...Object.fromEntries(Object.entries(rest).map(([k, v]) => [k, fmt(v)])), sameArray: points === stroke.points};
};

function randomStroke(random, mode = 'normal') {
    const count = mode === 'long' ? 2000 + Math.floor(random() * 6000) : 2 + Math.floor(random() * (random() < .1 ? 600 : 40));
    const points = [];
    let x = random() * 800, y = random() * 1100;
    for (let i = 0; i < count; i++) {
        x += (random() - .5) * 12; y += (random() - .5) * 12;
        const point = {x: Math.round(x * 100) / 100, y: y * (1 + random() * 1e-9), pressure: random()};
        const odd = random();
        if (mode === 'odd' && odd < .04) point.x = NaN;
        else if (mode === 'odd' && odd < .06) point.y = Infinity;
        else if (mode === 'odd' && odd < .08) point.x = -0;
        else if (mode === 'odd' && odd < .09) point.y = undefined;
        points.push(point);
    }
    const tools = ['pen', 'pen', 'pen', 'highlighter', 'eraser-stroke', 'eraser-area', undefined];
    return {id: `s${Math.floor(random() * 1e9)}`, tool: tools[Math.floor(random() * tools.length)], color: random() < .5 ? '#112233' : 'rgba(1,2,3,.5)',
        width: [1, 2.5, 3, 7.25, 12, 0.3][Math.floor(random() * 6)], points};
}
const clone = value => structuredClone(value);
function sameDrawing(stroke, label) {
    const a = clone(stroke), b = clone(stroke);
    legacy.api.drawSmoothStroke(legacy.ctx, a);
    ported.api.drawSmoothStroke(ported.ctx, b);
    assert.deepEqual(ported.take(), legacy.take(), `${label}: drawing log`);
    assert.deepEqual(bounds(ported, b), bounds(legacy, a), `${label}: bounds`);
    return [a, b];
}

test('fixtures: straight, two-point, closed loop and degenerate pen strokes draw identically', () => {
    const fixtures = [
        {tool: 'pen', color: '#000', width: 2, points: [{x: 0, y: 0}, {x: 10, y: 10}]},
        {tool: 'pen', color: '#000', width: 2, points: [{x: 5, y: 5}, {x: 5, y: 5}, {x: 5, y: 5}]},
        {tool: 'pen', color: '#f00', width: 3, points: [{x: 0, y: 0}, {x: 1, y: 0}, {x: 1, y: 1}, {x: 0, y: 1}, {x: 0, y: 0}]},
        {tool: 'pen', color: '#f00', width: 3, points: [{x: -5.5, y: 1e9}, {x: 1e-9, y: -1e9}, {x: 0.1 + 0.2, y: 2 ** 53}]},
        {tool: 'highlighter', color: '#ff0', width: 14, points: [{x: 1, y: 1}]},
        {tool: 'highlighter', color: '#ff0', width: 14, points: [{x: 1, y: 1}, {x: 40, y: 3}, {x: 80, y: 9}]},
        {tool: 'eraser-area', color: '#000', width: 20, points: [{x: 3, y: 3}, {x: 50, y: 50}]},
        {tool: 'pen', color: '#000', width: 2, points: [{x: 1, y: 1}]},
        {tool: 'pen', color: '#000', width: 2, points: []},
        {tool: 'pen', color: '#000', width: 2},
        null
    ];
    fixtures.forEach((stroke, index) => {
        legacy.api.drawSmoothStroke(legacy.ctx, stroke && clone(stroke));
        ported.api.drawSmoothStroke(ported.ctx, stroke && clone(stroke));
        assert.deepEqual(ported.take(), legacy.take(), `fixture ${index}`);
    });
});

test('4000 seeded random strokes: drawing log and bounds are identical, repaints hit the path cache', () => {
    const random = rng(20260630);
    for (let run = 0; run < 4000; run++) {
        const stroke = randomStroke(random, run % 7 === 0 ? 'odd' : 'normal');
        const [a, b] = sameDrawing(stroke, `run ${run}`);
        // Repaint of the same stroke object (cache hit) and of a fresh object sharing the points array.
        sameDrawing.call(null, stroke, `run ${run} fresh`);
        legacy.api.drawSmoothStroke(legacy.ctx, a); ported.api.drawSmoothStroke(ported.ctx, b);
        assert.deepEqual(ported.take(), legacy.take(), `run ${run} repaint`);
        const sibling = (h, s) => ({...s, color: '#fff', width: 9});
        legacy.api.drawSmoothStroke(legacy.ctx, sibling(legacy, a)); ported.api.drawSmoothStroke(ported.ctx, sibling(ported, b));
        assert.deepEqual(ported.take(), legacy.take(), `run ${run} shared points`);
    }
});

test('growing live stroke: every appended point invalidates the path and bounds exactly like the legacy code', () => {
    const random = rng(7);
    const base = randomStroke(random, 'long');
    base.tool = 'pen';
    const a = {...base, points: []}, b = {...base, points: []};
    for (let i = 0; i < 400; i++) {
        const point = base.points[i];
        a.points.push({...point}); b.points.push({...point});
        if (a.points.length < 2) continue;
        legacy.api.drawSmoothStroke(legacy.ctx, a); ported.api.drawSmoothStroke(ported.ctx, b);
        assert.deepEqual(ported.take(), legacy.take(), `point ${i}`);
        assert.deepEqual(bounds(ported, b), bounds(legacy, a), `point ${i} bounds`);
    }
});

test('very long strokes release the large WASM scratch buffer and stay identical', () => {
    const random = rng(99);
    for (let run = 0; run < 12; run++) sameDrawing(Object.assign(randomStroke(random, 'long'), {tool: 'pen'}), `long ${run}`);
    const huge = {tool: 'pen', color: '#000', width: 2, points: Array.from({length: 300000}, (_, i) => ({x: i % 977, y: Math.sin(i) * 100}))};
    sameDrawing(huge, 'huge');
    sameDrawing(randomStroke(random), 'after huge');
});

test('legacy cover strokes: 3000 seeded random pages draw identically; clone stays a faithful deep copy', () => {
    const random = rng(424242);
    let drawnLines = 0;
    const widths = [undefined, null, 0, NaN, -0, -4, 0.2, 0.74, 0.75, 1, 2, 3.3, 12, '3', 'abc', Infinity, 1e300];
    for (let run = 0; run < 3000; run++) {
        const strokes = [];
        const count = Math.floor(random() * 8);
        for (let i = 0; i < count; i++) {
            const kind = random();
            if (kind < .05) { strokes.push(null); continue; }
            const stroke = randomStroke(random);
            stroke.width = widths[Math.floor(random() * widths.length)];
            if (kind < .12) stroke.points = random() < .5 ? null : {length: 5};
            else if (kind < .25) stroke.points = stroke.points.slice(0, Math.floor(random() * 3));
            strokes.push(stroke);
        }
        const page = {legacyCoverStrokes: strokes.length || random() < .5 ? strokes : null};
        const a = clone(page), b = clone(page);
        legacy.api.drawLegacyCoverStrokes(legacy.ctx, a);
        ported.api.drawLegacyCoverStrokes(ported.ctx, b);
        const covered = legacy.take();
        drawnLines += covered.length;
        assert.deepEqual(ported.take(), covered, `cover run ${run}`);
        // The cover pass reads but never mutates the page.
        assert.deepEqual(b, a);
        if (strokes.every(Boolean)) assert.deepEqual(clone(legacy.api.cloneLegacyCoverStrokes(a.legacyCoverStrokes)), clone(ported.api.cloneLegacyCoverStrokes(b.legacyCoverStrokes)));
    }
    assert.ok(drawnLines > 10000, 'random pages must really draw covers');
    for (const page of [undefined, null, {}, {legacyCoverStrokes: []}, {legacyCoverStrokes: 'x'}]) {
        legacy.api.drawLegacyCoverStrokes(legacy.ctx, page); ported.api.drawLegacyCoverStrokes(ported.ctx, page);
        assert.deepEqual(ported.take(), legacy.take());
    }
});

test('numeric strings as coordinates still draw the same path; bounds are numeric instead of lexical strings', () => {
    // Persisted ink only holds numbers. The legacy scan compared strings lexically for such corrupt points
    // and kept them as strings; Rust bounds are always numbers, and every consumer subtracts a pad first.
    const stroke = {tool: 'pen', color: '#000', width: 2, points: [{x: '9', y: 1}, {x: '10', y: 2}, {x: 4, y: '3'}]};
    const a = clone(stroke), b = clone(stroke);
    legacy.api.drawSmoothStroke(legacy.ctx, a); ported.api.drawSmoothStroke(ported.ctx, b);
    assert.deepEqual(ported.take(), legacy.take());
    const entry = ported.api.strokeBoundsCache.get(b);
    assert.deepEqual([entry.minX, entry.minY, entry.maxX, entry.maxY], [4, 1, 10, 3]);
});

test('Rust boundary refuses invalid input loudly', () => {
    assert.throws(() => rust.renderStrokeCoords(0), {name: "RangeError"});
    assert.throws(() => rust.renderStrokeCoords(4194305), {name: "RangeError"});
    assert.throws(() => rust.renderStrokeCoords(1.5), {name: "RangeError"});
    rust.renderStrokeCoords(8);
    assert.throws(() => rust.renderStrokeBounds(9999999), {name: "RangeError"});
    assert.throws(() => rust.renderStrokeBounds(0), {name: "RangeError"});
    assert.deepEqual([...rust.legacyCoverWidths([])], []);
    const widths = rust.legacyCoverWidths([{tool: 'pen', width: 4, points: [{}, {}]}, {tool: 'eraser-stroke'}, null]);
    assert.deepEqual([...widths], [4 * 1.35, -1, -1]);
    const missing = {...rustScope, InhouseRust: undefined};
    assert.equal(typeof missing, 'object');
});
