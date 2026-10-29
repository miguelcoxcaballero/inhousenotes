import {test,expect} from '@playwright/test';

async function setup(page,pageCount=128){
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
 await page.evaluate(async pageCount=>{
  await InhouseOfflineLibrary.useAccount('context-projection@example.test');
  await __IHN_TEST_API__.resetLocalDocument(1,'Projection notebook');
  stopCollabPolling();beginDocumentSession();InhouseDriveInk.stop();
  state.driveFileId='context-projection';state.driveCanEdit=true;state.driveAutosave=false;
  state.pages[0].pageId='projection-0';
  for(let i=1;i<pageCount;i++)state.pages.push({...createBlankPageData(),pageId:`projection-${i}`});
  state.collabStructure=null;state.collabFields=null;driveDirty=false;resetDriveContentSyncVersion();
  ensureCollabFieldMetadata();
  // Preserve forward-compatible non-page fields and JavaScript's numeric key
  // ordering, while never exporting another page's private/future register.
  for(const key of ['z:future','10','a:future','2','page:projection-0:future']){
   state.collabFields.fields[key]={value:{label:key},stamp:{clock:5,actor:'projection-writer'}};
  }
  const Journal=InhouseDriveInkJournal.Journal;
  InhouseDriveInkJournal.Journal=class extends Journal{
   constructor(host){super({...host,actor:'projection-writer'});window.projectionJournal=this;}
   start(){return this;}wake(){}
  };
  InhouseDriveLivePreview=null;startLiveCollaboration=()=>{};
  schedulePreparedDrivePdf=()=>{};scheduleDriveSync=()=>{};
  InhouseDriveInk.start();await InhouseDriveInk.capture();
  window.projectionRaw=()=>[...projectionJournal.records.values()].flatMap(record=>record.ops);
  window.projectionPhotos=colors=>{
   const canvas=document.createElement('canvas');canvas.width=canvas.height=4;const ctx=canvas.getContext('2d');
   return colors.map(color=>{ctx.fillStyle=color;ctx.fillRect(0,0,4,4);return canvas.toDataURL('image/png');});
  };
  window.instrumentProjection=()=>{
   const entries=Object.entries,snapshot=getCollabFieldSnapshot,enqueue=projectionJournal.enqueue;
   const tracked=new WeakMap(),snapshots=[],captures=[],expected=[],envelopes=[];
   getCollabFieldSnapshot=()=>{
    const value=snapshot(),record={metadata:value,enumerations:0,enumeratedKeys:0,origin:new Error().stack};
    snapshots.push(record);tracked.set(value.fields,record);return value;
   };
   Object.entries=value=>{
    const result=entries(value),record=tracked.get(value);
    if(record){
     // Other app tasks may take a metadata snapshot during asynchronous IDB
     // writes. Only a map actually enumerated here enters the projection
     // measurement; retain unused origins so this distinction is auditable.
     if(record.enumerations===0)captures.push(record);
     record.enumerations++;record.enumeratedKeys+=result.length;
    }
    return result;
   };
   projectionJournal.enqueue=function(op,version){
    const captured=captures.at(-1).metadata,pageId=op.pages?.[0]?.pageId;
    const allowed=new Set(pageId?['background','template','size'].map(field=>`page:${pageId}:${field}`):[]);
    // The exact previous projection remains the wire-format oracle. Bypass
    // the instrumented enumeration so measuring it does not inflate work.
    const fields=Object.fromEntries(entries(captured.fields).filter(([key])=>!key.startsWith('page:')||allowed.has(key)));
    expected.push(JSON.stringify({...op,fields:{...op.fields,fields}}));
    envelopes.push(op.fields.fields);
    return enqueue.call(this,op,version);
   };
   return {
    captures,expected,envelopes,
    finish(){
     Object.entries=entries;getCollabFieldSnapshot=snapshot;projectionJournal.enqueue=enqueue;
     return {captures:captures.map(({metadata,enumerations,enumeratedKeys})=>({
      enumerations,enumeratedKeys,fieldCount:Object.keys(metadata.fields).length})),
      raw:projectionRaw(),expected,freshEnvelopes:new Set(envelopes).size===envelopes.length,
      unprojectedSnapshots:snapshots.filter(item=>!item.enumerations).map(item=>item.origin)};
    }
   };
  };
 },pageCount);
}

function expectExactProjection(result){
 expect(result.raw).toEqual(result.expected);expect(result.freshEnvelopes).toBe(true);
 for(const captured of result.captures){
  expect(captured.enumerations).toBe(1);expect(captured.enumeratedKeys).toBe(captured.fieldCount);
 }
 for(const raw of result.raw){
  const op=JSON.parse(raw),pageId=op.pages[0]?.pageId,fields=Object.keys(op.fields.fields);
  expect(fields.filter(key=>key.startsWith('page:'))).toEqual(pageId?
   ['background','size','template'].map(field=>`page:${pageId}:${field}`):[]);
  expect(fields).toEqual(expect.arrayContaining(['2','10','a:future','z:future']));
  expect(fields).not.toContain('page:projection-0:future');
 }
}

