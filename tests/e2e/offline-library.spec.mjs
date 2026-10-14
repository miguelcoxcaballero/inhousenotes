import {test,expect} from '@playwright/test';

test.beforeEach(async({page})=>{
    await page.route('https://accounts.google.com/**',route=>route.fulfill({status:200,contentType:'text/javascript',body:''}));
});

test('expired remembered session never paints the sign-in screen while libraries load',async({page})=>{
    await page.addInitScript(()=>{
        localStorage.setItem('drive-remember-v1','1');
        localStorage.setItem('drive-hint-email','offline@example.test');
        localStorage.setItem('drive-token-v1',JSON.stringify({token:'old',expiry:1}));
    });
    let release;
    const hold=new Promise(resolve=>{release=resolve;});
    await page.route('**/vendor/pdfjs/pdf.min.js',async route=>{await hold;await route.continue();});
    const navigation=page.goto('/?e2e=1',{waitUntil:'domcontentloaded'});
    await expect(page.locator('html')).toHaveAttribute('data-boot-destination','drive');
    await expect(page.locator('#welcome-view')).toBeHidden();
    await expect(page.locator('#drive-home')).toBeVisible();
    release();await navigation;
    await page.waitForFunction(()=>!!window.__IHN_TEST_API__);
    await page.evaluate(()=>window.__IHN_TEST_API__.ready());
    await expect(page.locator('#welcome-view')).toBeHidden();
});

test('offline folders, multiple document snapshots and pinned retention survive reload',async({page,context})=>{
    await page.addInitScript(()=>{localStorage.setItem('drive-hint-email','offline@example.test');localStorage.setItem('drive-remember-v1','1');});
    await page.goto('/?e2e=1');
    await page.evaluate(()=>window.__IHN_TEST_API__.ready());
    await context.setOffline(true);
    const ids=await page.evaluate(async()=>{
        const folder=await InhouseOfflineLibrary.createFolder('root','Offline folder');
        await createNewDriveDocumentWithFolder(folder.id,'First');
        const first=state.driveFileId;
        state.pages[0].strokes.push({id:'offline-ink',tool:'pen',color:'#000000',width:2,points:[{x:10,y:20},{x:40,y:50}]});
        bumpDriveContentVersion();await InhouseOfflineLibrary.capture();
        await createNewDriveDocumentWithFolder(folder.id,'Second');
        const second=state.driveFileId;
        await openDriveFile({id:first,name:'First.pdf'});
        return {first,second,folder:folder.id,stroke:state.pages[0].strokes[0]?.id};
    });
    expect(ids.stroke).toBe('offline-ink');
    const result=await page.evaluate(async ids=>{
        const S=InhouseOfflineStore,account=InhouseOfflineLibrary.account();
        const documents=await S.list('documents',account);
        await S.checkpoint(account,'old',{blob:new Blob(['pdf']),lastOpenedAt:1},false);
        await S.checkpoint(account,'pinned',{blob:new Blob(['pdf']),lastOpenedAt:1,pinned:true},false);
        await S.prune(account);
        return {ids:documents.map(d=>d.id),old:await S.read('documents',account,'old'),pinned:!!await S.read('documents',account,'pinned'),pending:(await S.list('outbox',account)).length};
    },ids);
    expect(result.ids).toEqual(expect.arrayContaining([ids.first,ids.second]));
    expect(result.old).toBeNull();expect(result.pinned).toBe(true);expect(result.pending).toBe(3);
});

test('unavailable documents are grey and cannot replace the open document offline',async({page,context})=>{
    await page.addInitScript(()=>localStorage.setItem('drive-hint-email','offline@example.test'));
    await page.goto('/?e2e=1');await page.evaluate(()=>window.__IHN_TEST_API__.ready());
    await page.evaluate(async()=>{
        await InhouseOfflineLibrary.observed([{id:'not-downloaded',name:'Remote only.pdf',mimeType:'application/pdf',parents:['root']}]);
    });
    await context.setOffline(true);
    await expect(page.locator('[data-drive-item-id="not-downloaded"]').first()).toHaveClass(/offline-unavailable/);
    await page.locator('[data-drive-item-id="not-downloaded"]').first().click();
    await expect(page.locator('#drive-home')).toBeVisible();
});

