// W6.2: main-thread CPU profile of one action on the `dense-page` fixture, self time by function (wasm functions
// appear by index unless the build keeps names: `CARGO_PROFILE_RELEASE_STRIP=false`, unoptimised binary).
//   node tests/v6/perf/cpuprofile.mjs <tree: a v6 checkout with build/, or a 5.18.0 tree> zoom|scroll|erase|lasso
import { chromium } from '@playwright/test';
import fs from 'node:fs';
import path from 'node:path';
import { startServer } from '../server.mjs';
import { createMock } from '../mock/google.mjs';
import { addScene, addAccounts, ANA } from '../mock/fixtures.mjs';
import { chromiumExecutable } from '../parity/matrix.mjs';
const [root, action = 'zoom'] = process.argv.slice(2);
const v6 = fs.existsSync(path.join(root, 'build', 'index.html'));
const server = await startServer({ root, overlay: v6 ? path.join(root, 'build') : null });
const browser = await chromium.launch({ executablePath: chromiumExecutable() });
const context = await browser.newContext({ viewport: { width: 1200, height: 900 }, serviceWorkers: 'block' });
const mock = createMock({ drive: { now: () => Date.now() } }); addAccounts(mock);
const file = addScene(mock, 'dense-page', { title: 'P.pdf' });
await mock.install(context);
await context.addInitScript(e => { if (!localStorage.getItem('s')) { localStorage.setItem('s', '1'); for (const [k, v] of Object.entries(e)) localStorage.setItem(k, v); } }, mock.session(ANA));
const page = await context.newPage();
await page.goto(`${server.url}/?drive=${file.id}&e2e=1`);
for (let i = 0; i < 400; i++) { if (await page.evaluate(() => !!document.querySelector('canvas.page-canvas')?.offsetParent)) break; await new Promise(r => setTimeout(r, 50)); }
await page.locator('#mode-toggle-btn').first().click();
await new Promise(r => setTimeout(r, 4000));
const r = await page.evaluate(() => { const c = document.querySelector('canvas.page-canvas[data-page="0"]'); const b = c.getBoundingClientRect(); return { x: b.left, y: b.top, w: b.width, h: b.height }; });
const cdp = await context.newCDPSession(page);
await cdp.send('Profiler.enable'); await cdp.send('Profiler.setSamplingInterval', { interval: 100 });
await cdp.send('Profiler.start');
const t0 = Date.now();
if (action === 'zoom') { await page.mouse.move(r.x + r.w / 2, 450); await page.keyboard.down('Control'); await page.mouse.wheel(0, -100); await page.keyboard.up("Control"); await new Promise(r => setTimeout(r, 8000)); }
if (action === 'scroll') { await page.mouse.move(r.x + r.w / 2, 450); for (let i = 0; i < 40; i++) { await page.mouse.wheel(0, i < 20 ? 120 : -120); await new Promise(r => setTimeout(r, 16)); } await new Promise(r => setTimeout(r, 1000)); }
if (action === 'erase' || action === 'lasso') {
  await page.locator(`[data-tool="${action === 'erase' ? 'eraser' : 'lasso'}"]`).first().click();
  const m = (type, x, y, buttons) => cdp.send('Input.dispatchMouseEvent', { type, x, y, button: 'left', buttons, clickCount: 1, pointerType: 'pen' });
  const cx = r.x + r.w * 0.5, cy = r.y + r.h * 0.4, R = Math.min(r.w, r.h) * 0.25;
  await m('mousePressed', cx + R, cy, 1);
  for (let k = 1; k <= 48; k++) await m('mouseMoved', cx + R * Math.cos(k / 48 * 2 * Math.PI), cy + R * Math.sin(k / 48 * 2 * Math.PI), 1);
  await m('mouseReleased', cx + R, cy, 0);
  await new Promise(r => setTimeout(r, 1500));
}
const { profile } = await cdp.send('Profiler.stop');
const self = new Map(); const byId = new Map(profile.nodes.map(n => [n.id, n]));
const dt = profile.timeDeltas; let total = 0;
profile.samples.forEach((id, i) => { const n = byId.get(id); const k = `${n.callFrame.functionName || '(anon)'} ${n.callFrame.url.split('/').pop().split('?')[0]}:${n.callFrame.lineNumber}`; self.set(k, (self.get(k) || 0) + (dt[i] || 0)); total += dt[i] || 0; });
console.log('wall', Date.now() - t0, 'profiled ms', Math.round(total / 1000));
for (const [k, v] of [...self].sort((a, b) => b[1] - a[1]).slice(0, 25)) console.log((v / 1000).toFixed(1).padStart(8), k);
await browser.close(); await server.close();
