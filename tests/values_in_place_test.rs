//! A value nothing else holds is updated where it is, and a value read
//! out of another is copied only by what it holds itself, on every tier.
//! A value someone else still holds is never changed.
//!
//! Each timed case is sized so the old copy takes several times its
//! bound in a release build, while in place it takes a small part of it.

use std::process::Command;

fn run(flags: &[&str], src: &str) -> String {
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("main.ol");
    std::fs::write(&path, src).expect("write");
    let out = Command::new(env!("CARGO_BIN_EXE_olang"))
        .args(flags)
        .arg(&path)
        .env("OLANG_COMPILE_CACHE", "0")
        .output()
        .expect("run olang");
    assert!(
        out.status.success(),
        "{flags:?}: {}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// The default tiers, everything compiled at once, and the interpreter.
const TIERS: [&[&str]; 3] = [&[], &["--ovm-tier=1"], &["--no-ovm"]];

#[test]
fn map_remove_in_the_rebind_form_removes_in_place() {
    // OL-524: `m = map_remove(m, k)` copied the map each time, as
    // `map_set` no longer does: 350 µs a call on 21,000 keys. 4,000
    // removals from a map of 60,000 copied 4,000 maps of ~58,000 keys.
    let src = r#"
fn shrink(n, k) = {
    let mut m = #{}
    for i in range(0, n) { m = map_set(m, "k" + to_string(i), i) }
    let kept = m
    let t0 = time.monotonic()
    for i in range(n - k, n) { m = map_remove(m, "k" + to_string(i)) }
    let ms = time.monotonic() - t0
    (map_len(m), map_len(kept), map_get(kept, "k" + to_string(n - 1)), map_get(m, "k" + to_string(n - 1)), ms)
}
fn shared() = {
    let mut m = #{ "a": 1, "b": 2, "c": 3 }
    let held = m
    m = map_remove(m, "b")
    m = map_remove(m, "a")
    (m, held)
}
let (a, b, c, d, ms) = shrink(60000, 4000)
println([a, b, c, d, ms < 400.0])
println(shared())
"#;
    for flags in TIERS {
        assert_eq!(
            run(flags, src),
            "[56000, 60000, 59999, (), true]\n(#{\"c\": 3}, #{\"a\": 1, \"b\": 2, \"c\": 3})",
            "{flags:?}"
        );
    }
}

#[test]
fn typeof_names_every_value_alike_on_every_tier() {
    // OL-521: on the bytecode tier a record or struct crossed to the
    // interpreter to be named; now it is named by its tag. The names
    // must stay the interpreter's.
    let src = r#"
type P = struct { a: Int, b: Int }
type E = enum { One(Int), Two }
fn names(xs) = map(xs, (x) => typeof(x))
fn main() = {
    let f = (x) => x + 1
    let xs = [{ a: 1 }, P { a: 1, b: 2 }, One(1), Two, Ok(1), Err("e"), range(0, 2), f, names, json.parse("{\"a\": 1}"), (1, 2), #{ "k": 1 }, [1], 1.5, "s", true, ()]
    let mut out = []
    for x in xs { out = out + [typeof(x)] }
    (out, names(xs) == out)
}
println(main())
"#;
    let outs: Vec<String> = TIERS.iter().map(|flags| run(flags, src)).collect();
    assert!(
        outs[0].starts_with("([\"Object\", \"P\", \"E\", \"E\", \"Result\", \"Result\", \"List\", \"Function\", \"Function\""),
        "{}",
        outs[0]
    );
    assert!(outs[0].ends_with("true)"), "{}", outs[0]);
    assert!(outs.iter().all(|o| o == &outs[0]), "{outs:?}");
}
