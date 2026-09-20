//! Roadmap W18 — what open-track and Shuttle asked for after 0.84.0:
//! the route index keeps the first route at a path, a module's
//! functions and macros reach their private helpers at expansion, a
//! package module reaches a sibling's macro, and a served test needs
//! no OLANG_TEST juggling for `http.defer`.

use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("olang_w18_{}_{}", std::process::id(), tag));
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
fn a_modules_shared_functions_and_macros_reach_its_private_helpers_at_expansion() {
    let ws = workspace("private");
    write(
        &ws,
        "lib/rules.ol",
        "fn hidden(s) = \"<\" + s + \">\"\n\
         share fn shout(s) = hidden(str.to_upper(s))\n\
         meta fn wrap(e) = meta.lit(hidden(unwrap(meta.eval(e))))\n",
    );
    write(
        &ws,
        "ok.ol",
        "use lib.rules { shout, wrap }\n\
         meta fn m(e) = meta.lit(shout(unwrap(meta.eval(e))))\n\
         println(@m(\"hi\"))\n\
         println(@wrap(\"x\"))\n\
         println(shout(\"rt\"))\n",
    );
    let (out, err, rc) = olang(&ws, &["run", "ok.ol"]);
    assert_eq!(rc, 0, "{out}{err}");
    assert_eq!(out, "<HI>\n<x>\n<RT>\n");
    // The importer still cannot name the helper: private is private.
    write(
        &ws,
        "private.ol",
        "use lib.rules { shout }\nmeta fn m(e) = `\"${hidden(\"x\")}\"`\nprintln(@m(1))\n",
    );
    let (out, err, rc) = olang(&ws, &["run", "private.ol"]);
    assert_ne!(rc, 0, "{out}");
    assert!((out + &err).contains("hidden"));
}

#[test]
fn a_package_module_uses_a_siblings_macro_from_anywhere() {
    let ws = workspace("sibling");
    write(
        &ws,
        "olang.toml",
        "[package]\nname = \"shuttle\"\nversion = \"0.1.0\"\n",
    );
    write(
        &ws,
        "lib/decl.ol",
        "fn quote(s) = \"\\\"\" + s + \"\\\"\"\n\
         meta fn resource(name) = \"#{ \\\"resource\\\": \" + quote(unwrap(meta.eval(name))) + \" }\"\n",
    );
    write(
        &ws,
        "lib/sample.ol",
        "use lib.decl { resource }\n\
         share let SAMPLE = @resource(\"issues\")\n\
         share fn sample_name() = map_get(SAMPLE, \"resource\")\n\
         test \"a sibling's macro expands inside the package\" {\n\
             assert_eq(sample_name(), \"issues\")\n\
         }\n",
    );
    write(
        &ws,
        "main.ol",
        "use lib.sample { sample_name }\nprintln(sample_name())\n",
    );
    // From the package root, as a consumer of the module.
    let (out, err, rc) = olang(&ws, &["run", "main.ol"]);
    assert_eq!(rc, 0, "{out}{err}");
    assert_eq!(out, "issues\n");
    // The module's own tests, and the module run from its own directory.
    let (out, err, rc) = olang(&ws, &["test", "lib/sample.ol"]);
    assert_eq!(rc, 0, "{out}{err}");
    assert!(out.contains("1 passed"), "{out}");
    let (out, err, rc) = olang(&ws.join("lib"), &["run", "sample.ol"]);
    assert_eq!(rc, 0, "{out}{err}");
}

