#!/usr/bin/env python3
"""Packs the vector data of the Isla Levante map (metres) into the compact
file the game's radar and pause map read (`data/maps/isla/minimap.bin`):
land, water, green areas, building footprints, roads by class (with their
one-way flag, for the GPS) and points of interest.

Usage: isla_minimap.py <isla-v5/data dir> <out minimap.bin>

Layout (gzip, little endian; coordinates are u16 steps of `scale` metres
from `(x0, y0)`, island axes: +x east, +y north):
  "FJMM", u32 version (1), f32 x0, f32 y0, f32 scale
  u32 areas:  u8 kind, u8 rings, then per ring u16 points + points
  u32 roads:  u8 class, u8 flags (1 one-way, 2 bridge, 4 tunnel), u16 points + points
  u32 pois:   u8 kind, u16 x, u16 y, u8 name bytes + UTF-8 name
The kinds and classes are those of `sandbox::minimap::data`.
"""
import gzip, json, math, os, re, struct, sys

src, out = sys.argv[1], sys.argv[2]
d = json.load(gzip.open(os.path.join(src, "isla-metros.geojson.gz")))

AREA = {
    "coast": 0, "water": 1, "river-water": 1, "lake": 1, "reservoir": 1,
    "green": 2, "designed-park": 2, "sports": 2, "forest": 3, "agriculture": 4, "beach": 5,
    "urban-area": 6, "industrial": 7, "airfield": 8, "wetland": 9, "rock": 10,
    "building": 11, "protected-landmark": 12, "landmark-footprint": 12, "flood-bed": 13, "flood-bank": 13, "airway": 14,
}
# Simplification tolerance (m) by area kind.
TOL = {0: 2.0, 1: 1.0, 2: 1.0, 3: 2.0, 4: 2.0, 5: 1.5, 6: 3.0, 7: 2.0, 8: 3.0, 9: 2.0, 10: 2.0, 11: 0.5, 12: 0.4, 13: 1.5, 14: 1.0}
ROAD = {
    "motorway": 0, "trunk": 1, "motorway_link": 1, "primary": 2, "trunk_link": 2, "secondary": 3, "primary_link": 3,
    "tertiary": 4, "secondary_link": 4, "residential": 5, "unclassified": 5, "living_street": 5, "tertiary_link": 5, "raceway": 5,
    "service": 6, "busway": 6, "track": 6, "rest_area": 6, "pedestrian": 7, "rail": 8, "tram": 8,
}
ROAD_KINDS = ("retained-road", "urban-arterial", "new-road", "bridge", "expressway", "secondary-road", "promenade", "railway", "circuit")

pts_all = []
for f in d["features"]:
    if f["properties"].get("kind") == "coast":
        for ring in f["geometry"]["coordinates"]:
            pts_all += ring
x0 = min(p[0] for p in pts_all) - 400.0
y0 = min(p[1] for p in pts_all) - 400.0
span = max(max(p[0] for p in pts_all) + 400.0 - x0, max(p[1] for p in pts_all) + 400.0 - y0)
scale = span / 65535.0


def q(p):
    return (max(0, min(65535, round((p[0] - x0) / scale))), max(0, min(65535, round((p[1] - y0) / scale))))


def simplify(pts, tol):
    """Douglas-Peucker."""
    if len(pts) < 3:
        return pts
    keep = [False] * len(pts)
    keep[0] = keep[-1] = True
    stack = [(0, len(pts) - 1)]
    while stack:
        a, b = stack.pop()
        ax, ay, bx, by = pts[a][0], pts[a][1], pts[b][0], pts[b][1]
        dx, dy = bx - ax, by - ay
        l2 = dx * dx + dy * dy
        worst, wi = tol, -1
        for i in range(a + 1, b):
            px, py = pts[i][0] - ax, pts[i][1] - ay
            t = 0.0 if l2 == 0 else max(0.0, min(1.0, (px * dx + py * dy) / l2))
            e = math.hypot(px - t * dx, py - t * dy)
            if e > worst:
                worst, wi = e, i
        if wi >= 0:
            keep[wi] = True
            stack += [(a, wi), (wi, b)]
    return [p for p, k in zip(pts, keep) if k]


def dedupe(qs):
    o = []
    for p in qs:
        if not o or o[-1] != p:
            o.append(p)
    return o


