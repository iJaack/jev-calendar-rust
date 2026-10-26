import test from 'node:test';
import assert from 'node:assert/strict';
import {existsSync,readFileSync} from 'node:fs';
import vm from 'node:vm';
import {createHash} from 'node:crypto';

const source = readFileSync(new URL('../public/assets/public.js', import.meta.url), 'utf8');
const functions = source.slice(source.indexOf('const AVAILABILITY_CACHE_MS'), source.indexOf('function renderSlots()'));
function fixture({prepared=true,allDates=false}={}) {
  let currentTime = 1000;
  const timers = new Map(); let timerId = 0;
  const first = {date:'2026-10-09', viewerTimeZone:'Europe/Rome', hostTimezone:'Europe/Rome', snapshotAgeMs:0,previewMaxAgeMs:120_000,candidates:[{start:'2026-10-09T13:00:00Z',end:'2026-10-09T13:30:00Z'}]};
  const calls = [], elements = new Map();
  const context = vm.createContext({
    document:{visibilityState:'hidden'},
    performance:{now:()=>currentTime},
    setTimeout:(fn,delay)=>{const id=++timerId;timers.set(id,{fn,due:currentTime+delay});return id;},
    clearTimeout:id=>timers.delete(id),showCalendar(){},
    state:{visibleMonth:new Date('2026-10-01'), durationMinutes:30, viewerTimeZone:'Europe/Rome', monthRequest:0, availabilityRequest:0, availabilityCache:new Map()},
    $: id => {if(!elements.has(id)) elements.set(id,{classList:{add(){},remove(){}},replaceChildren(){}});return elements.get(id);},
    localDateKey:()=> '2026-10-01',renderCalendar(){},setStatus(){},dayLabel:v=>v,zoneLabel:v=>v,renderSlots(){},
    request:async (path, options)=>{
      const body = JSON.parse(options.body); calls.push({path,body});
      return path==='/api/available-dates' ? {dates:['2026-10-09','2026-10-13'], recommendedDate:'2026-10-13', ...(prepared?{firstDateAvailability:first}:{}), ...(allDates?{availabilityByDate:{'2026-10-09':first,'2026-10-13':{...first,date:'2026-10-13'}}}:{})} : {...first,date:body.date};
    }
  });
  vm.runInContext(functions, context);
  return {context,calls,advance:milliseconds=>{currentTime+=milliseconds;for(const [id,timer] of timers)if(timer.due<=currentTime){timers.delete(id);timer.fn();}}};
}
test('the earliest date opens even when Tuesday has the highest recommendation',async()=>{
  const {context,calls}=fixture(); await context.loadMonth();
  assert.equal(context.state.selectedDate,'2026-10-09');
  assert.equal(context.state.availability.date,'2026-10-09');
  assert.equal(calls.length,1,'The prepared first day avoids a duplicate calendar request');
});
test('an explicit date selection survives a month refresh',async()=>{
  const {context,calls}=fixture(); await context.loadMonth('2026-10-13');
  assert.equal(context.state.selectedDate,'2026-10-13');
  assert.equal(calls[1].body.date,'2026-10-13');
});
test('an older response without prepared slots still opens the earliest date',async()=>{
  const {context,calls}=fixture({prepared:false}); await context.loadMonth();
  assert.equal(context.state.selectedDate,'2026-10-09');
  assert.equal(calls[1].body.date,'2026-10-09');
});

