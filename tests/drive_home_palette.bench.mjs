// Micro-benchmark: original JS palette maths vs the Rust batch call. Run: node tests/drive_home_palette.bench.mjs
import { readFile } from 'node:fs/promises';
import vm from 'node:vm';
import { legacySource, rng } from './helpers/legacy.mjs';
const bytes = await readFile(new URL('../wasm/inhouse_notes_core.wasm', import.meta.url));
const scope = { WebAssembly, Uint8Array, URL, location: new URL('https://notes.test/'), fetch: async () => new Response(bytes), console, AbortSignal, setTimeout, clearTimeout, TextEncoder, TextDecoder };
vm.createContext(scope);
vm.runInContext(await readFile(new URL('../rust-core-v5.js', import.meta.url), 'utf8'), scope);
await scope.InhouseRust.ready;
const rust = scope.InhouseRust;
const source = legacySource('app-v5.js');
const theme = { value: 'light' };
const legacy = vm.runInNewContext(`(function(){const DRIVE_DEFAULT_FOLDER_COLOR='#e3c58e';const document={documentElement:{dataset:{get theme(){return theme.value;}}}};
${source.slice(source.indexOf('function resolveDriveFolderColor(item)'), source.indexOf('function createDriveFolderArt('))}
return {applyDriveFolderPalette,getDriveFolderViewTint};})()`, { theme, Math });
const random = rng(7);
const hex = () => '#' + Math.floor(random() * 0x1000000).toString(16).padStart(6, '0');
const el = { style: { setProperty() {} }, dataset: {} };
const time = (fn, reps) => { for (let i = 0; i < Math.ceil(reps / 5); i++) fn(); const t = process.hrtime.bigint(); for (let i = 0; i < reps; i++) fn(); return Number(process.hrtime.bigint() - t) / 1e6 / reps; };
for (const [label, count, distinct] of [['theme repaint, 40 folders', 40, 24], ['theme repaint, 300 folders', 300, 24], ['300 folders, all distinct colours', 300, 300]]) {
    const pool = Array.from({ length: distinct }, hex), colours = Array.from({ length: count }, (_, i) => pool[i % distinct]);
    const js = time(() => { theme.value = 'dark'; for (const c of colours) legacy.applyDriveFolderPalette(el, c); }, 300);
    let n = 0;
    // cold: every call sees colours it has not cached (fresh random pool each run)
    const cold = time(() => { const fresh = colours.map(() => hex()); rust.driveFolderPalettes(fresh, true); n++; }, 300);
    const jsFresh = time(() => { const fresh = colours.map(() => hex()); theme.value = 'dark'; for (const c of fresh) legacy.applyDriveFolderPalette(el, c); }, 300);
    const warm = time(() => rust.driveFolderPalettes(colours, true), 2000);
    console.log(`${label}: JS ${js.toFixed(4)} ms | Rust cached ${warm.toFixed(4)} ms (${(js / warm).toFixed(1)}x) | all-cold: JS ${jsFresh.toFixed(4)} ms vs Rust batch ${cold.toFixed(4)} ms (${(jsFresh / cold).toFixed(2)}x)`);
}
const one = hex();
console.log('single card, uncached colour: JS', time(() => legacy.applyDriveFolderPalette(el, hex()), 20000).toFixed(5), 'ms | Rust', time(() => rust.driveFolderPalettes([hex()], false), 20000).toFixed(5), 'ms');
console.log('view tint: JS', time(() => legacy.getDriveFolderViewTint(hex()), 20000).toFixed(5), 'ms | Rust', time(() => rust.driveFolderViewTint(hex(), false), 20000).toFixed(5), 'ms');
