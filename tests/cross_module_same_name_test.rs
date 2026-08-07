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
        olang::Value::String(s) => Ok(s.to_string()),
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
