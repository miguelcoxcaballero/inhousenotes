#!/usr/bin/env python3
"""Verify the public APK and signing continuity before publishing a catalog."""
import os
import re
import subprocess
import sys
from pathlib import Path

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
assert "package: name='com.local.inhousenotes' versionCode='16' versionName='1.0.15'" in badging, "APK identity/version mismatch"
assert "uses-permission: name='android.permission.CAMERA'" in badging, "APK cannot request a live camera"
assert "uses-feature: name='android.hardware.camera'" not in badging, "Camera must remain optional for photo import"
print("Public APK verified: com.local.inhousenotes 1.0.15 / 16; signing key unchanged.")
