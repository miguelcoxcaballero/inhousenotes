import { spawnSync } from 'node:child_process';
import { readFile, writeFile, mkdir } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { homedir } from 'node:os';
const root = fileURLToPath(new URL('../', import.meta.url));
// Registry sources are absolute paths that differ per machine and would leak
// into panic locations, making the committed binary irreproducible in CI.
const cargoHome = process.env.CARGO_HOME || `${homedir()}/.cargo`;
const remap = [`--remap-path-prefix=${cargoHome}=/cargo`, `--remap-path-prefix=${root.replace(/\/$/, '')}=.`].join(' ');
const env = { ...process.env, RUSTFLAGS: [process.env.RUSTFLAGS, remap].filter(Boolean).join(' ') };
const result = spawnSync(process.env.CARGO || 'cargo', ['build', '--manifest-path', 'rust-core/Cargo.toml', '--release', '--target', 'wasm32-unknown-unknown', '--locked'], { cwd: root, env, stdio: 'inherit' });
if (result.error) throw result.error;
if (result.status) process.exit(result.status);
const bytes = await readFile(new URL('../rust-core/target/wasm32-unknown-unknown/release/inhouse_notes_core.wasm', import.meta.url));
const module = await WebAssembly.compile(bytes);
if (WebAssembly.Module.imports(module).length) throw new Error('Unexpected host imports');
const { exports } = await WebAssembly.instantiate(module);
if (exports.core_abi_version() !== 1) throw new Error('ABI mismatch');
await mkdir(new URL('../wasm/', import.meta.url), { recursive: true });
await writeFile(new URL('../wasm/inhouse_notes_core.wasm', import.meta.url), bytes);
console.log(`Rust pixel core: ${bytes.length} bytes`);