test('outbox acknowledgment cannot delete an edit made during upload',async({page})=>{
    await page.goto('/?e2e=1');await page.evaluate(()=>window.__IHN_TEST_API__.ready());
    const result=await page.evaluate(async()=>{
        const S=InhouseOfflineStore,a='test-account';
        const first=await S.checkpoint(a,'doc',{snapshot:{content:'one'},baseMeta:{version:'1'}});
        await S.checkpoint(a,'doc',{snapshot:{content:'two'},baseMeta:{version:'1'}});
        const settled=await S.acknowledge(a,'doc',first.generation,{id:'doc',version:'2'},new Blob(['one']));
        const doc=await S.read('documents',a,'doc');
        return {settled,dirty:doc.dirty,content:doc.snapshot.content,version:doc.baseMeta.version,pending:(await S.list('outbox',a)).length};
    });
    expect(result).toEqual({settled:false,dirty:true,content:'two',version:'1',pending:1});
});

test('cold offline navigation restores cached app, folder tree and independent documents',async({page,context})=>{
    await page.addInitScript(()=>{localStorage.setItem('drive-hint-email','cold@example.test');localStorage.setItem('drive-remember-v1','1');});
    await page.goto('/?e2e=1');await page.evaluate(()=>window.__IHN_TEST_API__.ready());
    await page.evaluate(()=>navigator.serviceWorker.ready);
    await page.waitForFunction(()=>!!navigator.serviceWorker.controller);
    await context.setOffline(true);
    const ids=await page.evaluate(async()=>{
        const folder=await InhouseOfflineLibrary.createFolder('root','Travel');
        await createNewDriveDocumentWithFolder(folder.id,'One');
        const first=state.driveFileId;
        state.pages[0].strokes.push({id:'cold-ink',tool:'pen',color:'#123456',width:2,points:[{x:10,y:20},{x:40,y:50}]});
        bumpDriveContentVersion();await InhouseOfflineLibrary.capture();
        await createNewDriveDocumentWithFolder(folder.id,'Two');
        await persistExitLocalCheckpoint();
        return {first,folder:folder.id};
    });
    await page.reload();await page.evaluate(()=>window.__IHN_TEST_API__.ready());
    const restored=await page.evaluate(async ids=>{
        const entries=InhouseOfflineLibrary.folder(ids.folder);
        await openDriveFile({id:ids.first,name:'One.pdf'});
        return {names:entries.map(e=>e.name),ink:state.pages[0].strokes.map(s=>s.id)};
    },ids);
    expect(restored.names).toEqual(expect.arrayContaining(['One.pdf','Two.pdf']));
    expect(restored.ink).toContain('cold-ink');
    await expect(page.locator('#welcome-view')).toBeHidden();
});

test('visible controls create a folder and document offline and keep the last edit when switching',async({page,context})=>{
 await page.addInitScript(()=>localStorage.setItem('drive-hint-email','controls@example.test'));
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());await context.setOffline(true);
 await page.locator('#home-nav [data-home-tab="files"]').click();
 await page.locator('#btn-drive-new-folder').click();await page.locator('#new-folder-name').fill('Trips');
 await page.locator('#new-folder-confirm').click();
 const folder=page.locator('#drive-folders [data-drive-item-id]').filter({hasText:'Trips'});
 await expect(folder).toBeVisible();await folder.click();
 await expect(page.locator('#drive-folder-view')).toBeVisible();await expect(page.locator('#drive-folder-view-title')).toHaveText('Trips');
 await page.locator('#btn-drive-new').click();await page.locator('#folder-picker-name').fill('Travel notes');
 await page.locator('#folder-picker-confirm').click();await expect(page.locator('#editor-view')).toBeVisible();
 const saved=await page.evaluate(async()=>{
  const first=state.driveFileId;
  state.pages[0].strokes.push({id:'last-moment',tool:'pen',color:'#000',width:2,points:[{x:2,y:3},{x:6,y:7}]});
  bumpDriveContentVersion();
  await createNewDriveDocumentWithFolder(state.driveParentIds[0],'Next');
  const doc=await InhouseOfflineStore.read('documents',InhouseOfflineLibrary.account(),first);
  return {first,ids:doc.snapshot.pages[0].strokes.map(s=>s.id)};
 });
 expect(saved.ids).toContain('last-moment');
});
