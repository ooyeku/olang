//! The cycle collector (OL-570): a loop through a cell or a shared
//! reference is freed once nothing else holds it, and nothing anything
//! still holds is ever touched.
//!
//! Each program runs on both tiers: the bytecode VM keeps its own form
//! of values in a cell, and its closures capture differently.

use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("olang_cycles_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(dir: &Path, source: &str, args: &[&str]) -> String {
    std::fs::write(dir.join("main.ol"), source).unwrap();
    let mut all: Vec<&str> = args.to_vec();
    all.push("main.ol");
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

fn both_tiers(tag: &str, source: &str) -> Vec<String> {
    let ws = workspace(tag);
    let a = run(&ws, source, &[]);
    let b = run(&ws, source, &["--no-ovm"]);
    vec![a, b]
}

/// Megabytes of values after the program's own `runtime.collect()`.
const MB: &str = "fn mb() = runtime.memory().values / 1048576\n";

#[test]
fn loops_nothing_holds_are_freed() {
    let src = format!(
        "{MB}\
fn listed() = {{ let c = cell(0); cell.set(c, [c, str.repeat(\"x\", 4000)]); () }}\n\
fn closed() = {{ let s = str.repeat(\"y\", 4000); let c = cell(0); cell.set(c, () => len(s) + len(to_string(cell.get(c)))); () }}\n\
fn mapped() = {{ let c = cell(0); cell.set(c, #{{ \"me\": Ok([c]), \"pad\": str.repeat(\"z\", 4000) }}); () }}\n\
fn pointed() = {{ let s = shared.new(0); shared.set(s, [s, str.repeat(\"w\", 4000)]); () }}\n\
fn pair() = {{ let a = cell(0); let b = cell(0); cell.set(a, [b, str.repeat(\"v\", 4000)]); cell.set(b, [a]); () }}\n\
fn handed() = {{ let s = str.repeat(\"u\", 4000); let c = cell(0); let f = (x) => x + len(s) + len(typeof(cell.get(c))); cell.set(c, f); len(map([1, 2, 3], f)) }}\n\
for i in range(0, 3000) {{ listed(); closed(); mapped(); pointed(); pair(); handed() }}\n\
let c = runtime.collect()\n\
println(mb() < 8)\n\
println(c.freed >= 0)\n"
    );
    for out in both_tiers("freed", &src) {
        assert_eq!(out, "true\ntrue\n", "{out}");
    }
}

#[test]
fn loops_are_found_without_being_asked() {
    // 60,000 loops of 4 KB each would be 240 MB kept.
    let src = format!(
        "{MB}\
fn leak() = {{ let c = cell(0); cell.set(c, [c, str.repeat(\"x\", 4000)]); () }}\n\
for i in range(0, 60000) {{ leak() }}\n\
println(mb() < 64)\n"
    );
    for out in both_tiers("auto", &src) {
        assert_eq!(out, "true\n", "{out}");
    }
}

#[test]
fn a_loop_something_holds_is_kept() {
    let src = "\
let top = cell(0)\n\
cell.set(top, [top, \"top\"])\n\
let reader = cell(0)\n\
cell.set(reader, () => cell.get(reader))\n\
let made = { let hidden = cell(5); () => cell.get(hidden) }\n\
let boxed = Ok(#{ \"c\": { let c = cell(0); cell.set(c, [c, \"boxed\"]); c } })\n\
let point = shared.new(0)\n\
shared.set(point, [point, \"shared\"])\n\
fn framed() = {\n\
    let c = cell(0)\n\
    let g = () => cell.get(c)\n\
    cell.set(c, g)\n\
    runtime.collect()\n\
    typeof(cell.get(c))\n\
}\n\
fn deep() = {\n\
    let c = cell(0)\n\
    cell.set(c, [c, \"deep\"])\n\
    let inner = () => { runtime.collect(); cell.get(c)[1] }\n\
    inner()\n\
}\n\
let back = cell(())\n\
let w = weak.new(back)\n\
cell.set(back, () => cell.get(weak.get(w)))\n\
runtime.collect()\n\
runtime.collect()\n\
println(cell.get(top)[1])\n\
println(typeof(cell.get(reader)))\n\
println(typeof(cell.get(reader)()))\n\
println(made())\n\
println(cell.get(unwrap(boxed).c)[1])\n\
println(shared.get(point)[1])\n\
println(framed())\n\
println(deep())\n\
println(typeof(cell.get(back)))\n";
    for out in both_tiers("kept", src) {
        assert_eq!(
            out, "top\nFunction\nFunction\n5\nboxed\nshared\nFunction\ndeep\nFunction\n",
            "{out}"
        );
    }
}

#[test]
fn a_loop_a_running_task_holds_is_kept() {
    let src = "\
let c = cell(0)\n\
cell.set(c, [c, 41])\n\
let ch = chan.new()\n\
let t = spawn {\n\
    let mine = cell(0)\n\
    cell.set(mine, [mine, 1])\n\
    unwrap(chan.recv(ch))\n\
    cell.get(mine)[1]\n\
}\n\
for i in range(0, 20) { runtime.collect() }\n\
unwrap(chan.send(ch, ()))\n\
println(cell.get(c)[1] + unwrap(task.join(t)))\n";
    for out in both_tiers("task", src) {
        assert_eq!(out, "42\n", "{out}");
    }
}
