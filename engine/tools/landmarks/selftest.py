#!/usr/bin/env python3
"""Offline checks for lmcommon.py / fetch.py helpers (no network).

    python3 -I engine/tools/landmarks/selftest.py
"""

import math
import os
import sys
import tomllib

TOOL_DIR = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, TOOL_DIR)

import lmcommon as L  # noqa: E402
import fetch as F  # noqa: E402


def snyder_utm(lat, lon, lon0=-3.0):
    """Independent (USGS/Snyder) transverse Mercator series for comparison."""
    a, f, k0 = 6378137.0, 1 / 298.257223563, 0.9996
    e2 = f * (2 - f)
    ep2 = e2 / (1 - e2)
    phi, lam = math.radians(lat), math.radians(lon - lon0)
    n = a / math.sqrt(1 - e2 * math.sin(phi) ** 2)
    t = math.tan(phi) ** 2
    c = ep2 * math.cos(phi) ** 2
    A = lam * math.cos(phi)
    m = a * ((1 - e2 / 4 - 3 * e2 ** 2 / 64 - 5 * e2 ** 3 / 256) * phi
             - (3 * e2 / 8 + 3 * e2 ** 2 / 32 + 45 * e2 ** 3 / 1024) * math.sin(2 * phi)
             + (15 * e2 ** 2 / 256 + 45 * e2 ** 3 / 1024) * math.sin(4 * phi)
             - (35 * e2 ** 3 / 3072) * math.sin(6 * phi))
    x = k0 * n * (A + (1 - t + c) * A ** 3 / 6 + (5 - 18 * t + t * t + 72 * c - 58 * ep2) * A ** 5 / 120) + 500000
    y = k0 * (m + n * math.tan(phi) * (A * A / 2 + (5 - t + 9 * c + 4 * c * c) * A ** 4 / 24
                                       + (61 - 58 * t + t * t + 600 * c - 330 * ep2) * A ** 6 / 720))
    return x, y


def test_projection():
    for lat, lon in [(39.46975, -0.37739), (39.4744, -0.3783), (39.42, -0.31), (39.53, -0.42), (40.4, -3.7)]:
        e, n = L.utm30n(lat, lon)
        se, sn = snyder_utm(lat, lon)
        assert abs(e - se) < 0.005 and abs(n - sn) < 0.005, (lat, lon, e - se, n - sn)
        la, lo = L.utm30n_inverse(e, n)
        assert abs(la - lat) < 1e-9 and abs(lo - lon) < 1e-9
    x, z = L.ll_to_city(L.ORIGIN_LAT, L.ORIGIN_LON)
    assert abs(x) < 1e-6 and abs(z) < 1e-6
    # north is -z, east is +x
    x, z = L.ll_to_city(L.ORIGIN_LAT + 0.001, L.ORIGIN_LON)
    assert z < -100 and abs(x) < 4  # meridian convergence ~1.67 deg
    assert abs(L.CONVERGENCE_DEG - 1.668) < 0.01
    x, z = L.ll_to_city(L.ORIGIN_LAT, L.ORIGIN_LON + 0.001)
    assert x > 80 and abs(z) < 4
    assert abs(L.ORIGIN_E - 725602.415) < 0.01 and abs(L.ORIGIN_N - 4372190.758) < 0.01


def test_geometry():
    sq = [(0, 0), (10, 0), (10, 10), (0, 10)]
    hole = [(4, 4), (6, 4), (6, 6), (4, 6)]
    assert abs(L.polygon_area(sq, [hole]) - 96) < 1e-9
    c = L.polygon_centroid(sq, [hole])
    assert abs(c[0] - 5) < 1e-9 and abs(c[1] - 5) < 1e-9
    lsh = [(0, 0), (20, 0), (20, 10), (10, 10), (10, 20), (0, 20)]
    c = L.polygon_centroid(lsh)
    assert abs(c[0] - 25 / 3) < 1e-9 and abs(c[1] - 25 / 3) < 1e-9
    assert L.point_in_polygon(1, 1, sq, [hole]) and not L.point_in_polygon(5, 5, sq, [hole])
    half = ([(5, 0), (15, 0), (15, 10), (5, 10)], [])
    fr = L.overlap_fraction(half, [(sq, [])])
    assert abs(fr - 0.5) < 0.03, fr
    assert L.overlap_fraction(([(2, 2), (3, 2), (3, 3), (2, 3)], []), [(sq, [])]) > 0.99
    assert L.overlap_fraction(([(4.5, 4.5), (5.5, 4.5), (5.5, 5.5), (4.5, 5.5)], []), [(sq, [hole])]) < 0.01
    assert abs(L.distance_point_polygon(13, 5, sq) - 3) < 1e-9
    assert L.open_ring([(0, 0), (1, 0), (1, 1), (0, 0)]) == [(0, 0), (1, 0), (1, 1)]


def test_rings():
    rings, left = F.assemble_rings([[1, 2, 3], [5, 4, 3], [5, 6, 1]])
    assert left == 0 and len(rings) == 1 and rings[0][0] == rings[0][-1] and len(rings[0]) == 7
    rings, left = F.assemble_rings([[1, 2, 3], [7, 8]])
    assert not rings and left == 2


