import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import vm from 'node:vm';

const source = readFileSync(new URL('../public/assets/public.js', import.meta.url), 'utf8');
const functions = source.slice(source.indexOf('const AVAILABILITY_CACHE_MS'), source.indexOf('function renderSlots()'));
function fixture({prepared=true,allDates=false}={}) {
  let currentTime = 1000;
  const first = {date:'2026-10-09', viewerTimeZone:'Europe/Rome', hostTimezone:'Europe/Rome', candidates:[{start:'2026-10-09T13:00:00Z',end:'2026-10-09T13:30:00Z'}]};
  const calls = [], elements = new Map();
  const context = vm.createContext({
    performance:{now:()=>currentTime},
    state:{visibleMonth:new Date('2026-10-01'), durationMinutes:30, viewerTimeZone:'Europe/Rome', monthRequest:0, availabilityRequest:0, availabilityCache:new Map()},
    $: id => {if(!elements.has(id)) elements.set(id,{classList:{add(){},remove(){}},replaceChildren(){}});return elements.get(id);},
    localDateKey:()=> '2026-10-01',renderCalendar(){},setStatus(){},dayLabel:v=>v,zoneLabel:v=>v,renderSlots(){},
    request:async (path, options)=>{
      const body = JSON.parse(options.body); calls.push({path,body});
      return path==='/api/available-dates' ? {dates:['2026-10-09','2026-10-13'], recommendedDate:'2026-10-13', ...(prepared?{firstDateAvailability:first}:{}), ...(allDates?{availabilityByDate:{'2026-10-09':first,'2026-10-13':{...first,date:'2026-10-13'}}}:{})} : {...first,date:body.date};
    }
  });
  vm.runInContext(functions, context);
  return {context,calls,advance:milliseconds=>{currentTime+=milliseconds;}};
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
