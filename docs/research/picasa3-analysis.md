# Picasa 3.9 — analysis for Pico

**English** · [Русский](picasa3-analysis.ru.md)

Goal: a clean-room implementation of Picasa 3's behavior in Pico (Rust +
egui). Google's strings, icons and resources are not copied into Pico; the
files in this folder are for reference only.

Source: `~/.wine/drive_c/Program Files/Google/Picasa3/` (build of 2014-03-11,
PE32 i386).

| File | What it is |
|---|---|
| `Picasa3.exe` (10 MB) | The main application, with its own UI engine (GDI/DirectDraw, fonts `runtime/*.ytf`) |
| `Picasa3i18n.dll` (27 MB) | Resources only: ~3000 XML files (`XMLF`) with strings in ~40 languages |
| `PicasaPhotoViewer.exe` | A separate fast viewer |
| `MovieThumb.exe` | Helper process: frames from videos (AVIFile + DirectShow) |
| `qtsupport.dll` | A bridge to Apple QuickTime for `.mov` |

REA Evidence: imports `ev_2647b822…`, i18n resources `ev_f8892e70…`.

## Data files in this folder

- `picasa-filters.txt` — every editor filter: ID in `.picasa.ini`, name,
  parameters.
- `picasa-menus.tsv` — items of all menus and context menus.
- `picasa-shortcuts.txt` — keyboard shortcuts.
- `picasa-strings-en.tsv` — every English UI string (4573 of them).

## Image and video formats

- Photos: jpg/jpeg, png, gif, bmp, tif/tiff, psd, webp, tga.
- RAW: 3fr arw cr2 crw dcr dng erf kdc mef mrw nef nrw orf pef raf raw rw2 sr2 srf x3f.
- Video: 3g2 3gp asf avi divx m2t m2ts m4v mkv mmv mod mov mp4 mpeg mpg mts mxf tod wmv.
- Each type is enabled in the settings (`SupportBMP`, `SupportPSD`,
  `SupportTIF`, `SupportWEBP`, `SupportPNG`, `SupportGIF`, `SupportMovies`).

Video in Picasa: thumbnails, playback (`/` pause, `,` back, `.` forward),
trimming the start/end (filters `moviestart`/`movieend`, no re-encoding), a
frame snapshot to JPEG (`%s.%d.jpg`), "Create Movie" from photos and videos
with export (`%s.%d.wmv`, `%s.%d.mov`). Decoding uses the OS codecs.

## `.picasa.ini`

Lives in every photo folder. INI, CRLF, and when there are non-ASCII
characters, an `[encoding] utf8=1` section. Format strings found in
`Picasa3.exe`:

```ini
[Picasa]
name=%s
description=%s
location=%s
category=%s          ; e.g. "Folders on Disk"
P2category=%s        ; example from disk: "Exported Pictures"
date=%f              ; OLE Automation date (days since 1899-12-30), e.g. 43880.743600

[.album:<token>]     ; Picasa albums (virtual); token = hex id
name= / date= / location= / category= / description= / token=

[Contacts2]          ; people for face recognition: <contactid>=<Name>;;

[<file name>]
star=yes
caption=%s
keywords=%s          ; comma-separated
rotate=rotate(%d)    ; 0..3 = ×90°
crop=rect64(%s)      ; old format
filters=<id>=<params>;<id>=<params>;...   ; edit stack, format "%s=%s;"
faces=rect64(%I64x),<contactid>;...      ; faces
albums=<token>,<token>
backuphash=%s
redo=...             ; undone edits for redo
moddate=...
textactive=...
IIDLIST_%s=%s
```

`rect64(hex)` is a 64-bit number: four 16-bit values (left, top, right,
bottom), each a fraction of the width/height multiplied by 65535 (the commonly
known description, not checked against the binary).

### The `filters=` format (confirmed by decompilation)

The serializer is `FUN_008fdab0`, the filter description parser is
`FUN_009023c0` (`research/picasa3/decomp/functions/`). Descriptions of all
filters are in `Picasa3/runtime/filterdesc.xml`. The generated parameter table
is `picasa-filter-layout.tsv` (script `research/scripts/filter_layout.py`).

Each edit: `<id>=<0|1>` (the `%c` flag), then comma-separated parameters,
ending with `;`:

