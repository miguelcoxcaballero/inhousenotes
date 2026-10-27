#!/usr/bin/env python3
"""Fetch Wikimedia Commons reference photos for the forja_flora species.

    python3 -I tools/flora_photos.py OUT_DIR [species ...]

Searches Commons (file namespace) for each species' query, keeps the first
JPEG results and saves them as OUT_DIR/<species>_<n>.jpg (1280 px wide via
Special:FilePath). At most one request every 2 seconds, with the engine's
User-Agent. Responses are untrusted: only titles from the JSON are used,
file names are sanitised, and nothing is executed. Writes OUT_DIR/SOURCES.txt
with the Commons page of every file (check each file's licence there before
redistributing a photo; the engine only uses them for visual comparison).
"""

import json
import os
import re
import sys
import time
import urllib.parse
import urllib.error
import urllib.request

UA = "ForjaEngine/0.1 (https://github.com/miguelcoxcaballero/inhousenotes)"
API = "https://commons.wikimedia.org/w/api.php"
FILEPATH = "https://commons.wikimedia.org/wiki/Special:FilePath/"

QUERIES = {
    "platanus": ["Platanus hispanica street trees Madrid", "Platanus x acerifolia bark"],
    "naranjo": ["Citrus aurantium street Valencia", "Citrus aurantium tree fruit street"],
    "olivo": ["Olea europaea old tree", "Olea europaea trunk"],
    "ficus": ["Ficus macrophylla Glorieta Valencia", "Ficus macrophylla roots"],
    "jacaranda": ["Jacaranda mimosifolia Madrid", "Jacaranda mimosifolia tree flowering"],
    "almez": ["Celtis australis tree", "Celtis australis bark"],
    "pino_pinonero": ["Pinus pinea tree", "Pinus pinea bark"],
    "cipres": ["Cupressus sempervirens tree", "Cupressus sempervirens stricta"],
    "phoenix": ["Phoenix dactylifera Valencia", "Phoenix dactylifera trunk"],
    "washingtonia": ["Washingtonia robusta Valencia", "Washingtonia robusta skirt"],
    "broadleaf": ["Tilia platyphyllos tree"],
    "conifer": ["Picea abies tree"],
}

_last = [0.0]


def get(url, tries=4):
    """GET with >= 2 s between requests and a long back-off on HTTP 429."""
    for k in range(tries):
        wait = 2.5 - (time.time() - _last[0])
        if wait > 0:
            time.sleep(wait)
        _last[0] = time.time()
        req = urllib.request.Request(url, headers={"User-Agent": UA})
        try:
            with urllib.request.urlopen(req, timeout=60) as r:
                return r.read(25 * 1024 * 1024)
        except urllib.error.HTTPError as e:
            if e.code != 429 or k + 1 == tries:
                raise
            time.sleep(30 * (k + 1))
    raise RuntimeError("unreachable")


def search(q, limit=8):
    params = {"action": "query", "list": "search", "srsearch": q, "srnamespace": "6", "srlimit": str(limit), "format": "json"}
    data = json.loads(get(API + "?" + urllib.parse.urlencode(params)).decode("utf-8", "replace"))
    out = []
    for hit in data.get("query", {}).get("search", []):
        t = str(hit.get("title", ""))
        if t.startswith("File:") and re.search(r"\.jpe?g$", t, re.I):
            out.append(t[5:])
    return out


def main():
    if len(sys.argv) < 2:
        print(__doc__)
        return 2
    out = sys.argv[1]
    os.makedirs(out, exist_ok=True)
    wanted = sys.argv[2:] or list(QUERIES)
    sources = []
    for sp in wanted:
        n = 0
        for q in QUERIES.get(sp, []):
            try:
                names = search(q)
            except Exception as e:  # network / JSON errors: skip this query
                print(f"{sp}: search failed: {e}")
                continue
            for name in names[:2]:
                if n >= 3:
                    break
                url = FILEPATH + urllib.parse.quote(name.replace(" ", "_")) + "?width=1280"
                try:
                    data = get(url)
                except Exception as e:
                    print(f"{sp}: {name}: {e}")
                    continue
                if not data.startswith(b"\xff\xd8"):
                    print(f"{sp}: {name}: not a JPEG, skipped")
                    continue
                path = os.path.join(out, f"{sp}_{n}.jpg")
                with open(path, "wb") as f:
                    f.write(data)
                sources.append(f"{os.path.basename(path)}\thttps://commons.wikimedia.org/wiki/File:{urllib.parse.quote(name.replace(' ', '_'))}")
                print(f"{sp}: {name} -> {path}")
                n += 1
    with open(os.path.join(out, "SOURCES.txt"), "a", encoding="utf-8") as f:
        f.write("\n".join(sources) + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
