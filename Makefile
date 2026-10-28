# Shortcuts for common tasks. Cargo does the real work; on systems without
# make, run the cargo commands shown here directly.

PICTURES ?= $(HOME)/Pictures

.PHONY: all build release windows run test lint fmt check testdata tour logo licenses places exif-tags install clean

all: release

build:
	cargo build

release:
	cargo build --release
	@echo "Built target/release/pico"

# Windows .exe built on Linux. One-time setup:
#   rustup target add x86_64-pc-windows-gnu
#   cargo install cargo-zigbuild
#   python3 -m venv ~/tools/zig-venv && ~/tools/zig-venv/bin/pip install ziglang
ZIG_BIN ?= $(HOME)/tools/zig-venv/bin

windows:
	PATH="$(ZIG_BIN):$$PATH" cargo zigbuild --release -p pico --target x86_64-pc-windows-gnu
	@echo "Built target/x86_64-pc-windows-gnu/release/pico.exe"

run:
	cargo run --release -p pico -- $(PICTURES)

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
	cargo test -p pico testdata_tour -- --ignored

# Draws the logos for the window and Help → About, and for README the title
# logo with its text turned into outlines, light and dark (needs Inkscape and
# the Kamerik 105 font the title is set in).
logo:
	inkscape docs/logo/pico.svg -w 256 -h 256 -o crates/pico/assets/pico-256.png
	inkscape docs/logo/pico-logo-with-title.svg -w 800 -o crates/pico/assets/pico-logo-with-title.png
	inkscape docs/logo/pico-logo-with-title.svg --export-text-to-path --export-plain-svg -o docs/logo/pico-logo-with-title-paths.svg
	sed 's/fill:#1e293b/fill:#e2e8f0/g' docs/logo/pico-logo-with-title-paths.svg > docs/logo/pico-logo-with-title-paths-dark.svg

# Lists the crates Pico ships and their license texts for Help → About; run
# after changing dependencies.
licenses:
	python3 -I crates/pico/assets/make_licenses.py

# Rarely changing data, refreshed from the internet (see docs/data-updates.md).
# Downloads go to target/data-download.
DATA_DL = target/data-download
NE_URL = https://raw.githubusercontent.com/nvkelso/natural-earth-vector/master/geojson

# Place names for Places: GeoNames towns, Natural Earth countries, seas and
# continents, packed into crates/pico-core/data/places.bin.
places:
	mkdir -p $(DATA_DL)
	curl -fL -o $(DATA_DL)/cities1000.zip https://download.geonames.org/export/dump/cities1000.zip
	curl -fL -o $(DATA_DL)/countryInfo.txt https://download.geonames.org/export/dump/countryInfo.txt
	cd $(DATA_DL) && unzip -o cities1000.zip
	for f in ne_50m_admin_0_countries ne_10m_geography_marine_polys ne_50m_geography_regions_polys; do \
		curl -fL -o $(DATA_DL)/$$f.geojson $(NE_URL)/$$f.geojson || exit 1; \
	done
	python3 -I crates/pico-core/data/make_places.py $(DATA_DL) crates/pico-core/data/places.bin

# The standard EXIF tag names for Properties, from the exiv2 and ExifTool
# references, into crates/pico-core/src/exif_tags.rs.
exif-tags:
	mkdir -p $(DATA_DL)
	curl -fL -o $(DATA_DL)/exiv2.html https://exiv2.org/tags.html
	curl -fL -o $(DATA_DL)/gps.html https://exiftool.org/TagNames/GPS.html
	python3 -I crates/pico-core/data/make_exif_tags.py $(DATA_DL) crates/pico-core/src/exif_tags.rs

# Installs the binary into ~/.cargo/bin; on Linux also the menu entry and
# icon into ~/.local/share (GNOME on Wayland takes the icon only from there).
install:
	cargo install --path crates/pico
	install -Dm644 docs/logo/pico.svg $(HOME)/.local/share/icons/hicolor/scalable/apps/pico.svg
	install -Dm644 crates/pico/assets/pico-256.png $(HOME)/.local/share/icons/hicolor/256x256/apps/pico.png
	sed 's|^Exec=pico|Exec=$(HOME)/.cargo/bin/pico|' packaging/linux/pico.desktop > $(HOME)/.local/share/applications/pico.desktop

clean:
	cargo clean
