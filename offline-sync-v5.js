/* The same outbox runner is used by the browser and Android's background host. */
(function (root) {
    'use strict';
    const S=root.InhouseOfflineStore;
    const API='https://www.googleapis.com/drive/v3/files';
    const UPLOAD='https://www.googleapis.com/upload/drive/v3/files';
    const fields='id,name,mimeType,parents,modifiedTime,version,headRevisionId,trashed,capabilities(canEdit),appProperties';
    const changed=(a,b)=> !a || (a.headRevisionId && b.headRevisionId ? a.headRevisionId!==b.headRevisionId
        : a.version && b.version ? String(a.version)!==String(b.version) : Date.parse(a.modifiedTime||'')!==Date.parse(b.modifiedTime||''));
    function failure(message,code) {return Object.assign(new Error(message),{code});}
    async function run(account,adapter={}) {
        return S.withLease(account,'sync',async assertLease=>{
            const summary={saved:0,pending:0,errors:[]};
            const progress=(id,phase)=>{try{adapter.progress?.({id,phase});}catch(_){}};
            const request=async (url,options={})=>{
                await assertLease();
                const response=await adapter.fetch(url,options);
                if (!response.ok) throw failure(`Drive request failed (${response.status})`,response.status);
                return response;
            };
            const metadata=async id=>{
                const response=await request(`${API}/${encodeURIComponent(id)}?fields=${encodeURIComponent(fields)}`,{headers:{'Cache-Control':'no-cache'}});
                return {meta:await response.json(),etag:response.headers.get('ETag')};
            };
            const reserve=async (store,id,record)=>{
                if (record.remoteId) return record.remoteId;
                const response=await request(`${API}/generateIds?count=1&space=drive&type=files`);
                const remoteId=(await response.json()).ids?.[0];
                if (!remoteId) throw failure('Drive did not reserve a file ID','retry');
                await S.patch(store,account,id,old=>({...old,remoteId}));
                await S.patch('entries',account,id,old=>({...old,remoteId}));
                return remoteId;
            };
            const multipart=async (id,blob,meta,etag,create,tokenApi='v3')=>{
                const legacyConditional=!create&&tokenApi==='v2';
                const uploadMeta=legacyConditional?{properties:Object.entries(meta.appProperties||{}).map(([key,value])=>({key,value,visibility:'PRIVATE'}))}:meta;
                const boundary=`ihn_${crypto.randomUUID()}`;
                const body=new Blob([`--${boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n`,JSON.stringify(uploadMeta),
                    `\r\n--${boundary}\r\nContent-Type: application/pdf\r\n\r\n`,blob,`\r\n--${boundary}--\r\n`]);
                const headers={'Content-Type':`multipart/related; boundary=${boundary}`};
                if (!create) {
                    if (!etag) throw failure('Drive did not provide a conditional-write token','needs-merge');
                    headers['If-Match']=etag;
                }
                // Use the same API representation that supplied the conditional token.
                const endpoint=legacyConditional?UPLOAD.replace('/v3/','/v2/'):UPLOAD;
                const projection=legacyConditional?'id,title,mimeType,modifiedDate,version,headRevisionId':fields;
                const result=await (await request(`${endpoint}${create?'':`/${encodeURIComponent(id)}`}?uploadType=multipart&fields=${encodeURIComponent(projection)}`,
                    {method:create?'POST':'PATCH',headers,body})).json();
                return legacyConditional?{...result,...(result.title?{name:result.title}:{}),...(result.modifiedDate?{modifiedTime:result.modifiedDate}:{}),appProperties:meta.appProperties}:result;
            };
            const queue=await S.list('outbox',account);
            // Resolve parents first, even when folder names are duplicated.
            let folders=queue.filter(op=>op.kind==='folder');
            for (let pass=0;folders.length && pass<=queue.length;pass++) {
                const deferred=[];
                for (const op of folders) {
                    try {
                        progress(op.id,'uploading');
                        const entry=await S.read('entries',account,op.id);
                        const parent=await S.resolveId(account,entry.parents?.[0]);
                        if (!parent) {deferred.push(op);continue;}
                        const remoteId=await reserve('entries',op.id,entry);
                        let existing;
                        try {existing=(await metadata(remoteId)).meta;} catch(error) {if(error.code!==404)throw error;}
                        if (!existing) await request(API,{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({
                            id:remoteId,name:entry.name,mimeType:'application/vnd.google-apps.folder',parents:[parent],appProperties:{ihnOfflineId:op.id}
                        })});
                        await S.finishFolder(account,op.id,remoteId); summary.saved++;
                    } catch(error) {summary.errors.push({id:op.id,code:error.code||'network',message:error.message});}
                }
                if (deferred.length===folders.length) {summary.pending+=deferred.length;break;} folders=deferred;
            }
            for (const op of queue.filter(op=>op.kind==='document')) {
                try {
                    let doc=await S.read('documents',account,op.id);
                    if (!doc?.dirty) continue;
                    if (adapter.skip?.(op.id)) {summary.pending++;continue;}
                    const generation=doc.generation;
                    progress(op.id,'checking');
                    const entry=await S.read('entries',account,op.id) || doc.entry;
                    const parent=await S.resolveId(account,entry?.parents?.[0]);
                    if (!parent) {summary.pending++;continue;}
                    const isNew=op.id.startsWith('local-');
                    const id=isNew?await reserve('documents',op.id,doc):(doc.remoteId||op.id);
                    let current;
                    try {current=await metadata(id);} catch(error) {if(error.code!==404 || !isNew)throw error;}
                    if (current?.meta.trashed) throw failure('The remote document is in the bin; local edits are retained','remote-deleted');
                    if (current?.meta.capabilities?.canEdit===false) throw failure('Editing permission changed; local edits are retained',403);
                    const receipt=`${op.id}:${generation}`;
                    if (current?.meta.appProperties?.ihnOfflineReceipt===receipt) {
                        // The server committed an earlier upload whose response was lost.
                        const committed=await (await request(`${API}/${encodeURIComponent(id)}?alt=media`,{headers:{'Cache-Control':'no-cache'}})).blob();
                        const confirmed=await metadata(id);
                        if(changed(current.meta,confirmed.meta))throw failure('Remote changed during recovery',412);
                        await S.acknowledge(account,op.id,generation,current.meta,committed);
                        await adapter.saved?.(op.id,current.meta,committed);summary.saved++;continue;
                    }
                    let blob=doc.preparedBlob;
                    if (doc.snapshot && (!blob || doc.preparedGeneration!==generation)) {
                        if (!adapter.build) throw failure('A saved draft is waiting for the document processor','needs-processor');
                        progress(op.id,'building');
                        blob=await adapter.build(doc);
                    }
                    blob ||= doc.blob;
                    if (!(blob instanceof Blob) || !blob.size) throw failure('A complete local document is not available','needs-processor');
                    if (current && changed(doc.baseMeta,current.meta)) {
                        const remote=await (await request(`${API}/${encodeURIComponent(id)}?alt=media`,{headers:{'Cache-Control':'no-cache'}})).blob();
                        // Full originals are durable before any merge, including same-item edits.
                        await S.put('recoveries',account,`${op.id}:${generation}:${current.meta.version||current.meta.headRevisionId}`,
                            {fileId:op.id,name:entry?.name,localBlob:blob,remoteBlob:remote,snapshot:doc.snapshot,createdAt:Date.now()});
                        if (!adapter.merge) throw failure('Concurrent edits are retained and waiting to merge','needs-merge');
                        progress(op.id,'merging');
                        blob=await adapter.merge(doc,current.meta,remote);
                        const after=await metadata(id);
                        if (changed(current.meta,after.meta)) throw failure('Remote document changed during merge; retrying',412);
                        current=after;
                    }
                    if ((await S.read('documents',account,op.id))?.generation!==generation) {summary.pending++;continue;}
                    // Drive v3 does not guarantee an exposed ETag. v2 supplies it in
                    // JSON; tie it to the same revision before conditional writing.
                    if(current&&!current.etag) {
                        const token=await (await request(`https://www.googleapis.com/drive/v2/files/${encodeURIComponent(id)}?fields=etag,version`,{headers:{'Cache-Control':'no-cache'}})).json();
                        if(!token.etag)throw failure('Drive did not provide a conditional-write token','needs-merge');
                        if(!current.meta.version || String(token.version)!==String(current.meta.version))throw failure('Remote document changed before upload',412);
                        current.etag=token.etag;current.tokenApi='v2';
                    }
                    progress(op.id,'uploading');
                    const saved=await multipart(id,blob,{...(current?{}:{id,name:entry?.name||'Untitled.pdf',mimeType:'application/pdf',parents:[parent]}),
                        appProperties:{...(current?.meta.appProperties||{}),ihnOfflineReceipt:receipt}},current?.etag,!current,current?.tokenApi);
                    await S.acknowledge(account,op.id,generation,saved,blob);
                    summary.saved++;
                    await adapter.saved?.(op.id,saved,blob);
                } catch(error) {
                    summary.pending++;
                    summary.errors.push({id:op.id,code:error.code||'network',message:error.message});
                    await S.patch('outbox',account,op.id,old=>old.kind?{...old,lastError:error.code||'network',lastErrorMessage:error.message,attemptedAt:Date.now()}:null);
                }
            }
            return summary;
        });
    }
    root.InhouseOfflineSync=Object.freeze({run,changed});
})(globalThis);
