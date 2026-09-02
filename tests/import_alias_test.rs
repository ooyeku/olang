//! Import aliasing (roadmap W8): `use lib.x { name as alias }` binds
//! the export under the alias, share-use re-exports under the alias,
//! and `olang check` warns when a later bare `use` shadows an earlier
//! explicit import. Runtime cases are differentials (tiered vs
//! `--no-ovm`); the warning cases drive the real `check` command.

use std::path::PathBuf;
use std::process::Command;

/// A scratch project directory with the given (path, source) files.
fn project(files: &[(&str, &str)]) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for (p, s) in files {
        p.hash(&mut h);
        s.hash(&mut h);
    }
    let dir = std::env::temp_dir().join(format!("olang_import_alias_{:x}", h.finish()));
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

fn assert_tiers_agree(dir: &std::path::Path, file: &str) -> String {
    let (tiered, rc_t) = run_in(dir, &["run", file]);
    let (oracle, rc_o) = run_in(dir, &["--no-ovm", "run", file]);
    assert_eq!(tiered, oracle, "tiers diverged on {file}");
    assert_eq!(rc_t, rc_o, "exit codes diverged on {file}");
    tiered
}

#[test]
fn aliased_imports_bind_the_alias() {
    let dir = project(&[
        (
            "lib/report.ol",
            "share fn table(rows) = \"report:\" + to_string(len(rows))\n\
             share fn title(t) = \"# \" + t\n",
        ),
        (
            "main.ol",
            "use lib.report { table as md_table, title }\n\
             println(md_table([1, 2, 3]))\n\
             println(title(\"hi\"))\n",
        ),
    ]);
    let out = assert_tiers_agree(&dir, "main.ol");
    assert!(out.contains("report:3"), "{out}");
    assert!(out.contains("# hi"), "{out}");
}

#[test]
fn the_source_name_is_not_bound_by_an_alias() {
    let dir = project(&[
        ("lib/m.ol", "share fn f(x) = x + 1\n"),
        (
            "main.ol",
            "use lib.m { f as g }\n\
             println(g(1))\n\
             println(f(1))\n",
        ),
    ]);
    let out = assert_tiers_agree(&dir, "main.ol");
    assert!(out.contains("2"), "{out}");
    assert!(
        out.contains("Undefined variable") || out.contains("not a function"),
        "the unaliased name must not be bound:\n{out}"
    );
}

#[test]
fn share_use_reexports_under_the_alias() {
    let dir = project(&[
        ("lib/inner.ol", "share fn greet(n) = \"hi \" + n\n"),
        (
            "lib/facade.ol",
            "share use lib.inner { greet as welcome }\n\
             share fn own(x) = x\n",
        ),
        (
            "main.ol",
            "use lib.facade { welcome, own }\n\
             println(welcome(\"ada\"))\n\
             println(own(7))\n",
        ),
    ]);
    let out = assert_tiers_agree(&dir, "main.ol");
    assert!(out.contains("hi ada"), "{out}");
}

#[test]
fn an_alias_resolves_the_shadowing_trap() {
    // The original incident: a bare `use term` rebinding `table` from an
    // earlier explicit import. Aliased, both coexist.
    let dir = project(&[
        ("lib/report.ol", "share fn table(rows) = len(rows)\n"),
        (
            "main.ol",
            "use lib.report { table as md_table }\n\
             use term\n\
             println(md_table([1, 2]))\n\
             println(term.table([\"h\"], [[\"v\"]]))\n",
        ),
    ]);
    let out = assert_tiers_agree(&dir, "main.ol");
    assert!(out.contains('2'), "{out}");
}

#[test]
fn check_warns_on_wildcard_shadowing_an_explicit_import() {
    let dir = project(&[
        ("lib/report.ol", "share fn table(rows) = len(rows)\n"),
        (
            "shadowed.ol",
            "use lib.report { table }\n\
             use term\n\
             println(table([1]))\n",
        ),
        (
            "aliased.ol",
            "use lib.report { table as md_table }\n\
             use term\n\
             println(md_table([1]))\n",
        ),
    ]);
    let (out, rc) = run_in(&dir, &["check", "shadowed.ol"]);
    assert_eq!(rc, 0, "advisory only:\n{out}");
    assert!(
        out.contains("shadowing the explicit import from line 1"),
        "warning missing:\n{out}"
    );
    assert!(out.contains("term.table"), "no qualified hint:\n{out}");

    let (out, rc) = run_in(&dir, &["check", "aliased.ol"]);
    assert_eq!(rc, 0);
    assert!(
        !out.contains("shadowing"),
        "aliased import must not warn:\n{out}"
    );
}

#[test]
fn check_warns_when_a_module_re_exports_one_name_twice() {
    // The web SDK's incident: `lib.sql.row` and `lib.ui.row` both
    // re-exported from index.ol; the later silently won for every
    // importer, and `use web { row }` failed far away as an arity
    // mismatch.
    let dir = project(&[
        ("lib/sql.ol", "share fn row(conn, q, params) = 1\n"),
        ("lib/ui.ol", "share fn row(children) = 2\n"),
        (
            "index.ol",
            "share use lib.sql { row }\nshare use lib.ui { row }\n",
        ),
        (
            "fixed.ol",
            "share use lib.sql { row as one }\nshare use lib.ui { row }\n",
        ),
    ]);
    let (out, rc) = run_in(&dir, &["check", "index.ol"]);
    assert_eq!(rc, 0, "{out}");
    assert!(
        out.contains("re-exports 'row', already re-exported on line 1"),
        "{out}"
    );
    let (out, rc) = run_in(&dir, &["check", "fixed.ol"]);
    assert_eq!(rc, 0, "{out}");
    assert!(!out.contains("already re-exported"), "{out}");
}
