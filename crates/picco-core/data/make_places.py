"""Packs place names for offline reverse geocoding into places.bin.

Usage: python3 -I make_places.py <download dir> <output file>

The download dir holds, unpacked:
  cities1000.txt, countryInfo.txt
      https://download.geonames.org/export/dump/ (CC BY 4.0, GeoNames)
  ne_50m_admin_0_countries.geojson, ne_10m_geography_marine_polys.geojson,
  ne_50m_geography_regions_polys.geojson
      https://github.com/nvkelso/natural-earth-vector/tree/master/geojson
      (public domain, Natural Earth)

The output is zlib-compressed; everything inside is little-endian:
  b"PICOGEO1"
  u16 countries, each: str name                (index = GeoNames country)
  u32 cities, each: i32 lat, i32 lon (1e-5 degrees), u8 radius km,
                    u16 country, str name
  three shape sets (countries, seas and oceans, continents), each:
    u16 shapes, each: str name, u16 rings,
                      each ring: u32 points, each: i32 lat, i32 lon
  where str is u8 length + UTF-8.
"""

import json
import struct
import sys
import zlib

# Sections of a city, and places that are gone.
SKIPPED_CODES = {"PPLX", "PPLH", "PPLQ", "PPLW"}


def radius_km(population):
    """How far from its center a photo still counts as taken in the place."""
    if population >= 1_000_000:
        return 35
    if population >= 100_000:
        return 20
    if population >= 10_000:
        return 12
    return 7


def text(s):
    b = s.encode("utf-8")[:255]
    return struct.pack("<B", len(b)) + b


def coord(x):
    return round(x * 100_000)


def countries(src):
    codes, out = {}, []
    for line in open(f"{src}/countryInfo.txt", encoding="utf-8"):
        if line.startswith("#"):
            continue
        cols = line.rstrip("\n").split("\t")
        codes[cols[0]] = len(out)
        out.append(cols[4])
    return codes, out


def cities(src, codes):
    out = []
    for line in open(f"{src}/cities1000.txt", encoding="utf-8"):
        cols = line.rstrip("\n").split("\t")
        if cols[7] in SKIPPED_CODES or cols[8] not in codes:
            continue
        lat, lon, population = float(cols[4]), float(cols[5]), int(cols[14] or 0)
        out.append((lat, lon, radius_km(population), codes[cols[8]], cols[1]))
    return out


def shapes(path, name_of, keep=lambda p: True):
    out = []
    for feature in json.load(open(path, encoding="utf-8"))["features"]:
        p, g = feature["properties"], feature["geometry"]
        name = name_of(p)
        if not g or not name or not keep(p):
            continue
        if name.isupper():
            name = name.title()
        polygons = [g["coordinates"]] if g["type"] == "Polygon" else g["coordinates"]
        rings = [ring for polygon in polygons for ring in polygon]
        out.append((name, rings))
    return out


def pack_shapes(items):
    b = struct.pack("<H", len(items))
    for name, rings in items:
        b += text(name) + struct.pack("<H", len(rings))
        for ring in rings:
            b += struct.pack("<I", len(ring))
            b += b"".join(struct.pack("<ii", coord(lat), coord(lon)) for lon, lat, *_ in ring)
    return b


def main(src, dst):
    codes, names = countries(src)
    b = b"PICOGEO1" + struct.pack("<H", len(names)) + b"".join(text(n) for n in names)
    places = cities(src, codes)
    b += struct.pack("<I", len(places))
    for lat, lon, radius, country, name in places:
        b += struct.pack("<iiBH", coord(lat), coord(lon), radius, country) + text(name)
    # The same country names as for towns, where Natural Earth has the code.
    by_code = {code: names[i] for code, i in codes.items()}
    b += pack_shapes(
        shapes(
            f"{src}/ne_50m_admin_0_countries.geojson",
            lambda p: by_code.get(p["ISO_A2_EH"], p["NAME_EN"]),
        )
    )
    b += pack_shapes(shapes(f"{src}/ne_10m_geography_marine_polys.geojson", lambda p: p["name"]))
    b += pack_shapes(
        shapes(
            f"{src}/ne_50m_geography_regions_polys.geojson",
            lambda p: p["NAME_EN"],
            lambda p: p["FEATURECLA"] == "Continent",
        )
    )
    open(dst, "wb").write(zlib.compress(b, 9))
    print(f"{len(places)} cities, {len(b)} bytes, {len(zlib.compress(b, 9))} packed")


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
