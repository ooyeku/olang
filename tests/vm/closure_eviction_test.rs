//! A lambda the tier compiles per closure is evicted when its closure
//! dies (open-track OL-146).
//!
//! A closure compiles with its captures baked in as constants, so its
//! compiled artifact holds whatever it captured. The index of those
//! artifacts (`hof_cache`) was cleared at 512 entries, but the artifacts
//! themselves — in the bytecode cache, the hot mirror, the JIT's table —
//! were never dropped: every closure a program ever handed to a builtin
//! from interpreted code stayed resident with its captures. On a server
//! that is every request's data, on every worker: open-track went from
//! 450 MB to 1.4 GB in a thousand reads and gave nothing back, while the
//! same load under `--no-ovm` stayed flat.

use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("olang_evict_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(dir: &Path, file: &str, source: &str, args: &[&str]) -> String {
    std::fs::write(dir.join(file), source).unwrap();
    let mut all: Vec<&str> = args.to_vec();
    all.push(file);
    let out = Command::new(env!("CARGO_BIN_EXE_olang"))
        .args(&all)
        .current_dir(dir)
        .output()
        .expect("run olang");
    assert!(
        out.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

/// The "request" runs on the interpreter (top level) and hands a builtin
/// a lambda that captures its own 20,000-element list: a new closure, and
/// a new compile, per iteration.
const REQUESTS: &str = "let mut i = 0\n\
let mut hits = 0\n\
while i < 1500 {\n\
    let big = map(range(0, 20000), (k) => k + i)\n\
    hits = hits + len(filter(range(0, 50), (k) => big[k] > i + 10))\n\
    i = i + 1\n\
}\n\
println(to_string(hits))\n\
println(to_string(map_get(runtime.memory(), \"values\") / 1048576))\n";

#[test]
fn a_dead_closures_compiled_lambda_does_not_keep_its_captures() {
    let ws = workspace("captures");
    let out = run(&ws, "requests.ol", REQUESTS, &[]);
    let lines: Vec<&str> = out.lines().collect();
    // Same answer as the interpreter alone: every iteration counts the
    // elements of its OWN list (k + i > i + 10 for k in 11..50).
    assert_eq!(lines[0], (1500 * 39).to_string(), "{out}");
    let mb: u64 = lines[1].parse().expect("megabytes");
    // Retained without eviction: about 0.3 MB an iteration, ~470 MB here.
    assert!(mb < 80, "live values after 1,500 closures: {mb} MB\n{out}");
}

#[test]
fn eviction_changes_nothing_a_program_can_see() {
    // Closures that stay alive are called again after hundreds of others
    // have been compiled and evicted around them; closures from one
    // factory keep their own captures; a closure made inside compiled code
    // and returned is still callable.
    let source = "fn adder(k) = (n) => n + k\n\
fn twice(f) = (x) => f(f(x))\n\
let keep = map(range(0, 40), (k) => adder(k * 100))\n\
let mut i = 0\n\
let mut sum = 0\n\
while i < 600 {\n\
    let data = map(range(0, 2000), (k) => k * i)\n\
    sum = sum + len(filter(range(0, 20), (k) => data[k] >= i))\n\
    sum = sum + map([1], keep[i % 40])[0]\n\
    sum = sum + map([1], twice(keep[(i + 7) % 40]))[0]\n\
    i = i + 1\n\
}\n\
println(to_string(sum))\n";
    let ws = workspace("semantics");
    let tiered = run(&ws, "semantics.ol", source, &[]);
    let interpreted = run(&ws, "semantics.ol", source, &["--no-ovm"]);
    assert_eq!(tiered, interpreted);
}
