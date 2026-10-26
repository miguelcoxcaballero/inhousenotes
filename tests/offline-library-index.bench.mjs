// Micro-benchmark: the pre-port library index (loaded from git) vs the Rust-backed one, on realistic Drive sizes.
// Run: node tests/offline-library-index.bench.mjs
import vm from 'node:vm';
import { readFileSync } from 'node:fs';
import { legacySource, rng } from './helpers/legacy.mjs';

const currentSource = readFileSync(new URL('../offline-library-v5.js', import.meta.url), 'utf8');
const rustSource = readFileSync(new URL('../rust-core-v5.js', import.meta.url), 'utf8');
const wasm = readFileSync(new URL('../wasm/inhouse_notes_core.wasm', import.meta.url));
const folderType = 'application/vnd.google-apps.folder';

async function load(source, rust) {
    const state = { entries: [], docs: [], views: {}, driveRoot: null };
    const store = { async list(kind) { return kind === 'entries' ? state.entries : kind === 'document-index' ? state.docs : []; }, async read() { return null; } };
    const sandbox = { URLSearchParams, location: { search: '?offline-worker=1', pathname: '/', href: 'https://notes.test/' }, navigator: { onLine: false },
        WebAssembly, URL, TextEncoder, TextDecoder, AbortSignal, fetch: async () => new Response(wasm),
        localStorage: { getItem: () => 'bench@example.test' }, sessionStorage: { getItem: () => null }, document: { querySelectorAll: () => [] },
        InhouseOfflineStore: store, InhouseRuntimeCore: { CoalescingQueue: class {} } };
    sandbox.window = sandbox; vm.createContext(sandbox);
    if (rust) { vm.runInContext(rustSource, sandbox); await sandbox.InhouseRust.ready; }
    vm.runInContext(source, sandbox);
    await sandbox.InhouseOfflineLibrary.init({ quickAccess: entry => entry.id.endsWith('7') });
    return { api: sandbox.InhouseOfflineLibrary, state };
}
const next = rng(42), pick = list => list[Math.floor(next() * list.length)];
const words = ['Invoice', 'Notes', 'Scan', 'Meeting', 'Ärzte', 'résumé', 'Zebra', 'alpha', 'Project', 'Taxes', '2024', 'Receipt', 'Lecture', 'Draft', 'Über'];
const name = i => `${pick(words)} ${pick(words)} ${i}`;
function catalogue(total, directories) {
    const folders = Array.from({ length: directories }, (_, i) => ({ id: `d${i}`, name: name(i), mimeType: folderType, parents: [i < 20 ? 'root' : `d${i % 20}`], modifiedTime: '2026-01-01T00:00:00Z' }));
    const files = Array.from({ length: total - directories }, (_, i) => ({ id: `f${i}`, name: `${name(i)}.pdf`, mimeType: 'application/pdf',
        parents: [i % 3 === 0 ? 'd0' : `d${i % directories}`], modifiedTime: new Date(1.7e12 + next() * 9e10).toISOString(), starred: i % 50 === 0, ownedByMe: i % 40 !== 0 }));
    return [...folders, ...files];
}
// best of N: other processes share the machine, the minimum is the least disturbed run
function time(fn, runs = 15) { fn(); let best = Infinity; for (let i = 0; i < runs; i++) { const s = performance.now(); fn(); best = Math.min(best, performance.now() - s); } return best; }
async function timeAsync(fn, runs = 15) { await fn(); let best = Infinity; for (let i = 0; i < runs; i++) { const s = performance.now(); await fn(); best = Math.min(best, performance.now() - s); } return best; }

const [legacy, current] = [await load(legacySource('offline-library-v5.js'), false), await load(currentSource, true)];
const rows = [];
for (const [total, directories] of [[2000, 100], [10000, 400], [40000, 1200]]) {
    const entries = catalogue(total, directories), docs = entries.filter((_, i) => i % 9 === 0).map((e, i) => ({ id: e.id, lastOpenedAt: 1e12 + i * 1000 }));
    for (const side of [legacy, current]) { side.state.entries = entries.map(e => ({ ...e })); side.state.docs = docs; }
    const label = `${total} entries`;
    rows.push([`reload (merge+index) ${label}`, await timeAsync(() => legacy.api.reload()), await timeAsync(() => current.api.reload())]);
    // d0 holds a third of the files, plus subfolders: the largest directory a user would open
    const remote = entries.filter(e => (e.parents || []).includes('d0')).slice(0, 1000).map(e => ({ ...e }));
    rows.push([`folder('d0') ${label}`, time(() => legacy.api.folder('d0')), time(() => current.api.folder('d0'))]);
    rows.push([`folder('root') ${label}`, time(() => legacy.api.folder('root')), time(() => current.api.folder('root'))]);
    rows.push([`folder('d0','inv') ${label}`, time(() => legacy.api.folder('d0', 'inv')), time(() => current.api.folder('d0', 'inv'))]);
    rows.push([`combine 1000 remote ${label}`, time(() => legacy.api.combine(remote, 'd0')), time(() => current.api.combine(remote, 'd0'))]);
    rows.push([`home() ${label}`, time(() => legacy.api.home()), time(() => current.api.home())]);
    const recents = legacy.api.home().recents.map(e => ({ ...e }));
    rows.push([`recentItems(12) ${label}`, time(() => legacy.api.recentItems(recents)), time(() => current.api.recentItems(recents))]);
}
console.log('operation'.padEnd(38), 'JS ms'.padStart(9), 'Rust ms'.padStart(9), 'speedup'.padStart(8));
for (const [label, js, rust] of rows) console.log(label.padEnd(38), js.toFixed(2).padStart(9), rust.toFixed(2).padStart(9), `${(js / rust).toFixed(1)}x`.padStart(8));
