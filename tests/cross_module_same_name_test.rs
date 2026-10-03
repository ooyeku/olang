//! Two modules may define functions with the same name and each namespace
//! resolves to its own. This guards a bytecode-tier bug where functions were
//! keyed by bare name, so the first-compiled `foo` ran for every module's
//! `foo` (and a caller's private helper leaked across modules). The default
//! tier threshold is 1, so the collision triggered on the first call.

use olang::{Interpreter, Parser};
use std::fs;
use std::path::{Path, PathBuf};

fn workspace(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("olang_samename_{}_{}", std::process::id(), tag));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn scaffold(ws: &Path) -> PathBuf {
    fs::write(
        ws.join("olang.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    fs::create_dir_all(ws.join("lib")).unwrap();
    // Two modules, same public names, different bodies. `viahelp` calls a
    // private `helper` — the helper must resolve within its own module.
    fs::write(
        ws.join("lib/a.ol"),
        "share fn who() = \"alpha\"\nshare fn viahelp() = helper()\nfn helper() = \"a-helper\"\n",
    )
    .unwrap();
    fs::write(
        ws.join("lib/b.ol"),
        "share fn who() = \"beta\"\nshare fn viahelp() = helper()\nfn helper() = \"b-helper\"\n",
    )
    .unwrap();
    ws.to_path_buf()
}

fn run(app: &Path, source: &str) -> Result<String, String> {
    let program = Parser::new().parse(source).map_err(|e| e.to_string())?;
    let mut interp = Interpreter::new();
    // Match the binary's default: OVM on, tier threshold 1 (compile eagerly),
    // which is what exposed the collision.
    interp.enable_bytecode_tier(1, false);
    interp.set_current_file(&app.join("main.ol"));
    match interp.eval_program(program).map_err(|e| e.to_string())? {
        olang::Value::String(ref s) => Ok(s.to_string()),
        other => Err(format!("expected string, got {:?}", other)),
    }
}

#[test]
fn same_named_functions_resolve_per_module() {
    let ws = workspace("who");
    let app = scaffold(&ws);
    let out = run(&app, "use lib.a\nuse lib.b\na.who() + \"/\" + b.who()");
    assert_eq!(out.as_deref(), Ok("alpha/beta"));
    let _ = fs::remove_dir_all(&ws);
}

#[test]
fn private_helpers_do_not_leak_across_modules() {
    let ws = workspace("helper");
    let app = scaffold(&ws);
    let out = run(
        &app,
        "use lib.a\nuse lib.b\na.viahelp() + \"/\" + b.viahelp()",
    );
    assert_eq!(out.as_deref(), Ok("a-helper/b-helper"));
    let _ = fs::remove_dir_all(&ws);
}

#[test]
fn repeated_calls_stay_correct_after_the_tier_warms_up() {
    // With threshold 1 the tier engages immediately; hammer both calls to be
    // sure a late promotion can't swap one module's function for the other's.
    let ws = workspace("hot");
    let app = scaffold(&ws);
    let out = run(
        &app,
        r#"
use lib.a
use lib.b
let mut ok = true
let mut i = 0
while i < 20 {
    ok = ok && (a.who() == "alpha") && (b.who() == "beta")
    i = i + 1
}
if ok => "all-correct" else => "corrupted"
"#,
    );
    assert_eq!(out.as_deref(), Ok("all-correct"));
    let _ = fs::remove_dir_all(&ws);
}

/// A dependency reached through its index (`use dep`) and by a dotted path
/// (`use dep.lib.layout`) is one set of modules, loaded once: the path
/// dependency's files were found as `app/../dep/lib/layout.ol` one way and
/// `dep/lib/layout.ol` the other, and each spelling loaded its own copy.
/// The copies' same-named functions made the bytecode tier resolve
/// `place`'s helper declared below it through a closure that lacked it.
#[test]
fn a_dependency_reached_two_ways_is_loaded_once() {
    let ws = workspace("twoways");
    let dep = ws.join("dep");
    let app = ws.join("app");
    fs::create_dir_all(dep.join("lib")).unwrap();
    fs::create_dir_all(app.join("lib")).unwrap();
    fs::write(
        dep.join("olang.toml"),
        "[package]\nname = \"dep\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    fs::write(dep.join("lib/layout.ol"), "println(\"loading the layout\")\nshare fn place(n) = if n <= 0 => \"placed\" else => ly_box(n)\nfn ly_box(n) = place(n - 1)\n").unwrap();
    fs::write(
        dep.join("lib/engine.ol"),
        "use lib.layout { place }\nshare fn frame(n) = place(n)\n",
    )
    .unwrap();
    fs::write(dep.join("index.ol"), "share use lib.engine { frame }\n").unwrap();
    fs::write(
        app.join("olang.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies.dep]\npath = \"../dep\"\n",
    )
    .unwrap();
    // the app has a lib/layout.ol of its own, with the same names
    fs::write(app.join("lib/layout.ol"), "share fn place(n) = if n <= 0 => \"mine\" else => ly_box(n)\nfn ly_box(n) = place(n - 1)\n").unwrap();
    fs::write(
        app.join("main.ol"),
        "use dep { frame }\nuse dep.lib.layout { place as dep_place }\nuse lib.layout { place }\nlet mut out = []\nfor i in 0..50 { out = [frame(i % 4), dep_place(3), place(3)] }\nprintln(out)\n",
    )
    .unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_olang"))
        .arg("main.ol")
        .current_dir(&app)
        .output()
        .expect("run olang");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{stdout}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(stdout.matches("loading the layout").count(), 1, "{stdout}");
    assert!(
        stdout.contains(r#"["placed", "placed", "mine"]"#),
        "{stdout}"
    );
    let _ = fs::remove_dir_all(&ws);
}