test('switching between fresh month dates reuses slots without availability requests',async()=>{
  const {context,calls}=fixture({allDates:true}); await context.loadMonth();
  await context.selectDate('2026-10-13'); await context.selectDate('2026-10-09');
  assert.equal(context.state.availability.date,'2026-10-09');
  assert.equal(calls.length,1);
});
test('cache expires after sixty seconds and day changes do not extend its lifetime',async()=>{
  const {context,calls,advance}=fixture({allDates:true}); await context.loadMonth();
  advance(59_999); await context.selectDate('2026-10-13'); assert.equal(calls.length,1);
  advance(1); await context.selectDate('2026-10-13'); assert.equal(calls.length,2);
  assert.equal(calls[1].path,'/api/availability');
  await context.selectDate('2026-10-13'); assert.equal(calls.length,2,'Fresh day reads are reusable too');
});
test("duration and timezone never reuse another context's slots",async()=>{
  const {context,calls}=fixture({allDates:true}); await context.loadMonth();
  context.state.durationMinutes=60; await context.selectDate('2026-10-13');
  assert.equal(calls[1].body.durationMinutes,60);
  context.state.viewerTimeZone='America/New_York'; await context.selectDate('2026-10-13');
  assert.equal(calls[2].body.viewerTimeZone,'America/New_York');
  await context.loadMonth(); assert.equal(context.state.availabilityCache.size,2,'Month reload replaces the cache');
});
test('slow replies cannot overwrite the latest selected date or seed its cache',async()=>{
  const {context,advance}=fixture({allDates:true}); await context.loadMonth(); advance(60_000);
  let resolveEarlier; context.request=async(path,options)=>{
    const {date}=JSON.parse(options.body);
    const value={date,viewerTimeZone:'Europe/Rome',hostTimezone:'Europe/Rome',candidates:[{start:date+'T13:00:00Z'}]};
    if(date==='2026-10-09') return new Promise(resolve=>{resolveEarlier=()=>resolve(value);});
    return value;
  };
  const earlier=context.selectDate('2026-10-09'); await context.selectDate('2026-10-13');
  resolveEarlier(); await earlier;
  assert.equal(context.state.selectedDate,'2026-10-13');
  assert.equal(context.state.availability.date,'2026-10-13');
  const key=JSON.stringify(['2026-10-09',30,'Europe/Rome']);
  assert.equal(context.state.availabilityCache.get(key).expiresAt,61_000,'Stale reply did not update the expired entry');
});

test('expired fresh cache remains visible during a background recheck',async()=>{
  const f=fixture({allDates:true});await f.context.loadMonth();f.advance(65_000);
  let resolve;f.context.request=()=>new Promise(r=>{resolve=r;});
  const pending=f.context.selectDate('2026-10-13');
  assert.equal(f.context.state.availability.date,'2026-10-13','Slots show before the network resolves');
  const value=f.context.state.availability;
  f.context.state.choice=value.candidates[0];resolve(value);await pending;
  assert.equal(f.context.state.choice.start,value.candidates[0].start,'Background refresh preserves a still-valid selection');
});
test('source age bounds reuse and failed revalidation withdraws slots at the hard deadline',async()=>{
  const f=fixture({allDates:true});await f.context.loadMonth();
  const value={...f.context.state.availability,snapshotAgeMs:50_000};
  f.context.cacheAvailability(JSON.stringify(['2026-10-13',30,'Europe/Rome']),{...value,date:'2026-10-13'},1000);
  f.advance(15_000);f.context.request=async()=>{throw Error('provider unavailable');};
  await f.context.selectDate('2026-10-13');assert.ok(f.context.state.availability);
  f.advance(55_000);assert.equal(f.context.state.availability,null,'Two-minute end-to-end bound includes source age');
  assert.equal(f.context.state.choice,null);
});
test('revalidation removes a chosen slot when it becomes occupied',async()=>{
  const f=fixture({allDates:true});await f.context.loadMonth();f.advance(65_000);
  let resolve;f.context.request=()=>new Promise(r=>{resolve=r;});
  const pending=f.context.selectDate('2026-10-13');
  f.context.state.choice=f.context.state.availability.candidates[0];
  resolve({...f.context.state.availability,candidates:[]});await pending;
  assert.equal(f.context.state.choice,null);assert.equal(f.context.state.selectedDate,null);
});