def ring_q(ring, tol):
    r = ring[:-1] if len(ring) > 1 and ring[0] == ring[-1] else ring
    if len(r) < 3:
        return None
    # Simplify as an open line from the vertex farthest from the first, so
    # both halves keep their shape.
    far = max(range(len(r)), key=lambda i: (r[i][0] - r[0][0]) ** 2 + (r[i][1] - r[0][1]) ** 2)
    s = simplify(r[: far + 1], tol)[:-1] + simplify(r[far:] + [r[0]], tol)[:-1]
    o = dedupe([q(p) for p in s])
    if len(o) > 1 and o[0] == o[-1]:
        o.pop()
    return o if 3 <= len(o) <= 65535 else None


def centroid(ring):
    return (sum(p[0] for p in ring) / len(ring), sum(p[1] for p in ring) / len(ring))


def ring_area(ring):
    return abs(sum(ring[i][0] * ring[(i + 1) % len(ring)][1] - ring[(i + 1) % len(ring)][0] * ring[i][1] for i in range(len(ring)))) * 0.5


POI_RULES = [
    (7, r"esgl[eé]sia|iglesia|catedral|bas[ií]lica|parr[oò]quia|convent|monest|ermita|capella|capilla"),
    (1, r"\bhospital\b"),
    (2, r"polic[ií]a|comisar|gu[aà]rdia civil|guardia civil"),
    (4, r"gasoliner|repsol|cepsa|\bbp\b|galp|estaci[oó]n de servicio"),
    (5, r"^estaci[oó]"),
    (3, r"mercadona|\bconsum\b|lidl|carrefour|supermerc|\baldi\b|maxi dia|hipercor|corte ingl|^mercat|^mercado"),
    (8, r"museu|museo|\bivam\b|oceanogr|hemisf"),
    (9, r"universitat|universidad|facultat|facultad|escola t[eè]cnica"),
    (10, r"^ajuntament|^ayuntamiento|^casa consistorial|palau de la generalitat"),
    (11, r"estadi|estadio|mestalla|pavell[oó]|pabell[oó]n|vel[oò]drom|pla[cç]a de bous|plaza de toros"),
    (12, r"teatre|teatro|palau de la m[uú]sica|auditori|cinema|[oò]pera"),
]

areas, roads, pois = [], [], []
seen_poi = []


def add_poi(kind, name, x, y):
    name = name.strip()[:60]
    for k, n, px, py in seen_poi:
        if (k == kind or n == name) and math.hypot(px - x, py - y) < (150.0 if n == name else 40.0):
            return
    seen_poi.append((kind, name, x, y))
    pois.append((kind, q((x, y)), name))


# The modelled landmarks first, at the centre of the footprint their model
# stands on (data/maps/isla/landmarks.toml, next to the output); the pins of
# landmarks.json that mark the same building are then duplicates.
modelled = os.path.join(os.path.dirname(os.path.abspath(out)), "landmarks.toml")
landmark_spots = []
if os.path.exists(modelled):
    import tomllib

    for lm in tomllib.load(open(modelled, "rb")).get("landmark", []):
        c = lm.get("centre") or lm.get("island")
        if c:
            add_poi(0, lm.get("label") or lm.get("name") or lm["slug"], c[0], c[1])
            landmark_spots.append((c, lm.get("pin")))

for lm in json.load(open(os.path.join(src, "landmarks.json"))):
    if lm.get("xy") and lm.get("name"):
        if any(pin and math.hypot(pin[0] - lm["xy"][0], pin[1] - lm["xy"][1]) < 1.0 for _, pin in landmark_spots):
            continue
        add_poi(0, lm["name"], lm["xy"][0], lm["xy"][1])

