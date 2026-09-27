//! A lambda written in a function the bytecode tier refuses — one with
//! an early exit (`return`, `?`) is the common case — costs nothing that
//! stays behind after the call.
//!
//! Such a function runs on the tree-walker, which makes a new closure
//! value every time it evaluates the lambda expression: once per call.
//! The tier compiled lambdas by closure identity (the closure's captures
//! are baked into the code), so it compiled — and JIT-compiled — the same
//! lambda again on every call. The bytecode went with the closure, but
//! the JIT never gives code back: a function like
//!
//! ```olang
//! fn leaky(xs) = {
//!     if len(xs) == 0 => return 0
//!     fold(xs, 0, (a, b) => a + b)
//! }
//! ```
//!
//! grew the heap by ~1.3 KB a call, forever (heddle-sql's FINDINGS: a
//! terminal UI leaking ~30 KB a frame), and spent ~80 µs a call
//! compiling. The same function with `break`, or with the lambda moved
//! to a helper, was flat because the tier took the whole function.
//!
//! Now closures of one body whose captures are the same values share one
//! compile, and a body whose closures keep changing (a capture that
//! differs every call) stops taking machine code after its first few.

use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("olang_early_exit_{}_{}", std::process::id(), tag));
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

/// Every shape of the table in heddle-sql's FINDINGS, and the early exits
/// around it. Each row is warmed up (500 calls: promotion, first
/// compiles, the JIT), then measured over 4,000 calls; it prints its
/// heap growth per call and a checksum of its results.
const TABLE: &str = r#"
fn heap() = map_get(runtime.memory(), "heap")

fn lambda_only(xs) = fold(xs, 0, (a, b) => a + b)
fn return_only(xs) = {
    if len(xs) == 0 => return 0
    len(xs) + 1
}
fn return_made(xs) = {
    if len(xs) == 0 => return 0
    let f = (a, b) => a + b
    len(xs)
}
fn return_fold(xs) = {
    if len(xs) == 0 => return 0
    fold(xs, 0, (a, b) => a + b)
}
fn return_taken(xs) = {
    let s = fold(xs, 0, (a, b) => a + b)
    if s > 0 => return s
    0
}
fn return_map(xs) = {
    if len(xs) == 0 => return 0
    len(map(xs, (x) => x * 2))
}
fn return_filter(xs) = {
    if len(xs) == 0 => return 0
    len(filter(xs, (x) => x > 1))
}
fn return_sort_by(xs) = {
    if len(xs) == 0 => return 0
    col.sort_by(xs, (x) => 0 - x)[0]
}
fn return_direct(xs) = {
    if len(xs) == 0 => return 0
    let f = (x) => x + 1
    f(len(xs))
}
fn return_capture(xs) = {
    if len(xs) == 0 => return 0
    let k = len(xs)
    fold(xs, 0, (a, b) => a + b + k)
}
fn return_capture_varying(xs) = {
    if len(xs) == 0 => return 0
    let k = xs[2]
    fold(xs, 0, (a, b) => a + b + k)
}
fn return_capture_list(xs) = {
    if len(xs) == 0 => return 0
    fold(xs, 0, (a, b) => a + b + len(xs))
}
fn helper_fn(x) = x * 3
fn return_capture_fn(xs) = {
    if len(xs) == 0 => return 0
    fold(map(xs, (x) => helper_fn(x)), 0, (a, b) => a + b)
}
fn half(x) = if x % 2 == 0 => Ok(x / 2) else => Err("odd")
fn try_fold(xs) = {
    let h = half(2)?
    Ok(fold(xs, h, (a, b) => a + b))
}
fn try_row(xs) = unwrap(try_fold(xs))
fn match_return(xs) = {
    match len(xs) {
        0 => return 0,
        _ => fold(xs, 0, (a, b) => a + b)
    }
}
fn nested_return(xs) = {
    if len(xs) > 0 => {
        if len(xs) > 100 => {
            return 0
        }
    }
    fold(xs, 0, (a, b) => a + b)
}
fn break_fold(xs) = {
    let mut t = 0
    for x in xs {
        if x > 1000 => break
        t = t + x
    }
    t + fold(xs, 0, (a, b) => a + b)
}
fn continue_fold(xs) = {
    let mut t = 0
    for x in xs {
        if x == 1 => continue
        t = t + fold(xs, 0, (a, b) => a + b)
    }
    t
}
fn sum_helper(xs) = fold(xs, 0, (a, b) => a + b)
fn return_helper(xs) = {
    if len(xs) == 0 => return 0
    sum_helper(xs)
}
fn make_escape(xs) = {
    if len(xs) == 0 => return (n) => n
    let k = xs[2]
    let s = fold(xs, 0, (a, b) => a + b)
    (n) => n + k + s
}
fn return_escape(xs) = make_escape(xs)(1)

fn measure(name, f) = {
    let mut check = 0
    for i in 0..500 { check = check + f([1, 2, i]) }
    let h = heap()
    for i in 0..4000 { check = check + f([1, 2, i]) }
    println(name + " " + to_string((heap() - h) / 4000) + " " + to_string(check))
}

