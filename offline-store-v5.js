/* Durable per-account library. A document and its outbox revision commit together. */
(function (root) {
    'use strict';
    const DB_NAME = 'inhouse-offline-v1';
    const stores = ['entries', 'documents', 'outbox', 'settings', 'leases', 'recoveries', 'document-index'];
    let opening;
    function open() {
        if (!opening) opening = new Promise((resolve, reject) => {
            const request = indexedDB.open(DB_NAME, 2);
            request.onupgradeneeded = () => {
                for (const name of stores) {
                    if (request.result.objectStoreNames.contains(name)) continue;
                    const store=request.result.createObjectStore(name,{keyPath:'key'});
                    store.createIndex('account','account');
                }
                const tx=request.transaction;
                tx.objectStore('documents').openCursor().onsuccess=event=>{
                    const cursor=event.target.result;if(!cursor)return;
                    tx.objectStore('document-index').put(documentInfo(cursor.value));cursor.continue();
                };
            };
            request.onerror = () => { opening = null; reject(request.error); };
            request.onblocked = () => { opening = null; reject(new Error('Offline storage is blocked by another window')); };
            request.onsuccess = () => { request.result.onversionchange = () => { request.result.close(); opening = null; }; resolve(request.result); };
        });
        return opening;
    }
    // List metadata without cloning every PDF and every stroke into the home UI.
    function documentInfo(doc) {
        const {blob,snapshot,originalBlob,preparedBlob,analysis,...info}=doc;
        return {...info,available:!!(blob?.size||snapshot?.pages?.length)};
    }
    const key = (account, id) => `${account}\u0000${id}`;
    async function transaction(names, mode, work) {
        const db = await open();
        return new Promise((resolve, reject) => {
            const tx = db.transaction(names.includes('documents')&&!names.includes('document-index')?[...names,'document-index']:names, mode);
            let result;
            const api = Object.fromEntries([...tx.objectStoreNames].map(name => [name, tx.objectStore(name)]));
            tx.oncomplete = () => resolve(result);
            tx.onabort = tx.onerror = () => reject(tx.error || new Error('Offline transaction failed'));
            try { work(api, value => { result = value; }, tx); } catch (error) { tx.abort(); reject(error); }
        });
    }
    const read = (store, account, id) => transaction([store], 'readonly', (s, done) => {
        s[store].get(key(account,id)).onsuccess = e => done(e.target.result || null);
    });
    const list = (store, account) => transaction([store], 'readonly', (s, done) => {
        s[store].index('account').getAll(account).onsuccess = e => done(e.target.result);
    });
    const put = (store, account, id, value) => transaction([store], 'readwrite', (s, done) => {
        const record = { ...value, account, id, key:key(account,id) }; s[store].put(record); if(store==='documents')s['document-index'].put(documentInfo(record)); done(record);
    });
    async function patch(store, account, id, update) {
        return transaction([store], 'readwrite', (s, done) => {
            s[store].get(key(account,id)).onsuccess = event => {
                const previous = event.target.result || { account,id,key:key(account,id) };
                const next = update(previous);
                if (next) s[store].put(next); else s[store].delete(previous.key);
                if(store==='documents'){if(next)s['document-index'].put(documentInfo(next));else s['document-index'].delete(previous.key);}
                done(next);
            };
        });
    }
    async function settings(account) {
        return { retentionDays:14, ...(await read('settings',account,'preferences')) };
    }
    async function setRetention(account, days) {
        if (!Number.isInteger(days) || days < 1 || days > 3650) throw new RangeError('Choose 1–3650 days');
        await patch('settings',account,'preferences',previous => ({...previous,retentionDays:days}));
    }
    async function checkpoint(account, id, value, dirty = true) {
        if (!account || !id) throw new Error('An account and document are required');
        return transaction(['documents','outbox','entries'], 'readwrite', (s, done) => {
            s.documents.get(key(account,id)).onsuccess = event => {
                const old = event.target.result || {};
                const generation = (old.generation || 0) + 1;
                const record = { ...old, ...value, account,id,key:key(account,id),generation,
                    // A prepared PDF belongs to exactly one immutable checkpoint.
                    preparedBlob:null, preparedGeneration:null,
                    updatedAt:Date.now(), lastOpenedAt:value.lastOpenedAt || old.lastOpenedAt || Date.now(),
                    pinned: value.pinned ?? old.pinned ?? false, dirty:dirty || old.dirty || false };
                s.documents.put(record);
                s['document-index'].put(documentInfo(record));
                if (dirty) s.outbox.put({ account,id,key:record.key,generation,kind:'document',createdAt:old.createdAt || Date.now() });
                if (value.entry) s.entries.get(record.key).onsuccess=entryEvent=>s.entries.put({...entryEvent.target.result,...value.entry,account,id,key:record.key});
                done(record);
            };
        });
    }
    async function acknowledge(account,id,generation,meta,blob) {
        return transaction(['documents','outbox','entries','recoveries'], 'readwrite', (s, done) => {
            s.documents.get(key(account,id)).onsuccess = event => {
                const doc = event.target.result;
                if (!doc) { done(false); return; }
                // Edits made while the upload was in flight must stay pending.
                const settled = doc.generation === generation;
                const next = {...doc,remoteId:meta.id || doc.remoteId || id,lastUploadMeta:meta,lastUploadedAt:Date.now()};
                // Keep one recoverable editable draft before replacing it with
                // the uploaded PDF. A successful HTTP response must not erase
                // the only independent copy of the user's stroke data.
                if (settled && doc.snapshot) {
                    const recoveryId = `last-upload-draft:${id}`;
                    s.recoveries.put({key:key(account,recoveryId),account,id:recoveryId,
                        fileId:id,generation,snapshot:doc.snapshot,originalBlob:doc.originalBlob,
                        previousBlob:doc.blob,uploadedMeta:meta,createdAt:Date.now()});
                }
                if (settled) next.baseMeta=meta;
                if (settled) { next.dirty=false; next.offlineEdited=false; next.snapshot=null; next.analysis=null; next.originalBlob=null; next.preparedBlob=null; next.preparedGeneration=null; next.blob=blob || doc.blob; s.outbox.delete(doc.key); }
                s.documents.put(next);
                s['document-index'].put(documentInfo(next));
                s.entries.get(doc.key).onsuccess = entryEvent => {
                    const entry = entryEvent.target.result;
                    if (entry) s.entries.put({...entry,...meta,id,key:doc.key,account,pending:!settled});
                };
                done(settled);
            };
        });
    }
    async function createFolder(account,parentId,name) {
        const id = `local-${crypto.randomUUID()}`;
        const entry = { id,name,mimeType:'application/vnd.google-apps.folder',parents:[parentId || 'root'],pending:true,ownedByMe:true };
        await transaction(['entries','outbox'], 'readwrite', s => {
            s.entries.put({...entry,account,key:key(account,id)});
            s.outbox.put({account,id,key:key(account,id),kind:'folder',generation:1,createdAt:Date.now()});
        });
        return entry;
    }
    async function cacheEntries(account, entries) {
        if (!account) return;
        await transaction(['entries'], 'readwrite', s => {
            for (const entry of entries) {
                if (!entry?.id) continue;
                const entryKey=key(account,entry.id);
                s.entries.get(entryKey).onsuccess = e => {
                    const previous=e.target.result || {};
                    const newer=!previous.pending&&Date.parse(entry.modifiedTime||'')>Date.parse(previous.modifiedTime||'');
                    s.entries.put({...previous,...entry,...(newer?{localPreview:null}:{}),account,key:entryKey});
                };
            }
        });
    }
    async function resolveId(account,id) {
        if (!id?.startsWith('local-')) return id || 'root';
        return (await read('entries',account,id))?.remoteId || null;
    }
    async function finishFolder(account,id,remoteId) {
        await transaction(['entries','outbox'], 'readwrite', s => {
            s.entries.get(key(account,id)).onsuccess = event => {
                const entry=event.target.result;
                if (entry) s.entries.put({...entry,remoteId,pending:false});
                s.outbox.delete(key(account,id));
            };
        });
    }
    async function prune(account, now=Date.now(), protectedIds=[]) {
        const {retentionDays}=await settings(account);
        const cutoff=now-retentionDays*86400000;
        return transaction(['documents','outbox'], 'readwrite', (s,done) => {
            let removed=0;
            s.documents.index('account').openCursor(account).onsuccess = event => {
                const cursor=event.target.result;
                if (!cursor) {done(removed); return;}
                const doc=cursor.value;
                if (!doc.pinned && !doc.dirty && !protectedIds.includes(doc.id) && doc.lastOpenedAt<cutoff) {
                    s.outbox.get(doc.key).onsuccess = e => { if (!e.target.result) {s.documents.delete(doc.key); s['document-index'].delete(doc.key); removed++;} cursor.continue(); };
                } else cursor.continue();
            };
        });
    }
    async function withLease(account, name, work) {
        const owner=crypto.randomUUID(), ttl=45000;
        const acquire=await patch('leases',account,name,old => old.expires>Date.now()?old:{...old,owner,expires:Date.now()+ttl});
        if (acquire.owner!==owner) return {busy:true};
        let lost=false;
        const assert=async () => {
            const lease=await read('leases',account,name);
            if (lost || lease?.owner!==owner || lease.expires<Date.now()) throw new Error('Sync ownership changed');
        };
        const timer=setInterval(() => patch('leases',account,name,old => {
            if (old.owner!==owner) {lost=true;return old;} return {...old,expires:Date.now()+ttl};
        }).catch(()=>{lost=true;}),10000);
        try {return await work(assert);} finally {clearInterval(timer);await patch('leases',account,name,old=>old.owner===owner?null:old);}
    }
    root.InhouseOfflineStore=Object.freeze({open,read,list,put,patch,settings,setRetention,checkpoint,acknowledge,
        createFolder,cacheEntries,resolveId,finishFolder,prune,withLease});
})(globalThis);
