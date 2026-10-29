// Background outbox scheduling with fake timers: the library must not poll,
// must start the hidden document processor only when an operation can make
// progress, must back off failing operations exponentially, must reuse one
// processor instead of reloading the whole app, and must react to real events.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import vm from 'node:vm';

const read = name => readFileSync(new URL(`../${name}`, import.meta.url), 'utf8');
const sources = { sync: read('offline-sync-v5.js'), library: read('offline-library-v5.js') };
const { CoalescingQueue } = createRequire(import.meta.url)('../runtime-core-v5.js');
const ACCOUNT = 'schedule@example.test';
const ORIGIN = 'https://notes.test';
const flush = async (turns = 30) => { for (let i = 0; i < turns; i++) await new Promise(resolve => setImmediate(resolve)); };

function harness({ outbox = [], documents = [], visible = true, online = true, hostOverrides = {} } = {}) {
    let now = Date.parse('2026-10-29T12:00:00Z');
    const timers = new Map(); let nextTimer = 1;
    const clock = {
        now: () => now,
        setTimeout(fn, delay = 0) { const id = nextTimer++; timers.set(id, { at: now + Math.max(0, Number(delay) || 0), fn }); return id; },
        clearTimeout(id) { timers.delete(id); },
        setInterval() { throw new Error('The outbox scheduler must not poll with setInterval'); },
        pending: () => [...timers.values()].map(timer => timer.at - now).sort((a, b) => a - b),
        async advance(ms) {
            const end = now + ms;
            for (;;) {
                await flush();
                const due = [...timers.entries()].filter(([, timer]) => timer.at <= end).sort((a, b) => a[1].at - b[1].at)[0];
                if (!due) break;
                timers.delete(due[0]); now = Math.max(now, due[1].at); due[1].fn();
            }
            now = end; await flush();
        }
    };
    class FakeDate extends Date { static now() { return now; } }

    // In-memory store with the semantics the scheduler and the runner rely on.
    const data = new Map(), writes = [];
    const k = (store, account, id) => `${store}\u0000${account}\u0000${id}`;
    const clone = value => value == null ? value : structuredClone(value);
    const store = {
        async read(name, account, id) { return clone(data.get(k(name, account, id)) || null); },
        async list(name, account) { return [...data.entries()].filter(([key]) => key.startsWith(`${name}\u0000${account}\u0000`)).map(([, value]) => clone(value)); },
        async put(name, account, id, value) { writes.push(name); const record = { ...value, account, id }; data.set(k(name, account, id), record); return clone(record); },
        async patch(name, account, id, update) {
            writes.push(name);
            const previous = data.get(k(name, account, id)) || { account, id };
            const next = update(clone(previous));
            if (next) data.set(k(name, account, id), next); else data.delete(k(name, account, id));
            return clone(next);
        },
        async resolveId(account, id) { return id && !id.startsWith('local-') ? id : id ? null : 'root'; },
        async acknowledge(account, id, generation) {
            const doc = data.get(k('documents', account, id));
            if (doc?.generation === generation) { doc.dirty = false; data.delete(k('outbox', account, id)); }
            return doc?.generation === generation;
        },
        async prune() { return 0; },
        async cacheEntries(account, entries) {
            writes.push('entries');
            for (const entry of entries) data.set(k('entries', account, entry.id), { ...(data.get(k('entries', account, entry.id)) || {}), ...clone(entry), account });
        },
        async finishFolder(account, id) { data.delete(k('outbox', account, id)); },
        async withLease(account, name, work) {
            const lease = data.get(k('leases', account, name));
            if (lease?.expires > now) return { busy: true };
            data.set(k('leases', account, name), { owner: 'runner', expires: now + 45000 });
            try { return await work(async () => {}); } finally { data.delete(k('leases', account, name)); }
        }
    };
    for (const op of outbox) data.set(k('outbox', ACCOUNT, op.id), { kind: 'document', generation: 1, account: ACCOUNT, ...op });
    for (const doc of documents) data.set(k('documents', ACCOUNT, doc.id), { account: ACCOUNT, generation: 1, dirty: true, blob: new Blob(['%PDF']), baseMeta: { version: '1' }, ...doc });

    // DOM, window and platform stand-ins.
    const listeners = { window: new Map(), document: new Map() };
    const on = scope => (type, fn) => { if (!listeners[scope].has(type)) listeners[scope].set(type, new Set()); listeners[scope].get(type).add(fn); };
    const off = scope => (type, fn) => listeners[scope].get(type)?.delete(fn);
    const emit = (scope, type, event = {}) => { for (const fn of [...(listeners[scope].get(type) || [])]) fn({ type, ...event }); };
    const frames = [], posted = [], probes = [], native = [];
    const documentRef = {
        visibilityState: visible ? 'visible' : 'hidden', get hidden() { return this.visibilityState !== 'visible'; },
        querySelectorAll: () => [], addEventListener: on('document'), removeEventListener: off('document'),
        createElement() {
            const frame = { hidden: false, title: '', src: '', removed: false, remove() { this.removed = true; } };
            frame.contentWindow = { postMessage(message) { posted.push(message); onWorkerMessage(frame, message); } };
            return frame;
        },
        body: { appendChild(frame) { frames.push(frame); onWorkerMessage(frame, { type: 'load' }); } }
    };
    const sandbox = {
        console, structuredClone, URLSearchParams, Blob, Promise, Map, Set, WeakMap, JSON, Math, Number, String, Object, Array, Error,
        Date: FakeDate, setTimeout: clock.setTimeout, clearTimeout: clock.clearTimeout, setInterval: clock.setInterval, clearInterval() {},
        location: { search: '', origin: ORIGIN }, navigator: { onLine: online },
        localStorage: { getItem: key => key === 'drive-hint-email' ? ACCOUNT : null },
        sessionStorage: { getItem: () => (online ? '0' : '1'), setItem() {} },
        document: documentRef, AbortSignal: { timeout: () => ({}) },
        crypto: { randomUUID: () => Math.random().toString(16).slice(2) },
        fetch: async url => { probes.push(url); if (!sandbox.__reachable) throw new Error('offline'); return { status: 401 }; },
        InhouseOfflineStore: store, InhouseRuntimeCore: { CoalescingQueue },
        InhouseBackground: { schedule: () => native.push('schedule'), cancel: () => native.push('cancel') },
        addEventListener: on('window'), removeEventListener: off('window'), __reachable: online
    };
    sandbox.window = sandbox; sandbox.globalThis = sandbox;
    vm.createContext(sandbox);
    vm.runInContext(sources.sync, sandbox); vm.runInContext(sources.library, sandbox);
    const L = sandbox.InhouseOfflineLibrary, Sync = sandbox.InhouseOfflineSync;

    // The hidden processor: each load or 'ihn-offline-run' performs one pass
    // with the real outbox runner against a scripted Drive.
    const drive = { status: 500, requests: 0 };
    const workerRuns = [];
    function onWorkerMessage(frame, message) {
        if (message.type !== 'load' && message.type !== 'ihn-offline-run') return;
        workerRuns.push({ at: now, frame: frames.indexOf(frame), reused: message.type !== 'load' });
        void (async () => {
            await flush(2);
            const result = await Sync.run(ACCOUNT, { background: true, build: async () => new Blob(['%PDF']),
                skip: id => (host.activeDocument() && id === host.currentId() ? 'owner' : false),
                fetch: async url => {
                    drive.requests++;
                    if (/\/upload\//.test(url)) return { ok: drive.status < 400, status: drive.status, json: async () => ({ id: 'doc' }) };
                    return { ok: true, status: 200, headers: { get: () => '"etag"' }, json: async () => ({ id: 'doc', version: '1', capabilities: { canEdit: true } }) };
                } });
            emit('window', 'message', { origin: ORIGIN, source: frame.contentWindow, data: { type: 'ihn-offline-complete', result, reusable: true } });
        })();
    }
    const host = {
        version: '0.0.0', busyValue: false, active: null, signedIn: true, syncActiveCalls: 0,
        busy: () => host.busyValue, activeDocument: () => !!host.active, currentId: () => host.active || '',
        canSync: () => host.signedIn, identity: () => ({ id: '', version: 0, session: 0 }), capture: async () => null,
        syncActive: async () => { host.syncActiveCalls++; }, syncChanged() {}, refreshed() {}, connectionChanged() {}, error() {}, status() {},
        ...hostOverrides
    };
    return { L, Sync, host, clock, store, data, writes, frames, posted, probes, native, drive, workerRuns, documentRef, emit, sandbox,
        op: id => clone(data.get(k('outbox', ACCOUNT, id))) ?? null,
        async start() { await L.init(host); await L.resume(); await flush(); },
        setVisible(value) { documentRef.visibilityState = value ? 'visible' : 'hidden'; emit('document', 'visibilitychange'); }
    };
}

test('an idle Home with an empty outbox does no background work at all', async () => {
    const h = harness();
    await h.start();
    const writesAfterStart = h.writes.length, probesAfterStart = h.probes.length;
    await h.clock.advance(10 * 60000);
    assert.equal(h.frames.length, 0, 'no document processor is loaded');
    assert.equal(h.writes.length, writesAfterStart, 'no lease or outbox writes while idle');
    assert.equal(h.probes.length, probesAfterStart, 'a connected app does not probe Drive');
    assert.deepEqual(h.clock.pending(), [], 'no timer remains armed');
});

test('a failing upload backs off exponentially and reuses one processor', async () => {
    const h = harness({ outbox: [{ id: 'other' }], documents: [{ id: 'other' }] });
    await h.start();
    assert.equal(h.frames.length, 1, 'launch processes the waiting document at once');
    await h.clock.advance(10 * 60000);
    const gaps = h.workerRuns.slice(1).map((run, i) => Math.round((run.at - h.workerRuns[i].at) / 1000));
    // 15 s, 30 s, 60 s, 120 s, 240 s ... instead of a full app load every 10 s.
    assert.deepEqual(gaps.slice(0, 4), [15, 30, 60, 120]);
    assert.ok(h.workerRuns.length <= 6, `at most 6 attempts in 10 minutes, got ${h.workerRuns.length}`);
    // Runs within the idle window reuse the loaded processor; a longer wait releases it.
    assert.deepEqual(h.workerRuns.slice(0, 4).map(run => run.reused), [false, true, true, true]);
    assert.ok(h.frames.length < h.workerRuns.length, 'the processor is reused instead of reloaded each time');
    assert.ok(h.frames.slice(0, -1).every(frame => frame.removed), 'an idle processor is released');
    assert.equal(h.op('other').attempts, h.workerRuns.length);
    assert.equal(h.op('other').lastError, 500, h.op('other').lastErrorMessage);
});

test('the processor is released as soon as the outbox is empty', async () => {
    const h = harness({ outbox: [{ id: 'other' }], documents: [{ id: 'other' }] });
    h.drive.status = 200;
    await h.start();
    assert.equal(h.workerRuns.length, 1);
    assert.equal(h.op('other'), null, 'uploaded');
    assert.equal(h.frames[0].removed, true);
    await h.clock.advance(10 * 60000);
    assert.equal(h.workerRuns.length, 1);
    assert.deepEqual(h.clock.pending(), []);
});

test('the processor is never loaded when it could not make progress', async () => {
    // Only the open document is pending: its editor saves it.
    let h = harness({ outbox: [{ id: 'open' }], documents: [{ id: 'open' }] });
    h.host.active = 'open';
    await h.start();
    await h.clock.advance(5 * 60000);
    assert.equal(h.frames.length, 0, 'open document');
    // Signed out or waiting for a new sign-in.
    h = harness({ outbox: [{ id: 'other' }], documents: [{ id: 'other' }] });
    h.host.signedIn = false;
    await h.start();
    await h.clock.advance(5 * 60000);
    assert.equal(h.frames.length, 0, 'signed out');
    h.host.signedIn = true; await h.L.kick({ force: true, reason: 'signed-in' }); await h.clock.advance(1000);
    assert.equal(h.frames.length, 1, 'signing in starts the waiting upload');
    // Offline.
    h = harness({ outbox: [{ id: 'other' }], documents: [{ id: 'other' }], online: false });
    await h.start();
    await h.clock.advance(5 * 60000);
    assert.equal(h.frames.length, 0, 'offline');
    // Backoff not elapsed: a checkpoint of the open document does not retry another one early.
    h = harness({ outbox: [{ id: 'other', attemptedAt: Date.parse('2026-10-29T12:00:00Z'), attempts: 3, lastError: 500 }], documents: [{ id: 'other' }] });
    await h.L.init(h.host);
    // (no launch kick: resume() would retry everything once)
    await h.L.kick({ reason: 'checkpoint' }); await h.clock.advance(1000);
    assert.equal(h.frames.length, 0, 'not resumed yet');
});

test('backoff is honoured by ordinary events and bypassed by launch, reconnection and explicit retry', async () => {
    const failedAt = Date.parse('2026-10-29T12:00:00Z');
    const h = harness({ outbox: [{ id: 'other', attemptedAt: failedAt, attempts: 5, lastError: 500 }], documents: [{ id: 'other' }] });
    await h.start();
    assert.equal(h.workerRuns.length, 1, 'a new launch retries left-over work at once');
    await h.L.kick({ reason: 'checkpoint' }); await h.clock.advance(20000);
    assert.equal(h.workerRuns.length, 1, 'a checkpoint of another document does not cut the backoff short');
    // Losing and regaining the connection retries at once.
    h.sandbox.__reachable = false; h.emit('window', 'offline'); await h.clock.advance(1000);
    h.sandbox.__reachable = true; h.emit('window', 'online'); await h.clock.advance(1000);
    assert.equal(h.workerRuns.length, 2, 'reconnection');
    await h.L.kick({ force: true, reason: 'retry' }); await h.clock.advance(6000);
    assert.equal(h.workerRuns.length, 3, 'explicit retry');
});

test('a new checkpoint of a waiting document is processed immediately', async () => {
    const h = harness({ outbox: [{ id: 'other' }], documents: [{ id: 'other' }] });
    await h.start();
    assert.equal(h.workerRuns.length, 1);
    // The checkpoint replaces the outbox record (new generation, no attempts).
    h.data.set(`outbox\u0000${ACCOUNT}\u0000other`, { kind: 'document', id: 'other', account: ACCOUNT, generation: 2 });
    h.data.get(`documents\u0000${ACCOUNT}\u0000other`).generation = 2;
    await h.clock.advance(5000);
    await h.L.kick({ reason: 'checkpoint' }); await h.clock.advance(100);
    assert.equal(h.workerRuns.length, 2);
    assert.equal(h.workerRuns[1].reused, true);
});

test('a held sync lease defers the processor until the lease is released', async () => {
    const h = harness({ outbox: [{ id: 'left' }], documents: [{ id: 'left' }] });
    h.data.set(`leases\u0000${ACCOUNT}\u0000sync`, { owner: 'foreground-exit-save', expires: Date.parse('2026-10-29T12:00:45Z') });
    await h.start();
    assert.equal(h.frames.length, 0, 'the foreground upload owns the document');
    // The foreground upload finished: the lease is gone and the outbox empty.
    h.data.delete(`leases\u0000${ACCOUNT}\u0000sync`); h.data.delete(`outbox\u0000${ACCOUNT}\u0000left`);
    h.L.leaseReleased(); await h.clock.advance(60000);
    assert.equal(h.frames.length, 0, 'nothing left to do');
    // A stale lease left by a closed app expires on its own.
    const g = harness({ outbox: [{ id: 'left' }], documents: [{ id: 'left' }] });
    g.data.set(`leases\u0000${ACCOUNT}\u0000sync`, { owner: 'closed-app', expires: Date.parse('2026-10-29T12:00:45Z') });
    await g.start();
    assert.equal(g.frames.length, 0);
    await g.clock.advance(46000);
    assert.equal(g.frames.length, 1, 'retried when the stale lease expires');
});

test('a document left with a pending upload is retried right after the app restarts', async () => {
    // The previous session failed several times and left a sync lease behind.
    const h = harness({ outbox: [{ id: 'left', attemptedAt: Date.parse('2026-10-29T11:59:50Z'), attempts: 6, lastError: 'network' }], documents: [{ id: 'left' }] });
    h.data.set(`leases\u0000${ACCOUNT}\u0000sync`, { owner: 'closed-app', expires: Date.parse('2026-10-29T12:00:30Z') });
    h.drive.status = 200;
    await h.start();
    await h.clock.advance(31000);
    assert.equal(h.op('left'), null, 'uploaded within the lease lifetime, not after the old 15-minute backoff');
});

test('a busy editor defers background work and resumes it without polling', async () => {
    const h = harness({ outbox: [{ id: 'other' }], documents: [{ id: 'other' }] });
    h.host.busyValue = true;
    await h.start();
    await h.clock.advance(2000);
    assert.equal(h.frames.length, 0);
    h.host.busyValue = false;
    await h.clock.advance(3000);
    assert.equal(h.frames.length, 1);
});

test('platform background work is requested only while the app is hidden with work pending', async () => {
    const h = harness({ outbox: [{ id: 'other' }], documents: [{ id: 'other' }] });
    h.host.signedIn = false; // keep the in-page processor out of this test
    await h.start();
    assert.deepEqual(h.native, ['cancel'], 'a visible app withdraws jobs left by an earlier session');
    await h.L.scheduleBackground();
    assert.deepEqual(h.native, ['cancel'], 'no headless app instance while this one is visible');
    h.setVisible(false); await h.L.scheduleBackground();
    assert.deepEqual(h.native, ['cancel', 'schedule']);
    h.setVisible(true); await flush();
    assert.deepEqual(h.native, ['cancel', 'schedule', 'cancel']);
    h.data.delete(`outbox\u0000${ACCOUNT}\u0000other`);
    h.setVisible(false); await h.L.scheduleBackground();
    assert.deepEqual(h.native, ['cancel', 'schedule', 'cancel'], 'nothing pending: nothing scheduled');
});

test('offline probes back off and stop once connected or hidden', async () => {
    const h = harness({ online: false });
    await h.start();
    const start = h.probes.length;
    await h.clock.advance(30000 + 60000 + 120000 + 240000 + 300000 + 300000);
    assert.equal(h.probes.length - start, 6, '30 s, 1, 2, 4, 5, 5 min');
    h.setVisible(false); const hidden = h.probes.length;
    await h.clock.advance(20 * 60000);
    assert.equal(h.probes.length, hidden, 'no probes while hidden');
    h.sandbox.__reachable = true; h.setVisible(true); await h.clock.advance(100);
    assert.equal(h.L.offline(), false);
    const connected = h.probes.length;
    await h.clock.advance(20 * 60000);
    assert.equal(h.probes.length, connected, 'no probes while connected');
});

test('the runner records attempts only for the queued revision and never for a pause', async () => {
    const h = harness({ outbox: [{ id: 'a' }, { id: 'b' }, { id: 'c', generation: 2 }],
        documents: [{ id: 'a' }, { id: 'b' }, { id: 'c', generation: 2 }] });
    const abort = Object.assign(new Error('Background document work interrupted'), { name: 'AbortError' });
    const result = await h.Sync.run(ACCOUNT, { background: true, build: async () => new Blob(['%PDF']),
        fetch: async url => {
            if (url.includes('/a?')) throw abort;
            if (url.includes('/c?')) {
                // A new checkpoint lands while this attempt is failing.
                h.data.set(`outbox\u0000${ACCOUNT}\u0000c`, { kind: 'document', id: 'c', account: ACCOUNT, generation: 3 });
                return { ok: false, status: 503 };
            }
            return { ok: false, status: 500 };
        } });
    assert.equal(result.errors.length, 3);
    assert.equal(h.op('a').attempts, 1, 'a pause waits at the first step');
    assert.equal(h.op('a').lastError, undefined, 'a pause is not reported as a failure');
    assert.equal(h.op('b').attempts, 1);
    assert.equal(h.op('b').lastError, 500);
    assert.equal(h.op('c').attempts, undefined, 'the newer checkpoint stays immediately due');
    assert.equal(h.Sync.retryAt(h.op('b')), h.clock.now() + 15000);
    assert.equal(h.Sync.retryAt({ attemptedAt: 1000, attempts: 3 }), 1000 + 60000);
    assert.equal(h.Sync.retryAt({ attemptedAt: 1000, attempts: 40 }), 1000 + 15 * 60000);
    assert.equal(h.Sync.retryAt({}), 0);
});

test('a runner without a document processor leaves editable drafts untouched and offline', async () => {
    const h = harness({ outbox: [{ id: 'draft' }], documents: [{ id: 'draft', snapshot: { pages: [] } }] });
    let requests = 0;
    const result = await h.Sync.run(ACCOUNT, { fetch: async () => { requests++; return { ok: true, status: 200 }; } });
    assert.deepEqual(JSON.parse(JSON.stringify(result)), { saved: 0, pending: 1, errors: [] });
    assert.equal(requests, 0);
    assert.equal(h.op('draft').attempts, undefined);
});

test('the owner page never backs off its own open document', async () => {
    const h = harness({ outbox: [{ id: 'open' }, { id: 'other-window' }], documents: [{ id: 'open' }, { id: 'other-window' }] });
    const result = await h.Sync.run(ACCOUNT, { background: true, build: async () => new Blob(['%PDF']),
        skip: id => (id === 'open' ? 'owner' : true), fetch: async () => ({ ok: true, status: 200 }) });
    assert.equal(result.pending, 2);
    assert.equal(h.op('open').attempts, undefined);
    assert.equal(h.op('other-window').attempts, 1, 'an editor in another window: wait before checking again');
});

test('an unchanged Home refresh writes and re-renders nothing', async () => {
    const h = harness();
    await h.start();
    const items = [{ id: 'a', name: 'A.pdf', mimeType: 'application/pdf', modifiedTime: '2026-10-29T10:00:00Z', parents: ['root'] },
        { id: 'b', name: 'B.pdf', mimeType: 'application/pdf', modifiedTime: '2026-10-29T09:00:00Z', parents: ['root'] }];
    await h.L.observed(items, null, 'recents');
    const before = h.writes.length;
    for (let i = 0; i < 5; i++) await h.L.observed(items.map(item => ({ ...item })), null, 'recents');
    assert.equal(h.writes.length, before, 'identical refreshes are skipped');
    await h.L.observed([{ ...items[0], name: 'Renamed.pdf' }, items[1]], null, 'recents');
    assert.ok(h.writes.length > before, 'a change is still written');
    assert.equal(h.L.folder('root').find(entry => entry.id === 'a').name, 'Renamed.pdf');
    await h.L.observed([items[1], { ...items[0], name: 'Renamed.pdf' }], null, 'recents');
    const reordered = h.writes.length;
    assert.ok(reordered > before, 'a new view order is saved');
});

test('the open document is pushed once per outside event and never by its own saves', async () => {
    const h = harness({ outbox: [{ id: 'open' }], documents: [{ id: 'open' }] });
    h.host.active = 'open';
    // Its foreground save leaves the record as it is and reports back, like queueSave.
    h.host.syncActive = async () => { if (++h.host.syncActiveCalls < 20) await h.L.kick({ reason: 'after-save' }); };
    await h.start();
    assert.equal(h.host.syncActiveCalls, 1, 'launch');
    await h.L.kick({ reason: 'checkpoint' }); await h.clock.advance(60000);
    assert.equal(h.host.syncActiveCalls, 1, 'checkpoints and its own saves do not force a save');
    await h.L.kick({ reason: 'probe' }); await h.clock.advance(60000);
    assert.equal(h.host.syncActiveCalls, 2, 'returning to the app pushes it once');
    assert.equal(h.frames.length, 0);
});

test('a run that found the upload lease busy keeps the processor for its retry', async () => {
    const h = harness({ outbox: [{ id: 'other' }], documents: [{ id: 'other' }] });
    h.drive.status = 200;
    // Another upload takes the lease while the processor is loading.
    h.data.set(`leases\u0000${ACCOUNT}\u0000sync`, null);
    const store = h.store, withLease = store.withLease;
    let busyOnce = true;
    store.withLease = async (...args) => { if (busyOnce) { busyOnce = false; return { busy: true }; } return withLease(...args); };
    await h.start();
    assert.equal(h.workerRuns.length, 1);
    assert.equal(h.frames[0].removed, false, 'kept for the retry');
    await h.clock.advance(16000);
    assert.equal(h.workerRuns.length, 2);
    assert.equal(h.workerRuns[1].reused, true);
    assert.equal(h.op('other'), null);
    assert.equal(h.frames[0].removed, true, 'released once the outbox is empty');
});

test('uploads waiting for sign-in resume on the next authenticated Drive response', async () => {
    const h = harness({ outbox: [{ id: 'other' }], documents: [{ id: 'other' }] });
    h.host.signedIn = false;
    await h.start();
    h.L.networkAvailable(); await h.clock.advance(1000);
    assert.equal(h.frames.length, 0, 'still signed out');
    h.host.signedIn = true;
    h.L.networkAvailable(); await h.clock.advance(1000);
    assert.equal(h.frames.length, 1);
});
