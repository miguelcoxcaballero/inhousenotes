import {test,expect} from '@playwright/test';
test.beforeEach(async({page})=>{await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());});
test('PDF commands wait before dispatch and before delivering a result; checkpoint queue remains available',async({page})=>{
 const result=await page.evaluate(async()=>{
  const prototype=InhouseRuntimeCore.WorkerRpcClient.prototype,original=prototype.run;
  const calls=[];let release;
  prototype.run=function(type){calls.push(type);return type==='pdfCreate'?new Promise(resolve=>release=resolve):Promise.resolve('checkpoint');};
  const pause=()=>new Promise(r=>setTimeout(r,80));
  try{
   isPenWriting=true;let delivered=false;
   const pdf=InhouseDocumentProcessing.run('pdfCreate',{}).then(value=>{delivered=true;return value;});
   const checkpoint=await InhouseDocumentProcessing.run('packInkCheckpoint',{});
   await pause();const before=calls.slice();
   isPenWriting=false;lastUserInteractionAt=0;
   while(!release)await pause();
   isPenWriting=true;release({id:'ready'});
   await pause();const during=delivered;
   isPenWriting=false;lastUserInteractionAt=0;
   const after=await pdf;
   return {before,checkpoint,during,after};
  }finally{isPenWriting=false;prototype.run=original;}
 });expect(result).toEqual({before:['packInkCheckpoint'],checkpoint:'checkpoint',during:false,after:{id:'ready'}});
});
test('upload pauses between chunks without cancelling or resending accepted bytes',async({page})=>{
 const result=await page.evaluate(async()=>{
  const original=driveFetch;let started;const first=new Promise(r=>started=r);const ranges=[];let finished=false;
  driveFetch=async(url,options)=>{
   if(options.method==='POST')return new Response('',{status:200,headers:{Location:'https://upload.test/session'}});
   ranges.push(options.headers['Content-Range']);
   if(ranges.length===1){isPenWriting=true;started();return new Response('',{status:308,headers:{Range:'bytes=0-1048575'}});}
   return new Response(JSON.stringify({id:'saved'}),{status:200});
  };
  try{
   const upload=uploadDriveBlobResumable({url:'https://upload.test/start',method:'POST',blob:new Blob([new Uint8Array(1048586)])}).then(r=>{finished=true;return r;});
   await first;await new Promise(r=>setTimeout(r,120));const during={chunks:ranges.length,finished};
   isPenWriting=false;lastUserInteractionAt=0;const saved=await upload;
   return {during,ranges,id:saved.id};
  }finally{isPenWriting=false;driveFetch=original;}
 });expect(result).toEqual({during:{chunks:1,finished:false},ranges:['bytes 0-1048575/1048586','bytes 1048576-1048585/1048586'],id:'saved'});
});
test('aborting work waiting for the pen never starts an upload',async({page})=>{
 const result=await page.evaluate(async()=>{
  const original=driveFetch;let requests=0;driveFetch=async()=>{requests++;throw Error('must not start');};
  const controller=new AbortController();isPenWriting=true;
  try{
   const work=uploadDriveBlobResumable({url:'https://upload.test/start',method:'POST',blob:new Blob(['data']),options:{signal:controller.signal}}).catch(e=>e.name);
   await new Promise(r=>setTimeout(r,60));controller.abort();return {error:await work,requests};
  }finally{isPenWriting=false;driveFetch=original;}
 });expect(result).toEqual({error:'AbortError',requests:0});
});
