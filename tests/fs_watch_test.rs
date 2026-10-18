//! `fs.watch`: a folder's tree followed as it changes — files made,
//! removed and renamed, a folder with a file inside, a burst of a
//! thousand as one batch, ignored folders left out on the watching
//! thread — through FSEvents on macOS.

use olang::{Interpreter, Parser, Value};
use std::fs;

fn eval(src: &str) -> Value {
    let program = Parser::new().parse(src).expect("parse");
    let mut interpreter = Interpreter::new();
    interpreter.eval_program(program).expect("eval")
}

fn s(v: Value) -> String {
    match v {
        Value::String(ref s) => s.to_string(),
        other => panic!("expected string, got {:?}", other),
    }
}

/// The program that watches `root`, runs `act` (olang statements) and
/// says every event it hears (`kind path` lines, then `rescan` ones)
/// until `want` (a substring) shows up or three seconds pass.
fn heard(root: &str, opts: &str, act: &str, want: &str) -> String {
    let src = format!(
        r#"
let root = "{root}"
let w = unwrap(fs.watch(root, {opts}))
{act}
let mut out = ""
let t0 = time.monotonic()
let mut going = true
while going {{
    match fs.watch_next(w, #{{ "timeout_ms": 200, "settle_ms": 100 }}) {{
        Ok(b) => if b != () => {{
            for e in b.events {{ out = out + e.kind + (if e.dir => "/" else => "") + " " + str.substring(e.path, str.length(root) + 1, str.length(e.path)) + "\n" }}
            for r in b.rescan {{ out = out + "rescan " + r + "\n" }}
        }},
        Err(e) => {{ going = false }}
    }}
    if str.contains(out, "{want}") || time.monotonic() - t0 > 3000 => {{ going = false }}
}}
let _c = fs.watch_close(w)
out
"#
    );
    s(eval(&src))
}

#[cfg(target_os = "macos")]
fn root_of(d: &tempfile::TempDir) -> String {
    d.path().to_str().unwrap().to_string()
}

#[cfg(target_os = "macos")]
#[test]
fn a_file_made_is_said_created_with_its_path_as_the_root_was_spelled() {
    let d = tempfile::tempdir().unwrap();
    let root = root_of(&d);
    let out = heard(
        &root,
        "#{}",
        r#"let _w = fs.write_file(fs.join(root, "a.ol"), "1")"#,
        "a.ol",
    );
    assert!(out.contains("created a.ol"), "{out}");
}

#[cfg(target_os = "macos")]
#[test]
fn a_folder_made_with_a_file_in_it() {
    let d = tempfile::tempdir().unwrap();
    let root = root_of(&d);
    let out = heard(
        &root,
        "#{}",
        r#"let _d = fs.create_dir_all(fs.join(root, "src/deep"))
let _w = fs.write_file(fs.join(root, "src/deep/b.ol"), "2")"#,
        "src/deep/b.ol",
    );
    assert!(out.contains("created/ src"), "{out}");
    assert!(out.contains("created src/deep/b.ol"), "{out}");
}

#[cfg(target_os = "macos")]
#[test]
fn a_file_removed_and_one_renamed() {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("gone.ol"), "x").unwrap();
    fs::write(d.path().join("old.ol"), "y").unwrap();
    let root = root_of(&d);
    // what FSEvents says of files made before the stream started: let it pass
    std::thread::sleep(std::time::Duration::from_millis(300));
    let out = heard(
        &root,
        "#{}",
        r#"let _r = fs.remove_file(fs.join(root, "gone.ol"))
let _m = fs.move_file(fs.join(root, "old.ol"), fs.join(root, "new.ol"))"#,
        "new.ol",
    );
    assert!(out.contains("removed gone.ol"), "{out}");
    assert!(out.contains("removed old.ol"), "{out}");
    assert!(out.contains("created new.ol"), "{out}");
}