test('server invalidation immediately withdraws every cached preview',async()=>{
  const f=fixture({allDates:true});await f.context.loadMonth();f.advance(65_000);
  f.context.request=async()=>{throw Object.assign(Error('Calendar changed'),{invalidatePreview:true});};
  await f.context.selectDate('2026-10-13');assert.equal(f.context.state.availability,null);
  assert.equal(f.context.state.availabilityCache.size,0);assert.equal(f.context.state.choice,null);
});

test('a visible calendar rechecks automatically before previews expire without clearing a valid choice',async()=>{
  const f=fixture({allDates:true});f.context.document.visibilityState='visible';await f.context.loadMonth();
  f.context.state.choice=f.context.state.availability.candidates[0];f.advance(60_000);
  await new Promise(resolve=>setImmediate(resolve));
  assert.equal(f.calls.length,2);assert.equal(f.context.state.availability.date,'2026-10-09');
  assert.ok(f.context.state.choice);assert.equal(f.context.state.availabilityCache.get(JSON.stringify(['2026-10-09',30,'Europe/Rome'])).validUntil,181_000);
});
test('opening booking details stops preview timers and ignores an in-flight recheck',async()=>{
  const f=fixture({allDates:true});await f.context.loadMonth();
  const showDetails=source.slice(source.indexOf('function showDetails(slot)'),source.indexOf('function showCalendar()'));
  f.context.fullDateTime=()=> 'Synthetic selected time';
  f.context.$('guest-name').focus=()=>{};
  vm.runInContext(showDetails,f.context);
  const requestId=f.context.state.availabilityRequest,choice=f.context.state.availability.candidates[0];
  f.context.showDetails(choice);f.advance(120_000);
  assert.equal(f.context.state.choice.start,choice.start,'Entering details preserves the proposed booking time');
  assert.ok(f.context.state.availabilityRequest>requestId,'Earlier preview responses cannot interrupt details');
});

test('booking layout follows the container and the visible viewport',()=>{
  const css=readFileSync(new URL('../public/assets/app.css',import.meta.url),'utf8');
  assert.match(css,/body\.booking-page\s*\{[^}]*min-height:\s*100dvh/);
  assert.match(css,/@container \(min-width: 520px\)/);
  assert.match(css,/env\(safe-area-inset-left\)/);
  assert.match(css,/env\(safe-area-inset-right\)/);
  assert.match(css,/@media \(max-height: 500px\)/);
  assert.doesNotMatch(css,/innerWidth|navigator\.userAgent/);
  for(const page of ['index.html','privacy.html','manage.html']) {
    const html=readFileSync(new URL('../public/'+page,import.meta.url),'utf8');
    assert.match(html,/viewport-fit=cover/);
  }
});

