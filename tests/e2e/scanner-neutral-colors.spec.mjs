import { expect, test } from '@playwright/test';

// The colour references are rendered by the production template, then passed
// through a camera response. These are recoverable illumination changes: no
// clipped channel is expected to reconstruct information it never recorded.
async function measure(page, cases) {
  return page.evaluate(async definitions => {
    const rows = [];
    for (const camera of definitions) {
      const canvas = document.createElement('canvas');
      canvas.width = 840; canvas.height = 1188;
      const context = canvas.getContext('2d', { willReadFrequently: true });
      ScannerPro.ScanTemplate.render(context, canvas.width, canvas.height, { background: true });
      if (camera.hideYellow || camera.greyYellow) {
        // The photograph has an already verified warp, but the thin yellow
        // border was washed out. The footer colour discs remain observable.
        context.fillStyle = '#fff';
        for (const [x, y, width, height] of [[55, 52, 730, 10], [55, 1132, 730, 10], [55, 52, 10, 1090], [775, 52, 10, 1090]]) {
          context.fillRect(x, y, width, height);
        }
      }
      if (camera.greyYellow) {
        // A neutral ridge can confirm geometry, but must not masquerade as
        // yellow pigment and distort a valid four-disc camera response.
        context.strokeStyle = '#aaa';
        context.lineWidth = 2.4;
        context.strokeRect(1.5 * 40, 1.43 * 40, 18 * 40, 27 * 40);
      }
      if (camera.missingMarker || camera.corruptMarker) {
        context.fillStyle = camera.corruptMarker || '#fff';
        context.beginPath(); context.arc(10.625 * 40, 27.68 * 40, 5.5, 0, Math.PI * 2); context.fill();
      }
      if (camera.crossDots) {
        context.strokeStyle = '#a8414e'; context.lineWidth = 2.5;
        for (let column=5; column<20; column++) for(let row=12;row<44;row++) {
          if ((column+row)%4) continue;
          const x=(20+column*5)*4,y=(19.3+row*5)*4;
          if ([290,440,590,740].some(inkY=>Math.abs(y-inkY)<22)) continue;
          context.beginPath(); context.moveTo(x-5,y-2);context.lineTo(x+5,y+2);context.stroke();
        }
      }
      for (const [index, y] of [290, 440, 590, 740].entries()) {
        context.fillStyle = (camera.inks || ['#d52b36', '#254acb', '#414141', '#aaa'])[index];
        context.fillRect(140, y, 210, camera.penWidth || 12);
      }
      const pixels = context.getImageData(0, 0, canvas.width, canvas.height);
      const gains = camera.gains || [1, 1, 1];
      const flare = camera.flare || 0;
      const exposure = camera.exposure || 1;
      for (let y = 0; y < canvas.height; y++) for (let x = 0; x < canvas.width; x++) {
        const offset = (y * canvas.width + x) * 4;
        const shade = (.94 - .1 * x / canvas.width)
          * (camera.footerShadow && y > 1000 ? .48 : 1);
        const source = [pixels.data[offset], pixels.data[offset + 1], pixels.data[offset + 2]];
        for (let channel = 0; channel < 3; channel++) {
          const localGain = camera.localGains ? 1 + (camera.localGains[channel] - 1) * Math.exp(-(((y - 580) / 290) ** 4)) : 1;
          pixels.data[offset + channel] = exposure * shade * gains[channel] * localGain
            * (flare + (camera.matrix
              ? camera.matrix[channel].reduce((sum, gain, c) => sum + gain * source[c], 0)
              : source[channel]) * (1 - flare / 255))
            + (camera.localFlare?.[channel] || 0) * Math.exp(-(((y - 580) / 290) ** 4));
        }
      }
      context.putImageData(pixels, 0, 0);
      if (camera.jpeg) {
        const bitmap = await createImageBitmap(await new Promise(resolve => canvas.toBlob(resolve, 'image/jpeg', .86)));
        context.drawImage(bitmap, 0, 0); bitmap.close();
      }
      const started = performance.now();
      const colors = await ScannerPro.Lightweight.correctColors(canvas, { useStencil: true, preciseStencil: true, disableRust: camera.disableRust });
      const elapsedMs = performance.now() - started;
      const result = context.getImageData(0, 0, canvas.width, canvas.height).data;
      const rgb = (x, y) => Array.from(result.slice((y * canvas.width + x) * 4, (y * canvas.width + x) * 4 + 3));
      const pens={};
      if(camera.localGains || camera.localFlare) for(const [name,y] of [['red',290],['blue',440],['black',590],['pencil',740]]) {
        const crop=context.getImageData(125,y-3,240,18);
        InhouseRust.separateInk(crop.data);
        const ink=await ScannerPro.InkVectorizer.fromPixels(crop,{forceJavaScript:camera.disableRust});
        pens[name]=[...new Set(ink.strokes.map(s=>s.color))];
      }
      const centre = Math.floor((camera.penWidth || 12) / 2);
      rows.push({ name: camera.name, pens, calibrated: colors.calibrated, calibration: colors.calibration,
        references: colors.references, illumination: colors.illumination, elapsedMs, backend: colors.pixelProcessing.backend,
        red: rgb(240, 290 + centre), blue: rgb(240, 440 + centre), black: rgb(240, 590 + centre), pencil: rgb(240, 740 + centre),
        paper: rgb(280, 370), outsideRed: rgb(247, 287), outsideBlue: rgb(247, 437) });
    }
    return rows;
  }, cases);
}

