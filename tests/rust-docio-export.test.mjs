// Differential test: the Rust export-meta unit (rust-core/src/docio) against the original JavaScript
// that lived in app-v5.js before the port. The original code is not re-typed here: it is cut out of the
// pre-port source (git show a2dea10:app-v5.js) so a change in that text fails the extraction.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import vm from 'node:vm';
import { legacySource, loadLegacy, rng } from './helpers/legacy.mjs';

const bytes = await readFile(new URL('../wasm/inhouse_notes_core.wasm', import.meta.url));
const scope = { WebAssembly, Uint8Array, Uint16Array, Uint32Array, Float32Array, Float64Array, DataView, Map, URL, TextEncoder, TextDecoder,
    location: new URL('https://notes.test/'), fetch: async () => new Response(bytes), console, AbortSignal, AbortController, setTimeout, clearTimeout };
vm.createContext(scope);
vm.runInContext(await readFile(new URL('../rust-core-v5.js', import.meta.url), 'utf8'), scope);
assert.equal(await scope.InhouseRust.ready, true, 'wasm/inhouse_notes_core.wasm must be built (node scripts/build-rust-core.mjs)');
const docio = scope.InhouseRust.docio;

// ---------------------------------------------------------------- legacy extraction
const app = legacySource('app-v5.js');
function between(start, end, from = 0) {
    const a = app.indexOf(start, from);
    assert.ok(a >= 0, `legacy text not found: ${start.slice(0, 60)}`);
    const b = app.indexOf(end, a);
    assert.ok(b >= 0, `legacy text not found: ${end.slice(0, 60)}`);
    return app.slice(a, b + end.length);
}
function legacyFunction(name) {
    const a = app.indexOf(`function ${name}(`);
    assert.ok(a >= 0, `legacy function ${name}`);
    let i = app.indexOf('{', app.indexOf(')', a)), depth = 0;
    for (; i < app.length; i++) {
        if (app[i] === '{') depth++;
        else if (app[i] === '}' && --depth === 0) return app.slice(a, i + 1);
    }
    throw new Error(`unterminated ${name}`);
}
const legacy = loadLegacy('collaboration-core-v5.js');
vm.runInContext([
    'const A4_WIDTH = 794, A4_HEIGHT = 1123, CANVAS_SCALE = 2;',
    "const TEMPLATE_KINDS = Object.freeze({ DEFAULT: 'default', AGENDA: 'agenda', DIARY: 'diary' });",
    ...['normalizeTemplateKind', 'normalizePageDimension', 'simpleHash', 'pointFingerprint', 'strokeSyncFingerprint', 'imageSyncFingerprint',
        'computePageHash', 'getPdfOverlayContentBounds', 'cloneTimelineValue'].map(legacyFunction)
].join('\n'), legacy);
assert.ok(app.includes("const TEMPLATE_KINDS = Object.freeze({ DEFAULT: 'default', AGENDA: 'agenda', DIARY: 'diary' });"));
assert.ok(app.includes('const A4_WIDTH = 794;') && app.includes('const A4_HEIGHT = 1123;') && app.includes('const CANVAS_SCALE = 2;'));

const pdfLibBlock = between('// Stroke metadata for STROKES_Z so re-opens still get vector strokes.', 'metadataPages.push(pageStrokeEntry);');
const rasterBlock = between('// Preserve distinct marks; legacy id-less strokes require all their points to match.', 'const metaJson = JSON.stringify(pageStrokeEntry);');
const rasterBackground = between('const backgroundSource = pageData.backgroundSource', "const backgroundImage = backgroundSource === 'pdf' ? null : (pageData.backgroundImage || null);",
    app.indexOf('async function buildPdfBlob(options = {})'));
