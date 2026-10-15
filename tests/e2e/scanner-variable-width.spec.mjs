import {expect,test} from '@playwright/test';

// A photographed pen changes width along the same connected letter. An
// ellipse rendered at a single width cannot expose median-width replacement
// that deletes thin tapers or turns small handwritten lobes into blobs.
test('photographed variable-width letters retain their silhouette, tapers and open counters',async({page})=>{
  test.setTimeout(90000);
  await page.goto('/scanner/index.html',{waitUntil:'domcontentloaded'});
  const reports=await page.evaluate(async()=>{
    const cameraWidth=1200,cameraHeight=Math.round(cameraWidth*1123/794);
    const width=1800,height=Math.round(width*1123/794),scale=width/794;
    const source=document.createElement('canvas');source.width=cameraWidth;source.height=cameraHeight;
    const photo=source.getContext('2d',{willReadFrequently:true});
    ScannerPro.ScanTemplate.render(photo,cameraWidth,cameraHeight,{background:true});
    const truth=document.createElement('canvas');truth.width=width;truth.height=height;
    const tc=truth.getContext('2d'),glyphs=[],colors=['#294e9b','#444444','#a52e3c'];
    const curve=(a,b,c,d,t)=>{
      const u=1-t;return {x:u*u*u*a[0]+3*u*u*t*b[0]+3*u*t*t*c[0]+t*t*t*d[0],
        y:u*u*u*a[1]+3*u*u*t*b[1]+3*u*t*t*c[1]+t*t*t*d[1]};
    };
    const paths=[];
    for(let row=0;row<3;row++)for(let column=0;column<8;column++){
      const x=104+column*72+(column%3)*.23,y=200+row*180+(column%2)*.31;
      const skew=(column%4-1.5)*.05;
      // Notebook letters have 8.4–12px x-height and 18px ascenders. Narrow
      // letters use the same physical pen as wider ones, as on real paper.
      const size=column<4?1:.7;
      const transform=(p,ascender=false)=>{
        const yy=ascender?p.y:18-(18-p.y)*size;
        return {x:x+p.x*size+skew*yy,y:y+yy};
      };
      const loop=Array.from({length:161},(_,i)=>{
        const a=i/160*Math.PI*2;
        return {...transform({x:8+5*Math.cos(a),y:12+6*Math.sin(a)}),width:.8+2*((1+Math.sin(a))/2)**1.5};
      });
      const stem=Array.from({length:91},(_,i)=>{
        const t=i/90;return {...transform({x:13+.5*Math.sin(t*Math.PI),y:18*t},true),width:.8+2*t*t*t};
      });
      const tail=Array.from({length:121},(_,i)=>{
        const t=i/120,p=curve([13,17],[17,25],[20,1],[25,13],t);
        return {...transform(p),width:.8+2*Math.sin(t*Math.PI)**2};
      });
      paths.push({color:colors[row],points:loop},{color:colors[row],points:stem},{color:colors[row],points:tail});
      // The narrow high stem is isolated from the loop and grid intersections;
      // retaining only the darkest neighbourhood core must not shorten it.
      glyphs.push({row,size,box:{x:x-2,y:y-3,w:31*size+4,h:31},counter:transform({x:8,y:12}),
        tip:transform({x:13,y:.6},true),thin:transform({x:13+.5*Math.sin(.2*Math.PI),y:3.6},true)});
    }
    for(const [ctx,s,mono] of [[photo,cameraWidth/794,false],[tc,scale,true]]){
      ctx.save();ctx.scale(s,s);ctx.lineCap=ctx.lineJoin='round';
      for(const path of paths){
        ctx.strokeStyle=mono?'#000':path.color;
        for(let i=1;i<path.points.length;i++){
          const a=path.points[i-1],b=path.points[i];ctx.lineWidth=(a.width+b.width)/2;
          ctx.beginPath();ctx.moveTo(a.x,a.y);ctx.lineTo(b.x,b.y);ctx.stroke();
        }
      }
      ctx.restore();
    }
    const photograph=photo.getImageData(0,0,cameraWidth,cameraHeight);
    for(let i=0;i<photograph.data.length;i+=4){
      const x=(i/4)%cameraWidth,shade=.93-.08*Math.abs(x/cameraWidth-.5);
      for(let c=0;c<3;c++)photograph.data[i+c]*=[.96,.9,.82][c]*shade;
    }
    photo.putImageData(photograph,0,0);
    const lens=document.createElement('canvas');lens.width=cameraWidth;lens.height=cameraHeight;
    const lc=lens.getContext('2d');lc.filter='blur(.35px)';lc.drawImage(source,0,0);lc.filter='none';
    const jpeg=await new Promise(resolve=>lens.toBlob(resolve,'image/jpeg',.92));
    const bitmap=await createImageBitmap(jpeg);
    const output=document.createElement('canvas');output.width=width;output.height=height;
    const oc=output.getContext('2d',{willReadFrequently:true});oc.drawImage(bitmap,0,0,width,height);bitmap.close();
    const captured=oc.getImageData(0,0,width,height),expected=tc.getImageData(0,0,width,height).data;
    const reports=[],tolerance=.5*scale;
    const offsets=[];for(let dy=-Math.ceil(tolerance);dy<=Math.ceil(tolerance);dy++)for(let dx=-Math.ceil(tolerance);dx<=Math.ceil(tolerance);dx++){
      if(dx*dx+dy*dy<=tolerance*tolerance)offsets.push([dx,dy]);
    }
    for(const forceJavaScript of [false,true]){
      oc.putImageData(captured,0,0);
      const correction=await ScannerPro.Lightweight.correctColors(output,{useStencil:true,preciseStencil:true,captureInkGeometry:true,disableRust:forceJavaScript});
      const pixels=oc.getImageData(0,0,width,height);InhouseRust.separateInk(pixels.data);
      const ink=await ScannerPro.InkVectorizer.fromPixels(pixels,{coverage:correction.inkCoverage,forceJavaScript});
      oc.clearRect(0,0,width,height);ScannerPro.InkVectorizer.render(oc,ink);
      const actual=oc.getImageData(0,0,width,height).data;
      const nearby=(data,x,y)=>offsets.some(([dx,dy])=>data[((y+dy)*width+x+dx)*4+3]>=128);
      const metrics=colors.map((_,row)=>{
        let original=0,rendered=0,missing=0,extra=0,open=0,tips=0;
        const individual=[];
        for(const g of glyphs.filter(g=>g.row===row)){
          let glyphOriginal=0,glyphRendered=0,glyphMissing=0,glyphExtra=0;
          for(let y=Math.floor(g.box.y*scale);y<Math.ceil((g.box.y+g.box.h)*scale);y++)
            for(let x=Math.floor(g.box.x*scale);x<Math.ceil((g.box.x+g.box.w)*scale);x++){
              const i=(y*width+x)*4;
              if(expected[i+3]>=128){glyphOriginal++;if(!nearby(actual,x,y))glyphMissing++;}
              if(actual[i+3]>=128){glyphRendered++;if(!nearby(expected,x,y))glyphExtra++;}
            }
          original+=glyphOriginal;rendered+=glyphRendered;missing+=glyphMissing;extra+=glyphExtra;
          const precision=1-glyphExtra/Math.max(1,glyphRendered),recall=1-glyphMissing/Math.max(1,glyphOriginal);
          individual.push({size:g.size,precision,recall,f1:2*precision*recall/(precision+recall)});
          const cx=Math.round(g.counter.x*scale),cy=Math.round(g.counter.y*scale);
          if(!offsets.some(([dx,dy])=>actual[((cy+dy)*width+cx+dx)*4+3]>=80))open++;
          if(nearby(actual,Math.round(g.tip.x*scale),Math.round(g.tip.y*scale)))tips++;
        }
        const precision=1-extra/Math.max(1,rendered),recall=1-missing/Math.max(1,original);
        return {row,original,rendered,missing,extra,precision,recall,f1:2*precision*recall/(precision+recall),open,tips,individual};
      });
      reports.push({backend:ink.backend,repairBackend:ink.repairBackend,repairCount:ink.repairCount,metrics,
        strokes:ink.strokes.length,points:ink.strokes.reduce((sum,s)=>sum+s.points.length,0)});
    }
    return reports;
  });
  console.log('VARIABLE_WIDTH_INK',JSON.stringify(reports));
  expect(reports.map(r=>r.backend)).toEqual(['rust','javascript']);
  for(const report of reports)for(const row of report.metrics){
    // Half an editor pixel allows antialiasing, subpixel alignment and the
    // requested gentle width normalisation, not missing loops or fat blobs.
    expect(row.recall,`${report.backend} row${row.row}: missing handwritten ink`).toBeGreaterThan(.98);
    expect(row.precision,`${report.backend} row${row.row}: enlarged ink silhouette`).toBeGreaterThan(.90);
    expect(row.f1,`${report.backend} row${row.row}: letter silhouette`).toBeGreaterThan(.91);
    expect(row.open,`${report.backend} row${row.row}: counters`).toBe(8);
    expect(row.tips,`${report.backend} row${row.row}: thin ascender ends`).toBe(8);
    for(const glyph of row.individual){
      expect(glyph.recall,`${report.backend} row${row.row}: individual letter`).toBeGreaterThan(.97);
      expect(glyph.f1,`${report.backend} row${row.row}: individual silhouette`).toBeGreaterThan(.95);
    }
  }
  expect(reports[0].metrics).toEqual(reports[1].metrics);
});
