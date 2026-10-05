#!/usr/bin/env python3
"""Restore the immutable Node oracle locally without production data or secrets."""
import hashlib
import json
from pathlib import Path
import tarfile

ROOT = Path(__file__).resolve().parent.parent


def restore():
    manifest = json.loads((ROOT / 'evidence/baseline-manifest.json').read_text())
    destination = ROOT / 'reference'
    for name, expected in manifest.items():
        path = destination / name
        if path.exists() and hashlib.sha256(path.read_bytes()).hexdigest() != expected:
            raise RuntimeError(f'Existing reference differs from captured baseline: {name}')
    with tarfile.open(ROOT / 'fixtures/node-baseline.tar.gz') as archive:
        members = archive.getmembers()
        if {m.name for m in members} != set(manifest) or not all(m.isfile() for m in members):
            raise RuntimeError('Archive does not match the captured file inventory')
        # Python's data filter confines archive paths and disallows unsafe links.
        archive.extractall(destination, filter='data')
    for name, expected in manifest.items():
        if hashlib.sha256((destination / name).read_bytes()).hexdigest() != expected:
            raise RuntimeError(f'Restored reference checksum failed: {name}')
    print(f'Verified {len(manifest)} reference files')


if __name__ == '__main__':
    restore()
