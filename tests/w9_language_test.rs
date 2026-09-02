//! Language rows from roadmap W9, pinned as differentials against
//! `--no-ovm`: pipelines into arbitrary callables (the native/wasm
//! split), `to_string` as the identity on strings, `meta.eval`'s budget,
//! and `col.any`/`col.all` agreeing (and staying fast) when a hot
//! function calls them with a capturing predicate.

use std::process::Command;

fn run(source: &str, no_ovm: bool) -> (String, i32) {
    let dir = std::env::temp_dir().join("olang_w9_language_tests");
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
fn a_pipeline_accepts_any_callable_on_its_right() {
    let (out, rc) = agree(
        "println(to_string(5 |> ((x) => x * 2)))\n\
         let steps = [(x) => x + 1, (x) => x * 10]\n\
         println(to_string(5 |> steps[1]))\n\
         let neg = (x) => 0 - x\n\
         println(to_string(7 |> neg))\n\
         println(to_string([3, 1, 2] |> ((xs) => len(xs))))",
    );
    assert_eq!(rc, 0, "{out}");
    assert_eq!(out, "10\n50\n-7\n3\n");
}

#[test]
fn identifiers_may_begin_with_an_underscore() {
    let (out, rc) = agree(
        "let _tmp = 41\n\
         let __gen0 = _tmp + 1\n\
         fn _helper(_x) = _x * 2\n\
         println(to_string(_helper(__gen0)))\n\
         for _ in 0..2 { print(\"x\") }\n\
         println(\"\")\n\
         let _ = 5\n\
         match 3 { _n if _n > 2 => println(\"big\"), _ => println(\"small\") }",
    );
    assert_eq!(rc, 0, "{out}");
    assert_eq!(out, "84\nxx\nbig\n");
    // A lone `_` is still the wildcard, never a readable name.
    let (out, rc) = agree("for _ in 0..1 { println(_) }");
    assert_ne!(rc, 0, "{out}");
    // A name that begins with a keyword stays a name: `spawn_cost` and
    // `meta_data` once read as `spawn _cost` / `meta _data` the moment
    // `_` could begin an identifier.
    let (out, rc) = agree(
        "let t0 = 5\n\
         let spawn_cost = 9 - t0\n\
         let meta_data = spawn_cost * 2\n\
         let t = spawn { 40 + 2 }\n\
         println(to_string(spawn_cost) + \" \" + to_string(meta_data) + \" \" + to_string(task.join(t)))",
    );
    assert_eq!(rc, 0, "{out}");
    assert_eq!(out, "4 8 42\n");
}

#[test]
fn piping_into_a_non_function_still_errors_by_name() {
    let (out, rc) = agree("let n = 3\nprintln(5 |> n)");
    assert_ne!(rc, 0);
    assert!(out.contains("not a function"), "{out}");
}

#[test]
fn to_string_is_the_identity_on_strings() {
    let (out, rc) = agree(
        "println(to_string(\"open\"))\n\
         println(len(to_string(\"open\")))\n\
         println(to_string(\"open\") == show(\"open\"))\n\
         let m = #{ \"state\": \"open\" }\n\
         println(to_string(map_get(m, \"state\")) == \"open\")\n\
         println(to_string([\"a\"]))\n\
         println(to_string(42) + to_string(true))",
    );
    assert_eq!(rc, 0, "{out}");
    assert_eq!(out, "open\n4\ntrue\ntrue\n[\"a\"]\n42true\n");
}

#[test]
fn meta_eval_step_budget_stops_a_runaway_rule() {
    let (out, rc) = agree(
        "let r = meta.eval(\"let mut i = 0\\nwhile true { i = i + 1 }\", #{ \"max_steps\": 10000 })\n\
         match r { Ok(v) => println(\"ran\"), Err(e) => println(e) }\n\
         let ok = meta.eval(\"let mut t = 0\\nfor i in 0..100 { t = t + i }\\nt\", #{ \"max_steps\": 10000 })\n\
         println(show(ok))\n\
         let deep = meta.eval(\"fn f(n) = if n == 0 => 0 else => f(n - 1)\\nf(50)\", #{ \"max_steps\": 20 })\n\
         match deep { Ok(v) => println(\"ran\"), Err(e) => println(e) }",
    );
    assert_eq!(rc, 0, "{out}");
    let lines: Vec<&str> = out.lines().collect();
    assert!(lines[0].contains("budget exceeded"), "{out}");
    assert!(lines[0].contains("max_steps"), "{out}");
    assert_eq!(lines[1], "Ok(4950)");
    assert!(lines[2].contains("budget exceeded"), "{out}");
}

#[test]
fn meta_eval_timeout_stops_a_runaway_rule() {
    let (out, rc) = agree(
        "let r = meta.eval(\"let mut i = 0\\nwhile true { i = i + 1 }\", #{ \"timeout_ms\": 30 })\n\
         match r { Ok(v) => println(\"ran\"), Err(e) => println(e) }",
    );
    assert_eq!(rc, 0, "{out}");
    assert!(out.contains("budget exceeded"), "{out}");
    assert!(out.contains("timeout_ms"), "{out}");
}

#[test]
fn meta_eval_rejects_malformed_options() {
    let (out, rc) = agree("meta.eval(\"1\", #{ \"steps\": 5 })");
    assert_ne!(rc, 0);
    assert!(out.contains("unknown option 'steps'"), "{out}");
    let (out, rc) = agree("meta.eval(\"1\", #{ \"max_steps\": -1 })");
    assert_ne!(rc, 0);
    assert!(out.contains("non-negative"), "{out}");
    let (out, rc) = agree("meta.eval(\"1\", 7)");
    assert_ne!(rc, 0);
    assert!(out.contains("options must be a map"), "{out}");
}

#[test]
fn any_and_all_agree_with_capturing_predicates_in_hot_functions() {
    let (out, rc) = agree(
        "let recs = map(0..500, (i) => #{ \"id\": i, \"tags\": [\"t\" + show(i % 7), \"x\" + show(i % 3)] })\n\
         let include = [\"t6\", \"nope\"]\n\
         fn any_hit(rec, inc) = col.any(map_get(rec, \"tags\"), (t) => contains(inc, t))\n\
         fn all_miss(rec, inc) = col.all(map_get(rec, \"tags\"), (t) => contains(inc, t) == false)\n\
         fn unit_pred(rec, inc) = col.any(map_get(rec, \"tags\"), (t) => ())\n\
         fn count(f) = { let mut n = 0; for r in recs { if f(r, include) => { n = n + 1 } }; n }\n\
         println(show(count(any_hit)) + \" \" + show(count(all_miss)) + \" \" + show(count(unit_pred)))\n\
         println(show(col.any([], (x) => true)) + \" \" + show(col.all([], (x) => false)))\n\
         println(show(col.any([1, 2], (x) => x)) + \" \" + show(col.all([0, 1], (x) => x)))",
    );
    assert_eq!(rc, 0, "{out}");
    assert_eq!(out, "71 429 0\nfalse true\ntrue true\n");
}

#[test]
fn any_in_a_hot_function_is_not_slower_with_the_tier_on() {
    // The regression this pins: the bridged path recompiled the predicate
    // per callback. Compare wall time tier-on vs tier-off on the same
    // program; the tier must not lose by more than noise allows.
    let source = "let recs = map(0..3000, (i) => #{ \"id\": i, \"tags\": [\"t\" + show(i % 7), \"x\" + show(i % 3)] })\n\
                  let include = [\"t6\", \"nope\"]\n\
                  fn any_hit(rec, inc) = col.any(map_get(rec, \"tags\"), (t) => contains(inc, t))\n\
                  fn count() = { let mut n = 0; for r in recs { if any_hit(r, include) => { n = n + 1 } }; n }\n\
                  let t0 = time.monotonic_ms()\n\
                  let n = count()\n\
                  println(show(time.monotonic_ms() - t0))";
    let ms = |no_ovm: bool| -> i64 {
        let (out, rc) = run(source, no_ovm);
        assert_eq!(rc, 0, "{out}");
        out.trim().parse().expect("ms")
    };
    let tiered = ms(false);
    let oracle = ms(true);
    assert!(
        tiered <= oracle.max(5) * 2,
        "col.any in a hot function: {tiered} ms with the tier vs {oracle} ms interpreted"
    );
}

/// A scratch project with the given files; returns its directory. The
/// tag keeps two tests with identical files out of one directory.
fn project(tag: &str, files: &[(&str, &str)]) -> std::path::PathBuf {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    tag.hash(&mut h);
    for (p, c) in files {
        p.hash(&mut h);
        c.hash(&mut h);
    }
    let dir = std::env::temp_dir().join(format!("olang_w9_project_{:x}", h.finish()));
    let _ = std::fs::remove_dir_all(&dir);
    for (path, contents) in files {
        let full = dir.join(path);
        std::fs::create_dir_all(full.parent().unwrap()).expect("mkdir");
        std::fs::write(&full, contents).expect("write");
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

#[test]
fn an_error_inside_an_imported_module_names_that_module_and_line() {
    let dir = project(
        "module-error",
        &[
            (
                "olang.toml",
                "[package]\nname = \"app\"\nversion = \"0.1.0\"\n",
            ),
            (
                "lib/rt.ol",
                "share fn g() = undefined_thing()\nlet boom = g()\n",
            ),
            ("t.ol", "use lib.rt { g }\nprintln(1)\n"),
        ],
    );
    for flags in [&[][..], &["--no-ovm"][..]] {
        let mut args: Vec<&str> = flags.to_vec();
        args.extend(["run", "t.ol"]);
        let (out, rc) = run_in(&dir, &args);
        assert_ne!(rc, 0);
        assert!(out.contains("Undefined variable: undefined_thing"), "{out}");
        // The module's own file and line, with its source rendered.
        assert!(out.contains("lib/rt.ol:2:1"), "{out}");
        assert!(out.contains("let boom = g()"), "{out}");
        assert!(
            !out.contains("t.ol:1:1"),
            "the importer's use line is not blamed:\n{out}"
        );
    }
    // A `share meta fn` in a module is a parse error naming the module.
    let dir = project(
        "share-meta",
        &[
            ("lib/m.ol", "share meta fn f(x) = x\nshare fn g() = 1\n"),
            ("t.ol", "use lib.m { g }\nprintln(g())\n"),
        ],
    );
    let (out, rc) = run_in(&dir, &["run", "t.ol"]);
    assert_ne!(rc, 0);
    // The report wraps at the terminal width; compare with whitespace folded.
    let flat = out.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(flat.contains("m.ol"), "{out}");
    assert!(flat.contains("A meta fn cannot be shared"), "{out}");
}

#[test]
fn olang_eval_prints_the_value_and_sees_the_projects_libraries() {
    let dir = project(
        "eval",
        &[
            (
                "olang.toml",
                "[package]\nname = \"app\"\nversion = \"0.1.0\"\n",
            ),
            ("lib/util.ol", "share fn twice(x) = x * 2\n"),
        ],
    );
    let (out, rc) = run_in(&dir, &["eval", "1 + 2"]);
    assert_eq!(rc, 0, "{out}");
    assert_eq!(out.trim(), "3");
    let (out, rc) = run_in(&dir, &["eval", "use lib.util { twice }\ntwice(21)"]);
    assert_eq!(rc, 0, "{out}");
    assert_eq!(out.trim(), "42");
    // Unit prints nothing; an error reports and exits non-zero.
    let (out, rc) = run_in(&dir, &["eval", "let x = 1"]);
    assert_eq!(rc, 0, "{out}");
    assert_eq!(out.trim(), "");
    let (out, rc) = run_in(&dir, &["eval", "nope()"]);
    assert_ne!(rc, 0);
    assert!(out.contains("Undefined variable: nope"), "{out}");
}

#[test]
fn a_script_outside_a_project_falls_back_to_the_working_directorys_project() {
    let dir = project(
        "outside",
        &[
            (
                "olang.toml",
                "[package]\nname = \"app\"\nversion = \"0.1.0\"\n",
            ),
            ("lib/util.ol", "share fn twice(x) = x * 2\n"),
        ],
    );
    let scratch = std::env::temp_dir().join("olang_w9_scratch_outside");
    std::fs::create_dir_all(&scratch).unwrap();
    let script = scratch.join("probe.ol");
    std::fs::write(&script, "use lib.util { twice }\nprintln(show(twice(4)))\n").unwrap();
    let (out, rc) = run_in(&dir, &["run", script.to_str().unwrap()]);
    assert_eq!(rc, 0, "{out}");
    assert_eq!(out.trim(), "8");
}

#[test]
fn a_closure_sees_a_top_level_binding_declared_after_it_on_every_tier() {
    // Interpreted, a closure's free name resolves through the live scope
    // chain, so a `let` declared after the closure is visible when it
    // runs. With the tier on, a function value the compiler declined ran
    // on the bridge interpreter, which knew the program's functions but
    // not its top-level `let`s — the open-track click that reported
    // "Undefined variable" only in the browser. The bridge now sees the
    // host's globals.
    let (out, rc) = agree(
        "fn call_it(f) = f()\n\
         let g = () => later\n\
         let later = 3\n\
         println(show(g()) + \" \" + show(call_it(g)))\n\
         fn h() = later2\n\
         let later2 = 5\n\
         println(show(call_it(h)))\n\
         let store = cell(())\n\
         fn register(f) = cell.set(store, f)\n\
         register(() => later3)\n\
         let later3 = 7\n\
         fn dispatch() = { let f2 = cell.get(store); f2() }\n\
         println(show(dispatch()))",
    );
    assert_eq!(rc, 0, "{out}");
    assert_eq!(out, "3 3\n5\n7\n");
}
