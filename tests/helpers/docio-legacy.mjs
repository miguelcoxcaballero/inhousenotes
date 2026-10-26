// Shared by tests/rust-docio-export.test.mjs and tests/bench/docio-export.bench.mjs.
// Differential helpers: the Rust export-meta unit (rust-core/src/docio) against the original JavaScript
// that lived in app-v5.js before the port. The original code is not re-typed here: it is cut out of the
// pre-port source (git show a2dea10:app-v5.js) so a change in that text fails the extraction.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import vm from 'node:vm';
import { legacySource, loadLegacy, rng } from './legacy.mjs';

const bytes = await readFile(new URL('../../wasm/inhouse_notes_core.wasm', import.meta.url));
const scope = { WebAssembly, Uint8Array, Uint16Array, Uint32Array, Float32Array, Float64Array, DataView, Map, URL, TextEncoder, TextDecoder,
    location: new URL('https://notes.test/'), fetch: async () => new Response(bytes), console, AbortSignal, AbortController, setTimeout, clearTimeout };
vm.createContext(scope);
vm.runInContext(await readFile(new URL('../../rust-core-v5.js', import.meta.url), 'utf8'), scope);
assert.equal(await scope.InhouseRust.ready, true, 'wasm/inhouse_notes_core.wasm must be built (node scripts/build-rust-core.mjs)');
export const docio = scope.InhouseRust.docio;

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
export const legacy = loadLegacy('collaboration-core-v5.js');
const legacyFunctionsCode = [
    'const A4_WIDTH = 794, A4_HEIGHT = 1123, CANVAS_SCALE = 2;',
    "const TEMPLATE_KINDS = Object.freeze({ DEFAULT: 'default', AGENDA: 'agenda', DIARY: 'diary' });",
    ...['normalizeTemplateKind', 'normalizePageDimension', 'simpleHash', 'pointFingerprint', 'strokeSyncFingerprint', 'imageSyncFingerprint',
        'computePageHash', 'getPdfOverlayContentBounds', 'cloneTimelineValue'].map(legacyFunction)
].join('\n');
vm.runInContext(legacyFunctionsCode, legacy);
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

export const legacyBlocksCode = `
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
`;
vm.runInContext(legacyBlocksCode, legacy);
/** Source text of the original functions, for benchmarks that must run in the main realm (vm globals are slow). */
export const legacyBundle = () => `${legacySource('collaboration-core-v5.js')}\n${legacyFunctionsCode}\n${legacyBlocksCode}`;

// ---------------------------------------------------------------- generators
export const jsonClone = value => JSON.parse(JSON.stringify(value));
export function makeGen(seed) {
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
    const point = (clean = false) => {
        if (clean) { const q = { x: number(), y: number() }; if (chance(0.7)) q.p = r(); else if (chance(0.3)) q.p = null; if (chance(0.1)) q.t = int(0, 99); return q; }
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
    const stroke = (idPool, clean = false) => {
        const s = {};
        if (clean || chance(0.8)) s.id = chance(0.2) ? pick(idPool) : `s${int(0, 1e6)}`;
        if (chance(0.95)) s.tool = pick(['pen', 'pen', 'pen', 'highlighter', 'eraser-stroke', 'marker', '']);
        if (chance(0.9)) s.color = pick(['#000000', '#ff0000', '#112233', 'rgba(1,2,3,.5)']);
        if (chance(0.85)) s.width = chance(0.1) ? pick([0, -3, '4', null, 0.2, 40]) : Math.round(r() * 3000) / 100;
        const n = pick([0, 1, 1, 2, 3, 5, 12, 40]);
        if (clean) s.points = Array.from({ length: n }, () => point(true));
        else if (chance(0.97)) s.points = Array.from({ length: n }, point);
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
        const clean = chance(0.5);
        for (let n = 0; n < strokeCount; n++) {
            if (strokes.length && chance(0.2)) strokes.push(jsonClone(pick(strokes)));
            else strokes.push(stroke(idPool, clean));
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

export const plain = value => value === null || value === undefined ? value : JSON.parse(JSON.stringify(value));

export function rustJob(page, i, mode, extra = {}) {
    return { mode, pageId: page.pageId, pageIndex: i, bgSource: page.backgroundSource, pdfIndex: page.pdfPageIndex, hasBgImage: !!page.backgroundImage,
        templateKind: legacy.normalizeTemplateKind(page.templateKind), pageWidth: legacy.normalizePageDimension(page.pageWidth, 794),
        pageHeight: legacy.normalizePageDimension(page.pageHeight, 1123), images: Array.isArray(page.images) ? page.images : [], deletedStrokeIds: page.deletedStrokeIds,
        deletedStrokeStamps: page.deletedStrokeStamps, sidePanel: page.sidePanel, ...extra };
}

