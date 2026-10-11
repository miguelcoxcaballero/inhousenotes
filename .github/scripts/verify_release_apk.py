#!/usr/bin/env python3
"""Verify the public APK and signing continuity before publishing a catalog."""
import os
import re
import subprocess
import sys
from pathlib import Path

sdk = Path(os.environ.get("ANDROID_HOME") or os.environ["ANDROID_SDK_ROOT"])
build_tools = sorted((sdk / "build-tools").iterdir(), key=lambda p: [int(n) for n in re.findall(r"\d+", p.name)])[-1]
apk, previous = map(Path, sys.argv[1:])


def certificate(path):
    output = subprocess.check_output([str(build_tools / "apksigner"), "verify", "--print-certs", str(path)], text=True)
    return re.search(r"Signer #1 certificate SHA-256 digest: ([a-f0-9]+)", output)[1]


assert certificate(apk) == certificate(previous), "Signing key changed"
badging = subprocess.check_output([str(build_tools / "aapt"), "dump", "badging", str(apk)], text=True)
assert "package: name='com.local.inhousenotes' versionCode='14' versionName='1.0.13'" in badging, "APK identity/version mismatch"
print("Public APK verified: com.local.inhousenotes 1.0.13 / 14; signing key unchanged.")
