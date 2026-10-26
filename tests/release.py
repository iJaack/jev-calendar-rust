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

    def test_readback_allows_initial_calendar_warmup(self):
        def healthy():
            value = BytesIO(b'{"ok":true}')
            value.headers = {'X-Jev-Backend': 'rust'}
            return value
        replies = [OSError('warming')] * 120 + [healthy(), healthy()]
        with patch.object(release.urllib.request, 'urlopen', side_effect=replies), patch.object(release.time, 'sleep'):
            release.readback(True)

    def test_readback_waits_until_calendar_previews_are_ready(self):
        def health(ready):
            value = BytesIO(json.dumps({'ok': True, 'previewReady': ready}).encode())
            value.headers = {'X-Jev-Backend': 'rust'}
            return value
        with patch.object(release.urllib.request, 'urlopen', side_effect=[health(False), health(True), health(True)]) as request, patch.object(release.time, 'sleep'):
            release.readback(True)
            self.assertEqual(request.call_count, 3)

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
# Spec Tests: Config validation (valid list, every invalid rule, exactly one
# default, unknown keys) and --set/--show CLI. Built binary + disposable DB only.
BOOKING_TYPES = [
    dict(id='personal', label='Personal', accountId='owner@example.test', calendarId='personal-work@group.example.test', default=True),
    dict(id='team1', label='Team1', accountId='team@team.example.test', calendarId='team-work@group.example.test', default=False),
]

def changed_type(**fields):
    return [dict(BOOKING_TYPES[0], **fields), dict(BOOKING_TYPES[1])]

INVALID_BOOKING_TYPES = {
    'not_array': {}, 'null_list': None, 'empty_list': [],
    'too_many_entries': [dict(BOOKING_TYPES[0], id=f'type-{n}', default=n == 0) for n in range(11)],
    'non_object_entry': [BOOKING_TYPES[0], None],
    'empty_id': changed_type(id=''), 'long_id': changed_type(id='a' * 33),
    'uppercase_id': changed_type(id='Personal'), 'underscore_id': changed_type(id='a_b'),
    'leading_hyphen_id': changed_type(id='-personal'), 'unicode_id': changed_type(id='café'),
    'newline_id': changed_type(id='personal\n'), 'numeric_id': changed_type(id=5),
    'duplicate_id': changed_type(id='team1'),
    'empty_label': changed_type(label=''), 'long_label': changed_type(label='x' * 41),
    'newline_label': changed_type(label='Personal\nCalendar'), 'carriage_return_label': changed_type(label='Personal\rCalendar'),
    'numeric_label': changed_type(label=4),
    'empty_account': changed_type(accountId=''), 'null_account': changed_type(accountId=None),
    'numeric_account': changed_type(accountId=3),
    'empty_calendar': changed_type(calendarId=''), 'null_calendar': changed_type(calendarId=None),
    'numeric_calendar': changed_type(calendarId=3),
    'no_default': changed_type(default=False), 'string_default': changed_type(default='true'),
    'two_defaults': [BOOKING_TYPES[0], dict(BOOKING_TYPES[1], default=True)],
    'unknown_key': changed_type(unexpected='synthetic'),
}
for field in ['id', 'label', 'accountId', 'calendarId', 'default']:
    INVALID_BOOKING_TYPES['missing_' + field] = [{k: v for k, v in BOOKING_TYPES[0].items() if k != field}, dict(BOOKING_TYPES[1])]

