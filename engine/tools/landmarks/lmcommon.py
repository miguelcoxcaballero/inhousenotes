"""Shared helpers for the Valencia landmark data tools (fetch.py, overlay.py).

Pure python3 stdlib + numpy. Imported by the tools after they put their own
directory on sys.path (they run with ``python3 -I``, which drops it).

Contents:
  * UTM zone 30N forward projection (Krueger series, 6th order in n) and the
    Forja city frame (origin Placa de l'Ajuntament, x east, z south, y up).
  * Polygon helpers: area, centroid, point-in-polygon (with holes), sampled
    overlap fraction, distances.
  * A small TOML writer (tomllib only reads).
  * A polite HTTP client with per-host rate limits and 429/5xx backoff.
"""

from __future__ import annotations

import json
import math
import os
import re
import socket
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

import numpy as np

# ---------------------------------------------------------------------------
# Projection
# ---------------------------------------------------------------------------

WGS84_A = 6378137.0
WGS84_F = 1.0 / 298.257223563
UTM_K0 = 0.9996
UTM_FALSE_EASTING = 500000.0
UTM30_LON0 = -3.0  # central meridian of zone 30

ORIGIN_LAT = 39.469750
ORIGIN_LON = -0.377390
FRAME = "utm30n-local-v1"

_N = WGS84_F / (2.0 - WGS84_F)
_E = math.sqrt(WGS84_F * (2.0 - WGS84_F))
_A_RECT = WGS84_A / (1.0 + _N) * (1.0 + _N**2 / 4.0 + _N**4 / 64.0 + _N**6 / 256.0)
_ALPHA = (
    _N / 2.0 - 2.0 / 3.0 * _N**2 + 5.0 / 16.0 * _N**3 + 41.0 / 180.0 * _N**4
    - 127.0 / 288.0 * _N**5 + 7891.0 / 37800.0 * _N**6,
    13.0 / 48.0 * _N**2 - 3.0 / 5.0 * _N**3 + 557.0 / 1440.0 * _N**4
    + 281.0 / 630.0 * _N**5 - 1983433.0 / 1935360.0 * _N**6,
    61.0 / 240.0 * _N**3 - 103.0 / 140.0 * _N**4 + 15061.0 / 26880.0 * _N**5
    + 167603.0 / 181440.0 * _N**6,
    49561.0 / 161280.0 * _N**4 - 179.0 / 168.0 * _N**5 + 6601661.0 / 7257600.0 * _N**6,
    34729.0 / 80640.0 * _N**5 - 3418889.0 / 1995840.0 * _N**6,
    212378941.0 / 319334400.0 * _N**6,
)
_BETA = (
    _N / 2.0 - 2.0 / 3.0 * _N**2 + 37.0 / 96.0 * _N**3 - 1.0 / 360.0 * _N**4
    - 81.0 / 512.0 * _N**5 + 96199.0 / 604800.0 * _N**6,
    1.0 / 48.0 * _N**2 + 1.0 / 15.0 * _N**3 - 437.0 / 1440.0 * _N**4
    + 46.0 / 105.0 * _N**5 - 1118711.0 / 3870720.0 * _N**6,
    17.0 / 480.0 * _N**3 - 37.0 / 840.0 * _N**4 - 209.0 / 4480.0 * _N**5
    + 5569.0 / 90720.0 * _N**6,
    4397.0 / 161280.0 * _N**4 - 11.0 / 504.0 * _N**5 - 830251.0 / 7257600.0 * _N**6,
    4583.0 / 161280.0 * _N**5 - 108847.0 / 3991680.0 * _N**6,
    20648693.0 / 638668800.0 * _N**6,
)