#[test]
fn a_record_shape_alias_is_usable_wherever_an_annotation_goes() {
    let ws = workspace("alias");
    write(
        &ws,
        "a.ol",
        "fn title_of(t: Task) = map_get(t, \"title\")\n\
         type Task = { title: String, done: Bool }\n\
         type Tasks = [Task]\n\
         fn first_title(ts: Tasks) -> String = title_of(head(ts))\n\
         let t: Task = #{ \"title\": \"ship\", \"done\": false }\n\
         println(title_of(t))\n\
         println(first_title([t]))\n\
         let f = (x: Task) => map_get(x, \"done\")\n\
         println(show(f(t)))\n\
         println(show(attempt(() => first_title(3))))\n\
         meta fn shaped(decl) = str.replace(decl, \"(r)\", \"(r: Task)\")\n\
         @shaped\n\
         fn read_title(r) = map_get(r, \"titel\")\n\
         println(show(read_title(t)))\n",
    );
    for mode in [vec!["run"], vec!["--no-ovm", "run"]] {
        let mut args = mode.clone();
        args.push("a.ol");
        let (out, err, rc) = olang(&ws, &args);
        assert_eq!(rc, 0, "{mode:?}: {out}{err}");
        assert_eq!(
            out,
            "ship\nship\nfalse\nErr(\"parameter 'ts' of first_title expects List, got Int\")\n()\n",
            "{mode:?}"
        );
    }
    // The checker reads the shape through the alias — in a macro's output too.
    let (out, err, _) = olang(&ws, &["check", "a.ol"]);
    let all = out + &err;
    assert!(
        all.contains("`titel` is not a key of the declared shape { title: String, done: Bool }"),
        "{all}"
    );
    assert!(all.contains("did you mean `title`?"), "{all}");
    // meta.parse names the alias.
    write(
        &ws,
        "m.ol",
        "let nodes = unwrap(meta.parse(\"type Task = { title: String }\"))\n\
         println(map_get(head(nodes), \"definition\") + \" \" + map_get(map_get(head(nodes), \"type\"), \"text\"))\n",
    );
    let (out, err, rc) = olang(&ws, &["run", "m.ol"]);
    assert_eq!(rc, 0, "{out}{err}");
    assert_eq!(out, "alias { title: String }\n");
}

#[test]
fn encode_query_takes_a_map_as_well_as_a_record() {
    let ws = workspace("query");
    write(
        &ws,
        "q.ol",
        "println(show(http.encode_query(#{ \"q\": \"a b\", \"page\": 2 })))\n\
         println(show(http.encode_query({ q: \"x&y\", ok: true })))\n\
         println(show(http.encode_query(\"nope\")))\n",
    );
    let (out, err, rc) = olang(&ws, &["run", "q.ol"]);
    assert_eq!(rc, 0, "{out}{err}");
    assert_eq!(
        out,
        "Ok(\"page=2&q=a%20b\")\nOk(\"ok=true&q=x%26y\")\nErr(\"encode_query: argument must be a map or a record\")\n"
    );
}

#[test]
fn a_modules_test_block_above_its_callee_runs_after_the_declarations() {
    let ws = workspace("testorder");
    // A project root, as the SDK has: module paths resolve against it.
    write(
        &ws,
        "olang.toml",
        "[package]\nname = \"testorder\"\nversion = \"0.1.0\"\n",
    );
    write(
        &ws,
        "lib/m.ol",
        "test \"calls a function declared below\" {\n    assert_eq(f(), 1)\n}\n\
         share fn f() = helper() + 1\n\
         fn helper() = 0\n",
    );
    write(
        &ws,
        "tests/t.ol",
        "use lib.m { f }\ntest \"uses the module\" { assert_eq(f(), 1) }\n",
    );
    // Imported under `olang test`, the module's tests run at load: the one
    // above its callee must see the whole module declared. A directory
    // run, because a named file runs its own blocks only (W21).
    let (out, err, rc) = olang(&ws, &["test", "tests"]);
    assert_eq!(rc, 0, "{out}{err}");
    assert!(out.contains("2 passed, 0 failed"), "{out}{err}");
    let (out, err, rc) = olang(&ws, &["test", "tests/t.ol"]);
    assert_eq!(rc, 0, "{out}{err}");
    assert!(out.contains("1 passed, 0 failed"), "{out}{err}");
}

#[test]
fn a_decorator_sits_above_a_share_declaration() {
    let ws = workspace("sharedeco");
    write(
        &ws,
        "olang.toml",
        "[package]\nname = \"deco\"\nversion = \"0.1.0\"\n",
    );
    write(
        &ws,
        "lib/shaped.ol",
        "meta fn shaped(decl) = \"type Row = { title: String }\\n\" + decl\n\
         @shaped\n\
         share let ROW = #{ \"title\": \"x\" }\n\
         share fn row_title(r: Row) = map_get(r, \"title\")\n",
    );
    write(
        &ws,
        "main.ol",
        "use lib.shaped { ROW, row_title }\nprintln(row_title(ROW))\n",
    );
    let (out, err, rc) = olang(&ws, &["run", "main.ol"]);
    assert_eq!(rc, 0, "{out}{err}");
    assert_eq!(out, "x\n");
    let (out, err, rc) = olang(&ws, &["expand", "lib/shaped.ol"]);
    assert_eq!(rc, 0, "{out}{err}");
    assert!(out.contains("share let ROW"), "{out}");
}

