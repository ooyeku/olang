//! Roadmap W21 — what open-track's build-out document asked for: a map
//! that crosses the tiers in O(1), a tier that refuses less and says
//! what it refused, and the editor rows of 2026-09-20.

use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("olang_w21_{}_{}", std::process::id(), tag));
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

fn olang_env(dir: &Path, args: &[&str], env: &[(&str, &str)]) -> (String, String, i32) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_olang"));
    cmd.args(args).current_dir(dir);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("run olang");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
        out.status.code().unwrap_or(-1),
    )
}

fn olang(dir: &Path, args: &[&str]) -> (String, String, i32) {
    olang_env(dir, args, &[])
}

const STORE: &str = "fn row(i) = fold(0..40, #{ \"id\": i }, (m, k) => map_set(m, \"f\" + to_string(k), k + i))\n\
fn store() = {\n\
    let base = fold(0..200, #{}, (m, k) => map_set(m, \"k\" + to_string(k), k))\n\
    let a = map_set(map_set(base, \"rows\", map(0..100, (i) => row(i))), \"count\", 0)\n\
    map_set(map_set(a, \"recent\", map(0..8, (i) => row(i))), \"by_id\", fold(0..60, #{}, (m, i) => map_set(m, \"id\" + to_string(i), row(i))))\n\
}\n";

#[test]
fn a_store_in_a_cell_costs_the_same_on_the_tier_as_off_it() {
    // The message loop the document measured: the store in a cell, a
    // compiled helper reading one key, `typeof` on the store, a write
    // back. 0.85.0: 1,225 ms with the tier against 27 without.
    let ws = workspace("cellloop");
    let program = format!(
        "{STORE}\
         let state = cell.new(store())\n\
         fn s_get(model, key, fallback) = if map_has_key(model, key) => map_get(model, key) else => fallback\n\
         fn deliver(msg) = {{\n\
             let m = cell.get(state)\n\
             let n = s_get(m, \"count\", 0) + map_get(msg, \"by\")\n\
             let t = typeof(m)\n\
             cell.set(state, map_set(m, \"count\", n))\n\
         }}\n\
         let t0 = time.monotonic_ms()\n\
         let mut i = 0\n\
         while i < 2000 {{ deliver(#{{ \"by\": 1 }}); i = i + 1 }}\n\
         println(to_string(map_get(cell.get(state), \"count\")))\n\
         println(unwrap(json.stringify(map_get(map_get(cell.get(state), \"by_id\"), \"id1\"))) == unwrap(json.stringify(row(1))))\n\
         println(\"MS \" + to_string(time.monotonic_ms() - t0))\n"
    );
    write(&ws, "loop.ol", &program);
    let ms = |args: &[&str]| -> i64 {
        let (out, err, rc) = olang(&ws, args);
        assert_eq!(rc, 0, "{out}{err}");
        let mut lines = out.lines();
        assert_eq!(lines.next(), Some("2000"), "{out}");
        assert_eq!(lines.next(), Some("true"), "{out}");
        lines
            .next()
            .and_then(|l| l.strip_prefix("MS "))
            .and_then(|n| n.parse().ok())
            .expect("a timing line")
    };
    let on = ms(&["run", "loop.ol"]);
    let off = ms(&["--no-ovm", "run", "loop.ol"]);
    // Within 1.5× (it was 45×); the floor keeps a fast machine's
    // single-digit timings from failing on noise.
    assert!(
        on <= (off * 3 / 2).max(off + 60),
        "tier on {on} ms against {off} ms off"
    );
}

