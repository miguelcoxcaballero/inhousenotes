import {test, expect} from '@playwright/test';
import {legacySource} from '../helpers/legacy.mjs';

// Pixel parity of the Rust-backed stroke drawing against the original JS drawing code (git base commit),
// both run on real canvases inside the app page with the app's own helpers.
function extract(source, name) {
    const start = source.indexOf(`\n        function ${name}(`);
    let i = source.indexOf('{', source.indexOf(')', start)), depth = 0;
    for (; i < source.length; i++) {
        const c = source[i];
        if (c === '/' && source[i + 1] === '/') { i = source.indexOf('\n', i); continue; }
        if (c === "'" || c === '"') { for (i++; source[i] !== c; i++) if (source[i] === '\\') i++; continue; }
        if (c === '{') depth++;
        else if (c === '}' && --depth === 0) return source.slice(start, i + 1);
    }
    throw new Error(`unterminated ${name}`);
}
const legacy = legacySource('app-v5.js');
const legacyCode = ['drawSmoothStroke', 'getStrokeBounds', 'drawLegacyCoverStrokes'].map(name => extract(legacy, name)).join('\n');

test('Rust stroke drawing and legacy covers render the same pixels as the original JS', async ({page}) => {
    await page.goto('/?e2e=1');
    await page.evaluate(() => __IHN_TEST_API__.ready());
    const result = await page.evaluate(async code => {
        if (!await InhouseRust.ready) throw new Error('Rust not loaded');
        const old = new Function('setupContext', 'setStrokeCaps', 'getStrokeWidth', 'drawFixedHighlighterStroke', `
            const penPathCache = new WeakMap(), strokeBoundsCache = new WeakMap();
            ${code}
            return {drawSmoothStroke, drawLegacyCoverStrokes, strokeBoundsCache};`)(setupContext, setStrokeCaps, getStrokeWidth, drawFixedHighlighterStroke);
        let seed = 987654321;
        const random = () => ((seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0) / 4294967296);
        const tools = ['pen', 'pen', 'pen', 'highlighter', 'eraser-stroke'];
        const strokes = Array.from({length: 120}, (_, s) => {
            let x = random() * 700, y = random() * 900;
            return {id: `p${s}`, tool: tools[Math.floor(random() * tools.length)], color: ['#102030', '#c0392b', '#ffde00'][s % 3], width: 1 + random() * 8,
                points: Array.from({length: s % 9 === 0 ? 1 : 2 + Math.floor(random() * (s % 5 === 0 ? 900 : 60))}, () => ({x: (x += (random() - .5) * 9), y: (y += (random() - .5) * 9), pressure: random()}))};
        });
        const render = (draw, cover) => {
            const canvas = document.createElement('canvas');
            canvas.width = 800; canvas.height = 1000;
            const ctx = canvas.getContext('2d', {willReadFrequently: true});
            ctx.fillStyle = '#ffffff'; ctx.fillRect(0, 0, 800, 1000);
            cover(ctx, {legacyCoverStrokes: strokes.filter((_, i) => i % 3 === 0)});
            for (const stroke of strokes) if (stroke.tool !== 'eraser-stroke') draw(ctx, stroke);
            // Second paint hits the path cache and must not change a pixel.
            for (const stroke of strokes) if (stroke.tool !== 'eraser-stroke') draw(ctx, stroke);
            return ctx.getImageData(0, 0, 800, 1000).data;
        };
        const before = render(old.drawSmoothStroke, old.drawLegacyCoverStrokes);
        const after = render(drawSmoothStroke, drawLegacyCoverStrokes);
        let differing = 0, inked = 0;
        for (let i = 0; i < before.length; i++) { if (before[i] !== after[i]) differing++; if (i % 4 === 0 && before[i] < 250) inked++; }
        const sample = strokes.find(stroke => stroke.tool === 'pen' && stroke.points.length > 2);
        const bounds = old.strokeBoundsCache.get(sample), seeded = getStrokeBounds(sample);
        return {differing, inked, boundsMatch: ['minX', 'minY', 'maxX', 'maxY'].every(key => bounds[key] === seeded[key])};
    }, legacyCode);
    expect(result.inked).toBeGreaterThan(5000);
    expect(result.differing).toBe(0);
    expect(result.boundsMatch).toBe(true);
});
