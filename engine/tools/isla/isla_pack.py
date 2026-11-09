#!/usr/bin/env python3
"""Packs the whole Isla Levante map into the compact binary the engine's
island world reads (`forja_world::island::data`).

Usage: isla_pack.py <isla-v5/data dir> <out dir>

Writes `<out>/isla.bin.gz` (every polygon, line and point of the island in
engine world coordinates) and copies the height map as `<out>/height.png`.
Building heights are the island data's (OSM height, or levels x 3.2 m),
except Valencia's buildings without levels, which take real heights from
`<out>/lidar_heights.json` when it is there (see `lidar_heights.py`).

World frame: x = east, z = south, metres, origin at the Plaça de
l'Ajuntament (the frame of `data/landmarks/valencia`). The island keeps
Valencia at real scale, so the island's local metres are that frame moved by
a constant (fitted on 146 landmark footprints, residual 1 mm):

    x = island_x - 3478.569        z = -(island_y - 2582.763)

Format (little endian, then gzip): magic "ISLA", version, the string table,
then polygons, lines and points. Every integer is an unsigned LEB128 varint;
coordinates are centimetres, zigzag-encoded, the first point of a ring
absolute and the others deltas.
"""
import gzip, json, math, os, shutil, struct, sys, zlib

OX, OY = 3478.569, 2582.763

POLY = {"coast": 0, "building": 1, "water": 2, "river-water": 3, "lake": 4, "reservoir": 5, "green": 6, "designed-park": 7,
        "agriculture": 8, "forest": 9, "beach": 10, "wetland": 11, "rock": 12, "industrial": 13, "sports": 14, "urban-area": 15,
        "airfield": 16, "flood-bed": 17, "flood-bank": 18, "airway": 19}
# Simplification tolerance (m) per polygon kind.
TOL = {1: 0.12, 0: 1.0}
ROAD_KINDS = {"retained-road", "urban-arterial", "expressway", "new-road", "secondary-road", "bridge", "promenade", "railway", "airway"}
CLASSES = ["motorway", "motorway_link", "trunk", "trunk_link", "primary", "primary_link", "secondary", "secondary_link", "tertiary",
           "tertiary_link", "residential", "unclassified", "living_street", "service", "pedestrian", "footway", "path", "steps",
           "cycleway", "track", "busway", "rail", "tram", "subway", "runway", "taxiway", "promenade", "raceway"]
CLASS = {c: i for i, c in enumerate(CLASSES)}
SKIP_CLASS = {"corridor", "elevator", "platform", "rest_area"}


def world(p):
    return (round((p[0] - OX) * 100), round(-(p[1] - OY) * 100))


def simplify(pts, tol):
    """Douglas-Peucker on integer centimetre points (open polyline)."""
    if len(pts) < 3:
        return pts
    tol2 = (tol * 100) ** 2
    keep = [False] * len(pts)
    keep[0] = keep[-1] = True
    stack = [(0, len(pts) - 1)]
    while stack:
        a, b = stack.pop()
        if b <= a + 1:
            continue
        ax, ay = pts[a]
        dx, dy = pts[b][0] - ax, pts[b][1] - ay
        l2 = dx * dx + dy * dy
        best, bi = -1.0, -1
        for i in range(a + 1, b):
            px, py = pts[i][0] - ax, pts[i][1] - ay
            if l2 == 0:
                d = px * px + py * py
            else:
                t = max(0.0, min(1.0, (px * dx + py * dy) / l2))
                ex, ey = px - t * dx, py - t * dy
                d = ex * ex + ey * ey
            if d > best:
                best, bi = d, i
        if best > tol2:
            keep[bi] = True
            stack.append((a, bi))
            stack.append((bi, b))
    return [p for p, k in zip(pts, keep) if k]


def ring(coords, tol):
    pts = [world(p) for p in coords]
    out = []
    for p in pts:
        if not out or out[-1] != p:
            out.append(p)
    if len(out) > 1 and out[0] == out[-1]:
        out.pop()
    if len(out) < 3:
        return None
    # Simplify as two open halves so the closure point is kept.
    h = len(out) // 2
    a = simplify(out[: h + 1], tol)
    b = simplify(out[h:] + [out[0]], tol)
    out = a[:-1] + b[:-1]
    return out if len(out) >= 3 else None


