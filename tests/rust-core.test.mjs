import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import vm from 'node:vm';
const bytes=await readFile(new URL('../wasm/inhouse_notes_core.wasm',import.meta.url));
const scope={WebAssembly,Uint8Array,Uint16Array,Float32Array,URL,location:new URL('https://notes.test/'),fetch:async()=>new Response(bytes),console,AbortSignal,AbortController,setTimeout,clearTimeout};
vm.createContext(scope);vm.runInContext(await readFile(new URL('../rust-core-v5.js',import.meta.url),'utf8'),scope);
await scope.InhouseRust.ready;
test('Rust core loads with no host imports',async()=>{
    assert.equal(scope.InhouseRust.available(),true);
    assert.deepEqual(WebAssembly.Module.imports(await WebAssembly.compile(bytes)),[]);
});
test('lossless RGB and alpha split preserves all byte values',()=>{
    const rgba=Uint8Array.from({length:4096},(_,i)=>(i*17+i%11)%256);
    const result=scope.InhouseRust.splitRgba(rgba);
    assert.equal(result.transparent,true);
    for(let i=0;i<rgba.length/4;i++){
        assert.deepEqual([...result.rgb.slice(i*3,i*3+3)],[...rgba.slice(i*4,i*4+3)]);
        assert.equal(result.alpha[i],rgba[i*4+3]);
    }
    for(let i=3;i<rgba.length;i+=4)rgba[i]=255;
    assert.equal(scope.InhouseRust.splitRgba(rgba).alpha,null);
});
test('mesh sampling matches clamped canvas bytes and is reusable after disposal',()=>{
    const width=80,height=93;
    const input={width,height,data:Uint8Array.from({length:width*height*4},(_,i)=>(i*19+i%23)%256)};
    for(let run=0;run<8;run++){
        const w=97,h=101,pixels=new Uint8ClampedArray(w*h*4);pixels.fill(255);
        const warp=scope.InhouseRust.warp(input,w,h);
        warp.cell(0,w,0,h,{x:0,y:0},{x:width,y:0},{x:0,y:height},{x:width,y:height});
        warp.copyTo(pixels);warp.dispose();warp.dispose();
        for(let y=0;y<h;y++)for(let x=0;x<w;x++){
            const sx=(x+.5)*width/w-.5,sy=(y+.5)*height/h-.5,index=(y*w+x)*4;
            if(sx<0||sy<0||sx>=width-1||sy>=height-1){assert.equal(pixels[index],255);continue;}
            const ix=Math.floor(sx),iy=Math.floor(sy),fx=sx-ix,fy=sy-iy;
            for(let c=0;c<3;c++){
                const a=(iy*width+ix)*4+c,b=a+width*4;
                const value=(input.data[a]*(1-fx)+input.data[a+4]*fx)*(1-fy)+(input.data[b]*(1-fx)+input.data[b+4]*fx)*fy;
                // Scalar reference computes x directly; production advances dx, allowing one rounding ULP.
                assert.ok(Math.abs(pixels[index+c]-new Uint8ClampedArray([value])[0])<=1);
            }
        }
    }
});

test('Rust ink separation reconstructs coloured and neutral photographs over white',()=>{
    const data=Uint8ClampedArray.from([255,255,255,255, 25,66,180,255, 193,193,193,255, 254,216,190,255, 0,0,0,255]);
    const original=data.slice();
    assert.equal(scope.InhouseRust.separateInk(data),true);
    assert.equal(data[3],0);
    for(let i=0;i<data.length;i+=4)for(let c=0;c<3;c++){
        const composited=data[i+c]*data[i+3]/255+255-data[i+3];
        assert.ok(Math.abs(composited-original[i+c])<=1);
    }
});
test('Rust scan buffers reject inconsistent paper fields before allocating',()=>{
    assert.equal(scope.InhouseRust.scanColors(new Uint8Array(16),new Uint8Array(8),2,2,new Float32Array(4),null,2,2,new Float64Array(61)),null);
    assert.equal(scope.InhouseRust.scanColors(new Uint8Array(0),new Uint8Array(0),0,2,new Float32Array(4),null,2,2,new Float64Array(61)),null);
    assert.equal(scope.InhouseRust.scanColors(new Uint8Array(16),new Uint8Array(16),2,2,new Float32Array(3),null,2,2,new Float64Array(61)),null);
    assert.equal(scope.InhouseRust.scanColors(new Uint8Array(16),new Uint8Array(16),2,2,new Float32Array(4),new Uint8Array(3),2,2,new Float64Array(61)),null);
});

