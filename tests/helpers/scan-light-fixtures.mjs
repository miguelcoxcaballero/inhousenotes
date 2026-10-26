// Shared helpers for the scan_light differential tests and benchmarks: the pre-port
// lightweight.js (with its private functions exposed for testing only), the Rust core
// loaded as the app loads it, and deterministic synthetic scanner pages.
import { readFile } from 'node:fs/promises';
import vm from 'node:vm';
import { legacySource, rng } from './legacy.mjs';

/** The original scanner/processing/lightweight.js from PORT_BASE with selected private functions exported. */
export function loadLegacyLightweight(exposed = ['removePhotographedStencil', 'isYellow']) {
    const source = legacySource('scanner/processing/lightweight.js');
    const marker = '\n})();';
    const end = source.lastIndexOf(marker);
    if (end < 0) throw new Error('Unexpected lightweight.js layout');
    const patched = `${source.slice(0, end)}\nSP.__legacy = { ${exposed.join(', ')} };${source.slice(end)}`;
    // Yields resolve at once: the differential tests only compare results, not scheduling.
    const scope = { console, setTimeout, clearTimeout, performance, Promise, URL, scheduler: { yield: () => Promise.resolve() } };
    scope.window = scope.self = scope.globalThis = scope;
    vm.createContext(scope);
    vm.runInContext(patched, scope, { filename: 'legacy:scanner/processing/lightweight.js' });
    return scope.ScannerPro.__legacy;
}

export async function loadRustCore() {
    const bytes = await readFile(new URL('../../wasm/inhouse_notes_core.wasm', import.meta.url));
    const scope = { WebAssembly, URL, location: new URL('https://notes.test/'), fetch: async () => new Response(bytes),
        console, AbortSignal, AbortController, setTimeout, clearTimeout, TextEncoder, TextDecoder };
    vm.createContext(scope);
    vm.runInContext(await readFile(new URL('../../rust-core-v5.js', import.meta.url), 'utf8'), scope);
    if (!await scope.InhouseRust.ready) throw new Error('Rust core did not load');
    return scope.InhouseRust;
}

/** Runs the Rust stencil removal to completion and returns {stats, steps}; pixels are edited in place. */
export function rustStencil(rust, data, width, height, pxPerCm, targetPaper, settings = {}) {
    const job = rust.scanLight.stencil(data, width, height, pxPerCm, targetPaper, settings.dotObservations, settings.photographedDotObservations);
    if (!job) throw new Error('Rust stencil rejected its input');
    try {
        let steps = 1;
        while (job.step()) steps++;
        job.copyTo(data);
        return { stats: job.result(), steps };
    } finally { job.dispose(); }
}

const clampByte = value => Math.max(0, Math.min(255, Math.round(value)));

/**
 * A colour-corrected scanner page at `pxPerCm`: paper with noise, the yellow printed frame, footer boxes and
 * reference dots, the 35 x 51 grey dot lattice (with regional drift like paper curl) and handwriting that
 * crosses some dots. Returns the pixels plus dot observations as the geometry stage reports them.
 */
