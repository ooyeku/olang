//! Roadmap W15 — closing the gap: a frame resolves names in its own
//! lexical scopes and its module's table, never in the caller's frame; a
//! drained `serve` answers `Ok(())`; `compress` exists; and the whole
//! program can run on a task (`--in-task`) to pin a task's speed.

use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("olang_w15_{}_{}", std::process::id(), tag));
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

const MODES: [&[&str]; 3] = [&["run"], &["--no-ovm", "run"], &["--in-task", "run"]];

fn in_every_mode(dir: &Path, file: &str, expect: &str) {
    for mode in MODES {
        let mut args: Vec<&str> = mode.to_vec();
        args.push(file);
        let (out, err, rc) = olang(dir, &args);
        assert_eq!(rc, 0, "{mode:?}: {out}{err}");
        assert_eq!(out, expect, "{mode:?}");
    }
}

#[test]
fn a_name_a_task_cannot_resolve_never_lands_on_the_callers_like_named_function() {
    // The open-track shape: the app's `head_for` is declared after the
    // function that names it; the SDK's `serve` has its own local
    // `head_for` that calls the app's head function on a worker task.
    // The app's name must reach the app's global, on every tier.
    let ws = workspace("callerframe");
    write(
        &ws,
        "olang.toml",
        "[package]\nname = \"callerframe\"\nversion = \"0.1.0\"\n",
    );
    write(
        &ws,
        "lib/sdk.ol",
        "share fn serve(head) = {\n\
             fn head_for(req) = head(req)\n\
             let t = spawn { head_for(0) }\n\
             show(task.join(t))\n\
         }\n\
         share fn serve_inline(head) = {\n\
             fn head_for(req) = head(req)\n\
             head_for(0)\n\
         }\n",
    );
    write(
        &ws,
        "main.ol",
        "use lib.sdk { serve, serve_inline }\n\
         fn page_head(req) = head_for(1)\n\
         fn head_for(n) = \"app \" + to_string(n)\n\
         println(serve_inline(page_head))\n\
         println(serve(page_head))\n",
    );
    in_every_mode(&ws, "main.ol", "app 1\napp 1\n");

    // The same shape in one file: the like-named local is a sibling.
    write(
        &ws,
        "one.ol",
        "fn page_head(req) = head_for(1)\n\
         fn head_for(n) = \"app \" + to_string(n)\n\
         fn serve(head) = {\n\
             fn head_for(req) = head(req)\n\
             let t = spawn { head_for(0) }\n\
             show(task.join(t))\n\
         }\n\
         println(serve(page_head))\n",
    );
    in_every_mode(&ws, "one.ol", "app 1\n");

    // A name nothing binds is an error that names it, not a lookup in
    // whatever frame made the call.
    write(
        &ws,
        "missing.ol",
        "use lib.sdk { serve }\n\
         fn page_head(req) = missing_name(1)\n\
         println(serve(page_head))\n",
    );
    for mode in [&["run"][..], &["--no-ovm", "run"][..]] {
        let mut args = mode.to_vec();
        args.push("missing.ol");
        let (out, err, _) = olang(&ws, &args);
        assert!(
            out.contains("Undefined variable: missing_name"),
            "{mode:?}: {out}{err}"
        );
    }
}

#[test]
fn a_nested_functions_parameter_named_like_a_builtin_reaches_the_parameter_on_a_task() {
    // `head` is a list builtin; as a parameter of the enclosing function
    // it is what a nested `fn` means by the name — on the main thread, on
    // a task, compiled or not.
    let ws = workspace("nestedparam");
    write(
        &ws,
        "p.ol",
        "fn outer(head) = {\n\
             fn inner(r) = head(r)\n\
             let t = spawn { inner(0) }\n\
             show(task.join(t))\n\
         }\n\
         fn outer_hot(head) = {\n\
             fn inner(r) = head(r)\n\
             inner(0)\n\
         }\n\
         fn page(r) = \"plain \" + to_string(r)\n\
         println(outer(page))\n\
         let mut n = 0\n\
         for i in range(0, 40) { if outer_hot(page) == \"plain 0\" => { n = n + 1 } else => () }\n\
         println(to_string(n))\n",
    );
    in_every_mode(&ws, "p.ol", "plain 0\n40\n");
}

#[test]
fn siblings_forward_references_and_later_globals_resolve_lexically_everywhere() {
    let ws = workspace("lexical");
    write(
        &ws,
        "olang.toml",
        "[package]\nname = \"lexical\"\nversion = \"0.1.0\"\n",
    );
    write(
        &ws,
        "lib/m.ol",
        "share fn a(n) = if n == 0 => \"a\" else => b(n - 1)\n\
         fn b(n) = if n == 0 => \"b\" else => a(n - 1)\n\
         share fn outer() = {\n\
             fn f() = g() + 1\n\
             fn g() = 41\n\
             let t = spawn { f() }\n\
             to_string(f()) + \" \" + show(task.join(t))\n\
         }\n\
         share fn lam(xs) = {\n\
             let k = 10\n\
             map(xs, (x) => helper(x) + k)\n\
         }\n\
         fn helper(x) = x * 2\n",
    );
    write(
        &ws,
        "main.ol",
        "use lib.m { a, outer, lam }\n\
         println(a(5))\n\
         println(outer())\n\
         println(show(lam([1, 2])))\n\
         fn later_user() = later_val + 1\n\
         let later_val = 41\n\
         println(to_string(later_user()))\n\
         let t = spawn { later_user() }\n\
         println(show(task.join(t)))\n",
    );
    in_every_mode(&ws, "main.ol", "b\n42 42\n[12, 14]\n42\n42\n");
}

#[test]
fn compress_round_trips_and_says_no_to_garbage() {
    let ws = workspace("compress");
    write(
        &ws,
        "c.ol",
        "let body = str.repeat(\"- [ ] item\\n\", 500)\n\
         let packed = compress.gzip(body)\n\
         println(show(len(packed) < 200))\n\
         println(show(unwrap(bytes.to_string(unwrap(compress.gunzip(packed)))) == body))\n\
         println(show(is_ok(compress.inflate(compress.deflate(\"hello\")))))\n\
         println(show(is_err(compress.gunzip(\"nope\"))))\n\
         println(show(len(compress.gzip_level(body, 9)) <= len(compress.gzip_level(body, 1))))\n",
    );
    in_every_mode(&ws, "c.ol", "true\ntrue\ntrue\ntrue\ntrue\n");
}

#[test]
fn the_whole_program_can_run_on_a_task_and_bench_measures_it_there() {
    let ws = workspace("intask");
    write(
        &ws,
        "loop.ol",
        "fn work(n) = fold(range(0, n), 0, (a, i) => a + str.length(to_string(i)))\n\
         println(to_string(work(20000)))\n",
    );
    let (out, err, rc) = olang(&ws, &["--in-task", "run", "loop.ol"]);
    assert_eq!(rc, 0, "{out}{err}");
    assert_eq!(out, "88890\n");
    let (out, err, rc) = olang(&ws, &["bench", "--in-task", "--runs", "1", "loop.ol"]);
    assert_eq!(rc, 0, "{out}{err}");
    assert!(out.contains("each on a spawned task"), "{out}");
    assert!(out.contains("loop"), "{out}");
}
