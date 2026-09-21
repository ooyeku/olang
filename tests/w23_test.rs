//! Roadmap W23 — the parser, the cliffs, and the checker.

use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("olang_w23_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(dir: &Path, rel: &str, content: &str) -> PathBuf {
    let path = dir.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, content).unwrap();
    path
}

fn olang(dir: &Path, args: &[&str]) -> (String, String, i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_olang"))
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run olang");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
        out.status.code().unwrap_or(-1),
    )
}

// --- The parser: a flat binary expression, built by precedence ---

#[test]
fn the_ladder_is_what_binary_expressions_mean() {
    // One flat grammar rule and precedence climbing replaced a rule per
    // level (the queue a parse needs fell by two thirds). What each level
    // means is unchanged — and every olang file in the repository builds
    // the same tree as before (tests/w21_test.rs holds the chunked parse to
    // the whole one; the change itself was held to 0.86.0's trees for 298
    // files, byte for byte).
    let ws = workspace("ladder");
    write(
        &ws,
        "main.ol",
        "let t = ()\n\
         println(show([\n\
             1 + 2 * 3 == 7 && true || false,\n\
             2 * 3 + 4 << 1,\n\
             1 << 2 < 5,\n\
             10 - 2 - 3,\n\
             [1, 2, 3] |> len == 3,\n\
             sum(0..2 + 2),\n\
             len(map(1..=3, (x) => x)) == 3,\n\
             if t == () => 1 else => 2,\n\
             !false && 1 + 1\n\
                 == 2\n\
         ]))\n",
    );
    for flags in [&["main.ol"][..], &["--no-ovm", "main.ol"][..]] {
        let (out, err, code) = olang(&ws, flags);
        assert_eq!(code, 0, "{flags:?}: {err}");
        // 2*3 + (4<<1) = 14: bitwise binds tighter than the arithmetic.
        assert_eq!(
            out, "[true, 14, true, 5, true, 6, true, 1, true]\n",
            "{flags:?}"
        );
    }
    // A range of a range was refused by the ladder's shape; it still is.
    write(&ws, "bad.ol", "println(show(1..2..3))\n");
    let (out, err, code) = olang(&ws, &["bad.ol"]);
    assert_ne!(code, 0, "{out}{err}");
    assert!(format!("{out}{err}").contains("range"), "{out}{err}");
}

// --- The cliffs: the forms people write cost what the fused form costs ---

#[test]
fn an_accumulation_costs_the_same_however_it_is_spelled() {
    // 0.86.0: `out = out + [x]` in a loop took 1 ms for 60,000 elements
    // and the same loop with a `let` between took 525; a `fold` that
    // builds a map with `map_set` took 3.5 s for 20,000 keys where the
    // loop took 3 ms; a fold that keeps some elements (`if … => acc + [x]
    // else => acc`) and one that matches were the same. All of them now
    // extend in place, on both tiers. The bounds are loose: each form
    // within 25x of the fused loop, where they were 100–1000x.
    let ws = workspace("cliffs");
    write(
        &ws,
        "main.ol",
        "fn fused(xs) = { let mut out = []; for x in xs { out = out + [x * 2] }; out }\n\
         fn apart(xs) = { let mut out = []; for x in xs { let next = out + [x * 2]; out = next }; out }\n\
         fn folded(xs) = fold(xs, [], (acc, x) => acc + [x * 2])\n\
         fn kept(xs) = fold(xs, [], (acc, x) => if x % 2 == 0 => acc + [x] else => acc)\n\
         fn keyed(xs) = fold(xs, #{}, (m, x) => map_set(m, \"k\" + to_string(x), x))\n\
         fn matched(xs) = fold(xs, #{}, (m, x) => match x % 3 { 0 => map_set(m, to_string(x), x), _ => m })\n\
         let xs = range(0, 30000)\n\
         let timed = (f) => { let t0 = time.monotonic(); let r = f(xs); (time.monotonic() - t0, r) }\n\
         let base = max(timed(fused)[0], 1.0)\n\
         let a = timed(apart)\n\
         let b = timed(folded)\n\
         let c = timed(kept)\n\
         let d = timed(keyed)\n\
         let e = timed(matched)\n\
         println(show([a[1] == fused(xs), b[1] == fused(xs), len(c[1]), len(map_keys(d[1])), len(map_keys(e[1]))]))\n\
         println(show(map([a, b, c, d, e], (t) => t[0] / base < 25.0)))\n",
    );
    for flags in [&["main.ol"][..], &["--no-ovm", "main.ol"][..]] {
        let (out, err, code) = olang(&ws, flags);
        assert_eq!(code, 0, "{flags:?}: {err}");
        assert_eq!(
            out, "[true, true, 15000, 30000, 10000]\n[true, true, true, true, true]\n",
            "{flags:?}: {err}"
        );
    }
}

