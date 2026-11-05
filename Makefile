# Shortcuts for common tasks. Cargo does the real work; on systems without
# make, run the cargo commands shown here directly.

PICTURES ?= $(HOME)/Pictures

.PHONY: all build release windows run test lint fmt check testdata tour screenshots logo licenses po places exif-tags maker-tags install clean

all: release

build:
	cargo build

release:
	cargo build --release
	@echo "Built target/release/picco"

# Windows .exe built on Linux. One-time setup:
#   rustup target add x86_64-pc-windows-gnu
#   cargo install cargo-zigbuild
#   python3 -m venv ~/tools/zig-venv && ~/tools/zig-venv/bin/pip install ziglang
ZIG_BIN ?= $(HOME)/tools/zig-venv/bin

windows:
	PATH="$(ZIG_BIN):$$PATH" cargo zigbuild --release -p picco --target x86_64-pc-windows-gnu
	@echo "Built target/x86_64-pc-windows-gnu/release/picco.exe"

run:
	cargo run --release -p picco -- $(PICTURES)

test:
	cargo test

lint:
	cargo clippy --all-targets -- -D warnings

fmt:
	cargo fmt --all

# Everything CI should check.
check:
	cargo fmt --all --check
	cargo clippy --all-targets -- -D warnings
	cargo test

# Test library for manual runs (needs Python with Pillow, and ffmpeg).
testdata:
	python3 testdata/make_testdata.py

# Screenshots of every test-library folder in target/ui-shots/.
tour: testdata
	cargo test -p picco testdata_tour -- --ignored

# README and website screenshots in docs/screenshots, dark and light, of a demo
# library of CC0 photos from Wikimedia Commons (needs Python with Pillow, and the
# internet only for photos not in DEMO yet). The photos stay in the project;
# the app sees them through the link /tmp/Photos, so the Properties panel
# shows a short path.
DEMO ?= $(CURDIR)/testdata/demo-photos
DEMO_LINK := /tmp/Photos

screenshots:
	python3 -I docs/screenshots/make_screenshots.py library $(DEMO)
	@[ ! -e $(DEMO_LINK) ] || [ -L $(DEMO_LINK) ] || { echo "$(DEMO_LINK) is a real folder: move it into $(DEMO)"; exit 1; }
	ln -sfn $(DEMO) $(DEMO_LINK)
	rm -rf target/screenshots
	PICCO_DEMO=$(DEMO_LINK) PICCO_SHOTS=$(CURDIR)/target/screenshots cargo test -p picco website_shots -- --ignored
	PICCO_DEMO=$(DEMO_LINK) PICCO_SHOTS=$(CURDIR)/target/screenshots PICCO_SUFFIX=-light cargo test -p picco website_shots -- --ignored
	python3 -I docs/screenshots/make_screenshots.py webp target/screenshots docs/screenshots

# Draws the logos for the window and Help → About, and for README the title
# logo with its text turned into outlines, light and dark (needs Inkscape and
# the Kamerik 105 font the title is set in).
logo:
	inkscape docs/logo/picco.svg -w 256 -h 256 -o crates/picco/assets/picco-256.png
	inkscape docs/logo/picco-logo-with-title.svg -w 800 -o crates/picco/assets/picco-logo-with-title.png
	inkscape docs/logo/picco-logo-with-title.svg --export-text-to-path --export-plain-svg -o docs/logo/picco-logo-with-title-paths.svg
	sed 's/fill:#1e293b/fill:#e2e8f0/g' docs/logo/picco-logo-with-title-paths.svg > docs/logo/picco-logo-with-title-paths-dark.svg

# Lists the crates Picco ships and their license texts for Help → About; run
# after changing dependencies.
licenses:
	python3 -I crates/picco/assets/make_licenses.py

# Collects the interface texts into crates/picco/po/picco.pot and brings the
# translations (*.po) up to date; run after changing texts in the code.
po:
	python3 -I crates/picco/po/update.py

# Rarely changing data, refreshed from the internet (see docs/data-updates.md).
# Downloads go to testdata/data-download (kept, out of git).
DATA_DL = testdata/data-download
NE_URL = https://raw.githubusercontent.com/nvkelso/natural-earth-vector/master/geojson

# Place names for Places: GeoNames towns, Natural Earth countries, seas and
# continents, packed into crates/picco-core/data/places.bin.
places:
	mkdir -p $(DATA_DL)
	curl -fL -o $(DATA_DL)/cities1000.zip https://download.geonames.org/export/dump/cities1000.zip
	curl -fL -o $(DATA_DL)/countryInfo.txt https://download.geonames.org/export/dump/countryInfo.txt
	cd $(DATA_DL) && unzip -o cities1000.zip
	for f in ne_50m_admin_0_countries ne_10m_geography_marine_polys ne_50m_geography_regions_polys; do \
		curl -fL -o $(DATA_DL)/$$f.geojson $(NE_URL)/$$f.geojson || exit 1; \
	done
	python3 -I crates/picco-core/data/make_places.py $(DATA_DL) crates/picco-core/data/places.bin

# The standard EXIF tag names for Properties, from the exiv2 and ExifTool
# references, into crates/picco-core/src/exif_tags.rs.
exif-tags:
	mkdir -p $(DATA_DL)
	curl -fL -o $(DATA_DL)/exiv2.html https://exiv2.org/tags.html
	curl -fL -o $(DATA_DL)/gps.html https://exiftool.org/TagNames/GPS.html
	python3 -I crates/picco-core/data/make_exif_tags.py $(DATA_DL) crates/picco-core/src/exif_tags.rs

# Installs the binary into ~/.cargo/bin; on Linux also the menu entry and
# icon into ~/.local/share (GNOME on Wayland takes the icon only from there).
# Names and value meanings of the camera makers' notes (MakerNote), from
# ExifRead's tables (BSD-3-Clause), into crates/picco-core/src/maker_tags.rs.
EXIFREAD = 3.5.1
maker-tags:
	mkdir -p $(DATA_DL)/exifread
	curl -fL -o $(DATA_DL)/exifread.tar.gz https://github.com/ianare/exif-py/archive/refs/tags/$(EXIFREAD).tar.gz
	tar xzf $(DATA_DL)/exifread.tar.gz -C $(DATA_DL)/exifread
	python3 -I crates/picco-core/data/make_maker_tags.py $(DATA_DL)/exifread crates/picco-core/src/maker_tags.rs
	rustfmt --edition 2024 crates/picco-core/src/maker_tags.rs

install:
	cargo install --path crates/picco
	install -Dm644 docs/logo/picco.svg $(HOME)/.local/share/icons/hicolor/scalable/apps/picco.svg
	install -Dm644 crates/picco/assets/picco-256.png $(HOME)/.local/share/icons/hicolor/256x256/apps/picco.png
	sed 's|^Exec=picco|Exec=$(HOME)/.cargo/bin/picco|' packaging/linux/picco.desktop > $(HOME)/.local/share/applications/picco.desktop

clean:
	cargo clean

preview:
	make build && ./target/debug/picco testdata/raw-samples/