test('Rust surface contexts reject invalid correspondences and release independently',()=>{
    const p=new Float64Array(16);p[5]=Math.log(1000);p[6]=Math.log(1000);
    assert.equal(scope.InhouseRust.surfaceFit(p,[{u:NaN,v:.5,x:100,y:150}],200,300,null),null);
    const points=[{u:.2,v:.3,x:37,y:90.6},{u:.8,v:.7,x:163,y:209.4}];
    const fit=scope.InhouseRust.surfaceFit(p,points,200,300,null);
    assert.ok(fit);
    for(let i=0;i<3;i++)fit.step();
    assert.ok(Number.isFinite(fit.result().cost));
    assert.equal(fit.result().p.length,16);
    fit.dispose();fit.dispose();
    assert.equal(fit.step(),false);
    assert.equal(fit.result(),null);
    const next=scope.InhouseRust.surfaceFit(p,points,200,300,null);
    assert.ok(next);next.dispose();
});

test('Rust scan axes preserve faint pen cores in the discrete palette without mutating inputs',()=>{
    const width=120,height=80,rgba=new Uint8Array(width*height*4);
    for(const [row,colour,alpha] of [[15,[0,0,0],26],[35,[0,40,225],180],[55,[220,0,0],200]]){
        for(let y=row-2;y<=row+2;y++)for(let x=10;x<110;x++)rgba.set([...colour,alpha],(y*width+x)*4);
    }
    const before=rgba.slice();
    for(let run=0;run<6;run++){
        const axes=scope.InhouseRust.inkAxes(rgba,width,height);
        assert.equal(axes.backend,'rust');
        for(const [y,label] of [[15,1],[35,2],[55,3]]){
            assert.equal(axes.mask[y*width+50],label);
            assert.ok(axes.radii[y*width+50]>=6);
        }
        assert.ok([...axes.mask].every(value=>value<=3));
        assert.deepEqual(rgba,before);
    }
    const blank=scope.InhouseRust.inkAxes(new Uint8Array(width*height*4),width,height);
    assert.ok(blank.mask.every(value=>value===0));assert.ok(blank.radii.every(value=>value===0));
});

test('Rust scan axes reject malformed and unbounded image allocations',()=>{
    for(const [data,width,height] of [[new Uint8Array(16),3,3],[new Uint8Array(16),Infinity,2],
        [new Uint8Array(16),2.5,2],[new Uint8Array(16),1,4],[new Uint8Array(4),4097,4097]]){
        assert.equal(scope.InhouseRust.inkAxes(data,width,height),null);
    }
});


test('local paper balance removes a measured cast without changing coverage or neighbouring pigment',()=>{
    const width=4,height=2,white=Float32Array.from([1,.5,.7,1, 1,2,1.4,1]);
    const input=Uint8ClampedArray.from([80,80,80,255, 40,80,160,255, 56,80,112,17, 0,40,200,255,
      40,40,40,255, 20,40,80,255, 28,40,56,255, 200,20,0,255]);
    const before=input.slice(),out=input.slice();
    const job=scope.InhouseRust.balancePaper(input,width,height,white,4,1);
    assert.ok(job); job.rows(0,1); job.rows(1,2); job.copyTo(out); job.dispose(); job.dispose();
    assert.deepEqual(input,before);
    for(let y=0;y<2;y++)for(let x=0;x<3;x++){
      const i=(y*width+x)*4;
      assert.ok(Math.max(...out.slice(i,i+3))-Math.min(...out.slice(i,i+3))<=1);
      assert.equal(out[i+3],input[i+3]);
    }
    assert.deepEqual([...out.slice(12,16)],[...input.slice(12,16)]);
    assert.deepEqual([...out.slice(28,32)],[...input.slice(28,32)]);
    assert.equal(scope.InhouseRust.balancePaper(input,width,height,new Float32Array(4),4,1),null);
});

