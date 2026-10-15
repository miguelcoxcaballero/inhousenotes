import { test, expect } from '@playwright/test';

test('shaded blue stays one colour across black patches and junctions, without recolouring a crossing black pen', async ({ page }) => {
  await page.goto('/scanner/index.html?embed=1&session=pen-colours');
  const report = await page.evaluate(async () => {
    const canvas = document.createElement('canvas'); canvas.width = 794; canvas.height = 1123;
    const ctx = canvas.getContext('2d');
    const line = (y, colour) => { ctx.fillStyle = colour; ctx.fillRect(60, y, 590, 3); };
    line(100, '#2546b8');
    for (const [x,w] of [[110,35],[240,40],[400,50],[550,60]]) {
      ctx.fillStyle = '#292929'; ctx.fillRect(x,100,w,3);
    }
    ctx.fillStyle = '#303030'; ctx.fillRect(349,65,3,78);
    line(220, '#353535');
    for (let x=95;x<630;x+=45) { ctx.fillStyle='#203c86';ctx.fillRect(x,220,2,3); }
    line(340, '#181e32'); // Dark navy: chroma below the old fixed 36 threshold.
    line(460, '#2a2f39'); // Neutral black with a small cold sensor cast.
    line(580, '#b5262a');
    for (const [x,w] of [[110,35],[240,40],[400,50],[550,60]]) {
      ctx.fillStyle='#303030';ctx.fillRect(x,580,w,3);
    }
    const pixels=ctx.getImageData(0,0,canvas.width,canvas.height);
    const ink=await ScannerPro.InkVectorizer.fromPixels(pixels);
    const fallback=await ScannerPro.InkVectorizer.fromPixels(pixels,{forceJavaScript:true});
    return {strokes:ink.strokes,backend:ink.backend,parity:JSON.stringify(ink.strokes)===JSON.stringify(fallback.strokes)};
  });
  expect(report.backend).toBe('rust'); expect(report.parity).toBe(true);
  for (const [row,colour] of [[100,'#002FD9'],[220,'#4D4D4D'],[340,'#002FD9'],[460,'#4D4D4D'],[580,'#E81010']]) {
    const ink=report.strokes.filter(s=>s.points.some(p=>p.x>70&&p.x<640&&Math.abs(p.x-350)>6&&Math.abs(p.y-(row+1))<2));
    expect(ink.length).toBeGreaterThan(0);
    expect(new Set(ink.map(s=>s.color))).toEqual(new Set([colour]));
    expect(Math.max(...ink.flatMap(s=>s.points.map(p=>p.x)))-Math.min(...ink.flatMap(s=>s.points.map(p=>p.x)))).toBeGreaterThan(570);
  }
  const crossing=report.strokes.filter(s=>s.points.some(p=>p.x>347&&p.x<354&&(p.y<90||p.y>112)&&p.y<150));
  expect(crossing.length).toBeGreaterThan(0);
  expect(crossing.every(s=>s.color==='#4D4D4D')).toBe(true);
  expect(report.strokes.length).toBeLessThan(20);
});

