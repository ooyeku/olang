//! A bare name resolves lexically, on every tier: the file's own
//! top-level names, what it imported, the stdlib modules and builtins.
//! Never another module's function — not a private helper, not a shared
//! function the file did not import.
//!
//! The compiled tier keeps by-name tables of every function the program
//! declared (the registry, the known function values, and the table a
//! bridge interpreter consults ahead of its root). A name the calling
//! file does not bind fell through to them: a module's `share fn time`
//! answered `time` in a caller that never imported it (in place of the
//! stdlib module), and a module's private `priv` answered `priv(1)` —
//! but only once a call was nested deeply enough for compiled code or
//! its bridge to run it, so the tree-walker raised where the VM and the
//! JIT returned the module's value.

use std::path::{Path, PathBuf};
use std::process::Command;

fn olang() -> &'static str {
    env!("CARGO_BIN_EXE_olang")
}

const TIERS: [&[&str]; 3] = [&["--no-ovm"], &["--ovm-tier=1"], &[]];

/// The module whose names must not leak: a shared `time` (the stdlib
/// module's name), a shared `secret` the importers never list, a
/// private `priv`, a private `len` (a builtin's name), and a shared
/// function that reaches its own private helpers through a nested call.
const LIB: &str = r#"
share fn head(a) = a
share fn time(a) = "lib's time"
share fn secret(a) = "lib's secret"
fn priv(a) = "lib's priv"
fn len(xs) = "lib's len"
fn helper() = priv(1) + "/" + len([1, 2])
share fn own() = helper()
"#;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("olang_name_scope_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("lib")).unwrap();
    std::fs::write(
        dir.join("olang.toml"),
        "[package]\nname = \"scope\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::write(dir.join("lib/h.ol"), LIB).unwrap();
    dir
}

