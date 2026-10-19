"""Exercise the generated native refresh policy and compile Android API calls."""
import importlib.util
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("ihn_android_builder", ROOT / "android app/html_to_apk_builder.py")
BUILDER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(BUILDER)
PACKAGE = "com.local.inhousenotes"


class Value:
    def __init__(self, value):
        self.value = value

    def get(self):
        return self.value


def make_builder():
    builder = object.__new__(BUILDER.ApkBuilderApp)
    builder.log = lambda *args: None
    builder.version_name = Value("1.0.test")
    builder.permission_internet = Value(True)
    builder.permission_location = Value(False)
    return builder


def java_tool(name):
    candidate = Path(os.environ.get("JAVA_HOME", "/nonexistent")) / "bin" / name
    return str(candidate) if candidate.exists() else shutil.which(name)


def android_jar():
    explicit = os.environ.get("ANDROID_JAR")
    if explicit and Path(explicit).is_file():
        return explicit
    for variable in ("ANDROID_HOME", "ANDROID_SDK_ROOT"):
        sdk = os.environ.get(variable)
        if not sdk:
            continue
        platforms = Path(sdk) / "platforms"
        jars = [(int(path.parent.name.split("-")[-1]), path)
                for path in platforms.glob("android-*/android.jar")
                if path.parent.name.split("-")[-1].isdigit()]
        if jars:
            version, path = max(jars)
            if version >= 35:
                return str(path)
    return None


class AndroidRefreshRateTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="ihn-refresh-test-")
        self.addCleanup(self.temporary.cleanup)
        self.project = Path(self.temporary.name)
        self.source = self.project / "android/app/src/main"
        self.java = self.source / "java/com/local/inhousenotes"
        self.builder = make_builder()
        self.builder.patch_display_performance(self.project, PACKAGE)

    def test_generated_java_policy_selects_supported_resolution_and_refresh(self):
        javac, java = java_tool("javac"), java_tool("java")
        if not javac or not java:
            self.skipTest("JDK required for native policy execution")
        harness = self.java / "RefreshRatePolicyTest.java"
        harness.write_text('''package com.local.inhousenotes;
public class RefreshRatePolicyTest {
    private static void expect(int expected, int current, int[] ids, int[] widths, int[] heights, float[] rates) {
        int actual = InhouseRefreshRatePolicy.selectMode(1440, 3200, current, ids, widths, heights, rates);
        if (actual != expected) throw new AssertionError("Expected mode index " + expected + ", got " + actual);
    }
    public static void main(String[] args) {
        expect(2, 1, new int[]{1,2,3,4}, new int[]{1440,1440,1440,1440},
            new int[]{3200,3200,3200,3200}, new float[]{60,90,120,144});
        expect(1, 1, new int[]{1,2,3}, new int[]{1440,1440,1080},
            new int[]{3200,3200,2400}, new float[]{60,90,120});
        expect(0, 1, new int[]{1}, new int[]{1440}, new int[]{3200}, new float[]{60});
        expect(1, 1, new int[]{1,2}, new int[]{1440,1440}, new int[]{3200,3200}, new float[]{60,90});
        expect(1, 2, new int[]{1,2}, new int[]{1440,1440}, new int[]{3200,3200}, new float[]{120,120});
        expect(0, 1, new int[]{1,2}, new int[]{1440,1440}, new int[]{3200,3200}, new float[]{120,120});
        expect(1, 1, new int[]{1,2}, new int[]{1440,1440}, new int[]{3200,3200}, new float[]{60,119.88f});
        expect(-1, 1, new int[]{1,2,3,4}, new int[]{1440,1440,1440,1440},
            new int[]{3200,3200,3200,3200}, new float[]{Float.NaN,Float.POSITIVE_INFINITY,-1,144});
        expect(-1, 1, new int[]{1}, new int[]{1080}, new int[]{2400}, new float[]{120});
        expect(-1, 1, new int[]{}, new int[]{}, new int[]{}, new float[]{});
        System.out.println("10 native display selection cases passed");
    }
}
''')
        classes = self.project / "classes"
        subprocess.run([javac, "-d", str(classes), str(self.java / "InhouseRefreshRatePolicy.java"), str(harness)], check=True)
        subprocess.run([java, "-cp", str(classes), PACKAGE + ".RefreshRatePolicyTest"], check=True)

    def test_controller_compiles_against_real_android_api(self):
        javac, jar = java_tool("javac"), android_jar()
        if not javac or not jar:
            self.skipTest("JDK and Android SDK 35+ required for Android API compilation")
        subprocess.run([javac, "-Xlint:deprecation", "-classpath", jar,
                        "-d", str(self.project / "android-classes"),
                        str(self.java / "InhouseRefreshRatePolicy.java"),
                        str(self.java / "InhouseDisplayPerformance.java")], check=True)

    def test_manifest_and_java_kotlin_hosts_activate_the_controller(self):
        manifest = self.source / "AndroidManifest.xml"
        manifest.write_text('''<manifest xmlns:android="http://schemas.android.com/apk/res/android">
<application android:hardwareAccelerated="false"><activity android:name=".MainActivity" android:hardwareAccelerated="false" /></application></manifest>''')
        self.builder.patch_manifest(self.project, "Inhouse Notes", PACKAGE)
        application = ET.parse(manifest).getroot().find("application")
        attribute = "{http://schemas.android.com/apk/res/android}hardwareAccelerated"
        self.assertEqual(application.get(attribute), "true")
        self.assertEqual(application.find("activity").get(attribute), "true")
        java_activity = self.java / "MainActivity.java"
        java_activity.write_text("// Capacitor-generated activity")
        self.builder.patch_oauth_persistence(self.project, PACKAGE)
        java_source = java_activity.read_text()
        self.assertIn("new InhouseDisplayPerformance(this, webView)", java_source)
        for callback in ("resume", "pause", "refresh", "dispose"):
            self.assertIn("displayPerformance." + callback + "()", java_source)
        java_activity.unlink()
        kotlin_activity = self.source / "kotlin/com/local/inhousenotes/MainActivity.kt"
        kotlin_activity.parent.mkdir(parents=True)
        kotlin_activity.write_text("// Capacitor-generated activity")
        self.builder.patch_oauth_persistence(self.project, PACKAGE)
        kotlin_source = kotlin_activity.read_text()
        self.assertIn("InhouseDisplayPerformance(this, webView)", kotlin_source)
        for callback in ("resume", "pause", "refresh", "dispose"):
            self.assertIn("displayPerformance?." + callback + "()", kotlin_source)


if __name__ == "__main__":
    unittest.main()