class BookingTypeCLI(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix='jev-booking-types-cli-')
        self.addCleanup(self.directory.cleanup)
        self.home = pathlib.Path(self.directory.name)
        self.path = self.home / 'state.sqlite'
        self.env = dict(PATH=os.environ['PATH'], DATABASE_PATH=str(self.path),
                        APP_ENCRYPTION_KEY='synthetic-booking-type-key', DEMO_MODE='true', PORT='0')
        result = self.run_binary('--backup', str(self.home / 'initial.sqlite'))
        self.assertEqual(result.returncode, 0, result.stderr)

    def run_binary(self, *args):
        return subprocess.run([str(ROOT / 'target/release/jev-calendar'), *args], cwd=self.home,
                              env=self.env, capture_output=True, text=True, timeout=10)

    def write_list(self, value):
        config = self.home / 'types.json'
        config.write_text(json.dumps(value))
        return self.run_binary('--set-booking-types', str(config))

    def stored(self):
        with sqlite3.connect(self.path) as db:
            row = db.execute("SELECT value FROM settings WHERE key='bookingTypes'").fetchone()
        return json.loads(row[0]) if row else None

    def check_valid(self, value):
        result = self.write_list(value)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.stored(), value)
        for entry in value:
            self.assertIn(entry['id'], result.stdout)
            self.assertIn(entry['label'], result.stdout)
            self.assertNotIn(entry['accountId'], result.stdout)
            self.assertNotIn(entry['calendarId'], result.stdout)
        shown = self.run_binary('--show-booking-types')
        self.assertEqual(shown.returncode, 0, shown.stderr)
        self.assertEqual(json.loads(shown.stdout), value)

    def test_show_existing_list(self):
        with sqlite3.connect(self.path) as db:
            db.execute("INSERT OR REPLACE INTO settings VALUES ('bookingTypes', ?)", (json.dumps(BOOKING_TYPES),))
        shown = self.run_binary('--show-booking-types')
        self.assertEqual(shown.returncode, 0, shown.stderr)
        self.assertEqual(json.loads(shown.stdout), BOOKING_TYPES)

    def check_invalid(self, value):
        # Seed directly so today's missing CLI cannot masquerade as validation.
        with sqlite3.connect(self.path) as db:
            db.execute("INSERT OR REPLACE INTO settings VALUES ('bookingTypes', ?)", (json.dumps(BOOKING_TYPES),))
        result = self.write_list(value)
        self.assertNotEqual(result.returncode, 0)
        self.assertRegex(result.stderr + result.stdout, r'(?i)booking[ _-]*types?.*(invalid|required|must|unique|default|unknown|array|empty|entry|entries|label|id|key|limit|reject)|(?i:invalid|required|unknown).*booking[ _-]*types?')
        self.assertEqual(self.stored(), BOOKING_TYPES, 'invalid lists must not partially replace the setting')

    def test_malformed_json_rejected_atomically(self):
        config = self.home / 'broken.json'
        config.write_text('[{"id":')
        result = self.run_binary('--set-booking-types', str(config))
        self.assertNotEqual(result.returncode, 0)
        self.assertRegex(result.stderr, r'(?i)json|parse|booking[ _-]*types?')
        self.assertIsNone(self.stored())

VALID_BOOKING_TYPES = {
    'standard': BOOKING_TYPES,
    'boundaries_and_primary': [dict(BOOKING_TYPES[0], id='a' * 32, label='é' * 40, calendarId='primary')],
    'maximum_and_last_default': [dict(BOOKING_TYPES[0], id=f'type-{n}', default=n == 9) for n in range(10)],
    'optional_false': [dict(BOOKING_TYPES[0]), {k: v for k, v in BOOKING_TYPES[1].items() if k != 'default'}],
}
for case, value in VALID_BOOKING_TYPES.items():
    def valid_case(self, value=value):
        self.check_valid(value)
    setattr(BookingTypeCLI, 'test_accept_' + case, valid_case)

for case, value in INVALID_BOOKING_TYPES.items():
    def invalid_case(self, value=value):
        self.check_invalid(value)
    setattr(BookingTypeCLI, 'test_reject_' + case, invalid_case)