#[test]
fn a_rewritten_accumulator_means_what_it_meant() {
    // The rewrites are of spelling, not meaning: a temporary that is read
    // again is left alone, an aliased accumulator is copied and its alias
    // untouched, a parameter shadowed by a `let` or a pattern is not the
    // parameter, and an annotated `let` keeps its check.
    let ws = workspace("rewrites");
    write(
        &ws,
        "main.ol",
        "fn reads_temp(xs) = { let mut out = []; let mut seen = 0\n\
             for x in xs { let next = out + [x]; seen = seen + len(next); out = next }\n\
             (out, seen) }\n\
         fn keeps_alias(xs) = { let start = [0]; let grown = fold(xs, start, (acc, x) => acc + [x]); (start, grown) }\n\
         fn shadowed(m, k) = { let m = #{ \"other\": 1 }; map_set(m, k, 2) }\n\
         fn arm_binds(m, v) = match v { Ok(m) => map_set(#{}, \"inner\", m), _ => m }\n\
         fn steps(acc, x) = if x > 2 => acc + [x] else => acc\n\
         println(show(reads_temp([1, 2, 3])))\n\
         println(show(keeps_alias([1, 2])))\n\
         println(show(shadowed(#{ \"mine\": 0 }, \"k\")))\n\
         println(show(arm_binds(#{ \"outer\": 1 }, Ok(5))))\n\
         let held = [9]\n\
         println(show((steps(held, 3), steps(held, 1), held)))\n",
    );
    for flags in [&["main.ol"][..], &["--no-ovm", "main.ol"][..]] {
        let (out, err, code) = olang(&ws, flags);
        assert_eq!(code, 0, "{flags:?}: {err}");
        assert_eq!(
            out,
            "([1, 2, 3], 6)\n([0], [0, 1, 2])\n#{\"k\": 2, \"other\": 1}\n#{\"inner\": 5}\n([9, 3], [9], [9])\n",
            "{flags:?}: {err}"
        );
    }
}

#[test]
fn a_loop_that_copies_what_it_builds_is_said() {
    let ws = workspace("copyclass");
    write(
        &ws,
        "olang.toml",
        "[package]\nname = \"copyclass\"\nversion = \"0.1.0\"\n\n[check]\npromote = [\"copy\"]\n",
    );
    write(
        &ws,
        "main.ol",
        "fn a(xs) = { let mut out = []\n\
             for x in xs {\n\
                 let next = out + [x]\n\
                 println(to_string(len(next)))\n\
                 out = next\n\
             }\n\
             out }\n\
         fn b(xs) = { let mut out = []; for x in xs { out = [x] + out }; out }\n\
         fn fine(xs) = { let mut out = []; for x in xs { let next = out + [x]; out = next }; out }\n\
         println(to_string(len(a([1])) + len(b([1])) + len(fine([1]))))\n",
    );
    let (out, err, code) = olang(&ws, &["check", "main.ol"]);
    let all = format!("{out}{err}");
    assert_ne!(code, 0, "{all}");
    assert!(all.contains("is copied on every pass"), "{all}");
    assert!(all.contains("moves every element"), "{all}");
    assert!(
        all.contains("2 problems"),
        "the fused spelling is not reported: {all}"
    );
}

// --- The checker: what the runtime will refuse ---

