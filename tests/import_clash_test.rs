//! What an import may bind (2026-10-20): a bare `use` of a stdlib module
//! binds its namespace alone; `use m {}` binds only the namespace; a
//! wildcard never replaces a stdlib module's name nor a name the program
//! already binds; a listed name the program already binds is an error.
//! `olang check` says so, and knows each stdlib module's members.
//! Runtime cases are differentials (tiered vs `--no-ovm`).

use std::path::PathBuf;
use std::process::Command;

fn project(files: &[(&str, &str)]) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for (p, s) in files {
        p.hash(&mut h);
        s.hash(&mut h);
    }
    let dir = std::env::temp_dir().join(format!("olang_import_clash_{:x}", h.finish()));
    let _ = std::fs::remove_dir_all(&dir);
    for (path, source) in files {
        let full = dir.join(path);
        std::fs::create_dir_all(full.parent().unwrap()).expect("mkdir");
        std::fs::write(&full, source).expect("write");
    }
    dir
}

fn run_in(dir: &std::path::Path, args: &[&str]) -> (String, i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_olang"))
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run");
    (
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
        out.status.code().unwrap_or(-1),
    )
}

fn tiers(dir: &std::path::Path, file: &str) -> (String, i32) {
    let (tiered, rc_t) = run_in(dir, &["run", file]);
    let (oracle, rc_o) = run_in(dir, &["--no-ovm", "run", file]);
    assert_eq!(tiered, oracle, "tiers diverged on {file}");
    assert_eq!(rc_t, rc_o, "exit codes diverged on {file}");
    (tiered, rc_t)
}

const LIB: &str = "share fn folds(x) = x * 2\n\
                   share fn plot() = \"lib.plot\"\n\
                   share fn helper() = \"lib.helper\"\n";

#[test]
fn a_bare_use_of_a_stdlib_module_binds_its_namespace_alone() {
    let dir = project(&[
        ("nopoint.ol", "use gg {}\nprintln(typeof(point))\n"),
        (
        "main.ol",
        "let folds = 3\n\
         use gg\n\
         use ml\n\
         println(typeof(plot))\n\
         println(typeof(folds))\n\
         println(typeof(gg.plot))\n\
         println(typeof(ml.folds))\n\
         use gg {}\n\
         println(map_get(#{}, \"point\") == ())\n\
         use gg { labs }\n\
         println(typeof(labs))\n",
        ),
    ]);
    let (out, rc) = tiers(&dir, "main.ol");
    assert_eq!(rc, 0, "{out}");
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[..4], ["Module", "Int", "Function", "Function"], "{out}");
    assert_eq!(lines[5], "Function", "{out}");
    let (out, rc) = run_in(&dir, &["run", "nopoint.ol"]);
    assert_ne!(rc, 0, "use gg {{}} must not import point:\n{out}");
}

#[test]
fn a_wildcard_keeps_stdlib_modules_and_the_program_s_own_names() {
    let dir = project(&[
        ("lib/cv.ol", LIB),
        (
            "main.ol",
            "let folds = 3\n\
             use lib.cv\n\
             use lib.cv { * }\n\
             println(typeof(folds))\n\
             println(typeof(plot))\n\
             println(helper())\n\
             println(cv.folds(2))\n\
             println(cv.plot())\n",
        ),
    ]);
    let (out, rc) = tiers(&dir, "main.ol");
    assert_eq!(rc, 0, "{out}");
    assert_eq!(
        out.lines().collect::<Vec<_>>(),
        ["Int", "Module", "lib.helper", "4", "lib.plot"],
        "{out}"
    );
    let (out, rc) = run_in(&dir, &["check", "main.ol"]);
    assert_eq!(rc, 0, "advisory only:\n{out}");
    assert!(out.contains("also exports 'folds'"), "{out}");
    assert!(out.contains("also exports 'plot'"), "{out}");
}

