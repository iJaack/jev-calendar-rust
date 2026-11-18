import test from 'node:test';
import assert from 'node:assert/strict';
import {createServer,request} from 'node:http';
import {once} from 'node:events';
import {spawn} from 'node:child_process';
import {createHash} from 'node:crypto';
import {mkdirSync,mkdtempSync,readFileSync,rmSync} from 'node:fs';
import {join} from 'node:path';
const root=new URL('..',import.meta.url).pathname;
const productionCsp="default-src 'self'; base-uri 'none'; connect-src 'self'; form-action 'self'; frame-ancestors 'none'; img-src 'self' data:; object-src 'none'; script-src 'self'; style-src 'self'";

test('real gateway serves the booking page and CSP-safe assets with the unchanged production policy',async t=>{
 mkdirSync(join(root,'.local'),{recursive:true});
 const dir=mkdtempSync(join(root,'.local','gateway-csp-'));
 const children=[];
 t.after(async()=>{
  for(const child of children.reverse())if(child.exitCode===null&&child.signalCode===null){child.kill();await once(child,'exit');}
  rmSync(dir,{recursive:true,force:true});
 });
 async function start(mode,extra){
  const listener=createServer().listen(0,'127.0.0.1');await once(listener,'listening');
  const port=listener.address().port;await new Promise(resolve=>listener.close(resolve));
  assert.ok(![3030,3031].includes(port));
  const child=spawn(root+'target/debug/jev-calendar',[mode],{cwd:root,env:{PATH:process.env.PATH,PORT:String(port),PUBLIC_PORT:String(port),DEMO_MODE:'true',DATABASE_PATH:join(dir,'demo.sqlite'),APP_ENCRYPTION_KEY:'synthetic-csp-key',APP_BASE_URL:`http://127.0.0.1:${port}`,PUBLIC_HOST:'book.example.test',JEV_ASSET_ROOT:root+'public',...extra},stdio:['ignore','pipe','pipe']});
  children.push(child);
  let stderr='';child.stderr.on('data',chunk=>stderr+=chunk);
  await Promise.race([once(child.stdout,'data'),once(child,'exit').then(()=>{throw Error('CSP fixture failed startup: '+stderr);})]);
  return port;
 }
 const privatePort=await start('--private',{});
 const publicPort=await start('--public',{INTERNAL_ORIGIN:`http://127.0.0.1:${privatePort}`});
 const html=readFileSync(join(root,'public/index.html'),'utf8');
 for(const path of ['/','/assets/book-lang.js','/assets/book-orari.css']){
  const expected=readFileSync(join(root,'public',path==='/'?'index.html':path));
  const version=createHash('sha256').update(expected).digest('hex').slice(0,16);
  if(path!=='/')assert.ok(html.includes(path+'?v='+version));
  for(const url of path==='/'?[path]:[path,path+'?v='+version]){
   const response=await new Promise((resolve,reject)=>{
    const req=request({hostname:'127.0.0.1',port:publicPort,path:url,headers:{Host:'book.example.test'}},res=>{
     const chunks=[];res.on('data',chunk=>chunks.push(chunk));
     res.on('end',()=>resolve({status:res.statusCode,headers:res.headers,body:Buffer.concat(chunks)}));
    });req.on('error',reject);req.end();
   });
   assert.equal(response.status,200,url);
   assert.equal(response.headers['content-security-policy'],productionCsp,url);
   assert.equal(response.headers['x-jev-backend'],'rust');
   assert.deepEqual(response.body,expected,url);
   if(path!=='/'){
    assert.match(response.headers['content-type'],path.endsWith('.js')?/javascript/:/text\/css/);
    assert.equal(response.headers['cache-control'],url.includes('?')?'public, max-age=31536000, immutable':'public, max-age=300');
   }
  }
 }
});

