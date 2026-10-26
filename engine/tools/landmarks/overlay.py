#!/usr/bin/env python3
"""Draw a landmark footprint.json to a PNG for inspection.

    python3 -I engine/tools/landmarks/overlay.py <footprint.json> [--sources sources.toml] [--out overlay.png]
    python3 -I engine/tools/landmarks/overlay.py --slug <slug> [--data-dir D] [--raw-dir R]

Colours: OSM outlines red, OSM building:part green, Catastro parts blue with
floor counts (above[/-below]), context grey (darker = taller), photo camera
positions as numbered yellow dots (with a heading tick when known; cameras off
the map are drawn on the border with their distance). North is up, x east,
z south (city frame). The anchor (local 0,0) is a black cross.
"""

from __future__ import annotations

import argparse
import json
import math
import os
import re
import sys
import tomllib

TOOL_DIR = os.path.dirname(os.path.abspath(__file__))
if TOOL_DIR not in sys.path:
    sys.path.insert(0, TOOL_DIR)

from PIL import Image, ImageDraw, ImageFont  # noqa: E402

from lmcommon import CONVERGENCE_DEG  # noqa: E402

ENGINE_DIR = os.path.normpath(os.path.join(TOOL_DIR, "..", ".."))
DEFAULT_DATA_DIR = os.path.join(ENGINE_DIR, "data", "landmarks", "valencia")
DEFAULT_RAW_DIR = os.path.join(ENGINE_DIR, "assets", "downloaded", "landmarks", "valencia")


FONT_FILES = ["/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
              "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
              "/usr/share/fonts/truetype/freefont/FreeSans.ttf"]


def _font(size):
    for f in FONT_FILES:
        if os.path.exists(f):
            try:
                return ImageFont.truetype(f, size)
            except OSError:
                pass
    try:
        return ImageFont.load_default(size=size)
    except TypeError:
        return ImageFont.load_default()


def _bbox(rings):
    xs = [p[0] for r in rings for p in r]
    zs = [p[1] for r in rings for p in r]
    return min(xs), min(zs), max(xs), max(zs)


def _centroid(ring):
    a = cx = cz = 0.0
    n = len(ring)
    for i in range(n):
        x0, z0 = ring[i]
        x1, z1 = ring[(i + 1) % n]
        c = x0 * z1 - x1 * z0
        a += c
        cx += (x0 + x1) * c
        cz += (z0 + z1) * c
    if abs(a) < 1e-9:
        return sum(p[0] for p in ring) / n, sum(p[1] for p in ring) / n
    return cx / (3 * a), cz / (3 * a)


def _area(ring):
    n = len(ring)
    return abs(sum(ring[i][0] * ring[(i + 1) % n][1] - ring[(i + 1) % n][0] * ring[i][1] for i in range(n))) / 2


