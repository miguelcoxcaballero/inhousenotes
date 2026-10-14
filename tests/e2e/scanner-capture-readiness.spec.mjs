import {expect,test} from '@playwright/test';

test.beforeEach(async({page})=>{
  await page.goto('/scanner/index.html',{waitUntil:'domcontentloaded'});
  await page.addScriptTag({url:'/tests/e2e/fixtures/scanner-synthetic.js'});
  await page.addScriptTag({url:'/tests/e2e/fixtures/scanner-developable.js'});
});

test('capture preflight finds the photographed lattice on sharp perspective and curved pages',async({page})=>{
  const report=await page.evaluate(()=>{
    const sources=[...ScannerSyntheticFixtures.CASES.slice(0,8).map(definition=>({name:definition.name,source:ScannerSyntheticFixtures.renderCase(definition).source})),
      ...ScannerDevelopableFixtures.CASES.map(definition=>({name:definition.name,source:ScannerDevelopableFixtures.render(definition).source}))];
    return sources.map(({name,source})=>{
      const probe=document.createElement('canvas');probe.width=360;probe.height=Math.round(source.height*360/source.width);
      const context=probe.getContext('2d');context.drawImage(source,0,0,probe.width,probe.height);
      const detection=ScannerPro.LiveDetection.detect(context.getImageData(0,0,probe.width,probe.height));
      const started=performance.now(),readiness=ScannerPro.CaptureQuality.inspectReadiness(source,detection);
      return {name,...readiness,ms:performance.now()-started};
    });
  });
  console.log('CAPTURE_READINESS',JSON.stringify(report));
  for(const item of report){expect(item.valid,item.name).toBe(true);expect(item.grid.matched,item.name).toBeGreaterThan(180);expect(item.grid.coverage,item.name).toBeGreaterThanOrEqual(8/9);}
});

test('capture preflight rejects blur before border confidence can trigger capture',async({page})=>{
  const report=await page.evaluate(()=>{
    const source=ScannerSyntheticFixtures.renderCase(ScannerSyntheticFixtures.CASES[0]).source;
    const probe=document.createElement('canvas');probe.width=360;probe.height=Math.round(source.height*360/source.width);
    const context=probe.getContext('2d');context.drawImage(source,0,0,probe.width,probe.height);
    const detection=ScannerPro.LiveDetection.detect(context.getImageData(0,0,probe.width,probe.height));
    return [0,.5,.8,1,1.2,1.5,2,3,6].map(blur=>{
      const image=document.createElement('canvas');image.width=source.width;image.height=source.height;
      const ctx=image.getContext('2d');ctx.filter=blur?`blur(${blur}px)`:'none';ctx.drawImage(source,0,0);
      return {blur,...ScannerPro.CaptureQuality.inspectReadiness(image,detection)};
    });
  });
  console.log('READINESS_BLUR',JSON.stringify(report));
  expect(report[0].valid).toBe(true);
  for(const item of report.filter(item=>item.blur>=.8))expect(item.valid,`${item.blur}px blur`).toBe(false);
});

