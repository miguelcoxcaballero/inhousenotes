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
const LIMITS = { wasm: 480 * 1024, wasmGzip: 160 * 1024, js: 100 * 1024 };

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
const wasm = await readFile(join(root, 'rust/target/wasm32-unknown-unknown/release/ihn.wasm'));

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
// IHN_SIZE_REPORT_ONLY=1 (local test builds only, never CI): the two wasm budgets are reported instead of enforced.
// Added by W3.1 because linking the document model put the binary over both budgets (docs/v6/NOTES-W3.1.md); the
// limits themselves are unchanged and the default build still fails.
const sizeError = message => { if (process.env.IHN_SIZE_REPORT_ONLY === '1') console.warn(`SIZE BUDGET EXCEEDED (reported only): ${message}`); else throw new Error(message); };
if (wasm.length > LIMITS.wasm) sizeError(`ihn.wasm is ${wasm.length} bytes (limit ${LIMITS.wasm})`);
if (gzip > LIMITS.wasmGzip) sizeError(`ihn.wasm is ${gzip} bytes gzipped (limit ${LIMITS.wasmGzip})`);

// First-party JS budget: boot, host/*, service worker (the export worker lives in host/).
const scripts = ['boot.js', 'service-worker.js', ...(await readdir(join(root, 'host'))).filter(name => name.endsWith('.js')).sort().map(name => `host/${name}`)];
let jsBytes = 0;
for (const script of scripts) jsBytes += (await readFile(join(root, script))).length;
// W2.8: the complete peer executor (5 KB) put the JS over budget too; the same report-only switch applies.
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
