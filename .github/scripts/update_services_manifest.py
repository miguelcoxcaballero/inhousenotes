# Inhouse services updater: Automatic publication defaults to A; users default to B.
# A requires development consent; B publication or promotion requires an explicit user request.
# The canonical publisher includes A+B in A and keeps the highest global versionCode; B includes only B.
import argparse,hashlib,json,re,runpy
from pathlib import Path
root=Path(__file__).resolve().parents[2]
parser=argparse.ArgumentParser()
parser.add_argument('--apk',type=Path,default=root/'inhouse-notes-release-v1.0.28.apk')
parser.add_argument('--channel',choices=['A','B'],default='A')
parser.add_argument('--user-requested-b',action='store_true')
args=parser.parse_args()
apk=args.apk
manifest={'version':'1.0.28','versionCode':29,'required':True,'apkSizeBytes':apk.stat().st_size,'apkSha256':hashlib.sha256(apk.read_bytes()).hexdigest(),'apkUrl':'https://github.com/miguelcoxcaballero/inhousenotes/releases/download/android-v1.0.28/'+apk.name,'publishedAppVersion':re.search(r"const APP_VERSION = '([^']+)'",(root/'index.html').read_text())[1],'releaseNotes':'Test the in-app updater.'}
(root/'android-update.json').write_text(json.dumps(manifest,indent=2)+'\n')
runpy.run_path(str(root/'vendor/inhouse-services-updater/catalog.py'))['write_release'](root,manifest,manifest['releaseNotes'],channel=args.channel,user_requested_b=args.user_requested_b)
