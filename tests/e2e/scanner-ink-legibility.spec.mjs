import {expect,test} from '@playwright/test';

// Small handwritten loops, ascenders and crossings, through an actual JPEG
// camera image. Checking only long straight lines hid filled letter counters.
async function handwriting(page) {
  await page.goto('/scanner/index.html',{waitUntil:'domcontentloaded'});
  return page.evaluate(async()=>{
    const width=1800,height=2546,scale=width/794;
    const source=document.createElement('canvas');source.width=794;source.height=1123;
    const ctx=source.getContext('2d');
    ScannerPro.ScanTemplate.render(ctx,794,1123,{background:true});
    const truth=document.createElement('canvas');truth.width=794;truth.height=1123;
    const tc=truth.getContext('2d');const counters=[],cores=[];
    const colors=['#294e9b','#a52e3c','#444444'];
    for(let row=0;row<3;row++)for(let column=0;column<18;column++){
      const x=100+column*29,y=211+row*100;
      const pen=column%3===0?1.6:2;
      counters.push({x,y,color:row});
      for(const c of [ctx,tc]){
        c.strokeStyle=c===ctx?colors[row]:'#000';c.lineWidth=pen;c.lineCap=c.lineJoin='round';
        c.beginPath();c.ellipse(x,y,2.4,3.3,-.15,0,Math.PI*2);c.stroke();
        // The stem meets the loop, as in a/d; it is not a thicker pen.
        c.beginPath();c.moveTo(x+3,y-9);c.lineTo(x+3,y+4);c.quadraticCurveTo(x+5,y+5,x+8,y+1);c.stroke();
      }
      for(let angle=0;angle<Math.PI*2;angle+=Math.PI/6)cores.push({x:x+2.4*Math.cos(angle),y:y+3.3*Math.sin(angle),color:row});
    }
    // Real paper exposure and lens softness, followed by JPEG chroma loss.
    const pixels=ctx.getImageData(0,0,794,1123);
    for(let i=0;i<pixels.data.length;i+=4)for(let c=0;c<3;c++)pixels.data[i+c]*=[.83,.78,.7][c];
    ctx.putImageData(pixels,0,0);
    const camera=document.createElement('canvas');camera.width=794;camera.height=1123;
    const cc=camera.getContext('2d');cc.filter='blur(.95px)';cc.drawImage(source,0,0);cc.filter='none';
    const blob=await new Promise(r=>camera.toBlob(r,'image/jpeg',.88)),bitmap=await createImageBitmap(blob);
    const output=document.createElement('canvas');output.width=width;output.height=height;
    const oc=output.getContext('2d',{willReadFrequently:true});oc.drawImage(bitmap,0,0,width,height);bitmap.close();
    const photographed=oc.getImageData(0,0,width,height);
    const reports=[];
    let rustCoverage;
    for(const disableRust of [false,true]){
      oc.putImageData(photographed,0,0);
      const correction=await ScannerPro.Lightweight.correctColors(output,{useStencil:true,preciseStencil:true,captureInkGeometry:true,disableRust});
      const inkPixels=oc.getImageData(0,0,width,height);InhouseRust.separateInk(inkPixels.data);
      const started=performance.now();
      const ink=await ScannerPro.InkVectorizer.fromPixels(inkPixels,{coverage:correction.inkCoverage,forceJavaScript:disableRust});
      const vectorMs=performance.now()-started;
      if(!disableRust)rustCoverage=correction.inkCoverage.slice();
      const maxCoverageDifference=Math.max(...Array.from({length:10000},(_,i)=>{
        const at=Math.floor(i/10000*width*height);return Math.abs(correction.inkCoverage[at]-rustCoverage[at]);}));
      oc.clearRect(0,0,width,height);ScannerPro.InkVectorizer.render(oc,ink);
      const rendered=oc.getImageData(0,0,width,height).data;
      const alpha=(x,y)=>rendered[(Math.round(y*scale)*width+Math.round(x*scale))*4+3];
      const rows=colors.map((_,row)=>({
        open:counters.filter(p=>p.color===row&&alpha(p.x,p.y)<40).length,
        coreRecall:cores.filter(p=>p.color===row).map(p=>{
          for(let dy=-.7;dy<=.7;dy+=.7)for(let dx=-.7;dx<=.7;dx+=.7)if(alpha(p.x+dx,p.y+dy)>150)return 1;
          return 0;
        }).reduce((a,b)=>a+b,0)/cores.filter(p=>p.color===row).length,
        widthP95:(()=>{const widths=ink.strokes.filter(s=>s.points.some(p=>Math.abs(p.y/scale-(211+row*100))<12)&&s.points.some(p=>p.x/scale>90&&p.x/scale<620)).map(s=>s.width/scale).sort((a,b)=>a-b);return widths[Math.floor(widths.length*.95)];})(),
        widest:Math.max(...ink.strokes.filter(s=>s.points.some(p=>Math.abs(p.y/scale-(211+row*100))<12)&&s.points.some(p=>p.x/scale>90&&p.x/scale<620)).map(s=>s.width/scale))
      }));
      reports.push({backend:ink.backend,vectorMs,rows,maxCoverageDifference,strokes:ink.strokes.length});
    }
    return reports;
  });
}

