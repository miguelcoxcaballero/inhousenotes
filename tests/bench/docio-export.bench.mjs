// Micro-benchmark: original JavaScript vs the Rust core for the PDF export bookkeeping on realistic pages.
// Run: node tests/bench/docio-export.bench.mjs   (needs wasm/inhouse_notes_core.wasm: node scripts/build-rust-core.mjs)
// Both implementations run in the main realm (code evaluated in a vm context pays a global-lookup penalty).
import fs from 'node:fs';
import vm from 'node:vm';
import { legacyBundle } from '../helpers/docio-legacy.mjs';
import { rng } from '../helpers/legacy.mjs';

const wasm = fs.readFileSync(new URL('../../wasm/inhouse_notes_core.wasm', import.meta.url));
globalThis.location = new URL('https://notes.test/');
globalThis.fetch = async () => new Response(wasm);
vm.runInThisContext(fs.readFileSync(new URL('../../rust-core-v5.js', import.meta.url), 'utf8'));
if (!(await globalThis.InhouseRust.ready)) throw new Error('wasm did not load');
vm.runInThisContext(legacyBundle());
const docio = globalThis.InhouseRust.docio;
const legacy = globalThis;

const r = rng(42);
// Pointer coordinates are full-precision doubles, pressure a fraction; ids are unique per stroke.
function handwrittenPage(strokes, pointsPerStroke) {
    const page = { strokes: [], images: [], pageId: 'bench', backgroundSource: 'template', pageWidth: 794, pageHeight: 1123, deletedStrokeIds: [], deletedStrokeStamps: {} };
    for (let s = 0; s < strokes; s++) {
        let x = r() * 700, y = r() * 1000;
        page.strokes.push({ id: `stroke-${s}-${Math.floor(r() * 1e9)}`, tool: 'pen', color: '#1b1b1b', width: 2.4, points:
            Array.from({ length: pointsPerStroke }, () => { x += (r() - 0.5) * 6; y += (r() - 0.5) * 6; return { x: x + 0.123456789, y: y + 0.987654321, p: r() }; }) });
    }
    return page;
}
const job = page => ({ mode: 'pdflib', pageId: page.pageId, pageIndex: 0, bgSource: page.backgroundSource, hasBgImage: false, templateKind: 'default',
    pageWidth: 794, pageHeight: 1123, images: page.images, deletedStrokeIds: page.deletedStrokeIds, deletedStrokeStamps: page.deletedStrokeStamps,
    sidePanel: null, renderPanels: true, bounds: { width: 794, height: 1123, scale: 2.5 } });
const nowMs = () => Number(process.hrtime.bigint()) / 1e6;
async function time(fn, rounds) {
    for (let i = 0; i < Math.max(3, rounds / 4 | 0); i++) await fn();
    const times = [];
    for (let i = 0; i < rounds; i++) { const t = nowMs(); await fn(); times.push(nowMs() - t); }
    times.sort((a, b) => a - b);
    return times[times.length >> 1]; // median: robust against other processes on a shared machine
}
console.log('| page | step | JS ms | Rust ms | speedup |\n|---|---|---|---|---|');
for (const [name, strokes, points, rounds] of [['light 40 strokes x 60 pts', 40, 60, 60], ['dense 300 x 80', 300, 80, 25], ['huge 1200 x 120', 1200, 120, 9]]) {
    const page = handwrittenPage(strokes, points);
    const jsEntry = (await time(() => legacy.legacyPdfLibEntry(page, 0, true, 794, 1123, 1), rounds))
        + (await time(() => legacy.getPdfOverlayContentBounds(page, page.strokes, 794, 1123, 2.5), rounds * 3));
    const rustEntry = await time(() => docio.pageExport(job(page), page.strokes), rounds * 3);
    const jsHash = await time(() => legacy.computePageHash(page), rounds * 3);
    const rustHash = await time(() => docio.pageHash(page, 794, 1123), rounds * 3);
    const row = (step, a, b) => console.log(`| ${name} | ${step} | ${a.toFixed(3)} | ${b.toFixed(3)} | ${(a / b).toFixed(1)}x |`);
    row('changed page: dedupe + compaction + entry + bounds', jsEntry, rustEntry);
    row('every page: page hash (cache key)', jsHash, rustHash);
    row('both', jsEntry + jsHash, rustEntry + rustHash);
}
