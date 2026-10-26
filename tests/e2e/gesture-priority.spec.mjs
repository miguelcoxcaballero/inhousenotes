import {test,expect} from '@playwright/test';

for(const input of ['touch','wheel','keyboard'])test(`${input} interrupts an active PDF worker and upload immediately`,async({page})=>{
 await page.addInitScript(()=>{
  const NativeWorker=Worker;
  window.blockProcessing=false;window.processingPosted=false;window.processingTerminated=0;
  window.Worker=class extends NativeWorker{
   constructor(url,...args){super(url,...args);this.processing=String(url).includes('document-processing-worker');}
   postMessage(message,...args){if(this.processing&&blockProcessing){window.processingPosted=true;return;}super.postMessage(message,...args);}
   terminate(){if(this.processing)window.processingTerminated++;super.terminate();}
  };
 });
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
 await page.evaluate(async()=>{
  await __IHN_TEST_API__.resetLocalDocument(2,'Gesture priority');await __IHN_TEST_API__.showEditorForTest();
  window.blockProcessing=true;window.processingTerminated=0;
  window.activeUpload=new AbortController();activeDriveSaveController=activeUpload;
  window.job=InhouseDocumentProcessing.run('pdfReady',{}).then(()=> 'unexpected',e=>e.name);
 });
 await page.waitForFunction(()=>processingPosted);
 const result=await page.evaluate(async input=>{
  const target=canvasViewport;
  if(input==='touch')target.dispatchEvent(new PointerEvent('pointerdown',{bubbles:true,pointerId:71,pointerType:'touch',buttons:1,clientX:200,clientY:300}));
  else if(input==='wheel')target.dispatchEvent(new WheelEvent('wheel',{bubbles:true,cancelable:true,deltaY:20,clientX:200,clientY:300}));
  else target.dispatchEvent(new KeyboardEvent('keydown',{bubbles:true,key:'ArrowDown'}));
  const value={aborted:activeUpload.signal.aborted,terminated:processingTerminated,error:await job};
  target.dispatchEvent(new PointerEvent('pointerup',{bubbles:true,pointerId:71,pointerType:'touch',clientX:200,clientY:300}));
  window.blockProcessing=false;activeDriveSaveController=null;
  return value;
 },input);
 expect(result).toEqual({aborted:true,terminated:1,error:'AbortError'});
 await page.evaluate(()=>InhouseDocumentProcessing.run('pdfReady',{}));
});

test('wheel and pinch frames reuse geometry and do not resize canvases mid-gesture',async({page})=>{
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
 await page.evaluate(async()=>{await __IHN_TEST_API__.resetLocalDocument(8,'Frames');await __IHN_TEST_API__.showEditorForTest();});
 await page.waitForTimeout(600);
 const result=await page.evaluate(async()=>{
  const read=canvasViewport.getBoundingClientRect.bind(canvasViewport),sync=syncActivePageCanvasResolution;
  let reads=0,resizes=0;canvasViewport.getBoundingClientRect=()=>{reads++;return read();};syncActivePageCanvasResolution=()=>resizes++;
  const frame=()=>new Promise(requestAnimationFrame);
  viewportGestureGeometry=null;const before=state.zoom;
  try{
   for(let i=0;i<25;i++){
    canvasViewport.dispatchEvent(new WheelEvent('wheel',{bubbles:true,cancelable:true,ctrlKey:true,deltaY:-1,clientX:200,clientY:300}));await frame();
   }
   const wheel={reads,resizes,zoom:state.zoom,transform:canvasContainer.style.transform};
   state.activePointers.set(81,{clientX:160,clientY:200});state.activePointers.set(82,{clientX:260,clientY:200});
   state.initialScale=state.zoom;state.initialPinchDist=100;state.pinchCenterX=210;state.pinchCenterY=200;state.initialPanX=state.panX;state.initialPanY=state.panY;
   for(let i=0;i<25;i++){handleViewportPointerMove({pointerId:82,pointerType:'touch',clientX:260+i,clientY:200});await frame();}
   const pinch={reads,resizes,transform:canvasContainer.style.transform};
   return {before,wheel,pinch};
  }finally{state.activePointers.clear();state.isPanning=false;canvasViewport.getBoundingClientRect=read;syncActivePageCanvasResolution=sync;}
 });
 console.log('GESTURE_FRAME_WORK',JSON.stringify(result));
 expect(result.wheel.zoom).toBeGreaterThan(result.before);expect(result.wheel.transform).toContain('scale(');
 expect(result.wheel.resizes).toBe(0);expect(result.pinch.resizes).toBe(0);
 expect(result.pinch.reads).toBeLessThanOrEqual(3);
});

