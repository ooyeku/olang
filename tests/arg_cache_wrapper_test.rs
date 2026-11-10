//! A wrapped value handed to a compiled callback is let go (OL-575).
//!
//! A compiled function called back from a builtin converts its arguments
//! at the tier boundary, and the tier keeps a struct's conversion for the
//! next call, watched by a `Weak` so it goes when the struct dies. A large
//! parsed JSON object converts to a wrapper around the same allocation:
//! the entry held its own value alive, and every object passed stayed
//! (until 512 were held on that thread).

use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("olang_argwrap_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(dir: &Path, source: &str) -> String {
    std::fs::write(dir.join("main.ol"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_olang"))
        .arg("main.ol")
        .current_dir(dir)
        .env("OLANG_COMPILE_CACHE", "0")
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

/// Each case prints the KB a call left behind, averaged over 30 calls
/// (the object is about 1,700 KB as values).
const CASES: &str = r#"fn kb() = runtime.memory().values / 1000
let text = unwrap(json.stringify(#{ "name": "x", "items": map(range(0, 2000), (k) => #{ "id": k, "tags": ["a", "b"] }) }))
fn run(f) = {
    let k0 = kb()
    for i in range(0, 30) { let _r = f() }
    println(to_string((kb() - k0) / 30))
}
run(() => len(col.sort_by([unwrap(json.parse(text))], (d) => 1.0)))
run(() => len(map_keys(col.count_by([unwrap(json.parse(text))], (d) => "k"))))
// on a worker that lives on, as a pool's
let work = chan.new()
let done = chan.new()
let w = spawn {
    let mut go = true
    while go {
        let m = unwrap(chan.recv(work))
        if m == () => { go = false } else => { let _s = chan.send(done, len(col.sort_by([unwrap(json.parse(text))], (d) => 1.0))) }
    }
    0
}
let k0 = kb()
for i in range(0, 30) { let _s = chan.send(work, 1); let _r = chan.recv(done) }
println(to_string((kb() - k0) / 30))
let _e = chan.send(work, ())
"#;

#[test]
fn a_parsed_object_passed_to_a_callback_is_freed() {
    let dir = workspace("cases");
    let out = run(&dir, CASES);
    let held: Vec<i64> = out.lines().map(|l| l.trim().parse().expect("a number a line")).collect();
    assert_eq!(held.len(), 3, "{out}");
    for (i, kb) in held.iter().enumerate() {
        assert!(*kb < 100, "case {i} left {kb} KB a call behind (the object is about 1,700 KB)\n{out}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