function diagnostics(row) {
  return { ...row, calibration: { method: row.calibration.method, matrixError: row.calibration.matrixError,
    references: row.calibration.references, sourcePaper: row.calibration.sourcePaper } };
}

test('neutral printed dots calibrate strong local reflections without recolouring real navy or red pens', async ({ page }) => {
  test.setTimeout(120000);
  await page.goto('/scanner/index.html', { waitUntil: 'domcontentloaded' });
  const cases = [false,true].flatMap(disableRust => ['#122b60','#405a68'].flatMap(blue => [
    { name: `strong-blue-${blue}-${disableRust}`, localFlare: [0,8,65], exposure: .62, disableRust, blue },
    { name: `strong-red-${blue}-${disableRust}`, localFlare: [65,8,0], exposure: .62, disableRust, blue }
  ]));
  const rows = await measure(page,cases.map(camera => ({ ...camera,jpeg:true,penWidth:3,inks:['#651520',camera.blue,'#333333','#aaa'] })));
  console.log('NEUTRAL_DOT_CALIBRATION',JSON.stringify(rows.map(diagnostics)));
  for (let index=0;index<4;index++) {
    expect(rows[index].backend).toBe('rust'); expect(rows[index+4].backend).toBe('javascript');
    for (const colour of ['black','pencil','blue','red']) expect(rows[index][colour]).toEqual(rows[index+4][colour]);
  }
  for(const row of rows){
    expect(row.pens.black,row.name).toEqual(['#4D4D4D']);
    expect(row.pens.pencil,row.name).toEqual(['#4D4D4D']);
    expect(row.pens.blue,row.name).toEqual(['#002FD9']);
    expect(row.pens.red,row.name).toEqual(['#E81010']);
  }
});

test('coloured handwriting crossing some reference dots cannot recolour nearby black ink', async ({ page }) => {
  test.setTimeout(60000);
  await page.goto('/scanner/index.html', { waitUntil: 'domcontentloaded' });
  const rows = await measure(page,[false,true].map(disableRust => ({
    name:`crossed-dots-${disableRust}`,crossDots:true,localFlare:[0,8,65],exposure:.62,disableRust,
    jpeg:true,penWidth:3,inks:['#651520','#405a68','#333333','#aaa']
  })));
  for(const row of rows) {
    expect(row.pens.black,row.name).toEqual(['#4D4D4D']);
    expect(row.pens.pencil,row.name).toEqual(['#4D4D4D']);
    expect(row.pens.blue,row.name).toEqual(['#002FD9']);
    expect(row.pens.red,row.name).toEqual(['#E81010']);
  }
});

test('neutral-axis calibration keeps saturated ink luminance when colour correction reaches RGB limits',async({page})=>{
  await page.goto('/scanner/index.html',{waitUntil:'domcontentloaded'});
  const result=await page.evaluate(async()=>{
    await InhouseRust.ready;
    const input=new Uint8ClampedArray([210,40,25,255, 15,30,220,255, 240,220,180,255, 20,45,30,255]);
    const output=input.slice();
    const job=InhouseRust.balancePaper(output,4,1,new Float32Array([1,1]),1,1,new Float32Array([1.6,.6,1.5,220]));
    if(!job)return null;
    try{job.rows(0,1);job.copyTo(output);}finally{job.dispose();}
    const luminance=data=>Array.from({length:4},(_,index)=>data[index*4]*.2126+data[index*4+1]*.7152+data[index*4+2]*.0722);
    return{before:luminance(input),after:luminance(output),changed:input.some((value,index)=>value!==output[index])};
  });
  expect(result).not.toBeNull(); expect(result.changed).toBe(true);
  for(let index=0;index<result.before.length;index++)expect(Math.abs(result.before[index]-result.after[index])).toBeLessThan(.51);
});
