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
