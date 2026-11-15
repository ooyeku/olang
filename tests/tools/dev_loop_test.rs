//! The dev-loop pair: `olang --watch` (rerun on change) and the REPL's
//! deep `:type`. Both drive the real binary.

use std::io::Write as _;
use std::process::{Command, Stdio};
use std::time::Duration;

fn olang_bin() -> &'static str {
    env!("CARGO_BIN_EXE_olang")
}

#[test]
fn repl_type_reports_deep_types() {
    let mut child = Command::new(olang_bin())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn repl");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b":type [1, 2, 3]\n:type Ok([1.5])\n:type (x: Int) => x\n:quit\n")
        .unwrap();
    let out = child.wait_with_output().expect("repl run");
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(text.contains("List<Int>"), "deep list type: {}", text);
    assert!(
        text.contains("Result<List<Float>, _>"),
        "result payload type: {}",
        text
    );
    assert!(text.contains("(Int) -> ?"), "function signature: {}", text);
}

#[test]
fn watch_reruns_when_the_file_changes() {
    let dir = std::env::temp_dir().join(format!("olang-watch-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("w.ol");
    std::fs::write(&script, "println(\"run A\")\n").unwrap();

    let mut child = Command::new(olang_bin())
        .arg("--watch")
        .arg(&script)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn watcher");

    // Let the first run land, then change the file (mtime granularity on
    // some filesystems is a full second — wait it out).
    std::thread::sleep(Duration::from_millis(1500));
    std::fs::write(&script, "println(\"run B\")\n").unwrap();
    std::thread::sleep(Duration::from_millis(1500));

    child.kill().expect("stop watcher");
    let out = child.wait_with_output().expect("collect output");
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(text.contains("run A"), "first run: {}", text);
    assert!(text.contains("run B"), "rerun after change: {}", text);
}
