# Inhouse services updater: A by default; B only on explicit user request.
import hashlib,json,re,runpy
from pathlib import Path
root=Path(__file__).resolve().parents[2]
apk=root/'inhouse-notes-release-v1.0.11.apk'
manifest={'version':'1.0.11','versionCode':12,'required':True,'apkSizeBytes':apk.stat().st_size,'apkSha256':hashlib.sha256(apk.read_bytes()).hexdigest(),'apkUrl':'https://github.com/miguelcoxcaballero/inhousenotes/releases/download/android-v1.0.11/'+apk.name,'publishedAppVersion':re.search(r"const APP_VERSION = '([^']+)'",(root/'index.html').read_text())[1],'releaseNotes':'Canales A/B y recordatorio opcional.'}
(root/'android-update.json').write_text(json.dumps(manifest,indent=2)+'\n')
runpy.run_path(str(root/'vendor/inhouse-services-updater/catalog.py'))['write_release'](root,manifest,manifest['releaseNotes'])
