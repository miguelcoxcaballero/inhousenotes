"""Register the independent OAuth provider/bridge. Does not touch the frozen updater."""
from pathlib import Path
import shutil
import xml.etree.ElementTree as ET

PACKAGES = ['com.local.inhousenotes', 'com.inhousesoftware.read', 'com.inhouse.paellas']
def register(project, repository, app_id):
    project, repository = Path(project), Path(repository)
    app = project / 'android/app'
    shared = repository / 'vendor/inhouse-services-updater/auth'
    target = app / 'src/main/java/com/inhouse/services/oauth'
    target.mkdir(parents=True, exist_ok=True)
    for name in ['GoogleSessionProvider.java', 'GoogleSessionBridge.java']:
        shutil.copyfile(shared / name, target / name)
    ns = 'http://schemas.android.com/apk/res/android'; ET.register_namespace('android', ns)
    attr = lambda key: '{' + ns + '}' + key
    manifest = app / 'src/main/AndroidManifest.xml'; tree = ET.parse(manifest, parser=ET.XMLParser(target=ET.TreeBuilder(insert_comments=True))); root = tree.getroot()
    queries = root.find('queries')
    if queries is None: queries = ET.SubElement(root, 'queries')
    for package in PACKAGES:
        if not any(node.get(attr('name')) == package for node in queries.findall('package')):
            ET.SubElement(queries, 'package', {attr('name'): package})
    application = root.find('application')
    provider = 'com.inhouse.services.oauth.GoogleSessionProvider'
    if not any(node.get(attr('name')) == provider for node in application.findall('provider')):
        ET.SubElement(application, 'provider', {attr('name'): provider,
            attr('authorities'): '${applicationId}.inhouse-google-session', attr('exported'): 'true',
            attr('grantUriPermissions'): 'false'})
    ET.indent(tree, space='    ')
    tree.write(manifest, encoding='utf-8', xml_declaration=True)
    manifest.write_text(manifest.read_text() + '\n')
    gradle = app / 'build.gradle'
    if gradle.exists():
        content = gradle.read_text()
        if 'androidx.webkit:webkit:' not in content:
            gradle.write_text(content + '\ndependencies { implementation "androidx.webkit:webkit:1.12.1" }\n')
    candidates = list((app / 'src/main/java').rglob('MainActivity.java'))
    if len(candidates) != 1: raise ValueError('Expected one Java activity')
    activity = candidates[0]; source = activity.read_text()
    if 'GoogleSessionBridge.install(' not in source:
        origins = {'notes': ['https://inhousenotes.com', 'https://localhost'],
                   'read': ['https://miguelcoxcaballero.github.io', 'https://localhost'],
                   'paellas': ['https://paellas.net']}[app_id]
        if app_id == 'read':
            # Production loader origin is supplied by Read's existing trusted URL constant.
            origins = ['https://miguelcoxcaballero.github.io', 'https://localhost']
        statement = 'com.inhouse.services.oauth.GoogleSessionBridge.install(webView, new java.util.HashSet<>(java.util.Arrays.asList(' + ', '.join('"' + o + '"' for o in origins) + ')));'
        marker = 'webView.addJavascriptInterface(new InhouseNativeBridge(), "InhouseNative");'
        if app_id == 'paellas': marker = 'configureWebView();'
        if source.count(marker) != 1: raise ValueError('OAuth bridge host changed')
        source = source.replace(marker, marker + '\n        ' + statement)
        activity.write_text(source)
