#!/usr/bin/env python3
"""Fetch and assemble source data for the Valencia landmark models.

Run with ``python3 -I`` (isolated mode). Pure stdlib + numpy + PIL.

    python3 -I engine/tools/landmarks/fetch.py --list engine/data/landmarks/valencia/list.toml \
        [--only slug,slug] [--force [steps]] [--data-dir DIR] [--raw-dir DIR]

For every ``[[landmark]]`` of the list (rank, slug, name, name_va, wikidata,
commons_category, lat, lon, osm = [] optional) it runs these steps, each of
which is skipped when its outputs exist and its inputs did not change:

  wikidata  entity JSON, facts, Commons category, Wikipedia links, P625
  osm       outline (list ids, or wikidata tag / name / containing building
            in a ~250 m map.json), multipolygon assembly, building:part
  catastro  INSPIRE BU:BuildingPart for the outline (+20 m) and a 240 m
            context box, BU:Building for the parcel facts, WMS cadastral map,
            cadastral facade photo
  footprint footprint.json in the city frame (utm30n-local-v1)
  commons   category tree listing (depth 1) + text classification
  photos    ranked exterior photos (<=16, width 1280) with camera positions
  plans     plans/drawings (<=6, width 1920 = closest standard thumb to 2000)
  contact   photos/contact.jpg labelled grid
  sources   sources.toml (merged with an existing one: manual edits kept)
  overlay   overlay.png (via overlay.py)

Downloaded files are untrusted data; they are only parsed (JSON, regex over
GML, PIL decode), never executed. Raw downloads go to --raw-dir/<slug>/, the
committed outputs (footprint.json, sources.toml) to --data-dir/<slug>/.
A summary table is written to --raw-dir/_summary.tsv.
"""

from __future__ import annotations

import argparse
import hashlib
import html
import errno
import io
import json
import math
import os
import re
import sys
import time
import tomllib
import traceback
import unicodedata
import urllib.parse

TOOL_DIR = os.path.dirname(os.path.abspath(__file__))
if TOOL_DIR not in sys.path:
    sys.path.insert(0, TOOL_DIR)

from PIL import Image, ImageDraw, ImageFile, ImageFont  # noqa: E402

import lmcommon as L  # noqa: E402

Image.MAX_IMAGE_PIXELS = 120_000_000
ImageFile.LOAD_TRUNCATED_IMAGES = True  # Catastro facade JPEGs often miss their last bytes

ENGINE_DIR = os.path.normpath(os.path.join(TOOL_DIR, "..", ".."))
DEFAULT_DATA_DIR = os.path.join(ENGINE_DIR, "data", "landmarks", "valencia")
DEFAULT_RAW_DIR = os.path.join(ENGINE_DIR, "assets", "downloaded", "landmarks", "valencia")

OSM_API = "https://api.openstreetmap.org/api/0.6"
NOMINATIM = "https://nominatim.openstreetmap.org"
WD_ENTITY = "https://www.wikidata.org/wiki/Special:EntityData/{}.json"
WD_API = "https://www.wikidata.org/w/api.php"
COMMONS_API = "https://commons.wikimedia.org/w/api.php"
COMMONS_FILEPATH = "https://commons.wikimedia.org/wiki/Special:FilePath/"
CATASTRO_BU = "http://ovc.catastro.meh.es/INSPIRE/wfsBU.aspx"
CATASTRO_WMS = "http://ovc.catastro.meh.es/Cartografia/WMS/ServidorWMS.aspx"
CATASTRO_FACHADA = ("http://ovc.catastro.meh.es/OVCServWeb/OVCWcfLibres/OVCFotoFachada.svc/"
                    "RecuperarFotoFachadaGet?ReferenciaCatastral={}")
CATASTRO_SEDE = "https://www1.sedecatastro.gob.es/CYCBienInmueble/OVCListaBienes.aspx?rc1={}&rc2={}"

ATTRIBUTION = ["© OpenStreetMap contributors (ODbL 1.0)", "Dirección General del Catastro"]
STEPS = ["wikidata", "osm", "catastro", "footprint", "commons", "photos", "plans",
         "contact", "sources", "overlay"]
# Wikimedia only serves a fixed set of thumbnail widths without HTTP 429.
STANDARD_WIDTHS = [250, 330, 500, 960, 1280, 1920, 3840]

MAP_HALF = 125.0          # m, half side of the map.json box around the point
CATASTRO_MARGIN = 20.0    # m around the outline bbox
CONTEXT_HALF = 120.0      # m, half side of the context box (240 m)
CONTEXT_RADIUS = 120.0    # m from the anchor
PART_MIN_OVERLAP = 0.5

VERSION = "1"
# Bump when the Commons classification / ranking rules change: re-ranks photos
# and plans from the cached listing (downloads of unchanged titles are reused).
RULES_VERSION = "3"


# ---------------------------------------------------------------------------
# logging
# ---------------------------------------------------------------------------

class Log:
    def __init__(self, path=None):
        self.path = path
        self.fh = None
        if path:
            os.makedirs(os.path.dirname(path), exist_ok=True)
            self.fh = open(path, "a", encoding="utf-8")

    def __call__(self, *parts):
        msg = " ".join(str(p) for p in parts)
        line = time.strftime("%H:%M:%S ") + msg
        print(line, file=sys.stderr, flush=True)
        if self.fh:
            self.fh.write(time.strftime("%Y-%m-%d ") + line + "\n")
            self.fh.flush()


LOG = Log()


def digest(obj) -> str:
    return hashlib.sha1(json.dumps(obj, sort_keys=True, ensure_ascii=False, default=str)
                        .encode("utf-8")).hexdigest()[:16]


def prune(dirpath, pattern, keep):
    """Delete cached files matching pattern that this run did not use."""
    if not os.path.isdir(dirpath):
        return
    for fn in os.listdir(dirpath):
        if re.match(pattern, fn) and fn not in keep:
            os.remove(os.path.join(dirpath, fn))


def file_digest(path) -> str:
    h = hashlib.sha1()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()[:16]


def norm_text(s: str) -> str:
    s = unicodedata.normalize("NFKD", s or "")
    s = "".join(c for c in s if not unicodedata.combining(c))
    s = s.lower().replace("·", "").replace("l.l", "ll")
    s = re.sub(r"[^a-z0-9]+", " ", s)
    return " ".join(s.split())


STOP = {"de", "la", "el", "del", "dels", "les", "los", "las", "y", "i", "the", "of", "d", "l",
        "en", "a", "valencia", "valence", "s"}
GENERIC = {"torre", "torres", "iglesia", "esglesia", "palau", "palacio", "casa", "edificio",
           "edifici", "mercado", "mercat", "puerta", "porta", "plaza", "placa", "museo", "museu",
           "parroquia", "convento", "convent", "real", "reial", "nuestra", "senora", "mare", "deu",
           "san", "sant", "santa", "santo", "building", "church", "tower", "gate", "central"}


def name_tokens(s: str) -> set:
    return {t for t in norm_text(s).split() if t not in STOP}


def names_match(a: str, b: str) -> bool:
    ta, tb = name_tokens(a), name_tokens(b)
    if not ta or not tb:
        return False
    if ta == tb:
        return True
    small, big = (ta, tb) if len(ta) <= len(tb) else (tb, ta)
    if not small <= big:
        return False
    distinctive = [t for t in small if t not in GENERIC and len(t) >= 4]
    return len(small) >= 2 or bool(distinctive)


def strip_html(s: str) -> str:
    s = re.sub(r"(?is)<(script|style).*?</\1>", " ", s or "")
    s = re.sub(r"(?s)<[^>]+>", " ", s)
    s = html.unescape(s)
    return " ".join(s.split())


# ---------------------------------------------------------------------------
# state
# ---------------------------------------------------------------------------

class State:
    def __init__(self, path):
        self.path = path
        self.data = {"steps": {}, "warnings": {}}
        if os.path.exists(path):
            try:
                self.data = L.read_json(path)
            except Exception:
                pass
        self.data.setdefault("steps", {})
        self.data.setdefault("warnings", {})

    def save(self):
        L.write_json(self.path, self.data)

    def done(self, step, key, outputs=()) -> bool:
        s = self.data["steps"].get(step)
        if not s or s.get("key") != key:
            return False
        return all(os.path.exists(p) for p in outputs)

    def mark(self, step, key, out_digest="", **extra):
        self.data["steps"][step] = {"key": key, "digest": out_digest,
                                    "at": time.strftime("%Y-%m-%dT%H:%M:%S"), **extra}
        self.save()

    def out(self, step) -> str:
        return self.data["steps"].get(step, {}).get("digest", "")

    def warn(self, step, msgs):
        self.data["warnings"][step] = list(msgs)
        self.save()

    def all_warnings(self):
        out = []
        for step, ws in self.data["warnings"].items():
            for w in ws:
                out.append(f"{step}: {w}")
        return out


# ---------------------------------------------------------------------------
# Wikidata
# ---------------------------------------------------------------------------

FACT_PROPS = [
    ("P2048", "height", "quantity"),
    ("P1101", "floors", "quantity"),
    ("P571", "built", "time"),
    ("P84", "architect", "item"),
    ("P149", "style", "item"),
    ("P1435", "heritage", "item"),
    ("P2046", "area", "quantity"),
    ("P2043", "length", "quantity"),
    ("P2049", "width", "quantity"),
    ("P2044", "elevation", "quantity"),
    ("P186", "material", "item"),
    ("P808", "bic_code", "string"),
    ("P6375", "address", "monolingual"),
]
UNIT_NAMES = {"Q11573": "m", "Q25343": "m2", "Q174728": "cm", "Q3710": "ft", "Q828224": "km",
              "Q35852": "ha", "Q232291": "mi2", "Q712226": "km2", "Q81292": "acre"}
IMAGE_PROPS = {"P18": "image", "P3311": "plan", "P8592": "aerial", "P3451": "night",
               "P4291": "panorama", "P5775": "interior", "P8517": "view", "P2716": "collage",
               "P5252": "winter"}


def wd_snak_value(snak):
    if snak.get("snaktype") != "value":
        return None
    return snak.get("datavalue", {}).get("value")


def wd_time(v):
    t = v.get("time", "")
    prec = int(v.get("precision", 9))
    m = re.match(r"([+-])(\d+)-(\d\d)-(\d\d)", t)
    if not m:
        return t
    sign, y, mo, d = m.groups()
    year = int(y) * (-1 if sign == "-" else 1)
    if prec <= 6:
        return f"{(year - 1) // 1000 + 1}. millennium"
    if prec == 7:
        c = (year - 1) // 100 + 1
        return f"{c}th century"
    if prec == 8:
        return f"{year // 10 * 10}s"
    if prec == 9:
        return year
    if prec == 10:
        return f"{year:04d}-{mo}"
    return f"{year:04d}-{mo}-{d}"


def wd_labels(http, ids):
    """QIDs/PIDs -> label (en, es, ca fallback)."""
    out = {}
    ids = sorted(set(ids))
    for i in range(0, len(ids), 50):
        chunk = ids[i:i + 50]
        d = http.get_json(WD_API, {"action": "wbgetentities", "ids": "|".join(chunk),
                                   "props": "labels", "languages": "en|es|ca", "format": "json"})
        for k, e in d.get("entities", {}).items():
            labs = e.get("labels", {})
            for lang in ("en", "es", "ca"):
                if lang in labs:
                    out[k] = labs[lang]["value"]
                    break
    return out


def step_wikidata(ctx):
    lm, raw, st = ctx["lm"], ctx["raw"], ctx["state"]
    qid = (lm.get("wikidata") or "").strip()
    path = os.path.join(raw, "wikidata.json")
    key = digest([VERSION, qid])
    if st.done("wikidata", key, [path]) and not ctx["force"]("wikidata"):
        return L.read_json(path)
    if not qid:
        doc = {"id": "", "entity": {}, "labels": {}}
        L.write_json(path, doc)
        st.mark("wikidata", key, digest(doc))
        return doc
    LOG(f"  wikidata {qid}")
    d = ctx["http"].get_json(WD_ENTITY.format(qid))
    ent = d["entities"].get(qid) or next(iter(d["entities"].values()))
    need = set()
    claims = ent.get("claims", {})
    for pid, _, kind in FACT_PROPS:
        for c in claims.get(pid, []):
            v = wd_snak_value(c["mainsnak"])
            if kind == "item" and v:
                need.add(v["id"])
            if kind == "quantity" and v:
                u = v.get("unit", "").rsplit("/", 1)[-1]
                if u.startswith("Q") and u not in UNIT_NAMES:
                    need.add(u)
            for qp, qs in c.get("qualifiers", {}).items():
                need.add(qp)
                for q in qs:
                    qv = wd_snak_value(q)
                    if isinstance(qv, dict) and qv.get("entity-type") == "item":
                        need.add(qv["id"])
    labels = wd_labels(ctx["http"], need) if need else {}
    doc = {"id": ent.get("id", qid), "entity": ent, "labels": labels}
    L.write_json(path, doc)
    st.mark("wikidata", key, digest(doc))
    return doc