#[test]
fn a_listed_name_the_program_binds_is_refused() {
    let dir = project(&[
        ("lib/cv.ol", LIB),
        ("main.ol", "let folds = 3\nuse lib.cv { folds }\nprintln(folds)\n"),
        (
            "ok.ol",
            "let folds = 3\n\
             use lib.cv { folds as cv_folds }\n\
             use lib.cv { helper }\n\
             use lib.cv { helper }\n\
             fn helper() = \"mine\"\n\
             println(folds)\n\
             println(cv_folds(5))\n\
             println(helper())\n",
        ),
    ]);
    let (out, rc) = tiers(&dir, "main.ol");
    assert_ne!(rc, 0, "{out}");
    assert!(out.contains("would replace `folds`"), "{out}");
    let (out, rc) = run_in(&dir, &["check", "main.ol"]);
    assert_ne!(rc, 0, "{out}");
    assert!(out.contains("would replace 'folds'"), "{out}");
    // an alias, a re-run import, and a later `fn` are all fine
    let (out, rc) = tiers(&dir, "ok.ol");
    assert_eq!(rc, 0, "{out}");
    assert_eq!(out.lines().collect::<Vec<_>>(), ["3", "10", "mine"], "{out}");
    let (out, rc) = run_in(&dir, &["check", "ok.ol"]);
    assert_eq!(rc, 0, "{out}");
}

#[test]
fn a_listed_stdlib_module_name_binds() {
    // Loom's `use lib.effects { task }`: listing it is the choice.
    let dir = project(&[
        ("lib/effects.ol", "share fn task(x) = \"wrapped \" + x\n"),
        ("main.ol", "use lib.effects { task }\nprintln(task(\"t\"))\n"),
    ]);
    let (out, rc) = tiers(&dir, "main.ol");
    assert_eq!(rc, 0, "{out}");
    assert!(out.contains("wrapped t"), "{out}");
}

#[test]
fn check_knows_each_stdlib_module_s_members() {
    let dir = project(&[
        (
            "bad.ol",
            "println(math.pi)\nprintln(str.trimm(\" a \"))\nuse gg\nlet p = gg.plott\n",
        ),
        (
            "good.ol",
            "println(math.PI)\n\
             if gui.no_such_thing == () => println(\"absent\")\n\
             let math2 = 1\n\
             use gg\n\
             let p = gg.plot\n\
             fn f(str) = str.anything\n",
        ),
    ]);
    let (out, rc) = run_in(&dir, &["check", "bad.ol"]);
    assert_ne!(rc, 0, "{out}");
    assert!(out.contains("`math.PI`"), "{out}");
    assert!(out.contains("`str.trim`"), "{out}");
    assert!(out.contains("`gg.plot`"), "{out}");
    let (out, rc) = run_in(&dir, &["check", "good.ol"]);
    assert_eq!(rc, 0, "{out}");
}

#[test]
fn an_item_named_as_its_module_stays_the_item() {
    // Loom's `use lib.theme { theme }`: the namespace never replaces the
    // item, on the first run nor a second.
    let dir = project(&[
        ("lib/theme.ol", "share fn theme(x) = \"themed \" + x\n"),
        (
            "main.ol",
            "use lib.theme { theme }\nprintln(theme(\"a\"))\nuse lib.theme { theme }\nprintln(theme(\"b\"))\n\
             use lib.theme\nprintln(theme(\"c\"))\n",
        ),
    ]);
    let (out, rc) = tiers(&dir, "main.ol");
    assert_eq!(rc, 0, "{out}");
    assert_eq!(out.lines().collect::<Vec<_>>(), ["themed a", "themed b", "themed c"], "{out}");
}

#[test]
fn an_enum_and_its_listed_variants_import_together() {
    // Importing an enum type binds its variants; listing them as well is
    // the same binding, not a clash.
    let dir = project(&[
        (
            "lib/shape.ol",
            "share type Shape = enum { Circle(Int), Dot }\n\
             share fn area(s) = match s { Circle(r) => r * r, Dot => 0 }\n",
        ),
        (
            "main.ol",
            "use lib.shape { Shape, Circle, Dot, area }\nprintln(area(Circle(3)) + area(Dot))\n",
        ),
    ]);
    let (out, rc) = tiers(&dir, "main.ol");
    assert_eq!(rc, 0, "{out}");
    assert_eq!(out.trim(), "9", "{out}");
}
