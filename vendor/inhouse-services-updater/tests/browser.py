"""Exercise the exact bundled common popup, preferences and native callbacks."""
import argparse,base64,json,os
from pathlib import Path
from playwright.sync_api import sync_playwright
p=argparse.ArgumentParser();p.add_argument('--root',type=Path,required=True);a=p.parse_args();root=a.root
assets=root/'app/assets/updates';config=json.loads((assets/'config.json').read_text());repository=config['repository']
def release(version,code):return dict(version=version,versionCode=code,apkUrl=f'https://github.com/{repository}/releases/download/v{version}/app.apk',apkSha256='a'*64,apkSizeBytes=500000,changelog='Canales A/B y recordatorio.')
rows=[release('0.0.4',4),release('0.0.3',3),release('0.0.2',2)]
catalogs={'A':dict(schemaVersion=1,channel='A',latest=rows[0],releases=rows),'B':dict(schemaVersion=1,channel='B',latest=None,releases=[])}
with sync_playwright() as p:
 executable=next((x for x in (os.environ.get('CHROMIUM'),'/usr/bin/chromium','/opt/pw-browsers/chromium') if x and Path(x).exists()),None)
 browser=p.chromium.launch(executable_path=executable,headless=True,args=['--no-sandbox'])
 context=browser.new_context(viewport={'width':390,'height':800})
 context.add_init_script('window.installs=[];window.offers=[];window.closedUpdater=0;window.InhouseNative={getAppVersion(){return "0.0.1"},getAppVersionCode(){return 1},installAppUpdate(url,sha){window.installs.push({url,sha});window.handleInhouseUpdateResult({status:"permission_required"})},closeUpdater(){window.closedUpdater++}};window.InhouseUpdateHost={offer(value){window.offers.push(JSON.parse(value))}}')
 def route(r):
  url=r.request.url
  if 'raw.githubusercontent.com' in url or 'api.github.com' in url:
   channel='B' if 'B.json' in url else 'A';value=catalogs[channel]
   if 'api.github.com' in url:value={'content':base64.b64encode(json.dumps(value).encode()).decode()}
   r.fulfill(content_type='application/json',headers={'Access-Control-Allow-Origin':'*'},body=json.dumps(value));return
  file=assets/url.split('/updates/')[-1].split('?')[0]
  kind='application/javascript' if file.suffix=='.js' else 'text/css' if file.suffix=='.css' else 'application/json' if file.suffix=='.json' else 'text/html'
  r.fulfill(content_type=kind,body=file.read_text())
 context.route('**/*',route);page=context.new_page();url='https://appassets.androidplatform.net/updates/index.html?inhouse_app=1'
 page.goto(url+'&settings=1');page.get_by_role('button',name='Version B').wait_for();assert page.get_by_role('button',name='Version B').get_attribute('aria-pressed')=='true'
 page.get_by_role('button',name='Version A').click();assert page.evaluate('InhouseUpdater.getState().channel')=='B'
 page.get_by_role('button',name='I understand',exact=True).click()
 page.locator('input[type=range]').focus();page.keyboard.press('End');page.keyboard.press('Tab')
 page.get_by_role('button',name='Update',exact=True).wait_for()
 assert page.locator('.isu-versions').inner_text()=='0.0.1 → 0.0.4' and page.locator('.isu-chip').inner_text()=='Version A'
 assert page.locator('.isu-changes small').all_inner_texts()==['0.0.4','0.0.3','0.0.2']
 output=root/'build/services-updater';output.mkdir(parents=True,exist_ok=True)
 for theme in ('light','dark'):
  page.emulate_media(color_scheme=theme);page.screenshot(path=str(output/f'{theme}.png'))
  assert page.evaluate('document.documentElement.scrollWidth<=innerWidth')
 page.get_by_role('button',name='Update',exact=True).click();assert page.evaluate('installs[0]')=={'url':rows[0]['apkUrl'],'sha':'a'*64}
 page.get_by_role('button',name='Continue',exact=True).wait_for()
 page.evaluate('handleInhouseUpdateResult({status:"downloading",downloadedBytes:250000,totalBytes:500000})')
 assert page.locator('progress').get_attribute('value')=='50';assert page.locator('[data-isu-install]').is_disabled()
 page.evaluate('handleInhouseUpdateResult({status:"error",message:"Checksum inválido"})');assert page.get_by_role('button',name='Try again').is_enabled()
 page.get_by_role('button',name='Later',exact=True).click();assert page.locator('[role=dialog]').count()==0
 state=page.evaluate('InhouseUpdater.getState()');assert state['snooze']['A']>page.evaluate('Date.now()+13*86400000')
 page.goto(url+'&quiet=1');page.wait_for_function('!!window.InhouseUpdater');page.wait_for_timeout(100);assert page.evaluate('offers.length')==0
 page.goto(url+'&settings=1');page.get_by_role('button',name='Version B').click();page.get_by_text('No releases on version B.').wait_for()
 page.reload();page.get_by_role('button',name='Version B').wait_for();assert page.get_by_role('button',name='Version B').get_attribute('aria-pressed')=='true'
 page.get_by_role('button',name='Version A').click();page.get_by_role('button',name='I understand',exact=True).click()
 page.locator('input[type=range]').focus();page.keyboard.press('End');page.keyboard.press('Tab')
 page.get_by_role('button',name='Update',exact=True).wait_for()
 assert page.evaluate('InhouseUpdater.getState().snooze.A')==state['snooze']['A'], 'Manual check must not erase the snooze'
 browser.close()
print('PASS: actual bundled popup, light/dark layout, exact download, progress/permission/error, persisted 14-day deferral, manual override and A/B settings')
