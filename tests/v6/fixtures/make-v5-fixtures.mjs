// Generates the 5.x fixtures of TESTING §5 by driving 5.18.0 itself (the tree of commit 1d6e8a0), under the parity
// harness' determinism (fixed clock, seeded randomness, also in workers), so a second run gives the same bytes.
//
//   node tests/v6/fixtures/make-v5-fixtures.mjs [--only template,r0] [--skip-storage] [--skip-pdf] [--check]
//
// `--check` regenerates into a temporary directory and compares with the committed files (exit 1 on any difference).
//
// Drive documents  → tests/v6/fixtures/v5/<name>.pdf + <name>.expect.json (what 5.18.0 reports after importing it)
// Storage dumps    → tests/v6/fixtures/v5/storage/<name>.dump.json + <name>.expect.json
//
// Every document is produced by 5.18.0's own exporter (`buildPdfBlob`). Three are then rewritten with 5.18.0's own
// codecs because no 5.18.0 code path writes them any more (older 5.x releases did): `legacy-no-norm` (ids removed or
// duplicated, no IH_NORM / IH_STRUCT / IH_FIELDS / IH_SYNC), `strokes-data` (STROKES_DATA instead of STROKES_Z) and
// `plain` (made with the vendored pdf-lib: not an Inhouse document at all).
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { startServer } from '../server.mjs';
import { Driver, ensureV5Ref, launchBrowser } from '../parity/run.mjs';
import { dumpStorage, ANA, addAccounts } from '../mock/fixtures.mjs';
import { FOLDER } from '../mock/drive.mjs';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const args = process.argv.slice(2), flag = name => args.includes(`--${name}`), opt = name => { const i = args.indexOf(`--${name}`); return i < 0 ? '' : args[i + 1]; };
const CHECK = flag('check'), ONLY = opt('only').split(',').filter(Boolean);
const OUT = CHECK ? fs.mkdtempSync(path.join(os.tmpdir(), 'ihn-v5-fixtures-')) : path.join(HERE, 'v5');
const CELL = { viewport: 'desktop-1280', theme: 'light', mode: 'web' };
const seedOf = name => { let h = 2166136261; for (const c of name) h = Math.imul(h ^ c.charCodeAt(0), 16777619); return (h >>> 0) || 1; };

