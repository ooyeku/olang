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

/// A 16-way tree of tuples and lists, `full` children filled per node.
const TREE: &str = r#"
fn tree(depth, full) = {
    if depth == 0 => return (0, (), "leaf")
    let mut kids = []
    for i in range(0, 16) { kids = kids + [if i < full => tree(depth - 1, full) else => ()] }
    (depth, 0, kids)
}
"#;

#[test]
fn a_list_changed_or_cut_shares_its_items_however_much_they_hold() {
    // OL-525, OL-574: col.set, take, drop, col.slice and col.splice on a
    // list something else holds converted every value beneath its items,
    // there and back: 300 µs a call over a 4-level tree of tuples, where
    // `ys[i] = v` on a copy is 0.2 µs.
    let src = format!(
        "{TREE}{}",
        r#"
fn calls(xs, n) = {
    let mut k = 0
    for i in range(0, n) {
        k = k + len(col.set(xs, 0, ())) + len(take(xs, 3)) + len(drop(xs, 3))
        k = k + len(col.slice(xs, 1, 4)) + len(col.splice(xs, 1, 1, [0, 0])) + len(col.take(xs, 2))
    }
    k
}
let xs = tree(4, 8)[2]
let before = show(xs)
let t0 = time.monotonic()
let k = calls(xs, 1000)
let ms = time.monotonic() - t0
let set = col.set(xs, 1, "new")
let cut = col.splice(xs, 0, 2, ["a"])
println([k, ms < 300.0, show(xs) == before, set[1], set[0] == xs[0], len(cut), cut[0], cut[1] == xs[2]])
println([take([1, 2, 3], 5), drop([1, 2, 3], 1), col.slice([1, 2, 3, 4], 3, 1), col.slice([1.5, 2.5, 3.5], 1, 9), col.splice([1, 2, 3], 2, 9, [7])])
"#
    );
    for flags in TIERS {
        assert_eq!(
            run(flags, &src),
            "[54000, true, true, \"new\", true, 15, \"a\", true]\n[[1, 2, 3], [2, 3], [], [2.5, 3.5], [1, 2, 7]]",
            "{flags:?}"
        );
    }
}

#[test]
fn the_window_builtins_raise_as_the_interpreter_does() {
    for (call, want) in [
        ("col.slice([1, 2], -1, 2)", "negative"),
        ("col.set([1, 2], 2, 0)", "out of bounds"),
        ("take([1, 2], -1)", "non-negative"),
        ("col.splice((1, 2), 0, 1, [])", "must be a list"),
    ] {
        let src = format!(
            "fn f() = {call}\nprintln(match attempt(f) {{ Ok(v) => show(v), Err(e) => e }})"
        );
        let outs: Vec<String> = TIERS.iter().map(|flags| run(flags, &src)).collect();
        assert!(outs[0].contains(want), "{call}: {}", outs[0]);
        assert!(outs.iter().all(|o| o == &outs[0]), "{call}: {outs:?}");
    }
}

/// Takes lane 8 out of a box, then its list of chunks, then one chunk,
/// and writes one slot: in place when nothing else holds any of them.
const BOX: &str = r#"
fn h1(box, p) = {
    let mut b = box
    let mut lane = b[8]
    b[8] = ()
    let mut chunks = lane[0]
    lane[0] = ()
    let ci = p / 32
    let mut chunk = chunks[ci]
    chunks[ci] = ()
    chunk[p % 32] = p
    chunks[ci] = chunk
    lane[0] = chunks
    b[8] = lane
    b
}
fn mk(nch) = [0, 0, 0, 0, 0, 0, 0, 0, [map(range(0, nch), (c) => map(range(0, 32), (s) => "x")), 0, 7], ()]
"#;

#[test]
fn a_nested_read_lets_go_of_the_inner_list_at_its_statement() {
    // OL-527: `let p = b[8][2]` kept `b[8]` in a temporary, so the next
    // in-place update of the lane saw it shared and copied its 20,000
    // chunks: 60 µs a turn, against 0.3 through `fn rd(b) = b[8][2]`.
    let src = format!(
        "{BOX}{}",
        r#"
fn go(nch, n) = {
    let mut b = mk(nch)
    let mut s = 0
    let t0 = time.monotonic()
    for k in range(0, n) {
        let p = b[8][2]
        s = s + p
        b = h1(b, k % (nch * 32))
    }
    (s, b[8][0][0][5], time.monotonic() - t0)
}
fn kept() = {
    let b = mk(2)
    let inner = b[8][0]
    let p = b[8][2]
    let b2 = h1(b, 3)
    (p, inner[0][3], b[8][0][0][3], b2[8][0][0][3])
}
let (s, slot, ms) = go(20000, 10000)
println([s, slot, ms < 400.0])
println(kept())
"#
    );
    for flags in TIERS {
        // The interpreter has no last-use moves: `h1` copies the chunks
        // there inline or through a function alike, so it runs small.
        let src = if flags == ["--no-ovm"] {
            src.replace("go(20000,", "go(20,")
        } else {
            src.clone()
        };
        assert_eq!(
            run(flags, &src),
            "[70000, 5, true]\n(7, \"x\", \"x\", 3)",
            "{flags:?}"
        );
    }
}

#[test]
fn a_value_taken_out_of_a_returned_record_is_left_unshared() {
    // OL-529: `b = w(b).box` kept the record in a temporary, so `b`'s
    // lists stayed shared with it and the next update copied the lane's
    // 20,000 chunks.
    let src = format!(
        "{BOX}{}",
        r#"
fn w(box, p) = { let mut b = box; b = h1(b, p); { box: b, n: p } }
fn go(nch, n) = {
    let mut b = mk(nch)
    let t0 = time.monotonic()
    for k in range(0, n) { b = w(b, k % (nch * 32)).box }
    (b[8][0][0][5], time.monotonic() - t0)
}
fn kept() = {
    let r = w(mk(2), 1)
    let b = r.box
    let b2 = h1(b, 2)
    (r.box[8][0][0][2], b[8][0][0][2], b2[8][0][0][2], b2[8][0][0][1])
}
let (slot, ms) = go(20000, 10000)
println([slot, ms < 400.0])
println(kept())
"#
    );
    for flags in TIERS {
        let src = if flags == ["--no-ovm"] {
            src.replace("go(20000,", "go(20,")
        } else {
            src.clone()
        };
        assert_eq!(
            run(flags, &src),
            "[5, true]\n(\"x\", \"x\", 2, 1)",
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