const pxToMmText = between('const pxToMm = (px, fallbackPx) => {', 'const getPageOrientation = (wMm, hMm) => (wMm > hMm ? \'landscape\' : \'portrait\');');
const pdfLibPlan = between('const optimizeForUpload = !!options.optimizeForUpload && !options.highQuality;', "'raw'}`;", app.indexOf('async function buildPdfBlobWithPdfLib'));
const rasterPlan = between('const optimizeForUpload = !!options.optimizeForUpload && !options.highQuality;', 'q${jpegQuality}`;', app.indexOf('async function buildPdfBlob(options = {})'));
const copyPlan = between('const sourceIndices = [];', 'let copiedPages = [];', app.indexOf('async function buildPdfBlobWithPdfLib'));
// The placement and pt/px lines are tiny; assert they are still the original text, then use the identical expressions.
for (const text of ['pageWidthPx = pageWidthPt / PX_TO_PT;', 'pageHeightPt = pageHeightPx * PX_TO_PT;', 'const scaleX = pageWidthPt / logicalPageWidthPx;',
    'x: overlayBounds.x * scaleX,', 'y: pageHeightPt - ((overlayBounds.y + overlayBounds.height) * scaleY),', 'width: overlayBounds.width * scaleX,',
    'height: overlayBounds.height * scaleY']) assert.ok(app.includes(text), text);

vm.runInContext(`
async function legacyPdfLibEntry(pageData, i, renderPanels, logicalPageWidthPx, logicalPageHeightPx, sessionToken) {
    const pdfCompactStrokeCache = new WeakMap(), pageDirtyGeneration = new Map(), metadataParts = [], metadataPages = [];
    const isMobileLike = false, yieldToUI = async () => {}, generatePageId = () => 'generated-id', shouldRenderPageCalendarPanels = () => renderPanels;
    ${pdfLibBlock}
    return { metaJson: metadataParts[0], exportStrokes, metadataPages };
}
async function legacyRasterEntry(pageData, i, renderPanels, resolvedPageWidth, resolvedPageHeight) {
    const isMobileLike = false, maybeYieldForSlice = async () => {}, throwIfAborted = () => {}, throwIfSessionInvalid = () => {};
    const generatePageId = () => 'generated-id', shouldRenderPageCalendarPanels = () => renderPanels;
    ${rasterBackground}
    ${rasterBlock}
    return { metaJson, exportStrokes, pageStrokeEntry };
}
function legacyPxToMm(px, fallbackPx) { const f = (() => { ${pxToMmText} return pxToMm; })(); return f(px, fallbackPx); }
function legacyPdfLibPlan(options, backends, pageCount, state, originalCount) {
    const window = { PDFLib: backends.window }, PDFLib = backends.backend;
    const originalDoc = originalCount ? { getPageCount: () => originalCount } : null;
    ${pdfLibPlan}
    ${copyPlan}
    return { overlayScale, exportProfile, stateToSourceIdx, sourceIndices };
}
function legacyRasterPlan(options, pageCount) {
    ${rasterPlan}
    return { exportScale, jpegQuality, exportProfile };
}
`, legacy);

