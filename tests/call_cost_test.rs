//! What a call costs on the bytecode tier, held as complexity checks:
//! each program times the same work at two sizes (or two ways) in one
//! run and prints whether the ratio stayed in bounds, best of a few
//! tries, so a loaded machine slows both sides alike. Each program's
//! answers are also checked equal with the tier off.

use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("olang_callcost_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(dir: &Path, source: &str, flags: &[&str]) -> String {
    run_with(dir, source, flags, &[])
}

fn run_with(dir: &Path, source: &str, flags: &[&str], env: &[(&str, &str)]) -> String {
    std::fs::write(dir.join("main.ol"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_olang"))
        .args(flags)
        .arg("main.ol")
        .current_dir(dir)
        .env("OLANG_COMPILE_CACHE", "0")
        .envs(env.iter().copied())
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

/// The program's lines that start with `=`: its answers, which must be
/// the same on both tiers.
fn answers(out: &str) -> Vec<&str> {
    out.lines().filter(|l| l.starts_with('=')).collect()
}

fn check(tag: &str, src: &str) {
    check_with(tag, src, &[])
}

fn check_with(tag: &str, src: &str, env: &[(&str, &str)]) {
    let ws = workspace(tag);
    let out = run_with(&ws, src, &[], env);
    for line in out.lines().filter(|l| l.starts_with("ok ")) {
        assert!(line.starts_with("ok true"), "{tag}: {line}\n{out}");
    }
    assert!(out.contains("ok "), "{tag}: no check ran\n{out}");
    let plain = run(&ws, src, &["--no-ovm"]);
    assert_eq!(answers(&out), answers(&plain), "{tag}: the tiers disagree");
}

/// A call through a function value, held in a record field or a local,
/// costs about what a direct call to the same function with the same
/// arguments does (OL-520): it was 80 to 100 ns more a call, two to three
/// times the direct call. (The arguments include a record holding a
/// function.) Both run on the bytecode VM: with machine code on, the
/// direct loop can go native and a call through a value cannot yet.
#[test]
fn a_call_through_a_function_value_costs_about_a_direct_call() {
    check_with(
        "fnvalue",
        r#"fn idp(p, x) = x
fn l_call(n) = { let r = { f: idp }; let t0 = time.monotonic(); let mut acc = 0; let mut i = 0; while i < n { acc = acc + idp(r, i); i = i + 1 }; (time.monotonic() - t0, acc) }
fn l_rec(n) = { let r = { f: idp }; let t0 = time.monotonic(); let mut acc = 0; let mut i = 0; while i < n { acc = acc + r.f(r, i); i = i + 1 }; (time.monotonic() - t0, acc) }
fn l_local(n) = { let r = { f: idp }; let f = r.f; let t0 = time.monotonic(); let mut acc = 0; let mut i = 0; while i < n { acc = acc + f(r, i); i = i + 1 }; (time.monotonic() - t0, acc) }
fn run() = {
    let n = 200000
    let mut best_rec = 100.0
    let mut best_local = 100.0
    for k in 0..4 {
        let d = l_call(n)[0]
        best_rec = math.min(best_rec, l_rec(n)[0] / d)
        best_local = math.min(best_local, l_local(n)[0] / d)
    }
    println("= " + to_string(l_call(1000)[1]) + " " + to_string(l_rec(1000)[1]) + " " + to_string(l_local(1000)[1]))
    println("ok " + to_string(best_rec < 1.6) + " record field " + to_string(best_rec))
    println("ok " + to_string(best_local < 1.6) + " local " + to_string(best_local))
}
run()
"#,
        &[("OLANG_NO_JIT", "1")],
    );
}

/// `b = f(b, x)` through a function value hands `b` over by move, as a
/// direct call does, so the callee's append is in place: O(1) a call,
/// not a copy of the list a call (OL-523).
#[test]
fn a_call_through_a_function_value_moves_a_rebound_argument() {
    check(
        "fnvaluemove",
        r#"type R = struct { f: Any }
fn g(xs, v) = {
    let mut x = xs
    x = x + [v]
    { box: x, n: v }
}
let REC = R { f: g }
fn viarec(n) = { let R2 = REC; let mut b = []; for i in range(0, n) { b = R2.f(b, i).box }; b }
fn viavar(n) = { let f = REC.f; let mut b = []; for i in range(0, n) { b = f(b, i).box }; b }
fn lam(n) = { let f = (xs, v) => { let mut x = xs; x = x + [v]; { box: x } }; let mut b = []; for i in range(0, n) { b = f(b, i).box }; b }
fn per_call(f, n) = { let t0 = time.monotonic(); let b = f(n); (time.monotonic() - t0) / to_float(n) }
fn ratio(f) = {
    let mut best = 100.0
    for k in 0..3 { best = math.min(best, per_call(f, 40000) / per_call(f, 10000)) }
    best
}
fn run() = {
    println("= " + to_string(sum(viarec(300))) + " " + to_string(sum(viavar(300))) + " " + to_string(sum(lam(300))))
    for (name, f) in [("record", viarec), ("local", viavar), ("lambda", lam)] {
        let r = ratio(f)
        println("ok " + to_string(r < 2.0) + " " + name + " " + to_string(r))
    }
}
run()
"#,
    );
}

/// Seconds `olang` takes to run `src` (best of two).
fn run_secs(dir: &Path, src: &str) -> f64 {
    let mut best = f64::MAX;
    for _ in 0..2 {
        let t0 = std::time::Instant::now();
        run(dir, src, &[]);
        best = best.min(t0.elapsed().as_secs_f64());
    }
    best
}

/// Compiling a function costs about linearly in its size, however many
/// lambdas it holds (OL-361): the dead-moves liveness was a registers-by-
/// instructions table, the JIT read every compiled function's signature
/// for each group, and a pipeline step copied the rest of its chain, all
/// quadratic. Four times the lambdas took 9 to 16 times as long.
#[test]
fn compile_time_is_linear_in_the_lambdas_of_a_function() {
    let flat = |n: usize| {
        let mut s = String::from("fn f() = {\n let mut s = 0\n");
        for i in 0..n {
            s += &format!(" let g{i} = (x) => x + {i}\n s = s + g{i}(1)\n");
        }
        s + " s\n}\nprintln(\"= \" + to_string(f()))\n"
    };
    let pipe = |n: usize| {
        "fn f() = 1".to_string()
            + &" |> ((x) => x + 1)".repeat(n)
            + "\nprintln(\"= \" + to_string(f()))\n"
    };
    for (name, make, n) in [("flat", &flat as &dyn Fn(usize) -> String, 1000), ("pipe", &pipe, 900)] {
        let ws = workspace(&format!("lambdas_{name}"));
        let small = make(n);
        let large = make(4 * n);
        assert_eq!(
            answers(&run(&ws, &large, &[])),
            answers(&run(&ws, &large, &["--no-ovm"])),
            "{name}: the tiers disagree"
        );
        // the start-up is the same at both sizes: what grows is the compile
        let base = run_secs(&ws, "println(1)\n");
        let t1 = (run_secs(&ws, &small) - base).max(0.01);
        let t4 = (run_secs(&ws, &large) - base).max(0.01);
        assert!(
            t4 / t1 < 7.0,
            "{name}: {n} lambdas {t1:.3} s, {} lambdas {t4:.3} s",
            4 * n
        );
    }
}

/// `str.index_of`, `str.substring` and `str.starts_with` from compiled
/// code are answered on the VM's strings, not bridged with a conversion
/// each way: on rc.7 each cost 100-170 ns, three to six times a map
/// lookup (OL-522). Their answers, errors included, are the interpreter's.
#[test]
fn string_tests_from_compiled_code_cost_about_a_map_lookup() {
    check(
        "strops",
        r#"fn l_mhk(n, m) = { let t0 = time.monotonic(); let mut acc = 0; let mut i = 0; while i < n { if map_has_key(m, "worker") => acc = acc + 1; i = i + 1 }; time.monotonic() - t0 }
fn l_idx(n, s) = { let t0 = time.monotonic(); let mut acc = 0; let mut i = 0; while i < n { acc = acc + str.index_of(s, ":"); i = i + 1 }; time.monotonic() - t0 }
fn l_sub(n, s) = { let t0 = time.monotonic(); let mut acc = 0; let mut i = 0; while i < n { acc = acc + len(str.substring(s, 0, 8)); i = i + 1 }; time.monotonic() - t0 }
fn l_sw(n, s) = { let t0 = time.monotonic(); let mut acc = 0; let mut i = 0; while i < n { if str.starts_with(s, "net.send") => acc = acc + 1; i = i + 1 }; time.monotonic() - t0 }
fn answers() = {
    let cases = ["net.send:lane/a", "", "é:ü:x", "no colon", ":"]
    for s in cases {
        println("= " + show(str.index_of(s, ":")) + " " + show(str.substring(s, 1, 3)) + " " + show(str.substring(s, 0, 99)) + " " + show(str.substring(s, 3, 1))
            + " " + show(str.starts_with(s, "net")) + " " + show(str.ends_with(s, "x")) + " " + show(str.contains(s, "ü")) + " " + show(str.index_of(s, "")))
    }
    println("= " + show(attempt(() => str.substring("abc", -1, 2))) + " " + show(attempt(() => str.index_of(1, ":"))) + " " + show(attempt(() => str.starts_with("a", 2))))
}
fn run() = {
    answers()
    let s = "net.send:lane/a"
    let m = #{ "worker": 1, "net": 2 }
    let n = 200000
    let mut worst = 100.0
    for k in 0..3 {
        let base = l_mhk(n, m)
        let r = math.max(l_idx(n, s), math.max(l_sub(n, s), l_sw(n, s))) / base
        worst = math.min(worst, r)
    }
    println("ok " + to_string(worst < 2.0) + " " + to_string(worst))
}
run()
"#,
    );
}
