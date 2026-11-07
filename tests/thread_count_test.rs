//! What a program's threads cost: the parallel pool is made by the first
//! parallel operation and ended when idle, and `task.set_parallelism`
//! caps it (OL-552); children's pipes are read by one shared thread, not
//! two a child (OL-497). Counts the real binary's threads with `ps -M`.
#![cfg(target_os = "macos")]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

fn workspace(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("olang_threads_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn threads(pid: u32) -> usize {
    let out = Command::new("ps")
        .args(["-M", "-p", &pid.to_string()])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .count()
        .saturating_sub(1)
}

fn wait_for(dir: &Path, mark: &str) {
    let until = Instant::now() + Duration::from_secs(30);
    while !dir.join(mark).exists() {
        assert!(Instant::now() < until, "the program never reached {mark}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// The program marks each phase with a file and waits in it.
const PHASES: &str = r#"
fs.write_file("a", "")
time.sleep(1000)
let s = sort(map(range(0, 300000), (i) => (i * 7919) % 100003))
fs.write_file("b", "")
time.sleep(5000)
fs.write_file("c", "")
let ps = map(range(0, 6), (i) => unwrap(proc.spawn(["sleep", "3"])))
fs.write_file("d", "")
time.sleep(1500)
"#;

fn run(tag: &str, prelude: &str) -> (usize, usize, usize, usize) {
    let ws = workspace(tag);
    std::fs::write(ws.join("main.ol"), format!("{prelude}\n{PHASES}")).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_olang"))
        .current_dir(&ws)
        .arg("main.ol")
        .env_remove("OLANG_PARALLELISM")
        .spawn()
        .unwrap();
    let pid = child.id();
    wait_for(&ws, "a");
    std::thread::sleep(Duration::from_millis(300));
    let before = threads(pid);
    wait_for(&ws, "b");
    let parallel = threads(pid);
    wait_for(&ws, "c");
    let idle = threads(pid);
    wait_for(&ws, "d");
    std::thread::sleep(Duration::from_millis(300));
    let kids = threads(pid);
    assert!(child.wait().unwrap().success());
    (before, parallel, idle, kids)
}

#[test]
fn the_pool_comes_with_parallel_work_and_goes_when_idle_and_children_share_one_reader() {
    let (before, parallel, idle, kids) = run("lazy", "");
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    if cores > 1 {
        assert!(
            parallel >= before + cores.min(4),
            "the pool grows to the cores: {before} threads before, {parallel} sorting"
        );
    }
    // the runtime starts a thread or two of its own with the hot lambda
    // (the tiers' compiler); the pool's 18 are what must be gone
    assert!(
        idle <= before + 2,
        "an idle pool ends: {before} before, {idle} idle"
    );
    assert!(
        kids <= idle + 1,
        "six children cost one reader thread: {idle} before, {kids} with them"
    );
}

#[test]
fn a_program_caps_its_pool() {
    let (before, parallel, _, _) = run("capped", "let n = task.set_parallelism(2)\nassert(n == 2)");
    assert!(
        parallel <= before + 5,
        "two workers, the idle watch and the runtime's own: {before} before, {parallel} sorting"
    );
}
