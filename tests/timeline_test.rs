//! The Open Timeline end to end: a run's nondeterministic inputs
//! (random, time, env) are recorded to a portable .olt trace, and
//! `olang replay` reproduces the run bit-for-bit — even from a directory
//! where the original program no longer exists, and even when the run
//! crashed. Divergence is detected. Runs the real binary.

use std::path::PathBuf;
use std::process::Command;

fn olang() -> &'static str {
    env!("CARGO_BIN_EXE_olang")
}

fn workspace(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("olang_tl_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(path: &std::path::Path, s: &str) {
    std::fs::write(path, s).unwrap();
}

const FLAKY: &str = "\
let now = time.now_ms()
let dice = random.randint(1, 1000000)
let token = crypto.random_hex(8)
println(show(now) + \" \" + show(dice) + \" \" + token)
";

fn stdout_of(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[test]
fn replay_reproduces_a_recorded_run_bit_for_bit() {
    let ws = workspace("repro");
    write(&ws.join("flaky.ol"), FLAKY);

    let rec = Command::new(olang())
        .current_dir(&ws)
        .args(["--record", "run.olt", "flaky.ol"])
        .output()
        .unwrap();
    assert!(rec.status.success());
    let recorded = stdout_of(&rec);
    assert!(ws.join("run.olt").exists(), "trace not written");

    // A live run differs (overwhelmingly — dice is 1..1_000_000).
    let live = Command::new(olang())
        .current_dir(&ws)
        .arg("flaky.ol")
        .output()
        .unwrap();
    assert_ne!(stdout_of(&live), recorded, "live run matched by chance?");

    // Replay reproduces the recorded output exactly.
    let replay = Command::new(olang())
        .current_dir(&ws)
        .args(["replay", "run.olt"])
        .output()
        .unwrap();
    assert!(replay.status.success());
    assert_eq!(stdout_of(&replay), recorded);
    assert!(String::from_utf8_lossy(&replay.stderr).contains("clean"));
    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn a_trace_is_portable_without_the_source_file() {
    let ws = workspace("portable");
    write(&ws.join("flaky.ol"), FLAKY);
    let rec = Command::new(olang())
        .current_dir(&ws)
        .args(["--record", "run.olt", "flaky.ol"])
        .output()
        .unwrap();
    let recorded = stdout_of(&rec);

    // Move only the trace to a fresh directory — no program on disk.
    let elsewhere = workspace("portable_elsewhere");
    std::fs::copy(ws.join("run.olt"), elsewhere.join("run.olt")).unwrap();

    let replay = Command::new(olang())
        .current_dir(&elsewhere)
        .args(["replay", "run.olt"])
        .output()
        .unwrap();
    assert!(replay.status.success());
    assert_eq!(
        stdout_of(&replay),
        recorded,
        "embedded source did not replay"
    );
    let _ = std::fs::remove_dir_all(&ws);
    let _ = std::fs::remove_dir_all(&elsewhere);
}

#[test]
fn a_crashed_run_is_recorded_and_its_crash_replays() {
    let ws = workspace("crash");
    write(
        &ws.join("boom.ol"),
        "let r = random.randint(0, 1000)\nprintln(show(r))\nunwrap(str.parse_int(\"x\"))\n",
    );
    let rec = Command::new(olang())
        .current_dir(&ws)
        .args(["--record", "boom.olt", "boom.ol"])
        .output()
        .unwrap();
    assert!(!rec.status.success(), "the run should crash");
    assert!(
        ws.join("boom.olt").exists(),
        "a crashed run must still record"
    );
    let recorded_r = stdout_of(&rec);

    let replay = Command::new(olang())
        .current_dir(&ws)
        .args(["replay", "boom.olt"])
        .output()
        .unwrap();
    assert!(
        !replay.status.success(),
        "the replay should crash the same way"
    );
    assert_eq!(
        stdout_of(&replay),
        recorded_r,
        "the recorded random value must replay"
    );
    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn replay_detects_divergence() {
    let ws = workspace("diverge");
    write(
        &ws.join("p.ol"),
        "let a = time.now_ms()\nlet b = random.randint(1, 6)\nprintln(show(a) + \" \" + show(b))\n",
    );
    Command::new(olang())
        .current_dir(&ws)
        .args(["--record", "t.olt", "p.ol"])
        .output()
        .unwrap();

    // Tamper: rewrite the embedded source so the op sequence flips, but
    // leave the recorded events (time first, random second).
    let text = std::fs::read_to_string(ws.join("t.olt")).unwrap();
    let mut trace: serde_json::Value = serde_json::from_str(&text).unwrap();
    trace["source"] = serde_json::Value::String(
        "let b = random.randint(1, 6)\nlet a = time.now_ms()\nprintln(show(a))\n".into(),
    );
    std::fs::write(ws.join("t.olt"), serde_json::to_string(&trace).unwrap()).unwrap();

    let replay = Command::new(olang())
        .current_dir(&ws)
        .args(["replay", "t.olt"])
        .output()
        .unwrap();
    assert!(!replay.status.success());
    assert!(
        String::from_utf8_lossy(&replay.stderr).contains("diverged"),
        "expected a divergence report"
    );
    let _ = std::fs::remove_dir_all(&ws);
}

/// S4: replay detects a *changed argument*, not only a changed op. Record a
/// call to `os.get_env("HOME")`, then rewrite the trace's embedded source to
/// call `os.get_env("PATH")` — same op, different argument — and confirm
/// replay refuses instead of serving HOME's recorded value for PATH.
#[test]
fn replay_detects_a_changed_argument() {
    let ws = workspace("argdiv");
    write(
        &ws.join("p.ol"),
        "let v = os.get_env(\"HOME\")\nprintln(\"ok\")\n",
    );
    let rec = Command::new(olang())
        .current_dir(&ws)
        .args(["--record", "t.olt", "p.ol"])
        .output()
        .unwrap();
    assert!(rec.status.success(), "record failed");

    let text = std::fs::read_to_string(ws.join("t.olt")).unwrap();
    let mut trace: serde_json::Value = serde_json::from_str(&text).unwrap();
    trace["source"] =
        serde_json::Value::String("let v = os.get_env(\"PATH\")\nprintln(\"ok\")\n".into());
    std::fs::write(ws.join("t.olt"), serde_json::to_string(&trace).unwrap()).unwrap();

    let replay = Command::new(olang())
        .current_dir(&ws)
        .args(["replay", "t.olt"])
        .output()
        .unwrap();
    assert!(!replay.status.success(), "an argument change must diverge");
    assert!(
        String::from_utf8_lossy(&replay.stderr).contains("different arguments"),
        "expected an argument-divergence report, got: {}",
        String::from_utf8_lossy(&replay.stderr)
    );
    let _ = std::fs::remove_dir_all(&ws);
}

/// S6: machine-identity `os.*` is recorded, so a trace is portable — replay
/// serves the recorded `os.arch()` / `os.pid()`, not the replay machine's
/// live values (proved by replay output matching the recorded output for
/// `os.pid`, which is per-process).
#[test]
fn replay_reproduces_machine_identity() {
    let ws = workspace("machine");
    write(
        &ws.join("p.ol"),
        "println(unwrap(os.arch()) + \" \" + show(unwrap(os.pid())))\n",
    );
    let rec = Command::new(olang())
        .current_dir(&ws)
        .args(["--record", "t.olt", "p.ol"])
        .output()
        .unwrap();
    assert!(rec.status.success());
    let recorded = stdout_of(&rec);

    let rep = Command::new(olang())
        .current_dir(&ws)
        .args(["replay", "t.olt"])
        .output()
        .unwrap();
    assert!(rep.status.success(), "replay should be clean");
    assert_eq!(
        stdout_of(&rep),
        recorded,
        "replay must serve the recorded arch/pid, not live values"
    );
    let _ = std::fs::remove_dir_all(&ws);
}
