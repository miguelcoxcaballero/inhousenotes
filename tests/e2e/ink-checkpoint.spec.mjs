import {test,expect} from '@playwright/test';
for(const mode of ['rust','no-wasm','no-worker'])test(`binary ink checkpoints recover exact points without loss (${mode})`,async({page,context})=>{
 const disableWasm=mode!=='rust';
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
 const stored=await page.evaluate(async()=>{
  await __IHN_TEST_API__.resetLocalDocument(1,'Binary ink');
  const points=[{x:-0,y:2,p:0},{x:1.123456789012345,y:400},{x:-15,y:22,p:.123456789}];
  state.pages[0].strokes=[{id:'exact',tool:'pen',color:'#123456',width:1.25,points},
   {id:'extra',tool:'pen',width:2,points:[{x:1,y:2,p:.6,tiltX:7}]},
   {id:'empty',tool:'pen',points:[]}];
  state.pages[0].legacyCoverStrokes=[{id:'legacy-cover',points:[{x:2,y:3,p:.7}]}];
  markPageDirty(0,'full');await __IHN_TEST_API__.checkpointBeforeLeaving();
  const db=await openNotebookDb();return new Promise(resolve=>{
   const request=db.transaction(PAGE_STORE,'readonly').objectStore(PAGE_STORE).get(0);
   request.onsuccess=()=>resolve({codec:request.result.__inkCheckpoint.codec,range:request.result.strokes[0].points.inkRange});
  });
 });
 expect(stored.codec).toBe('xor-v1');expect(stored.range).toEqual([0,3]);
 if(disableWasm)await context.route('**/wasm/inhouse_notes_core.wasm*',route=>route.abort());
 await page.reload();await page.evaluate(()=>__IHN_TEST_API__.ready());
 const actual=await page.evaluate(async mode=>{
  if(mode==='no-worker'){
   const prototype=InhouseRuntimeCore.WorkerRpcClient.prototype,run=prototype.run;
   prototype.run=function(type,...args){return type==='unpackInkCheckpoint'?Promise.reject(new Error('Checkpoint worker unavailable')):run.call(this,type,...args);};
  }
  const p=await loadPageFromIndexedDb(0),points=p.strokes[0].points;
  return {negativeZero:Object.is(points[0].x,-0),pressure:points[0].p,noPressure:!Object.hasOwn(points[1],'p'),x:points[1].x,
   other:p.strokes[1].points,empty:p.strokes[2].points,cover:p.legacyCoverStrokes[0].points,width:p.strokes[0].width};
 },mode);
 expect(actual).toEqual({negativeZero:true,pressure:0,noPressure:true,x:1.123456789012345,other:[{x:1,y:2,p:.6,tiltX:7}],empty:[],cover:[{x:2,y:3,p:.7}],width:1.25});
});

test('invalid binary lengths and point ranges are rejected',async({page})=>{
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
 const result=await page.evaluate(async()=>{
  const checkpoint={inkCheckpoint:1,codec:'xor-v1',rawLength:32,bytes:new Uint8Array([9])};
  const error=await InhouseDocumentProcessing.run('unpackInkCheckpoint',checkpoint).then(()=>null,e=>e.message);
  const pageError=await unpackInkPageFromStorage({strokes:[{points:{inkRange:[0,99]}}],__inkCheckpoint:{inkCheckpoint:1,codec:'raw-v1',rawLength:32,bytes:new Uint8Array(32)}}).then(()=>null,e=>e.message);
  return {error,pageError};
 });
 expect(result.error).toMatch(/Invalid|Truncated/);expect(result.pageError).toMatch(/Invalid ink point range/);
});
