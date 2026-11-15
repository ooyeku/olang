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
    let tag: String = args
        .join("")
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect();
    let file = dir.join(format!("m_{tag}.ol"));
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

const SHOW_MEM: &str = "fn show_mem() = { let m = runtime.memory(); println(to_string(m.heap) + \" \" + to_string(m.program) + \" \" + to_string(m.code) + \" \" + to_string(m.values)) }\n";

/// How much `code` must grow, at the least, for each of the 400
/// functions called: a compiled function, or a body built, is kilobytes;
/// a floor far under that keeps the check about where the bytes are
/// counted, not about how many a compile takes.
const PER_FUNCTION_FLOOR: i64 = 256;

/// A program of 400 functions, each called enough to compile, that then
/// builds a list of 100,000 strings (about 7.8 MB). It prints, before
/// the calls, after them and after the list, `heap program code values`.
fn source() -> String {
    let mut src = String::new();
    for i in 0..400 {
        src.push_str(&format!(
            "fn f{i}(x) = {{\n  let mut acc = x\n  for k in 0..3 {{ acc = acc + k * {i} + len(to_string(acc)) }}\n  if acc % 2 == 0 => acc / 2 else => acc * 3 + 1\n}}\n"
        ));
    }
    src.push_str(SHOW_MEM);
    src.push_str("show_mem()\nlet mut total = 0\nfor r in 0..3 {\n");
    for i in 0..400 {
        src.push_str(&format!("  total = total + f{i}(r)\n"));
    }
    src.push_str("}\n");
    src.push_str(
        "show_mem()\n\
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

/// The three lines' figures, each checked to add up to the heap.
fn three(out: &str) -> [[i64; 4]; 3] {
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 4, "{out}");
    let f = [figures(lines[0]), figures(lines[1]), figures(lines[2])];
    for x in f {
        assert_eq!(
            x[1] + x[2] + x[3],
            x[0],
            "program + code + values = heap: {out}"
        );
    }
    f
}

/// What a run shows, whichever tier: the calls grow `code` by more than
/// the floor a function, and the list grows `values` and leaves `code`.
fn split_holds(out: &str) {
    let [start, called, listed] = three(out);
    assert!(
        called[2] - start[2] > 400 * PER_FUNCTION_FLOOR,
        "the 400 functions' code is counted under code: {out}"
    );
    assert!(
        listed[3] - called[3] > 5 << 20,
        "values hold the list: {out}"
    );
    assert!(
        (listed[2] - called[2]).abs() < 256 << 10,
        "code did not grow with the list: {out}"
    );
}

#[test]
fn compiled_code_is_counted_apart_from_the_programs_values() {
    split_holds(&run(&source(), &["--ovm-tier=1"]));
    split_holds(&run(&source(), &[]));
}

#[test]
fn without_the_tier_code_is_the_bodies_built_on_first_call() {
    split_holds(&run(&source(), &["--no-ovm"]));
}
