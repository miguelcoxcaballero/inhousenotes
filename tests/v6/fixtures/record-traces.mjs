// Records the 5.18.0 canvas trace of every `canvas` scene of TESTING §3.3 (and the live-ink scenes of §3.4) into
// tests/v6/fixtures/traces/<scene>.json. W2.3's cargo trace-golden tests compare the Rust renderer's command stream
// with these files before a v6 editor exists; the parity harness compares both apps' traces live.
//
//   node tests/v6/fixtures/record-traces.mjs [--only dense-page,ink-pen] [--check] [--out dir]
//
// Scenes: dense-page, dense-page-erase (full and damage repaint), thumbnails (Pages panel), page-preview and
// page-preview-images (the preview kept for a released page), scroll-preview (the scroll-preview queue over several
// pages), template-backgrounds, ink-pen, ink-highlighter.
//
// File format (normalize.mjs `stringifyTrace`): {"scene", "app", "viewport", "dpr", "input"?, "surfaces":[{"label",
// "kind", "width", "height", "ops":[[name, ...args]]}]}, one op per line. Numbers are "~" + the 16 hex digits of their
// IEEE-754 bits; see trace-init.js for the other argument forms. Surfaces are sorted by label; `offscreen#k` are
// canvases outside the document in order of first use within the recorded window.
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { startServer } from '../server.mjs';
import { Driver, ensureV5Ref, launchBrowser } from '../parity/run.mjs';
import { normalizeTrace, stringifyTrace } from '../parity/normalize.mjs';
import { VIEWPORTS } from '../parity/matrix.mjs';
import { ANA } from '../mock/fixtures.mjs';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const args = process.argv.slice(2), flag = name => args.includes(`--${name}`), opt = name => { const i = args.indexOf(`--${name}`); return i < 0 ? '' : args[i + 1]; };
const CHECK = flag('check'), ONLY = opt('only').split(',').filter(Boolean);
const OUT = opt('out') ? path.resolve(opt('out')) : CHECK ? fs.mkdtempSync(path.join(os.tmpdir(), 'ihn-traces-')) : path.join(HERE, 'traces');
const CELL = { viewport: 'desktop-1280', theme: 'light', mode: 'web' };

let server, browser;
async function start(options = {}) {
  const t = new Driver({ browser, baseUrl: server.url, cell: options.cell || CELL, state: {} });
  await t.start();
  return t;
}
const take = t => t.page.evaluate(() => self.__ihnTrace.take({ clear: true }));
const reset = t => t.page.evaluate(() => self.__ihnTrace.reset());
const pageBox = t => t.page.evaluate(() => { const r = document.querySelector('canvas.page-canvas[data-page="0"]').getBoundingClientRect(); return { x: r.left, y: r.top, w: r.width, h: r.height }; });
/** Pen samples relative to the page's top-left corner, every 8 ms: a line of loops. */
const loops = (x, y, n = 40) => Array.from({ length: n }, (_, k) => ({ x: x + k * 5.5, y: y + Math.sin(k / 2.5) * 22 + (k % 5) * 0.75, t: k * 8, p: 0.25 + ((k * 7) % 11) / 20 }));
const onPage = (box, points) => points.map(p => ({ ...p, x: box.x + p.x, y: box.y + p.y }));
async function localEditor(t, pages = 1) {
  await t.open();
  await t.job(async count => { await __IHN_TEST_API__.resetLocalDocument(count, 'Trace scene'); await __IHN_TEST_API__.showEditorForTest(); }, pages);
  await t.advance(600);
}