class W:
    def __init__(self):
        self.b = bytearray()

    def u(self, v):
        v = int(v)
        assert v >= 0
        while v >= 0x80:
            self.b.append(v & 0x7F | 0x80)
            v >>= 7
        self.b.append(v)

    def i(self, v):
        v = int(v)
        self.u((v << 1) ^ (v >> 63) if v >= 0 else ((-v) << 1) - 1)

    def pts(self, pts):
        self.u(len(pts))
        px = py = 0
        for x, y in pts:
            self.i(x - px)
            self.i(y - py)
            px, py = x, y


def main():
    src, out = sys.argv[1], sys.argv[2]
    d = json.load(gzip.open(f"{src}/isla-metros.geojson.gz"))
    strings, sidx = [""], {"": 0}

    def s(v):
        v = (v or "").strip()
        if v not in sidx:
            sidx[v] = len(strings)
            strings.append(v)
        return sidx[v]

    def osm_id(p):
        sid = p.get("source_id") or p.get("id") or ""
        if len(sid) > 1 and sid[0] in "wr" and sid[1:].isdigit():
            return int(sid[1:]) * 2 + (1 if sid[0] == "r" else 0)
        return 0

    # Points shared by several ways are junctions: never simplified away,
    # so the road graph keeps its topology.
    seen = {}
    for f in d["features"]:
        g = f["geometry"]
        if f["properties"].get("kind") in ROAD_KINDS and g["type"] in ("LineString", "MultiLineString"):
            for line in [g["coordinates"]] if g["type"] == "LineString" else g["coordinates"]:
                for q in set(world(q) for q in line):
                    seen[q] = seen.get(q, 0) + 1
    shared = {q for q, n in seen.items() if n > 1}
    del seen

    # Real building heights (tools/isla/lidar_heights.py): Valencia's
    # buildings that only have the default two storeys take the LiDAR height
    # sampled for them, or that of one of the nearest sampled buildings
    # (which of the four nearest: by the building's id, so streets keep the
    # mix of heights of their neighbourhood).
    lidar, lidar_grid = {}, {}
    if os.path.exists(f"{out}/lidar_heights.json"):
        lidar = {k: v for k, v in json.load(open(f"{out}/lidar_heights.json")).items() if v and 2.5 <= v <= 150}
    centre = {}
    for f in d["features"]:
        p, g = f["properties"], f["geometry"]
        if p.get("kind") == "building" and p.get("source_id") in lidar and g["type"] in ("Polygon", "MultiPolygon"):
            r = g["coordinates"][0] if g["type"] == "Polygon" else g["coordinates"][0][0]
            c = (sum(q[0] for q in r) / len(r), sum(q[1] for q in r) / len(r))
            centre[p["source_id"]] = c
            lidar_grid.setdefault((int(c[0] // 250), int(c[1] // 250)), []).append((c, lidar[p["source_id"]]))

    def real_height(p, ring):
        """(height, flag): flag 2 sampled, 4 from the neighbours, 0 as given."""
        h = float(p.get("height") or 0)
        if p.get("zone") != "valencia" or abs(h - 6.4) > 0.01 or not lidar:
            return h, 0
        sid = p.get("source_id")
        if sid in lidar:
            return lidar[sid], 2
        c = (sum(q[0] for q in ring) / len(ring), sum(q[1] for q in ring) / len(ring))
        gx, gy = int(c[0] // 250), int(c[1] // 250)
        near = sorted(((q[0] - c[0]) ** 2 + (q[1] - c[1]) ** 2, v) for dx in (-1, 0, 1) for dy in (-1, 0, 1) for q, v in lidar_grid.get((gx + dx, gy + dy), []))[:4]
        near = [v for d2, v in near if d2 < 400 ** 2]
        if not near:
            return h, 0
        return near[zlib.crc32((sid or "").encode()) % len(near)], 4

    seen_lines = set()
    polys, lines, points = W(), W(), W()
    npoly = nline = npoint = 0
    stats = {}
    for f in d["features"]:
        p, g = f["properties"], f["geometry"]
        kind, t = p.get("kind"), g["type"]
        if kind in POLY and t in ("Polygon", "MultiPolygon"):
            k = POLY[kind]
            tol = TOL.get(k, 0.6)
            for poly in [g["coordinates"]] if t == "Polygon" else g["coordinates"]:
                rings = [r for r in (ring(c, tol) for c in poly) if r]
                if not rings or len(rings[0]) < 3:
                    continue
                flags = 0
                if kind == "agriculture":
                    flags = {"orchard": 1, "arable": 2, "fallow": 3}.get(p.get("crop"), 0)
                height = float(p.get("height") or 0)
                if kind == "building":
                    height, hsrc = real_height(p, poly[0])
                    flags |= hsrc | (1 if p.get("airport") else 0)
                    stats["lidar " + str(hsrc)] = stats.get("lidar " + str(hsrc), 0) + 1
                polys.u(k)
                polys.u(flags)
                polys.u(round(height * 10))
                polys.u(s(p.get("name")))
                polys.u(osm_id(p))
                polys.u(len(rings))
                for r in rings:
                    polys.pts(r)
                npoly += 1
                stats[kind] = stats.get(kind, 0) + sum(len(r) for r in rings)
        elif kind in ROAD_KINDS and t in ("LineString", "MultiLineString"):
            rc = p.get("roadclass") or p.get("aeroway") or ("promenade" if kind == "promenade" else "residential")
            if rc == "stopway":
                rc = "runway"
            if rc in SKIP_CLASS or rc not in CLASS:
                continue
            tunnel = p.get("tunnel") in ("yes",)
            layer = int(p.get("layer") or 0) if str(p.get("layer") or "0").lstrip("-").isdigit() else 0
            bridge = kind == "bridge" or p.get("bridge") not in (None, "no")
            flags = (1 if bridge else 0) | (2 if tunnel else 0) | (4 if p.get("oneway") == "yes" else 0) | (8 if p.get("tunnel") == "building_passage" else 0)
            for line in [g["coordinates"]] if t == "LineString" else g["coordinates"]:
                pts = []
                for q in line:
                    w = world(q)
                    if not pts or pts[-1] != w:
                        pts.append(w)
                cuts = [0] + [i for i in range(1, len(pts) - 1) if pts[i] in shared] + [len(pts) - 1]
                simple = [pts[0]]
                for a, b in zip(cuts, cuts[1:]):
                    simple += simplify(pts[a : b + 1], 0.25)[1:]
                pts = simple
                if len(pts) < 2:
                    continue
                # The source lists bridges twice (as roads and as bridges).
                dup = (CLASS[rc], tuple(pts))
                if dup in seen_lines or (CLASS[rc], tuple(reversed(pts))) in seen_lines:
                    continue
                seen_lines.add(dup)
                lines.u(CLASS[rc])
                lines.u(flags)
                lines.i(layer)
                lines.u(round(float(p.get("width") or 0) * 10))
                lines.u(s(p.get("name")))
                lines.pts(pts)
                nline += 1
                stats[kind] = stats.get(kind, 0) + len(pts)
        elif kind == "station" and t == "Point":
            x, y = world(g["coordinates"])
            points.u(2); points.u(s(p.get("name"))); points.u(0); points.u(0); points.i(x); points.i(y)
            npoint += 1
    for l in json.load(open(f"{src}/landmarks.json")):
        x, y = world(l["xy"])
        sid = l.get("id") or ""
        oid = int(sid[1:]) * 2 + (1 if sid[0] == "r" else 0) if len(sid) > 1 and sid[0] in "wr" and sid[1:].isdigit() else 0
        points.u(1); points.u(s(l.get("name"))); points.u(0); points.u(oid); points.i(x); points.i(y)
        npoint += 1
    for l in json.load(open(f"{src}/pois.json")):
        x, y = world(l["xy"])
        points.u(0); points.u(s(l.get("name"))); points.u(s(l.get("category"))); points.u(0); points.i(x); points.i(y)
        npoint += 1
    terrain = json.load(open(f"{src}/terrain.json"))
    e = terrain["extent"]
    w = W()
    w.b += b"ISLA"
    w.u(1)
    # Height map extent in world centimetres: west, north (min z), east, south.
    for v in (e[0] - OX, -(e[3] - OY), e[2] - OX, -(e[1] - OY)):
        w.i(round(v * 100))
    w.u(terrain["size"])
    w.u(len(strings))
    for t in strings:
        b = t.encode("utf-8")
        w.u(len(b))
        w.b += b
    for n, sec in ((npoly, polys), (nline, lines), (npoint, points)):
        w.u(n)
        w.b += sec.b
    with gzip.GzipFile(f"{out}/isla.bin.gz", "wb", compresslevel=9, mtime=0) as fh:
        fh.write(bytes(w.b))
    shutil.copyfile(f"{src}/altura-decimetros.png", f"{out}/height.png")
    print(f"{npoly} polygons, {nline} lines, {npoint} points, {len(strings)} strings, {len(w.b) / 1e6:.1f} MB raw")
    print({k: v for k, v in sorted(stats.items())})


if __name__ == "__main__":
    main()
