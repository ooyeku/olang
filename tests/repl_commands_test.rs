//! The REPL's `:` commands, one implementation for two faces.
//!
//! - **The terminal REPL is pinned.** Each `tests/fixtures/repl_terminal/
//!   <family>.in` is typed into `olang repl` over a pipe; what it prints
//!   (stdout, then stderr) must read exactly as `<family>.out`, recorded
//!   from the binary before the commands were shared with the protocol
//!   (`OLANG_REPL_GOLDEN_BIN=<olang> OLANG_UPDATE_REPL_GOLDEN=1` records).
//!   Times, the folder and the version are normalised.
//! - **Every command over the protocol.** `olang repl --serve` answers
//!   each command of the registry through the `command` op (and `eval`
//!   with a leading `:`), streams what `:sh` prints, is interrupted in a
//!   `:sh`, lists the commands, and completes command lines.

use serde_json::{Value as J, json};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn bin() -> PathBuf {
    std::env::var_os("OLANG_REPL_GOLDEN_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_olang")))
}

/// A folder with a file to run, a subfolder and a note; a home of its own.
fn sandbox() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.ol"), "println(\"ran a\")\n1 + 1\n").unwrap();
    std::fs::create_dir_all(dir.path().join("sub")).unwrap();
    std::fs::create_dir_all(dir.path().join("home")).unwrap();
    std::fs::write(dir.path().join("notes.txt"), "notes\n").unwrap();
    dir
}

