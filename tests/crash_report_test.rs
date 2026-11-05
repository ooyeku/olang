//! Crash reports (OL-446): with `OLANG_CRASH_DIR` or `runtime.on_crash`
//! armed, a Rust panic that ends the process and a fatal signal each
//! leave one JSON report: the message and place, the thread, every
//! thread's olang frames, the native backtrace with the images' UUIDs
//! and load addresses, and the runtime's identity.

#![cfg(target_os = "macos")]

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn olang() -> &'static str {
    env!("CARGO_BIN_EXE_olang")
}

fn tmp(sub: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("olang_crash_{}_{}", sub, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn the_report(dir: &Path) -> serde_json::Value {
    let files: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    assert_eq!(files.len(), 1, "one report: {files:?}");
    let name = files[0].file_name().unwrap().to_string_lossy().to_string();
    assert!(name.starts_with("olang-crash-"), "{name}");
    let text = std::fs::read_to_string(&files[0]).unwrap();
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{e}: {text}"))
}

fn frames_of(report: &serde_json::Value) -> Vec<String> {
    report["threads"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|t| t["frames"].as_array().unwrap().clone())
        .map(|f| f["function"].as_str().unwrap_or("").to_string())
        .collect()
}

fn common_checks(report: &serde_json::Value) {
    assert_eq!(report["kind"], "crash");
    assert_eq!(report["olang"]["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(report["os"]["name"], "macos");
    assert!(report["os"]["version"].as_str().is_some_and(|v| !v.is_empty()));
    assert!(report["time"].as_u64().is_some_and(|t| t > 1_700_000_000));
    let native = report["native"]["frames"].as_array().unwrap();
    assert!(native.len() > 3, "a native backtrace: {native:?}");
    let images = report["native"]["images"].as_array().unwrap();
    let main = &images[0];
    assert!(main["name"].as_str().unwrap().ends_with("olang"), "{main}");
    assert_eq!(main["uuid"].as_str().unwrap().len(), 36, "{main}");
    assert!(main["load_address"].as_str().unwrap().starts_with("0x"));
    // what a report never holds
    let text = report.to_string();
    assert!(!text.contains("SECRET_VALUE"), "no values, no environment");
}

#[test]
fn a_panic_writes_a_report_with_every_threads_olang_frames() {
    let dir = tmp("panic");
    let program = dir.join("p.ol");
    std::fs::write(
        &program,
        format!(
            "let secret = \"SECRET_VALUE\"\n\
             fn waiter(ch) = chan.recv(ch)\n\
             let ch = chan.new()\n\
             let t = spawn waiter(ch)\n\
             time.sleep(100)\n\
             fn arm_again(n) = if n == 0 => runtime.on_crash({:?}) else => arm_again(n - 1)\n\
             fn outer() = arm_again(3)\n\
             outer()\n\
             println(\"not reached\")\n",
            dir.join("reports").display().to_string()
        ),
    )
    .unwrap();
    let out = Command::new(olang())
        .arg(&program)
        .env("OLANG_CRASH_DIR", dir.join("reports"))
        .env("OLANG_TEST_PANIC", "crash.on_crash")
        .env("SECRET_ENV", "SECRET_VALUE")
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success());
    assert!(stderr.contains("A crash report was written to"), "{stderr}");
    let report = the_report(&dir.join("reports"));
    common_checks(&report);
    assert_eq!(report["reason"], "panic");
    assert!(report["message"].as_str().unwrap().contains("crash.on_crash"), "{report}");
    assert!(report["location"].as_str().unwrap().contains(".rs:"), "{report}");
    assert_eq!(report["thread"]["name"], "main");
    let functions = frames_of(&report);
    for f in ["outer", "arm_again", "waiter"] {
        assert!(functions.iter().any(|n| n == f), "{f} in {functions:?}");
    }
    // the crashing thread is marked, and a panic's frames have lines
    let crashed: Vec<_> = report["threads"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|t| t["crashed"] == true)
        .collect();
    assert_eq!(crashed.len(), 1, "{report}");
    let outer = crashed[0]["frames"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["function"] == "outer")
        .expect("outer on the crashing thread");
    assert_eq!(outer["line"], 7, "{outer}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_fatal_signal_writes_a_report_and_the_process_still_dies_of_it() {
    let dir = tmp("signal");
    let program = dir.join("s.ol");
    std::fs::write(
        &program,
        format!(
            "unwrap(runtime.on_crash({:?}, #{{ \"max_bytes\": 32768 }}))\n\
             fn sleeper() = {{ println(\"ready\"); while true {{ time.sleep(20) }} }}\n\
             sleeper()\n",
            dir.join("reports").display().to_string()
        ),
    )
    .unwrap();
    let mut child = Command::new(olang())
        .arg(&program)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    assert_eq!(line.trim(), "ready");
    std::thread::sleep(std::time::Duration::from_millis(100));
    let t0 = std::time::Instant::now();
    unsafe { libc::kill(child.id() as i32, libc::SIGSEGV) };
    let mut status = None;
    while t0.elapsed().as_secs() < 10 {
        if let Some(st) = child.try_wait().unwrap() {
            status = Some(st);
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let Some(status) = status else {
        let _ = child.kill();
        panic!("the process lived on after SIGSEGV");
    };
    use std::os::unix::process::ExitStatusExt;
    assert_eq!(status.signal(), Some(libc::SIGSEGV), "it still dies of the signal");
    assert!(t0.elapsed().as_millis() < 2000);
    let report = the_report(&dir.join("reports"));
    common_checks(&report);
    assert_eq!(report["reason"], "signal");
    assert_eq!(report["signal"], "SIGSEGV");
    let functions = frames_of(&report);
    assert!(functions.iter().any(|n| n == "sleeper"), "{functions:?}");
    assert!(std::fs::metadata(std::fs::read_dir(dir.join("reports")).unwrap().next().unwrap().unwrap().path()).unwrap().len() <= 32768);
    let _ = std::fs::remove_dir_all(&dir);
}