measure("lambda_only", lambda_only)
measure("return_only", return_only)
measure("return_made", return_made)
measure("return_fold", return_fold)
measure("return_taken", return_taken)
measure("return_map", return_map)
measure("return_filter", return_filter)
measure("return_sort_by", return_sort_by)
measure("return_direct", return_direct)
measure("return_capture", return_capture)
measure("return_capture_varying", return_capture_varying)
measure("return_capture_list", return_capture_list)
measure("return_capture_fn", return_capture_fn)
measure("try_fold", try_row)
measure("match_return", match_return)
measure("nested_return", nested_return)
measure("break_fold", break_fold)
measure("continue_fold", continue_fold)
measure("return_helper", return_helper)
measure("return_escape", return_escape)
"#;

/// Rows as `(name, bytes per call, checksum)`.
fn rows(out: &str) -> Vec<(String, i64, String)> {
    out.lines()
        .map(|line| {
            let parts: Vec<&str> = line.split_whitespace().collect();
            assert_eq!(parts.len(), 3, "unexpected line {line:?} in\n{out}");
            (
                parts[0].to_string(),
                parts[1]
                    .parse()
                    .unwrap_or_else(|_| panic!("bytes in {line:?}")),
                parts[2].to_string(),
            )
        })
        .collect()
}

#[test]
fn a_lambda_in_a_function_with_an_early_exit_leaves_nothing_behind() {
    let ws = workspace("table");
    let tiered = rows(&run(&ws, "table.ol", TABLE, &[]));
    assert_eq!(tiered.len(), 20, "{tiered:?}");
    // Before: 830–2,800 B a call for every row with an early exit and a
    // called lambda. What a row may still grow by is bounded in total
    // (the VM's id-indexed mirror of compiled code), not per call; the
    // bound is generous so an allocator's noise cannot fail it.
    for (name, bytes, _) in &tiered {
        assert!(
            *bytes < 64,
            "{name}: the heap grew {bytes} B a call over 4,000 calls\n{tiered:?}"
        );
    }
    // And the answers are the interpreter's.
    let interpreted = rows(&run(&ws, "table.ol", TABLE, &["--no-ovm"]));
    let answers = |rows: &[(String, i64, String)]| {
        rows.iter()
            .map(|(name, _, check)| (name.clone(), check.clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(answers(&tiered), answers(&interpreted));
}

/// Closures made in functions with early exits escape — returned, stored
/// in a map, sent on a channel, run by a spawned task — and each keeps
/// its own captures, however many closures of the same body were
/// compiled, shared, and evicted around it.
const ESCAPES: &str = r#"
fn adder(k) = {
    if k < 0 => return (n) => 0
    let total = fold([1, 2, 3], 0, (a, b) => a + b)
    (n) => n + k + total - 6
}
fn scaler(xs) = {
    if len(xs) == 0 => return (n) => n
    let m = fold(xs, 0, (a, b) => a + b)
    (n) => n * m + len(xs)
}
fn tagged(label) = {
    if label == "" => return #{ "f": (n) => n }
    let unused = map([1], (x) => x)
    #{ "f": (n) => label + to_string(n) }
}
fn half(x) = if x % 2 == 0 => Ok(x / 2) else => Err("odd")
fn try_adder(x) = {
    let h = half(x)?
    let s = fold([h], 0, (a, b) => a + b)
    Ok((n) => n + s)
}
fn signed(z) = {
    if z > 1.0 => return (n) => "big"
    let unused = fold([1], 0, (a, b) => a + b)
    (n) => to_string(z)
}

let adders = map(range(0, 100), (k) => adder(k))
let mut noise = 0
for i in 0..3000 { noise = noise + adder(i % 3)(1) + scaler([1, i])(2) }
println(to_string(noise))
println(to_string(fold(map(adders, (f) => f(10)), 0, (a, b) => a + b)))
println(to_string(adders[0](1)) + " " + to_string(adders[57](1)) + " " + to_string(adders[99](1)))
let direct = adders[42]
println(to_string(direct(0)))
let s = scaler([2, 3, 4])
println(to_string(s(10)) + " " + to_string(map([1, 2], s)))
let tags = map(["a", "b", "c"], (l) => tagged(l))
println(to_string(map(tags, (t) => map_get(t, "f")(7))))
println(to_string(unwrap(try_adder(8))(1)) + " " + show(is_err(try_adder(3))))
println(signed(0.0)(0) + " " + signed(-0.0)(0) + " " + signed(0.0)(0))
let c = chan.new()
chan.send(c, adder(5))
println(to_string(unwrap(chan.recv(c))(1)))
let t = spawn adder(7)(3)
println(to_string(task.join(t)))
let g = scaler([5])
let t2 = spawn g(2)
println(to_string(task.join(t2)))
"#;

#[test]
fn escaping_closures_keep_their_own_captures_after_an_early_exit() {
    let ws = workspace("escapes");
    let tiered = run(&ws, "escapes.ol", ESCAPES, &[]);
    let expected = "9015000\n5950\n1 58 100\n42\n93 [12, 21]\n[\"a7\", \"b7\", \"c7\"]\n\
                    5 true\n0.0 -0.0 0.0\n6\n10\n11\n";
    assert_eq!(tiered, expected);
    let interpreted = run(&ws, "escapes.ol", ESCAPES, &["--no-ovm"]);
    assert_eq!(interpreted, expected);
}
