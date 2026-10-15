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


test('still-photo frame verification survives a lamp change after the live detection', async ({page}) => {
  test.setTimeout(90000);
  await page.goto('/scanner/index.html');
  await page.addScriptTag({url:'/tests/e2e/fixtures/scanner-developable.js'});
  const rows = await page.evaluate(async () => {
    const f=ScannerDevelopableFixtures, rows=[];
    const previewOf=source=>{
      const p=document.createElement('canvas'),scale=480/Math.max(source.width,source.height);
      p.width=Math.round(source.width*scale);p.height=Math.round(source.height*scale);
      const c=p.getContext('2d');c.drawImage(source,0,0,p.width,p.height);
      const pixels=c.getImageData(0,0,p.width,p.height);
      return {...ScannerPro.LiveDetection.detect(pixels),hasColourReferences:!!ScannerPro.LiveDetection.findColourReferences(pixels)};
    };
    for (const mode of ['faded-after-capture','mixed-cool','mixed-warm','missing-frame']) {
      const definition={...f.CASES[2],dotColor:'#a8a8a8'};
      const live=previewOf(f.render(definition).source);
      const {source}=f.render({...definition,...(mode==='faded-after-capture'?{stencilColor:'#d2d5d8'}:mode==='missing-frame'?{stencilColor:'#fff'}:{})});
      if (mode.startsWith('mixed-')) {
        const ctx=source.getContext('2d'),im=ctx.getImageData(0,0,source.width,source.height);
        const gains=mode==='mixed-cool'?[.48,.7,1]:[1,.65,.4];
        for(let y=0;y<source.height;y++)for(let x=0;x<source.width;x++)for(let c=0;c<3;c++){
          const mix=Math.exp(-(((y/source.height-.43)/.28)**4)),i=(y*source.width+x)*4+c;
          im.data[i]*=1+(gains[c]-1)*mix;
        }
        ctx.putImageData(im,0,0);
      }
      const detection=ScannerPro.Lightweight.detectPage(source,{liveDetection:live});
      const changedLive=previewOf(source);
      let scan;
      if(mode==='mixed-warm') {
        const blob=await new Promise(resolve=>source.toBlob(resolve,'image/png'));
        const result=await ScannerPro.Lightweight.processPageAsync(blob,{liveDetection:changedLive,requireGrid:true,maxDimension:1200});
        scan={corrected:result.corrected,alignment:result.dotAlignment?.status,reason:result.failureReason};
      }
      rows.push({mode,scan,changedLive:changedLive?.confidence,changedMode:changedLive?.detectionMode,changedRefs:changedLive?.hasColourReferences,seed:live?.detectionMode,quality:detection.quality,method:detection.method,confidence:detection.confidence});
    }
    return rows;
  });
  console.log('STILL_LIGHT_CHANGE',JSON.stringify(rows));
  for(const row of rows) {
    expect(row.quality?.valid,JSON.stringify(row)).toBe(row.mode!=='missing-frame');
    if(row.mode!=='missing-frame')expect(row.changedLive,JSON.stringify(row)).toBeGreaterThan(.86);
  }
  const warm=rows.find(r=>r.mode==='mixed-warm');
  expect(warm.scan.corrected,JSON.stringify(warm)).toBe(true);
  expect(warm.scan.alignment).toBe('verified');
});
