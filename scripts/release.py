#!/usr/bin/env python3
"""Package, inspect and switch the existing JEV LaunchAgents without replacing data."""
import argparse, datetime, hashlib, json, os, pathlib, plistlib, shutil, sqlite3, subprocess, time, urllib.request
ROOT = pathlib.Path(__file__).resolve().parent.parent
SOURCE = pathlib.Path('/Users/jaack/clawd/apps/jev-calendar')
LABELS = ['me.jaack.jev-calendar', 'me.jaack.jev-calendar-public']
DOMAIN = f'gui/{os.getuid()}'

def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def baseline():
    manifest = json.loads((ROOT / 'evidence/baseline-manifest.json').read_text())
    changed = [p for p, h in manifest.items() if not (ROOT / 'reference' / p).is_file() or digest(ROOT / 'reference' / p) != h]
    if changed:
        raise RuntimeError('Captured baseline changed: ' + ', '.join(changed))
    # Updated production documentation does not change the retained runtime.
    changed = [p for p, h in manifest.items() if pathlib.Path(p).suffix != '.md' and (not (SOURCE / p).is_file() or digest(SOURCE / p) != h)]
    if changed:
        raise RuntimeError('Original source changed since capture: ' + ', '.join(changed))

def inspect_database(path):
    with sqlite3.connect(f'file:{path.resolve()}?mode=ro', uri=True, timeout=3) as db:
        integrity = db.execute('PRAGMA quick_check').fetchone()[0]
        counts = dict(db.execute('SELECT status,COUNT(*) FROM bookings GROUP BY status').fetchall())
        changes = db.execute('SELECT COUNT(*) FROM bookings WHERE pending_change IS NOT NULL').fetchone()[0]
        journal = db.execute("SELECT COUNT(*) FROM jaack_os_booking_changes WHERE state IN ('pending','uncertain')").fetchone()[0]
    return {'integrity': integrity, 'bookings': counts, 'pendingChanges': changes, 'unresolvedJournal': journal}

def safe(path):
    state = inspect_database(path)
    if state['integrity'] != 'ok' or state['bookings'].get('pending', 0) or state['pendingChanges'] or state['unresolvedJournal']:
        raise RuntimeError('Reconcile unresolved provider operations before switching: ' + json.dumps(state))
    return state

def backup(path, destination):
    if destination.exists():
        raise RuntimeError('Backup destination already exists')
    destination.parent.mkdir(parents=True, exist_ok=True, mode=448)
    with sqlite3.connect(f'file:{path.resolve()}?mode=ro', uri=True) as src, sqlite3.connect(destination) as dst:
        src.backup(dst)
    destination.chmod(384)
    if inspect_database(destination)['integrity'] != 'ok':
        raise RuntimeError('Backup integrity check failed')

def package():
    baseline()
    binary = ROOT / 'target/release/jev-calendar'
    if not binary.is_file():
        raise RuntimeError('Build release first')
    identity = datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%SZ') + '-' + digest(binary)[:12]
    release = ROOT / 'dist' / identity
    release.mkdir(parents=True)
    shutil.copy2(binary, release / 'jev-calendar')
    shutil.copytree(ROOT / 'reference/public', release / 'public')
    shutil.copytree(ROOT / 'public', release / 'public', dirs_exist_ok=True)
    shutil.copy2(ROOT / 'Cargo.lock', release / 'Cargo.lock')
    shutil.copy2(ROOT / 'rust-toolchain.toml', release / 'rust-toolchain.toml')
    for mode in ['private', 'public']:
        variables = 'export DEMO_MODE=false\nexport PORT=3030\n' if mode == 'private' else 'export PUBLIC_HOST=book.jaack.me\nexport PUBLIC_PORT=3031\nexport INTERNAL_ORIGIN=http://127.0.0.1:3030\n'
        script = release / f'run-{mode}.sh'
        script.write_text(f'#!/bin/zsh\nset -eu\ncd {SOURCE}\nset -a\nsource .env\nset +a\nexport JEV_ALLOW_LIVE_STATE=1\nexport NODE_ENV=production\nexport JEV_ASSET_ROOT={release}/public\n' + variables + f'exec {release}/jev-calendar --{mode}\n')
        script.chmod(493)
    files = {str(p.relative_to(release)): digest(p) for p in sorted(release.rglob('*')) if p.is_file()}
    (release / 'manifest.json').write_text(json.dumps({'id': identity, 'platform': os.uname().machine, 'binarySha256': digest(binary), 'files': files}, indent=2) + '\n')
    print(release)
    return release

def verify(release):
    manifest = json.loads((release / 'manifest.json').read_text())
    for file, expected in manifest['files'].items():
        if digest(release / file) != expected:
            raise RuntimeError('Release checksum failed: ' + file)

