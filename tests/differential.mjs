import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { SecretBox } from '../reference/lib/store.js';
import { BookingLinks } from '../reference/lib/booking-management.js';
import { AddressSelectionCodec } from '../reference/lib/address-search.js';
import * as scheduling from '../reference/lib/scheduling.js';
import * as workload from '../reference/lib/workload.js';
import { writeFileSync } from 'node:fs';
const cases=[];
function add(operation,input,expected){cases.push({operation,input,expected});}
const zones=['Europe/Rome','America/New_York','Asia/Tokyo','Australia/Sydney','Pacific/Auckland','Asia/Kathmandu','Asia/Kolkata','Pacific/Chatham','UTC'];
const days=['2030-03-09','2030-03-10','2030-03-11','2030-03-30','2030-03-31','2030-04-01','2030-10-26','2030-10-27','2030-10-28','2030-11-02','2030-11-03','2030-11-04'];
for(const zone of zones)for(const day of days){
 for(const hour of [0,2,9,18,24])add('zonedEpoch',{day,hour,zone},scheduling.zonedEpoch(day,hour,zone));
 for(const durationMinutes of [15,30,60,90])for(const bufferMinutes of [0,10,30]){
  const a=scheduling.zonedEpoch(day,10,zone),b=scheduling.zonedEpoch(day,12,zone);
  const busy=[{start:new Date(a).toISOString(),end:new Date(b).toISOString()},{start:new Date(scheduling.zonedEpoch(day,23,zone)).toISOString(),end:new Date(scheduling.zonedEpoch(day,24,zone)).toISOString()}];
  const input={day,zone,hours:{start:9,end:18},durationMinutes,bufferMinutes,busy,limit:96,notBefore:scheduling.zonedEpoch(day,9.5,zone)};
  const expected=scheduling.generateCandidates(input);add('generateCandidates',input,expected);
  for(const slot of expected.slice(0,3))add('hardConstraint',{...slot,busy,bufferMinutes},scheduling.hardConstraint({...slot,busy,bufferMinutes}));
 }
}
const rules=workload.normalizeProjectRules(workload.DEFAULT_PROJECT_RULES);
for(let pattern=0;pattern<32;pattern++){
 const day='2030-10-07',zone='Europe/Rome',hours={start:9,end:18},busy=[],events=[];
 for(let n=0;n<7;n++){
  const d=scheduling.shiftDay(day,n),start=new Date(scheduling.zonedEpoch(d,9+(pattern+n)%5,zone)).toISOString(),end=new Date(scheduling.zonedEpoch(d,11+(pattern+n)%5,zone)).toISOString();
  busy.push({start,end});if(n%2===0)busy.push({start:new Date(Date.parse(start)+30*60000).toISOString(),end});
  events.push({start,end,summary:['Team1 [meeting]','Redbridge call','THICC [focus]','Jaack Website deep work','Team1 [deadline]','Unassigned'][ (pattern+n)%6 ],calendarId:'fixture',calendarName:'fixture'});
 }
 const input={day,zone,hours,busy,events,rules,learning:{baselineBusyMinutes:135,typicalProjectMinutes:{'Team1 Italy':120}},weekBusy:busy,weekEvents:events};
 const candidates=scheduling.generateCandidates({...input,durationMinutes:30,bufferMinutes:10,limit:96});input.candidates=candidates;
 add('dayWorkload',input,workload.dayWorkload(input));add('arrangeWorkloadSlots',input,workload.arrangeWorkloadSlots(input));add('weeklyDatePreference',input,workload.weeklyDatePreference(input));
 const rank={candidates,zone,busy,meetingDay:true,feedback:[{start:candidates[0]?.start||busy[0].start,outcome:'confirmed'},{start:busy[1].start,outcome:'cancelled'}]};add('rankCandidateSlots',rank,workload.rankCandidateSlots(rank));
 const learning={...input,startDay:day,days:28};const learned=workload.learnWorkload(learning);delete learned.updatedAt;add('learnWorkload',learning,learned);
 const alert={...input,days:Array.from({length:5},(_,n)=>scheduling.shiftDay(day,n))};add('weeklyWorkloadAlert',alert,workload.weeklyWorkloadAlert(alert));
}
const secret='synthetic-cross-language-secret',plaintext=JSON.stringify({refresh_token:'synthetic-token',scope:'synthetic'}),ciphertext=new SecretBox(secret).encrypt(plaintext);
add('decrypt',{secret,ciphertext},plaintext);
const links=new BookingLinks({secret,publicOrigin:'https://example.test'});add('managementToken',{secret,key:'synthetic-booking-key'},links.token('synthetic-booking-key'));add('managementKey',{secret,token:links.token('synthetic-booking-key')},'synthetic-booking-key');
const address=new AddressSelectionCodec(secret),selection={providerId:'N:42',text:'Test Street 1, Rome, Italy',latitude:41.9,longitude:12.5};const token=address.encode(selection);add('addressDecode',{secret,token},address.decode(token));
const child=spawnSync(new URL('../target/debug/jev-calendar',import.meta.url).pathname,['--evaluate'],{input:cases.map(({operation,input})=>JSON.stringify({operation,input})).join('\n')+'\n',encoding:'utf8',maxBuffer:64*1024*1024});
assert.equal(child.status,0,child.stderr);const actual=child.stdout.trim().split('\n').map(line=>JSON.parse(line));assert.equal(actual.length,cases.length);
for(let i=0;i<cases.length;i++){try{assert.deepEqual(actual[i],cases[i].expected)}catch(e){writeFileSync(new URL('../evidence/differential-failure.json',import.meta.url),JSON.stringify({case:i,...cases[i],actual:actual[i]},null,2));throw new Error(`Case ${i} ${cases[i].operation}: ${e.message}`)}}
const encrypted=spawnSync(new URL('../target/debug/jev-calendar',import.meta.url).pathname,['--evaluate'],{input:JSON.stringify({operation:'encrypt',input:{secret,plaintext}})+'\n',encoding:'utf8'});assert.equal(encrypted.status,0,encrypted.stderr);assert.equal(new SecretBox(secret).decrypt(JSON.parse(encrypted.stdout)),plaintext);
writeFileSync(new URL('../evidence/differential-result.json',import.meta.url),JSON.stringify({cases:cases.length,pass:cases.length,crypto:'bidirectional',zones,days},null,2)+'\n');
console.log(`${cases.length} Node/Rust comparisons passed; encrypted tokens verified in both directions`);