// ---------------------------------------------------------------- generators
const jsonClone = value => JSON.parse(JSON.stringify(value));
function makeGen(seed) {
    const r = rng(seed);
    const int = (a, b) => a + Math.floor(r() * (b - a + 1));
    const pick = list => list[int(0, list.length - 1)];
    const chance = p => r() < p;
    const number = () => {
        const kind = int(0, 9);
        if (kind === 0) return int(-50, 1500);
        if (kind === 1) return int(0, 100000) / 100 + (chance(0.3) ? 0.005 : 0) ;
        if (kind === 2) return Math.round(r() * 1e6) / 1e3 + 0.0049999999;
        if (kind === 3) return (r() - 0.5) * 1e-5;
        if (kind === 4) return r() * 794;
        if (kind === 5) return pick([0, 0.5, 1.005, 2.675, 1.255, 0.015, 0.045, 1e21, 1.5e-7, 123456789.125, 1e15 + 0.5, -0.004, 0.125, 99.995]);
        return r() * 1200;
    };
    const text = () => pick(['', 'a', 'pen', '#112233', 'x|y', 'é😀', ' ', 'quote"s', 'back\\slash', 'tab\t', '\ud800', 'ab\ude00', '12', '0']);
    const point = () => {
        const p = { x: number(), y: number() };
        if (chance(0.7)) p.p = chance(0.1) ? 0 : r();
        else if (chance(0.15)) p.p = null;
        if (chance(0.1)) p.t = int(0, 99999);
        if (chance(0.03)) p.x = String(number());
        if (chance(0.02)) delete p.y;
        if (chance(0.02)) p.p = String(r());
        if (chance(0.05)) { const k = pick(['pressure', 'tilt', '7']); p[k] = number(); }
        return p;
    };
    const stroke = idPool => {
        const s = {};
        if (chance(0.8)) s.id = chance(0.2) ? pick(idPool) : `s${int(0, 1e6)}`;
        if (chance(0.95)) s.tool = pick(['pen', 'pen', 'pen', 'highlighter', 'eraser-stroke', 'marker', '']);
        if (chance(0.9)) s.color = pick(['#000000', '#ff0000', '#112233', 'rgba(1,2,3,.5)']);
        if (chance(0.85)) s.width = chance(0.1) ? pick([0, -3, '4', null, 0.2, 40]) : Math.round(r() * 3000) / 100;
        const n = pick([0, 1, 1, 2, 3, 5, 12, 40]);
        if (chance(0.97)) s.points = Array.from({ length: n }, point);
        else if (chance(0.5)) s.points = null; else s.points = 'x';
        if (chance(0.3)) s.extra = { nested: [1, 'two', { three: 3 }], flag: true, none: null };
        if (chance(0.1)) s.syncStamp = { clock: int(0, 10), actor: text() };
        if (chance(0.03)) s['12'] = 'numeric-key';
        if (chance(0.5)) { const keys = Object.keys(s); const shuffled = {}; keys.sort(() => r() - 0.5).forEach(k => { shuffled[k] = s[k]; }); return shuffled; }
        return s;
    };
    const image = () => ({ id: chance(0.9) ? `img${int(0, 99)}` : undefined, src: chance(0.9) ? `data:image/png;base64,${'QUJD'.repeat(int(0, 40))}${chance(0.1) ? '\ud83d' : ''}` : undefined,
        x: number(), y: number(), width: chance(0.1) ? pick([0, -5, 1e9]) : r() * 400, height: r() * 400, rotation: chance(0.5) ? 0 : pick([0.5, Math.PI / 2, -2.3, 1e6, number()]) });
    const stamps = ids => {
        const kind = int(0, 6);
        const stamp = () => chance(0.2) ? { stamp: { clock: int(0, 9), actor: text() } } : { clock: pick([int(0, 99), -3, 2.9, '7', null, 1e21]), actor: chance(0.8) ? text() : pick([5, null, true]) };
        if (kind === 0) return undefined;
        if (kind === 1) return null;
        if (kind === 2) return Object.fromEntries(ids.filter(() => chance(0.6)).map(id => [id, stamp()]).concat(chance(0.3) ? [['', stamp()]] : []));
        if (kind === 3) return ids.filter(() => chance(0.6)).map(id => chance(0.5) ? { id, ...stamp() } : id);
        if (kind === 4) return new Map(ids.filter(() => chance(0.6)).map(id => [id, stamp()]));
        if (kind === 5) return [1, 0, '', null, [], [1], { id: '' }, 'z'];
        return {};
    };
    const page = () => {
        const idPool = Array.from({ length: 5 }, () => `s${int(0, 1e6)}`);
        const strokeCount = pick([0, 1, 2, 3, 6, 10]);
        const strokes = [];
        for (let n = 0; n < strokeCount; n++) {
            if (strokes.length && chance(0.2)) strokes.push(jsonClone(pick(strokes)));
            else strokes.push(stroke(idPool));
        }
        const ids = chance(0.7) ? idPool.concat(chance(0.4) ? ['12', '3', '007', 'é', '😀', '￮'] : []) : [];
        const p = { strokes, images: Array.from({ length: pick([0, 0, 1, 2, 4]) }, image),
            deletedStrokeIds: chance(0.9) ? ids.concat(chance(0.2) ? [0, null, '', 5, true, [1, 2]] : []) : pick([undefined, null, 'abc', { a: 1 }]),
            deletedStrokeStamps: stamps(ids),
            backgroundSource: pick(['pdf', 'template', 'custom', undefined, '', 'pdf', null]),
            pdfPageIndex: pick([undefined, null, 0, 1, 2, 7, '3', NaN]), templateKind: pick([undefined, 'agenda', 'diary', 'default', 'weird']),
            pageWidth: pick([undefined, 794, 595.4, 20, 1e4, NaN, '800', null, 1123.5]), pageHeight: pick([undefined, 1123, 842.5, 3, -1, Infinity]),
            backgroundImage: pick([undefined, null, '', 'data:image/png;base64,AAAA' + 'B'.repeat(int(0, 400)), 'data:x\ud83d' + 'q'.repeat(158)]),
            sidePanel: pick([undefined, null, { a: [1, 2, { b: 'c' }], text: 'é\n' }, 0, '', 'str', { }]),
            pageId: pick([undefined, 'page-1', 'pé', '']) };
        if (!Array.isArray(p.images)) p.images = [];
        if (chance(0.05)) p.strokes = pick([undefined, null, 'abc', {}]);
        if (chance(0.05)) p.images = pick([undefined, null, 'x']);
        return p;
    };
    return { r, int, pick, chance, page, number };
}

