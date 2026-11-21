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
# Inhouse Notes 6 (docs/v6/ARCHITECTURE.md §14.2): the v6 shell, the generated files committed by the release, the
# A/B catalogs, and the 5.18.0 runtime that stays deployed (unreferenced) for still-open 5.18.0 tabs until 6.0.1 (§10.4).
names = ['index.html', 'boot.js', 'service-worker.js', 'offline-assets.js', 'native-assets.json', 'wasm/ihn.wasm', 'wasm/ihn-2.wasm',
         'offline-v5.css', 'vendor/fonts/ui.css', 'updates/A.json', 'updates/B.json']
names += sorted(f'host/{path.name}' for path in (root / 'host').glob('*.js'))
expected = {name: hashlib.sha256((root / name).read_bytes()).hexdigest() for name in names}
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
        print(f'PUBLIC RELEASE VERIFIED: https://inhousenotes.com/ v{version}; commit {revision}; {len(expected)} files match (shell, wasm, service worker, A/B catalogs, 5.18.0 runtime).', flush=True)
        break
    except Exception as error:
        if time.monotonic() >= deadline:
            raise SystemExit(f'Public release verification failed: {error}')
        print(f'Waiting for public release v{version} (attempt {attempt}): {error}', flush=True)
        time.sleep(5)
