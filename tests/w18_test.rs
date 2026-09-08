//! Roadmap W18 — what open-track and Shuttle asked for after 0.84.0:
//! the route index keeps the first route at a path, a module's
//! functions and macros reach their private helpers at expansion, a
//! package module reaches a sibling's macro, and a served test needs
//! no OLANG_TEST juggling for `http.defer`.

use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("olang_w18_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(dir: &Path, rel: &str, content: &str) -> PathBuf {
    let path = dir.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, content).unwrap();
    path
}

fn olang(dir: &Path, args: &[&str]) -> (String, String, i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_olang"))
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run olang");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
        out.status.code().unwrap_or(-1),
    )
}

#[test]
fn a_modules_shared_functions_and_macros_reach_its_private_helpers_at_expansion() {
    let ws = workspace("private");
    write(
        &ws,
        "lib/rules.ol",
        "fn hidden(s) = \"<\" + s + \">\"\n\
         share fn shout(s) = hidden(str.to_upper(s))\n\
         meta fn wrap(e) = meta.lit(hidden(unwrap(meta.eval(e))))\n",
    );
    write(
        &ws,
        "ok.ol",
        "use lib.rules { shout, wrap }\n\
         meta fn m(e) = meta.lit(shout(unwrap(meta.eval(e))))\n\
         println(@m(\"hi\"))\n\
         println(@wrap(\"x\"))\n\
         println(shout(\"rt\"))\n",
    );
    let (out, err, rc) = olang(&ws, &["run", "ok.ol"]);
    assert_eq!(rc, 0, "{out}{err}");
    assert_eq!(out, "<HI>\n<x>\n<RT>\n");
    // The importer still cannot name the helper: private is private.
    write(
        &ws,
        "private.ol",
        "use lib.rules { shout }\nmeta fn m(e) = `\"${hidden(\"x\")}\"`\nprintln(@m(1))\n",
    );
    let (out, err, rc) = olang(&ws, &["run", "private.ol"]);
    assert_ne!(rc, 0, "{out}");
    assert!((out + &err).contains("hidden"));
}

#[test]
fn a_package_module_uses_a_siblings_macro_from_anywhere() {
    let ws = workspace("sibling");
    write(
        &ws,
        "olang.toml",
        "[package]\nname = \"shuttle\"\nversion = \"0.1.0\"\n",
    );
    write(
        &ws,
        "lib/decl.ol",
        "fn quote(s) = \"\\\"\" + s + \"\\\"\"\n\
         meta fn resource(name) = \"#{ \\\"resource\\\": \" + quote(unwrap(meta.eval(name))) + \" }\"\n",
    );
    write(
        &ws,
        "lib/sample.ol",
        "use lib.decl { resource }\n\
         share let SAMPLE = @resource(\"issues\")\n\
         share fn sample_name() = map_get(SAMPLE, \"resource\")\n\
         test \"a sibling's macro expands inside the package\" {\n\
             assert_eq(sample_name(), \"issues\")\n\
         }\n",
    );
    write(
        &ws,
        "main.ol",
        "use lib.sample { sample_name }\nprintln(sample_name())\n",
    );
    // From the package root, as a consumer of the module.
    let (out, err, rc) = olang(&ws, &["run", "main.ol"]);
    assert_eq!(rc, 0, "{out}{err}");
    assert_eq!(out, "issues\n");
    // The module's own tests, and the module run from its own directory.
    let (out, err, rc) = olang(&ws, &["test", "lib/sample.ol"]);
    assert_eq!(rc, 0, "{out}{err}");
    assert!(out.contains("1 passed"), "{out}");
    let (out, err, rc) = olang(&ws.join("lib"), &["run", "sample.ol"]);
    assert_eq!(rc, 0, "{out}{err}");
}
