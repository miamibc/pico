"""Demo library for the README and website screenshots, and their WebP copies.

    python3 -I docs/screenshots/make_screenshots.py library DIR
        Downloads the photos below from Wikimedia Commons (all CC0) into DIR,
        one folder per topic, and writes `.picasa.ini` files with folder
        descriptions, stars, captions and an album. Photos already there are
        kept.

    python3 -I docs/screenshots/make_screenshots.py webp SRC DST
        Converts the PNG screenshots in SRC into WebP in DST.

`make screenshots` runs both around the `website_shots` UI test. Needs Pillow.
"""

import io
import json
import os
import sys
import time
import urllib.parse
import urllib.request

from PIL import Image

UA = {"User-Agent": "picco-demo-screenshots/1.0 (https://github.com/miamibc/picco)"}

# Folder -> (file name, title on Wikimedia Commons). All are CC0.
PHOTOS = {
    "Autumn Forest": [
        ("179.jpg", "2021 Forest church Równica.jpg"),
        ("180.jpg", "Autumn Forest Path with Tall Trees.jpg"),
        ("181.jpg", "Autumn Forest Pathway with Fallen Leaves.jpg"),
        ("182.jpg", "Autumn Forest Wet Bark.jpg"),
        ("184.jpg", "Autumn forest 01 – Panorama (Andreas Mischok via Poly Haven).jpg"),
        ("185.jpg", "Autumn forest 02 – Panorama (Andreas Mischok via Poly Haven).jpg"),
        ("186.jpg", "Autumn forest 03 – Panorama (Andreas Mischok via Poly Haven).jpg"),
        ("189.jpg", "Beautiful autumn forest (Unsplash).jpg"),
    ],
    "Breakfasts": [
        ("271.jpg", "All Day Breakfast 5 - Lizzie's Cafe 2024-11-25.jpg"),
        ("272.jpg", "All Day Breakfast Set 3 - Breakfast At Tiffany's Brighton 2025-02-16.jpg"),
        ("273.jpg", "All Day Breakfast Set 4 - Breakfast At Tiffany's 2025-07-29.jpg"),
        ("274.jpg", "BUNGAROOSH BREAKFAST - Bungaroosh 2025-10-01.jpg"),
        ("275.jpg", "Bluebird Cafe - June 2024 - Sarah Stierch 07.jpg"),
        ("276.jpg", "Breakfast At Tiffany's Brighton menu.jpg"),
        ("277.jpg", "Breakfast at Coffee 1, North Street, Taunton 2025-07-28.jpg"),
        ("278.jpg", "Breakfast at Ikea Vantaa.jpg"),
        ("280.jpg", "Breakfast in Le Grand St Marc, Rouen 2025-03-23.jpg"),
    ],
    "Cats": [
        ("001.jpg", "'Radhe' the pet cat of Worli fish marke in Mumbai..(Wednesday 14-3-2012).JPG"),
        ("019.jpg", "Cat Bathing in Sunbeam.jpg"),
        ("027.jpg", "Domestic shorthair cat portrait in grass.jpg"),
        ("028.jpg", "Domestic tabby shorthair kitten.jpg"),
        ("030.jpg", "Orange tabby cat-932269.jpg"),
        ("036.jpg", "Tabby cat with blue eyes-3336579.jpg"),
        ("038.jpg", "Young tabby cat keeping watch.jpg"),
        ("071.jpg", "A curious kitten (Pixabay).jpg"),
        ("072.jpg", "A focused kitten (Pixabay).jpg"),
        ("073.jpg", "A kitten in a carton (Flickr).jpg"),
        ("074.jpg", "A kitten in the hand (Flickr).jpg"),
        ("075.jpg", "A kitten on the lawn (Pixabay).jpg"),
    ],
    "Flowers": [
        ("310.jpg", "20250818 garden flowers Katowice.jpg"),
        ("311.jpg", "20260906 140251 Encore Garden Flower.jpg"),
        ("318.jpg", "Flower garden Hopewell Furnace.jpg"),
        ("321.jpg", "Flowers at English Garden in Taliesin 10.jpg"),
        ("322.jpg", "Flowers at English Garden in Taliesin 11.jpg"),
        ("323.jpg", "Flowers at English Garden in Taliesin 12.jpg"),
        ("324.jpg", "Flowers at English Garden in Taliesin 13.jpg"),
        ("325.jpg", "Flowers at English Garden in Taliesin 14.jpg"),
        ("326.jpg", "Flowers at English Garden in Taliesin 16.jpg"),
        ("327.jpg", "Flowers at English Garden in Taliesin 17.jpg"),
    ],
    "Lighthouses": [
        ("144.jpg", "Cape Reinga Light NZ (25676793788).jpg"),
        ("145.jpg", "Cape Reinga Lighthouse. NZ (20193721576).jpg"),
        ("148.jpg", "Fort Gratiot Light state historic marker.jpg"),
        ("149.jpg", "Fort Saint Nicolas Rhodes.jpg"),
        ("150.jpg", "Fugui-2025.jpg"),
        ("151.jpg", "Izola - Petelinji rt s svetilnikom (2014).jpg"),
        ("154.jpg", "Mare a Trieste fotografato dalla riva 3 novembre.jpg"),
        ("159.jpg", "Paull Lighthouse, on the side of the River Humber, near Hull, UK 01.jpg"),
        ("160.jpg", "Paull Lighthouse, on the side of the River Humber, near Hull, UK 02.jpg"),
        ("161.jpg", "Pigeon Point Lighthouse, Pescadero, Vereinigte Staaten Jul 12, 2025 09-44-24 AM.jpeg"),
    ],
    "Mountain Lakes": [
        ("080.jpg", "Bear Lake, August 2024.jpg"),
        ("081.jpg", "Bierstadt Lake Trail, Rocky Mountain National Park, 2009.jpg"),
        ("083.jpg", "Cloudscape above Bierstadt Lake, Rocky Mountain National Park, 2009.jpg"),
        ("086.jpg", "Golden Bull Mountain Ridge Park - lake, center - 01.jpg"),
        ("087.jpg", "Half-frozen mountain lake (Unsplash).jpg"),
        ("088.jpg", "Kayaks on a mountain lake (Unsplash).jpg"),
        ("091.jpg", "Lake Mountain Landscape.jpg"),
        ("092.jpg", "Mountain Lake Dusk.jpg"),
        ("099.jpg", "Mountain lake (Unsplash).jpg"),
        ("100.jpg", "Mountain lake dam.jpg"),
        ("102.jpg", "Peaceful mountain lake (Unsplash).jpg"),
        ("104.jpg", "Red kayaks on a mountain lake (Unsplash).jpg"),
        ("105.jpg", "Ripples on a mountain lake (Unsplash).jpg"),
    ],
    "Sunsets": [
        ("197.jpg", "Beach sunset 2022 Clearwater beach.jpg"),
        ("198.jpg", "Brighton beach at sunset 2025-02-27.jpg"),
        ("201.jpg", "Conus Way Park Sunset Beach.jpg"),
        ("202.jpg", "El Yaque beach sunset 2.jpg"),
        ("203.jpg", "El Yaque beach sunset 3.jpg"),
        ("204.jpg", "Juan Griego Sunset.jpg"),
        ("206.jpg", "Lovely beach view with sunset - July 2025.jpg"),
        ("207.jpg", "Marshall Beach Sunset (6753666737).jpg"),
        ("209.jpg", "Sunset, Lone Ranch Beach, Oregon.jpg"),
    ],
    "Winter": [
        ("387.jpg", "2010.01 - 'Photo of Dutch Winter nature', Dutch snow-landscape in the dunes , winter of 2010; Nature photography in the Netherlands by Fons Heijnsbroek, in the public domain; geotagged landscape picture in The Netherlands (27114819613).jpg"),
        ("388.jpg", "2010.01 - 'Photo of Dutch Winter nature', a view on a Dutch winter landscape with snow, in the dunes area around Haarlem; nature photography in the public domain by Fons Heijnsbroek, The Netherlands (27696197256).jpg"),
        ("393.jpg", "Local bar outdoor seating during winter snow two.jpg"),
        ("394.jpg", "Rochester road during winter snow.jpg"),
        ("395.jpg", "Snow-Covered Patio Furniture in a Winter Garden.jpg"),
        ("398.jpg", "Snow crystals glittering in strong direct sunlight 42 - wide crop - high contrast.jpg"),
        ("404.jpg", "Trees in winter with snow -- 20 of 33.jpg"),
        ("407.jpg", "Winter (The Vicarage Garden under Snow) - Van Gogh.jpg"),
        ("408.jpg", "Winter snow Mt Hood, Oregon (32345466421).jpg"),
    ],
}

