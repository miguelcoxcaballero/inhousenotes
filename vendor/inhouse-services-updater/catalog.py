#!/usr/bin/env python3
"""Publish A by default. Publishing B needs explicit user authorization, never inferred.

Only run after the exact signed APK is public. A includes all A and B releases and
selects the greatest global versionCode; B contains only explicitly reviewed releases.
Keep changelogs tiny in all apps.
"""
import argparse, datetime, json, re
from pathlib import Path

IDENTITY = ('version', 'apkUrl', 'apkSha256', 'apkSizeBytes', 'changelog')


def checked_release(release):
    if not isinstance(release, dict): raise ValueError('Invalid catalog release')
    value = dict(release)
    if not isinstance(value.get('version'), str) or not value['version'].strip(): raise ValueError('Missing release version')
    if type(value.get('versionCode')) is not int or value['versionCode'] < 1: raise ValueError('Invalid version code')
    if not isinstance(value.get('apkUrl'), str) or not value['apkUrl'].strip(): raise ValueError('Missing APK URL')
    sha = value.get('apkSha256')
    if not isinstance(sha, str) or not re.fullmatch(r'[a-fA-F0-9]{64}', sha): raise ValueError('Missing APK integrity metadata')
    if type(value.get('apkSizeBytes')) is not int or value['apkSizeBytes'] < 1: raise ValueError('Missing APK integrity metadata')
    if not isinstance(value.get('changelog'), str) or not 1 <= len(value['changelog'].strip()) <= 120: raise ValueError('Invalid catalog changelog')
    if not isinstance(value.get('releasedAt'), str) or not value['releasedAt'].strip(): raise ValueError('Missing release date')
    value['apkSha256'] = sha.lower()
    return value


def add_release(releases, release):
    """Deduplicate immutable versions before any catalog is written."""
    code = release['versionCode']
    same = releases.get(code)
    if same is not None and any(same[key] != release[key] for key in IDENTITY):
        raise ValueError('Never replace a published version')
    if same is None: releases[code] = release


def read_catalog(target, channel):
    value = json.loads(target.read_text()) if target.exists() else dict(schemaVersion=1, channel=channel, latest=None, releases=[])
    if not isinstance(value, dict) or value.get('schemaVersion') != 1 or value.get('channel') != channel or not isinstance(value.get('releases'), list):
        raise ValueError('Invalid channel catalog')
    releases = {}
    for release in value['releases']:
        add_release(releases, checked_release(release))
    # Preserve a published latest even when an older publisher omitted its history.
    if value.get('latest') is not None:
        add_release(releases, checked_release(value['latest']))
    return value, releases


def write_release(root, manifest, note, channel='A', user_requested_b=False):
    if channel not in ('A', 'B'): raise ValueError('Unknown update channel')
    if channel=='B' and not user_requested_b: raise ValueError('B requires the user\'s explicit request; do not infer approval from tests.')
    if not isinstance(note, str) or not 1<=len(note.strip())<=120: raise ValueError('Changelog must be 1–120 characters; prefer 2–10 words.')
    sha=manifest.get('apkSha256') or manifest.get('sha256');size=manifest.get('apkSizeBytes') or manifest.get('sizeBytes')
    release={k:manifest.get(k) for k in ('version','versionCode','apkUrl')}
    release.update(apkSha256=sha,apkSizeBytes=size,changelog=note.strip(),releasedAt=datetime.datetime.now(datetime.timezone.utc).isoformat(timespec='seconds'))
    release=checked_release(release)
    path=Path(root)/'updates'
    values={}; histories={}
    for c in ('A', 'B'):
        values[c], histories[c] = read_catalog(path/(c+'.json'), c)
    all_releases={}
    for c in ('A', 'B'):
        for existing in histories[c].values(): add_release(all_releases, existing)
    if release['versionCode'] not in all_releases and all_releases and release['versionCode'] <= max(all_releases):
        raise ValueError('Version code must advance globally')
    add_release(all_releases, release)
    # Existing records keep their original date and changelog on promotion/retry.
    release=all_releases[release['versionCode']]
    histories['A']=all_releases
    if channel=='B':
        histories['B'][release['versionCode']]=release
    pending={}
    for c in ('A', 'B'):
        releases=[all_releases[code] for code in sorted(histories[c], reverse=True)]
        updated={**values[c], 'latest':releases[0] if releases else None, 'releases':releases}
        target=path/(c+'.json')
        if not target.exists() or updated != values[c]:
            pending[target]=json.dumps(updated,indent=2,ensure_ascii=False)+'\n'
    # Validate both complete catalogs first: a conflict must never partly publish.
    if pending:
        path.mkdir(exist_ok=True)
        for target, content in pending.items(): target.write_text(content)
        return path/(channel+'.json')

if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--root',type=Path,default=Path('.'));p.add_argument('--manifest',type=Path,required=True);p.add_argument('--changelog',required=True);p.add_argument('--channel',choices=['A','B'],default='A');p.add_argument('--user-explicitly-requested-b',action='store_true');a=p.parse_args()
    print(write_release(a.root,json.loads(a.manifest.read_text()),a.changelog,a.channel,a.user_explicitly_requested_b))
