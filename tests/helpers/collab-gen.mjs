// Seeded generator of JSON-like collaboration data (pages, structure meta, field meta) for differential tests.
import { rng } from './legacy.mjs';

export function makeGenerator(seed) {
    const random = rng(seed);
    const int = n => Math.floor(random() * n);
    const pick = list => list[int(list.length)];
    const f64 = new Float64Array(1), u32 = new Uint32Array(f64.buffer);
    const finiteDouble = () => {
        for (;;) { u32[0] = int(2 ** 32); u32[1] = int(2 ** 32); if (Number.isFinite(f64[0])) return f64[0]; }
    };
    const number = () => pick([
        () => int(2000) - 1000, () => (int(2000000) - 1000000) / 1000, () => random() * 1000, finiteDouble,
        () => pick([0, -0, 1e21, 1e-7, 1.5e-7, 0.1 + 0.2, 123456789012345680000, 5e-324, 1.7976931348623157e308, 2 ** 53, 2 ** 60, 4.35, 1e15, 1e14 + 0.5]),
        () => int(2 ** 31) * (random() < .5 ? -1 : 1)
    ])();
    const unitPool = ['a', 'b', 'z', 'A', '0', '1', '9', ' ', '"', '\\', '/', '\n', '\t', '\u0001', '\u001f', '\u007f', ' ', 'é', '中',
        '～', '😀', '\ud83d', '\ude00', ':', '-', '_', '__proto__', 'id', 'x'];
    const string = (max = 6) => { let s = ''; for (let n = int(max); n > 0; n -= 1) s += pick(unitPool); return s; };
    const key = () => pick([() => string(), () => String(int(30)), () => String(int(3)), () => pick(['a', 'b', 'id', 'x', 'y', 'p', 'stamp', 'value', 'clock', 'actor']),
        () => pick(['10', '9', '2', '01', '4294967294', '4294967295', '-1', '1.5'])])();
    const plain = (depth = 0, specials = false) => {
        const roll = int(depth > 3 ? 6 : 9);
        if (roll === 0) return null;
        if (roll === 1) return random() < .5;
        if (roll === 2 || roll === 3) return number();
        if (roll === 4 || roll === 5) return string(8);
        if (specials && random() < .05) return pick([NaN, Infinity, -Infinity, undefined]);
        if (roll === 6 || roll === 7) return Array.from({ length: int(5) }, () => plain(depth + 1, specials));
        const object = {};
        for (let n = int(5); n > 0; n -= 1) { const k = key(); if (k === '__proto__') continue; const v = plain(depth + 1, specials); if (specials || v !== undefined) object[k] = v; }
        return object;
    };
    const loose = () => pick([null, '', 0, 1, 2.5, -3, 'x', '12', ' 7 ', '0x10', '1e3', 'Infinity', true, false, [], [4], [1, 2], {}, { clock: 3 }]);
    const stamp = () => {
        if (random() < .1) return loose();
        const value = {};
        if (random() < .9) value.clock = pick([int(100), int(1e6), number(), String(int(50)), null, '', true, -5, 3.7, '3.9', [7], [], {}, 'abc']);
        if (random() < .85) value.actor = pick([string(), 'peer-a:tab-a', 'peer-b', 0, 5, null, ['q', 'r'], {}, true, '']);
        return value;
    };
    const pageIdSet = () => Array.from({ length: 2 + int(7) }, () => pick(['page-a', 'page-b', 'page-c', 'p1', 'p2', '1', '2', '10', 'zz', string(4), 'é', '😀']));
    const structure = ids => {
        if (random() < .08) return pick([null, undefined, 5, 'x', [], {}, { entries: [stamp(), stamp()] }]);
        const meta = {};
        if (random() < .8) meta.v = pick([1, 2, '1']);
        if (random() < .85) meta.clock = pick([int(100), number(), String(int(9)), null, 'z']);
        const entries = {};
        for (const id of ids) if (random() < .8) entries[id] = random() < .08 ? loose() : { rank: pick([int(10) * 1024, number(), null, '5', 'x', '', [], 0.5, undefined]), orderStamp: stamp(), createdStamp: stamp() };
        if (random() < .1) entries[''] = { rank: 3 };
        if (random() < .9) meta.entries = random() < .05 ? [loose(), { rank: 2 }] : entries;
        const tombstones = {};
        for (const id of ids) if (random() < .3) tombstones[id] = stamp();
        if (random() < .1) tombstones[''] = stamp();
        if (random() < .9) meta.tombstones = tombstones;
        return meta;
    };
    const fields = () => {
        if (random() < .08) return pick([null, undefined, 3, 'x', [], { fields: [{ value: 1 }] }, { fields: 'no' }]);
        const meta = {};
        if (random() < .85) meta.clock = pick([int(100), number(), '4', null]);
        const records = {};
        for (let n = int(6); n > 0; n -= 1) {
            const k = key(); if (k === '__proto__') continue;
            const roll = random();
            if (roll < .08) records[k] = loose();
            else if (roll < .14) records[k] = { stamp: stamp() };
            else records[k] = { value: plain(0, true), stamp: stamp() };
        }
        meta.fields = random() < .93 ? records : pick([null, [], 'x', [{ value: 1, stamp: { clock: 2 } }]]);
        return meta;
    };
    const point = () => pick([() => ({ x: number(), y: number(), p: random() }), () => [number(), number(), number()], () => ({ x: int(900), y: int(900) })])();
    const item = (kind, usedIds) => {
        if (random() < .05) return pick([null, 0, 7, 'str', true, [], [1, { b: 1, a: 2 }, 'z'], NaN]);
        const value = { ...(plain(1, true) && typeof plain === 'function' ? {} : {}) };
        const idRoll = random();
        if (idRoll < .7) value.id = `${kind}-${int(40)}`;
        else if (idRoll < .78) value.id = pick([0, 5, 7.5, '', null, [1, 2], { a: 1 }, true, false, [], '😀', 'ÿ']);
        else if (idRoll < .85) value.id = usedIds[int(usedIds.length)] ?? 'dup';
        usedIds.push(value.id);
        if (random() < .8) value.points = Array.from({ length: int(12) }, point);
        if (random() < .5) value.color = pick(['#000', '#ff0000', null, 3]);
        if (random() < .5) value.width = number();
        if (random() < .3) value.preview = plain(1);
        if (random() < .3) value.needsRedraw = true;
        if (random() < .2) value.unloaded = pick([true, false, null]);
        if (random() < .2) value.pendingUnload = plain(1);
        if (random() < .25) value.extra = plain(0, true);
        if (random() < .1) value.src = `data:image/png;base64,${string(30)}`;
        if (random() < .1) value[pick(['2', '10', '1'])] = plain(2);
        return value;
    };
    const page = pageId => {
        if (random() < .03) return pick([null, 5, 'page', [], [1, 2], true]);
        const used = [];
        const value = {};
        if (random() < .95) value.pageId = random() < .93 ? pageId : pick([0, 5, null, ['a', 'b'], '', true]);
        if (random() < .9) value.strokes = random() < .96 ? Array.from({ length: int(9) }, () => item('stroke', used)) : pick([null, 'x', {}, 5]);
        if (random() < .7) value.images = random() < .96 ? Array.from({ length: int(4) }, () => item('image', used)) : pick([null, {}]);
        if (random() < .7) value.deletedStrokeIds = random() < .94
            ? Array.from({ length: int(6) }, () => pick(['stroke-1', 'stroke-2', 'stroke-3', 'gone', '', null, 0, 5, 5, false, true, ['a'], 'é', '😀', { id: 'obj' }]))
            : pick([null, 'abc', {}, 5]);
        if (random() < .7) {
            const form = random();
            if (form < .55) { const stamps = {}; for (let n = int(5); n > 0; n -= 1) stamps[pick(['stroke-1', 'stroke-2', 'stroke-9', '', '1', '10', 'gone', string(3)])] = random() < .2 ? loose() : stamp(); value.deletedStrokeStamps = stamps; }
            else if (form < .9) value.deletedStrokeStamps = Array.from({ length: int(5) }, () => pick([
                () => ({ id: pick(['stroke-1', 'stroke-2', '', 7, null, 'gone']), stamp: stamp() }),
                () => ({ id: pick(['stroke-1', 'stroke-3']), clock: pick([int(9), '4', null]), actor: pick(['a', 'b', 3]) }),
                () => pick(['stroke-1', 'bare', '', 5, null, true, [], [1]]),
                () => ({ id: 'stroke-2', stamp: pick([null, 0, '', {}, { stamp: stamp() }]) })
            ])());
            else value.deletedStrokeStamps = pick([null, 'x', 5, true, false, 0]);
        }
        if (random() < .75) value.backgroundSource = pick(['template', 'pdf', 'image', '', null, 0, [], ['x'], 5, 'é']);
        if (random() < .7) value.templateKind = pick(['default', 'lined', '', null, 0, 5, [], 'é']);
        if (random() < .8) value.pageWidth = pick([210, 595.28, number(), '300', '', null, 'x', [], [7], {}, true, 0, -0]);
        if (random() < .8) value.pageHeight = pick([297, 841.89, number(), '400', null, 'x', 0, false]);
        if (random() < .6) value.sidePanel = random() < .3 ? pick([null, 0, '', false]) : random() < .12 ? pick(['str', 5, true, [], [1], { z: 1 }])
            : { title: pick(['Notes', '', null, 0, 5, 'é', ['a']]), ...(random() < .8 ? { dateKeys: random() < .9 ? Array.from({ length: int(4) }, () => pick(['2024-01-01', '2024-02-01', 5, null, '', ['q']])) : pick([null, 'x', {}]) } : {}) };
        if (random() < .15) value.unknownField = plain(0, true);
        return value;
    };
    return { random, int, pick, string, plain, structure, fields, page, pageIdSet, number, stamp, key };
}
