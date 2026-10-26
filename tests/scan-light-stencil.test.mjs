// Differential test for the Rust port of removePhotographedStencil / isYellow:
// pixels and statistics must be byte-identical with the original JavaScript.
import test from 'node:test';
import assert from 'node:assert/strict';
import { rng } from './helpers/legacy.mjs';
import { loadLegacyLightweight, loadRustCore, rustStencil, syntheticPage, noisePage, mutatedPage } from './helpers/scan-light-fixtures.mjs';

const legacy = loadLegacyLightweight();
const rust = await loadRustCore();

async function compare(label, page, settings, target, pxPerCm = page.pxPerCm) {
    const expected = page.data.slice(), actual = page.data.slice();
    const wantStats = await legacy.removePhotographedStencil(expected, page.width, page.height, pxPerCm, target, settings);
    const { stats } = rustStencil(rust, actual, page.width, page.height, pxPerCm, target, settings);
    assert.deepEqual(JSON.parse(JSON.stringify(stats)), JSON.parse(JSON.stringify(wantStats)), `${label}: statistics`);
    let first = -1;
    for (let i = 0; i < expected.length; i++) if (expected[i] !== actual[i]) { first = i; break; }
    assert.equal(first, -1, `${label}: first differing byte ${first} (pixel ${first >> 2}, channel ${first & 3}): js ${expected[first]} rust ${actual[first]}`);
    return { stats, changed: expected.reduce((n, v, i) => n + (v !== page.data[i] ? 1 : 0), 0) };
}

test('isYellow predicate is identical for every one of the 16.7 million 8-bit colours', () => {
    let yellow = 0;
    for (let r = 0; r < 256; r++) for (let g = 0; g < 256; g++) for (let b = 0; b < 256; b++) {
        const expected = legacy.isYellow(r, g, b);
        if (expected) yellow++;
        if (rust.scanLight.isYellow(r, g, b) !== expected) assert.fail(`isYellow(${r},${g},${b}): js ${expected}`);
    }
    assert.ok(yellow > 10000);
});

test('observed-dot path matches the original on synthetic pages', async () => {
    const methods = new Set();
    let removed = 0;
    for (let seed = 1; seed <= 12; seed++) {
        const page = syntheticPage(seed, 10 + seed % 5 * 3, { observationRate: 0.9, strokes: seed * 3 });
        const settings = { dotObservations: page.geometric, photographedDotObservations: page.photographed };
        const { stats } = await compare(`observed seed ${seed}`, page, settings, page.paper);
        methods.add(stats.method); removed += stats.isolatedRemoved;
    }
    assert.ok(methods.has('observed-dots'), 'the observed-dot path must be exercised');
    assert.ok(removed > 100, 'isolated glyph removal must be exercised');
});

test('recovered-phase path matches the original on synthetic pages', async () => {
    let removed = 0, rejected = 0, protectedComponents = 0;
    for (let seed = 100; seed < 112; seed++) {
        const page = syntheticPage(seed, 9 + seed % 4 * 3, { strokes: 4 + (seed % 7) * 6 });
        const { stats } = await compare(`phase seed ${seed}`, page, {}, page.paper);
        assert.equal(stats.method, 'recovered-dot-phase');
        removed += stats.isolatedRemoved; rejected += stats.isolatedRejected; protectedComponents += stats.protectedComponents;
    }
    assert.ok(removed > 0 && rejected > 0 && protectedComponents > 0, `branch coverage ${removed}/${rejected}/${protectedComponents}`);
});

test('chromatic print, extreme observations and sparse pages match the original', async () => {
    for (let seed = 200; seed < 216; seed++) {
        const page = syntheticPage(seed, 11 + seed % 3 * 4, { neutral: seed % 2 === 0, missing: seed % 4 === 0 ? 0.6 : 0.03,
            observationJitter: seed % 3 ? 0.6 : 2.5, observationRate: seed % 5 === 0 ? 0.3 : 0.97, noise: seed % 6 === 0 ? 8 : 1 });
        const settings = seed % 4 === 3 ? { dotObservations: page.geometric } : { dotObservations: page.geometric, photographedDotObservations: page.photographed };
        await compare(`variant seed ${seed}`, page, settings, seed % 7 === 0 ? 247.5 : page.paper);
    }
});

test('malformed observation records are ignored exactly like the original', async () => {
    const page = syntheticPage(300, 12, { observationRate: 1 });
    const pool = [...page.geometric, ...page.photographed];
    const random = rng(7);
    const damaged = pool.map(point => {
        const copy = { ...point };
        const roll = random();
        if (roll < 0.05) copy.x = '12'; else if (roll < 0.1) copy.y = NaN; else if (roll < 0.15) { delete copy.col; delete copy.column; }
        else if (roll < 0.2) copy.row = 1.5; else if (roll < 0.25) copy.col = 99; else if (roll < 0.3) { copy.col = null; copy.column = 4; copy.row = -1; }
        else if (roll < 0.35) copy.col = '3'; else if (roll < 0.4) { copy.centerRefined = 1; copy.dotContrast = Infinity; }
        return copy;
    });
    await compare('malformed', page, { dotObservations: damaged.slice(0, damaged.length >> 1), photographedDotObservations: damaged.slice(damaged.length >> 1) }, page.paper);
    // Lists outside the 600..6000 window are ignored.
    await compare('too few', page, { dotObservations: damaged.slice(0, 599) }, page.paper);
    await compare('too many', page, { dotObservations: Array.from({ length: 6001 }, (_, i) => pool[i % pool.length]) }, page.paper);
    await compare('not arrays', page, { dotObservations: 'x', photographedDotObservations: {} }, page.paper);
});

