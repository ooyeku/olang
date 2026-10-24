//! A module's code sees its own file's names, what it imports, and the
//! stdlib modules and builtins — never the program's. The program's file
//! is one more file under the same rule. On every tier, in a nested call,
//! a closure, a task or `par_map`; and `olang check` says so statically.

use std::path::{Path, PathBuf};
use std::process::Command;

fn olang() -> &'static str {
    env!("CARGO_BIN_EXE_olang")
}

const TIERS: [&[&str]; 3] = [&["--no-ovm"], &["--ovm-tier=1"], &[]];

/// A module that reads names only the program defines (`secret`, a
/// function, and `limit`, a value), each on its own path; and one that
/// reaches only what it may: its own later and private names, a nested
/// forward reference, an import, the stdlib and the builtins.
const LEAKY: &str = r#"
share fn direct() = secret()
share fn value() = limit + 1
share fn nested() = deeper()
fn deeper() = deepest()
fn deepest() = secret()
share fn in_map() = map([1, 2], (x) => secret())
share fn in_par() = par_map([1, 2], (x) => secret())
share fn in_task() = unwrap(task.join(spawn { secret() }))
share fn piped() = 1 |> bump
share fn hot(n) = if n < 300 => n else => secret()
"#;

const OWN: &str = r#"
use lib.other { helper }
share fn first() = later() + "/" + hidden + "/" + to_string(forward())
share fn tools() = str.to_upper("ok") + " " + to_string(len([1, 2])) + " " + to_string(math.sqrt(16.0))
share fn imported() = helper()
share fn run(f) = f()
share fn run_par(f) = par_map([1, 2], (x) => f(x))
share fn run_task(f) = unwrap(task.join(spawn { f() }))
share fn sum_hot(n) = fold(range(0, n), 0, (acc, i) => acc + step(i))
fn step(i) = i % 3
fn later() = "later"
fn forward() = {
    fn a() = b()
    fn b() = 7
    a()
}
let hidden = "private"
"#;

const OTHER: &str = r#"share fn helper() = "imported""#;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("olang_lexical_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("lib")).unwrap();
    std::fs::write(
        dir.join("olang.toml"),
        "[package]\nname = \"lexical\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::write(dir.join("lib/leaky.ol"), LEAKY).unwrap();
    std::fs::write(dir.join("lib/own.ol"), OWN).unwrap();
    std::fs::write(dir.join("lib/other.ol"), OTHER).unwrap();
    dir
}

fn run_in(dir: &Path, args: &[&str]) -> (String, String, bool) {
    let out = Command::new(olang())
        .args(args)
        .current_dir(dir)
        .env("OLANG_HOME", dir.join(".home"))
        .output()
        .expect("run olang");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
        out.status.success(),
    )
}

/// Run `main` on every tier; each prints `expected` exactly.
fn agrees(tag: &str, main: &str, expected: &str) -> PathBuf {
    let dir = scratch(tag);
    std::fs::write(dir.join("main.ol"), main).unwrap();
    for flags in TIERS {
        let mut args: Vec<&str> = flags.to_vec();
        args.push("main.ol");
        let (out, err, _) = run_in(&dir, &args);
        assert_eq!(
            out.trim_end(),
            expected.trim_end(),
            "flags {flags:?}\nstderr: {err}"
        );
    }
    dir
}

