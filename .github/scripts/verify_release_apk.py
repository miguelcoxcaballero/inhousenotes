#!/usr/bin/env python3
"""Verify APK identity and signing continuity before release and catalog publication."""
import os
import re
import subprocess
import sys
from pathlib import Path
from zipfile import ZipFile

sdk = Path(os.environ.get("ANDROID_HOME") or os.environ["ANDROID_SDK_ROOT"])
stable_tools = [p for p in (sdk / "build-tools").iterdir() if re.fullmatch(r"\d+\.\d+\.\d+", p.name)]
# Preview SDK tools may change their certificate-reporting format. Prefer the
# release toolchain used to verify all five app packages.
build_tools = sdk / "build-tools" / "36.0.0"
if not build_tools.is_dir():
    build_tools = sorted(stable_tools, key=lambda p: [int(n) for n in p.name.split(".")])[-1]
apk, previous = map(Path, sys.argv[1:])


def certificate(path):
    output = subprocess.check_output([str(build_tools / "apksigner"), "verify", "--verbose", "--print-certs", str(path)], text=True)
    digests = re.findall(r"certificate SHA-256 digest:\s*([a-f0-9]{64})", output, flags=re.IGNORECASE)
    assert digests, "Android signing tool did not report a certificate digest"
    return sorted(set(d.lower() for d in digests))


assert certificate(apk) == certificate(previous), "Signing key changed"
badging = subprocess.check_output([str(build_tools / "aapt"), "dump", "badging", str(apk)], text=True)
assert "package: name='com.local.inhousenotes' versionCode='17' versionName='1.0.16'" in badging, "APK identity/version mismatch"
assert "uses-permission: name='android.permission.CAMERA'" in badging, "APK cannot request a live camera"
assert "uses-feature: name='android.hardware.camera'" not in badging, "Camera must remain optional for photo import"
manifest = subprocess.check_output([str(build_tools / "aapt"), "dump", "xmltree", str(apk), "AndroidManifest.xml"], text=True)
assert 'com.inhouse.services.updater.InstallActivity' in manifest, "Installer result activity is not registered"
with ZipFile(apk) as archive:
    dex = b''.join(archive.read(name) for name in archive.namelist() if name.endswith('.dex'))
    for symbol in (b'InstallActivity', b'InstallState', b'InstallResultPolicy', b'getInstallState'):
        assert symbol in dex, f"Installer result implementation missing: {symbol!r}"
print("APK verified: com.local.inhousenotes 1.0.16 / 17; signing key unchanged.")