test('small photographed letters keep their counters, thin strokes and measured widths in Rust and JS',async({page})=>{
  test.setTimeout(60000);
  const reports=await handwriting(page);console.log('INK_LEGIBILITY',JSON.stringify(reports));
  expect(reports.map(r=>r.backend)).toEqual(['rust','javascript']);
  for(const report of reports){
    expect(report.maxCoverageDifference).toBeLessThanOrEqual(1);
    for(const row of report.rows){
      expect(row.open).toBeGreaterThanOrEqual(17);
      expect(row.coreRecall).toBeGreaterThan(.92);
      expect(row.widthP95).toBeLessThan(2.7);
      // A few unresolved JPEG junctions can be broader than the pen. They
      // must not turn into the 7–8px blobs of the old conversion.
      expect(row.widest).toBeLessThan(4.5);
    }
  }
  expect(reports[0].rows).toEqual(reports[1].rows);
});

test('pigment enhancement cannot change pen geometry or resurrect erased template pixels',async({page})=>{
  await page.goto('/scanner/index.html',{waitUntil:'domcontentloaded'});
  const report=await page.evaluate(async()=>{
    const width=794,height=1123,c=document.createElement('canvas');c.width=width;c.height=height;
    const ctx=c.getContext('2d');ctx.lineCap='round';ctx.lineWidth=2;ctx.strokeStyle='#000';
    ctx.beginPath();ctx.arc(150,180,8,0,2*Math.PI);ctx.stroke();
    ctx.beginPath();ctx.moveTo(158,165);ctx.lineTo(158,190);ctx.lineTo(166,186);ctx.stroke();
    const source=ctx.getImageData(0,0,width,height),coverage=new Uint8Array(width*height);
    for(let i=0;i<coverage.length;i++)coverage[i]=Math.round(source.data[i*4+3]*.5);
    // Low-contrast pencil survives alongside stronger, enhanced colour.
    for(let y=240;y<242;y++)for(let x=100;x<180;x++)coverage[y*width+x]=12;
    // The photographed dot was removed by stencil cleanup. Raw coverage
    // alone is insufficient evidence to bring it back as a pen.
    coverage[300*width+150]=160;
    const results=[];
    for(const [rgb,alpha] of [[[0,49,225],230],[[225,0,0],160]]){
      const pixels=new ImageData(width,height);
      for(let i=0;i<coverage.length;i++)if(coverage[i]&&i!==300*width+150){
        const pencil=Math.floor(i/width)>=240;
        pixels.data.set(pencil?[0,0,0,40]:[...rgb,alpha],i*4);
      }
      const ink=await ScannerPro.InkVectorizer.fromPixels(pixels,{coverage});
      const js=await ScannerPro.InkVectorizer.fromPixels(pixels,{coverage,forceJavaScript:true});
      results.push({shape:ink.strokes.map(({width,points})=>({width,points})),
        parity:JSON.stringify(ink.strokes)===JSON.stringify(js.strokes),
        pencil:ink.strokes.some(s=>s.color==='#4D4D4D'&&s.points.some(p=>p.y>=240&&p.y<243)),
        template:ink.strokes.some(s=>s.points.some(p=>p.y>=298&&p.y<=302)),
        colours:[...new Set(ink.strokes.map(s=>s.color))].sort()});
    }
    return results;
  });
  expect(report[0].shape).toEqual(report[1].shape);
  expect(report[0].colours).toEqual(['#002FD9','#4D4D4D']);
  expect(report[1].colours).toEqual(['#4D4D4D','#E81010']);
  for(const row of report){expect(row.parity).toBe(true);expect(row.pencil).toBe(true);expect(row.template).toBe(false);}
});
