#!/usr/bin/env python3
"""Cuts a window of the Isla Levante map (metres) into an OSM `map.json`
the Spanish city builder reads. Usage: isla_to_osm.py isla-metros.geojson.gz out.json cx cy half"""
import gzip, json, math, sys
src, out, cx, cy, half = sys.argv[1], sys.argv[2], float(sys.argv[3]), float(sys.argv[4]), float(sys.argv[5])
LAT0, LON0 = 39.4745, -0.3765
KY = 111320.0
KX = KY * math.cos(math.radians(LAT0))
d = json.load(gzip.open(src))
nodes, elements, ids = {}, [], [0]
def node(x, y):
    k = (round(x, 1), round(y, 1))
    if k not in nodes:
        ids[0] += 1
        nodes[k] = ids[0]
        elements.append({"type": "node", "id": ids[0], "lat": LAT0 + (y - cy) / KY, "lon": LON0 + (x - cx) / KX})
    return nodes[k]
def inside(p, m=0.0):
    return abs(p[0] - cx) <= half + m and abs(p[1] - cy) <= half + m
def way(pts, tags):
    refs = []
    for p in pts:
        n = node(p[0], p[1])
        if not refs or refs[-1] != n:
            refs.append(n)
    if len(refs) >= 2:
        ids[0] += 1
        elements.append({"type": "way", "id": ids[0], "nodes": refs, "tags": tags})
HW = {"motorway": "primary", "trunk": "primary", "primary": "primary", "secondary": "secondary", "tertiary": "tertiary"}
for f in d["features"]:
    p, g = f["properties"], f["geometry"]
    kind = p.get("kind")
    if kind in ("building", "protected-landmark", "landmark-footprint") and g["type"] in ("Polygon", "MultiPolygon"):
        polys = [g["coordinates"]] if g["type"] == "Polygon" else g["coordinates"]
        for poly in polys:
            ring = poly[0]
            if len(ring) >= 4 and all(inside(q) for q in ring):
                tags = {"building": "yes"}
                if p.get("height"):
                    tags["height"] = str(round(float(p["height"]), 1))
                    tags["building:levels"] = str(max(1, round(float(p["height"]) / 3.2)))
                if p.get("name"):
                    tags["name"] = p["name"]
                way(ring, tags)
    elif kind in ("retained-road", "urban-arterial", "new-road", "bridge", "promenade") and g["type"] in ("LineString", "MultiLineString"):
        lines = [g["coordinates"]] if g["type"] == "LineString" else g["coordinates"]
        rc = p.get("roadclass") or "residential"
        if rc in ("footway", "path", "cycleway", "steps", "track", "service", "corridor", "platform"):
            continue
        tags = {"highway": HW.get(rc, rc if rc in ("residential", "pedestrian", "living_street", "unclassified") else "residential")}
        if p.get("name"):
            tags["name"] = p["name"]
        if p.get("oneway") == "yes":
            tags["oneway"] = "yes"
        for line in lines:
            run = []
            for q in line:
                if inside(q):
                    run.append(q)
                else:
                    way(run, tags); run = []
            way(run, tags)
b = {"minlat": LAT0 - half / KY, "maxlat": LAT0 + half / KY, "minlon": LON0 - half / KX, "maxlon": LON0 + half / KX}
json.dump({"version": "0.6", "generator": "isla_to_osm", "copyright": "OpenStreetMap contributors, ODbL", "bounds": b, "elements": elements}, open(out, "w"), separators=(",", ":"))
print(len(elements), "elements")
