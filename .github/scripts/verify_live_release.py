#!/usr/bin/env python3
"""Verify the public Pages origin, bypassing intermediary caches after deployment."""
import hashlib
import os
from pathlib import Path
import re
import time
import urllib.request

root = Path(__file__).resolve().parents[2]
version = re.search(r"const APP_VERSION = '([^']+)'", (root / 'index.html').read_text())[1]
revision = os.environ['GITHUB_SHA']
expected = {name: hashlib.sha256((root / name).read_bytes()).hexdigest()
            for name in ('app-v5.js', 'runtime-core-v5.js', 'peer-transport-v5.js', 'document-processing-v5.js', 'document-processing-worker-v5.js', 'timeline-core-v5.js', 'live-collaboration-v5.js', 'service-worker.js', 'updates/A.json', 'updates/B.json',
                         'drive-ink-journal-v5.js', 'drive-ink-app-v5.js', 'offline-store-v5.js', 'offline-library-v5.js', 'offline-app-v5.js', 'offline-media-v5.js',
                         'offline-assets.js', 'vendor/fonts/ui.css', 'boot-v5.js', 'native-assets.json',
                         'rust-core-v5.js', 'wasm/inhouse_notes_core.wasm', 'ink-palette.js',
                         'scanner/processing/ink-vectorizer.js',
                         'scanner/index.html', 'scanner/configLoader.js', 'scanner/live-camera.js', 'scanner/live-camera.css',
                         'scanner/processing/detection-worker.js', 'scanner/processing/liveDetection.js',
                         'scanner/processing/capture-quality.js', 'scanner/processing/capture-quality-worker.js',
                         'scanner/processing/lightweight.js',
                         'scanner/processing/metric-surface.js')}
deadline = time.monotonic() + 180
attempt = 0
while True:
    attempt += 1
    try:
        def fetch(name):
            url = f'https://inhousenotes.com/{name}?verify_release={revision}&attempt={attempt}'
            request = urllib.request.Request(url, headers={'Cache-Control': 'no-cache, no-store', 'Pragma': 'no-cache'})
            with urllib.request.urlopen(request, timeout=20) as response:
                return response.read()
        html = fetch('').decode('utf-8')
        actual = re.search(r"const APP_VERSION = '([^']+)'", html)
        if not actual or actual[1] != version:
            raise RuntimeError('Pages has not activated the expected version yet')
        for name, digest in expected.items():
            if hashlib.sha256(fetch(name)).hexdigest() != digest:
                raise RuntimeError(f'Pages still serves a different {name}')
        print(f'PUBLIC RELEASE VERIFIED: https://inhousenotes.com/ v{version}; commit {revision}; app, scanner, Rust, service worker and A/B catalogs match.', flush=True)
        break
    except Exception as error:
        if time.monotonic() >= deadline:
            raise SystemExit(f'Public release verification failed: {error}')
        print(f'Waiting for public release v{version} (attempt {attempt}): {error}', flush=True)
        time.sleep(5)
