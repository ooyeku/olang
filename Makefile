.PHONY: all build install remove clean test wasm dist prepush

WASM_TARGET := wasm32-unknown-unknown
WASM_ARTIFACT := target/$(WASM_TARGET)/release/olang_playground.wasm

all: build

build:
	cargo build --release -p olang -p otc

# Build the browser runtime, then install olang and otc with it embedded.
install:
	cargo xtask install

# Remove the binaries `make install` put in Cargo's bin folder. ~/.olang
# (packages, history, toolchains from `otc update`) is left alone; delete
# it yourself to remove everything.
remove:
	cargo uninstall olang || true
	cargo uninstall otc || true

clean:
	cargo clean

# The test profile is optimized like release but skips whole-program LTO,
# so the ~170 test binaries link in reasonable time.
test:
	cargo test --workspace --profile fasttest

# The checks CI runs, before a push. Clippy runs on fresh fingerprints:
# Cargo skips re-linting crates whose files have not changed, so a new
# toolchain lint can pass locally from cache and then fail on CI's clean
# checkout. Touching each crate root sends every crate through the linter
# again, for about a minute more.
prepush:
	cargo fmt --all -- --check
	@touch src/lib.rs otc/src/main.rs olang-ods/src/lib.rs playground/src/lib.rs
	cargo clippy --workspace --all-targets -- -D warnings
	cargo test --workspace --profile fasttest
	cargo build --release
	cd examples && ../target/release/olang run run_all.ol

# Build the browser runtimes: the profile the web SDK and the examples
# ship, and the whole-stdlib one (target/wasm-full/olang.wasm), which the
# next `cargo build` of olang embeds in the binary (build.rs). Needs
# `rustup target add wasm32-unknown-unknown` once.
wasm:
	cargo xtask wasm

# Stage everything a release ships into dist/out/ (gitignored), with a
# SHA256SUMS to compare against CI's: the release binaries, the playground
# wasm, and the VS Code .vsix.
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