const scenes = {
  // Dense page (2,400 strokes) opened from Drive: the full repaint of the page canvas, staged or not.
  async 'dense-page'(t, out) {
    await t.open({ account: ANA, scene: 'dense-page', settleMs: 8000 });
    out['dense-page'] = { trace: await take(t) };
    // Damage repaint: the stroke eraser dragged across two rows of strokes (a document opened from Drive starts in
    // Read mode: switch to Edit like a user).
    await t.click('#mode-toggle-btn');
    await t.advance(600);
    await t.click('[data-tool="eraser"]');
    await reset(t);
    const box = await pageBox(t), input = Array.from({ length: 30 }, (_, k) => ({ x: 120 + k * 9, y: 150 + k * 0.9, t: k * 8, p: 0.5 }));
    await t.pen(onPage(box, input));
    await t.advance(1500);
    out['dense-page-erase'] = { trace: await take(t), input };
  },

  // Thumbnails, part 1: the Pages panel of the template notebook (page previews drawn into their own canvases).
  // Parts 2 and 3 are the page-preview and scroll-preview scenes below.
  async thumbnails(t, out) {
    await t.open({ account: ANA, scene: 'template', settleMs: 6000 });
    await t.click('#mode-toggle-btn');
    await t.advance(600);
    await reset(t);
    await t.click('#btn-edit-pages');
    await t.advance(2500);
    out.thumbnails = { trace: await take(t) };
  },

  // Page previews (`renderPagePreview`: PAGE_PREVIEW_SCALE canvas, JPEG 0.6), the picture a page keeps when its
  // canvas is released: five pages of the 20-page notebook, then (second file) pages with inline images.
  async 'page-preview'(t, out) { out['page-preview'] = await pagePreviews(t, 'dense-20', [0, 1, 2, 9, 19]); },
  async 'page-preview-images'(t, out) { out['page-preview-images'] = await pagePreviews(t, 'inline-images', null); },

  // Scroll previews (`processScrollPagePreview`: min(.25, 320/w, 440/h) canvas, JPEG 0.65): the queue working through
  // the pages of the 20-page notebook that come into view while the user scrolls down with the wheel.
  async 'scroll-preview'(t, out) {
    await t.open({ account: ANA, scene: 'dense-20', settleMs: 8000 });
    await reset(t);
    await t.page.mouse.move(640, 500);
    // One wheel notch moves the document 100 px: twelve of them bring the next page into view, then the user rests
    // (the queue waits until the viewport is still) — four times, and each page that came into view gets its preview.
    const input = [];
    for (let block = 0; block < 4; block++) {
      for (let notch = 0; notch < 12; notch++) { await t.wheel(0, 900); await t.advance(60); input.push({ wheel: [0, 900], thenMs: notch < 11 ? 60 : 2560 }); }
      await t.advance(2500);
    }
    const made = await t.page.evaluate(() => state.pages.filter(page => page.preview).length);
    if (made < 6) throw new Error(`scroll preview: only ${made} pages have a preview`);
    out['scroll-preview'] = { trace: await take(t), input };
  },

  // Template backgrounds at every backing scale the editor uses (2, 2.25, …, 6), and the agenda and diary templates.
  async 'template-backgrounds'(t, out) {
    await t.open();
    await reset(t);
    const sizes = await t.job(async () => {
      const done = [];
      const size = (width, height, scale) => { const w = Math.max(1, Math.round(width * scale)); return [w, Math.max(1, Math.round(height * (w / width)))]; };
      for (let scale = 2; scale <= 6; scale += 0.25) { const [w, h] = size(A4_WIDTH, A4_HEIGHT, scale); _renderTemplateToCanvas(w, h); done.push({ kind: 'default', scale, w, h }); }
      for (const kind of ['agenda', 'diary']) {
        const image = _getSvgImage(kind);
        if (!image.complete) await new Promise(resolve => image.addEventListener('load', resolve, { once: true }));
        for (const scale of [2, 3]) { const [w, h] = size(559, 794, scale); _renderSvgTemplateToCanvas(kind, w, h); done.push({ kind, scale, w, h }); }
      }
      return done;
    });
    out['template-backgrounds'] = { trace: await take(t), input: sizes };
  },

  // Live ink (TESTING §3.4): the page, cursor and offscreen surfaces while a pen stroke is drawn, at lift, and after.
  async 'ink-pen'(t, out) { await liveInk(t, out, 'ink-pen', 'pen'); },
  async 'ink-highlighter'(t, out) { await liveInk(t, out, 'ink-highlighter', 'highlighter'); }
};

/** `renderPagePreview` of the given pages (null: all) of a Drive fixture, each on fully loaded page data. */
async function pagePreviews(t, scene, pages) {
  await t.open({ account: ANA, scene, settleMs: 8000 });
  await t.advance(4000);
  await reset(t);
  const input = await t.job(async wanted => {
    const done = [];
    for (const index of wanted || state.pages.map((_, i) => i)) {
      if (!isPageLoaded(state.pages[index])) await ensurePageDataLoaded(index, { keepLoaded: true });
      const page = state.pages[index], sources = [page.backgroundImage, ...(page.images || []).map(image => image.src)].filter(Boolean);
      await Promise.all(sources.map(async src => { const image = getCachedImage(src); if (!image.complete || !image.naturalWidth) await image.decode(); }));
      const url = renderPagePreview(index);
      done.push({ page: index, strokes: page.strokes.length, images: (page.images || []).length, jpeg: !!url && url.startsWith('data:image/jpeg') });
    }
    return done;
  }, pages);
  if (!input.length || input.some(row => !row.jpeg)) throw new Error(`page preview not rendered: ${JSON.stringify(input)}`);
  return { trace: await take(t), input };
}

async function liveInk(t, out, name, tool) {
  await localEditor(t);
  await t.click(`[data-tool="${tool}"]`);
  await reset(t);
  const box = await pageBox(t), input = loops(110, 180);
  await t.pen(onPage(box, input));
  await t.advance(1200);
  out[name] = { trace: await take(t), input };
}

async function main() {
  server = await startServer({ root: ensureV5Ref() });
  fs.mkdirSync(OUT, { recursive: true });
  try {
    for (const [name, record] of Object.entries(scenes)) {
      if (ONLY.length && !ONLY.includes(name) && !(name === 'dense-page' && ONLY.includes('dense-page-erase'))) continue;
      browser = await launchBrowser();
      const t = await start(), out = {};
      try { await record(t, out); } finally { await t.stop(); await browser.close(); }
      for (const [scene, { trace, input }] of Object.entries(out)) {
        const normalized = normalizeTrace(trace), v = VIEWPORTS[CELL.viewport];
        const text = stringifyTrace(normalized, { scene, app: '5.18.0', viewport: `${v.width}x${v.height}`, dpr: v.dpr, ...(input ? { input } : {}) });
        fs.writeFileSync(path.join(OUT, `${scene}.json`), text);
        console.log(`${scene}.json ${(text.length / 1024).toFixed(0)} KiB — ${normalized.surfaces.map(s => `${s.label}:${s.ops.length}`).join(' ')}${t.problems.length ? ' problems: ' + t.problems.join('; ') : ''}`);
      }
    }
  } finally { await server.close(); }
  if (CHECK) {
    const committed = path.join(HERE, 'traces'), bad = fs.readdirSync(OUT).filter(f => !fs.existsSync(path.join(committed, f)) || !fs.readFileSync(path.join(committed, f)).equals(fs.readFileSync(path.join(OUT, f))));
    if (!opt('out')) fs.rmSync(OUT, { recursive: true, force: true });
    console.log(bad.length ? `DIFFERENT: ${bad.join(', ')}` : 'Traces regenerate byte-identically.');
    process.exit(bad.length ? 1 : 0);
  }
}
main().catch(error => { console.error(error); process.exit(2); });