def wd_facts(wd):
    ent, labels = wd.get("entity", {}), wd.get("labels", {})
    qid = wd.get("id", "")
    claims = ent.get("claims", {})
    facts = {}

    def qual_note(c):
        notes = []
        for qp, qs in c.get("qualifiers", {}).items():
            vals = []
            for q in qs:
                v = wd_snak_value(q)
                if isinstance(v, dict) and v.get("entity-type") == "item":
                    vals.append(labels.get(v["id"], v["id"]))
                elif isinstance(v, dict) and "time" in v:
                    vals.append(str(wd_time(v)))
                elif isinstance(v, dict) and "amount" in v:
                    vals.append(str(float(v["amount"])))
                elif isinstance(v, str):
                    vals.append(v)
            if vals:
                notes.append(f"{labels.get(qp, qp)}: {', '.join(vals)}")
        return "; ".join(notes)

    for pid, name, kind in FACT_PROPS:
        stmts = [c for c in claims.get(pid, []) if c.get("rank") != "deprecated"]
        stmts.sort(key=lambda c: c.get("rank") != "preferred")
        if not stmts:
            continue
        src = f"https://www.wikidata.org/wiki/{qid}#{pid}"
        if kind in ("item", "string", "monolingual"):
            vals, notes = [], []
            for c in stmts:
                v = wd_snak_value(c["mainsnak"])
                if v is None:
                    continue
                if kind == "item":
                    vals.append(labels.get(v["id"], v["id"]))
                elif kind == "monolingual":
                    vals.append(v.get("text", ""))
                else:
                    vals.append(str(v))
                n = qual_note(c)
                if n:
                    notes.append(f"{vals[-1]} ({n})")
            if not vals:
                continue
            f = {"value": vals[0] if len(vals) == 1 else vals, "source": src}
            if notes:
                f["note"] = "; ".join(notes)
            facts[name] = f
            continue
        n_out = 0
        for c in stmts:
            v = wd_snak_value(c["mainsnak"])
            if v is None:
                continue
            f = {}
            if kind == "quantity":
                amt = float(v["amount"])
                f["value"] = int(amt) if amt.is_integer() and name == "floors" else amt
                u = v.get("unit", "").rsplit("/", 1)[-1]
                if u and u != "1":
                    f["unit"] = UNIT_NAMES.get(u, labels.get(u, u))
            else:
                f["value"] = wd_time(v)
            f["source"] = src
            n = qual_note(c)
            if n:
                f["note"] = n
            n_out += 1
            facts[name if n_out == 1 else f"{name}_{n_out}"] = f
    return facts


def wd_coord(wd):
    for c in wd.get("entity", {}).get("claims", {}).get("P625", []):
        v = wd_snak_value(c["mainsnak"])
        if v:
            return float(v["latitude"]), float(v["longitude"])
    return None


def wd_string(wd, pid):
    for c in wd.get("entity", {}).get("claims", {}).get(pid, []):
        v = wd_snak_value(c["mainsnak"])
        if isinstance(v, str):
            return v
    return None


def wd_sitelinks(wd):
    sl = wd.get("entity", {}).get("sitelinks", {})
    out = {}
    for lang in ("es", "ca", "en"):
        s = sl.get(f"{lang}wiki")
        if s:
            out[lang] = s.get("url") or ("https://%s.wikipedia.org/wiki/%s" % (
                lang, urllib.parse.quote(s["title"].replace(" ", "_"))))
    return out


def wd_label_names(wd):
    labs = wd.get("entity", {}).get("labels", {})
    names = [labs[l]["value"] for l in ("es", "ca", "en") if l in labs]
    for l in ("es", "ca", "en"):
        for a in wd.get("entity", {}).get("aliases", {}).get(l, [])[:4]:
            names.append(a["value"])
    return names


# ---------------------------------------------------------------------------
# OSM
# ---------------------------------------------------------------------------

class OsmStore:
    def __init__(self):
        self.nodes, self.ways, self.rels = {}, {}, {}

    def add(self, elements):
        for e in elements:
            t = e.get("type")
            if t == "node":
                self.nodes[e["id"]] = (e["lat"], e["lon"])
            elif t == "way":
                self.ways[e["id"]] = e
            elif t == "relation":
                old = self.rels.get(e["id"])
                if old is None or len(e.get("members", [])) >= len(old.get("members", [])):
                    self.rels[e["id"]] = e

    def way_ll(self, wid):
        w = self.ways.get(wid)
        if not w:
            return None
        pts = [self.nodes.get(n) for n in w.get("nodes", [])]
        if any(p is None for p in pts):
            return None
        return pts


def assemble_rings(segments):
    """segments: list of node-id lists. Returns (closed rings as node lists, leftovers)."""
    segs = [list(s) for s in segments if len(s) >= 2]
    rings, left = [], 0
    while segs:
        cur = segs.pop(0)
        changed = True
        while cur[0] != cur[-1] and changed:
            changed = False
            for i, s in enumerate(segs):
                if s[0] == cur[-1]:
                    cur = cur + s[1:]
                elif s[-1] == cur[-1]:
                    cur = cur + s[::-1][1:]
                elif s[-1] == cur[0]:
                    cur = s + cur[1:]
                elif s[0] == cur[0]:
                    cur = s[::-1] + cur[1:]
                else:
                    continue
                segs.pop(i)
                changed = True
                break
        if cur[0] == cur[-1] and len(cur) >= 4:
            rings.append(cur)
        else:
            left += 1
    return rings, left


def to_city_ring(pts_ll):
    return L.open_ring([L.ll_to_city(lat, lon) for lat, lon in pts_ll])


def element_polys(store, etype, eid):
    """-> (list of (outer, inners) in city frame, warnings). None if not an area."""
    warns = []
    if etype == "way":
        w = store.ways.get(eid)
        if not w:
            return None, ["missing way"]
        nds = w.get("nodes", [])
        if len(nds) < 4 or nds[0] != nds[-1]:
            return None, []
        pts = store.way_ll(eid)
        if pts is None:
            return None, ["incomplete way"]
        ring = to_city_ring(pts)
        if len(ring) < 3:
            return None, []
        return [(ring, [])], warns
    rel = store.rels.get(eid)
    if not rel:
        return None, ["missing relation"]
    rtype = rel.get("tags", {}).get("type")
    members = rel.get("members", [])
    if rtype == "building":
        outl = [m for m in members if m.get("role") == "outline"]
        if outl:
            m = outl[0]
            return element_polys(store, m["type"], m["ref"])
        return None, ["type=building relation without outline"]
    if rtype not in ("multipolygon", "boundary"):
        return None, []
    outer_segs, inner_segs = [], []
    for m in members:
        if m.get("type") != "way":
            continue
        w = store.ways.get(m["ref"])
        if not w:
            warns.append(f"member way {m['ref']} missing")
            continue
        if any(n not in store.nodes for n in w.get("nodes", [])):
            warns.append(f"member way {m['ref']} incomplete")
            continue
        (inner_segs if m.get("role") == "inner" else outer_segs).append(w["nodes"])
    outers, l1 = assemble_rings(outer_segs)
    inners, l2 = assemble_rings(inner_segs)
    if l1 or l2:
        warns.append(f"{l1 + l2} unclosed ring(s) dropped")
    o_rings = [to_city_ring([store.nodes[n] for n in r]) for r in outers]
    i_rings = [to_city_ring([store.nodes[n] for n in r]) for r in inners]
    o_rings = [r for r in o_rings if len(r) >= 3]
    polys = [(r, []) for r in sorted(o_rings, key=L.ring_area, reverse=True)]
    for ir in i_rings:
        if len(ir) < 3:
            continue
        best = None
        for k, (o, _) in enumerate(polys):
            if L.point_in_polygon(ir[0][0], ir[0][1], o) or \
               L.point_in_polygon(*L.ring_centroid(ir)[0], o):
                if best is None or L.ring_area(o) < L.ring_area(polys[best][0]):
                    best = k
        if best is not None:
            polys[best][1].append(ir)
        else:
            warns.append("inner ring outside all outers")
    return (polys or None), warns


def is_yes(v):
    return v is not None and v != "no"


def osm_bbox_ll(lat, lon, half_m):
    dlat = half_m / 111320.0
    dlon = half_m / (111320.0 * math.cos(math.radians(lat)))
    return (lon - dlon, lat - dlat, lon + dlon, lat + dlat)


def fetch_map(ctx, bbox, prefix):
    name = f"{prefix}_{digest([round(v, 6) for v in bbox])[:8]}.json"
    ctx.setdefault("used_files", set()).add(name)
    path = os.path.join(ctx["raw"], "osm", name)
    if os.path.exists(path) and not ctx["force"]("osm"):
        return L.read_json(path)
    LOG(f"  osm map {name} bbox={','.join('%.5f' % v for v in bbox)}")
    d = _map_json(ctx["http"], bbox, depth=0)
    L.write_json(path, d, indent=None)
    return d


def _map_json(http, bbox, depth):
    """map.json; on HTTP 400 (too many nodes) split the box in four and merge."""
    try:
        return http.get_json(OSM_API + "/map.json", {"bbox": ",".join("%.6f" % v for v in bbox)})
    except L.HttpError as ex:
        if ex.code != 400 or depth >= 3:
            raise
    x0, y0, x1, y1 = bbox
    xm, ym = (x0 + x1) / 2, (y0 + y1) / 2
    LOG(f"  osm map too dense, splitting (depth {depth + 1})")
    seen, els = set(), []
    for bb in ((x0, y0, xm, ym), (xm, y0, x1, ym), (x0, ym, xm, y1), (xm, ym, x1, y1)):
        for e in _map_json(http, bb, depth + 1).get("elements", []):
            k = (e["type"], e["id"])
            if k not in seen:
                seen.add(k)
                els.append(e)
    return {"version": "0.6", "elements": els}


def fetch_full(ctx, etype, eid):
    path = os.path.join(ctx["raw"], "osm", f"{etype}_{eid}.json")
    if os.path.exists(path) and not ctx["force"]("osm"):
        return L.read_json(path)
    LOG(f"  osm {etype}/{eid}/full")
    d = ctx["http"].get_json(f"{OSM_API}/{etype}/{eid}/full.json")
    L.write_json(path, d, indent=None)
    return d


def parse_osm_id(s):
    s = str(s).strip()
    m = re.match(r"^(?:osm:)?(way|relation|w|r|node|n)\s*/?\s*(\d+)$", s, re.I)
    if m:
        t = m.group(1).lower()
        t = {"w": "way", "r": "relation", "n": "node"}.get(t, t)
        return t, int(m.group(2))
    if s.isdigit():
        return "way", int(s)
    raise ValueError(f"bad osm id {s!r}")


def tile_bboxes(x0, z0, x1, z1, tile=280.0):
    """Split a city-frame box into <= tile m tiles; return lat/lon bboxes."""
    out = []
    nx = max(1, math.ceil((x1 - x0) / tile))
    nz = max(1, math.ceil((z1 - z0) / tile))
    for i in range(nx):
        for j in range(nz):
            ax, bx = x0 + (x1 - x0) * i / nx, x0 + (x1 - x0) * (i + 1) / nx
            az, bz = z0 + (z1 - z0) * j / nz, z0 + (z1 - z0) * (j + 1) / nz
            la0, lo0 = L.city_to_ll(ax, bz)
            la1, lo1 = L.city_to_ll(bx, az)
            out.append((min(lo0, lo1), min(la0, la1), max(lo0, lo1), max(la0, la1)))
    return out


def poly_to_ll(polys):
    return [[[list(L.city_to_ll(x, z)) for x, z in o],
             [[list(L.city_to_ll(x, z)) for x, z in r] for r in inn]] for o, inn in polys]


def polys_from_ll(polys_ll):
    out = []
    for o, inn in polys_ll:
        out.append(([L.ll_to_city(a, b) for a, b in o], [[L.ll_to_city(a, b) for a, b in r] for r in inn]))
    return out


