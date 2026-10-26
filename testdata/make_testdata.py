"""Builds testdata/library: folders covering the .picasa.ini variants Pico must handle.

Run from anywhere:  python3 testdata/make_testdata.py [SOURCE_PICTURES_DIR]
Needs Pillow and ffmpeg. Source photos default to ~/Pictures; only pictures
without people are used. The library is rebuilt from scratch each time.

Every INI is written as exact bytes, so line endings and encodings are
deliberate: CRLF (as Picasa writes) unless a folder says otherwise.
"""

import shutil
import subprocess
import sys
from pathlib import Path

from PIL import Image

ROOT = Path(__file__).resolve().parent / "library"
SRC = Path(sys.argv[1]) if len(sys.argv) > 1 else Path.home() / "Pictures"

GIRAFFE = SRC / "zhiraff.png"
ISCHIA = SRC / "ischia.jpg"
TYPEWRITER = SRC / "typewriter-2653187_1920.jpg"
ROSE = SRC / "rose.jpg"
OWL = SRC / "blueowl_lg.jpg"
PALM = SRC / "palm_tree.png"  # has transparency
DOT = SRC / "dot.jpg"  # grayscale JPEG
TRIANGLES = SRC / "traingles.png"
CAGE = SRC / "kletka.jpg"  # portrait


def ini(folder: Path, name: str, text: str, encoding: str = "utf-8", newline: str = "\r\n"):
    data = text.strip("\n").replace("\n", newline) + newline
    (folder / name).write_bytes(data.encode(encoding))


def copy(src: Path, folder: Path, name: str | None = None) -> str:
    name = name or src.name
    shutil.copyfile(src, folder / name)
    return name


def folder(name: str) -> Path:
    path = ROOT / name
    path.mkdir(parents=True)
    return path


def exif_oriented(src: Path, dst: Path, orientation: int):
    """Stores pixels transformed so that applying `orientation` shows the original upright."""
    im = Image.open(src).convert("RGB")
    im.thumbnail((1200, 1200))
    # Inverse of the EXIF display transform for each orientation value.
    inverse = {
        1: [],
        2: [Image.Transpose.FLIP_LEFT_RIGHT],
        3: [Image.Transpose.ROTATE_180],
        4: [Image.Transpose.FLIP_TOP_BOTTOM],
        5: [Image.Transpose.TRANSPOSE],
        6: [Image.Transpose.ROTATE_90],
        7: [Image.Transpose.TRANSVERSE],
        8: [Image.Transpose.ROTATE_270],
    }[orientation]
    for op in inverse:
        im = im.transpose(op)
    exif = Image.Exif()
    exif[0x0112] = orientation
    im.save(dst, quality=90, exif=exif)


