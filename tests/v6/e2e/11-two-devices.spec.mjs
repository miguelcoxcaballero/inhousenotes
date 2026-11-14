// 11-two-devices (TESTING §2 row 11 and §4, SYNC §19.3, BUILD-PLAN W4.5): two browser contexts (separate storage,
// one mock Drive) on the same document — real WebRTC between them (Chromium loopback), the Drive relay through the
// mock. (a) P2P: concurrent strokes and erases converge, live ink reaches the other device, latency samples;
// (b) P2P blocked + a signed-in viewer of another account: the relay converges, the viewer sees it and never writes
// or connects; (c) B offline, both edit, B back → converge; (d) two tabs of one context: tab 2 shows tab 1's stroke at
// once, closing the `ihn6-net` holder hands polling and the relay to tab 2; (e) a Drive-side rename reaches the open
// document's title. Presence avatars go `p2p-connecting` → `p2p-connected`.
import zlib from 'node:zlib';
import { test, expect, seedSession, ANA, BEN, CHO } from '../mock/fixtures.mjs';

test.use({ serviceWorkers: 'block', viewport: { width: 1280, height: 800 }, launchOptions: { ...(process.env.IHN_CHROMIUM ? { executablePath: process.env.IHN_CHROMIUM } : {}), args: ['--disable-features=WebRtcHideLocalIpsWithMdns'] } });
test.describe.configure({ timeout: 120_000 });

const ASSET = '6a'.repeat(32);
const PNG = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==', 'base64');

/** A PDF with `/Keywords` and embedded files: the v6 container of ARCHITECTURE §8.2. */
function containerPdf({ keywords, attachments = [], pages = 3 }) {
  const chunks = [Buffer.from('%PDF-1.7\n%\xe2\xe3\xcf\xd3\n', 'latin1')], offsets = [];
  let size = chunks[0].length;
  const push = buffer => { chunks.push(buffer); size += buffer.length; };
  const object = (number, body, stream) => {
    offsets[number] = size;
    push(Buffer.from(`${number} 0 obj\n${body}\n`, 'latin1'));
    if (stream) { push(Buffer.from('stream\n', 'latin1')); push(stream); push(Buffer.from('\nendstream\n', 'latin1')); }
    push(Buffer.from('endobj\n', 'latin1'));
  };
  const first = 4, files = first + pages;
  const names = attachments.map((a, i) => `(${a.name}) ${files + i * 2} 0 R`).join(' ');
  object(1, `<< /Type /Catalog /Pages 2 0 R${attachments.length ? ` /Names << /EmbeddedFiles << /Names [${names}] >> >>` : ''} >>`);
  object(2, `<< /Type /Pages /Kids [${Array.from({ length: pages }, (_, i) => `${first + i} 0 R`).join(' ')}] /Count ${pages} >>`);
  object(3, `<< /Title (Cuaderno Digital) /Keywords (${keywords}) >>`);
  for (let i = 0; i < pages; i++) object(first + i, '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595.5 842.25] >>');
  attachments.forEach((a, i) => {
    object(files + i * 2, `<< /Type /Filespec /F (${a.name}) /UF (${a.name}) /EF << /F ${files + i * 2 + 1} 0 R >> >>`);
    object(files + i * 2 + 1, `<< /Type /EmbeddedFile /Length ${a.data.length} >>`, a.data);
  });
  const count = files + attachments.length * 2, xref = size;
  let table = `xref\n0 ${count}\n0000000000 65535 f \n`;
  for (let n = 1; n < count; n++) table += `${String(offsets[n]).padStart(10, '0')} 00000 n \n`;
  push(Buffer.from(`${table}trailer\n<< /Size ${count} /Root 1 0 R /Info 3 0 R >>\nstartxref\n${xref}\n%%EOF\n`, 'latin1'));
  return Buffer.concat(chunks);
}

const cmd = (page, name, args) => page.evaluate(([n, a]) => window.__IHN_TEST_API__.cmd(n, a), [name, args]);
async function boot(page, url) {
  await page.goto(url);
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  expect(await page.evaluate(() => window.__IHN_TEST_API__.ready())).toBe(true);
}
const session = page => cmd(page, 'doc.session.state');
const collab = page => cmd(page, 'collab.state');

