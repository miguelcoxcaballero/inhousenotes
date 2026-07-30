/* Shared deterministic collaboration primitives for Inhouse Notes.
 *
 * This file deliberately has no DOM or network dependencies. The browser app
 * uses it for page-structure convergence and live-state hashes, while Node
 * tests exercise the same production code directly.
 */
(function exposeInhouseCollaborationCore(root, factory) {
    const api = factory();
    if (typeof module === 'object' && module.exports) module.exports = api;
    if (root && typeof root === 'object') Object.assign(root, api);
}(typeof globalThis !== 'undefined' ? globalThis : this, function createInhouseCollaborationCore() {
    'use strict';

    const IHN_STRUCTURE_SCHEMA_VERSION = 1;
    const IHN_FIELD_SCHEMA_VERSION = 1;
    const IHN_STRUCTURE_RANK_STEP = 1024;
    const IHN_STRUCTURE_MIN_GAP = 1e-7;

    function ihnHashString(value) {
        const text = String(value || '');
        let first = 0x811c9dc5;
        let second = 0x9e3779b9;
        for (let index = 0; index < text.length; index += 1) {
            const code = text.charCodeAt(index);
            first ^= code;
            first = Math.imul(first, 0x01000193);
            second ^= code + index;
            second = Math.imul(second, 0x85ebca6b);
        }
        return `${(first >>> 0).toString(16).padStart(8, '0')}${(second >>> 0).toString(16).padStart(8, '0')}`;
    }

    function ihnStableValue(value) {
        if (Array.isArray(value)) return value.map(ihnStableValue);
        if (!value || typeof value !== 'object') {
            if (typeof value === 'number' && !Number.isFinite(value)) return null;
            return value === undefined ? null : value;
        }
        const next = {};
        Object.keys(value).sort().forEach(key => {
            if (value[key] !== undefined) next[key] = ihnStableValue(value[key]);
        });
        return next;
    }

    function ihnStableStringify(value) {
        return JSON.stringify(ihnStableValue(value));
    }

    function ihnNormalizeCollabStamp(stamp) {
        return {
            clock: Math.max(0, Math.floor(Number(stamp?.clock) || 0)),
            actor: String(stamp?.actor || '')
        };
    }

    function ihnCompareCollabStamps(left, right) {
        const a = ihnNormalizeCollabStamp(left);
        const b = ihnNormalizeCollabStamp(right);
        if (a.clock !== b.clock) return a.clock < b.clock ? -1 : 1;
        if (a.actor === b.actor) return 0;
        return a.actor < b.actor ? -1 : 1;
    }

    function ihnMaxCollabStamp(left, right) {
        return ihnCompareCollabStamps(left, right) >= 0
            ? ihnNormalizeCollabStamp(left)
            : ihnNormalizeCollabStamp(right);
    }

    function ihnNormalizeStructureEntry(entry, fallbackRank = 0) {
        const rank = Number(entry?.rank);
        return {
            rank: Number.isFinite(rank) ? rank : fallbackRank,
            orderStamp: ihnNormalizeCollabStamp(entry?.orderStamp),
            createdStamp: ihnNormalizeCollabStamp(entry?.createdStamp)
        };
    }

    function ihnNormalizePageIdList(pageIds) {
        const seen = new Set();
        const result = [];
        for (const rawId of Array.isArray(pageIds) ? pageIds : []) {
            const pageId = String(rawId || '');
            if (!pageId || seen.has(pageId)) continue;
            seen.add(pageId);
            result.push(pageId);
        }
        return result;
    }

    function ihnNormalizeStructureMeta(meta, pageIds = []) {
        const normalized = {
            v: IHN_STRUCTURE_SCHEMA_VERSION,
            clock: Math.max(0, Math.floor(Number(meta?.clock) || 0)),
            entries: {},
            tombstones: {}
        };
        const rawEntries = meta?.entries && typeof meta.entries === 'object' ? meta.entries : {};
        Object.keys(rawEntries).sort().forEach((pageId, index) => {
            if (!pageId) return;
            const entry = ihnNormalizeStructureEntry(rawEntries[pageId], index * IHN_STRUCTURE_RANK_STEP);
            normalized.entries[pageId] = entry;
            normalized.clock = Math.max(
                normalized.clock,
                entry.orderStamp.clock,
                entry.createdStamp.clock
            );
        });
        const rawTombstones = meta?.tombstones && typeof meta.tombstones === 'object'
            ? meta.tombstones
            : {};
        Object.keys(rawTombstones).sort().forEach(pageId => {
            if (!pageId) return;
            const stamp = ihnNormalizeCollabStamp(rawTombstones[pageId]);
            normalized.tombstones[pageId] = stamp;
            normalized.clock = Math.max(normalized.clock, stamp.clock);
        });
        ihnNormalizePageIdList(pageIds).forEach((pageId, index) => {
            if (!normalized.entries[pageId]) {
                normalized.entries[pageId] = {
                    rank: index * IHN_STRUCTURE_RANK_STEP,
                    orderStamp: { clock: 0, actor: '' },
                    createdStamp: { clock: 0, actor: '' }
                };
            }
        });
        return normalized;
    }

    function ihnCloneStructureMeta(meta, pageIds = []) {
        return JSON.parse(JSON.stringify(ihnNormalizeStructureMeta(meta, pageIds)));
    }

    function ihnNextStructureStamp(meta, actor, now = Date.now()) {
        const currentClock = Math.max(0, Math.floor(Number(meta?.clock) || 0));
        const wallClock = Math.max(0, Math.floor(Number(now) || 0));
        const stamp = {
            clock: Math.max(currentClock + 1, wallClock),
            actor: String(actor || '')
        };
        meta.clock = stamp.clock;
        return stamp;
    }

    function ihnStructureRankForIndex(meta, pageIds, pageId, index) {
        const ids = ihnNormalizePageIdList(pageIds);
        const safeIndex = Math.max(0, Math.min(ids.length - 1, Number(index) || 0));
        const previousId = safeIndex > 0 ? ids[safeIndex - 1] : '';
        const nextId = safeIndex < ids.length - 1 ? ids[safeIndex + 1] : '';
        const previousRank = previousId && previousId !== pageId
            ? Number(meta.entries[previousId]?.rank)
            : NaN;
        const nextRank = nextId && nextId !== pageId
            ? Number(meta.entries[nextId]?.rank)
            : NaN;
        if (Number.isFinite(previousRank) && Number.isFinite(nextRank)) {
            const gap = nextRank - previousRank;
            if (gap > IHN_STRUCTURE_MIN_GAP) return previousRank + gap / 2;
            return NaN;
        }
        if (Number.isFinite(previousRank)) return previousRank + IHN_STRUCTURE_RANK_STEP;
        if (Number.isFinite(nextRank)) return nextRank - IHN_STRUCTURE_RANK_STEP;
        return 0;
    }

    function ihnRebalanceStructureRanks(meta, pageIds, stamp) {
        ihnNormalizePageIdList(pageIds).forEach((pageId, index) => {
            const current = ihnNormalizeStructureEntry(
                meta.entries[pageId],
                index * IHN_STRUCTURE_RANK_STEP
            );
            current.rank = index * IHN_STRUCTURE_RANK_STEP;
            current.orderStamp = ihnNormalizeCollabStamp(stamp);
            meta.entries[pageId] = current;
        });
    }

    function ihnRecordPageAdded(meta, pageIds, pageId, index, actor, now = Date.now()) {
        const ids = ihnNormalizePageIdList(pageIds);
        const next = ihnNormalizeStructureMeta(meta, ids);
        const stamp = ihnNextStructureStamp(next, actor, now);
        let rank = ihnStructureRankForIndex(next, ids, pageId, index);
        if (!Number.isFinite(rank)) {
            ihnRebalanceStructureRanks(next, ids, stamp);
            rank = Number(next.entries[pageId]?.rank) || 0;
        }
        next.entries[pageId] = {
            rank,
            orderStamp: stamp,
            createdStamp: stamp
        };
        delete next.tombstones[pageId];
        return next;
    }

    function ihnRecordPageMoved(meta, pageIds, pageId, index, actor, now = Date.now()) {
        const ids = ihnNormalizePageIdList(pageIds);
        const next = ihnNormalizeStructureMeta(meta, ids);
        const stamp = ihnNextStructureStamp(next, actor, now);
        let rank = ihnStructureRankForIndex(next, ids, pageId, index);
        if (!Number.isFinite(rank)) {
            ihnRebalanceStructureRanks(next, ids, stamp);
            rank = Number(next.entries[pageId]?.rank) || 0;
        }
        const previous = ihnNormalizeStructureEntry(next.entries[pageId], rank);
        next.entries[pageId] = {
            ...previous,
            rank,
            orderStamp: stamp
        };
        return next;
    }

    function ihnRecordPageDeleted(meta, remainingPageIds, pageId, actor, now = Date.now()) {
        const ids = [...ihnNormalizePageIdList(remainingPageIds), String(pageId || '')].filter(Boolean);
        const next = ihnNormalizeStructureMeta(meta, ids);
        const stamp = ihnNextStructureStamp(next, actor, now);
        next.tombstones[String(pageId || '')] = stamp;
        return next;
    }

    function ihnRecordStructureReplacement(meta, previousPageIds, nextPageIds, actor, now = Date.now()) {
        const previousIds = ihnNormalizePageIdList(previousPageIds);
        const nextIds = ihnNormalizePageIdList(nextPageIds);
        const next = ihnNormalizeStructureMeta(meta, [...previousIds, ...nextIds]);
        const stamp = ihnNextStructureStamp(next, actor, now);
        const previousSet = new Set(previousIds);
        const nextSet = new Set(nextIds);
        previousIds.forEach(pageId => {
            if (!nextSet.has(pageId)) next.tombstones[pageId] = stamp;
        });
        nextIds.forEach((pageId, index) => {
            const previous = ihnNormalizeStructureEntry(
                next.entries[pageId],
                index * IHN_STRUCTURE_RANK_STEP
            );
            const restoresPage = !previousSet.has(pageId)
                || ihnIsPageTombstoned(next, pageId);
            next.entries[pageId] = {
                rank: index * IHN_STRUCTURE_RANK_STEP,
                orderStamp: stamp,
                createdStamp: restoresPage || !previous.createdStamp.clock
                    ? stamp
                    : previous.createdStamp
            };
            delete next.tombstones[pageId];
        });
        return next;
    }

    function ihnChooseStructureEntry(left, right, fallbackRank = 0) {
        if (!left) return ihnNormalizeStructureEntry(right, fallbackRank);
        if (!right) return ihnNormalizeStructureEntry(left, fallbackRank);
        const a = ihnNormalizeStructureEntry(left, fallbackRank);
        const b = ihnNormalizeStructureEntry(right, fallbackRank);
        const orderCompare = ihnCompareCollabStamps(a.orderStamp, b.orderStamp);
        let winner;
        if (orderCompare > 0) winner = a;
        else if (orderCompare < 0) winner = b;
        else {
            const aKey = `${a.rank}:${ihnStableStringify(a)}`;
            const bKey = `${b.rank}:${ihnStableStringify(b)}`;
            winner = aKey >= bKey ? a : b;
        }
        return {
            rank: winner.rank,
            orderStamp: ihnMaxCollabStamp(a.orderStamp, b.orderStamp),
            createdStamp: ihnMaxCollabStamp(a.createdStamp, b.createdStamp)
        };
    }

    function ihnIsPageTombstoned(meta, pageId) {
        const normalized = ihnNormalizeStructureMeta(meta);
        const tombstone = normalized.tombstones[pageId];
        if (!tombstone) return false;
        const created = normalized.entries[pageId]?.createdStamp || { clock: 0, actor: '' };
        return ihnCompareCollabStamps(tombstone, created) >= 0;
    }

    function ihnCanonicalStructurePayload(meta, pageIds = null) {
        const normalized = ihnNormalizeStructureMeta(meta, pageIds || []);
        const ids = pageIds
            ? new Set(ihnNormalizePageIdList(pageIds))
            : new Set([
                ...Object.keys(normalized.entries),
                ...Object.keys(normalized.tombstones)
            ]);
        const entries = {};
        const tombstones = {};
        [...ids].sort().forEach(pageId => {
            if (normalized.entries[pageId]) entries[pageId] = normalized.entries[pageId];
            if (normalized.tombstones[pageId]) tombstones[pageId] = normalized.tombstones[pageId];
        });
        return { v: IHN_STRUCTURE_SCHEMA_VERSION, entries, tombstones };
    }

    function ihnStructureHash(meta, pageIds = null) {
        return ihnHashString(ihnStableStringify(ihnCanonicalStructurePayload(meta, pageIds)));
    }

    function ihnMergeStructureMeta(localMeta, remoteMeta, localPageIds = [], remotePageIds = []) {
        const local = ihnNormalizeStructureMeta(localMeta, localPageIds);
        const remote = ihnNormalizeStructureMeta(remoteMeta, remotePageIds);
        const merged = {
            v: IHN_STRUCTURE_SCHEMA_VERSION,
            clock: Math.max(local.clock, remote.clock),
            entries: {},
            tombstones: {}
        };
        const allIds = new Set([
            ...Object.keys(local.entries),
            ...Object.keys(remote.entries),
            ...Object.keys(local.tombstones),
            ...Object.keys(remote.tombstones),
            ...ihnNormalizePageIdList(localPageIds),
            ...ihnNormalizePageIdList(remotePageIds)
        ]);
        [...allIds].sort().forEach((pageId, index) => {
            const entry = ihnChooseStructureEntry(
                local.entries[pageId],
                remote.entries[pageId],
                index * IHN_STRUCTURE_RANK_STEP
            );
            if (entry) merged.entries[pageId] = entry;
            const localTombstone = local.tombstones[pageId];
            const remoteTombstone = remote.tombstones[pageId];
            if (localTombstone || remoteTombstone) {
                merged.tombstones[pageId] = ihnMaxCollabStamp(localTombstone, remoteTombstone);
            }
        });
        const orderedPageIds = [...allIds]
            .filter(pageId => merged.entries[pageId] && !ihnIsPageTombstoned(merged, pageId))
            .sort((leftId, rightId) => {
                const left = merged.entries[leftId];
                const right = merged.entries[rightId];
                if (left.rank !== right.rank) return left.rank < right.rank ? -1 : 1;
                const stampCompare = ihnCompareCollabStamps(left.orderStamp, right.orderStamp);
                if (stampCompare !== 0) return stampCompare;
                return leftId < rightId ? -1 : (leftId > rightId ? 1 : 0);
            });
        // Two replicas can each delete a different page while still obeying
        // the UI rule that a document keeps one page. Their delete-wins union
        // would otherwise yield an unusable zero-page document. Project one
        // deterministic fallback into the visible order, but keep its causal
        // tombstone in the join. Deleting the tombstone here makes the merge
        // non-associative and can resurrect different pages depending on
        // snapshot arrival order. The application may explicitly revive this
        // projected page later as a new causal write.
        let recoveredFallbackPageId = '';
        if (orderedPageIds.length === 0 && allIds.size > 0) {
            const fallbackPageId = [...allIds]
                .filter(pageId => merged.entries[pageId])
                .sort()[0];
            if (fallbackPageId) {
                orderedPageIds.push(fallbackPageId);
                recoveredFallbackPageId = fallbackPageId;
            }
        }
        const localHash = ihnStructureHash(local);
        const remoteHash = ihnStructureHash(remote);
        const mergedHash = ihnStructureHash(merged);
        return {
            meta: merged,
            orderedPageIds,
            recoveredFallbackPageId,
            tombstonedPageIds: [...allIds].filter(pageId => ihnIsPageTombstoned(merged, pageId)).sort(),
            localContributed: mergedHash !== remoteHash,
            remoteContributed: mergedHash !== localHash,
            hash: mergedHash,
            localHash,
            remoteHash
        };
    }

    function ihnGetItemStamp(item) {
        return ihnNormalizeCollabStamp(
            item?.syncStamp
            || item?._syncStamp
            || (item?.updatedClock || item?.updatedBy
                ? { clock: item.updatedClock, actor: item.updatedBy }
                : null)
        );
    }

    function ihnChooseConcurrentItem(localItem, remoteItem, localFingerprint = '', remoteFingerprint = '') {
        if (!localItem) return remoteItem;
        if (!remoteItem) return localItem;
        const stampCompare = ihnCompareCollabStamps(
            ihnGetItemStamp(localItem),
            ihnGetItemStamp(remoteItem)
        );
        if (stampCompare > 0) return localItem;
        if (stampCompare < 0) return remoteItem;
        const localKey = String(localFingerprint || ihnStableStringify(localItem));
        const remoteKey = String(remoteFingerprint || ihnStableStringify(remoteItem));
        return localKey >= remoteKey ? localItem : remoteItem;
    }

    function ihnNormalizeDeletionStamps(stamps, legacyDeletedIds = []) {
        const collected = new Map();
        const record = (rawId, rawStamp = null) => {
            const id = String(rawId || '');
            if (!id) return;
            const stamp = ihnNormalizeCollabStamp(rawStamp?.stamp || rawStamp);
            const previous = collected.get(id);
            collected.set(id, previous ? ihnMaxCollabStamp(previous, stamp) : stamp);
        };
        const consume = value => {
            if (!value) return;
            if (value instanceof Map) {
                value.forEach((stamp, id) => record(id, stamp));
                return;
            }
            if (Array.isArray(value)) {
                value.forEach(entry => {
                    if (entry && typeof entry === 'object') {
                        record(entry.id, entry.stamp || entry);
                    } else {
                        // Legacy deletedStrokeIds contained only IDs. Lamport
                        // zero lets any later causal undo recreate the item.
                        record(entry, { clock: 0, actor: '' });
                    }
                });
                return;
            }
            if (typeof value === 'object') {
                Object.keys(value).forEach(id => record(id, value[id]));
            }
        };
        consume(stamps);
        consume(legacyDeletedIds);
        const normalized = {};
        [...collected.keys()].sort().forEach(id => {
            normalized[id] = ihnNormalizeCollabStamp(collected.get(id));
        });
        return normalized;
    }

    function ihnMergeDeletionStamps(localStamps, remoteStamps) {
        const local = ihnNormalizeDeletionStamps(localStamps);
        const remote = ihnNormalizeDeletionStamps(remoteStamps);
        const merged = {};
        const ids = new Set([...Object.keys(local), ...Object.keys(remote)]);
        [...ids].sort().forEach(id => {
            merged[id] = ihnMaxCollabStamp(local[id], remote[id]);
        });
        return merged;
    }

    function ihnDeletionWinsItem(deletionStamps, item, itemId = '') {
        const id = String(itemId || item?.id || '');
        if (!id) return false;
        const deletion = ihnNormalizeDeletionStamps(deletionStamps)[id];
        if (!deletion) return false;
        return ihnCompareCollabStamps(deletion, ihnGetItemStamp(item)) >= 0;
    }

    function ihnCloneFieldValue(value) {
        if (value === undefined) return null;
        return JSON.parse(JSON.stringify(ihnStableValue(value)));
    }

    function ihnNormalizeFieldMeta(meta) {
        const normalized = {
            v: IHN_FIELD_SCHEMA_VERSION,
            clock: Math.max(0, Math.floor(Number(meta?.clock) || 0)),
            fields: {}
        };
        const rawFields = meta?.fields && typeof meta.fields === 'object'
            ? meta.fields
            : {};
        Object.keys(rawFields).sort().forEach(key => {
            if (!key) return;
            const rawRecord = rawFields[key];
            if (!rawRecord || typeof rawRecord !== 'object'
                || !Object.prototype.hasOwnProperty.call(rawRecord, 'value')) return;
            const stamp = ihnNormalizeCollabStamp(rawRecord.stamp);
            normalized.fields[key] = {
                value: ihnCloneFieldValue(rawRecord.value),
                stamp
            };
            normalized.clock = Math.max(normalized.clock, stamp.clock);
        });
        return normalized;
    }

    function ihnSeedFieldValue(meta, key, value) {
        const normalized = ihnNormalizeFieldMeta(meta);
        const fieldKey = String(key || '');
        if (!fieldKey || normalized.fields[fieldKey]) return normalized;
        normalized.fields[fieldKey] = {
            value: ihnCloneFieldValue(value),
            stamp: { clock: 0, actor: '' }
        };
        return normalized;
    }

    function ihnRecordFieldValue(meta, key, value, actor, now = Date.now()) {
        const normalized = ihnNormalizeFieldMeta(meta);
        const fieldKey = String(key || '');
        if (!fieldKey) return normalized;
        const stamp = {
            clock: Math.max(
                normalized.clock + 1,
                Math.max(0, Math.floor(Number(now) || 0))
            ),
            actor: String(actor || '')
        };
        normalized.clock = stamp.clock;
        normalized.fields[fieldKey] = {
            value: ihnCloneFieldValue(value),
            stamp
        };
        return normalized;
    }

    function ihnChooseFieldRecord(localRecord, remoteRecord) {
        if (!localRecord) {
            return remoteRecord ? {
                value: ihnCloneFieldValue(remoteRecord.value),
                stamp: ihnNormalizeCollabStamp(remoteRecord.stamp)
            } : null;
        }
        if (!remoteRecord) {
            return {
                value: ihnCloneFieldValue(localRecord.value),
                stamp: ihnNormalizeCollabStamp(localRecord.stamp)
            };
        }
        const stampCompare = ihnCompareCollabStamps(localRecord.stamp, remoteRecord.stamp);
        let winner = stampCompare > 0
            ? localRecord
            : (stampCompare < 0 ? remoteRecord : null);
        if (!winner) {
            const localKey = ihnStableStringify(localRecord.value);
            const remoteKey = ihnStableStringify(remoteRecord.value);
            winner = localKey >= remoteKey ? localRecord : remoteRecord;
        }
        return {
            value: ihnCloneFieldValue(winner.value),
            stamp: ihnMaxCollabStamp(localRecord.stamp, remoteRecord.stamp)
        };
    }

    function ihnMergeFieldMeta(localMeta, remoteMeta) {
        const local = ihnNormalizeFieldMeta(localMeta);
        const remote = ihnNormalizeFieldMeta(remoteMeta);
        const merged = {
            v: IHN_FIELD_SCHEMA_VERSION,
            clock: Math.max(local.clock, remote.clock),
            fields: {}
        };
        const keys = new Set([
            ...Object.keys(local.fields),
            ...Object.keys(remote.fields)
        ]);
        [...keys].sort().forEach(key => {
            const record = ihnChooseFieldRecord(local.fields[key], remote.fields[key]);
            if (record) merged.fields[key] = record;
        });
        return merged;
    }

    function ihnResolveFieldValue(meta, key, fallback = null) {
        const normalized = ihnNormalizeFieldMeta(meta);
        const record = normalized.fields[String(key || '')];
        return record
            ? ihnCloneFieldValue(record.value)
            : ihnCloneFieldValue(fallback);
    }

    function ihnFieldMetaHash(meta) {
        const normalized = ihnNormalizeFieldMeta(meta);
        // The aggregate clock is only an allocation aid. Field stamps and
        // values contain all semantic state and are what replicas converge on.
        return ihnHashString(ihnStableStringify({
            v: IHN_FIELD_SCHEMA_VERSION,
            fields: normalized.fields
        }));
    }

    function ihnCanonicalContentItem(item) {
        if (!item || typeof item !== 'object') return item;
        const next = { ...item };
        delete next.preview;
        delete next.needsRedraw;
        delete next.unloaded;
        delete next.pendingUnload;
        return next;
    }

    function ihnCanonicalLivePage(page) {
        const strokes = (Array.isArray(page?.strokes) ? page.strokes : [])
            .map(ihnCanonicalContentItem)
            .sort((left, right) => {
                const a = String(left?.id || ihnStableStringify(left));
                const b = String(right?.id || ihnStableStringify(right));
                return a < b ? -1 : (a > b ? 1 : 0);
            });
        const images = (Array.isArray(page?.images) ? page.images : [])
            .map(ihnCanonicalContentItem)
            .sort((left, right) => {
                const a = String(left?.id || ihnStableStringify(left));
                const b = String(right?.id || ihnStableStringify(right));
                return a < b ? -1 : (a > b ? 1 : 0);
            });
        const sidePanel = page?.sidePanel
            ? {
                title: String(page.sidePanel.title || ''),
                dateKeys: Array.isArray(page.sidePanel.dateKeys)
                    ? [...page.sidePanel.dateKeys].map(String)
                    : null
            }
            : null;
        return {
            pageId: String(page?.pageId || ''),
            strokes,
            images,
            deletedStrokeIds: [...new Set(
                (Array.isArray(page?.deletedStrokeIds) ? page.deletedStrokeIds : [])
                    .filter(Boolean)
                    .map(String)
            )].sort(),
            deletedStrokeStamps: ihnNormalizeDeletionStamps(
                page?.deletedStrokeStamps,
                page?.deletedStrokeIds
            ),
            backgroundSource: String(page?.backgroundSource || 'template'),
            templateKind: String(page?.templateKind || ''),
            pageWidth: Number(page?.pageWidth) || 0,
            pageHeight: Number(page?.pageHeight) || 0,
            sidePanel
        };
    }

    function ihnCanonicalDocumentHash(pages, structure, calendarPageConfig, exportName, fieldMeta = null) {
        const canonicalPages = (Array.isArray(pages) ? pages : []).map(ihnCanonicalLivePage);
        return ihnHashString(ihnStableStringify({
            pages: canonicalPages,
            // Keep tombstones for pages that are no longer materialized. They
            // are what prevents an older Drive/P2P snapshot resurrecting a
            // deleted page later.
            structure: ihnCanonicalStructurePayload(structure),
            fields: ihnNormalizeFieldMeta(fieldMeta).fields,
            calendarPageConfig: calendarPageConfig || null,
            exportName: String(exportName || '')
        }));
    }

    function ihnComputeReconnectDelay(failures, peerId = '') {
        const attempt = Math.max(0, Math.min(6, Math.floor(Number(failures) || 0)));
        const base = Math.min(30_000, 1000 * (2 ** attempt));
        const jitterSeed = parseInt(ihnHashString(`${peerId}:${attempt}`).slice(0, 8), 16) / 0xffffffff;
        return Math.round(base * (0.8 + jitterSeed * 0.4));
    }

    function ihnChooseMaterializableFallbackPageId(
        recoveredFallbackPageId,
        localPageIds = [],
        remotePageIds = []
    ) {
        const recoveredId = String(recoveredFallbackPageId || '');
        const materializableIds = [...new Set([
            ...(Array.isArray(localPageIds) ? localPageIds : []),
            ...(Array.isArray(remotePageIds) ? remotePageIds : [])
        ].filter(Boolean).map(String))].sort();
        if (recoveredId && materializableIds.includes(recoveredId)) return recoveredId;
        return materializableIds[0] || recoveredId;
    }

    return {
        IHN_STRUCTURE_SCHEMA_VERSION,
        IHN_FIELD_SCHEMA_VERSION,
        ihnHashString,
        ihnStableStringify,
        ihnNormalizeCollabStamp,
        ihnCompareCollabStamps,
        ihnNormalizeStructureMeta,
        ihnCloneStructureMeta,
        ihnRecordPageAdded,
        ihnRecordPageMoved,
        ihnRecordPageDeleted,
        ihnRecordStructureReplacement,
        ihnIsPageTombstoned,
        ihnStructureHash,
        ihnMergeStructureMeta,
        ihnChooseConcurrentItem,
        ihnNormalizeDeletionStamps,
        ihnMergeDeletionStamps,
        ihnDeletionWinsItem,
        ihnNormalizeFieldMeta,
        ihnSeedFieldValue,
        ihnRecordFieldValue,
        ihnMergeFieldMeta,
        ihnResolveFieldValue,
        ihnFieldMetaHash,
        ihnCanonicalDocumentHash,
        ihnComputeReconnectDelay,
        ihnChooseMaterializableFallbackPageId
    };
}));
