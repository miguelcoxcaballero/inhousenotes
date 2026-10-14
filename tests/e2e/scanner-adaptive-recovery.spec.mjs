import {expect,test} from '@playwright/test';

test('a failed final raster check retries detailed observation before rejecting the photo',async({page})=>{
  test.setTimeout(60000);
  await page.goto('/scanner/index.html',{waitUntil:'domcontentloaded'});
  await page.addScriptTag({url:'/tests/e2e/fixtures/scanner-developable.js'});
  const report=await page.evaluate(async()=>{
    const source=ScannerDevelopableFixtures.render(ScannerDevelopableFixtures.CASES[0]).source;
    const blob=await new Promise(resolve=>source.toBlob(resolve,'image/jpeg',.94));
    const certify=ScannerPro.MetricSurface.certifyRaster,worker=globalThis.Worker;
    let calls=0;
    globalThis.Worker=undefined;
    ScannerPro.MetricSurface.certifyRaster=(...args)=>++calls===1
      ? {status:'failed',reason:'photographed-dots-misaligned'} : certify(...args);
    try{
      const scan=await ScannerPro.Lightweight.processPageAsync(blob);
      return{calls,corrected:scan.corrected,quality:scan.detection.quality,
        scale:scan.detection.scaleSelection,recoveryMs:scan.timings.recoveryMs};
    }finally{globalThis.Worker=worker;ScannerPro.MetricSurface.certifyRaster=certify;}
  });
  expect(report.calls).toBe(2);
  expect(report.corrected,JSON.stringify(report)).toBe(true);
  expect(report.quality.valid).toBe(true);
  expect(report.scale.reason).toBe('raster-recovery');
  expect(report.recoveryMs).toBeGreaterThan(0);
});
