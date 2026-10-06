import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import vm from 'node:vm';

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