#[test]
fn a_tests_imported_module_reaches_a_siblings_macro_from_any_directory() {
    let ws = workspace("testsibling");
    write(
        &ws,
        "olang.toml",
        "[package]\nname = \"shuttle\"\nversion = \"0.1.0\"\n",
    );
    write(
        &ws,
        "index.ol",
        "share use lib.sample { SAMPLE, sample_name }\n",
    );
    write(
        &ws,
        "lib/decl.ol",
        "fn quote(s) = \"\\\"\" + s + \"\\\"\"\n\
         meta fn resource(name) = \"#{ \\\"resource\\\": \" + quote(unwrap(meta.eval(name))) + \" }\"\n",
    );
    write(
        &ws,
        "lib/sample.ol",
        "use lib.decl { resource }\n\
         share let SAMPLE = @resource(\"issues\")\n\
         share fn sample_name() = map_get(SAMPLE, \"resource\")\n",
    );
    write(
        &ws,
        "tests/t.ol",
        "use lib.sample { sample_name }\n\
         test \"a test's import reaches the sibling macro\" { assert_eq(sample_name(), \"issues\") }\n",
    );
    for (dir, file) in [(ws.clone(), "tests/t.ol"), (ws.join("tests"), "t.ol")] {
        let (out, err, rc) = olang(&dir, &["test", file]);
        assert_eq!(rc, 0, "{file}: {out}{err}");
        assert!(out.contains("1 passed, 0 failed"), "{file}: {out}{err}");
    }
}

#[test]
fn meta_parse_emits_patterns_and_annotations_as_nodes() {
    let ws = workspace("metanodes");
    write(
        &ws,
        "m.ol",
        "let src = \"fn area(s: Shape, t: { title: String, n: [Int] }) -> Float = match s { Circle(r) => 1.0, Rect(w, h) if w > 0.0 => w * h, 1 | 2 => 0.5, _ => 0.0 }\\nlet p: Task = (x: Int) => x\"\n\
         let nodes = unwrap(meta.parse(src))\n\
         let f = nodes[0]\n\
         let t = nodes[1]\n\
         let ps = map_get(f, \"parameters\")\n\
         println(map_get(map_get(ps[0], \"type\"), \"form\") + \" \" + map_get(map_get(ps[0], \"type\"), \"name\"))\n\
         let shape = map_get(ps[1], \"type\")\n\
         println(map_get(shape, \"form\") + \" \" + join(map(map_get(shape, \"fields\"), (fl) => map_get(fl, \"name\") + \":\" + map_get(map_get(fl, \"type\"), \"text\")), \",\"))\n\
         println(map_get(map_get(f, \"return_type\"), \"text\"))\n\
         let arms = map_get(map_get(f, \"body\"), \"arms\")\n\
         for a in arms { let pt = map_get(a, \"pattern\"); println(map_get(pt, \"form\") + \" \" + map_get(pt, \"text\")) }\n\
         println(map_get(arms[1], \"pattern\") |> map_get(\"variant\"))\n\
         println(show(map_has_key(arms[1], \"guard\")))\n\
         println(map_get(map_get(t, \"type\"), \"text\") + \" \" + map_get(map_get(t, \"pattern\"), \"form\"))\n\
         println(map_get(head(map_get(map_get(t, \"value\"), \"parameters\")), \"type\") |> map_get(\"name\"))\n",
    );
    let (out, err, rc) = olang(&ws, &["run", "m.ol"]);
    assert_eq!(rc, 0, "{out}{err}");
    assert_eq!(
        out,
        "named Shape\nrecord title:String,n:[Int]\nFloat\nenum Circle(r)\nenum Rect(w, h)\nor 1 | 2\nwildcard _\nRect\ntrue\nTask ident\nInt\n"
    );
}

