<img src="docs/logo/picco-logo-with-title-paths.svg#gh-light-mode-only" width="100%" alt="Picco, the photo organizer you remember">
<img src="docs/logo/picco-logo-with-title-paths-dark.svg#gh-dark-mode-only" width="100%" alt="Picco, the photo organizer you remember">

**English** · [Русский](README.ru.md)

A small, fast, offline photo organizer for Linux, Windows and macOS that keeps
your Picasa library alive.

Google stopped Picasa in 2016, but millions of photo folders still carry its
`.picasa.ini` files with stars, captions, albums and edits. **Picco** is a new
photo organizer written from scratch in Rust that reads and writes those files,
so a library built in Picasa years ago opens in Picco as it was, and keeps
working the other way round.

Picco is small (a single file, no installer needed) and works completely
offline: nothing is uploaded anywhere, and originals are never touched until
you press **Save**.

<img src="docs/screenshots/picco-1.webp#gh-dark-mode-only" width="100%" alt="The library: every folder in one continuous ribbon">
<img src="docs/screenshots/picco-1-light.webp#gh-light-mode-only" width="100%" alt="The library: every folder in one continuous ribbon">

## Library

- **Folders, albums, timeline and places** in one continuous ribbon: where one
  folder ends, the next begins, with its title and description.
- **Places without the internet**: photos with GPS are grouped by the nearest
  town, otherwise by country, sea or continent. The place names (GeoNames,
  Natural Earth) are built into the program.
- Stars, captions, tags, albums with your own order, search, filters for
  starred photos and videos, duplicate finder.
- Drag and drop between folders and albums, selection with a rectangle,
  context menus, kinetic scrolling.

<img src="docs/screenshots/picco-4.webp#gh-dark-mode-only" width="100%" alt="Places: photos grouped by where they were taken">
<img src="docs/screenshots/picco-4-light.webp#gh-light-mode-only" width="100%" alt="Places: photos grouped by where they were taken">

## Editing

- **Basic fixes**: crop with ready-made proportions, straighten, *I'm Feeling
  Lucky*, auto contrast and auto color.
- **Tuning**: fill light, highlights, shadows, color temperature, with a live
  histogram.
- **Effects** from Picasa and the **Creative Kit** of Picnik: sepia, B&W,
  warmify, film grain, soft focus, glow, Lomo, Holga, HDR-ish, Polaroid, pencil
  sketch and more.
- Every edit is non-destructive and stored the way Picasa stored it;
  **Original** shows the photo before edits while the button is held.

<img src="docs/screenshots/picco-2.webp#gh-dark-mode-only" width="100%" alt="Editing: basic fixes, tuning and effects">
<img src="docs/screenshots/picco-2-light.webp#gh-light-mode-only" width="100%" alt="Editing: basic fixes, tuning and effects">

## Properties for the curious

Picco shows **every** metadata field: EXIF, XMP, IPTC, ICC, PNG, WebP, video
streams, and the **camera makers' notes** of Canon, Nikon, Olympus, Fujifilm,
Sony, Apple, DJI and others. Fields that should agree across metadata kinds
but don't get a **!** mark — an easy way to tell an edited file from one
straight out of the camera.

<img src="docs/screenshots/picco-3.webp#gh-dark-mode-only" width="100%" alt="Properties: every metadata field, checked against each other">
<img src="docs/screenshots/picco-3-light.webp#gh-light-mode-only" width="100%" alt="Properties: every metadata field, checked against each other">

## Create

- **Collages**: mosaic, frame mosaic, grid, contact sheet, multiple exposure.
- **Movies** from photos: MP4 in 720p or 1080p with smooth transitions and
  music (with FFmpeg installed).
- **Slideshows** that play videos too, **export** with downsizing and a choice
  of JPEG quality.

<img src="docs/screenshots/picco-5.webp#gh-dark-mode-only" width="100%" alt="Collage from the selected photos">
<img src="docs/screenshots/picco-5-light.webp#gh-light-mode-only" width="100%" alt="Collage from the selected photos">

## Everything else

- Light and dark themes that follow the system, English and Russian interface,
  ready for more languages.
- The source is open under the Apache 2.0 license.

*The photos in the screenshots are from Wikimedia Commons, released into the
public domain (CC0).*

## Downloads

The GitHub releases page has archives for every system. Intel and AMD
processors use the same `x86_64` (64-bit) or `x86` (32-bit) builds.

