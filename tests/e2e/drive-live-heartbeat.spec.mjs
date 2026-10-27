import {test,expect} from '@playwright/test';
import {fakeInkDrive} from '../helpers/fake-ink-drive.mjs';

const DOC='heartbeat-document',SOURCE='heartbeat-preview';
const CLIENT='remote-client-abcdefghijklmnopqrstuvwxyz0123456789';
async function setup(page,api,client=CLIENT){
 const property=`ihn_live_${client.replace(/[^a-zA-Z0-9]/g,'').slice(-24)}`;
 await page.exposeFunction('heartbeatRequest',(url,options)=>api.request(url,options));
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
 await page.evaluate(async({doc,source,property,client})=>{
  await InhouseOfflineLibrary.useAccount('heartbeat-reader@example.test');
  await __IHN_TEST_API__.resetLocalDocument(1,'Heartbeat resume');
  stopCollabPolling();beginDocumentSession();InhouseDriveInk.stop();
  state.driveFileId=doc;state.driveCanEdit=true;state.driveAutosave=false;
  state.pages[0].pageId='heartbeat-page';state.collabStructure=null;state.collabFields=null;
  driveDirty=false;resetDriveContentSyncVersion();
  latestPresenceUsers=[];liveCollabUpdatePeers=()=>{};ihnConsumeSignalMailboxesFromProperties=async()=>{};
  startLiveCollaboration=()=>{};schedulePreparedDrivePdf=()=>{};
  window.heartbeatReceived=[];window.heartbeatReads=[];
  driveFetch=async(url,options={})=>{
   if(url.includes(`/${source}?alt=media`))heartbeatReads.push(performance.now());
   const body=typeof options.body==='string'?options.body:await options.body?.text?.();
   const r=await heartbeatRequest(url,{method:options.method,headers:options.headers,body});
   return new Response(r.body,{status:r.status,headers:{'Content-Type':'application/json'}});
  };
  const Journal=InhouseDriveInkJournal.Journal;
  InhouseDriveInkJournal.Journal=class extends Journal{start(){return this;}wake(){}};
  const Live=InhouseDriveLivePreview;
  InhouseDriveLivePreview=class extends Live{
   constructor(host){super({...host,preview:packet=>{heartbeatReceived.push({packet,at:performance.now()});host.preview?.(packet);}});window.heartbeatLive=this;}
   start(){return this;}scheduleFrame(){}
  };
  InhouseDriveInk.start();await InhouseDriveInk.capture();
  window.heartbeatFixture={doc,source,property,client};
  window.sleepHeartbeatSource=()=>{
   clearTimeout(heartbeatLive.pollTimer);heartbeatLive.pollTimer=null;
   const now=Date.now(),entry=heartbeatLive.previewSources.get(source)||{frame:1};
   entry.next=now+15000;entry.activeUntil=0;heartbeatLive.previewSources.set(source,entry);
   heartbeatLive.nextList=now+60000;heartbeatLive.schedulePoll(15000);
   return entry.next;
  };
  window.deliverHeartbeat=(h,{clientOverride=client,t=Date.now(),file=doc,includePresence=true,propertyOverride=property,packed=false}={})=>{
   const props={[propertyOverride]:source};
   // Parse actual public presence properties; do not inject its derived user
   // list. Normal revision polling updates presence before passing ink hints.
   if(includePresence&&packed){
    props.p_heartbeatfixture_q=`account|${clientOverride.slice(0,16)}|writer@example.test|Remote tablet|t|Tablet`;
    props.p_heartbeatfixture_r=`${Math.floor(h).toString(36)}||`;
   }else if(includePresence)props.p_heartbeatfixture=JSON.stringify({n:'Remote tablet',e:'writer@example.test',c:clientOverride,t,h});
   updatePresenceFromProps(props);
   const started=performance.now();InhouseDriveInk.hints(file,props);
   return {started,next:heartbeatLive.previewSources.get(source)?.next};
  };
 },{doc:DOC,source:SOURCE,property,client});
 api.files.set(SOURCE,{id:SOURCE,raw:JSON.stringify({v:1,docId:DOC,actor:'remote-writer',frame:2,at:Date.now(),packets:[{
  v:1,type:'live-stroke',fileId:DOC,pageId:'heartbeat-page',actorId:'remote-writer',strokeId:'unfinished-ink',sequence:2,
  offset:0,tool:'pen',color:'#000000',width:2,points:[{x:10,y:20,p:.5},{x:20,y:30,p:.5}]
 }]})});
}