export function syntheticPage(seed, pxPerCm = 12, options = {}) {
    const random = rng(seed);
    const range = (a, b) => a + (b - a) * random();
    const width = Math.round(21 * pxPerCm), height = Math.round(29.7 * pxPerCm);
    const data = new Uint8ClampedArray(width * height * 4);
    const paper = options.paper ?? Math.round(range(226, 252));
    const tint = [range(-4, 4), range(-4, 4), range(-6, 2)];
    const noise = options.noise ?? range(0.5, 3);
    for (let i = 0; i < width * height; i++) {
        const n = (random() - 0.5) * noise * 2;
        data[i * 4] = clampByte(paper + tint[0] + n); data[i * 4 + 1] = clampByte(paper + tint[1] + n);
        data[i * 4 + 2] = clampByte(paper + tint[2] + n); data[i * 4 + 3] = 255;
    }
    const blend = (x, y, color, alpha) => {
        x = Math.round(x); y = Math.round(y);
        if (x < 0 || y < 0 || x >= width || y >= height || alpha <= 0) return;
        const o = (y * width + x) * 4;
        for (let c = 0; c < 3; c++) data[o + c] = clampByte(data[o + c] * (1 - alpha) + color[c] * alpha);
    };
    const line = (x1, y1, x2, y2, halfWidth, color, alpha = 1) => {
        const steps = Math.ceil(Math.hypot(x2 - x1, y2 - y1) * 2) + 1;
        const reach = Math.ceil(halfWidth + 1);
        for (let s = 0; s <= steps; s++) {
            const px = x1 + (x2 - x1) * s / steps, py = y1 + (y2 - y1) * s / steps;
            for (let dy = -reach; dy <= reach; dy++) for (let dx = -reach; dx <= reach; dx++) {
                const d = Math.hypot(dx - (px - Math.round(px)), dy - (py - Math.round(py)));
                blend(px + dx, py + dy, color, Math.max(0, Math.min(1, halfWidth + 0.5 - d)) * alpha);
            }
        }
    };
    const yellow = options.yellow ?? [range(215, 248), range(180, 215), range(40, 110)];
    const grey = Math.round(range(40, 90));
    // Printed frame rails, footer boxes and ticks.
    const railWidth = Math.max(0.6, 0.04 * pxPerCm);
    for (const x of [1.5, 19.5]) line(x * pxPerCm, 1.25 * pxPerCm, x * pxPerCm, 28.6 * pxPerCm, railWidth, yellow);
    for (const y of [1.43, 28.43]) line(1.35 * pxPerCm, y * pxPerCm, 19.65 * pxPerCm, y * pxPerCm, railWidth, yellow);
    for (const y of [27.43, 27.93]) {
        line(1.82 * pxPerCm, y * pxPerCm, 10.18 * pxPerCm, y * pxPerCm, railWidth * 0.8, yellow, 0.8);
        line(10.82 * pxPerCm, y * pxPerCm, 19.18 * pxPerCm, y * pxPerCm, railWidth * 0.8, [150, 150, 150], 0.6);
    }
    for (let x = 2; x <= 10.0001; x += 0.5) line(x * pxPerCm, 27.24 * pxPerCm, x * pxPerCm, 28.12 * pxPerCm, railWidth * 0.7, yellow, 0.9);
    for (let x = 11; x <= 19.0001; x += 0.5) line(x * pxPerCm, 27.24 * pxPerCm, x * pxPerCm, 28.12 * pxPerCm, railWidth * 0.7, [130, 130, 130], 0.7);
    for (const x of [10.125, 10.375, 10.625, 10.875]) {
        const cx = x * pxPerCm, cy = 27.68 * pxPerCm, r = 0.13 * pxPerCm;
        for (let dy = -Math.ceil(r) - 1; dy <= Math.ceil(r) + 1; dy++) for (let dx = -Math.ceil(r) - 1; dx <= Math.ceil(r) + 1; dx++)
            blend(cx + dx, cy + dy, [35, 35, 35], Math.max(0, Math.min(1, r + 0.5 - Math.hypot(dx, dy))));
    }
    // Dot lattice with a smooth regional displacement.
    const driftX = range(-0.08, 0.08), driftY = range(-0.08, 0.08), curl = range(0, 0.06);
    const dotAmplitude = range(35, 80), sigma = Math.max(0.9, range(0.032, 0.05) * pxPerCm);
    const tone = options.neutral === false ? [1, 0.93, 0.75] : [1, 1, 1];
    const centres = [];
    for (let column = 0; column < 35; column++) for (let row = 0; row < 51; row++) {
        const cx = (2 + column * 0.5 + driftX + curl * Math.sin(row / 8) + curl * 0.5 * Math.cos(column / 5)) * pxPerCm;
        const cy = (1.93 + row * 0.5 + driftY + curl * Math.cos(column / 6)) * pxPerCm;
        centres.push({ column, row, x: cx, y: cy });
        if (random() < (options.missing ?? 0.02)) continue;
        const reach = Math.ceil(sigma * 3);
        for (let dy = -reach; dy <= reach; dy++) for (let dx = -reach; dx <= reach; dx++) {
            const x = Math.round(cx) + dx, y = Math.round(cy) + dy;
            const d2 = ((x - cx) ** 2 + (y - cy) ** 2) / (sigma * sigma);
            const o = (Math.max(0, Math.min(height - 1, y)) * width + Math.max(0, Math.min(width - 1, x))) * 4;
            if (x < 0 || y < 0 || x >= width || y >= height) continue;
            const depth = dotAmplitude * Math.exp(-d2);
            for (let c = 0; c < 3; c++) data[o + c] = clampByte(data[o + c] - depth * tone[c]);
        }
    }
    // Handwriting: smooth strokes in several inks, some crossing dots, plus a few pencil scribbles and speckles.
    const inks = [[25, 40, 130], [20, 20, 22], [90, 90, 96], [150, 30, 40], [60, 60, 66]];
    const strokes = options.strokes ?? Math.round(range(6, 40));
    for (let s = 0; s < strokes; s++) {
        let x = range(2, 19) * pxPerCm, y = range(2, 26.5) * pxPerCm, angle = range(0, Math.PI * 2);
        const ink = inks[Math.floor(random() * inks.length)], half = Math.max(0.5, range(0.012, 0.05) * pxPerCm), alpha = range(0.5, 1);
        for (let segment = 0; segment < 4 + Math.floor(random() * 14); segment++) {
            angle += range(-0.9, 0.9);
            const nx = x + Math.cos(angle) * range(0.1, 0.7) * pxPerCm, ny = y + Math.sin(angle) * range(0.1, 0.7) * pxPerCm;
            line(x, y, nx, ny, half, ink, alpha); x = nx; y = ny;
        }
    }
    if (random() < 0.5) for (let i = 0; i < 25; i++) { // i-dots and speckles landing on lattice sites
        const centre = centres[Math.floor(random() * centres.length)];
        line(centre.x + range(-1, 1), centre.y + range(-1, 1), centre.x + range(-1, 1), centre.y + range(-1, 1), range(0.7, 1.6), inks[Math.floor(random() * 3)], range(0.4, 1));
    }
    // Observations as the geometry stage reports them (Canvas edge coordinates = pixel centre + 0.5).
    const geometric = [], photographed = [];
    const rate = options.observationRate ?? range(0.5, 0.95);
    for (const centre of centres) {
        if (random() > rate) continue;
        const jitter = options.observationJitter ?? 0.6;
        const nominalX = (2 + centre.column * 0.5) * pxPerCm, nominalY = (1.93 + centre.row * 0.5) * pxPerCm;
        const point = { x: centre.x + 0.5 + range(-jitter, jitter), y: centre.y + 0.5 + range(-jitter, jitter),
            centerRefined: random() < 0.6, dotContrast: random() < 0.8 ? range(2, 40) : 0 };
        if (random() < 0.5) { point.col = centre.column; point.row = centre.row; } else { point.column = centre.column; point.row = centre.row; }
        if (random() < 0.12) { point.x = nominalX + range(-0.3, 0.3) * pxPerCm; point.y = nominalY + range(-0.3, 0.3) * pxPerCm; }
        (random() < 0.7 ? geometric : photographed).push(point);
    }
    return { data, width, height, pxPerCm, paper, geometric, photographed };
}

