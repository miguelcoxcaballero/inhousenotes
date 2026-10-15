import {expect,test} from '@playwright/test';

// Tight cursive turns at different angles, with darker strokes crossing them.
// A four-direction contrast test removed portions of the faint continuous pen.
test('oblique cursive curves survive darker crossings without changing Rust/JS geometry',async({page})=>{
  await page.goto('/scanner/index.html',{waitUntil:'domcontentloaded'});
  const results=[];
  for(const forceJavaScript of [false,true])results.push(await page.evaluate(async forceJavaScript=>{

  const scale=2,w=794*scale,h=1123*scale,c=document.createElement('canvas');c.width=w;c.height=h;const ctx=c.getContext('2d');ctx.scale(scale,scale);ctx.lineCap=ctx.lineJoin='round';const truth=[];
  for(let row=0;row<8;row++)for(let col=0;col<8;col++){
   const angle=row*Math.PI/16,cx=90+col*85,cy=80+row*115;
   const point=(x,y)=>({x:cx+x*Math.cos(angle)-y*Math.sin(angle),y:cy+x*Math.sin(angle)+y*Math.cos(angle)});
   const curve=[];for(let x=-25;x<=25;x+=.2)curve.push(point(x,(2+col*.5)*Math.sin(x*Math.PI/(4+col))));
   ctx.strokeStyle='rgba(0,0,0,.18)';ctx.lineWidth=1.3;ctx.beginPath();ctx.moveTo(curve[0].x,curve[0].y);for(const p of curve.slice(1))ctx.lineTo(p.x,p.y);ctx.stroke();truth.push(curve);
   ctx.strokeStyle='rgba(0,0,0,.85)';ctx.lineWidth=2;for(let x=-18;x<22;x+=9){let a=point(x,-7),b=point(x,7);ctx.beginPath();ctx.moveTo(a.x,a.y);ctx.lineTo(b.x,b.y);ctx.stroke();}
  }
  const im=ctx.getImageData(0,0,w,h),coverage=new Uint8Array(w*h);for(let i=0;i<coverage.length;i++){let a=im.data[i*4+3];coverage[i]=a;im.data.set([0,49,225,Math.min(255,a*2)],i*4);}
  const ink=await ScannerPro.InkVectorizer.fromPixels(im,{coverage,forceJavaScript});ctx.setTransform(1,0,0,1,0,0);ctx.clearRect(0,0,w,h);ScannerPro.InkVectorizer.render(ctx,ink);const pixels=ctx.getImageData(0,0,w,h).data;
  const covered=p=>{for(let dy=-.4;dy<=.4;dy+=.4)for(let dx=-.4;dx<=.4;dx+=.4)if(pixels[(Math.floor((p.y+dy)*scale)*w+Math.floor((p.x+dx)*scale))*4+3]>100)return true;return false;};
  return {strokes:ink.strokes.length,recalls:truth.map(c=>c.filter(covered).length/c.length),shape:JSON.stringify(ink.strokes)};
  },forceJavaScript));
  expect(results[0].shape).toEqual(results[1].shape);
  for(const result of results){
    const mean=result.recalls.reduce((a,b)=>a+b,0)/result.recalls.length;
    console.log('OBLIQUE_INK',JSON.stringify({mean,minimum:Math.min(...result.recalls),strokes:result.strokes}));
    expect(mean).toBeGreaterThan(.99);
    expect(Math.min(...result.recalls)).toBeGreaterThan(.92);
    // More samples must not become thousands of artificial short branches.
    expect(result.strokes).toBeLessThan(1500);
  }
});
