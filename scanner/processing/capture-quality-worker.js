/* A single bounded camera preflight runs away from rendering and pen input. */
'use strict';
self.window=self;
self.document={createElement(name){
  if(name!=='canvas')throw new Error('Unsupported capture verification element');
  return new OffscreenCanvas(1,1);
}};
importScripts(`../../rust-core-v5.js${self.location.search}`,`capture-quality.js${self.location.search}`);
self.onmessage=async event=>{
  const {id,bitmap,detection}=event.data;
  try {
    if(!await self.InhouseRust.ready) throw new Error('Rust core is unavailable for capture verification');
    self.postMessage({id,result:self.ScannerPro.CaptureQuality.inspectReadiness(bitmap,detection)});
  } catch(error) {
    self.postMessage({id,error:error?.message||'Capture verification failed'});
  } finally {bitmap?.close();}
};
