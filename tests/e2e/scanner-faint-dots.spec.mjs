import {expect,test} from '@playwright/test';

test('faint photographed dots retain their measured lattice and independently verified 3D bend',async({page})=>{
  test.setTimeout(60000);
  await page.goto('/scanner/index.html',{waitUntil:'domcontentloaded'});
  await page.addScriptTag({url:'/tests/e2e/fixtures/scanner-developable.js'});
  const report=await page.evaluate(()=>{
    const fixture=ScannerDevelopableFixtures;
    const rendered=fixture.render({...fixture.CASES[0],dotColor:'#f8f7f4'});
    const diagnostics={};
    const surface=ScannerPro.MetricSurface.fit(rendered.source,{frame:fixture.exactFrame(rendered.map)},{diagnostics});
    const errors=[];
    if(surface)for(let row=0;row<20;row++)for(let col=0;col<15;col++){
      const u=(25+col*11)/210,v=(25+row*12)/297;
      const measured=ScannerPro.MetricSurface.map(surface,u,v),actual=rendered.map(u,v);
      errors.push(Math.hypot(measured.x-actual.x,measured.y-actual.y));
    }
    return {accepted:!!surface,diagnostics,maximum:Math.max(...errors),validation:surface?.diagnostics.validation};
  });
  expect(report.accepted,JSON.stringify(report.diagnostics)).toBe(true);
  expect(report.diagnostics.faintDotPass).toBe(true);
  expect(report.validation).toBeGreaterThan(500);
  expect(report.maximum).toBeLessThan(2.5);
});
