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
