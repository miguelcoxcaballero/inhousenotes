// Loads a source file exactly as it was before the Rust port, so Rust output can be
// compared with the original JavaScript on identical inputs (differential tests).
import { execFileSync } from 'node:child_process';
import vm from 'node:vm';

/** Last commit on main before the Rust port started. */
export const PORT_BASE = process.env.RUST_PORT_BASE || 'a2dea10';

export function legacySource(path) {
    return execFileSync('git', ['show', `${PORT_BASE}:${path}`], { encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 });
}

/** Evaluate a legacy browser script in a fresh vm context and return that context. */
export function loadLegacy(path, globals = {}) {
    const scope = { console, setTimeout, clearTimeout, TextEncoder, TextDecoder, URL, Uint8Array, Uint8ClampedArray, Uint16Array,
        Uint32Array, Int32Array, Float32Array, Float64Array, Math, JSON, Map, Set, WeakMap, Promise, ...globals };
    scope.window = scope.self = scope.globalThis = scope;
    vm.createContext(scope);
    vm.runInContext(legacySource(path), scope, { filename: `legacy:${path}` });
    return scope;
}

/** Deterministic PRNG so failing random cases can be replayed from the seed. */
export function rng(seed = 1) {
    let state = seed >>> 0;
    return () => { state = (state + 0x6d2b79f5) >>> 0; let t = state; t = Math.imul(t ^ (t >>> 15), t | 1);
        t ^= t + Math.imul(t ^ (t >>> 7), t | 61); return ((t ^ (t >>> 14)) >>> 0) / 4294967296; };
}