test('sharp borders and markers cannot substitute for a sufficiently covered two-axis dot lattice',async({page})=>{
  const report=await page.evaluate(()=>{
    const width=720,height=1080;
    const quad=[{x:.08,y:.05},{x:.92,y:.05},{x:.92,y:.95},{x:.08,y:.95}];
    const detection={quad,frameEdges:quad.map((a,index)=>Array.from({length:33},(_,step)=>{
      const b=quad[(index+1)%4];return{x:a.x+(b.x-a.x)*step/32,y:a.y+(b.y-a.y)*step/32};}))};
    let randomState=12345;const random=()=>((randomState=Math.imul(randomState,1664525)+1013904223)>>>0)/4294967296;
    return ['grid','blank','handwriting','ruled','random-specks','single-axis','partial-grid','glare'].map(kind=>{
      const image=document.createElement('canvas');image.width=width;image.height=height;
      const ctx=image.getContext('2d');ctx.fillStyle='white';ctx.fillRect(0,0,width,height);
      ctx.strokeStyle='#f0db4c';ctx.lineWidth=3;ctx.strokeRect(width*.08,height*.05,width*.84,height*.9);
      const x0=width*.08,y0=height*.05,sx=width*.84/36,sy=height*.9/54;
      ctx.fillStyle='#777';
      if(['grid','partial-grid','glare','single-axis'].includes(kind))for(let row=1;row<=51;row++)for(let col=1;col<=35;col++) {
        if(kind==='partial-grid'&&(row>24||col>17)||kind==='single-axis'&&row%4!==0)continue;
        ctx.beginPath();ctx.arc(x0+col*sx,y0+row*sy,1.1,0,Math.PI*2);ctx.fill();
      }
      if(kind==='random-specks')for(let index=0;index<1800;index++){
        ctx.beginPath();ctx.arc(x0+random()*width*.84,y0+random()*height*.9,1.1,0,Math.PI*2);ctx.fill();
      }
      if(kind==='handwriting') {
        ctx.font='16px cursive';ctx.fillStyle='#444';
        for(let row=1;row<28;row++)ctx.fillText('Handwritten notes, liliiiii 0123456',x0+20,y0+row*32);
      }
      if(kind==='ruled') {
        ctx.strokeStyle='#777';ctx.lineWidth=1.3;ctx.beginPath();
        for(let row=1;row<51;row++){ctx.moveTo(x0+10,y0+row*sy);ctx.lineTo(width*.92-10,y0+row*sy);}
        for(let col=1;col<35;col++){ctx.moveTo(x0+col*sx,y0+10);ctx.lineTo(x0+col*sx,height*.95-10);}
        ctx.stroke();
      }
      if(kind==='glare'){ctx.fillStyle='#fff';ctx.fillRect(width*.35,height*.2,width*.3,height*.6);}
      ctx.strokeStyle='#f0db4c';for(const left of [1,19])ctx.strokeRect(x0+left*sx,y0+52*sy,16*sx,sy);
      ['red','#222','blue','green'].forEach((color,index)=>{ctx.fillStyle=color;ctx.beginPath();ctx.arc(x0+(17.25+index*.5)*sx,y0+52.5*sy,3.6,0,Math.PI*2);ctx.fill();});
      return {kind,...ScannerPro.CaptureQuality.inspectReadiness(image,detection)};
    });
  });
  console.log('READINESS_NEGATIVES',JSON.stringify(report));
  expect(report[0].valid).toBe(true);
  for(const item of report.slice(1)) {expect(item.valid,item.kind).toBe(false);expect(item.grid?.valid,item.kind).not.toBe(true);}
});

test('readiness worker matches fallback evidence and leaves animation frames responsive',async({page})=>{
  const workers=[];page.on('worker',worker=>workers.push(worker.url()));
  const report=await page.evaluate(async()=>{
    const source=ScannerSyntheticFixtures.renderCase(ScannerSyntheticFixtures.CASES[0]).source;
    const probe=document.createElement('canvas');probe.width=360;probe.height=Math.round(source.height*360/source.width);
    const ctx=probe.getContext('2d');ctx.drawImage(source,0,0,probe.width,probe.height);
    const detection=ScannerPro.LiveDetection.detect(ctx.getImageData(0,0,probe.width,probe.height));
    const expected=ScannerPro.CaptureQuality.inspectReadiness(source,detection);
    let ticks=0;const timer=setInterval(()=>ticks++,8);
    const actual=[];for(let count=0;count<4;count++)actual.push(await ScannerPro.CaptureQuality.inspectReadinessAsync(source,detection));
    clearInterval(timer);ScannerPro.CaptureQuality.disposeReadiness();
    return {expected,actual,ticks};
  });
  expect(report.actual).toEqual(Array(4).fill(report.expected));
  expect(report.ticks).toBeGreaterThan(4);
  expect(workers.filter(url=>url.includes('capture-quality-worker.js'))).toHaveLength(1);
});

