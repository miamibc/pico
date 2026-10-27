# Shortcuts for common tasks. Cargo does the real work; on systems without
# make, run the cargo commands shown here directly.

PICTURES ?= $(HOME)/Pictures

.PHONY: all build release windows run test lint fmt check testdata tour logo licenses install clean

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

# Draws the logos for the window and Help → About (needs Inkscape and the
# Kamerik 105 font the title is set in).
logo:
	inkscape docs/logo/pico.svg -w 256 -h 256 -o crates/pico/assets/pico-256.png
	inkscape docs/logo/pico-logo-with-title.svg -w 800 -o crates/pico/assets/pico-logo-with-title.png

# Lists the crates Pico ships and their license texts for Help → About; run
# after changing dependencies.
licenses:
	python3 -I crates/pico/assets/make_licenses.py

# Installs the binary into ~/.cargo/bin.
install:
	cargo install --path crates/pico

clean:
	cargo clean
