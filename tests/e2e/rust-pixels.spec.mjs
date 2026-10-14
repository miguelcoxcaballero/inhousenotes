import {test,expect} from '@playwright/test';

test('Rust scan warp matches the production JavaScript path on perspective and curled meshes',async({page})=>{
    test.setTimeout(90000);
    await page.goto('/scanner/index.html?embed=1');
    await page.addScriptTag({url:'/tests/e2e/fixtures/scanner-synthetic.js'});
    const report=await page.evaluate(async()=>{
        ScannerPro.LiveCamera.close();
        await InhouseRust.ready;
        const reports=[];
        for(const testCase of ScannerSyntheticFixtures.CASES.slice(0,3)){
            const {source}=ScannerSyntheticFixtures.renderCase(testCase);
            const detection=await ScannerPro.Lightweight.detectPageAsync(source);
            const a=await ScannerPro.Lightweight.warp(source,detection.pageQuad,detection.frame,{maxDimension:1200,disableRust:true});
            const b=await ScannerPro.Lightweight.warp(source,detection.pageQuad,detection.frame,{maxDimension:1200});
            const left=a.getContext('2d').getImageData(0,0,a.width,a.height).data;
            const right=b.getContext('2d').getImageData(0,0,b.width,b.height).data;
            let max=0,sum=0;for(let i=0;i<left.length;i++){const d=Math.abs(left[i]-right[i]);max=Math.max(max,d);sum+=d;}
            reports.push({max,sum,bytes:left.length});
        }
        return {available:InhouseRust.available(),reports};
    });
    expect(report.available).toBe(true);
    for(const result of report.reports){expect(result.max).toBeLessThanOrEqual(1);expect(result.sum/result.bytes).toBeLessThan(.0001);}
});

test('pixel benchmark includes all buffer copies and reports medians',async({page})=>{
    test.setTimeout(90000);
    await page.goto('/scanner/index.html?embed=1');
    const result=await page.evaluate(async()=>{
        ScannerPro.LiveCamera.close();await InhouseRust.ready;
        const source=document.createElement('canvas');source.width=1800;source.height=2546;
        const ctx=source.getContext('2d');const gradient=ctx.createLinearGradient(0,0,1800,2546);gradient.addColorStop(0,'#123456');gradient.addColorStop(1,'#eeddcc');ctx.fillStyle=gradient;ctx.fillRect(0,0,1800,2546);
        const quad=[{x:80,y:90},{x:1720,y:70},{x:1750,y:2470},{x:50,y:2480}];
        const times={js:[],rust:[]};
        for(let round=0;round<7;round++)for(const mode of round%2?['rust','js']:['js','rust']){
            const start=performance.now();
            const output=await ScannerPro.Lightweight.warp(source,quad,null,{maxDimension:1800,disableRust:mode==='js'});
            if(round>1)times[mode].push(performance.now()-start);
            output.width=output.height=0;
        }
        const rgba=ctx.getImageData(0,0,1800,2546).data;
        const split={js:[],rust:[]};
        for(let round=0;round<10;round++)for(const mode of ['js','rust']){
            const start=performance.now();
            if(mode==='rust')InhouseRust.splitRgba(rgba);
            else {const rgb=new Uint8Array(rgba.length/4*3),alpha=new Uint8Array(rgba.length/4);let transparent=false;
                for(let p=0,c=0;p<alpha.length;p++,c+=4){rgb[p*3]=rgba[c];rgb[p*3+1]=rgba[c+1];rgb[p*3+2]=rgba[c+2];alpha[p]=rgba[c+3];transparent ||= alpha[p]!==255;}}
            if(round>2)split[mode].push(performance.now()-start);
        }
        const median=values=>values.sort((a,b)=>a-b)[Math.floor(values.length/2)];
        return {warp:{js:median(times.js),rust:median(times.rust)},split:{js:median(split.js),rust:median(split.rust)}};
    });
    console.log('RUST_PIXEL_BENCHMARK',JSON.stringify(result));
    // Timing is evidence for selecting the implementation, not a flaky CI threshold.
    expect(result.warp.rust).toBeGreaterThan(0);
});
