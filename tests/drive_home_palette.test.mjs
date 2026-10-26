// Differential test: Rust folder palette maths (drive_home.palette / view_tint / resolve_color)
// against the original JavaScript from app-v5.js at the pre-port commit. Byte-exact.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import vm from 'node:vm';
import { legacySource, rng } from './helpers/legacy.mjs';

// Objects built inside vm contexts have foreign prototypes; compare their JSON shape.
const plain = value => JSON.parse(JSON.stringify(value));

const bytes = await readFile(new URL('../wasm/inhouse_notes_core.wasm', import.meta.url));
const scope = { WebAssembly, Uint8Array, Uint16Array, Float32Array, URL, location: new URL('https://notes.test/'), fetch: async () => new Response(bytes),
    console, AbortSignal, AbortController, setTimeout, clearTimeout, TextEncoder, TextDecoder };
vm.createContext(scope);
vm.runInContext(await readFile(new URL('../rust-core-v5.js', import.meta.url), 'utf8'), scope);
assert.equal(await scope.InhouseRust.ready, true, 'WASM core must load');
const rust = scope.InhouseRust;

// Original algorithm: the exact text between resolveDriveFolderColor and createDriveFolderArt.
const source = legacySource('app-v5.js');
const from = source.indexOf('function resolveDriveFolderColor(item)');
const to = source.indexOf('function createDriveFolderArt(');
assert.ok(from > 0 && to > from, 'legacy palette source not found');
const DEFAULT = '#e3c58e';
const theme = { value: 'light' };
const legacy = vm.runInNewContext(`(function(){
    const DRIVE_DEFAULT_FOLDER_COLOR='${DEFAULT}';
    const document={documentElement:{dataset:{get theme(){return theme.value;}}}};
    ${source.slice(from, to)}
    return {resolveDriveFolderColor,applyDriveFolderPalette,getDriveFolderViewTint,displayDriveFolderColor,driveHexToHsl,driveHslToHex,driveRelativeLuminance};
})()`, { theme, Math });

const KEYS = { '--folder-color': 'color', '--folder-cover': 'cover', '--folder-inside': 'inside', '--folder-tab': 'tab', '--folder-ink': 'ink',
    '--folder-hl': 'hl', '--folder-hl-strong': 'hlStrong', '--folder-line': 'line' };
function legacyPalette(base, night, exact) {
    theme.value = night ? 'dark' : 'light';
    const vars = {}, el = { style: { setProperty: (k, v) => { vars[KEYS[k]] = v; } }, dataset: {} };
    legacy.applyDriveFolderPalette(el, base, { exact });
    return vars;
}
const hex = n => '#' + n.toString(16).padStart(6, '0');
const random = rng(20240611);
const randomHex = () => hex(Math.floor(random() * 0x1000000));

// Google Drive's own folder colours, extremes, near-greys and warm/yellow hues (the special-cased range).
const FIXTURES = ['#ac725e', '#d06b64', '#f83a22', '#fa573c', '#ff7537', '#ffad46', '#42d692', '#16a765', '#7bd148', '#b3dc6c', '#fbe983', '#fad165',
    '#92e1c0', '#9fe1e7', '#9fc6e7', '#4986e7', '#9a9cff', '#b99aff', '#c2c2c2', '#cabdbf', '#cca6ac', '#f691b2', '#cd74e6', '#a47ae2', '#8f8f8f',
    DEFAULT, '#000000', '#ffffff', '#808080', '#0a0a0a', '#f5f5f5', '#ff0000', '#00ff00', '#0000ff', '#ffff00', '#00ffff', '#ff00ff',
    '#8f8f90', '#e8e8e9', '#fffe00', '#b3b300', '#999900', '#fdf5bf', '#ffe066', '#d4a017', '#6b4f00', '#c9ff00', '#ffee00', '#ABCDEF'];

function compare(bases, night, exact) {
    const got = rust.driveFolderPalettes(bases, night, exact);
    bases.forEach((base, index) => assert.deepEqual(plain(got[index]), plain(legacyPalette(base, night, exact)), `${base} night=${night} exact=${exact}`));
}

test('palette matches the original JS on fixtures in every mode', () => {
    for (const night of [false, true]) for (const exact of [false, true]) compare(FIXTURES, night, exact);
});