const plain = value => value === null || value === undefined ? value : JSON.parse(JSON.stringify(value));

function rustJob(page, i, mode, extra = {}) {
    return { mode, pageId: page.pageId, pageIndex: i, bgSource: page.backgroundSource, pdfIndex: page.pdfPageIndex, hasBgImage: !!page.backgroundImage,
        templateKind: legacy.normalizeTemplateKind(page.templateKind), pageWidth: legacy.normalizePageDimension(page.pageWidth, 794),
        pageHeight: legacy.normalizePageDimension(page.pageHeight, 1123), images: Array.isArray(page.images) ? page.images : [], deletedStrokeIds: page.deletedStrokeIds,
        deletedStrokeStamps: page.deletedStrokeStamps, sidePanel: page.sidePanel, ...extra };
}

// ---------------------------------------------------------------- tests
const ROUNDS = Number(process.env.DOCIO_ROUNDS || 1500);

test('pdf-lib path: dedupe, compaction and metadata entry are byte-identical to the original', async () => {
    const g = makeGen(101);
    let checked = 0, strokesSeen = 0, deduped = 0, withStamps = 0;
    for (let n = 0; n < ROUNDS; n++) {
        const page = g.page();
        const i = g.int(0, 30), renderPanels = g.chance(0.6);
        if (!page.pageId) page.pageId = 'generated-id';
        const width = legacy.normalizePageDimension(page.pageWidth, 794), height = legacy.normalizePageDimension(page.pageHeight, 1123);
        const expectation = await legacy.legacyPdfLibEntry(structuredClone(page), i, renderPanels, width, height, 1).catch(error => ({ error }));
        const strokes = Array.isArray(page.strokes) ? page.strokes : [];
        const input = rustJob(page, i, 'pdflib', { renderPanels, bounds: { width, height, scale: 2.5 } });
        let actual;
        try { actual = docio.pageExport(input, JSON.stringify(strokes)); } catch (error) { actual = { error }; }
        if (expectation.error || actual.error) {
            assert.ok(expectation.error && actual.error, `error parity (round ${n}): legacy=${expectation.error?.message} rust=${actual.error?.message}`);
            continue;
        }
        assert.equal(actual.meta, expectation.metaJson, `meta (round ${n})`);
                assert.equal(actual.keep.length, expectation.exportStrokes.length, `kept count (round ${n})`);
        assert.deepEqual(Array.from(actual.keep, k => JSON.stringify(strokes[k])), Array.from(expectation.exportStrokes, s => JSON.stringify(s)), `kept strokes (round ${n})`);
        const legacyBounds = legacy.getPdfOverlayContentBounds(page, expectation.exportStrokes, width, height, 2.5);
        assert.deepEqual(plain(actual.bounds), plain(legacyBounds), `bounds (round ${n})`);
        // Reusing the cached compact strokes yields the same entry (what a second save of an unchanged page does).
        const reused = docio.pageExport(rustJob(page, i, 'pdflib', { renderPanels, reuse: true }), '', actual.compact);
        assert.equal(reused.meta, actual.meta);
        strokesSeen += expectation.exportStrokes.length; checked++;
        if (expectation.exportStrokes.length < strokes.length) deduped++;
        if (actual.meta.includes('"clock"')) withStamps++;
    }
    assert.ok(checked > ROUNDS * 0.9 && strokesSeen > ROUNDS);
    assert.ok(deduped > ROUNDS / 40 && withStamps > ROUNDS / 4, `generator coverage: deduped=${deduped} stamps=${withStamps}`);
});

