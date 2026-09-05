//! Roadmap W11 — the second reading of open-track: test blocks scope
//! their bindings, a path import wins over a stdlib name, a missing
//! dependency is named at the `use` that needs it, `dates.stamp_ms`,
//! `http.serve`'s bind option, a raising task reports itself, `olang
//! check`'s shadow warnings, and a name defined in several modules costs
//! no more to call than a name defined once.

use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("olang_w11_{}_{}", std::process::id(), tag));
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

/// (stdout, stderr, exit code) of `olang <args>` run in `dir`.
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
fn a_test_block_is_its_own_scope() {
    let ws = workspace("testscope");
    write(
        &ws,
        "t.ol",
        "test \"a binding stays inside its block\" {\n\
             let fs = 5\n\
             assert_eq(fs, 5)\n\
         }\n\
         test \"the module is still the module\" {\n\
             assert_eq(fs.exists(\"/\"), true)\n\
         }\n",
    );
    let (out, err, rc) = olang(&ws, &["test", "t.ol"]);
    assert_eq!(rc, 0, "{out}{err}");
    assert!(out.contains("2 passed, 0 failed"), "{out}");
}

#[test]
fn a_path_import_wins_over_a_stdlib_module_of_the_same_name() {
    let ws = workspace("csvshadow");
    write(
        &ws,
        "lib/csv.ol",
        "share fn to_records(text) = [\"mine: \" + text]\n",
    );
    write(
        &ws,
        "main.ol",
        "use lib.csv\nprintln(show(csv.to_records(\"x\")))\n",
    );
    let (out, err, rc) = olang(&ws, &["run", "main.ol"]);
    assert_eq!(rc, 0, "{out}{err}");
    assert_eq!(out, "[\"mine: x\"]\n");
    // `olang check` says what happened at the `use`.
    let (out, err, _) = olang(&ws, &["check", "main.ol"]);
    assert!(
        (out.clone() + &err).contains("binds `csv`, the name of a stdlib module"),
        "{out}{err}"
    );
}

