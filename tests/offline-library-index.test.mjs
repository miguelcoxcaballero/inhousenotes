import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';

const source = readFileSync(new URL('../offline-library-v5.js', import.meta.url), 'utf8');
const folderType = 'application/vnd.google-apps.folder';
const file = (id, name = `${id}.pdf`, extra = {}) => ({ id, name, mimeType: 'application/pdf', parents: ['root'], ...extra });
const directory = (id, name = id, extra = {}) => ({ id, name, mimeType: folderType, parents: ['root'], ...extra });

async function library(initial = {}) {
    const state = { entries: [], docs: [], views: {}, driveRoot: null, ...initial };
    const store = {
        async list(kind) { return kind === 'entries' ? state.entries : kind === 'document-index' ? state.docs : []; },
        async read(kind, account, key) { return key === 'drive-root' ? state.driveRoot : key === 'home-views' ? state.views : null; }
    };
    const sandbox = {
        URLSearchParams, location: { search: '?offline-worker=1' }, navigator: { onLine: false },
        localStorage: { getItem: () => 'index@example.test' }, sessionStorage: { getItem: () => null },
        document: { querySelectorAll: () => [] },
        InhouseOfflineStore: store, InhouseRuntimeCore: { CoalescingQueue: class {} }
    };
    sandbox.window = sandbox;
    vm.runInNewContext(source, sandbox);
    const api = sandbox.InhouseOfflineLibrary;
    await api.init({});
    return { api, state };
}
function card() {
    const attributes = new Map(), classes = new Set();
    return {
        attributes, classes, dataset: {}, querySelector: () => null,
        classList: { toggle(name, enabled) { enabled ? classes.add(name) : classes.delete(name); } },
        setAttribute(name, value) { attributes.set(name, value); },
        removeAttribute(name) { attributes.delete(name); }
    };
}
const ids = entries => Array.from(entries, entry => entry.id);

test('indexed directories preserve aliases, multiple parents, search and ordering', async () => {
    const { api } = await library({
        driveRoot: { remoteId: 'drive-root' },
        entries: [
            directory('local-parent', 'Parent', { remoteId: 'remote-parent', pending: true, parents: ['drive-root'] }),
            directory('remote-parent', 'Remote copy'),
            file('older', 'Alpha.pdf', { parents: ['remote-parent'], modifiedTime: '2026-10-01T00:00:00Z' }),
            file('newer', 'Beta.pdf', { parents: ['remote-parent', 'other', 'other'], modifiedTime: '2026-10-03T00:00:00Z' }),
            file('trashed', 'Hidden.pdf', { parents: ['remote-parent'], trashed: true }),
            directory('z', 'Zeta', { parents: ['other'] }), directory('a', 'Alpha', { parents: ['other'] })
        ]
    });
    assert.deepEqual(ids(api.folder('root')), ['local-parent']);
    assert.deepEqual(ids(api.folder('local-parent')), ['newer', 'older']);
    assert.deepEqual(ids(api.folder('local-parent', 'ALPHA')), ['older']);
    assert.equal(ids(api.folder('other')).filter(id => id === 'newer').length, 1);
    assert.deepEqual(ids(api.folder('other')).filter(id => id === 'a' || id === 'z'), ['a', 'z']);
    api.folder('local-parent').splice(0);
    assert.deepEqual(ids(api.folder('local-parent')), ['newer', 'older']);
});

test('document aliases retain direct-ID priority and offline decoration without losing pinned state', async () => {
    const first = { id: 'local-first', remoteId: 'remote-only', available: true, pinned: true };
    const direct = { id: 'remote-direct', available: false, pinned: false };
    const { api } = await library({
        entries: [directory('folder'), file('remote-only'), file('remote-direct')],
        docs: [first, { id: 'local-second', remoteId: 'remote-only', available: false },
            { id: 'local-third', remoteId: 'remote-direct', available: true }, direct]
    });
    assert.equal(api.record('remote-only'), first);
    assert.equal(api.record('remote-direct'), direct);
    const pinned = card(); api.decorate(pinned, 'remote-only');
    assert.equal(pinned.dataset.offlineAvailable, 'true');
    assert.equal(pinned.dataset.offlinePinned, 'true');
    assert.equal(pinned.classes.has('offline-unavailable'), false);
    const unavailable = card(); api.decorate(unavailable, 'remote-direct');
    assert.equal(unavailable.attributes.get('aria-disabled'), 'true');
    assert.equal(unavailable.classes.has('offline-unavailable'), true);
    const folder = card(); api.decorate(folder, 'folder');
    assert.deepEqual(folder.dataset, {});
});

test('Home resolves saved remote aliases in deterministic order and omits removed entries', async () => {
    const { api } = await library({
        entries: [file('local-doc', 'Offline.pdf', { remoteId: 'remote-doc' }), file('second'),
            file('removed', 'Removed.pdf', { trashed: true }), directory('dir')],
        views: { recents: ['second', 'remote-doc', 'removed', 'unknown'], starred: ['dir'], shared: [] }
    });
    const home = api.home();
    assert.deepEqual(ids(home.recents), ['local-doc', 'second']);
    assert.deepEqual(ids(home.starred), ['dir']);
    assert.deepEqual(ids(home.shared), []);
});

test('reload and logout replace catalogue indexes and document aliases atomically', async () => {
    const { api, state } = await library({
        entries: [file('moving', 'Old.pdf', { parents: ['before'] })],
        docs: [{ id: 'local-doc', remoteId: 'old-remote', available: true }]
    });
    state.entries = [file('moving', 'New.pdf', { parents: ['after'] })];
    state.docs = [{ id: 'local-doc', remoteId: 'new-remote', available: false }];
    await api.reload();
    assert.deepEqual(ids(api.folder('before')), []);
    assert.deepEqual(ids(api.folder('after', 'new')), ['moving']);
    assert.equal(api.record('old-remote'), undefined);
    assert.equal(api.record('new-remote').available, false);
    api.logout();
    assert.equal(api.ready(), false);
    assert.deepEqual(ids(api.folder('after')), []);
    assert.deepEqual(ids(api.home().recents), []);
    assert.equal(api.record('new-remote'), undefined);
});

test('ordering reflects a changed observed timestamp without reparsing stale values', async () => {
    const { api } = await library({ entries: [
        file('first', 'First.pdf', { modifiedTime: '2026-10-03T00:00:00Z' }),
        file('second', 'Second.pdf', { modifiedTime: '2026-10-01T00:00:00Z' })
    ] });
    const first = api.folder('root');
    assert.deepEqual(ids(first), ['first', 'second']);
    first[1].modifiedTime = '2026-10-04T00:00:00Z';
    assert.deepEqual(ids(api.folder('root')), ['second', 'first']);
});

test('recently opened documents do not swap when background uploads change modification times',async()=>{
 const {api,state}=await library({entries:[file('a','A.pdf',{modifiedTime:'2026-10-22T10:00:00Z'}),file('b','B.pdf',{modifiedTime:'2026-10-22T11:00:00Z'})],docs:[{id:'a',lastOpenedAt:200},{id:'b',lastOpenedAt:100}],views:{recents:['b','a']}});
 assert.deepEqual(ids(api.home().recents),['a','b']);
 assert.deepEqual(ids(api.recentItems([file('b','B.pdf',{modifiedTime:'2026-10-23T11:00:00Z'}),file('a') ])),['a','b']);
});