// ---- page-side helpers (5.18.0 internals; this file is the only place that touches them) ----
const LIB = () => {
  const b64 = bytes => { let s = ''; for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode.apply(null, bytes.subarray(i, i + 0x8000)); return btoa(s); };
  const unb64 = text => { const s = atob(text), out = new Uint8Array(s.length); for (let i = 0; i < s.length; i++) out[i] = s.charCodeAt(i); return out; };
  const sha = async data => Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256', typeof data === 'string' ? new TextEncoder().encode(data) : data)), b => b.toString(16).padStart(2, '0')).join('');
  self.mk = {
    b64, unb64, sha,
    /** A handwriting-like stroke: `n` points starting at (x, y). */
    stroke(id, x, y, n = 24, { tool = 'pen', color = '#1a1a1a', width = 2, dx = 3, amp = 9 } = {}) {
      return { id, tool, color, width, points: Array.from({ length: n }, (_, k) => ({ x: x + k * dx, y: y + Math.sin(k / 3) * amp, p: 0.35 + (k % 7) * 0.05 })) };
    },
    add(pageIndex, strokes) { const page = state.pages[pageIndex]; page.strokes.push(...strokes); page.strokeCount = getVisibleStrokeCount(page); markPageDirty(pageIndex, 'full'); },
    async pdf() { return b64(new Uint8Array(await (await buildPdfBlob({ status: false })).arrayBuffer())); },
    async importPdf(base64) { await __IHN_TEST_API__.ready(); await __IHN_TEST_API__.importPdfBlobForTest(Array.from(unb64(base64))); },
    /** The PDF `/Keywords` of an exported document, read with the vendored pdf-lib. */
    async keywords(base64) { const doc = await PDFLib.PDFDocument.load(unb64(base64), { updateMetadata: false }); return doc.getKeywords() || ''; },
    async rewriteKeywords(base64, change) {
      const doc = await PDFLib.PDFDocument.load(unb64(base64), { updateMetadata: false });
      doc.setKeywords([await change(doc.getKeywords() || '')]);
      return b64(await doc.save({ useObjectStreams: false }));
    },
    /** What 5.18.0 holds after importing a document: the record v6's importer is compared with. */
    async summary() {
      await ensureVersionHistoryRestored();
      const pages = [];
      for (let i = 0; i < state.pages.length; i++) {
        if (!isPageLoaded(state.pages[i])) await ensurePageDataLoaded(i, { keepLoaded: true });
        const p = state.pages[i];
        const strokeRow = s => [s.id ?? null, s.tool ?? null, s.color ?? null, s.width ?? null, Array.isArray(s.points) ? s.points.length : 0];
        pages.push({
          pageId: p.pageId, pageWidth: p.pageWidth, pageHeight: p.pageHeight, templateKind: p.templateKind, backgroundSource: p.backgroundSource,
          pdfPageIndex: p.pdfPageIndex ?? null, backgroundImage: p.backgroundImage ? await sha(p.backgroundImage) : null, sidePanel: p.sidePanel || null,
          strokes: (p.strokes || []).map(strokeRow),
          pointsSha256: await sha(JSON.stringify((p.strokes || []).map(s => (s.points || []).map(q => [q.x, q.y, q.p ?? null])))),
          images: await Promise.all((p.images || []).map(async im => ({ id: im.id ?? null, x: im.x, y: im.y, width: im.width, height: im.height, rotation: im.rotation ?? 0, srcSha256: await sha(String(im.src || '')) }))),
          deletedStrokeIds: [...(p.deletedStrokeIds || [])], legacyCoverStrokes: (p.legacyCoverStrokes || []).map(strokeRow)
        });
      }
      return {
        app: '5.18.0', exportName: state.exportName || '', pageCount: state.pages.length,
        strokeCount: pages.reduce((n, p) => n + p.strokes.length, 0), imageCount: pages.reduce((n, p) => n + p.images.length, 0),
        hasLegacyBakedOverlay: !!state.hasLegacyBakedOverlay, cleanOriginalBytes: state.cleanOriginalPdfBytes ? state.cleanOriginalPdfBytes.byteLength : 0,
        versions: (state.versionHistory || []).map(v => ({ id: v.id, ts: v.ts ?? v.timestamp ?? null, milestone: !!v.milestone, pages: Array.isArray(v.pages) ? v.pages.length : null })),
        pages
      };
    }
  };
};

// ---- sessions ----
let server, browser;
async function session(name, { drive, account = null, scene, sceneOptions, settleMs } = {}) {
  const t = new Driver({ browser, baseUrl: server.url, cell: CELL, state: { drive }, seed: seedOf(name), trace: false });
  await t.start();
  await t.open({ account, scene, sceneOptions, settleMs });
  await t.page.evaluate(`(${LIB})()`);
  return t;
}
/** Local editor with `pages` blank template pages, shown and ready for input. */
async function editor(t, pages, title) {
  await t.job(async ([count, name]) => { await __IHN_TEST_API__.resetLocalDocument(count, name); await __IHN_TEST_API__.showEditorForTest(); }, [pages, title]);
  await t.advance(400);
}
const pageBox = (t, index = 0) => t.page.evaluate(i => { const r = document.querySelector(`canvas.page-canvas[data-page="${i}"]`).getBoundingClientRect(); return { x: r.left, y: r.top, w: r.width, h: r.height }; }, index);
/** Handwriting through real pen input: `rows` lines of loops inside the visible part of the page. */
async function write(t, { rows = 3, perRow = 4, top = 0.12, tool = 'pen' } = {}) {
  await t.click(`#toolbar [data-tool="${tool}"], [data-tool="${tool}"]`);
  const box = await pageBox(t);
  for (let r = 0; r < rows; r++) for (let c = 0; c < perRow; c++) {
    const x0 = box.x + box.w * (0.1 + c * 0.2), y0 = box.y + box.h * top + r * 46;
    await t.pen(Array.from({ length: 18 }, (_, k) => ({ x: x0 + k * 4.5, y: y0 + Math.sin(k / 2.2 + c) * 14, t: k * 8, p: 0.3 + (k % 6) * 0.08 })));
    await t.advance(120);
  }
  await t.advance(800);
}
const pdfOf = t => t.job(() => mk.pdf()).then(text => Buffer.from(text, 'base64'));

