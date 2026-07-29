#!/usr/bin/env python3
"""Headless Android build using the Inhouse APK Builder implementation."""

from __future__ import annotations

import importlib.util
import os
import shutil
import subprocess
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
BUILDER_PATH = REPO_ROOT / "android app" / "html_to_apk_builder.py"
SOURCE_HTML = REPO_ROOT / "rebuild-v2" / "dist" / "index.html"
ICON_PATH = REPO_ROOT / "android app" / "Inhouse Notes Logo.png"
OUTPUT_APK = REPO_ROOT / "inhouse-notes-release-v1.0.1.apk"
APP_NAME = "Inhouse Notes"
PACKAGE_ID = "com.local.inhousenotes"
ANDROID_VERSION_NAME = "1.0.1"
ANDROID_VERSION_CODE = 2


class Value:
    def __init__(self, value):
        self.value = value

    def get(self):
        return self.value


def run(command: list[str], cwd: Path) -> None:
    print("+", " ".join(command), flush=True)
    subprocess.run(command, cwd=cwd, check=True)


def load_builder_module():
    spec = importlib.util.spec_from_file_location("inhouse_apk_builder", BUILDER_PATH)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"Cannot load builder: {BUILDER_PATH}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def main() -> None:
    runner_temp = Path(os.environ["RUNNER_TEMP"]).resolve()
    project_dir = runner_temp / "inhousenotes-android-build"
    keystore_path = Path(os.environ["INHOUSE_KEYSTORE_PATH"]).resolve()
    keystore_password = os.environ["INHOUSE_KEYSTORE_PASSWORD"]
    key_alias = os.environ["INHOUSE_KEY_ALIAS"]

    if project_dir.exists():
        shutil.rmtree(project_dir)
    www_dir = project_dir / "www"
    www_dir.mkdir(parents=True)

    module = load_builder_module()
    builder = object.__new__(module.ApkBuilderApp)
    builder.log = lambda message, tag=None: print(message, flush=True)
    builder.copy_sibling_assets = Value(False)
    builder.permission_internet = Value(True)
    builder.permission_location = Value(False)
    builder.version_name = Value(ANDROID_VERSION_NAME)
    builder.version_code = Value(ANDROID_VERSION_CODE)
    builder.keystore_path = Value(str(keystore_path))
    builder.keystore_password = Value(keystore_password)
    builder.key_alias = Value(key_alias)

    builder.copy_web_files(SOURCE_HTML, www_dir)
    builder.write_node_project(project_dir, APP_NAME, PACKAGE_ID, SOURCE_HTML)

    run([module.tool("npm"), "install", "--no-audit", "--no-fund", "--loglevel=error"], project_dir)
    run([module.tool("npx"), "cap", "add", "android"], project_dir)
    run([module.tool("npx"), "cap", "sync", "android"], project_dir)

    builder.patch_manifest(project_dir, APP_NAME, PACKAGE_ID)
    builder.patch_oauth_persistence(project_dir, PACKAGE_ID)
    builder.patch_gradle_versions(project_dir)
    builder.patch_android_dependencies(project_dir)
    builder.write_gradle_properties(project_dir)
    builder.apply_launcher_icon(project_dir, ICON_PATH)
    builder.write_signing_properties(project_dir)
    builder.patch_release_signing(project_dir)

    gradlew = project_dir / "android" / "gradlew"
    gradlew.chmod(gradlew.stat().st_mode | 0o111)
    run([str(gradlew), "assembleRelease", "--console=plain", "--warning-mode=none"], project_dir / "android")

    built_apk = builder.find_apk(project_dir, "release")
    shutil.copy2(built_apk, OUTPUT_APK)
    print(f"APK created: {OUTPUT_APK} ({OUTPUT_APK.stat().st_size} bytes)", flush=True)


if __name__ == "__main__":
    main()
