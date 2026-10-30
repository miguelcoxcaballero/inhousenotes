// Content-addressed Android resources. A newer web release can use unchanged
// APK assets; changed bytes must fall through to the normal network/SW path.
import {readFile,writeFile} from 'node:fs/promises';
import {createHash} from 'node:crypto';
const hash=bytes=>createHash('sha256').update(bytes).digest('hex');
let css=await readFile('vendor/fonts/ui.css','utf8');
for(const match of [...css.matchAll(/url\((ui-\d+\.ttf)(?:\?native=[a-f0-9]+)?\)/g)]) {
 css=css.replace(match[0],`url(${match[1]}?native=${hash(await readFile(`vendor/fonts/${match[1]}`))})`);
}
await writeFile('vendor/fonts/ui.css',css);
const source=await readFile('offline-assets.js','utf8');
const paths=JSON.parse(source.slice(source.indexOf('=')+1).replace(/;\s*$/,''));
const assets={};
const types={js:'application/javascript',css:'text/css',wasm:'application/wasm',ttf:'font/ttf',png:'image/png',svg:'image/svg+xml',json:'application/json'};
for(const path of paths) {
 if(path.endsWith('.html')||path==='/offline-assets.js')continue;
 const bytes=await readFile(`.${path}`),ext=path.split('.').pop();
 assets[path]={sha256:hash(bytes),size:bytes.length,mime:types[ext]||'application/octet-stream'};
}
await writeFile('native-assets.json',JSON.stringify({schemaVersion:1,assets},null,2)+'\n');
const hashes=Object.fromEntries(Object.entries(assets).map(([path,info])=>[path,info.sha256]));
for(const page of ['index.html']) {
 let html=await readFile(page,'utf8');
 html=html.replace(/\s*<script type="application\/json" id="inhouse-native-assets">[\s\S]*?<\/script>/,'');
 html=html.replace(/((?:src|href)=")([^"#]+)(")/g,(whole,start,url,end)=>{
  if(/^(https?:|data:|\/\/)/.test(url))return whole;
  const resolved=new URL(url,`https://inhousenotes.com/${page}`),info=assets[resolved.pathname];
  if(!info)return whole;
  const cleaned=url.replace(/(?:&amp;|&|\?)native=[a-f0-9]+/g,'');
  return `${start}${cleaned}${cleaned.includes('?')?'&amp;':'?'}native=${info.sha256}${end}`;
 });
 if(page==='index.html')html=html.replace(/(    <script src="boot-v5\.js)/,`    <script type="application/json" id="inhouse-native-assets">${JSON.stringify(hashes)}</script>\n$1`);
 await writeFile(page,html);
}
console.log(`Android bundle: ${Object.keys(assets).length} verified resources`);
