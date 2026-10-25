import { test, expect } from '@playwright/test';

test.beforeEach(async ({page}) => {
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
 await page.evaluate(async()=>{await __IHN_TEST_API__.resetLocalDocument(1,'Local recovery');await __IHN_TEST_API__.checkpointBeforeLeaving();});
});

test('editing during point packing defers the obsolete checkpoint and saves the latest ink automatically',async({page})=>{
 const result=await page.evaluate(async()=>{
  const clone=cloneSanitizedPageForStorage;
  let changed=false;
  cloneSanitizedPageForStorage=async(...args)=>{
   const snapshot=await clone(...args);
   if(!changed){changed=true;state.pages[0].strokes.push({id:'during-pack',tool:'pen',color:'#000000',width:2,points:[{x:10,y:20},{x:30,y:40}]});markPageDirty(0,'full');}
   return snapshot;
  };
  markPageDirty(0,'full');const outcome=await queueSave();
  cloneSanitizedPageForStorage=clone;
  return {deferred:outcome.deferred,error:getLocalSaveStatus().error,pending:getLocalSaveStatus().pending};
 });
 expect(result).toEqual({deferred:true,error:'',pending:true});
 await page.waitForFunction(()=>!hasPendingLocalSave());
 expect(await page.evaluate(async()=>({error:getLocalSaveStatus().error,ids:(await loadPageFromIndexedDb(0)).strokes.map(s=>s.id)}))).toEqual({error:'',ids:['during-pack']});
});

test('page-order lock defers local saving without Needs action and resumes when released',async({page})=>{
 const result=await page.evaluate(async()=>{
  localPageStructureMutationToken={};markPageDirty(0,'full');
  const result=await queueSave();
  const error=getLocalSaveStatus().error;
  localPageStructureMutationToken=null;
  return {deferred:result.deferred,error};
 });
 expect(result).toEqual({deferred:true,error:''});
 await page.waitForFunction(()=>!hasPendingLocalSave());
 expect(await page.evaluate(()=>getLocalSaveStatus().error)).toBe('');
});

test('real IndexedDB failure remains visible until the automatic retry commits',async({page})=>{
 const error=await page.evaluate(async()=>{
  const put=IDBObjectStore.prototype.put;let failed=false;
  IDBObjectStore.prototype.put=function(...args){if(this.name===PAGE_STORE&&!failed){failed=true;throw new DOMException('Transient test failure','UnknownError');}return put.apply(this,args);};
  state.pages[0].strokes.push({id:'recover-me',tool:'pen',color:'#000000',width:2,points:[{x:1,y:2},{x:3,y:4}]});markPageDirty(0,'full');
  await queueSave();IDBObjectStore.prototype.put=put;
  return getLocalSaveStatus().error;
 });
 expect(error).not.toBe('');
 await page.waitForFunction(()=>!hasPendingLocalSave()&&!getLocalSaveStatus().error);
 expect(await page.evaluate(async()=>(await loadPageFromIndexedDb(0)).strokes[0].id)).toBe('recover-me');
});

test('an edit during metadata commit is not reported as fully saved until retried',async({page})=>{
 const result=await page.evaluate(async()=>{
  const save=saveToIndexedDb;let changed=false;
  saveToIndexedDb=async(...args)=>{
   const result=await save(...args);
   if(!changed){changed=true;state.pages[0].strokes.push({id:'during-meta',tool:'pen',color:'#000000',width:2,points:[{x:10,y:20},{x:30,y:40}]});markPageDirty(0,'full');}
   return result;
  };
  markPageDirty(0,'full');const outcome=await queueSave();saveToIndexedDb=save;
  return {deferred:outcome.deferred,error:getLocalSaveStatus().error,pending:getLocalSaveStatus().pending};
 });
 expect(result).toEqual({deferred:true,error:'',pending:true});
 await page.waitForFunction(()=>!hasPendingLocalSave());
 expect(await page.evaluate(async()=>(await loadPageFromIndexedDb(0)).strokes[0].id)).toBe('during-meta');
});
