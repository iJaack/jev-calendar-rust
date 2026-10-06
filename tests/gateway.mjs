import test from 'node:test';
import assert from 'node:assert/strict';
import {createServer,request} from 'node:http';
import {once} from 'node:events';
import {spawn} from 'node:child_process';
import {createHash} from 'node:crypto';
const root=new URL('..',import.meta.url).pathname;
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
