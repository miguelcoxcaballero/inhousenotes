// Content-addressed Android resources. A newer web release can use unchanged
// APK assets; changed bytes must fall through to the normal network/SW path.
// Writes <out>/native-assets.json and <out>/index.html (native map and `native=` hashes filled in); generated files
// (wasm, offline-assets.js) are read from <out>, everything else from the tree (ARCHITECTURE §14.1).
// Usage: node scripts/build-native-assets.mjs [--out build]   (run from the repository root; the build does)
import {existsSync} from 'node:fs';
import {readFile,writeFile} from 'node:fs/promises';
import {createHash} from 'node:crypto';
import {join} from 'node:path';
const out=process.argv.includes('--out')?process.argv[process.argv.indexOf('--out')+1]:'build';
const hash=bytes=>createHash('sha256').update(bytes).digest('hex');
const read=path=>readFile(existsSync(join(out,path))?join(out,path):path);
// The font hashes inside ui.css are committed (the fonts never change); a stale one must fail the build.
const css=await readFile('vendor/fonts/ui.css','utf8');
for(const match of css.matchAll(/url\((ui-\d+\.ttf)(?:\?native=([a-f0-9]+))?\)/g)) {
 if(match[2]!==hash(await readFile(`vendor/fonts/${match[1]}`)))throw new Error(`vendor/fonts/ui.css: stale native hash for ${match[1]}`);
}
const source=await readFile(join(out,'offline-assets.js'),'utf8');
const paths=JSON.parse(source.slice(source.indexOf('=')+1).replace(/;\s*$/,''));
const assets={};
const types={js:'application/javascript',css:'text/css',wasm:'application/wasm',ttf:'font/ttf',png:'image/png',svg:'image/svg+xml',json:'application/json'};
for(const path of paths) {
 if(path.endsWith('.html')||path==='/offline-assets.js')continue;
 const bytes=await read(path.slice(1)),ext=path.split('.').pop();
 assets[path]={sha256:hash(bytes),size:bytes.length,mime:types[ext]||'application/octet-stream'};
}
await writeFile(join(out,'native-assets.json'),JSON.stringify({schemaVersion:1,assets},null,2)+'\n');
const hashes=Object.fromEntries(Object.entries(assets).map(([path,info])=>[path,info.sha256]));
let html=await readFile('index.html','utf8');
html=html.replace(/((?:src|href)=")([^"#]+)(")/g,(whole,start,url,end)=>{
 if(/^(https?:|data:|\/\/)/.test(url))return whole;
 const resolved=new URL(url,'https://inhousenotes.com/index.html'),info=assets[resolved.pathname];
 const cleaned=url.replace(/(?:&amp;|&|\?)native=(?:[a-f0-9]+|__NATIVE__)/g,'');
 if(!info)return `${start}${cleaned}${end}`;
 return `${start}${cleaned}${cleaned.includes('?')?'&amp;':'?'}native=${info.sha256}${end}`;
});
// On the v6 branch the committed map is `{}`; on main it holds the previous release's hashes. Both are replaced.
const map=/(<script type="application\/json" id="inhouse-native-assets">)[\s\S]*?(<\/script>)/;
if(!map.test(html))throw new Error('index.html: the #inhouse-native-assets block is missing');
html=html.replace(map,(_,open,close)=>`${open}${JSON.stringify(hashes)}${close}`);
if(html.includes('__NATIVE__'))throw new Error('index.html: unresolved native=__NATIVE__ placeholder');
await writeFile(join(out,'index.html'),html);
console.log(`Android bundle: ${Object.keys(assets).length} verified resources`);