DESCRIPTIONS = {
    "Mountain Lakes": "Still water, high peaks and a lot of walking",
    "Cats": "The real owners of the house",
    "Lighthouses": "Towers that keep watch over the sea",
    "Sunsets": "The last light of the day",
    "Breakfasts": "The most important meal, photographed before eating",
    "Flowers": "Gardens in bloom",
    "Winter": "Snow, frost and quiet streets",
    "Autumn Forest": "Gold and red leaves underfoot",
}
CAPTIONS = {
    "Mountain Lakes": "Calm morning on the lake",
    "Cats": "Who, me?",
    "Sunsets": "Golden hour",
    "Lighthouses": "Watching over the coast",
}
# Photos (by position in the folder) in the album "Best of the Year".
ALBUM = {"Mountain Lakes": [1, 4], "Cats": [0, 6], "Lighthouses": [1], "Sunsets": [0, 5], "Flowers": [4], "Autumn Forest": [0]}
ALBUM_TOKEN = "f00d1234"


def urls(titles):
    """Original file URLs by title, asked from the Commons API 40 at a time."""
    found = {}
    for start in range(0, len(titles), 40):
        chunk = titles[start : start + 40]
        query = urllib.parse.urlencode(
            dict(action="query", format="json", prop="imageinfo", iiprop="url", titles="|".join("File:" + t for t in chunk))
        )
        request = urllib.request.Request("https://commons.wikimedia.org/w/api.php?" + query, headers=UA)
        reply = json.load(urllib.request.urlopen(request, timeout=60))
        names = {n["to"]: n["from"] for n in reply["query"].get("normalized", [])}
        for page in reply["query"]["pages"].values():
            title = names.get(page["title"], page["title"])[len("File:") :]
            if "imageinfo" in page:
                found[title] = page["imageinfo"][0]["url"]
    return found


