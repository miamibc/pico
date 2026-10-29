"""Writes licenses.txt: the free software and data Pico ships, for Help → About.

Usage (from the repository root): python3 -I crates/pico/assets/make_licenses.py

Lists every crate built into the app for Linux and Windows (normal
dependencies only: build tools and test crates are not shipped), then the
license texts found in those crates, each text once with the crates using it.
"""

import json
import os
import re
import subprocess
import sys

TARGETS = ["x86_64-unknown-linux-gnu", "x86_64-pc-windows-gnu"]
LICENSE_FILE = re.compile(r"^(licen[cs]e|copying|notice|ofl|ufl)", re.I)
OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "licenses.txt")

HEADER = """\
Pico is built on free software and open data. Thank you to their authors.

DATA AND RECIPES
  Place names: GeoNames (www.geonames.org), cities1000 and country names.
    License: CC BY 4.0, https://creativecommons.org/licenses/by/4.0/
  Countries, seas and continents: Natural Earth (www.naturalearthdata.com).
    Public domain.
  Camera makers' notes (MakerNote): tag names and value meanings from
    ExifRead's tables (github.com/ianare/exif-py), (c) Gene Cash, Ianaré
    Sévi and contributors. License: BSD-3-Clause.
  Creative Kit effect recipes follow the Picnik source by Google.
    License: Apache-2.0, https://www.apache.org/licenses/LICENSE-2.0

PROGRAMS PICO CALLS WHEN INSTALLED (not shipped with Pico)
  FFmpeg (ffmpeg, ffprobe, ffplay) for video: LGPL-2.1 or GPL-2.0,
    https://ffmpeg.org/legal.html
"""


def shipped_crates():
    crates = {}
    for target in TARGETS:
        meta = json.loads(
            subprocess.run(
                ["cargo", "metadata", "--format-version", "1", "--filter-platform", target],
                check=True,
                capture_output=True,
            ).stdout
        )
        packages = {p["id"]: p for p in meta["packages"]}
        nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
        root = next(p["id"] for p in meta["packages"] if p["name"] == "pico")
        seen, todo = set(), [root]
        while todo:
            node = todo.pop()
            if node in seen:
                continue
            seen.add(node)
            for dep in nodes[node]["deps"]:
                if any(kind["kind"] is None for kind in dep["dep_kinds"]):
                    todo.append(dep["pkg"])
        for node in seen:
            package = packages[node]
            if package["source"] is not None:
                crates[(package["name"], package["version"])] = package
    return [crates[key] for key in sorted(crates)]


def license_files(package):
    found = []
    top = os.path.dirname(package["manifest_path"])
    for folder in (top, os.path.join(top, "fonts"), os.path.join(top, "LICENSES")):
        if not os.path.isdir(folder):
            continue
        for name in sorted(os.listdir(folder)):
            path = os.path.join(folder, name)
            # Font folders keep each font's license next to it as a .txt.
            fonts = folder.endswith("fonts") and name.endswith(".txt")
            if os.path.isfile(path) and (LICENSE_FILE.search(name) or fonts):
                try:
                    found.append((name, open(path, encoding="utf-8").read().strip()))
                except UnicodeDecodeError:
                    pass
    return found


def main():
    crates = shipped_crates()
    lines = [HEADER, f"RUST CRATES ({len(crates)})"]
    texts = {}  # normalized text -> (file name, text, users)
    for package in crates:
        label = f"{package['name']} {package['version']}"
        license = package.get("license") or "see its license file"
        lines.append(f"  {label}: {license}")
        files = license_files(package)
        if not files:
            lines.append("    (no license file in the package; the standard text of the license applies)")
        for name, text in files:
            key = re.sub(r"\s+", " ", text)
            texts.setdefault(key, (name, text, []))[2].append(label)
    lines.append("")
    lines.append("LICENSE TEXTS")
    for name, text, users in sorted(texts.values(), key=lambda t: (t[0].lower(), t[2])):
        lines.append("")
        lines.append("=" * 72)
        lines.append(f"{name}, used by: {', '.join(users)}")
        lines.append("=" * 72)
        lines.append(text)
    with open(OUT, "w", encoding="utf-8") as out:
        out.write("\n".join(lines) + "\n")
    print(f"{len(crates)} crates, {len(texts)} license texts, {os.path.getsize(OUT)} bytes", file=sys.stderr)


if __name__ == "__main__":
    main()