def utm30n(lat: float, lon: float) -> tuple[float, float]:
    """WGS84/ETRS89 lat, lon (degrees) -> UTM zone 30N (E, N) in metres."""
    phi = math.radians(lat)
    lam = math.radians(lon - UTM30_LON0)
    s = math.sin(phi)
    t = math.sinh(math.atanh(s) - _E * math.atanh(_E * s))
    xi = math.atan2(t, math.cos(lam))
    eta = math.atanh(math.sin(lam) / math.sqrt(1.0 + t * t))
    x = eta
    y = xi
    for j, a in enumerate(_ALPHA, start=1):
        x += a * math.cos(2 * j * xi) * math.sinh(2 * j * eta)
        y += a * math.sin(2 * j * xi) * math.cosh(2 * j * eta)
    return UTM_FALSE_EASTING + UTM_K0 * _A_RECT * x, UTM_K0 * _A_RECT * y


def utm30n_inverse(e: float, n: float) -> tuple[float, float]:
    """UTM zone 30N (E, N) -> (lat, lon) degrees (Krueger series)."""
    xi = n / (UTM_K0 * _A_RECT)
    eta = (e - UTM_FALSE_EASTING) / (UTM_K0 * _A_RECT)
    xi_p = xi
    eta_p = eta
    for j, b in enumerate(_BETA, start=1):
        xi_p -= b * math.sin(2 * j * xi) * math.cosh(2 * j * eta)
        eta_p -= b * math.cos(2 * j * xi) * math.sinh(2 * j * eta)
    tau_p = math.sin(xi_p) / math.sqrt(math.sinh(eta_p) ** 2 + math.cos(xi_p) ** 2)
    lam = math.atan2(math.sinh(eta_p), math.cos(xi_p))
    # Newton iteration tau' -> tau (Karney 2011, eq. 19-21)
    tau = tau_p
    for _ in range(8):
        sigma = math.sinh(_E * math.atanh(_E * tau / math.sqrt(1 + tau * tau)))
        tau_i = tau * math.sqrt(1 + sigma * sigma) - sigma * math.sqrt(1 + tau * tau)
        d = (tau_p - tau_i) / math.sqrt(1 + tau_i * tau_i) * (
            1 + (1 - _E * _E) * tau * tau) / ((1 - _E * _E) * math.sqrt(1 + tau * tau))
        tau += d
        if abs(d) < 1e-14:
            break
    return math.degrees(math.atan(tau)), UTM30_LON0 + math.degrees(lam)


ORIGIN_E, ORIGIN_N = utm30n(ORIGIN_LAT, ORIGIN_LON)
# Meridian convergence at the origin: true north is rotated this many degrees
# counter-clockwise (towards -x) from grid north (-z). A compass heading h
# (clockwise from true north) is the grid azimuth h - CONVERGENCE_DEG.
CONVERGENCE_DEG = math.degrees(math.atan(
    math.tan(math.radians(ORIGIN_LON - UTM30_LON0)) * math.sin(math.radians(ORIGIN_LAT))))


def origin_dict() -> dict:
    return {"lat": ORIGIN_LAT, "lon": ORIGIN_LON, "E0": round(ORIGIN_E, 3), "N0": round(ORIGIN_N, 3)}


def en_to_city(e: float, n: float) -> tuple[float, float]:
    """UTM 30N -> city frame (x east, z = -north)."""
    return e - ORIGIN_E, -(n - ORIGIN_N)


def city_to_en(x: float, z: float) -> tuple[float, float]:
    return x + ORIGIN_E, ORIGIN_N - z


def ll_to_city(lat: float, lon: float) -> tuple[float, float]:
    return en_to_city(*utm30n(lat, lon))


def city_to_ll(x: float, z: float) -> tuple[float, float]:
    return utm30n_inverse(*city_to_en(x, z))


# ---------------------------------------------------------------------------
# Polygon helpers. A ring is a list of (x, y) tuples, not closed.
# A polygon is (outer_ring, [inner_rings]).
# ---------------------------------------------------------------------------


def open_ring(ring):
    ring = [tuple(map(float, p)) for p in ring]
    while len(ring) > 1 and ring[0] == ring[-1]:
        ring.pop()
    # drop consecutive duplicates
    out = []
    for p in ring:
        if not out or (abs(p[0] - out[-1][0]) > 1e-9 or abs(p[1] - out[-1][1]) > 1e-9):
            out.append(p)
    if len(out) > 1 and out[0] == out[-1]:
        out.pop()
    return out


