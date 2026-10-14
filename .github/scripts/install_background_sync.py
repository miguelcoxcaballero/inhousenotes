"""Install Notes-owned Android background sync; never modifies the shared updater."""
from pathlib import Path
import shutil

def install(project_dir: Path, repo: Path) -> None:
    java = project_dir / 'android/app/src/main/java/com/local/inhousenotes'
    java.mkdir(parents=True, exist_ok=True)
    for source in (repo / '.github/android/background').glob('*.java'):
        shutil.copy2(source, java / source.name)
    activity = java / 'MainActivity.java'
    text = activity.read_text()
    anchor = 'webView.addJavascriptInterface(new InhouseNativeBridge(), "InhouseNative");'
    if anchor not in text:
        raise RuntimeError('Cannot locate the trusted Notes WebView host')
    if 'InhouseBackgroundSync.attach' not in text:
        text = text.replace(anchor, anchor + '\n        InhouseBackgroundSync.attach(this, webView);')
    activity.write_text(text)
    gradle = project_dir / 'android/app/build.gradle'
    text = gradle.read_text()
    dependency = "implementation 'androidx.work:work-runtime:2.10.1'"
    if dependency not in text:
        text += '\ndependencies {\n    ' + dependency + '\n    implementation \'androidx.webkit:webkit:1.12.1\'\n}\n'
    gradle.write_text(text)