def step_osm(ctx, point_ll, names):
    lm, raw, st = ctx["lm"], ctx["raw"], ctx["state"]
    qid = (lm.get("wikidata") or "").strip()
    osm_ids = [parse_osm_id(x) for x in (lm.get("osm") or [])]
    sel_path = os.path.join(raw, "osm", "selection.json")
    key = digest([VERSION, qid, osm_ids, [round(v, 6) for v in point_ll] if point_ll else None,
                  sorted(names)])
    if st.done("osm", key, [sel_path]) and not ctx["force"]("osm"):
        return L.read_json(sel_path)
    warns = []
    store = OsmStore()
    fetched_boxes = []  # city-frame boxes covered by map.json
    if point_ll:
        bb = osm_bbox_ll(point_ll[0], point_ll[1], MAP_HALF)
        store.add(fetch_map(ctx, bb, "map_point")["elements"])
        px, pz = L.ll_to_city(*point_ll)
        fetched_boxes.append((px - MAP_HALF, pz - MAP_HALF, px + MAP_HALF, pz + MAP_HALF))
    else:
        px = pz = None

    def polys_of(etype, eid, full=False):
        if etype == "relation" and full:
            store.add(fetch_full(ctx, "relation", eid)["elements"])
        return element_polys(store, etype, eid)

    elements = {}  # key -> dict(type,id,tags,polys)

    def consider(etype, eid, full=False):
        k = f"{etype}/{eid}"
        if k in elements:
            return elements[k]
        src = store.ways.get(eid) if etype == "way" else store.rels.get(eid)
        tags = (src or {}).get("tags", {})
        polys, w = polys_of(etype, eid, full)
        if w:
            warns.extend(f"{k}: {x}" for x in w)
        elements[k] = {"type": etype, "id": eid, "tags": tags, "polys": polys}
        return elements[k]

    candidates_info = []
    method = ""
    chosen = []
    if osm_ids:
        method = "list"
        for etype, eid in osm_ids:
            if etype == "node":
                warns.append(f"node/{eid} in list ignored (not an area)")
                continue
            d = fetch_full(ctx, etype, eid)
            store.add(d["elements"])
            e = consider(etype, eid)
            if e["polys"]:
                chosen.append(f"{etype}/{eid}")
            else:
                warns.append(f"{etype}/{eid}: no area geometry")
    else:
        # all area elements from the map; complete relations of interest
        for wid, w in list(store.ways.items()):
            t = w.get("tags", {})
            if is_yes(t.get("building")) or is_yes(t.get("building:part")) or t.get("wikidata"):
                consider("way", wid)
        for rid, r in list(store.rels.items()):
            t = r.get("tags", {})
            if t.get("type") in ("multipolygon", "building") and (
                    is_yes(t.get("building")) or is_yes(t.get("building:part")) or
                    (qid and t.get("wikidata") == qid)):
                consider("relation", rid, full=True)
        areas = {k: e for k, e in elements.items() if e["polys"]}

        def bld(e):
            return is_yes(e["tags"].get("building"))

        def part(e):
            return is_yes(e["tags"].get("building:part")) and not bld(e)

        def has_qid(e):
            return qid and qid in [x.strip() for x in e["tags"].get("wikidata", "").split(";")]

        def contains(e):
            return px is not None and any(L.point_in_polygon(px, pz, o, i) for o, i in e["polys"])

        def dist(e):
            if px is None:
                return 0.0
            return min(L.distance_point_polygon(px, pz, o, i) for o, i in e["polys"])

        def area(e):
            return sum(L.polygon_area(o, i) for o, i in e["polys"])

        def is_block(e):
            """A building that contains >= 2 other buildings' centroids."""
            n = 0
            for k2, e2 in areas.items():
                if e2 is e or not bld(e2):
                    continue
                c = L.polygon_centroid(*e2["polys"][0])
                if area(e2) < area(e) * 0.9 and any(L.point_in_polygon(c[0], c[1], o, i) for o, i in e["polys"]):
                    n += 1
            return n >= 2

        for k, e in areas.items():
            if bld(e) or part(e) or has_qid(e):
                candidates_info.append({
                    "id": "osm:" + k, "name": e["tags"].get("name", ""), "building": e["tags"].get("building", ""),
                    "part": e["tags"].get("building:part", ""), "wikidata": e["tags"].get("wikidata", ""),
                    "area": round(area(e), 1), "contains_point": contains(e), "dist": round(dist(e), 1)})
        q_b = [k for k, e in areas.items() if has_qid(e) and bld(e)]
        q_p = [k for k, e in areas.items() if has_qid(e) and part(e)]
        q_other = [k for k, e in areas.items() if has_qid(e) and not bld(e) and not part(e)]
        if q_b:
            chosen, method = q_b, "wikidata"
        elif q_other:
            # e.g. an amenity/historic area carrying the QID: buildings inside it
            inside = []
            for k, e in areas.items():
                if bld(e) and k not in q_other:
                    p = (e["polys"][0][0], e["polys"][0][1])
                    zone = [pp for kk in q_other for pp in areas[kk]["polys"]]
                    if L.overlap_fraction(p, zone) >= 0.5:
                        inside.append(k)
            if inside:
                chosen, method = inside, "wikidata-area"
                warns.append(f"QID on non-building {q_other}; using {len(inside)} building(s) inside it")
            elif q_p:
                chosen, method = q_p, "wikidata-part"
            else:
                chosen, method = q_other, "wikidata-nonbuilding"
                warns.append(f"QID only on non-building area(s) {q_other}; used as outline")
        elif q_p:
            chosen, method = q_p, "wikidata-part"
            warns.append("QID only on building:part elements")
        if not chosen:
            nm = []
            for k, e in areas.items():
                if not bld(e):
                    continue
                en = [e["tags"].get(t, "") for t in ("name", "name:es", "name:ca", "name:va", "name:en",
                                                       "alt_name", "official_name", "old_name", "short_name")]
                if any(names_match(a, b) for a in en if a for b in names if b) and dist(e) < 150:
                    nm.append(k)
            if nm:
                inside = [k for k in nm if contains(areas[k])]
                chosen = inside or [min(nm, key=lambda k: dist(areas[k]))]
                method = "name"
                if len(nm) > len(chosen):
                    warns.append(f"ambiguous name match: {nm}, chose {chosen}")
        if not chosen and px is not None:
            cont = [k for k, e in areas.items() if bld(e) and contains(e)]
            nonblock = [k for k in cont if not is_block(areas[k])]
            pool = nonblock or cont
            if pool:
                best = min(pool, key=lambda k: area(areas[k]))
                chosen, method = [best], "contains"
                if len(pool) > 1:
                    warns.append(f"ambiguous: point inside {len(pool)} buildings {pool}; chose smallest {best}")
                if len(cont) > len(nonblock) and nonblock:
                    warns.append(f"skipped block-like buildings {sorted(set(cont) - set(nonblock))}")
            else:
                near = sorted((dist(e), k) for k, e in areas.items() if bld(e))
                if near and near[0][0] <= 30:
                    chosen, method = [near[0][1]], "nearest"
                    warns.append(f"point inside no building; nearest {near[0][1]} at {near[0][0]:.1f} m")
        if not chosen:
            warns.append("no OSM outline found")
    # ordering: list order, else by area
    if method != "list":
        chosen.sort(key=lambda k: -sum(L.polygon_area(o, i) for o, i in elements[k]["polys"]))
    outline_polys = [p for k in chosen for p in elements[k]["polys"]]
    # make sure the whole outline is covered by map data, then collect parts
    parts = []
    if outline_polys:
        x0, z0, x1, z1 = L.bbox_of([o for o, _ in outline_polys])
        x0, z0, x1, z1 = x0 - 10, z0 - 10, x1 + 10, z1 + 10
        covered = any(bx0 <= x0 and bz0 <= z0 and bx1 >= x1 and bz1 >= z1
                      for bx0, bz0, bx1, bz1 in fetched_boxes)
        if not covered:
            tiles = tile_bboxes(x0, z0, x1, z1)
            if len(tiles) > 16:
                warns.append(f"outline bbox too large for part search ({len(tiles)} tiles); limited to 16")
                tiles = tiles[:16]
            for i, bb in enumerate(tiles):
                store.add(fetch_map(ctx, bb, f"map_outline{i}")["elements"])
            fetched_boxes.append((x0, z0, x1, z1))
        for wid, w in list(store.ways.items()):
            if is_yes(w.get("tags", {}).get("building:part")):
                consider("way", wid)
        for rid, r in list(store.rels.items()):
            t = r.get("tags", {})
            if is_yes(t.get("building:part")) and t.get("type") == "multipolygon":
                consider("relation", rid, full=True)
        for k, e in elements.items():
            if k in chosen or not e["polys"] or not is_yes(e["tags"].get("building:part")):
                continue
            o, i = e["polys"][0]
            ox0, oz0, ox1, oz1 = L.bbox_of([o])
            if ox1 < x0 or ox0 > x1 or oz1 < z0 or oz0 > z1:
                continue
            frac = L.overlap_fraction((o, i), outline_polys)
            if frac >= PART_MIN_OVERLAP:
                parts.append(k)
        parts.sort(key=lambda k: -sum(L.polygon_area(o, i) for o, i in elements[k]["polys"]))
    sel = {
        "method": method,
        "point": list(point_ll) if point_ll else None,
        "warnings": warns,
        "outlines": [{"id": "osm:" + k, "tags": elements[k]["tags"],
                      "polys_ll": poly_to_ll(elements[k]["polys"])} for k in chosen],
        "parts": [{"id": "osm:" + k, "tags": elements[k]["tags"],
                   "polys_ll": poly_to_ll(elements[k]["polys"])} for k in parts],
        "candidates": sorted(candidates_info, key=lambda c: (not c["contains_point"], c["dist"]))[:40],
    }
    L.write_json(sel_path, sel)
    prune(os.path.join(raw, "osm"), r"^map_.*\.json$", ctx.get("used_files", set()))
    st.warn("osm", warns)
    st.mark("osm", key, digest(sel))
    for w in warns:
        LOG("  WARN osm:", w)
    LOG(f"  osm: method={method or '-'} outlines={chosen} parts={len(parts)}")
    return sel


# ---------------------------------------------------------------------------
# Catastro
# ---------------------------------------------------------------------------

def parse_poslist(s):
    v = [float(x) for x in s.split()]
    return L.open_ring(list(zip(v[0::2], v[1::2])))


def parse_gml_features(text, tag):
    """Regex GML reader for Catastro INSPIRE BU (BuildingPart / Building)."""
    if "ExceptionReport" in text[:3000] or "ExceptionText" in text[:3000]:
        m = re.search(r"<[^>]*ExceptionText>(.*?)</", text, re.S)
        raise RuntimeError("catastro exception: " + (m.group(1).strip() if m else text[:300]))
    feats = []
    for m in re.finditer(r"<bu-ext2d:%s\s+gml:id=\"([^\"]+)\"(.*?)</bu-ext2d:%s>" % (tag, tag), text, re.S):
        gid, body = m.group(1), m.group(2)
        lid = re.search(r"<base:localId>([^<]+)</base:localId>", body)
        local = lid.group(1).strip() if lid else gid.rsplit(".", 1)[-1]
        polys = []
        for pm in re.finditer(r"<gml:PolygonPatch>(.*?)</gml:PolygonPatch>", body, re.S):
            pb = pm.group(1)
            ext = re.search(r"<gml:exterior>.*?<gml:posList[^>]*>([^<]*)</gml:posList>", pb, re.S)
            if not ext:
                continue
            outer = parse_poslist(ext.group(1))
            inners = [parse_poslist(x) for x in
                      re.findall(r"<gml:interior>.*?<gml:posList[^>]*>([^<]*)</gml:posList>", pb, re.S)]
            if len(outer) >= 3:
                polys.append((outer, [r for r in inners if len(r) >= 3]))

        def num(t):
            mm = re.search(r"<bu-ext2d:%s[^>]*>([^<]*)<" % t, body)
            if mm and mm.group(1).strip():
                try:
                    return int(float(mm.group(1)))
                except ValueError:
                    return None
            return None

        f = {"gml_id": gid, "local_id": local, "refcat": local.split("_")[0], "polys": polys,
             "floors_above": num("numberOfFloorsAboveGround"),
             "floors_below": num("numberOfFloorsBelowGround")}
        if tag == "Building":
            for k, pat in (("built", r"<bu-core2d:beginning>(\d{4})"),
                           ("use", r"<bu-ext2d:currentUse>([^<]+)<"),
                           ("condition", r"<bu-core2d:conditionOfConstruction>([^<]+)<"),
                           ("units", r"<bu-ext2d:numberOfBuildingUnits>(\d+)<"),
                           ("dwellings", r"<bu-ext2d:numberOfDwellings>(\d+)<"),
                           ("area", r"<bu-ext2d:value uom=\"m2\">([\d.]+)<"),
                           ("facade_photo", r"<bu-ext2d:documentLink>([^<]+)<")):
                mm = re.search(pat, body)
                if mm:
                    f[k] = mm.group(1).strip()
        feats.append(f)
    return feats


def catastro_query(ctx, typename, e0, n0, e1, n1, fname):
    """GetFeature over a UTM box (tiled at 1 km); cached raw GML per tile."""
    feats, seen = [], set()
    nx = max(1, math.ceil((e1 - e0) / 1000.0))
    ny = max(1, math.ceil((n1 - n0) / 1000.0))
    for i in range(nx):
        for j in range(ny):
            a0, a1 = e0 + (e1 - e0) * i / nx, e0 + (e1 - e0) * (i + 1) / nx
            b0, b1 = n0 + (n1 - n0) * j / ny, n0 + (n1 - n0) * (j + 1) / ny
            suffix = "" if nx * ny == 1 else f"_{i}_{j}"
            gname = f"{fname}{suffix}_{digest([round(v, 2) for v in (a0, b0, a1, b1)])[:8]}.gml"
            ctx.setdefault("used_files", set()).add(gname)
            path = os.path.join(ctx["raw"], "catastro", gname)
            if os.path.exists(path) and not ctx["force"]("catastro"):
                data = open(path, "rb").read()
            else:
                LOG(f"  catastro {typename} {gname}")
                data = ctx["http"].get(CATASTRO_BU, {
                    "service": "WFS", "version": "2.0.0", "request": "GetFeature",
                    "typeNames": typename,
                    "bbox": f"{a0:.2f},{b0:.2f},{a1:.2f},{b1:.2f}", "SRSname": "EPSG::25830"})
                L.write_bytes(path, data)
            text = data.decode("latin-1")
            tag = typename.split(":")[1]
            for f in parse_gml_features(text, tag):
                if f["gml_id"] in seen:
                    continue
                seen.add(f["gml_id"])
                feats.append(f)
    return feats


def en_poly_to_city(poly):
    o, inn = poly
    return ([L.en_to_city(e, n) for e, n in o], [[L.en_to_city(e, n) for e, n in r] for r in inn])


def outline_anchor(sel):
    polys = []
    for o in sel.get("outlines", []):
        polys.extend(polys_from_ll(o["polys_ll"]))
    if not polys:
        return None, []
    o, i = polys[0]
    c = L.polygon_centroid(o, i)
    return (round(c[0], 2), round(c[1], 2)), polys


