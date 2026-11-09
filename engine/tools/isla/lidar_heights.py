#!/usr/bin/env python3
"""Samples real building heights for Valencia from the IGN PNOA-LiDAR
building layer (wms-mapa-lidar.idee.es, layer edificios-h30, CC BY 4.0
scne.es) for buildings whose height in the island data is only the default
(two storeys, no OSM levels).

Usage: lidar_heights.py <isla-v5/data dir> <out.json> [max requests]

One GetFeatureInfo per building, at a point inside its footprint; a few
buildings per 200 m cell, largest first, so the whole city is covered with
few requests. Resumable: the output is the cache. `isla_pack.py` applies it
(sampled buildings exactly, their unsampled neighbours by the local median).
"""
import gzip, json, os, re, sys, time, urllib.request

OX, OY = 3478.569, 2582.763
E0, N0 = 725602.415, 4372190.758  # UTM 30N of the Plaça de l'Ajuntament (the world origin)
src, out = sys.argv[1], sys.argv[2]
limit = int(sys.argv[3]) if len(sys.argv) > 3 else 3000
per_cell = 3


def inside(ring, x, y):
    c = False
    j = len(ring) - 1
    for i in range(len(ring)):
        (xi, yi), (xj, yj) = ring[i][:2], ring[j][:2]
        if (yi > y) != (yj > y) and x < (xj - xi) * (y - yi) / (yj - yi) + xi:
            c = not c
        j = i
    return c


def area(r):
    return abs(sum(r[i][0] * r[(i + 1) % len(r)][1] - r[(i + 1) % len(r)][0] * r[i][1] for i in range(len(r)))) / 2


def interior(ring):
    cx = sum(p[0] for p in ring[:-1]) / (len(ring) - 1)
    cy = sum(p[1] for p in ring[:-1]) / (len(ring) - 1)
    if inside(ring, cx, cy):
        return cx, cy
    # A point just inside the longest edge.
    best = max(range(len(ring) - 1), key=lambda i: (ring[i][0] - ring[i + 1][0]) ** 2 + (ring[i][1] - ring[i + 1][1]) ** 2)
    (ax, ay), (bx, by) = ring[best][:2], ring[best + 1][:2]
    mx, my, dx, dy = (ax + bx) / 2, (ay + by) / 2, bx - ax, by - ay
    l = (dx * dx + dy * dy) ** 0.5 or 1
    for s in (2.5, -2.5):
        x, y = mx - dy / l * s, my + dx / l * s
        if inside(ring, x, y):
            return x, y
    return cx, cy


d = json.load(gzip.open(f"{src}/isla-metros.geojson.gz"))
cells = {}
for f in d["features"]:
    p, g = f["properties"], f["geometry"]
    if p.get("kind") != "building" or p.get("zone") != "valencia" or abs(float(p.get("height") or 0) - 6.4) > 0.01:
        continue
    ring = g["coordinates"][0] if g["type"] == "Polygon" else g["coordinates"][0][0]
    a = area(ring)
    if a < 60:
        continue
    x, y = interior(ring)
    cells.setdefault((int(x // 200), int(y // 200)), []).append((a, p["source_id"], x, y))
todo = []
for k in range(per_cell):
    for c in sorted(cells):
        v = sorted(cells[c], reverse=True)
        if k < len(v):
            todo.append(v[k])
D = json.load(open(out)) if os.path.exists(out) else {}
n = 0
for a, sid, x, y in todo:
    if sid in D:
        continue
    if n >= limit:
        break
    E, N = E0 + (x - OX), N0 + (y - OY)
    b = "%.2f,%.2f,%.2f,%.2f" % (E - 1.2, N - 1.2, E + 1.2, N + 1.2)
    url = "https://wms-mapa-lidar.idee.es/lidar?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetFeatureInfo&LAYERS=EL.GridCoverage&QUERY_LAYERS=EL.GridCoverage&STYLES=&CRS=EPSG:25830&BBOX=%s&WIDTH=3&HEIGHT=3&FORMAT=image/png&INFO_FORMAT=text/plain&I=1&J=1" % b
    for t in range(4):
        try:
            s = urllib.request.urlopen(urllib.request.Request(url, headers={"User-Agent": "ForjaEngine/0.1"}), timeout=30).read().decode()
            m = re.search(r"edificios-h30'.*?value_0 = '([-\d.]+)'", s, re.S)
            D[sid] = round(float(m.group(1)), 1) if m else None
            break
        except Exception as e:
            print("ERR", sid, e, flush=True)
            time.sleep(4 * (t + 1))
    n += 1
    if n % 50 == 0:
        json.dump(D, open(out, "w"), separators=(",", ":"), sort_keys=True)
        print(n, len(todo), flush=True)
    time.sleep(0.25)
json.dump(D, open(out, "w"), separators=(",", ":"), sort_keys=True)
print("DONE", len(D), "of", len(todo))
