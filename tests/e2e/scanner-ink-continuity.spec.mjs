import {expect,test} from '@playwright/test';

for(const scale of [1,2])test(`fine handwriting stays continuous beside darker ink at ${scale}x`,async({page})=>{
  await page.goto('/scanner/index.html',{waitUntil:'domcontentloaded'});
  const report=await page.evaluate(async scale=>{
    const width=794*scale,height=1123*scale,c=document.createElement('canvas');c.width=width;c.height=height;
    const ctx=c.getContext('2d');ctx.scale(scale,scale);ctx.lineCap=ctx.lineJoin='round';
    const samples=[];
    ctx.strokeStyle='rgba(0,0,0,.14)';ctx.lineWidth=1.4;ctx.beginPath();ctx.moveTo(110,180.5);ctx.lineTo(400,180.5);ctx.stroke();
    for(let x=120;x<=390;x+=.5)samples.push({x,y:180.5,line:true});
    ctx.strokeStyle='rgba(0,0,0,.8)';ctx.lineWidth=2;ctx.beginPath();ctx.moveTo(110,183.5);ctx.lineTo(400,183.5);ctx.stroke();
    const curve=[];
    for(let x=110;x<=410;x+=.25)curve.push({x,y:300+5*Math.sin((x-110)*Math.PI/12)});
    ctx.strokeStyle='rgba(0,0,0,.18)';ctx.lineWidth=1.4;ctx.beginPath();ctx.moveTo(curve[0].x,curve[0].y);
    for(const p of curve.slice(1))ctx.lineTo(p.x,p.y);ctx.stroke();samples.push(...curve);
    ctx.strokeStyle='rgba(0,0,0,.85)';ctx.lineWidth=2;
    for(let x=122;x<405;x+=24){ctx.beginPath();ctx.moveTo(x,294);ctx.lineTo(x,312);ctx.stroke();}
    const photo=ctx.getImageData(0,0,width,height),coverage=new Uint8Array(width*height);
    for(let i=0;i<coverage.length;i++){
      const a=photo.data[i*4+3];coverage[i]=a;
      photo.data.set([0,49,225,Math.min(255,a*2)],i*4);
    }
    const rows=[];
    for(const forceJavaScript of [false,true]){
      const start=performance.now();const ink=await ScannerPro.InkVectorizer.fromPixels(photo,{coverage,forceJavaScript});
      ctx.setTransform(1,0,0,1,0,0);ctx.clearRect(0,0,width,height);ScannerPro.InkVectorizer.render(ctx,ink);
      const result=ctx.getImageData(0,0,width,height).data;
      const covered=p=>{
        for(let dy=-.45;dy<=.45;dy+=.45)for(let dx=-.45;dx<=.45;dx+=.45){
          const i=(Math.floor((p.y+dy)*scale)*width+Math.floor((p.x+dx)*scale))*4;
          if(result[i+3]>100)return true;
        }return false;
      };
      const line=samples.filter(p=>p.line),wave=samples.filter(p=>!p.line);
      let gap=0,longest=0;const gaps=[];
      for(const p of wave){if(covered(p)){if(gap)gaps.push({end:p.x,length:gap});gap=0;}else gap+=.25;longest=Math.max(longest,gap);}
      let gapPixels=0;
      for(let x=120;x<=390;x++){
        let minimum=255;
        for(let y=Math.ceil(181.25*scale);y<=Math.floor(182.5*scale);y++)minimum=Math.min(minimum,result[(y*width+Math.floor(x*scale))*4+3]);
        if(minimum>100)gapPixels++;
      }
      rows.push({backend:ink.backend,ms:performance.now()-start,strokes:ink.strokes.length,
        lineRecall:line.filter(covered).length/line.length,waveRecall:wave.filter(covered).length/wave.length,
        gaps,longestGap:longest,filledGap:gapPixels/271,shape:JSON.stringify(ink.strokes)});
    }
    const parity=rows[0].shape===rows[1].shape;for(const row of rows)delete row.shape;
    return{rows,parity};
  },scale);
  console.log('INK_CONTINUITY',JSON.stringify(report));
  expect(report.parity).toBe(true);
  for(const row of report.rows){
    expect(row.lineRecall).toBeGreaterThan(.98);
    expect(row.waveRecall).toBeGreaterThan(.98);
    expect(row.longestGap).toBeLessThanOrEqual(1);
    expect(row.filledGap).toBeLessThan(.02);
  }
});
