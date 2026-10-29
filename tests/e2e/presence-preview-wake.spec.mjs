import {test,expect} from '@playwright/test';
import {fakeInkDrive} from '../helpers/fake-ink-drive.mjs';

// Read-only viewers cannot write presence properties and have no P2P link, so
// a writer watched only by viewers sees nobody and keeps its 20 s solo
// heartbeat. A viewer's preview reader sleeps once the newest frame is 20 s
// old and wakes early only on a new heartbeat from the writer. The live
// preview publisher therefore has to trigger a heartbeat itself after the
// first frame that follows a quiet spell.
const DOC='wake-doc';
async function setup(page,api){
 api.files.set(DOC,{id:DOC,properties:{},appProperties:{}});
 await page.exposeFunction('wakeRequest',(url,options)=>api.request(url,options));
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
 await page.evaluate(async doc=>{
  await InhouseOfflineLibrary.useAccount('wake-writer@example.test');
  await __IHN_TEST_API__.resetLocalDocument(1,'Wake');
  stopCollabPolling();beginDocumentSession();InhouseDriveInk.stop();
  state.driveFileId=doc;state.driveCanEdit=true;state.driveAutosave=false;state.pages[0].pageId='page-one';
  state.collabStructure=null;state.collabFields=null;driveDirty=false;resetDriveContentSyncVersion();
  driveAccessToken='wake-token';driveUserProfile={email:'wake-writer@example.test',name:'Writer',photo:''};
  latestPresenceUsers=[];liveCollabUpdatePeers=()=>{};ihnConsumeSignalMailboxesFromProperties=async()=>{};
  startLiveCollaboration=()=>{};
  window.wakeLog=[];
  driveFetch=async(url,options={})=>{
   const body=typeof options.body==='string'?options.body:await options.body?.text?.();
   const parsed=new URL(url),at=Date.now();
   const result=await wakeRequest(url,{method:options.method,headers:options.headers,body});
   if(parsed.searchParams.get('uploadType'))wakeLog.push({kind:'frame',at});
   else if(options.method==='PATCH'&&parsed.pathname.endsWith(`/files/${doc}`)){
    const keys=Object.keys(JSON.parse(body).properties||{});
    // A heartbeat writes only the packed runtime field ('<prefix>_r').
    if(keys.length===1&&/^p_.+_r$/.test(keys[0]))
     wakeLog.push({kind:'beat',at,h:parseInt(JSON.parse(body).properties[keys[0]].split('|')[0],36)});
   }
   return new Response(result.body,{status:result.status,headers:{'Content-Type':'application/json'}});
  };
  const Journal=InhouseDriveInkJournal.Journal;
  InhouseDriveInkJournal.Journal=class extends Journal{start(){return this;}wake(){}};
  const Live=InhouseDriveLivePreview;
  InhouseDriveLivePreview=class extends Live{constructor(host){super(host);window.wakeLive=this;}start(){return this;}scheduleFrame(){}schedulePoll(){}};
  InhouseDriveInk.start();await InhouseDriveInk.capture();
  startPresencePing();
 },DOC);
}
const frame=(page,points)=>page.evaluate(async points=>{
 const stroke={id:'unfinished',tool:'pen',color:'#0000ff',width:2,points};
 publishLiveStrokePreview('page-one',stroke);ihnFlushLiveStrokePreview('unfinished');
 await wakeLive.flushFrame();
},points);

test('alone, the first live-preview frame after a quiet spell sends a presence heartbeat at once',async({page})=>{
 const api=fakeInkDrive();await setup(page,api);
 // The startup beat runs, then the solo cadence (next beat 20 s later).
 await expect.poll(()=>page.evaluate(()=>wakeLog.filter(e=>e.kind==='beat').length),{timeout:6000}).toBeGreaterThan(0);
 await page.waitForTimeout(500);
 expect(await page.evaluate(()=>[getCollabPresenceMode(),getPresenceHeartbeatDelay()])).toEqual(['solo',20000]);
 const before=await page.evaluate(()=>wakeLog.length);
 await frame(page,[{x:10,y:10,p:.5},{x:20,y:20,p:.5}]);
 await expect.poll(()=>page.evaluate(n=>wakeLog.slice(n).filter(e=>e.kind==='beat').length,before),{timeout:1500,intervals:[20,50,100]}).toBe(1);
 const first=await page.evaluate(n=>wakeLog.slice(n),before);
 const sent=first.find(e=>e.kind==='frame'),beat=first.find(e=>e.kind==='beat');
 expect(sent).toBeTruthy();
 // A viewer that sees this heartbeat reads a frame that already exists.
 expect(beat.h).toBeGreaterThanOrEqual(sent.at);
 expect(beat.at-sent.at).toBeLessThan(1000);
 console.log('PREVIEW_WAKE_BEAT_MS',beat.at-sent.at);
 // More frames of the same burst cost no further heartbeat.
 const burst=await page.evaluate(()=>wakeLog.length);
 await frame(page,[{x:10,y:10,p:.5},{x:20,y:20,p:.5},{x:30,y:30,p:.5}]);
 await page.waitForTimeout(300);
 await frame(page,[{x:10,y:10,p:.5},{x:20,y:20,p:.5},{x:30,y:30,p:.5},{x:40,y:40,p:.5}]);
 await page.waitForTimeout(1200);
 const after=await page.evaluate(n=>wakeLog.slice(n),burst);
 expect(after.filter(e=>e.kind==='frame').length).toBe(2);
 expect(after.filter(e=>e.kind==='beat').length).toBe(0);
 await page.evaluate(()=>{stopPresencePing();InhouseDriveInk.stop();});
});