test('a cached core without measured coverage or neutral calibration requests the safe JS fallback',async()=>{
    let allocations=0;
    const exports={core_abi_version:()=>1,alloc(){allocations++;throw new Error('Legacy core cannot measure ink');}};
    exports.correct_scan_rows=function(a,b,c,d,e,f,g,h,i,j,k){};
    exports.ink_axes=function(a,b,c,d,e){};
    exports.balance_paper_rows=function(a,b,c,d,e,f,g,h){};
    const legacy={...scope,WebAssembly:{instantiate:async()=>({instance:{exports}})}};
    vm.createContext(legacy);vm.runInContext(await readFile(new URL('../rust-core-v5.js',import.meta.url),'utf8'),legacy);
    await legacy.InhouseRust.ready;
    const pixels=new Uint8Array(16),coverage=new Uint8Array(4);
    assert.equal(legacy.InhouseRust.scanColors(pixels,pixels,2,2,new Float32Array(4),null,2,2,new Float64Array(61),coverage),null);
    // v17 supports measured geometry, but not the local neutral-evidence
    // plane. Extra Wasm arguments are otherwise silently ignored.
    exports.correct_scan_rows=function(a,b,c,d,e,f,g,h,i,j,k,l){};
    assert.equal(legacy.InhouseRust.scanColors(pixels,pixels,2,2,new Float32Array(4),null,2,2,new Float64Array(61),coverage,new Float32Array(4)),null);
    assert.equal(legacy.InhouseRust.inkAxes(pixels,2,2,coverage),null);
    // Six-argument kernels predate the adaptive silhouette contract too.
    exports.ink_axes=function(a,b,c,d,e,f){};
    assert.equal(legacy.InhouseRust.inkAxes(pixels,2,2,coverage),null);
    assert.equal(legacy.InhouseRust.balancePaper(pixels,2,2,new Float32Array(8),2,2,new Float32Array(16)),null);
    assert.equal(legacy.InhouseRust.inkRepair(pixels,new Uint16Array(4),coverage,2,2),null);
    assert.equal(allocations,0);
});

test('Rust checkpoint codec preserves every bit and rejects truncated streams',()=>{
    let seed=12345;
    const raw=Uint8Array.from({length:4096},()=>{seed=(Math.imul(seed,1664525)+1013904223)>>>0;return seed>>>24;});
    const packed=scope.InhouseRust.checkpointCodec(raw);
    assert.deepEqual(scope.InhouseRust.checkpointCodec(packed,raw.length),raw);
    for(const end of [0,1,7,packed.length-1]) {
        if(end)assert.throws(()=>scope.InhouseRust.checkpointCodec(packed.slice(0,end),raw.length),/Invalid/);
    }
    assert.throws(()=>scope.InhouseRust.checkpointCodec(new Uint8Array([9]),8),/Invalid/);
    assert.throws(()=>scope.InhouseRust.checkpointCodec(new Uint8Array(7)),/Invalid/);
});

test('Rust PDF ink preserves every point, width and color with bounded raster fallback',()=>{
    const stroke={tool:'pen',color:'#142dd2',width:2.5,points:[{x:30,y:40},{x:30,y:40},{x:31.125,y:40.875}]};
    const bounds={x:20,y:30,width:100,height:100};
    const text=new TextDecoder().decode(scope.InhouseRust.pdfInk([stroke],bounds));
    assert.match(text,/-20\.000000 130\.000000 cm/);
    assert.match(text,/2\.500000 w\n0\.078431 0\.176471 0\.823529 RG/);
    assert.match(text,/30\.000000 40\.000000 m\n30\.000000 40\.000000 l\n31\.125000 40\.875000 l/);
    for(const tool of ['eraser-area','highlighter'])assert.equal(scope.InhouseRust.pdfInk([{...stroke,tool}],bounds),null);
    assert.equal(scope.InhouseRust.pdfInk([{...stroke,points:Array(5000).fill({x:1,y:2})}],bounds),null);
    assert.equal(scope.InhouseRust.pdfInk([stroke],{...bounds,width:5,height:5}),null);
    assert.equal(scope.InhouseRust.pdfInk([{...stroke,width:NaN}],bounds),null);
    assert.equal(scope.InhouseRust.pdfInk([{...stroke,points:[{x:NaN,y:1},{x:2,y:3}]}],bounds),null);
    assert.equal(scope.InhouseRust.pdfInk([{...stroke,color:'invalid'}],bounds),null);
    assert.equal(scope.InhouseRust.pdfInk([stroke],{...bounds,x:Infinity}),null);
    assert.ok(scope.InhouseRust.pdfInk([stroke],bounds),'invalid data does not poison later exports');
});
