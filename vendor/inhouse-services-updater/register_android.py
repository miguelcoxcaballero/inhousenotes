#!/usr/bin/env python3
"""Wire the SAME installer into generated Read/Notes shells; no second downloader.

Canonical updater changes must be synced to all five apps. A + tiny changelog
for every new release; B only on the user's explicit request.
"""
from pathlib import Path
import re, shutil, xml.etree.ElementTree as ET

def register(project, repository, trust_read=False):
    project,repository=Path(project),Path(repository)
    app=project/'android/app';shared=repository/'vendor/inhouse-services-updater/android'
    target=app/'src/main/java/com/inhouse/services/updater';target.mkdir(parents=True,exist_ok=True)
    for file in shared.iterdir():
        if file.suffix in ('.java','.kt'):shutil.copyfile(file,target/file.name)
    candidates=list((app/'src/main/java').rglob('MainActivity.java'))
    if len(candidates)!=1:raise ValueError('Expected exactly one Java shell activity')
    activity=candidates[0];source=activity.read_text()
    found=re.search(r'public void installAppUpdate\(String url(?:, String (\w+))?\)\s*\{',source)
    if not found:raise ValueError('Generated shell updater entrypoint changed')
    # Replace only the old installer method body; existing progress callbacks remain.
    start=found.end();depth=1;end=start
    while depth:
        if source[end]=='{':depth+=1
        elif source[end]=='}':depth-=1
        end+=1
    argument=found.group(1) or 'null'
    source=source[:start]+'\n            installInhouseUpdate(url, '+argument+');\n        '+source[end-1:]
    bridge='''
        // Inhouse services updater: change central repo first and sync all five apps.
        // A + tiny changelog by default. B needs explicit user request. Updates are optional.
        private com.inhouse.services.updater.PhotosUpdateInstaller servicesInstaller;
        @JavascriptInterface public long getAppVersionCode() { return com.inhouse.services.updater.UpdateState.versionCode(MainActivity.this); }
        @JavascriptInterface public String getInstallState() { return com.inhouse.services.updater.InstallState.read(MainActivity.this); }
        @JavascriptInterface public String getUpdateState() { return com.inhouse.services.updater.UpdateState.read(MainActivity.this); }
        @JavascriptInterface public void saveUpdateState(String value) { com.inhouse.services.updater.UpdateState.save(MainActivity.this,value); }
        @JavascriptInterface public void installInhouseUpdate(String url,String hash) {
            runOnUiThread(() -> {
                TRUST_CHECK
                if(servicesInstaller==null) servicesInstaller=new com.inhouse.services.updater.PhotosUpdateInstaller(MainActivity.this,new com.inhouse.services.updater.PhotosUpdateInstaller.Listener(){
                    public void progress(long downloaded,long total){notifyAppUpdateProgress(downloaded,total);}
                    public void stage(String value){if("verifying".equals(value))notifyAppUpdateResult("downloading","Verificando…");}
                });
                servicesInstaller.installUpdate(url,hash,new com.inhouse.services.updater.PhotosUpdateInstaller.Result(){
                    public void success(String value){notifyAppUpdateResult(value,"");}
                    public void error(String code,String message,Object details){notifyAppUpdateResult("error",message);}
                });
            });
        }
'''.replace('TRUST_CHECK','if(!isTrustedReadPage())return;' if trust_read else '')
    marker='public class InhouseNativeBridge {'
    if source.count(marker)!=1:raise ValueError('Expected one trusted native bridge')
    source=source.replace(marker,marker+bridge);activity.write_text(source)
    root_gradle=project/'android/build.gradle';text=root_gradle.read_text()
    if 'org.jetbrains.kotlin:kotlin-gradle-plugin' not in text:
        text=text.replace('dependencies {',"dependencies {\n        classpath 'org.jetbrains.kotlin:kotlin-gradle-plugin:2.0.21'",1)
    root_gradle.write_text(text)
    gradle=app/'build.gradle';text=gradle.read_text()
    if "apply plugin: 'org.jetbrains.kotlin.android'" not in text:text=text.replace("apply plugin: 'com.android.application'", "apply plugin: 'com.android.application'\napply plugin: 'org.jetbrains.kotlin.android'", 1)
    # Capacitor's Java target and Kotlin must match. Preserve the generated target.
    target_version=re.search(r'targetCompatibility JavaVersion.VERSION_(\d+)',text)
    java_target=target_version.group(1) if target_version else '21'
    text+='\nandroid { kotlinOptions { jvmTarget = "'+java_target+'" } }\n'
    gradle.write_text(text)
    ns='http://schemas.android.com/apk/res/android';ET.register_namespace('android',ns)
    path=app/'src/main/AndroidManifest.xml';tree=ET.parse(path);application=tree.getroot().find('application')
    install_activity='com.inhouse.services.updater.InstallActivity'
    if not any(node.get(f'{{{ns}}}name')==install_activity for node in application.findall('activity')):
        ET.SubElement(application,'activity',{f'{{{ns}}}name':install_activity,f'{{{ns}}}exported':'false',f'{{{ns}}}theme':'@android:style/Theme.DeviceDefault.NoActionBar'})
    provider=ET.SubElement(application,'provider',{f'{{{ns}}}name':'androidx.core.content.FileProvider',f'{{{ns}}}authorities':'${applicationId}.update-provider',f'{{{ns}}}exported':'false',f'{{{ns}}}grantUriPermissions':'true'})
    ET.SubElement(provider,'meta-data',{f'{{{ns}}}name':'android.support.FILE_PROVIDER_PATHS',f'{{{ns}}}resource':'@xml/inhouse_update_paths'})
    tree.write(path,encoding='unicode')
    xml=app/'src/main/res/xml/inhouse_update_paths.xml';xml.parent.mkdir(parents=True,exist_ok=True)
    xml.write_text('<?xml version="1.0" encoding="utf-8"?><paths><cache-path name="updates" path="updates/"/></paths>\n')
