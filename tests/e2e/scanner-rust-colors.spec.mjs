import { expect, test } from '@playwright/test';

test('Rust colour processing agrees with the JavaScript fallback under warm light', async ({ page }) => {
  await page.goto('/scanner/index.html', {waitUntil:'domcontentloaded'});
  await page.addScriptTag({url:'/tests/e2e/fixtures/scanner-paper-contrast.js'});
  const report=await page.evaluate(async()=>{
    const results=[];
    for(const disableRust of [true,false]) {
      const fixture=await ScannerPaperContrastFixtures.render({gains:[1,.84,.66],jpeg:true});
      const started=performance.now();
      const processing=await ScannerPro.Lightweight.correctColors(fixture.source,{useStencil:true,preciseStencil:true,dotObservations:fixture.dotObservations,disableRust});
      results.push({processing,elapsed:performance.now()-started,measure:ScannerPaperContrastFixtures.measure(fixture.source,fixture),data:fixture.source.getContext('2d').getImageData(0,0,fixture.source.width,fixture.source.height).data});
    }
    let errors=0,max=0;for(let i=0;i<results[0].data.length;i++){const d=Math.abs(results[0].data[i]-results[1].data[i]);if(d>1)errors++;max=Math.max(max,d);}
    return {errors,max,runs:results.map(r=>({elapsed:r.elapsed,processing:r.processing.pixelProcessing,black:r.measure.writing.blackPen.median}))};
  });
  console.log('RUST_COLOR_PARITY',JSON.stringify(report));
  expect(report.errors).toBe(0);
  expect(report.runs[0].processing.backend).toBe('javascript');
  expect(report.runs[1].processing.backend).toBe('rust');
});