// ---- Drive documents ----
const documents = {
  // Not an Inhouse document: two pages, the second rotated, vector content and text.
  async plain(t) {
    return Buffer.from(await t.job(async () => {
      const { PDFDocument, StandardFonts, rgb, degrees } = PDFLib;
      const doc = await PDFDocument.create();
      doc.setTitle('Plain fixture'); doc.setProducer('fixture'); doc.setCreator('fixture');
      const font = await doc.embedFont(StandardFonts.Helvetica);
      const a = doc.addPage([595.28, 841.89]);
      a.drawText('Plain PDF, page 1', { x: 60, y: 760, size: 24, font, color: rgb(0.1, 0.1, 0.1) });
      a.drawRectangle({ x: 60, y: 520, width: 300, height: 160, borderColor: rgb(0.2, 0.4, 0.8), borderWidth: 2 });
      a.drawLine({ start: { x: 60, y: 480 }, end: { x: 520, y: 430 }, thickness: 1.5, color: rgb(0.8, 0.2, 0.2) });
      const b = doc.addPage([419.53, 595.28]);
      b.setRotation(degrees(90));
      b.drawText('Page 2, rotated 90', { x: 40, y: 520, size: 18, font, color: rgb(0, 0.3, 0) });
      b.drawEllipse({ x: 200, y: 300, xScale: 90, yScale: 50, color: rgb(0.95, 0.85, 0.3) });
      return mk.b64(await doc.save({ useObjectStreams: false }));
    }), 'base64');
  },

  // A template notebook written with real pen input: pen, highlighter, and a stroke removed with the eraser.
  async template(t) {
    await editor(t, 3, 'Template notebook');
    await write(t, { rows: 3, perRow: 4 });
    await write(t, { rows: 1, perRow: 3, top: 0.42, tool: 'highlighter' });
    await t.job(async () => { mk.add(1, [mk.stroke('tpl-p2-a', 80, 140, 40), mk.stroke('tpl-p2-b', 80, 220, 40, { color: '#c62828', width: 3 }), mk.stroke('tpl-p2-hl', 80, 300, 30, { tool: 'highlighter', color: '#ffeb3b', width: 14 })]); });
    return pdfOf(t);
  },

  // Two revisions of one document. R1 loses two strokes and gets an A5 second page, both without deletion or field
  // stamps (as a 5.x device that never synchronised would write them).
  async r0(t, share) {
    await editor(t, 2, 'Revisions');
    await t.job(async () => {
      mk.add(0, Array.from({ length: 6 }, (_, i) => mk.stroke(`rev-s${i + 1}`, 70, 100 + i * 60, 30)));
      mk.add(1, Array.from({ length: 3 }, (_, i) => mk.stroke(`rev-s${i + 7}`, 70, 100 + i * 60, 30, { color: '#1565c0' })));
    });
    const r0 = await pdfOf(t);
    await t.job(async () => {
      const page = state.pages[0];
      page.strokes = page.strokes.filter(s => s.id !== 'rev-s2' && s.id !== 'rev-s5'); page.strokeCount = page.strokes.length;
      state.pages[1].pageWidth = 559; state.pages[1].pageHeight = 794;
      mk.add(0, [mk.stroke('rev-s10', 70, 520, 30, { color: '#2e7d32' })]);
      markPageDirty(1, 'full');
    });
    await t.advance(1500);
    share.r1 = await pdfOf(t);
    return r0;
  },
  async r1(t, share) { return share.r1; },

  // An imported PDF annotated in place: pdf-lib path, original pages kept, one overlay per page (IH_OVERLAY_REFS).
  async 'imported-overlays'(t, share) {
    await t.job(base64 => mk.importPdf(base64), share.plain.toString('base64'));
    await t.advance(1500);
    await t.job(async () => {
      mk.add(0, [mk.stroke('imp-a', 70, 200, 36), mk.stroke('imp-hl', 70, 90, 40, { tool: 'highlighter', color: '#ffeb3b', width: 14 })]);
      mk.add(1, [mk.stroke('imp-b', 60, 160, 28, { color: '#6a1b9a', width: 2.5 })]);
    });
    return pdfOf(t);
  },

  // The same import after its structure changed (a template page inserted, PDF pages swapped): the clean original is
  // attached as IH_CLEAN_ORIGINAL.pdf.
  async 'structure-changed'(t, share) {
    await t.job(base64 => mk.importPdf(base64), share.plain.toString('base64'));
    await t.advance(1500);
    await t.job(async () => {
      for (let i = 0; i < state.pages.length; i++) if (!isPageLoaded(state.pages[i])) await ensurePageDataLoaded(i, { keepLoaded: true });
      const [first, second] = state.pages;
      state.pages = [second, createBlankPageData(), first];
      bumpPageStructureVersion();
      state.pages.forEach((_, i) => markPageDirty(i, 'full'));
      mk.add(1, [mk.stroke('str-new', 80, 150, 32)]);
      mk.add(2, [mk.stroke('str-old', 80, 300, 32, { color: '#ad1457' })]);
    });
    await t.advance(1500);
    return pdfOf(t);
  },

  // A photo page (custom background image) between template pages, with ink on it.
  async 'photo-page'(t) {
    await editor(t, 1, 'Photo page');
    await t.job(async () => { await __IHN_TEST_API__.addPhotoPageForTest(); });
    await t.advance(800);
    await t.job(async () => { mk.add(1, [mk.stroke('photo-ink', 120, 200, 30, { color: '#ffffff', width: 3 })]); mk.add(0, [mk.stroke('photo-first', 80, 120, 20)]); });
    return pdfOf(t);
  },

  // Images placed on a page (two different pictures, one of them twice).
  async 'inline-images'(t) {
    await editor(t, 2, 'Inline images');
    await t.job(async () => {
      const picture = (w, h, draw) => { const c = document.createElement('canvas'); c.width = w; c.height = h; const x = c.getContext('2d'); draw(x); return c.toDataURL('image/png'); };
      const a = picture(64, 48, x => { x.fillStyle = '#1a73e8'; x.fillRect(0, 0, 64, 48); x.fillStyle = '#fff'; x.fillRect(8, 8, 20, 12); });
      const b = picture(40, 40, x => { x.fillStyle = '#e53935'; x.beginPath(); x.arc(20, 20, 18, 0, Math.PI * 2); x.fill(); });
      addImageToPage(0, a, { x: 120, y: 160 }); addImageToPage(0, b, { x: 420, y: 300 }, 1); addImageToPage(1, a, { x: 200, y: 500 });
      mk.add(0, [mk.stroke('img-ink', 100, 420, 30)]);
    });
    await t.advance(1500);
    return pdfOf(t);
  },

  // Existing agenda and diary pages (A5 templates) with their side-panel data; 5.18.0 keeps and re-exports them.
  async 'agenda-diary'(t) {
    await editor(t, 1, 'Agenda 2026-2027');
    await t.job(async () => {
      const agenda = createBlankPageData({ templateKind: 'agenda' }), diary = createBlankPageData({ templateKind: 'diary' });
      agenda.sidePanel = { title: 'Week 44', dateKeys: ['2026-10-26', '2026-10-27', '2026-10-28'] };
      diary.sidePanel = { title: '30 October 2026', dateKeys: ['2026-10-30'] };
      state.pages = [agenda, diary, createBlankPageData()];
      bumpPageStructureVersion();
      state.pages.forEach((_, i) => markPageDirty(i, 'full'));
      mk.add(0, [mk.stroke('ag-a', 60, 120, 24), mk.stroke('ag-b', 60, 200, 24, { color: '#0277bd' })]);
      mk.add(1, [mk.stroke('di-a', 60, 140, 30)]);
      renderAllPages();
    });
    await t.advance(1000);
    return pdfOf(t);
  },

  // A document as releases before IH_NORM wrote it: strokes without ids or with repeated ids, pages without ids, and
  // only the STROKES_Z section. Rewritten from a 5.18.0 export with 5.18.0's own codec.
  async 'legacy-no-norm'(t) {
    await editor(t, 2, 'Legacy notebook');
    await t.job(async () => {
      mk.add(0, Array.from({ length: 5 }, (_, i) => mk.stroke(`leg-${i}`, 70, 110 + i * 55, 22)));
      mk.add(1, Array.from({ length: 4 }, (_, i) => mk.stroke(`leg-${i + 5}`, 70, 110 + i * 55, 22, { color: '#4e342e' })));
    });
    const modern = (await pdfOf(t)).toString('base64');
    return Buffer.from(await t.job(base64 => mk.rewriteKeywords(base64, async keywords => {
      const packed = /(?:^|;)STROKES_Z:([^;]*)/.exec(keywords)[1];
      const json = await new Response(new Blob([mk.unb64(packed)]).stream().pipeThrough(new DecompressionStream('deflate'))).text();
      const pages = JSON.parse(json);
      pages.forEach((page, p) => {
        delete page.pageId;
        (page.strokes || []).forEach((stroke, i) => { if (i === 1) delete stroke.id; else if (i === 3) stroke.id = page.strokes[2].id; else if (p === 1 && i === 0) stroke.id = pages[0].strokes[0].id; });
      });
      const bytes = new Uint8Array(await new Response(new Blob([JSON.stringify(pages)]).stream().pipeThrough(new CompressionStream('deflate'))).arrayBuffer());
      return `STROKES_Z:${mk.b64(bytes)}`;
    }), modern), 'base64');
  },

  // The uncompressed variant a 5.x writer falls back to when compression throws: STROKES_DATA (base64 JSON).
  async 'strokes-data'(t) {
    await editor(t, 1, 'Strokes data');
    await t.job(async () => { mk.add(0, Array.from({ length: 4 }, (_, i) => mk.stroke(`sd-${i}`, 70, 110 + i * 60, 26, { color: i % 2 ? '#283593' : '#1a1a1a' }))); });
    const modern = (await pdfOf(t)).toString('base64');
    return Buffer.from(await t.job(base64 => mk.rewriteKeywords(base64, async keywords => {
      const packed = /(?:^|;)STROKES_Z:([^;]*)/.exec(keywords)[1];
      const json = await new Response(new Blob([mk.unb64(packed)]).stream().pipeThrough(new DecompressionStream('deflate'))).text();
      return keywords.replace(`STROKES_Z:${packed}`, `STROKES_DATA:${btoa(unescape(encodeURIComponent(json)))}`);
    }), modern), 'base64');
  },

  // Raster fallback: pdf-lib unavailable while saving an annotated import, so jsPDF flattens the pages and the ink
  // is kept as white-backed cover strokes.
  async 'raster-cover'(t, share) {
    await t.job(base64 => mk.importPdf(base64), share.plain.toString('base64'));
    await t.advance(1500);
    await t.job(async () => { mk.add(0, [mk.stroke('ras-a', 80, 240, 34), mk.stroke('ras-b', 80, 320, 34, { color: '#00695c' })]); });
    return Buffer.from(await t.job(async () => { const lib = window.PDFLib; window.PDFLib = undefined; try { return await mk.pdf(); } finally { window.PDFLib = lib; } }), 'base64');
  },

  // Thirty timeline entries, six minutes apart, every third a milestone.
  async 'versions-30'(t) {
    await editor(t, 1, 'Timeline');
    await t.job(async () => {
      const start = Date.now() - 31 * 6 * 60_000;
      for (let i = 0; i < 30; i++) {
        mk.add(0, [mk.stroke(`ver-${i}`, 60 + (i % 5) * 120, 100 + Math.floor(i / 5) * 90, 14)]);
        captureVersionSnapshot(state.pages, { ts: start + i * 6 * 60_000, hint: i % 3 === 0 ? { milestone: true, label: `Milestone ${i / 3 + 1}` } : null });
      }
    });
    return pdfOf(t);
  },

  // The profile document: 20 pages, 120 strokes of 40 points each (tests/v6/perf/profile.mjs).
  async 'dense-20'(t) {
    await t.job(async () => {
      await __IHN_TEST_API__.resetLocalDocument(12, 'Profile document');
      while (state.pages.length < 20) state.pages.push(createBlankPageData());
      state.pages.forEach((page, i) => {
        for (let s = 0; s < 120; s++) page.strokes.push({ id: `p${i}-s${s}`, tool: 'pen', color: '#111111', width: 2,
          points: Array.from({ length: 40 }, (_, k) => ({ x: 60 + s % 10 * 60 + k * 1.2, y: 80 + Math.floor(s / 10) * 80 + Math.sin(k / 4) * 9, p: 0.5 })) });
        page.strokeCount = page.strokes.length; markPageDirty(i, 'full');
      });
    });
    return pdfOf(t);
  },

  // One page with 2,400 strokes (the `canvas` dense-page scene): 30 columns × 80 rows, 12 points each.
  async 'dense-page'(t) {
    await t.job(async () => {
      await __IHN_TEST_API__.resetLocalDocument(1, 'Dense handwriting');
      const page = state.pages[0];
      page.strokes = Array.from({ length: 2400 }, (_, index) => ({ id: `dense-${index}`, tool: index % 97 === 0 ? 'highlighter' : 'pen', color: index % 97 === 0 ? '#ffeb3b' : ['#123456', '#1a1a1a', '#b71c1c'][index % 3], width: index % 97 === 0 ? 12 : 1.5,
        points: Array.from({ length: 12 }, (_, p) => ({ x: 30 + (index % 30) * 24 + p * 1.6, y: 30 + Math.floor(index / 30) * 13 + Math.sin(p * 0.9 + index) * 4, p: 0.5 })) }));
      page.strokeCount = page.strokes.length; markPageDirty(0, 'full');
    });
    return pdfOf(t);
  }
};

