# Simple Makefile for Olang

.PHONY: build install remove clean test wasm dist prepush

WASM_TARGET := wasm32-unknown-unknown
WASM_ARTIFACT := target/$(WASM_TARGET)/release/olang_playground.wasm

# Default target
all: build

# Build the project
build:
	cargo build --release

# Install using the setup script
install:
	@echo "Installing Olang and OTC using setup script..."
	./setup.sh

# Remove installed binaries and directories
remove:
	@echo "Removing Olang and OTC..."
	cargo uninstall olang || true
	cargo uninstall otc || true
	rm -rf ~/.olang
	@echo "Removal complete - all .olang directories and files removed"

# Clean build artifacts
clean:
	cargo clean

# Run tests
test:
	cargo test

# The full pre-push gate, matching what CI checks — with one difference
# that is the whole point: clippy runs on FRESH fingerprints. Cargo skips
# re-linting crates whose files have not changed, so a toolchain update
# that adds a new lint passes locally on stale cache and then fails CI
# (this happened: chunks_exact_to_as_chunks landed in stable, local
# clippy reported clean from cache, CI's fresh toolchain refused).
# Touching each crate root forces every crate through the linter the
# way CI's clean checkout does. Costs ~a minute over the cached run.
prepush:
	cargo fmt --all -- --check
	@touch src/lib.rs otc/src/main.rs olang-ods/src/lib.rs playground/src/lib.rs
	cargo clippy --workspace --all-targets -- -D warnings
	cargo test --workspace
	cargo build --release
	cd examples && ../target/release/olang run run_all.ol

# Build the browser runtime. `cargo xtask wasm --full` builds the
# website playground's profile (the whole stdlib) and stages it at
# website/static/playground/olang.wasm, then the browser profile the
# web SDK and the examples ship — which the next `cargo build` of olang
# embeds in the binary (build.rs), so no app copies a file. Needs
# `rustup target add wasm32-unknown-unknown` once.
wasm:
	cargo xtask wasm --full
# Local cross-check of everything a release ships: the release binaries,
# the playground wasm, and the VS Code .vsix, staged into dist/out/
# (gitignored) with a SHA256SUMS to compare against CI's.
dist: build wasm
	rm -rf dist/out
	mkdir -p dist/out
	cp target/release/olang target/release/otc dist/out/
	cp $(WASM_ARTIFACT) dist/out/olang_playground.wasm
	cd editors/vscode && ([ -x node_modules/.bin/vsce ] || npm ci --silent) \
		&& node_modules/.bin/vsce package --out ../../dist/out/
	cd dist/out && shasum -a 256 -- * > SHA256SUMS
	@echo "dist/out ready:"
	@ls -l dist/out