#[test]
fn ordinary_code_compiles_and_a_refusal_no_longer_takes_its_callers() {
    let ws = workspace("refusals");
    write(
        &ws,
        "olang.toml",
        "[package]\nname = \"t\"\nversion = \"0.1.0\"\n",
    );
    write(
        &ws,
        "lib/other.ol",
        "share fn other(v) = str_of(v)\nfn str_of(v) = \"o\" + to_string(v)\n",
    );
    write(
        &ws,
        "lib/weft.ol",
        "use lib.other { other }\n\
         share fn read(m, k) = get_or(m, k, \"none\")\n\
         fn get_or(m, k, fallback) = if map_has_key(m, k) => map_get(m, k) else => fallback\n\
         share fn uses_attempt(f) = match attempt(f) { Ok(v) => v, Err(e) => \"raised\" }\n\
         share fn uses_drop(xs) = drop(xs, 1)\n\
         share fn uses_map_path(m) = map_path(m, [\"a\", \"b\"])\n\
         share fn in_lambda(xs) = map(xs, (x) => str_of(x))\n\
         fn str_of(v) = if typeof(v) == \"String\" => v else => to_string(v)\n",
    );
    write(
        &ws,
        "main.ol",
        "use lib.weft { read, uses_attempt, uses_drop, uses_map_path, in_lambda }\n\
         fn uncompilable(x) = { let t = spawn { x + 1 }; task.join(t) }\n\
         fn caller(x) = uncompilable(x) * 2\n\
         fn dispatch(n) = a0(n) + a1(n) + a2(n) + a3(n)\n\
         fn a0(n) = n\nfn a1(n) = n + 1\nfn a2(n) = n + 2\nfn a3(n) = n + 3\n\
         let m = #{ \"a\": #{ \"b\": 7 }, \"k\": \"v\" }\n\
         let mut acc = 0\n\
         for i in range(0, 60) {\n\
             acc = acc + len(read(m, \"k\")) + len(uses_drop([1, 2, 3])) + uses_map_path(m) + len(in_lambda([1, 2]))\n\
             if uses_attempt(() => i) != i => { acc = acc - 1000 } else => ()\n\
         }\n\
         println(to_string(acc))\n\
         for i in range(0, 40) { acc = caller(i) + dispatch(i) }\n\
         println(to_string(acc))\n",
    );
    let (tiered, err, rc) = olang_env(&ws, &["run", "main.ol"], &[("OLANG_TIER_STATS", "1")]);
    assert_eq!(rc, 0, "{tiered}{err}");
    let (plain, _, _) = olang(&ws, &["--no-ovm", "run", "main.ol"]);
    assert_eq!(tiered, plain);
    // Each shape the document found refused now compiles.
    for name in [
        "read",
        "get_or",
        "uses_attempt",
        "uses_drop",
        "uses_map_path",
        "in_lambda",
        "caller",
        "dispatch",
    ] {
        assert!(
            err.contains(&format!("tier-fn: {name} ")),
            "{name} did not reach the tier:\n{err}"
        );
    }
    // The one real refusal is reported with its reason — and did not
    // take `caller` with it.
    assert!(err.contains("tier-refused: uncompilable: "), "{err}");
    assert!(err.contains("`spawn`"), "{err}");
    assert!(!err.contains("tier-refused: caller"), "{err}");
}

#[test]
fn check_tier_lists_refusals_ahead_of_time() {
    let ws = workspace("checktier");
    write(
        &ws,
        "olang.toml",
        "[package]\nname = \"t\"\nversion = \"0.1.0\"\n",
    );
    write(
        &ws,
        "lib/a.ol",
        "share fn fine(x) = helper(x) + 1\n\
         fn helper(x) = x * 2\n\
         share fn starts(x) = { let t = spawn { x }; task.join(t) }\n\
         share fn leaks(x) = never_imported(x)\n",
    );
    let (out, err, rc) = olang(&ws, &["check", "--tier", "lib"]);
    assert_eq!(rc, 0, "{out}{err}");
    assert!(out.contains("`starts` stays on the tree-walker"), "{out}");
    assert!(out.contains("`spawn`"), "{out}");
    assert!(out.contains("`leaks` stays on the tree-walker"), "{out}");
    assert!(out.contains("nothing in its scope defines"), "{out}");
    assert!(!out.contains("`fine`"), "{out}");
    assert!(out.contains("2 functions refused"), "{out}");
    // Promoted, a refusal fails the check.
    write(
        &ws,
        "olang.toml",
        "[package]\nname = \"t\"\nversion = \"0.1.0\"\n\n[check]\npromote = [\"tier\"]\n",
    );
    let (out, _, rc) = olang(&ws, &["check", "--tier", "lib"]);
    assert_eq!(rc, 1, "{out}");
    // A clean file says so.
    write(
        &ws,
        "lib/a.ol",
        "share fn fine(x) = helper(x) + 1\nfn helper(x) = x * 2\n",
    );
    let (out, _, rc) = olang(&ws, &["check", "--tier", "lib"]);
    assert_eq!(rc, 0, "{out}");
    assert!(out.contains("every function compiles"), "{out}");
}