async function makeDocuments() {
  const share = {}, dir = OUT;
  fs.mkdirSync(dir, { recursive: true });
  for (const [name, build] of Object.entries(documents)) {
    const wanted = !ONLY.length || ONLY.includes(name);
    const needed = wanted || (name === 'plain' && ONLY.some(n => ['imported-overlays', 'structure-changed', 'raster-cover'].includes(n))) || (name === 'r0' && ONLY.includes('r1'));
    if (!needed) continue;
    const t = await session(name);
    let pdf;
    try { pdf = await build(t, share); } finally { await t.stop(); }
    share[name] = pdf;
    if (!wanted) continue;
    fs.writeFileSync(path.join(dir, `${name}.pdf`), pdf);
    // The import record comes from a fresh 5.18.0 session that only knows the bytes.
    const reader = await session(`${name}:import`);
    try {
      await reader.job(base64 => mk.importPdf(base64), pdf.toString('base64'));
      await reader.advance(2000);
      const expect = await reader.job(() => mk.summary());
      expect.keywordSections = (await reader.job(base64 => mk.keywords(base64), pdf.toString('base64'))).split(';').map(part => part.split(':')[0]).filter(Boolean);
      expect.pdfBytes = pdf.length;
      fs.writeFileSync(path.join(dir, `${name}.expect.json`), JSON.stringify(expect, null, 1) + '\n');
      console.log(`${name}.pdf ${pdf.length} bytes — ${expect.pageCount} pages, ${expect.strokeCount} strokes, ${expect.imageCount} images, ${expect.versions.length} versions [${expect.keywordSections.join(' ')}]${reader.problems.length ? ' problems: ' + reader.problems.join('; ') : ''}`);
    } finally { await reader.stop(); }
  }
}

