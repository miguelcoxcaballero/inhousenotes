// Differential test: the Rust export-meta unit (rust-core/src/docio) against the original JavaScript
// that lived in app-v5.js before the port (see tests/helpers/docio-legacy.mjs for how it is extracted).
import test from 'node:test';
import assert from 'node:assert/strict';
import vm from 'node:vm';
import { docio, legacy, makeGen, plain, rustJob } from './helpers/docio-legacy.mjs';

// ---------------------------------------------------------------- tests
const ROUNDS = Number(process.env.DOCIO_ROUNDS || 1500);

test('pdf-lib path: dedupe, compaction and metadata entry are byte-identical to the original', async () => {
    const g = makeGen(101);
    let checked = 0, strokesSeen = 0, deduped = 0, withStamps = 0, packedRounds = 0;
    for (let n = 0; n < ROUNDS; n++) {
        const page = g.page();
        const i = g.int(0, 30), renderPanels = g.chance(0.6);
        if (!page.pageId) page.pageId = 'generated-id';
        const width = legacy.normalizePageDimension(page.pageWidth, 794), height = legacy.normalizePageDimension(page.pageHeight, 1123);
        const expectation = await legacy.legacyPdfLibEntry(structuredClone(page), i, renderPanels, width, height, 1).catch(error => ({ error }));
        const strokes = Array.isArray(page.strokes) ? page.strokes : [];
        const input = rustJob(page, i, 'pdflib', { renderPanels, bounds: { width, height, scale: 2.5 } });
        let actual;
        try { actual = docio.pageExport(input, strokes, undefined, { pack: n % 2 === 0 }); } catch (error) { actual = { error }; }
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
        const reused = docio.pageExport(rustJob(page, i, 'pdflib', { renderPanels, reuse: true }), null, actual.compact);
        assert.equal(reused.meta, actual.meta);
        strokesSeen += expectation.exportStrokes.length; checked++;
        if (n % 2 === 0 && docio.packable(strokes) && strokes.some(s => s.points?.length)) packedRounds++;
        if (expectation.exportStrokes.length < strokes.length) deduped++;
        if (actual.meta.includes('"clock"')) withStamps++;
    }
    assert.ok(checked > ROUNDS * 0.9 && strokesSeen > ROUNDS);
    assert.ok(deduped > ROUNDS / 40 && withStamps > ROUNDS / 4 && packedRounds > ROUNDS / 8, `generator coverage: deduped=${deduped} stamps=${withStamps} packed=${packedRounds}`);
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
        try { actual = docio.pageExport(rustJob(page, i, 'raster', { renderPanels }), strokes, undefined, { pack: n % 2 === 0 }); } catch (error) { actual = { error }; }
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
        const actual = docio.overlayBounds(strokes, Array.isArray(page.images) ? page.images : [], width, height, scale, { pack: n % 2 === 0 });
        assert.deepEqual(plain(actual), plain(expected), `round ${n}`);
        if (expected) found++;
    }
    assert.ok(found > ROUNDS / 2, `only ${found} rounds produced bounds`);
    const rotated = Array.from({ length: 20000 }, (_, k) => ({ x: 100, y: 120, width: 90 + (k % 17), height: 40 + (k % 11), rotation: (k - 10000) * 0.00137 }));
    for (const image of rotated) {
        assert.deepEqual(plain(docio.overlayBounds([], [image], 794, 1123, 2.5)), plain(legacy.getPdfOverlayContentBounds({ images: [image] }, [], 794, 1123, 2.5)));
    }
});

test('page hash equals computePageHash', () => {
    const g = makeGen(404);
    for (let n = 0; n < ROUNDS * 2; n++) {
        const page = g.page();
        let expected, actual;
        try { expected = legacy.computePageHash(structuredClone(page)); } catch (error) { expected = { error }; }
        try { actual = docio.pageHash(page, 794, 1123, { pack: n % 2 === 0 }); } catch (error) { actual = { error }; }
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
    assert.throws(() => docio.pageExport({ mode: 'bogus' }, []), /Unknown export mode/);
    assert.throws(() => docio.pageExport({ mode: 'pdflib' }, [{ id: 'a', points: [null] }]), /null/);
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
        const { meta } = docio.pageExport({ mode: 'raster', pageId: 'p', pageIndex: 0, templateKind: 'default', pageWidth: 794, pageHeight: 1123, renderPanels: true, sidePanel: { n: slice } }, []);
        assert.ok(meta.endsWith(`"sidePanel":{"n":${JSON.stringify(slice)}}}`), `numbers at ${at}`);
    }
});
