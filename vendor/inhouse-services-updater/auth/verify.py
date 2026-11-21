"""Build guard for canonical OAuth copies, independent of the frozen updater lock."""
import hashlib
import json
from pathlib import Path
import re
import sys
root = Path(sys.argv[1] if len(sys.argv) > 1 else '.').resolve()
lock = json.loads((root / 'inhouse-oauth.lock.json').read_text())
assert lock['repository'] == 'miguelcoxcaballero/inhouse-services-updater'
assert re.fullmatch('[a-f0-9]{40}', lock['revision'])
assert lock['files'], 'Empty OAuth lock'
for name, entry in lock['files'].items():
    assert hashlib.sha256((root / name).read_bytes()).hexdigest() == entry['sha256'], f'OAuth drift: {name}; edit the central repository and sync --auth-only'
print('PASS: shared OAuth matches ' + lock['revision'])