| System | 64-bit | 32-bit | ARM |
|---|---|---|---|
| Linux (glibc 2.28+, 2019 and newer) | `picco-linux-x86_64` | `picco-linux-x86` | `picco-linux-arm64`, `picco-linux-armv7` (32-bit, Raspberry Pi) |
| Windows 10/11 | `picco-windows-x86_64` | `picco-windows-x86` | `picco-windows-arm64` |
| macOS 11+ | `picco-macos` — one app for both Apple Silicon and Intel | — | — |

The macOS build is not signed with an Apple certificate: on first launch,
open `Picco.app` with right click → **Open** (or **System Settings →
Privacy & Security → Open Anyway**). For video, HEIC, AVIF and JPEG XL, install
ffmpeg with `brew install ffmpeg`, or put `ffmpeg`, `ffprobe` and `ffplay` into
`Picco.app/Contents/MacOS`.

Releases are built by `.github/workflows/release.yml`. For now it is enabled
only for Linux x86_64, arm64 and armv7, Windows x86_64 and macOS. Every push to `main` builds the
program once and updates the `latest` pre-release. If there is no release yet
for the version in `Cargo.toml`, the same archives become the `v<version>`
release, described by `docs/releases/v<version>.md`, and the tag is created
automatically. To ship a new version, bump it in `Cargo.toml`, add the release
notes and merge into `main`. The other systems are commented out in the file to
save Actions minutes; uncomment them to build everything.

## Building and running

You need Rust 1.95 or newer (`rustup`). Video, HEIC, AVIF and JPEG XL need
`ffmpeg` (together with `ffprobe`; `ffplay` for sound) in `PATH` or next to
the program. Everything else works without it, camera RAW files too: Picco
shows the JPEG the camera keeps inside them (Sony ARW, Nikon NEF, Canon CR2
and CR3, DNG, Fujifilm RAF and other TIFF-based RAWs).

```sh
make release          # or: cargo build --release
make run              # open ~/Pictures
./target/release/picco ~/Photos ~/Downloads   # open your own folders
```

The finished program is a single file, `target/release/picco`, which you can
copy anywhere. `make install` puts it into `~/.cargo/bin`.

**Windows.** `make windows` builds `picco.exe` right on Linux (via zig; setup
is described in a comment in the `Makefile`); on Windows itself a plain
`cargo build --release` works. The program needs only the system DLLs of
Windows 10/11. For video, put `ffmpeg.exe`, `ffprobe.exe` and `ffplay.exe`
next to `picco.exe`.

**macOS** is built on a Mac (`cargo build --release`) or in GitHub Actions.
`.github/workflows/build.yml` checks and builds Linux, Windows and macOS
(Intel and Apple Silicon) on every push and uploads the programs as artifacts.

Other commands: `make test` (tests), `make check` (formatting, clippy,
tests), `make testdata` (a test library in `testdata/library`, see
`testdata/README.md`).

## How to use

### Library

- On the left are **★ Starred Photos** (every starred photo), **Albums**
  (Picasa albums), **Timeline** (by year and month taken), **Places** (photos
  with GPS, grouped by place: the nearest city, otherwise the country, sea or
  ocean, continent; works offline) and **Folders**.
  Folders given on the command line or added with
  **File → Add Folder to Picco…** are remembered. Folder Manager shows the disk tree on the left and, for the
  selected folder, **Watch for photos** or **Leave out of Picco** on the right.
  The rule of the nearest folder wins: you can watch `~/Pictures`, leave out
  `~/Pictures/Screenshots`, and inside it turn one folder back on. The path can
  be typed in or picked with the system dialog (**Browse…**). The tree opens to
  every watched folder. Changes apply on **OK**; the files themselves are not
  touched.
- The **Photo Tray** is hidden until **View → Photo Tray** turns it on; albums
  do its job for most. Then it sits at the bottom: while it is empty, it shows
  the selection; **Hold** (**Ctrl+H**) puts the selected photos into the tray,
  and they stay there as you move between folders. The **Slideshow**,
  **Export…**, **Make a Collage…** and **Make a Movie…** buttons work on the
  tray (or on the selection if the tray is empty); **Unhold** takes photos out
  of it, **Clear** empties it. A **Photo Tray** section appears on the left. No
  files are moved.
- Thumbnail size: **View → Thumbnail size** or **Ctrl+wheel** over the photos
  (pinch on a touchpad); **Ctrl+1** / **Ctrl+2** — small and normal.
