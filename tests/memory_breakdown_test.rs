//! `runtime.memory()` splits the heap into what the loaded program holds
//! (`program`), what the runtime's own code holds once the program runs
//! (`code`: functions compiled to bytecode, their machine code, bodies
//! built on first read, bridge interpreters) and the rest (`values`).
//!
//! `values` was the heap minus the program alone (OL-418), so everything
//! the tiers compiled after load counted as the program's values: about
//! 130 MB of Studio's "values" was compiled code, which sent two
//! investigations looking in the wrong place.

use std::process::Command;

fn run(source: &str, args: &[&str]) -> String {
    let dir = std::env::temp_dir().join(format!("olang_mem_split_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join(format!("m{}.ol", args.len()));
    std::fs::write(&file, source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_olang"))
        .args(args)
        .arg(&file)
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

/// A program of 400 functions, each called enough to compile, that then
/// builds a list of 100,000 strings (about 7.8 MB). It prints, after the
/// calls and after the list, `heap program code values`.
fn source() -> String {
    let mut src = String::new();
    for i in 0..400 {
        src.push_str(&format!(
            "fn f{i}(x) = {{\n  let mut acc = x\n  for k in 0..3 {{ acc = acc + k * {i} + len(to_string(acc)) }}\n  if acc % 2 == 0 => acc / 2 else => acc * 3 + 1\n}}\n"
        ));
    }
    src.push_str("let mut total = 0\nfor r in 0..3 {\n");
    for i in 0..400 {
        src.push_str(&format!("  total = total + f{i}(r)\n"));
    }
    src.push_str("}\n");
    src.push_str(
        "fn show_mem() = { let m = runtime.memory(); println(to_string(m.heap) + \" \" + to_string(m.program) + \" \" + to_string(m.code) + \" \" + to_string(m.values)) }\n\
         show_mem()\n\
         let mut xs = []\n\
         for i in 0..100000 { xs = xs + [to_string(i)] }\n\
         show_mem()\n\
         println(len(xs) + total * 0)\n",
    );
    src
}

fn figures(line: &str) -> [i64; 4] {
    let v: Vec<i64> = line
        .split_whitespace()
        .map(|n| n.parse().unwrap())
        .collect();
    [v[0], v[1], v[2], v[3]]
}

#[test]
fn compiled_code_is_counted_apart_from_the_programs_values() {
    let out = run(&source(), &["--ovm-tier=1"]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 3, "{out}");
    let before = figures(lines[0]);
    let after = figures(lines[1]);
    for f in [before, after] {
        assert_eq!(
            f[1] + f[2] + f[3],
            f[0],
            "program + code + values = heap: {out}"
        );
    }
    // 400 functions compiled to bytecode are the runtime's, not values
    assert!(
        before[2] > 1 << 20,
        "code holds the compiled functions: {out}"
    );
    // the list is the program's own value, and none of it is code
    let values_grew = after[3] - before[3];
    let code_grew = after[2] - before[2];
    assert!(values_grew > 5 << 20, "values hold the list: {out}");
    assert!(
        code_grew.abs() < 1 << 20,
        "code did not grow with the list: {out}"
    );
}

#[test]
fn without_the_tier_code_is_the_bodies_built_on_first_call() {
    let out = run(&source(), &["--no-ovm"]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 3, "{out}");
    let before = figures(lines[0]);
    let after = figures(lines[1]);
    for f in [before, after] {
        assert_eq!(f[1] + f[2] + f[3], f[0], "{out}");
    }
    // 400 bodies, each built the first time its function was called
    assert!(before[2] > 256 << 10, "code holds the built bodies: {out}");
    assert!(
        after[3] - before[3] > 5 << 20,
        "values hold the list: {out}"
    );
    assert_eq!(
        after[2], before[2],
        "code did not grow with the list: {out}"
    );
}