// Every `<digits>.<digits>ms` (and ` ms`) as `<ms>`.
fn normalise_ms(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_digit() && (i == 0 || !b[i - 1].is_ascii_alphanumeric()) {
            let mut j = i;
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            if j < b.len() && b[j] == b'.' {
                let mut k = j + 1;
                while k < b.len() && b[k].is_ascii_digit() {
                    k += 1;
                }
                let rest = &s[k..];
                if k > j + 1 && (rest.starts_with("ms") || rest.starts_with(" ms")) {
                    out.push_str("<ms>");
                    i = k;
                    continue;
                }
            }
        }
        let ch = s[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn normalise(s: &str, dir: &Path) -> String {
    let real = std::fs::canonicalize(dir).unwrap_or(dir.to_path_buf());
    let s = s.replace(&real.to_string_lossy().to_string(), "<dir>");
    let s = s.replace(&dir.to_string_lossy().to_string(), "<dir>");
    normalise_ms(&s.replace(VERSION, "<version>"))
}

fn terminal(input: &str) -> String {
    let dir = sandbox();
    let mut child = Command::new(bin())
        .arg("repl")
        .current_dir(dir.path())
        .env("HOME", dir.path().join("home"))
        .env("OLANG_HOME", dir.path().join("home/.olang"))
        .env("NO_COLOR", "1")
        .env_remove("CLICOLOR_FORCE")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn olang repl");
    child.stdin.as_mut().unwrap().write_all(input.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    let text = format!(
        "{}\n--- stderr ---\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    normalise(&text, dir.path())
}

fn pinned(family: &str) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/repl_terminal");
    let input = std::fs::read_to_string(root.join(format!("{family}.in"))).unwrap();
    let got = terminal(&input);
    let golden = root.join(format!("{family}.out"));
    if std::env::var("OLANG_UPDATE_REPL_GOLDEN").is_ok() {
        std::fs::write(&golden, &got).unwrap();
        return;
    }
    let want = std::fs::read_to_string(&golden).expect("a recorded transcript");
    if got != want {
        let first = got.lines().zip(want.lines()).position(|(a, b)| a != b);
        panic!(
            "the terminal REPL's {family} commands print differently (first differing line {:?})\n--- got ---\n{got}\n--- want ---\n{want}",
            first.map(|i| i + 1)
        );
    }
}

#[test]
fn terminal_help_commands_print_as_before() {
    pinned("help");
}

#[test]
fn terminal_value_commands_print_as_before() {
    pinned("values");
}

#[test]
fn terminal_debugging_commands_print_as_before() {
    pinned("debugging");
}

#[test]
fn terminal_file_and_input_commands_print_as_before() {
    pinned("files");
}

// ── the protocol ─────────────────────────────────────────────────────

struct Serve {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next: u64,
    events: Vec<J>,
}

impl Serve {
    fn start(dir: &Path) -> Serve {
        let mut child = Command::new(env!("CARGO_BIN_EXE_olang"))
            .args(["repl", "--serve"])
            .current_dir(dir)
            .env("HOME", dir.join("home"))
            .env("OLANG_HOME", dir.join("home/.olang"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn olang repl --serve");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let mut s = Serve { child, stdin, stdout, next: 1, events: Vec::new() };
        let hello = s.recv();
        assert_eq!(hello["event"], json!("hello"));
        for op in ["command", "commands", "complete"] {
            assert!(hello["ops"].as_array().unwrap().contains(&json!(op)), "hello lists {op}");
        }
        s
    }

    fn recv(&mut self) -> J {
        let mut line = String::new();
        self.stdout.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap_or_else(|e| panic!("not JSON ({e}): {line:?}"))
    }

    fn write(&mut self, mut m: J) -> u64 {
        let id = self.next;
        self.next += 1;
        m["id"] = json!(id);
        writeln!(self.stdin, "{}", m).unwrap();
        self.stdin.flush().unwrap();
        id
    }

    /// The reply to `m`; the events before it kept.
    fn ask(&mut self, m: J) -> J {
        let id = self.write(m);
        loop {
            let r = self.recv();
            if r.get("event").is_some() {
                self.events.push(r);
                continue;
            }
            assert_eq!(r["id"], json!(id), "a reply for another request: {r}");
            return r;
        }
    }

    fn cmd(&mut self, line: &str) -> J {
        self.ask(json!({ "op": "command", "line": line }))
    }
}

impl Drop for Serve {
    fn drop(&mut self) {
        let _ = writeln!(self.stdin, "{}", json!({ "op": "shutdown", "id": 0 }));
        let _ = self.stdin.flush();
        let t = Instant::now();
        while t.elapsed() < Duration::from_secs(5) {
            if let Ok(Some(_)) = self.child.try_wait() {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn out(r: &J) -> String {
    r["out"].as_str().unwrap_or("").to_string()
}

#[test]
fn every_command_answers_over_the_protocol() {
    let dir = sandbox();
    let mut s = Serve::start(dir.path());
    // the registry, listed
    let list = s.ask(json!({ "op": "commands" }));
    let names: Vec<String> = list["commands"].as_array().unwrap().iter().map(|c| c["name"].as_str().unwrap().to_string()).collect();
    assert!(names.len() >= 39, "{names:?}");
    for want in [":help", ":type", ":time", ":sh", ":ml", ":end", ":cancel", ":quit", ":history", ":clear", ":pkg", ":use", ":let", ":fn", ":!"] {
        assert!(names.contains(&want.to_string()), "{want} listed");
    }
    let ml = list["commands"].as_array().unwrap().iter().find(|c| c["name"] == json!(":ml")).unwrap();
    assert_eq!(ml["client"], json!(true));

    // a value in the prompt's scope, which the commands see
    let r = s.ask(json!({ "op": "eval", "code": "let total = 41" }));
    assert_eq!(r["ok"], json!(true), "{r}");

    // help: the overview with the command reference, a function, styled lines
    let r = s.cmd(":help");
    assert_eq!(r["ok"], json!(true), "{r}");
    assert_eq!(r["command"], json!(":help"));
    assert!(!out(&r).is_empty() && !out(&r).contains('\x1b'), "plain text: {r}");
    assert!(r["commands"].as_array().unwrap().len() >= 39);
    assert!(r["lines"].as_array().unwrap().iter().any(|l| l.as_array().unwrap().iter().any(|run| run.get("c").is_some() || run.get("b").is_some())), "styled runs");
    assert!(out(&s.cmd(":help len")).contains("len"));
    for c in [":help_advanced", ":contextual_help", ":search map limit:2", ":tutorial", ":help syntax"] {
        let r = s.cmd(c);
        assert_eq!(r["ok"], json!(true), "{c}: {r}");
        assert!(!out(&r).trim().is_empty(), "{c} says something");
    }
    // a tutorial's steps, for an editor to take one by one
    let r = s.cmd(":tutorial_run basic_operations");
    assert!(r["tutorial"]["steps"].as_array().map(|a| !a.is_empty()).unwrap_or(false), "{r}");

    // values: the type, the value as structure, the time
    let r = s.cmd(":type total + 1");
    assert_eq!(r["type"], json!("Int"), "{r}");
    assert_eq!(r["value"]["s"], json!("42"));
    assert_eq!(out(&r).trim(), "total + 1 : Int");
    let r = s.cmd(":time [1, 2, 3]");
    assert!(r["time_ms"].as_f64().is_some(), "{r}");
    assert_eq!(r["value"]["k"], json!("list"));
    assert!(r["eval"].as_u64().is_some(), "a value's handles are kept");
    let r = s.cmd(":inspect total");
    assert_eq!(r["values"][0]["label"], json!("total"));
    assert!(out(&r).contains("Variable Inspection: total"));
    let r = s.cmd(":set total 7");
    assert!(out(&r).contains("updated"), "{r}");
    let r = s.cmd(":let doubled = total * 2");
    assert_eq!(r["value"]["s"], json!("14"), "{r}");
    let r = s.cmd(":fn twice(a) = a * 2");
    assert_eq!(r["ok"], json!(true), "{r}");
    let r = s.ask(json!({ "op": "eval", "code": "twice(doubled)" }));
    assert_eq!(r["value"]["s"], json!("28"), "the prompt sees what a command bound: {r}");
    let r = s.cmd(":use collections");
    assert_eq!(r["ok"], json!(true), "{r}");
    let r = s.cmd(":env");
    assert!(out(&r).contains("User Environment"), "{r}");
    let env = r["values"].as_array().unwrap().iter().find(|v| v["label"] == json!("bindings")).expect("bindings as a table");
    assert!(env["value"]["table"].is_object(), "{env}");

    // timing and the tiers: results, and a place the editor has
    let r = s.cmd(":benchmark 1 + 1");
    assert_eq!(r["bench"].as_array().unwrap().len(), 5, "{r}");
    for (c, place) in [(":stats", "tiers"), (":ovm", "tiers"), (":profile fold(range(0, 20000), 0, (a, x) => a + x)", "profiler")] {
        let r = s.cmd(c);
        assert_eq!(r["open"], json!(place), "{c}: {r}");
        assert!(!out(&r).is_empty());
    }
    for c in [":memory", ":parallel", ":parallel status", ":config", ":config show_types true", ":version", ":stack", ":debug", ":debug 1 + 2", ":trace twice", ":trace", ":watch total", ":watch"] {
        let r = s.cmd(c);
        assert_eq!(r["ok"], json!(true), "{c}: {r}");
        assert!(!out(&r).trim().is_empty(), "{c} says something");
    }
    // a watched variable changed at the prompt is said
    let r = s.ask(json!({ "op": "eval", "code": "let total = total + 1" }));
    assert!(r["watched"].as_array().map(|w| !w.is_empty()).unwrap_or(false), "{r}");

    // the history is the prompt's inputs
    let r = s.cmd(":history");
    assert!(out(&r).contains("twice(doubled)"), "{r}");
    let r = s.cmd(":history search twice");
    assert!(out(&r).contains("twice(doubled)"));

    // files and the shell, from the working folder
    assert!(out(&s.cmd(":pwd")).trim().ends_with(dir.path().file_name().unwrap().to_str().unwrap()));
    let r = s.cmd(":ls");
    assert!(out(&r).contains("a.ol") && out(&r).contains("notes.txt"), "{r}");
    s.events.clear();
    let r = s.cmd(":sh echo one; echo two");
    assert_eq!(out(&r), "one\ntwo\n", "{r}");
    assert!(s.events.iter().any(|e| e["event"] == json!("out") && e["text"].as_str().unwrap().contains("one")), "streamed: {:?}", s.events);
    let r = s.cmd("!echo bang");
    assert_eq!(out(&r), "bang\n");
    assert!(out(&s.cmd(":sh false")).contains("exit code 1"));
    let r = s.cmd(":run a.ol");
    assert!(out(&r).contains("ran a") && out(&r).contains("executed successfully"), "{r}");
    let r = s.cmd(":cd sub");
    assert!(r["cwd"].as_str().unwrap().ends_with("sub"), "{r}");
    s.cmd(":cd ..");
    assert!(out(&s.cmd(":pkg")).contains("No package here"));

    // what an editor does itself
    assert_eq!(s.cmd(":clear")["effects"], json!(["clear"]));
    assert_eq!(s.cmd(":ml")["effects"], json!(["multiline"]));
    assert!(out(&s.cmd(":end")).contains("No multi-line input"));
    assert!(out(&s.cmd(":cancel")).contains("No multi-line input"));
    assert_eq!(s.cmd(":clear env")["ok"], json!(true));
    assert!(s.ask(json!({ "op": "eval", "code": "total" }))["ok"] == json!(false), ":clear env forgot the bindings");
    s.ask(json!({ "op": "eval", "code": "let w = 3" }));
    assert!(out(&s.cmd(":!1")).contains("Re-executing"));
    let r = s.cmd(":quit");
    assert_eq!(r["effects"], json!(["quit"]), "{r}");
    // … and the session is still there: the editor ends it
    assert_eq!(s.ask(json!({ "op": "eval", "code": "1 + 1" }))["value"]["s"], json!("2"));

    // an unknown command: the nearest one
    let r = s.cmd(":halp");
    assert_eq!(r["unknown"], json!(true));
    assert_eq!(r["suggest"], json!(":help"));
    assert!(out(&r).contains("did you mean :help"), "{r}");
    let r = s.cmd(":ml_x_unknown");
    assert!(out(&r).contains(":help lists them"), "{r}");

    // eval with a leading colon is a command (what Studio's prompt typed)
    let r = s.ask(json!({ "op": "eval", "code": ":help" }));
    assert_eq!(r["command"], json!(":help"), "{r}");
}

#[test]
fn a_shell_command_is_interrupted() {
    let dir = sandbox();
    let mut s = Serve::start(dir.path());
    let id = s.write(json!({ "op": "command", "line": ":sh echo started; sleep 30; echo never" }));
    // the first line arrives while the command runs
    let t = Instant::now();
    loop {
        let e = s.recv();
        if e["event"] == json!("out") {
            assert_eq!(e["id"], json!(id));
            assert!(e["text"].as_str().unwrap().contains("started"));
            break;
        }
        assert!(t.elapsed() < Duration::from_secs(10));
    }
    let iid = s.write(json!({ "op": "interrupt" }));
    let mut reply = J::Null;
    while reply.is_null() {
        let r = s.recv();
        if r.get("event").is_some() {
            continue;
        }
        if r["id"] == json!(id) {
            reply = r;
        } else if r["id"] == json!(iid) {
            assert_eq!(r["running"], json!(true));
        }
    }
    assert!(t.elapsed() < Duration::from_secs(10), "stopped promptly");
    assert_eq!(reply["interrupted"], json!(true), "{reply}");
    assert!(out(&reply).contains("(interrupted)") && !out(&reply).contains("never"), "{reply}");
    assert_eq!(s.ask(json!({ "op": "eval", "code": "2 * 21" }))["value"]["s"], json!("42"));
}

#[test]
fn command_lines_complete() {
    let dir = sandbox();
    let mut s = Serve::start(dir.path());
    s.ask(json!({ "op": "eval", "code": "let total = 1" }));
    s.ask(json!({ "op": "eval", "code": "fn twice(a) = a * 2" }));
    let labels = |r: &J| r["items"].as_array().unwrap().iter().map(|i| i["label"].as_str().unwrap().to_string()).collect::<Vec<_>>();
    let r = s.ask(json!({ "op": "complete", "line": ":ti", "pos": 3 }));
    assert!(labels(&r).contains(&":time".to_string()), "{r}");
    assert_eq!(r["start"], json!(0));
    let r = s.ask(json!({ "op": "complete", "line": ":cd ", "pos": 4 }));
    assert_eq!(labels(&r), vec!["home/", "sub/"], "{r}");
    assert_eq!(r["command"]["usage"], json!(":cd [<folder>]"), "help as it is typed");
    let r = s.ask(json!({ "op": "complete", "line": ":run ", "pos": 5 }));
    assert!(labels(&r).contains(&"a.ol".to_string()) && !labels(&r).contains(&"notes.txt".to_string()), "{r}");
    let r = s.ask(json!({ "op": "complete", "line": ":inspect to", "pos": 11 }));
    assert_eq!(labels(&r), vec!["total"]);
    assert_eq!(r["start"], json!(9));
    let r = s.ask(json!({ "op": "complete", "line": ":trace tw", "pos": 9 }));
    assert_eq!(labels(&r), vec!["twice"]);
    let r = s.ask(json!({ "op": "complete", "line": "str.tr", "pos": 6 }));
    assert!(labels(&r).contains(&"str.trim".to_string()), "{r}");
    let r = s.ask(json!({ "op": "complete", "line": "tot", "pos": 3 }));
    assert_eq!(labels(&r), vec!["total"]);
    let r = s.ask(json!({ "op": "complete", "line": ":tutorial ", "pos": 10 }));
    assert!(!labels(&r).is_empty());
}