test('palette matches the original JS on 6000 seeded random colours in every mode', () => {
    const colours = Array.from({ length: 6000 }, randomHex);
    for (const night of [false, true]) for (const exact of [false, true]) compare(colours, night, exact);
});

test('palette matches on a dense RGB lattice including saturated warm hues', () => {
    const lattice = [];
    for (let r = 0; r < 256; r += 5) for (let g = 0; g < 256; g += 5) for (let b = 0; b < 256; b += 15) lattice.push(hex((r << 16) | (g << 8) | b));
    for (const night of [false, true]) compare(lattice, night, false);
});

test('ink decision matches for the 400 colours closest to the contrast switch point', () => {
    // Exhaustive over 2^24 colours with the original luminance formula. A rounding difference between
    // V8 and wasm pow() could only flip the decision for a colour that sits within ~1e-15 of the switch.
    const lin = Array.from({ length: 256 }, (_, v) => { const c = v / 255; return c <= 0.03928 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4); });
    const closest = [];
    for (let r = 0; r < 256; r++) for (let g = 0; g < 256; g++) for (let b = 0; b < 256; b++) {
        const l = 0.2126 * lin[r] + 0.7152 * lin[g] + 0.0722 * lin[b];
        const gap = Math.abs((l + 0.05) / 0.0614 - 1.05 / (l + 0.05));
        if (gap < 3e-4) closest.push([gap, (r << 16) | (g << 8) | b]);
    }
    assert.ok(closest.length >= 400, `expected 400 near-ties, found ${closest.length}`);
    closest.sort((x, y) => x[0] - y[0]);
    const near = closest.slice(0, 400).map(([, n]) => hex(n));
    assert.ok(closest[0][0] < 1e-6, 'sanity: the sweep reaches the switch point');
    compare(near, false, true);
    compare(near, true, true);
});

test('view tint matches the original JS by day and night', () => {
    const colours = [...FIXTURES, ...Array.from({ length: 4000 }, randomHex)];
    for (const night of [false, true]) for (const color of colours) {
        theme.value = night ? 'dark' : 'light';
        assert.deepEqual(plain(rust.driveFolderViewTint(color, night)), plain(legacy.getDriveFolderViewTint(color)), `${color} night=${night}`);
    }
});

test('resolve_color matches resolveDriveFolderColor for valid, invalid and odd strings', () => {
    const odd = ['', ' ', '#8f8f8f', '#8F8F8F', ' #8f8f8f ', '#abc', '#abcdefg', 'abcdef', '#ABCDEF', '  #AbCdEf\n', '\t#123456\r', ' #123456 ', '﻿#123456',
        '\u0085#123456', '#12345g', '#１２３４５６', '#ab\u0000cde', 'rgb(1,2,3)', '#e3c58e', '#00000', '#0000000', 'İ', '#ab\ud800cde', undefined, null, 0, false, 123456];
    const items = odd.map(value => ({ folderColorRgb: value }));
    for (let i = 0; i < 2000; i++) {
        const chars = '0123456789abcdefABCDEF#g \n';
        let text = random() < 0.8 ? '#' : '';
        for (let j = 0, n = Math.floor(random() * 8); j < n; j++) text += chars[Math.floor(random() * chars.length)];
        items.push({ folderColorRgb: text });
    }
    items.push(null, undefined, {});
    const expected = items.map(item => legacy.resolveDriveFolderColor(item));
    const raws = items.map(item => String(item?.folderColorRgb || ''));
    assert.deepEqual(plain(rust.driveFolderColors(raws, DEFAULT)), expected);
});

test('invalid colours fail loudly instead of producing NaN styles', () => {
    assert.throws(() => rust.driveFolderPalettes(['nope'], false), /Invalid folder colour/);
    assert.throws(() => rust.driveFolderViewTint('', true), /Invalid folder colour/);
});

test('cache returns identical frozen palettes for repeated colours and keeps modes separate', () => {
    const [a, b, c] = rust.driveFolderPalettes(['#4986e7', '#4986e7', '#fbe983'], false);
    assert.equal(a, b);
    assert.ok(Object.isFrozen(a));
    const [night] = rust.driveFolderPalettes(['#4986e7'], true);
    assert.notDeepEqual(plain(night), plain(a));
    assert.equal(c.color, legacyPalette('#fbe983', false, false).color);
});