#[test]
fn a_missing_dependency_is_named_at_the_use_that_needs_it() {
    let ws = workspace("missingdep");
    write(
        &ws,
        "olang.toml",
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\nhelper = { path = \"../helper_pkg\" }\nghost = { shelf = \"no_such_shelf_library_w11\" }\n",
    );
    write(
        &ws.parent().unwrap().join("helper_pkg"),
        "olang.toml",
        "[package]\nname = \"helper\"\nversion = \"0.1.0\"\n",
    );
    write(
        &ws.parent().unwrap().join("helper_pkg"),
        "index.ol",
        "share fn twice(x) = x * 2\n",
    );
    // A module that never imports the missing package runs.
    write(
        &ws,
        "fine.ol",
        "use helper { twice }\nprintln(to_string(twice(21)))\n",
    );
    let (out, err, rc) = olang(&ws, &["run", "fine.ol"]);
    assert_eq!(rc, 0, "{out}{err}");
    assert_eq!(out, "42\n");
    // The one that does is told which package is missing and why.
    write(&ws, "needs.ol", "use ghost\nprintln(\"unreachable\")\n");
    let (out, err, rc) = olang(&ws, &["run", "needs.ol"]);
    assert_ne!(rc, 0);
    let all = out + &err;
    assert!(
        all.contains("dependency 'ghost' is declared in olang.toml"),
        "{all}"
    );
    assert!(all.contains("no_such_shelf_library_w11"), "{all}");
    let _ = std::fs::remove_dir_all(ws.parent().unwrap().join("helper_pkg"));
}

#[test]
fn stamp_ms_carries_milliseconds_and_sorts() {
    let ws = workspace("stampms");
    write(
        &ws,
        "s.ol",
        "let a = dates.stamp_ms()\n\
         println(to_string(str.length(a)))\n\
         println(str.substring(a, 19, 20) + str.substring(a, 23, 24))\n\
         println(to_string(str.length(dates.stamp())))\n",
    );
    let (out, _, rc) = olang(&ws, &["run", "s.ol"]);
    assert_eq!(rc, 0, "{out}");
    assert_eq!(out, "24\n.Z\n20\n");
}

#[test]
fn serve_takes_a_bind_address_and_refuses_a_bad_one() {
    let ws = workspace("bind");
    write(
        &ws,
        "b.ol",
        "match http.serve(0, (req) => \"x\", #{ \"bind\": 5 }) {\n\
             Err(e) => println(\"err: \" + show(e)), Ok(v) => println(\"ok\") }\n",
    );
    let (out, err, rc) = olang(&ws, &["run", "b.ol"]);
    assert_eq!(rc, 0, "{out}{err}");
    assert!(out.contains("bind must be an address string"), "{out}");
}

#[test]
fn a_task_that_raises_reports_itself_on_stderr() {
    let ws = workspace("taskraise");
    write(
        &ws,
        "t.ol",
        "let t = spawn { unwrap(Err(\"the builder died\")) }\n\
         time.sleep(100)\n\
         println(\"main continues\")\n",
    );
    let (out, err, rc) = olang(&ws, &["run", "t.ol"]);
    assert_eq!(rc, 0, "{out}{err}");
    assert_eq!(out, "main continues\n");
    assert!(
        err.contains("task olang-spawn-") && err.contains("the builder died"),
        "{err}"
    );
}

#[test]
fn check_warns_about_a_parameter_that_shadows_a_called_import() {
    let ws = workspace("paramshadow");
    write(&ws, "lib/ui.ol", "share fn span(x) = x\n");
    write(
        &ws,
        "v.ol",
        "use lib.ui { span }\n\
         fn row(s, span) = span(s)\n\
         fn ok(s, other) = span(s)\n\
         println(to_string(ok(1, 2)))\n",
    );
    let (out, err, _) = olang(&ws, &["check", "v.ol"]);
    let all = out + &err;
    assert!(
        all.contains("parameter 'span' of `row` shadows the function 'span'"),
        "{all}"
    );
    assert!(!all.contains("`ok`"), "{all}");
}

#[test]
fn check_warns_about_a_let_that_takes_a_stdlib_module_name() {
    let ws = workspace("letshadow");
    write(
        &ws,
        "l.ol",
        "fn f() = {\n    let fs = 1\n    fs\n}\nprintln(to_string(f()))\n",
    );
    let (out, err, _) = olang(&ws, &["check", "l.ol"]);
    let all = out + &err;
    assert!(
        all.contains("`let fs` shadows the stdlib module `fs`"),
        "{all}"
    );
}

#[test]
fn a_name_defined_in_several_modules_costs_no_more_to_call() {
    let ws = workspace("multidef");
    for i in 1..=7 {
        write(
            &ws,
            &format!("lib/m{i}.ol"),
            &format!(
                "share fn as_str(v) = if typeof(v) == \"String\" => v else => to_string(v)\n\
                 share fn work{i}(n) = {{\n\
                     let mut acc = 0\n\
                     for i in range(0, n) {{ acc = acc + str.length(as_str(i)) }}\n\
                     acc\n\
                 }}\n"
            ),
        );
    }
    write(
        &ws,
        "lib/u.ol",
        "share fn only_here(v) = if typeof(v) == \"String\" => v else => to_string(v)\n\
         share fn workx(n) = {\n\
             let mut acc = 0\n\
             for i in range(0, n) { acc = acc + str.length(only_here(i)) }\n\
             acc\n\
         }\n",
    );
    write(
        &ws,
        "main.ol",
        "use lib.m1\nuse lib.m2\nuse lib.m3\nuse lib.m4\nuse lib.m5\nuse lib.m6\nuse lib.m7\nuse lib.u\n\
         let t0 = time.monotonic_ms()\n\
         let a = m1.work1(20000)\n\
         let seven = time.monotonic_ms() - t0\n\
         let t1 = time.monotonic_ms()\n\
         let b = u.workx(20000)\n\
         let one = time.monotonic_ms() - t1\n\
         println(to_string(a == b))\n\
         println(to_string(seven) + \" \" + to_string(one))\n",
    );
    let (out, err, rc) = olang(&ws, &["run", "main.ol"]);
    assert_eq!(rc, 0, "{out}{err}");
    let mut lines = out.lines();
    assert_eq!(lines.next(), Some("true"));
    let nums: Vec<f64> = lines
        .next()
        .unwrap()
        .split(' ')
        .map(|n| n.parse().unwrap())
        .collect();
    // The report was 300×; a generous bound still catches a return of it.
    assert!(
        nums[0] <= nums[1].max(2.0) * 10.0,
        "seven definitions {} ms vs one {} ms",
        nums[0],
        nums[1]
    );
}
