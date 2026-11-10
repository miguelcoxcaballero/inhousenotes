#!/usr/bin/env python3
"""Places the hand-modelled Valencia landmarks on the Isla Levante.

    python3 -I tools/isla/isla_landmarks.py <isla-v5/data> [engine dir]

For every `data/landmarks/valencia/<slug>/landmark.toml` the real footprint
(`footprint.json`: OSM outlines in the landmark frame, UTM 30N metres minus
the Plaça de l'Ajuntament origin, x east, z south) is matched to the island's
footprint of the same OSM id (`isla-metros.geojson.gz`, island metres, x east,
y north) with a rigid fit (rotation + translation, no scale). The result is
`data/maps/isla/landmarks.toml`: where each model stands in the island world
(`world.x = island.x - ORIGIN.x`, `world.z = ORIGIN.y - island.y`), how it is
turned, the fit's residual and the island buildings it replaces.

Input is data: it is only parsed.
"""
import glob
import gzip
import json
import math
import os
import re
import sys

import numpy as np

# The world origin in island metres (forja_world::island::ORIGIN).
ORIGIN = (3478.569, 2582.763)


def kabsch(a, b):
    """Rigid transform (R, t) taking points `a` onto `b`, and the errors."""
    ca, cb = a.mean(0), b.mean(0)
    u, _, vt = np.linalg.svd((a - ca).T @ (b - cb))
    d = np.sign(np.linalg.det(vt.T @ u.T))
    r = vt.T @ np.diag([1.0, d]) @ u.T
    t = cb - r @ ca
    return r, t, np.sqrt((((r @ a.T).T + t - b) ** 2).sum(1))


def ring_area(r):
    x, y = r[:, 0], r[:, 1]
    return 0.5 * float(np.dot(x, np.roll(y, -1)) - np.dot(np.roll(x, -1), y))


def inside(ring, p):
    x, y = p
    n, c = len(ring), False
    j = n - 1
    for i in range(n):
        (xi, yi), (xj, yj) = ring[i], ring[j]
        if (yi > y) != (yj > y) and x < (xj - xi) * (y - yi) / (yj - yi) + xi:
            c = not c
        j = i
    return c


def open_ring(r):
    r = np.array(r, dtype=float)
    return r[:-1] if len(r) > 1 and (r[0] == r[-1]).all() else r