def step_catastro(ctx, sel, point_ll):
    raw, st = ctx["raw"], ctx["state"]
    out_path = os.path.join(raw, "catastro", "catastro.json")
    key = digest([VERSION, st.out("osm"), point_ll])
    if st.done("catastro", key, [out_path]) and not ctx["force"]("catastro"):
        return L.read_json(out_path)
    warns = []
    anchor, polys = outline_anchor(sel)
    result = {"parts": [], "context": [], "buildings": [], "fallback_outline": None}
    if anchor is None and point_ll:
        # No OSM outline: fall back to the Catastro building containing the point.
        px, pz = L.ll_to_city(*point_ll)
        e, n = L.city_to_en(px, pz)
        blds = catastro_query(ctx, "BU:Building", e - 60, n - 60, e + 60, n + 60, "building_point")
        best = None
        for b in blds:
            for poly in b["polys"]:
                cp = en_poly_to_city(poly)
                if L.point_in_polygon(px, pz, *cp):
                    a = L.polygon_area(*cp)
                    if best is None or a < best[0]:
                        best = (a, b, cp)
        if best:
            warns.append(f"outline from Catastro building {best[1]['refcat']} (no OSM outline)")
            result["fallback_outline"] = {"id": "catastro:" + best[1]["refcat"],
                                          "tags": {k: v for k, v in best[1].items() if k not in ("polys",)},
                                          "polys_ll": poly_to_ll([best[2]])}
            sel = {"outlines": [result["fallback_outline"]]}
            anchor, polys = outline_anchor(sel)
    if anchor is None:
        warns.append("no outline: catastro skipped")
        L.write_json(out_path, result)
        st.warn("catastro", warns)
        st.mark("catastro", key, digest(result))
        return result
    x0, z0, x1, z1 = L.bbox_of([o for o, _ in polys])
    e0, n1 = L.city_to_en(x0, z0)
    e1, n0 = L.city_to_en(x1, z1)
    m = CATASTRO_MARGIN
    try:
        parts = catastro_query(ctx, "BU:BuildingPart", e0 - m, n0 - m, e1 + m, n1 + m, "parts_outline")
        blds = catastro_query(ctx, "BU:Building", e0 - m, n0 - m, e1 + m, n1 + m, "buildings_outline")
        ae, an = L.city_to_en(*anchor)
        h = CONTEXT_HALF
        ctxparts = catastro_query(ctx, "BU:BuildingPart", ae - h, an - h, ae + h, an + h, "parts_context")
    except (L.HttpError, RuntimeError) as ex:
        warns.append(f"catastro query failed: {ex}")
        L.write_json(out_path, result)
        st.warn("catastro", warns)
        st.mark("catastro", "failed:" + key, digest(result))
        return result
    chosen_ids = set()
    refcat_area = {}
    for f in parts:
        for poly in f["polys"]:
            cp = en_poly_to_city(poly)
            frac = L.overlap_fraction(cp, polys)
            if frac >= PART_MIN_OVERLAP:
                a = L.polygon_area(*cp)
                result["parts"].append({
                    "id": f["gml_id"], "refcat": f["refcat"], "floors_above": f["floors_above"],
                    "floors_below": f["floors_below"], "overlap": round(frac, 3), "area": round(a, 1),
                    "polys_ll": poly_to_ll([cp])})
                chosen_ids.add(f["gml_id"])
                refcat_area[f["refcat"]] = refcat_area.get(f["refcat"], 0.0) + a * frac
    ax, az = anchor
    for f in ctxparts:
        if f["gml_id"] in chosen_ids:
            continue
        for poly in f["polys"]:
            cp = en_poly_to_city(poly)
            if L.distance_point_polygon(ax, az, cp[0]) <= CONTEXT_RADIUS:
                result["context"].append({"id": f["gml_id"], "floors_above": f["floors_above"],
                                          "polys_ll": poly_to_ll([(cp[0], [])])})
    for b in blds:
        if b["refcat"] in refcat_area:
            info = {k: v for k, v in b.items() if k != "polys"}
            info["area_in_outline"] = round(refcat_area[b["refcat"]], 1)
            result["buildings"].append(info)
    result["buildings"].sort(key=lambda b: -b["area_in_outline"])
    if not result["parts"]:
        warns.append("no Catastro BuildingPart overlaps the outline by >= 50% "
                     "(public monuments are often missing from Catastro)")
    # cadastral WMS map (a 'catastro' plan) of the outline bbox + 30 m
    wms_path = os.path.join(raw, "plans", "catastro_wms.png")
    pad = 30.0
    we0, wn0, we1, wn1 = e0 - pad, n0 - pad, e1 + pad, n1 + pad
    res = max(0.08, max(we1 - we0, wn1 - wn0) / 2000.0)
    wbox = [round(v, 2) for v in (we0, wn0, we1, wn1)]
    prev = {}
    if os.path.exists(out_path):
        try:
            prev = L.read_json(out_path).get("wms") or {}
        except Exception:  # noqa: BLE001
            prev = {}
    if os.path.exists(wms_path) and prev.get("bbox_utm") == wbox and not ctx["force"]("catastro"):
        result["wms"] = prev
    else:
        try:
            W = int(round((we1 - we0) / res))
            H = int(round((wn1 - wn0) / res))
            LOG("  catastro WMS map")
            data = ctx["http"].get(CATASTRO_WMS, {
                "SERVICE": "WMS", "VERSION": "1.1.1", "REQUEST": "GetMap", "LAYERS": "Catastro",
                "STYLES": "", "SRS": "EPSG:25830", "BBOX": f"{we0:.2f},{wn0:.2f},{we1:.2f},{wn1:.2f}",
                "WIDTH": str(W), "HEIGHT": str(H), "FORMAT": "image/png", "TRANSPARENT": "FALSE"})
            im = Image.open(io.BytesIO(data))
            im.load()
            L.write_bytes(wms_path, data)
            result["wms"] = {"file": "plans/catastro_wms.png", "bbox_utm": wbox,
                             "width": im.width, "height": im.height, "m_per_px": round(res, 4)}
        except Exception as ex:  # noqa: BLE001
            warns.append(f"catastro WMS failed: {ex}")
    # cadastral facade photos of the main parcels
    result["facade_photos"] = []
    for b in result["buildings"][:2]:
        if b.get("area_in_outline", 0) < 30:
            continue
        rc = b["refcat"]
        path = os.path.join(raw, "photos", f"catastro_{rc}.jpg")
        if not os.path.exists(path) or ctx["force"]("catastro"):
            try:
                LOG(f"  catastro facade photo {rc}")
                data = ctx["http"].get(CATASTRO_FACHADA.format(rc))
                if not data:
                    raise RuntimeError("no facade photo published (empty response)")
                im = Image.open(io.BytesIO(data))
                im.load()
                if im.width < 200 or im.height < 200:
                    raise RuntimeError("tiny image")
                L.write_bytes(path, data)
            except Exception as ex:  # noqa: BLE001
                warns.append(f"catastro facade photo {rc}: {ex}")
                continue
        im = Image.open(path)
        result["facade_photos"].append({"file": f"photos/catastro_{rc}.jpg", "refcat": rc,
                                        "width": im.width, "height": im.height,
                                        "url": CATASTRO_FACHADA.format(rc)})
    result["warnings"] = warns
    L.write_json(out_path, result)
    prune(os.path.join(raw, "catastro"), r"^.*\.gml$", ctx.get("used_files", set()))
    st.warn("catastro", warns)
    st.mark("catastro", key, digest(result))
    for w in warns:
        LOG("  WARN catastro:", w)
    LOG(f"  catastro: parts={len(result['parts'])} context={len(result['context'])} "
        f"buildings={[b['refcat'] for b in result['buildings']]}")
    return result


# ---------------------------------------------------------------------------
# footprint.json
# ---------------------------------------------------------------------------

def local_ring(ring, anchor):
    ax, az = anchor
    return [[round(x - ax, 2), round(z - az, 2)] for x, z in ring]


def step_footprint(ctx, sel, cat):
    lm, st = ctx["lm"], ctx["state"]
    path = os.path.join(ctx["data"], "footprint.json")
    key = digest([VERSION, st.out("osm"), st.out("catastro")])
    if st.done("footprint", key, [path]) and not ctx["force"]("footprint"):
        return L.read_json(path)
    outlines_src = list(sel.get("outlines", []))
    if not outlines_src and cat.get("fallback_outline"):
        outlines_src = [cat["fallback_outline"]]
    anchor, _ = outline_anchor({"outlines": outlines_src})
    if anchor is None:
        LOG("  footprint: no outline, not written")
        st.warn("footprint", ["no outline: footprint.json not written"])
        return None
    fp = {"slug": lm["slug"], "frame": L.FRAME, "origin": L.origin_dict(), "anchor": list(anchor),
          "attribution": list(ATTRIBUTION), "outlines": [], "parts": [], "catastro": [], "context": []}

    def entries(src):
        out = []
        for el in src:
            for o, inn in polys_from_ll(el["polys_ll"]):
                out.append({"id": el["id"], "tags": el.get("tags", {}), "outer": local_ring(o, anchor),
                            "inners": [local_ring(r, anchor) for r in inn]})
        return out

    fp["outlines"] = entries(outlines_src)
    fp["parts"] = entries(sel.get("parts", []))
    for p in cat.get("parts", []):
        for o, inn in polys_from_ll(p["polys_ll"]):
            fp["catastro"].append({"id": p["id"], "refcat": p["refcat"], "floors_above": p["floors_above"],
                                   "floors_below": p["floors_below"], "outer": local_ring(o, anchor),
                                   "inners": [local_ring(r, anchor) for r in inn]})
    for c in cat.get("context", []):
        for o, _ in polys_from_ll(c["polys_ll"]):
            fp["context"].append({"source": "catastro", "floors_above": c["floors_above"],
                                  "outer": local_ring(o, anchor)})
    os.makedirs(ctx["data"], exist_ok=True)
    tmp = path + ".tmp"
    with open(tmp, "w", encoding="utf-8") as f:
        f.write(dump_footprint(fp))
    os.replace(tmp, path)
    st.mark("footprint", key, file_digest(path), anchor=list(anchor))
    LOG(f"  footprint: anchor={anchor} outlines={len(fp['outlines'])} parts={len(fp['parts'])} "
        f"catastro={len(fp['catastro'])} context={len(fp['context'])}")
    return fp


def dump_footprint(fp):
    """Readable JSON: one ring per line."""
    def ring(r):
        return "[" + ",".join("[%s,%s]" % (_num(x), _num(z)) for x, z in r) + "]"

    def entry(e, keys):
        parts = []
        for k in keys:
            if k not in e:
                continue
            v = e[k]
            if k == "outer":
                parts.append('"outer": ' + ring(v))
            elif k == "inners":
                parts.append('"inners": [' + ",".join(ring(r) for r in v) + "]")
            else:
                parts.append(json.dumps(k) + ": " + json.dumps(v, ensure_ascii=False))
        return "    {" + ",\n     ".join(parts) + "}"

    lines = ["{"]
    for k in ("slug", "frame", "origin", "anchor", "attribution"):
        lines.append(f"  {json.dumps(k)}: {json.dumps(fp[k], ensure_ascii=False)},")
    groups = [("outlines", ["id", "tags", "outer", "inners"]), ("parts", ["id", "tags", "outer", "inners"]),
              ("catastro", ["id", "refcat", "floors_above", "floors_below", "outer", "inners"]),
              ("context", ["source", "floors_above", "outer"])]
    for gi, (g, keys) in enumerate(groups):
        items = fp[g]
        end = "," if gi < len(groups) - 1 else ""
        if not items:
            lines.append(f'  "{g}": []{end}')
            continue
        lines.append(f'  "{g}": [')
        lines.append(",\n".join(entry(e, keys) for e in items))
        lines.append(f"  ]{end}")
    lines.append("}")
    text = "\n".join(lines) + "\n"
    json.loads(text)  # sanity
    return text


def _num(v):
    s = "%.2f" % v
    s = s.rstrip("0").rstrip(".") if "." in s else s
    return "0" if s in ("-0", "") else s


# ---------------------------------------------------------------------------
# Commons
# ---------------------------------------------------------------------------

def commons_api(http, params):
    p = {"format": "json", "formatversion": "2", **params}
    return http.get_json(COMMONS_API, p)


def commons_query_all(http, params, max_requests=30):
    pages, lists = {}, []
    cont = {}
    for _ in range(max_requests):
        d = commons_api(http, {**params, **cont})
        if "error" in d:
            raise RuntimeError(f"commons api error: {d['error']}")
        q = d.get("query", {})
        for p in q.get("pages", []):
            k = p.get("pageid") or p.get("title")
            cur = pages.setdefault(k, {})
            for kk, v in p.items():
                if isinstance(v, list) and isinstance(cur.get(kk), list):
                    for x in v:
                        if x not in cur[kk]:
                            cur[kk].append(x)
                elif kk not in cur:
                    cur[kk] = v
        for kk, v in q.items():
            if kk != "pages" and isinstance(v, list):
                lists.extend(v)
        if "continue" not in d:
            break
        cont = d["continue"]
    else:
        LOG(f"  commons: listing truncated after {max_requests} requests")
    return list(pages.values()), lists


SKIP_SUBCAT = re.compile(
    r"interior|inside|indoor|\bnave\b|altar|retablo|retaule|chapel|capilla|capella|ceiling|"
    r"techo|sostre|b[oó]veda|vault|\borgans?\b|[oó]rgano|\bsalas?\b|\brooms?\b|museum|museo|museu|"
    r"collection|colecci[oó]n|col[·.]?lecci[oó]|exhibit|exposici|exposici[oó]|\bexpo\b|"
    r"people|persons|visitors|portraits|\bevents?\b|festival|fallas|falles|concert|ceremon|"
    r"protest|celebrat|wedding|replica|r[eé]plica|maquet|\bmodels?\b|\bstamps|sellos|\bcoins|"
    r"postcards|logos?\b|paintings|pinturas|pintures|sculptures in|esculturas|artworks|"
    r"videos|audio|sound|columbian exposition|documents|documentos|archive|books|libros|"
    r"objects|objetos|furniture|mobiliario|tapestr|vitrales|stained glass|"
    r"\bcrypt|cripta|cloister|claustro|claustre|staircase|stairs|escalera|"
    r"ticket|\bmaps?\b|\bmapas?\b|fish market|\bstalls?\b|\bpuestos\b|\bparades\b|vendors|"
    r"products|productos|\bfood\b|comida",
    re.I)
EXTERIOR_SUBCAT = re.compile(r"exterior|fa[cç]ade|fachada|fa[cç]ana|outside|views? of|vistas?|"
                             r"aerial|a[eè]rea|surroundings|entorno|night views", re.I)
# parts of the outside of the building: exterior evidence, but no ranking bonus
ASPECT_EXT_SUBCAT = re.compile(r"towers?\b|\broofs?\b|cubierta|teulada|\bdomes?\b|c[uú]pula|\bentrances?\b|"
                               r"portals?\b|\bgates?\b|\bwalls?\b|muralla", re.I)
# acceptable subcategories even without a shared name token
ASPECT_SUBCAT = re.compile(ASPECT_EXT_SUBCAT.pattern + r"|details?\b|decorat|ornament|\bin \d{4}\b|"
                           r"photographs|photos|images", re.I)
# open-air but inside the building (courtyards): exterior, but ranked after street views
COURTYARD_SUBCAT = re.compile(r"\bpatio\b|\bpati\b|courtyard|garden|jard[ií]n|jard[ií]|claustro|cloister", re.I)
VEG_RE = re.compile(r"taronger|naranj|orange tree|\btrees?\b|[aá]rbol|arbre|zahrad|garden|jard[ií]n?\b|"
                    r"flowers?|flores|plants?\b", re.I)
# Drawing vocabulary. STRONG terms identify a drawing on their own; WEAK ones
# ("planta noble", "center section") only count in category names or for
# non-photographic files.
PLAN_STRONG_RE = re.compile(
    r"\bplan(?:o|os|ol|ols)\b|pl[aà]nols?|floor ?plans?|ground ?plans?|site ?plans?|\bplans? of\b|"
    r"\bplanta (?:baja|general|primera|segunda|alta|de la|del|de los|de les|i\b|y\b)|"
    r"alzados?|al[çc]ats?|elevation drawing|secci[oó]n|secci[oó]\b|cross.section|"
    r"corte (?:transversal|longitudinal)|dibujos?|drawings?|croquis|\bfichas?\b|levantamiento|blueprint|"
    r"\bsketch|esbozo|grabado|engraving|litograf|lithograph|\bl[aá]minas?\b|dibuix|"
    r"architectural drawing|technical drawing", re.I)
