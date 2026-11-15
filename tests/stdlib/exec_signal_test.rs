//! OL-375: a child olang started in a process group of its own (a timed
//! `os.exec`, a `proc.spawn` with `#{ group: true }`) is out of the
//! terminal's foreground group, so the terminal's Ctrl-C never reaches
//! it. When that Ctrl-C (or a SIGTERM) ends olang, the child goes too.
#![cfg(unix)]

use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn alive(pid: i32) -> bool {
    // SAFETY: kill with signal 0 only checks the pid.
    unsafe { libc::kill(pid, 0) == 0 }
}

fn wait_for(mut f: impl FnMut() -> bool, limit: Duration) -> bool {
    let end = Instant::now() + limit;
    while Instant::now() < end {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    f()
}

/// Run `body` (which starts a shell that writes its pid to `PIDFILE` and
/// then sleeps) in olang, in a session of its own as a terminal would
/// start it, signal olang's group with `sig`, and answer whether the
/// child outlived olang.
fn child_outlives(body: &str, sig: i32) -> bool {
    let dir = tempfile::tempdir().unwrap();
    let pidfile = dir.path().join("pid");
    let script = dir.path().join("main.ol");
    std::fs::write(&script, body.replace("PIDFILE", pidfile.to_str().unwrap())).unwrap();
    let mut olang = Command::new(env!("CARGO_BIN_EXE_olang"))
        .arg(&script)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .unwrap();
    let read_pid =
        |p: &Path| -> Option<i32> { std::fs::read_to_string(p).ok()?.trim().parse().ok() };
    assert!(
        wait_for(|| read_pid(&pidfile).is_some(), Duration::from_secs(20)),
        "the child never started"
    );
    let child = read_pid(&pidfile).unwrap();
    assert!(alive(child));
    // As a terminal's Ctrl-C does: the signal to the foreground group.
    // SAFETY: signals olang's own group, which the test made.
    unsafe {
        libc::kill(-(olang.id() as i32), sig);
    }
    let status = olang.wait().unwrap();
    assert!(!status.success(), "olang ended by the signal");
    let outlived = !wait_for(|| !alive(child), Duration::from_secs(3));
    if outlived {
        // SAFETY: tidy up the orphan the bug leaves.
        unsafe {
            libc::kill(child, libc::SIGKILL);
        }
    }
    outlived
}

const TIMED_EXEC: &str = r#"
os.exec(["sh", "-c", "echo $$ > PIDFILE; exec sleep 30"], #{ "timeout_ms": 20000 })
"#;

const GROUP_SPAWN: &str = r#"
let p = unwrap(proc.spawn(["sh", "-c", "echo $$ > PIDFILE; exec sleep 30"], #{ "group": true }))
proc.wait(p)
"#;

#[test]
fn ctrl_c_ends_a_timed_exec_child_with_olang() {
    assert!(!child_outlives(TIMED_EXEC, libc::SIGINT));
}

#[test]
fn sigterm_ends_a_timed_exec_child_with_olang() {
    assert!(!child_outlives(TIMED_EXEC, libc::SIGTERM));
}

#[test]
fn ctrl_c_ends_a_grouped_spawn_with_olang() {
    assert!(!child_outlives(GROUP_SPAWN, libc::SIGINT));
}
