import {test,expect} from '@playwright/test';

test('observed footer and frame layout recover non-yellow printing on curved sheets',async({page})=>{
  await page.goto('/scanner/index.html?embed=1&session=frame-layout');
  await page.addScriptTag({url:'/tests/e2e/fixtures/scanner-developable.js'});
  const results=await page.evaluate(()=>{
    const f=ScannerDevelopableFixtures;
    return f.CASES.map(def=>{
      const {source,map}=f.render({...def,stencilColor:'#d2d5d8',dotColor:'#a8a8a8'});
      const preview=document.createElement('canvas'),scale=480/Math.max(source.width,source.height);
      preview.width=Math.round(source.width*scale);preview.height=Math.round(source.height*scale);
      const ctx=preview.getContext('2d');ctx.drawImage(source,0,0,preview.width,preview.height);
      const start=performance.now(),detection=ScannerPro.LiveDetection.detect(ctx.getImageData(0,0,preview.width,preview.height));
      const initialMs=performance.now()-start;
      const nextStarted=performance.now();
      const tracked=ScannerPro.LiveDetection.detect(ctx.getImageData(0,0,preview.width,preview.height),detection);
      const trackedMs=performance.now()-nextStarted;
      const reference=Object.values(f.exactFrame(map).paths).flat().map(p=>({x:p.x/source.width,y:p.y/source.height}));
      const errors=(detection?.contour||[]).map(p=>Math.min(...reference.map(q=>Math.hypot(p.x-q.x,p.y-q.y))));
      return {name:def.name,ms:initialMs,trackedMs,trackedMode:tracked?.detectionMode,trackedConfidence:tracked?.confidence,mode:detection?.detectionMode,confidence:detection?.confidence,
        markers:detection?.markers.length,markerSupport:detection?.markerSupport,maxError:Math.max(...errors)};
    });
  });
  console.log('LIVE_LAYOUT',JSON.stringify(results));
  for(const r of results){expect(r.trackedMode,r.name).toBe('layout');expect(r.trackedConfidence,r.name).toBeGreaterThan(.86);expect(r.mode,r.name).toBe('layout');expect(r.confidence,r.name).toBeGreaterThan(.86);
    expect(r.markers,r.name).toBe(2);expect(r.markerSupport,r.name).toBeGreaterThanOrEqual(.65);expect(r.maxError,r.name).toBeLessThan(.018);}
});

test('a grey-frame camera scan retains full photo and output dot-grid verification',async({page})=>{
  test.setTimeout(60000);
  await page.goto('/scanner/index.html?embed=1&session=frame-layout-process');
  await page.addScriptTag({url:'/tests/e2e/fixtures/scanner-developable.js'});
  const result=await page.evaluate(async()=>{
    const f=ScannerDevelopableFixtures,{source}=f.render({...f.CASES[2],stencilColor:'#d2d5d8',dotColor:'#a8a8a8'});
    const preview=document.createElement('canvas'),scale=480/Math.max(source.width,source.height);
    preview.width=Math.round(source.width*scale);preview.height=Math.round(source.height*scale);
    const c=preview.getContext('2d');c.drawImage(source,0,0,preview.width,preview.height);
    const live=ScannerPro.LiveDetection.detect(c.getImageData(0,0,preview.width,preview.height));
    const blob=await new Promise(resolve=>source.toBlob(resolve,'image/png'));
    const scan=await ScannerPro.Lightweight.processPageAsync(blob,{liveDetection:live,requireGrid:true,maxDimension:1200});
    return {mode:live?.detectionMode,corrected:scan.corrected,reason:scan.failureReason,quality:scan.detection.quality,
      surfaceDiagnostics:scan.detection.frame?.surfaceDiagnostics,metric:scan.detection.frame?.metricSurface?.type,alignment:scan.dotAlignment?.status,strokes:scan.editableInk?.strokes.length};
  });
  console.log('LAYOUT_CAMERA',JSON.stringify(result));
  expect(result.mode).toBe('layout');expect(result.corrected,JSON.stringify(result)).toBe(true);
  expect(result.metric).toBe('developable-pinhole');expect(result.alignment).toBe('verified');expect(result.strokes).toBeGreaterThan(0);
});