def render(fp_path, sources_path=None, out_path=None, size=1600, radius=None, cameras=True):
    with open(fp_path, encoding="utf-8") as f:
        fp = json.load(f)
    photos = []
    if sources_path and os.path.exists(sources_path):
        with open(sources_path, "rb") as f:
            src = tomllib.load(f)
        for i, p in enumerate(src.get("photo", [])):
            if "camera" in p:
                m = re.match(r"^(?:photos/)?(\d+)_", p.get("file", ""))
                photos.append({"n": m.group(1).lstrip("0") if m else str(i + 1), "cam": p["camera"],
                               "heading": p.get("heading"), "ext": p.get("exterior")})
    main = [o["outer"] for o in fp["outlines"]] + [o["outer"] for o in fp["parts"]] + \
           [o["outer"] for o in fp["catastro"]]
    if not main:
        raise SystemExit("footprint has no geometry")
    x0, z0, x1, z1 = _bbox(main)
    span = max(x1 - x0, z1 - z0)
    margin = max(25.0, 0.35 * span) if radius is None else 0
    if radius is not None:
        x0, z0, x1, z1 = -radius, -radius, radius, radius
    else:
        x0, z0, x1, z1 = x0 - margin, z0 - margin, x1 + margin, z1 + margin
        # include nearby cameras (up to 150 m) so their positions are visible
        if cameras:
            for p in photos:
                cx, cz = p["cam"]
                if math.hypot(cx, cz) <= 150:
                    x0, z0 = min(x0, cx - 12), min(z0, cz - 12)
                    x1, z1 = max(x1, cx + 12), max(z1, cz + 12)
    w_m, h_m = x1 - x0, z1 - z0
    scale = (size - 40) / max(w_m, h_m)
    W, H = int(w_m * scale) + 40, int(h_m * scale) + 140
    img = Image.new("RGBA", (W, H), (250, 250, 247, 255))
    lay = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    d = ImageDraw.Draw(lay)

    def P(x, z):
        return (20 + (x - x0) * scale, 120 + (z - z0) * scale)

    def poly(ring, fill=None, outline=None, width=1):
        pts = [P(x, z) for x, z in ring]
        if len(pts) >= 3:
            d.polygon(pts, fill=fill)
            if outline:
                d.line(pts + [pts[0]], fill=outline, width=width, joint="curve")

    fsmall, fmed, fbig = _font(max(11, int(size / 120))), _font(max(13, int(size / 95))), _font(max(18, int(size / 70)))
    # context
    for c in fp.get("context", []):
        fl = c.get("floors_above") or 0
        g = max(120, 225 - 12 * fl)
        poly(c["outer"], fill=(g, g, g, 150), outline=(130, 130, 130, 200), width=1)
    # catastro
    for c in fp.get("catastro", []):
        poly(c["outer"], fill=(60, 110, 230, 55), outline=(30, 70, 220, 255), width=2)
        for r in c.get("inners", []):
            poly(r, fill=(250, 250, 247, 200), outline=(30, 70, 220, 255), width=1)
    # osm parts
    for c in fp.get("parts", []):
        poly(c["outer"], fill=None, outline=(20, 160, 40, 255), width=3)
        for r in c.get("inners", []):
            poly(r, outline=(20, 160, 40, 255), width=2)
    # outlines
    for c in fp.get("outlines", []):
        poly(c["outer"], fill=None, outline=(220, 20, 20, 255), width=4)
        for r in c.get("inners", []):
            poly(r, outline=(220, 20, 20, 255), width=3)
    img = Image.alpha_composite(img, lay)
    d = ImageDraw.Draw(img)
    # catastro floor labels (skip tiny parts when crowded)
    for c in fp.get("catastro", []):
        if _area(c["outer"]) < 4:
            continue
        cx, cz = _centroid(c["outer"])
        lab = str(c.get("floors_above") if c.get("floors_above") is not None else "?")
        if c.get("floors_below"):
            lab += f"/-{c['floors_below']}"
        px, py = P(cx, cz)
        d.text((px, py), lab, fill=(10, 30, 160), font=fmed, anchor="mm",
               stroke_width=2, stroke_fill=(255, 255, 255))
    # osm part labels: height/levels
    for c in fp.get("parts", []):
        t = c.get("tags", {})
        bits = []
        if t.get("height"):
            bits.append(f"h{t['height']}")
        if t.get("min_height"):
            bits.append(f"mh{t['min_height']}")
        if t.get("building:levels"):
            bits.append(f"L{t['building:levels']}")
        if t.get("roof:shape"):
            bits.append(t["roof:shape"][:6])
        if bits:
            cx, cz = _centroid(c["outer"])
            px, py = P(cx, cz)
            d.text((px, py + 16), " ".join(bits), fill=(10, 110, 20), font=fsmall, anchor="mm",
                   stroke_width=2, stroke_fill=(255, 255, 255))
    # anchor
    ax, ay = P(0, 0)
    d.line([(ax - 10, ay), (ax + 10, ay)], fill=(0, 0, 0), width=2)
    d.line([(ax, ay - 10), (ax, ay + 10)], fill=(0, 0, 0), width=2)
    # cameras
    off = []
    if cameras:
        for p in photos:
            cx, cz = p["cam"]
            inside = x0 <= cx <= x1 and z0 <= cz <= z1
            if not inside:
                # clamp onto the border along the ray from the anchor
                t = 1.0
                for lim, v in ((x1, cx), (x0, cx)):
                    if v != 0 and (v > lim if lim == x1 else v < lim):
                        t = min(t, lim / v)
                for lim, v in ((z1, cz), (z0, cz)):
                    if v != 0 and (v > lim if lim == z1 else v < lim):
                        t = min(t, lim / v)
                t *= 0.97
                off.append(f"{p['n']}@{math.hypot(cx, cz):.0f}m")
                cx, cz = cx * t, cz * t
            px, py = P(cx, cz)
            col = (250, 200, 0) if p["ext"] is True else ((250, 120, 0) if p["ext"] == "unknown" else (200, 0, 200))
            if p.get("heading") is not None:
                # compass heading (true north) -> grid azimuth in the city frame
                h = math.radians(float(p["heading"]) - CONVERGENCE_DEG)
                d.line([(px, py), (px + 28 * math.sin(h), py - 28 * math.cos(h))], fill=(0, 0, 0), width=3)
            r = 10
            d.ellipse([px - r, py - r, px + r, py + r], fill=col, outline=(0, 0, 0),
                      width=2 if inside else 1)
            d.text((px, py), p["n"], fill=(0, 0, 0), font=fsmall, anchor="mm")
    # header, scale bar, north arrow, legend
    d.rectangle([0, 0, W, 112], fill=(255, 255, 255))
    title = f"{fp['slug']}  anchor x={fp['anchor'][0]} z={fp['anchor'][1]}  ({fp['frame']})"
    d.text((20, 10), title, fill=(0, 0, 0), font=fbig)
    stats = (f"outlines {len(fp['outlines'])} (red)   osm parts {len(fp['parts'])} (green)   "
             f"catastro {len(fp['catastro'])} (blue, floors)   context {len(fp['context'])} (grey)   "
             f"cameras {len(photos)} (yellow ext / orange ? / magenta int)")
    d.text((20, 44), stats, fill=(40, 40, 40), font=fmed)
    if off:
        d.text((20, 70), "off-map cameras: " + ", ".join(off)[:200], fill=(120, 60, 0), font=fsmall)
    ids = ", ".join(sorted({o["id"] for o in fp["outlines"]}))[:150]
    d.text((20, 90), ids, fill=(150, 0, 0), font=fsmall)
    nice = [5, 10, 20, 25, 50, 100, 200, 250, 500, 1000]
    L = next((n for n in nice if n * scale >= 120), nice[-1])
    bx, by = 30, H - 30
    d.rectangle([bx, by - 6, bx + L * scale, by], fill=(0, 0, 0))
    d.text((bx + L * scale + 8, by - 3), f"{L} m", fill=(0, 0, 0), font=fmed, anchor="lm")
    nx, ny = W - 40, 150
    d.polygon([(nx, ny - 22), (nx - 9, ny + 6), (nx + 9, ny + 6)], fill=(0, 0, 0))
    d.text((nx, ny + 18), "N", fill=(0, 0, 0), font=fmed, anchor="mm")
    out_path = out_path or os.path.splitext(fp_path)[0] + "_overlay.png"
    os.makedirs(os.path.dirname(os.path.abspath(out_path)), exist_ok=True)
    img.convert("RGB").save(out_path)
    return out_path


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("footprint", nargs="?", help="footprint.json")
    ap.add_argument("--sources", help="sources.toml (camera positions); default: next to footprint.json")
    ap.add_argument("--out", help="output PNG")
    ap.add_argument("--slug", help="use <data-dir>/<slug>/footprint.json and write <raw-dir>/<slug>/overlay.png")
    ap.add_argument("--data-dir", default=DEFAULT_DATA_DIR)
    ap.add_argument("--raw-dir", default=DEFAULT_RAW_DIR)
    ap.add_argument("--size", type=int, default=1600)
    ap.add_argument("--radius", type=float, help="fixed half extent in metres around the anchor (e.g. 120 for context)")
    ap.add_argument("--no-cameras", action="store_true")
    a = ap.parse_args(argv)
    if a.slug:
        fpp = os.path.join(a.data_dir, a.slug, "footprint.json")
        out = a.out or os.path.join(a.raw_dir, a.slug, "overlay.png")
    elif a.footprint:
        fpp, out = a.footprint, a.out
    else:
        ap.error("give a footprint.json or --slug")
    src = a.sources or os.path.join(os.path.dirname(os.path.abspath(fpp)), "sources.toml")
    p = render(fpp, src if os.path.exists(src) else None, out, a.size, a.radius, not a.no_cameras)
    print(p)


if __name__ == "__main__":
    main()
