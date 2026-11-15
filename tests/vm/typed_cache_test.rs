//! A list the typed-conversion cache saw is let go once nothing else
//! holds it (OL-412).
//!
//! An interpreter list of numbers crossing into compiled code converts
//! to its typed form once, and the cache answers that conversion again
//! for the same list. It held the list and its typed copy strongly, the
//! last sixteen of them, so a loop over `len(map(range(0, n), f))` kept
//! sixteen dead results alive: 75 MB at n = 100,000.

use std::process::Command;

#[test]
fn dead_results_of_a_map_over_a_range_are_let_go() {
    let dir = std::env::temp_dir().join(format!("olang_typedcache_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let source = "let h = () => runtime.memory().heap\n\
fn big(n) = map(range(0, n), (i) => i + 1)\n\
fn run() = {\n\
    let base = h()\n\
    let mut i = 0\n\
    let mut total = 0\n\
    while i < 40 {\n\
        total = total + len(big(100000))\n\
        i = i + 1\n\
    }\n\
    println(to_string(total))\n\
    println(to_string((h() - base) / 1024))\n\
}\n\
run()\n";
    std::fs::write(dir.join("main.ol"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_olang"))
        .arg("main.ol")
        .current_dir(&dir)
        .env("OLANG_COMPILE_CACHE", "0")
        .output()
        .expect("run olang");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "4000000");
    let held: i64 = lines[1].trim().parse().unwrap();
    // one result is about 4,700 KB; one dead typed copy (800 KB) may wait
    // for the next conversion
    assert!(held < 2048, "40 dead results still hold {held} KB");
}