fn run_in(dir: &Path, args: &[&str]) -> (String, String) {
    let out = Command::new(olang())
        .args(args)
        .current_dir(dir)
        .env("OLANG_HOME", dir.join(".home"))
        .output()
        .expect("run olang");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

/// Run `main` on every tier: each prints `expected`, and, when `error`
/// is given, raises an error naming that undefined name.
fn agrees(tag: &str, main: &str, expected: &str, error: Option<&str>) {
    let dir = scratch(tag);
    std::fs::write(dir.join("main.ol"), main).unwrap();
    for flags in TIERS {
        let mut args: Vec<&str> = flags.to_vec();
        args.push("main.ol");
        let (out, err) = run_in(&dir, &args);
        assert_eq!(
            out.trim_end(),
            expected.trim_end(),
            "flags {flags:?}\nstderr: {err}"
        );
        if let Some(name) = error {
            assert!(
                err.contains(&format!("Undefined variable: {name}")),
                "flags {flags:?}: expected an undefined `{name}`\nstderr: {err}"
            );
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_modules_function_does_not_replace_a_stdlib_module_in_nested_calls() {
    agrees(
        "stdlib",
        r#"use lib.h { head }
fn inner() = typeof(time)
fn outer() = inner()
fn deep() = outer()
fn clock() = time.monotonic_ms() >= 0
fn ticks() = clock()
println(inner())
println(outer())
println(deep())
println(ticks())
"#,
        "Module\nModule\nModule\ntrue",
        None,
    );
}

#[test]
fn a_private_function_is_unreachable_from_an_importer() {
    agrees(
        "private",
        r#"use lib.h { head }
fn inner(n) = priv(n)
fn outer(n) = inner(n)
println("start")
println(outer(1))
"#,
        "start",
        Some("priv"),
    );
}

#[test]
fn a_shared_function_not_imported_is_unreachable() {
    agrees(
        "shared",
        r#"use lib.h { head }
fn inner() = secret(1)
fn outer() = inner()
println("start")
println(outer())
"#,
        "start",
        Some("secret"),
    );
}

#[test]
fn a_private_function_read_as_a_value_is_unreachable() {
    agrees(
        "value",
        r#"use lib.h { head }
fn inner() = typeof(priv)
fn outer() = inner()
println("start")
println(outer())
"#,
        "start",
        Some("priv"),
    );
}

#[test]
fn closures_and_tasks_resolve_in_their_own_file() {
    agrees(
        "closures",
        r#"use lib.h { head }
fn kinds() = map([1, 2], (x) => typeof(time))
fn outer() = kinds()
fn par() = par_map([1, 2, 3], (x) => typeof(time))[2]
fn in_task() = unwrap(task.join(spawn { outer() }))
fn nested_task() = in_task()
fn tinner() = typeof(time)
fn touter() = tinner()
fn each() = map([1], (x) => touter())
println(show(outer()))
println(par())
println(show(nested_task()))
println(unwrap(task.join(spawn { touter() })))
println(show(each()))
println(show(par_map([1, 2], (x) => touter())))
"#,
        "[\"Module\", \"Module\"]\nModule\n[\"Module\", \"Module\"]\nModule\n[\"Module\"]\n[\"Module\", \"Module\"]",
        None,
    );
}

#[test]
fn a_closure_cannot_reach_a_private_function() {
    agrees(
        "closure_private",
        r#"use lib.h { head }
fn run() = map([1, 2], (x) => priv(x))
fn outer() = run()
println("start")
println(show(outer()))
"#,
        "start",
        Some("priv"),
    );
}

#[test]
fn a_task_cannot_reach_a_private_function() {
    agrees(
        "task_private",
        r#"use lib.h { head }
fn inner() = priv(1)
fn outer() = inner()
println("start")
println(show(task.join(spawn { outer() })))
"#,
        "start\nErr(\"Undefined variable: priv\")",
        None,
    );
}

#[test]
fn what_the_file_binds_and_imports_still_resolves() {
    // The file's own `secret` (declared after its caller), an imported
    // name, a builtin another module shadows, and the module's own
    // private helpers reached through its shared function.
    agrees(
        "own",
        r#"use lib.h { head, own }
fn outer() = inner()
fn inner() = secret(1) + " " + head("imported") + " " + to_string(len([1, 2, 3]))
fn secret(a) = "main's secret"
fn via() = own()
println(outer())
println(via())
"#,
        "main's secret imported 3\nlib's priv/lib's len",
        None,
    );
}

#[test]
fn a_modules_shadowed_builtin_stays_the_builtin_elsewhere() {
    // `len` is shadowed by lib/h's private function; a second module
    // and the program both still call the builtin, nested and hot.
    let dir = scratch("builtin");
    std::fs::write(
        dir.join("lib/k.ol"),
        "fn count(xs) = len(xs)\nshare fn total(n) = fold(range(0, n), 0, (acc, i) => acc + count([i, i]))\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("main.ol"),
        "use lib.h { own }\nuse lib.k { total }\nfn sizes(n) = fold(range(0, n), 0, (acc, i) => acc + len([i]))\nfn outer() = sizes(500)\nprintln(own())\nprintln(total(500))\nprintln(outer())\n",
    )
    .unwrap();
    for flags in TIERS {
        let mut args: Vec<&str> = flags.to_vec();
        args.push("main.ol");
        let (out, err) = run_in(&dir, &args);
        assert_eq!(
            out.trim_end(),
            "lib's priv/lib's len\n1000\n500",
            "flags {flags:?}\nstderr: {err}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_builtin_shadowed_after_its_caller_compiled_stays_the_builtin() {
    // `g` compiles calling the builtin `len`; then a module whose private
    // `len` shadows it loads, and its function compiles. The compiled
    // call keeps calling the builtin, as the tree-walker does.
    agrees(
        "late_shadow",
        r#"fn f(x) = len(x)
fn g(x) = f(x)
println(g([1]))
use lib.h { own }
println(own())
println(g([1, 2]))
"#,
        "1\nlib's priv/lib's len\n2",
        None,
    );
}
