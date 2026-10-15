import {expect,test} from '@playwright/test';

test('recovering pen lobes cannot borrow a nearby different ink colour',async({page})=>{
  test.setTimeout(60000);
  await page.goto('/scanner/index.html',{waitUntil:'domcontentloaded'});
  const reports=await page.evaluate(async()=>{
    const width=794,height=1123,canvas=document.createElement('canvas');
    canvas.width=width;canvas.height=height;
    const context=canvas.getContext('2d',{willReadFrequently:true});
    context.lineCap='round';
    // Three disconnected pens pass close to one another. Their wider lobes
    // are closer to a neighbouring pen's axis than to their own axis. Using
    // the nearest rendered pixel would paint blue/red lobes in another ink.
    for(let row=0;row<3;row++){
      context.strokeStyle=context.fillStyle=InhouseInkPalette[row];
      context.lineWidth=2;
      const y=160+row*10;
      context.beginPath();context.moveTo(100,y);context.lineTo(600,y);context.stroke();
      for(let column=0;column<8;column++){
        context.beginPath();context.arc(125+column*55+row*15,y,7.5,0,Math.PI*2);context.fill();
      }
    }
    const photographed=context.getImageData(0,0,width,height),coverage=new Uint8Array(width*height);
    for(let i=0;i<coverage.length;i++)coverage[i]=Math.round(photographed.data[i*4+3]*.75);
    const colour=(r,g,b)=>b-r>30?1:r-b>30?2:0;
    const reports=[];
    for(const forceJavaScript of [false,true]){
      const ink=await ScannerPro.InkVectorizer.fromPixels(photographed,{coverage,forceJavaScript});
      context.clearRect(0,0,width,height);ScannerPro.InkVectorizer.render(context,ink);
      const rendered=context.getImageData(0,0,width,height).data;
      const compared=[0,0,0],wrong=[0,0,0];
      for(let i=0;i<coverage.length;i++){
        const at=i*4;
        if(photographed.data[at+3]<230||rendered[at+3]<230)continue;
        const expected=colour(photographed.data[at],photographed.data[at+1],photographed.data[at+2]);
        compared[expected]++;
        if(colour(rendered[at],rendered[at+1],rendered[at+2])!==expected)wrong[expected]++;
      }
      reports.push({backend:ink.backend,repairBackend:ink.repairBackend,compared,wrong,strokes:ink.strokes});
    }
    return {backends:reports.map(r=>r.backend),repairBackends:reports.map(r=>r.repairBackend),compared:reports.map(r=>r.compared),
      wrong:reports.map(r=>r.wrong),parity:JSON.stringify(reports[0].strokes)===JSON.stringify(reports[1].strokes)};
  });
  expect(reports.backends).toEqual(['rust','javascript']);
  expect(reports.repairBackends).toEqual(['rust','javascript']);
  for(const compared of reports.compared)for(const count of compared)expect(count).toBeGreaterThan(520);
  expect(reports.wrong).toEqual([[0,0,0],[0,0,0]]);
  expect(reports.parity).toBe(true);
});

test('a large filled drawing keeps its hole without generating thousands of repair pens',async({page})=>{
  test.setTimeout(60000);
  await page.goto('/scanner/index.html',{waitUntil:'domcontentloaded'});
  const reports=await page.evaluate(async()=>{
    const width=794,height=1123,canvas=document.createElement('canvas');
    canvas.width=width;canvas.height=height;
    const context=canvas.getContext('2d',{willReadFrequently:true});
    context.fillStyle=InhouseInkPalette[0];context.fillRect(100,300,440,180);
    context.globalCompositeOperation='destination-out';
    context.beginPath();context.arc(320,390,42,0,Math.PI*2);context.fill();
    context.globalCompositeOperation='source-over';
    const photographed=context.getImageData(0,0,width,height),coverage=new Uint8Array(width*height);
    for(let i=0;i<coverage.length;i++)coverage[i]=Math.round(photographed.data[i*4+3]*.75);
    const reports=[];
    for(const forceJavaScript of [false,true]){
      const started=performance.now();
      const ink=await ScannerPro.InkVectorizer.fromPixels(photographed,{coverage,forceJavaScript});
      const elapsed=performance.now()-started;
      context.clearRect(0,0,width,height);ScannerPro.InkVectorizer.render(context,ink);
      const rendered=context.getImageData(0,0,width,height).data;
      const alpha=(x,y)=>rendered[(y*width+x)*4+3];
      reports.push({backend:ink.backend,repairBackend:ink.repairBackend,elapsed,strokes:ink.strokes.length,
        points:ink.strokes.reduce((sum,stroke)=>sum+stroke.points.length,0),
        hole:alpha(320,390),ink:alpha(200,390)});
    }
    return reports;
  });
  expect(reports.map(r=>r.backend)).toEqual(['rust','javascript']);
  expect(reports.map(r=>r.repairBackend)).toEqual(['rust','javascript']);
  for(const report of reports){
    expect(report.strokes).toBeLessThan(1000);
    expect(report.points).toBeLessThan(50000);
    expect(report.hole).toBe(0);
    expect(report.ink).toBeGreaterThan(230);
    // A generous watchdog catches unbounded candidate work, rather than
    // turning CI scheduling differences into a microbenchmark failure.
    expect(report.elapsed).toBeLessThan(10000);
  }
  expect(reports[0].strokes).toBe(reports[1].strokes);
  expect(reports[0].points).toBe(reports[1].points);
});
