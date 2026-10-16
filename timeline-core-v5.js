(function attachInhouseTimelineCore(root) {
    'use strict';

    const ARCHIVE_SCHEMA_VERSION = 3;
    const DEFAULT_CHECKPOINT_INTERVAL = 8;
    const MAX_ARCHIVE_ENTRIES = 200;
    const MAX_ARCHIVE_PAGES = 2000;
    const MAX_PAGE_ID_LENGTH = 256;
    const META_FIELDS = [
        'id', 'originId', 'ts', 'summary', 'kind', 'isMilestone',
        'schemaVersion', 'parentId', 'contentHash', 'deviceId'
    ];

    function cloneJson(value, fallback = null) {
        if (value === undefined) return fallback;
        try {
            return JSON.parse(JSON.stringify(value));
        } catch (error) {
            return fallback;
        }
    }

    function normalizeForStableJson(value, seen = new Set()) {
        if (value === null || typeof value !== 'object') return value;
        if (seen.has(value)) throw new TypeError('Timeline data must not contain cycles');
        seen.add(value);
        let normalized;
        if (Array.isArray(value)) {
            normalized = value.map(item => normalizeForStableJson(item, seen));
        } else {
            normalized = {};
            Object.keys(value).sort().forEach(key => {
                if (key === '__proto__' || key === 'prototype' || key === 'constructor') return;
                normalized[key] = normalizeForStableJson(value[key], seen);
            });
        }
        seen.delete(value);
        return normalized;
    }

    function stableStringify(value) {
        return JSON.stringify(normalizeForStableJson(value));
    }

    function hashString(value) {
        const input = String(value || '');
        let hash = 0x811c9dc5;
        for (let index = 0; index < input.length; index += 1) {
            const code = input.charCodeAt(index);
            hash ^= code & 0xff;
            hash = Math.imul(hash, 0x01000193);
            hash ^= code >>> 8;
            hash = Math.imul(hash, 0x01000193);
        }
        return (hash >>> 0).toString(36).padStart(7, '0');
    }

    function snapshotFromEntry(entry) {
        return {
            pages: cloneJson(Array.isArray(entry?.pages) ? entry.pages : [], []),
            calendarPageConfig: cloneJson(entry?.calendarPageConfig ?? null, null),
            exportName: String(entry?.exportName || '').slice(0, 220)
        };
    }

    function snapshotHash(snapshot) {
        return hashString(stableStringify(snapshot));
    }

    function copyMetadata(entry) {
        const metadata = {};
        META_FIELDS.forEach(field => {
            if (!Object.prototype.hasOwnProperty.call(entry || {}, field)) return;
            metadata[field] = cloneJson(entry[field], null);
        });
        const author = entry?.author && typeof entry.author === 'object' ? entry.author : {};
        metadata.author = {
            email: String(author.email || '').slice(0, 256),
            name: String(author.name || author.email || 'Unknown editor').slice(0, 160),
            photo: String(author.photo || '').slice(0, 1024)
        };
        metadata.schemaVersion = ARCHIVE_SCHEMA_VERSION;
        return metadata;
    }

    function pageIdentity(page) {
        const id = typeof page?.pageId === 'string' ? page.pageId : '';
        if (!id || id.length > MAX_PAGE_ID_LENGTH) return '';
        return id;
    }

    function snapshotPageMap(snapshot) {
        const pages = Array.isArray(snapshot?.pages) ? snapshot.pages : [];
        if (pages.length > MAX_ARCHIVE_PAGES) throw new RangeError('Timeline snapshot has too many pages');
        const map = new Map();
        const order = [];
        for (const page of pages) {
            const id = pageIdentity(page);
            if (!id || map.has(id)) return null;
            map.set(id, page);
            order.push(id);
        }
        return { map, order };
    }

    function arraysEqual(left, right) {
        return left.length === right.length && left.every((value, index) => value === right[index]);
    }

    function buildSnapshotDelta(previous, next) {
        const previousPages = snapshotPageMap(previous);
        const nextPages = snapshotPageMap(next);
        if (!previousPages || !nextPages) return null;

        const delta = {};
        if (!arraysEqual(previousPages.order, nextPages.order)) delta.order = nextPages.order.slice();

        const upserts = [];
        nextPages.order.forEach(id => {
            const nextPage = nextPages.map.get(id);
            const previousPage = previousPages.map.get(id);
            if (!previousPage || stableStringify(previousPage) !== stableStringify(nextPage)) {
                upserts.push([id, cloneJson(nextPage, {})]);
            }
        });
        if (upserts.length > 0) delta.upserts = upserts;

        const removed = previousPages.order.filter(id => !nextPages.map.has(id));
        if (removed.length > 0) delta.removed = removed;

        const documentPatch = {};
        if (stableStringify(previous.calendarPageConfig) !== stableStringify(next.calendarPageConfig)) {
            documentPatch.calendarPageConfig = cloneJson(next.calendarPageConfig, null);
        }
        if (previous.exportName !== next.exportName) documentPatch.exportName = next.exportName;
        if (Object.keys(documentPatch).length > 0) delta.document = documentPatch;
        return delta;
    }

    function createArchive(history, options = {}) {
        if (!Array.isArray(history)) throw new TypeError('Timeline history must be an array');
        if (history.length > MAX_ARCHIVE_ENTRIES) throw new RangeError('Timeline history has too many entries');
        const requestedInterval = Number(options.checkpointInterval);
        const checkpointInterval = Number.isFinite(requestedInterval)
            ? Math.max(2, Math.min(32, Math.floor(requestedInterval)))
            : DEFAULT_CHECKPOINT_INTERVAL;
        const records = [];
        let previousSnapshot = null;
        let previousHash = '';

        history.forEach((entry, index) => {
            const snapshot = snapshotFromEntry(entry);
            if (snapshot.pages.length > MAX_ARCHIVE_PAGES) {
                throw new RangeError('Timeline snapshot has too many pages');
            }
            const hash = snapshotHash(snapshot);
            const metadata = copyMetadata(entry);
            const forceCheckpoint = !previousSnapshot
                || index % checkpointInterval === 0
                || !!entry?.isMilestone;
            const delta = forceCheckpoint ? null : buildSnapshotDelta(previousSnapshot, snapshot);
            if (!delta) {
                records.push({
                    type: 'checkpoint',
                    meta: metadata,
                    hash,
                    snapshot
                });
            } else {
                records.push({
                    type: 'delta',
                    meta: metadata,
                    baseHash: previousHash,
                    hash,
                    delta
                });
            }
            previousSnapshot = snapshot;
            previousHash = hash;
        });

        return {
            schema: 'inhouse-timeline',
            schemaVersion: ARCHIVE_SCHEMA_VERSION,
            checkpointInterval,
            entries: records
        };
    }

    function assertSafeRecord(record, index) {
        if (!record || typeof record !== 'object' || Array.isArray(record)) {
            throw new TypeError(`Invalid timeline record at ${index}`);
        }
        if (record.type !== 'checkpoint' && record.type !== 'delta') {
            throw new TypeError(`Invalid timeline record type at ${index}`);
        }
        if (typeof record.hash !== 'string' || record.hash.length > 64) {
            throw new TypeError(`Invalid timeline record hash at ${index}`);
        }
    }

    function applySnapshotDelta(previous, delta) {
        if (!delta || typeof delta !== 'object' || Array.isArray(delta)) {
            throw new TypeError('Invalid timeline delta');
        }
        const previousPages = snapshotPageMap(previous);
        if (!previousPages) throw new TypeError('Timeline delta base has invalid page identities');
        const map = new Map(previousPages.order.map(id => [id, cloneJson(previousPages.map.get(id), {})]));

        if (delta.removed !== undefined) {
            if (!Array.isArray(delta.removed) || delta.removed.length > MAX_ARCHIVE_PAGES) {
                throw new TypeError('Invalid removed-page list');
            }
            delta.removed.forEach(id => {
                if (typeof id !== 'string' || id.length > MAX_PAGE_ID_LENGTH) throw new TypeError('Invalid removed page id');
                map.delete(id);
            });
        }

        if (delta.upserts !== undefined) {
            if (!Array.isArray(delta.upserts) || delta.upserts.length > MAX_ARCHIVE_PAGES) {
                throw new TypeError('Invalid page upsert list');
            }
            delta.upserts.forEach(pair => {
                if (!Array.isArray(pair) || pair.length !== 2) throw new TypeError('Invalid page upsert');
                const [id, page] = pair;
                if (typeof id !== 'string' || id.length > MAX_PAGE_ID_LENGTH || pageIdentity(page) !== id) {
                    throw new TypeError('Invalid page upsert identity');
                }
                map.set(id, cloneJson(page, {}));
            });
        }

        let order = previousPages.order.filter(id => map.has(id));
        if (delta.order !== undefined) {
            if (!Array.isArray(delta.order) || delta.order.length > MAX_ARCHIVE_PAGES) {
                throw new TypeError('Invalid timeline page order');
            }
            const seen = new Set();
            order = delta.order.map(id => {
                if (typeof id !== 'string' || id.length > MAX_PAGE_ID_LENGTH || seen.has(id) || !map.has(id)) {
                    throw new TypeError('Timeline page order is inconsistent');
                }
                seen.add(id);
                return id;
            });
            if (seen.size !== map.size) throw new TypeError('Timeline page order is incomplete');
        } else {
            map.forEach((_page, id) => {
                if (!order.includes(id)) order.push(id);
            });
        }

        const documentPatch = delta.document && typeof delta.document === 'object' && !Array.isArray(delta.document)
            ? delta.document
            : {};
        return {
            pages: order.map(id => map.get(id)),
            calendarPageConfig: Object.prototype.hasOwnProperty.call(documentPatch, 'calendarPageConfig')
                ? cloneJson(documentPatch.calendarPageConfig, null)
                : cloneJson(previous.calendarPageConfig, null),
            exportName: Object.prototype.hasOwnProperty.call(documentPatch, 'exportName')
                ? String(documentPatch.exportName || '').slice(0, 220)
                : previous.exportName
        };
    }

    function entryFromRecord(record, snapshot) {
        const metadata = copyMetadata(record.meta || {});
        return {
            ...metadata,
            calendarPageConfig: cloneJson(snapshot.calendarPageConfig, null),
            exportName: snapshot.exportName,
            pages: cloneJson(snapshot.pages, [])
        };
    }

    function materializeArchive(value) {
        if (Array.isArray(value)) return cloneJson(value, []);
        if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
        if (value.schema !== 'inhouse-timeline' || Number(value.schemaVersion) !== ARCHIVE_SCHEMA_VERSION) return null;
        if (!Array.isArray(value.entries) || value.entries.length > MAX_ARCHIVE_ENTRIES) {
            throw new RangeError('Invalid timeline archive length');
        }

        const history = [];
        let currentSnapshot = null;
        let currentHash = '';
        value.entries.forEach((record, index) => {
            assertSafeRecord(record, index);
            if (record.type === 'checkpoint') {
                if (!record.snapshot || typeof record.snapshot !== 'object' || Array.isArray(record.snapshot)) {
                    throw new TypeError(`Invalid timeline checkpoint at ${index}`);
                }
                currentSnapshot = {
                    pages: cloneJson(Array.isArray(record.snapshot.pages) ? record.snapshot.pages : [], []),
                    calendarPageConfig: cloneJson(record.snapshot.calendarPageConfig ?? null, null),
                    exportName: String(record.snapshot.exportName || '').slice(0, 220)
                };
                if (currentSnapshot.pages.length > MAX_ARCHIVE_PAGES) throw new RangeError('Timeline checkpoint has too many pages');
            } else {
                if (!currentSnapshot || record.baseHash !== currentHash) {
                    throw new Error(`Timeline delta chain is broken at ${index}`);
                }
                currentSnapshot = applySnapshotDelta(currentSnapshot, record.delta);
            }
            currentHash = snapshotHash(currentSnapshot);
            if (currentHash !== record.hash) throw new Error(`Timeline integrity check failed at ${index}`);
            history.push(entryFromRecord(record, currentSnapshot));
        });
        return history;
    }

    function estimateArchiveBytes(history, options = {}) {
        if (!Array.isArray(history) || history.length === 0) return 0;
        return stableStringify(createArchive(history, options)).length;
    }

    const api = Object.freeze({
        ARCHIVE_SCHEMA_VERSION,
        DEFAULT_CHECKPOINT_INTERVAL,
        createArchive,
        materializeArchive,
        estimateArchiveBytes,
        stableStringify,
        snapshotHash
    });

    root.InhouseTimelineCore = api;
    if (typeof module !== 'undefined' && module.exports) module.exports = api;
})(typeof globalThis !== 'undefined' ? globalThis : window);
