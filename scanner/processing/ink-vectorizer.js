/* Recover pen centre-lines from already corrected, template-free scan ink. */
(function (root) {
  'use strict';
  const SP = root.ScannerPro = root.ScannerPro || {};
  const palette = root.InhouseInkPalette;
  const yieldTask = () => new Promise(resolve => setTimeout(resolve, 0));
  function check(signal) { if (signal?.aborted) throw new DOMException('Scan cancelled', 'AbortError'); }
  function colour(r, g, b) {
    const hi=Math.max(r,g,b), lo=Math.min(r,g,b);
    if (hi-lo<36 || hi-lo<hi*.2) return 1;
    const scale=225/(hi-lo), c=[(r-lo)*scale,(g-lo)*scale,(b-lo)*scale];
    const targets=[[0,0,0],[0,49,225],[225,0,0]];
    let best=Infinity, label=1;
    targets.forEach((target,index)=>{const error=target.reduce((sum,value,k)=>sum+(value-c[k])**2,0);
      if(error<best){best=error;label=index+1;}});
    return label;
  }
  // Same bounded algorithm for browsers that cannot instantiate WebAssembly.
  // Yield between passes so cancellation and the camera controls still work.
  async function fallback(rgba, width, height, signal) {
    const n=width*height, mask=new Uint8Array(n), radii=new Uint16Array(n);
    const offsets=[-width,1-width,1,1+width,width,width-1,-1,-width-1];
    for(let y=1;y<height-1;y++)for(let x=1;x<width-1;x++){
      const i=y*width+x;
      if(rgba[i*4+3]>=16)mask[i]=colour(rgba[i*4],rgba[i*4+1],rgba[i*4+2]);
    }
    await yieldTask();check(signal);
    const initial=mask.slice();
    for(let y=1;y<height-1;y++)for(let x=1;x<width-1;x++){
      const i=y*width+x;if(!initial[i])continue;
      const alpha=rgba[i*4+3];let peak=alpha,bestAlpha=0,bestDistance=0,bestLabel=0;
      for(let yy=Math.max(0,y-3);yy<=Math.min(height-1,y+3);yy++)for(let xx=Math.max(0,x-3);xx<=Math.min(width-1,x+3);xx++){
        const j=yy*width+xx,a=rgba[j*4+3],distance=(xx-x)**2+(yy-y)**2;peak=Math.max(peak,a);
        if(initial[j]>1&&a>=48&&a*(bestDistance+1)>bestAlpha*(distance+1)){
          bestAlpha=a;bestDistance=distance;bestLabel=initial[j];}
      }
      if(alpha*100<peak*35)mask[i]=0;
      else if(initial[i]===1&&bestLabel>1&&alpha*5<bestAlpha*4)mask[i]=bestLabel;
    }
    await yieldTask();check(signal);
    const seen=new Uint8Array(n), queue=[],anchors=[];let active=[];
    for(let start=0;start<n;start++){
      if(!mask[start]||seen[start])continue;
      queue.length=0;queue.push(start);seen[start]=1;
      const label=mask[start];let head=0,peak=0;
      while(head<queue.length){const i=queue[head++];peak=Math.max(peak,rgba[i*4+3]);
        for(const delta of offsets){const j=i+delta;if(!seen[j]&&mask[j]===label){seen[j]=1;queue.push(j);}}}
      if(peak<24||(queue.length===1&&peak<90)){for(const i of queue)mask[i]=0;}
      else {
        for(const i of queue){active.push(i);radii[i]=12000;}
        if(queue.length<=32){
          const cx=queue.reduce((s,i)=>s+i%width,0)/queue.length,cy=queue.reduce((s,i)=>s+Math.floor(i/width),0)/queue.length;
          let best=Infinity,index=queue[0];
          for(const i of queue){const d=(i%width-cx)**2+(Math.floor(i/width)-cy)**2;if(d<best){best=d;index=i;}}
          anchors.push(index);
        }
      }
    }
    seen.fill(0);for(const i of anchors)seen[i]=1;
    await yieldTask();check(signal);
    for(let i=0;i<n;i++)if(mask[i])for(const [delta,cost] of [[-1,3],[-width,3],[-width-1,4],[1-width,4]]){
      const j=i+delta;radii[i]=Math.min(radii[i],mask[j]===mask[i]?radii[j]+cost:cost);}
    for(let i=n-1;i>=0;i--)if(mask[i])for(const [delta,cost] of [[1,3],[width,3],[width+1,4],[width-1,4]]){
      const j=i+delta;radii[i]=Math.min(radii[i],mask[j]===mask[i]?radii[j]+cost:cost);}
    const p=new Uint8Array(8),remove=[];let work=0;
    for(;;){let changed=false;
      for(let phase=0;phase<2;phase++){
        work+=active.length;if(work>40000000)throw new Error('Ink could not be separated. Try a clearer photo.');
        remove.length=0;
        for(const i of active){const label=mask[i];if(!label||seen[i])continue;
          let count=0,changes=0;
          for(let k=0;k<8;k++){p[k]=+(mask[i+offsets[k]]===label);count+=p[k];}
          if(count<2||count>6)continue;
          for(let k=0;k<8;k++)changes+=+(p[k]===0&&p[(k+1)%8]===1);
          if(changes!==1)continue;
          // Explicit products keep the topology test identical to Rust.
          if(phase===0?(p[0]*p[2]*p[4]===0&&p[2]*p[4]*p[6]===0):(p[0]*p[2]*p[6]===0&&p[0]*p[4]*p[6]===0))remove.push(i);
        }
        if(remove.length){changed=true;for(const i of remove)mask[i]=0;}
      }
      if(!changed)break;
      active=active.filter(i=>mask[i]);await yieldTask();check(signal);
    }
    return {mask,radii,backend:'javascript'};
  }
  function simplify(points, tolerance=.3) {
    if(points.length<3)return points;
    const keep=new Uint8Array(points.length);keep[0]=keep[points.length-1]=1;
    const stack=[[0,points.length-1]];
    while(stack.length){const [a,b]=stack.pop(),start=points[a],end=points[b];
      const dx=end.x-start.x,dy=end.y-start.y,length=dx*dx+dy*dy;
      let best=tolerance*tolerance,index=-1;
      for(let k=a+1;k<b;k++){const p=points[k],t=length?Math.max(0,Math.min(1,((p.x-start.x)*dx+(p.y-start.y)*dy)/length)):0;
        const d=(p.x-start.x-t*dx)**2+(p.y-start.y-t*dy)**2;
        if(d>best){best=d;index=k;}}
      if(index>=0){keep[index]=1;stack.push([a,index],[index,b]);}
    }
    return points.filter((_,index)=>keep[index]);
  }
  async function trace(axes,width,height,signal) {
    const {mask,radii}=axes,used=new Uint8Array(mask.length),strokes=[];
    const offsets=[-width,1-width,1,1+width,width,width-1,-1,-width-1];
    const nodes=[];let totalPoints=0;
    const edges=i=>{let bits=0;for(let k=0;k<8;k++){
      if(mask[i+offsets[k]]!==mask[i])continue;
      // A diagonal beside a cardinal edge is the same junction, not a tiny
      // triangular loop. True diagonal handwriting remains connected.
      if(k%2&&(mask[i+offsets[(k+7)%8]]===mask[i]||mask[i+offsets[(k+1)%8]]===mask[i]))continue;
      bits|=1<<k;
    }return bits;};
    const count=bits=>{let n=0;while(bits){bits&=bits-1;n++;}return n;};
    for(let i=0;i<mask.length;i++)if(mask[i])nodes.push(i);
    const emit=path=>{
      // Split only sustained width changes; never force a thick crossing's
      // local radius onto the entire handwritten line.
      const chunks=[];let begin=0;
      for(let k=3;k<path.length-3;k++){
        const base=radii[path[begin+Math.floor((k-begin)/2)]],next=radii[path[k+2]];
        if(Math.abs(base-next)>=6&&Math.max(base,next)>Math.min(base,next)*1.8&&k-begin>=12){chunks.push(path.slice(begin,k+1));begin=k;}
      }
      chunks.push(path.slice(begin));
      for(const chunk of chunks){
        const widths=chunk.map(i=>radii[i]).sort((a,b)=>a-b);
        const penWidth=Math.max(1,widths[Math.floor(widths.length/2)]*2/3-1);
        const reduced=simplify(chunk.map(i=>({x:i%width+.5,y:Math.floor(i/width)+.5})));
        const points=[reduced[0]],step=Math.max(1,width/397);
        // Native partial erasing splits sampled pens. Keep at most two logical
        // editor pixels between points, even on a perfectly straight line.
        for(let j=1;j<reduced.length;j++){
          const a=reduced[j-1],b=reduced[j],n=Math.ceil(Math.hypot(b.x-a.x,b.y-a.y)/step);
          for(let k=1;k<=n;k++)points.push({x:a.x+(b.x-a.x)*k/n,y:a.y+(b.y-a.y)*k/n});
        }
        if(points.length===1)points.push({x:points[0].x+.001,y:points[0].y});
        totalPoints+=points.length;
        if(strokes.length>=60000||totalPoints>500000)throw new Error('This scan contains too much detail. Try a clearer photo.');
        strokes.push({tool:'pen',color:palette[mask[chunk[0]]-1],width:penWidth,points});
      }
    };
    const walk=(start,direction)=>{
      const path=[start];let i=start,k=direction;
      for(;;){used[i]|=1<<k;const next=i+offsets[k];used[next]|=1<<((k+4)%8);path.push(next);
        if(next===start)break;
        const adjacent=edges(next),available=adjacent&~used[next];
        if(count(adjacent)!==2||!available)break;
        i=next;k=0;while(!(available&(1<<k)))k++;
      }
      emit(path);
    };
    for(const loops of [false,true]){
      let work=0;
      for(const i of nodes){const adjacent=edges(i);
        if(!loops&&count(adjacent)===2)continue;
        if(!loops&&!adjacent){emit([i]);continue;}
        for(let k=0;k<8;k++)if((adjacent&~used[i])&(1<<k))walk(i,k);
        if(++work%2000===0){await yieldTask();check(signal);}
      }
    }
    return strokes;
  }
  async function fromPixels(pixels,options={}) {
    const {width,height,data}=pixels;
    if(!palette||!Number.isInteger(width)||!Number.isInteger(height)||width<2||height<2
      ||width>4096||height>4096||width*height>12000000||data?.length!==width*height*4)throw new Error('Invalid scan ink dimensions');
    check(options.signal);await root.InhouseRust?.ready;check(options.signal);
    const axes=(!options.forceJavaScript&&root.InhouseRust?.inkAxes?.(data,width,height))
      ||await fallback(data,width,height,options.signal);
    if(axes.rejected)throw new Error('Ink could not be separated. Try a clearer photo.');
    check(options.signal);
    const strokes=await trace(axes,width,height,options.signal);
    return {version:1,width,height,strokes,backend:axes.backend};
  }
  // Exactly the editor's opaque pen path, also used for the scan preview.
  function render(context,ink) {
    context.save();context.globalAlpha=1;context.globalCompositeOperation='source-over';
    context.lineCap=context.lineJoin='round';
    for(const stroke of ink.strokes){const p=stroke.points;
      context.strokeStyle=stroke.color;context.lineWidth=stroke.width;context.beginPath();context.moveTo(p[0].x,p[0].y);
      if(p.length===2)context.lineTo(p[1].x,p[1].y);
      else {context.lineTo((p[0].x+p[1].x)/2,(p[0].y+p[1].y)/2);
        for(let i=1;i<p.length-1;i++)context.quadraticCurveTo(p[i].x,p[i].y,(p[i].x+p[i+1].x)/2,(p[i].y+p[i+1].y)/2);
        context.lineTo(p[p.length-1].x,p[p.length-1].y);}
      context.stroke();
    }
    context.restore();
  }
  SP.InkVectorizer=Object.freeze({fromPixels,render});
})(globalThis);
