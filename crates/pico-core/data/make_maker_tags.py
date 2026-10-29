#!/usr/bin/env python3
"""Writes crates/pico-core/src/maker_tags.rs, the names and value meanings of
the camera makers' notes (MakerNote), from ExifRead's tag tables.

    python3 -I make_maker_tags.py EXIF-PY-SOURCE OUT.rs

EXIF-PY-SOURCE is an unpacked ExifRead source tree (`make maker-tags` fetches
it). Its tables are Python dict literals; they are read with `ast`, never
run. Entries whose meaning is a Python function are kept by name only.
ExifRead is BSD-3-Clause, (c) Gene Cash, Ianaré Sévi and contributors.
"""

import ast
import re
import sys
from pathlib import Path

# File, table, the maker's name as Pico writes it, and the list (by the
# maker's tag) a table of places describes; 0 for the main directory.
TABLES = [
    ("apple.py", "TAGS", "APPLE", 0),
    ("canon.py", "TAGS", "CANON", 0),
    ("canon.py", "_CAMERA_SETTINGS", "CANON", 0x0001),
    ("canon.py", "_FOCAL_LENGTH", "CANON", 0x0002),
    ("canon.py", "_SHOT_INFO", "CANON", 0x0004),
    ("canon.py", "_AF_INFO", "CANON", 0x0012),
    ("canon.py", "_FILE_INFO", "CANON", 0x0093),
    ("casio.py", "TAGS", "CASIO", 0),
    ("dji.py", "TAGS", "DJI", 0),
    ("fujifilm.py", "TAGS", "FUJIFILM", 0),
    ("nikon.py", "TAGS_NEW", "NIKON", 0),
    ("olympus.py", "TAGS", "OLYMPUS", 0),
    ("sony.py", "TAGS", "SONY", 0),
]

# Places in lists that are not fixed, by table: AF Info's later fields follow
# lists as long as the number of AF points. (AF Info 2 is left out: its
# places are one off from ExifTool's, which the cameras bear out.)
LAST_FIXED = {"_AF_INFO": 7}

# Words that keep their capitals, or are written apart differently.
WORDS = {"Makernote": "Maker Note", "Whitebalance": "White Balance", "Fnumber": "F Number"}


def words(name):
    """`FlashExposureComp` → `Flash Exposure Comp`, `ISOSetting` → `ISO Setting`."""
    for joined, apart in WORDS.items():
        name = name.replace(joined, apart)
    name = re.sub(r"(?<=[a-z0-9])(?=[A-Z])", " ", name)
    name = re.sub(r"(?<=[A-Z])(?=[A-Z][a-z])", " ", name)
    return re.sub(r"\s+", " ", name.replace("_", " ")).strip()


def constant(node):
    """An int or str literal, negative numbers included; None otherwise."""
    if isinstance(node, ast.Constant) and isinstance(node.value, (int, str)) and not isinstance(node.value, bool):
        return node.value
    if isinstance(node, ast.UnaryOp) and isinstance(node.op, ast.USub):
        value = constant(node.operand)
        return -value if isinstance(value, int) else None
    return None


def meanings(node):
    """A dict literal of int → str, as [(value, text)]."""
    if not isinstance(node, ast.Dict):
        return []
    out = []
    for key, value in zip(node.keys, node.values):
        k, v = constant(key), constant(value)
        if isinstance(k, int) and isinstance(v, str):
            out.append((k, v))
    return out


def table(module, name):
    """The entries of table `name`: [(tag, name, [(value, text)])]."""
    for node in module.body:
        target = None
        if isinstance(node, ast.Assign) and len(node.targets) == 1:
            target, value = node.targets[0], node.value
        elif isinstance(node, ast.AnnAssign):
            target, value = node.target, node.value
        if not (isinstance(target, ast.Name) and target.id == name and isinstance(value, ast.Dict)):
            continue
        entries = []
        for key, item in zip(value.keys, value.values):
            tag = constant(key)
            if not isinstance(tag, int) or not isinstance(item, ast.Tuple) or not item.elts:
                continue
            title = constant(item.elts[0])
            if not isinstance(title, str) or not title:
                continue
            found = []
            for part in item.elts[1:]:
                found = found or meanings(part)
            entries.append((tag, words(title), found))
        return entries
    raise SystemExit(f"no table {name}")


def rust(text):
    return '"' + text.replace("\\", "\\\\").replace('"', '\\"') + '"'


def main():
    source, out = Path(sys.argv[1]), Path(sys.argv[2])
    notes = next(source.rglob("tags/makernote"))
    lines = [
        "//! The camera makers' notes: names and value meanings, from ExifRead's",
        "//! tag tables (BSD-3-Clause, (c) Gene Cash, Ianaré Sévi and contributors;",
        "//! the license is in `crates/pico-core/data/LICENSE-ExifRead`).",
        "//! Made by `crates/pico-core/data/make_maker_tags.py` (`make maker-tags`);",
        "//! do not edit.",
        "",
        "/// An entry: its tag (or place in a list), name and value meanings.",
        "pub(crate) struct Tag {",
        "    pub(crate) tag: u16,",
        "    pub(crate) name: &'static str,",
        "    pub(crate) values: &'static [(i64, &'static str)],",
        "}",
        "",
    ]
    index = []
    total = 0
    for file, name, maker, list_tag in TABLES:
        module = ast.parse((notes / file).read_text())
        entries = [e for e in table(module, name) if e[0] <= LAST_FIXED.get(name, 0xFFFF)]
        # "Canon Image Width" reads "Canon Canon Image Width" after the maker.
        entries = [(tag, re.sub(rf"^{maker} ", "", title, flags=re.I) or title, v) for tag, title, v in entries]
        total += len(entries)
        const = f"{maker}_{name.strip('_')}" if name != "TAGS" else maker
        const = const.replace("TAGS_NEW", "").strip("_") or maker
        index.append((maker, list_tag, const))
        lines.append(f"const {const}: &[Tag] = &[")
        for tag, title, values in sorted(entries):
            pairs = ", ".join(f"({v}, {rust(t)})" for v, t in values)
            lines.append(f"    Tag {{ tag: 0x{tag:04x}, name: {rust(title)}, values: &[{pairs}] }},")
        lines.append("];")
        lines.append("")
    lines.append("/// The table of `maker`'s main directory (`list` 0), or of the list under")
    lines.append("/// its tag `list`.")
    lines.append("pub(crate) fn table(maker: &str, list: u16) -> Option<&'static [Tag]> {")
    lines.append("    Some(match (maker, list) {")
    for maker, list_tag, const in index:
        lines.append(f'        ("{maker}", 0x{list_tag:04x}) => {const},')
    lines.append("        _ => return None,")
    lines.append("    })")
    lines.append("}")
    out.write_text("\n".join(lines) + "\n")
    print(f"{out}: {len(TABLES)} tables, {total} entries")


if __name__ == "__main__":
    main()