test('booking and privacy asset versions identify the exact shipped contents',()=>{
 for(const page of ['index.html','privacy.html']) {
  const html=readFileSync(new URL('../public/'+page,import.meta.url),'utf8');
  const references=[...html.matchAll(/\/assets\/(public\.js|app\.css)\?v=([^"']+)/g)];
  assert.equal(references.length,page==='index.html'?2:1);
  for(const [,asset,version] of references) {
   const override=new URL('../public/assets/'+asset,import.meta.url);
   const path=existsSync(override)?override:new URL('../reference/public/assets/'+asset,import.meta.url);
   assert.equal(version,createHash('sha256').update(readFileSync(path)).digest('hex').slice(0,16));
  }
 }
});

/* Spec Tests: Frontend radio group rendered only when /api/config is configured,
 * required and accessible, no multi-option preselection, selected value posted,
 * confirmation label shown; content-digest asset references remain consistent.
 * Execute HTML/JS fetched from the Rust binary with only local API responses.
 */
async function bookingTypeFrontend(t,bookingTypes) {
 const [{spawn},{once},{createServer},{mkdtempSync,rmSync},{tmpdir},{join},{testDocument}]=await Promise.all([
  import('node:child_process'),import('node:events'),import('node:http'),import('node:fs'),import('node:os'),import('node:path'),import('./booking-types-dom.mjs'),
 ]);
 const root=new URL('..',import.meta.url).pathname,dir=mkdtempSync(join(tmpdir(),'jev-types-ui-'));
 const listener=createServer().listen(0,'127.0.0.1');await once(listener,'listening');const port=listener.address().port;await new Promise(r=>listener.close(r));
 const origin=`http://127.0.0.1:${port}`;
 const child=spawn(root+'target/debug/jev-calendar',['--private'],{cwd:root,env:{PATH:process.env.PATH,PORT:String(port),DATABASE_PATH:join(dir,'ui.sqlite'),APP_ENCRYPTION_KEY:'synthetic-ui-key',DEMO_MODE:'true',APP_BASE_URL:origin,JEV_ASSET_ROOT:root+'public'},stdio:['ignore','pipe','pipe']});
 t.after(async()=>{if(child.exitCode===null){child.kill();await once(child,'exit');}rmSync(dir,{recursive:true,force:true});});
 let stderr='';child.stderr.on('data',d=>stderr+=d);
 await Promise.race([once(child.stdout,'data'),once(child,'exit').then(()=>{throw Error('UI fixture startup failed: '+stderr);})]);
 const htmlResponse=await fetch(origin+'/');assert.equal(htmlResponse.status,200);const html=await htmlResponse.text();
 const script=html.match(/<script[^>]+src="([^"]*public\.js[^\"]*)"/)[1];
 const jsResponse=await fetch(origin+script);assert.equal(jsResponse.status,200);const shipped=await jsResponse.text();
 const document=testDocument(html),calls=[];
 const slot={start:'2030-10-07T12:00:00.000Z',end:'2030-10-07T12:30:00.000Z'};
 const context=vm.createContext({document,Intl,Date,URL,URLSearchParams,Map,Set,console,performance:{now:()=>1000},setTimeout:()=>1,clearTimeout(){},crypto:{randomUUID:()=> 'synthetic-ui-request-key'},
  FormData:class{constructor(form){this.entries=form.querySelectorAll('input').filter(n=>n.name&&(n.type!=='radio'||n.checked)).map(n=>[n.name,n.value]);}get(name){return this.entries.find(e=>e[0]===name)?.[1]??null;}[Symbol.iterator](){return this.entries[Symbol.iterator]();}},
  fetch:async(path,options={})=>{
   assert.ok(String(path).startsWith('/'),'frontend must only request local API paths');
   const body=options.body?JSON.parse(options.body):undefined;calls.push({path,body});
   let value={};if(path==='/api/config')value={hostTimezone:'Europe/Rome',...(bookingTypes?{bookingTypes,defaultBookingType:bookingTypes[0].id}:{})};
   else if(path==='/api/available-dates')value={dates:[]};
   else if(path==='/api/book')value={booking:{meetingTitle:'Synthetic UI booking',bookingType:body.bookingType,bookingTypeLabel:bookingTypes?.find(v=>v.id===body.bookingType)?.label,managementUrl:'https://book.example.test/manage#synthetic'}};
   else throw Error('Unexpected frontend API request '+path);
   return {ok:true,json:async()=>value};
  },
 });
 vm.runInContext(shipped,context);await new Promise(r=>setImmediate(r));
 // Select a synthetic offer, as the existing preview tests do, then use the real
 // details form and its registered submit handler without replacing that code.
 vm.runInContext(`state.selectedDate='2030-10-07';showDetails(${JSON.stringify(slot)});`,context);
 document.getElementById('guest-name').value='Synthetic Guest';document.getElementById('guest-email').value='guest@example.test';
 const form=document.getElementById('booking-form');
 return {document,calls,form,html,origin,async submit(){
  if(!form.checkValidity())return false;
  const submitter=form.querySelector('button[type="submit"]');
  for(const fn of form.listeners.submit||[])await fn({preventDefault(){},submitter,target:form});return true;
 }};
}
const publicTypeChoices=[{id:'personal',label:'Personal'},{id:'team1',label:'Team1'}];
function typeRadios(f){return f.form.querySelectorAll('input[type="radio"]');}
function assertTypeGroup(f) {
 const radios=typeRadios(f);assert.equal(radios.length,2,'configured frontend must render both booking-type radios');
 assert.deepEqual(radios.map(r=>r.value),['personal','team1']);assert.ok(radios.every(r=>r.name&&r.name===radios[0].name));
 const group=radios[0].closest('fieldset')||radios[0].closest('[role="radiogroup"]');assert.ok(group,'radio choices have a semantic group');
 const legend=group.querySelector('legend'),labelled=group.getAttribute('aria-labelledby');
 const label=legend?.textContent||group.getAttribute('aria-label')||(labelled&&f.document.getElementById(labelled)?.textContent);
 assert.equal(label?.trim(),'What is this for?');
 for(const [n,radio] of radios.entries()) {
  const label=radio.closest('label')||f.document.querySelector(`label[for="${radio.id}"]`);
  assert.ok(label?.textContent.includes(publicTypeChoices[n].label),'each radio has its visible accessible label');
 }
 return radios;
}
test('booking types frontend: configured choices are accessible, required and initially unselected',async t=>{
 const f=await bookingTypeFrontend(t,publicTypeChoices),radios=assertTypeGroup(f);
 assert.ok(radios.every(r=>!r.checked),'two choices require an explicit guest selection');
 assert.ok(radios.some(r=>r.required),'the native radio group is required');
 assert.equal(await f.submit(),false);assert.ok(!f.calls.some(c=>c.path==='/api/book'));
});
test('booking types frontend: choosing a radio posts its id and shows its label on confirmation',async t=>{
 const f=await bookingTypeFrontend(t,publicTypeChoices),radios=assertTypeGroup(f);radios[1].click();
 assert.equal(await f.submit(),true);assert.equal(f.calls.find(c=>c.path==='/api/book')?.body.bookingType,'team1');
 assert.match(f.document.getElementById('success-step').textContent,/Team1/);
});
test('booking types frontend: an unconfigured form has no choice and preserves its legacy payload',async t=>{
 const f=await bookingTypeFrontend(t);assert.equal(typeRadios(f).length,0);
 assert.doesNotMatch(f.form.textContent,/What is this for\?/);
 assert.equal(await f.submit(),true);const call=f.calls.find(c=>c.path==='/api/book');assert.ok(call);assert.ok(!('bookingType' in call.body));
});
test('booking types frontend: a single configured type renders one required choice',async t=>{
 const f=await bookingTypeFrontend(t,publicTypeChoices.slice(0,1)),radios=typeRadios(f);
 assert.equal(radios.length,1);assert.equal(radios[0].value,'personal');assert.ok(radios[0].required);
 radios[0].click();assert.equal(await f.submit(),true);assert.equal(f.calls.find(c=>c.path==='/api/book')?.body.bookingType,'personal');
});
test('booking types frontend: binary-served asset digests match every referring override page',async t=>{
 const f=await bookingTypeFrontend(t);
 for(const page of ['/','/privacy','/manage']) {
  const r=await fetch(f.origin+page);assert.equal(r.status,200);const html=await r.text();
  const refs=[...html.matchAll(/(?:src|href)="(\/assets\/(?:public\.js|app\.css)\?v=([^"&]+))"/g)];assert.ok(refs.length);
  for(const [,path,digest] of refs) {
   const asset=await fetch(f.origin+path);assert.equal(asset.status,200);const bytes=Buffer.from(await asset.arrayBuffer());
   assert.equal(digest,createHash('sha256').update(bytes).digest('hex').slice(0,16));
   assert.equal(asset.headers.get('etag'),'"'+createHash('sha256').update(bytes).digest('hex')+'"');
  }
 }
});
