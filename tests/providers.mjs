import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { once } from 'node:events';
import { spawn } from 'node:child_process';
import { mkdtempSync,rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { SqliteStore } from '../reference/lib/store.js';
import { BookingLinks } from '../reference/lib/booking-management.js';
import { GOOGLE_SCOPES } from '../reference/lib/google-accounts.js';
const root=new URL('..',import.meta.url).pathname;
const binary=new URL('../target/debug/jev-calendar',import.meta.url).pathname;
async function port(){const s=createServer().listen(0,'127.0.0.1');await once(s,'listening');const p=s.address().port;await new Promise(r=>s.close(r));return p;}
async function fixture(t,{travel=false,travelDates=['2030-10-07'],learningAgeMs=0,jevLow=false,calendarRefreshMs=60_000,previewMaxAgeMs=120_000}={}){
 const dir=mkdtempSync(join(tmpdir(),'jev-rust-provider-')),path=join(dir,'fixture.sqlite'),store=new SqliteStore({path,encryptionSecret:'synthetic-key'});
 const token={access_token:'owner-token',refresh_token:'synthetic-refresh',scope:GOOGLE_SCOPES,expires_at:Date.now()+3600_000};store.connectGoogle('owner@example.test',token);const session=store.newSession();
 store.putSetting('settings',{timezone:'Europe/Rome',hours:{start:9,end:18},baseLocation:travel?'Synthetic Base Street 1':'',minimumBookingNoticeMinutes:0,minimumTravelBufferMinutes:10});
 store.putSetting('projectRules',[]);
 store.putSetting('workloadLearning',{signature:JSON.stringify(['week-intelligence-v1',{start:9,end:18},0,[],'Europe/Rome']),updatedAt:new Date(Date.now()-learningAgeMs).toISOString(),bookingNotice:{source:'fallback',minutes:0},baselineBusyMinutes:135,typicalProjectMinutes:{}});
 const state={jevLow,calls:[],events:new Map(),creationCount:0,updateCount:0,loseCreate:false,loseGet:false,loseCancel:false,missingCalendar:false,newCalendar:false,routeLimit:false,refreshGate:null};
 const provider=createServer(async(req,res)=>{const url=new URL(req.url,'http://fixture.test');let raw='';for await(const chunk of req)raw+=chunk;const body=raw&&req.headers['content-type']?.includes('json')?JSON.parse(raw):{};const bearer=req.headers.authorization||'';state.calls.push({path:url.pathname,query:url.search,method:req.method,body,bearer,etag:req.headers['if-match']});const send=(code,value)=>{res.writeHead(code,{'Content-Type':'application/json'});res.end(JSON.stringify(value));};
  if(url.pathname==='/oauth/token'){if(state.refreshGate)await state.refreshGate;const form=new URLSearchParams(raw);const identity=form.get('code')==='wrong-owner'?'other':'owner';return send(200,{access_token:identity+'-token',refresh_token:'synthetic-refresh',scope:GOOGLE_SCOPES,expires_in:3600});}
  if(url.pathname==='/calendar/v3/users/me/settings/timezone')return send(200,{value:'Europe/Rome'});
  if(url.pathname==='/calendar/v3/users/me/calendarList'){const identity=bearer==='Bearer other-token'?'other':'owner';const calendars=[{id:identity+'@example.test',primary:true,accessRole:'owner',summary:'Synthetic primary'},...Array.from({length:52},(_,n)=>({id:'shared-'+n,hidden:n===0,selected:false,accessRole:n===1?'freeBusyReader':'reader',summary:'Synthetic '+n}))];if(state.newCalendar)calendars.push({id:'new-hidden',hidden:true,accessRole:'freeBusyReader'});return send(200,{items:url.searchParams.has('pageToken')?calendars.slice(30):calendars.slice(0,30),...(!url.searchParams.has('pageToken')?{nextPageToken:'page-two'}:{})});}
  if(url.pathname==='/calendar/v3/freeBusy'){const calendars={};for(const item of body.items){if(state.missingCalendar&&item.id==='shared-0')continue;calendars[item.id]={busy:[...(item.id==='shared-0'?[{start:'2030-10-07T07:00:00.000Z',end:'2030-10-07T08:00:00.000Z'},...(state.extraBusy?[state.extraBusy]:[])]:[]),...(travel&&item.id==='shared-2'?travelDates.map(date=>({start:date+'T10:00:00.000Z',end:date+'T11:00:00.000Z'})):[])]};}return send(200,{calendars});}
  if(url.pathname==='/calendar/v3/calendars/primary/events'&&req.method==='POST'){if(state.createGate)await state.createGate;state.creationCount++;assert.match(body.id,/^[0-9a-v]{5,1024}$/);if(state.events.has(body.id))return send(409,{error:'duplicate'});const event={...body,id:body.id,etag:'revision-0',hangoutLink:'https://meet.google.com/synthetic'};state.events.set(body.id,event);if(state.loseCreate){state.loseCreate=false;state.loseGet=true;req.socket.destroy();return;}return send(200,event);}
  if(url.pathname.startsWith('/calendar/v3/calendars/primary/events/')){const id=decodeURIComponent(url.pathname.split('/').at(-1)),event=state.events.get(id);if(req.method==='GET'){if(state.loseGet){state.loseGet=false;return send(503,{error:'readback unavailable'});}return event?send(200,event):send(404,{});}if(!event)return send(404,{});if(req.headers['if-match']!==event.etag)return send(412,{});state.updateCount++;if(req.method==='DELETE'){state.events.set(id,{id,status:'cancelled'});if(state.loseCancel){state.loseCancel=false;req.socket.destroy();return;}return send(204,null);}const moved={...event,...body,etag:'revision-'+state.updateCount};state.events.set(id,moved);return send(200,moved);}
  if(/\/calendar\/v3\/calendars\/.+\/events$/.test(url.pathname)){const id=decodeURIComponent(url.pathname.split('/').at(-2));return send(200,{items:travel&&id==='shared-2'?travelDates.map(date=>({start:{dateTime:date+'T10:00:00.000Z'},end:{dateTime:date+'T11:00:00.000Z'},location:'Synthetic Event Street 2',summary:'Team1 [meeting]',status:'confirmed'})):[]});}
  if(url.pathname==='/routes'){if(state.routeLimit||(state.routeWalkFail&&body.travelMode==='WALK'))return send(429,{});if(state.routeReply)return send(200,state.routeReply(body));return send(200,{routes:[{duration:body.travelMode==='WALK'?'300s':'600s',distanceMeters:1000}]});}
  if(url.pathname==='/jev'){const answers={};for(const[key,q]of Object.entries(body.questions)){const choices=Object.keys(q.criteria),choice=key==='travel_mode'?'WALK':key==='extra_buffer'?'MIN_5':key==='booking_notice'?'BASE':'cluster_meetings';answers[key]={type:'choice',choice,confidence:state.jevLow?.2:.9,probabilities:Object.fromEntries(choices.map(c=>[c,c===choice?1:0]))};}return send(200,{model:'jev-1.13.0',answers});}
  if(url.pathname==='/photon')return send(200,{features:[{properties:{osm_type:'N',osm_id:42,street:'Synthetic Street',housenumber:'1',city:'Rome',country:'Italy'},geometry:{coordinates:[12.5,41.9]}}]});
  return send(404,{});
 }).listen(0,'127.0.0.1');await once(provider,'listening');const providerOrigin=`http://127.0.0.1:${provider.address().port}`;const p=await port(),origin=`http://127.0.0.1:${p}`;
 const processOptions={cwd:root+'reference',env:{PATH:process.env.PATH,PORT:String(p),DATABASE_PATH:path,APP_ENCRYPTION_KEY:'synthetic-key',DEMO_MODE:'false',APP_BASE_URL:origin,GOOGLE_CLIENT_ID:'synthetic-client',GOOGLE_CLIENT_SECRET:'synthetic-secret',GOOGLE_ROUTES_API_KEY:'synthetic-routes',TYPESAFE_API_KEY:'synthetic-jev',JEV_TEST_MODE:'true',JEV_TEST_CALENDAR_REFRESH_MS:String(calendarRefreshMs),JEV_TEST_PREVIEW_MAX_AGE_MS:String(previewMaxAgeMs),JEV_TEST_PROVIDER_ORIGIN:providerOrigin},stdio:['ignore','pipe','pipe']};let child=spawn(binary,['--private'],processOptions);let logs='';child.stderr.on('data',d=>logs+=d);await Promise.race([once(child.stdout,'data'),once(child,'exit').then(()=>{throw Error('Failed startup: '+logs)})]);
 t.after(async()=>{if(child.exitCode===null){child.kill();await once(child,'exit');}provider.closeAllConnections();await new Promise(r=>provider.close(r));store.db.close();rmSync(dir,{recursive:true,force:true});});
 const api=(path,body,headers={})=>fetch(origin+path,{method:body===undefined?'GET':'POST',headers:{'Content-Type':'application/json',...headers},...(body!==undefined?{body:JSON.stringify(body)}:{}),redirect:'manual'});
 const owner={Cookie:`jev_session=${session}`,Origin:origin,'X-Jev-Owner':'1'};
 return{state,store,api,origin,owner,get child(){return child;},providerOrigin,async restart(){child.kill();await once(child,'exit');child=spawn(binary,['--private'],processOptions);child.stderr.on('data',d=>logs+=d);await Promise.race([once(child.stdout,'data'),once(child,'exit').then(()=>{throw Error('Failed restart: '+logs)})]);}};
}
test('real adapters paginate every calendar, fail closed and discover new hidden calendars',async t=>{const f=await fixture(t);const input={date:'2030-10-07',viewerTimeZone:'Europe/Rome',durationMinutes:30};let r=await f.api('/api/availability',input);assert.equal(r.status,200,await r.clone().text());let v=await r.json();assert.ok(v.candidates.every(c=>Date.parse(c.start)>=Date.parse('2030-10-07T08:10:00Z')));assert.ok(f.state.calls.some(c=>c.path.endsWith('calendarList')&&c.query.includes('pageToken')));assert.ok(f.state.calls.filter(c=>c.path.endsWith('freeBusy')).every(c=>c.body.items.length<=50));assert.ok(f.state.calls.some(c=>c.path.endsWith('freeBusy')&&c.body.items.some(i=>i.id==='shared-1')));f.state.newCalendar=true;await f.api('/api/availability',input);assert.ok(f.state.calls.some(c=>c.path.endsWith('freeBusy')&&c.body.items.some(i=>i.id==='new-hidden')));f.state.missingCalendar=true;r=await f.api('/api/availability',input);assert.equal(r.status,400);assert.match((await r.json()).error,/every Google calendar/);});
test('Google creation survives lost response, replays once and preserves event and Meet during changes',async t=>{const f=await fixture(t);const input={date:'2030-10-07',viewerTimeZone:'Europe/Rome',durationMinutes:30};const slots=await(await f.api('/api/availability',input)).json();const body={...input,...slots.candidates[0],idempotencyKey:'lost-creation-synthetic-key',guestName:'Synthetic Guest',guestEmail:'guest@example.test',meetingTitle:'Synthetic test'};f.state.loseCreate=true;let r=await f.api('/api/book',body);assert.equal(r.status,503,await r.clone().text());await f.restart();r=await f.api('/api/book',body);assert.equal(r.status,200,await r.clone().text());const booking=(await r.json()).booking;assert.equal(f.state.creationCount,1);assert.equal(booking.meetLink,'https://meet.google.com/synthetic');r=await f.api('/api/book',body);assert.equal(r.status,200);assert.equal(f.state.creationCount,1);const token=new URLSearchParams(new URL(booking.managementUrl).hash.slice(1)).get('token');const view=await(await f.api('/api/booking/view',{token})).json();const next=await(await f.api('/api/booking/slots',{token,date:'2030-10-08',viewerTimeZone:'Europe/Rome'})).json();r=await f.api('/api/booking/change',{token,action:'reschedule',revision:view.revision,date:'2030-10-08',viewerTimeZone:'Europe/Rome',...next.candidates[0]});assert.equal(r.status,200,await r.clone().text());assert.equal((await r.json()).meetLink,booking.meetLink);assert.equal(f.state.events.get(booking.eventId).hangoutLink,booking.meetLink);assert.ok(f.state.calls.some(c=>c.method==='PATCH'&&c.etag==='revision-0'&&c.query.includes('sendUpdates=all')));f.state.loseCancel=true;r=await f.api('/api/booking/change',{token,action:'cancel',revision:1});assert.equal(r.status,200);assert.equal((await r.json()).status,'cancelled');assert.equal(f.state.creationCount,1);assert.equal(f.store.get(body.idempotencyKey).status,'cancelled');});
test('OAuth rejects wrong owner and replay; refresh persists compatible credentials',async t=>{const f=await fixture(t);let r=await f.api('/auth/google');assert.equal(r.status,302);const url=new URL(r.headers.get('location')),state=url.searchParams.get('state');assert.ok(url.searchParams.get('scope').includes('calendar.events.freebusy'));r=await fetch(f.origin+'/auth/google/callback?'+new URLSearchParams({state,code:'wrong-owner'}),{headers:{Cookie:`jev_oauth=${state}`},redirect:'manual'});assert.equal(r.status,400);assert.match((await r.json()).error,/original dashboard owner/);r=await f.api('/auth/google');const second=new URL(r.headers.get('location')).searchParams.get('state');const callback='/auth/google/callback?'+new URLSearchParams({state:second,code:'owner'});r=await fetch(f.origin+callback,{headers:{Cookie:`jev_oauth=${second}`},redirect:'manual'});assert.equal(r.status,302);assert.ok(r.headers.get('set-cookie').includes('jev_session='));r=await fetch(f.origin+callback,{headers:{Cookie:`jev_oauth=${second}`},redirect:'manual'});assert.equal(r.status,400);const a=f.store.googleAccount('owner@example.test');f.store.refreshGoogle(a.id,a.version,{...a.token,expires_at:0});r=await f.api('/api/availability',{date:'2030-10-07'});assert.equal(r.status,200);assert.ok(f.store.googleAccount(a.id).token.expires_at>Date.now());assert.ok(f.state.calls.some(c=>c.path==='/oauth/token'));});

test('concurrent conflicting guests produce one provider event',async t=>{const f=await fixture(t);const input={date:'2030-10-07',viewerTimeZone:'Europe/Rome',durationMinutes:30};const slots=await(await f.api('/api/availability',input)).json();const body={...input,...slots.candidates[0],guestName:'Synthetic Guest',guestEmail:'guest@example.test'};const replies=await Promise.all(['race-one-synthetic','race-two-synthetic'].map(idempotencyKey=>f.api('/api/book',{...body,idempotencyKey})));assert.deepEqual(replies.map(r=>r.status).sort(),[201,400]);assert.equal(f.state.creationCount,1);});
test('stale refresh cannot resurrect a disconnected or replaced account',async t=>{const f=await fixture(t);await new Promise(r=>setTimeout(r,300));const account=f.store.googleAccount('owner@example.test');f.store.refreshGoogle(account.id,account.version,{...account.token,expires_at:0});let release;f.state.refreshGate=new Promise(r=>release=r);const before=f.state.calls.filter(c=>c.path==='/oauth/token').length;const pending=f.api('/api/availability',{date:'2030-10-07'});for(let i=0;i<100&&!f.state.calls.filter(c=>c.path==='/oauth/token').slice(before).length;i++)await new Promise(r=>setTimeout(r,20));assert.ok(f.state.calls.filter(c=>c.path==='/oauth/token').length>before);f.store.disconnectGoogle(account.id);release();assert.equal((await pending).status,400);assert.equal(f.store.googleAccount(account.id),null);});

test('provider success with local confirmation failure reconciles without a second event',async t=>{const f=await fixture(t);const input={date:'2030-10-07',viewerTimeZone:'Europe/Rome',durationMinutes:30};const slots=await(await f.api('/api/availability',input)).json();const body={...input,...slots.candidates[0],guestName:'Synthetic Guest',guestEmail:'guest@example.test',idempotencyKey:'local-commit-failure-key'};f.store.db.exec("CREATE TRIGGER reject_confirmation BEFORE UPDATE OF status ON bookings WHEN NEW.status='confirmed' BEGIN SELECT RAISE(ABORT,'synthetic confirmation failure'); END;");const failed=await f.api('/api/book',body);assert.ok(failed.status>=400);assert.equal(f.state.creationCount,1);assert.equal(f.store.get(body.idempotencyKey).status,'pending');f.store.db.exec('DROP TRIGGER reject_confirmation');const replay=await f.api('/api/book',body);assert.equal(replay.status,200,await replay.clone().text());assert.equal(f.state.creationCount,1);assert.equal(f.store.get(body.idempotencyKey).status,'confirmed');});
test('SIGTERM drains an in-flight Google creation before exiting',async t=>{const f=await fixture(t);const input={date:'2030-10-07',viewerTimeZone:'Europe/Rome',durationMinutes:30};const slots=await(await f.api('/api/availability',input)).json();let release;f.state.createGate=new Promise(r=>release=r);const body={...input,...slots.candidates[0],guestName:'Synthetic Guest',guestEmail:'guest@example.test',idempotencyKey:'drain-in-flight-creation'};const result=f.api('/api/book',body);for(let i=0;i<100&&!f.state.calls.some(c=>c.path==='/calendar/v3/calendars/primary/events'&&c.method==='POST');i++)await new Promise(r=>setTimeout(r,20));assert.ok(f.state.calls.some(c=>c.path==='/calendar/v3/calendars/primary/events'&&c.method==='POST'));const exit=once(f.child,'exit');f.child.kill('SIGTERM');await new Promise(r=>setTimeout(r,100));assert.equal(f.child.exitCode,null);release();assert.equal((await result).status,201);await exit;assert.equal(f.store.get(body.idempotencyKey).status,'confirmed');});

test('expired or tampered management capability fails before provider access',async t=>{const f=await fixture(t);const key='synthetic-expired-booking';f.store.db.prepare("INSERT INTO bookings (idempotency_key,start,end,buffer_minutes,status,event_id,created_at) VALUES (?,?,?,?,?,?,?)").run(key,'2020-10-07T07:00:00Z','2020-10-07T07:30:00Z',10,'confirmed','synthetic-expired-id','2020-10-01T00:00:00Z');const token=new BookingLinks({secret:'synthetic-key',publicOrigin:'https://book.example.test'}).token(key);const before=f.state.calls.filter(c=>c.path.includes('synthetic-expired-id')).length;let r=await f.api('/api/booking/view',{token});assert.equal(r.status,400);assert.match((await r.json()).error,/no longer available/);r=await f.api('/api/booking/view',{token:token.slice(0,-1)+'!'});assert.equal(r.status,400);assert.match((await r.json()).error,/invalid/);assert.equal(f.state.calls.filter(c=>c.path.includes('synthetic-expired-id')).length,before);});


test('public availability uses saved advice and never calls Jev or routing', async t => {
  const f = await fixture(t, {travel: true});
  f.state.routeLimit = true;
  f.state.jevLow = true;
  f.store.putSetting('settings', {...f.store.getSetting('settings'), baseLocation: ''});
  const response = await f.api('/api/availability', {date: '2030-10-07', durationMinutes: 30});
  assert.equal(response.status, 200, await response.clone().text());
  const value = await response.json();
  assert.ok(value.candidates.length);
  assert.equal(value.travel.routeProvider, 'none');
  assert.equal(value.travel.adviceSource, 'none');
  assert.equal(value.travel.maximumBufferMinutes, 10);
  assert.ok(f.state.calls.every(call => call.path !== '/routes' && call.path !== '/jev'));
  const addresses = await f.api('/api/address-search', {input:'Synthetic Street', languageCode:'en'}, f.owner);
  assert.equal(addresses.status, 200);
  assert.ok((await addresses.json()).suggestions[0].selectionToken);
});

test('background Jev refresh is due at six hours, stays private and survives a restart', async t => {
  const f = await fixture(t, {learningAgeMs: 6*60*60*1000+1, jevLow: true});
  for (let n=0;n<100 && f.store.getSetting('workloadLearning').bookingNotice.source !== 'fallback';n++)
    await new Promise(resolve=>setTimeout(resolve,20));
  for (let n=0;n<100 && !f.state.calls.some(call=>call.path==='/jev');n++)
    await new Promise(resolve=>setTimeout(resolve,20));
  assert.ok(f.state.calls.some(call=>call.path==='/jev'), 'A stale record refreshes without a page visit');
  for (let n=0;n<100 && Date.now()-Date.parse(f.store.getSetting('workloadLearning').updatedAt)>5000;n++)
    await new Promise(resolve=>setTimeout(resolve,20));
  assert.ok(Date.now()-Date.parse(f.store.getSetting('workloadLearning').updatedAt)<5000);
  assert.equal(f.store.getSetting('workloadLearning').bookingNotice.source, 'fallback');
  const before = f.state.calls.filter(call=>call.path==='/jev').length;
  for (const call of f.state.calls.filter(call=>call.path==='/jev')) {
    assert.ok(!JSON.stringify(call.body.state).includes('Synthetic'));
    assert.ok(!JSON.stringify(call.body.state).includes('@example'));
    assert.ok(!JSON.stringify(call.body.state).includes('Team1'));
    assert.ok(!call.body.questions.travel_mode);
  }
  await f.restart();
  const response = await f.api('/api/availability', {date:'2030-10-07'});
  assert.equal(response.status,200);
  assert.equal(f.state.calls.filter(call=>call.path==='/jev').length, before);
});

test('month results include every eligible day and booking still rejects newly occupied slots', async t => {
  const f = await fixture(t);
  const month = new Date().toISOString().slice(0,7);
  const response = await f.api('/api/available-dates', {month, durationMinutes:30,viewerTimeZone:'Europe/Rome'});
  assert.equal(response.status,200,await response.clone().text());
  const value = await response.json();
  assert.equal(value.partial,false);
  assert.ok(value.dates.length);
  assert.equal(value.firstDateAvailability.date,value.dates[0]);
  assert.deepEqual(Object.keys(value.availabilityByDate).sort(), [...value.dates].sort());
  assert.deepEqual(value.availabilityByDate[value.dates[0]], value.firstDateAvailability);
  for (const date of value.dates) {
    assert.equal(value.availabilityByDate[date].date, date);
    assert.equal(value.availabilityByDate[date].viewerTimeZone, 'Europe/Rome');
    assert.equal(value.availabilityByDate[date].durationMinutes, 30);
    assert.ok(value.availabilityByDate[date].candidates.length);
  }
  assert.equal(f.state.calls.filter(call=>call.path==='/calendar/v3/freeBusy').length, 2, 'One month snapshot covers every prepared day');
  assert.ok(!JSON.stringify(value.availabilityByDate).includes('Synthetic'), 'Only public slot data is exposed');
  for (const date of value.dates.slice(0,2)) {
    const slot = value.availabilityByDate[date].candidates[0];
    f.state.extraBusy = slot;
    const booking = await f.api('/api/book', {...slot,date,durationMinutes:30,viewerTimeZone:'Europe/Rome',guestName:'Synthetic Guest',guestEmail:'guest@example.test',idempotencyKey:'new-conflict-after-preview-'+date});
    assert.ok([400,409].includes(booking.status), await booking.clone().text());
    assert.equal(f.state.creationCount,0);
  }
  assert.ok(f.state.calls.every(call=>call.path!=='/routes'&&call.path!=='/jev'));
});

async function currentPreview(f) {
  const response=await f.api('/api/available-dates',{month:new Date().toISOString().slice(0,7),durationMinutes:30,viewerTimeZone:'Europe/Rome'});
  assert.equal(response.status,200,await response.clone().text());return response.json();
}
test('a shared snapshot serves concurrent visits without Google or Jev reads',async t=>{
  const f=await fixture(t);const month=await currentPreview(f);const before=f.state.calls.length;
  const values=await Promise.all(Array.from({length:8},()=>f.api('/api/availability',{date:month.dates[0],durationMinutes:30,viewerTimeZone:'Europe/Rome'}).then(async r=>{assert.equal(r.status,200);return r.json();})));
  assert.equal(f.state.calls.length,before);assert.ok(values.every(v=>v.snapshotAgeMs>=0&&v.previewMaxAgeMs===120_000));
  assert.ok(!JSON.stringify(values).includes('Synthetic'));
});
test('background calendar refresh discovers conflicts independently of visits',async t=>{
  const f=await fixture(t,{calendarRefreshMs:150});const month=await currentPreview(f),date=month.dates[0],slot=month.firstDateAvailability.candidates[0];
  f.state.extraBusy=slot;const before=f.state.calls.filter(c=>c.path.endsWith('freeBusy')).length;
  for(let i=0;i<100&&f.state.calls.filter(c=>c.path.endsWith('freeBusy')).length===before;i++)await new Promise(r=>setTimeout(r,20));
  await new Promise(r=>setTimeout(r,80));
  const response=await f.api('/api/availability',{date,viewerTimeZone:'Europe/Rome'});assert.equal(response.status,200);const value=await response.json();
  assert.ok(!value.candidates.some(c=>c.start===slot.start));assert.equal(f.state.creationCount,0);
  assert.ok(f.state.calls.every(c=>c.path!=='/routes'&&c.path!=='/jev'));
});
test('failed background refresh retains bounded previews then fails closed',async t=>{
  const f=await fixture(t,{calendarRefreshMs:100,previewMaxAgeMs:500});const month=await currentPreview(f);
  f.state.missingCalendar=true;await new Promise(r=>setTimeout(r,200));
  let response=await f.api('/api/availability',{date:month.dates[0]});assert.equal(response.status,200);
  await new Promise(r=>setTimeout(r,350));response=await f.api('/api/availability',{date:month.dates[0]});assert.equal(response.status,400);
  assert.match((await response.json()).error,/refreshing/);
  f.state.missingCalendar=false;await new Promise(r=>setTimeout(r,250));response=await f.api('/api/availability',{date:month.dates[0]});assert.equal(response.status,200);
});
test('replaced accounts invalidate snapshots while policy changes apply immediately',async t=>{
  const f=await fixture(t);const month=await currentPreview(f),date=month.dates[0];
  const before=f.state.calls.length;
  f.store.putSetting('settings',{...f.store.getSetting('settings'),minimumBookingNoticeMinutes:10080});
  let response=await f.api('/api/availability',{date});assert.equal(response.status,200);assert.equal((await response.json()).candidates.length,0);
  assert.equal(f.state.calls.length,before);
  const account=f.store.googleAccount('owner@example.test');f.store.connectGoogle(account.id,account.token);
  response=await f.api('/api/availability',{date});assert.equal(response.status,400);assert.match((await response.json()).error,/refreshing/);
});
