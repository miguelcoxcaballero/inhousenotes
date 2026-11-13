// Records the exact-maths corpus of ARCHITECTURE §6.5 / TESTING §1: what Chromium's V8 returns, bit for bit, for
// Math.sin/cos/exp/round/atan2/pow/hypot and Number.prototype.toString/toFixed over fixed edge cases, grids and a seeded
// random sample. `cargo test -p ihn-core` compares `jsmath`/`jsfmt` with these files.
//
//   node tests/v6/fixtures/record-math.mjs            rewrite tests/v6/fixtures/math/*.json
//   node tests/v6/fixtures/record-math.mjs --check    fail if the committed files differ from a fresh recording
//   --out <dir> --random <n> --seed <n>               a larger corpus elsewhere (IHN_MATH_CORPUS=<dir> cargo test)
//   --engine node                                     record from this Node instead of Chromium (comparison only)
//
// The reference is Chromium, not Node: Node 22 computes sin/cos with fdlibm and pow with fdlibm's e_pow, Chromium
// with glibc's IBM routines and the C library pow, and they differ in the last bit on a few percent of inputs.
// Every run prints how many rows this Node disagrees on. Chromium comes from IHN_CHROMIUM or Playwright's default.
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const argv = process.argv.slice(2);
const flag = name => argv.includes(name);
const option = (name, fallback) => (argv.includes(name) ? argv[argv.indexOf(name) + 1] : fallback);
const outDir = option('--out', fileURLToPath(new URL('./math/', import.meta.url)));
const randomCount = Number(option('--random', 600));

// ---- inputs (integer-only PRNG and bit patterns: identical on every engine and platform) ----
const view = new DataView(new ArrayBuffer(8));
const toHex = x => { view.setFloat64(0, x); return view.getBigUint64(0).toString(16).padStart(16, '0'); };
const fromHex = h => { view.setBigUint64(0, BigInt('0x' + h)); return view.getFloat64(0); };
let seed = Number(option('--seed', 0x1f123bb5)) >>> 0 || 1;
const u32 = () => { seed ^= seed << 13; seed >>>= 0; seed ^= seed >>> 17; seed ^= seed << 5; seed >>>= 0; return seed; };
const unit = () => (u32() * 2097152 + (u32() >>> 11)) / 2 ** 53; // 53 random bits, exact
const anyBits = () => { view.setUint32(0, u32()); view.setUint32(4, u32()); return view.getFloat64(0); };
const range = (lo, hi) => lo + (hi - lo) * unit();
const near = x => { if (x === 0) return [x]; const b = BigInt('0x' + toHex(x)); return [x, fromHex((b + 1n).toString(16).padStart(16, '0')), fromHex((b - 1n).toString(16).padStart(16, '0'))]; };
const signed = list => list.flatMap(x => [x, -x]);
const times = (n, f) => Array.from({ length: n }, (_, i) => f(i));

const SPECIAL = [0, -0, NaN, Infinity, -Infinity, ...signed([1, 0.5, 1.5, 2, 2.5, 3, 10, 0.1, 0.2, 0.3, 1 / 3, Number.MIN_VALUE,
  2.2250738585072014e-308, 2.225073858507201e-308, 1e-320, Number.EPSILON, Number.MAX_VALUE, Number.MAX_SAFE_INTEGER, 2 ** 53,
  2 ** 52 + 0.5, 2 ** 31, 2 ** 32, 2 ** 63, 2 ** 64, 2 ** -26, 2 ** -27, 2 ** -28, 2 ** -54, 2 ** -65, 1e-7, 1e-6, 1e21, 1e300, 1e-300])];
const PI_MULTIPLES = signed(times(81, i => i * Math.PI / 4).flatMap(near));
// |x| thresholds of the algorithms (0.126, 0.4375, 0.855469, 2.426265, 105414350, ln2 multiples, exp over/underflow).
const THRESHOLDS = signed([0.126, 0.4375, 0.6875, 1.1875, 2.4375, 0.855469, 2.426265, 105414350, 105414357.85, 2 ** 66, 0.34657359027997264,
  1.0397207708399179, 709.782712893384, 745.1332191019411, 708.3964185322641, 1e22, 1e100, 1e308].flatMap(near));
