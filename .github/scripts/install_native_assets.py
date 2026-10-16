"""Install Notes-owned local resources without modifying the shared updater."""
import hashlib,json,shutil
from pathlib import Path

def install(project: Path, repo: Path) -> None:
    app=project/'android/app/src/main'
    destination=app/'assets/inhouse-native'
    destination.mkdir(parents=True,exist_ok=True)
    manifest=json.loads((repo/'native-assets.json').read_text())
    for path,info in manifest['assets'].items():
        source=(repo/path.lstrip('/')).resolve()
        if not source.is_relative_to(repo.resolve()) or '..' in path:raise ValueError('Invalid bundled path')
        content=source.read_bytes()
        if len(content)!=info['size'] or hashlib.sha256(content).hexdigest()!=info['sha256']:
            raise ValueError(f'Stale native asset manifest: {path}; run npm run build:offline')
        target=destination/path.lstrip('/');target.parent.mkdir(parents=True,exist_ok=True)
        shutil.copyfile(source,target)
    shutil.copyfile(repo/'native-assets.json',destination/'native-assets.json')
    java=app/'java/com/local/inhousenotes'
    shutil.copyfile(repo/'.github/android/performance/InhouseLocalAssets.java',java/'InhouseLocalAssets.java')
    activity=java/'MainActivity.java';text=activity.read_text()
    anchor='InhouseBackgroundSync.attach(this, webView);'
    if anchor not in text:raise ValueError('Cannot find Notes host installation point')
    text=text.replace(anchor,anchor+'\n        InhouseLocalAssets.attach(getBridge());')
    activity.write_text(text)
