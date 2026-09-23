//! The browser runtime with its heap above 2 GiB.
//!
//! A pointer crosses the wasm boundary as an i32, which JavaScript reads
//! as signed. Once the heap passed 2 GiB the shim read a selector from
//! the wrong bytes (`dom.query: no element matches "#app"` on a page that
//! had one; a "selector" of spaces and free-list words), then failed on
//! every result buffer after (`Offset is outside the bounds of the
//! DataView`, `offset is out of bounds` at `mem().set`). The dom harness's
//! `--high-memory` mode runs the shim's own boundary block against the
//! runtime with the low 2 GiB held, and drives dispatches that succeed,
//! raise, refuse a non-string selector, and fire one-shot callbacks.
//! Skips where node or the wasm artifact (`cargo xtask wasm`) is absent.

use std::path::Path;
use std::process::Command;

#[test]
fn dispatches_answer_readable_results_with_the_heap_above_2_gib() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let wasm = root.join("target/wasm32-unknown-unknown/release/olang_playground.wasm");
    if !wasm.exists() || Command::new("node").arg("--version").output().is_err() {
        eprintln!("skipping: needs node and the wasm artifact (cargo xtask wasm)");
        return;
    }
    let run = Command::new("node")
        .arg(root.join("playground/dom_harness.mjs"))
        .arg(&wasm)
        .arg("--high-memory")
        .output()
        .expect("node");
    let text = String::from_utf8_lossy(&run.stdout);
    assert!(
        run.status.success(),
        "{text}{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let stats: serde_json::Value = serde_json::from_str(text.lines().last().unwrap_or(""))
        .unwrap_or_else(|_| panic!("{text}"));
    // The run really was above the line: the host was handed pointers
    // past 0x8000_0000.
    assert!(
        stats["highest"].as_u64().unwrap() >= 1 << 31,
        "the harness never reached high memory: {stats}"
    );
    // Every third click was followed by a raising handler; each raise
    // came back as an error inside a readable result.
    assert_eq!(stats["raised"], 40, "{stats}");
    // Thirty one-shot timeouts were registered and run; none of them is
    // still held by the registry.
    assert_eq!(stats["live_after"], stats["live_before"], "{stats}");
    assert!(stats["registered"].as_u64().unwrap() >= 30, "{stats}");
}
