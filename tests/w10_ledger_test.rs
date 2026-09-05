//! The W10 ledger rows: fixed-decimal formatting, `db.transaction`,
//! `db.migrate`, the `query_one` absent shape, and the Result-returning
//! constructors (`ods.try_series`, `ods.try_frame_from_records`,
//! `viz.check`). Differentials against `--no-ovm` where a tier is involved.

use std::process::Command;

fn run(source: &str, no_ovm: bool) -> (String, i32) {
    let dir = std::env::temp_dir().join("olang_w10_ledger_tests");
    std::fs::create_dir_all(&dir).expect("mkdir");
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    source.hash(&mut h);
    let path = dir.join(format!("t{:x}.ol", h.finish()));
    std::fs::write(&path, source).expect("write");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_olang"));
    if no_ovm {
        cmd.arg("--no-ovm");
    }
    let out = cmd.arg("run").arg(&path).output().expect("run");
    (
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
        out.status.code().unwrap_or(-1),
    )
}

fn agree(source: &str) -> (String, i32) {
    let (tiered, rc_t) = run(source, false);
    let (oracle, rc_o) = run(source, true);
    assert_eq!(tiered, oracle, "tiers diverged on:\n{source}");
    assert_eq!(rc_t, rc_o);
    (tiered, rc_t)
}

#[test]
fn fixed_and_thousands_format_columns_not_values() {
    let (out, rc) = agree(
        "println(str.fixed(12.5, 2))\n\
         println(str.fixed(3, 2))\n\
         println(str.fixed(-3.075, 2))\n\
         println(str.fixed(-0.001, 2))\n\
         println(str.fixed(1e21, 0))\n\
         println(str.fixed(2.5, 0))\n\
         println(str.fixed(9007199254740993, 0))\n\
         println(str.thousands(1234567.891, 2))\n\
         println(str.thousands(-1234, 0))\n\
         println(str.thousands(999.5, 1))\n\
         println(str.thousands(1000000, 0))\n\
         println(str.thousands(0.5, 2))",
    );
    assert_eq!(rc, 0, "{out}");
    assert_eq!(
        out,
        "12.50\n3.00\n-3.08\n0.00\n1000000000000000000000\n2\n9007199254740993\n1,234,567.89\n-1,234\n999.5\n1,000,000\n0.50\n"
    );
}

#[test]
fn fixed_refuses_non_numbers_and_bad_digits() {
    let (out, rc) = run("println(str.fixed(\"12\", 2))", false);
    assert_ne!(rc, 0);
    assert!(out.contains("expected a number, got String"), "{out}");
    let (out, rc) = run("println(str.fixed(1.0, 21))", false);
    assert_ne!(rc, 0);
    assert!(out.contains("digits must be between 0 and 20"), "{out}");
}

#[test]
fn transaction_commits_on_ok_and_rolls_back_on_err_or_raise() {
    let (out, rc) = agree(
        "let c = unwrap(db.open(\":memory:\"))\n\
         unwrap(db.execute(c, \"CREATE TABLE t (n INTEGER)\"))\n\
         let committed = db.transaction(c, (c) => {\n\
             unwrap(db.execute(c, \"INSERT INTO t VALUES (1)\"))\n\
             Ok(\"in\")\n\
         })\n\
         println(show(committed))\n\
         let rolled = db.transaction(c, (c) => {\n\
             unwrap(db.execute(c, \"INSERT INTO t VALUES (2)\"))\n\
             Err(\"changed my mind\")\n\
         })\n\
         println(show(rolled))\n\
         // A plain value commits too, and is handed through.\n\
         let plain = db.transaction(c, (c) => {\n\
             unwrap(db.execute(c, \"INSERT INTO t VALUES (3)\"))\n\
             42\n\
         })\n\
         println(show(plain))\n\
         println(show(map_get(unwrap(db.query_one(c, \"SELECT COUNT(*) AS n FROM t\")), \"n\")))\n\
         println(show(map_get(unwrap(db.query_one(c, \"SELECT SUM(n) AS s FROM t\")), \"s\")))\n\
         // A raise inside f rolls back and propagates.\n\
         db.transaction(c, (c) => {\n\
             unwrap(db.execute(c, \"INSERT INTO t VALUES (4)\"))\n\
             unwrap(Err(\"boom\"))\n\
         })",
    );
    assert_ne!(rc, 0, "{out}");
    assert!(
        out.starts_with("Ok(\"in\")\nErr(\"changed my mind\")\n42\n2\n4\n"),
        "{out}"
    );
    assert!(out.contains("boom"), "{out}");
    // After the raise the row is gone: a fresh program proves the count.
    let (out, rc) = agree(
        "let c = unwrap(db.open(\":memory:\"))\n\
         unwrap(db.execute(c, \"CREATE TABLE t (n INTEGER)\"))\n\
         let r = db.transaction(c, (c) => {\n\
             unwrap(db.execute(c, \"INSERT INTO t VALUES (4)\"))\n\
             Err(\"no\")\n\
         })\n\
         println(show(map_get(unwrap(db.query_one(c, \"SELECT COUNT(*) AS n FROM t\")), \"n\")))",
    );
    assert_eq!(rc, 0, "{out}");
    assert_eq!(out, "0\n");
}