#[test]
fn an_alias_let_checks_the_aliased_annotation_on_every_tier_and_thread() {
    let ws = workspace("aliaslet");
    write(
        &ws,
        "lib/types.ol",
        "share type Task = { title: String, done: Bool }\n\
         share fn by_param(t: Task) = map_get(t, \"title\")\n\
         share fn by_let(m) = {\n    let t: Task = m\n    map_get(t, \"title\")\n}\n",
    );
    write(
        &ws,
        "a.ol",
        "use lib.types { Task, by_param, by_let }\n\
         let t = #{ \"title\": \"ship\", \"done\": false }\n\
         fn local_let(m) = { let x: Task = m; map_get(x, \"done\") }\n\
         fn outer(m, i) = by_let(m) + to_string(i)\n\
         let mut acc = \"\"\n\
         for i in range(0, 5) { acc = outer(t, i) }\n\
         println(acc)\n\
         println(show(task.join(spawn { by_let(t) + \"/\" + by_param(t) + \"/\" + show(local_let(t)) })))\n\
         unwrap(os.on_shutdown((why) => http.shutdown()))\n\
         let srv = spawn { http.serve(0, (req) => by_let(t) + \"/\" + show(local_let(t))) }\n\
         time.sleep(300)\n\
         http.shutdown()\n\
         println(show(task.join(srv)))\n",
    );
    for mode in [vec!["run"], vec!["--no-ovm", "run"]] {
        let mut args = mode.clone();
        args.push("a.ol");
        let (out, err, rc) = olang(&ws, &args);
        assert_eq!(rc, 0, "{mode:?}: {out}{err}");
        assert!(
            out.starts_with("ship4\nship/ship/false\n"),
            "{mode:?}: {out}{err}"
        );
        assert!(out.ends_with("Ok(())\n"), "{mode:?}: {out}{err}");
    }
}

#[test]
fn a_nested_packages_file_expands_against_its_own_root_under_olang_test() {
    let ws = workspace("nestedroot");
    // The app's root has a `lib/decl.ol` of its own, with a same-named macro.
    write(
        &ws,
        "olang.toml",
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n",
    );
    write(
        &ws,
        "lib/decl.ol",
        "meta fn resource(name) = \"#{ \\\"resource\\\": \\\"APP\\\" }\"\n",
    );
    write(
        &ws,
        "vendor/shuttle/olang.toml",
        "[package]\nname = \"shuttle\"\nversion = \"0.1.0\"\n",
    );
    write(
        &ws,
        "vendor/shuttle/lib/decl.ol",
        "fn quote(s) = \"\\\"\" + s + \"\\\"\"\n\
         meta fn resource(name) = \"#{ \\\"resource\\\": \" + quote(unwrap(meta.eval(name))) + \" }\"\n",
    );
    write(
        &ws,
        "vendor/shuttle/lib/sample.ol",
        "use lib.decl { resource }\n\
         share let SAMPLE = @resource(\"issues\")\n\
         share fn sample_name() = map_get(SAMPLE, \"resource\")\n\
         test \"expands against its own package\" { assert_eq(sample_name(), \"issues\") }\n",
    );
    // From the app's root, scanning everything: the nested file's macro is
    // the nested package's, not the app's.
    let (out, err, rc) = olang(&ws, &["test", "."]);
    assert_eq!(rc, 0, "{out}{err}");
    assert!(out.contains("1 passed, 0 failed"), "{out}{err}");
}

#[test]
fn meta_parse_emits_an_alias_and_struct_field_types_as_nodes() {
    let ws = workspace("aliasnodes");
    write(
        &ws,
        "n.ol",
        "let nodes = unwrap(meta.parse(\"type Task = { title: String, tags: [String] }\\ntype P = struct { x: Int }\"))\n\
         let t = map_get(nodes[0], \"type\")\n\
         println(map_get(t, \"form\") + \" \" + join(map(map_get(t, \"fields\"), (f) => map_get(f, \"name\") + \":\" + map_get(map_get(f, \"type\"), \"form\")), \",\"))\n\
         let f = head(map_get(nodes[1], \"fields\"))\n\
         println(map_get(f, \"type\") + \" \" + map_get(map_get(f, \"type_node\"), \"name\"))\n",
    );
    let (out, err, rc) = olang(&ws, &["run", "n.ol"]);
    assert_eq!(rc, 0, "{out}{err}");
    assert_eq!(out, "record title:basic,tags:list\nInt Int\n");
}
