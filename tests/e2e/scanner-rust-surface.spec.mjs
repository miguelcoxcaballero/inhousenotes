import {expect,test} from '@playwright/test';

test('Rust 3D optimization preserves independently validated camera and sheet geometry',async({page})=>{
  test.setTimeout(90000);
  await page.goto('/scanner/index.html',{waitUntil:'domcontentloaded'});
  await page.addScriptTag({url:'/tests/e2e/fixtures/scanner-developable.js'});
  const reports=await page.evaluate(async()=>{
    await InhouseRust.ready;const core=InhouseRust,reports=[];
    try{for(const definition of [ScannerDevelopableFixtures.CASES[0],ScannerDevelopableFixtures.CASES[2]]){
      const fixture=ScannerDevelopableFixtures.render(definition),models=[];
      for(const rust of [false,true]){
        globalThis.InhouseRust=rust?core:{...core,surfaceFit:()=>null};
        const model=ScannerPro.MetricSurface.fit(fixture.source,{frame:ScannerDevelopableFixtures.exactFrame(fixture.map)},{budgetMs:7800});
        if(!model)throw new Error('No verified surface for '+definition.name);
        models.push(model);
      }
      let maximum=0;
      for(let r=0;r<20;r++)for(let c=0;c<15;c++){
        const u=(25+c*11)/210,v=(25+r*12)/297;
        const a=ScannerPro.MetricSurface.map(models[0],u,v),b=ScannerPro.MetricSurface.map(models[1],u,v);
        maximum=Math.max(maximum,Math.hypot(a.x-b.x,a.y-b.y));
      }
      reports.push({name:definition.name,maximum,backends:models.map(model=>model.diagnostics.backend),
        validation:models.map(model=>model.diagnostics.validation),milliseconds:models.map(model=>model.diagnostics.elapsedMs)});
    }}finally{globalThis.InhouseRust=core;}
    return reports;
  });
  console.log('RUST_SURFACE_PARITY',JSON.stringify(reports));
  for(const report of reports){
    expect(report.backends).toEqual(['javascript','rust']);
    expect(Math.min(...report.validation)).toBeGreaterThan(500);
    expect(report.maximum,report.name).toBeLessThan(.02);
  }
});