def ring_signed_area(ring) -> float:
    if len(ring) < 3:
        return 0.0
    a = np.asarray(ring, dtype=float)
    x, y = a[:, 0], a[:, 1]
    return 0.5 * float(np.dot(x, np.roll(y, -1)) - np.dot(np.roll(x, -1), y))


def ring_area(ring) -> float:
    return abs(ring_signed_area(ring))


def polygon_area(outer, inners=()) -> float:
    return max(0.0, ring_area(outer) - sum(ring_area(r) for r in inners))


def ring_centroid(ring):
    """Area centroid and absolute area of a simple ring."""
    a = np.asarray(ring, dtype=float)
    # shift for numerical stability
    o = a.mean(axis=0)
    x, y = a[:, 0] - o[0], a[:, 1] - o[1]
    x1, y1 = np.roll(x, -1), np.roll(y, -1)
    cross = x * y1 - x1 * y
    s = cross.sum()
    if abs(s) < 1e-12:
        return (float(o[0]), float(o[1])), 0.0
    cx = ((x + x1) * cross).sum() / (3.0 * s)
    cy = ((y + y1) * cross).sum() / (3.0 * s)
    return (float(cx + o[0]), float(cy + o[1])), float(abs(s) / 2.0)


def polygon_centroid(outer, inners=()):
    """Area centroid of a polygon with holes."""
    (cx, cy), a = ring_centroid(outer)
    sx, sy, sa = cx * a, cy * a, a
    for r in inners:
        (hx, hy), ha = ring_centroid(r)
        sx -= hx * ha
        sy -= hy * ha
        sa -= ha
    if sa <= 1e-9:
        return (float(cx), float(cy))
    return (float(sx / sa), float(sy / sa))


def bbox_of(rings):
    xs = [p[0] for r in rings for p in r]
    ys = [p[1] for r in rings for p in r]
    return min(xs), min(ys), max(xs), max(ys)


