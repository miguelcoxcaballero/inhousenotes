/* Transient drawing frames, independent of durable journals and PDF work.
 * Each frame carries complete active paths: missing an HTTP response cannot
 * leave a hole that hides the rest of an unfinished stroke. */
(function(root){
    'use strict';
    const Base=root.InhouseDriveInkJournal?.Journal
        ||(typeof require==='function'?require('./drive-ink-journal-v5.js').Journal:null);
    const API='https://www.googleapis.com/drive/v3/files';
    const UPLOAD='https://www.googleapis.com/upload/drive/v3/files';
    const KIND='ink-preview-v1',FRAME_MS=100,MAX_BYTES=2*1024*1024;
    const escape=value=>String(value).replace(/\\/g,'\\\\').replace(/'/g,"\\'");
    class LivePreview extends Base {
        constructor(host){
            super(host);this.paths=new Map();this.frame=0;this.sentFrame=0;
            this.fileId='';this.exists=false;this.frameTimer=null;this.lastFrameAt=0;
            this.pollTimer=null;this.previewSources=new Map();this.nextList=0;this.retryAt=0;
        }
        push(packet){
            if(this.stopped||!this.docId||packet?.fileId!==this.docId||packet.type!=='live-stroke'
                ||!packet.strokeId||!packet.actorId||!Array.isArray(packet.points))return;
            const key=`${packet.actorId}:${packet.strokeId}`,old=this.paths.get(key);
            if(old&&packet.sequence<=old.sequence)return;
            const offset=Number(packet.offset)||0;
            if(offset<0||offset+packet.points.length>24000)return;
            const points=old?.points||[];
            for(let i=0;i<packet.points.length;i++)points[offset+i]=packet.points[i];
            if(packet.cancel)points.length=0;
            this.paths.set(key,{...packet,offset:0,points,updatedAt:Date.now()});
            this.frame++;
            for(const [id,path]of this.paths)if(Date.now()-path.updatedAt>15000)this.paths.delete(id);
            while(this.paths.size>24)this.paths.delete(this.paths.keys().next().value);
            this.scheduleFrame();
        }
        scheduleFrame(){
            if(this.stopped||this.frameTimer||this.writing||this.sentFrame===this.frame)return;
            this.frameTimer=setTimeout(()=>{this.frameTimer=null;void this.flushFrame();},
                Math.max(0,this.lastFrameAt+FRAME_MS-Date.now(),this.retryAt-Date.now()));
        }
        async flushFrame(){
            if(this.stopped||this.writing||this.sentFrame===this.frame)return;
            this.writing=true;this.lastFrameAt=Date.now();
            try{
                const frame=this.frame;
                const packets=[...this.paths.values()].map(path=>({...path,points:path.points.slice(0, path.points.findIndex(point=>!point)<0?path.points.length:path.points.findIndex(point=>!point))}));
                const raw=JSON.stringify({v:1,docId:this.docId,actor:this.actor,frame,at:Date.now(),packets});
                if(raw.length>MAX_BYTES)throw new Error('Live frame is too large; the complete stroke stays in the durable journal');
                const folder=await this.ensureFolder();
                if(!this.fileId){const reserved=await(await this.request(`${API}/generateIds?count=1&space=drive&type=files`)).json();this.fileId=reserved.ids?.[0];}
                if(!this.fileId)throw new Error('Drive did not reserve the live preview');
                const update=()=>this.request(`${UPLOAD}/${this.fileId}?uploadType=media&fields=id`,{
                    method:'PATCH',headers:{'Content-Type':'application/json'},body:raw,bufferResponse:true});
                let response;
                if(this.exists)response=await update();
                else{
                    const boundary=`ink_${crypto.randomUUID()}`;
                    const meta={id:this.fileId,name:`live-${this.docId}-${this.actor}.json`,mimeType:'application/json',parents:[folder],
                        appProperties:{ihnKind:KIND,ihnDoc:this.docId,ihnActor:this.actor}};
                    try{response=await this.request(`${UPLOAD}?uploadType=multipart&fields=id`,{method:'POST',
                        headers:{'Content-Type':`multipart/related; boundary=${boundary}`},bufferResponse:true,
                        body:new Blob([`--${boundary}\r\nContent-Type: application/json\r\n\r\n`,JSON.stringify(meta),
                            `\r\n--${boundary}\r\nContent-Type: application/json\r\n\r\n`,raw,`\r\n--${boundary}--\r\n`])});}
                    catch(error){
                        if(error.status!==409)throw error;
                        const meta=await(await this.request(`${API}/${this.fileId}?fields=appProperties`)).json();
                        if(meta.appProperties?.ihnActor!==this.actor||meta.appProperties?.ihnDoc!==this.docId||meta.appProperties?.ihnKind!==KIND)throw new Error('Live preview identity mismatch');
                        response=await update();
                    }
                }
                const receipt=await response.json();if(receipt.id!==this.fileId)throw new Error('Live preview was not confirmed');
                this.exists=true;this.sentFrame=frame;this.retryAt=0;
                this.notifyPeers(this.docId,this.fileId,true);
            }catch(error){
                if(error.status===404){this.exists=false;this.fileId='';this.folder=null;}
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
        async pollFrames(){
            if(this.stopped||this.reading)return;
            this.reading=true;
            try{
                const now=Date.now();
                if(now>=this.nextList&&!this.discovery){
                    this.nextList=now+5000;
                    this.discovery=this.discoverPreviews().catch(()=>{}).finally(()=>{this.discovery=null;});
                }
                const due=[...this.previewSources].filter(([id,s])=>id!==this.fileId&&s.next<=now)
                    .sort((a,b)=>b[1].activeUntil-a[1].activeUntil||a[1].next-b[1].next).slice(0,4);
                await Promise.allSettled(due.map(async([id,source])=>{
                    source.next=now+2000;
                    try{
                        const response=await this.request(`${API}/${id}?alt=media`,{headers:{'Cache-Control':'no-cache'},bufferResponse:true});
                        const raw=await response.text();if(this.stopped||raw.length>MAX_BYTES)return;
                        const value=JSON.parse(raw);
                        if(value.v!==1||value.docId!==this.docId||value.actor===this.actor||!Number.isSafeInteger(value.frame)
                            ||!Array.isArray(value.packets)||value.packets.length>24||!Number.isFinite(value.at))return;
                        if(now-value.at>20000||value.at>now+30000){source.activeUntil=0;source.next=now+15000;return;}
                        if(value.frame>source.frame){
                            for(const packet of value.packets){
                                if(packet?.fileId!==this.docId||packet.type!=='live-stroke'||!Array.isArray(packet.points)||packet.points.length>24000)continue;
                                this.host.preview?.(packet);
                            }
                            source.frame=value.frame;source.activeUntil=Date.now()+5000;
                        }
                        source.next=Date.now()+(source.activeUntil>Date.now()?FRAME_MS:2000);
                    }catch(_){source.next=Date.now()+3000;}
                }));
            }finally{this.reading=false;this.schedulePoll();}
        }
        start(){if(this.host.canPublish!==false){this.frame++;this.scheduleFrame();}this.schedulePoll(0);return this;}
        stop(){super.stop();clearTimeout(this.frameTimer);clearTimeout(this.pollTimer);this.paths.clear();this.previewSources.clear();}
    }
    root.InhouseDriveLivePreview=LivePreview;
    if(typeof module==='object'&&module.exports)module.exports={LivePreview};
})(globalThis);
