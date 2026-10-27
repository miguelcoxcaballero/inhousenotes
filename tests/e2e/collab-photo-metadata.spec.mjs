import {test,expect} from '@playwright/test';

test('repeated collaborative snapshots do not rehash unchanged photo bytes',async({page})=>{
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
 const result=await page.evaluate(()=>{
  const pages=Array.from({length:3},(_,i)=>({pageId:'photo-'+i,backgroundSource:'custom',
   backgroundImage:'data:image/png;base64,'+String(i).repeat(256*1024),pageWidth:794,pageHeight:1123}));
  const hash=simpleHash;let bytes=0;
  simpleHash=value=>{if(value.startsWith('data:image/'))bytes+=value.length;return hash(value);};
  try{
   const seeded=seedCollabFieldMetadata(null,pages,'Photos',null),first=bytes;bytes=0;
   let snapshot=seeded;
   for(let i=0;i<30;i++)snapshot=seedCollabFieldMetadata(snapshot,pages,'Photos',null);
   return {first,repeated:bytes,identical:JSON.stringify(snapshot)===JSON.stringify(seeded)};
  }finally{simpleHash=hash;}
 });
 expect(result.first).toBeGreaterThan(750000);expect(result.repeated).toBe(0);expect(result.identical).toBe(true);
 console.log('PHOTO_METADATA_HASH_BYTES',result);
});

test('missing photo registers are seeded while existing remote revisions stay authoritative',async({page})=>{
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
 const result=await page.evaluate(()=>{
  const old={pageId:'old-photo',backgroundSource:'custom',backgroundImage:'data:image/png;base64,OLD',pageWidth:794,pageHeight:1123};
  const fresh={...old,pageId:'new-photo',backgroundImage:'data:image/png;base64,NEW'};
  const seeded=seedCollabFieldMetadata(null,[old],'Photos',null);
  const key=getCollabPageFieldKey(old.pageId,'background');
  seeded.fields[key]={value:{backgroundSource:'custom',backgroundImageHash:'remote-hash'},stamp:{clock:8,actor:'other-device'}};
  const snapshot=seedCollabFieldMetadata(seeded,[old,fresh],'Photos',null);
  // Explicit background edits still calculate a fresh value for their LWW register.
  const changed=getCollabBackgroundValue({...fresh,backgroundImage:'data:image/png;base64,EDITED'});
  return {kept:snapshot.fields[key],fresh:snapshot.fields[getCollabPageFieldKey(fresh.pageId,'background')].value,
   expected:getCollabBackgroundValue(fresh),changed:changed.backgroundImageHash,expectedEdit:simpleHash('data:image/png;base64,EDITED')};
 });
 expect(result.kept).toEqual({value:{backgroundSource:'custom',backgroundImageHash:'remote-hash'},stamp:{clock:8,actor:'other-device'}});
 expect(result.fresh).toEqual(result.expected);expect(result.changed).toBe(result.expectedEdit);
});
