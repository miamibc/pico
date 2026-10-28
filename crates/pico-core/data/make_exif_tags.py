"""Writes src/exif_tags.rs: the standard EXIF tags with their names and words.

Usage: python3 -I make_exif_tags.py <download dir> <output file>

The download dir holds two saved web pages:
  exiv2.html  https://exiv2.org/tags.html (the TIFF, EXIF, DNG and
              Interoperability tags: groups Image, Photo and Iop)
  gps.html    https://exiftool.org/TagNames/GPS.html (the GPS tags)

GPS tags become 0x1_0000 + their number, Interoperability ones 0x2_0000 +,
so that all of them fit one table sorted by tag.
"""

import html
import re
import sys

HEADER = """\
//! The standard TIFF, EXIF (up to 3.0), DNG, GPS and Interoperability tags:
//! number, name as written in XMP (`exif:ExposureTime`) and words for people.
//! After the exiv2 tag reference (https://exiv2.org/tags.html) and ExifTool's
//! GPS tags (https://exiftool.org/TagNames/GPS.html). GPS tags are
//! 0x1_0000 + their number, Interoperability ones 0x2_0000 +.
//! Made by `data/make_exif_tags.py`; don't edit by hand.

/// `(tag, name, words)`, by tag.
const TAGS: &[(u32, &str, &str)] = &[
"""

FOOTER = """\
];

pub(crate) fn standard_name(tag: u32) -> Option<&'static str> {
    TAGS.binary_search_by_key(&tag, |t| t.0).ok().map(|i| TAGS[i].2)
}

/// The tag an XMP property of the `exif:` or `tiff:` namespace stands for.
pub(crate) fn tag_named(name: &str) -> Option<u32> {
    TAGS.iter().find(|t| t.1 == name).map(|t| t.0)
}
"""

GROUP_BASE = {"Image": 0, "Photo": 0, "Iop": 0x2_0000}

# Tags exiv2 doesn't list: Microsoft's, after ExifTool's EXIF table
# (https://exiftool.org/TagNames/EXIF.html).
EXTRA = [(0xEA1C, "Padding"), (0xEA1D, "OffsetSchema")]

# Names the word splitting gets wrong.
WORDS = {"T4Options": "T4 Options", "T6Options": "T6 Options", "GPSDOP": "GPS DOP"}


def words(name):
    if name in WORDS:
        return WORDS[name]
    return " ".join(re.findall(r"[A-Z]+(?=[A-Z][a-z])|[A-Z]?[a-z]+|[A-Z]+|\d+", name))


def exiv2_tags(path):
    page = open(path, encoding="utf-8").read()
    row = re.compile(r"<td>0x([0-9a-f]{4})</td>\s*<td>\d+</td>\s*<td>\w+</td>\s*<td>Exif\.(\w+)\.(\w+)</td>")
    return [(GROUP_BASE[g] + int(tag, 16), name) for tag, g, name in row.findall(page) if g in GROUP_BASE]


def gps_tags(path):
    page = open(path, encoding="utf-8").read()
    row = re.compile(r"<td title='0x([0-9a-f]{4}) = \d+'>0x[0-9a-f]{4}</td>\s*<td>(\w+)</td>")
    return [(0x1_0000 + int(tag, 16), html.unescape(name)) for tag, name in row.findall(page)]


def main(src, dst):
    tags = dict(exiv2_tags(f"{src}/exiv2.html") + EXTRA)
    tags.update(gps_tags(f"{src}/gps.html"))
    out = HEADER
    for tag, name in sorted(tags.items()):
        out += f'    ({tag:#x}, "{name}", "{words(name)}"),\n'
    open(dst, "w", encoding="utf-8").write(out + FOOTER)
    print(f"{len(tags)} tags")


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