def main():
    if ROOT.exists():
        shutil.rmtree(ROOT)
    ROOT.mkdir(parents=True)

    # 01: what current Picasa writes: .picasa.ini, CRLF, UTF-8 with the marker.
    f = folder("01-modern-utf8")
    for src in (ISCHIA, TYPEWRITER, ROSE, OWL):
        copy(src, f)
    ini(f, ".picasa.ini", """
[Picasa]
name=Отпуск на Искье
description=Modern .picasa.ini with UTF-8, stars, keywords, albums and faces
location=Ischia, Italy
category=Folders on Disk
date=43880.743600
[.album:4f2a9c1e0b7d6a53]
name=Best of 2020
token=4f2a9c1e0b7d6a53
date=2020-03-01T10:00:00+01:00
[ischia.jpg]
star=yes
caption=Вид на залив 🌊
keywords=sea,italy,вид
albums=4f2a9c1e0b7d6a53
filters=enhance=1;
backuphash=60720
[typewriter-2653187_1920.jpg]
filters=unsharp=1;crop64=1,a5c0050fb46f104;finetune2=1,0.561404,0.148772,0.188070,ff686761,-0.450292;sat=1,0.005848;
crop=rect64(a5c0050fb46f104)
moddate=ba8726788231d401
[rose.jpg]
star=yes
rotate=rotate(2)
filters=bw=1;
[blueowl_lg.jpg]
faces=rect64(3f845bcb59418507),8e62398ebda8c1a5
filters=finetune2=1,0.707602,0.000000,0.098246,00000000,0.263158;sat=1,-0.017544;
[Contacts2]
8e62398ebda8c1a5=Сова Совушкина;owl@example.com;
[encoding]
utf8=1
""")

    # 02: the older file name, capitalised and without the dot, ANSI (cp1252).
    f = folder("02-legacy-Picasa.ini")
    for src in (OWL, ROSE):
        copy(src, f)
    ini(f, "Picasa.ini", """
[Picasa]
name=Café crème
description=Legacy Picasa.ini written in Windows-1252 without an encoding marker
[blueowl_lg.jpg]
caption=Hibou à l'œil vif – 5€
star=yes
filters=sepia=1;
[rose.jpg]
rotate=rotate(1)
""", encoding="cp1252")

    # 03: lowercase name with Unix line endings (copied through Linux/Mac tools).
    f = folder("03-lowercase-picasa.ini-LF")
    for src in (ISCHIA, DOT):
        copy(src, f)
    ini(f, "picasa.ini", """
[Picasa]
name=Lowercase name, LF endings
[ischia.jpg]
star=yes
filters=warm=1;
[dot.jpg]
caption=Grayscale JPEG
""", newline="\n")

    # 04: Russian Windows: ANSI is cp1251, no encoding marker.
    f = folder("04-ansi-cp1251")
    copy(TYPEWRITER, f)
    copy(OWL, f)
    ini(f, ".picasa.ini", """
[Picasa]
name=Пишущая машинка
description=ANSI .picasa.ini from a Russian Windows (cp1251)
[typewriter-2653187_1920.jpg]
caption=Старая пишущая машинка
keywords=ретро,машинка
star=yes
[blueowl_lg.jpg]
caption=Сова
""", encoding="cp1251")

    # 05: both names present; Picasa 3 reads .picasa.ini.
    f = folder("05-both-dot-and-legacy")
    copy(ROSE, f)
    ini(f, ".picasa.ini", """
[Picasa]
name=Both files: .picasa.ini wins
[rose.jpg]
caption=From .picasa.ini (expected)
star=yes
""")
    ini(f, "Picasa.ini", """
[Picasa]
name=WRONG: legacy file was read
[rose.jpg]
caption=From Picasa.ini (wrong file)
""")

    # 06: EXIF orientations 1-8, some combined with Picasa rotate.
    f = folder("06-exif-orientation")
    for o in range(1, 9):
        exif_oriented(ISCHIA, f / f"exif-{o}.jpg", o)
    exif_oriented(CAGE, f / "exif-6-portrait.jpg", 6)
    ini(f, ".picasa.ini", """
[Picasa]
name=EXIF orientation (all should look upright, except the rotate ones)
[exif-6.jpg]
caption=EXIF 6 + Picasa rotate(1): turned once more clockwise
rotate=rotate(1)
[exif-8.jpg]
caption=EXIF 8 + Picasa rotate(3)
rotate=rotate(3)
""")

    # 07: edits Pico cannot draw yet must survive saving.
    f = folder("07-picnik-and-unknown-filters")
    for src in (ISCHIA, TYPEWRITER, OWL, ROSE):
        copy(src, f)
    ini(f, ".picasa.ini", """
[Picasa]
name=Picnik, Straighten and unknown filters
[ischia.jpg]
filters=crop64=1,a4f0a44f56ef546;HDR=1,6.362573,1.771930,19.298244;finetune2=1,0.274854,0.050526,0.084211,00000000,0.000000;Vignette=1,50.000000,1.400000,84.210526,00000000;
[typewriter-2653187_1920.jpg]
filters=finetune2=1,0.450292,0.000000,0.151579,00000000,0.590643;unsharp=1;radblur=1,0.500000,0.500000,1.000000,-0.497076;crop64=1,11410000dd40ffff;
[blueowl_lg.jpg]
filters=tint=1,255.000000,fffdfd69;tilt=1,0.691700,0.000000;glow=1,0.650000,3.000000;
[rose.jpg]
filters=bw=0;FutureFilter=1,42,cafebabe;sepia=1;
redo=sat=1,0.500000;
textactive=0
""")

    # 08: video with trim points (moviestart/movieend, hex 100 ns units).
    f = folder("08-video")
    subprocess.run(
        ["ffmpeg", "-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=640x360:rate=25:duration=6",
         "-f", "lavfi", "-i", "sine=frequency=440:duration=6", "-shortest",
         "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", str(f / "testsrc.mp4")],
        check=True,
    )
    shutil.copyfile(f / "testsrc.mp4", f / "trimmed.mp4")
    copy(ROSE, f)
    ini(f, ".picasa.ini", """
[Picasa]
name=Video
[trimmed.mp4]
caption=Trimmed to 3s-5s
filters=moviestart=1c9c380;movieend=2faf080;
star=yes
""")

    # 09: many formats, no INI at all.
    f = folder("09-formats-no-ini")
    im = Image.open(ISCHIA).convert("RGB")
    im.thumbnail((800, 800))
    for ext in ("png", "gif", "bmp", "tiff", "webp", "tga", "ppm"):
        im.save(f / f"ischia.{ext}")
    copy(PALM, f, "palm-transparent.png")
    copy(TRIANGLES, f, "triangles-rgba.png")
    (f / "not-a-photo.txt").write_text("Pico should ignore this file.\n")
    (f / "broken.jpg").write_bytes(b"\xff\xd8\xff\xe0 this is not really a JPEG")

    # 10: crop and rotate on the giraffe; each copy is a separate INI section.
    f = folder("10-crop-rotate-giraffe")
    head = "80000000ffff8000"  # left 0.5, top 0, right 1.0, bottom 0.5: the head
    names = {
        "giraffe-original.png": "",
        "giraffe-rotate1.png": "rotate=rotate(1)",
        "giraffe-rotate2.png": "rotate=rotate(2)",
        "giraffe-rotate3.png": "rotate=rotate(3)",
        "giraffe-crop-head.png": f"filters=crop64=1,{head};\ncrop=rect64({head})",
        "giraffe-crop-head-rotate1.png": f"filters=crop64=1,{head};\ncrop=rect64({head})\nrotate=rotate(1)",
        "giraffe-crop-head-rotate3.png": f"filters=crop64=1,{head};\ncrop=rect64({head})\nrotate=rotate(3)",
        "giraffe-crop-twice.png": "filters=crop64=1,7fffffff;crop64=1,3fff3fffbfffbfff;",  # left half, then its middle
        "giraffe-straighten.png": "filters=tilt=1,0.691700,0.000000;",
        "giraffe-crop-then-bw.png": f"filters=crop64=1,{head};bw=1;",
    }
    sections = ["[Picasa]", "name=Giraffe: crop and rotate"]
    for name, keys in names.items():
        copy(GIRAFFE, f, name)
        if keys:
            sections += [f"[{name}]", keys]
    ini(f, ".picasa.ini", "\n".join(sections))

    # 11: malformed INI: BOM, junk lines, keys before any section, empty values.
    f = folder("11-malformed-ini")
    copy(OWL, f)
    copy(ROSE, f)
    (f / ".picasa.ini").write_bytes(
        b"\xef\xbb\xbforphan=key before any section\r\n"
        b"; a comment\r\n"
        b"\r\n"
        b"[Picasa]\r\n"
        b"name=Malformed INI (BOM, junk, empty values)\r\n"
        b"this line has no equals sign\r\n"
        b"[blueowl_lg.jpg]\r\n"
        b"star=yes\r\n"
        b"caption=\r\n"
        b"rotate=rotate(9)\r\n"
        b"filters=;;bw=1;;\r\n"
        b"[rose.jpg]\r\n"
        b"rotate=garbage\r\n"
        b"crop=rect64(nothex)\r\n"
        b"filters=crop64=1,zzzz;\r\n"
        b"[missing-file.jpg]\r\n"
        b"star=yes\r\n"
    )

    # 12: nested folders, one deep folder, and a hidden backup folder.
    f = folder("12-nested/level-1/level-2")
    copy(OWL, f)
    copy(ROSE, ROOT / "12-nested")
    originals = ROOT / "12-nested" / ".picasaoriginals"
    originals.mkdir()
    copy(ROSE, originals)
    ini(originals, ".picasa.ini", """
[rose.jpg]
moddate=ba8726788231d401
width=1041
height=508
textactive=0
""")

    count = sum(1 for p in ROOT.rglob("*") if p.is_file())
    print(f"wrote {count} files to {ROOT}")


if __name__ == "__main__":
    main()
