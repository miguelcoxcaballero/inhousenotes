#!/usr/bin/env python3
"""Reject locally forked updater files before an app build. See UPDATER.md."""
import hashlib, json, re, sys
from pathlib import Path
root=Path(sys.argv[1] if len(sys.argv)>1 else '.').resolve()
lock=json.loads((root/'inhouse-updater.lock.json').read_text())
assert lock['repository']=='miguelcoxcaballero/inhouse-services-updater'
assert re.fullmatch('[a-f0-9]{40}',lock['revision'])
# FROZEN Android installer: these exact bytes installed an update from inside the
# app on a real Samsung phone (2026-10-20). Read FROZEN_INSTALLER.md. Never change
# these hashes unless the user, in their own words, unfroze the installer and a
# real phone verified the new version.
FROZEN_INSTALLER={
    'android/PhotosUpdateInstaller.kt':'ff033ec1968b047c507e5c2aeeb351cb856033c33270616b8d58d1b23776ab2f',
    'android/UpdateFileProvider.java':'046b4d96d93abb67e2149029fa29692e5576e969b9d7d7931ad6df20201c8ede',
    'tools/register_android.py':'84ebb89d4941353e5eb135c6c08f9db1f057e4065a41a98684f7f2afe3db98ad',
}
for name, entry in lock['files'].items():
    assert hashlib.sha256((root/name).read_bytes()).hexdigest()==entry['sha256'], f'Updater drift: {name}. Edit the central repository and sync every app in tools/sync.py.'
    if entry['source'] in FROZEN_INSTALLER:
        assert entry['sha256']==FROZEN_INSTALLER[entry['source']], f'Frozen installer changed: {name}. It is verified on a real phone and must not change; read FROZEN_INSTALLER.md.'
assert 'explicit request' in (root/'UPDATER.md').read_text()
print('PASS: shared updater files match locked revision '+lock['revision'])
