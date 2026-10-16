#!/usr/bin/env python3
"""Reject locally forked updater files before an app build. See UPDATER.md."""
import hashlib, json, re, sys
from pathlib import Path
root=Path(sys.argv[1] if len(sys.argv)>1 else '.').resolve()
lock=json.loads((root/'inhouse-updater.lock.json').read_text())
assert lock['repository']=='miguelcoxcaballero/inhouse-services-updater'
assert re.fullmatch('[a-f0-9]{40}',lock['revision'])
for name, entry in lock['files'].items():
    assert hashlib.sha256((root/name).read_bytes()).hexdigest()==entry['sha256'], f'Updater drift: {name}. Edit the central repository and sync all five apps.'
assert 'explicit request' in (root/'UPDATER.md').read_text()
print('PASS: shared updater files match locked revision '+lock['revision'])