const ONE_ARG = n => [...SPECIAL, ...PI_MULTIPLES, ...THRESHOLDS,
  ...times(721, i => (i - 360) * Math.PI / 180),            // every degree, as 5.18.0 computes radians
  ...times(400, i => (i - 200) / 16),                        // exact binary fractions
  ...times(n, () => range(-7, 7)), ...times(n >> 1, () => range(-1000, 1000)), ...times(n >> 2, () => range(-1e9, 1e9)),
  ...times(n >> 2, () => 2 ** range(-60, 60) * (u32() & 1 ? 1 : -1)), ...times(n >> 1, anyBits)];
const PIXELS = () => Math.round(range(-3000, 3000) * 4) / 4;
const PAIRS = {
  atan2: n => [...SPECIAL.flatMap(a => SPECIAL.slice(0, 16).flatMap(b => [[a, b], [b, a]])),
    ...times(n, () => [PIXELS(), PIXELS()]), ...times(n, () => [range(-1, 1), range(-1, 1)]),
    ...times(n >> 1, () => [2 ** range(-70, 70), -(2 ** range(-70, 70))]), ...times(n >> 1, () => [anyBits(), anyBits()])],
  hypot: n => [...SPECIAL.flatMap(a => SPECIAL.slice(0, 16).flatMap(b => [[a, b], [b, a]])),
    ...times(n, () => [PIXELS(), PIXELS()]), ...times(n, () => [range(-50, 50), range(-50, 50)]),
    ...times(n >> 1, () => [2 ** range(-500, 500), 2 ** range(-500, 500)]), ...times(n >> 1, () => [anyBits(), anyBits()])],
  pow: n => [...SPECIAL.flatMap(a => [...SPECIAL.slice(0, 20), 4, -4, 5, -5, 0.25, 1e300, 2 ** -66].flatMap(b => [[a, b], [b, a]])),
    ...times(n, () => [1.0015, range(-1200, 1200)]), ...times(n >> 1, () => [1.0015, -Math.round(range(-600, 600))]), // wheel zoom
    ...times(n >> 1, () => [2, range(-20, 20)]), ...times(n >> 1, () => [range(0, 1), 2.2]), ...times(n >> 2, () => [range(0, 1), 2.4]),
    ...times(n >> 2, () => [range(0, 1), 1 / 2.4]), ...times(n, () => [range(0, 40), range(-12, 12)]),
    ...times(n >> 2, () => [Math.round(range(-30, 30)), Math.round(range(-25, 25))]), ...times(n >> 2, () => [10, Math.round(range(-330, 330))]),
    ...times(n >> 1, () => [2 ** range(-1074, 1024), range(-2, 2)]), ...times(n >> 2, () => [range(0.99, 1.01), range(-1e5, 1e5)]),
    ...times(n >> 2, () => [range(0.5, 2), range(-1100, 1100)]), ...times(n >> 2, () => [-range(0, 40), Math.round(range(-12, 12))]),
    ...times(n >> 1, () => [anyBits(), anyBits()])]
};
const TO_STRING = n => [...SPECIAL, ...THRESHOLDS, ...times(64, i => 10 ** (i - 32)), ...times(160, i => 2 ** (i - 100)), ...times(48, i => 1.5 * 10 ** (i - 12)),
  ...[1e21, 1e-7, 1e-6, 123456789012345680000, 0.000001234, 999999999999999900000].flatMap(near),
  ...times(n, () => Math.round(range(-5000, 5000) * 100) / 100), ...times(n, () => range(-2000, 2000)), ...times(n, () => range(0, 1)),
  ...times(n >> 1, () => Math.round(range(-1e6, 1e6))), ...times(n >> 1, () => range(0, 100) / 100 * 360), ...times(n >> 1, () => range(-1, 1) * 1e-5),
  ...times(n >> 1, () => 2 ** range(-80, 80)), ...times(n >> 1, () => Math.round(range(0, 1e21))), ...times(n, anyBits),
  // Exactly half-way between two shortest candidates: the spec picks the even digit.
  ...times(n >> 1, () => Math.floor(range(2 ** 50, 2 ** 53)) + (u32() & 1 ? 0.25 : 0.75)),
  ...times(n >> 2, () => Math.floor(range(2 ** 47, 2 ** 50)) + (2 * (u32() & 3) + 1) / 8),
  ...times(n >> 2, () => (Math.floor(range(2 ** 50, 2 ** 53)) + (u32() & 1 ? 0.25 : 0.75)) / 2 ** (1 + u32() % 40))];
