// Run the preserved HTTP assertions against the Rust processes. Only launchers
// and import paths change; assertions remain byte-for-byte from the snapshot.
import { mkdirSync,readFileSync,writeFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
const root=fileURLToPath(new URL('..',import.meta.url));
const binary=fileURLToPath(new URL('../target/debug/jev-calendar',import.meta.url));
const dir=new URL('../.http-compat/',import.meta.url);mkdirSync(dir,{recursive:true});
const files=['available-dates.test.js','booking-management-routes.test.js','google-account-routes.test.js','owner-bookings-http.test.js'];
for(const file of files){let source=readFileSync(new URL(`../reference/test/${file}`,import.meta.url),'utf8');source=source.replaceAll("from '../lib/","from '../reference/lib/").replaceAll("spawn(process.execPath, ['server.js'],",`spawn(${JSON.stringify(binary)}, ['--private'],`).replaceAll("spawn(process.execPath, ['public-server.js'],",`spawn(${JSON.stringify(binary)}, ['--public'],`).replaceAll("cwd: new URL('..', import.meta.url)",`cwd: ${JSON.stringify(root+'/reference')}`);writeFileSync(new URL(file,dir),source);}
writeFileSync(new URL('package.json',dir),'{"type":"module"}\n');
const result=spawnSync(process.execPath,['--test',...files.map(f=>fileURLToPath(new URL(f,dir)))],{cwd:root,encoding:'utf8'});process.stdout.write(result.stdout);process.stderr.write(result.stderr);process.exit(result.status??1);