- The wheel scrolls the thumbnails with some weight: they glide on a little
  and spring back at the top and bottom (**Tools → Options… → Animations**
  turns it off).
- You can select with a rectangle: drag the mouse across the thumbnails (with
  **Ctrl** — add to the selection). Near the edge of the window the grid
  scrolls by itself.
- **Drag and drop.** Thumbnails (all selected ones at once) can be dropped on
  a folder on the left — Picco asks and moves the files together with their
  captions, stars, edits and saved original; on an album — to add them to the
  album; on the tray (when shown) — to put them in the tray. Folders (and files — by their
  folder) from a file manager can be dropped right into the Picco window, and
  they are added to the library.
- Right click on a thumbnail opens a menu: star, rotate, album, hide, export,
  locate on disk, rename, copy path, move to trash. If the photo was not
  selected, the menu works on it alone; otherwise on the whole selection.
  Right click on a folder on the left: slideshow, locate on disk, copy path,
  rescan, remove from Picco.
- The sections on the left (**Albums**, **Folders**, **Timeline**, **Places**)
  can be collapsed. The **…** button next to **Folders**: a flat list or a tree (folders where they sit on the disk), sorted
  by name, date (newest first, grouped by year), recent changes or size.
- Folders are shown on the right as one continuous ribbon, as in Picasa: each
  has a header with its name, photo count, months taken and description.
  Clicking a folder on the left scrolls the ribbon to it, and while scrolling
  the folder at the top is highlighted. Arrow keys, the viewer and the
  slideshow stay within one folder; the open photo stays in it even when it no
  longer matches a filter (say, unstarred while only starred ones are shown).
  Albums, months of the **Timeline** and
  **Places** follow one another in a ribbon of their own the same way; a photo
  clicked in the next one makes that one current.
- On the right are the thumbnails. Double click or **Enter** opens a photo;
  double click on the open photo (or **Esc**) returns to the library.
- Above the open photo is a filmstrip of the neighbouring shots; clicking a
  thumbnail opens it. The filmstrip can be hidden: **View → Filmstrip**. The
  bar below the photo takes more rows where the window is narrow.
- The mouse wheel over a photo flips through the shots, **Ctrl+wheel** (or a
  pinch on a touchpad) zooms around the pointer. At the bottom is a switch of
  three: **Fit** (the whole photo in the window), **1:1** (one photo pixel per
  screen pixel) and the zoom in percent with a slider from 30% to 300% inside
  it. The percent shows the zoom on screen; a click on it, the slider or
  Ctrl+wheel keep that zoom. The chosen mode stays from photo to photo and
  between runs. A zoomed photo can be dragged with the mouse. When you zoom
  into a large shot, Picco loads it at full resolution, so 1:1 shows real
  pixels. Two photos on each side are prepared in advance, so
  paging shows no blank frame.
- Above the photo is a line about it: name, date taken (from EXIF; for MP4 and
  MOV videos, when they were recorded; otherwise the file date), size in pixels (after a crop — the original
  one too) and file size; for a video — frame size and duration.
- **Original** at the bottom (or the **\\** key): while the button is held, you
  see the photo without edits — handy for comparing before and after.
- **Undo** (**Ctrl+Z**) removes the last edit, **Redo** (**Ctrl+Y** or
  **Ctrl+Shift+Z**) brings it back. After **Undo All Edits**, Redo brings the
  edits back one by one. A new edit clears the Redo history.
- **Home/End** — the first and the last photo. **F** or **F11** (or the **⛶** button) —
  full screen: only the photo, no menu or panels; arrows, wheel and zoom work,
  **Esc**, **F** or **F11** leaves.
- **Search** (the field at the top right, **Ctrl+F**) looks through the whole
  library: file and folder names, captions, keywords.
- The status bar at the bottom right: name, date taken and size of the photo
  under the cursor (or the selected one), and for several photos — how many
  are selected and how much they weigh. While Picco looks for photos, it also
  shows how many have been found so far.
- Badges in the corner of a thumbnail: **✏** — the photo has edits, **📍** —
  it is known where it was taken (the tooltip says the same).
- The **★** button next to the search shows only starred photos, **▶** — only
  videos.
- **View**: thumbnail size, captions under them (file name, caption, tags),
  sorting (by date taken — the default —, name, size, reversed), showing
  hidden photos, the Photo Tray, slideshow. A file without a date taken sorts
  by the date in its name (`IMG_20220421_153012`), else by the earlier of its
  creation and modification times; all dates are brought to the computer's
  time zone, so videos (dated in UTC) sit among the photos.