#[test]
fn the_checker_reports_what_every_run_would_refuse() {
    // 0.86.0 answered "1 file clean" for this file, whose every call fails
    // when run. Each finding is provable: a literal reaches a builtin that
    // refuses it on the function's unconditional path.
    let ws = workspace("needs");
    write(
        &ws,
        "main.ol",
        "fn total(xs) = fold(xs, 0, (acc, x) => acc + x)\n\
         fn label(n) = \"n=\" + n\n\
         fn pick(m) = map_get(m, \"k\")\n\
         fn apply_to(f) = f(1, 2)\n\
         println(to_string(total(5)))\n\
         println(label(3))\n\
         println(to_string(pick(7)))\n\
         println(to_string(apply_to((a) => a)))\n\
         println(to_string(len(map_keys(12))))\n",
    );
    let (out, err, code) = olang(&ws, &["check", "main.ol"]);
    let all = format!("{out}{err}");
    assert_ne!(code, 0, "{all}");
    for expected in [
        "argument 1 of `total` is an Int",
        "argument 1 of `label` is an Int",
        "argument 1 of `pick` is an Int",
        "argument 1 of `apply_to` is a function of 1 parameter",
        "the first argument of `map_keys` is an Int",
        "5 problems",
    ] {
        assert!(all.contains(expected), "missing {expected:?} in:\n{all}");
    }
    // What is not provable is not reported: a guard, a branch, a local of
    // the builtin's name, a value whose type is not known.
    write(
        &ws,
        "quiet.ol",
        "fn total(xs) = if typeof(xs) == \"List\" => fold(xs, 0, (a, x) => a + x) else => xs\n\
         fn either(v) = typeof(v) == \"Map\" && map_has_key(v, \"k\")\n\
         fn local(fold) = fold(5)\n\
         fn unknown(v) = total(v)\n\
         println(show((total(5), either(7), local((n) => n), unknown(5))))\n",
    );
    let (out, err, code) = olang(&ws, &["check", "quiet.ol"]);
    assert_eq!(code, 0, "{out}{err}");
    let (out, err, code) = olang(&ws, &["quiet.ol"]);
    assert_eq!((code, out.as_str()), (0, "(5, false, 5, 5)\n"), "{err}");
}

#[test]
fn a_name_used_and_never_imported_is_an_error() {
    // It resolves in a browser bundle, which is one namespace, from
    // whichever module is spliced beside the file — and is an undefined
    // variable natively. A bare `use` brings every name of its module, a
    // type brings its variants, and a module is its own namespace; none of
    // those is reported.
    let ws = workspace("unimported");
    write(
        &ws,
        "olang.toml",
        "[package]\nname = \"unimported\"\nversion = \"0.1.0\"\n",
    );
    write(
        &ws,
        "lib/nav.ol",
        "share type Msg = enum { Home, Open(Int) }\n\
         share fn go(m) = m\n\
         share fn refresh_fx(m) = [m]\n",
    );
    write(&ws, "lib/all.ol", "share fn everything() = 1\n");
    write(
        &ws,
        "main.ol",
        "use lib.nav { go, Msg }\n\
         use lib.all\n\
         fn step(m) = match m { Home => go(m), Open(n) => refresh_fx(m) }\n\
         println(show((everything(), all.everything(), math.sqrt(4.0), step(Home))))\n",
    );
    let (out, err, code) = olang(&ws, &["check", "main.ol"]);
    let all = format!("{out}{err}");
    assert_ne!(code, 0, "{all}");
    assert!(all.contains("Undefined variable: refresh_fx"), "{all}");
    assert!(all.contains("1 problem"), "only that one: {all}");
    write(
        &ws,
        "main.ol",
        "use lib.nav { go, Msg, refresh_fx }\n\
         use lib.all\n\
         fn step(m) = match m { Home => go(m), Open(n) => refresh_fx(m) }\n\
         println(show((everything(), all.everything(), math.sqrt(4.0), step(Home))))\n",
    );
    let (out, err, code) = olang(&ws, &["check", "main.ol"]);
    assert_eq!(code, 0, "{out}{err}");
}