def test_names_and_classes():
    assert F.names_match("Llotja de la Seda", "Lonja de la Seda") is False
    assert F.names_match("Llotja de la Seda", "Llotja de la Seda de València")
    assert F.names_match("Mercat Central", "Mercat Central de València")
    assert not F.names_match("Torre", "Torres de Serranos")
    assert F.parse_osm_id("way/123") == ("way", 123) and F.parse_osm_id("r45") == ("relation", 45)

    def cls(title, mime="image/jpeg", cats=(), src="Category:X", cam=None):
        r = {"title": title, "mime": mime, "categories": list(cats), "sources": [src], "description": "",
             "wikidata_role": "", "date": "", "camera": cam}
        F.classify(r, "Category:X")
        return r

    assert cls("File:Planta de la Lonja de Valencia.jpg")["class"] == "plan"
    assert cls("File:Torre, porta d'entrada a la planta noble.jpg")["class"] == "photo"
    assert cls("File:X - cross section.png")["plan_kind"] == "section"
    assert cls("File:Plano de Valencia 1704.jpg")["class"] == "map"
    assert cls("File:Llotja interior.jpg")["exterior"] is False
    assert cls("File:P1.JPG", cats=["Plaques installed in 1908 in Spain"])["exterior"] != False  # noqa: E712
    assert cls("File:P2.JPG", src="Category:Llotja - Exterior")["exterior"] is True


def test_toml():
    doc = {"slug": "x", "wikipedia": {"es": "https://es.wikipedia.org/wiki/A"},
           "facts": {"height": {"value": 17.4, "unit": "m", "source": "https://w/Q1#P2048"},
                     "heritage": {"value": ["a \"q\"", "b\\c"], "source": "s"}},
           "photo": [{"file": "photos/01_a.jpg", "camera": [1.5, -2.25], "exterior": "unknown",
                      "selected": False, "side": ""}]}
    back = tomllib.loads(L.toml_dumps(doc, "header"))
    assert back["facts"]["heritage"]["value"] == ["a \"q\"", "b\\c"]
    assert back["photo"][0]["camera"] == [1.5, -2.25] and back["photo"][0]["exterior"] == "unknown"
    merged = F.merge_sources(
        {"slug": "x", "facts": {"height": {"value": 1.0, "source": "https://www.wikidata.org/x"}},
         "photo": [{"file": "photos/01_a.jpg", "title": "File:A.jpg", "selected": False, "side": ""}],
         "plan": [], "reference": []},
        {"slug": "x", "facts": {"height": {"value": 2.0, "source": "https://www.wikidata.org/x"},
                                "eaves": {"value": 12.0, "source": "survey"}},
         "photo": [{"file": "photos/03_a.jpg", "title": "File:A.jpg", "selected": True, "side": "north"},
                   {"file": "/elsewhere/b.jpg", "title": "B", "selected": True}],
         "reference": [{"url": "u", "title": "t", "notes": ""}], "notes": "keep"})
    assert merged["facts"]["height"]["value"] == 1.0 and merged["facts"]["eaves"]["value"] == 12.0
    assert merged["photo"][0]["selected"] is True and merged["photo"][0]["side"] == "north"
    assert merged["photo"][0]["file"] == "photos/01_a.jpg" and len(merged["photo"]) == 2
    assert merged["notes"] == "keep" and merged["reference"][0]["url"] == "u"
    # 3-way: values edited by hand win, regenerated values replace old generated ones
    base = {"slug": "x", "facts": {"height": {"value": 2.0, "source": "https://www.wikidata.org/x"}},
            "photo": [{"file": "photos/03_a.jpg", "title": "File:A.jpg", "exterior": "unknown",
                       "selected": False, "side": ""},
                      {"file": "photos/04_c.jpg", "title": "File:C.jpg", "exterior": True, "selected": False}],
            "plan": [], "reference": []}
    old = {"slug": "x", "facts": {"height": {"value": 2.0, "source": "https://www.wikidata.org/x"},
                                  "eaves": {"value": 12.0, "source": "https://www.wikidata.org/y"}},
           "photo": [{"file": "photos/03_a.jpg", "title": "File:A.jpg", "exterior": False,
                      "selected": True, "side": "north"},
                     {"file": "photos/04_c.jpg", "title": "File:C.jpg", "exterior": True, "selected": False}],
           "plan": [], "reference": []}
    new = {"slug": "x", "facts": {"height": {"value": 3.0, "source": "https://www.wikidata.org/x"}},
           "photo": [{"file": "photos/01_a.jpg", "title": "File:A.jpg", "exterior": True,
                      "selected": False, "side": ""}], "plan": [], "reference": []}
    m = F.merge_sources(new, old, base)
    assert m["facts"]["height"]["value"] == 3.0 and m["facts"]["eaves"]["value"] == 12.0
    p = m["photo"][0]
    assert p["file"] == "photos/01_a.jpg" and p["exterior"] is False and p["selected"] is True and p["side"] == "north"
    assert len(m["photo"]) == 1  # C was generated, untouched, and is no longer ranked


if __name__ == "__main__":
    for name, fn in list(globals().items()):
        if name.startswith("test_"):
            fn()
            print("ok", name)