// ---- storage dumps ----
/*STORAGE*/
async function makeStorage() {}

async function main() {
  server = await startServer({ root: ensureV5Ref() });
  browser = await launchBrowser();
  try {
    if (!flag('skip-pdf')) await makeDocuments();
    if (!flag('skip-storage')) await makeStorage({ session, editor, write, pdfOf, out: path.join(OUT, 'storage'), only: ONLY, v5: path.join(HERE, 'v5') });
  } finally { await browser.close(); await server.close(); }
  if (CHECK) {
    const committed = path.join(HERE, 'v5'), bad = [];
    const walk = dir => fs.readdirSync(dir, { withFileTypes: true }).flatMap(e => e.isDirectory() ? walk(path.join(dir, e.name)) : [path.join(dir, e.name)]);
    for (const file of walk(OUT)) { const rel = path.relative(OUT, file), other = path.join(committed, rel); if (!fs.existsSync(other) || !fs.readFileSync(other).equals(fs.readFileSync(file))) bad.push(rel); }
    fs.rmSync(OUT, { recursive: true, force: true });
    console.log(bad.length ? `DIFFERENT: ${bad.join(', ')}` : 'Fixtures regenerate byte-identically.');
    process.exit(bad.length ? 1 : 0);
  }
}
main().catch(error => { console.error(error); process.exit(2); });
