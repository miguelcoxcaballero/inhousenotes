/* Small, durable Drive journals. Like Inhouse Learn's live files, discovered
 * IDs are polled directly; PDF generation is not part of this transport. */
(function(root,factory){
    const api=factory();
    if(typeof module==='object'&&module.exports)module.exports=api;
    root.InhouseDriveInkJournal=api;
})(globalThis,function(){
    'use strict';
    const API='https://www.googleapis.com/drive/v3/files';
    const UPLOAD='https://www.googleapis.com/upload/drive/v3/files';
    const OUTBOX='ink-journal-outbox';
    const KIND='ink-journal-v1', PREFIX='ink-journal:', MAX_BODY=8*1024*1024;
    const folderLookups=new WeakMap();
    const escape=value=>String(value).replace(/\\/g,'\\\\').replace(/'/g,"\\'");
    class Journal {
        constructor(host){
            this.host=host;this.store=host.store;this.account=host.account;this.docId=host.docId||'';
            this.actor=host.actor||crypto.randomUUID();this.part=0;this.current=null;
            this.records=new Map();this.sources=new Map();this.seen=new Map();
            this.readTasks=new Map();this.applyChain=Promise.resolve();this.bootstrapWait=null;
            this.outboxReady=!!this.store.patchAndQueue;this.persistChain=Promise.resolve();this.stopped=false;this.writing=false;this.reading=false;
            this.nextOutboxCheck=0;this.outboxWakeVersion=0;this.announcements=new Map();this.announcing=false;this.controllers=new Set();this.latestParts=new Map();this.lastWrite=0;this.nextDiscovery=0;this.timer=null;this.folder=null;this.lastError='';
        }
        key(record){return `${PREFIX}${record.docId}:${record.actor}:${record.part}`;}
        async read(key){return this.store.read('settings',this.account,key);}
        async put(key,value){return this.store.put('settings',this.account,key,value);}
        async patch(key,fn){return this.store.patch('settings',this.account,key,fn);}
        async request(url,options={}){
            if(this.stopped)throw new DOMException('Journal closed','AbortError');
            const controller=new AbortController();this.controllers.add(controller);
            let response;
            try { response=await this.host.fetch(url,{timeout:8000,...options,signal:controller.signal}); }
            finally { this.controllers.delete(controller); }
            if(this.stopped)throw new DOMException('Journal closed','AbortError');
            if(!response.ok)throw Object.assign(new Error(`Drive ink request failed (${response.status})`),{status:response.status});
            return response;
        }
        status(){
            return {writing:this.writing,error:this.lastError,
                pending:[...this.records.values()].reduce((n,r)=>n+Math.max(0,r.rev-(r.ack||0)),0)};
        }
        enqueue(operation,version){
            if(this.stopped||!this.docId)return Promise.reject(new Error('No journal document'));
            const encoded=JSON.stringify(operation);
            if(encoded.length>MAX_BODY/2)return Promise.reject(new Error('This edit requires the full PDF save'));
            if(!this.current||this.current.ops.length>=128||this.current.bytes+encoded.length>256*1024){
                const previous=this.current;
                if(previous&&previous.ack>=previous.rev)this.records.delete(this.key(previous));
                this.current={kind:KIND,docId:this.docId,actor:this.actor,part:this.part++,
                    ops:[],bytes:0,rev:0,ack:0,version:0,previousKey:previous?this.key(previous):'',createdAt:Date.now()};
            }
            const record=this.current;record.ops.push(encoded);record.bytes+=encoded.length;
            record.rev++;record.version=version;this.records.set(this.key(record),record);
            // Freeze this revision before crossing the async storage boundary.
            const snapshot={...record,ops:record.ops.slice()};
            this.persistChain=this.persistChain.catch(()=>{}).then(async()=>{
                const update=old=>({...old,...snapshot,
                    etag:old.etag||null,fileId:old.fileId||snapshot.fileId,exists:old.exists||snapshot.exists,
                    ack:Math.max(old.ack||0,snapshot.ack||0)});
                if(this.store.patchAndQueue)await this.store.patchAndQueue(this.account,this.key(record),update,OUTBOX,snapshot.rev);
                else{
                    await this.patch(this.key(record),update);
                    await this.patch(OUTBOX,old=>({...old,pending:{...old.pending,[this.key(record)]:snapshot.rev}}));
                }
            });
            this.persistChain.then(()=>this.wake(),error=>this.fail(error));
            return this.persistChain;
        }
        hint(id,active=true){
            if(typeof id!=='string'||!/^[\w-]{1,200}$/.test(id))return;
            if(!this.sources.has(id)&&this.sources.size<4096)this.sources.set(id,{next:0,retryAt:0,rev:0,hotUntil:0,idleReads:0});
            const source=this.sources.get(id);
            if(source&&active){source.hotUntil=Date.now()+6000;if(source.next!==Infinity)source.next=Math.max(source.retryAt||0,Math.min(source.next,Date.now()+120));}
        }
        async ensureFolder(){
            if(this.folder)return this.folder;
            // Live previews and durable journals start together. Share their
            // folder lookup within this account/store so neither creates a
            // second folder while the other's first lookup is still pending.
            let accounts=folderLookups.get(this.store);
            if(!accounts){accounts=new Map();folderLookups.set(this.store,accounts);}
            let pending=accounts.get(this.account);
            if(!pending){pending=this.findOrCreateFolder();accounts.set(this.account,pending);}
            try{
                const id=await pending;
                if(this.stopped)throw new DOMException('Journal closed','AbortError');
                return this.folder=id;
            }finally{if(accounts.get(this.account)===pending)accounts.delete(this.account);}
        }
        async findOrCreateFolder(){
            const cached=await this.read('ink-journal-folder');
            if(cached?.fileId)return cached.fileId;
            const q="trashed=false and mimeType='application/vnd.google-apps.folder' and 'me' in owners and (name='inhouse notes' or name='Inhouse Notes')";
            const found=[];let token='';
            do{
                const data=await (await this.request(`${API}?q=${encodeURIComponent(q)}&fields=nextPageToken,files(id)&pageSize=100${token?'&pageToken='+encodeURIComponent(token):''}`)).json();
                found.push(...(data.files||[]));token=data.nextPageToken||'';
            }while(token);
            let id=found.map(file=>file.id).sort()[0];
            if(!id){const file=await (await this.request(API,{method:'POST',headers:{'Content-Type':'application/json'},
                body:JSON.stringify({name:'inhouse notes',mimeType:'application/vnd.google-apps.folder'})})).json();id=file.id;}
            if(!id)throw new Error('Drive did not confirm the ink folder');
            await this.put('ink-journal-folder',{fileId:id});return id;
        }
        async reserve(record,key){
            if(record.fileId)return record.fileId;
            const data=await (await this.request(`${API}/generateIds?count=1&space=drive&type=files`)).json();
            const id=data.ids?.[0];if(!id)throw new Error('Drive did not reserve an ink file');
            await this.patch(key,old=>({...old,fileId:id}));record.fileId=id;return id;
        }
        async upload(record,key){
            // These requests are independent. Wait for BOTH even on failure:
            // a reservation still writing its durable ID must not outlive the
            // write lease and race a retry. Existing files need no folder read.
            const setup=await Promise.allSettled([
                record.exists?Promise.resolve(null):this.ensureFolder(),this.reserve(record,key)
            ]);
            for(const result of setup)if(result.status==='rejected')throw result.reason;
            const folder=setup[0].value,id=setup[1].value;
            let previous='',previousRev=0;
            if(record.previousKey){const parent=await this.read(record.previousKey);if(parent){previous=await this.reserve(parent,record.previousKey);previousRev=parent.rev;}}
            const prefix=JSON.stringify({v:1,docId:record.docId,actor:record.actor,part:record.part,rev:record.rev,previous,previousRev});
            const body=new Blob([prefix.slice(0,-1),',"ops":[',record.ops.join(','),']}'],{type:'application/json'});
            if(body.size>MAX_BODY)throw new Error('Ink journal exceeds the safe transfer size; PDF copy remains pending');
            const update=async()=>{
                let etag=record.etag;
                if(!etag){
                    const metadata=await (await this.request(`https://www.googleapis.com/drive/v2/files/${encodeURIComponent(id)}?fields=id,etag`)).json();
                    etag=metadata.etag;
                    if(!etag)throw new Error('Drive did not provide an ink revision precondition');
                    const existing=await (await this.request(`${API}/${encodeURIComponent(id)}?alt=media`,{bufferResponse:true})).json();
                    if(existing.docId!==record.docId||existing.actor!==record.actor||existing.part!==record.part
                        ||!Array.isArray(existing.ops))throw new Error('Ink file identity mismatch');
                    for(let i=0;i<Math.min(existing.ops.length,record.ops.length);i++)
                        if(JSON.stringify(existing.ops[i])!==record.ops[i])throw new Error('Ink history conflict; both copies retained');
                    if(existing.rev>=record.rev)return new Response(JSON.stringify({id,etag}),{status:200});
                }
                return this.request(`https://www.googleapis.com/upload/drive/v2/files/${encodeURIComponent(id)}?uploadType=media&fields=id,etag`,
                    {method:'PUT',headers:{'Content-Type':'application/json','If-Match':etag},body,bufferResponse:true});
            };
            let response;
            if(record.exists)response=await update();
            else{
                const boundary=`ink_${crypto.randomUUID()}`;
                const metadata={id,name:`ink-${record.docId}-${record.actor}-${record.part}.json`,mimeType:'application/json',parents:[folder],
                    appProperties:{ihnKind:KIND,ihnDoc:record.docId,ihnActor:record.actor,ihnPart:String(record.part)}};
                try{response=await this.request(`${UPLOAD}?uploadType=multipart&fields=id,modifiedTime`,{method:'POST',
                    headers:{'Content-Type':`multipart/related; boundary=${boundary}`},bufferResponse:true,
                    body:new Blob([`--${boundary}\r\nContent-Type: application/json\r\n\r\n`,JSON.stringify(metadata),
                        `\r\n--${boundary}\r\nContent-Type: application/json\r\n\r\n`,body,`\r\n--${boundary}--\r\n`])});}
                catch(error){
                    if(error.status!==409)throw error;
                    // The previous create may have reached Drive before its
                    // response was lost. Verify ownership before retrying it.
                    const meta=await (await this.request(`${API}/${id}?fields=id,appProperties`)).json();
                    if(meta.appProperties?.ihnDoc!==record.docId||meta.appProperties?.ihnActor!==record.actor
                        ||meta.appProperties?.ihnPart!==String(record.part))throw new Error('Ink file identity mismatch');
                    response=await update();
                }
            }
            if(response.status!==200&&response.status!==201)throw new Error('Drive has not confirmed the ink write');
            const receipt=await response.json();if(receipt.id!==id)throw new Error('Drive confirmed a different ink file');
            await this.patch(key,old=>({...old,fileId:id,exists:true,etag:receipt.etag||response.headers.get('ETag')||null,retryAt:0,failures:0,ack:Math.max(old.ack||0,record.rev)}));
            const local=this.records.get(key);
            if(local){local.fileId=id;local.exists=true;local.etag=receipt.etag||response.headers.get('ETag')||null;local.ack=Math.max(local.ack||0,record.rev);}
            if(local&&local!==this.current&&local.ack>=local.rev)this.records.delete(key);
            await this.patch(OUTBOX,old=>{const pending={...old.pending};if((pending[key]||0)<=record.rev)delete pending[key];return {...old,pending};});
            this.hint(id);if(this.sources.has(id))this.sources.get(id).next=Infinity;
            this.lastError='';this.host.receipt?.({...receipt,docId:record.docId,version:record.version,actor:record.actor,part:record.part,operations:record.ops.slice(record.ack||0)});
            this.notifyPeers(record.docId,id);
        }
        notifyPeers(docId,id,preview=false){
            if(this.stopped||!this.host.announce)return;
            // Discovery metadata is advisory. A slow hint must not hold the
            // next confirmed write (or its cross-tab storage lease).
            this.announcements.set(`${docId}:${preview}`,{docId,id,preview});
            if(this.announcing)return;
            this.announcing=true;
            void (async()=>{
                try{
                    while(!this.stopped&&this.announcements.size){
                        const [key,notice]=this.announcements.entries().next().value;
                        this.announcements.delete(key);
                        try{await this.host.announce(notice.docId,notice.id,this.actor,notice.preview);}
                        catch(error){this.host.warning?.(error);}
                    }
                }finally{this.announcing=false;}
            })();
        }
        async flush(){
            if(this.stopped||this.writing||Date.now()-this.lastWrite<120||Date.now()<this.nextOutboxCheck)return;
            this.writing=true;
            const wakeVersion=this.outboxWakeVersion;
            try{
                await this.persistChain;
                await this.store.withLease(this.account,'ink-journal-write',async(assertLease)=>{
                    if(!this.outboxReady){
                        // Repair the small index once on process start, including
                        // a crash between the record commit and index update.
                        const pending={};
                        for(const record of await this.store.list('settings',this.account))
                            if(record.kind===KIND&&record.rev>(record.ack||0))pending[this.key(record)]=record.rev;
                        await this.patch(OUTBOX,old=>{for(const [key,rev]of Object.entries(old.pending||{}))pending[key]=Math.max(pending[key]||0,rev);return {...old,pending};});
                        this.outboxReady=true;
                    }
                    const index=await this.read(OUTBOX);
                    const keys=Object.keys(index?.pending||{}).sort((a,b)=>(a.startsWith(`${PREFIX}${this.docId}:`)?0:1)-(b.startsWith(`${PREFIX}${this.docId}:`)?0:1));
                    let earliestRetry=Infinity,attempted=false;
                    for(const key of keys){
                        const record=await this.read(key);
                        if(!record||record.rev<=(record.ack||0)){
                            await this.patch(OUTBOX,old=>{const pending={...old.pending};if(!record||(pending[key]||0)<=(record.ack||0))delete pending[key];return {...old,pending};});
                            continue;
                        }
                        if(record.retryAt>Date.now()){earliestRetry=Math.min(earliestRetry,record.retryAt);continue;}
                        attempted=true;
                        await assertLease();this.lastWrite=Date.now();
                        try{await this.upload(record,key);}
                        catch(error){
                            await this.patch(key,old=>({...old,retryAt:Date.now()+Math.min(30000,600*2**Math.min(6,old.failures||0)),failures:(old.failures||0)+1}));
                            if(error.status===412)await this.patch(key,old=>({...old,etag:null}));
                            if(error.status===404){this.folder=null;await this.patch('ink-journal-folder',()=>null);await this.patch(key,old=>({...old,fileId:null,etag:null,exists:false}));}
                            throw error;
                        }
                        break;
                    }
                    // Backing-off records need no lease/read/indicator work on
                    // every 120 ms network tick. Recheck cross-process work at
                    // most two seconds later; a fresh local edit wakes at once.
                    // Do not overwrite a wake that arrived during this scan.
                    if(!attempted&&wakeVersion===this.outboxWakeVersion)
                        this.nextOutboxCheck=Math.min(earliestRetry,Date.now()+2000);
                });
            }catch(error){this.fail(error);}
            finally{this.writing=false;this.host.changed?.();}
        }
        async discover(){
            if(!this.docId||this.stopped)return;
            const q=`trashed=false and appProperties has { key='ihnKind' and value='${KIND}' } and appProperties has { key='ihnDoc' and value='${escape(this.docId)}' }`;
            let token='';
            do{
                const data=await (await this.request(`${API}?q=${encodeURIComponent(q)}&fields=nextPageToken,files(id)&pageSize=100${token?'&pageToken='+encodeURIComponent(token):''}`)).json();
                for(const file of data.files||[])this.hint(file.id,false);
                token=data.nextPageToken||'';
            }while(token&&!this.stopped);
        }
        async readSource(id,source){
            try{
                const now=Date.now();
                source.next=now+(source.hotUntil>now?120:1800);
                const response=await this.request(`${API}/${encodeURIComponent(id)}?alt=media`,{
                    headers:{'Cache-Control':'no-cache'},bufferResponse:true});
                const raw=await response.text();
                if(this.stopped)return;
                if(raw.length>MAX_BODY)throw new Error('Ink journal is too large');
                // Fetches are independent, but document mutation is still
                // serialized. A pen-gated merge cannot race another merge.
                const apply=this.applyChain.catch(()=>{}).then(async()=>{
                    if(this.stopped)return;
                    const receivedAt=Date.now();
                    source.retryAt=0;
                    if(raw===source.raw){
                        source.idleReads++;
                        source.next=receivedAt+(source.hotUntil>receivedAt?120:Math.min(30000,1800*2**Math.floor(source.idleReads/3)));
                        return;
                    }
                    await this.host.idle?.();if(this.stopped)return;
                    const value=JSON.parse(raw);
                    if(value.v!==1||value.docId!==this.docId||typeof value.actor!=='string'||!Number.isSafeInteger(value.rev)
                        ||!Array.isArray(value.ops)||value.ops.length>128||value.rev!==value.ops.length)throw new Error('Invalid ink journal');
                    this.hint(value.previous,false);
                    const predecessor=this.sources.get(value.previous);
                    if(predecessor&&Number.isSafeInteger(value.previousRev)){
                        predecessor.requiredRev=Math.max(predecessor.requiredRev||0,value.previousRev);
                        if(predecessor.rev<predecessor.requiredRev)predecessor.next=0;
                    }
                    const latest=Math.max(this.latestParts.get(value.actor)||0,value.part||0);
                    this.latestParts.set(value.actor,latest);
                    source.actor=value.actor;source.part=value.part;
                    const seen=this.seen.get(id)||0;
                    if(value.rev<=seen)return; // An out-of-order read cannot roll back the replica.
                    if(value.actor!==this.actor)await this.host.apply(value.ops.slice(seen),{id,actor:value.actor,rev:value.rev});
                    if(this.stopped)return;
                    this.seen.set(id,value.rev);source.raw=raw;source.idleReads=0;source.rev=value.rev;
                    source.hotUntil=Date.now()+6000;source.next=Date.now()+120;
                    for(const entry of this.sources.values())if(entry.actor===value.actor&&entry.part<latest&&entry.requiredRev&&entry.rev>=entry.requiredRev){entry.next=Infinity;delete entry.raw;}
                });
                this.applyChain=apply;
                await apply;
            }catch(error){source.retryAt=Date.now()+5000;source.next=source.retryAt;this.fail(error);}
        }
        async poll(){
            if(this.stopped||!this.docId||this.reading)return;
            const now=Date.now();
            if(now>=this.nextDiscovery&&!this.discovery){
                this.nextDiscovery=now+5000;
                this.discovery=this.discover().catch(error=>this.fail(error)).finally(()=>{this.discovery=null;});
            }
            if(!this.sources.size&&this.discovery){
                // Only the first empty poll waits for initial discovery. It
                // does not own the scheduling guard: a direct-ID hint arriving
                // meanwhile can start a read without waiting for the index.
                if(this.bootstrapWait)return;
                const pending=this.discovery;this.bootstrapWait=pending;
                try{await pending;}finally{if(this.bootstrapWait===pending)this.bootstrapWait=null;}
            }
            if(this.stopped||this.reading)return;
            this.reading=true;
            let started=[];
            try{
                const now=Date.now();
                const due=[...this.sources.entries()]
                    .filter(([id,source])=>!this.readTasks.has(id)&&source.next<=now&&(source.retryAt||0)<=now)
                    .sort((a,b)=>(b[1].hotUntil>now?1:0)-(a[1].hotUntil>now?1:0)||a[1].next-b[1].next)
                    .slice(0,Math.max(0,4-this.readTasks.size));
                started=due.map(([id,source])=>{
                    const task=this.readSource(id,source).finally(()=>this.readTasks.delete(id));
                    this.readTasks.set(id,task);return task;
                });
            }catch(error){this.fail(error);}
            finally{this.reading=false;}
            // Manual callers can await this batch, while the periodic scheduler
            // may reuse free lanes without waiting for a slower remote device.
            await Promise.allSettled(started);
        }
        fail(error){if(error?.name!=='AbortError'){this.lastError=String(error?.message||error);this.host.error?.(error);}}
        wake(){
            if(this.stopped)return;
            this.nextOutboxCheck=0;this.outboxWakeVersion++;
            clearTimeout(this.timer);
            this.timer=setTimeout(()=>{this.timer=null;this.tick();},Math.max(0,this.lastWrite+120-Date.now()));
        }
        tick(){
            if(this.stopped)return;
            // Receiving may wait for the local pen to lift. It must never hold
            // the independent durable upload queue behind that wait.
            void this.flush().catch(error=>this.fail(error));
            void this.poll().catch(error=>this.fail(error));
            this.timer=setTimeout(()=>{this.timer=null;this.tick();},120);
        }
        async retry(){
            this.lastWrite=0;
            for(const source of this.sources.values())if(source.next!==Infinity){
                source.retryAt=0;source.next=0;source.hotUntil=Date.now()+6000;
            }
            this.nextDiscovery=0;
            const index=await this.read(OUTBOX);
            for(const key of Object.keys(index?.pending||{}))await this.patch(key,old=>({...old,retryAt:0}));
            this.wake();
        }
        start(){this.wake();return this;}
        stop(){this.stopped=true;this.announcements.clear();for(const controller of this.controllers)controller.abort();clearTimeout(this.timer);this.timer=null;}
    }
    return {Journal,KIND};
});