#[test]
fn the_checker_resolves_imports_from_the_package_root_and_flags_shape_writes() {
    let ws = workspace("pkgroot");
    write(
        &ws,
        "olang.toml",
        "[package]\nname = \"ck\"\nversion = \"0.1.0\"\n",
    );
    write(
        &ws,
        "lib/app.ol",
        "share type Task = { title: String }\nshare fn two(a, b) = a + b\n",
    );
    write(
        &ws,
        "lib/client/view.ol",
        "use lib.app { Task, two }\n\
         share fn render(t: Task) = map_set(t, \"titel\", two(1))\n",
    );
    // From the nested directory, and from the root: the import resolves
    // against the package, so the arity and the shape are checked.
    for (dir, target) in [(ws.join("lib/client"), "view.ol"), (ws.clone(), ".")] {
        let (out, err, _) = olang(&dir, &["check", target]);
        let all = out + &err;
        assert!(all.contains("Missing required argument: b"), "{all}");
        assert!(
            all.contains("`titel` is not a key of the declared"),
            "{all}"
        );
        assert!(all.contains("map_set adds"), "{all}");
    }
}

// --- Item 3: what a declared function costs ---

/// `n` one-line functions, each calling the one before it, as a module.
fn many_functions(n: usize) -> String {
    let mut src = String::from("share fn f0(x) = x\n");
    for i in 1..n {
        src.push_str(&format!("share fn f{i}(x) = f{}(x) + 1\n", i - 1));
    }
    src
}

