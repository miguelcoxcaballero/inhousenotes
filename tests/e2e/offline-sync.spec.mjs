import {test,expect} from '@playwright/test';
test.beforeEach(async({page})=>{
 await page.route('https://accounts.google.com/**',r=>r.fulfill({status:200,body:''}));
 await page.goto('/?e2e=1');await page.evaluate(()=>window.__IHN_TEST_API__.ready());
});
test('concurrent remote edits are archived, merged and conditionally uploaded',async({page})=>{
 const result=await page.evaluate(async()=>{
  const S=InhouseOfflineStore,a='merge@test',calls=[];
  await S.checkpoint(a,'doc',{blob:new Blob(['local']),baseMeta:{version:'1'},entry:{name:'Notes.pdf',parents:['root']}});
  const meta={id:'doc',version:'2',headRevisionId:'h2',capabilities:{canEdit:true}};
  const adapter={fetch:async(url,options={})=>{
   calls.push({url,method:options.method,match:options.headers?.['If-Match']});
   if(options.method==='PATCH')return Response.json({...meta,version:'3'});
   if(url.includes('alt=media'))return new Response('remote');
   return Response.json(meta,{headers:{ETag:'"v2"'}});
  },merge:async(doc,remote,blob)=>{
   const recoveries=await S.list('recoveries',a);
   if(recoveries.length!==1||await blob.text()!=='remote')throw Error('Missing originals');
   return new Blob(['local+remote']);
  }};
  const summary=await InhouseOfflineSync.run(a,adapter);
  const doc=await S.read('documents',a,'doc'),recovery=(await S.list('recoveries',a))[0];
  return {summary,content:await doc.blob.text(),dirty:doc.dirty,pending:(await S.list('outbox',a)).length,
   originals:[await recovery.localBlob.text(),await recovery.remoteBlob.text()],match:calls.find(c=>c.method==='PATCH')?.match};
 });
 expect(result).toEqual({summary:{saved:1,pending:0,errors:[]},content:'local+remote',dirty:false,pending:0,originals:['local','remote'],match:'"v2"'});
});
test('conditional write failure retains local content and the pending operation',async({page})=>{
 const result=await page.evaluate(async()=>{
  const S=InhouseOfflineStore,a='race@test';
  await S.checkpoint(a,'doc',{blob:new Blob(['local']),baseMeta:{version:'1'},entry:{parents:['root']}});
  const summary=await InhouseOfflineSync.run(a,{fetch:async(url,o={})=>o.method==='PATCH'?new Response('',{status:412}):Response.json({id:'doc',version:'1'},{headers:{ETag:'"v1"'}})});
  const doc=await S.read('documents',a,'doc');return {summary,dirty:doc.dirty,content:await doc.blob.text(),pending:(await S.list('outbox',a)).length};
 });
 expect(result.summary.errors[0].code).toBe(412);expect(result.dirty).toBe(true);expect(result.content).toBe('local');expect(result.pending).toBe(1);
});
test('lost upload response recovers the actual merged server PDF without uploading twice',async({page})=>{
 const result=await page.evaluate(async()=>{
  const S=InhouseOfflineStore,a='receipt@test';
  const first=await S.checkpoint(a,'doc',{blob:new Blob(['local']),baseMeta:{version:'1'},entry:{parents:['root']}});
  let writes=0;
  const summary=await InhouseOfflineSync.run(a,{fetch:async(url,o={})=>{
   if(o.method){writes++;throw Error('Unexpected write');}
   if(url.includes('alt=media'))return new Response('merged-server-content');
   return Response.json({id:'doc',version:'2',appProperties:{ihnOfflineReceipt:`doc:${first.generation}`}},{headers:{ETag:'"v2"'}});
  }});
  return {summary,writes,content:await (await S.read('documents',a,'doc')).blob.text()};
 });
 expect(result.summary.saved).toBe(1);expect(result.writes).toBe(0);expect(result.content).toBe('merged-server-content');
});
test('folder dependencies resolve before child documents and retries reuse reserved IDs',async({page})=>{
 const result=await page.evaluate(async()=>{
  const S=InhouseOfflineStore,a='tree@test',parent=await S.createFolder(a,'root','Parent'),child=await S.createFolder(a,parent.id,'Child');
  await S.checkpoint(a,'local-doc',{blob:new Blob(['PDF']),entry:{name:'Doc.pdf',parents:[child.id]}});
  let reserved=0;const created=[];
  const summary=await InhouseOfflineSync.run(a,{fetch:async(url,o={})=>{
   if(url.includes('generateIds'))return Response.json({ids:[`remote-${++reserved}`]});
   if(o.method==='POST'){
    if(typeof o.body==='string'){const data=JSON.parse(o.body);created.push(data);return Response.json(data);}
    const text=await o.body.text();created.push({pdf:text});return Response.json({id:'remote-3',version:'1'});
   }
   return new Response('',{status:404});
  }});
  return {summary,reserved,created,pending:(await S.list('outbox',a)).length};
 });
 expect(result.summary.saved).toBe(3);expect(result.pending).toBe(0);expect(result.created[0].parents).toEqual(['root']);
 expect(result.created[1].parents).toEqual(['remote-1']);expect(result.created[2].pdf).toContain('"parents":["remote-2"]');
});
