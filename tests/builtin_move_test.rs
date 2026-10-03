//! A value handed to a builtin or to `+` by a register that is not read
//! again is extended in place, on both tiers — and a value something
//! else still holds is never changed.
//!
//! The bytecode tier cloned every argument of a builtin call out of its
//! register, and its native `map_set` cloned the map besides; `+` on a
//! list copied its left side however temporary it was. Loom's layout
//! grows a memo table through `cell.take` (`map_set(cell.take(c), k, v)`,
//! `cell.take(c) + [e]`) and paid a copy per insert: 8,000 inserts, 548 ms
//! compiled against 12 ms interpreted. Builtin calls now take a mask of
//! the argument registers dead after the call (as user-function calls
//! do), `map_set` inserts into a map it owns outright, and an `Add` whose
//! left register is dead extends it.

use std::process::Command;

fn run(flags: &[&str], src: &str) -> String {
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("main.ol");
    std::fs::write(&path, src).expect("write");
    let out = Command::new(env!("CARGO_BIN_EXE_olang"))
        .args(flags)
        .arg(&path)
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

const TIERS: [&[&str]; 2] = [&[], &["--no-ovm"]];

#[test]
fn a_table_grown_through_take_is_extended_not_copied() {
    // Quadratic copying of 20,000 entries takes tens of seconds even in
    // a release build; in place it is well under the bound in a debug one.
    let src = r#"
fn put(c, k, v) = { cell.set(c, map_set(cell.take(c), k, v)); () }
fn push(c, v) = { cell.set(c, cell.take(c) + [v]); () }
let m = cell(#{})
let l = cell([])
let t0 = time.monotonic()
for i in range(0, 20000) {
    put(m, to_string(i), i)
    push(l, (to_string(i), i))
}
let ms = time.monotonic() - t0
println([map_len(cell.get(m)), len(cell.get(l)), ms < 8000.0])
"#;
    for flags in TIERS {
        assert_eq!(run(flags, src), "[20000, 20000, true]", "{flags:?}");
    }
}

#[test]
fn a_value_still_held_elsewhere_is_never_changed() {
    let src = r#"
let a = #{ "x": 1 }
let b = map_set(a, "y", 2)
let xs = [1, 2]
let ys = xs + [3]
let base = [(1, 2)]
fn from_base() = base
let grown = from_base() + [(3, 4)]
let s = "ab"
let t = s + "c"
fn kept(m) = { let m2 = map_set(m, "z", 3); m }
println([a, b, xs, ys, base, grown, s, t, kept(#{ "w": 0 })])
"#;
    let want = r#"[#{"x": 1}, #{"x": 1, "y": 2}, [1, 2], [1, 2, 3], [(1, 2)], [(1, 2), (3, 4)], "ab", "abc", #{"w": 0}]"#;
    for flags in TIERS {
        assert_eq!(run(flags, src), want, "{flags:?}");
    }
}

#[test]
fn a_builtin_handed_the_same_register_twice_sees_it_twice() {
    // Only registers that appear once in a call are moved; a list passed
    // as both arguments must reach the builtin whole both times.
    let src = r#"
fn twice(xs) = zip(xs, xs)
let r = twice([1, 2, 3] + [4])
println(r)
"#;
    for flags in TIERS {
        assert_eq!(
            run(flags, src),
            "[(1, 1), (2, 2), (3, 3), (4, 4)]",
            "{flags:?}"
        );
    }
}