for f in d["features"]:
    p, g = f["properties"], f["geometry"]
    kind, t = p.get("kind"), g["type"]
    if t in ("Polygon", "MultiPolygon") and kind in AREA:
        k = AREA[kind]
        polys = [g["coordinates"]] if t == "Polygon" else g["coordinates"]
        for poly in polys:
            if not poly or len(poly[0]) < 4:
                continue
            a = ring_area(poly[0])
            if (k == 11 and a < 12.0) or (k not in (11, 12) and a < 30.0):
                continue
            rings = [ring_q(poly[0], TOL[k])]
            if rings[0] is None:
                continue
            for hole in poly[1:]:
                if ring_area(hole) >= (60.0 if k in (11, 12) else 400.0):
                    h = ring_q(hole, TOL[k])
                    if h and len(rings) < 255:
                        rings.append(h)
            areas.append((k, rings))
        name = p.get("name") or ""
        if kind == "building" and name:
            low = name.lower()
            for pk, rx in POI_RULES:
                if re.search(rx, low):
                    if pk == 1 and re.search(r"veterin|odontol", low):
                        break
                    c = centroid(polys[0][0])
                    add_poi(pk, name, c[0], c[1])
                    break
        elif kind == "airfield" and name:
            c = centroid(polys[0][0])
            add_poi(6, name, c[0], c[1])
    elif t == "Point" and kind == "station" and p.get("name"):
        c = g["coordinates"]
        add_poi(6 if p.get("id") == "airport" else 5, p["name"], c[0], c[1])

# Junction vertices dropped by the simplification of one of the roads that
# share them would disconnect the GPS graph: put back every original vertex
# that another road also uses.
use = {}
for f in d["features"]:
    p, g = f["properties"], f["geometry"]
    if g["type"] in ("LineString", "MultiLineString") and p.get("kind") in ROAD_KINDS:
        for line in [g["coordinates"]] if g["type"] == "LineString" else g["coordinates"]:
            for pt in set(q(pt) for pt in line):
                use[pt] = use.get(pt, 0) + 1
shared = {k for k, v in use.items() if v >= 2}
roads = []
for f in d["features"]:
    p, g = f["properties"], f["geometry"]
    kind, t = p.get("kind"), g["type"]
    if t not in ("LineString", "MultiLineString"):
        continue
    if kind == "airway":
        cls, flags = 9, 0
    elif kind in ROAD_KINDS:
        rc = p.get("roadclass")
        if rc is None:
            rc = {"secondary-road": "secondary", "promenade": "pedestrian", "railway": "rail", "circuit": "residential"}.get(kind, "residential")
        if rc not in ROAD:
            continue
        cls = ROAD[rc]
        flags = (1 if p.get("oneway") == "yes" else 0) | (2 if p.get("bridge") == "yes" or kind == "bridge" else 0) | (4 if p.get("tunnel") == "yes" else 0)
        if cls == 8 and flags & 4:
            continue
    else:
        continue
    for line in [g["coordinates"]] if t == "LineString" else g["coordinates"]:
        # Simplify between the vertices that must stay.
        o, run = [], []
        for pt in line:
            run.append(pt)
            if q(pt) in shared and len(run) > 1:
                o += [q(v) for v in simplify(run, 0.6)][:-1]
                run = [pt]
        o += [q(v) for v in simplify(run, 0.6)]
        o = dedupe(o)
        for i in range(0, len(o) - 1, 60000):
            part = o[i : i + 60001]
            if len(part) >= 2:
                roads.append((cls, flags, part))

areas.sort(key=lambda a: a[0])
buf = bytearray()
buf += b"FJMM" + struct.pack("<Ifff", 1, x0, y0, scale)
buf += struct.pack("<I", len(areas))
for k, rings in areas:
    buf += struct.pack("<BB", k, len(rings))
    for r in rings:
        buf += struct.pack("<H", len(r)) + struct.pack(f"<{len(r) * 2}H", *[c for pt in r for c in pt])
buf += struct.pack("<I", len(roads))
for cls, flags, pts in roads:
    buf += struct.pack("<BBH", cls, flags, len(pts)) + struct.pack(f"<{len(pts) * 2}H", *[c for pt in pts for c in pt])
buf += struct.pack("<I", len(pois))
for k, pt, name in pois:
    nb = name.encode("utf-8")[:255]
    buf += struct.pack("<BHHB", k, pt[0], pt[1], len(nb)) + nb
os.makedirs(os.path.dirname(os.path.abspath(out)), exist_ok=True)
with open(out, "wb") as fh:
    with gzip.GzipFile(fileobj=fh, mode="wb", compresslevel=9, mtime=0, filename="") as z:
        z.write(bytes(buf))
print(f"{len(areas)} areas, {len(roads)} roads, {len(pois)} pois, {len(buf) / 1e6:.2f} MB raw, {os.path.getsize(out) / 1e6:.2f} MB packed, scale {scale:.3f} m")
by = {}
for k, _, n in pois:
    by[k] = by.get(k, 0) + 1
print("pois by kind", sorted(by.items()))
