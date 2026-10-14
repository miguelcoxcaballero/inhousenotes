import {expect,test} from '@playwright/test';

for(const delayMs of [400,600])test(`slow ${delayMs}ms preflight still captures three distinct verified frames`,async({page})=>{
  test.setTimeout(20000);
  await page.goto('/scanner/index.html',{waitUntil:'domcontentloaded'});
  await page.addScriptTag({url:'/tests/e2e/fixtures/scanner-synthetic.js'});
  await page.evaluate(delayMs=>{
    const probe=window.slowReadiness={events:[],frames:[],processed:0,ticks:0};
    probe.timer=setInterval(()=>probe.ticks++,16);
    addEventListener('scanner-capture-readiness',event=>probe.events.push(event.detail));
    Object.defineProperty(navigator.mediaDevices,'getUserMedia',{configurable:true,value:async()=>{
      const canvas=ScannerSyntheticFixtures.renderCase(ScannerSyntheticFixtures.CASES[0]).source;
      const ctx=canvas.getContext('2d');let frame=0;
      const paint=()=>{ctx.fillStyle=`rgb(${++frame%250},20,80)`;ctx.fillRect(0,0,32,32);};
      paint();const stream=canvas.captureStream(30),track=stream.getVideoTracks()[0];
      const refresh=setInterval(paint,33),stop=track.stop.bind(track);
      track.stop=()=>{clearInterval(refresh);stop();};track.applyConstraints=async()=>{};
      return stream;
    }});
    const inspect=ScannerPro.CaptureQuality.inspectReadinessAsync;
    ScannerPro.CaptureQuality.inspectReadinessAsync=async(source,detection,options)=>{
      // Every call runs the real focus/lattice preflight. Delaying its result
      // models a slower phone without replacing any geometric evidence.
      const marker=source.getContext('2d').getImageData(16,16,1,1).data[0];
      const entry={marker,frozenAt:performance.now()};probe.frames.push(entry);
      const result=await inspect(source,detection,options);
      await new Promise((resolve,reject)=>{
        const timer=setTimeout(()=>{options?.signal?.removeEventListener('abort',abort);resolve();},delayMs);
        const abort=()=>{clearTimeout(timer);reject(new DOMException('Cancelled','AbortError'));};
        if(options?.signal?.aborted)abort();else options?.signal?.addEventListener('abort',abort,{once:true});
      });
      entry.completedAt=performance.now();entry.valid=result.valid;return result;
    };
    // Stop at the correction boundary: this regression exercises capture
    // scheduling, not a second expensive physical reconstruction test.
    ScannerPro.Lightweight.processPageAsync=async(_blob,options)=>{
      probe.processed++;
      return new Promise((_resolve,reject)=>{
        const cancel=()=>reject(new DOMException('Cancelled','AbortError'));
        if(options.signal.aborted)cancel();else options.signal.addEventListener('abort',cancel,{once:true});
      });
    };
    void ScannerPro.LiveCamera.open();
  },delayMs);
  await expect.poll(()=>page.evaluate(()=>slowReadiness.processed),{timeout:15000}).toBeGreaterThan(0);
  const report=await page.evaluate(()=>{
    const {events,frames,ticks}=slowReadiness;
    clearInterval(slowReadiness.timer);ScannerPro.LiveCamera.close();
    return {events,frames,ticks};
  });
  const ready=report.events.find(event=>event.ready);
  expect(ready).toBeTruthy();
  expect(ready.samples).toBeGreaterThanOrEqual(3);
  expect(ready.elapsedMs).toBeGreaterThan(1000);
  expect(report.events.slice(0,3).map(event=>event.samples)).toEqual([1,2,3]);
  expect(report.frames.slice(0,3).every(frame=>frame.valid)).toBe(true);
  expect(new Set(report.frames.slice(0,3).map(frame=>frame.marker)).size).toBe(3);
  expect(report.frames[1].frozenAt-report.frames[0].frozenAt).toBeGreaterThan(500);
  expect(report.ticks).toBeGreaterThan(50);
});
