import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import vm from 'node:vm';

const bytes = await readFile(new URL('../wasm/inhouse_notes_core.wasm', import.meta.url));
const source = await readFile(new URL('../rust-core-v5.js', import.meta.url), 'utf8');
const plain = value => JSON.parse(JSON.stringify(value));
const bounds = { left: 24, top: 75, width: 390, height: 844 };
const card = { left: 57, top: 154, width: 160, height: 220 };
function adapter(options = {}) {
    const scope = { WebAssembly, Uint8Array, Uint16Array, Float32Array, Float64Array, URL,
        location: new URL('https://notes.test/'), fetch: async () => new Response(bytes),
        console, AbortSignal, AbortController, setTimeout, clearTimeout, ...options };
    vm.createContext(scope);
    vm.runInContext(source, scope);
    return scope.InhouseRust;
}
const api = adapter();
assert.equal(await api.ready, true);
const near = (actual, expected, tolerance = 1e-8) => assert.ok(Math.abs(actual - expected) <= tolerance * Math.max(1, Math.abs(expected)), `${actual} != ${expected}`);
function transform(frame) {
    const [, x, y, scale] = /^translate\(([-\de+.]+)px, ([-\de+.]+)px\) scale\(([-\de+.]+)\)$/.exec(frame.transform);
    return { x: +x, y: +y, scale: +scale };
}

test('folder plans preserve card and screen endpoints', () => {
    for (const container of [bounds, { left: -173, top: 12.5, width: 1280, height: 720 }, { left: 0, top: 0, width: 320, height: 480 }]) {
        for (const rect of [card, { left: 700, top: -800, width: 87.5, height: 43 }, { left: -20, top: 0, width: 1000, height: 1700 }]) {
            const motion = api.folderMotion(container, rect), closing = api.folderMotion(container, rect, true);
            assert.equal(motion.duration, 600);
            assert.equal(closing.duration, 560);
            assert.equal(motion.hingeFrom, 0);
            assert.equal(motion.hingeTo, -112);
            assert.equal(closing.hingeFrom, -112);
            assert.equal(closing.hingeTo, 0);
            const initial = transform(motion.zoomFrames[0]), final = transform(motion.zoomFrames[1]);
            // The initial enlarged raster must visually occupy the original card.
            near(initial.scale * motion.width, rect.width);
            near(initial.scale * motion.height, rect.height);
            near(initial.x, 0); near(initial.y, 0);
            near(final.scale, 1);
            near(rect.left - container.left + final.x, motion.finalLeft);
            near(rect.top - container.top + final.y, motion.finalTop);
            assert.ok(motion.width >= container.width + 4 - 1e-8);
            assert.ok(motion.height * .94 >= container.height + 4 - 1e-8);
            near(motion.finalLeft + motion.width / 2, container.width / 2);
            near(motion.finalTop + motion.height * .06 + motion.height * .94 / 2, container.height / 2);
            assert.deepEqual(plain(closing.zoomFrames), plain([...motion.zoomFrames].reverse()));
        }
    }
});

test('motion rejects invalid or overflowing layout data without producing invalid CSS', () => {
    for (const invalid of [null, {}, { ...bounds, width: 0 }, { ...bounds, height: -1 }, { ...bounds, left: NaN }, { ...bounds, top: Infinity }, { ...bounds, width: '390' }]) {
        assert.equal(api.folderMotion(invalid, card), null);
        assert.equal(api.folderMotion(bounds, invalid), null);
    }
    assert.equal(api.folderMotion(bounds, { ...card, width: Number.MIN_VALUE }), null);
    assert.equal(api.folderMotion({ ...bounds, width: Number.MAX_VALUE }, { ...card, width: 1 }), null);
});