test('light smoothing removes raster zigzags at either resolution, retaining endpoints, sharp turns and closed loops', async ({page}) => {
  await page.goto('/scanner/index.html?embed=1&session=pen-smooth');
  const reports=await page.evaluate(async()=>{
    const reports=[];
    for(const scale of [1,2]) {
      const c=document.createElement('canvas');c.width=794*scale;c.height=1123*scale;
      const x=c.getContext('2d');x.scale(scale,scale);x.fillStyle='#333';
      x.strokeStyle='#333';x.lineWidth=3;x.lineCap=x.lineJoin='round';
      x.beginPath();
      for(let px=60;px<=650;px++) {
        const y=250+20*Math.sin((px-60)/50)+((Math.floor(px/2)%2)? .65:-.65);
        if(px===60)x.moveTo(px,y);else x.lineTo(px,y);
      }
      x.stroke();
      x.beginPath();x.moveTo(100,500);x.lineTo(200,600);x.lineTo(300,500);x.stroke();
      x.beginPath();x.arc(450,700,40,0,Math.PI*2);x.stroke();
      const ink=await ScannerPro.InkVectorizer.fromPixels(x.getImageData(0,0,c.width,c.height));
      const pens=ink.strokes.map(s=>({...s,points:s.points.map(p=>({x:p.x/scale,y:p.y/scale}))}));
      const curve=pens.filter(s=>s.points.every(p=>p.y<300)).sort((a,b)=>b.points.length-a.points.length)[0];
      const points=curve.points.slice().sort((a,b)=>a.x-b.x);
      const at=px=>{const i=points.findIndex(p=>p.x>=px),a=points[i-1],b=points[i];return a&&b?a.y+(b.y-a.y)*(px-a.x)/(b.x-a.x):null;};
      let error=0,roughness=0,count=0;
      for(let px=70;px<640;px++) {
        const y=at(px),a=at(px-2),b=at(px+2);if(y===null||a===null||b===null)continue;
        error+=(y-(250+20*Math.sin((px-60)/50)))**2;
        roughness+=Math.abs(y-(a+b)/2);count++;
      }
      const corner=pens.filter(s=>s.points.some(p=>p.y>490&&p.y<610));
      const loop=pens.find(s=>s.points.every(p=>p.y>650));
      reports.push({scale,rms:Math.sqrt(error/count),roughness:roughness/count,
        start:points[0],end:points.at(-1),
        cornerDistance:Math.min(...corner.flatMap(s=>s.points.map(p=>Math.hypot(p.x-200,p.y-600)))),
        loopGap:Math.hypot(loop.points[0].x-loop.points.at(-1).x,loop.points[0].y-loop.points.at(-1).y),
        loopRadiusError:Math.max(...loop.points.map(p=>Math.abs(Math.hypot(p.x-450,p.y-700)-40))),
        strokes:ink.strokes.length,points:ink.strokes.reduce((n,s)=>n+s.points.length,0)});
    }
    return reports;
  });
  console.log('PEN_SMOOTHNESS',JSON.stringify(reports));
  for(const r of reports) {
    expect(r.rms).toBeLessThan(.65);
    expect(r.roughness).toBeLessThan(.3);
    expect(r.start.x).toBeLessThan(63);expect(r.end.x).toBeGreaterThan(647);
    expect(r.cornerDistance).toBeLessThan(1.6);
    expect(r.loopGap).toBeLessThan(.01);expect(r.loopRadiusError).toBeLessThan(1.5);
    expect(r.strokes).toBeLessThan(10);expect(r.points).toBeLessThan(1500);
  }
});

test('a shaded near-standard pen keeps one width and smooth axis while a cold black remains black',async({page})=>{
  await page.goto('/scanner/index.html?embed=1&session=pen-width-coherence');
  const reports=await page.evaluate(async()=>{
    const reports=[];
    for(const scale of [1,2]){
      const c=document.createElement('canvas');c.width=794*scale;c.height=1123*scale;
      const x=c.getContext('2d');x.scale(scale,scale);x.lineCap=x.lineJoin='round';
      for(const [row,colour] of [[200,'#203daf'],[400,'#b52a30'],[600,'#18212c']]){
        for(let px=60;px<650;px++){
          const y=dx=>row+10*Math.sin((dx-60)/45)+.35*Math.sin(dx*2);
          x.strokeStyle=px>280&&px<340?'#303030':colour;
          x.lineWidth=2.1+.5*Math.sin(px/26);
          x.beginPath();x.moveTo(px,y(px));x.lineTo(px+1,y(px+1));x.stroke();
        }
      }
      const pixels=x.getImageData(0,0,c.width,c.height);
      const ink=await ScannerPro.InkVectorizer.fromPixels(pixels);
      const js=await ScannerPro.InkVectorizer.fromPixels(pixels,{forceJavaScript:true});
      reports.push({scale,parity:JSON.stringify(ink.strokes)===JSON.stringify(js.strokes),
        strokes:ink.strokes.map(s=>({...s,width:s.width/scale,points:s.points.map(p=>({x:p.x/scale,y:p.y/scale}))}))});
    }
    return reports;
  });
  for(const report of reports){
    expect(report.parity).toBe(true);
    for(const [row,colour] of [[200,'#002FD9'],[400,'#E81010'],[600,'#4D4D4D']]){
      const pens=report.strokes.filter(s=>s.points.some(p=>Math.abs(p.y-row)<15));
      expect(pens.length).toBeGreaterThan(0);expect(pens.length).toBeLessThan(8);
      expect(new Set(pens.map(s=>s.color))).toEqual(new Set([colour]));
      expect(new Set(pens.map(s=>s.width))).toEqual(new Set([2]));
      const points=pens.flatMap(s=>s.points);
      expect(Math.min(...points.map(p=>p.x))).toBeLessThan(63);
      expect(Math.max(...points.map(p=>p.x))).toBeGreaterThan(647);
      const rms=Math.sqrt(points.reduce((sum,p)=>sum+(p.y-(row+10*Math.sin((p.x-60)/45)))**2,0)/points.length);
      expect(rms).toBeLessThan(.65);
    }
  }
});
