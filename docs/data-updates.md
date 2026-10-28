# Updating rarely changing data

**English** · [Русский](data-updates.ru.md)

Some of Pico's data comes not from the code but from external references:
place names, the list of EXIF tags, dependency licenses. These references
change slowly, so updating them about once a year or before a big release is
enough. The program never goes online while running: everything is downloaded
here, on the developer's machine, and lands in the repository as ready-made
files.

You need `curl`, `unzip` and Python 3 (standard library only). Downloads go
into `target/data-download/` and stay out of git.

## Place names (Places)

```sh
make places
```

What it does: downloads `cities1000.zip` (every populated place with at least
1000 inhabitants) and `countryInfo.txt` (country names) from GeoNames, and the
outlines of countries, seas and oceans, and continents
(`ne_50m_admin_0_countries`, `ne_10m_geography_marine_polys`,
`ne_50m_geography_regions_polys`) from Natural Earth, then packs everything
with `crates/pico-core/data/make_places.py` into
`crates/pico-core/data/places.bin`. The file is embedded into the program at
build time (`crates/pico-core/src/geo.rs`). The file format is described at the
top of the script.

Then:

1. The script prints how many cities came out and the size. Right now it is
   about 162,000 cities and 3.5 MB; noticeably more or less is a reason to check
   whether the source format has changed.
2. `make check` — the `geo.rs` tests check that the file loads and that known
   points are found (city, country, sea, continent).
3. Run Pico and open the **Places** section on photos with GPS.
4. If a source or a license has changed, fix the header of
   `crates/pico/assets/make_licenses.py` (the DATA AND RECIPES section), run
   `make licenses` and fix the paragraph about data in `README.md` and `README.ru.md`.
5. Commit `places.bin` (and the script, if it changed).

If Natural Earth renames fields in the GeoJSON (`ISO_A2_EH`, `NAME_EN`, `name`,
`FEATURECLA`), the script fails with a `KeyError` — fix the field names in the
script's `main()`.

## EXIF tags (the Properties panel)

```sh
make exif-tags
```

What it does: downloads the exiv2 tag reference (<https://exiv2.org/tags.html>,
groups Image, Photo, Iop) and the ExifTool GPS tag table
(<https://exiftool.org/TagNames/GPS.html>), and
`crates/pico-core/data/make_exif_tags.py` writes
`crates/pico-core/src/exif_tags.rs`: the tag number, the name as in XMP
(`ExposureTime`) and a human-readable title (`Exposure Time`). GPS tags are
stored as 0x1_0000 + number, Interoperability tags as 0x2_0000 + number. This
file is not edited by hand.

Then:

1. `git diff crates/pico-core/src/exif_tags.rs` — usually a few new tags are
   added (new DNG or EXIF versions). If many lines disappeared, the page layout
   most likely changed, and the regular expressions in
   `exiv2_tags()`/`gps_tags()` need adjusting.
2. Names are split into words automatically (`ExposureTime` →
   `Exposure Time`). If the result is poor (`T4Options` → `T 4 Options`), add an
   exception to the script's `WORDS` dictionary. Tags that exiv2 lacks go into
   the `EXTRA` list.
3. `make check` and look at **Properties** on a few photos.
4. Commit `exif_tags.rs`, and the script if it changed.

Not generated but written by hand in `crates/pico-core/src/exif.rs`:

- `tag_name()` — more familiar names for the most common tags (they win over
  the table in `exif_tags.rs`);
- `choice()` — the meaning of codes (Exposure Program, Metering Mode, Light
  Source, Scene Type…), from the [ExifTool EXIF table](https://exiftool.org/TagNames/EXIF.html);
- `describe()` — units and formats (shutter speed, aperture, focal length,
  flash).

Check them against the ExifTool table when a new version of the EXIF standard
(CIPA DC-008) comes out — it may add new code values.

The other formats (XMP, IPTC, ICC, PNG, WebP) are parsed according to their
specifications, linked in the "For developers" section of `README.md`. They
hardly ever change and need no separate updates.

## Licenses

```sh
make licenses
```

Rebuilds `crates/pico/assets/licenses.txt` (the list in **Help → About**) from
`cargo metadata`. Run it after any change of dependencies in `Cargo.toml` or a
`cargo update`, then commit `licenses.txt`.

## Agreement of fields across metadata kinds (the "!" badge)

A photo can store the same thing — a title, an author, a date — in EXIF, XMP,
IPTC and PNG text chunks at once. When such fields exist in several places and
disagree, the **Properties** panel puts a "!" badge next to each of them, and
the tooltip shows what each one says. The rules are in
`crates/pico-core/src/agree.rs` (`GROUPS`).

Groups of fields that must agree:

| What | EXIF | XMP | IPTC | PNG | Compared as |
|---|---|---|---|---|---|
| Title | Title, Image Title | Title | Title | — | text |
| Description | Description | Description | Caption/Abstract | Description | text |
| Author | Artist, Author, Photographer | Creator | By-line | Author | text |
| Rights | Copyright | Rights | Copyright Notice | Copyright | text |
| Keywords | Keywords | Subject | Keywords | — | set of words |
| Date taken | Date Taken | Date Taken, Date Created | Date Created | — | date |
| Date digitized | Date Digitized | Create Date, Date Digitized | — | — | date |
| Date modified | Date Modified | Modify Date | — | Modified | date |
| Software | Software | Creator Tool | Originating Program | Software | text |
| Rating | Rating | Rating | — | — | number |
| Lens | Lens Model | Lens Model, Lens | — | — | text |

Besides that, every XMP property from the `exif:` and `tiff:` namespaces is a
copy of the EXIF tag of the same name (`exif:ExposureTime` ↔ Exposure Time) and
is compared with it: as a date if the name starts with "Date", otherwise as
text. The exception is GPS latitude and longitude (`NOT_ALIKE`): EXIF and XMP
write them differently.

How values are compared:

- **text** — ignoring case and extra spaces;
- **set of words** — words separated by commas or semicolons; order and
  repeats don't matter;
- **date** — by digits down to the second, and only as precisely as both
  values go (`2018:02:09 19:17:10` = `2018-02-09T19:17:10.57+02:00`; the time
  zone is ignored);
- **number** — a rating as stars or a digit (`★★★☆☆ (3)` = `3`).

Only fields that are present are compared; a field without a pair gets no
badge. To add a group, append it to `GROUPS` — the labels must match the ones
shown in Properties — and, if you like, a case to the
`finds_fields_that_disagree_across_kinds` test.