test('fade plans have ordered offsets and continuous opacity at late interior mounts', () => {
    for (const fraction of [-3, 0, .1, .37, .69, .88, 1, 4]) {
        const frames = api.folderFade(fraction);
        assert.equal(frames[0].offset, 0);
        assert.equal(frames.at(-1).offset, 1);
        assert.equal(frames[0].opacity, 0);
        assert.equal(frames[1].opacity, 0);
        assert.equal(frames.at(-1).opacity, 1);
        for (let i = 1; i < frames.length; i++) assert.ok(frames[i].offset >= frames[i - 1].offset);
        for (const frame of frames) assert.ok(frame.opacity >= 0 && frame.opacity <= 1);
        if (fraction >= 0 && fraction <= .8) assert.ok(frames[1].offset >= fraction);
        assert.deepEqual(plain(api.folderFade(fraction, true)), [
            { offset: 0, opacity: 1 }, { offset: .3, opacity: 1 }, { offset: 1, opacity: 0 }
        ]);
    }
    for (const invalid of [undefined, NaN, Infinity, '0']) assert.equal(api.folderFade(invalid), null);
});

// Independent interval intersection reference, including empty intervals inside
// row gaps and extra CSS tracks that contain no documents.
function expectedRange(heights, gap, columns, count, top, bottom, overscan = 0) {
    let y = 0, start = count, end = count, found = false;
    for (let row = 0; row < Math.ceil(count / columns); row++) {
        const rowBottom = y + heights[row];
        if (rowBottom >= top - overscan && !found) { start = row * columns; end = start; found = true; }
        if (rowBottom >= top - overscan && y <= bottom + overscan) end = Math.min(count, (row + 1) * columns);
        y = rowBottom + gap;
    }
    return { start, end };
}
function checkRange(layout, heights, gap, columns, count, top, bottom, overscan = 0) {
    const actual = layout.range(top, bottom, overscan);
    const { start, end } = actual;
    assert.deepEqual({ start, end }, expectedRange(heights, gap, columns, count, top, bottom, overscan));
    assert.ok(start >= 0 && start <= end && end <= count);
    assert.equal(start === count ? 0 : start % columns, 0);
    assert.equal(end === count ? 0 : end % columns, 0);
}
test('variable row queries match visible physical tracks, gaps, overscan and partial last rows', () => {
    let seed = 72167;
    const random = () => ((seed = Math.imul(seed, 1664525) + 1013904223 >>> 0) / 0x100000000);
    for (let sample = 0; sample < 120; sample++) {
        const columns = 1 + Math.floor(random() * 8), count = Math.floor(random() * 180);
        const gap = random() * 50;
        const heights = Float64Array.from({ length: Math.ceil(count / columns) + 1 }, () => 40 + random() * 400);
        const layout = api.folderRows(heights, gap, columns, count);
        const total = [...heights].reduce((sum, height) => sum + height + gap, 0);
        for (let query = 0; query < 35; query++) {
            const top = random() * (total + 600) - 300, bottom = top + random() * 1100;
            checkRange(layout, heights, gap, columns, count, top, bottom, random() * 100);
        }
        let y = 0;
        for (let row = 0; row < Math.ceil(count / columns); row++) {
            checkRange(layout, heights, gap, columns, count, y, y);
            checkRange(layout, heights, gap, columns, count, y + heights[row], y + heights[row]);
            checkRange(layout, heights, gap, columns, count, y + heights[row] + gap / 2, y + heights[row] + gap / 2);
            near(layout.span(row, row + 1), heights[row]);
            y += heights[row] + gap;
        }
        const rowCount = Math.ceil(count / columns);
        near(layout.span(0, rowCount), rowCount ? y - gap : 0);
        layout.dispose();
    }
});