const FUZZ = Number(process.env.SCAN_LIGHT_FUZZ || 300), NOISE = Number(process.env.SCAN_LIGHT_NOISE || 24);

test(`${NOISE} seeded noise pages (phase search, tiny sizes, fractional targets) match the original`, async () => {
    // Set SCAN_LIGHT_NOISE=3000 for the long run (the original needs seconds per page).
    for (let seed = 1; seed <= NOISE; seed++) {
        const page = noisePage(seed);
        await compare(`noise seed ${seed} (${page.width}x${page.height} @${page.pxPerCm})`, page, {}, page.target);
    }
});

test(`${FUZZ} seeded damaged pages (crops, wrapped rows, corrupted pixels, bad observations) match the original`, async () => {
    // Set SCAN_LIGHT_FUZZ=3000 for the long run. Pages that fall back to the (slow in JS) phase search are capped.
    let phaseBudget = Math.max(10, FUZZ >> 4), observed = 0, phase = 0, skipped = 0, removed = 0;
    for (let seed = 1; seed <= FUZZ; seed++) {
        const page = mutatedPage(seed);
        const settings = { dotObservations: page.geometric, photographedDotObservations: page.photographed };
        const probe = page.data.slice();
        const method = rustStencil(rust, probe, page.width, page.height, page.pxPerCm, page.target, settings).stats.method;
        if (method === 'recovered-dot-phase' && phase >= phaseBudget) { skipped++; continue; }
        const { stats } = await compare(`fuzz seed ${seed} (${page.width}x${page.height} @${page.pxPerCm.toFixed(2)})`, page, settings, page.target);
        if (method === 'observed-dots') observed++; else phase++;
        removed += stats.isolatedRemoved;
    }
    console.log(`fuzz: ${observed} observed-dot pages, ${phase} phase-search pages, ${skipped} skipped, ${removed} isolated glyphs removed`);
    assert.ok(observed + phase >= Math.min(FUZZ, 100) * 0.4, 'most cases must be compared');
});

test('random page sizes and scales with real structure match the original', async () => {
    const random = rng(99);
    for (let seed = 400; seed < 440; seed++) {
        const px = 4 + random() * 14;
        const page = syntheticPage(seed, px, { strokes: Math.round(random() * 60), observationRate: random() < 0.5 ? 0.9 : 0 });
        const settings = page.geometric.length > 600 ? { dotObservations: page.geometric, photographedDotObservations: page.photographed } : {};
        await compare(`scale ${px.toFixed(2)} seed ${seed}`, page, settings, page.paper - (random() < 0.3 ? 0.5 : 0));
    }
});

test('step API yields control and rejects misuse', () => {
    const page = syntheticPage(5, 8);
    const job = rust.scanLight.stencil(page.data.slice(), page.width, page.height, 8, 240, [], []);
    let steps = 1; while (job.step()) steps++;
    assert.ok(steps > 40, 'the work is split into many bounded slices');
    assert.equal(job.result().method, 'recovered-dot-phase');
    job.dispose(); job.dispose();
    assert.throws(() => job.step(), /released/);
    assert.equal(rust.scanLight.stencil(new Uint8ClampedArray(12), 2, 2, 8, 240, [], []), null);
    assert.equal(rust.scanLight.stencil(new Uint8ClampedArray(16), 2, 2, NaN, 240, [], []), null);
    assert.equal(rust.scanLight.stencil(new Uint8ClampedArray(16), 2, 2, 8, Infinity, [], []), null);
    const early = rust.scanLight.stencil(new Uint8ClampedArray(16), 2, 2, 8, 240, [], []);
    assert.throws(() => early.result(), /unavailable/);
    early.dispose();
});

test('scene from the observed-dot cleanup browser spec (displaced grey dots, pencil, punctuation, blue ink)', async () => {
    const width = 840, height = 1188, data = new Uint8ClampedArray(width * height * 4).fill(255);
    const disc = (cx, cy, radius, value, rgb = [value, value, value]) => {
        for (let y = Math.floor(cy - radius - 1); y <= Math.ceil(cy + radius + 1); y++) for (let x = Math.floor(cx - radius - 1); x <= Math.ceil(cx + radius + 1); x++) {
            if (x < 0 || y < 0 || x >= width || y >= height || Math.hypot(x + 0.5 - cx, y + 0.5 - cy) > radius) continue;
            for (let c = 0; c < 3; c++) data[(y * width + x) * 4 + c] = rgb[c];
        }
    };
    const stroke = (y, rgb, x1 = 105, x2 = 735, half = 0.75) => {
        for (let x = x1; x <= x2; x++) for (let dy = -Math.ceil(half); dy <= Math.ceil(half); dy++) {
            const o = ((y + dy) * width + x) * 4; for (let c = 0; c < 3; c++) data[o + c] = rgb[c];
        }
    };
    const observations = [];
    for (let col = 0; col < 35; col++) for (let row = 0; row < 51; row++) {
        const x = (2 + col * .5) * 40 + 4 * Math.sin(row * .23) * Math.sin(col * .12);
        const y = (1.93 + row * .5) * 40 + 3 * Math.sin(col * .28);
        disc(x, y, 1.2, 0xb8);
        if ((row * 3 + col * 7) % 11) observations.push({ x, y, row, col });
    }
    const punctuation = observations.find(point => point.row === 25 && point.col === 17);
    disc(punctuation.x, punctuation.y, 1.8, 0x64);
    stroke(627, [0x96, 0x96, 0x96]); stroke(700, [0x21, 0x4f, 0xb0], 105, 735, 1);
    const { stats } = await compare('browser-spec scene', { data, width, height, pxPerCm: 40 }, { dotObservations: observations }, 255);
    assert.equal(stats.method, 'observed-dots');
    assert.ok(stats.confirmed > 1000);
});
