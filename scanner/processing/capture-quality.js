/* Measure the frozen photograph, independently of reduced live-view confidence.
 * Edge acutance distinguishes a narrow ink edge from a broad motion/focus smear;
 * raw Laplacian variance alone can be inflated by contrast and camera noise.
 */
(() => {
  const SP = window.ScannerPro = window.ScannerPro || {};
  function readSource(source) {
    const scale=Math.min(1,1100/Math.max(source.width,source.height));
    const canvas=document.createElement('canvas');
    canvas.width=Math.max(1,Math.round(source.width*scale));
    canvas.height=Math.max(1,Math.round(source.height*scale));
    try {
      const context=canvas.getContext('2d',{willReadFrequently:true});
      context.drawImage(source,0,0,canvas.width,canvas.height);
      const rgba=context.getImageData(0,0,canvas.width,canvas.height).data;
      const gray=new Float32Array(canvas.width*canvas.height);
      for(let index=0;index<gray.length;index++) {
        const at=index*4;gray[index]=rgba[at]*.2126+rgba[at+1]*.7152+rgba[at+2]*.0722;
      }
      return {width:canvas.width,height:canvas.height,gray};
    } finally {canvas.width=canvas.height=0;}
  }
  function inspect(source, detection, raster) {
    if (!detection?.quad || !source.width || !source.height) return {valid:false,reason:'missing-page'};
    const {width,height,gray}=raster||readSource(source);
      const quad=detection.quad.map(p=>({x:p.x*width,y:p.y*height}));
      const luma=(x,y)=>gray[Math.max(0,Math.min(height-1,y))*width+Math.max(0,Math.min(width-1,x))];
      const gradient=(x,y)=>{
        const dx=luma(x+1,y)-luma(x-1,y),dy=luma(x,y+1)-luma(x,y-1);
        const bx=luma(x+3,y)-luma(x-3,y),by=luma(x,y+3)-luma(x,y-3);
        const fine=dx*dx+dy*dy,broad=bx*bx+by*by;
        return fine+broad>=144 ? {fine,broad} : null;
      };
      const zones=Array.from({length:9},()=>({fine:0,broad:0,count:0}));
      const left=Math.max(4,Math.floor(Math.min(...quad.map(p=>p.x)))),right=Math.min(width-5,Math.ceil(Math.max(...quad.map(p=>p.x))));
      const top=Math.max(4,Math.floor(Math.min(...quad.map(p=>p.y)))),bottom=Math.min(height-5,Math.ceil(Math.max(...quad.map(p=>p.y))));
      const inside=(x,y)=>{let sign=0;for(let i=0;i<4;i++){const a=quad[i],b=quad[(i+1)%4];const d=(b.x-a.x)*(y-a.y)-(b.y-a.y)*(x-a.x);
        if(Math.abs(d)<1)continue;if(sign&&Math.sign(d)!==sign)return false;sign=Math.sign(d);}return true;};
      const bottomEdge={x:quad[2].x-quad[3].x,y:quad[2].y-quad[3].y};
      const bottomLength=Math.hypot(bottomEdge.x,bottomEdge.y);
      const markerGap=(Math.hypot(quad[3].x-quad[0].x,quad[3].y-quad[0].y)+Math.hypot(quad[2].x-quad[1].x,quad[2].y-quad[1].y))*.045;
      for(let y=top+6;y<bottom-6;y+=2)for(let x=left+6;x<right-6;x+=2){
        if(!inside(x,y)||Math.abs((x-quad[3].x)*bottomEdge.y-(y-quad[3].y)*bottomEdge.x)/bottomLength<markerGap)continue;
        const value=gradient(x,y);if(!value)continue;
        const c=Math.min(2,Math.floor((x-left)/(right-left)*3)),r=Math.min(2,Math.floor((y-top)/(bottom-top)*3));
        const zone=zones[r*3+c];zone.fine+=value.fine;zone.broad+=value.broad;zone.count++;
      }
      const measured=zones.filter(z=>z.count>=12&&z.broad>0&&z.fine>z.broad*.02).map(z=>z.fine/z.broad).sort((a,b)=>a-b);
      let fine=0,broad=0,edges=0;
      for(const edge of detection.frameEdges || []) for(const point of edge) {
        const x=Math.round(point.x*width),y=Math.round(point.y*height);
        for(let dy=-2;dy<=2;dy++)for(let dx=-2;dx<=2;dx++){
          const value=gradient(x+dx,y+dy);if(!value)continue;
          fine+=value.fine;broad+=value.broad;edges++;
        }
      }
      const border=edges>=60&&broad>0?fine/broad:0;
      const interior=measured.length>=3?measured[Math.floor(measured.length*.25)]:null;
      const clarity=interior===null?border:Math.min(border,interior);
      return {valid:border>=.32&&(interior===null||interior>=.32),clarity,border,interior,zones:measured.length,
        edges,interiorMinimum:measured.length?measured[0]:null,reason:clarity>=.32?'sharp-page':'focus-or-motion-blur'};
  }
  const clamp = (value, low, high) => Math.max(low, Math.min(high, value));
  const mix = (a,b,t) => ({x:a.x+(b.x-a.x)*t,y:a.y+(b.y-a.y)*t});
  const along = (points,t) => {
    const at=clamp(t,0,1)*(points.length-1), index=Math.floor(at);
    return mix(points[index],points[Math.min(index+1,points.length-1)],at-index);
  };

  // A yellow rectangle (or sharp handwriting) is not evidence that the
  // photographed bullet lattice can be recovered. This bounded preflight
  // samples the actual frozen image, without generating any template dots or
  // doing the much more expensive physical surface fit. Neighbor cycles may
  // bend smoothly; we do not require the raw photograph's rows to be straight.
  function inspectGrid(source,detection,raster) {
    const empty = {valid:false,dots:0,matched:0,rows:0,columns:0,coverage:0,confidence:0};
    const quad=detection?.quad;
    if (!quad?.length || quad.length!==4 || quad.some(p=>!Number.isFinite(p.x)||!Number.isFinite(p.y))) return empty;
    const {width:sourceWidth,height:sourceHeight,gray:sourceData}=raster||readSource(source);
      const distance=(a,b)=>Math.hypot((a.x-b.x)*sourceWidth,(a.y-b.y)*sourceHeight);
      const landscape=distance(quad[0],quad[1])+distance(quad[2],quad[3])
        > distance(quad[1],quad[2])+distance(quad[3],quad[0]);
      const width=landscape?810:540,height=landscape?540:810,pitch=15;
      const gray=new Float32Array(width*height), integral=new Float64Array((width+1)*(height+1));
      const edges=detection.frameEdges;
      const useEdges=edges?.length===4 && edges.every(edge=>edge?.length>=2);
      const sides=[width,height,width,height].map((length,side)=>Array.from({length},(_,index)=>
        useEdges?along(edges[side],index/(length-1)):mix(quad[side],quad[(side+1)%4],index/(length-1))));
      let clipped=0;
      for(let y=0;y<height;y++) {
        const v=y/(height-1),left=sides[3][height-1-y],right=sides[1][y];
        let rowSum=0;
        for(let x=0;x<width;x++) {
          const u=x/(width-1),top=sides[0][x],bottom=sides[2][width-1-x];
          const baseX=(quad[0].x*(1-u)+quad[1].x*u)*(1-v)+(quad[3].x*(1-u)+quad[2].x*u)*v;
          const baseY=(quad[0].y*(1-u)+quad[1].y*u)*(1-v)+(quad[3].y*(1-u)+quad[2].y*u)*v;
          const sx=(top.x*(1-v)+bottom.x*v+left.x*(1-u)+right.x*u-baseX)*sourceWidth;
          const sy=(top.y*(1-v)+bottom.y*v+left.y*(1-u)+right.y*u-baseY)*sourceHeight;
          if(sx<1||sy<1||sx>=sourceWidth-2||sy>=sourceHeight-2) clipped++;
          const ix=clamp(Math.floor(sx),0,sourceWidth-2),iy=clamp(Math.floor(sy),0,sourceHeight-2);
          const fx=clamp(sx-ix,0,1),fy=clamp(sy-iy,0,1),at=iy*sourceWidth+ix;
          const value=(sourceData[at]*(1-fx)+sourceData[at+1]*fx)*(1-fy)
            +(sourceData[at+sourceWidth]*(1-fx)+sourceData[at+sourceWidth+1]*fx)*fy;
          gray[y*width+x]=value;rowSum+=value;
          integral[(y+1)*(width+1)+x+1]=integral[y*(width+1)+x+1]+rowSum;
        }
      }
      if(clipped>width*height*.01) return {...empty,reason:'page-clipped'};
      const mask=new Uint8Array(width*height), contrast=new Float32Array(width*height);
      const radius=4,margin=11,stride=width+1;
      for(let y=margin;y<height-margin;y++)for(let x=margin;x<width-margin;x++) {
        const a=(y-radius)*stride+x-radius,b=(y+radius+1)*stride+x+radius+1;
        const mean=(integral[b]-integral[(y-radius)*stride+x+radius+1]
          -integral[(y+radius+1)*stride+x-radius]+integral[a])/81;
        const at=y*width+x, deficit=mean-gray[at];
        // Relative local contrast retains real dots under dim or tinted light;
        // the absolute floor excludes quantization noise in almost black areas.
        if(deficit>=Math.max(4,mean*.022)) {mask[at]=1;contrast[at]=deficit;}
      }
      const queue=new Int32Array(width*height),dots=[];
      for(let start=0;start<mask.length;start++) {
        if(mask[start]!==1) continue;
        let read=0,end=1,minX=width,maxX=0,minY=height,maxY=0,weight=0,sx=0,sy=0,peak=0;
        queue[0]=start;mask[start]=2;
        while(read<end) {
          const at=queue[read++],x=at%width,y=Math.floor(at/width),w=contrast[at];
          minX=Math.min(minX,x);maxX=Math.max(maxX,x);minY=Math.min(minY,y);maxY=Math.max(maxY,y);
          weight+=w;sx+=x*w;sy+=y*w;peak=Math.max(peak,w);
          for(let dy=-1;dy<=1;dy++)for(let dx=-1;dx<=1;dx++) {
            const next=at+dy*width+dx;
            if(mask[next]===1) {mask[next]=2;queue[end++]=next;}
          }
        }
        const bw=maxX-minX+1,bh=maxY-minY+1;
        if(end<1||end>45||Math.max(bw,bh)>8||Math.max(bw,bh)>Math.min(bw,bh)*2.5||peak<6) continue;
        dots.push({x:sx/weight,y:sy/weight,contrast:peak,links:[-1,-1,-1,-1]});
        // Texture/noise is not a recoverable printed lattice. Bound later work
        // independently of the number of dark camera-noise components.
        if(dots.length>5500) return {...empty,dots:dots.length,reason:'page-texture'};
      }
      const buckets=new Map(),key=(x,y)=>Math.floor(x/pitch)+','+Math.floor(y/pitch);
      dots.forEach((dot,index)=>{const k=key(dot.x,dot.y);if(!buckets.has(k))buckets.set(k,[]);buckets.get(k).push(index);});
      dots.forEach((dot,index)=>{
        const best=[Infinity,Infinity,Infinity,Infinity],bx=Math.floor(dot.x/pitch),by=Math.floor(dot.y/pitch);
        for(let y=by-2;y<=by+2;y++)for(let x=bx-2;x<=bx+2;x++)for(const other of buckets.get(x+','+y)||[]) {
          if(other===index)continue;
          const dx=dots[other].x-dot.x,dy=dots[other].y-dot.y;
          const horizontal=Math.abs(dx)>Math.abs(dy),major=Math.abs(horizontal?dx:dy),minor=Math.abs(horizontal?dy:dx);
          if(major<pitch*.65||major>pitch*1.35||minor>pitch*.32) continue;
          const direction=horizontal?(dx>0?0:2):(dy>0?1:3);
          const error=Math.abs(major-pitch)+minor*1.5;
          if(error<best[direction]) {best[direction]=error;dot.links[direction]=other;}
        }
      });
      // Reciprocal links and complete elementary cells reject text baselines,
      // crossing ruled lines, random specks, and one-dimensional dot patterns.
      const matched=new Set(),cells=[];
      dots.forEach((dot,index)=>{
        const right=dot.links[0],down=dot.links[1];
        if(right<0||down<0||dots[right].links[2]!==index||dots[down].links[3]!==index) return;
        const corner=dots[right].links[1];
        if(corner<0||corner!==dots[down].links[0]||dots[corner].links[3]!==right||dots[corner].links[2]!==down)return;
        const a={x:dots[right].x-dot.x,y:dots[right].y-dot.y};
        const b={x:dots[down].x-dot.x,y:dots[down].y-dot.y};
        const c={x:dots[corner].x-dots[down].x,y:dots[corner].y-dots[down].y};
        const d={x:dots[corner].x-dots[right].x,y:dots[corner].y-dots[right].y};
        if(Math.hypot(a.x-c.x,a.y-c.y)>pitch*.25||Math.hypot(b.x-d.x,b.y-d.y)>pitch*.25) return;
        for(const member of [index,right,down,corner]) matched.add(member);
        cells.push(index);
      });
      const zones=Array(9).fill(0),rowBands=new Set(),columnBands=new Set(),contrasts=[];
      for(const index of matched) {
        const dot=dots[index];
        zones[Math.min(2,Math.floor(dot.y/height*3))*3+Math.min(2,Math.floor(dot.x/width*3))]++;
        rowBands.add(Math.floor(dot.y/pitch));columnBands.add(Math.floor(dot.x/pitch));contrasts.push(dot.contrast);
      }
      contrasts.sort((a,b)=>a-b);
      const coverage=zones.filter(count=>count>=12).length/9;
      const confidence=Math.min(1,matched.size/500)*coverage;
      const valid=cells.length>=160&&matched.size>=400&&coverage===1
        &&rowBands.size>=(landscape?21:32)&&columnBands.size>=(landscape?32:21);
      return {valid,dots:dots.length,matched:matched.size,cells:cells.length,rows:rowBands.size,columns:columnBands.size,
        coverage,confidence,contrast:contrasts.length?contrasts[Math.floor(contrasts.length*.25)]:0};
  }
  function inspectReadiness(source,detection) {
    if(!source?.width||!source?.height||!detection?.quad) return {valid:false,reason:'missing-page',grid:null};
    const raster=readSource(source);
    const sharpness=inspect(source,detection,raster);
    // The capture threshold is deliberately stricter than the legacy focus
    // diagnostic: a barely readable lattice is not a sufficiently sharp scan.
    // Sharp outer zones must not conceal a defocused patch on a curved page.
    // Every measured region must at least meet the established acutance floor.
    if(!sharpness.valid||sharpness.clarity<.42
      ||(sharpness.interiorMinimum!==null&&sharpness.interiorMinimum<.32)) {
      return {...sharpness,valid:false,reason:'focus-or-motion-blur',grid:null};
    }
    const grid=inspectGrid(source,detection,raster);
    return {...sharpness,valid:grid.valid,grid,reason:grid.valid?'sharp-grid':grid.reason||'grid-not-found'};
  }
  const moduleUrl=typeof document.currentScript?.src==='string'?document.currentScript.src:null;
  let readinessWorker=null,workerUnavailable=false,workerGeneration=0,nextRequest=0,pending=null,creating=false;
  const abortError=()=>new DOMException('Capture verification cancelled','AbortError');
  function disposeReadiness() {
    workerGeneration++;
    const previous=pending;pending=null;
    previous?.finish(null,abortError());
    readinessWorker?.terminate();readinessWorker=null;
  }
  async function inspectReadinessAsync(source,detection,{signal}={}) {
    if(signal?.aborted) throw abortError();
    if(!moduleUrl||workerUnavailable||typeof Worker!=='function'||typeof createImageBitmap!=='function'
      ||typeof OffscreenCanvas!=='function') return inspectReadiness(source,detection);
    // Only one candidate is inspected at a time. Dropping duplicate work is
    // preferable to building an unbounded queue of stale camera frames.
    if(pending||creating) return {valid:false,reason:'readiness-busy',grid:null};
    const generation=workerGeneration;
    let bitmap;
    creating=true;
    try {bitmap=await createImageBitmap(source);}
    catch(error) {
      if(signal?.aborted||generation!==workerGeneration) throw abortError();
      workerUnavailable=true;return inspectReadiness(source,detection);
    } finally {creating=false;}
    if(signal?.aborted||generation!==workerGeneration) {bitmap.close();throw abortError();}
    try {
      if(!readinessWorker) {
        const url=new URL('capture-quality-worker.js',moduleUrl);url.search=new URL(moduleUrl).search;
        readinessWorker=new Worker(url);
        readinessWorker.onmessage=event=>{
          const current=pending;
          if(!current||event.data.id!==current.id)return;
          current.finish(event.data.result,event.data.error?new Error(event.data.error):null);
        };
        readinessWorker.onerror=()=>pending?.finish(null,new Error('Capture verification worker unavailable'));
      }
      return await new Promise((resolve,reject)=>{
        const id=++nextRequest;
        let settled=false,timer;
        const cancel=()=>{
          readinessWorker?.terminate();readinessWorker=null;
          finish(null,abortError());
        };
        const finish=(result,error)=>{
          if(settled)return;settled=true;
          clearTimeout(timer);signal?.removeEventListener('abort',cancel);
          if(pending?.id===id)pending=null;
          if(error)reject(error);else resolve(result);
        };
        pending={id,finish};
        signal?.addEventListener('abort',cancel,{once:true});
        timer=setTimeout(()=>{
          readinessWorker?.terminate();readinessWorker=null;
          finish(null,new Error('Capture verification worker timed out'));
        },2000);
        try {readinessWorker.postMessage({id,bitmap,detection},[bitmap]);bitmap=null;}
        catch(error){finish(null,error);}
      });
    } catch(error) {
      if(error?.name==='AbortError'||signal?.aborted||generation!==workerGeneration)throw abortError();
      readinessWorker?.terminate();readinessWorker=null;workerUnavailable=true;
      return inspectReadiness(source,detection);
    } finally {bitmap?.close();}
  }
  SP.CaptureQuality = {inspect,inspectGrid,inspectReadiness,inspectReadinessAsync,disposeReadiness};
})();