def download(url):
    for attempt in range(6):
        try:
            return urllib.request.urlopen(urllib.request.Request(url, headers=UA), timeout=120).read()
        except Exception as error:  # Commons asks to back off now and then.
            last = error
            time.sleep(5 * (attempt + 1))
    raise last


def without_zero_gps(exif):
    """Drops GPS that is all zeros: the camera had no fix."""
    gps = exif.get_ifd(0x8825)
    if not gps:
        return False

    def value(r):
        try:
            return float(r)
        except ZeroDivisionError:
            return 0.0

    lat, lon = gps.get(2), gps.get(4)
    zero = lat is None or lon is None or all(value(x) == 0 for x in (*lat, *lon))
    if not zero:
        return False
    exif.get_ifd(0x8769)  # load the Exif IFD so it is written back
    del exif[0x8825]
    return True


def library(root):
    titles = [t for files in PHOTOS.values() for _, t in files]
    found = urls(titles)
    for folder, files in PHOTOS.items():
        os.makedirs(os.path.join(root, folder), exist_ok=True)
        for name, title in files:
            out = os.path.join(root, folder, name)
            if os.path.exists(out):
                continue
            if title not in found:
                print("not on Commons any more:", title)
                continue
            image = Image.open(io.BytesIO(download(found[title])))
            exif = image.getexif()
            without_zero_gps(exif)
            image.thumbnail((2048, 2048))
            image.convert("RGB").save(out, quality=90, exif=exif.tobytes())
            print("got", folder, name, flush=True)
            time.sleep(1)
        write_ini(root, folder, [name for name, _ in files])


def write_ini(root, folder, files):
    lines = ["[Picasa]", f"name={folder}", f"description={DESCRIPTIONS[folder]}"]
    if folder == "Mountain Lakes":
        lines += [f"[.album:{ALBUM_TOKEN}]", "name=Best of the Year", f"token={ALBUM_TOKEN}"]
    for k, name in enumerate(sorted(files)):
        section = []
        if k % 3 == 0:
            section.append("star=yes")
        if k == 1 and folder in CAPTIONS:
            section.append(f"caption={CAPTIONS[folder]}")
        if k in ALBUM.get(folder, []):
            section.append(f"albums={ALBUM_TOKEN}")
        if section:
            lines += [f"[{name}]"] + section
    with open(os.path.join(root, folder, ".picasa.ini"), "w", newline="") as f:
        f.write("\r\n".join(lines) + "\r\n")


def webp(src, dst):
    os.makedirs(dst, exist_ok=True)
    for name in sorted(os.listdir(src)):
        if name.endswith(".png"):
            out = os.path.join(dst, name[: -len(".png")] + ".webp")
            Image.open(os.path.join(src, name)).save(out, quality=88, method=6)
            print(out)


if __name__ == "__main__":
    if sys.argv[1:2] == ["library"] and len(sys.argv) == 3:
        library(sys.argv[2])
    elif sys.argv[1:2] == ["webp"] and len(sys.argv) == 4:
        webp(sys.argv[2], sys.argv[3])
    else:
        sys.exit(__doc__)
