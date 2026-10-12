//! Standard-library additions of 0.88: each a gap a program in the
//! workspace ran into (docs/roadmap.md's rows, closed one by one).

use std::process::Command;

fn olang() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_olang"));
    // Never the user's own state.
    c.env("OLANG_HOME", std::env::temp_dir().join("olang-stdlib-additions-home"));
    c
}

fn script(name: &str, source: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("olang-stdlib-additions-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join(name);
    std::fs::write(&path, source).expect("write");
    path
}

/// stdout and stderr of a run, on the interpreter and on the tiers.
fn run(name: &str, source: &str) -> Vec<(String, String)> {
    let path = script(name, source);
    let mut out = Vec::new();
    for flags in [&["--no-ovm"][..], &["--ovm-tier=1"][..]] {
        let o = olang().args(flags).arg(&path).output().expect("runs");
        assert!(o.status.success(), "{flags:?}: {}", String::from_utf8_lossy(&o.stderr));
        out.push((
            String::from_utf8_lossy(&o.stdout).into_owned(),
            String::from_utf8_lossy(&o.stderr).into_owned(),
        ));
    }
    out
}

#[test]
fn eprint_and_eprintln_write_to_stderr() {
    let src = "fn report(n) = {\n    eprintln(\"skipped\", n, \"rows\")\n    eprint(\"no newline\")\n    eprintln()\n    println(\"data\")\n}\nreport(3)\nreport(4)\n";
    for (stdout, stderr) in run("eprint.ol", src) {
        assert_eq!(stdout, "data\ndata\n");
        assert_eq!(stderr, "skipped 3 rows\nno newline\nskipped 4 rows\nno newline\n");
    }
}

#[test]
fn str_is_ascii() {
    let src = "fn plain(s) = str.is_ascii(s)\nprintln([plain(\"\"), plain(\"abc ~\\n\"), plain(\"café\"), plain(\"日本\")])\n";
    for (stdout, _) in run("ascii.ol", src) {
        assert_eq!(stdout, "[true, true, false, false]\n");
    }
}
