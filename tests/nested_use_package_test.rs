//! A `use` inside a function resolves as a top-level one in its file
//! would, on every tier: against the package that defines the function
//! (OL-538), and finding the dependencies the module it loads uses
//! (OL-551). Before, the first resolved against the root project, and
//! the second failed on the bytecode tier ("Module 'dep.thing' not
//! found") while `--no-ovm` worked.

use std::fs;
use std::path::Path;
use std::process::Command;

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn run(dir: &Path, args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_olang"))
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "olang {:?}: {}",
        args,
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

const APP_TOML: &str = "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\ndep = { path = \"../dep\" }\n";

#[test]
fn a_module_a_function_loads_finds_its_own_dependencies() {
    let dir = tempfile::tempdir().unwrap();
    let dep = dir.path().join("dep");
    write(
        &dep.join("olang.toml"),
        "[package]\nname = \"dep\"\nversion = \"0.1.0\"\npublic = [\"thing\"]\n",
    );
    write(&dep.join("index.ol"), "share fn top() = \"dep\"\n");
    write(&dep.join("thing.ol"), "share fn thing() = \"thing\"\n");
    let app = dir.path().join("app");
    write(&app.join("olang.toml"), APP_TOML);
    write(
        &app.join("lib/inner.ol"),
        "use dep.thing { thing }\nshare fn uses_dep() = \"inner+\" + thing()\n",
    );
    // `later` runs through `main`, so on the bytecode tier
    write(
        &app.join("main.ol"),
        "fn later() = {\n    use lib.inner { uses_dep }\n    uses_dep()\n}\nfn main() = println(later())\nmain()\n",
    );
    assert_eq!(run(&app, &["main.ol"]), "inner+thing\n");
    assert_eq!(run(&app, &["--no-ovm", "main.ol"]), "inner+thing\n");
}

#[test]
fn a_use_in_a_dependency_function_resolves_in_that_dependency() {
    let dir = tempfile::tempdir().unwrap();
    let dep = dir.path().join("dep");
    write(
        &dep.join("olang.toml"),
        "[package]\nname = \"dep\"\nversion = \"0.1.0\"\npublic = [\"lib.top\"]\n",
    );
    write(&dep.join("lib/inner.ol"), "share fn inner() = \"inner\"\n");
    write(
        &dep.join("lib/top.ol"),
        "share fn top() = {\n    use lib.inner { inner }\n    \"top+\" + inner()\n}\n",
    );
    let app = dir.path().join("app");
    write(&app.join("olang.toml"), APP_TOML);
    // the app has a lib/inner.ol of its own, which must not be the one found
    write(
        &app.join("lib/inner.ol"),
        "share fn inner() = \"the app's\"\n",
    );
    write(
        &app.join("main.ol"),
        "use dep.lib.top { top }\nprintln(top())\n",
    );
    assert_eq!(run(&app, &["main.ol"]), "top+inner\n");
    assert_eq!(run(&app, &["--no-ovm", "main.ol"]), "top+inner\n");

    // and in a built application, run from elsewhere
    run(&app, &["build", "--app", "main.ol", "-o", "built"]);
    let elsewhere = tempfile::tempdir().unwrap();
    let ran = Command::new(app.join("built"))
        .current_dir(elsewhere.path())
        .output()
        .unwrap();
    assert!(
        ran.status.success(),
        "{}",
        String::from_utf8_lossy(&ran.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&ran.stdout), "top+inner\n");
}