/** The fixture document as a v6 container in Ana's Drive, shared with `permissions`. */
async function seedDoc(page, mock, { id = 'v6-doc', permissions = [] } = {}) {
  await boot(page, '/?e2e=1');
  const snapshot = Buffer.from(await cmd(page, 'doc.session.fixture'), 'base64');
  const data = containerPdf({ keywords: `STROKES_Z:!;IHN6:${zlib.deflateSync(snapshot).toString('base64')}`, attachments: [{ name: `IHN6-${ASSET}`, data: PNG }] });
  mock.drive.addFile({ id, name: 'Fixture.pdf', owner: ANA, data, permissions, modifiedTime: '2026-10-29T09:00:00.000Z' });
}

/** A device: its own context (storage), signed in as `email`, the mock installed. */
async function device(browser, mock, email = ANA, { block = false } = {}) {
  const context = await browser.newContext({ serviceWorkers: 'block', viewport: { width: 1280, height: 800 } });
  await mock.install(context);
  await seedSession(context, mock, email);
  // P2P blocked: relay-only ICE without a TURN server gathers no candidate, so every connection fails.
  if (block) await context.addInitScript(() => { const Native = window.RTCPeerConnection; window.RTCPeerConnection = class extends Native { constructor(c) { super({ ...c, iceTransportPolicy: 'relay' }); } }; });
  const page = await context.newPage();
  const problems = [], writes = [];
  page.on('pageerror', e => problems.push(`pageerror: ${e.message}`));
  page.on('request', r => { if (/googleapis\.com/.test(r.url()) && r.method() !== 'GET') writes.push(`${r.method()} ${r.url()}`); });
  return { context, page, problems, writes };
}

/** Opens the document and switches to Edit mode (5.18.0 opens shared links in Read mode). */
async function openDoc(page, { edit = true, id = 'v6-doc' } = {}) {
  await boot(page, `/?drive=${id}&e2e=1`);
  await expect.poll(async () => (await cmd(page, 'doc.open.state')).log.some(e => e[0] === id && e[1]), { timeout: 20_000 }).toBe(true);
  await expect.poll(async () => (await collab(page)).active, { timeout: 10_000 }).toBe(true);
  if (edit) {
    await page.click('#mode-toggle-btn');
    await expect(page.locator('#editor-view')).not.toHaveClass(/read-only/);
  }
}

/** A stylus stroke through CDP on page 1 (`points` in page units). */
async function stroke(page, cdp, points, { gap = 0 } = {}) {
  const box = await page.locator('canvas.page-canvas[data-page="0"]').boundingBox();
  const at = p => [box.x + p[0] * box.width / 794, box.y + p[1] * box.height / 1123];
  const send = (type, p, buttons) => cdp.send('Input.dispatchMouseEvent', { type, x: at(p)[0], y: at(p)[1], button: 'left', buttons, clickCount: 1, pointerType: 'pen', force: 0.5 });
  await send('mousePressed', points[0], 1);
  for (const p of points.slice(1)) {
    await send('mouseMoved', p, 1);
    if (gap) await page.waitForTimeout(gap);
  }
  await send('mouseReleased', points.at(-1), 0);
}
const line = (x, y, n = 6) => Array.from({ length: n }, (_, i) => [x + i * 12, y + (i % 2) * 6]);

const same = async (a, b, timeout = 20_000) => {
  await expect.poll(async () => { const [x, y] = [await session(a), await session(b)]; return x.snapshot === y.snapshot && x.live === y.live ? x.live : -1; }, { timeout }).toBeGreaterThan(0);
};

