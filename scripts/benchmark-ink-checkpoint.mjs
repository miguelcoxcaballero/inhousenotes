// Start node tests/e2e/server.mjs first. Measures the same 300,000 points with
// the legacy object checkpoint and the Rust binary checkpoint at 4x CPU throttle.
import { chromium } from '@playwright/test';
const browser = await chromium.launch();
try {
    const page = await browser.newPage();
    await page.goto('http://127.0.0.1:4173/?e2e=1');
    await page.evaluate(() => __IHN_TEST_API__.ready());
    await page.evaluate(() => __IHN_TEST_API__.seedDensePageForTest(3000, 100));
    await page.waitForFunction(() => !__IHN_TEST_API__.densePaintStatusForTest().pending);
    const cdp = await page.context().newCDPSession(page);
    await cdp.send('Emulation.setCPUThrottlingRate', { rate: 4 });
    console.log(await page.evaluate(async () => {
        const results = [];
        forceFlushingForExit = true;
        const clone = cloneSanitizedPageForStorage;
        const put = IDBObjectStore.prototype.put;
        try {
            // Drain fixture setup before timing either implementation.
            await queueSave();
            await flushDirtyPages({ force: true });
            for (const packed of [false, true]) {
                let putMs = 0, writes = 0;
                IDBObjectStore.prototype.put = function (...args) {
                    const start = performance.now();
                    try { return put.apply(this, args); }
                    finally { if (this.name === PAGE_STORE) { writes++; putMs += performance.now() - start; } }
                };
                cloneSanitizedPageForStorage = (page, session, stale) => clone(page, session, stale, packed);
                const started = performance.now();
                const saved = await savePageToIndexedDb(0, state.pages[0]);
                const elapsed = performance.now() - started;
                const restored = await loadPageFromIndexedDb(0);
                if (writes !== 1) throw new Error('Unrelated checkpoint interfered with the benchmark');
                results.push({ packed, saved, elapsed, putMs, points: restored.strokes.reduce((n, s) => n + s.points.length, 0) });
            }
        } finally {
            cloneSanitizedPageForStorage = clone;
            IDBObjectStore.prototype.put = put;
            forceFlushingForExit = false;
        }
        return results;
    }));
} finally { await browser.close(); }
