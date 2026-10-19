//! A kernel (`olang repl --serve`, a notebook's or not) and the end of its
//! input: requests piped then the input closed are every one answered,
//! then it ends; its parent gone, it ends within a second even mid-loop
//! (orphaned kernels spun at full speed for minutes after their editor
//! died); its replies' reader gone, it ends.

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn olang() -> &'static str {
    env!("CARGO_BIN_EXE_olang")
}

fn notebook(dir: &std::path::Path) -> std::path::PathBuf {
    let nb = dir.join("k.olmd");
    std::fs::write(&nb, "# K\n\n```olang\nlet a = 1\n```\n").unwrap();
    nb
}

fn args(nb: Option<&std::path::Path>) -> Vec<String> {
    let mut a = vec!["repl".to_string(), "--serve".to_string()];
    if let Some(nb) = nb {
        a.extend(["--notebook".to_string(), nb.to_str().unwrap().to_string()]);
    }
    a
}

fn piped_then_closed(with_notebook: bool) {
    let dir = tempfile::tempdir().unwrap();
    let nb = notebook(dir.path());
    let mut child = Command::new(olang())
        .args(args(with_notebook.then_some(nb.as_path())))
        .current_dir(dir.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    {
        let mut stdin = child.stdin.take().unwrap();
        for i in 1..=3 {
            writeln!(stdin, r#"{{"op":"eval","id":{i},"cell":"c{i}","code":"{i} + 40","line":4}}"#).unwrap();
        }
    } // closed right after the requests
    let out = child.wait_with_output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("protocol"), "the hello: {text}");
    for i in 1..=3 {
        assert!(text.contains(&format!("\"s\":\"{}\"", i + 40)), "reply {i} in: {text}");
    }
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn requests_piped_then_closed_are_every_one_answered() {
    piped_then_closed(true);
    piped_then_closed(false);
}

fn alive(pid: &str) -> bool {
    Command::new("kill").args(["-0", pid]).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false)
}

// A kernel started by a shell that then ends, its input kept open and a
// cell looping for ever: it ends within a second or two of its parent.
fn ends_with_its_parent(with_notebook: bool) {
    let dir = tempfile::tempdir().unwrap();
    let nb = notebook(dir.path());
    let a = args(with_notebook.then_some(nb.as_path())).join(" ");
    let eval = r#"{"op":"eval","id":1,"cell":"c1","code":"let mut i = 0\nwhile true { i = i + 1 }","line":4}"#;
    std::fs::write(dir.path().join("req"), format!("{eval}\n")).unwrap();
    let script = format!("(cat req; sleep 30) | '{}' {} >/dev/null 2>&1 & echo $!; sleep 0.8", olang(), a);
    let out = Command::new("sh").args(["-c", &script]).current_dir(dir.path()).output().unwrap();
    let pid = String::from_utf8_lossy(&out.stdout).trim().to_string();
    assert!(!pid.is_empty());
    let t0 = Instant::now();
    while alive(&pid) && t0.elapsed() < Duration::from_secs(4) {
        std::thread::sleep(Duration::from_millis(50));
    }
    let gone = !alive(&pid);
    if !gone {
        let _ = Command::new("kill").args(["-9", &pid]).status();
    }
    assert!(gone, "the kernel went on after its parent ended");
    assert!(t0.elapsed() < Duration::from_secs(3), "ended in {:?}", t0.elapsed());
}

#[test]
fn a_notebook_kernel_ends_with_its_parent_mid_loop() {
    ends_with_its_parent(true);
}

#[test]
fn a_repl_session_ends_with_its_parent_mid_loop() {
    ends_with_its_parent(false);
}