PLAN_WEAK_RE = re.compile(r"\bplans?\b|\bplanta\b|\bsections?\b|\belevations?\b", re.I)
PLAN_RE = re.compile(PLAN_STRONG_RE.pattern + "|" + PLAN_WEAK_RE.pattern, re.I)
MAP_RE = re.compile(r"\bmapas?\b|\bmaps?\b|plano de (?:la ciudad de )?valencia|plano geom[eé]trico|"
                    r"city plan|town plan|stadtplan|plan of (?:the city of )?valencia|"
                    r"pl[aà]nol de (?:la ciutat de )?val[eè]ncia|tosca", re.I)
INTERIOR_RE = re.compile(
    r"interior|inside|indoor|\bdins\b|\bdentro\b|\bnave\b|\baltar|retablo|retaule|capilla|capella|"
    r"chapel|ceiling|\btecho\b|sostre|b[oó]veda|\bvault|\bvolta\b|\bsalas?\b|\bsal[oó]n\b|"
    r"exposici[oó]n?|exhibition|exhibit|museum (?:objects|collection)|showcase|vitrina|"
    r"\bpuestos?\b|\bparadas?\b|\bstalls?\b|\bstands?\b|cr[ií]pta|\bcrypt|staircase|escalera interior|"
    r"columnas? helicoidal|helicoidal|sala de contrataci|contractaci[oó]|\borgans?\b|[oó]rgano|choir|\bcoro\b|"
    r"cimborio|dome interior|c[uú]pula interior|vidriera|stained glass|tapestr|"
    r"pescader|fish stall|market stall|\bfrutas\b|\bverduras\b|vegetables|\bfruits?\b", re.I)
EXTERIOR_RE = re.compile(
    r"exterior|fa[cç]ade|fachada|fa[cç]ana|outside|\bview\b|\bvista\b|\bvistes\b|\bfrom\b|\bdesde\b|"
    r"\bdes de\b|street|\bcalle\b|\bcarrer\b|\bplaza\b|\bpla[cç]a\b|square|panor[aà]mica?|skyline|"
    r"aerial|a[eè]rea|\bdrone\b|\btorres?\b|\btowers?\b|\broof\b|tejado|teulada|cubierta|\bgates?\b|"
    r"\bpuerta\b|\bportal\b|entrance|entrada|\bnorth|\bsouth|\beast|\bwest|\bnorte|\bsur\b|\boeste|"
    r"\bnord\b|\bmerlet|battlement|almena|gargoyle|g[aá]rgola", re.I)
CLOSEUP_RE = re.compile(
    r"detail|detalle|detall|gargoyle|g[aá]rgola|relieve|relief|escudo|\bescut\b|blas[oó]n|coat of arms|inscrip|plaque|"
    r"placa|sculpture|escultura|capital|capitel|knocker|aldaba|mosaic|azulejo|\btiles?\b|close.?up|"
    r"window|ventana|finestra|\bdoor\b|\bpuerta\b|\bporta\b|letrero|\bsign\b|clock|reloj|rellotge|"
    r"weather ?vane|veleta|lamp|farola|statue|estatua|portale?\b|portada|doorway|banderuola|cotorra|"
    r"parrot|\bloro\b|fl[eè]che|weathercock|penell|girouette|finial|pinn?acle|decoraci|decorat|ornament", re.I)
NIGHT_RE = re.compile(r"\bnight|\bnoche|\bnit\b|nocturn|illuminat|iluminad|il[·.]?luminad|by night|"
                      r"sunset|dusk|atardecer|ocaso|capvespre|posta de sol|blue hour", re.I)
HIST_RE = re.compile(r"\b1[89]\d\d\b|laurent|historic|hist[oó]ric|antigua|antic|old photo|"
                     r"postcard|postal|engraving|grabado", re.I)
MEDIA_SKIP = ("video/", "audio/", "application/ogg")


def plan_kind(text, desc=""):
    t = text.lower()
    if re.search(r"ficha|pgou|plan general|ordenaci[oó]n urbana|cat[aà]logo? de (?:bienes|edificios)", t + " " + desc.lower()):
        return "ficha"
    if re.search(r"secci[oó]n?|\bsections?\b|cross.section|corte (?:transversal|longitudinal)", t):
        return "section"
    if re.search(r"alzados?|al[çc]ats?|\belevations?\b", t):
        return "elevation"
    if re.search(r"\bplan(?:o|ol|s)?\b|\bplanta\b|pl[aà]nols?|floor ?plan|ground ?plan|levantamiento|blueprint", t):
        return "plan"
    if re.search(r"dibujo|drawing|croquis|sketch|grabado|engraving|litograf|lithograph|l[aá]mina|dibuix", t):
        return "drawing"
    return "other"


def resolve_category(ctx, wd, names):
    http = ctx["http"]
    cands = []
    if ctx["lm"].get("commons_category"):
        cands.append(ctx["lm"]["commons_category"])
    p373 = wd_string(wd, "P373")
    if p373:
        cands.append(p373)
    sl = wd.get("entity", {}).get("sitelinks", {}).get("commonswiki", {}).get("title", "")
    if sl.startswith("Category:"):
        cands.append(sl)
    for c in cands:
        title = c if c.startswith("Category:") else "Category:" + c
        d = commons_api(http, {"action": "query", "titles": title, "prop": "categoryinfo", "redirects": "1"})
        pages = d.get("query", {}).get("pages", [])
        if pages and not pages[0].get("missing"):
            return pages[0]["title"], "list/wikidata"
    for nm in names:
        if not nm:
            continue
        d = commons_api(http, {"action": "query", "list": "search", "srnamespace": "14",
                               "srsearch": nm, "srlimit": "5"})
        hits = d.get("query", {}).get("search", [])
        for h in hits:
            if names_match(h["title"].split(":", 1)[1], nm):
                return h["title"], "search"
    return None, ""


LIST_PROPS = {
    "prop": "imageinfo|categories|coordinates",
    "iiprop": "size|mime|extmetadata",
    "iiextmetadatafilter": "Artist|LicenseShortName|ImageDescription|Assessments|DateTimeOriginal|ObjectName",
    "iiextmetadatalanguage": "en",
    "cllimit": "max", "clshow": "!hidden",
    "coprimary": "all", "coprop": "type|dim", "colimit": "max",
}


def list_category_files(http, cat, limit_requests=12):
    pages, _ = commons_query_all(http, {"action": "query", "generator": "categorymembers",
                                        "gcmtitle": cat, "gcmtype": "file", "gcmlimit": "200",
                                        **LIST_PROPS}, max_requests=limit_requests)
    return pages


def list_subcats(http, cat):
    _, lst = commons_query_all(http, {"action": "query", "list": "categorymembers", "cmtitle": cat,
                                      "cmtype": "subcat", "cmlimit": "500"}, max_requests=3)
    return [x["title"] for x in lst if x.get("ns") == 14]


ALWAYS_SKIP_SUBCAT = re.compile(r"interior|inside|indoor|\bviews? from\b|vistas? desde|vistes des de|"
                                r"seen from|panoramas? from|people|\bevents?\b|collection|museum objects|"
                                r"replica|r[eé]plica|exposition|\bmaps?\b|\bmapas?\b", re.I)


def name_mask(main_cat, names) -> set:
    """Normalised words of the landmark's own names that would by themselves
    trigger the interior/skip vocabulary (e.g. 'Exposició' in Palau de
    l'Exposició, 'Museu' in a museum's name). They are blanked out before
    those rules run, so the building's name does not make every file of its
    category look like an interior."""
    text = " ".join([main_cat.split(":", 1)[-1] if main_cat else ""] + [n for n in names if n])
    return {t for t in name_tokens(text)
            if INTERIOR_RE.search(t) or SKIP_SUBCAT.search(t) or ALWAYS_SKIP_SUBCAT.search(t)}


def mask_words(text: str, mask: set) -> str:
    if not mask or not text:
        return text
    return re.sub(r"\w+", lambda m: " " if norm_text(m.group()) in mask else m.group(), text)


def subcat_ok(name, main_cat, names):
    """Depth-1 subcategory filter: skip interiors/people/events/collections and
    subcategories about something else (no shared name token, e.g. a
    neighbouring building filed under the landmark)."""
    masked = mask_words(name, name_mask(main_cat, names))
    if ALWAYS_SKIP_SUBCAT.search(masked):
        return False
    if SKIP_SUBCAT.search(masked) and not EXTERIOR_SUBCAT.search(name):
        return False
    toks = {t for t in name_tokens(" ".join([main_cat.split(":", 1)[-1] if main_cat else ""] + list(names)))
            if len(t) >= 4}
    if toks & {t for t in name_tokens(name) if len(t) >= 4}:
        return True
    return bool(EXTERIOR_SUBCAT.search(name) or COURTYARD_SUBCAT.search(name) or ASPECT_SUBCAT.search(name))


def apply_subcat_rules(doc, names):
    """Re-apply subcat_ok to a cached listing: files reached only through a
    now-rejected subcategory are marked class 'skip'."""
    cat = doc.get("category")
    bad = {s for s in doc.get("subcats_used", []) if not subcat_ok(s.split(":", 1)[1], cat, names)}
    if not bad:
        return
    for r in doc.get("files", []):
        if r["sources"] and all(s in bad for s in r["sources"]):
            r["class"] = "skip"


def step_commons(ctx, wd, names):
    raw, st = ctx["raw"], ctx["state"]
    path = os.path.join(raw, "commons", "files.json")
    wd_imgs = {}
    for pid, role in IMAGE_PROPS.items():
        for c in wd.get("entity", {}).get("claims", {}).get(pid, []):
            v = wd_snak_value(c["mainsnak"])
            if isinstance(v, str):
                wd_imgs.setdefault("File:" + v, role)
    review = ctx.get("review") or {}
    extra_cats = list(review.get("extra_categories", []))
    extra_files = list(review.get("extra_files", []))
    key = digest([VERSION, ctx["lm"].get("commons_category"), wd_string(wd, "P373"), sorted(wd_imgs.items())]
                 + ([extra_cats, extra_files] if extra_cats or extra_files else []))
    if st.done("commons", key, [path]) and not ctx["force"]("commons"):
        doc = L.read_json(path)
        for r in doc.get("files", []):  # classification is cheap: always use current rules
            classify(r, doc.get("category"), names)
        apply_subcat_rules(doc, names)
        apply_review(doc, review)
        return doc
    http = ctx["http"]
    warns = []
    cat, how = resolve_category(ctx, wd, names)
    files = {}

    def add_pages(pages, source):
        for p in pages:
            t = p.get("title")
            if not t or p.get("ns") != 6:
                continue
            if t in files:
                if source not in files[t]["sources"]:
                    files[t]["sources"].append(source)
                continue
            files[t] = {"page": p, "sources": [source]}

    subcats_used, subcats_skipped = [], []
    if cat:
        LOG(f"  commons category {cat} ({how})")
        add_pages(list_category_files(http, cat), cat)
        subs = list_subcats(http, cat)
        queue = [(s, 1) for s in subs]
        while queue and len(subcats_used) < 25:
            s, depth = queue.pop(0)
            name = s.split(":", 1)[1]
            if not subcat_ok(name, cat, names):
                subcats_skipped.append(s)
                continue
            subcats_used.append(s)
            LOG(f"  commons subcategory {s}")
            add_pages(list_category_files(http, s, limit_requests=6), s)
            # one extra level only below clearly exterior subcategories
            if depth == 1 and EXTERIOR_SUBCAT.search(name):
                queue.extend((x, 2) for x in list_subcats(http, s))
    else:
        warns.append("no Commons category found; using file search")
        for nm in names[:2]:
            if not nm:
                continue
            pages, _ = commons_query_all(http, {"action": "query", "generator": "search",
                                                "gsrnamespace": "6", "gsrsearch": f'"{nm}" valencia',
                                                "gsrlimit": "100", **LIST_PROPS}, max_requests=4)
            add_pages(pages, "search:" + nm)
    for xc in extra_cats:  # hand-picked extra categories (photo_review.toml)
        xc = xc if xc.startswith("Category:") else "Category:" + xc
        LOG(f"  commons extra category {xc}")
        add_pages(list_category_files(http, xc, limit_requests=6), xc)
    if extra_files:
        for i in range(0, len(extra_files), 50):
            pages, _ = commons_query_all(http, {"action": "query", "titles": "|".join(extra_files[i:i + 50]),
                                                **LIST_PROPS}, max_requests=3)
            add_pages(pages, "review")
    if wd_imgs:
        titles = list(wd_imgs)
        for i in range(0, len(titles), 50):
            pages, _ = commons_query_all(http, {"action": "query", "titles": "|".join(titles[i:i + 50]),
                                                **LIST_PROPS}, max_requests=3)
            add_pages(pages, "wikidata")
    out = []
    for t, f in files.items():
        p = f["page"]
        ii = (p.get("imageinfo") or [{}])[0]
        em = ii.get("extmetadata", {})

        def emv(k):
            return strip_html(em.get(k, {}).get("value", ""))

        cats = [c["title"].split(":", 1)[1] for c in p.get("categories", [])]
        cams = [c for c in p.get("coordinates", []) if c.get("type", "camera" if c.get("primary") else "") == "camera"]
        objs = [c for c in p.get("coordinates", []) if c.get("type") == "object"]
        rec = {
            "title": t, "pageid": p.get("pageid"), "mime": ii.get("mime", ""),
            "width": ii.get("width", 0), "height": ii.get("height", 0),
            "artist": emv("Artist"), "license": emv("LicenseShortName"),
            "description": emv("ImageDescription")[:600], "date": emv("DateTimeOriginal")[:80],
            "assessments": em.get("Assessments", {}).get("value", ""),
            "categories": cats, "sources": f["sources"], "wikidata_role": wd_imgs.get(t, ""),
            "camera": [cams[0]["lat"], cams[0]["lon"]] if cams else None,
            "object": [objs[0]["lat"], objs[0]["lon"]] if objs else None,
        }
        classify(rec, cat, names)
        out.append(rec)
    doc = {"category": cat, "category_how": how, "subcats_used": subcats_used,
           "subcats_skipped": subcats_skipped, "files": out, "warnings": warns}
    L.write_json(path, doc)
    apply_review(doc, review)
    st.warn("commons", warns)
    st.mark("commons", key, digest([cat, sorted(r["title"] for r in out)]))
    n_plan = sum(r["class"] == "plan" for r in out)
    n_int = sum(r["class"] == "photo" and r["exterior"] is False for r in out)
    LOG(f"  commons: {len(out)} files ({n_plan} plans/drawings, {n_int} interior-ish), "
        f"{len(subcats_used)} subcats used, {len(subcats_skipped)} skipped")
    return doc


