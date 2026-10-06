"""Rehearse compatible backups and guarded switchback on disposable SQLite only."""
import importlib.util, json, pathlib, sqlite3, tempfile, unittest, subprocess, os
from io import BytesIO
from unittest.mock import patch
ROOT = pathlib.Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location('release', ROOT / 'scripts/release.py')
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)
restore_spec = importlib.util.spec_from_file_location('restore_reference', ROOT / 'scripts/restore_reference.py')
restore_reference = importlib.util.module_from_spec(restore_spec)
restore_spec.loader.exec_module(restore_reference)

class Recovery(unittest.TestCase):

    def test_restore_fresh_reference_and_reject_local_tampering(self):
        with tempfile.TemporaryDirectory(prefix='jev-reference-test-') as directory:
            root = pathlib.Path(directory)
            for name in ['evidence/baseline-manifest.json', 'fixtures/node-baseline.tar.gz']:
                target = root / name
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes((ROOT / name).read_bytes())
            with patch.object(restore_reference, 'ROOT', root):
                restore_reference.restore()
                restore_reference.restore()
                (root / 'reference/README.md').write_text('tampered fixture')
                with self.assertRaisesRegex(RuntimeError, 'Existing reference differs'):
                    restore_reference.restore()

    def test_package_overlays_current_frontend_and_checksums_assets(self):
        with tempfile.TemporaryDirectory(prefix='jev-package-test-') as directory:
            root = pathlib.Path(directory)
            for name, content in {
                'reference/public/assets/public.js': 'captured frontend',
                'reference/public/assets/favicon.png': 'captured image',
                'public/assets/public.js': 'current earliest-date frontend',
                'public/privacy.html': 'current privacy text',
                'target/release/jev-calendar': 'synthetic binary',
                'Cargo.lock': 'synthetic lockfile',
                'rust-toolchain.toml': 'synthetic toolchain',
            }.items():
                target = root / name
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_text(content)
            with patch.object(release, 'ROOT', root), patch.object(release, 'baseline', lambda: None):
                package = release.package()
                self.assertEqual((package / 'public/assets/public.js').read_text(), 'current earliest-date frontend')
                self.assertEqual((package / 'public/assets/favicon.png').read_text(), 'captured image')
                self.assertEqual((package / 'public/privacy.html').read_text(), 'current privacy text')
                release.verify(package)
                (package / 'public/assets/public.js').write_text('tampered release asset')
                with self.assertRaisesRegex(RuntimeError, 'Release checksum failed'):
                    release.verify(package)

    def test_current_backend_checks_both_services_and_health(self):
        def response(rust, ok=True):
            value = BytesIO(json.dumps({'ok': ok}).encode())
            value.headers = {'X-Jev-Backend': 'rust'} if rust else {}
            return value
        for rust in [False, True]:
            with patch.object(release.urllib.request, 'urlopen', side_effect=[response(rust), response(rust)]):
                self.assertEqual(release.current_backend(), rust)
        with patch.object(release.urllib.request, 'urlopen', side_effect=[response(True), response(False)]):
            with self.assertRaisesRegex(RuntimeError, 'different backends'):
                release.current_backend()
        with patch.object(release.urllib.request, 'urlopen', return_value=response(True, False)):
            with self.assertRaisesRegex(RuntimeError, 'health failed'):
                release.current_backend()

    def test_baseline_allows_status_docs_and_rejects_runtime_drift(self):
        with tempfile.TemporaryDirectory(prefix='jev-baseline-test-') as directory:
            root = pathlib.Path(directory)
            source = root / 'original'
            reference = root / 'reference'
            source.mkdir()
            reference.mkdir()
            (root / 'evidence').mkdir()
            manifest = {}
            for name, content in {'README.md': 'captured status', 'server.js': 'original runtime'}.items():
                (source / name).write_text(content)
                (reference / name).write_text(content)
                manifest[name] = release.digest(reference / name)
            (root / 'evidence/baseline-manifest.json').write_text(json.dumps(manifest))
            with patch.object(release, 'ROOT', root), patch.object(release, 'SOURCE', source):
                (source / 'README.md').write_text('Rust is now live')
                release.baseline()
                (source / 'server.js').write_text('changed runtime')
                with self.assertRaisesRegex(RuntimeError, 'Original source changed'):
                    release.baseline()
                (source / 'server.js').write_text('original runtime')
                (reference / 'README.md').write_text('corrupt oracle')
                with self.assertRaisesRegex(RuntimeError, 'Captured baseline changed'):
                    release.baseline()

    def test_backup_restore_and_current_state_switchback(self):
        with tempfile.TemporaryDirectory(prefix='jev-rust-rehearsal-') as d:
            path = pathlib.Path(d) / 'state.sqlite'
            backup = pathlib.Path(d) / 'backup.sqlite'
            env = {'PATH': os.environ['PATH'], 'DATABASE_PATH': str(path), 'APP_ENCRYPTION_KEY': 'synthetic-rehearsal-key', 'DEMO_MODE': 'true'}
            subprocess.run([str(ROOT / 'target/release/jev-calendar'), '--backup', str(pathlib.Path(d) / 'empty.sqlite')], env=env, check=True, capture_output=True)
            with sqlite3.connect(path) as db:
                db.execute("INSERT INTO bookings (idempotency_key,start,end,buffer_minutes,status,event_id,created_at) VALUES ('pre-cutover-synthetic','2030-10-07T07:00:00Z','2030-10-07T07:30:00Z',10,'confirmed','synthetic-id','2026-10-05T08:00:00Z')")
            release.backup(path, backup)
            self.assertEqual(release.safe(backup)['bookings']['confirmed'], 1)
            with sqlite3.connect(path) as db:
                db.execute("INSERT INTO bookings (idempotency_key,start,end,buffer_minutes,status,event_id,created_at) VALUES ('post-cutover-synthetic','2030-10-08T07:00:00Z','2030-10-08T07:30:00Z',10,'confirmed','synthetic-id-2','2026-10-05T09:00:00Z')")
            code = "import {SqliteStore} from './reference/lib/store.js';const db=new SqliteStore({path:process.env.DATABASE_PATH,encryptionSecret:'synthetic-rehearsal-key'});if(!db.get('pre-cutover-synthetic')||!db.get('post-cutover-synthetic'))process.exit(1);db.db.close();"
            subprocess.run(['node', '--input-type=module', '-e', code], cwd=ROOT, env=env, check=True, capture_output=True)
            with sqlite3.connect(path) as db:
                db.execute("UPDATE bookings SET status='pending' WHERE idempotency_key='post-cutover-synthetic'")
            with self.assertRaises(RuntimeError):
                release.safe(path)
            with sqlite3.connect(path) as db:
                db.execute("UPDATE bookings SET status='confirmed',pending_change='{}' WHERE idempotency_key='post-cutover-synthetic'")
            with self.assertRaises(RuntimeError):
                release.safe(path)
            self.assertEqual(release.safe(backup)['bookings']['confirmed'], 1)

    def test_launch_switchback_keeps_current_database(self):
        for previous_rust in [False, True]:
            with self.subTest(previous_rust=previous_rust):
                self.rehearse_switchback(previous_rust)

    def test_failed_rust_upgrade_restores_prior_rust_release(self):
        self.rehearse_switchback(True, failed=True)

    def rehearse_switchback(self, previous_rust, failed=False):
        with tempfile.TemporaryDirectory(prefix='jev-rust-switch-') as d:
            home = pathlib.Path(d)
            agents = home / 'Library/LaunchAgents'
            agents.mkdir(parents=True)
            path = home / 'state.sqlite'
            env = {'PATH': os.environ['PATH'], 'DATABASE_PATH': str(path), 'DEMO_MODE': 'true'}
            subprocess.run([str(ROOT / 'target/release/jev-calendar'), '--backup', str(home / 'empty.sqlite')], env=env, check=True, capture_output=True)
            candidate = home / 'candidate'
            candidate.mkdir()
            (candidate / 'manifest.json').write_text('{"files":{}}')
            original = {}
            for label in release.LABELS:
                v = {'Label': label, 'ProgramArguments': ['retained-rust-script' if previous_rust else 'retained-node-script'], 'WorkingDirectory': 'original-checkout'}
                original[label] = v
                (agents / f'{label}.plist').write_bytes(release.plistlib.dumps(v))
            loaded = set(release.LABELS)
            events = []
            def readback(rust):
                events.append(('readback', rust))
                if failed and len([e for e in events if e[0] == 'readback']) == 1:
                    raise RuntimeError('Synthetic candidate startup failure')
            with patch.object(release, 'baseline', lambda: None), \
                 patch.object(release, 'current_backend', lambda: previous_rust), \
                 patch.object(release, 'pid', lambda label: 1 if label in loaded else None), \
                 patch.object(release, 'stop', lambda label: (events.append(('stop', label)), loaded.remove(label))), \
                 patch.object(release, 'start', lambda label: (events.append(('start', label)), loaded.add(label))), \
                 patch.object(release, 'readback', readback), \
                 patch.object(release, 'loaded', lambda label: label in loaded), \
                 patch.object(release, 'ROOT', home), \
                 patch.object(pathlib.Path, 'home', return_value=home):
                if failed:
                    with self.assertRaisesRegex(RuntimeError, 'Synthetic candidate startup failure'):
                        release.switch(candidate, path)
                    self.assertEqual(events[-1], ('readback', previous_rust))
                    self.assertEqual(loaded, set(release.LABELS))
                    for label in release.LABELS:
                        self.assertEqual(release.plistlib.loads((agents / f'{label}.plist').read_bytes()), original[label])
                    return
                release.switch(candidate, path)
                for label in release.LABELS:
                    self.assertIn('run-', release.plistlib.loads((agents / f'{label}.plist').read_bytes())['ProgramArguments'][0])
                with sqlite3.connect(path) as db:
                    db.execute("INSERT INTO bookings (idempotency_key,start,end,buffer_minutes,status,created_at) VALUES ('new-after-switch','2030-10-07T07:00:00Z','2030-10-07T07:30:00Z',10,'confirmed','2026-10-05T08:00:00Z')")
                if not previous_rust:
                    # Retained initial Node cutovers have no backend metadata.
                    (home / '.local/cutover-candidate/previous-backend.json').unlink()
                release.rollback(home / '.local/cutover-candidate', path)
                self.assertEqual(release.safe(path)['bookings']['confirmed'], 1)
                for label in release.LABELS:
                    self.assertEqual(release.plistlib.loads((agents / f'{label}.plist').read_bytes()), original[label])
                self.assertIn(('readback', True), events)
                self.assertEqual(events[-1], ('readback', previous_rust))
if __name__ == '__main__':
    unittest.main()
