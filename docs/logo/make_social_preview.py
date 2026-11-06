"""Draws docs/logo/social-preview.png, the picture GitHub shows when the
repository is linked (Settings > General > Social preview): 1280 x 640.

The light logo on a dark backdrop, a line about Picco under it, and a
screenshot of the library leaning out of the right edge.

Usage (from the repository root, needs Inkscape, Pillow and the Kamerik 105
font the logo is set in): python3 -I docs/logo/make_social_preview.py
"""

import os
import subprocess
import sys
import tempfile

from PIL import Image, ImageDraw, ImageFilter, ImageFont

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
LOGO = os.path.join(ROOT, "docs/logo/picco-logo-with-title-paths-dark.svg")
SHOT = os.path.join(ROOT, "docs/screenshots/picco-1.webp")
OUT = os.path.join(ROOT, "docs/logo/social-preview.png")
FONT = os.path.expanduser("~/.local/share/fonts/Kamerik105-Light.otf")

W, H = 1280, 640
TEXT = (226, 232, 240)
DIM = (148, 163, 184)


def backdrop():
    """A dark slate, a little lighter towards the top left."""
    img = Image.new("RGB", (W, H))
    px = img.load()
    for y in range(H):
        for x in range(W):
            t = min(1.0, (x / W) * 0.6 + (y / H) * 0.4)
            px[x, y] = tuple(round(a + (b - a) * t) for a, b in zip((30, 41, 59), (15, 23, 42)))
    return img


def logo(width):
    with tempfile.TemporaryDirectory() as tmp:
        png = os.path.join(tmp, "logo.png")
        subprocess.run(["inkscape", LOGO, "-w", str(width), "-o", png], check=True, capture_output=True)
        return Image.open(png).convert("RGBA")


def screenshot(width):
    shot = Image.open(SHOT).convert("RGBA")
    shot = shot.resize((width, round(shot.height * width / shot.width)), Image.LANCZOS)
    mask = Image.new("L", shot.size, 0)
    ImageDraw.Draw(mask).rounded_rectangle((0, 0, *shot.size), radius=14, fill=255)
    shot.putalpha(mask)
    # A thin light edge sets the dark window off the dark backdrop.
    ImageDraw.Draw(shot).rounded_rectangle((0, 0, shot.width - 1, shot.height - 1), radius=14, outline=(100, 116, 139), width=2)
    return shot


def main():
    if not os.path.exists(FONT):
        sys.exit(f"needs the logo's font: {FONT}")
    img = backdrop().convert("RGBA")

    # The screenshot, with a soft shadow, cut by the right edge.
    shot = screenshot(780)
    at = (630, (H - shot.height) // 2 + 10)
    shadow = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    shape = Image.new("L", shot.size, 0)
    ImageDraw.Draw(shape).rounded_rectangle((0, 0, *shot.size), radius=14, fill=170)
    shadow.paste((0, 0, 0, 255), (at[0] + 6, at[1] + 14), shape)
    img.alpha_composite(shadow.filter(ImageFilter.GaussianBlur(18)))
    img.alpha_composite(shot, at)

    mark = logo(490)
    img.alpha_composite(mark, (48, 160))

    draw = ImageDraw.Draw(img)
    big = ImageFont.truetype(FONT, 28)
    small = ImageFont.truetype(FONT, 22)
    y = 160 + mark.height + 26
    for line in ("A fast, offline photo organizer", "that keeps your Picasa library alive"):
        draw.text((62, y), line, font=big, fill=TEXT)
        y += 38
    draw.text((62, y + 16), "Linux  ·  Windows  ·  macOS", font=small, fill=DIM)

    img.convert("RGB").save(OUT, optimize=True)
    print(OUT)


if __name__ == "__main__":
    main()