1. `,%f,%f` — position of the "puck" cursor (the effect's center), if
   `<cursor persist="1">`.
2. `,%f` — sliders 0, 1, 2 (unless the slider has `persist="0"`).
3. `,%08x` — a color, if there is a `<colorcircle>` (neutral color picker).
4. `,%08x` — one color per `<colorwheel>`.
5. `,%f` — slider 3.
6. `,%d` — rotation 0..3, if the filter has `rotate="1"` (Graduated Tint).
7. Then a list of strokes/areas (format `,%f:%g:%g:%d[:%g:%g][:%g|%g|…]`);
   which filters use it is not known yet.

Special cases: `crop64=1,<hex>` — 64 bits, four 16-bit fields from left to
right (high bits first): left, top, right, bottom, fractions of 65535;
`rot=1,<%d>`.

Example: `finetune2=1,0.000000,0.000000,0.190000,fff7e7e6,0.000000;` =
fill light, highlights, shadows, neutral color, color temperature.

### Checked against real `.picasa.ini` files from disk (2026-10-26)

9 files found, three of them with edits. Examples (file names omitted):

```
filters=crop64=1,b042c64ffff9a6e;finetune2=1,0.134503,0.053333,0.101053,00000000,0.333333;unsharp=1;
filters=unsharp=1;crop64=1,a5c0050fb46f104;finetune2=1,0.561404,0.148772,0.188070,ff686761,-0.450292;sat=1,0.005848;
filters=...;tint=1,255.000000,fffdfd69;...;tilt=1,0.691700,0.000000;crop64=1,e2c0c2dffff9797;
filters=...;radblur=1,0.500000,0.500000,1.000000,-0.497076;...
filters=...;glow=1,0.650000,3.000000;
filters=crop64=1,a4f0a44f56ef546;HDR=1,6.362573,1.771930,19.298244;...;Vignette=1,50.000000,1.400000,84.210526,00000000;
crop=rect64(b042c64ffff9a6e)
rotate=rotate(3)
moddate=ba8726788231d401
backuphash=60720
```

Conclusions:
- The order of fields matches the table (`finetune2` — 3 sliders, a color,
  slider 3; `radblur` — x, y of the center, then 2 sliders; `tint` — a slider,
  a color).
- Values are stored already in the UI scale: from `-offset` to `range-offset`
  (`sat=-0.0175`, temperature `-0.45`, `tint` color preservation = 255).
- Parameters may be missing: `unsharp=1;` without a value (the "Sharpen" menu
  item with default settings). They must be read with defaults filled in from
  `filterdesc.xml`.
- The same edit can appear in the stack several times (`finetune2` and
  `crop64` repeat) — it is history, applied in order.
- `crop64` is written without leading zeros (`cbc3ffff` = left 0, top 0,
  right 0xcbc3, bottom 0xffff). The last `crop64` is duplicated by the
  `crop=rect64(...)` key.
- Picnik effects store their parameters too: the values of the MXML controls
  in document order, then colors (`HDR` = radius, contrast, fade; `Vignette` =
  3 numbers + a color).
- `moddate` is a FILETIME in hex, bytes little-endian.
- `.picasaoriginals/.picasa.ini` contains `width`, `height`, `moddate`,
  `textactive`.

**Still not checked:** the `redo=` format.

### Video trimming: `moviestart` / `movieend` (from the code)

This is not a generic filter but the `CTimeFilter` class (a subclass of
`CSimpleFilter`, vtable `0xc7af2c`). The parameter is a single 64-bit time,
format `%I64x` (`FUN_0044d670`), with no `0/1` flag:

```
filters=moviestart=<hex>;movieend=<hex>;
```

Time is in 100 ns units (like DirectShow's `REFERENCE_TIME`): the player code
divides it by 10,000,000 to get seconds (`0x78f6c0`, `0x6986e0`). Example:
5 seconds = `2faf080`. Reading: `FUN_0049c670`, `FUN_0049ce00` (they look for
`moviestart=` in the string and parse `%I64x`).

### Faces: `faces=` and `[Contacts2]` (from the code)

`FUN_004d2550`:

```
faces=rect64(<hex>),<contact id hex>;rect64(<hex>),<contact id hex>
```

`rect64` is encoded the same way as `crop64` (left, top, right, bottom in 16
bits each, fractions of 65535). Faces without a name are not written to the
INI.

People's names are a section in the same `.picasa.ini` (`FUN_00637ed0`):

```
[Contacts2]
<contact id hex>=<full name>;<email>;<gaia_id>
```

Picasa keeps the global contact list in `contacts.xml`. The internal database
(`imagedata_facerect.pmp`, `facetemplatesV2_*.db`) is a recognition cache that
Pico doesn't need. No face data was found in the `.picasa.ini` files on the
user's disk.

### Picnik effects: where the meaning of parameters comes from

1. **`filterdesc.xml`** describes each effect completely: controls with `id`,
   ranges and default values, and the operation with formulas these values are
   substituted into. Example:

   ```xml
   <HSliderFastDrag minimum="0" maximum="50" value="35" id="_sldrBlur"/>
   <HSliderFastDrag minimum="1" maximum="2" value="1.4" id="_sldrStr"/>
   <ImageColorSwatch id="_clrsw" color="0x000000"/>
   <HSliderPlus minimum="0" maximum="100" value="0" id="_sldrFade"/>
   <imageOperations:GlowImageOperation color="{_clrsw.color}"
       xblur="{_sldrBlur.value*.02*Math.max(imagewidth, imageheight)/4}" ...
       strength="{_sldrStr.value}" innerglow="true"
       BlendAlpha="{1-(_sldrFade.value/100)}"/>
   ```

   The INI gets the slider values in document order, then the colors:
   `Vignette=1,50.0,1.4,84.2,00000000` = blur, strength, fade, color (confirmed
   on one real example).
2. **`Picasa3.exe`** contains a table of readable control names
   (`_sldrFade` → "Fade", `_sldrSteps` → "Number of Colors"…), its own
   expression interpreter (`Math.min`, `Math.pow`, `imagewidth`, `zeroR`…) and
   native implementations of the operations.
3. **Picnik's sources are open** (Google, Apache 2.0): a clone at
   <https://github.com/darrinm/PicEditor>, locally `research/picnik-src/`
   (in .gitignore). `client/imagine/imageOperations/` has 73
   `*ImageOperation.as` classes with the algorithms (Glow, LocalContrast,
   Pixelate, QuantizePalette, IR…). The license allows porting the code to Pico
   while keeping the attribution notice (NOTICE).

### Other files in `Picasa3/runtime/`

`filterdesc.xml` (filters), `filters.txt` (folders skipped when scanning:
windows, winnt, temp, Program Files, Originals), `*.fen` (dialog layouts),
`constants.ui`, `properties.xml`, `respack.yt` (UI graphics). A copy is in
`research/picasa3/runtime/` (in .gitignore).

On `Save`, the original is moved into the `.picasaoriginals/` subfolder, and
the edits are "baked" into the file.

## Internal database (not required for Pico)

`AppData/Local/Google/Picasa2/db3/` is a columnar store: one `.pmp` file per
field (`imagedata_caption.pmp`, `imagedata_crop64.pmp`, `albumdata_name.pmp`…),
`thumbs*_index.db`, `bigthumbs_index.db`, `facetemplatesV2_*.db`. The source of
truth for the user is `.picasa.ini`; the database is a cache. The list of
watched folders: `Picasa2Albums/watchedfolders.txt`, exclusions:
`frexcludefolders.txt`.

## Features (by menu)

- **Library:** folder tree / flat list, sorting (date, name, size, recent
  changes), albums, people (faces), places (geo), timeline, search, filters by
  stars, videos, faces, hidden; color labels; the "tray" (Hold).
- **View:** small/normal thumbnails, captions (name, resolution, tags,
  caption), slideshow, full screen, EXIF properties.
- **Files:** add a folder or a file, import, rename (in batch), move, delete,
  "Locate on Disk", save, save a copy, export to a folder with resizing, undo
  all edits.
- **Editor:** tabs "Basic Fixes" (crop, straighten, red eye, "I'm Feeling
  Lucky", auto contrast, auto color, retouch, text, fill light), "Tuning" (fill
  light, highlights, shadows, color temperature), "Effects" (sharpen, sepia,
  B&W, warmify, film grain, tint, saturation, soft focus, glow, filtered B&W,
  focal B&W, graduated tint) and the set of Picnik effects. Copying and pasting
  all effects, undo and redo.
- **Tools:** duplicate finder, changing date and time, backup, folder manager,
  people manager, geotagging, writing faces to XMP, collage, poster, contact
  sheet, HTML export, movie, printing.

## Not carried over (the user's decision)

Everything online: Picasa Web / Google+, YouTube, Blogger, FTP, e-mail, print
ordering, updates, online sync, importing from the cloud.
