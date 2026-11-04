# Test library

`library/` is a photo library for manual test runs of Pico; each folder covers
one way `.picasa.ini` files appear in the wild. It is generated, not committed:

```sh
python3 testdata/make_testdata.py        # needs Pillow and ffmpeg; photos from ~/Pictures
cargo run -p pico -- testdata/library    # open it in Pico
cargo test -p pico testdata_tour -- --ignored   # screenshots of every folder in target/ui-shots/
```

| Folder | What it checks |
|---|---|
| `01-modern-utf8` | `.picasa.ini` as Picasa 3 writes it: UTF-8 marker, stars, captions, keywords, albums, faces, contacts, real edit stacks |
| `02-legacy-Picasa.ini` | Old `Picasa.ini` name, Windows-1252 text without the marker |
| `03-lowercase-picasa.ini-LF` | `picasa.ini` in lower case with Unix line endings |
| `04-ansi-cp1251` | Russian Windows: Windows-1251 text without the marker |
| `05-both-dot-and-legacy` | Both files; `.picasa.ini` must win |
| `06-exif-orientation` | EXIF orientations 1–8, some combined with Picasa rotation |
| `07-picnik-and-unknown-filters` | Edits Pico cannot draw yet (Picnik, Straighten, unknown, disabled); must survive saving |
| `08-video` | Video thumbnails and trim points (`moviestart`/`movieend`) |
| `09-formats-no-ini` | Many formats, transparency, a broken JPEG, a non-photo file, no INI |
| `10-crop-rotate-giraffe` | Crop and rotation order, double crop, crop then B&W |
| `11-malformed-ini` | BOM, junk lines, keys outside sections, bad values, a missing file |
| `12-nested` | Nested folders and a hidden `.picasaoriginals` backup |

Expected looks are written into each folder's name or captions.

## Kept locally, not committed

These folders live in the project so they are at hand for every release and
never lost or downloaded again; they are too big for the repository.

| Folder | What it is | Where it came from |
|---|---|---|
| `demo-photos/` | The demo library of the README and website screenshots (`make screenshots`), 80 photos in 8 topics with `.picasa.ini` files | CC0 photos from Wikimedia Commons, listed in `docs/screenshots/make_screenshots.py`; `make screenshots` downloads only the ones missing |
| `data-download/` | What `make places`, `make exif-tags` and `make maker-tags` download (GeoNames, Natural Earth, exiv2 and ExifTool tag pages, ExifRead); its `exifread` tests hold a few more RAW samples | The sources in `docs/data-updates.md`; those targets download afresh each time |
| `raw-samples/` | Camera RAW files for `crates/pico-core/src/raw.rs` (its `real_raw_files` test reads them): Sony ILCE-7CM2 ARW, Nikon Z8 NEF | CC0 samples from [raw.pixls.us](https://raw.pixls.us/) (`Sony/ILCE-7CM2`, `Nikon/Z 8`) |
