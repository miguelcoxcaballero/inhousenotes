// Micro-benchmark: original JavaScript eraser hit loop vs the Rust resident stroke store.
// usage: node tests/bench-editor-input.mjs
import {readFile} from 'node:fs/promises';
import {legacySource, rng} from './helpers/legacy.mjs';
const bytes = await readFile(new URL('../wasm/inhouse_notes_core.wasm', import.meta.url));
// Run both implementations in the main realm: vm contexts slow every global lookup and would distort the comparison.
globalThis.location = new URL('https://notes.test/');
globalThis.fetch = async () => new Response(bytes);
(0, eval)(await readFile(new URL('../rust-core-v5.js', import.meta.url), 'utf8'));
await globalThis.InhouseRust.ready;
const rust = globalThis.InhouseRust;
const source = legacySource('app-v5.js');
const a = source.indexOf('function getStrokeBounds(stroke) {'), b = source.indexOf('function eraseAreaAtPoint(', a);
const legacy = (0, eval)(`(() => { const strokeBoundsCache = new WeakMap(); const window = { InhouseRust: null };
    const getDocumentSessionToken = () => 1, pageMutationGeneration = 1;
    ${source.slice(a, b)}
    return (strokes, x, y, r) => { const out = []; for (let i = strokes.length - 1; i >= 0; i--) {
        const stroke = strokes[i]; if (stroke.tool && stroke.tool.startsWith('eraser')) continue;
        if (strokeHitsPoint(stroke, x, y, r)) out.push(i); } return out; }; })()`);

function page(count, length, random) {
    return Array.from({length: count}, (_, i) => {
        const cx = 30 + (i % 30) * 22, cy = 30 + Math.floor(i / 30) * 10;
        return {id: `s${i}`, tool: 'pen', width: 1.5, points: Array.from({length}, (_, p) => ({x: cx + p * .12 + random() * .05, y: cy + Math.sin(p * .3) * 3, p: .5}))};
    });
}
function run(label, strokes, queries, radius) {
    const sweep = fn => { let h = 0; for (const [x, y] of queries) h += fn(strokes, x, y, radius).length; return h; };
    legacy(strokes, 0, 0, 1); rust.strokesHitCircle(strokes, 0, 0, 1, 'warm');   // warm both caches (bounds, resident points)
    for (let i = 0; i < 3; i++) { sweep(legacy); sweep((s, x, y, r) => rust.strokesHitCircle(s, x, y, r, 'g')); }
    const timeIt = fn => { const t = performance.now(); let h = 0; for (let rep = 0; rep < 5; rep++) h = sweep(fn); return [(performance.now() - t) / (5 * queries.length) * 1000, h]; };
    const [jsUs, jsHits] = timeIt(legacy);
    const [rsUs, rsHits] = timeIt((s, x, y, r) => rust.strokesHitCircle(s, x, y, r, 'g'));
    if (jsHits !== rsHits) throw new Error(`hit mismatch ${jsHits} vs ${rsHits}`);
    // first contact after a document edit: generation changes, so retained coordinates are re-uploaded for candidates only
    let gen = 0; const t0 = performance.now();
    for (let i = 0; i < 40; i++) rust.strokesHitCircle(strokes, queries[i][0], queries[i][1], radius, `edit${gen++}`);
    const coldUs = (performance.now() - t0) / 40 * 1000;
    console.log(`${label.padEnd(44)} JS ${jsUs.toFixed(1).padStart(8)} us/query   Rust ${rsUs.toFixed(1).padStart(8)} us/query   speedup x${(jsUs / rsUs).toFixed(1)}   (after-edit query ${coldUs.toFixed(1)} us, ${rsHits} hits)`);
}
const random = rng(5);
const sample = (strokes, n) => Array.from({length: n}, () => { const s = strokes[Math.floor(random() * strokes.length)], p = s.points[Math.floor(random() * s.points.length)]; return [p.x + (random() - .5) * 6, p.y + (random() - .5) * 6]; });
let strokes = page(1, 3000, random);
run('1 long stroke, 3000 points, r=15', strokes, sample(strokes, 400), 15);
run('1 long stroke, 3000 points, r=2', strokes, sample(strokes, 400), 2);
strokes = page(300, 100, random);
run('300 strokes x 100 points (typical page), r=15', strokes, sample(strokes, 400), 15);
strokes = page(2400, 100, random);
run('2400 strokes x 100 points (dense page), r=15', strokes, sample(strokes, 400), 15);
run('2400 strokes x 100 points (dense page), r=4', strokes, sample(strokes, 400), 4);
strokes = page(600, 600, random);
run('600 strokes x 600 points (long ink), r=15', strokes, sample(strokes, 200), 15);