const TO_FIXED = n => [...SPECIAL.flatMap(x => [0, 1, 2, 20].map(d => [x, d])), ...[1e21, 1e-7, 0.000001234, 1.005, 1.45, 8.345, 0.5, 1.5, 2.5, 1e20, 123.456]
  .flatMap(near).flatMap(x => times(8, d => [[x, d], [-x, d]]).flat()),
  ...times(n, () => [Math.round(range(-4000, 4000) * 8) / 8, u32() % 4]),         // exact ties: JS rounds half away from zero
  ...times(n >> 1, () => [Math.round(range(-64, 64) * 1024) / 1024, u32() % 11]),
  ...times(n, () => [Math.round(range(-5000, 5000) * 1000) / 1000, u32() % 3]),   // decimal "ties" that are not exact
  ...times(n, () => [range(-1000, 1000), u32() % 5]), ...times(n >> 1, () => [range(0, 100), 1]), ...times(n >> 1, () => [range(0, 1), u32() % 8]),
  ...times(n >> 2, () => [2 ** range(-40, 72), u32() % 25]), ...times(n >> 2, () => [anyBits(), u32() % 101])].map(([x, d]) => [x, String(d)]);

const n = randomCount;
const sets = { sin: ONE_ARG(n), cos: ONE_ARG(n), exp: [...ONE_ARG(n >> 1), ...times(n, () => range(-745, 710)), ...times(n >> 1, () => range(-1, 1))],
  round: [...ONE_ARG(n >> 2), ...times(n, () => Math.round(range(-1e4, 1e4)) + 0.5), ...times(n, () => range(-1e4, 1e4)),
    ...signed([0.49999999999999994, 0.5, 4503599627370495.5, 4503599627370496, 4503599627370497, 9007199254740991])],
  atan2: PAIRS.atan2(n), pow: PAIRS.pow(n), hypot: PAIRS.hypot(n), tostring: TO_STRING(n), tofixed: TO_FIXED(n) };
const inputs = Object.fromEntries(Object.entries(sets).map(([name, list]) => [name, list.map(row => [].concat(row).map(v => (typeof v === 'string' ? v : toHex(v))))]));

// ---- evaluation (runs unchanged in Node and in the page) ----
function evaluate(inputs) {
  const view = new DataView(new ArrayBuffer(8));
  const toHex = x => { view.setFloat64(0, x); return view.getBigUint64(0).toString(16).padStart(16, '0'); };
  const fromHex = h => { view.setBigUint64(0, BigInt('0x' + h)); return view.getFloat64(0); };
  const out = {};
  for (const name of Object.keys(inputs)) {
    out[name] = inputs[name].map(row => {
      const x = fromHex(row[0]);
      if (name === 'tostring') return `${row[0]} ${String(x)}`;
      if (name === 'tofixed') return `${row[0]} ${row[1]} ${x.toFixed(Number(row[1]))}`;
      const value = Math[name](...row.map(fromHex));
      return [...row, Number.isNaN(value) ? '7ff8000000000000' : toHex(value)].join(' ');
    });
  }
  return out;
}

async function inChromium() {
  const { chromium } = await import('playwright');
  const browser = await chromium.launch(process.env.IHN_CHROMIUM ? { executablePath: process.env.IHN_CHROMIUM } : {});
  try {
    const page = await browser.newPage();
    return { rows: await page.evaluate(evaluate, inputs), version: browser.version() };
  } finally {
    await browser.close();
  }
}

const node = evaluate(inputs);
const recorded = option('--engine', 'chromium') === 'node' ? { rows: node, version: `node ${process.version}` } : await inChromium();
let stale = 0;
if (!flag('--check')) mkdirSync(outDir, { recursive: true });
for (const [name, rows] of Object.entries(recorded.rows)) {
  const unique = [...new Set(rows)];
  const text = `{"fn":"${name}","rows":[\n${unique.map(row => JSON.stringify(row)).join(',\n')}\n]}\n`;
  const file = join(outDir, `${name}.json`);
  const differs = rows.filter((row, i) => row !== node[name][i]).length;
  if (flag('--check')) {
    let current = '';
    try { current = readFileSync(file, 'utf8'); } catch {}
    if (current !== text) { stale++; console.error(`stale: ${file}`); }
  } else {
    writeFileSync(file, text);
  }
  console.log(`${name.padEnd(8)} ${String(unique.length).padStart(6)} rows; this Node differs on ${differs}`);
}
console.log(`recorded from ${recorded.version}${flag('--check') ? (stale ? ' — FIXTURES ARE STALE' : ' — fixtures are current') : ` into ${outDir}`}`);
process.exit(stale ? 1 : 0);