test('native public gateway keeps private routes, credentials and cross-site mutations out',async t=>{
 const calls=[];const upstream=createServer((req,res)=>{calls.push({path:req.url,headers:req.headers});res.writeHead(req.url.includes('missing')?404:200,{'Content-Type':'application/json','Set-Cookie':'private=secret','ETag':'\"'+createHash('sha256').update('{}').digest('hex')+'\"'});res.end('{}');}).listen(0,'127.0.0.1');await once(upstream,'listening');
 const free=createServer().listen(0,'127.0.0.1');await once(free,'listening');const port=free.address().port;await new Promise(r=>free.close(r));
 const child=spawn(root+'target/debug/jev-calendar',['--public'],{cwd:root,env:{PATH:process.env.PATH,PUBLIC_PORT:String(port),PUBLIC_HOST:'book.jaack.me',APP_BASE_URL:'https://owner.example.test',INTERNAL_ORIGIN:`http://127.0.0.1:${upstream.address().port}`},stdio:['ignore','pipe','pipe']});
 t.after(async()=>{child.kill();await once(child,'exit');upstream.closeAllConnections();await new Promise(r=>upstream.close(r));});await Promise.race([once(child.stdout,'data'),once(child,'exit').then(()=>{throw Error('Gateway failed startup')})]);
 const call=(path,{method='GET',headers={},body=''}={})=>new Promise((resolve,reject)=>{const r=request({hostname:'127.0.0.1',port,path,method,headers:{Host:'book.jaack.me',...headers}},s=>{let text='';s.on('data',c=>text+=c);s.on('end',()=>resolve({status:s.statusCode,headers:s.headers,text}));});r.on('error',reject);r.end(body);});
 for(const path of ['/auth/google','/auth/google/callback','/api/google/accounts','/api/settings','/api/workload','/api/integrations/jaack-os/bookings']) for(const method of ['GET','POST','PUT','DELETE'])assert.equal((await call(path,{method})).status,404,path);
 for(const method of ['GET','HEAD']){const r=await call('/owner?code=secret&next=evil',{method});assert.equal(r.status,302);assert.equal(r.headers.location,'https://owner.example.test/owner');assert.equal(r.text,'');}
 assert.equal((await call('/health',{headers:{Host:'evil.test'}})).status,421);
 const h={'Content-Type':'application/json','X-Jev-Booking':'1',Origin:'https://book.jaack.me','Sec-Fetch-Site':'same-origin'};
 for(const headers of [{...h,Origin:'https://evil.test'},{...h,'Sec-Fetch-Site':'cross-site'},{...h,'X-Jev-Booking':'0'}])assert.equal((await call('/api/availability',{method:'POST',headers,body:'{}'})).status,403);
 assert.equal((await call('/api/availability',{method:'POST',headers:{...h,'Content-Type':'text/plain'},body:'{}'})).status,415);
 assert.equal((await call('/api/availability',{method:'POST',headers:h,body:'['})).status,400);
 assert.equal((await call('/api/availability',{method:'POST',headers:h,body:'['.repeat(100001)})).status,413);
 assert.equal((await call('/api/book',{method:'POST',headers:h,body:'{}'})).status,400);
 const good=await call('/api/config',{headers:{Authorization:'Bearer never-forward',Cookie:'jev_session=never-forward'}});assert.equal(good.status,200);assert.equal(good.headers['set-cookie'],undefined);assert.ok(good.headers['content-security-policy'].includes("default-src 'self'"));assert.equal(good.headers['x-jev-backend'],'rust');assert.equal(calls.at(-1).headers.authorization,undefined);assert.equal(calls.at(-1).headers.cookie,undefined);
 assert.equal(good.headers['cache-control'],'no-store');
 const version=createHash('sha256').update('{}').digest('hex').slice(0,16);
 for(const method of ['GET','HEAD']) {
  const asset=await call('/assets/public.js?v='+version,{method});
  assert.equal(asset.status,200);assert.equal(asset.headers['cache-control'],'public, max-age=31536000, immutable');
  assert.equal(asset.headers.etag,'"'+createHash('sha256').update('{}').digest('hex')+'"');
 }
 for(const path of ['/assets/public.js','/assets/public.js?v=18','/assets/public.js?v=0000000000000000'])assert.equal((await call(path)).headers['cache-control'],'public, max-age=300');
 assert.equal((await call('/assets/missing.js?v='+version)).headers['cache-control'],'no-store');
 assert.equal((await call('/?v='+version)).headers['cache-control'],'no-store');
 for(let i=0;i<30;i++)assert.equal((await call('/api/availability',{method:'POST',headers:{...h,'CF-Connecting-IP':'synthetic-rate-client'},body:'{}'})).status,200);
 const limit=await call('/api/availability',{method:'POST',headers:{...h,'CF-Connecting-IP':'synthetic-rate-client'},body:'{}'});assert.equal(limit.status,429);assert.ok(Number(limit.headers['retry-after'])>0);
});
