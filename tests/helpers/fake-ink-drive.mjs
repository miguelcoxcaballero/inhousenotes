// Authenticated transport substitute: actual multipart bodies and direct file IDs.
export function fakeInkDrive(){
 const files=new Map(),calls=[];let next=0;
 const api={files,calls,hideSearch:false,failSearch:false,loseCreate:false,hold:null};
 const json=(body,status=200)=>({body:JSON.stringify(body),status});
 api.request=async(url,options={})=>{
  const parsed=new URL(url),method=options.method||'GET',id=parsed.pathname.split('/').at(-1);
  const body=typeof options.body==='string'?options.body:await options.body?.text?.();
  calls.push({url,method,body});
  if(id==='generateIds')return json({ids:[`file-${++next}`]});
  if(parsed.searchParams.get('uploadType')==='multipart'){
   const parts=body.split(/--ink_[^\r\n]+\r\n/).slice(1);
   const meta=JSON.parse(parts[0].split('\r\n\r\n')[1].trim());
   const raw=parts[1].split('\r\n\r\n')[1].split('\r\n--ink_')[0];
   if(files.has(meta.id))return json({},409);
   files.set(meta.id,{...meta,raw,etag:`"revision-${++next}"`});
   if(api.loseCreate){api.loseCreate=false;throw new Error('Lost create response');}
   if(api.hold)await api.hold;
   return json({id:meta.id,etag:files.get(meta.id).etag});
  }
  if(parsed.searchParams.get('uploadType')==='media'){
   if(!files.has(id))return json({},404);
   if(options.headers?.['If-Match']&&options.headers['If-Match']!==files.get(id).etag)return json({},412);
   files.get(id).raw=body;files.get(id).etag=`"revision-${++next}"`;
   if(api.hold)await api.hold;
   return json({id,etag:files.get(id).etag});
  }
  if(id==='files'&&method==='POST'){
   const meta=JSON.parse(body),id=`folder-${++next}`;files.set(id,{...meta,id});return json({id});
  }
  if(id==='files'){
   const query=parsed.searchParams.get('q')||'';
   if(query.includes('ihnKind')){
    if(api.failSearch)return json({},503);
    if(api.hideSearch)return json({files:[]});
    const doc=query.match(/key='ihnDoc' and value='([^']+)'/)?.[1];
    return json({files:[...files.values()].filter(f=>f.appProperties?.ihnDoc===doc).map(({id})=>({id}))});
   }
   return json({files:[...files.values()].filter(f=>f.mimeType==='application/vnd.google-apps.folder').map(({id})=>({id}))});
  }
  if(parsed.searchParams.get('alt')==='media')return files.has(id)?{body:files.get(id).raw,status:200}:json({},404);
  if(method==='PATCH'){const existing=files.get(id)||{id};Object.assign(existing,JSON.parse(body));files.set(id,existing);return json({id});}
  return files.has(id)?json(files.get(id)):json({},404);
 };
 api.fetch=async(url,options)=>{const res=await api.request(url,options);return new Response(res.body,{status:res.status,headers:{'Content-Type':'application/json'}});};
 return api;
}
export function memoryInkStore(){
 const data=new Map();const key=(a,id)=>`${a}\0${id}`;
 return {data,read:async(_,a,id)=>structuredClone(data.get(key(a,id))||null),
  list:async(_,a)=>structuredClone([...data.values()].filter(r=>r.account===a)),
  put:async(_,a,id,value)=>{const next={...structuredClone(value),account:a,id,key:key(a,id)};data.set(next.key,next);return structuredClone(next);},
  patch:async(_,a,id,fn)=>{const next=fn(structuredClone(data.get(key(a,id))||{account:a,id,key:key(a,id)}));if(next)data.set(key(a,id),structuredClone(next));else data.delete(key(a,id));return structuredClone(next);},
  withLease:async(a,name,fn)=>fn(async()=>{})};
}
