// Installs globalThis.InhouseRust (the production adapter plus the built WASM core) for Node tests that
// load collaboration-core-v5.js directly. Importing this module is enough; it awaits the WASM instance.
import { readFile } from 'node:fs/promises';
import vm from 'node:vm';

if (!globalThis.InhouseRust) {
    const bytes = await readFile(new URL('../../wasm/inhouse_notes_core.wasm', import.meta.url));
    const scope = { WebAssembly, Uint8Array, Uint16Array, Float32Array, TextEncoder, TextDecoder, URL, location: new URL('https://notes.test/'),
        fetch: async () => new Response(bytes), console, AbortSignal, AbortController, setTimeout, clearTimeout };
    vm.createContext(scope);
    vm.runInContext(await readFile(new URL('../../rust-core-v5.js', import.meta.url), 'utf8'), scope);
    if (!(await scope.InhouseRust.ready)) throw new Error('Rust core failed to load; run node scripts/build-rust-core.mjs');
    globalThis.InhouseRust = scope.InhouseRust;
}
export const rustReady = true;