test('raster path: dedupe, compaction and metadata entry are byte-identical to the original', async () => {
    const g = makeGen(202);
    let checked = 0;
    for (let n = 0; n < ROUNDS; n++) {
        const page = g.page();
        const i = g.int(0, 30), renderPanels = g.chance(0.6);
        if (!page.pageId) page.pageId = 'generated-id';
        const width = legacy.normalizePageDimension(page.pageWidth, 794), height = legacy.normalizePageDimension(page.pageHeight, 1123);
        const expectation = await legacy.legacyRasterEntry(structuredClone(page), i, renderPanels, width, height).catch(error => ({ error }));
        const strokes = Array.isArray(page.strokes) ? page.strokes : [];
        let actual;
        try { actual = docio.pageExport(rustJob(page, i, 'raster', { renderPanels }), JSON.stringify(strokes)); } catch (error) { actual = { error }; }
        if (expectation.error || actual.error) {
            assert.ok(expectation.error && actual.error, `error parity (round ${n}): legacy=${expectation.error?.message} rust=${actual.error?.message}`);
            continue;
        }
        assert.equal(actual.meta, expectation.metaJson, `meta (round ${n})`);
        assert.deepEqual(Array.from(actual.keep, k => JSON.stringify(strokes[k])), Array.from(expectation.exportStrokes, s => JSON.stringify(s)), `kept strokes (round ${n})`);
        checked++;
    }
    assert.ok(checked > ROUNDS * 0.9);
});

test('overlay bounds equal getPdfOverlayContentBounds, including trigonometry for rotated images', () => {
    const g = makeGen(303);
    let found = 0;
    for (let n = 0; n < ROUNDS * 2; n++) {
        const page = g.page();
        const strokes = (Array.isArray(page.strokes) ? page.strokes : []).filter(s => s && typeof s === 'object');
        const width = g.pick([794, 595, 1123, 64, NaN, 0, 2000]), height = g.pick([1123, 842, 64, undefined, -4]);
        const scale = g.pick([2, 2.5, 3.5, 1, 0, NaN, 1.75]);
        const expected = legacy.getPdfOverlayContentBounds(page, strokes, width, height, scale);
        const actual = docio.overlayBounds(JSON.stringify(strokes), Array.isArray(page.images) ? page.images : [], width, height, scale);
        assert.deepEqual(plain(actual), plain(expected), `round ${n}`);
        if (expected) found++;
    }
    assert.ok(found > ROUNDS / 2, `only ${found} rounds produced bounds`);
    const rotated = Array.from({ length: 20000 }, (_, k) => ({ x: 100, y: 120, width: 90 + (k % 17), height: 40 + (k % 11), rotation: (k - 10000) * 0.00137 }));
    for (const image of rotated) {
        assert.deepEqual(plain(docio.overlayBounds('[]', [image], 794, 1123, 2.5)), plain(legacy.getPdfOverlayContentBounds({ images: [image] }, [], 794, 1123, 2.5)));
    }
});