#[test]
fn migrate_applies_each_version_once_and_names_a_failing_statement() {
    let (out, rc) = agree(
        "let c = unwrap(db.open(\":memory:\"))\n\
         let steps = [\n\
             [\"CREATE TABLE todos (id INTEGER PRIMARY KEY, title TEXT NOT NULL)\"],\n\
             [\"ALTER TABLE todos ADD COLUMN done INTEGER NOT NULL DEFAULT 0\",\n\
              \"CREATE INDEX idx_done ON todos(done)\"]\n\
         ]\n\
         println(show(db.migrate(c, steps)))\n\
         // Again: nothing to do, same version.\n\
         println(show(db.migrate(c, steps)))\n\
         // A third version whose second statement fails: v3 rolls back whole,\n\
         // the version stays 2, and the Err names the statement's failure.\n\
         let broken = steps + [[\"CREATE TABLE extra (id INTEGER)\", \"CREATE TABLE todos (x)\"]]\n\
         match db.migrate(c, broken) { Ok(v) => println(\"ok\"), Err(e) => println(e) }\n\
         println(show(map_get(unwrap(db.query_one(c, \"SELECT version FROM schema_version\")), \"version\")))\n\
         println(show(len(unwrap(db.query(c, \"SELECT name FROM sqlite_master WHERE name = 'extra'\")))))\n\
         // A single statement may stand for a version.\n\
         println(show(db.migrate(c, steps + [\"CREATE TABLE notes (id INTEGER)\"])))",
    );
    assert_eq!(rc, 0, "{out}");
    assert!(
        out.starts_with("Ok(2)\nOk(2)\ndb.migrate: migration v3 failed: "),
        "{out}"
    );
    assert!(out.contains("already exists"), "{out}");
    assert!(out.ends_with("\n2\n0\nOk(3)\n"), "{out}");
}

#[test]
fn query_one_absent_is_unit_and_a_row_is_a_map() {
    let (out, rc) = agree(
        "let c = unwrap(db.open(\":memory:\"))\n\
         unwrap(db.execute(c, \"CREATE TABLE t (n INTEGER)\"))\n\
         let none = unwrap(db.query_one(c, \"SELECT n FROM t\"))\n\
         println(show(none == ()))\n\
         unwrap(db.execute(c, \"INSERT INTO t VALUES (NULL)\"))\n\
         let row = unwrap(db.query_one(c, \"SELECT n FROM t\"))\n\
         println(show(row == ()))\n\
         println(typeof(row))",
    );
    assert_eq!(rc, 0, "{out}");
    assert_eq!(out, "true\nfalse\nMap\n");
}

#[test]
fn try_constructors_and_viz_check_return_results() {
    let (out, rc) = agree(
        "use viz\n\
         println(show(is_ok(ods.try_series([1, 2, 3]))))\n\
         println(show(ods.try_series([1, \"x\"])))\n\
         println(show(is_ok(ods.try_frame_from_records([#{ \"a\": 1 }, #{ \"a\": 2 }]))))\n\
         match ods.try_frame_from_records([#{ \"a\": 1 }, #{ \"a\": \"x\" }]) {\n\
             Ok(f) => println(\"ok\"), Err(e) => println(e) }\n\
         println(show(ods.try_frame_from_records(3)))\n\
         let good = #{ \"mark\": \"bar\", \"data\": [#{ \"a\": \"x\", \"b\": 1 }], \"x\": \"a\", \"y\": \"b\" }\n\
         println(show(is_ok(viz.check(good))))\n\
         println(show(viz.check(#{ \"mark\": \"bar\", \"data\": [], \"x\": \"a\", \"y\": \"b\" })))\n\
         match viz.check(#{ \"type\": \"bar\", \"data\": [#{ \"a\": 1 }] }) {\n\
             Ok(s) => println(\"ok\"), Err(e) => println(str.substring(e, 0, 30)) }\n\
         println(show(viz.check(#{ \"layers\": [#{ \"mark\": \"line\", \"data\": [], \"x\": \"a\", \"y\": \"b\" }] })))\n\
         // A check that passes renders.\n\
         println(show(str.length(viz.chart(unwrap(viz.check(good)))) > 100))",
    );
    assert_eq!(rc, 0, "{out}");
    assert_eq!(
        out,
        "true\nErr(\"ods.series: cannot mix String with other element types\")\ntrue\ncolumn 'a': ods.series: cannot mix String with other element types\nErr(\"ods.try_frame_from_records expects a list of maps, got Int\")\ntrue\nErr(\"viz: no data points to draw\")\nviz: unknown key 'type' in the\nErr(\"viz: no data points to draw\")\ntrue\n"
    );
}
