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
async function fixture(t,{travel=false,travelDates=['2030-10-07'],learningAgeMs=0,jevLow=false,calendarRefreshMs=60_000,previewMaxAgeMs=120_000,tripitEvents=null,travelFlag='',tripitName='Synthetic (TripIt)',bookingTypes,bookingFlag='',bookingAccounts=false}={}){
 const dir=mkdtempSync(join(tmpdir(),'jev-rust-provider-')),path=join(dir,'fixture.sqlite'),store=new SqliteStore({path,encryptionSecret:'synthetic-key'});
 const token={access_token:'owner-token',refresh_token:'synthetic-refresh',scope:GOOGLE_SCOPES,expires_at:Date.now()+3600_000};store.connectGoogle('owner@example.test',token);const session=store.newSession();
 if(bookingAccounts)store.connectGoogle('team@team.example.test',{...token,access_token:'synthetic-team-token'});
 if(bookingTypes!==undefined)store.putSetting('bookingTypes',bookingTypes);
 store.putSetting('settings',{timezone:'Europe/Rome',hours:{start:9,end:18},baseLocation:travel?'Synthetic Base Street 1':'',minimumBookingNoticeMinutes:0,minimumTravelBufferMinutes:10});
 store.putSetting('projectRules',[]);
 store.putSetting('workloadLearning',{signature:JSON.stringify(['week-intelligence-v1',{start:9,end:18},0,[],'Europe/Rome']),updatedAt:new Date(Date.now()-learningAgeMs).toISOString(),bookingNotice:{source:'fallback',minutes:0},baselineBusyMinutes:135,typicalProjectMinutes:{}});
 const state={extraCalendars:bookingAccounts?[{id:'personal-work@group.example.test',accessRole:'writer',account:'owner@example.test'},{id:'team-work@group.example.test',accessRole:'owner',account:'team@team.example.test'}]:[],eventDestinations:new Map(),jevLow,tripitEvents,calls:[],events:new Map(),creationCount:0,updateCount:0,loseCreate:false,loseGet:false,loseCancel:false,missingCalendar:false,newCalendar:false,routeLimit:false,refreshGate:null};
 const provider=createServer(async(req,res)=>{const url=new URL(req.url,'http://fixture.test');let raw='';for await(const chunk of req)raw+=chunk;const body=raw&&req.headers['content-type']?.includes('json')?JSON.parse(raw):{};const bearer=req.headers.authorization||'';state.calls.push({path:url.pathname,query:url.search,method:req.method,body,bearer,etag:req.headers['if-match']});const send=(code,value)=>{res.writeHead(code,{'Content-Type':'application/json'});res.end(JSON.stringify(value));};
  if(url.pathname==='/oauth/token'){if(state.refreshGate)await state.refreshGate;const form=new URLSearchParams(raw);const identity=form.get('code')==='wrong-owner'?'other':'owner';return send(200,{access_token:identity+'-token',refresh_token:'synthetic-refresh',scope:GOOGLE_SCOPES,expires_in:3600});}
  if(url.pathname==='/calendar/v3/users/me/settings/timezone')return send(200,{value:'Europe/Rome'});
  if(url.pathname==='/calendar/v3/users/me/calendarList'){const identity=bearer==='Bearer synthetic-team-token'?'team@team.example.test':bearer==='Bearer other-token'?'other@example.test':'owner@example.test';const calendars=[{id:identity,primary:true,accessRole:'owner',summary:'Synthetic primary'},...Array.from({length:52},(_,n)=>({id:'shared-'+n,hidden:n===0,selected:false,accessRole:n===1?'freeBusyReader':'reader',summary:'Synthetic '+n}))];calendars.push(...state.extraCalendars.filter(c=>c.account===identity));if(state.tripitEvents)calendars.push({id:'synthetic-travel-calendar',summary:tripitName,accessRole:'reader',timeZone:'UTC'});if(state.subscription)calendars.push({id:'synthetic@group.v.calendar.google.com',accessRole:state.subscriptionRole||'reader',timeZone:'Pacific/Honolulu'});if(state.newCalendar)calendars.push({id:'new-hidden',hidden:true,accessRole:'freeBusyReader'});return send(200,{items:url.searchParams.has('pageToken')?calendars.slice(30):calendars.slice(0,30),...(!url.searchParams.has('pageToken')?{nextPageToken:'page-two'}:{})});}
  if(url.pathname==='/calendar/v3/freeBusy'){if(Date.parse(body.timeMax)-Date.parse(body.timeMin)>35*86400_000)return send(400,{error:{reason:'timeRangeTooLong'}});const calendars={};for(const item of body.items){if(state.missingCalendar&&item.id==='shared-0')continue;if(item.id==='synthetic@group.v.calendar.google.com'){calendars[item.id]={errors:[{domain:'global',reason:state.subscriptionError||'notFound'}],busy:[]};continue;}calendars[item.id]={busy:[...(item.id==='synthetic-travel-calendar'&&!state.omitTripitBusy?(state.tripitEvents||[]).map(e=>({start:e.start.dateTime,end:e.end.dateTime})):[]),...(item.id==='shared-0'?[{start:'2030-10-07T07:00:00.000Z',end:'2030-10-07T08:00:00.000Z'},...(state.extraBusy?[state.extraBusy]:[])]:[]),...(travel&&item.id==='shared-2'?travelDates.map(date=>({start:date+'T10:00:00.000Z',end:date+'T11:00:00.000Z'})):[])]};}return send(200,{calendars});}
  if(/^\/calendar\/v3\/calendars\/[^/]+\/events$/.test(url.pathname)&&req.method==='POST'){if(state.createGate)await state.createGate;state.creationCount++;assert.match(body.id,/^[0-9a-v]{5,1024}$/);if(state.events.has(body.id))return send(409,{error:'duplicate'});const event={...body,id:body.id,etag:'revision-0',hangoutLink:bookingAccounts?'https://meet.example.test/synthetic':'https://meet.google.com/synthetic'};state.events.set(body.id,event);state.eventDestinations.set(body.id,{calendar:decodeURIComponent(url.pathname.split('/').at(-2)),bearer});if(state.loseCreate){state.loseCreate=false;state.loseGet=true;req.socket.destroy();return;}return send(200,event);}
  if(/^\/calendar\/v3\/calendars\/[^/]+\/events\/[^/]+$/.test(url.pathname)){const id=decodeURIComponent(url.pathname.split('/').at(-1)),destination=state.eventDestinations.get(id),calendar=decodeURIComponent(url.pathname.split('/').at(-3));const event=(!destination||(destination.calendar===calendar&&destination.bearer===bearer))?state.events.get(id):undefined;if(req.method==='GET'){if(state.loseGet){state.loseGet=false;return send(503,{error:'readback unavailable'});}return event?send(200,event):send(404,{});}if(!event)return send(404,{});if(req.headers['if-match']!==event.etag)return send(412,{});state.updateCount++;if(req.method==='DELETE'){state.events.set(id,{id,status:'cancelled'});if(state.loseCancel){state.loseCancel=false;req.socket.destroy();return;}return send(204,null);}const moved={...event,...body,etag:'revision-'+state.updateCount};state.events.set(id,moved);return send(200,moved);}
  if(/\/calendar\/v3\/calendars\/.+\/events$/.test(url.pathname)){const id=decodeURIComponent(url.pathname.split('/').at(-2));if(id==='synthetic-travel-calendar')return send(200,{items:(state.tripitEvents||[]).filter(e=>Date.parse(e.end.dateTime)>Date.parse(url.searchParams.get('timeMin'))&&Date.parse(e.start.dateTime)<Date.parse(url.searchParams.get('timeMax')))});if(id==='synthetic@group.v.calendar.google.com'){if(state.subscriptionFailure)return send(503,{});if(state.subscriptionMalformed)return send(200,{timeZone:'Pacific/Honolulu',items:[{start:{date:'invalid'},end:{date:'2030-10-09'}}]});const page=url.searchParams.has('pageToken');return send(200,{timeZone:'Pacific/Honolulu',items:page?[{start:{date:'2030-10-08'},end:{date:'2030-10-09'}}]:[{start:{dateTime:'2030-10-07T09:00:00Z'},end:{dateTime:'2030-10-07T10:00:00Z'}},...['transparent','cancelled','declined'].map(kind=>({start:{dateTime:'2030-10-07T10:00:00Z'},end:{dateTime:'2030-10-07T18:00:00Z'},...(kind==='transparent'?{transparency:kind}:kind==='cancelled'?{status:kind}:{attendees:[{self:true,responseStatus:kind}]})}))],...(!page?{nextPageToken:'events-two'}:{})});}return send(200,{items:travel&&id==='shared-2'?travelDates.map(date=>({start:{dateTime:date+'T10:00:00.000Z'},end:{dateTime:date+'T11:00:00.000Z'},location:'Synthetic Event Street 2',summary:'Team1 [meeting]',status:'confirmed'})):[]});}
  if(url.pathname==='/routes'){if(state.routeLimit||(state.routeWalkFail&&body.travelMode==='WALK'))return send(429,{});if(state.routeReply)return send(200,state.routeReply(body));return send(200,{routes:[{duration:body.travelMode==='WALK'?'300s':'600s',distanceMeters:1000}]});}
  if(url.pathname==='/jev'){const answers={};for(const[key,q]of Object.entries(body.questions)){const choices=Object.keys(q.criteria),choice=key==='travel_mode'?'WALK':key==='extra_buffer'?'MIN_5':key==='booking_notice'?'BASE':'cluster_meetings';answers[key]={type:'choice',choice,confidence:state.jevLow?.2:.9,probabilities:Object.fromEntries(choices.map(c=>[c,c===choice?1:0]))};}return send(200,{model:'jev-1.13.0',answers});}
  if(url.pathname==='/photon')return send(200,{features:[{properties:{osm_type:'N',osm_id:42,street:'Synthetic Street',housenumber:'1',city:'Rome',country:'Italy'},geometry:{coordinates:[12.5,41.9]}}]});
  return send(404,{});
 }).listen(0,'127.0.0.1');await once(provider,'listening');const providerOrigin=`http://127.0.0.1:${provider.address().port}`;const p=await port(),origin=`http://127.0.0.1:${p}`;
 const processOptions={cwd:root+'reference',env:{PATH:process.env.PATH,PORT:String(p),DATABASE_PATH:path,APP_ENCRYPTION_KEY:'synthetic-key',DEMO_MODE:'false',APP_BASE_URL:origin,GOOGLE_CLIENT_ID:'synthetic-client',GOOGLE_CLIENT_SECRET:'synthetic-secret',GOOGLE_ROUTES_API_KEY:'synthetic-routes',TYPESAFE_API_KEY:'synthetic-jev',JEV_TEST_MODE:'true',JEV_TRAVEL_TIMEZONES:travelFlag,JEV_BOOKING_TYPES:bookingFlag,...(bookingAccounts?{PUBLIC_HOST:'book.example.test'}:{}),JEV_TEST_CALENDAR_REFRESH_MS:String(calendarRefreshMs),JEV_TEST_PREVIEW_MAX_AGE_MS:String(previewMaxAgeMs),JEV_TEST_PROVIDER_ORIGIN:providerOrigin},stdio:['ignore','pipe','pipe']};let child=spawn(binary,['--private'],processOptions);let logs='';child.stderr.on('data',d=>logs+=d);await Promise.race([once(child.stdout,'data'),once(child,'exit').then(()=>{throw Error('Failed startup: '+logs)})]);
 t.after(async()=>{if(child.exitCode===null){child.kill();await once(child,'exit');}provider.closeAllConnections();await new Promise(r=>provider.close(r));store.db.close();rmSync(dir,{recursive:true,force:true});});
 const api=(path,body,headers={})=>fetch(origin+path,{method:body===undefined?'GET':'POST',headers:{'Content-Type':'application/json',...headers},...(body!==undefined?{body:JSON.stringify(body)}:{}),redirect:'manual'});
 const owner={Cookie:`jev_session=${session}`,Origin:origin,'X-Jev-Owner':'1'};
 return{state,store,api,origin,owner,get child(){return child;},providerOrigin,async restart(bookingFlag){if(bookingFlag!==undefined)processOptions.env.JEV_BOOKING_TYPES=bookingFlag;child.kill();await once(child,'exit');child=spawn(binary,['--private'],processOptions);child.stderr.on('data',d=>logs+=d);await Promise.race([once(child.stdout,'data'),once(child,'exit').then(()=>{throw Error('Failed restart: '+logs)})]);}};
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
  assert.equal(f.state.calls.filter(call=>call.path==='/calendar/v3/freeBusy').length, 8, 'One shared snapshot covers every prepared day using bounded free/busy windows');
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
  assert.ok(f.state.calls.filter(c=>c.path.endsWith('freeBusy')).every(c=>Date.parse(c.body.timeMax)-Date.parse(c.body.timeMin)<=30*86400_000));
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

test('subscribed calendars without free/busy preserve paginated timed and all-day conflicts',async t=>{
 const f=await fixture(t);f.state.subscription=true;
 const input={date:'2030-10-07',viewerTimeZone:'Europe/Rome',durationMinutes:30};
 let r=await f.api('/api/availability',input);assert.equal(r.status,200,await r.clone().text());
 const slots=(await r.json()).candidates;assert.ok(slots.length);assert.ok(slots.every(c=>Date.parse(c.end)<=Date.parse('2030-10-07T09:00:00Z')||Date.parse(c.start)>=Date.parse('2030-10-07T10:00:00Z')));
 assert.ok(slots.some(c=>Date.parse(c.start)>=Date.parse('2030-10-07T10:00:00Z')),'free, cancelled and declined entries must not block');
 assert.ok(f.state.calls.some(c=>c.path.includes('synthetic%40group.v.calendar.google.com')&&c.query.includes('pageToken')));
 r=await f.api('/api/availability',{...input,date:'2030-10-08'});assert.equal(r.status,200);const morning=(await r.json()).candidates;assert.ok(morning.length);assert.ok(morning.every(c=>Date.parse(c.end)<=Date.parse('2030-10-08T10:00:00Z')),'all-day dates use the source calendar timezone');
 for(const failure of ['subscriptionFailure','subscriptionMalformed']){f.state[failure]=true;r=await f.api('/api/availability',input);assert.equal(r.status,400);f.state[failure]=false;}
 f.state.subscriptionError='internalError';r=await f.api('/api/availability',input);assert.equal(r.status,400);
 f.state.subscriptionError='notFound';f.state.subscriptionRole='freeBusyReader';r=await f.api('/api/availability',input);assert.equal(r.status,400);
 f.state.subscriptionRole='reader';f.state.missingCalendar=true;r=await f.api('/api/availability',input);assert.equal(r.status,400);
});
test('unsupported subscription can populate a fresh background preview',async t=>{
 const f=await fixture(t,{calendarRefreshMs:100});f.state.subscription=true;
 const before=f.state.calls.length;await currentPreview(f);
 for(let i=0;i<100&&!f.state.calls.slice(before).some(c=>c.path.includes('synthetic%40group.v.calendar.google.com'));i++)await new Promise(r=>setTimeout(r,20));
 await new Promise(r=>setTimeout(r,150));
 const health=await(await f.api('/health')).json();assert.equal(health.previewReady,true);
 assert.ok(f.state.calls.slice(before).some(c=>c.path.includes('synthetic%40group.v.calendar.google.com')));
 assert.equal(f.state.creationCount,0);
});


// Pick a Monday several years ahead, independent of the machine's current day.
function travelMonday() {
 const d=new Date(Date.UTC(new Date().getUTCFullYear()+4,9,1));
 while(d.getUTCDay()!==1)d.setUTCDate(d.getUTCDate()+1);
 return d.toISOString().slice(0,10);
}
function shiftTravelDate(day,n){return new Date(Date.parse(day+'T12:00:00Z')+n*86400_000).toISOString().slice(0,10);}
function tripitFlight(summary,start,end){return{summary,location:'Synthetic terminal',start:{dateTime:start},end:{dateTime:end},status:'confirmed'};}
function syntheticJourney(monday) {
 const friday=shiftTravelDate(monday,4),saturday=shiftTravelDate(monday,5),tuesday=shiftTravelDate(monday,1);
 return[
  tripitFlight('XX100 FCO to ATL',monday+'T09:25:00Z',monday+'T20:24:00Z'),
  tripitFlight('XX101 ATL to CUN',monday+'T22:35:00Z',tuesday+'T00:55:00Z'),
  tripitFlight('XX102 CUN to ATL',friday+'T17:35:00Z',friday+'T20:14:00Z'),
  tripitFlight('XX103 ATL to FCO',friday+'T21:55:00Z',saturday+'T07:15:00Z'),
 ];
}
function assertCancunDay(value,day) {
 assert.equal(value.hostTimezone,'America/Cancun');assert.equal(value.hostTimezoneSource,'travel');
 assert.ok(value.candidates.length);
 assert.equal(value.candidates[0].start,day+'T14:00:00.000Z','09:00 Cancun');
 assert.ok(value.candidates.every(c=>Date.parse(c.start)>=Date.parse(day+'T14:00:00Z')&&Date.parse(c.end)<=Date.parse(day+'T23:00:00Z')));
}
test('TripIt Cancun hours flow through fresh availability, booking and rescheduling verification',async t=>{
 const monday=travelMonday(),tuesday=shiftTravelDate(monday,1),wednesday=shiftTravelDate(monday,2),thursday=shiftTravelDate(monday,3);
 const f=await fixture(t,{tripitEvents:syntheticJourney(monday)});
 const input={date:tuesday,viewerTimeZone:'America/Cancun',durationMinutes:30};
 let r=await f.api('/api/availability',input);assert.equal(r.status,200,await r.clone().text());
 const slots=await r.json();assertCancunDay(slots,tuesday);
 const body={...input,...slots.candidates[0],guestName:'Synthetic Guest',guestEmail:'guest@example.test',idempotencyKey:'synthetic-travel-booking'};
 r=await f.api('/api/book',body);assert.equal(r.status,201,await r.clone().text());
 const booking=(await r.json()).booking,token=new URLSearchParams(new URL(booking.managementUrl).hash.slice(1)).get('token');
 r=await f.api('/api/booking/slots',{token,date:wednesday,viewerTimeZone:'America/Cancun'});assert.equal(r.status,200);
 const next=await r.json();assertCancunDay(next,wednesday);
 const view=await(await f.api('/api/booking/view',{token})).json();
 r=await f.api('/api/booking/change',{token,action:'reschedule',revision:view.revision,date:wednesday,viewerTimeZone:'America/Cancun',...next.candidates[0]});
 assert.equal(r.status,200,await r.clone().text());
 const revision=(await r.json()).revision;
 const offered=await(await f.api('/api/availability',{...input,date:thursday})).json();assertCancunDay(offered,thursday);
 // A newly discovered journey blocks an offered slot even though its original
 // preview and Google's deliberately stale free/busy response looked free.
 const friday=shiftTravelDate(monday,4);
 f.state.omitTripitBusy=true;
 f.state.tripitEvents=[...f.state.tripitEvents.slice(0,2),
  tripitFlight('XX104 CUN to ATL',thursday+'T13:00:00Z',thursday+'T20:00:00Z'),
  tripitFlight('XX105 ATL to FCO',thursday+'T21:00:00Z',friday+'T07:00:00Z')];
 const before=f.state.updateCount;
 r=await f.api('/api/booking/change',{token,action:'reschedule',revision,date:thursday,viewerTimeZone:'America/Cancun',...offered.candidates[0]});
 assert.equal(r.status,400,await r.clone().text());assert.equal(f.state.updateCount,before);
 r=await f.api('/api/book',{...body,date:thursday,...offered.candidates[0],idempotencyKey:'synthetic-transit-rejection'});
 assert.equal(r.status,400,await r.clone().text());assert.equal(f.state.creationCount,1);
 assert.ok(f.state.calls.filter(c=>c.path.endsWith('/freeBusy')).every(c=>Date.parse(c.body.timeMax)-Date.parse(c.body.timeMin)<=30*86400_000));
});
test('TripIt inference reads old arrivals in fresh requests and shared month snapshots',async t=>{
 const monday=travelMonday(),arrivalDay=shiftTravelDate(monday,-20);
 const f=await fixture(t,{tripitEvents:[tripitFlight('XX110 FCO to CUN',arrivalDay+'T08:00:00Z',arrivalDay+'T20:00:00Z')]});
 let r=await f.api('/api/availability',{date:monday,viewerTimeZone:'America/Cancun',durationMinutes:30});assert.equal(r.status,200);
 assertCancunDay(await r.json(),monday);
 const reads=f.state.calls.filter(c=>c.path.endsWith('/synthetic-travel-calendar/events'));
 assert.ok(reads.some(c=>Date.parse(new URLSearchParams(c.query).get('timeMin'))<=Date.parse(arrivalDay+'T08:00:00Z')));
 const today=new Date().toISOString().slice(0,10),old=shiftTravelDate(today,-20);
 const g=await fixture(t,{tripitEvents:[tripitFlight('XX111 FCO to CUN',old+'T08:00:00Z',old+'T20:00:00Z')]});
 let date=shiftTravelDate(today,2);while([0,6].includes(new Date(date+'T12:00:00Z').getUTCDay()))date=shiftTravelDate(date,1);
 r=await g.api('/api/available-dates',{month:date.slice(0,7),viewerTimeZone:'America/Cancun',durationMinutes:30});assert.equal(r.status,200);
 const month=await r.json();assertCancunDay(month.availabilityByDate[date],date);
 const before=g.state.calls.length;
 r=await g.api('/api/availability',{date,viewerTimeZone:'America/Cancun',durationMinutes:30});assert.equal(r.status,200);
 assertCancunDay(await r.json(),date);assert.equal(g.state.calls.length,before,'shared snapshot retains travel history');
});
test('disabled travel and non-TripIt calendars preserve the complete legacy response',async t=>{
 const monday=travelMonday(),old=shiftTravelDate(monday,-20),events=[tripitFlight('XX120 FCO to CUN',old+'T08:00:00Z',old+'T20:00:00Z')];
 const baseline=await fixture(t),input={date:monday,viewerTimeZone:'Europe/Rome',durationMinutes:30};
 const expected=await(await baseline.api('/api/availability',input)).json();
 for(const travelFlag of ['off','0','false']) {
  const f=await fixture(t,{tripitEvents:events,travelFlag});
  assert.deepEqual(await(await f.api('/api/availability',input)).json(),expected);
 }
 const f=await fixture(t,{tripitEvents:events,tripitName:'Synthetic personal calendar'});
 assert.deepEqual(await(await f.api('/api/availability',input)).json(),expected);
});
test('a viewer date crossing Cancun and Rome selects the host date with most returned slots',async t=>{
 const monday=travelMonday(),wednesday=shiftTravelDate(monday,2),thursday=shiftTravelDate(monday,3);
 const f=await fixture(t,{tripitEvents:[tripitFlight('XX130 CUN to FCO',wednesday+'T20:00:00Z',thursday+'T07:00:00Z')]});
 const input={date:thursday,viewerTimeZone:'Pacific/Auckland',durationMinutes:30};
 let r=await f.api('/api/availability',input);assert.equal(r.status,200);
 const value=await r.json(),counts=[0,0];
 for(const slot of value.candidates){counts[Date.parse(slot.start)<Date.parse(wednesday+'T20:00:00Z')?0:1]++;}
 assert.ok(counts.every(n=>n>0),'both actual host dates must contribute');
 assert.equal(value.hostTimezone,counts[1]>counts[0]?'Europe/Rome':'America/Cancun');assert.equal(value.hostTimezoneSource,'travel');
 f.store.putSetting('settings',{...f.store.getSetting('settings'),hours:{start:9,end:9}});
 r=await f.api('/api/availability',input);assert.equal(r.status,200);
 const empty=await r.json();assert.deepEqual(empty.candidates,[]);assert.equal(empty.hostTimezone,'America/Cancun');assert.equal(empty.hostTimezoneSource,'travel');
});

/* Booking-types spec: shared synthetic setup for the Tests bullets below.
 * The binary alone handles requests; the frozen Node store only seeds SQLite.
 */
const syntheticBookingTypes = () => [
 {id:'personal',label:'Personal',accountId:'owner@example.test',calendarId:'personal-work@group.example.test',default:true},
 {id:'team1',label:'Team1',accountId:'team@team.example.test',calendarId:'team-work@group.example.test',default:false},
];
const typeFixture = (t,options={}) => fixture(t,{bookingAccounts:true,bookingTypes:syntheticBookingTypes(),...options});
async function typeInput(f,key,day=travelMonday()) {
 const input={date:day,viewerTimeZone:'Europe/Rome',durationMinutes:30};
 const response=await f.api('/api/availability',input);
 assert.equal(response.status,200,await response.clone().text());
 const slots=await response.json();assert.ok(slots.candidates.length,'synthetic fixture offers a future slot');
 return {...input,...slots.candidates[0],guestName:'Synthetic Guest',guestEmail:'guest@example.test',meetingTitle:'Synthetic booking types',idempotencyKey:key};
}
function noDestinationLeaks(value) {
 const serialized=JSON.stringify(value);
 assert.doesNotMatch(serialized,/"(?:accountId|googleAccountId|calendarId|google_account_id|calendar_id)"/);
 for(const entry of syntheticBookingTypes()) {
  assert.ok(!serialized.includes(entry.accountId),'public output hides account identity');
  assert.ok(!serialized.includes(entry.calendarId),'public output hides calendar identity');
 }
}
function createdCall(f) { return f.state.calls.find(c=>c.method==='POST'&&/\/calendars\/[^/]+\/events$/.test(c.path)); }
function assertDestination(call,type,method='POST',eventId='') {
 assert.ok(call,`${method} must reach the fake Google provider`);
 assert.equal(call.path,`/calendar/v3/calendars/${encodeURIComponent(type.calendarId)}/events${eventId?'/'+eventId:''}`);
 assert.equal(call.bearer,type.accountId==='team@team.example.test'?'Bearer synthetic-team-token':'Bearer owner-token');
}

/* Spec Tests: Config validation (valid list); active config only exposes id/label.
 * Boundary/invalid validation rules are also exercised by the binary CLI below.
 */
test('booking types: active config exposes ordered choices and default without destination leaks',async t=>{
 const f=await typeFixture(t),r=await f.api('/api/config');assert.equal(r.status,200);
 const value=await r.json();
 assert.deepEqual(value.bookingTypes,[{id:'personal',label:'Personal'},{id:'team1',label:'Team1'}]);
 assert.equal(value.defaultBookingType,'personal');noDestinationLeaks(value);
});

/* Spec Tests: Flag off (off/0/false) is byte-for-byte legacy; on without a list
 * is legacy. Invalid stored lists are wholly unconfigured; inactive input ignored.
 */
test('booking types: rollback spellings and absent/invalid lists preserve legacy config and primary creation',async t=>{
 const baseline=await fixture(t),expected=await(await baseline.api('/api/config')).text();
 const scenarios=[...['off','OFF','0','false','FaLsE'].map(bookingFlag=>({bookingFlag})),
  {bookingFlag:'on',bookingTypes:undefined},{bookingTypes:[]},{bookingTypes:[{...syntheticBookingTypes()[0],extra:true}]}];
 for(const [n,options] of scenarios.entries())await t.test(JSON.stringify(options),async t=>{
  const f=await typeFixture(t,options);assert.equal(await(await f.api('/api/config')).text(),expected);
  const body=await typeInput(f,`synthetic-inactive-${n}`);f.state.calls.length=0;
  const r=await f.api('/api/book',{...body,bookingType:{ignored:true}});assert.equal(r.status,201,await r.clone().text());
  assertDestination(createdCall(f),{accountId:'owner@example.test',calendarId:'primary'});
  const columns=f.store.db.prepare('PRAGMA table_info(bookings)').all().map(c=>c.name);
  assert.ok(columns.includes('booking_type')&&columns.includes('calendar_id'),'startup adds nullable booking-type columns even when inactive');
  const row=f.store.db.prepare('SELECT booking_type,calendar_id FROM bookings WHERE idempotency_key=?').get(body.idempotencyKey);
  assert.equal(row.booking_type,null);assert.equal(row.calendar_id,null);
 });
});

/* Spec Tests: Active personal/team1/missing type routing; stored row; no public
 * account/calendar leak. Primary is a config-only destination change.
 */
for(const [name,bookingType,index,primary] of [['personal','personal',0,false],['team1','team1',1,false],['missing',undefined,0,false],['null',null,0,false],['primary','personal',0,true]]) {
 test(`booking types: ${name} creates on its configured account and calendar and persists destination`,async t=>{
  const types=syntheticBookingTypes();if(primary)types[0].calendarId='primary';
  const f=await typeFixture(t,{bookingTypes:types}),body=await typeInput(f,'synthetic-routing-'+name);
  f.state.calls.length=0;const r=await f.api('/api/book',{...body,...(bookingType===undefined?{}:{bookingType})});
  assert.equal(r.status,201,await r.clone().text());assertDestination(createdCall(f),types[index]);
  const columns=f.store.db.prepare('PRAGMA table_info(bookings)').all().map(c=>c.name);
  assert.ok(columns.includes('booking_type')&&columns.includes('calendar_id'),'booking destination columns must exist');
  const row=f.store.db.prepare('SELECT booking_type,calendar_id,google_account_id FROM bookings WHERE idempotency_key=?').get(body.idempotencyKey);
  assert.deepEqual({...row},{booking_type:types[index].id,calendar_id:types[index].calendarId,google_account_id:types[index].accountId});
  const value=await r.json();noDestinationLeaks(value);
  const token=new URLSearchParams(new URL(value.booking.managementUrl).hash.slice(1)).get('token');
  const view=await f.api('/api/booking/view',{token});assert.equal(view.status,200,await view.clone().text());noDestinationLeaks(await view.json());
 });
}

/* Spec Tests: Unknown/non-string type rejects with 400 and ZERO provider calls;
 * disconnected or non-writable destination fails closed with zero creations.
 */
for(const bookingType of ['unknown',42,false,{},[]])test(`booking types: invalid request type ${JSON.stringify(bookingType)} is rejected before provider access`,async t=>{
 const f=await typeFixture(t),body=await typeInput(f,'synthetic-invalid-type');f.state.calls.length=0;
 const r=await f.api('/api/book',{...body,bookingType});const value=await r.json();
 assert.equal(r.status,400,JSON.stringify(value));assert.match(value.error,/booking\s*type/i);
 assert.equal(f.state.calls.length,0);assert.equal(f.state.creationCount,0);noDestinationLeaks(value);
});
for(const reason of ['disconnected','missing calendar','reader','freeBusyReader'])test(`booking types: ${reason} destination cannot create or silently fall back`,async t=>{
 const f=await typeFixture(t),body=await typeInput(f,'synthetic-closed-destination');
 if(reason==='disconnected')f.store.disconnectGoogle('team@team.example.test');
 else if(reason==='missing calendar')f.state.extraCalendars=f.state.extraCalendars.filter(c=>c.account!=='team@team.example.test');
 else f.state.extraCalendars.find(c=>c.account==='team@team.example.test').accessRole=reason;
 f.state.calls.length=0;const r=await f.api('/api/book',{...body,bookingType:'team1'}),value=await r.json();
 assert.ok(r.status>=400&&r.status<500,JSON.stringify(value));assert.match(value.error,reason==='disconnected'?/connect|account/i:/calendar|writ|access/i);
 assert.equal(f.state.creationCount,0);assert.equal(createdCall(f),undefined);noDestinationLeaks(value);
});

/* Spec Tests: Stored row's calendar/account survives reschedule, cancel and flag
 * rollback; legacy NULL uses primary. Seed columns here to isolate routing from
 * migration (the separate CLI test verifies migration by the unmodified binary).
 */
function seedTypedBooking(f,{legacy=false}={}) {
 const columns=f.store.db.prepare('PRAGMA table_info(bookings)').all().map(c=>c.name);
 for(const name of ['booking_type','calendar_id'])if(!columns.includes(name))f.store.db.exec(`ALTER TABLE bookings ADD COLUMN ${name} TEXT`);
 const key='synthetic-stored-type',id='syntheticstored123',day=travelMonday(),type=syntheticBookingTypes()[1];
 const start=day+'T12:00:00.000Z',end=day+'T12:30:00.000Z',calendar=legacy?'primary':type.calendarId;
 f.store.db.prepare("INSERT INTO bookings (idempotency_key,start,end,buffer_minutes,status,event_id,created_at,google_account_id,booking_type,calendar_id,revision) VALUES (?,?,?,?,?,?,?,?,?,?,0)").run(key,start,end,10,'confirmed',id,new Date().toISOString(),type.accountId,legacy?null:type.id,legacy?null:calendar);
 f.state.events.set(id,{id,etag:'revision-0',start:{dateTime:start},end:{dateTime:end},hangoutLink:'https://meet.example.test/synthetic'});
 f.state.eventDestinations.set(id,{calendar,bearer:'Bearer synthetic-team-token'});
 const token=new BookingLinks({secret:'synthetic-key',publicOrigin:'https://book.example.test'}).token(key);
 return {token,key,id,type:{...type,calendarId:calendar},day};
}
for(const flag of ['on','off'])for(const legacy of [false,true])for(const action of ['view','reschedule','cancel']) {
 test(`booking types: stored ${legacy?'NULL primary':'calendar'} ${action} with flag ${flag} retains original destination`,async t=>{
  const f=await typeFixture(t,{bookingFlag:flag}),b=seedTypedBooking(f,{legacy});
  // Changing both the default and configured destination must not reroute a row.
  f.store.putSetting('bookingTypes',[{...syntheticBookingTypes()[0],default:true}]);
  f.state.calls.length=0;
  const next=shiftTravelDate(b.day,1),payload=action==='reschedule'?{date:next,start:next+'T12:00:00.000Z',end:next+'T12:30:00.000Z',viewerTimeZone:'Europe/Rome'}:{};
  const r=await f.api(action==='view'?'/api/booking/view':'/api/booking/change',{token:b.token,...(action==='view'?{}:{action,revision:0,...payload})});
  assertDestination(f.state.calls.find(c=>c.method==='GET'&&c.path.endsWith('/'+b.id)),b.type,'GET',b.id);
  assert.equal(r.status,200,await r.clone().text());noDestinationLeaks(await r.json());
  if(action!=='view')assertDestination(f.state.calls.find(c=>c.method===(action==='cancel'?'DELETE':'PATCH')),b.type,action==='cancel'?'DELETE':'PATCH',b.id);
  if(action==='reschedule') {
   const row=f.store.db.prepare('SELECT booking_type,calendar_id,google_account_id FROM bookings WHERE idempotency_key=?').get(b.key);
   assert.deepEqual({...row},{booking_type:legacy?null:'team1',calendar_id:legacy?null:b.type.calendarId,google_account_id:b.type.accountId});
  }
 });
}

/* Spec Tests: Availability still queries ALL calendars of ALL accounts, including
 * both destinations; booking and reschedule fresh checks keep this coverage.
 */
function assertAllTypeCalendars(calls) {
 for(const type of syntheticBookingTypes()) {
  const bearer=type.accountId==='owner@example.test'?'Bearer owner-token':'Bearer synthetic-team-token';
  assert.ok(calls.some(c=>c.path.endsWith('/calendarList')&&c.bearer===bearer));
  for(const id of [type.calendarId,type.accountId,'shared-0','shared-51'])assert.ok(calls.some(c=>c.path.endsWith('/freeBusy')&&c.bearer===bearer&&c.body.items.some(i=>i.id===id)),`freeBusy includes ${id} under correct token`);
 }
}
test('booking types: previews and fresh booking verification read every account and target calendar',async t=>{
 const f=await typeFixture(t),body=await typeInput(f,'synthetic-all-calendars');assertAllTypeCalendars(f.state.calls);
 f.state.calls.length=0;const r=await f.api('/api/book',{...body,bookingType:'personal'});assert.equal(r.status,201,await r.clone().text());assertAllTypeCalendars(f.state.calls);
 const b=seedTypedBooking(f,{legacy:true});f.state.calls.length=0;const next=shiftTravelDate(b.day,2);
 const changed=await f.api('/api/booking/change',{token:b.token,action:'reschedule',revision:0,date:next,start:next+'T12:00:00.000Z',end:next+'T12:30:00.000Z',viewerTimeZone:'Europe/Rome'});
 assert.equal(changed.status,200,await changed.clone().text());assertAllTypeCalendars(f.state.calls);
});

/* Spec Tests: Idempotent replay with changed bookingType rejects like changed
 * fields; exact replay is stable and no additional provider event is dispatched.
 */
test('booking types: a changed type cannot replay an existing idempotency key',async t=>{
 const f=await typeFixture(t),body={...await typeInput(f,'synthetic-type-idempotency'),bookingType:'personal'};
 let r=await f.api('/api/book',body);assert.equal(r.status,201,await r.clone().text());
 r=await f.api('/api/book',body);assert.equal(r.status,200,await r.clone().text());assert.equal(f.state.creationCount,1);
 f.state.calls.length=0;r=await f.api('/api/book',{...body,bookingType:'team1'});
 assert.equal(r.status,400,await r.clone().text());assert.match((await r.json()).error,/different|changed|already used/i);
 assert.equal(f.state.creationCount,1);assert.equal(f.state.calls.length,0);
});

/* Public API/UI spec: preview_fingerprint includes stored bookingTypes; changing
 * configuration invalidates an otherwise reusable snapshot without a restart.
 */
test('booking types: changing the stored list invalidates a warmed calendar preview',async t=>{
 const f=await typeFixture(t),month=await currentPreview(f);
 f.store.putSetting('bookingTypes',syntheticBookingTypes().map(v=>({...v,label:v.label+' changed'})));
 const r=await f.api('/api/availability',{date:month.dates[0],viewerTimeZone:'Europe/Rome'});
 assert.equal(r.status,400,await r.clone().text());assert.match((await r.json()).error,/refresh|calendar changed/i);
});

/* Spec Tests: Config defaults need not be the first entry; unknown flag values
 * enable the feature. Missing/null requests choose the configured default.
 */
for(const bookingType of [undefined,null])test(`booking types: ${bookingType===undefined?'missing':'null'} type honors a non-first default`,async t=>{
 const types=syntheticBookingTypes().map(v=>({...v,default:v.id==='team1'}));
 const f=await typeFixture(t,{bookingTypes:types,bookingFlag:'enabled'}),body=await typeInput(f,'synthetic-nonfirst-default');
 const r=await f.api('/api/book',{...body,...(bookingType===undefined?{}:{bookingType})});
 assert.equal(r.status,201,await r.clone().text());assertDestination(createdCall(f),types[1]);
});

/* Spec Tests: No accountId/calendarId leaks in public config, booking, manage,
 * slots or change responses. Kept independent of the routing assertions.
 */
test('booking types: every public lifecycle response hides destination identities',async t=>{
 const f=await typeFixture(t),body=await typeInput(f,'synthetic-public-destination-redaction');
 const config=await f.api('/api/config');noDestinationLeaks(await config.json());
 const created=await f.api('/api/book',{...body,bookingType:'team1'});assert.equal(created.status,201);
 const value=await created.json();noDestinationLeaks(value);
 const token=new URLSearchParams(new URL(value.booking.managementUrl).hash.slice(1)).get('token');
 for(const [path,input] of [['/api/booking/view',{token}],['/api/booking/slots',{token,date:shiftTravelDate(body.date,1),viewerTimeZone:'Europe/Rome'}],['/api/booking/change',{token,action:'cancel',revision:0}]]) {
  const response=await f.api(path,input);assert.equal(response.status,200,await response.clone().text());noDestinationLeaks(await response.json());
 }
});
