import {expect,test} from '@playwright/test';

test('six measured references restore consistent red and blue pen colours across lamps and camera flare',async({page})=>{
  await page.goto('/scanner/index.html',{waitUntil:'domcontentloaded'});
  const reports=await page.evaluate(async()=>{
    const rows=[];
    for(const camera of [{name:'neutral',gains:[1,1,1],flare:0},{name:'warm',gains:[1,.85,.7],flare:18},{name:'cool',gains:[.72,.86,1],flare:24}]) {
      const canvas=document.createElement('canvas');canvas.width=840;canvas.height=1188;
      const ctx=canvas.getContext('2d',{willReadFrequently:true});ScannerPro.ScanTemplate.render(ctx,840,1188,{background:true});
      ctx.fillStyle='#d52b36';ctx.fillRect(140,290,210,18);
      ctx.fillStyle='#254acb';ctx.fillRect(140,440,210,18);
      const image=ctx.getImageData(0,0,840,1188),data=image.data;
      for(let y=0;y<1188;y++)for(let x=0;x<840;x++)for(let c=0;c<3;c++){
        const i=(y*840+x)*4+c;const shade=.92-.12*x/840;
        data[i]=Math.min(255,(camera.flare+data[i]*(1-camera.flare/255))*camera.gains[c]*shade);
      }
      ctx.putImageData(image,0,0);
      const colors=await ScannerPro.Lightweight.correctColors(canvas,{useStencil:true,preciseStencil:true});
      const out=ctx.getImageData(0,0,840,1188).data;
      const rgb=(x,y)=>Array.from(out.slice((y*840+x)*4,(y*840+x)*4+3));
      rows.push({name:camera.name,red:rgb(240,298),blue:rgb(240,448),paper:rgb(280,370),calibration:colors.calibration,calibrated:colors.calibrated});
    }
    return rows;
  });
  console.log('REFERENCE_COLORS',JSON.stringify(reports.map(({name,red,blue,paper,calibration})=>({name,red,blue,paper,method:calibration.method}))));
  for(const row of reports){
    expect(row.calibrated,row.name).toBe(true);
    expect(row.calibration.references).toEqual(['red','black','blue','green','yellow','paper']);
    expect(Math.min(...row.paper),row.name+' white paper').toBeGreaterThan(245);
    expect(row.red[0]-Math.max(row.red[1],row.red[2]),row.name+' red ink is not pastel').toBeGreaterThan(125);
    expect(row.blue[2]-Math.max(row.blue[0],row.blue[1]),row.name+' blue ink is not pastel').toBeGreaterThan(120);
  }
  for(const colour of ['red','blue'])for(let c=0;c<3;c++){
    const values=reports.map(row=>row[colour][c]);
    expect(Math.max(...values)-Math.min(...values),colour+' lamp consistency').toBeLessThan(35);
  }
});