def points_in_ring(px, py, ring) -> np.ndarray:
    """Even-odd test of many points against one ring (vectorised)."""
    a = np.asarray(ring, dtype=float)
    x1, y1 = a[:, 0], a[:, 1]
    x2, y2 = np.roll(x1, -1), np.roll(y1, -1)
    px = np.asarray(px, dtype=float)
    py = np.asarray(py, dtype=float)
    out = np.zeros(len(px), dtype=bool)
    # Chunk over points so the (points x edges) temporaries stay ~32 MB
    # (a large Catastro part against a 200-edge ring needed 2 GB and was
    # OOM-killed).
    chunk = max(1, 4_000_000 // max(1, len(a)))
    for s in range(0, len(px), chunk):
        cx = px[s:s + chunk, None]
        cy = py[s:s + chunk, None]
        cond = (y1 > cy) != (y2 > cy)
        with np.errstate(divide="ignore", invalid="ignore"):
            xint = x1 + (cy - y1) * (x2 - x1) / (y2 - y1)
        hit = cond & (cx < xint)
        out[s:s + chunk] = (hit.sum(axis=1) % 2) == 1
    return out


def points_in_polygon(px, py, outer, inners=()) -> np.ndarray:
    inside = points_in_ring(px, py, outer)
    for r in inners:
        if len(r) >= 3:
            inside &= ~points_in_ring(px, py, r)
    return inside


def point_in_polygon(x, y, outer, inners=()) -> bool:
    return bool(points_in_polygon([x], [y], outer, inners)[0])


def sample_polygon(outer, inners=(), target=900, max_step=0.5):
    """Grid sample points inside a polygon. Returns (px, py, cell_area)."""
    x0, y0, x1, y1 = bbox_of([outer])
    area = max(polygon_area(outer, inners), 1e-6)
    step = min(max_step, max(0.02, math.sqrt(area / target)))
    # Cap the grid at ~250k cells for very large polygons (port areas, parks).
    step = max(step, math.sqrt(max((x1 - x0) * (y1 - y0), 0.0) / 250_000))
    xs = np.arange(x0 + step / 2, x1, step)
    ys = np.arange(y0 + step / 2, y1, step)
    if len(xs) == 0 or len(ys) == 0:
        c = polygon_centroid(outer, inners)
        return np.array([c[0]]), np.array([c[1]]), area
    gx, gy = np.meshgrid(xs, ys)
    px, py = gx.ravel(), gy.ravel()
    m = points_in_polygon(px, py, outer, inners)
    if not m.any():
        c = polygon_centroid(outer, inners)
        return np.array([c[0]]), np.array([c[1]]), area
    return px[m], py[m], step * step


def overlap_fraction(part, polys) -> float:
    """Fraction of the area of polygon `part` (outer, inners) covered by the
    union of `polys` (list of (outer, inners)). Grid sampled."""
    px, py, _ = sample_polygon(part[0], part[1])
    if len(px) == 0:
        return 0.0
    cov = np.zeros(len(px), dtype=bool)
    for o, i in polys:
        bx0, by0, bx1, by1 = bbox_of([o])
        m = (px >= bx0) & (px <= bx1) & (py >= by0) & (py <= by1) & ~cov
        if m.any():
            idx = np.nonzero(m)[0]
            cov[idx] = points_in_polygon(px[idx], py[idx], o, i)
    return float(cov.mean())


def seg_point_dist(px, py, ring) -> float:
    a = np.asarray(ring, dtype=float)
    x1, y1 = a[:, 0], a[:, 1]
    x2, y2 = np.roll(x1, -1), np.roll(y1, -1)
    dx, dy = x2 - x1, y2 - y1
    l2 = dx * dx + dy * dy
    with np.errstate(divide="ignore", invalid="ignore"):
        t = np.where(l2 > 0, ((px - x1) * dx + (py - y1) * dy) / l2, 0.0)
    t = np.clip(t, 0, 1)
    qx, qy = x1 + t * dx, y1 + t * dy
    return float(np.min(np.hypot(px - qx, py - qy)))


def distance_point_polygon(x, y, outer, inners=()) -> float:
    """0 inside, else distance to the boundary."""
    if point_in_polygon(x, y, outer, inners):
        return 0.0
    d = seg_point_dist(x, y, outer)
    for r in inners:
        d = min(d, seg_point_dist(x, y, r))
    return d


# ---------------------------------------------------------------------------
# TOML writer (subset: str, int, float, bool, list, dict, list of dicts)
# ---------------------------------------------------------------------------

_BARE = re.compile(r"^[A-Za-z0-9_-]+$")


def toml_key(k: str) -> str:
    return k if _BARE.match(k) else toml_str(k)


def toml_str(s: str) -> str:
    out = ['"']
    for ch in s:
        o = ord(ch)
        if ch == '"':
            out.append('\\"')
        elif ch == "\\":
            out.append("\\\\")
        elif ch == "\n":
            out.append("\\n")
        elif ch == "\t":
            out.append("\\t")
        elif ch == "\r":
            out.append("\\r")
        elif o < 0x20 or o == 0x7F:
            out.append("\\u%04x" % o)
        else:
            out.append(ch)
    out.append('"')
    return "".join(out)


def toml_value(v) -> str:
    if isinstance(v, bool):
        return "true" if v else "false"
    if isinstance(v, int):
        return str(v)
    if isinstance(v, (np.integer,)):
        return str(int(v))
    if isinstance(v, (float, np.floating)):
        v = float(v)
        if math.isnan(v) or math.isinf(v):
            return toml_str(str(v))
        r = repr(round(v, 6))
        if "e" in r or "E" in r:
            r = "%.6f" % v
        if "." not in r:
            r += ".0"
        return r
    if isinstance(v, str):
        return toml_str(v)
    if isinstance(v, (list, tuple)):
        return "[" + ", ".join(toml_value(x) for x in v) + "]"
    if isinstance(v, dict):
        return "{ " + ", ".join(f"{toml_key(k)} = {toml_value(x)}" for k, x in v.items() if x is not None) + " }"
    if v is None:
        return '""'
    return toml_str(str(v))


def _is_table_array(v) -> bool:
    return isinstance(v, list) and len(v) > 0 and all(isinstance(x, dict) for x in v)


def toml_dumps(doc: dict, header_comment: str = "") -> str:
    """Serialise: top-level scalars/inline values first, then [tables]
    (whose values are written inline), then [[arrays of tables]]."""
    lines = []
    if header_comment:
        for l in header_comment.splitlines():
            lines.append("# " + l if l else "#")
        lines.append("")
    scalars = {k: v for k, v in doc.items() if not isinstance(v, dict) and not _is_table_array(v)}
    tables = {k: v for k, v in doc.items() if isinstance(v, dict) and k not in ("wikipedia",)}
    if "wikipedia" in doc and isinstance(doc["wikipedia"], dict):
        scalars["wikipedia"] = doc["wikipedia"]
    arrays = {k: v for k, v in doc.items() if _is_table_array(v)}
    for k, v in scalars.items():
        if v is None:
            continue
        lines.append(f"{toml_key(k)} = {toml_value(v)}")
    for k, v in tables.items():
        lines.append("")
        lines.append(f"[{toml_key(k)}]")
        for kk, vv in v.items():
            if vv is None:
                continue
            lines.append(f"{toml_key(kk)} = {toml_value(vv)}")
    for k, arr in arrays.items():
        for item in arr:
            lines.append("")
            lines.append(f"[[{toml_key(k)}]]")
            for kk, vv in item.items():
                if vv is None:
                    continue
                lines.append(f"{toml_key(kk)} = {toml_value(vv)}")
    return "\n".join(lines) + "\n"


# ---------------------------------------------------------------------------
# HTTP
# ---------------------------------------------------------------------------

USER_AGENT = "ForjaEngineLandmarks/0.1 (https://github.com/miguelcoxcaballero/inhousenotes)"

HOST_INTERVAL = {
    "api.openstreetmap.org": 1.05,
    "nominatim.openstreetmap.org": 1.1,
    "www.wikidata.org": 0.5,
    "query.wikidata.org": 1.0,
    "commons.wikimedia.org": 0.6,
    "upload.wikimedia.org": 0.6,
    "thumb.wikimedia.org": 0.6,
    "ovc.catastro.meh.es": 0.5,
}
# Hosts that share one rate-limit bucket (redirect targets).
HOST_GROUP = {"upload.wikimedia.org": "wikimedia", "thumb.wikimedia.org": "wikimedia",
              "commons.wikimedia.org": "wikimedia"}


class HttpError(Exception):
    def __init__(self, url, code, msg=""):
        super().__init__(f"HTTP {code} for {url} {msg}")
        self.url = url
        self.code = code


class _NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


class Http:
    def __init__(self, log=None, max_tries: int = 9, max_wait: float = 900.0):
        self.log = log or (lambda *a: print(*a, file=sys.stderr))
        self.next_ok: dict[str, float] = {}
        self.interval = dict(HOST_INTERVAL)
        self.max_tries = max_tries
        self.max_wait = max_wait
        self.requests = 0
        self._opener = urllib.request.build_opener()
        self._opener_noredir = urllib.request.build_opener(_NoRedirect)

    def _bucket(self, host):
        return HOST_GROUP.get(host, host)

    def _wait_turn(self, host):
        b = self._bucket(host)
        now = time.monotonic()
        t = self.next_ok.get(b, 0.0)
        if t > now:
            time.sleep(t - now)
        self.next_ok[b] = time.monotonic() + self.interval.get(host, 1.0)

    def _penalise(self, host, seconds):
        b = self._bucket(host)
        self.next_ok[b] = max(self.next_ok.get(b, 0.0), time.monotonic() + seconds)
        # slow down subsequent requests to this host a bit
        self.interval[host] = min(5.0, self.interval.get(host, 1.0) * 1.5)

    def request(self, url, params=None, data=None, headers=None, redirect=True,
                accept_codes=(200,), timeout=90):
        """Returns (status, headers, body, final_url). Retries 429/5xx/network
        errors with backoff. Raises HttpError on other errors."""
        if params:
            url = url + ("&" if "?" in url else "?") + urllib.parse.urlencode(params, quote_via=urllib.parse.quote)
        host = urllib.parse.urlsplit(url).hostname or ""
        hdrs = {"User-Agent": USER_AGENT, "Accept-Encoding": "identity"}
        if headers:
            hdrs.update(headers)
        start = time.monotonic()
        last_err = None
        for attempt in range(self.max_tries):
            self._wait_turn(host)
            req = urllib.request.Request(url, data=data, headers=hdrs)
            opener = self._opener if redirect else self._opener_noredir
            try:
                self.requests += 1
                with opener.open(req, timeout=timeout) as r:
                    body = r.read()
                    # recover slowly from earlier 429 penalties
                    base = HOST_INTERVAL.get(host, 1.0)
                    if self.interval.get(host, base) > base:
                        self.interval[host] = max(base, self.interval[host] * 0.85)
                    return r.status, dict(r.headers), body, r.geturl()
            except urllib.error.HTTPError as e:
                code = e.code
                if not redirect and code in (301, 302, 303, 307, 308):
                    base = HOST_INTERVAL.get(host, 1.0)
                    if self.interval.get(host, base) > base:
                        self.interval[host] = max(base, self.interval[host] * 0.85)
                    return code, dict(e.headers), b"", e.headers.get("Location", "")
                if code in accept_codes:
                    return code, dict(e.headers), e.read(), url
                if code == 429 or code >= 500 or code == 408:
                    ra = e.headers.get("Retry-After") if e.headers else None
                    try:
                        wait = float(ra) if ra else 0.0
                    except ValueError:
                        wait = 0.0
                    wait = max(wait, min(120.0, 3.0 * (2 ** attempt)))
                    if code == 429:
                        e_host = urllib.parse.urlsplit(e.geturl() or url).hostname or host
                        self._penalise(e_host, wait)
                        self._penalise(host, wait)
                    last_err = HttpError(url, code)
                    if time.monotonic() - start + wait > self.max_wait:
                        break
                    self.log(f"  http {code} on {host}, retry in {wait:.0f}s ({attempt + 1}/{self.max_tries})")
                    time.sleep(wait)
                    continue
                raise HttpError(url, code) from None
            except (urllib.error.URLError, socket.timeout, ConnectionError, TimeoutError) as e:
                wait = min(120.0, 3.0 * (2 ** attempt))
                last_err = e
                if time.monotonic() - start + wait > self.max_wait:
                    break
                self.log(f"  network error on {host}: {e}; retry in {wait:.0f}s")
                time.sleep(wait)
                continue
        if isinstance(last_err, HttpError):
            raise last_err
        raise HttpError(url, -1, str(last_err))

    def get(self, url, params=None, **kw) -> bytes:
        return self.request(url, params=params, **kw)[2]

    def get_json(self, url, params=None, **kw):
        body = self.get(url, params=params, **kw)
        try:
            return json.loads(body.decode("utf-8"))
        except (UnicodeDecodeError, json.JSONDecodeError) as e:
            raise HttpError(url, -2, f"bad json: {e}; body starts {body[:120]!r}") from None


def write_json(path, obj, indent=1):
    os.makedirs(os.path.dirname(os.path.abspath(path)), exist_ok=True)
    tmp = path + ".tmp"
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(obj, f, ensure_ascii=False, indent=indent)
        f.write("\n")
    os.replace(tmp, path)


def write_bytes(path, data: bytes):
    os.makedirs(os.path.dirname(os.path.abspath(path)), exist_ok=True)
    tmp = path + ".tmp"
    with open(tmp, "wb") as f:
        f.write(data)
    os.replace(tmp, path)


def read_json(path):
    with open(path, encoding="utf-8") as f:
        return json.load(f)
