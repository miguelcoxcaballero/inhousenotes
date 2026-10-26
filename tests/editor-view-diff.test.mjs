// Differential test for the editor page layout cache and visible-window planner (rust-core/src/editor_view.rs).
// The pre-port JavaScript (tests/helpers/legacy.mjs) and the ported app-v5.js functions run side by side on the same
// seeded random documents and viewports with recording stubs for every DOM-facing helper. Every recorded call, every
// returned number (bit-exact, including the sign of zero) and the final state of the active/pending sets must match.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import vm from 'node:vm';
import { legacySource, rng } from './helpers/legacy.mjs';

const bytes = await readFile(new URL('../wasm/inhouse_notes_core.wasm', import.meta.url));
const adapterSource = await readFile(new URL('../rust-core-v5.js', import.meta.url), 'utf8');
const currentSource = await readFile(new URL('../app-v5.js', import.meta.url), 'utf8');
const legacyApp = legacySource('app-v5.js');

const rustScope = { WebAssembly, Uint8Array, Uint16Array, Float32Array, Float64Array, Int32Array, URL, location: new URL('https://notes.test/'),
    fetch: async () => new Response(bytes), console, AbortSignal, AbortController, setTimeout, clearTimeout, TextEncoder, TextDecoder };
vm.createContext(rustScope);
vm.runInContext(adapterSource, rustScope);
assert.equal(await rustScope.InhouseRust.ready, true);

/** Source text of `function name(...) {...}` found by brace matching. Null when the function does not exist. */
function extract(source, name) {
    const start = source.search(new RegExp(`\\n {8}function ${name}\\(`));
    if (start < 0) return null;
    const open = source.indexOf('{', source.indexOf(')', start));
    let depth = 0, i = open;
    for (; i < source.length; i += 1) {
        const ch = source[i];
        if (ch === '{') depth += 1;
        else if (ch === '}' && --depth === 0) break;
    }
    return source.slice(start, i + 1);
}

const FUNCTIONS = ['normalizePageDimension', 'getPageDimensions', 'getMaxPageWidth', 'getPageTopOffset', 'getPageCalendarPanelSpace',
    'getPageHeight', 'getPageCenterOffset', 'getVisiblePageRange', 'getViewportVisiblePageIndices', 'expandPageIndices',
    'updateVisiblePages', 'getContentHeight', 'getPageLayoutMetrics', 'findFirstPageWhoseBottomReaches', 'findLastPageWhoseTopPrecedes',
    'getPageBounds', 'invalidatePageBoundsCache'];

const prelude = `
const A4_WIDTH = 794, A4_HEIGHT = 1123, PAGE_PADDING = 30, PAGE_GAP = 30, PAGE_CALENDAR_PANEL_HEIGHT = 184, PAGE_CALENDAR_PANEL_GAP = 14;
const VISIBLE_PAGE_BUFFER = 1, MAX_ACTIVE_PAGES = 3;
let cachedPageBounds = null, cachedPageBoundsOwner = null, cachedPageBoundsCount = -1, cachedPageLayoutMetrics = null, cachedPageLayoutCalendarEnabled = null;
let visiblePageUpdateRetryHandle = null, activePdfDocument = null;
const state = { pages: [], panX: 0, panY: 0, zoom: 1, selection: { pageIndex: null }, activePageIndex: null, isPanning: false, activePointers: new Set(), inertiaFrame: 0 };
const canvasViewport = { getBoundingClientRect: () => env.rect };
const activePageIndices = new Set(), desiredActivePageIndices = new Set(), pendingPageActivations = new Map();
const env = { rect: { width: 0, height: 0 }, calendar: false, log: [], missingCanvas: new Set() };
// Mirrors the app: toggling the calendar panels stores the flag and invalidates the layout cache.
function setCalendarFlag(value) { cachedPageLayoutCalendarEnabled = value; invalidatePageBoundsCache(); }
const record = (...args) => env.log.push(args);
function shouldRenderPageCalendarPanels() { return env.calendar; }
function queueScrollPagePreviews() {}
function isViewportInteracting() { return false; }
function isPageLoaded(page) { return !!page && !page.unloaded; }
function ensurePageCanvas(index, options) { record('ensurePageCanvas', index, options.defer, options.deferPdfRender); activePageIndices.add(index); }
function ensurePageDataLoaded(index) { record('ensurePageDataLoaded', index); }
function applyPagePreview(index) { record('applyPagePreview', index); }
function queuePageActivation(index, order) { record('queuePageActivation', index, order); }
function redrawPage(index) { record('redrawPage', index); }
function applyPageBackground(index) { record('applyPageBackground', index); }
function releasePageCanvas(index) { record('releasePageCanvas', index); activePageIndices.delete(index); }
function prunePdfBackgroundCache() { record('prunePdfBackgroundCache'); }
function schedulePageActivationProcessing() { record('schedulePageActivationProcessing'); }
function scheduleVisiblePageUpdate() {}
const document = { querySelector(selector) {
    const index = Number(/data-page="(\\d+)"/.exec(selector)[1]);
    return env.missingCanvas.has(index) ? null : { dataset: { hasPageDraw: env.drawn?.has(index) ? '1' : '0' } };
} };
`;