/** Pure noise images for edge and boundary behaviour (tiny sizes, odd scales, extreme colours). */
export function noisePage(seed) {
    const random = rng(seed);
    const range = (a, b) => a + (b - a) * random();
    const pxPerCm = [0.7, 1, 2, 3, 4, 5.5, 8, 11.5][Math.floor(random() * 8)];
    const width = Math.max(1, Math.round(range(0.2, 1.1) * 21 * pxPerCm)), height = Math.max(1, Math.round(range(0.2, 1.1) * 29.7 * pxPerCm));
    const data = new Uint8ClampedArray(width * height * 4);
    const mode = Math.floor(random() * 4);
    for (let i = 0; i < width * height; i++) {
        const base = mode === 0 ? range(0, 255) : mode === 1 ? range(120, 255) : mode === 2 ? (random() < 0.5 ? 245 : range(60, 200)) : 235;
        for (let c = 0; c < 3; c++) data[i * 4 + c] = clampByte(mode === 3 ? base + range(-30, 6) * (random() < 0.3 ? 1 : 0) : base + range(-25, 25) * (random() < 0.6 ? 1 : 0));
        data[i * 4 + 3] = 255;
    }
    return { data, width, height, pxPerCm, target: [255, 250, 245, 240.5, 245.5, 100.25, 0, 254.5][Math.floor(random() * 8)] };
}

/**
 * A structured page that is then damaged at random: cropped to odd sizes (so lattice cells fall outside the buffer and
 * wrap onto neighbouring rows), rescaled in pxPerCm independently of the rendering, with random pixel corruption,
 * fractional paper targets and damaged observation lists.
 */
export function mutatedPage(seed) {
    const random = rng(seed * 7919 + 13);
    const range = (a, b) => a + (b - a) * random();
    const px = [4, 5, 6, 7, 8, 9][Math.floor(random() * 6)];
    const page = syntheticPage(seed, px, { strokes: Math.floor(range(0, 30)), observationRate: range(0.6, 1), missing: range(0, 0.2),
        neutral: random() < 0.7, observationJitter: range(0.1, 1.5) });
    let { data, width, height } = page;
    if (random() < 0.5) { // crop
        const x0 = Math.floor(random() * width * 0.3), y0 = Math.floor(random() * height * 0.3);
        const w = Math.max(1, Math.floor(width * range(0.3, 1)) - x0), h = Math.max(1, Math.floor(height * range(0.3, 1)) - y0);
        const cropped = new Uint8ClampedArray(w * h * 4);
        for (let y = 0; y < h; y++) cropped.set(data.subarray(((y0 + y) * width + x0) * 4, ((y0 + y) * width + x0 + w) * 4), y * w * 4);
        data = cropped; width = w; height = h;
    }
    const corrupt = Math.floor(random() * 4000);
    for (let i = 0; i < corrupt; i++) {
        const o = Math.floor(random() * width * height) * 4;
        const mode = random();
        for (let c = 0; c < 3; c++) data[o + c] = mode < 0.4 ? 255 * random() : mode < 0.7 ? data[o + c] + range(-40, 40) : mode < 0.85 ? 0 : 255;
    }
    const scale = random() < 0.6 ? px : px * range(0.8, 1.25);
    const target = random() < 0.4 ? Math.round(range(200, 255)) : range(180, 255);
    const damage = list => list.filter(() => random() > 0.03).map(point => random() < 0.02 ? { ...point, x: point.x + range(-20, 20), centerRefined: undefined } : point);
    return { data, width, height, pxPerCm: scale, target, geometric: damage(page.geometric), photographed: damage(page.photographed) };
}
