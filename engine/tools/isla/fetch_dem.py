#!/usr/bin/env python3
"""Downloads the real terrain of Valencia and its surroundings for the
island: IGN MDT25 (Modelo Digital del Terreno, 25 m, CC BY 4.0 scne.es)
from the IGN WCS, one request, in UTM 30N (the island keeps Valencia at
real scale and position: world x = E - E0, z = -(N - N0)).

Usage: fetch_dem.py <out dir>     (writes dem_valencia.png and .json)

The service delivers whole metres; the grid is smoothed (Gaussian, sigma
125 m) so the plain has no 1 m terraces, and stored as 16-bit centimetres.
"""
import io, json, sys, urllib.request
import numpy as np
from PIL import Image

E0, N0 = 725602.415, 4372190.758
W, S, E, N = 712400, 4363800, 734100, 4379000
out = sys.argv[1]
url = f"https://servicios.idee.es/wcs-inspire/mdt?SERVICE=WCS&REQUEST=GetCoverage&VERSION=2.0.1&COVERAGEID=Elevacion25830_25&SUBSET=x({W},{E})&SUBSET=y({S},{N})&FORMAT=image/tiff"
raw = urllib.request.urlopen(urllib.request.Request(url, headers={"User-Agent": "ForjaEngine/0.1"}), timeout=180).read()
a = np.array(Image.open(io.BytesIO(raw))).astype(np.float32)
assert a.shape == ((N - S) // 25, (E - W) // 25), a.shape
a = np.clip(a, 0, 2000)
k = np.exp(-0.5 * (np.arange(-15, 16) / 5.0) ** 2)
k /= k.sum()
s = np.pad(a, 15, mode="edge")
s = np.apply_along_axis(lambda r: np.convolve(r, k, mode="valid"), 1, s)
s = np.apply_along_axis(lambda r: np.convolve(r, k, mode="valid"), 0, s)
assert s.shape == a.shape
Image.fromarray(np.round(s * 100).astype(np.uint16)).save(f"{out}/dem_valencia.png", optimize=True)
# World extent of the pixel centres: west, north (min z), east, south.
meta = {"source": "IGN MDT25 (CC BY 4.0 scne.es), WCS servicios.idee.es/wcs-inspire/mdt, smoothed sigma 125 m", "units": "centimetres above sea level", "width": a.shape[1], "height": a.shape[0],
        "extent": [W + 12.5 - E0, -(N - 12.5 - N0), E - 12.5 - E0, -(S + 12.5 - N0)]}
json.dump(meta, open(f"{out}/dem_valencia.json", "w"), indent=1)
print(meta, float(s.min()), float(s.max()))