- **Alt+Enter** (or **View → Properties and Tags**) opens a panel on the right
  with the photo's histogram (red, green, blue, with the edits; the same in the
  library and the viewer), properties (size, resolution, camera, shutter speed,
  ISO, GPS…) and tags. Below them is all of the file's metadata by section (Picco
  remembers which are open and which are collapsed between runs): every EXIF
  field (internal and binary ones in hex), XMP, IPTC, the ICC profile, JPEG,
  PNG and WebP headers, video tags, and the camera maker's notes (MakerNote:
  Canon, Nikon, Olympus, Fujifilm, Sony, Casio, Apple, DJI), read out where
  their meaning is known. If fields with the same meaning disagree (author,
  title, description, rights, keywords, dates, software, rating, lens — in
  EXIF, XMP, IPTC and PNG; copies of EXIF tags in XMP; the camera's settings,
  serial numbers, owner and clock in the maker's notes against EXIF and XMP),
  each of them gets a "!" badge, and the tooltip shows all the values. Maker's
  notes moved by an editor without fixing them say so. Such traces tell an
  edited file from one straight out of the camera. The coordinates in
  the **GPS** row are gathered from EXIF, XMP and video tags and work as a
  link: they open the place on OpenStreetMap in the browser; if the sources
  disagree, they get a "!" badge too. The **📋 Copy** icon to the right of the
  header copies all of this as a table (TSV) to paste into a spreadsheet.
  **Ctrl+K** puts the cursor straight into the tag field: a tag is added to all
  selected photos, the button with a cross removes it.

### Albums

An album is a virtual collection of photos from any folders, as in Picasa:
which photo is in which album is recorded in the folders' `.picasa.ini`, and
the album itself (its name and the order of photos) in a `.pal` file, as
Picasa does. **Ctrl+N** or **Picture → Add to Album → New Album…** creates an
album from the selected photos, **Add to Album → name** adds them to an
existing one, **Remove from Album** or **Delete** (when the album is open)
takes them out of the album without touching the file. Photos in an album can
be rearranged by dragging — sorting switches to **View → Sort → Custom
Order**. Photos from folders that are not in Picco are shown as black tiles
with an **Add Folder** button. Right click on an album on the left — **Rename
Album…** and **Delete Album…** (the photos stay where they are).

Selection: click — one photo, **Ctrl+click** — add/remove, **Shift+click** —
a range (across the folders and albums of the ribbon, too), **Ctrl+A / Ctrl+I / Ctrl+D** — select all / invert / select none.
The **Picture** and **File** menu commands work on all selected photos.

### Viewing and editing photos

At the bottom is a bar: back to the library, **Edit**, slideshow, previous and
next, star, rotate and the caption field (saved on Enter).

The **Edit** button opens the editing panel:

- **Basic Fixes**: Crop (with a frame, with ready-made proportions),
  Straighten (level the horizon), I'm Feeling Lucky, Auto Contrast, Auto Color.
- **Tuning**: fill light, highlights, shadows, color temperature.
- **Effects**: Picasa effects (sharpen, sepia, B&W, warmify, film grain, tint,
  saturation, soft focus, glow, filtered B&W, focal B&W, graduated tint).
- **Creative Kit**: Picnik effects (HDR-ish, Lomo, Holga, vignette, frames,
  Polaroid, Pencil Sketch, Duo-Tone and more).

Effects with settings open a panel with sliders; the effect's center is set by
clicking the photo. **Enter** applies, **Esc** cancels. **Undo** removes the
last edit, **Undo All Edits** removes them all.

Edits are stored in `.picasa.ini`; the file itself does not change. To write
them into the file, use **File → Save** (the original is kept in
`.picasaoriginals`, **File → Revert** brings it back) or **Save a Copy**
(`name-1.jpg` appears next to it).

### Video

Videos open the same way as photos. **/** or **Space** — play/pause (or the
white ▶ over a paused video, or a click on the picture), **,** and **.** — 5
seconds back/forward, the slider seeks, **Mute** turns the sound off. When the
video plays, ▶ turns into ⏸, which fades a second later and comes back for two
seconds when the mouse moves over the picture. Frames are decoded at the
video's own size, up to the screen's. **Save Frame as Photo** in the bottom bar saves
the current frame next to the video. In a slideshow a video plays to the end, from its start, and crossfades into
the next slide while it still plays.
A video turned with the rotate buttons plays turned, in slideshows too, and
its saved frames are turned the same way. Videos are not edited in Picco:
**Edit** is greyed out for them.

### Movie from photos

**Create → Movie…** makes an MP4 from the selected photos (with their edits):
720p or 1080p, how long each photo is shown, smooth transitions, music if you
like.

### Collage

**Create → Picture Collage…** puts the selected photos (or all shown ones) on
one sheet. Layouts: **Mosaic** (photos edge to edge, filling the sheet),
**Frame Mosaic** (one large photo in the center, the rest around it as a
frame), **Grid** (an even grid), **Contact Sheet** (thumbnails with names and
a title), **Multiple Exposure** (photos laid over each other). You can set the
sheet format, the background (a color or one of the photos), borders (white or
"Instant" with a caption), shadows and spacing; **Shuffle Pictures** shuffles
the order. At the bottom is the list of photos: remove one, make it the
background or the center. **Create Collage** saves a JPG (3000 px wide) next to
the first photo and puts it into the "Collages" album; **Desktop Background**
also sets it as the wallpaper (GNOME).

### Slideshow

**Ctrl+4** or **View → Slideshow** — full screen. **Space** — pause, arrows —
previous and next, **−/+** — display time, **Esc** — exit. On a video, a click
on the picture also pauses and plays it, and ▶ / ⏸ show when the mouse moves.

### Files

- **Export Picture to Folder…** (**Ctrl+Shift+S**) saves copies with edits into
  the chosen folder, with downsizing and a choice of JPEG quality.
- **Rename…** (**F2**), **Locate on Disk** (**Ctrl+Enter**), **Open in Editor**
  (**Ctrl+Shift+O**), **Move to Trash…** (**Delete**; the Linux, Windows and
  macOS trash).
- **Tools → Show Duplicate Files** finds identical files across the whole
  library.
- **Tools → Options…** — theme, animations, language, size and cleanup of the
  thumbnail cache, whether ffmpeg was found. Changes are kept at once.

## Keyboard shortcuts

The full list is also in the program itself: **Help → Keyboard Shortcuts**
(**F1**).

| Where | Keys | Action |
|---|---|---|
| Library | arrows, Enter (or Ctrl+3) | select, open |
| Library | Home / End, PgUp / PgDn | first / last, page up / down |
| Library | Ctrl+H | hold in the tray (when shown) |
| Library | S | star |
| Everywhere | Ctrl+R / Ctrl+Shift+R | rotate clockwise / counterclockwise |
| Library | Ctrl+Shift+E / Ctrl+Shift+B | I'm Feeling Lucky / B&W |
| Library | Ctrl+1 / Ctrl+2 | small / normal thumbnails |
| Library | Ctrl+F | search |
| Library | Ctrl+N | new album from the selection |
| Everywhere | Alt+Enter / Ctrl+K | properties and tags / add a tag |
| Everywhere | Ctrl+4 | slideshow |
| Photo | ← → , Home / End, Esc, S | previous and next, first / last, back, star |
| Photo | wheel, Ctrl+wheel | previous and next, zoom |
| Photo | Ctrl+Z / Ctrl+Y | undo / redo an edit |
| Photo | \\ (hold) | show the original |
| Photo | F, F11 | full screen |

## Appearance and language

**Tools → Options… → Look**: theme as in the system (the default), dark or
light. "As in the system" on Linux follows the desktop settings (the XDG
portal: GNOME, KDE and others) and changes at once, without a restart. Photos
in the viewer and the slideshow are always on a dark background.
**Animations** turns the flying star and the scrolling with weight on or off.

**Language**: as in the system (the default), English or Russian; it changes at
once. Translations are gettext `.po` files in `crates/picco/po`; how to add a
language is in `crates/picco/po/README.md`.

## Where things are stored

- `.picasa.ini` in every folder — stars, captions, edits (compatible with
  Picasa).
- `.picasaoriginals/` — originals of photos saved with **Save**.
- `~/.config/picco/watched_folders.txt` and `excluded_folders.txt` — the
  library's folders and the ones left out of it.
- `~/.config/picco/settings.txt` — what Picco remembers between runs: thumbnail
  size, captions, sorting, the filmstrip, the Photo Tray, the folder list view,
  the properties panel, the zoom mode, theme, language, animations, the window size, the widths of the left and right panels, which
  sections on the left and in the properties panel are open, and the last open
  folder (`key=value` lines, can be edited by hand; written a second after a
  change and on exit).
- `~/.config/picco/<token>.pal` — albums in the Picasa format: name,
  description, date and photos in album order.
- `~/.cache/picco/thumbs/` — the thumbnail cache, safe to delete.

## For developers 

- `crates/picco-core` — everything without a UI: `.picasa.ini` (`ini`, `meta`),
  collages (`collage`), the edit stack (`filters`, `rect64`), effects
  (`adjust`, `effects`, `ops`, `render`), files (`media`, `library`, `export`,
  `fileops`), metadata (`exif`, `exif_tags`, `maker_notes`, `maker_tags`,
  `xmp`, `metadata`, and `agree` for the fields that must agree), albums
  (`album_file`), thumbnails, video and movies (`thumbnail`, `video`, `movie`),
  settings (`config`).
- `crates/picco` — the egui app: library (`app`, `grid`), collages
  (`collage_view`), properties and tags (`info`), viewing and editing
  (`viewer`, `tools`), video (`player`), slideshow (`slideshow`), translations
  (`i18n`, the texts in `po/`), windowless UI tests (`tests`, screenshots in
  `target/ui-shots/`).
- `docs/` — project notes, release notes
  (`releases/`), and how to update rarely
  changing data — places, EXIF tags, makers' notes, licenses — and which metadata fields must
  agree (`data-updates.md`).

Creative Kit effects follow Picnik's recipes (Google, Apache License 2.0).
Place names come from [GeoNames](https://www.geonames.org/) (`cities1000`,
CC BY 4.0) and [Natural Earth](https://www.naturalearthdata.com/) (countries,
seas, continents; public domain). They are packed into
`crates/picco-core/data/places.bin` by `crates/picco-core/data/make_places.py`
(`make places`).
Metadata in the properties panel is read according to the format
specifications:

- EXIF — the [CIPA DC-008 (Exif 3.0)](https://www.cipa.jp/e/std/std-sec.html)
  standard; names and types of the tags of the main directory, EXIF, DNG and
  Interoperability follow the [exiv2 reference](https://exiv2.org/tags.html)
  (`crates/picco-core/src/exif_tags.rs`, `make exif-tags`), GPS tags and their
  values follow [ExifTool](https://exiftool.org/TagNames/GPS.html), and the
  meaning of codes (Exposure Program, Metering Mode, Flash…) follows the
  [ExifTool EXIF table](https://exiftool.org/TagNames/EXIF.html).
- XMP — the [Adobe XMP Specification](https://github.com/adobe/xmp-docs)
  (parts 1–3) and [XMP tags in ExifTool](https://exiftool.org/TagNames/XMP.html);
  `exif:` and `tiff:` properties are decoded like the EXIF tags of the same
  name.
- IPTC — [IPTC-IIM 4.2](https://www.iptc.org/std/IIM/4.2/specification/IIMV4.2.pdf)
  (record 2), in JPEG — inside the Photoshop block (resource 0x0404).
- ICC profile — [ICC.1:2022](https://www.color.org/specification/ICC.1-2022-05.pdf).
- PNG — the [W3C specification](https://www.w3.org/TR/png-3/) (`IHDR`, `tIME`,
  `pHYs`, `gAMA`, `sRGB`, `iCCP`, text chunks, `eXIf`) and ImageMagick's
  "Raw profile type".
- WebP — the [RIFF container description](https://developers.google.com/speed/webp/docs/riff_container).
- Video — the output of [ffprobe](https://ffmpeg.org/ffprobe.html).
- HEIC, HEIF and AVIF — EXIF found through the item boxes of
  [ISO/IEC 23008-12](https://www.iso.org/standard/83650.html).
- The camera makers' notes (MakerNote) — entry names and value meanings from
  [ExifRead](https://github.com/ianare/exif-py)'s tables (BSD-3-Clause;
  `crates/picco-core/src/maker_tags.rs`, `make maker-tags`), the arithmetic
  after [ExifTool's tag tables](https://exiftool.org/TagNames/). What the notes
  say is checked against EXIF and XMP, and fields that disagree get a "!".

All third-party packages and the texts of their licenses are shown in
**Help → About**; the list lives in `crates/picco/assets/licenses.txt` and is
updated with `make licenses` after the dependencies change.

## License

Picco is licensed under the [Apache License 2.0](LICENSE).

[![ko-fi](https://ko-fi.com/img/githubbutton_sm.svg)](https://ko-fi.com/miamibc)
