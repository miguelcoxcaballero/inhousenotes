import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {createHash} from 'node:crypto';
import {execFileSync} from 'node:child_process';

const manifest=JSON.parse(readFileSync('native-assets.json','utf8'));
test('APK manifest and inline resource map describe the exact public bytes',()=>{
 const html=readFileSync('index.html','utf8');
 const hashes=JSON.parse(html.match(/id="inhouse-native-assets">(.*?)<\/script>/)[1]);
 assert.ok(Object.keys(manifest.assets).length>200);
 for(const [path,info] of Object.entries(manifest.assets)){
  assert.ok(path.startsWith('/')&&!path.includes('..')&&!path.endsWith('.html'));
  const bytes=readFileSync(`.${path}`);
  assert.equal(bytes.length,info.size,path);
  assert.equal(createHash('sha256').update(bytes).digest('hex'),info.sha256,path);
  assert.equal(hashes[path],info.sha256,path);
 }
 for(const page of ['index.html']){
  for(const [,url] of readFileSync(page,'utf8').matchAll(/(?:src|href)="([^"]+[?&](?:amp;)?native=[a-f0-9]+)"/g)){
   const resolved=new URL(url.replaceAll('&amp;','&'),`https://inhousenotes.com/${page}`);
   assert.equal(resolved.searchParams.get('native'),manifest.assets[resolved.pathname].sha256);
  }
 }
});
test('Android installer bundles verified files and rejects stale bytes',()=>{
 execFileSync('python3',['-c',`
import importlib.util,json,tempfile
from pathlib import Path
spec=importlib.util.spec_from_file_location('native','.github/scripts/install_native_assets.py')
module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
repo=Path.cwd()
with tempfile.TemporaryDirectory() as tmp:
 project=Path(tmp)/'project'; java=project/'android/app/src/main/java/com/local/inhousenotes';java.mkdir(parents=True)
 (java/'MainActivity.java').write_text('InhouseBackgroundSync.attach(this, webView);')
 module.install(project,repo)
 assert 'InhouseLocalAssets.attach(getBridge())' in (java/'MainActivity.java').read_text()
 assert (project/'android/app/src/main/assets/inhouse-native/app-v5.js').read_bytes()==(repo/'app-v5.js').read_bytes()
 fake=Path(tmp)/'fake';fake.mkdir();(fake/'asset.js').write_text('modified')
 (fake/'native-assets.json').write_text(json.dumps({'assets':{'/asset.js':{'size':0,'sha256':'0'*64}}}))
 try: module.install(project,fake)
 except ValueError as error: assert 'Stale native asset' in str(error)
 else: raise AssertionError('accepted invalid bytes')
 `],{stdio:'pipe'});
});