def run(*args, required=True):
    r = subprocess.run(args, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    if required and r.returncode:
        raise RuntimeError(f'{args[0]} failed ({r.returncode})')
    return r

def loaded(label):
    return run('launchctl', 'print', f'{DOMAIN}/{label}', required=False).returncode == 0

def pid(label):
    r = run('launchctl', 'print', f'{DOMAIN}/{label}', required=False)
    for line in r.stdout.splitlines():
        if line.strip().startswith('pid = '):
            return int(line.split('=')[1])
    return None

def stop(label):
    current = pid(label)
    run('launchctl', 'bootout', f'{DOMAIN}/{label}')
    if current:
        for _ in range(200):
            try:
                os.kill(current, 0)
            except ProcessLookupError:
                return
            time.sleep(0.1)
        raise RuntimeError('Process did not drain; it was not force killed')

def start(label):
    run('launchctl', 'bootstrap', DOMAIN, str(pathlib.Path.home() / 'Library/LaunchAgents' / f'{label}.plist'))

def readback(rust):
    for port in [3030, 3031]:
        deadline = time.monotonic() + 60
        while time.monotonic() < deadline:
            try:
                req = urllib.request.Request(f'http://127.0.0.1:{port}/health', headers={'Host': 'book.jaack.me'})
                with urllib.request.urlopen(req, timeout=2) as r:
                    if (r.headers.get('X-Jev-Backend') == 'rust') != rust:
                        raise RuntimeError('Unexpected backend')
                    if json.load(r).get('ok'):
                        break
            except (OSError, ValueError):
                time.sleep(0.1)
        else:
            raise RuntimeError('Startup health failed')

def current_backend():
    backends = []
    for port in [3030, 3031]:
        req = urllib.request.Request(f'http://127.0.0.1:{port}/health', headers={'Host': 'book.jaack.me'})
        with urllib.request.urlopen(req, timeout=2) as response:
            if not json.load(response).get('ok'):
                raise RuntimeError('Current backend health failed')
            backends.append(response.headers.get('X-Jev-Backend') == 'rust')
    if backends[0] != backends[1]:
        raise RuntimeError('Current LaunchAgents use different backends')
    return backends[0]

def switch(release, path):
    baseline()
    verify(release)
    safe(path)
    if not all((pid(label) for label in LABELS)):
        raise RuntimeError('Both current LaunchAgents must be running')
    previous_rust = current_backend()
    state = ROOT / '.local' / ('cutover-' + release.name)
    state.mkdir(parents=True, mode=448)
    (state / 'previous-backend.json').write_text(json.dumps({'rust': previous_rust}) + '\n')
    for label in LABELS:
        source = pathlib.Path.home() / 'Library/LaunchAgents' / f'{label}.plist'
        shutil.copy2(source, state / source.name)
    try:
        stop(LABELS[1])
        stop(LABELS[0])
        # A reservation committed during drain must block activation.
        safe(path)
        backup(path, state / 'before.sqlite')
        for label, mode in zip(LABELS, ['private', 'public']):
            source = pathlib.Path.home() / 'Library/LaunchAgents' / f'{label}.plist'
            value = plistlib.loads((state / source.name).read_bytes())
            value['ProgramArguments'] = [str(release / f'run-{mode}.sh')]
            with source.open('wb') as f:
                plistlib.dump(value, f)
        start(LABELS[0])
        start(LABELS[1])
        readback(True)
        (state / 'receipt.json').write_text(json.dumps({'release': str(release), 'database': str(path), 'at': datetime.datetime.now(datetime.timezone.utc).isoformat(), 'status': 'rust-active'}, indent=2) + '\n')
        print('Rust active. Retained rollback directory: ' + str(state))
    except Exception:
        # Stop every candidate before restoring the exact prior launch files.
        for label in reversed(LABELS):
            if loaded(label):
                stop(label)
        for label in LABELS:
            source = pathlib.Path.home() / 'Library/LaunchAgents' / f'{label}.plist'
            shutil.copy2(state / source.name, source)
        safe(path)
        start(LABELS[0])
        start(LABELS[1])
        readback(previous_rust)
        raise

def rollback(state, path):
    for label in LABELS:
        plistlib.loads((state / f'{label}.plist').read_bytes())
    metadata = state / 'previous-backend.json'
    # The initial Node cutover predates this metadata file.
    previous_rust = json.loads(metadata.read_text())['rust'] if metadata.exists() else False
    safe(path)
    stop(LABELS[1])
    stop(LABELS[0])
    safe(path)
    # Preserve current data and every post-cutover booking, never restore an old DB.
    backup(path, state / f'before-rollback-{time.time_ns()}.sqlite')
    for label in LABELS:
        target = pathlib.Path.home() / 'Library/LaunchAgents' / f'{label}.plist'
        shutil.copy2(state / target.name, target)
    start(LABELS[0])
    start(LABELS[1])
    readback(previous_rust)
    print(('Rust' if previous_rust else 'Node') + ' restored with current database')
if __name__ == '__main__':
    p = argparse.ArgumentParser()
    p.add_argument('action', choices=['package', 'inspect', 'backup', 'switch', 'rollback'])
    p.add_argument('--database', type=pathlib.Path)
    p.add_argument('--release', type=pathlib.Path)
    p.add_argument('--destination', type=pathlib.Path)
    p.add_argument('--live', action='store_true')
    a = p.parse_args()
    if a.action == 'package':
        package()
    elif a.action == 'inspect':
        print(json.dumps(safe(a.database), indent=2))
    elif a.action == 'backup':
        backup(a.database, a.destination)
        print('Consistent backup verified')
    elif not a.live:
        p.error('Activation requires explicit live-cutover authorization and --live')
    elif a.action == 'switch':
        switch(a.release.resolve(), a.database.resolve())
    else:
        rollback(a.destination.resolve(), a.database.resolve())