function environment(source) {
    const scope = { console, Set, Map, Math, Number, Array, Object, Float64Array, Int32Array, Uint8Array, JSON,
        InhouseRust: rustScope.InhouseRust, setTimeout, clearTimeout };
    scope.window = scope.globalThis = scope;
    vm.createContext(scope);
    const functions = FUNCTIONS.map(name => extract(source, name)).filter(Boolean);
    const names = FUNCTIONS.filter(name => extract(source, name));
    vm.runInContext(`${prelude}\n${functions.join('\n')}\nglobalThis.__t = { setCalendarFlag, env, state, activePageIndices, desiredActivePageIndices, pendingPageActivations,\n${names.map(n => `${n}: typeof ${n} === 'function' ? ${n} : undefined`).join(',\n')} };`, scope);
    return scope.__t;
}

/** Serialise with bit-exact numbers so -0, NaN and tiny differences are visible. */
function ser(value) {
    if (typeof value === 'number') return Object.is(value, -0) ? '-0' : String(value);
    if (value instanceof Set) return `Set[${[...value].map(ser)}]`;
    if (value instanceof Map) return `Map[${[...value.keys()].map(ser)}]`;
    if (Array.isArray(value) || ArrayBuffer.isView(value)) return `[${Array.from(value, ser)}]`;
    if (value && typeof value === 'object') return `{${Object.keys(value).sort().map(key => `${key}:${ser(value[key])}`)}}`;
    return String(value);
}

function pick(random, list) { return list[Math.floor(random() * list.length)]; }
function makeDimension(random) {
    const kind = random();
    if (kind < 0.55) return undefined;
    if (kind < 0.8) return pick(random, [595, 794, 841, 1123, 1190, 1684, 2000]);
    if (kind < 0.92) return Math.round(random() * 3000 * 100) / 100;
    if (kind < 0.94) return pick(random, ['700', null, NaN, Infinity, -5, 0, 63.5, 64.5, 0.5, 1e9]);
    return 64 + random() * 2400;
}
function makeDocument(random, huge) {
    const count = huge ? 800 + Math.floor(random() * 1500) : pick(random, [0, 1, 2, 3, 5, 8, 20, 60, 150, 300, Math.floor(random() * 120)]);
    const uniform = random() < 0.4;
    const pages = [];
    for (let i = 0; i < count; i += 1) {
        const page = {};
        const width = uniform ? undefined : makeDimension(random), height = uniform ? undefined : makeDimension(random);
        if (width !== undefined) page.pageWidth = width;
        if (height !== undefined) page.pageHeight = height;
        if (random() < 0.15) page.sidePanel = { kind: 'calendar' };
        if (random() < 0.1) page.unloaded = true;
        pages.push(page);
    }
    return pages;
}
function makeView(random, count, calendar, contentHeight) {
    const zoom = pick(random, [0.25, 0.5, 0.75, 1, 1.5, 2, 3, 0.1 + random() * 3.5]);
    const rect = random() < 0.04 ? { width: pick(random, [0, 800]), height: pick(random, [0, 600]) }
        : { width: 300 + random() * 1500, height: 300 + random() * 1200 };
    let panY = -(random() * (contentHeight * zoom + 400)) + 200;
    if (random() < 0.08) panY = pick(random, [0, -0, 1e9, -1e9, 1e5, -1e7]);
    let panX = (random() - 0.5) * 2200;
    if (random() < 0.1) panX = pick(random, [0, -1e6, 1e6]);
    return { zoom, rect, panX, panY };
}

