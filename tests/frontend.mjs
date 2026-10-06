import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import vm from 'node:vm';

const source = readFileSync(new URL('../public/assets/public.js', import.meta.url), 'utf8');
const functions = source.slice(source.indexOf('async function loadMonth('), source.indexOf('function renderSlots()'));
function fixture({prepared=true}={}) {
  const first = {date:'2026-10-09', viewerTimeZone:'Europe/Rome', hostTimezone:'Europe/Rome', candidates:[{start:'2026-10-09T13:00:00Z',end:'2026-10-09T13:30:00Z'}]};
  const calls = [], elements = new Map();
  const context = vm.createContext({
    state:{visibleMonth:new Date('2026-10-01'), durationMinutes:30, viewerTimeZone:'Europe/Rome', monthRequest:0, availabilityRequest:0},
    $: id => {if(!elements.has(id)) elements.set(id,{classList:{add(){},remove(){}},replaceChildren(){}});return elements.get(id);},
    localDateKey:()=> '2026-10-01',renderCalendar(){},setStatus(){},dayLabel:v=>v,zoneLabel:v=>v,renderSlots(){},
    request:async (path, options)=>{
      const body = JSON.parse(options.body); calls.push({path,body});
      return path==='/api/available-dates' ? {dates:['2026-10-09','2026-10-13'], recommendedDate:'2026-10-13', ...(prepared?{firstDateAvailability:first}:{})} : {...first,date:body.date};
    }
  });
  vm.runInContext(functions, context);
  return {context,calls};
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
