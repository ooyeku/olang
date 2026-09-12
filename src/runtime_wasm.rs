//! The browser runtime the `olang` binary carries: the playground wasm
//! (olang compiled for `wasm32-unknown-unknown`, the browser profile the
//! web SDK ships), embedded by `build.rs` from the artifact `cargo xtask
//! wasm` builds. A binary built without that artifact embeds nothing and
//! answers `None` here; `runtime.wasm()` then says how to build one that
//! does.
//!
//! The hash and the compressed forms are computed once per process, on
//! first use: `serve` asks at boot, and every request after that is a
//! slice of memory.

use std::sync::OnceLock;

static BYTES: &[u8] = include_bytes!(env!("OLANG_RUNTIME_WASM"));

/// The runtime's bytes, when this binary carries them.
pub fn embedded() -> Option<&'static [u8]> {
    if BYTES.is_empty() { None } else { Some(BYTES) }
}

/// The first sixteen hex digits of the runtime's SHA-256: its ETag and
/// the name in its content-addressed URL (`/olang.<hash>.wasm`).
pub fn hash() -> Option<&'static str> {
    static HASH: OnceLock<String> = OnceLock::new();
    embedded().map(|bytes| {
        HASH.get_or_init(|| {
            use sha2::{Digest, Sha256};
            let digest = Sha256::digest(bytes);
            let hex = format!("{:x}", digest);
            hex[..16].to_string()
        })
        .as_str()
    })
}

/// The runtime gzipped (level 6), for a client that accepts gzip.
pub fn gzipped() -> Option<&'static [u8]> {
    static GZ: OnceLock<Vec<u8>> = OnceLock::new();
    embedded().and_then(|bytes| {
        GZ.get_or_init(|| crate::stdlib::compress::gzip(bytes, 6).unwrap_or_default());
        let gz = GZ.get()?;
        if gz.is_empty() {
            None
        } else {
            Some(gz.as_slice())
        }
    })
}

/// The runtime brotli-compressed (quality 5), for a client that accepts
/// br — a quarter of the raw bytes on the wire.
pub fn brotli() -> Option<&'static [u8]> {
    static BR: OnceLock<Vec<u8>> = OnceLock::new();
    embedded().and_then(|bytes| {
        BR.get_or_init(|| crate::stdlib::compress::brotli(bytes, 5).unwrap_or_default());
        let br = BR.get()?;
        if br.is_empty() {
            None
        } else {
            Some(br.as_slice())
        }
    })
}