# Spec Tests: Migration opens an old schema, adds nullable booking_type/calendar_id
# idempotently, and preserves rows/values. Backup and restore retain both schemas.
class BookingTypeMigration(unittest.TestCase):
    def test_old_schema_migrates_idempotently_preserving_values_and_backups(self):
        with tempfile.TemporaryDirectory(prefix='jev-booking-types-migration-') as directory:
            home = pathlib.Path(directory)
            path = home / 'old.sqlite'
            original = ('synthetic-old-row', '2030-10-07T12:00:00Z', '2030-10-07T12:30:00Z', 10,
                        'Synthetic Guest', 'guest@example.test', 'confirmed', 'synthetic-event', '2026-10-01T12:00:00Z')
            with sqlite3.connect(path) as db:
                db.execute('CREATE TABLE bookings (idempotency_key TEXT PRIMARY KEY,start TEXT NOT NULL,end TEXT NOT NULL,buffer_minutes INTEGER NOT NULL,guest_name TEXT,guest_email TEXT,status TEXT NOT NULL,event_id TEXT,created_at TEXT NOT NULL)')
                db.execute('INSERT INTO bookings VALUES (?,?,?,?,?,?,?,?,?)', original)
                self.assertNotIn('booking_type', [r[1] for r in db.execute('PRAGMA table_info(bookings)')])
            env = dict(PATH=os.environ['PATH'], DATABASE_PATH=str(path), APP_ENCRYPTION_KEY='synthetic-migration-key', DEMO_MODE='true', PORT='0')
            for n in range(2):
                backup = home / f'backup-{n}.sqlite'
                result = subprocess.run([str(ROOT / 'target/release/jev-calendar'), '--backup', str(backup)], cwd=home, env=env, capture_output=True, text=True, timeout=10)
                self.assertEqual(result.returncode, 0, result.stderr)
                for current in [path, backup]:
                    with sqlite3.connect(current) as db:
                        columns = {r[1]: r for r in db.execute('PRAGMA table_info(bookings)')}
                        for name in ['booking_type', 'calendar_id']:
                            self.assertIn(name, columns, 'startup migration must add booking-type columns')
                            self.assertEqual(columns[name][2].upper(), 'TEXT')
                            self.assertEqual(columns[name][3], 0, 'column must be nullable')
                            self.assertIsNone(columns[name][4])
                        row = db.execute('SELECT idempotency_key,start,end,buffer_minutes,guest_name,guest_email,status,event_id,created_at,booking_type,calendar_id FROM bookings').fetchone()
                        self.assertEqual(row, original + (None, None))
                    self.assertEqual(release.safe(current)['bookings']['confirmed'], 1)
            restored = home / 'restored.sqlite'
            with sqlite3.connect(backup) as source, sqlite3.connect(restored) as target:
                source.backup(target)
            with sqlite3.connect(restored) as db:
                self.assertEqual(db.execute('SELECT booking_type,calendar_id FROM bookings').fetchall(), [(None, None)])

    def test_backup_restore_retains_existing_typed_destination_values(self):
        # Spec Tests: Backup/restore safety also works with the added columns.
        with tempfile.TemporaryDirectory(prefix='jev-booking-types-backup-') as directory:
            home = pathlib.Path(directory)
            path, backup, restored = [home / name for name in ['state.sqlite', 'backup.sqlite', 'restored.sqlite']]
            env = dict(PATH=os.environ['PATH'], DATABASE_PATH=str(path), APP_ENCRYPTION_KEY='synthetic-backup-key', DEMO_MODE='true', PORT='0')
            binary = str(ROOT / 'target/release/jev-calendar')
            initial = subprocess.run([binary, '--backup', str(home / 'empty.sqlite')], cwd=home, env=env, capture_output=True, text=True, timeout=10)
            self.assertEqual(initial.returncode, 0, initial.stderr)
            with sqlite3.connect(path) as db:
                columns = {r[1] for r in db.execute('PRAGMA table_info(bookings)')}
                # Seed a post-feature schema independently of migration coverage.
                for name in ['booking_type', 'calendar_id']:
                    if name not in columns:
                        db.execute(f'ALTER TABLE bookings ADD COLUMN {name} TEXT')
                db.execute("INSERT INTO bookings (idempotency_key,start,end,buffer_minutes,status,created_at,google_account_id,booking_type,calendar_id) VALUES ('synthetic-typed-backup','2030-10-07T12:00:00Z','2030-10-07T12:30:00Z',10,'confirmed','2026-10-01T12:00:00Z','team@team.example.test','team1','team-work@group.example.test')")
            result = subprocess.run([binary, '--backup', str(backup)], cwd=home, env=env, capture_output=True, text=True, timeout=10)
            self.assertEqual(result.returncode, 0, result.stderr)
            with sqlite3.connect(backup) as source, sqlite3.connect(restored) as target:
                source.backup(target)
            for current in [path, backup, restored]:
                with sqlite3.connect(current) as db:
                    self.assertEqual(db.execute('SELECT booking_type,calendar_id,google_account_id FROM bookings').fetchall(), [('team1', 'team-work@group.example.test', 'team@team.example.test')])
                self.assertEqual(release.safe(current)['bookings']['confirmed'], 1)

if __name__ == '__main__':
    unittest.main()
