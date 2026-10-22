import {test,expect} from '@playwright/test';
const account='upload@example.test';
test.beforeEach(async({page})=>{
 await page.route('https://accounts.google.com/**',r=>r.fulfill({body:''}));
 await page.addInitScript(account=>{localStorage.setItem('drive-hint-email',account);localStorage.setItem('drive-remember-v1','1');},account);
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
 await page.evaluate(()=>showDriveHome());
});
for(const width of [390,820,1280])test(`plus menu offers create and upload at ${width}px`,async({page})=>{
 await page.setViewportSize({width,height:900});
 const add=page.getByRole('button',{name:'Add file',exact:true});await expect(add).toBeVisible();await expect(add).not.toContainText('New PDF');
 await add.click();await expect(page.getByRole('menuitem',{name:'Create PDF'})).toBeVisible();await expect(page.getByRole('menuitem',{name:'Upload file'})).toBeVisible();
 await page.keyboard.press('Escape');await expect(page.locator('#drive-create-menu')).toBeHidden();await expect(add).toBeFocused();
 await add.click();await page.getByRole('menuitem',{name:'Create PDF'}).click();await expect(page.locator('#modal-folder-picker')).toHaveClass(/visible/);
 await expect(page.locator('#folder-picker-title')).toHaveText('New PDF');
});
test('a device PDF is durable offline in the selected Drive folder before closing the picker',async({page,context})=>{
 await context.setOffline(true);
 await page.evaluate(async()=>{await InhouseOfflineLibrary.observed([{id:'destination',name:'Destination',mimeType:'application/vnd.google-apps.folder',parents:['root']}]);});
 await page.getByRole('button',{name:'Add file',exact:true}).click();
 const chooser=page.waitForEvent('filechooser');await page.getByRole('menuitem',{name:'Upload file'}).click();
 await (await chooser).setFiles({name:'Imported.pdf',mimeType:'application/pdf',buffer:Buffer.from('%PDF-original-bytes')});
 await expect(page.locator('#folder-picker-title')).toHaveText('Upload file');
 await page.locator('#folder-picker-list').getByText('Destination',{exact:true}).click();
 await page.locator('#folder-picker-confirm').click();await expect(page.locator('#modal-folder-picker')).not.toHaveClass(/visible/);
 const result=await page.evaluate(async account=>{
  const doc=(await InhouseOfflineStore.list('documents',account)).find(doc=>doc.entry?.name==='Imported.pdf');
  return {body:await doc.blob.text(),parents:doc.entry.parents,dirty:doc.dirty,outbox:!!await InhouseOfflineStore.read('outbox',account,doc.id)};
 },account);
 expect(result).toEqual({body:'%PDF-original-bytes',parents:['destination'],dirty:true,outbox:true});
});
test('non-PDF device uploads preserve MIME, bytes, name and chosen Drive location',async({page,context})=>{
 let metadata,content,contentType;
 await context.route('https://www.googleapis.com/**',route=>{
  const req=route.request(),url=new URL(req.url());
  if(url.pathname==='/upload-session') {content=req.postData();contentType=req.headers()['content-type'];return route.fulfill({json:{id:'uploaded',name:'sample.txt'}});}
  if(url.pathname.includes('/upload/')){metadata=JSON.parse(req.postData());return route.fulfill({headers:{Location:'https://www.googleapis.com/upload-session','Access-Control-Expose-Headers':'Location'},body:''});}
  return route.fulfill({json:{files:[{id:'destination',name:'Destination',mimeType:'application/vnd.google-apps.folder'}]}});
 });
 await page.evaluate(()=>{driveSignedOut=false;driveAccessToken='test';driveTokenExpiry=Date.now()+600000;});
 await page.locator('#drive-upload-input').setInputFiles({name:'sample.txt',mimeType:'text/plain',buffer:Buffer.from('original text')});
 await page.locator('#folder-picker-list').getByText('Destination',{exact:true}).click();
 await page.locator('#folder-picker-confirm').click();await expect(page.locator('#modal-folder-picker')).not.toHaveClass(/visible/);
 expect(metadata).toEqual({name:'sample.txt',mimeType:'text/plain',parents:['destination']});expect(content).toBe('original text');expect(contentType).toBe('text/plain');
});