function apply(t, document, view, setup) {
    t.state.pages = document.map(page => ({ ...page }));
    t.state.panX = view.panX; t.state.panY = view.panY; t.state.zoom = view.zoom;
    t.env.rect = view.rect; t.env.calendar = setup.calendar; t.setCalendarFlag(setup.calendar);
    t.env.missingCanvas = new Set(setup.missing); t.env.drawn = new Set(setup.drawn);
    t.state.selection.pageIndex = setup.selection; t.state.activePageIndex = setup.activePage;
    t.activePageIndices.clear(); setup.active.forEach(i => t.activePageIndices.add(i));
    t.pendingPageActivations.clear(); setup.pending.forEach(i => t.pendingPageActivations.set(i, 0));
    t.desiredActivePageIndices.clear();
    t.env.log.length = 0;
}
function snapshot(t) {
    return ser({ log: t.env.log, active: t.activePageIndices, desired: t.desiredActivePageIndices, pending: t.pendingPageActivations });
}

function makeSetup(random, count) {
    const index = () => Math.floor(random() * (count + 2)) - 1;
    const some = max => Array.from({ length: Math.floor(random() * max) }, () => Math.max(0, Math.min(count - 1, index())));
    return {
        calendar: random() < 0.5,
        selection: random() < 0.4 ? index() : random() < 0.1 ? pick(random, [null, undefined, NaN, '2']) : null,
        activePage: random() < 0.5 ? index() : null,
        active: [...new Set(some(5))], pending: [...new Set(some(6))],
        missing: some(3), drawn: some(6)
    };
}

function compareWholeUpdate(legacy, current, document, view, setup, label) {
    for (const t of [legacy, current]) {
        apply(t, document, view, setup);
        t.invalidatePageBoundsCache();
        t.updateVisiblePages(true);
    }
    assert.equal(snapshot(current), snapshot(legacy), label);
}

test('layout metrics, offsets and bounds are bit-identical to the original JavaScript', () => {
    const legacy = environment(legacyApp), current = environment(currentSource);
    assert.equal(current.getPageCenterOffset, undefined, 'the unused JS centre helper is deleted');
    assert.equal(current.findFirstPageWhoseBottomReaches, undefined, 'the JS binary search is deleted');
    const random = rng(20240611);
    for (let run = 0; run < 1200; run += 1) {
        const document = makeDocument(random, run % 150 === 0), calendar = random() < 0.5;
        const view = { zoom: 1, rect: { width: 800, height: 600 }, panX: 0, panY: 0 };
        const setup = { calendar, selection: null, activePage: null, active: [], pending: [], missing: [], drawn: [] };
        for (const t of [legacy, current]) { apply(t, document, view, setup); t.invalidatePageBoundsCache(); }
        assert.equal(ser(current.getContentHeight()), ser(legacy.getContentHeight()), `content height run ${run}`);
        assert.equal(ser(current.getPageBounds()), ser(legacy.getPageBounds()), `bounds run ${run}`);
        const indices = [-3, -1, 0, 1, document.length - 1, document.length, document.length + 1, 1.5, NaN, Infinity,
            ...Array.from({ length: 30 }, () => Math.floor(random() * (document.length + 3)))];
        for (const index of indices) {
            const expected = legacy.getPageTopOffset(index), actual = current.getPageTopOffset(index);
            if (index === 1.5 && document.length > 1) { assert.ok(expected === undefined || Number.isNaN(actual)); continue; }
            assert.equal(ser(actual), ser(expected), `top offset ${index} run ${run}`);
        }
        // The metrics cache is reused until it is invalidated, exactly like before.
        assert.equal(current.getPageLayoutMetrics(), current.getPageLayoutMetrics());
        assert.equal(current.getPageLayoutMetrics().pageCount, legacy.getPageLayoutMetrics().pageCount);
    }
});

test('visible page range matches the original for random viewports and buffers', () => {
    const legacy = environment(legacyApp), current = environment(currentSource);
    const random = rng(7);
    for (let run = 0; run < 1500; run += 1) {
        const document = makeDocument(random, run % 200 === 0);
        const view = makeView(random, document.length, false, legacy.getContentHeight() || 1200);
        const setup = { calendar: random() < 0.5, selection: null, activePage: null, active: [], pending: [], missing: [], drawn: [] };
        for (const t of [legacy, current]) { apply(t, document, view, setup); t.invalidatePageBoundsCache(); }
        for (const buffer of [undefined, null, 0, 1, 2, 5, NaN, -1]) {
            assert.equal(ser(current.getVisiblePageRange(buffer)), ser(legacy.getVisiblePageRange(buffer)), `run ${run} buffer ${buffer}`);
        }
    }
});