test('page hash equals computePageHash', () => {
    const g = makeGen(404);
    for (let n = 0; n < ROUNDS * 2; n++) {
        const page = g.page();
        let expected, actual;
        try { expected = legacy.computePageHash(structuredClone(page)); } catch (error) { expected = { error }; }
        try { actual = docio.pageHash(page, 794, 1123); } catch (error) { actual = { error }; }
        if (expected.error || actual.error) { assert.ok(expected.error && actual.error, `error parity (round ${n}): ${expected.error?.message} / ${actual?.error?.message}`); continue; }
        assert.equal(actual, expected, `round ${n}`);
    }
});

test('page hash fixtures', () => {
    const page = { strokes: [{ id: 'a', tool: 'pen', color: '#000', width: 2.345, points: [{ x: 1.005, y: 2, p: 0.5 }, { x: 3, y: 4 }] }, null],
        images: [{ id: 'i', src: 'data:image/png;base64,AAAA', x: 1, y: 2, width: 3, height: 4, rotation: 0.12345 }],
        deletedStrokeIds: ['q', '', 'r'], deletedStrokeStamps: { q: { clock: 3, actor: 'me' } }, backgroundSource: 'pdf', pdfPageIndex: 2,
        backgroundImage: 'data:abc', pageWidth: 800.4, pageHeight: 1000 };
    assert.equal(docio.pageHash(page, 794, 1123), legacy.computePageHash(page));
    assert.equal(docio.pageHash({}, 794, 1123), legacy.computePageHash({}));
    assert.equal(docio.pageHash({ strokes: [] }, 794, 1123), legacy.computePageHash({ strokes: [] }));
});

test('page geometry, mm size and orientation equal the original expressions', () => {
    const g = makeGen(505);
    const values = [undefined, null, NaN, Infinity, -5, 0, 10, 63.5, 64.4, 794, 1123, 595.28, 841.89, 1e6, '800', 2.5, 3000.49999];
    for (let n = 0; n < 4000; n++) {
        const w = g.pick(values), h = g.pick(values);
        const geometry = docio.pageGeometry(w, h, 794, 1123);
        const width = legacy.normalizePageDimension(w, 794), height = legacy.normalizePageDimension(h, 1123);
        assert.equal(geometry.width, width); assert.equal(geometry.height, height);
        const mmW = legacy.legacyPxToMm(width, 794), mmH = legacy.legacyPxToMm(height, 1123);
        assert.equal(geometry.mmWidth, mmW); assert.equal(geometry.mmHeight, mmH);
        assert.equal(geometry.landscape, mmW > mmH);
    }
});

test('pdf-lib page size and overlay placement equal the original arithmetic', () => {
    const g = makeGen(606);
    const PX_TO_PT = 0.75;
    for (let n = 0; n < 4000; n++) {
        const w = g.pick([794, 595.4, 20, 1e3, undefined]), h = g.pick([1123, 842.5, 3, undefined]);
        const lw = legacy.normalizePageDimension(w, 794), lh = legacy.normalizePageDimension(h, 1123);
        const copied = g.chance(0.5);
        const cw = copied ? 100 + g.r() * 700 : NaN, ch = copied ? 100 + g.r() * 900 : NaN;
        const geometry = docio.pdfLibGeometry(w, h, 794, 1123, cw, ch);
        const expected = copied ? { widthPt: cw, heightPt: ch, widthPx: cw / PX_TO_PT, heightPx: ch / PX_TO_PT }
            : { widthPx: lw, heightPx: lh, widthPt: lw * PX_TO_PT, heightPt: lh * PX_TO_PT };
        assert.deepEqual(plain(geometry), { logicalWidth: lw, logicalHeight: lh, ...expected });
        const bounds = { x: Math.floor(g.r() * 3e4) / 40, y: Math.floor(g.r() * 4e4) / 40, width: 1 + g.r() * 700, height: 1 + g.r() * 900 };
        const scaleX = geometry.widthPt / lw, scaleY = geometry.heightPt / lh;
        assert.deepEqual(plain(docio.overlayPlacement(bounds, geometry.widthPt, geometry.heightPt, lw, lh)),
            { x: bounds.x * scaleX, y: geometry.heightPt - ((bounds.y + bounds.height) * scaleY), width: bounds.width * scaleX, height: bounds.height * scaleY });
    }
});

