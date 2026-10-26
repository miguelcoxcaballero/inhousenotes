// Differential test: the Rust resident stroke store against the original JavaScript eraser geometry
// (getStrokeBounds / strokeMayIntersectCircle / strokeHitsPoint / distancePointToSegment / eraseStrokeAtPoint hit loop).
import test from 'node:test';
import assert from 'node:assert/strict';
import vm from 'node:vm';
import {readFile} from 'node:fs/promises';
import {legacySource, rng} from './helpers/legacy.mjs';

const bytes = await readFile(new URL('../wasm/inhouse_notes_core.wasm', import.meta.url));
const scope = {WebAssembly, URL, location: new URL('https://notes.test/'), fetch: async () => new Response(bytes), console, AbortSignal, AbortController, setTimeout, clearTimeout};
vm.createContext(scope);
vm.runInContext(await readFile(new URL('../rust-core-v5.js', import.meta.url), 'utf8'), scope);
assert.equal(await scope.InhouseRust.ready, true);
const raw = scope.InhouseRust;
// The adapter runs in its own realm; normalise arrays so deepEqual compares values.
const rust = {...raw, strokesHitCircle: (...a) => [...raw.strokesHitCircle(...a)]};

// The unmodified JavaScript from the commit before the port (no Rust acceleration: pure reference semantics).
const source = legacySource('app-v5.js');
const slice = (from, to) => { const a = source.indexOf(from), b = source.indexOf(to, a); assert.ok(a > 0 && b > a, from); return source.slice(a, b); };
const legacy = vm.runInNewContext(`(() => {
    const strokeBoundsCache = new WeakMap();
    const window = { InhouseRust: null };
    const getDocumentSessionToken = () => 1, pageMutationGeneration = 1;
    ${slice('function getStrokeBounds(stroke) {', 'function eraseAreaAtPoint(')}
    return { getStrokeBounds, strokeMayIntersectCircle, strokeHitsPoint, distancePointToSegment,
        hits(strokes, x, y, r) { const out = []; for (let i = strokes.length - 1; i >= 0; i--) {
            const stroke = strokes[i];
            if (stroke.tool && stroke.tool.startsWith('eraser')) continue;
            if (strokeHitsPoint(stroke, x, y, r)) out.push(i);
        } return out; } };
})()`, {Math});

legacy.hits = (hits => (...args) => [...hits(...args)])(legacy.hits);
const same = (a, b) => Object.is(a, b);
const GEN = (n) => `1:${n}`;

function randomStroke(random, i, opts = {}) {
    const length = opts.length ?? [1, 2, 3, 5, 20, 80, 255, 256, 300, 700][Math.floor(random() * 10)];
    const cx = random() * 600, cy = random() * 800, spread = random() < .3 ? 400 : 30;
    let x = cx, y = cy;
    const points = [];
    for (let k = 0; k < length; k++) {
        const roll = random();
        if (roll < .05 && k) { /* duplicate point: zero-length segment */ }
        else if (roll < .08) { x = Math.round(x); y = Math.round(y); }
        else { x += (random() - .5) * spread * .2; y += (random() - .5) * spread * .2; }
        points.push({x, y, p: .5});
    }
    return {id: `s${i}`, tool: random() < .1 ? 'eraser-stroke' : random() < .1 ? 'highlighter' : 'pen', color: '#000', width: random() < .15 ? undefined : random() * 12, points};
}

test('segment distance and hypot are bit-identical to the original JavaScript', () => {
    const random = rng(7);
    const specials = [0, -0, 1, -1, 1e-300, 1e300, Infinity, -Infinity, NaN, 5e-324, 123456.789, 0.1, 3, 4];
    for (let n = 0; n < 200000; n++) {
        const pick = () => random() < .1 ? specials[Math.floor(random() * specials.length)] : (random() - .5) * (random() < .5 ? 10 : 2000);
        const a = [pick(), pick(), pick(), pick(), pick(), pick()];
        if (random() < .1) { a[4] = a[2]; a[5] = a[3]; }
        const expected = legacy.distancePointToSegment(...a), actual = rust.segmentDistance(...a);
        assert.ok(same(expected, actual), `${a} -> ${expected} vs ${actual}`);
    }
});

