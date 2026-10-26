/* Measure the frozen photograph, independently of reduced live-view confidence.
 * Edge acutance distinguishes a narrow ink edge from a broad motion/focus smear;
 * raw Laplacian variance alone can be inflated by contrast and camera noise.
 */
(() => {
  const SP = window.ScannerPro = window.ScannerPro || {};
  // The measurements run in the Rust core (rust-core/src/scan_color/quality.rs); this file only reads
  // pixels from a canvas and manages the worker. A missing core is an error, never a silent fallback.
  const rust=()=>{
    const core=window.InhouseRust;
    if(!core?.available?.()||!core.captureQuality) throw new Error('Rust core is not ready: capture verification needs InhouseRust');
    return core.captureQuality;
  };
  function readSource(source) {
    const scale=Math.min(1,1100/Math.max(source.width,source.height));
    const canvas=document.createElement('canvas');
    canvas.width=Math.max(1,Math.round(source.width*scale));
    canvas.height=Math.max(1,Math.round(source.height*scale));
    try {
      const context=canvas.getContext('2d',{willReadFrequently:true});
      context.drawImage(source,0,0,canvas.width,canvas.height);
      return {width:canvas.width,height:canvas.height,rgba:context.getImageData(0,0,canvas.width,canvas.height).data};
    } finally {canvas.width=canvas.height=0;}
  }
  function inspect(source, detection, raster) {
    if (!detection?.quad || !source.width || !source.height) return {valid:false,reason:'missing-page'};
    return rust().inspect(raster||readSource(source),detection);
  }
  // A yellow rectangle (or sharp handwriting) is not evidence that the photographed bullet lattice can
  // be recovered; the bounded preflight samples the actual frozen image.
  function inspectGrid(source,detection,raster) {
    return rust().inspectGrid(raster||(()=>readSource(source)),detection);
  }
  function inspectReadiness(source,detection) {
    if(!source?.width||!source?.height||!detection?.quad) return {valid:false,reason:'missing-page',grid:null};
    return rust().inspectReadiness(readSource(source),detection);
  }
  const moduleUrl=typeof document.currentScript?.src==='string'?document.currentScript.src:null;
  let readinessWorker=null,workerUnavailable=false,workerGeneration=0,nextRequest=0,pending=null,creating=false;
  const abortError=()=>new DOMException('Capture verification cancelled','AbortError');
  function disposeReadiness() {
    workerGeneration++;
    const previous=pending;pending=null;
    previous?.finish(null,abortError());
    readinessWorker?.terminate();readinessWorker=null;
  }
  async function inspectReadinessAsync(source,detection,{signal}={}) {
    if(signal?.aborted) throw abortError();
    if(!window.InhouseRust?.available?.()) {
      await window.InhouseRust?.ready;
      if(signal?.aborted) throw abortError();
    }
    if(!moduleUrl||workerUnavailable||typeof Worker!=='function'||typeof createImageBitmap!=='function'
      ||typeof OffscreenCanvas!=='function') return inspectReadiness(source,detection);
    // Only one candidate is inspected at a time. Dropping duplicate work is
    // preferable to building an unbounded queue of stale camera frames.
    if(pending||creating) return {valid:false,reason:'readiness-busy',grid:null};
    const generation=workerGeneration;
    let bitmap;
    creating=true;
    try {bitmap=await createImageBitmap(source);}
    catch(error) {
      if(signal?.aborted||generation!==workerGeneration) throw abortError();
      workerUnavailable=true;return inspectReadiness(source,detection);
    } finally {creating=false;}
    if(signal?.aborted||generation!==workerGeneration) {bitmap.close();throw abortError();}
    try {
      if(!readinessWorker) {
        const url=new URL('capture-quality-worker.js',moduleUrl);url.search=new URL(moduleUrl).search;
        readinessWorker=new Worker(url);
        readinessWorker.onmessage=event=>{
          const current=pending;
          if(!current||event.data.id!==current.id)return;
          current.finish(event.data.result,event.data.error?new Error(event.data.error):null);
        };
        readinessWorker.onerror=()=>pending?.finish(null,new Error('Capture verification worker unavailable'));
      }
      return await new Promise((resolve,reject)=>{
        const id=++nextRequest;
        let settled=false,timer;
        const cancel=()=>{
          readinessWorker?.terminate();readinessWorker=null;
          finish(null,abortError());
        };
        const finish=(result,error)=>{
          if(settled)return;settled=true;
          clearTimeout(timer);signal?.removeEventListener('abort',cancel);
          if(pending?.id===id)pending=null;
          if(error)reject(error);else resolve(result);
        };
        pending={id,finish};
        signal?.addEventListener('abort',cancel,{once:true});
        timer=setTimeout(()=>{
          readinessWorker?.terminate();readinessWorker=null;
          finish(null,new Error('Capture verification worker timed out'));
        },2000);
        try {readinessWorker.postMessage({id,bitmap,detection},[bitmap]);bitmap=null;}
        catch(error){finish(null,error);}
      });
    } catch(error) {
      if(error?.name==='AbortError'||signal?.aborted||generation!==workerGeneration)throw abortError();
      readinessWorker?.terminate();readinessWorker=null;workerUnavailable=true;
      return inspectReadiness(source,detection);
    } finally {bitmap?.close();}
  }
  SP.CaptureQuality = {inspect,inspectGrid,inspectReadiness,inspectReadinessAsync,disposeReadiness};
})();
