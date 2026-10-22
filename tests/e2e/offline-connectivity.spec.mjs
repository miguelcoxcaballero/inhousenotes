import {test,expect} from '@playwright/test';
const account='network@example.test';
async function start(page,{navigatorOffline=false}={}){
 await page.route('https://accounts.google.com/**',r=>r.fulfill({body:''}));
 await page.route('**/service-worker.js?connectivity=*',r=>r.abort());
 await page.route('https://www.googleapis.com/drive/v3/about?fields=kind',r=>r.fulfill({status:401,json:{error:'Authentication required'}}));
 await page.addInitScript(({account,navigatorOffline})=>{
  if(window.top!==window)return;
  localStorage.setItem('drive-hint-email',account);sessionStorage.setItem('ihn-network-offline','1');
  if(navigatorOffline)Object.defineProperty(navigator,'onLine',{get:()=>false});
 },{account,navigatorOffline});
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
}
for(const navigatorOffline of [false,true])test(`reachable Drive clears stale offline status even when shell probe fails (navigatorOffline=${navigatorOffline})`,async({page})=>{
 await start(page,{navigatorOffline});
 await expect.poll(()=>page.evaluate(()=>InhouseOfflineLibrary.offline())).toBe(false);
 expect(await page.evaluate(()=>sessionStorage.getItem('ihn-network-offline'))).toBe('0');
});
test('a late failed probe cannot undo an authenticated Drive response',async({page})=>{
 await start(page);await expect.poll(()=>page.evaluate(()=>InhouseOfflineLibrary.offline())).toBe(false);
 let release,entered;const held=new Promise(r=>release=r),started=new Promise(r=>entered=r);
 await page.route('https://www.googleapis.com/drive/v3/about?fields=kind',async r=>{entered();await held;await r.abort();});
 await page.evaluate(()=>{dispatchEvent(new Event('offline'));dispatchEvent(new Event('online'));});await started;
 await page.evaluate(()=>InhouseOfflineLibrary.networkAvailable());release();
 await page.waitForTimeout(100);
 expect(await page.evaluate(()=>InhouseOfflineLibrary.offline())).toBe(false);
});
test('failed uploads expose the persisted reason on the document without opening the editor',async({page})=>{
 await start(page);
 await page.evaluate(async account=>{
  await InhouseOfflineStore.checkpoint(account,'failed',{blob:new Blob(['pdf']),entry:{id:'failed',name:'Pending.pdf',parents:['root'],mimeType:'application/pdf'}});
  await InhouseOfflineStore.patch('outbox',account,'failed',old=>({...old,lastError:412,lastErrorPhase:'merging',lastErrorMessage:'Remote document changed during merge; retrying'}));
  await InhouseOfflineLibrary.reload();renderDriveRecents([{id:'failed',name:'Pending.pdf',mimeType:'application/pdf'}]);
  dispatchEvent(new Event('offline'));
 },account);
 const badge=page.locator('#drive-recents .document-sync-status');await expect(badge).toHaveText('Retry upload');
 await badge.click();await expect(page.locator('#offline-sync-error')).toBeVisible();
 await expect(page.locator('.sync-error-detail')).toContainText('merging · 412');
 await expect(page.locator('.sync-error-detail')).toContainText('Remote document changed during merge');
 await page.getByRole('button',{name:'Close',exact:true}).click();await expect(page.locator('#offline-sync-error')).toHaveCount(0);
});