test('(a) two devices over P2P: live ink, concurrent strokes and erases converge, presence connected', async ({ browser, page, mock, problems }) => {
  await seedDoc(page, mock);
  const A = await device(browser, mock), B = await device(browser, mock);
  await openDoc(A.page);
  const cdpA = await A.context.newCDPSession(A.page);
  // A device alone writes nothing until it edits (SYNC §9.2): A's first stroke publishes its record.
  await stroke(A.page, cdpA, line(100, 200));
  await expect.poll(async () => (await collab(A.page)).unsealed, { timeout: 20_000 }).toBe(0);
  await B.context.addInitScript(() => addEventListener('DOMContentLoaded', () => {
    window.__seen = new Set();
    const row = document.getElementById('presence-avatars');
    new MutationObserver(() => row.querySelectorAll('.presence-avatar-wrapper').forEach(w => window.__seen.add(w.className))).observe(row, { childList: true, subtree: true, attributes: true });
  }));
  await openDoc(B.page);
  // Presence: the newcomer sees A at once, writes its record (Company) and the handshake starts.
  await expect(B.page.locator('#presence-avatars .presence-avatar-wrapper')).toHaveCount(1, { timeout: 10_000 });
  await expect(A.page.locator('#presence-avatars .presence-avatar-wrapper')).toHaveCount(1, { timeout: 30_000 });
  await expect(A.page.locator('#presence-avatars .presence-avatar-wrapper.p2p-connected')).toHaveCount(1, { timeout: 40_000 });
  await expect(B.page.locator('#presence-avatars .presence-avatar-wrapper.p2p-connected')).toHaveCount(1, { timeout: 10_000 });
  await expect(A.page.locator('#presence-avatars .presence-peer-badge')).toHaveCount(1);
  expect(await B.page.evaluate(() => [...window.__seen])).toEqual(expect.arrayContaining(['presence-avatar-wrapper p2p-connecting', 'presence-avatar-wrapper p2p-connected']));
  expect((await collab(A.page)).mode).toBe('Linked');

  // Live ink: while A's pen is down, B holds the stroke in its overlay table.
  const cdpB = await B.context.newCDPSession(B.page);
  const base = (await session(A.page)).live;
  const box = await A.page.locator('canvas.page-canvas[data-page="0"]').boundingBox();
  const at = p => [box.x + p[0] * box.width / 794, box.y + p[1] * box.height / 1123];
  const t0 = Date.now();
  await cdpA.send('Input.dispatchMouseEvent', { type: 'mousePressed', x: at([200, 300])[0], y: at([200, 300])[1], button: 'left', buttons: 1, clickCount: 1, pointerType: 'pen', force: 0.5 });
  for (let i = 1; i < 6; i++) await cdpA.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: at([200 + i * 15, 300 + i * 4])[0], y: at([200 + i * 15, 300 + i * 4])[1], button: 'left', buttons: 1, pointerType: 'pen', force: 0.5 });
  await expect.poll(async () => (await cmd(B.page, 'collab.live')).length, { timeout: 2_000, intervals: [10] }).toBe(1);
  const liveMs = Date.now() - t0;
  await expect.poll(async () => (await collab(B.page)).overlayPages).toBe(1);
  await cdpA.send('Input.dispatchMouseEvent', { type: 'mouseReleased', x: at([290, 324])[0], y: at([290, 324])[1], button: 'left', buttons: 0, pointerType: 'pen', force: 0.5 });
  await expect.poll(async () => (await session(B.page)).live, { timeout: 5_000 }).toBe(base + 1);
  await expect.poll(async () => (await cmd(B.page, 'collab.live')).length).toBe(0);
  test.info().annotations.push({ type: 'p2p-live-first-point-ms', description: String(liveMs) });

  // Concurrent strokes on both devices.
  const samples = [];
  for (let i = 0; i < 6; i++) {
    const t = Date.now(), before = (await session(B.page)).live;
    await Promise.all([stroke(A.page, cdpA, line(100, 500 + i * 30)), stroke(B.page, cdpB, line(450, 500 + i * 30))]);
    await expect.poll(async () => (await session(B.page)).live, { timeout: 5_000, intervals: [10] }).toBeGreaterThanOrEqual(before + 2);
    samples.push(Date.now() - t);
  }
  await same(A.page, B.page);
  expect((await session(A.page)).live).toBe(base + 13);
  test.info().annotations.push({ type: 'p2p-batch-ms', description: samples.join(',') });

  // Stroke eraser on B removes one of A's strokes on both.
  await B.page.click('#toolbar .tool-btn[data-tool="eraser"]').catch(() => {});
  await B.page.click('#toolbar .tool-btn[data-tool="eraser-stroke"], #toolbar .tool-btn[data-tool="eraser"]').catch(() => {});
  const live = (await session(B.page)).live;
  await stroke(B.page, cdpB, [[160, 490], [160, 520], [160, 545]]);
  await expect.poll(async () => (await session(B.page)).live, { timeout: 5_000 }).toBeLessThan(live);
  await same(A.page, B.page);

  // Both relays sealed their batches.
  await expect.poll(async () => (await collab(A.page)).unsealed, { timeout: 20_000 }).toBe(0);
  await expect.poll(async () => (await collab(B.page)).unsealed, { timeout: 20_000 }).toBe(0);
  const records = Object.keys(mock.drive.file('v6-doc').properties).filter(k => k.startsWith('ihn6_'));
  expect(records.length).toBe(2);

  // Popup: own device first, the other device over peer-to-peer.
  await A.page.click('#presence-avatars');
  await expect(A.page.locator('#presence-popup')).not.toHaveClass(/hidden/);
  await expect(A.page.locator('.presence-device-connection')).toHaveCount(2);
  await expect(A.page.locator('.presence-device-connection').nth(1)).toContainText('Peer-to-peer');
  await A.page.keyboard.press('Escape');
  await expect(A.page.locator('#presence-popup')).toHaveClass(/hidden/);
  expect([...A.problems, ...B.problems, ...problems]).toEqual([]);
  await A.context.close();
  await B.context.close();
});

