//! The browser runtime the `olang` binary carries: the playground wasm
//! (olang compiled for `wasm32-unknown-unknown`, the browser profile the
//! web SDK ships), embedded by `build.rs` from the artifact `cargo xtask
//! wasm` builds. A binary built without that artifact embeds nothing and
//! answers `None` here; `runtime.wasm()` then says how to build one that
//! does.
//!
//! The hash and the compressed forms are made by `build.rs` and embedded
//! beside the raw bytes, so every form is a slice of the binary's own
//! image: nothing is computed at boot, nothing is copied to the heap,
//! and the pages are file-backed — the system can drop and re-read them.

static BYTES: &[u8] = include_bytes!(env!("OLANG_RUNTIME_WASM"));
static GZ: &[u8] = include_bytes!(env!("OLANG_RUNTIME_WASM_GZ"));
static BR: &[u8] = include_bytes!(env!("OLANG_RUNTIME_WASM_BR"));
static HASH: &str = include_str!(env!("OLANG_RUNTIME_WASM_HASH"));

fn non_empty(bytes: &'static [u8]) -> Option<&'static [u8]> {
    if bytes.is_empty() { None } else { Some(bytes) }
}

/// The runtime's bytes, when this binary carries them.
pub fn embedded() -> Option<&'static [u8]> {
    non_empty(BYTES)
}

/// The first sixteen hex digits of the runtime's SHA-256: its ETag and
/// the name in its content-addressed URL (`/olang.<hash>.wasm`).
pub fn hash() -> Option<&'static str> {
    embedded().map(|_| HASH)
}

/// The runtime gzipped (level 6), for a client that accepts gzip.
pub fn gzipped() -> Option<&'static [u8]> {
    embedded().and_then(|_| non_empty(GZ))
}

/// The runtime brotli-compressed (quality 5), for a client that accepts
/// br — a quarter of the raw bytes on the wire.
pub fn brotli() -> Option<&'static [u8]> {
    embedded().and_then(|_| non_empty(BR))
}

/// Bytes of the binary's image the embedded runtime accounts for.
pub fn embedded_bytes() -> usize {
    BYTES.len() + GZ.len() + BR.len()
}
