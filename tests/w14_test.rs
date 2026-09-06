//! Roadmap W14 — the fourth reading of open-track: a second same-named
//! definition no longer slows every caller, a lib function reached inside
//! a task compiles as it does inline, later top-level bindings resolve in
//! tasks, `==` across types answers false, and `attempt(f)` turns a raise
//! into a Result.

use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("olang_w14_{}_{}", std::process::id(), tag));
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

fn ms(line: &str) -> f64 {
    line.split_whitespace()
        .find_map(|w| w.parse::<f64>().ok())
        .unwrap_or_else(|| panic!("no number in {line:?}"))
}

#[test]
fn a_second_same_named_definition_does_not_slow_every_caller() {
    let ws = workspace("dup");
    let pkg = ws
        .parent()
        .unwrap()
        .join(format!("dup_pkg_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&pkg);
    write(
        &pkg,
        "olang.toml",
        "[package]\nname = \"query\"\nversion = \"0.1.0\"\n",
    );
    write(
        &pkg,
        "index.ol",
        "share fn as_str(v) = if typeof(v) == \"String\" => v else => to_string(v)\n",
    );
    write(
        &ws,
        "olang.toml",
        &format!(
            "[package]\nname = \"dup\"\nversion = \"0.1.0\"\n\n[dependencies]\nquery = {{ path = \"{}\" }}\n",
            pkg.display()
        ),
    );
    write(
        &ws,
        "lib/checklist.ol",
        "use query { as_str }\n\
         share fn count(text) = {\n\
             let lines = str.lines(as_str(text))\n\
             let n = len(filter(lines, (l) => str.starts_with(str.trim(l), \"- [\")))\n\
             map_set(#{}, \"total\", n)\n\
         }\n",
    );
    write(
        &ws,
        "lib/memo.ol",
        "fn as_str(v) = to_string(v)\nshare fn memo(x) = x\n",
    );
    let body = "fn work() = fold(range(0, 4000), 0, (a, n) => a + map_get(checklist.count(\"- [ ] a\"), \"total\"))\n\
                let t0 = time.monotonic_ms()\n\
                let r = work()\n\
                println(to_string(time.monotonic_ms() - t0) + \" ms \" + to_string(r))\n";
    write(&ws, "without.ol", &format!("use lib.checklist\n{body}"));
    write(
        &ws,
        "with.ol",
        &format!("use lib.memo\nuse lib.checklist\n{body}"),
    );
    let (a, err, rc) = olang(&ws, &["run", "without.ol"]);
    assert_eq!(rc, 0, "{a}{err}");
    let (b, err, rc) = olang(&ws, &["run", "with.ol"]);
    assert_eq!(rc, 0, "{b}{err}");
    assert!(a.contains(" 4000") && b.contains(" 4000"), "{a}{b}");
    // The report was 80×. Same work, second definition present or not.
    assert!(ms(&b) <= ms(&a).max(4.0) * 4.0, "without: {a} with: {b}");
    let _ = std::fs::remove_dir_all(&pkg);
}

#[test]
fn a_lib_function_reached_in_a_task_costs_what_it_costs_inline() {
    let ws = workspace("taskcall");
    write(
        &ws,
        "olang.toml",
        "[package]\nname = \"taskcall\"\nversion = \"0.1.0\"\n",
    );
    write(
        &ws,
        "lib/app.ol",
        "share fn as_str(v) = if typeof(v) == \"String\" => v else => to_string(v)\n",
    );
    write(
        &ws,
        "lib/checklist.ol",
        "use lib.app { as_str }\n\
         share fn count(issue) = {\n\
             let desc = as_str(map_get(issue, \"description\"))\n\
             len(filter(str.lines(desc), (l) => str.starts_with(str.trim(l), \"- [\")))\n\
         }\n",
    );
    write(
        &ws,
        "main.ol",
        "use lib.app { as_str }\n\
         use lib.checklist\n\
         let issue = unwrap(json.parse(\"{\\\"description\\\": \\\"- [ ] a\\\"}\"))\n\
         fn in_task(f) = {\n\
             let c = chan.new()\n\
             let t = spawn { chan.send(c, f()) }\n\
             chan.recv(c)\n\
         }\n\
         fn qualified() = {\n\
             let t0 = time.monotonic_ms()\n\
             let mut acc = 0\n\
             for i in range(0, 20000) { acc = acc + checklist.count(issue) }\n\
             time.monotonic_ms() - t0\n\
         }\n\
         fn by_name() = {\n\
             let t0 = time.monotonic_ms()\n\
             let mut acc = 0\n\
             for i in range(0, 20000) { acc = acc + str.length(as_str(i)) }\n\
             time.monotonic_ms() - t0\n\
         }\n\
         println(to_string(qualified()) + \" \" + to_string(unwrap(in_task(() => qualified()))))\n\
         println(to_string(by_name()) + \" \" + to_string(unwrap(in_task(() => by_name()))))\n",
    );
    let (out, err, rc) = olang(&ws, &["run", "main.ol"]);
    assert_eq!(rc, 0, "{out}{err}");
    for line in out.lines() {
        let nums: Vec<f64> = line.split(' ').map(|n| n.parse().unwrap()).collect();
        // The report was 50× (later 10×); a task pays what the main thread
        // pays: within 1.5×, with two milliseconds of timer slack.
        assert!(
            nums[1] <= nums[0].max(5.0) * 1.5 + 2.0,
            "inline {} ms vs task {} ms",
            nums[0],
            nums[1]
        );
    }
}

#[test]
fn a_function_naming_a_later_top_level_binding_resolves_in_a_task() {
    let ws = workspace("later");
    write(
        &ws,
        "lib/m.ol",
        "share fn digest(x) = fmt_minutes(x) + 1\n\nshare fn helper() = 0\n\nshare fn fmt_minutes(x) = x * 2\n",
    );
    write(
        &ws,
        "main.ol",
        "use lib.m { digest }\n\
         println(to_string(digest(1)))\n\
         let t = spawn { digest(2) }\n\
         println(show(task.join(t)))\n\
         fn in_task(f) = {\n\
             let c = chan.new()\n\
             let t2 = spawn { chan.send(c, f()) }\n\
             chan.recv(c)\n\
         }\n\
         println(show(in_task(() => digest(3))))\n\
         let handlers = [(req) => digest(4)]\n\
         let t3 = spawn { handlers[0](()) }\n\
         println(show(task.join(t3)))\n",
    );
    for flags in [vec!["run", "main.ol"], vec!["--no-ovm", "run", "main.ol"]] {
        let (out, err, rc) = olang(&ws, &flags);
        assert_eq!(rc, 0, "{flags:?}: {out}{err}");
        assert_eq!(out, "3\n5\nOk(7)\n9\n", "{flags:?}");
    }
}

#[test]
fn equality_across_types_answers_instead_of_raising() {
    let ws = workspace("eq");
    write(
        &ws,
        "e.ol",
        "fn same(a, b) = a == b\n\
         fn differ(a, b) = a != b\n\
         let mut warm = 0\n\
         for i in range(0, 50) { if same(i, i) => { warm = warm + 1 } else => () }\n\
         println(show(same(\"yes\", true)))\n\
         println(show(same(4, \"4\")))\n\
         println(show(same(1, 1.0)))\n\
         println(show(same((), false)))\n\
         println(show(differ([1], \"x\")))\n\
         println(show(same(#{ \"a\": 1 }, [1])))\n",
    );
    for flags in [vec!["run", "e.ol"], vec!["--no-ovm", "run", "e.ol"]] {
        let (out, err, rc) = olang(&ws, &flags);
        assert_eq!(rc, 0, "{flags:?}: {out}{err}");
        assert_eq!(out, "false\nfalse\ntrue\nfalse\ntrue\nfalse\n", "{flags:?}");
    }
    write(
        &ws,
        "c.ol",
        "let s = \"yes\"\nlet b = true\nprintln(show(s == b))\nprintln(show(1 == 2.0))\n",
    );
    let (out, err, _) = olang(&ws, &["check", "c.ol"]);
    let all = out + &err;
    assert!(
        all.contains("`==` between String and Bool is always false"),
        "{all}"
    );
    assert!(!all.contains("Int and Float"), "{all}");
}

#[test]
fn attempt_turns_a_raise_into_a_result_and_passes_control_flow_through() {
    let ws = workspace("attempt");
    write(
        &ws,
        "a.ol",
        "println(show(attempt(() => map_get(5, \"k\"))))\n\
         println(show(attempt(() => 41 + 1)))\n\
         println(show(attempt(() => Err(\"already a result\"))))\n\
         fn guarded(xs) = {\n\
             for x in xs {\n\
                 let r = attempt(() => { if x == 2 => unwrap(Err(\"two\")) else => x * 10 })\n\
                 match r { Ok(v) => println(\"ok \" + to_string(v)), Err(e) => println(\"caught \" + e) }\n\
             }\n\
             \"done\"\n\
         }\n\
         println(guarded([1, 2, 3]))\n\
         // A return inside the attempted function is control flow, not a failure.\n\
         fn early() = { attempt(() => { return 7 }) }\n\
         println(show(early()))\n",
    );
    for flags in [vec!["run", "a.ol"], vec!["--no-ovm", "run", "a.ol"]] {
        let (out, err, rc) = olang(&ws, &flags);
        assert_eq!(rc, 0, "{flags:?}: {out}{err}");
        assert!(out.starts_with("Err(\"map_get: first argument must be a map or object\")\nOk(42)\nOk(Err(\"already a result\"))\nok 10\ncaught Unwrap failed on error: \"two\"\nok 30\ndone\n"), "{flags:?}: {out}");
    }
}
