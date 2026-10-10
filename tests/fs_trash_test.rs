//! `fs.trash`: a file and a folder moved to the Trash (never deleted),
//! their place in the Trash answered, and put back from there; a missing
//! path refused. macOS only (the one Trash olang uses).

use olang::{Interpreter, Parser, Value};

fn eval(src: &str) -> Value {
    let program = Parser::new().parse(src).expect("parse");
    Interpreter::new().eval_program(program).expect("eval")
}

#[test]
fn a_file_and_a_folder_go_to_the_trash_and_come_back() {
    if !cfg!(target_os = "macos") {
        let v = eval(r#"match fs.trash("/nonexistent-olang-trash-test") { Ok(p) => "ok", Err(e) => e }"#);
        assert!(format!("{v}").contains("no such file"), "{v}");
        return;
    }
    let d = tempfile::tempdir().unwrap();
    let file = d.path().join("olang-trash-test-file.txt");
    let folder = d.path().join("olang-trash-test-folder");
    std::fs::write(&file, "kept").unwrap();
    std::fs::create_dir(&folder).unwrap();
    std::fs::write(folder.join("inner.ol"), "1").unwrap();
    let src = format!(
        r#"
        let a = fs.trash("{f}")
        let b = fs.trash("{g}")
        let gone = [fs.exists("{f}"), fs.exists("{g}")]
        let ta = match a {{ Ok(p) => p, Err(e) => "" }}
        let tb = match b {{ Ok(p) => p, Err(e) => "" }}
        let there = [fs.exists(ta), fs.exists(fs.join(tb, "inner.ol"))]
        // put back: moved from the Trash to where they were
        let back = [fs.move_file(ta, "{f}"), fs.move_file(tb, "{g}")]
        let missing = match fs.trash("{f}.nope") {{ Ok(p) => "ok", Err(e) => e }}
        [gone, there, str.contains(ta, ".Trash"), fs.read_file("{f}"), fs.exists(fs.join("{g}", "inner.ol")), missing]
        "#,
        f = file.display(),
        g = folder.display()
    );
    let v = format!("{}", eval(&src));
    assert!(v.starts_with(r#"[[false, false], [true, true], true, Ok("kept"), true, "#), "{v}");
    assert!(v.contains("no such file or folder"), "{v}");
}
