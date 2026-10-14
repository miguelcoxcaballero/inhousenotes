import {expect,test} from '@playwright/test';

test('camera JPEG colour bleed is not amplified into a halo around pen strokes',async({page})=>{
  await page.goto('/scanner/index.html',{waitUntil:'domcontentloaded'});
  const report=await page.evaluate(async()=>{
    const source=document.createElement('canvas');source.width=840;source.height=1188;
    const ctx=source.getContext('2d',{willReadFrequently:true});
    ScannerPro.ScanTemplate.render(ctx,840,1188,{background:true});
    const mask=document.createElement('canvas');mask.width=840;mask.height=1188;
    const mc=mask.getContext('2d',{willReadFrequently:true});
    for(const [color,y] of [['#d52b36',290],['#254acb',440]]){
      for(const context of [ctx,mc]){
        context.strokeStyle=context===ctx?color:'#000';context.lineWidth=3;
        context.beginPath();context.moveTo(140,y);context.lineTo(650,y+27);context.stroke();
      }
    }
    const truth=mc.getImageData(0,0,840,1188).data;
    const image=ctx.getImageData(0,0,840,1188);
    for(let i=0;i<image.data.length;i+=4)for(let c=0;c<3;c++)image.data[i+c]*=[.88,.74,.61][c];
    ctx.putImageData(image,0,0);
    const blob=await new Promise(resolve=>source.toBlob(resolve,'image/jpeg',.68));
    const bmp=await createImageBitmap(blob);ctx.drawImage(bmp,0,0);bmp.close();
    const line=(x1,y1,x2,y2)=>Array.from({length:129},(_,i)=>({x:(x1+(x2-x1)*i/128)*40,y:(y1+(y2-y1)*i/128)*40}));
    const frame={confidence:1,paths:{top:line(1.5,1.43,19.5,1.43),right:line(19.5,1.43,19.5,28.43),
      bottom:line(1.5,28.43,19.5,28.43),left:line(1.5,1.43,1.5,28.43)}};
    const observed=ScannerPro.MetricSurface.observe(source,frame);
    await ScannerPro.Lightweight.correctColors(source,{useStencil:true,preciseStencil:true,dotObservations:observed?.points});
    const out=ctx.getImageData(0,0,840,1188).data;
    let count=0,halos=0;const cores=[];
    for(const start of [290,440])for(let x=150;x<640;x++)for(let y=Math.floor(start+(x-140)/510*27)-8;y<=Math.ceil(start+(x-140)/510*27)+8;y++){
      const i=(y*840+x)*4;
      if(truth[i+3]>=245){cores.push(Math.max(out[i],out[i+1],out[i+2])-Math.min(out[i],out[i+1],out[i+2]));continue;}
      if(truth[i+3])continue;
      let nearInk=false;for(let dy=-1;dy<=1;dy++)for(let dx=-1;dx<=1;dx++)if(truth[((y+dy)*840+x+dx)*4+3])nearInk=true;
      if(nearInk)continue;
      count++;const chroma=Math.max(out[i],out[i+1],out[i+2])-Math.min(out[i],out[i+1],out[i+2]);
      if(chroma>12&&Math.min(out[i],out[i+1],out[i+2])<240)halos++;
    }
    return{count,haloFraction:halos/count,coreChroma:cores.sort((a,b)=>a-b)[Math.floor(cores.length*.5)],observations:observed?.points.length};
  });
  console.log('INK_EDGES',JSON.stringify(report));
  expect(report.count).toBeGreaterThan(3000);
  expect(report.observations).toBeGreaterThan(1000);
  expect(report.haloFraction).toBeLessThan(.005);
  expect(report.coreChroma).toBeGreaterThan(80);
});
