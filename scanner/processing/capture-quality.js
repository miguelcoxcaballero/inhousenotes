/* Measure the frozen photograph, independently of reduced live-view confidence.
 * Edge acutance distinguishes a narrow ink edge from a broad motion/focus smear;
 * raw Laplacian variance alone can be inflated by contrast and camera noise.
 */
(() => {
  const SP = window.ScannerPro = window.ScannerPro || {};
  function inspect(source, detection) {
    if (!detection?.quad || !source.width || !source.height) return { valid:false, reason:'missing-page' };
    const scale = Math.min(1, 1100 / Math.max(source.width, source.height));
    const canvas = document.createElement('canvas');
    canvas.width = Math.max(1, Math.round(source.width * scale));
    canvas.height = Math.max(1, Math.round(source.height * scale));
    try {
      const context = canvas.getContext('2d', {willReadFrequently:true});
      context.drawImage(source,0,0,canvas.width,canvas.height);
      const data = context.getImageData(0,0,canvas.width,canvas.height).data;
      const width=canvas.width,height=canvas.height;
      const quad=detection.quad.map(p=>({x:p.x*width,y:p.y*height}));
      const luma=(x,y)=>{const i=(Math.max(0,Math.min(height-1,y))*width+Math.max(0,Math.min(width-1,x)))*4;
        return data[i]*.2126+data[i+1]*.7152+data[i+2]*.0722;};
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
        edges,reason:clarity>=.32?'sharp-page':'focus-or-motion-blur'};
    } finally {canvas.width=canvas.height=0;}
  }
  SP.CaptureQuality = {inspect};
})();
