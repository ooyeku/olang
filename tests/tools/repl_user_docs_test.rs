//! `:help` answers for the user's own documented code: a `///` block
//! above any declaration — entered in the session, in a `:run` file, or
//! in a module a program `use`d — is what `:help <name>` shows. The
//! REPL clears the interpreter's module cache after every input, so
//! these tests also pin that the doc scan survives that.

use std::io::Write;
use std::process::{Command, Stdio};

fn olang() -> &'static str {
    env!("CARGO_BIN_EXE_olang")
}

/// Drive the REPL with stdin lines from `dir`, return combined output.
fn repl(dir: &std::path::Path, input: &str) -> String {
    let mut child = Command::new(olang())
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("OLANG_WARM", "0")
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[test]
fn help_covers_session_files_and_modules() {
    let base = std::env::temp_dir().join(format!("olang_userdocs_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("lib")).unwrap();

    std::fs::write(
        base.join("lib/money.ol"),
        "//! money — cent-precise currency helpers.\n\n\
         /// Format cents as \"$1,234.56\", negative-safe.\n\
         share fn money(cents) = to_string(cents)\n\n\
         /// The month key (\"2026-08\") for a date string.\n\
         share fn month_of(date) = str.substring(date, 0, 7)\n",
    )
    .unwrap();
    std::fs::write(
        base.join("tool.ol"),
        "//! tool — a demo script.\n\n\
         /// Double a number.\n\
         fn double(x) = x * 2\n\n\
         println(double(21))\n",
    )
    .unwrap();

    // Session decl: the /// block arrives on its own line, the fn on
    // the next input — the pending buffer must join them.
    let out = repl(
        &base,
        "/// Steps in the collatz orbit of n.\n\
         fn collatz(n, s) = if n <= 1 => s else => collatz(n - 1, s + 1)\n\
         :help collatz\n\
         use lib.money { money }\n\
         :help money\n\
         :help money.month_of\n\
         :run tool.ol\n\
         :help double\n\
         :help tool\n\
         quit\n",
    );

    assert!(
        out.contains("Steps in the collatz orbit of n."),
        "session-entered /// doc must answer :help:\n{out}"
    );
    assert!(
        out.contains("defined this session"),
        "session origin named:\n{out}"
    );
    assert!(
        out.contains("negative-safe"),
        "use-loaded module fn doc must answer :help money:\n{out}"
    );
    assert!(
        out.contains("The month key"),
        "qualified module.name lookup:\n{out}"
    );
    assert!(
        out.contains("Double a number."),
        ":run file docs must answer :help:\n{out}"
    );
    assert!(
        out.contains("tool — a demo script."),
        "module view shows the //! note:\n{out}"
    );

    // Builtins keep priority: a documented user fn does not hide the
    // registry, and `:help map` is still the builtin.
    let out = repl(&base, ":help map\nquit\n");
    assert!(
        out.contains("Applies a function to each element"),
        "builtin help unchanged:\n{out}"
    );

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn a_packages_functions_answer_by_module_name_and_bare() {
    // The geometry shape: a package whose entry file is index.ol, so a
    // file-stem match can never answer `:help geometry.point`. The
    // registered module name maps to the file; lib/ siblings are
    // scanned for re-exported declarations; and an *undocumented*
    // function still answers with its signature instead of losing to a
    // fuzzy builtin match (`area` used to come back as plot.area).
    let base = std::env::temp_dir().join(format!("olang_pkg_help_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let pkg = base.join("geometry");
    std::fs::create_dir_all(pkg.join("lib")).unwrap();
    std::fs::write(
        pkg.join("olang.toml"),
        "[package]\nname = \"geometry\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::write(
        pkg.join("index.ol"),
        "//! geometry — points and shapes.\n\
         use lib.shapes { circle }\n\
         share use lib.shapes { circle }\n\
         /// A point as a map with x and y.\n\
         share fn point(x, y) = #{ \"x\": x, \"y\": y }\n\
         share fn area(shape) = map_get(shape, \"w\") * map_get(shape, \"h\")\n",
    )
    .unwrap();
    std::fs::write(
        pkg.join("lib/shapes.ol"),
        "/// A circle of radius r.\n\
         share fn circle(r) = #{ \"r\": r }\n",
    )
    .unwrap();

    let out = repl(
        &pkg,
        "use geometry\n:help geometry.point\n:help geometry.area\n:help area\n:help geometry.circle\nquit\n",
    );
    assert!(
        out.contains("A point as a map with x and y."),
        "documented fn by module name:\n{out}"
    );
    assert!(
        out.contains("area(shape)"),
        "undocumented fn answers with its signature:\n{out}"
    );
    assert!(
        !out.contains("plot.area"),
        "bare `area` must not lose to a fuzzy builtin:\n{out}"
    );
    assert!(
        out.contains("A circle of radius r."),
        "re-exported declaration found in lib/:\n{out}"
    );
    assert!(
        out.contains("undocumented"),
        "the hint teaches the /// convention:\n{out}"
    );

    let _ = std::fs::remove_dir_all(&base);
}