test('(b) relay only, and a viewer of another account that never writes or connects', async ({ browser, page, mock, problems }) => {
  await seedDoc(page, mock, { permissions: [{ type: 'user', role: 'writer', emailAddress: BEN }, { type: 'user', role: 'reader', emailAddress: CHO }] });
  mock.latency = 100;
  const A = await device(browser, mock, ANA, { block: true }), B = await device(browser, mock, BEN, { block: true }), V = await device(browser, mock, CHO, { block: true });
  await openDoc(A.page);
  const cdpA = await A.context.newCDPSession(A.page);
  await stroke(A.page, cdpA, line(100, 200));
  await expect.poll(async () => (await collab(A.page)).unsealed, { timeout: 20_000 }).toBe(0);
  await openDoc(B.page);
  await openDoc(V.page, { edit: false });
  expect((await session(V.page)).writable).toBe(false);
  await expect(B.page.locator('#presence-avatars .presence-avatar-wrapper')).toHaveCount(1, { timeout: 10_000 });
  await expect(A.page.locator('#presence-avatars .presence-avatar-wrapper')).toHaveCount(1, { timeout: 30_000 });
  // The viewer shows both editors and is shown by nobody.
  await expect(V.page.locator('#presence-avatars .presence-avatar-wrapper')).toHaveCount(2, { timeout: 30_000 });
  expect(['Company', 'RelayOnly']).toContain((await collab(A.page)).mode);
  const cdpB = await B.context.newCDPSession(B.page);
  const base = (await session(A.page)).live;
  const samples = [];
  for (let i = 0; i < 4; i++) {
    const t = Date.now();
    await stroke(A.page, cdpA, line(100, 400 + i * 40));
    await expect.poll(async () => (await session(B.page)).live, { timeout: 10_000, intervals: [50] }).toBe(base + 1 + i * 2);
    samples.push(Date.now() - t);
    await stroke(B.page, cdpB, line(400, 400 + i * 40));
    await page.waitForTimeout(150);
  }
  await same(A.page, B.page, 15_000);
  await same(A.page, V.page, 15_000);
  // Relay-only presence: no P2P classes.
  await expect(A.page.locator('#presence-avatars .presence-avatar-wrapper.p2p-connected')).toHaveCount(0);
  expect(V.writes).toEqual([]);
  expect(Object.keys(mock.drive.file('v6-doc').properties).filter(k => k.startsWith('ihn6_')).length).toBe(2);
  samples.sort((x, y) => x - y);
  test.info().annotations.push({ type: 'relay-ms', description: samples.join(',') });
  expect(samples[Math.floor(samples.length * 0.95)] ?? samples.at(-1)).toBeLessThan(4000);
  expect([...A.problems, ...B.problems, ...V.problems, ...problems]).toEqual([]);
  for (const d of [A, B, V]) await d.context.close();
});

