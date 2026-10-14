/* A single bounded camera preflight runs away from rendering and pen input. */
'use strict';
self.window=self;
self.document={createElement(name){
  if(name!=='canvas')throw new Error('Unsupported capture verification element');
  return new OffscreenCanvas(1,1);
}};
importScripts(`capture-quality.js${self.location.search}`);
self.onmessage=event=>{
  const {id,bitmap,detection}=event.data;
  try {
    self.postMessage({id,result:self.ScannerPro.CaptureQuality.inspectReadiness(bitmap,detection)});
  } catch(error) {
    self.postMessage({id,error:error?.message||'Capture verification failed'});
  } finally {bitmap?.close();}
};
