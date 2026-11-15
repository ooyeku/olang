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