/// The program binds `secret`, `limit` and `bump`; every path in the
/// module that reads one raises, inside a task so each is seen.
#[test]
fn a_module_never_reads_a_name_only_the_program_defines() {
    let main = r#"use lib.leaky { direct, value, nested, in_map, in_par, in_task, piped, hot }
fn secret() = "main's secret"
let limit = 10
fn bump(x) = x + 1
fn try(f) = match task.join(spawn { f() }) {
    Ok(v) => "ok " + show(v)
    Err(e) => e
}
println(try(() => direct()))
println(try(() => value()))
println(try(() => nested()))
println(try(() => in_map()))
println(try(() => in_par()))
println(try(() => in_task()))
println(try(() => piped()))
println(try(() => fold(range(0, 301), 0, (acc, i) => acc + hot(i))))
println(secret())
"#;
    let undefined = |name: &str| format!("Undefined variable: {name}");
    let expected = [
        undefined("secret"),
        undefined("limit"),
        undefined("secret"),
        undefined("secret"),
        undefined("secret"),
        format!("Runtime error: Unwrap failed on error: \"{}\"", undefined("secret")),
        undefined("bump"),
        undefined("secret"),
        "main's secret".to_string(),
    ]
    .join("\n");
    let dir = agrees("leak", main, &expected);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_raise_says_where_to_get_the_name() {
    let dir = scratch("hint");
    std::fs::write(
        dir.join("main.ol"),
        "use lib.leaky { direct }\nfn secret() = 1\nprintln(direct())\n",
    )
    .unwrap();
    for flags in TIERS {
        let mut args: Vec<&str> = flags.to_vec();
        args.push("main.ol");
        let (_, err, ok) = run_in(&dir, &args);
        assert!(!ok, "flags {flags:?}: expected a failure");
        assert!(
            err.contains("Undefined variable: secret") && err.contains("the program's names"),
            "flags {flags:?}: {err}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn check_flags_a_module_reading_the_programs_name() {
    let dir = scratch("check");
    std::fs::write(
        dir.join("main.ol"),
        "use lib.leaky { direct }\nfn secret() = 1\nprintln(direct())\n",
    )
    .unwrap();
    let (out, err, ok) = run_in(&dir, &["check", "lib/leaky.ol"]);
    let said = format!("{out}{err}");
    assert!(!ok, "check passed: {said}");
    for name in ["secret", "limit", "bump"] {
        assert!(
            said.contains(&format!("Undefined variable: {name}")),
            "no report of {name}: {said}"
        );
    }
    assert!(
        said.contains("doesn't see the program's names"),
        "the report says where to get it: {said}"
    );
    // A module that reads only what it may is clean: its later and
    // private names, an import, the stdlib and the builtins.
    std::fs::write(
        dir.join("lib/clean.ol"),
        "use lib.other { helper }\nshare fn go() = later() + hidden + helper() + str.to_upper(\"x\") + to_string(len([1]))\nfn later() = \"later\"\nlet hidden = \"private\"\n",
    )
    .unwrap();
    let (out, err, ok) = run_in(&dir, &["check", "lib/clean.ol"]);
    assert!(ok, "lib/clean.ol: {out}{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The program's own names from its functions, closures, tasks and
/// `par_map`, and from a callback it passes a module as a value.
#[test]
fn the_programs_own_names_and_its_callbacks_still_resolve() {
    let main = r#"use lib.own { run, run_par, run_task }
fn early() = later() + 1
fn later() = base
let base = 40
fn twice(x) = x * 2 + base
println(early())
println(show(map([1, 2], (x) => twice(x))))
println(show(par_map([1, 2], (x) => twice(x))))
println(unwrap(task.join(spawn { early() })))
println(run(() => early()))
println(run(early))
println(show(run_par(twice)))
println(show(run_par((x) => later() + x)))
println(run_task(() => twice(1)))
"#;
    let dir = agrees(
        "program",
        main,
        "41\n[42, 44]\n[42, 44]\n41\n41\n41\n[42, 44]\n[41, 42]\n42",
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A module's own later and private names, a nested forward reference,
/// its import, the stdlib and the builtins — and the program's like-named
/// bindings change none of them.
#[test]
fn a_modules_own_names_imports_and_the_stdlib_resolve() {
    let main = r#"use lib.own { first, tools, imported, sum_hot }
fn later() = "main's later"
let hidden = "main's hidden"
fn helper() = "main's helper"
fn len(xs) = 99
fn step(i) = 1000
println(first())
println(tools())
println(imported())
println(sum_hot(600))
"#;
    let dir = agrees(
        "own",
        main,
        "later/private/7\nOK 2 4.0\nimported\n600",
    );
    let _ = std::fs::remove_dir_all(&dir);
}