test('readiness cancellation discards old work and duplicate frames cannot queue indefinitely',async({page})=>{
  const report=await page.evaluate(async()=>{
    const source=ScannerSyntheticFixtures.renderCase(ScannerSyntheticFixtures.CASES[0]).source;
    const probe=document.createElement('canvas');probe.width=360;probe.height=Math.round(source.height*360/source.width);
    const ctx=probe.getContext('2d');ctx.drawImage(source,0,0,probe.width,probe.height);
    const detection=ScannerPro.LiveDetection.detect(ctx.getImageData(0,0,probe.width,probe.height));
    const controller=new AbortController();
    const first=ScannerPro.CaptureQuality.inspectReadinessAsync(source,detection,{signal:controller.signal}).then(()=>null,error=>error.name);
    const duplicate=await ScannerPro.CaptureQuality.inspectReadinessAsync(source,detection);
    controller.abort();const cancelled=await first;
    const next=await ScannerPro.CaptureQuality.inspectReadinessAsync(source,detection);
    const pending=ScannerPro.CaptureQuality.inspectReadinessAsync(source,detection).then(()=>null,error=>error.name);
    ScannerPro.CaptureQuality.disposeReadiness();const disposed=await pending;
    const fresh=await ScannerPro.CaptureQuality.inspectReadinessAsync(source,detection);
    ScannerPro.CaptureQuality.disposeReadiness();
    return {duplicate,cancelled,next,disposed,fresh};
  });
  expect(report.duplicate.valid).toBe(false);
  expect(report.duplicate.reason).toBe('readiness-busy');
  expect(report.cancelled).toBe('AbortError');
  expect(report.disposed).toBe('AbortError');
  expect(report.next.valid).toBe(true);
  expect(report.fresh.valid).toBe(true);
});

test('worker-unavailable fallback retains the same sharpness and lattice requirements',async({page})=>{
  const report=await page.evaluate(async()=>{
    const source=ScannerSyntheticFixtures.renderCase(ScannerSyntheticFixtures.CASES[0]).source;
    const probe=document.createElement('canvas');probe.width=360;probe.height=Math.round(source.height*360/source.width);
    const ctx=probe.getContext('2d');ctx.drawImage(source,0,0,probe.width,probe.height);
    const detection=ScannerPro.LiveDetection.detect(ctx.getImageData(0,0,probe.width,probe.height));
    const expected=ScannerPro.CaptureQuality.inspectReadiness(source,detection);
    const original=window.Worker;window.Worker=undefined;
    try{return {expected,actual:await ScannerPro.CaptureQuality.inspectReadinessAsync(source,detection)};}
    finally {window.Worker=original;}
  });
  expect(report.actual).toEqual(report.expected);
  expect(report.actual.valid).toBe(true);
});

test('camera noise and a crisp border cannot conceal a smeared or washed out interior',async({page})=>{
  const report=await page.evaluate(()=>{
    const source=ScannerSyntheticFixtures.renderCase(ScannerSyntheticFixtures.CASES[0]).source;
    const probe=document.createElement('canvas');probe.width=360;probe.height=Math.round(source.height*360/source.width);
    const probeContext=probe.getContext('2d');probeContext.drawImage(source,0,0,probe.width,probe.height);
    const detection=ScannerPro.LiveDetection.detect(probeContext.getImageData(0,0,probe.width,probe.height));
    return [{blur:2,noise:6},{blur:3,noise:12},{blur:5,noise:20},{blur:0,noise:0,washed:true}].map(setting=>{
      const image=document.createElement('canvas');image.width=source.width;image.height=source.height;
      const ctx=image.getContext('2d');ctx.drawImage(source,0,0);
      ctx.save();ctx.beginPath();ctx.rect(175,145,540,850);ctx.clip();
      ctx.filter=`blur(${setting.blur}px)`;ctx.drawImage(source,0,0);ctx.restore();
      const pixels=ctx.getImageData(0,0,image.width,image.height);let state=321;
      for(let y=145;y<995;y++)for(let x=175;x<715;x++){
        const index=(y*image.width+x)*4;state=(Math.imul(state,1664525)+1013904223)>>>0;
        const noise=(state/4294967296-.5)*2*setting.noise;
        for(let c=0;c<3;c++)pixels.data[index+c]=setting.washed?255-(255-pixels.data[index+c])*.05:pixels.data[index+c]+noise;
      }
      ctx.putImageData(pixels,0,0);
      return {setting,...ScannerPro.CaptureQuality.inspectReadiness(image,detection)};
    });
  });
  console.log('READINESS_NOISE',JSON.stringify(report));
  for(const item of report)expect(item.valid,JSON.stringify(item.setting)).toBe(false);
});