test('(c) one device offline, both edit, back online → converge', async ({ browser, page, mock, problems }) => {
  await seedDoc(page, mock);
  const A = await device(browser, mock, ANA, { block: true }), B = await device(browser, mock, ANA, { block: true });
  await openDoc(A.page);
  await openDoc(B.page);
  const cdpA = await A.context.newCDPSession(A.page), cdpB = await B.context.newCDPSession(B.page);
  const base = (await session(A.page)).live;
  await B.context.setOffline(true);
  await B.page.evaluate(() => window.dispatchEvent(new Event('offline')));
  await stroke(A.page, cdpA, line(100, 300));
  await stroke(B.page, cdpB, line(400, 300));
  await stroke(B.page, cdpB, line(400, 340));
  await A.page.waitForTimeout(1500);
  expect((await session(A.page)).live).toBe(base + 1);
  await B.context.setOffline(false);
  await B.page.evaluate(() => window.dispatchEvent(new Event('online')));
  await same(A.page, B.page, 30_000);
  expect((await session(A.page)).live).toBe(base + 3);
  await expect.poll(async () => (await collab(B.page)).unsealed, { timeout: 20_000 }).toBe(0);
  expect([...A.problems, ...B.problems, ...problems]).toEqual([]);
  await A.context.close();
  await B.context.close();
});

test('(d) two tabs of one context: instant tab delivery, the net lock moves to tab 2 when tab 1 closes', async ({ browser, page, mock, problems }) => {
  await seedDoc(page, mock);
  const A = await device(browser, mock);
  const tab2 = await A.context.newPage();
  await openDoc(A.page);
  await openDoc(tab2);
  await expect.poll(async () => (await collab(A.page)).leader).toBe(true);
  expect((await collab(tab2)).leader).toBe(false);
  const cdp = await A.context.newCDPSession(A.page);
  const base = (await session(A.page)).live;
  await stroke(A.page, cdp, line(100, 300));
  await expect.poll(async () => (await session(tab2)).live, { timeout: 2_000, intervals: [20] }).toBe(base + 1);
  await A.page.close();
  await expect.poll(async () => (await collab(tab2)).leader, { timeout: 10_000 }).toBe(true);
  const cdp2 = await A.context.newCDPSession(tab2);
  await stroke(tab2, cdp2, line(100, 400));
  await expect.poll(async () => (await collab(tab2)).unsealed, { timeout: 20_000 }).toBe(0);
  // A fresh device sees both strokes through the relay.
  const C = await device(browser, mock);
  await openDoc(C.page, { edit: false });
  await expect.poll(async () => (await session(C.page)).live, { timeout: 20_000 }).toBe(base + 2);
  expect([...A.problems, ...C.problems, ...problems]).toEqual([]);
  await A.context.close();
  await C.context.close();
});

test('(e) a Drive-side rename reaches the open document and stays', async ({ browser, page, mock, problems }) => {
  await seedDoc(page, mock, { permissions: [{ type: 'user', role: 'writer', emailAddress: BEN }] });
  const A = await device(browser, mock, ANA, { block: true });
  await openDoc(A.page);
  await expect.poll(async () => (await collab(A.page)).phase, { timeout: 15_000 }).toMatch(/Live/);
  // B renames from Home: a metadata PATCH of the name (Drive keeps the content).
  const token = mock.drive.accounts.get(BEN).token;
  mock.drive.handle({ method: 'PATCH', url: 'https://www.googleapis.com/drive/v3/files/v6-doc?fields=id,name', headers: { authorization: `Bearer ${token}`, 'content-type': 'application/json' }, body: Buffer.from(JSON.stringify({ name: 'Renamed.pdf' })) });
  await expect.poll(async () => (await session(A.page)).name, { timeout: 30_000 }).toBe('Renamed');
  await A.page.waitForTimeout(2_000);
  expect((await session(A.page)).name).toBe('Renamed');
  expect([...A.problems, ...problems]).toEqual([]);
  await A.context.close();
});