def main():
    src = sys.argv[1]
    engine = sys.argv[2] if len(sys.argv) > 2 else os.path.join(os.path.dirname(os.path.abspath(__file__)), "../..")
    geo = json.load(gzip.open(os.path.join(src, "isla-metros.geojson.gz")))
    by_id, buildings = {}, []
    for f in geo["features"]:
        p, g = f["properties"], f["geometry"]
        if p.get("kind") not in ("building", "protected-landmark", "landmark-footprint"):
            continue
        if g["type"] not in ("Polygon", "MultiPolygon"):
            continue
        polys = g["coordinates"] if g["type"] == "MultiPolygon" else [g["coordinates"]]
        outers = [open_ring(poly[0]) for poly in polys if poly and len(poly[0]) >= 4]
        sid = p.get("source_id") or p.get("id") or ""
        by_id.setdefault(sid, []).extend(outers)
        if p.get("kind") == "building":
            for o in outers:
                buildings.append((sid, o, o.min(0), o.max(0)))
    pins = {}
    for lm in json.load(open(os.path.join(src, "landmarks.json"))):
        if lm.get("xy"):
            for sid in lm.get("source_ids") or [lm.get("id")]:
                pins[sid] = (lm["name"], lm["xy"])

    out = [
        "# Where the hand-modelled Valencia landmarks stand on the Isla Levante.",
        "# Written by tools/isla/isla_landmarks.py (do not edit): each landmark's real",
        "# footprint (data/landmarks/valencia/<slug>/footprint.json) fitted to the",
        "# island footprint of the same OSM id.",
        "#   pos       world x (east) and z (south) of the landmark's origin (m)",
        "#   rotation  degrees counter-clockwise seen from above",
        "#   residual  largest footprint vertex error of the fit (m), over `matched` vertices",
        "#   replaces  island buildings that give way to the model",
        "#   island    the origin in island metres (x east, y north); centre: of the footprint",
        "#   pin       the landmark's map pin in isla-v5/data/landmarks.json and its label, when it has one",
        f"origin = [{ORIGIN[0]}, {ORIGIN[1]}]",
        "",
    ]
    report, pin_off = [], []
    for d in sorted(glob.glob(os.path.join(engine, "data/landmarks/valencia/*/"))):
        slug = os.path.basename(d[:-1])
        if not os.path.exists(d + "landmark.toml"):
            continue
        text = open(d + "landmark.toml", encoding="utf-8").read()
        m = re.search(r'^\s*name\s*=\s*"((?:[^"\\]|\\.)*)"', text, re.M)
        name = m.group(1) if m else slug
        fp = json.load(open(d + "footprint.json"))
        ax, az = fp["anchor"]
        # Landmark outlines in island axes (x east, y north) about the city origin.
        a_pts, b_pts, rings, ids = [], [], [], []
        for o in fp.get("outlines", []):
            ring = open_ring(o["outer"])
            a = np.column_stack([ax + ring[:, 0], -(az + ring[:, 1])])
            rings.append(a)
            mm = re.match(r"osm:(way|relation)/(\d+)$", str(o.get("id", "")))
            if not mm:
                continue
            sid = mm.group(1)[0] + mm.group(2)
            ids.append(sid)
            cands = by_id.get(sid)
            if not cands:
                continue
            cloud = np.vstack(cands)
            # Nearest island vertex of the same OSM object, from the nominal frame.
            guess = a + np.array(ORIGIN)
            dist = np.sqrt(((guess[:, None, :] - cloud[None, :, :]) ** 2).sum(2))
            k = dist.argmin(1)
            ok = dist[np.arange(len(a)), k] < 3.0
            a_pts.append(a[ok])
            b_pts.append(cloud[k][ok])
        if not a_pts or sum(len(a) for a in a_pts) < 3:
            report.append((slug, "NOT ON THE ISLAND", 0, 0, 0))
            continue
        a_all, b_all = np.vstack(a_pts), np.vstack(b_pts)
        r, t, err = kabsch(a_all, b_all)
        rot = math.degrees(math.atan2(r[1, 0], r[0, 0]))
        anchor = r @ np.array([ax, -az]) + t
        pos = (anchor[0] - ORIGIN[0], ORIGIN[1] - anchor[1])
        # Island buildings under the model: the same OSM objects, and those
        # mostly inside its outlines (parts, annexes mapped on their own).
        placed = [(r @ ring.T).T + t for ring in rings]
        lo = np.min([p.min(0) for p in placed], 0)
        hi = np.max([p.max(0) for p in placed], 0)
        replaces = []
        for sid, o, bl, bh in buildings:
            if (bh < lo).any() or (bl > hi).any():
                continue
            hit = sid in ids
            if not hit:
                # Sample the building: vertices, edge midpoints and centroid.
                pts = list(o) + list((o + np.roll(o, -1, 0)) * 0.5) + [o.mean(0)]
                pts = [o.mean(0) + (p - o.mean(0)) * 0.9 for p in pts]
                n_in = sum(any(inside(p2, q) for p2 in placed) for q in pts)
                hit = n_in >= 0.6 * len(pts)
            if hit and sid and sid not in replaces:
                replaces.append(sid)
        pin = next((pins[s] for s in ids if s in pins), None)
        out.append("[[landmark]]")
        out.append(f'slug = "{slug}"')
        out.append('name = "' + name.replace("\\", "\\\\").replace('"', '\\"') + '"')
        out.append(f"pos = [{pos[0]:.3f}, {pos[1]:.3f}]")
        out.append(f"rotation = {rot + 0.0:.4f}")
        out.append(f"residual = {err.max():.4f}")
        out.append(f"matched = {len(a_all)}")
        out.append(f"island = [{anchor[0]:.3f}, {anchor[1]:.3f}]")
        centre = (lo + hi) * 0.5
        out.append(f"centre = [{centre[0]:.3f}, {centre[1]:.3f}]")
        if pin:
            out.append('label = "' + pin[0].replace("\\", "\\\\").replace('"', '\\"') + '"')
            out.append(f"pin = [{pin[1][0]:.3f}, {pin[1][1]:.3f}]")
            pin_off.append((math.hypot(pin[1][0] - centre[0], pin[1][1] - centre[1]), slug))
        out.append("replaces = [" + ", ".join('"%s/%s"' % ("way" if s[0] == "w" else "relation", s[1:]) for s in replaces) + "]")
        out.append("")
        off = math.hypot(pos[0] - ax, pos[1] - az)
        report.append((slug, "ok", len(a_all), err.max(), rot, off, len(replaces), sum(len(x) for x in rings)))
    path = os.path.join(engine, "data/maps/isla/landmarks.toml")
    open(path, "w", encoding="utf-8", newline="\n").write("\n".join(out))
    ok = [x for x in report if x[1] == "ok"]
    for x in report:
        if x[1] == "ok":
            print(f"{x[0]:36s} {x[2]:4d}/{x[7]:<4d} vertices  residual {x[3]:.3f} m  rotation {x[4]:+.4f} deg  off-frame {x[5]:.3f} m  replaces {x[6]}")
        else:
            print(f"{x[0]:36s} {x[1]}")
    pin_off.sort(reverse=True)
    print("map pins furthest from their footprint centre:", ", ".join(f"{s} {d:.0f} m" for d, s in pin_off[:6]), f"({len(pin_off)} pins)")
    print(f"{len(ok)} landmarks placed, worst residual {max(x[3] for x in ok):.3f} m, worst rotation {max(abs(x[4]) for x in ok):.4f} deg -> {path}")


if __name__ == "__main__":
    main()
