/* Transient drawing frames, independent of durable journals and PDF work.
 * Each frame carries complete active paths: missing an HTTP response cannot
 * leave a hole that hides the rest of an unfinished stroke. */
(function(root){
    'use strict';
    const Base=root.InhouseDriveInkJournal?.Journal
        ||(typeof require==='function'?require('./drive-ink-journal-v5.js').Journal:null);
    const API='https://www.googleapis.com/drive/v3/files';
    const UPLOAD='https://www.googleapis.com/upload/drive/v3/files';
    const KIND='ink-preview-v1',FRAME_MS=100,MAX_BYTES=2*1024*1024,READ_CACHE_CHARS=4*1024*1024;
    const escape=value=>String(value).replace(/\\/g,'\\\\').replace(/'/g,"\\'");
    class LivePreview extends Base {
        constructor(host){
            super(host);this.paths=new Map();this.frame=0;this.sentFrame=0;
            this.fileId='';this.exists=false;this.frameTimer=null;this.lastFrameAt=0;
            this.pollTimer=null;this.previewSources=new Map();this.nextList=0;this.retryAt=0;
            this.readCache=new Map();this.readCacheChars=0;
        }
        push(packet){
            if(this.stopped||!this.docId||packet?.fileId!==this.docId||packet.type!=='live-stroke'
                ||!packet.strokeId||!packet.actorId||!Array.isArray(packet.points))return;
            const key=`${packet.actorId}:${packet.strokeId}`,old=this.paths.get(key);
            if(old&&packet.sequence<=old.sequence)return;
            const offset=Number(packet.offset)||0;
            if(offset<0||offset+packet.points.length>24000)return;
            // Final batches may contain a shorter/refined path. Start from
            // that geometry rather than retaining the preview's stale tail.
            const points=packet.finalBatch&&offset===0?[]:(old?.points||[]);
            for(let i=0;i<packet.points.length;i++)points[offset+i]=packet.points[i];
            if(packet.finalBatch&&Number.isSafeInteger(packet.totalPoints)&&packet.totalPoints>=0)
                points.length=Math.min(24000,packet.totalPoints);
            if(packet.cancel)points.length=0;
            this.paths.set(key,{...packet,offset:0,points,updatedAt:Date.now()});
            this.frame++;
            for(const [id,path]of this.paths)if(Date.now()-path.updatedAt>15000)this.paths.delete(id);
            while(this.paths.size>24)this.paths.delete(this.paths.keys().next().value);
            this.scheduleFrame();
        }
        retireConfirmed(operations){
            if(this.stopped||!this.paths.size||!Array.isArray(operations))return;
            const confirmed=new Map();
            for(const encoded of operations){
                let op;try{op=JSON.parse(encoded);}catch(_){continue;}
                if(op.kind==='stroke'&&op.stroke?.id)confirmed.set(op.stroke.id,op.stroke.syncStamp);
                for(const page of op.pages||[])for(const stroke of page.strokes||[])
                    if(stroke?.id)confirmed.set(stroke.id,stroke.syncStamp);
            }
            let changed=false;
            for(const [key,path]of this.paths){
                if(!path.final||!confirmed.has(path.strokeId))continue;
                const stamp=confirmed.get(path.strokeId);
                if(stamp?.clock!==path.syncStamp?.clock||stamp?.actor!==path.syncStamp?.actor)continue;
                this.paths.delete(key);changed=true;
            }
            // The durable journal is now the source of the finished stroke.
            // Keep unconfirmed/unfinished paths intact even if another write
            // completed out of order. No local or remote ink is deleted here.
            if(changed){this.frame++;this.scheduleFrame();}
        }
        scheduleFrame(){
            if(this.stopped||this.frameTimer||this.writing||this.sentFrame===this.frame)return;
            this.frameTimer=setTimeout(()=>{this.frameTimer=null;void this.flushFrame();},
                Math.max(0,this.lastFrameAt+FRAME_MS-Date.now(),this.retryAt-Date.now()));
        }
        async reservePreviewFile(){
            if(this.fileId)return this.fileId;
            const reserved=await(await this.request(`${API}/generateIds?count=1&space=drive&type=files`)).json();
            if(this.stopped)throw new DOMException('Preview closed','AbortError');
            const id=reserved.ids?.[0];
            if(!id)throw new Error('Drive did not reserve the live preview');
            this.fileId=id;return id;
        }
        async flushFrame(){
            if(this.stopped||this.writing||this.sentFrame===this.frame)return;
            this.writing=true;this.lastFrameAt=Date.now();
            try{
                // Cold folder discovery and ID reservation are independent.
                // Keep the writing lock until BOTH settle, including failure
                // and cancellation, so retries cannot reserve competing IDs.
                const setup=await Promise.allSettled([
                    this.exists?Promise.resolve(null):this.ensureFolder(),this.reservePreviewFile()
                ]);
                for(const result of setup)if(result.status==='rejected')throw result.reason;
                const folder=setup[0].value,id=setup[1].value;
                if(this.stopped)throw new DOMException('Preview closed','AbortError');
                // Setup can span several pen frames. Freeze the latest complete
                // paths immediately before sending, not an empty/stale warm-up
                // snapshot. Once serialized, this in-flight frame stays fixed.
                const frame=this.frame;
                const packets=[...this.paths.values()].map(path=>{
                    const gap=path.points.findIndex(point=>!point);
                    return {...path,points:path.points.slice(0,gap<0?path.points.length:gap)};
                });
                const raw=JSON.stringify({v:1,docId:this.docId,actor:this.actor,frame,at:Date.now(),packets});
                if(raw.length>MAX_BYTES)throw new Error('Live frame is too large; the complete stroke stays in the durable journal');
                const update=()=>this.request(`${UPLOAD}/${id}?uploadType=media&fields=id`,{
                    method:'PATCH',headers:{'Content-Type':'application/json'},body:raw,bufferResponse:true});
                let response;
                if(this.exists)response=await update();
                else{
                    const boundary=`ink_${crypto.randomUUID()}`;
                    const meta={id,name:`live-${this.docId}-${this.actor}.json`,mimeType:'application/json',parents:[folder],
                        appProperties:{ihnKind:KIND,ihnDoc:this.docId,ihnActor:this.actor}};
                    try{response=await this.request(`${UPLOAD}?uploadType=multipart&fields=id`,{method:'POST',
                        headers:{'Content-Type':`multipart/related; boundary=${boundary}`},bufferResponse:true,
                        body:new Blob([`--${boundary}\r\nContent-Type: application/json\r\n\r\n`,JSON.stringify(meta),
                            `\r\n--${boundary}\r\nContent-Type: application/json\r\n\r\n`,raw,`\r\n--${boundary}--\r\n`])});}
                    catch(error){
                        if(error.status!==409)throw error;
                        const meta=await(await this.request(`${API}/${id}?fields=appProperties`)).json();
                        if(meta.appProperties?.ihnActor!==this.actor||meta.appProperties?.ihnDoc!==this.docId||meta.appProperties?.ihnKind!==KIND)throw new Error('Live preview identity mismatch');
                        response=await update();
                    }
                }
                const receipt=await response.json();if(receipt.id!==id)throw new Error('Live preview was not confirmed');
                this.exists=true;this.sentFrame=frame;this.retryAt=0;
                this.notifyPeers(this.docId,this.fileId,true);
            }catch(error){
                if(error.status===404){
                    this.exists=false;this.fileId='';this.folder=null;
                    try{await this.patch('ink-journal-folder',()=>null);}catch(_){}
                }
                this.retryAt=Date.now()+(error.status===429?5000:1000);
                this.host.warning?.(error);
            }finally{this.writing=false;this.scheduleFrame();}
        }
        hint(id){
            if(typeof id!=='string'||!/^[\w-]{1,200}$/.test(id)||id===this.fileId)return;
            const previous=this.previewSources.get(id);
            if(previous){previous.activeUntil=Date.now()+5000;return;}
            if(this.previewSources.size<128)this.previewSources.set(id,{next:0,frame:0,activeUntil:Date.now()+5000});
            this.schedulePoll(0);
        }
        async discoverPreviews(){
            const q=`trashed=false and appProperties has { key='ihnKind' and value='${KIND}' } and appProperties has { key='ihnDoc' and value='${escape(this.docId)}' }`;
            const data=await(await this.request(`${API}?q=${encodeURIComponent(q)}&fields=files(id)&orderBy=modifiedTime%20desc&pageSize=30`)).json();
            for(const file of data.files||[])this.hint(file.id);
        }
        schedulePoll(delay=FRAME_MS){
            if(this.stopped||!this.docId||this.pollTimer)return;
            this.pollTimer=setTimeout(()=>{this.pollTimer=null;void this.pollFrames();},delay);
        }
        forgetReadFrame(id){
            const cached=this.readCache.get(id);if(!cached)return;
            this.readCacheChars-=cached.raw.length;this.readCache.delete(id);
        }
        rememberReadFrame(id,source,raw,value){
            if(this.stopped||this.previewSources.get(id)!==source)return;
            this.forgetReadFrame(id);
            while(this.readCache.size&&this.readCacheChars+raw.length>READ_CACHE_CHARS)
                this.forgetReadFrame(this.readCache.keys().next().value);
            if(raw.length>READ_CACHE_CHARS)return;
            this.readCache.set(id,{source,raw,at:value.at,frame:value.frame,docId:this.docId});
            this.readCacheChars+=raw.length;
        }
        pruneReadFrames(now){
            for(const [id,cached]of this.readCache)
                if(this.previewSources.get(id)!==cached.source||cached.docId!==this.docId
                    ||now-cached.at>20000||cached.at>now+30000)this.forgetReadFrame(id);
        }
        async readPreview(id,source){
            const now=Date.now();source.inFlight=true;source.next=now+2000;
            try{
                const response=await this.request(`${API}/${id}?alt=media`,{headers:{'Cache-Control':'no-cache'},bufferResponse:true});
                const raw=await response.text();if(this.stopped||raw.length>MAX_BYTES)return;
                const receivedAt=Date.now(),cached=this.readCache.get(id);
                // Only identical, already validated bytes can skip decoding.
                // Keep timestamps live: repeated polls never refresh an old
                // frame's age or extend its source's activity window.
                if(cached?.source===source&&cached.docId===this.docId
                    &&cached.frame<=source.frame&&cached.raw===raw){
                    if(receivedAt-cached.at>20000||cached.at>receivedAt+30000){
                        source.activeUntil=0;source.next=receivedAt+15000;this.forgetReadFrame(id);return;
                    }
                    this.readCache.delete(id);this.readCache.set(id,cached);
                    source.next=receivedAt+(source.activeUntil>receivedAt?FRAME_MS:2000);
                    return;
                }
                const value=JSON.parse(raw);
                if(value.v!==1||value.docId!==this.docId||value.actor===this.actor||!Number.isSafeInteger(value.frame)
                    ||!Array.isArray(value.packets)||value.packets.length>24||!Number.isFinite(value.at))return;
                if(receivedAt-value.at>20000||value.at>receivedAt+30000){source.activeUntil=0;source.next=receivedAt+15000;this.forgetReadFrame(id);return;}
                if(value.frame>source.frame){
                    for(const packet of value.packets){
                        if(packet?.fileId!==this.docId||packet.type!=='live-stroke'||!Array.isArray(packet.points)||packet.points.length>24000)continue;
                        this.host.preview?.(packet);
                    }
                    source.frame=value.frame;source.activeUntil=receivedAt+5000;
                }
                source.next=receivedAt+(source.activeUntil>receivedAt?FRAME_MS:2000);
                // Install proof only after every preview callback succeeds.
                // Bound retained strings across all sources, not just per file.
                this.rememberReadFrame(id,source,raw,value);
            }catch(_){source.next=Date.now()+3000;}
            finally{source.inFlight=false;}
        }
        async pollFrames(){
            if(this.stopped)return;
            const now=Date.now();
            this.pruneReadFrames(now);
            if(now>=this.nextList&&!this.discovery){
                this.nextList=now+5000;
                this.discovery=this.discoverPreviews().catch(()=>{}).finally(()=>{this.discovery=null;});
            }
            const sources=[...this.previewSources];
            const available=Math.max(0,4-sources.filter(([,source])=>source.inFlight).length);
            const due=sources.filter(([id,source])=>id!==this.fileId&&!source.inFlight&&source.next<=now)
                .sort((a,b)=>b[1].activeUntil-a[1].activeUntil||a[1].next-b[1].next).slice(0,available);
            const reads=due.map(([id,source])=>this.readPreview(id,source));
            // Run the next scheduling pass independently. One stalled device
            // consumes one slot, never the next frames of every healthy peer.
            this.schedulePoll();
            await Promise.allSettled(reads);
        }
        start(){if(this.host.canPublish!==false){this.frame++;this.scheduleFrame();}this.schedulePoll(0);return this;}
        stop(){super.stop();clearTimeout(this.frameTimer);clearTimeout(this.pollTimer);this.paths.clear();this.previewSources.clear();this.readCache.clear();this.readCacheChars=0;}
    }
    root.InhouseDriveLivePreview=LivePreview;
    if(typeof module==='object'&&module.exports)module.exports={LivePreview};
})(globalThis);