fn field(json: &str, key: &str) -> i64 {
    let at = json
        .find(&format!("\"{key}\":"))
        .unwrap_or_else(|| panic!("no {key} in {json}"));
    json[at + key.len() + 3..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .unwrap_or_else(|_| panic!("{key} is not a number in {json}"))
}

#[test]
fn a_declared_function_costs_kilobytes_not_tens_of_them() {
    // 0.85.0 held a version of the scope map per declaration: 32 KB a
    // function, 137 MB resident for a file of 4,000. A run of
    // declarations now shares one closure, and `runtime.memory()` is how
    // a program sees the result.
    let ws = workspace("fnmem");
    write(&ws, "lib/many.ol", &many_functions(2000));
    write(
        &ws,
        "main.ol",
        "use lib.many { f1999 }\n\
         println(to_string(f1999(0)))\n\
         println(json.stringify(runtime.memory()))\n",
    );
    for flags in [&["main.ol"][..], &["--no-ovm", "main.ol"][..]] {
        let (out, err, code) = olang(&ws, flags);
        assert_eq!(code, 0, "{flags:?}: {err}");
        let mut lines = out.lines();
        assert_eq!(
            lines.next(),
            Some("1999"),
            "{flags:?}: every sibling resolves"
        );
        let memory = lines.next().expect("the memory line");
        let program = field(memory, "program");
        let per_function = program / 2000;
        assert!(
            per_function < 8 * 1024,
            "{flags:?}: {per_function} bytes a function ({program} for 2,000)"
        );
        let heap = field(memory, "heap");
        assert_eq!(heap, program + field(memory, "values"), "{memory}");
    }
}

#[test]
fn a_shared_closure_keeps_what_a_snapshot_per_declaration_meant() {
    // The three things the per-declaration snapshot guaranteed, in both
    // tiers, on the main thread, in a task, and through a lambda: an
    // earlier sibling is the one that was declared THEN (a later
    // redefinition does not reach back), a sibling shadows a builtin of
    // its name, and a binding between two declarations is seen by the
    // second only.
    let ws = workspace("runs");
    write(
        &ws,
        "main.ol",
        "fn helper(x) = x + 1\n\
         fn go(xs) = xs |> map(helper) |> sum\n\
         fn via_lambda(xs) = map(xs, (x) => helper(x)) |> sum\n\
         fn clamp(n) = n + 1000\n\
         fn use_it(n) = clamp(n)\n\
         fn before_k() = if map_has_key(#{}, \"k\") => 0 else => 7\n\
         let k = 5\n\
         fn after_k() = k\n\
         let xs = [10, 20, 30]\n\
         let first = go(xs)\n\
         fn helper(x) = x + 1000\n\
         fn late(xs) = xs |> map(helper) |> sum\n\
         println(to_string(first) + \" \" + to_string(go(xs)) + \" \" + to_string(via_lambda(xs)))\n\
         println(to_string(late(xs)))\n\
         println(to_string(use_it(1)))\n\
         println(to_string(before_k() + after_k()))\n\
         println(to_string(task.join(spawn go(xs))))\n",
    );
    for flags in [&["main.ol"][..], &["--no-ovm", "main.ol"][..]] {
        let (out, err, code) = olang(&ws, flags);
        assert_eq!(code, 0, "{flags:?}: {err}");
        assert_eq!(out, "63 63 63\n3060\n1001\n12\n63\n", "{flags:?}");
    }
}

#[test]
fn memory_names_its_tasks_and_the_embedded_runtime_costs_no_heap() {
    let ws = workspace("memory");
    write(
        &ws,
        "main.ol",
        "let before = map_get(runtime.memory(), \"heap\")\n\
         let rt = runtime.wasm()\n\
         let after = map_get(runtime.memory(), \"heap\")\n\
         println(to_string(after - before < 262144))\n\
         let t = spawn { let xs = map(0..50000, (i) => i * 2); time.sleep(300); len(xs) }\n\
         time.sleep(120)\n\
         let m = runtime.memory()\n\
         let tasks = map_get(m, \"tasks\")\n\
         println(to_string(len(tasks)))\n\
         println(map_get(tasks[0], \"name\"))\n\
         println(to_string(map_get(tasks[0], \"bytes\") > 1000000))\n\
         println(to_string(task.join(t)))\n\
         println(to_string(len(map_get(runtime.memory(), \"tasks\"))))\n",
    );
    let (out, err, code) = olang(&ws, &["main.ol"]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "true\n1\nolang-spawn-1\ntrue\n50000\n0\n", "{err}");
}

// --- Item 4: repaints, counted and cheap to keep ---

const REPAINT_CLIENT: &str = r##"use web { mount, apply, action, paint_stats, unwatch, memo, memo_list, div, button, ul, li, el, text }

fn view(s) = div(#{}, [
    button(#{ "data-action": "inc" }, ["+"]),
    memo("panel", [map_get(s, "n")], () => div(#{ "class": "panel" }, [to_string(map_get(s, "n"))])),
    ul(#{}, memo_list("rows", map_get(s, "rows"), (r) => map_get(r, "id"), (r) => r,
        (r) => li(#{}, [map_get(r, "t")]))),
    if map_get(s, "open") => memo("drawer", [1], () => el("aside", #{}, ["drawer"])) else => text("")
])

unwatch(["live"])
action("inc", (ev) => apply((s) => map_set(s, "n", map_get(s, "n") + 1)))
action("idle", (ev) => ())
action("beat", (ev) => apply((s) => map_set(s, "live", map_get(s, "live") + 1)))
action("retitle", (ev) => apply((s) => map_set(s, "rows",
    map(map_get(s, "rows"), (r) => if map_get(r, "id") == 2 => map_set(r, "t", "renamed") else => r))))
action("toggle", (ev) => apply((s) => map_set(s, "open", !map_get(s, "open"))))
action("stats", (ev) => dom.set_text(dom.query("#log"), unwrap(json.stringify(paint_stats()))))

mount("#app", view, #{ "n": 0, "live": 0, "open": false,
    "rows": [#{ "id": 1, "t": "one" }, #{ "id": 2, "t": "two" }, #{ "id": 3, "t": "three" }] })
"##;

#[test]
fn the_harness_pins_repaint_counts_against_the_sdk() {
    // One repaint for a handled action, none for a handler that changes
    // nothing or a change no view reads, a memo_list that keeps the rows
    // that did not move, and a stale `keep` that heals in one extra pass
    // — counted where the patches arrive, in the dom harness, over a
    // client bundled as `serve` bundles it. Skips where node or the wasm
    // artifact is absent.
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let wasm = root.join("target/wasm32-unknown-unknown/release/olang_playground.wasm");
    if !wasm.exists() || Command::new("node").arg("--version").output().is_err() {
        eprintln!("skipping: needs node and the wasm artifact (cargo xtask wasm)");
        return;
    }
    let ws = workspace("repaints");
    write(&ws, "client.ol", REPAINT_CLIENT);
    write(
        &ws,
        "bundle.ol",
        "use lib.server { bundle_client }\n\
         let dir = os.args()[1]\n\
         unwrap(fs.write_file(dir + \"/bundle.ol\", bundle_client(unwrap(fs.read_file(dir + \"/client.ol\")))))\n",
    );
    let script = ws.join("bundle.ol").to_string_lossy().to_string();
    let dir = ws.to_string_lossy().to_string();
    let (out, err, rc) = olang(&root.join("frameworks/web-sdk"), &["run", &script, &dir]);
    assert_eq!(rc, 0, "{out}{err}");
    let run = Command::new("node")
        .arg(root.join("playground/dom_harness.mjs"))
        .arg(&wasm)
        .arg("--repaints")
        .arg(ws.join("bundle.ol"))
        .output()
        .expect("node");
    let text = String::from_utf8_lossy(&run.stdout);
    assert!(
        run.status.success(),
        "{text}{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let stats: serde_json::Value = serde_json::from_str(text.lines().last().unwrap_or(""))
        .unwrap_or_else(|_| panic!("{text}"));
    // mount, inc, retitle, three toggles: six asked for; one healed.
    assert_eq!(stats["repaints"], 6, "{stats}");
    assert_eq!(stats["healed"], 1, "{stats}");
    assert_eq!(stats["skipped"], 1, "{stats}");
    assert!(stats["nodes"].as_i64().unwrap() > 0, "{stats}");
    assert!(stats["memo_hits"].as_i64().unwrap() >= 4, "{stats}");
    assert!(stats["last"]["nodes"].as_i64().unwrap() > 0, "{stats}");
}

// --- Item 6: the smaller gaps ---

#[test]
fn the_test_runner_filters_times_and_keeps_a_named_file_to_its_own_blocks() {
    let ws = workspace("runner");
    write(
        &ws,
        "olang.toml",
        "[package]\nname = \"runner\"\nversion = \"0.1.0\"\n",
    );
    write(
        &ws,
        "lib/helper.ol",
        "share fn twice(n) = n * 2\n\
         test \"helper doubles\" { assert_eq(twice(2), 4) }\n",
    );
    write(
        &ws,
        "tests/app.ol",
        "use lib.helper { twice }\n\
         test \"app uses the helper\" { assert_eq(twice(3), 6) }\n\
         test \"app rate limit\" { assert_eq(1, 1) }\n",
    );
    // A named file: its own two blocks, not the helper's.
    let (out, err, code) = olang(&ws, &["test", "tests/app.ol"]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(out.contains("2 passed, 0 failed"), "{out}");
    assert!(!out.contains("helper doubles"), "{out}");
    // A directory: every file's blocks, the helper's once.
    let (out, _, code) = olang(&ws, &["test", "."]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("3 passed, 0 failed"), "{out}");
    // --only: one block, and only its file is named.
    let (out, _, code) = olang(&ws, &["test", ".", "--only", "rate limit"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("1 passed, 0 failed"), "{out}");
    assert!(
        out.contains("app rate limit") && !out.contains("helper.ol"),
        "{out}"
    );
    let (out, _, _) = olang(&ws, &["test", ".", "--only", "no such block"]);
    assert!(out.contains("no test block's name contains"), "{out}");
    // --times: milliseconds beside each block and the slowest at the end.
    let (out, _, code) = olang(&ws, &["test", ".", "--times"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains(" ms") && out.contains("slowest:"), "{out}");
}

#[test]
fn a_local_named_like_a_module_is_an_error_when_shadow_is_promoted() {
    let ws = workspace("shadowclass");
    write(
        &ws,
        "olang.toml",
        "[package]\nname = \"shadowclass\"\nversion = \"0.1.0\"\n\n[check]\npromote = [\"shadow\"]\n",
    );
    write(
        &ws,
        "main.ol",
        "fn f() = {\n    let cell = 1\n    cell + 1\n}\nprintln(to_string(f()))\n",
    );
    let (out, err, code) = olang(&ws, &["check", "main.ol"]);
    let all = format!("{out}{err}");
    assert_ne!(code, 0, "{all}");
    assert!(all.contains("shadows the stdlib module"), "{all}");
    assert!(all.contains("promoted to an error"), "{all}");
}

#[test]
fn file_info_answers_the_modification_time_without_reading_the_file() {
    let ws = workspace("fileinfo");
    write(&ws, "data.txt", "hello");
    write(
        &ws,
        "main.ol",
        "let info = unwrap(fs.file_info(\"data.txt\"))\n\
         let age = time.now_ms() - info.modified_ms\n\
         println(to_string(info.size) + \" \" + to_string(age >= 0 && age < 600000))\n\
         println(to_string(unwrap(fs.file_size(\"data.txt\"))))\n",
    );
    let (out, err, code) = olang(&ws, &["main.ol"]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "5 true\n5\n", "{err}");
}

#[test]
fn transactions_on_a_shared_connection_do_not_interleave() {
    // Eight tasks, one connection, each transaction a read-modify-write
    // with a pause between the read and the write. Interleaved — as
    // 0.85.0 ran them — a second BEGIN failed inside the first's
    // transaction or an update was lost; serialized, every one lands.
    let ws = workspace("txlock");
    write(
        &ws,
        "main.ol",
        "let conn = unwrap(db.open(\":memory:\"))\n\
         unwrap(db.execute(conn, \"CREATE TABLE c (n INTEGER)\", []))\n\
         unwrap(db.execute(conn, \"INSERT INTO c VALUES (0)\", []))\n\
         fn bump(i) = db.transaction(conn, (c) => {\n\
             let n = map_get(unwrap(db.query(c, \"SELECT n FROM c\", []))[0], \"n\")\n\
             time.sleep(5)\n\
             db.execute(c, \"UPDATE c SET n = ?\", [n + 1])\n\
         })\n\
         let tasks = map(range(0, 8), (i) => spawn bump(i))\n\
         let failed = filter(map(tasks, (t) => task.join(t)), (r) => match r { Err(e) => true, _ => false })\n\
         println(to_string(len(failed)) + \" \" + to_string(map_get(unwrap(db.query(conn, \"SELECT n FROM c\", []))[0], \"n\")))\n",
    );
    let (out, err, code) = olang(&ws, &["main.ol"]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "0 8\n", "{err}");
}

#[test]
fn meta_eval_inside_a_meta_fn_sees_the_functions_the_expansion_sees() {
    // A meta fn could call an imported shared function directly and not
    // through `meta.eval`, which evaluated in an empty world. The child
    // is still pure: functions only, meta mode, no effects.
    let ws = workspace("metaeval");
    write(
        &ws,
        "olang.toml",
        "[package]\nname = \"metaeval\"\nversion = \"0.1.0\"\n",
    );
    write(
        &ws,
        "lib/names.ol",
        "share fn label_of(n) = \"item-\" + to_string(n)\n",
    );
    write(
        &ws,
        "main.ol",
        "use lib.names { label_of }\n\
         fn local_helper(n) = n * 10\n\
         meta fn labels(n) = {\n\
             let direct = label_of(2)\n\
             let through_eval = unwrap(meta.eval(\"label_of(3)\"))\n\
             let local = unwrap(meta.eval(\"local_helper(4)\"))\n\
             let effect = meta.eval(\"fs.read_file(\\\"main.ol\\\")\")\n\
             let refused = match effect { Err(e) => true, _ => false }\n\
             meta.lit([direct, through_eval, local, refused])\n\
         }\n\
         println(show(@labels(1)))\n",
    );
    let (out, err, code) = olang(&ws, &["main.ol"]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "[\"item-2\", \"item-3\", 40, true]\n", "{err}");
}

#[test]
fn the_allocation_counter_does_not_serialize_parallel_workers() {
    // `runtime.memory()`'s counter once had one slot for every thread
    // that claimed none, so the workers of a `par_map` wrote the same
    // cache line on every allocation: eight blocks took 1,987 ms in
    // parallel against 2,170 alone on the interpreter, and an example ran
    // for two minutes. Each thread now has its own; the same blocks take
    // 573 ms. The bound is loose (three quarters of the sequential time,
    // best of two) and needs four cores to mean anything.
    if std::thread::available_parallelism().map_or(1, |n| n.get()) < 4 {
        eprintln!("skipping: needs four cores");
        return;
    }
    let ws = workspace("parcount");
    write(
        &ws,
        "main.ol",
        "fn work(block) = {\n\
             let mut count = 0\n\
             let mut n = block * 4000 + 2\n\
             let hi = n + 4000\n\
             while n < hi {\n\
                 let mut d = 2\n\
                 let mut prime = true\n\
                 while d * d <= n { if n % d == 0 => { prime = false; break }; d = d + 1 }\n\
                 if prime => { count = count + 1 }\n\
                 n = n + 1\n\
             }\n\
             count\n\
         }\n\
         let blocks = range(0, 8)\n\
         let mut best = 1000.0\n\
         for attempt in range(0, 2) {\n\
             let t0 = time.monotonic_ms()\n\
             let a = map(blocks, work)\n\
             let t1 = time.monotonic_ms()\n\
             let b = par_map(blocks, work)\n\
             let t2 = time.monotonic_ms()\n\
             let ratio = to_float(t2 - t1) / to_float(max(t1 - t0, 1))\n\
             if a == b && ratio < best => { best = ratio }\n\
         }\n\
         println(to_string(best < 0.75) + \" \" + to_string(best))\n",
    );
    let (out, err, code) = olang(&ws, &["--no-ovm", "main.ol"]);
    assert_eq!(code, 0, "{err}");
    assert!(
        out.starts_with("true"),
        "par_map / map on the interpreter: {out}{err}"
    );
}
