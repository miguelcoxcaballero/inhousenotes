// Micro-benchmark: original JavaScript vs Rust stencil removal on a full-size 2546 px page.
// Run: node tests/scan-light-stencil.bench.mjs
import { performance } from 'node:perf_hooks';
import { loadLegacyLightweight, loadRustCore, rustStencil, syntheticPage } from './helpers/scan-light-fixtures.mjs';

const legacy = loadLegacyLightweight();
const rust = await loadRustCore();
const pxPerCm = 2546 / 29.7;
const median = values => values.sort((a, b) => a - b)[values.length >> 1];
for (const [label, observed] of [['observed dots (geometry found the lattice)', true], ['recovered phase (no observations)', false]]) {
    const page = syntheticPage(11, pxPerCm, { observationRate: 0.93, strokes: 40 });
    const settings = observed ? { dotObservations: page.geometric, photographedDotObservations: page.photographed } : {};
    const jsTimes = [], rustTimes = [], slices = [];
    let same = true;
    for (let run = 0; run < 5; run++) {
        const a = page.data.slice(), b = page.data.slice();
        let started = performance.now();
        // Original timing includes its own yields (microtasks only here), no browser scheduling.
        await legacy.removePhotographedStencil(a, page.width, page.height, pxPerCm, page.paper, settings);
        jsTimes.push(performance.now() - started);
        started = performance.now();
        const result = rustStencil(rust, b, page.width, page.height, pxPerCm, page.paper, settings);
        rustTimes.push(performance.now() - started); slices.push(result.steps);
        same &&= a.every((value, i) => value === b[i]);
    }
    const js = median(jsTimes), rs = median(rustTimes);
    console.log(`${label}: ${page.width}x${page.height}  JS ${js.toFixed(0)} ms  Rust ${rs.toFixed(0)} ms (incl. copy in/out, ${slices[0]} slices)  speedup ${(js / rs).toFixed(1)}x  identical=${same}`);
}
