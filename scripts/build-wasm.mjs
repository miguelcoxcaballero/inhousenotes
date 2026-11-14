// `npm run build`: the reproducible v6 build (ARCHITECTURE §14.1). Compiles rust/ to wasm, checks the ABI, the
// import allowlist, the exact-maths gate and the size budgets, and writes the generated files to build/:
//   build/wasm/ihn.wasm, build/offline-assets.js, build/native-assets.json, build/index.html
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
// W6.2s (docs/v6/NOTES-W6.2s.md): the design budget (480 KB / 160 KB gzip / 100 KB JS) is not reachable without
// dropping features; these are the measured sizes after the size work (1,646,831 / 723,729 / 120,398 bytes) plus
// ~5 % headroom for the parity fixes still landing. Any growth beyond that fails the build.
const LIMITS = { wasm: 1700 * 1024, wasmGzip: 740 * 1024, js: 124 * 1024 };

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
run(process.env.CARGO || 'cargo', ['build', '--manifest-path', 'rust/Cargo.toml', '-p', 'ihn-wasm', '--release',
  '--target', 'wasm32-unknown-unknown', '--locked', ...(process.env.IHN_CARGO_JOBS ? ['-j', process.env.IHN_CARGO_JOBS] : [])],
  { RUSTFLAGS: `--remap-path-prefix=${cargoHome}=/cargo --remap-path-prefix=${root}=.` });
const linked = join(root, 'rust/target/wasm32-unknown-unknown/release/ihn.wasm');
// W6.2s: binaryen's wasm-opt -Os (npm devDependency, pinned) removes what LLVM leaves behind: -12 % raw, which is
// also compile time. Without node_modules/binaryen (a checkout that shares an older node_modules) the unoptimised
// binary is used and the sizes are only reported; CI (`npm ci`) always has it.
// Exactly the features rustc 1.90 enables for wasm32-unknown-unknown (its `target_features` section, which `strip`
// removes), so wasm-opt validates the input and never emits anything newer.
const WASM_FEATURES = ['bulk-memory', 'bulk-memory-opt', 'call-indirect-overlong', 'multivalue', 'mutable-globals', 'nontrapping-float-to-int', 'reference-types', 'sign-ext'];
const wasmOpt = join(root, 'node_modules/binaryen/bin/wasm-opt');
const optimised = existsSync(wasmOpt);
if (optimised) run(process.execPath, [wasmOpt, '-Os', ...WASM_FEATURES.map(f => `--enable-${f}`), linked, '-o', `${linked}.opt`]);
else if (process.env.CI) throw new Error('node_modules/binaryen is missing: run npm ci');
else console.warn('node_modules/binaryen is missing: wasm-opt skipped, size budgets reported only');
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
node('build-offline-assets.mjs', '--out', 'build');
node('build-native-assets.mjs', '--out', 'build');

console.log(`ihn.wasm: ${wasm.length} bytes (${gzip} gzip -9), first-party JS: ${jsBytes} bytes in ${scripts.length} files`);
for (const file of ['wasm/ihn.wasm', 'offline-assets.js', 'native-assets.json', 'index.html']) {
  if (!existsSync(join(out, file))) throw new Error(`build/${file} was not written`);
  console.log(`${sha256(await readFile(join(out, file)))}  build/${file}`);
}