test('bounds and may-intersect match for fixtures, odd widths and non-finite coordinates', () => {
    const fixtures = [
        {points: [{x: 1, y: 2}], width: 4},
        {points: [{x: 1, y: 2}, {x: -3, y: 9}, {x: 7, y: -1}], width: undefined},
        {points: [{x: NaN, y: 1}, {x: 2, y: NaN}, {x: 5, y: 5}], width: 2},
        {points: [{x: Infinity, y: 0}, {x: 0, y: -Infinity}], width: 1},
        {points: [{x: 0, y: 0}, {x: 10, y: 0}], width: '6'},
        {points: [{x: 0, y: 0}, {x: 10, y: 0}], width: NaN},
        {points: [{x: -0, y: 0}, {x: 1e308, y: 1e308}], width: 1e308},
        {points: [], width: 3}, null, {points: null}, {},
    ];
    for (const stroke of fixtures) {
        const a = legacy.getStrokeBounds(stroke), b = rust.strokeBounds(stroke);
        if (a === null) assert.equal(b, null);
        else for (const key of ['minX', 'minY', 'maxX', 'maxY']) assert.ok(same(a[key], b[key]), `${JSON.stringify(stroke)} ${key}`);
        for (const [x, y, r, extra] of [[0, 0, 0, 0], [5, 5, 3, 2], [-10, 0, 1, 2], [NaN, 0, 1, 0], [1e9, 0, Infinity, 0], [0, 0, NaN, 2], [3, 3, -50, 2], [20, 0, 4, 2]]) {
            assert.equal(legacy.strokeMayIntersectCircle(stroke, x, y, r, extra), rust.strokeMayIntersect(stroke, x, y, r, extra), JSON.stringify([stroke, x, y, r, extra]));
        }
    }
});

test('eraser hit lists equal the original loop on thousands of seeded pages, including exact-radius cases', () => {
    const random = rng(2024);
    let total = 0, hitCount = 0, boundary = 0;
    for (let page = 0; page < 120; page++) {
        const strokes = Array.from({length: 1 + Math.floor(random() * 45)}, (_, i) => randomStroke(random, i));
        for (let q = 0; q < 60; q++) {
            let x, y, r;
            const target = strokes[Math.floor(random() * strokes.length)];
            const pt = target.points[Math.floor(random() * target.points.length)];
            const roll = random();
            if (roll < .5) { x = pt.x + (random() - .5) * 20; y = pt.y + (random() - .5) * 20; r = random() * 8; }
            else if (roll < .8 && target.points.length > 1) {
                // exact boundary: radius equals the segment distance, then one ulp either side
                const k = 1 + Math.floor(random() * (target.points.length - 1)), a = target.points[k - 1], b = target.points[k];
                x = a.x + (b.x - a.x) * random() + (random() - .5) * 6; y = a.y + (b.y - a.y) * random() + (random() - .5) * 6;
                const d = legacy.distancePointToSegment(x, y, a.x, a.y, b.x, b.y);
                const f = new Float64Array([d]), u = new BigUint64Array(f.buffer);
                const choice = Math.floor(random() * 3);
                if (choice === 1) { u[0] += 1n; } else if (choice === 2 && u[0] > 0n) { u[0] -= 1n; }
                r = f[0]; boundary++;
            } else { x = random() * 600; y = random() * 800; r = random() * 60; }
            const generation = GEN(page * 1000 + Math.floor(q / 20));
            const expected = legacy.hits(strokes, x, y, r);
            const actual = rust.strokesHitCircle(strokes, x, y, r, generation);
            assert.deepEqual(actual, expected, `page ${page} query ${q}: ${[x, y, r]}`);
            total++; hitCount += expected.length;
        }
    }
    assert.ok(hitCount > 500 && boundary > 1000, `${hitCount} hits, ${boundary} boundary cases`);
    console.log(`eraser differential: ${total} queries, ${hitCount} hits, ${boundary} threshold cases`);
});

