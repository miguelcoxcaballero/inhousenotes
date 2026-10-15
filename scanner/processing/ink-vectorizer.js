/* Recover pen centre-lines from already corrected, template-free scan ink. */
(function (root) {
  'use strict';
  const SP = root.ScannerPro = root.ScannerPro || {};
  const palette = root.InhouseInkPalette;
  const yieldTask = () => new Promise(resolve => setTimeout(resolve, 0));
  function check(signal) { if (signal?.aborted) throw new DOMException('Scan cancelled', 'AbortError'); }
  function colour(r, g, b, alpha) {
    const hi=Math.max(r,g,b), lo=Math.min(r,g,b);
    if (hi-lo<18 || hi-lo<hi*.2 || (hi-lo)*alpha<255*12) return 1;
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
      if(rgba[i*4+3]>=16)mask[i]=colour(rgba[i*4],rgba[i*4+1],rgba[i*4+2],rgba[i*4+3]);
    }
    await yieldTask();check(signal);
    const initial=mask.slice();
    for(let y=1;y<height-1;y++)for(let x=1;x<width-1;x++){
      const i=y*width+x;if(!initial[i])continue;
      const alpha=rgba[i*4+3];let peak=alpha;
      for(let yy=Math.max(0,y-3);yy<=Math.min(height-1,y+3);yy++)for(let xx=Math.max(0,x-3);xx<=Math.min(width-1,x+3);xx++){
        peak=Math.max(peak,rgba[(yy*width+xx)*4+3]);
      }
      if(alpha*100<peak*35)mask[i]=0;
    }
    await yieldTask();check(signal);
    const seen=new Uint8Array(n), queue=[],anchors=[];let active=[];
    for(let start=0;start<n;start++){
      if(!mask[start]||seen[start])continue;
      queue.length=0;queue.push(start);seen[start]=1;
      let head=0,peak=0;
      while(head<queue.length){const i=queue[head++];peak=Math.max(peak,rgba[i*4+3]);
        for(const delta of offsets){const j=i+delta;if(!seen[j]&&mask[j]!==0){seen[j]=1;queue.push(j);}}}
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
      const j=i+delta;radii[i]=Math.min(radii[i],mask[j]!==0?radii[j]+cost:cost);}
    for(let i=n-1;i>=0;i--)if(mask[i])for(const [delta,cost] of [[1,3],[width,3],[width+1,4],[width-1,4]]){
      const j=i+delta;radii[i]=Math.min(radii[i],mask[j]!==0?radii[j]+cost:cost);}
    const p=new Uint8Array(8),remove=[];let work=0;
    for(;;){let changed=false;
      for(let phase=0;phase<2;phase++){
        work+=active.length;if(work>40000000)throw new Error('Ink could not be separated. Try a clearer photo.');
        remove.length=0;
        for(const i of active){const label=mask[i];if(!label||seen[i])continue;
          let count=0,changes=0;
          for(let k=0;k<8;k++){p[k]=+(mask[i+offsets[k]]!==0);count+=p[k];}
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
    for(const i of active){
      if(!mask[i])continue;
      const x=i%width,y=Math.floor(i/width),radius=Math.max(2,Math.min(8,Math.floor((radii[i]+2)/3))),votes=[0,0,0,0];
      for(let yy=Math.max(0,y-radius);yy<=Math.min(height-1,y+radius);yy++)for(let xx=Math.max(0,x-radius);xx<=Math.min(width-1,x+radius);xx++){
        const j=yy*width+xx,label=initial[j],d2=(xx-x)**2+(yy-y)**2;
        if(!label||d2>radius*radius)continue;
        const alpha=rgba[j*4+3];votes[label]+=Math.floor(alpha*alpha*256/(1+d2));
      }
      const total=votes[1]+votes[2]+votes[3],label=votes[2]>votes[3]?2:3;
      mask[i]=votes[label]*100>=total*22&&votes[label]*2>=votes[5-label]*3?label:1;
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
  // Smooth raster stair-steps before simplifying/resampling the native pen.
  // The radius is in page units, so a 2x scan gets the same light treatment.
  // Endpoints and sharp intentional turns stay put; loops wrap at their seam.
  function smoothAxis(points, scale, penWidth) {
    if(points.length<5)return points;
    const closed=points[0].x===points.at(-1).x&&points[0].y===points.at(-1).y;
    const count=points.length-(closed?1:0);
    const sigma=Math.max(.65*scale,Math.min(scale,penWidth*.4));
    const radius=Math.ceil(sigma*2.5),cornerSpan=Math.max(2,Math.round(scale*3));
    const maximumMove=Math.min(.55*scale,penWidth*.3);
    const pointAt=i=>points[closed?(i%count+count)%count:Math.max(0,Math.min(count-1,i))];
    const smoothed=points.slice(0,count).map((p,i)=>{
      if(!closed&&(i===0||i===count-1))return p;
      const before=pointAt(i-cornerSpan),after=pointAt(i+cornerSpan);
      const ax=p.x-before.x,ay=p.y-before.y,bx=after.x-p.x,by=after.y-p.y;
      const lengths=Math.hypot(ax,ay)*Math.hypot(bx,by);
      // Keep corners of letters/checkmarks, including tight reversals.
      if(lengths&&ax*bx+ay*by<lengths*.5)return p;
      let x=p.x,y=p.y,weight=1;
      for(const sign of [-1,1]){
        let distance=0,previous=p;
        for(let k=1;k<=radius;k++){
          if(!closed&&(i+sign*k<0||i+sign*k>=count))break;
          const q=pointAt(i+sign*k);
          distance+=Math.hypot(q.x-previous.x,q.y-previous.y);previous=q;
          const w=Math.exp(-distance*distance/(2*sigma*sigma));
          x+=q.x*w;y+=q.y*w;weight+=w;
        }
      }
      const dx=x/weight-p.x,dy=y/weight-p.y,distance=Math.hypot(dx,dy);
      const blend=distance?Math.min(1,maximumMove/distance):0;
      return {x:p.x+dx*blend,y:p.y+dy*blend};
    });
    if(closed)smoothed.push({...smoothed[0]});
    return smoothed;
  }

  // Join only opposite branches at a junction. A blue line crossing a black
  // line remains two colour decisions; a shaded half of the same line can
  // borrow reliable colour from its continuation instead of becoming black.
  function coherentPathColours(paths, mask, width, height, rgba, radii) {
    const scale=width/794,parents=paths.map((_,i)=>i),ends=new Map();
    const find=i=>{while(parents[i]!==i){parents[i]=parents[parents[i]];i=parents[i];}return i;};
    const tangent=(path,atStart)=>{
      const start=atStart?0:path.length-1,step=atStart?1:-1,p=path[start];
      let q=p,distance=0,previous=p;
      for(let k=start+step;k>=0&&k<path.length;k+=step){
        q=path[k];distance+=Math.hypot(q%width-previous%width,Math.floor(q/width)-Math.floor(previous/width));
        previous=q;if(distance>=Math.max(3,scale*6))break;
      }
      const dx=q%width-p%width,dy=Math.floor(q/width)-Math.floor(p/width),length=Math.hypot(dx,dy);
      return {x:length?dx/length:0,y:length?dy/length:0};
    };
    paths.forEach((path,index)=>{
      if(path.length<2)return;
      for(const atStart of [true,false]){
        const node=atStart?path[0]:path.at(-1),list=ends.get(node)||[];
        list.push({index,...tangent(path,atStart)});ends.set(node,list);
      }
    });
    for(const branches of ends.values()){
      if(branches.length<2)continue;
      const best=branches.map((a,i)=>{
        let chosen=-1,dot=-.82;
        branches.forEach((b,j)=>{
          const score=a.x*b.x+a.y*b.y;
          if(i!==j&&a.index!==b.index&&score<dot){dot=score;chosen=j;}
        });return chosen;
      });
      best.forEach((j,i)=>{if(j>i&&best[j]===i)parents[find(branches[j].index)]=find(branches[i].index);});
    }
    const groups=new Map();
    paths.forEach((path,index)=>{
      const id=find(index),group=groups.get(id)||{votes:[0,0,0,0],longest:[0,0,0,0],strong:[0,0,0,0]};
      let previous=0,run=0;
      for(const node of path){
        const label=mask[node];group.votes[label]++;
        if(label>1){
          // Require pigment in the covered cross-section, not a tiny cold
          // bias which hue normalization can turn into saturated blue. Vote
          // across the axis to retain a red/blue pen with a dark centre.
          const x=node%width,y=Math.floor(node/width),reach=Math.min(4,Math.max(2,Math.ceil(radii[node]/3)));
          let r=0,g=0,b=0,alpha=0,total=0;
          for(let yy=Math.max(0,y-reach);yy<=Math.min(height-1,y+reach);yy++)for(let xx=Math.max(0,x-reach);xx<=Math.min(width-1,x+reach);xx++){
            const at=(yy*width+xx)*4,a=rgba[at+3],weight=a*a/(1+(xx-x)**2+(yy-y)**2);
            r+=rgba[at]*weight;g+=rgba[at+1]*weight;b+=rgba[at+2]*weight;alpha+=a*weight;total+=weight;
          }
          if(total){
            r/=total;g/=total;b/=total;alpha/=total;
            const hi=Math.max(r,g,b),chroma=hi-Math.min(r,g,b);
            if(chroma>=24 && (chroma>=45 || chroma>=hi*.28) && chroma*alpha>=255*14
              &&colour(r,g,b,alpha)===label)group.strong[label]++;
          }
        }
        run=previous===label?run+1:1;previous=label;
        group.longest[label]=Math.max(group.longest[label],run);
      }
      groups.set(id,group);
    });
    const labels=new Map();
    for(const [id,{votes,longest,strong}] of groups){
      const total=votes[1]+votes[2]+votes[3];
      const supported=label=>votes[label]>=total*.24
        &&longest[label]>=Math.min(total,Math.max(3,scale*3)) && strong[label]>=Math.max(1,total*.08);
      const blue=supported(2),red=supported(3);
      // Sustained *different hues* can still be separate inks touching end to
      // end. Neutral camera patches alone never split a coherent coloured pen.
      labels.set(id,blue&&red?0:blue?2:red?3:1);
    }
    return {colours:paths.map((_,i)=>labels.get(find(i))),groups:paths.map((_,i)=>find(i))};
  }
  async function trace(axes,width,height,signal,rgba) {
    const {mask,radii}=axes,used=new Uint8Array(mask.length),strokes=[];
    const offsets=[-width,1-width,1,1+width,width,width-1,-1,-width-1];
    const nodes=[],paths=[];let totalPoints=0;
    const edges=i=>{let bits=0;for(let k=0;k<8;k++){
      if(!mask[i+offsets[k]])continue;
      // A diagonal beside a cardinal edge is the same junction, not a tiny
      // triangular loop. True diagonal handwriting remains connected.
      if(k%2&&(mask[i+offsets[(k+7)%8]]||mask[i+offsets[(k+1)%8]]))continue;
      bits|=1<<k;
    }return bits;};
    const count=bits=>{let n=0;while(bits){bits&=bits-1;n++;}return n;};
    for(let i=0;i<mask.length;i++)if(mask[i])nodes.push(i);
    const defaults=root.InhouseInkDefaults;
    const standard=defaults.width*Math.sqrt(width/defaults.pageWidth*height/defaults.pageHeight);
    const measureWidths=chunk=>chunk.map((i,k)=>{
          const a=chunk[Math.max(0,k-3)],b=chunk[Math.min(chunk.length-1,k+3)];
          const dx=b%width-a%width,dy=Math.floor(b/width)-Math.floor(a/width),length=Math.hypot(dx,dy);
          if(!length)return Math.max(1,radii[i]*2/3-1);
          const nx=-dy/length,ny=dx/length,x=i%width+.5,y=Math.floor(i/width)+.5;
          let coverage=rgba[i*4+3],peak=coverage;
          // Very short junction branches can point across the main axis.
          // Limit the cross-section by the inscribed radius, never by the
          // length of a coincident neighbouring stroke.
          const limit=Math.min(32,radii[i]/3+1);
          for(const sign of [-1,1])for(let d=.5;d<=limit;d+=.5){
            const xx=Math.floor(x+nx*d*sign),yy=Math.floor(y+ny*d*sign);
            if(xx<0||xx>=width||yy<0||yy>=height||!radii[yy*width+xx])break;
            const alpha=rgba[(yy*width+xx)*4+3];coverage+=alpha;peak=Math.max(peak,alpha);
          }
          return Math.min(radii[i]*2/3+1,coverage/Math.max(1,peak)*.5);
        });
    const emit=(path,label,fixedWidth)=>{
      // Split only sustained width changes; never force a thick crossing's
      // local radius onto the entire handwritten line.
      const chunks=[];let begin=0;
      for(let k=3;!fixedWidth&&k<path.length-3;k++){
        const base=radii[path[begin+Math.floor((k-begin)/2)]],next=radii[path[k+2]];
        if(Math.abs(base-next)>=6&&Math.max(base,next)>Math.min(base,next)*1.8&&k-begin>=12){chunks.push(path.slice(begin,k+1));begin=k;}
      }
      chunks.push(path.slice(begin));
      for(const chunk of chunks){
        // Measure both sides across the axis. A one-sided distance transform
        // underestimates every even-width pen by one pixel (2px became 1px).
        const widths=fixedWidth ? null : measureWidths(chunk).sort((a,b)=>a-b);
        const measured=fixedWidth || Math.max(1,widths[Math.floor(widths.length/2)]);
        const penWidth=fixedWidth || (measured>=standard*.75&&measured<=standard*1.6?standard:measured);
        const reduced=simplify(smoothAxis(chunk.map(i=>({x:i%width+.5,y:Math.floor(i/width)+.5})),width/794,penWidth),.18*width/794);
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
        strokes.push({tool:'pen',color:palette[label-1],width:penWidth,points});
      }
    };
    const emitColours=(path,coherentColour,fixedWidth)=>{
      if(coherentColour){emit(path,coherentColour,fixedWidth);return;}
      // Only genuinely mixed red/blue paths need local colour runs.
      const radius=Math.max(2,Math.round(width/794*2));
      const labels=path.map((_,index)=>{
        const votes=[0,0,0,0];
        for(let k=Math.max(0,index-radius);k<=Math.min(path.length-1,index+radius);k++)votes[mask[path[k]]]++;
        let label=mask[path[index]];
        for(let k=1;k<=3;k++)if(votes[k]>votes[label])label=k;
        return label;
      });
      const runs=[];
      for(let start=0;start<path.length;){let end=start+1;while(end<path.length&&labels[end]===labels[start])end++;
        runs.push({start,end,label:labels[start]});start=end;}
      const minimum=Math.max(4,Math.round(width/794*4));
      for(let k=0;k<runs.length;k++){
        const run=runs[k];if(run.end-run.start>=minimum)continue;
        const left=runs[k-1],right=runs[k+1];
        if(left&&right&&left.label===right.label)run.label=left.label;
        else if(left&&left.end-left.start>=minimum&&(!right||right.end-right.start<minimum))run.label=left.label;
        else if(right&&right.end-right.start>=minimum&&(!left||left.end-left.start<minimum))run.label=right.label;
      }
      for(let k=0;k<runs.length;k++){
        const run=runs[k];let end=run.end;
        while(k+1<runs.length&&runs[k+1].label===run.label)end=runs[++k].end;
        emit(path.slice(run.start,Math.min(path.length,end+1)),run.label,fixedWidth);
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
      if(paths.length>=60000)throw new Error('This scan contains too much detail. Try a clearer photo.');
      paths.push(path);
    };
    for(const loops of [false,true]){
      let work=0;
      for(const i of nodes){const adjacent=edges(i);
        if(!loops&&count(adjacent)===2)continue;
        if(!loops&&!adjacent){
          if(paths.length>=60000)throw new Error('This scan contains too much detail. Try a clearer photo.');
          paths.push([i]);continue;
        }
        for(let k=0;k<8;k++)if((adjacent&~used[i])&(1<<k))walk(i,k);
        if(++work%2000===0){await yieldTask();check(signal);}
      }
    }
    check(signal);
    const {colours,groups}=coherentPathColours(paths,mask,width,height,rgba,radii);
    const widthsByGroup=new Map();
    for(let i=0;i<paths.length;i++){
      const widths=widthsByGroup.get(groups[i])||[];
      for(const value of measureWidths(paths[i]))widths.push(value);
      widthsByGroup.set(groups[i],widths);
      if(i%500===499){await yieldTask();check(signal);}
    }
    const standardGroups=new Set();
    for(const [id,widths] of widthsByGroup){
      widths.sort((a,b)=>a-b);
      const median=widths[Math.floor(widths.length/2)];
      // Decide for the whole continuous pen, then smooth that same axis.
      // Shaded thin patches and junctions cannot create alternating widths.
      const close=widths.filter(w=>w>=standard*.65&&w<=standard*1.65).length;
      if(median>=standard*.75&&median<=standard*1.6&&close>=widths.length*.72)standardGroups.add(id);
    }
    for(let i=0;i<paths.length;i++){
      emitColours(paths[i],colours[i],standardGroups.has(groups[i])?standard:0);
      if(i%500===499){await yieldTask();check(signal);}
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
    const strokes=await trace(axes,width,height,options.signal,data);
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