test('export profile, scale/quality and copyPages plan equal the original', () => {
    const g = makeGen(707);
    for (let n = 0; n < 3000; n++) {
        const options = { highQuality: g.pick([true, false, undefined]), optimizeForUpload: g.pick([true, false, undefined]) };
        const pageCount = g.pick([1, 3, 19, 20, 21, 200]);
        const rawBackend = g.chance(0.5);
        const raster = legacy.legacyRasterPlan(options, pageCount);
        const rasterPlan = docio.exportPlan({ kind: 'raster', pageCount, highQuality: !!options.highQuality, optimizeForUpload: !!options.optimizeForUpload, canvasScale: 2 });
        assert.deepEqual(plain(rasterPlan), plain(raster));
        const backend = {}, windowBackend = rawBackend ? {} : backend;
        const originalPageCount = g.pick([0, 1, 3, 10]);
        const pages = Array.from({ length: pageCount % 23 }, () => g.pick([null, { backgroundSource: 'pdf', pdfPageIndex: g.pick([0, 1, 2, 3, 4, 10, 1.5, '2', undefined, null, NaN]) },
            { backgroundSource: 'template' }, { backgroundSource: 'pdf' }, {}]));
        const expected = legacy.legacyPdfLibPlan(options, { window: windowBackend, backend }, pages.length, { pages }, originalPageCount);
        const actual = docio.exportPlan({ kind: 'pdflib', canvasScale: 2, highQuality: !!options.highQuality, optimizeForUpload: !!options.optimizeForUpload,
            rawBackend, originalPageCount, pages: pages.map(p => p ? { s: p.backgroundSource, i: p.pdfPageIndex } : null) });
        assert.deepEqual(plain(actual), plain(expected));
    }
});

test('known boundary behaviour: invalid frames and null points fail loudly', () => {
    assert.throws(() => docio.pageExport({ mode: 'bogus' }, '[]'), /Unknown export mode/);
    assert.throws(() => docio.pageExport({ mode: 'pdflib' }, '[{"id":"a","points":[null]}]'), /null/);
    assert.throws(() => docio.pageExport({ mode: 'pdflib' }, '{"not":"an array"}'), /array/);
    assert.throws(() => docio.exportPlan({ kind: 'nope' }), /Unknown export kind/);
});

test('numbers echoed through images and side panels print exactly like JSON.stringify', () => {
    const g = makeGen(808);
    const float32 = new Float32Array(1);
    const values = [];
    for (let n = 0; n < 60000; n++) {
        const kind = n % 6;
        if (kind === 0) values.push(g.r() * 10 ** g.int(-9, 24));
        else if (kind === 1) { float32[0] = g.r() * 2000; values.push(float32[0]); }
        else if (kind === 2) values.push(g.int(0, 2 ** 30) / 2 ** g.int(1, 24));
        else if (kind === 3) values.push((g.r() - 0.5) * 2 ** g.int(-1074 + 60, 1000));
        else if (kind === 4) values.push(Math.round(g.r() * 1e9) / 10 ** g.int(0, 12));
        else values.push(g.pick([5e-324, 1.7976931348623157e308, 2 ** 53, 2 ** 53 + 2, 1e21, 1e-7, 0.000001, 123456789012345680000, 4.35, 0.1 + 0.2, -1e-7]));
    }
    for (let at = 0; at < values.length; at += 50) {
        const slice = values.slice(at, at + 50);
        const { meta } = docio.pageExport({ mode: 'raster', pageId: 'p', pageIndex: 0, templateKind: 'default', pageWidth: 794, pageHeight: 1123, renderPanels: true, sidePanel: { n: slice } }, '[]');
        assert.ok(meta.endsWith(`"sidePanel":{"n":${JSON.stringify(slice)}}}`), `numbers at ${at}`);
    }
});