def classify(rec, main_cat, names=()):
    title = rec["title"].split(":", 1)[1]
    base = re.sub(r"\.[a-z0-9]+$", "", title, flags=re.I)
    src_names = " ".join(s.split(":", 1)[-1] for s in rec["sources"])
    cats = " | ".join(rec["categories"])
    mask = name_mask(main_cat, names)
    m_base, m_cats = mask_words(base, mask), mask_words(cats, mask)
    strong = f"{base} | {cats} | {src_names}"
    weak = rec["description"]
    mime = rec["mime"]
    rec["class"] = "photo"
    rec["plan_kind"] = ""
    if any(mime.startswith(m) for m in MEDIA_SKIP):
        rec["class"] = "skip"
    elif MAP_RE.search(base) or (MAP_RE.search(cats) and not PLAN_RE.search(base)):
        rec["class"] = "map"
    elif rec["wikidata_role"] == "plan" or mime in ("image/svg+xml", "application/pdf", "image/vnd.djvu") or (
            not rec.get("camera") and (
                PLAN_STRONG_RE.search(base) or PLAN_RE.search(cats) or
                (PLAN_WEAK_RE.search(base) and mime not in ("image/jpeg",)))):
        rec["class"] = "plan"
        rec["plan_kind"] = "plan" if rec["wikidata_role"] == "plan" else plan_kind(f"{base} | {cats}", weak)
    ext_src = any(EXTERIOR_SUBCAT.search(s.split(":", 1)[-1]) for s in rec["sources"])
    court_src = any(COURTYARD_SUBCAT.search(s.split(":", 1)[-1]) for s in rec["sources"])
    aspect_src = any(ASPECT_EXT_SUBCAT.search(s.split(":", 1)[-1]) for s in rec["sources"])
    int_src = any(re.search(r"interior|inside", s, re.I) for s in rec["sources"])
    if int_src or INTERIOR_RE.search(m_base) or INTERIOR_RE.search(m_cats):
        ext = False
    elif ext_src or court_src or aspect_src or EXTERIOR_RE.search(base) or rec["wikidata_role"] in ("image", "aerial", "night",
                                                                           "panorama", "view", "winter"):
        ext = True
    elif INTERIOR_RE.search(mask_words(weak, mask)):
        ext = False
    elif EXTERIOR_RE.search(weak) or EXTERIOR_RE.search(cats):
        ext = True
    else:
        ext = "unknown"
    if rec["wikidata_role"] == "interior":
        ext = False
    rec["exterior"] = ext
    rec["closeup"] = bool(CLOSEUP_RE.search(base) or CLOSEUP_RE.search(src_names))
    rec["night"] = bool(NIGHT_RE.search(strong) or NIGHT_RE.search(weak))
    rec["historic"] = bool(HIST_RE.search(base) or HIST_RE.search(cats) or re.match(r"^1[89]\d\d", rec["date"] or ""))
    rec["main_cat"] = main_cat in rec["sources"] if main_cat else False
    rec["ext_src"] = ext_src
    rec["courtyard"] = court_src and not ext_src
    rec["vegetation"] = bool(VEG_RE.search(base))


def class_digest(com):
    return digest([(r["title"], r["class"], r["plan_kind"], r["exterior"], r["closeup"], r["night"],
                    r["historic"], r.get("courtyard"), r.get("vegetation"), r.get("reviewed"))
                   for r in com.get("files", [])])


def apply_review(doc, review):
    """Hand review from photo_review.toml (looked at on the contact sheets):
    'exterior' titles are confirmed exterior views of the building, 'exclude'
    titles are interiors, objects, people or another building and are never
    selected. It runs after classify, so it wins over the text rules."""
    if not review:
        return
    ext = set(review.get("exterior", []))
    exc = set(review.get("exclude", []))
    for r in doc.get("files", []):
        if r["title"] in exc:
            r["exterior"], r["reviewed"] = False, "exclude"
        elif r["title"] in ext:
            r["exterior"], r["reviewed"] = True, "exterior"
            r["closeup"] = False


def ll_dist(a, b):
    ax, az = L.ll_to_city(*a)
    bx, bz = L.ll_to_city(*b)
    return math.hypot(ax - bx, az - bz)


def photo_score(r, anchor_ll, polys, anchor):
    s = 0.0
    a = (r["assessments"] or "").lower()
    if "featured" in a:
        s += 40
    if "quality" in a:
        s += 30
    if "valued" in a:
        s += 20
    if r["wikidata_role"] in ("image", "aerial", "view"):
        s += 25
    if r["exterior"] is True:
        s += 15
    if r.get("reviewed") == "exterior":
        s += 25
    if r["ext_src"]:
        s += 6
    elif r["main_cat"]:
        s += 4
    mp = (r["width"] or 0) * (r["height"] or 0) / 1e6
    s += min(15.0, mp * 2.0)
    if max(r["width"] or 0, r["height"] or 0) < 1000:
        s -= 20
    if (r["width"] or 1) / max(1, r["height"] or 1) > 2.6:
        s -= 10
    if r["closeup"]:
        s -= 15
    if r.get("courtyard"):
        s -= 8
    if r.get("vegetation"):
        s -= 12
    if r["night"]:
        s -= 10
    if r["historic"]:
        s -= 14
    r["cam_dist"] = None
    if r.get("camera") and anchor_ll:
        d = ll_dist(r["camera"], anchor_ll)
        r["cam_dist"] = round(d, 1)
        if d > 700:
            s -= 30
        else:
            s += 8
            if d > 350:
                s -= 8
        cx, cz = L.ll_to_city(*r["camera"])
        inside = any(L.point_in_polygon(cx, cz, o, i) for o, i in polys)
        r["cam_inside"] = inside
        if inside and r["exterior"] != True:
            s -= 20
        elif polys and not inside:
            edge = min(L.distance_point_polygon(cx, cz, o, i) for o, i in polys)
            if edge < 4:
                s -= 6  # right next to a wall: probably a close-up
    if r.get("object") and anchor_ll and ll_dist(r["object"], anchor_ll) > 300:
        s -= 40
    return s