test('row contexts reject invalid data, are isolated and cannot be queried after disposal', () => {
    for (const arguments_ of [
        [[40], 0, 1, 1], [new Float64Array([40]), -1, 1, 1], [new Float64Array([40]), NaN, 1, 1],
        [new Float64Array([40]), 0, 0, 1], [new Float64Array([40]), 0, 1.5, 1],
        [new Float64Array([40]), 0, 0x100000000, 1], [new Float64Array([40]), 0, 1, 0x100000000],
        [new Float64Array([40]), 0, 1, 2], [new Float64Array([40]), 0, 1, -1],
        [new Float64Array([0]), 0, 1, 1], [new Float64Array([Infinity]), 0, 1, 1],
        [new Float64Array([1, NaN]), 0, 1, 1], [new Float64Array([Number.MAX_VALUE, Number.MAX_VALUE]), 1, 1, 2]
    ]) assert.equal(api.folderRows(...arguments_), null);
    const heights = new Float64Array([100, 150, 300]);
    const first = api.folderRows(heights, 25, 2, 5), second = api.folderRows(new Float64Array([250]), 0, 3, 3);
    heights.fill(1); // Caller changes cannot mutate an already measured context.
    assert.deepEqual(plain(first.range(120, 140)), { start: 2, end: 4 });
    for (const invalid of [[NaN, 10], [0, Infinity], [20, 10], [0, 10, -1], [-Number.MAX_VALUE, 10, Number.MAX_VALUE]]) assert.equal(first.range(...invalid), null);
    for (const invalid of [[-1, 1], [1, 0], [0, 4], [0, 1.5], [NaN, 2]]) assert.equal(first.span(...invalid), null);
    assert.equal(first.span(2, 2), 0);
    first.dispose(); first.dispose();
    assert.equal(first.range(0, 100), null);
    assert.equal(first.span(0, 1), null);
    assert.deepEqual(plain(second.range(0, 100)), { start: 0, end: 3 });
    second.dispose();
    const empty = api.folderRows(new Float64Array(), 0, 2, 0);
    assert.deepEqual(plain(empty.range(-100, 100)), { start: 0, end: 0 });
    assert.equal(empty.span(0, 0), 0);
    empty.dispose();
});

test('navigation stays synchronous before loading, without WASM, and never calls the core', async () => {
    let respond;
    const pending = adapter({ fetch: () => new Promise(resolve => { respond = resolve; }) });
    const absent = adapter({ WebAssembly: undefined });
    const old = adapter({ WebAssembly: { instantiate: async () => ({ instance: { exports: {
        core_abi_version: () => 1,
        alloc() { throw new Error('An old core must not be called for folder planning'); }
    } } }) } });
    await Promise.all([absent.ready, old.ready]);
    for (const fallback of [pending, absent, old]) {
        for (const backwards of [false, true]) {
            const motion = fallback.folderMotion(bounds, card, backwards);
            assert.deepEqual(plain(motion), plain(api.folderMotion(bounds, card, backwards)));
            for (const elapsed of [-1, 0, .37, 1, 2]) {
                const frames = fallback.folderFade(elapsed, backwards);
                assert.deepEqual(plain(frames), plain(api.folderFade(elapsed, backwards)));
            }
        }
        const heights = new Float64Array([170, 240, 30, 100]);
        const rows = fallback.folderRows(heights, 13, 2, 7);
        for (let y = -100; y < 900; y += 17) checkRange(rows, heights, 13, 2, 7, y, y + 5, 10);
        assert.equal(rows.span(1, 4), 396);
        rows.dispose();
        assert.equal(rows.range(0, 50), null);
        assert.equal(rows.span(0, 1), null);
        assert.equal(fallback.folderMotion(bounds, { ...card, width: Number.MIN_VALUE }), null);
        assert.equal(fallback.folderRows(new Float64Array([Number.MAX_VALUE, Number.MAX_VALUE]), 0, 1, 2), null);
    }
    const beforeReady = pending.folderRows(new Float64Array([150]), 0, 1, 1);
    respond(new Response(bytes));
    assert.equal(await pending.ready, true);
    assert.deepEqual(plain(beforeReady.range(0, 50)), { start: 0, end: 1 });
    beforeReady.dispose();
});
