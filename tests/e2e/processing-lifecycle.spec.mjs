import {test,expect} from '@playwright/test';

test('two loaded source PDFs remain independent and reject foreign page handles',async({page})=>{
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
 const result=await page.evaluate(async()=>{
  const source=await PDFLib.PDFDocument.create();source.addPage([150,200]);const bytes=await source.save();
  const library=await InhouseDocumentProcessing.pdfLib(PDFLib);
  const [first,second]=await Promise.all([library.PDFDocument.load(bytes),library.PDFDocument.load(bytes)]);
  const out=await library.PDFDocument.create();
  try {
   for(const input of [first,second]){const pages=await out.copyPages(input,[0]);await out.addPage(pages[0]);}
   const foreign=await second.addPage([10,10]);
   let rejected=false;try{await out.addPage(foreign);}catch(_){rejected=true;}
   const pdf=await PDFLib.PDFDocument.load(await out.save());
   return {pages:pdf.getPageCount(),reported:out.getPageCount(),rejected};
  }finally{await Promise.all([first.dispose(),second.dispose(),out.dispose()]);}
 });
 expect(result).toEqual({pages:2,reported:2,rejected:true});
});

test('worker reset cannot make an old PDF proxy address a new document',async({page})=>{
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
 const result=await page.evaluate(async()=>{
  const oldLib=await InhouseDocumentProcessing.pdfLib(PDFLib),old=await oldLib.PDFDocument.create();
  await old.addPage([100,100]);InhouseDocumentProcessing.reset();
  const newLib=await InhouseDocumentProcessing.pdfLib(PDFLib),fresh=await newLib.PDFDocument.create();
  await fresh.addPage([200,200]);let staleRejected=false;
  try{await old.setTitle('wrong document');}catch(error){staleRejected=error.name==='InvalidStateError';}
  await old.dispose();const pdf=await PDFLib.PDFDocument.load(await fresh.save());await fresh.dispose();
  return {staleRejected,title:pdf.getTitle()||'',width:pdf.getPages()[0].getSize().width};
 });
 expect(result).toEqual({staleRejected:true,title:'',width:200});
});

test('Drive save requests wait for their own batch and preserve forced options',async({page})=>{
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
 const result=await page.evaluate(async()=>{
  const original=window.InhouseOfflineApp;
  let release;const gate=new Promise(resolve=>release=resolve);const calls=[];
  window.InhouseOfflineApp={...original,queueSave:async(force,options)=>{calls.push({force,keepalive:options.keepalive});if(calls.length===1)await gate;return calls.length;}};
  try {
   const first=queueDriveSave(false);
   const forced=queueDriveSave(true,{keepalive:true});
   const last=queueDriveSave(false);
   let prematurelyCompleted=false;forced.then(()=>prematurelyCompleted=true);
   await Promise.resolve();const before={calls:calls.length,prematurelyCompleted};
   release();const values=await Promise.all([first,forced,last]);
   return {before,calls,values};
  }finally{window.InhouseOfflineApp=original;}
 });
 expect(result.before).toEqual({calls:1,prematurelyCompleted:false});
 expect(result.calls).toEqual([{force:false,keepalive:undefined},{force:true,keepalive:true}]);
 expect(result.values).toEqual([1,2,2]);
});

test('overlapping exports keep their source alive through cache replacement',async({page})=>{
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
 const result=await page.evaluate(async()=>{
  const source=await PDFLib.PDFDocument.create();source.addPage([200,300]).drawText('Original content');
  await __IHN_TEST_API__.primePdfPageForTest(Array.from(await source.save()));
  const outputs=await Promise.all([buildPdfBlobWithPdfLib({status:false}),buildPdfBlobWithPdfLib({status:false})]);
  return Promise.all(outputs.map(async blob=>{
   const pdf=await pdfjsLib.getDocument({data:await blob.arrayBuffer()}).promise;
   const text=(await (await pdf.getPage(1)).getTextContent()).items.map(i=>i.str).join('');
   const count=pdf.numPages;await pdf.destroy();return {count,text};
  }));
 });
 expect(result).toEqual([{count:1,text:'Original content'},{count:1,text:'Original content'}]);
});