test('many dirty pages and stroke chunks enumerate the full field map only once without changing serialized edits',async({page})=>{
 await setup(page);
 const result=await page.evaluate(async()=>{
  for(let index=0;index<16;index++){
   state.pages[index].pageWidth+=1;noteCollabPageFieldChanged(state.pages[index],'size');
   InhouseDriveInk.page(index);
  }
  state.pages[0].strokes=Array.from({length:65},(_,i)=>({id:`chunk-stroke-${i}`,tool:'pen',color:'#000000',width:2,
   points:[{x:i,y:10,p:.5},{x:i+10,y:20,p:.75}],syncStamp:{clock:100+i,actor:'projection-writer'}}));
  bumpDriveContentVersion();driveDirty=true;
  const probe=instrumentProjection();
  try{
   await InhouseDriveInk.capture();
   const result=probe.finish();InhouseDriveInk.stop();return result;
  }catch(error){probe.finish();throw error;}
 });
 expect(result.captures).toHaveLength(1);expect(result.raw).toHaveLength(18);expectExactProjection(result);
 const chunks=result.raw.map(JSON.parse).filter(op=>op.pages[0].pageId==='projection-0');
 expect(chunks.map(op=>op.pages[0].strokes.length)).toEqual([32,32,1]);
 expect(chunks.flatMap(op=>op.pages[0].strokes.map(s=>s.id))).toEqual(Array.from({length:65},(_,i)=>`chunk-stroke-${i}`));
 if(result.unprojectedSnapshots.length)console.log('CONTEXT_UNPROJECTED_SNAPSHOTS',result.unprojectedSnapshots);
 console.log('CONTEXT_PROJECTION_WORK',JSON.stringify({pages:128,operations:18,
  enumerations:result.captures[0].enumerations,keys:result.captures[0].enumeratedKeys,
  previousEnumerations:19,previousKeys:result.captures[0].fieldCount*19}));
});

test('document-only updates retain numeric and unknown field order without leaking any page fields',async({page})=>{
 await setup(page,8);
 const result=await page.evaluate(async()=>{
  state.collabFields.fields['z:future']={value:{label:'updated future document field'},stamp:{clock:100,actor:'projection-writer'}};
  state.collabFields.clock=100;bumpDriveContentVersion();driveDirty=true;InhouseDriveInk.change();
  const probe=instrumentProjection();
  try{await InhouseDriveInk.capture();const result=probe.finish();InhouseDriveInk.stop();return result;}
  catch(error){probe.finish();throw error;}
 });
 expect(result.captures).toHaveLength(1);expect(result.raw).toHaveLength(1);expectExactProjection(result);
 const op=JSON.parse(result.raw[0]);expect(op.pages).toEqual([]);
 expect(Object.keys(op.fields.fields).slice(0,2)).toEqual(['2','10']);
 expect(op.fields.fields['z:future'].value).toEqual({label:'updated future document field'});
});

test('a newer photo and document revision during persistence uses a fresh context and leaves the first encoded packet intact',async({page})=>{
 await setup(page,2);
 const result=await page.evaluate(async()=>{
  const [firstA,firstB,nextA,nextB]=projectionPhotos(['#f00','#0f0','#00f','#fc0']);
  const setRevision=(name,photos)=>{
   state.exportName=name;noteCollabDocumentFieldChanged('exportName');
   photos.forEach((photo,index)=>{
    state.pages[index].backgroundSource='custom';state.pages[index].backgroundImage=photo;
    noteCollabPageFieldChanged(state.pages[index],'background');InhouseDriveInk.page(index);
   });
   bumpDriveContentVersion();driveDirty=true;
  };
  setRevision('First revision',[firstA,firstB]);
  const probe=instrumentProjection(),enqueue=projectionJournal.enqueue;
  let release,entered=false;
  const gate=new Promise(resolve=>release=resolve);
  projectionJournal.enqueue=function(...args){
   const pending=enqueue.apply(this,args);
   if(entered)return pending;
   entered=true;return pending.then(()=>gate);
  };
  const capture=InhouseDriveInk.capture();
  try{
   const deadline=performance.now()+3000;
   while(!entered){if(performance.now()>deadline)throw Error('Capture did not reach persistence');await new Promise(r=>setTimeout(r,5));}
   const firstRaw=projectionRaw()[0];
   setRevision('Second revision',[nextA,nextB]);release();await capture;
   const premature=projectionRaw().length;
   projectionJournal.enqueue=enqueue;await InhouseDriveInk.capture();
   const result=probe.finish();InhouseDriveInk.stop();
   return {...result,premature,firstRaw,photos:[firstA,nextA,nextB],
    capturedNames:probe.captures.map(item=>item.metadata.fields[COLLAB_DOC_NAME_KEY].value)};
  }finally{release();probe.finish();}
 });
 expect(result.premature).toBe(1);expect(result.captures).toHaveLength(2);expect(result.raw).toHaveLength(3);
 expectExactProjection(result);expect(result.raw[0]).toBe(result.firstRaw);
 const ops=result.raw.map(JSON.parse);
 expect(ops.map(op=>op.name)).toEqual(['First revision','Second revision','Second revision']);
 expect(result.capturedNames).toEqual(['First revision','Second revision']);
 expect(ops.map(op=>op.pages[0].backgroundImage)).toEqual(result.photos);
 expect(ops.map(op=>op.pages[0].pageId)).toEqual(['projection-0','projection-0','projection-1']);
 for(const op of ops){
  const pageId=op.pages[0].pageId;
  expect(op.fields.fields[`page:${pageId}:background`].value.backgroundSource).toBe('custom');
  expect(op.fields.fields['doc:exportName'].value).toBe(op.name);
 }
});
