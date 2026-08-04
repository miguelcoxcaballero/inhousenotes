(function installCollaborationChaosHarness(root) {
    'use strict';

    let actor = '';
    let room = '';
    let channel = null;
    let online = true;
    let logicalClock = 0;
    let received = 0;
    let state = null;

    function clone(value) {
        return JSON.parse(JSON.stringify(value));
    }

    function basePage(pageId) {
        return {
            pageId,
            strokes: [],
            deletedStrokeStamps: {},
            backgroundSource: 'template',
            templateKind: 'default',
            pageWidth: 210,
            pageHeight: 297,
            sidePanel: null
        };
    }

    function makeInitialState() {
        const order = ['page-a', 'page-b', 'page-c'];
        return {
            order,
            structure: root.ihnNormalizeStructureMeta(null, order),
            pages: Object.fromEntries(order.map(pageId => [pageId, basePage(pageId)])),
            fields: root.ihnNormalizeFieldMeta(null)
        };
    }

    function mergePage(localPage, remotePage) {
        if (!localPage) return clone(remotePage);
        if (!remotePage) return clone(localPage);
        const deletedStrokeStamps = root.ihnMergeDeletionStamps(
            localPage.deletedStrokeStamps,
            remotePage.deletedStrokeStamps
        );
        const localById = new Map((localPage.strokes || []).map(stroke => [stroke.id, stroke]));
        const remoteById = new Map((remotePage.strokes || []).map(stroke => [stroke.id, stroke]));
        const strokes = [...new Set([...localById.keys(), ...remoteById.keys()])]
            .sort()
            .map(id => root.ihnChooseConcurrentItem(localById.get(id), remoteById.get(id)))
            .filter(stroke => stroke && !root.ihnDeletionWinsItem(deletedStrokeStamps, stroke));
        return {
            ...clone(root.ihnChooseConcurrentItem(localPage, remotePage)),
            pageId: localPage.pageId || remotePage.pageId,
            strokes: clone(strokes),
            deletedStrokeStamps
        };
    }

    function mergeSnapshot(remote) {
        if (!remote || typeof remote !== 'object') return;
        const structureResult = root.ihnMergeStructureMeta(
            state.structure,
            remote.structure,
            state.order,
            remote.order
        );
        const pageIds = new Set([...Object.keys(state.pages), ...Object.keys(remote.pages || {})]);
        const mergedPages = {};
        pageIds.forEach(pageId => {
            const merged = mergePage(state.pages[pageId], remote.pages?.[pageId]);
            if (merged) mergedPages[pageId] = merged;
        });
        state = {
            order: structureResult.orderedPageIds,
            structure: structureResult.meta,
            pages: mergedPages,
            fields: root.ihnMergeFieldMeta(state.fields, remote.fields)
        };
        logicalClock = Math.max(logicalClock, Number(remote.clock) || 0);
        received += 1;
    }

    function envelope() {
        return {
            actor,
            clock: logicalClock,
            order: clone(state.order),
            structure: clone(state.structure),
            pages: clone(state.pages),
            fields: clone(state.fields)
        };
    }

    function handleMessage(event) {
        if (!online || event.data?.actor === actor) return;
        const previousHash = snapshot().hash;
        mergeSnapshot(event.data);
        if (snapshot().hash !== previousHash) queueMicrotask(() => publish());
    }

    function connect() {
        if (!online || !room || channel) return;
        channel = new BroadcastChannel(`ihn-e2e-chaos:${room}`);
        channel.onmessage = handleMessage;
    }

    function disconnect() {
        try {
            channel?.close();
        } catch (error) {}
        channel = null;
    }

    function publish(snapshot = envelope(), delayMs = 0) {
        const send = () => {
            if (online && channel) channel.postMessage(snapshot);
        };
        if (delayMs > 0) setTimeout(send, delayMs);
        else send();
    }

    function tick() {
        logicalClock += 1;
        return logicalClock;
    }

    function visiblePages() {
        return state.order.map(pageId => state.pages[pageId]).filter(Boolean);
    }

    function snapshot() {
        const pages = visiblePages();
        const exportName = root.ihnResolveFieldValue(state.fields, 'exportName', 'Document');
        return {
            actor,
            online,
            received,
            order: clone(state.order),
            pages: clone(pages),
            exportName,
            hash: root.ihnCanonicalDocumentHash(pages, state.structure, null, exportName, state.fields)
        };
    }

    root.__CHAOS__ = Object.freeze({
        init(nextActor, nextRoom) {
            actor = String(nextActor || 'unknown');
            room = String(nextRoom || 'default');
            logicalClock = 0;
            received = 0;
            online = true;
            state = makeInitialState();
            disconnect();
            connect();
            publish();
            return snapshot();
        },
        addStroke(pageId, strokeId) {
            const clock = tick();
            const page = state.pages[pageId];
            if (!page) throw new Error('Unknown page');
            page.strokes.push({
                id: String(strokeId),
                syncStamp: { clock, actor },
                tool: 'pen',
                points: [{ x: clock, y: clock + 1 }]
            });
            publish();
            return snapshot();
        },
        eraseStroke(pageId, strokeId) {
            const clock = tick();
            const page = state.pages[pageId];
            if (!page) throw new Error('Unknown page');
            page.deletedStrokeStamps[String(strokeId)] = { clock, actor };
            page.strokes = page.strokes.filter(stroke => stroke.id !== String(strokeId));
            publish();
            return snapshot();
        },
        movePage(pageId, index) {
            const clock = tick();
            const order = state.order.filter(id => id !== pageId);
            order.splice(Math.max(0, Math.min(order.length, Number(index) || 0)), 0, pageId);
            state.structure = root.ihnRecordPageMoved(state.structure, order, pageId, index, actor, clock);
            state.order = order;
            publish();
            return snapshot();
        },
        deletePage(pageId) {
            const clock = tick();
            const order = state.order.filter(id => id !== pageId);
            state.structure = root.ihnRecordPageDeleted(state.structure, order, pageId, actor, clock);
            state.order = order;
            publish();
            return snapshot();
        },
        rename(value) {
            const clock = tick();
            state.fields = root.ihnRecordFieldValue(state.fields, 'exportName', String(value), actor, clock);
            publish();
            return snapshot();
        },
        setOnline(value) {
            online = !!value;
            if (!online) disconnect();
            else {
                connect();
                publish();
            }
            return snapshot();
        },
        scheduleStaleFallback(delayMs = 500) {
            const stale = envelope();
            setTimeout(() => {
                if (online && channel) channel.postMessage({ ...stale, transport: 'slow-drive-fallback' });
            }, Math.max(0, Number(delayMs) || 0));
        },
        publish,
        snapshot
    });
})(window);
