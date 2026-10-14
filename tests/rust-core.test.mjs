import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import vm from 'node:vm';
const bytes=await readFile(new URL('../wasm/inhouse_notes_core.wasm',import.meta.url));
const scope={WebAssembly,Uint8Array,URL,location:new URL('https://notes.test/'),fetch:async()=>new Response(bytes),console,AbortSignal,AbortController,setTimeout,clearTimeout};
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