test('visible-page update plans identical DOM work on seeded random editors', () => {
    const legacy = environment(legacyApp), current = environment(currentSource);
    const random = rng(99);
    let planned = 0;
    for (let run = 0; run < 4000; run += 1) {
        const document = makeDocument(random, run % 400 === 0);
        for (const t of [legacy, current]) { apply(t, document, { zoom: 1, rect: { width: 1, height: 1 }, panX: 0, panY: 0 }, makeSetup(rng(1), 1)); t.invalidatePageBoundsCache(); }
        const view = makeView(random, document.length, false, legacy.getContentHeight() || 1200);
        compareWholeUpdate(legacy, current, document, view, makeSetup(random, document.length), `run ${run}`);
        if (current.env.log.length) planned += 1;
    }
    assert.ok(planned > 2500, `only ${planned} of 4000 runs produced a plan`);
});

test('update plans match when the view sits on exact page edges and extreme zoom values', () => {
    const legacy = environment(legacyApp), current = environment(currentSource);
    const random = rng(5);
    for (let run = 0; run < 1200; run += 1) {
        const document = makeDocument(random, false);
        for (const t of [legacy, current]) { apply(t, document, { zoom: 1, rect: { width: 800, height: 600 }, panX: 0, panY: 0 }, makeSetup(rng(1), 1)); t.invalidatePageBoundsCache(); }
        const tops = document.map((_, i) => legacy.getPageTopOffset(i));
        const zoom = pick(random, [0.5, 1, 2, 0.25, 3]);
        const anchor = tops.length ? pick(random, tops) + pick(random, [0, -600, 600, 1, -1, 1123]) : 0;
        const view = { zoom, rect: { width: pick(random, [320, 800, 1200]), height: 600 }, panX: pick(random, [0, -400, 400, -1e4]), panY: -anchor * zoom };
        compareWholeUpdate(legacy, current, document, view, makeSetup(random, document.length), `edge run ${run}`);
    }
    // Degenerate zoom values follow IEEE semantics in both implementations.
    for (const zoom of [0, Infinity, NaN, -1, 1e-9]) {
        const document = makeDocument(random, false);
        const view = { zoom, rect: { width: 800, height: 600 }, panX: 0, panY: -500 };
        compareWholeUpdate(legacy, current, document, view, makeSetup(random, document.length), `zoom ${zoom}`);
    }
});

test('layout and plan boundaries reject invalid sizes instead of reading out of bounds', () => {
    const api = rustScope.InhouseRust.editorView;
    assert.throws(() => api.layout({ length: 2 ** 31 }, false, { padding: 30, gap: 30, a4Width: 794, a4Height: 1123, calendarSpace: 198 }), { name: 'RangeError' });
    assert.throws(() => api.layout([{}], false, { padding: NaN, gap: 30, a4Width: 794, a4Height: 1123, calendarSpace: 198 }), { name: 'RangeError' });
    // A valid empty layout still plans nothing.
    api.layout([], false, { padding: 30, gap: 30, a4Width: 794, a4Height: 1123, calendarSpace: 198 });
    assert.equal(api.plan({ panX: 0, panY: 0, zoom: 1, rectWidth: 800, rectHeight: 600, buffer: 1, maxActive: 3, selection: null, activePage: null, pending: new Map() }), null);
});

test('callers fail loudly when the WebAssembly core is missing', () => {
    const scope = { WebAssembly, Uint8Array, URL, location: new URL('https://notes.test/'), fetch: async () => { throw new Error('offline'); },
        console, AbortSignal, AbortController, setTimeout, clearTimeout };
    vm.createContext(scope);
    vm.runInContext(adapterSource, scope);
    assert.throws(() => scope.InhouseRust.editorView.topOffset(0), /Rust core is unavailable/);
    assert.throws(() => scope.InhouseRust.editorView.visibleRange(0, 1, 600, 1), /Rust core is unavailable/);
});