test('a stationary held touch keeps background work paused until release',async({page})=>{
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
 const result=await page.evaluate(async()=>{
  await __IHN_TEST_API__.resetLocalDocument(1,'Held');await __IHN_TEST_API__.showEditorForTest();
  canvasViewport.dispatchEvent(new PointerEvent('pointerdown',{bubbles:true,pointerId:91,pointerType:'touch',buttons:1,clientX:200,clientY:300}));
  let resumed=false;const waiting=waitForDocumentBackgroundTurn().then(()=>resumed=true);
  await new Promise(resolve=>setTimeout(resolve,1400));
  const during={resumed,gesture:isGestureInteracting,held:state.activePointers.size};
  canvasViewport.dispatchEvent(new PointerEvent('pointerup',{bubbles:true,pointerId:91,pointerType:'touch',clientX:200,clientY:300}));
  await waiting;return {during,resumed};
 });
 expect(result).toEqual({during:{resumed:false,gesture:true,held:1},resumed:true});
});

test('queued previews wait for gestures and then recover without losing cached page previews',async({page})=>{
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
 await page.evaluate(async()=>{await __IHN_TEST_API__.resetLocalDocument(4,'Preview');await __IHN_TEST_API__.showEditorForTest();queueScrollPagePreviews(true);});
 await page.waitForFunction(()=>state.pages.every(p=>p.preview));
 const during=await page.evaluate(async()=>{
  const cached=state.pages[0].preview;
  state.activePointers.set(92,{clientX:200,clientY:200});state.isPanning=true;noteInteractionActivity();
  state.pages[3].preview=null;scrollPreviewQueue.set(3,0);
  await processScrollPagePreview();await new Promise(resolve=>setTimeout(resolve,300));
  return {pending:scrollPreviewQueue.has(3),unbuilt:!state.pages[3].preview,cached:state.pages[0].preview===cached};
 });
 expect(during).toEqual({pending:true,unbuilt:true,cached:true});
 await page.evaluate(()=>{state.activePointers.clear();state.isPanning=false;});
 await page.waitForFunction(()=>state.pages[3].preview);
});

test('native background sync resumes after gesture cooldown without another user action',async({page,context})=>{
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());await context.setOffline(true);
 await page.evaluate(async()=>{
  await InhouseOfflineLibrary.useAccount('gesture-native@example.test');
  await __IHN_TEST_API__.resetLocalDocument(1,'Native');await __IHN_TEST_API__.showEditorForTest();
  window.stops=0;window.restarts=0;window.InhouseBackground={cancel:()=>stops++,schedule:()=>restarts++};
  noteInteractionActivity();
 });
 await page.waitForTimeout(350);
 expect(await page.evaluate(()=>({stops,restarts}))).toEqual({stops:1,restarts:0});
 await page.waitForFunction(()=>restarts>0);
});

test('losing focus releases held viewport pointers so background work can resume',async({page})=>{
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
 const result=await page.evaluate(async()=>{
  await __IHN_TEST_API__.resetLocalDocument(1,'Focus');await __IHN_TEST_API__.showEditorForTest();
  state.activePointers.set(99,{clientX:100,clientY:100});state.isPanning=true;noteInteractionActivity();
  window.dispatchEvent(new Event('blur'));
  await waitForDocumentBackgroundTurn();
  return {pointers:state.activePointers.size,panning:state.isPanning,inertia:state.inertiaFrame,busy:hasSmoothInteraction()};
 });
 expect(result).toEqual({pointers:0,panning:false,inertia:null,busy:false});
});
