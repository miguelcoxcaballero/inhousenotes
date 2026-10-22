import {test,expect} from '@playwright/test';
const account='priority@example.test';
test.beforeEach(async({page})=>{
 await page.route('https://accounts.google.com/**',r=>r.fulfill({body:''}));
 await page.addInitScript(account=>{localStorage.setItem('drive-hint-email',account);localStorage.setItem('drive-remember-v1','1');},account);
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
});
test('Save now waits for the in-flight upload then saves the open document ahead of the backlog',async({page})=>{
 const result=await page.evaluate(async account=>{
  const S=InhouseOfflineStore;await __IHN_TEST_API__.resetLocalDocument(1,'Priority');state.driveFileId='c';state.offlineLocalId=null;
  driveOpenInProgress=true;
  for(const id of ['a','b','c'])await S.checkpoint(account,id,{blob:new Blob([id]),baseMeta:{version:'1'},entry:{id,parents:['root']}});
  let release,started;const held=new Promise(resolve=>release=resolve),entered=new Promise(resolve=>started=resolve),order=[];
  const fetch=async(url,o={})=>{
   const id=new URL(url).pathname.split('/').pop();
   if(o.method==='PATCH'){order.push(id);if(id==='a'){started();await held;}return Response.json({id,version:'2'});}
   return Response.json({id,version:'1'},{headers:{ETag:'"v1"'}});
  };
  const original=queueDriveSave;
  queueDriveSave=()=>InhouseOfflineSync.run(account,{fetch,targetId:'c',skip:id=>id!=='c'});
  try{
   const background=InhouseOfflineSync.run(account,{fetch,background:true});await entered;
   const manual=InhouseOfflineApp.saveNow();
   while(!(await S.read('settings',account,'sync-priority'))?.documentId)await new Promise(resolve=>setTimeout(resolve,5));
   release();await background;await manual;
   const afterManual=[...order],priority=await S.read('settings',account,'sync-priority');
   await InhouseOfflineSync.run(account,{fetch,background:true});
   return {afterManual,order,priority,pending:(await S.list('outbox',account)).length};
  }finally{release();queueDriveSave=original;driveOpenInProgress=false;}
 },account);
 expect(result.afterManual).toEqual(['a','c']);expect(result.order).toEqual(['a','c','b']);expect(result.priority).toBeNull();expect(result.pending).toBe(0);
});
test('expired manual priority cannot block unattended synchronization',async({page})=>{
 const result=await page.evaluate(async account=>{
  const S=InhouseOfflineStore;
  await S.checkpoint(account,'pending',{blob:new Blob(['ink']),baseMeta:{version:'1'},entry:{parents:['root']}});
  await S.put('settings',account,'sync-priority',{documentId:'closed-document',expires:Date.now()-1});
  return InhouseOfflineSync.run(account,{background:true,fetch:async(url,o={})=>Response.json({id:'pending',version:o.method?'2':'1'},{headers:{ETag:'"v1"'}})});
 },account);
 expect(result.saved).toBe(1);expect(result.errors).toEqual([]);
});
