//! An application carries the modules a `use` inside a block reaches,
//! in its entry and in its modules, and runs them from anywhere. Before,
//! `olang build --app` followed only top-level `use`s; once its files
//! were keyed by virtual paths (OL-444) a nested `use` found nothing and
//! OlangStudio stopped at launch.

use std::fs;
use std::path::Path;
use std::process::Command;

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

#[test]
fn a_built_app_runs_its_nested_uses_from_another_folder() {
    let dir = tempfile::tempdir().unwrap();
    let app = dir.path().join("app");
    write(
        &app.join("olang.toml"),
        "[package]\nname = \"nest\"\nversion = \"0.1.0\"\n",
    );
    // a module whose own block-level `use` runs as it loads
    write(
        &app.join("core/top.ol"),
        "let found = if true => {\n    use core.leaf { leaf }\n    leaf()\n} else => \"\"\nshare fn top() = \"top \" + found\n",
    );
    write(&app.join("core/leaf.ol"), "share fn leaf() = \"leaf\"\n");
    write(&app.join("core/late.ol"), "share fn late() = \"late\"\n");
    write(
        &app.join("main.ol"),
        "use core.top { top }\nprintln(top())\nlet x = if true => {\n    use core.late { late }\n    late()\n} else => \"\"\nprintln(x)\n",
    );
    let built = Command::new(env!("CARGO_BIN_EXE_olang"))
        .args(["build", "--app", "main.ol", "-o", "nest"])
        .current_dir(&app)
        .output()
        .unwrap();
    assert!(built.status.success(), "{}", String::from_utf8_lossy(&built.stderr));
    // run where none of the sources are
    let elsewhere = tempfile::tempdir().unwrap();
    let ran = Command::new(app.join("nest"))
        .current_dir(elsewhere.path())
        .output()
        .unwrap();
    assert!(ran.status.success(), "{}", String::from_utf8_lossy(&ran.stderr));
    assert_eq!(String::from_utf8_lossy(&ran.stdout), "top leaf\nlate\n");
}