def select_photos(files, anchor, polys, max_n):
    anchor_ll = L.city_to_ll(*anchor) if anchor else None
    cands = []
    for r in files:
        if r["class"] != "photo" or r["exterior"] is False:
            continue
        if not r["mime"].startswith("image/") or r["mime"] in ("image/svg+xml",):
            continue
        r["score"] = round(photo_score(r, anchor_ll, polys, anchor), 2)
        cands.append(r)
    cands.sort(key=lambda r: -r["score"])
    chosen = []
    sectors, authors, stems, subcats = {}, {}, {}, {}
    pool = cands[:]

    def subcat_of(r):
        subs = [x for x in r["sources"] if x.startswith("Category:") and not r.get("main_cat_name") == x]
        if r["main_cat"] or not subs:
            return None
        return subs[0]
    while pool and len(chosen) < max_n:
        best, best_s = None, -1e9
        for r in pool:
            s = r["score"]
            sec = None
            if r.get("camera") and anchor and r.get("cam_dist") and r["cam_dist"] > 5:
                cx, cz = L.ll_to_city(*r["camera"])
                ang = math.degrees(math.atan2(cx - anchor[0], -(cz - anchor[1]))) % 360
                sec = int(((ang + 22.5) % 360) // 45)
                s -= 9 * sectors.get(sec, 0)
            au = r["artist"][:40]
            s -= 6 * max(0, authors.get(au, 0) - 1)
            stem = re.sub(r"[\d_\-\s().]+", " ", r["title"].lower()).strip()[:30]
            s -= 8 * stems.get(stem, 0)
            sc = subcat_of(r)
            if sc and not EXTERIOR_SUBCAT.search(sc):
                s -= 8 * max(0, subcats.get(sc, 0) - 1)  # e.g. 'Domes of ...': at most a couple
            if s > best_s:
                best, best_s = (r, sec, au, stem), s
        r, sec, au, stem = best
        sc = subcat_of(r)
        if sc:
            subcats[sc] = subcats.get(sc, 0) + 1
        pool.remove(r)
        chosen.append(r)
        if sec is not None:
            sectors[sec] = sectors.get(sec, 0) + 1
        authors[au] = authors.get(au, 0) + 1
        stems[stem] = stems.get(stem, 0) + 1
    return chosen


def safe_name(title, maxlen=60):
    base = title.split(":", 1)[-1]
    base = re.sub(r"\.[A-Za-z0-9]+$", "", base)
    base = unicodedata.normalize("NFKD", base)
    base = "".join(c for c in base if not unicodedata.combining(c))
    base = re.sub(r"[^A-Za-z0-9]+", "_", base).strip("_")
    return base[:maxlen] or "file"


def thumb_width(target, orig_w, mime):
    std = [w for w in STANDARD_WIDTHS if w <= target]
    if mime in ("image/svg+xml",):
        return std[-1]
    cands = [w for w in std if w < (orig_w or 0)]
    return cands[-1] if cands else None


def download_thumb(ctx, title, width):
    """Special:FilePath/<name>?width=W -> bytes. The redirect is followed by
    hand so we never fetch an unscaled original (those return HTTP 429)."""
    name = title.split(":", 1)[1]
    url = COMMONS_FILEPATH + urllib.parse.quote(name.replace(" ", "_")) + f"?width={width}"
    http = ctx["http"]
    target = url
    for _ in range(4):
        code, hdrs, body, loc = http.request(target, redirect=False)
        if code in (301, 302, 303, 307, 308):
            loc = urllib.parse.urljoin(target, loc)
            if "/thumb/" not in loc and "thumb.wikimedia.org" not in loc and "upload.wikimedia.org" in loc:
                raise RuntimeError("would fetch unscaled original")
            target = loc
            continue
        return body, target
    raise RuntimeError("too many redirects")


def save_image(data, path_noext, force_jpeg=True):
    im = Image.open(io.BytesIO(data))
    im.load()
    fmt = (im.format or "").upper()
    if force_jpeg:
        path = path_noext + ".jpg"
        if fmt == "JPEG":
            L.write_bytes(path, data)
        else:
            if im.mode in ("RGBA", "LA", "P"):
                im = im.convert("RGBA")
                bg = Image.new("RGB", im.size, (255, 255, 255))
                bg.paste(im, mask=im.split()[-1])
                im = bg
            else:
                im = im.convert("RGB")
            buf = io.BytesIO()
            im.save(buf, "JPEG", quality=90)
            L.write_bytes(path, buf.getvalue())
    else:
        ext = {"JPEG": ".jpg", "PNG": ".png", "GIF": ".gif", "WEBP": ".webp"}.get(fmt, ".png")
        path = path_noext + ext
        if ext in (".gif", ".webp"):
            path = path_noext + ".png"
            buf = io.BytesIO()
            im.convert("RGBA").save(buf, "PNG")
            data = buf.getvalue()
        L.write_bytes(path, data)
    return path, im.width, im.height


HEADING_RE = re.compile(r"\{\{\s*(?:location|camera location|location dec|location-panorama)\b[^{}]*?"
                        r"heading\s*:\s*([0-9.]+|[nsew]{1,3})", re.I)
COMPASS = {d: i * 22.5 for i, d in enumerate(
    "N NNE NE ENE E ESE SE SSE S SSW SW WSW W WNW NW NNW".split())}


def fetch_headings(ctx, titles):
    out = {}
    for i in range(0, len(titles), 40):
        chunk = titles[i:i + 40]
        try:
            d = commons_api(ctx["http"], {"action": "query", "titles": "|".join(chunk), "prop": "revisions",
                                          "rvprop": "content", "rvslots": "main"})
        except Exception as ex:  # noqa: BLE001
            LOG(f"  WARN headings: {ex}")
            continue
        for p in d.get("query", {}).get("pages", []):
            revs = p.get("revisions") or []
            if not revs:
                continue
            text = revs[0].get("slots", {}).get("main", {}).get("content", "")
            m = HEADING_RE.search(text)
            if m:
                h = m.group(1).upper()
                try:
                    out[p["title"]] = float(h) % 360 if h[0].isdigit() else COMPASS.get(h)
                except ValueError:
                    pass
    return out


def step_photos(ctx, com, fp):
    raw, st = ctx["raw"], ctx["state"]
    meta_path = os.path.join(raw, "photos", "photos.json")
    anchor = tuple(fp["anchor"]) if fp else None
    review = ctx.get("review") or {}
    key = digest([VERSION, RULES_VERSION, st.out("commons"), anchor, ctx["max_photos"], st.out("catastro"),
                  class_digest(com)] + ([review.get("catastro_not_building")]
                                        if review.get("catastro_not_building") else []))
    if st.done("photos", key, [meta_path]) and not ctx["force"]("photos"):
        return L.read_json(meta_path)
    polys = []
    if fp:
        a = fp["anchor"]
        polys = [([(x + a[0], z + a[1]) for x, z in o["outer"]],
                  [[(x + a[0], z + a[1]) for x, z in r] for r in o["inners"]]) for o in fp["outlines"]]
    ranked = select_photos(com.get("files", []), anchor, polys, ctx["max_photos"] + 12)
    photos, warns = [], []
    pdir = os.path.join(raw, "photos")
    os.makedirs(pdir, exist_ok=True)
    # unique, stable file stems per title
    uniq, used = {}, set()
    for r in ranked:
        base = safe_name(r["title"])
        if not re.search(r"\.jpe?g$", r["title"], re.I):
            base += "_" + r["title"].rsplit(".", 1)[-1].lower()[:4]
        n, k = base, 2
        while n in used:
            n, k = f"{base}_{k}", k + 1
        used.add(n)
        uniq[r["title"]] = n
    # earlier downloads (any index) are reused: NN_<stem>.jpg or _dl_<stem>.jpg
    existing = {}
    for fn in os.listdir(pdir):
        m = re.match(r"^(?:\d\d_|_dl_)(.+)\.jpg$", fn)
        if m:
            existing[m.group(1)] = fn
    got = []
    for r in ranked:
        if len(got) >= ctx["max_photos"]:
            break
        u = uniq[r["title"]]
        if u in existing and not ctx["force"]("images"):
            got.append((r, existing[u]))
            continue
        w = thumb_width(1280, r["width"], r["mime"]) or 960
        try:
            LOG(f"  photo {len(got) + 1:02d} {r['title']} (w={w})")
            data, _ = download_thumb(ctx, r["title"], w)
            path, _, _ = save_image(data, os.path.join(pdir, "_dl_" + u))
            got.append((r, os.path.basename(path)))
        except Exception as ex:  # noqa: BLE001
            if isinstance(ex, OSError) and ex.errno == errno.ENOSPC:
                raise  # disk full: fail the landmark instead of marking the step done
            warns.append(f"download failed {r['title']}: {ex}")
    # number them in rank order (two-phase rename so names never collide)
    moves = []
    for i, (r, fn) in enumerate(got, start=1):
        final = f"{i:02d}_{uniq[r['title']]}.jpg"
        if fn != final:
            tmp = f".mv_{final}"
            os.replace(os.path.join(pdir, fn), os.path.join(pdir, tmp))
            moves.append((tmp, final))
    for tmp, final in moves:
        os.replace(os.path.join(pdir, tmp), os.path.join(pdir, final))
    chosen = [r for r, _ in got]
    headings = fetch_headings(ctx, [r["title"] for r in chosen if r.get("camera")]) if chosen else {}
    for i, r in enumerate(chosen, start=1):
        path = os.path.join(pdir, f"{i:02d}_{uniq[r['title']]}.jpg")
        with Image.open(path) as im:
            iw, ih = im.size
        cam = None
        if r.get("camera") and anchor:
            cx, cz = L.ll_to_city(*r["camera"])
            cam = [round(cx - anchor[0], 2), round(cz - anchor[1], 2)]
        photos.append({
            "file": os.path.relpath(path, raw), "title": r["title"],
            "url": "https://commons.wikimedia.org/wiki/" + urllib.parse.quote(r["title"].replace(" ", "_"), safe=":/"),
            "author": r["artist"], "license": r["license"], "width": iw, "height": ih,
            "camera": cam, "camera_ll": r.get("camera"), "heading": headings.get(r["title"]),
            "exterior": r["exterior"], "score": r["score"], "description": r["description"][:300],
            "date": r["date"], "assessments": r["assessments"], "categories": r["categories"][:12],
            "source": "commons",
        })
    # remove downloads that are no longer selected
    keep = {os.path.basename(p["file"]) for p in photos}
    for fn in os.listdir(pdir):
        if re.match(r"^(?:\d\d_|_dl_|\.mv_).*\.jpg$", fn) and fn not in keep:
            os.remove(os.path.join(pdir, fn))
    cat = L.read_json(os.path.join(raw, "catastro", "catastro.json")) \
        if os.path.exists(os.path.join(raw, "catastro", "catastro.json")) else {}
    bad_fac = set(review.get("catastro_not_building", []))
    for fph in cat.get("facade_photos", []):
        if fph["refcat"] in bad_fac:  # reviewed: the parcel's facade photo shows another building
            continue
        photos.append({
            "file": fph["file"], "title": f"Catastro fachada {fph['refcat']}", "url": fph["url"],
            "author": "Dirección General del Catastro", "license": "Catastro open data (attribution required)",
            "width": fph["width"], "height": fph["height"], "camera": None, "camera_ll": None, "heading": None,
            "exterior": True, "score": 0, "description": f"Cadastral facade photo of parcel {fph['refcat']}",
            "date": "", "assessments": "", "categories": [], "source": "catastro"})
    doc = {"photos": photos, "warnings": warns}
    L.write_json(meta_path, doc)
    st.warn("photos", warns)
    st.mark("photos", key, digest([(p["file"], p["title"]) for p in photos]))
    LOG(f"  photos: {len(photos)} ({sum(p['exterior'] is True for p in photos)} likely exterior)")
    return doc


def step_plans(ctx, com, cat):
    raw, st = ctx["raw"], ctx["state"]
    meta_path = os.path.join(raw, "plans", "plans.json")
    key = digest([VERSION, RULES_VERSION, st.out("commons"), st.out("catastro"), ctx["max_plans"],
                  class_digest(com)])
    if st.done("plans", key, [meta_path]) and not ctx["force"]("plans"):
        return L.read_json(meta_path)
    prio = {"plan": 30, "elevation": 25, "section": 25, "ficha": 15, "drawing": 10, "other": 5}
    cands = [r for r in com.get("files", []) if r["class"] == "plan"]
    for r in cands:
        s = prio.get(r["plan_kind"], 0) + (30 if r["wikidata_role"] == "plan" else 0)
        s += min(10.0, (r["width"] or 0) * (r["height"] or 0) / 1e6 * 2)
        if r["exterior"] is False and r["plan_kind"] == "drawing":
            s -= 5
        r["score"] = s
    cands.sort(key=lambda r: -r["score"])
    plans, warns = [], []
    pdir = os.path.join(raw, "plans")
    os.makedirs(pdir, exist_ok=True)
    idx = 0
    for r in cands:
        if idx >= ctx["max_plans"]:
            break
        idx += 1
        stem = os.path.join(pdir, f"{idx:02d}_{safe_name(r['title'])}")
        existing = [stem + e for e in (".jpg", ".png") if os.path.exists(stem + e)]
        if existing and not ctx["force"]("images"):
            path = existing[0]
        else:
            w = thumb_width(1920, r["width"], r["mime"])
            if r["mime"] in ("application/pdf", "image/vnd.djvu") and w is None:
                w = 960
            if w is None:
                warns.append(f"plan too small {r['title']}")
                idx -= 1
                continue
            try:
                LOG(f"  plan {idx:02d} {r['title']} (w={w})")
                data, _ = download_thumb(ctx, r["title"], w)
                path, _, _ = save_image(data, stem, force_jpeg=False)
            except Exception as ex:  # noqa: BLE001
                if isinstance(ex, OSError) and ex.errno == errno.ENOSPC:
                    raise  # disk full: fail the landmark instead of marking the step done
                warns.append(f"download failed {r['title']}: {ex}")
                idx -= 1
                continue
        with Image.open(path) as im:
            iw, ih = im.size
        plans.append({"file": os.path.relpath(path, raw), "title": r["title"],
                      "url": "https://commons.wikimedia.org/wiki/" + urllib.parse.quote(
                          r["title"].replace(" ", "_"), safe=":/"),
                      "author": r["artist"], "license": r["license"], "kind": r["plan_kind"] or "other",
                      "notes": (r["description"][:200] + (f" [{iw}x{ih}px]")).strip(),
                      "width": iw, "height": ih})
    keep = {os.path.basename(p["file"]) for p in plans}
    for fn in os.listdir(pdir):
        if re.match(r"^\d\d_.*\.(jpg|png)$", fn) and fn not in keep:
            os.remove(os.path.join(pdir, fn))
    if cat.get("wms"):
        w = cat["wms"]
        rcs = ", ".join(b["refcat"] for b in cat.get("buildings", [])[:4])
        plans.append({"file": w["file"], "title": "Catastro WMS cadastral map",
                      "url": CATASTRO_WMS + "?SERVICE=WMS&REQUEST=GetMap&LAYERS=Catastro&SRS=EPSG:25830&BBOX="
                      + ",".join("%.2f" % v for v in w["bbox_utm"]),
                      "author": "Dirección General del Catastro", "license": "Catastro open data (attribution required)",
                      "kind": "catastro",
                      "notes": (f"Cadastral map, EPSG:25830 bbox {[round(v, 2) for v in w['bbox_utm']]}, "
                                f"{w['m_per_px']:.3f} m/px, north up; roman numerals = floors above ground"
                                + (f"; parcels {rcs}" if rcs else "")),
                      "width": w["width"], "height": w["height"]})
    doc = {"plans": plans, "warnings": warns}
    L.write_json(meta_path, doc)
    st.warn("plans", warns)
    st.mark("plans", key, digest([(p["file"], p["title"]) for p in plans]))
    LOG(f"  plans: {len(plans)}")
    return doc


FONT_FILES = ["/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
              "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
              "/usr/share/fonts/truetype/freefont/FreeSans.ttf"]


def font(size):
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


def step_contact(ctx, photos):
    raw, st = ctx["raw"], ctx["state"]
    path = os.path.join(raw, "photos", "contact.jpg")
    key = digest([VERSION, st.out("photos")])
    if st.done("contact", key, [path]) and not ctx["force"]("contact"):
        return path
    items = photos.get("photos", [])
    if not items:
        st.mark("contact", key, "")
        return None
    cols = 4
    cw, ch, lh = 400, 300, 40
    rows = math.ceil(len(items) / cols)
    sheet = Image.new("RGB", (cols * cw, rows * (ch + lh)), (24, 24, 24))
    d = ImageDraw.Draw(sheet)
    f1, f2 = font(17), font(13)
    for i, p in enumerate(items):
        c, r = i % cols, i // cols
        x, y = c * cw, r * (ch + lh)
        try:
            with Image.open(os.path.join(raw, p["file"])) as im:
                im = im.convert("RGB")
                im.thumbnail((cw - 8, ch - 8))
                sheet.paste(im, (x + (cw - im.width) // 2, y + (ch - im.height) // 2))
        except Exception:  # noqa: BLE001
            d.text((x + 10, y + 10), "unreadable", fill=(255, 80, 80), font=f1)
        label = os.path.basename(p["file"]).split("_", 1)[0]
        num = label if label.isdigit() else "C"
        ext = {True: "ext", False: "int"}.get(p["exterior"], "?")
        title = p["title"].split(":", 1)[-1]
        title = re.sub(r"\.[A-Za-z]+$", "", title)
        cam = " cam" if p.get("camera") else ""
        hd = f" h{int(p['heading'])}" if p.get("heading") is not None else ""
        d.rectangle([x, y + ch, x + cw - 1, y + ch + lh - 1], fill=(0, 0, 0))
        d.text((x + 6, y + ch + 2), f"{num} [{ext}{cam}{hd}]", fill=(255, 220, 90), font=f1)
        d.text((x + 6, y + ch + 22), title[:52], fill=(230, 230, 230), font=f2)
        d.rectangle([x, y, x + 34, y + 24], fill=(0, 0, 0))
        d.text((x + 4, y + 3), num, fill=(255, 220, 90), font=f1)
    sheet.save(path, "JPEG", quality=85)
    st.mark("contact", key, file_digest(path))
    LOG(f"  contact sheet: {len(items)} photos")
    return path


# ---------------------------------------------------------------------------
# sources.toml
# ---------------------------------------------------------------------------

GENERATED_FACT_SOURCES = ("https://www.wikidata.org/", "Catastro", "http://ovc.catastro", "https://www1.sedecatastro")


def build_sources(ctx, wd, sel, cat, com, photos, plans, fp):
    lm = ctx["lm"]
    qid = (lm.get("wikidata") or "").strip()
    doc = {"slug": lm["slug"], "name": lm.get("name", ""), "name_va": lm.get("name_va", ""),
           "wikidata": qid, "commons_category": (com.get("category") or "").split(":", 1)[-1],
           "wikipedia": wd_sitelinks(wd)}
    facts = wd_facts(wd)
    blds = cat.get("buildings", [])
    if blds:
        b = blds[0]
        src = CATASTRO_SEDE.format(b["refcat"][:7], b["refcat"][7:14])
        facts["catastro_refcat"] = {"value": [x["refcat"] for x in blds] if len(blds) > 1 else b["refcat"],
                                    "source": src}
        if b.get("built"):
            facts["catastro_built"] = {"value": int(b["built"]), "source": src,
                                       "note": "Catastro dateOfConstruction (often the last major works)"}
        if b.get("use"):
            facts["catastro_use"] = {"value": b["use"], "source": src}
        if b.get("area"):
            facts["catastro_floor_area"] = {"value": float(b["area"]), "unit": "m2", "source": src,
                                            "note": "Catastro officialArea (gross floor area)"}
    floors = [p["floors_above"] for p in cat.get("parts", []) if p.get("floors_above") is not None]
    if floors:
        facts["catastro_floors_max"] = {"value": max(floors), "source": "Catastro INSPIRE BU:BuildingPart",
                                        "note": f"max numberOfFloorsAboveGround over {len(floors)} parts"}
    doc["facts"] = facts
    doc["photo"] = []
    for p in photos.get("photos", []):
        e = {"file": p["file"], "title": p["title"] if p["source"] != "commons" else p["title"],
             "url": p["url"], "author": p["author"], "license": p["license"],
             "width": p["width"], "height": p["height"]}
        if p.get("camera"):
            e["camera"] = p["camera"]
        if p.get("heading") is not None:
            e["heading"] = round(float(p["heading"]), 1)
        e["exterior"] = p["exterior"]
        e["selected"] = False
        e["side"] = ""
        doc["photo"].append(e)
    doc["plan"] = [{"file": p["file"], "title": p["title"], "url": p["url"], "author": p["author"],
                    "license": p["license"], "kind": p["kind"], "notes": p["notes"]}
                   for p in plans.get("plans", [])]
    refs = []
    if qid:
        refs.append({"url": f"https://www.wikidata.org/wiki/{qid}", "title": f"Wikidata {qid}", "notes": ""})
    for lang, url in doc["wikipedia"].items():
        refs.append({"url": url, "title": f"Wikipedia ({lang})", "notes": ""})
    if com.get("category"):
        refs.append({"url": "https://commons.wikimedia.org/wiki/" + urllib.parse.quote(
            com["category"].replace(" ", "_"), safe=":/"), "title": com["category"],
            "notes": "Commons category (photos listed above are a ranked subset)"})
    for o in sel.get("outlines", []):
        t, i = o["id"].split(":", 1)[1].split("/")
        refs.append({"url": f"https://www.openstreetmap.org/{t}/{i}", "title": f"OSM {t} {i} (outline)",
                     "notes": f"selection method: {sel.get('method', '')}"})
    for b in blds:
        refs.append({"url": CATASTRO_SEDE.format(b["refcat"][:7], b["refcat"][7:14]),
                     "title": f"Catastro parcel {b['refcat']}",
                     "notes": "; ".join(f"{k}={b[k]}" for k in ("built", "use", "area", "area_in_outline") if b.get(k))})
    for pid, label in (("P856", "Official website"), ("P973", "Described at")):
        for c in wd.get("entity", {}).get("claims", {}).get(pid, []):
            v = wd_snak_value(c["mainsnak"])
            if isinstance(v, str):
                refs.append({"url": v, "title": label, "notes": f"Wikidata {pid}"})
    doc["reference"] = refs
    return doc


def merge_sources(new, old, base=None):
    """Merge a freshly generated sources doc into the existing sources.toml.

    3-way when `base` (what the tool generated last time) is known: any value
    in `old` that differs from `base` was edited by hand and wins; entries
    added by hand (photos, plans, references, facts, top-level keys) are kept.
    Without a base: keep selected/side flags, unknown keys and non-generated
    facts. Deleting a generated entry by hand is not remembered."""
    if not old:
        return new
    b = base or {}
    out = dict(new)
    arrays = ("photo", "plan", "reference")
    for k, v in old.items():
        if k in arrays or k == "facts":
            continue
        if k not in out or (base is not None and k in b and v != b[k]) or (base is not None and k not in b):
            out[k] = v
    facts = dict(new.get("facts", {}))
    bf = b.get("facts", {})
    for k, v in old.get("facts", {}).items():
        if base is not None:
            if k not in bf or bf[k] != v:
                facts[k] = v  # added or edited by hand
        else:
            src = v.get("source", "") if isinstance(v, dict) else ""
            if k not in facts and not str(src).startswith(GENERATED_FACT_SOURCES):
                facts[k] = v
    out["facts"] = facts

    def keyf(x):
        return x.get("title") or x.get("file") or x.get("url")

    def merge_list(name, always_keep=("selected", "side")):
        old_items = {keyf(x): x for x in old.get(name, []) if isinstance(x, dict)}
        base_items = {keyf(x): x for x in b.get(name, []) if isinstance(x, dict)}
        res, seen = [], set()
        for x in new.get(name, []):
            k = keyf(x)
            seen.add(k)
            o = old_items.get(k)
            if not o:
                res.append(x)
                continue
            y = dict(x)
            bo = base_items.get(k)
            for kk, vv in o.items():
                if base is not None and bo is not None:
                    if bo.get(kk, None) != vv:
                        y[kk] = vv
                elif kk in always_keep or kk not in x:
                    y[kk] = vv
            res.append(y)
        for k, o in old_items.items():
            if k in seen:
                continue
            bo = base_items.get(k)
            if base is not None:
                if bo is None or bo != o or o.get("selected"):
                    res.append(o)  # added/edited by hand, or chosen by a modeler
            elif not str(o.get("file", "")).startswith(("photos/", "plans/")) or o.get("selected"):
                res.append(o)
        out[name] = res

    merge_list("photo")
    merge_list("plan")
    merge_list("reference")
    return out


def step_sources(ctx, wd, sel, cat, com, photos, plans, fp):
    st = ctx["state"]
    path = os.path.join(ctx["data"], "sources.toml")
    key = digest([VERSION, st.out("wikidata"), st.out("osm"), st.out("catastro"), st.out("commons"),
                  st.out("photos"), st.out("plans")])
    if st.done("sources", key, [path]) and not ctx["force"]("sources"):
        return
    new = build_sources(ctx, wd, sel, cat, com, photos, plans, fp)
    old = None
    if os.path.exists(path):
        try:
            with open(path, "rb") as f:
                old = tomllib.load(f)
        except Exception as ex:  # noqa: BLE001
            LOG(f"  WARN existing sources.toml unreadable ({ex}); backing up")
            os.replace(path, path + ".bak")
    base_path = os.path.join(ctx["raw"], "sources.generated.json")
    base = None
    if old is not None and os.path.exists(base_path):
        try:
            base = L.read_json(base_path)
        except Exception:  # noqa: BLE001
            base = None
    doc = merge_sources(new, old, base)
    L.write_json(base_path, json.loads(json.dumps(new, default=float)))
    header = (f"Sources for landmark '{doc['slug']}'. Generated by engine/tools/landmarks/fetch.py; "
              "re-running it\nmerges (keeps selected/side flags, extra facts, photos and references added by hand).\n"
              "photo/plan 'file' paths are relative to engine/assets/downloaded/landmarks/valencia/<slug>/ "
              "(not committed).\ncamera = [x, z] landmark-local metres (city frame minus footprint anchor); "
              "heading = degrees clockwise\nfrom TRUE north as given on Commons "
              f"(city-frame grid azimuth from -z = heading - {L.CONVERGENCE_DEG:.2f}).")
    text = L.toml_dumps(doc, header)
    tomllib.loads(text)  # validate
    os.makedirs(ctx["data"], exist_ok=True)
    with open(path + ".tmp", "w", encoding="utf-8") as f:
        f.write(text)
    os.replace(path + ".tmp", path)
    st.mark("sources", key, file_digest(path))
    LOG(f"  sources.toml: {len(doc['facts'])} facts, {len(doc['photo'])} photos, {len(doc['plan'])} plans")


def step_overlay(ctx):
    st = ctx["state"]
    fp_path = os.path.join(ctx["data"], "footprint.json")
    src_path = os.path.join(ctx["data"], "sources.toml")
    out = os.path.join(ctx["raw"], "overlay.png")
    if not os.path.exists(fp_path):
        return None
    key = digest([VERSION, file_digest(fp_path), file_digest(src_path) if os.path.exists(src_path) else ""])
    if st.done("overlay", key, [out]) and not ctx["force"]("overlay"):
        return out
    import overlay  # noqa: E402  (tool dir is on sys.path)
    overlay.render(fp_path, src_path if os.path.exists(src_path) else None, out)
    st.mark("overlay", key, file_digest(out))
    return out


# ---------------------------------------------------------------------------
# driver
# ---------------------------------------------------------------------------

def nominatim_point(http, name):
    d = http.get_json(NOMINATIM + "/search", {"q": f"{name}, València", "format": "jsonv2", "limit": "1",
                                              "viewbox": "-0.45,39.52,-0.30,39.42", "bounded": "1"})
    if d:
        return float(d[0]["lat"]), float(d[0]["lon"])
    return None


def process(lm, args, http):
    slug = lm["slug"]
    raw = os.path.join(args.raw_dir, slug)
    data = os.path.join(args.data_dir, slug)
    os.makedirs(raw, exist_ok=True)
    st = State(os.path.join(raw, "state.json"))
    forced = args.force_steps

    def force(step):
        return "all" in forced or step in forced

    review = args.review.get(slug, {})
    ctx = {"lm": lm, "raw": raw, "data": data, "state": st, "http": http, "force": force,
           "max_photos": int(review.get("max_photos", args.max_photos)), "max_plans": args.max_plans,
           "review": review}
    st.data["inputs"] = {k: lm.get(k) for k in ("rank", "slug", "name", "name_va", "wikidata",
                                                 "commons_category", "lat", "lon", "osm")}
    st.save()
    LOG(f"[{slug}] {lm.get('name', '')}")
    wd = step_wikidata(ctx)
    point = None
    if lm.get("lat") is not None and lm.get("lon") is not None:
        point = (float(lm["lat"]), float(lm["lon"]))
    elif wd_coord(wd):
        point = wd_coord(wd)
    else:
        try:
            point = nominatim_point(http, lm.get("name_va") or lm.get("name"))
        except Exception as ex:  # noqa: BLE001
            LOG(f"  WARN nominatim: {ex}")
    wc = wd_coord(wd)
    in_warns = []
    if point and wc and ll_dist(point, wc) > 150:
        in_warns.append(f"list point and Wikidata P625 differ by {ll_dist(point, wc):.0f} m")
    if not point:
        in_warns.append("no coordinates (list, Wikidata P625, Nominatim)")
    for w in in_warns:
        LOG("  WARN", w)
    st.warn("input", in_warns)
    names = [lm.get("name", ""), lm.get("name_va", "")] + wd_label_names(wd)
    names = [n for i, n in enumerate(names) if n and n not in names[:i]]
    sel = step_osm(ctx, point, names)
    cat = step_catastro(ctx, sel, point)
    fp = step_footprint(ctx, sel, cat)
    try:
        com = step_commons(ctx, wd, names)
    except Exception as ex:  # noqa: BLE001
        LOG(f"  ERROR commons: {ex}")
        st.warn("commons", [f"commons failed: {ex}"])
        com = {"files": [], "category": None}
    photos = step_photos(ctx, com, fp)
    plans = step_plans(ctx, com, cat)
    step_contact(ctx, photos)
    step_sources(ctx, wd, sel, cat, com, photos, plans, fp)
    try:
        step_overlay(ctx)
    except Exception as ex:  # noqa: BLE001
        LOG(f"  WARN overlay: {ex}")
    row = {
        "slug": slug,
        "outline": ",".join(o["id"] for o in (fp or {}).get("outlines", [])[:3]) or "-",
        "method": sel.get("method") or ("catastro" if cat.get("fallback_outline") else "-"),
        "osm_parts": len((fp or {}).get("parts", [])),
        "catastro_parts": len((fp or {}).get("catastro", [])),
        "context": len((fp or {}).get("context", [])),
        "photos": len(photos.get("photos", [])),
        "exterior_photos": sum(1 for p in photos.get("photos", []) if p["exterior"] is True),
        "plans": len(plans.get("plans", [])),
        "warnings": " | ".join(st.all_warnings())[:600],
    }
    st.data["summary"] = row
    st.save()
    return row


SUMMARY_COLS = ["rank", "slug", "outline", "method", "osm_parts", "catastro_parts", "context", "photos",
                "exterior_photos", "plans", "warnings"]


def write_summary(args, landmarks):
    path = os.path.join(args.raw_dir, "_summary.tsv")
    rows = []
    for lm in landmarks:
        sp = os.path.join(args.raw_dir, lm["slug"], "state.json")
        if not os.path.exists(sp):
            continue
        try:
            s = L.read_json(sp).get("summary")
        except Exception:  # noqa: BLE001
            s = None
        if s:
            rows.append({"rank": lm.get("rank", ""), **s})
    with open(path + ".tmp", "w", encoding="utf-8") as f:
        f.write("\t".join(SUMMARY_COLS) + "\n")
        for r in rows:
            f.write("\t".join(str(r.get(c, "")).replace("\t", " ").replace("\n", " ") for c in SUMMARY_COLS) + "\n")
    os.replace(path + ".tmp", path)
    return path


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--list", required=True, help="list.toml with [[landmark]] entries")
    ap.add_argument("--only", default="", help="comma separated slugs")
    ap.add_argument("--force", nargs="?", const="all", default="",
                    help="redo steps even if finished: no value = all steps (images already downloaded "
                         "are kept unless 'images' is listed), or a comma list of " + ",".join(STEPS) + ",images")
    ap.add_argument("--data-dir", default=DEFAULT_DATA_DIR)
    ap.add_argument("--raw-dir", default=DEFAULT_RAW_DIR)
    ap.add_argument("--max-photos", type=int, default=16)
    ap.add_argument("--max-plans", type=int, default=6)
    ap.add_argument("--review", default=None,
                    help="photo_review.toml with per-slug hand review (default: next to --list, if present)")
    args = ap.parse_args(argv)
    args.force_steps = {s.strip() for s in args.force.split(",") if s.strip()}
    bad = args.force_steps - set(STEPS) - {"all", "images"}
    if bad:
        ap.error(f"unknown steps for --force: {sorted(bad)}")
    with open(args.list, "rb") as f:
        lst = tomllib.load(f)
    landmarks = lst.get("landmark", [])
    rpath = args.review or os.path.join(os.path.dirname(os.path.abspath(args.list)), "photo_review.toml")
    args.review = {}
    if os.path.exists(rpath):
        with open(rpath, "rb") as f:
            args.review = tomllib.load(f)
    slugs = [lm.get("slug") for lm in landmarks]
    dup = {s for s in slugs if slugs.count(s) > 1}
    if dup:
        ap.error(f"duplicate slugs in list: {sorted(dup)}")
    for lm in landmarks:
        if not re.match(r"^[a-z0-9][a-z0-9_-]*$", lm.get("slug", "")):
            ap.error(f"bad slug {lm.get('slug')!r}")
    only = [s.strip() for s in args.only.split(",") if s.strip()]
    if only:
        missing = set(only) - set(slugs)
        if missing:
            ap.error(f"--only slugs not in list: {sorted(missing)}")
        todo = [lm for lm in landmarks if lm["slug"] in only]
    else:
        todo = landmarks
    os.makedirs(args.raw_dir, exist_ok=True)
    global LOG
    LOG = Log(os.path.join(args.raw_dir, "_fetch.log"))
    http = L.Http(log=LOG)
    LOG(f"fetch.py: {len(todo)} landmark(s); data={args.data_dir} raw={args.raw_dir} "
        f"force={sorted(args.force_steps) or '-'}")
    failures = 0
    for lm in todo:
        t0 = time.time()
        try:
            row = process(lm, args, http)
            LOG(f"[{lm['slug']}] done in {time.time() - t0:.0f}s: " +
                ", ".join(f"{k}={row[k]}" for k in SUMMARY_COLS[2:-1]))
        except KeyboardInterrupt:
            raise
        except Exception as ex:  # noqa: BLE001
            failures += 1
            LOG(f"[{lm['slug']}] FAILED: {ex}")
            LOG(traceback.format_exc())
        write_summary(args, landmarks)
    path = write_summary(args, landmarks)
    LOG(f"summary: {path} ({http.requests} HTTP requests, {failures} failure(s))")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
