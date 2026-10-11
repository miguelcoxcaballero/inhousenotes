#!/usr/bin/env python3
"""Publish A by default. B promotion needs explicit user authorization, never inferred.

Only run after the exact signed APK is public. Keep changelogs tiny in all apps.
"""
import argparse, datetime, hashlib, json, re
from pathlib import Path

def write_release(root, manifest, note, channel='A', user_requested_b=False):
    if channel=='B' and not user_requested_b: raise ValueError('B requires the user\'s explicit request; do not infer approval from tests.')
    if not 1<=len(note.strip())<=120: raise ValueError('Changelog must be 1–120 characters; prefer 2–10 words.')
    sha=manifest.get('apkSha256') or manifest.get('sha256');size=manifest.get('apkSizeBytes') or manifest.get('sizeBytes')
    if not re.fullmatch(r'[a-fA-F0-9]{64}',sha or '') or not isinstance(size,int) or size<1: raise ValueError('Missing APK integrity metadata')
    path=Path(root)/'updates';path.mkdir(exist_ok=True)
    for c in ('A','B'):
        target=path/(c+'.json')
        if not target.exists(): target.write_text(json.dumps({'schemaVersion':1,'channel':c,'latest':None,'releases':[]},indent=2)+'\n')
    target=path/(channel+'.json');value=json.loads(target.read_text())
    release={k:manifest[k] for k in ('version','versionCode','apkUrl')}
    release.update(apkSha256=sha.lower(),apkSizeBytes=size,changelog=note.strip(),releasedAt=datetime.datetime.now(datetime.timezone.utc).isoformat(timespec='seconds'))
    if channel=='B':
        source=json.loads((path/'A.json').read_text())
        match=next((r for r in source['releases'] if r['versionCode']==release['versionCode']),None)
        if not match or any(match[k]!=release[k] for k in ('version','apkUrl','apkSha256','apkSizeBytes')): raise ValueError('B must promote an exact published A artifact')
        release=match
    same=next((r for r in value['releases'] if r['versionCode']==release['versionCode']),None)
    if same:
        if any(same[k]!=release[k] for k in ('version','apkUrl','apkSha256','apkSizeBytes','changelog')): raise ValueError('Never replace a published version')
        return
    if value['latest'] and release['versionCode']<=value['latest']['versionCode']: raise ValueError('Version code must advance')
    value['releases'].insert(0,release);value['latest']=release
    target.write_text(json.dumps(value,indent=2,ensure_ascii=False)+'\n')
    return target

if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--root',type=Path,default=Path('.'));p.add_argument('--manifest',type=Path,required=True);p.add_argument('--changelog',required=True);p.add_argument('--channel',choices=['A','B'],default='A');p.add_argument('--user-explicitly-requested-b',action='store_true');a=p.parse_args()
    print(write_release(a.root,json.loads(a.manifest.read_text()),a.changelog,a.channel,a.user_explicitly_requested_b))
