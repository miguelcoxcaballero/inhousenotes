import {expect,test} from '@playwright/test';

test('the frozen photo focus gate rejects smeared text even with confident yellow detection',async({page})=>{
  await page.goto('/scanner/index.html',{waitUntil:'domcontentloaded'});
  await page.addScriptTag({url:'/tests/e2e/fixtures/scanner-synthetic.js'});
  const report=await page.evaluate(()=>{
    const source=ScannerSyntheticFixtures.renderCase(ScannerSyntheticFixtures.CASES[0]).source;
    const probe=document.createElement('canvas');probe.width=360;probe.height=Math.round(source.height*360/source.width);
    probe.getContext('2d').drawImage(source,0,0,probe.width,probe.height);
    const detection=ScannerPro.LiveDetection.detect(probe.getContext('2d').getImageData(0,0,probe.width,probe.height));
    return [0,3,6].map(blur=>{
      const image=document.createElement('canvas');image.width=source.width;image.height=source.height;
      const ctx=image.getContext('2d');ctx.filter=blur?`blur(${blur}px)`:'none';ctx.drawImage(source,0,0);
      return {blur,confidence:detection.confidence,...ScannerPro.CaptureQuality.inspect(image,detection)};
    });
  });
  console.log('PHOTO_FOCUS',JSON.stringify(report));
  expect(report[0].valid).toBe(true);
  expect(report[1].valid).toBe(false);
  expect(report[2].valid).toBe(false);
  expect(report[2].confidence).toBeGreaterThan(.78);
});
