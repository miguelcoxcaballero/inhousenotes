// Splits ihn.wasm in two (6.2): the functions the app runs to boot and show Home stay in the first module; every
// other function moves to a second module that host/main.js compiles in the background right after boot.
//
// How it works (binaryen.js, already a build dependency; no other tool):
// - In the first module every moved function keeps its index and signature but its body becomes a stub: if the
//   table slot reserved for it is still empty it calls `env.host_split()` (which instantiates the second module,
//   synchronously if needed: see host/main.js), then `call_indirect`s through that slot. Table entries that pointed
//   to a moved function (Rust function pointers) now point to its stub, so they keep working.
// - The second module imports the first one's memory, table, globals and the functions it calls (export names
//   `t`, `g<i>`, `f<base36>`), has no data segments (the memory is already initialised) and fills the reserved slots
//   with an active element segment when it is instantiated.
// So a moved function can never run before the second module exists: correctness does not depend on the profile,
// only how early the second module is needed does.
//
//   import { split, instrument } from './split-wasm.mjs'
//   split(bytes, keepNames)  → { first, second, moved, kept }
//   instrument(bytes)        → { bytes, names }  (each function calls env.host_mark(index) on entry: profiling)
import binaryen from 'binaryen';

// Exactly the features build-wasm.mjs enables for wasm-opt (WASM_FEATURES).
const F = binaryen.Features;
const FEATURES = F.BulkMemory | F.BulkMemoryOpt | F.CallIndirectOverlong | F.Multivalue | F.MutableGlobals
  | F.NontrappingFPToInt | F.ReferenceTypes | F.SignExt;

function read(bytes) {
  const module = binaryen.readBinary(bytes);
  module.setFeatures(FEATURES);
  return module;
}

function functions(module) {
  const list = [];
  for (let i = 0; i < module.getNumFunctions(); i++) list.push(binaryen.getFunctionInfo(module.getFunctionByIndex(i)));
  return list;
}

function exportsOf(module) {
  const list = [];
  for (let i = 0; i < module.getNumExports(); i++) list.push(binaryen.getExportInfo(module.getExportByIndex(i)));
  return list;
}

function finish(module, what) {
  if (!module.validate()) throw new Error(`split-wasm: the ${what} module does not validate`);
  const bytes = module.emitBinary();
  module.dispose();
  return bytes;
}

/** Profiling build: every defined function calls `env.host_mark(i)` first; `names[i]` is its name. */
export function instrument(bytes) {
  const module = read(bytes);
  module.addFunctionImport('__ihn_mark', 'env', 'host_mark', binaryen.i32, binaryen.none);
  const names = [];
  for (const [i, f] of functions(module).entries()) {
    names[i] = f.name;
    if (f.module || f.name === '__ihn_mark') continue;
    const fn = module.getFunction(f.name);
    binaryen.Function.setBody(fn, module.block(null, [module.call('__ihn_mark', [module.i32.const(i)], binaryen.none), f.body], f.results));
  }
  return { bytes: finish(module, 'instrumented'), names };
}

/** A function name without the hashes Rust puts in its symbols (legacy `17h<16 hex>E`, v0 `Cs<hash>_`): they change
 *  with the cargo features, so the test build and the production build would name the same function differently.
 *  Generic instances then share a name and are kept together, which only keeps a little more. */
export const stableName = name => name.replace(/17h[0-9a-f]{16}E$/, 'E').replace(/Cs[0-9A-Za-z]+_/g, 'Cs_');

/** `keep`: stable names (`stableName`) of the functions that stay in the first module (with the exported ones). */
export function split(bytes, keep) {
  const first = read(bytes), second = read(bytes);
  const all = functions(first);
  const exported = new Set(exportsOf(first).filter(e => e.kind === binaryen.ExternalFunction).map(e => e.value));
  // Functions without a name in the name section get their index as name, which differs between builds (the test
  // and the production build, or after any change), so a profile cannot name them: they always stay.
  const moved = all.filter(f => !f.module && !exported.has(f.name) && !keep.has(stableName(f.name)) && !/^\d+$/.test(f.name));
  const movedNames = new Set(moved.map(f => f.name));
  if (!moved.length) throw new Error('split-wasm: nothing to move');

  const table = binaryen.getTableInfo(first.getTableByIndex(0));
  const base = table.initial;

  // ---- second module: the moved functions, everything else imported from the first one.
  for (const name of second.getTableSegments(table.name)) second.removeElementSegment(name);
  second.runPasses(['remove-memory']);
  second.addMemoryImport('0', 'p', 'memory');
  // The imported table has the first module's new size (base + one slot per moved function).
  binaryen._BinaryenTableSetInitial(second.getTable(table.name), base + moved.length);
  binaryen._BinaryenTableSetMax(second.getTable(table.name), base + moved.length);
  second.addTableImport(table.name, 'p', 't');
  const globals = [];
  for (let i = 0; i < second.getNumGlobals(); i++) globals.push(binaryen.getGlobalInfo(second.getGlobalByIndex(i)));
  globals.forEach((g, i) => {
    second.removeGlobal(g.name);
    second.addGlobalImport(g.name, 'p', `g${i}`, g.type, g.mutable);
  });
  const exportName = new Map();
  for (const f of all) {
    if (f.module || movedNames.has(f.name)) continue;
    const name = `f${exportName.size.toString(36)}`;
    exportName.set(f.name, name);
    second.removeFunction(f.name);
    second.addFunctionImport(f.name, 'p', name, f.params, f.results);
  }
  for (const e of exportsOf(second)) second.removeExport(e.name);
  second.addActiveElementSegment(table.name, '__ihn_split', moved.map(f => f.name), second.i32.const(base));
  second.runPasses(['remove-unused-module-elements']);
  const needed = [];
  for (const f of functions(second)) if (f.module === 'p') needed.push(f.name);

  // ---- first module: stubs, a larger table, and the exports the second module imports.
  const tableRef = first.getTable(table.name);
  binaryen._BinaryenTableSetInitial(tableRef, base + moved.length);
  binaryen._BinaryenTableSetMax(tableRef, base + moved.length);
  first.addFunctionImport('__ihn_split', 'env', 'host_split', binaryen.none, binaryen.none);
  moved.forEach((f, i) => {
    const slot = () => first.i32.const(base + i);
    const args = binaryen.expandType(f.params).map((type, index) => first.local.get(index, type));
    const ensure = first.if(first.ref.is_null(first.table.get(table.name, slot(), binaryen.funcref)), first.call('__ihn_split', [], binaryen.none));
    const forward = first.call_indirect(table.name, slot(), args, f.params, f.results);
    binaryen.Function.setBody(first.getFunction(f.name), first.block(null, [ensure, forward], f.results));
  });
  first.addTableExport(table.name, 't');
  globals.forEach((g, i) => first.addGlobalExport(g.name, `g${i}`));
  for (const name of needed) first.addFunctionExport(name, exportName.get(name));
  // The stubs leave their old locals unused; drop them and anything only the moved bodies used.
  first.runPasses(['remove-unused-module-elements', 'simplify-locals', 'vacuum']);
  return { first: finish(first, 'first'), second: finish(second, 'second'), moved: moved.length, kept: all.length - moved.length };
}
