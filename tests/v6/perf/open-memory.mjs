// W6.2: opens a 5.x scene in v6 (this checkout + build/) and prints JS heap, wasm memory, page count and the
// importer's memory marks every 500 ms; console lines are passed through.
//   node tests/v6/perf/open-memory.mjs [scene=dense-20]
import { chromium } from '@playwright/test';
import { startServer } from '../server.mjs';
import { createMock } from '../mock/google.mjs';
import { addScene, addAccounts, ANA } from '../mock/fixtures.mjs';
import { chromiumExecutable } from '../parity/matrix.mjs';
import { REPO } from '../parity/run.mjs';
import path from 'node:path';
const scene = process.argv[2] || 'dense-20';
const server = await startServer({ root: REPO, overlay: path.join(REPO, 'build') });
const browser = await chromium.launch({ executablePath: chromiumExecutable() });
const context = await browser.newContext({ viewport: { width: 1200, height: 900 }, serviceWorkers: 'block' });
const mock = createMock({ drive: { now: () => Date.now() } }); addAccounts(mock);
const file = addScene(mock, scene, { title: 'P.pdf' });
await mock.install(context);
await context.addInitScript(e => { if (!localStorage.getItem('s')) { localStorage.setItem('s', '1'); for (const [k, v] of Object.entries(e)) localStorage.setItem(k, v); } }, mock.session(ANA));
const page = await context.newPage();
page.on('console', m => console.log('console', m.type(), m.text().slice(0, 6000)));
page.on('crash', () => console.log('CRASH'));
page.on('pageerror', e => console.log('pageerror', e.message));
const cdp = await context.newCDPSession(page); await cdp.send('Performance.enable');
const t0 = Date.now();
await page.goto(`${server.url}/?drive=${file.id}&e2e=1`);
for (let i = 0; i < 8; i++) {
  await new Promise(r => setTimeout(r, 500));
  try {
    const m = Object.fromEntries((await cdp.send('Performance.getMetrics')).metrics.map(m => [m.name, m.value]));
    const w = await page.evaluate(() => ({ wasm: self.__IHN_TEST_API__?.wasmMemoryBytes?.(), pages: document.querySelectorAll('.page-wrapper').length, canv: document.querySelectorAll('canvas.page-canvas').length, v5: self.__IHN_TEST_API__?.cmd('doc.v5open.state') }));
    console.log(Date.now() - t0, 'heap', (m.JSHeapUsedSize / 1048576).toFixed(1), 'wasm', (w.wasm / 1048576).toFixed(1), 'pages', w.pages, w.canv, JSON.stringify(w.v5)?.slice(0, 300));
  } catch (e) { console.log('ERR', e.message.slice(0, 200)); break; }
}
await browser.close(); await server.close();