#[cfg(target_os = "macos")]
#[test]
fn ignored_folders_are_left_out_and_kept_paths_come_through() {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir_all(d.path().join("target/debug")).unwrap();
    fs::create_dir_all(d.path().join(".git/objects")).unwrap();
    let root = root_of(&d);
    std::thread::sleep(std::time::Duration::from_millis(300));
    let out = heard(
        &root,
        r#"#{ "ignore": ["target", ".git"], "keep": [".git/HEAD"] }"#,
        r#"for i in range(50) { let _w = fs.write_file(fs.join(root, "target/debug/o" + to_string(i)), "x") }
let _o = fs.write_file(fs.join(root, ".git/objects/ab"), "x")
let _h = fs.write_file(fs.join(root, ".git/HEAD"), "ref: refs/heads/main")
let _w = fs.write_file(fs.join(root, "last.ol"), "z")"#,
        "last.ol",
    );
    assert!(!out.contains("target/"), "{out}");
    assert!(!out.contains("objects"), "{out}");
    assert!(out.contains(".git/HEAD"), "{out}");
    assert!(out.contains("last.ol"), "{out}");
}

#[cfg(target_os = "macos")]
#[test]
fn a_burst_of_a_thousand_is_one_batch_and_past_max_a_rescan() {
    let d = tempfile::tempdir().unwrap();
    let root = root_of(&d);
    let src = format!(
        r#"
let root = "{root}"
let w = unwrap(fs.watch(root))
let _d = fs.create_dir(fs.join(root, "many"))
for i in range(1000) {{ let _w = fs.write_file(fs.join(root, "many/f" + to_string(i) + ".ol"), "x") }}
let mut paths = 0
let mut batches = 0
let mut going = true
let t0 = time.monotonic()
while going {{
    match fs.watch_next(w, #{{ "timeout_ms": 400, "settle_ms": 150 }}) {{
        Ok(b) => if b == () => {{ going = false }} else => {{ batches = batches + 1; paths = paths + len(b.events) }},
        Err(e) => {{ going = false }}
    }}
    if time.monotonic() - t0 > 5000 => {{ going = false }}
}}
let _c = fs.watch_close(w)
// past max: the root to read again, no events
let w2 = unwrap(fs.watch(root))
for i in range(50) {{ let _w = fs.write_file(fs.join(root, "many/g" + to_string(i) + ".ol"), "x") }}
let b2 = unwrap(fs.watch_next(w2, #{{ "timeout_ms": 2000, "settle_ms": 150, "max": 10 }}))
let _c2 = fs.watch_close(w2)
to_string(batches) + " " + to_string(paths) + " " + to_string(len(b2.events)) + " " + show(b2.rescan == [root])
"#
    );
    let out = s(eval(&src));
    let parts: Vec<&str> = out.split(' ').collect();
    let batches: usize = parts[0].parse().unwrap();
    let paths: usize = parts[1].parse().unwrap();
    assert!(batches <= 3, "{out}");
    assert!(paths >= 1000, "{out}");
    assert_eq!(parts[2], "0", "{out}");
    assert_eq!(parts[3], "true", "{out}");
}

#[test]
fn closed_watches_answer_err_and_misuse_raises() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().to_str().unwrap().to_string();
    let src = format!(
        r#"
match fs.watch("{root}") {{
    Ok(w) => {{
        let a = unwrap(fs.watch_close(w))
        let b = unwrap(fs.watch_close(w))
        let n = match fs.watch_next(w, #{{ "timeout_ms": 10 }}) {{ Ok(x) => "ok", Err(e) => "err" }}
        show(a) + " " + show(b) + " " + n
    }},
    Err(e) => "unsupported"
}}
"#
    );
    let out = s(eval(&src));
    if cfg!(target_os = "macos") {
        assert_eq!(out, "true false err");
    }
    let missing = s(eval(
        r#"match fs.watch("/no/such/folder/here") { Ok(w) => "ok", Err(e) => "err" }"#,
    ));
    assert_eq!(missing, "err");
    let bad = Parser::new()
        .parse(r#"fs.watch(".", #{ "nope": 1 })"#)
        .unwrap();
    assert!(Interpreter::new().eval_program(bad).is_err());
}

#[test]
fn a_read_of_the_file_system_as_the_gate_sees_it() {
    for name in ["fs.watch", "fs.watch_next", "fs.watch_close"] {
        assert_eq!(
            olang::effects::classify(name).cap,
            Some(olang::caps::CapUse::FsRead),
            "{name}"
        );
    }
}