test('readiness uses locally visible dots under colour casts and rejects clipped-away dots',async({page})=>{
  const report=await page.evaluate(()=>{
    const source=ScannerSyntheticFixtures.renderCase(ScannerSyntheticFixtures.CASES[0]).source;
    const probe=document.createElement('canvas');probe.width=360;probe.height=Math.round(source.height*360/source.width);
    const pctx=probe.getContext('2d');pctx.drawImage(source,0,0,probe.width,probe.height);
    const detection=ScannerPro.LiveDetection.detect(pctx.getImageData(0,0,probe.width,probe.height));
    const settings=[{name:'dim-warm',gain:[.58,.44,.32],offset:0},{name:'bright-cool',gain:[.76,.82,.88],offset:30},
      {name:'magenta',gain:[.88,.62,.9],offset:0},{name:'green',gain:[.65,.92,.7],offset:0},{name:'clipped',gain:[1.5,1.5,1.5],offset:0}];
    return settings.map(setting=>{
      const image=document.createElement('canvas');image.width=source.width;image.height=source.height;
      const ctx=image.getContext('2d');ctx.drawImage(source,0,0);const pixels=ctx.getImageData(0,0,image.width,image.height);
      for(let index=0;index<pixels.data.length;index+=4)for(let c=0;c<3;c++)pixels.data[index+c]=pixels.data[index+c]*setting.gain[c]+setting.offset;
      ctx.putImageData(pixels,0,0);
      return {name:setting.name,...ScannerPro.CaptureQuality.inspectReadiness(image,detection)};
    });
  });
  console.log('READINESS_EXPOSURE',JSON.stringify(report));
  for(const item of report.slice(0,4))expect(item.valid,item.name).toBe(true);
  expect(report.at(-1).valid).toBe(false);
});

test('sharp page edges and outer regions cannot mask a defocused centre',async({page})=>{
  const report=await page.evaluate(()=>{
    const source=ScannerSyntheticFixtures.renderCase(ScannerSyntheticFixtures.CASES[0]).source;
    const probe=document.createElement('canvas');probe.width=360;probe.height=Math.round(source.height*360/source.width);
    const pctx=probe.getContext('2d');pctx.drawImage(source,0,0,probe.width,probe.height);
    const detection=ScannerPro.LiveDetection.detect(pctx.getImageData(0,0,probe.width,probe.height));
    return [0,1.5,2,3].map(blur=>{
      const image=document.createElement('canvas');image.width=source.width;image.height=source.height;
      const ctx=image.getContext('2d');ctx.drawImage(source,0,0);
      ctx.save();ctx.beginPath();ctx.rect(300,400,280,370);ctx.clip();ctx.filter=`blur(${blur}px)`;ctx.drawImage(source,0,0);ctx.restore();
      return {blur,...ScannerPro.CaptureQuality.inspectReadiness(image,detection)};
    });
  });
  expect(report[0].valid).toBe(true);
  for(const item of report.slice(1)) {
    expect(item.clarity).toBeGreaterThan(.6);
    expect(item.interiorMinimum).toBeLessThan(.32);
    expect(item.reason).toBe('focus-or-motion-blur');
    expect(item.valid,`local ${item.blur}px blur`).toBe(false);
  }
});
