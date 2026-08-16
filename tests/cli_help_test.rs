//! The command-line surface, exercised through the real binary. These guard
//! the two properties a user relies on: every command is discoverable from
//! `--help`, and the file-first form (`olang <file>`) keeps working even
//! though the tool now has named subcommands.

use std::path::PathBuf;
use std::process::Command;

fn olang() -> Command {
    Command::new(env!("CARGO_BIN_EXE_olang"))
}

fn fixture(tag: &str, name: &str, src: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("olang_cli_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join(name);
    std::fs::write(&file, src).unwrap();
    file
}

#[test]
fn top_level_help_lists_every_command() {
    let out = olang().arg("--help").output().expect("run");
    assert!(out.status.success(), "--help should exit 0");
    let help = String::from_utf8_lossy(&out.stdout);
    // The whole point of the facelift: the commands are discoverable.
    for cmd in [
        "run", "repl", "check", "fmt", "test", "build", "inspect", "caps", "record", "replay",
        "doc", "bench", "lsp",
    ] {
        assert!(
            help.contains(cmd),
            "`olang --help` must list the `{cmd}` command; got:\n{help}"
        );
    }
}

#[test]
fn every_command_has_its_own_help() {
    // A representative slice — the ones whose flags a user needs to discover.
    let checks: &[(&str, &[&str])] = &[
        ("inspect", &["--against", "--caps", "--verify"]),
        ("build", &["--output"]),
        ("check", &["--rules"]),
        ("doc", &["--markdown"]),
    ];
    for (cmd, flags) in checks {
        let out = olang().arg(cmd).arg("--help").output().expect("run");
        assert!(out.status.success(), "`olang {cmd} --help` should exit 0");
        let help = String::from_utf8_lossy(&out.stdout);
        for flag in *flags {
            assert!(
                help.contains(flag),
                "`olang {cmd} --help` must document `{flag}`; got:\n{help}"
            );
        }
    }
}

#[test]
fn file_first_form_runs_a_program() {
    let file = fixture("run", "hello.ol", r#"println("hi from olang")"#);
    // Bare form: `olang <file>`.
    let out = olang().arg(&file).output().expect("run");
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("hi from olang"));
    // Explicit form: `olang run <file>` runs the same way.
    let out = olang().arg("run").arg(&file).output().expect("run");
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("hi from olang"));
}

#[test]
fn a_file_named_like_a_command_still_runs() {
    // `build.ol` contains a command word but is a path with an extension, so
    // it is run as a file, not dispatched as `olang build`.
    let file = fixture("named", "build.ol", r#"println("i am a script")"#);
    let out = olang().arg(&file).output().expect("run");
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("i am a script"));
}

#[test]
fn script_arguments_pass_through_after_the_file() {
    // Everything after the file — including hyphenated flags — is the
    // program's own argv, not olang's.
    let file = fixture("argv", "argv.ol", r#"for a in os.args() { println(a) }"#);
    let out = olang()
        .arg(&file)
        .arg("--port")
        .arg("8080")
        .output()
        .expect("run");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("--port"),
        "script must see --port; got:\n{stdout}"
    );
    assert!(
        stdout.contains("8080"),
        "script must see 8080; got:\n{stdout}"
    );
}

#[test]
fn record_writes_a_trace_that_replay_reproduces() {
    // `olang record` is the command form of the timeline: it runs the
    // program (printing a nondeterministic value) and writes a trace next to
    // it; `olang replay` must reproduce that exact value.
    let file = fixture(
        "record",
        "roll.ol",
        r#"println("roll:" + to_string(random.randint(1, 1000000000)))"#,
    );
    let dir = file.parent().unwrap();

    let live = olang().arg("record").arg(&file).output().expect("record");
    assert!(live.status.success(), "record should exit 0");
    let live_out = String::from_utf8_lossy(&live.stdout);
    let roll = live_out
        .lines()
        .find(|l| l.starts_with("roll:"))
        .expect("record should print the roll")
        .to_string();

    // The trace defaults to the program's stem + .olt.
    let trace = dir.join("roll.olt");
    assert!(
        trace.exists(),
        "record must write roll.olt next to the program"
    );

    // Replay reproduces the recorded value, every time.
    for _ in 0..2 {
        let out = olang().arg("replay").arg(&trace).output().expect("replay");
        assert!(out.status.success(), "replay should exit 0");
        let replayed = String::from_utf8_lossy(&out.stdout);
        assert!(
            replayed.contains(&roll),
            "replay must reproduce {roll:?}; got:\n{replayed}"
        );
    }
}
