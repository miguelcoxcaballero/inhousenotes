// internal-canvas (BUILD-PLAN W2.3): the command batches the Rust renderer produces for the recorded `canvas`
// scenes are replayed by the real host/canvas.js in Chromium, traced with tests/v6/parity/trace-init.js, and must
// equal the 5.18.0 traces of tests/v6/fixtures/traces — the same comparison the cargo goldens make against a
// model of the host, here against the executor that ships.
// The batches come from `IHN_CMD_DUMP=<dir> cargo test -p ihn-core render::tests`, run here unless IHN_CMD_DUMP
// names a directory that already has them. Scenes that draw images are left to cargo: an image's trace identity
// is its `src`.
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { expect, test } from '@playwright/test';
import { normalizeTrace, stringifyTrace } from '../parity/normalize.mjs';

const root = fileURLToPath(new URL('../../../', import.meta.url));
const dumps = process.env.IHN_CMD_DUMP || path.join(root, 'rust/target/canvas-cmds');
const SCENES = ['thumbnails', 'ink-pen', 'ink-highlighter', 'page-preview', 'scroll-preview', 'dense-page-erase'];

test.beforeAll(() => {
  test.setTimeout(600_000);
  if (process.env.IHN_CMD_DUMP && SCENES.every(scene => fs.existsSync(path.join(dumps, `${scene}.json`)))) return;
  execFileSync('cargo', ['test', '--manifest-path', path.join(root, 'rust/Cargo.toml'), '--locked', '-j2', '-p', 'ihn-core', 'render::tests'], { env: { ...process.env, IHN_CMD_DUMP: dumps }, stdio: 'ignore' });
});

for (const scene of SCENES) {
  test(`canvas.js replays ${scene} like 5.18.0`, async ({ page }) => {
    test.setTimeout(120_000);
    await page.addInitScript({ path: path.join(root, 'tests/v6/parity/trace-init.js') });
    await page.route('**/__canvas.html', route => route.fulfill({ contentType: 'text/html', body: '<!doctype html><body></body>' }));
    await page.goto('/__canvas.html');
    const dump = JSON.parse(fs.readFileSync(path.join(dumps, `${scene}.json`), 'utf8'));
    // One answer per encode (`toDataURL` at once, `toBlob` when the encoder is done).
    const encodes = (dump.segments.map(s => Buffer.from(s, 'base64').toString('latin1')).join('').match(/image\/jpeg/g) || []).length;
    const raw = await page.evaluate(async ({ dom, detach, segments, encodes }) => {
      const nodes = new Map(), results = [];
      for (const [handle, label, width, height] of dom) {
        const canvas = document.createElement('canvas'), match = /^canvas\.([\w-]+)\[page=(\d+)\]$/.exec(label);
        if (label[0] === '#') canvas.id = label.slice(1); else { canvas.className = match[1]; canvas.setAttribute('data-page', match[2]); }
        canvas.setAttribute('width', width); canvas.setAttribute('height', height);
        document.body.appendChild(canvas);
        nodes.set(handle, canvas);
      }
      let memory = new ArrayBuffer(0);
      const host = {
        view: () => new DataView(memory), u8: (p, n) => new Uint8Array(memory, p, n), node: h => nodes.get(h) || null,
        obj: () => null, hold: obj => (obj ? 1 : 0), io: (req, status, obj, bytes) => results.push([req, status, bytes ? bytes.length : 0])
      };
      const run = (await import('/host/canvas.js')).default(host)[2 << 8 | 1];
      self.__ihnTrace.reset();
      for (const [index, segment] of segments.entries()) {
        if (index) self.__ihnTrace.reset();
        const bytes = Uint8Array.from(atob(segment), c => c.charCodeAt(0));
        memory = new ArrayBuffer(bytes.length + 4);
        new DataView(memory).setUint32(0, bytes.length, true);
        new Uint8Array(memory, 4).set(bytes);
        run({ p: 4 });
      }
      for (const handle of detach) nodes.get(handle).remove();
      for (let waited = 0; results.length < encodes && waited < 20_000; waited += 20) await new Promise(resolve => setTimeout(resolve, 20));
      return { trace: self.__ihnTrace.take(), results };
    }, { ...dump, encodes });
    const surfaces = text => text.slice(text.indexOf('  "surfaces": ['));
    const ours = surfaces(stringifyTrace(normalizeTrace(raw.trace))).split('\n');
    const recorded = surfaces(fs.readFileSync(path.join(root, 'tests/v6/fixtures/traces', `${scene}.json`), 'utf8')).split('\n');
    const first = ours.findIndex((line, index) => line !== recorded[index]);
    expect(first < 0 ? '' : `line ${first}: ${ours[first]} | 5.18.0: ${recorded[first]}`).toBe('');
    expect(ours.length).toBe(recorded.length);
    // Every encode answered: a data URL (status 0, bytes) or a Blob.
    expect(raw.results.length).toBe(encodes);
    expect(raw.results.every(([, status, size]) => status === 0 && size > 0)).toBe(true);
  });
}
