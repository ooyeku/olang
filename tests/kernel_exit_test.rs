//! A kernel (`olang repl --serve`, a notebook's or not) ends when whoever
//! started it is gone: its stdin closed, even while a cell loops for ever
//! (orphaned kernels spun at full speed for minutes after their editor died).

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn ends_when_stdin_closes(notebook: bool) {
    let dir = tempfile::tempdir().unwrap();
    let mut args = vec!["repl", "--serve"];
    let nb = dir.path().join("k.olmd");
    if notebook {
        std::fs::write(&nb, "# K\n\n```olang\nlet a = 1\n```\n").unwrap();
        args.extend(["--notebook", nb.to_str().unwrap()]);
    }
    let mut child = Command::new(env!("CARGO_BIN_EXE_olang"))
        .args(&args)
        .current_dir(dir.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut out = BufReader::new(child.stdout.take().unwrap());
    let mut hello = String::new();
    out.read_line(&mut hello).unwrap();
    assert!(hello.contains("protocol"), "{hello}");
    let mut stdin = child.stdin.take().unwrap();
    let eval = if notebook {
        r#"{"op":"eval","id":1,"cell":"c1","code":"let mut i = 0\nwhile true { i = i + 1 }","line":4}"#
    } else {
        r#"{"op":"eval","id":1,"code":"let mut i = 0\nwhile true { i = i + 1 }"}"#
    };
    writeln!(stdin, "{eval}").unwrap();
    stdin.flush().unwrap();
    std::thread::sleep(Duration::from_millis(300));
    assert!(child.try_wait().unwrap().is_none(), "it loops");
    drop(stdin);
    let t0 = Instant::now();
    let mut ended = None;
    while t0.elapsed() < Duration::from_secs(3) {
        if let Some(st) = child.try_wait().unwrap() {
            ended = Some(st);
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    if ended.is_none() {
        let _ = child.kill();
        let _ = child.wait();
        panic!("the kernel went on after its stdin closed");
    }
    assert!(t0.elapsed() < Duration::from_secs(2), "ended in {:?}", t0.elapsed());
}

#[test]
fn a_notebook_kernel_ends_when_its_stdin_closes_mid_loop() {
    ends_when_stdin_closes(true);
}

#[test]
fn a_repl_session_ends_when_its_stdin_closes_mid_loop() {
    ends_when_stdin_closes(false);
}
