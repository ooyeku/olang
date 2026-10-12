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

#[test]
fn json_stringify_pairs_keeps_the_order() {
    let src = "fn row(names, values) = json.stringify_pairs(zip(names, values))\nprintln(unwrap(row([\"z\", \"a\", \"m\"], [1, \"x\", #{ \"b\": 2, \"a\": [()] }])))\nprintln(json.stringify_pairs([]))\nprintln(attempt(() => json.stringify_pairs([(1, 2)])))\n";
    for (stdout, _) in run("pairs.ol", src) {
        assert_eq!(
            stdout,
            "{\"z\":1,\"a\":\"x\",\"m\":{\"a\":[null],\"b\":2}}\nOk(\"{}\")\nErr(\"json.stringify_pairs: the key of pair 0 is Int, not a String\")\n"
        );
    }
}

#[test]
fn a_dropped_db_connection_is_closed() {
    let dir = std::env::temp_dir().join(format!("olang-db-drop-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("dir");
    let file = dir.join("t.db");
    let src = format!(
        "fn fds() = len(unwrap(fs.list_dir(\"/dev/fd\")))\nlet before = fds()\nfor i in 0..2000 {{\n    let c = unwrap(db.open(\"{f}\"))\n    let n = unwrap(db.execute(c, \"create table if not exists t (a)\"))\n}}\nprintln(fds() - before < 20)\nlet c = unwrap(db.open(\"{f}\"))\nlet cur = unwrap(db.cursor(c, \"select 1\"))\nprintln(typeof(c.handle))\n",
        f = file.display()
    );
    for (stdout, _) in run("dbdrop.ol", &src) {
        assert_eq!(stdout, "true\nConnectionHandle\n");
    }
}

#[test]
fn a_string_snapshot_is_its_text_and_the_caller_may_refresh_it() {
    let dir = std::env::temp_dir().join(format!("olang-snap-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("__snapshots__")).expect("dir");
    let test = |body: &str, envs: &[(&str, &str)]| {
        std::fs::write(dir.join("snap_test.ol"), format!("test \"s\" {{\n{body}\n}}\n")).expect("write");
        let mut c = olang();
        c.arg("test").arg(dir.join("snap_test.ol")).env_remove("OLANG_UPDATE_SNAPSHOTS").env_remove("UPDATE_SNAPSHOTS");
        for (k, v) in envs {
            c.env(k, v);
        }
        c.output().expect("runs").status.success()
    };
    let snap = |name: &str| std::fs::read_to_string(dir.join("__snapshots__").join(name)).expect("snap");
    // Written bare on the first run; a list keeps its display form.
    assert!(test("testing.snapshot(\"text\", \"<p>hi</p>\")\ntesting.snapshot(\"list\", [\"a\"])", &[]));
    assert_eq!(snap("text.snap"), "<p>hi</p>");
    assert_eq!(snap("list.snap"), "[\"a\"]");
    // An old, quoted snapshot of a String still matches.
    std::fs::write(dir.join("__snapshots__/old.snap"), "\"kept\"").expect("write");
    assert!(test("testing.snapshot(\"old\", \"kept\")", &[]));
    // A change fails, unless the caller, or either variable, accepts it.
    assert!(!test("testing.snapshot(\"text\", \"<p>new</p>\")", &[]));
    assert!(test("testing.snapshot(\"text\", \"<p>new</p>\", #{ \"update\": true })", &[]));
    assert_eq!(snap("text.snap"), "<p>new</p>");
    assert!(test("testing.snapshot(\"text\", \"<p>3</p>\")", &[("UPDATE_SNAPSHOTS", "1")]));
    assert_eq!(snap("text.snap"), "<p>3</p>");
    assert!(test("testing.snapshot(\"old\", \"kept\")", &[("OLANG_UPDATE_SNAPSHOTS", "1")]));
    assert_eq!(snap("old.snap"), "kept");
}

#[test]
fn a_connection_writes_as_json_as_it_did() {
    let src = "let c = unwrap(db.open(\":memory:\"))\nprintln(unwrap(json.stringify(#{ \"db\": c })))\n";
    for (stdout, _) in run("dbjson.ol", src) {
        assert!(stdout.starts_with("{\"db\":{\"id\":") && stdout.contains("\"path\":\":memory:\"}}"), "{stdout}");
    }
}

#[test]
fn another_modules_function_of_a_builtins_name_does_not_shadow_it() {
    // lib defines `split` with five parameters; main's functions call the
    // builtin `split` — on every tier (a self-call rule once reached too far).
    script("shadowlib.ol", "share fn split(key, ratio, on_resize, a, b) = [key, ratio, on_resize, a, b]\n");
    let src = "use shadowlib { split as five }\nfn words(s) = split(s, \",\")\nfn many() = { let mut n = 0\n for i in 0..200 { n = n + len(words(\"a,b,c\")) }\n n }\nprintln(many())\nprintln(five(1, 2, 3, 4, 5))\n";
    for (stdout, _) in run("shadowmain.ol", src) {
        assert_eq!(stdout, "600\n[1, 2, 3, 4, 5]\n");
    }
}
