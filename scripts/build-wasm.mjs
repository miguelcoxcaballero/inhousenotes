// `npm run build`: the reproducible v6 build (ARCHITECTURE §14.1). Compiles rust/ to wasm, checks the ABI, the
// import allowlist, the exact-maths gate and the size budgets, and writes the generated files to build/:
//   build/wasm/ihn.wasm, build/wasm/ihn-test.wasm (e2e), build/offline-assets.js, build/native-assets.json, build/index.html
// Nothing generated is committed on the v6 branch; `tests/v6/server.mjs --overlay build` serves build/ over the tree.
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync } from 'node:fs';
import { mkdir, readdir, readFile, rm, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { gzipSync } from 'node:zlib';

const root = fileURLToPath(new URL('../', import.meta.url)).replace(/[\\/]$/, '');
const out = join(root, 'build');
const IMPORTS = ['host_flush', 'host_rect', 'host_metric', 'host_now', 'host_time', 'host_random', 'host_ls', 'host_intl', 'host_log'];
const EXPORTS = ['memory', 'ihn_abi', 'ihn_alloc', 'ihn_free', 'ihn_init', 'ihn_event', 'ihn_frame', 'ihn_out_ptr', 'ihn_out_len', 'ihn_out_reset', 'ihn_worker'];
// W6.2s/W6.2t/W6.2z/W6.2y (docs/v6/NOTES-W6.2s.md, NOTES-W6.2t.md, NOTES-W6.2z.md, NOTES-W6.2y.md): the design
// budget (480 KB / 160 KB gzip / 100 KB JS) is not reachable without dropping features or splitting the module;
// these are the measured production sizes after W6.2y (1,337,634 / 596,635 bytes) plus 5 %, rounded up to KiB. JS
// keeps the W6.2t limit (121,245 + 5 %; it is 124,718 now). Any growth beyond that fails the build. ihn-test.wasm
// is not budgeted.
const LIMITS = { wasm: 1372 * 1024, wasmGzip: 612 * 1024, js: 125 * 1024 };

function run(command, args, env = {}) {
  const result = spawnSync(command, args, { cwd: root, stdio: 'inherit', env: { ...process.env, ...env } });
  if (result.error) throw result.error;
  if (result.status) process.exit(result.status);
}
const node = (script, ...args) => run(process.execPath, [join('scripts', script), ...args]);
const sha256 = bytes => createHash('sha256').update(bytes).digest('hex');

node('gen-ui-ids.mjs', '--check');
node('check-math.mjs');

// Paths never reach the binary: the workspace has no dependencies, and both prefixes are remapped anyway.
const cargoHome = process.env.CARGO_HOME || join(process.env.HOME || process.env.USERPROFILE || '', '.cargo');
const linked = join(root, 'rust/target/wasm32-unknown-unknown/release/ihn.wasm');
// W6.2s: binaryen's wasm-opt -Os (npm devDependency, pinned) removes what LLVM leaves behind: -12 % raw, which is
// also compile time. Without node_modules/binaryen (a checkout that shares an older node_modules) the unoptimised
// binary is used and the sizes are only reported; CI (`npm ci`) always has it.
// Exactly the features rustc 1.90 enables for wasm32-unknown-unknown (its `target_features` section, which `strip`
// removes), so wasm-opt validates the input and never emits anything newer.
// W6.2z: `--low-memory-unused` lets wasm-opt fold small constant address offsets into the load/store `offset`
// immediates. Its assumption (nothing reads or writes the first 1 KiB of memory) holds for rustc's wasm32 layout: the
// stack comes first and grows down from 1 MiB, statics start at 1 MiB, the heap after them, so addresses below 1024
// are only touched by a stack within 1 KiB of overflowing (measured: -37 KB raw, -9 KB gzip). Then
// `--merge-similar-functions` (functions that differ only in constants share one body) and a second `-Os` round
// over the result (-7 KB raw, -3 KB gzip; `--converge` would give -0.1 KB more for 40 s per variant).
// W6.2y: `--reorder-functions-by-name` then sorts the function bodies by their (mangled) symbol name, which puts the
// functions of one module next to each other, so gzip finds their repeated instruction sequences inside its 32 KiB
// window (measured: +6.6 KB raw from longer call indices, -12.3 KB gzip). Only the order of the bodies and the
// indices that refer to them change; exports, imports and the table keep their meaning. The names come from the
// linker's name section, kept for this step (`CARGO_PROFILE_RELEASE_STRIP=debuginfo` below) and removed with the
// producers and target-features sections afterwards, so the output has exactly the sections it had before.
// W6.2y: `-ocimfs 20` (`--one-caller-inline-max-function-size`, an option of the inlining passes inside both `-Os`
// rounds): binaryen inlines a function that has a single caller only up to 20 instructions instead of at any size.
// LLVM already inlined what it found worth it; the large single-caller bodies binaryen inlined on top grew the code
// and moved it away from its own module's functions (measured on the name-ordered output: -6.0 KB raw, -5.4 KB
// gzip; 10-30 all give the same, 50 and above lose most of it). Tiny bodies are still inlined.
// `-aimfs 4` (`--always-inline-max-function-size`, default 2): bodies of up to 4 instructions (field getters, small
// wrappers) are inlined at every call site; the call sequence they replace is about as long (measured: -0.4 KB raw,
// -0.4 KB gzip; 5 and above grow the code).
const WASM_OPT = ['-ocimfs', '20', '-aimfs', '4', '-Os', '--low-memory-unused', '--merge-similar-functions', '-Os', '--reorder-functions-by-name', '--strip-debug', '--strip-producers', '--strip-target-features'];
const WASM_FEATURES = ['bulk-memory', 'bulk-memory-opt', 'call-indirect-overlong', 'multivalue', 'mutable-globals', 'nontrapping-float-to-int', 'reference-types', 'sign-ext'];
const wasmOpt = join(root, 'node_modules/binaryen/bin/wasm-opt');
const optimised = existsSync(wasmOpt);
if (!optimised && process.env.CI) throw new Error('node_modules/binaryen is missing: run npm ci');
if (!optimised) console.warn('node_modules/binaryen is missing: wasm-opt skipped, size budgets reported only');

// Builds, optimises and checks one variant. W6.2t: two variants from the same sources:
// - `wasm/ihn.wasm` (production): no test API;
// - `wasm/ihn-test.wasm` (`--features testapi`): the same crates plus `testapi::dispatch` and every module's
//   `test_cmd` in the module table. host/main.js loads it only with `?e2e=1` on localhost (e2e, parity, perf).
async function build(features) {
  run(process.env.CARGO || 'cargo', ['build', '--manifest-path', 'rust/Cargo.toml', '-p', 'ihn-wasm', '--release',
    '--target', 'wasm32-unknown-unknown', '--locked', ...features, ...(process.env.IHN_CARGO_JOBS ? ['-j', process.env.IHN_CARGO_JOBS] : [])],
    // W6.2y: the symbol names stay in the linked binary for wasm-opt's `--reorder-functions-by-name` (see WASM_OPT).
    { RUSTFLAGS: `--remap-path-prefix=${cargoHome}=/cargo --remap-path-prefix=${root}=.`, CARGO_PROFILE_RELEASE_STRIP: optimised ? 'debuginfo' : 'true' });
  if (optimised) run(process.execPath, [wasmOpt, ...WASM_OPT, ...WASM_FEATURES.map(f => `--enable-${f}`), linked, '-o', `${linked}.opt`]);
  const wasm = await readFile(optimised ? `${linked}.opt` : linked);

  const module = await WebAssembly.compile(wasm);
  const imports = WebAssembly.Module.imports(module).map(entry => `${entry.module}.${entry.name}:${entry.kind}`).sort();
  const allowed = IMPORTS.map(name => `env.${name}:function`).sort();
  if (imports.join() !== allowed.join()) {
    throw new Error(`Wasm imports differ from the ABI allowlist:\n  found   ${imports.join(' ')}\n  allowed ${allowed.join(' ')}`);
  }
  const exported = WebAssembly.Module.exports(module).map(entry => entry.name);
  const missing = EXPORTS.filter(name => !exported.includes(name));
  if (missing.length) throw new Error(`Wasm exports missing: ${missing.join(', ')}`);
  const stubs = Object.fromEntries(IMPORTS.map(name => [name, () => 0]));
  const { exports } = await WebAssembly.instantiate(module, { env: stubs });
  if (exports.ihn_abi() !== 6) throw new Error(`ABI mismatch: ihn_abi() = ${exports.ihn_abi()}`);
  return wasm;
}
const testWasm = await build(['--features', 'testapi']);
const wasm = await build([]);

const gzip = gzipSync(wasm, { level: 9 }).length;
// IHN_SIZE_REPORT_ONLY=1 (local test builds only, never CI): the budgets are reported instead of enforced (W3.1);
// the same applies when wasm-opt was skipped.
const sizeError = message => { if (process.env.IHN_SIZE_REPORT_ONLY === '1' || !optimised) console.warn(`SIZE BUDGET EXCEEDED (reported only): ${message}`); else throw new Error(message); };
if (wasm.length > LIMITS.wasm) sizeError(`ihn.wasm is ${wasm.length} bytes (limit ${LIMITS.wasm})`);
if (gzip > LIMITS.wasmGzip) sizeError(`ihn.wasm is ${gzip} bytes gzipped (limit ${LIMITS.wasmGzip})`);

// First-party JS budget: boot, host/*, service worker (the export worker lives in host/).
const scripts = ['boot.js', 'service-worker.js', ...(await readdir(join(root, 'host'))).filter(name => name.endsWith('.js')).sort().map(name => `host/${name}`)];
let jsBytes = 0;
for (const script of scripts) jsBytes += (await readFile(join(root, script))).length;
if (jsBytes > LIMITS.js) sizeError(`First-party JS is ${jsBytes} bytes (limit ${LIMITS.js})`);

await rm(out, { recursive: true, force: true });
await mkdir(join(out, 'wasm'), { recursive: true });
await writeFile(join(out, 'wasm/ihn.wasm'), wasm);
await writeFile(join(out, 'wasm/ihn-test.wasm'), testWasm);
node('build-offline-assets.mjs', '--out', 'build');
node('build-native-assets.mjs', '--out', 'build');

console.log(`ihn.wasm: ${wasm.length} bytes (${gzip} gzip -9), first-party JS: ${jsBytes} bytes in ${scripts.length} files`);
console.log(`ihn-test.wasm (e2e only, not budgeted): ${testWasm.length} bytes (${gzipSync(testWasm, { level: 9 }).length} gzip -9)`);
for (const file of ['wasm/ihn.wasm', 'wasm/ihn-test.wasm', 'offline-assets.js', 'native-assets.json', 'index.html']) {
  if (!existsSync(join(out, file))) throw new Error(`build/${file} was not written`);
  console.log(`${sha256(await readFile(join(out, file)))}  build/${file}`);
}