test('non-finite coordinates and radii match the original (NaN never hits, Infinity behaves identically)', () => {
    const strokes = [
        {points: [{x: 0, y: 0}, {x: NaN, y: 4}, {x: 10, y: 10}], width: 2},
        {points: [{x: 0, y: 0}, {x: Infinity, y: 0}], width: 2},
        {points: [{x: -Infinity, y: 5}, {x: 5, y: 5}], width: 2},
        {points: [{x: 1, y: 1}, {x: 1, y: 1}], width: 2},
        {points: [{x: 1e300, y: 1e300}, {x: -1e300, y: 1e300}], width: 0},
    ];
    for (const [x, y, r] of [[0, 0, 1], [NaN, 0, 1], [0, 0, NaN], [5, 5, Infinity], [1e300, 1e300, 1], [1, 1, 0], [1, 1, -0], [1, 1, -1], [Infinity, 0, 1], [3, 5, 2]]) {
        assert.deepEqual(rust.strokesHitCircle(strokes, x, y, r, GEN(1)), legacy.hits(strokes, x, y, r), String([x, y, r]));
    }
});

test('retained coordinates follow generation, array replacement and length changes', () => {
    const line = (y, n = 300) => Array.from({length: n}, (_, i) => ({x: i, y, p: .5}));
    const stroke = {tool: 'pen', width: 2, points: line(0)};
    assert.deepEqual(rust.strokesHitCircle([stroke], 150, 3, 1, GEN(1)), []);
    stroke.points[150].y = 3;                                      // in-place edit: same bounds, new coordinates
    assert.deepEqual(rust.strokesHitCircle([stroke], 150, 3, 1, GEN(1)), [], 'same generation keeps retained geometry');
    assert.deepEqual(rust.strokesHitCircle([stroke], 150, 3, 1, GEN(2)), [0], 'a document edit invalidates it');
    stroke.points = line(100, 300);                                // array replaced
    assert.deepEqual(rust.strokesHitCircle([stroke], 20, 100, 1, GEN(2)), [0]);
    stroke.points = line(100, 250);                                // shorter
    assert.deepEqual(rust.strokesHitCircle([stroke], 280, 100, 1, GEN(2)), []);
    assert.equal(rust.strokeBounds(stroke).maxX, 249);
    const eraser = {tool: 'eraser-area', width: 30, points: line(100)};
    assert.deepEqual(rust.strokesHitCircle([eraser, stroke], 20, 100, 1, GEN(2)), [1]);
    assert.deepEqual(rust.strokesHitCircle([], 0, 0, 1, GEN(2)), []);
});

test('a page larger than the store capacity is cleared and re-uploaded without wrong answers', () => {
    const shared = Array.from({length: 2_200_000}, (_, i) => ({x: i % 5000, y: Math.floor(i / 5000), p: .5}));
    const strokes = [0, 1, 2].map(i => ({id: `big${i}`, tool: 'pen', width: 1, points: shared}));
    for (let round = 0; round < 2; round++) {
        assert.deepEqual(rust.strokesHitCircle(strokes, 100, 3, .5, GEN(round + 50)), [2, 1, 0]);
        assert.deepEqual(rust.strokesHitCircle(strokes, 100, 3000, .5, GEN(round + 50)), []);
    }
    assert.throws(() => rust.strokesHitCircle([{tool: 'pen', points: shared.concat(shared, shared)}], 0, 0, 1, GEN(99)), /too many points/);
});

test('the per-page list follows splices, replaced strokes and replaced point arrays within one generation', () => {
    const random = rng(99);
    const strokes = Array.from({length: 30}, (_, i) => randomStroke(random, i, {length: 20 + (i % 5) * 60}));
    const check = (label) => {
        for (let q = 0; q < 25; q++) {
            const s = strokes[Math.floor(random() * strokes.length)], p = s.points[Math.floor(random() * s.points.length)];
            const [x, y, r] = [p.x + (random() - .5) * 8, p.y + (random() - .5) * 8, random() * 6];
            assert.deepEqual(rust.strokesHitCircle(strokes, x, y, r, GEN(777)), legacy.hits(strokes, x, y, r), `${label} ${[x, y, r]}`);
        }
    };
    check('initial');
    strokes.splice(5, 1); check('after splice');
    strokes[3] = randomStroke(random, 300, {length: 80}); check('replaced stroke');
    strokes[10].points = strokes[10].points.map(pt => ({...pt, x: pt.x + 40})); check('replaced point array');
    strokes[11].points = strokes[11].points.slice(0, -4); check('shorter point array');
    strokes.push(randomStroke(random, 400, {length: 300})); check('pushed');
    strokes.length = 0; assert.deepEqual(rust.strokesHitCircle(strokes, 0, 0, 5, GEN(777)), []);
});