test('a new matching presence heartbeat wakes an existing preview without P2P or a 15 second wait',async({page})=>{
 const api=fakeInkDrive();await setup(page,api,'c1234-abcd');
 const signal=await page.evaluate(()=>{
  const asleepUntil=sleepHeartbeatSource(),h=Date.now();
  const passive=deliverHeartbeat(0,{includePresence:false});
  const active=deliverHeartbeat(h,{packed:true});return {asleepUntil,passive,active,h};
 });
 expect(signal.passive.next).toBe(signal.asleepUntil);
 expect(signal.active.next).toBeLessThan(signal.asleepUntil-14000);
 await expect.poll(()=>page.evaluate(()=>heartbeatReceived.length),{timeout:1500,intervals:[10,20,50]}).toBe(1);
 const result=await page.evaluate(()=>({readAt:heartbeatReads[0],received:heartbeatReceived[0],durable:state.pages[0].strokes.length}));
 expect(result.readAt-signal.active.started).toBeLessThan(500);
 expect(result.received.packet.points.map(p=>p.x)).toEqual([10,20]);expect(result.durable).toBe(0);
 console.log('HEARTBEAT_PREVIEW_RESUME_MS',result.readAt-signal.active.started);
 // Once awakened, the ordinary preview cadence keeps growing the same path
 // without another heartbeat, P2P connection, or durable stroke receipt.
 const changedAt=await page.evaluate(()=>performance.now()),frame=JSON.parse(api.files.get(SOURCE).raw);
 frame.frame++;frame.at=Date.now();frame.packets[0].sequence++;frame.packets[0].points.push({x:30,y:40,p:.5});
 api.files.get(SOURCE).raw=JSON.stringify(frame);
 await expect.poll(()=>page.evaluate(()=>heartbeatReceived.length),{timeout:1500,intervals:[10,20,50]}).toBe(2);
 const next=await page.evaluate(()=>heartbeatReceived[1]);
 expect(next.packet.points.map(p=>p.x)).toEqual([10,20,30]);expect(next.at-changedAt).toBeLessThan(500);
 console.log('HEARTBEAT_PREVIEW_FOLLOWUP_MS',next.at-changedAt);
 await page.evaluate(()=>InhouseDriveInk.stop());
});

test('repeated, stale, future, and mismatched presence cannot repeatedly wake a sleeping preview',async({page})=>{
 const api=fakeInkDrive();await setup(page,api);
 const result=await page.evaluate(()=>{
  const h=Date.now()-100;sleepHeartbeatSource();deliverHeartbeat(h);
  const cases=[
   ['same',()=>deliverHeartbeat(h)],
   ['older',()=>deliverHeartbeat(h-1)],
   ['stale',()=>deliverHeartbeat(h-60000)],
   ['future',()=>deliverHeartbeat(h+60000)],
   ['wrong-client',()=>deliverHeartbeat(h+1,{clientOverride:'unrelated-device'})],
   ['wrong-property',()=>deliverHeartbeat(h+2,{propertyOverride:'ihn_live_anotherDevice'})],
   ['property-only',()=>deliverHeartbeat(0,{includePresence:false})],
   ['fresh-t-only',()=>deliverHeartbeat(0,{t:h+3})],
   ['other-document',()=>deliverHeartbeat(h+4,{file:'different-document'})]
  ];
  const checks=cases.map(([kind,send])=>{const deadline=sleepHeartbeatSource();send();return {kind,deadline,next:heartbeatLive.previewSources.get(heartbeatFixture.source).next};});
  // Rejected notifications must not consume a genuine later heartbeat.
  const deadline=sleepHeartbeatSource();deliverHeartbeat(h+5);
  const next=heartbeatLive.previewSources.get(heartbeatFixture.source).next;
  // A modest sender clock skew is allowed once; replaying that same future
  // heartbeat must not continually reactivate an otherwise idle preview.
  const skewHeartbeat=Date.now()+500,skewDeadline=sleepHeartbeatSource();deliverHeartbeat(skewHeartbeat);
  const skewNext=heartbeatLive.previewSources.get(heartbeatFixture.source).next;
  const repeatDeadline=sleepHeartbeatSource();deliverHeartbeat(skewHeartbeat);
  const repeatNext=heartbeatLive.previewSources.get(heartbeatFixture.source).next;
  InhouseDriveInk.stop();return {checks,deadline,next,skewDeadline,skewNext,repeatDeadline,repeatNext};
 });
 for(const check of result.checks)expect(check.next,check.kind).toBe(check.deadline);
 expect(result.next).toBeLessThan(result.deadline-14000);
 expect(result.skewNext).toBeLessThan(result.skewDeadline-14000);expect(result.repeatNext).toBe(result.repeatDeadline);
});

test('a heartbeat for another document cannot consume activity or leak across a restarted session',async({page})=>{
 const api=fakeInkDrive();await setup(page,api);
 const result=await page.evaluate(async()=>{
  const h=Date.now(),old=sleepHeartbeatSource();deliverHeartbeat(h,{file:'different-document'});
  const ignored=heartbeatLive.previewSources.get(heartbeatFixture.source).next;
  deliverHeartbeat(h);const accepted=heartbeatLive.previewSources.get(heartbeatFixture.source).next;
  const oldLive=heartbeatLive,sessionBefore=getDocumentSessionToken();
  beginDocumentSession();InhouseDriveInk.stop();latestPresenceUsers=[];
  state.driveFileId=heartbeatFixture.doc;driveDirty=false;resetDriveContentSyncVersion();
  InhouseDriveInk.start();await InhouseDriveInk.capture();
  const freshDeadline=sleepHeartbeatSource();deliverHeartbeat(h);
  const freshNext=heartbeatLive.previewSources.get(heartbeatFixture.source).next;
  InhouseDriveInk.stop();
  return {old,ignored,accepted,freshDeadline,freshNext,oldStopped:oldLive.stopped,newSession:getDocumentSessionToken()!==sessionBefore};
 });
 expect(result.ignored).toBe(result.old);expect(result.accepted).toBeLessThan(result.old-14000);
 expect(result.oldStopped).toBe(true);expect(result.newSession).toBe(true);
 expect(result.freshNext).toBeLessThan(result.freshDeadline-14000);
});
