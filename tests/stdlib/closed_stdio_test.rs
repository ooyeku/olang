//! A process whose stdout or stderr has no reader any more — an editor
//! that stopped its language server, its REPL, a test run, a tier check
//! or a profile, or a shell pipeline whose reader left — ends quietly:
//! never a panic about the printing, never an abort (which leaves a crash
//! report behind). Stdout gone ends the process with 141 (128 + SIGPIPE);
//! stderr gone drops what would have been said.
//!
//! Each case spawns the real binary with the stream(s) a pipe whose read
//! end is already closed, so every write fails with EPIPE from the first.
#![cfg(unix)]

use std::io::Write;
use std::os::fd::{FromRawFd, OwnedFd};
use std::os::unix::process::ExitStatusExt;
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

fn olang() -> Command {
    // CLOSED_STDIO_OLANG runs the cases against another build (one from
    // before the fix, to see them fail)
    let bin = std::env::var_os("CLOSED_STDIO_OLANG")
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_olang").into());
    let mut c = Command::new(bin);
    // a panic's report would be the same either way; keep it short
    c.env_remove("RUST_BACKTRACE");
    c
}

/// A pipe's write end whose read end is closed: every write is EPIPE.
fn dead_pipe() -> Stdio {
    let mut fds = [0; 2];
    assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0, "pipe");
    unsafe { libc::close(fds[0]) };
    Stdio::from(unsafe { OwnedFd::from_raw_fd(fds[1]) })
}

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("loud.ol"),
        "for i in 0..2000 { println(\"line \" + to_string(i)) }\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("fails.ol"),
        "println(\"before\")\nlet x = unwrap(Err(\"boom\"))\nprintln(x)\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("broken.ol"),
        "let x = \nfn f( = 3\nprintln(undefined_name)\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("calc.ol"),
        "fn add(a, b) = a + b\nfn twice(f, x) = f(f(x))\nlet r = twice((v) => add(v, 1), 3)\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("calc_test.ol"),
        "test \"adds\" { assert_eq(1 + 2, 3) }\ntest \"fails\" { assert_eq(1 + 2, 4) }\ntest \"again\" { assert(true) }\n",
    )
    .unwrap();
    dir
}

/// Wait for `child` at most `secs`; kill it and fail if it is still running.
fn wait(mut child: std::process::Child, secs: u64, what: &str) -> ExitStatus {
    let t0 = Instant::now();
    loop {
        if let Some(s) = child.try_wait().expect("try_wait") {
            return s;
        }
        if t0.elapsed() > Duration::from_secs(secs) {
            let _ = child.kill();
            let _ = child.wait();
            panic!("{what}: still running after {secs} s");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn quiet(status: ExitStatus, what: &str) {
    assert_eq!(
        status.signal(),
        None,
        "{what}: ended by a signal ({status:?}) — a panic about printing that aborted"
    );
    assert_ne!(status.code(), Some(101), "{what}: a panic ({status:?})");
}

/// `olang <args>` in `dir` with the given stdout and stderr.
fn run(dir: &std::path::Path, args: &[&str], out: Stdio, err: Stdio) -> ExitStatus {
    let child = olang()
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(out)
        .stderr(err)
        .spawn()
        .expect("spawn olang");
    wait(child, 60, &args.join(" "))
}

#[test]
fn a_program_printing_to_a_closed_stdout_ends_with_141() {
    let p = project();
    for err in [Stdio::null(), dead_pipe()] {
        let s = run(p.path(), &["loud.ol"], dead_pipe(), err);
        quiet(s, "loud.ol");
        assert_eq!(s.code(), Some(141), "loud.ol: {s:?}");
    }
}

#[test]
fn an_error_reported_to_a_closed_stderr_ends_with_its_status() {
    let p = project();
    let s = run(p.path(), &["fails.ol"], Stdio::null(), dead_pipe());
    quiet(s, "fails.ol, stderr closed");
    assert_eq!(s.code(), Some(1), "fails.ol: {s:?}");
    // both closed: the first println ends it
    let s = run(p.path(), &["fails.ol"], dead_pipe(), dead_pipe());
    quiet(s, "fails.ol, both closed");
}

#[test]
fn check_with_its_output_closed() {
    let p = project();
    for (out, err) in [
        (dead_pipe(), Stdio::null()),
        (Stdio::null(), dead_pipe()),
        (dead_pipe(), dead_pipe()),
    ] {
        quiet(
            run(p.path(), &["check", "broken.ol"], out, err),
            "check broken.ol",
        );
    }
    for args in [
        &["check", "--tier", "calc.ol"][..],
        &["check", "--tier", "--format", "json", "calc.ol"][..],
    ] {
        for (out, err) in [(dead_pipe(), Stdio::null()), (dead_pipe(), dead_pipe())] {
            quiet(run(p.path(), args, out, err), &args.join(" "));
        }
    }
}

#[test]
fn a_test_run_with_its_output_closed() {
    let p = project();
    for format in ["text", "json"] {
        for (out, err) in [
            (dead_pipe(), Stdio::null()),
            (Stdio::null(), dead_pipe()),
            (dead_pipe(), dead_pipe()),
        ] {
            let s = run(
                p.path(),
                &["test", "--format", format, "calc_test.ol"],
                out,
                err,
            );
            quiet(s, &format!("test --format {format}"));
        }
    }
}

#[test]
fn a_profile_and_a_bench_with_their_output_closed() {
    let p = project();
    for args in [
        &["profile", "calc.ol"][..],
        &["profile", "--format", "json", "calc.ol"][..],
        &["bench", "calc.ol"][..],
    ] {
        for (out, err) in [(dead_pipe(), Stdio::null()), (dead_pipe(), dead_pipe())] {
            quiet(run(p.path(), args, out, err), &args.join(" "));
        }
    }
}

/// A server whose editor has gone: its replies (stdout) and its notes
/// (stderr) have nowhere to go; a request and then the end of its input
/// end it quietly.
fn serve(args: &[&str], request: &str, out: Stdio, err: Stdio) -> ExitStatus {
    let p = project();
    let mut child = olang()
        .args(args)
        .current_dir(p.path())
        .stdin(Stdio::piped())
        .stdout(out)
        .stderr(err)
        .spawn()
        .expect("spawn olang");
    {
        let mut stdin = child.stdin.take().unwrap();
        let _ = stdin.write_all(request.as_bytes());
        let _ = stdin.flush();
        std::thread::sleep(Duration::from_millis(500));
    }
    wait(child, 30, &args.join(" "))
}

#[test]
fn a_repl_server_with_its_output_closed() {
    let eval = concat!(
        r#"{"id":1,"op":"eval","code":"1 + 2"}"#,
        "\n",
        r#"{"id":2,"op":"eval","code":"unwrap(Err(\"boom\"))"}"#,
        "\n"
    );
    for (out, err) in [
        (dead_pipe(), Stdio::null()),
        (Stdio::null(), dead_pipe()),
        (dead_pipe(), dead_pipe()),
    ] {
        quiet(serve(&["repl", "--serve"], eval, out, err), "repl --serve");
    }
}

#[test]
fn a_language_server_with_its_output_closed() {
    let body =
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"capabilities\":{}}}";
    let msg = format!("Content-Length: {}\r\n\r\n{}", body.len(), body);
    for (out, err) in [
        (dead_pipe(), Stdio::null()),
        (Stdio::null(), dead_pipe()),
        (dead_pipe(), dead_pipe()),
    ] {
        quiet(serve(&["lsp"], &msg, out, err), "lsp");
    }
}
