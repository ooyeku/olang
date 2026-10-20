//! `olang repl --serve`: the REPL protocol over the real binary's stdio —
//! the handshake, values by kind with handles and `expand`, a file's
//! scope (its declarations, a parameter that is not bound, errors with
//! frames), what a program prints, interrupting a tight loop on every
//! tier without losing the session, a module reloaded (and a reload that
//! fails keeping the old one), a Loom view rendered headless, a
//! function's parameters bound for one evaluation (`bind`), an
//! evaluation under no capability (`pure`) refused when it reaches out,
//! and a notebook's kernel (`--notebook`): its params, its grant, its
//! folder, its cells' definitions replaced, its lines in every error.

use serde_json::{Value as J, json};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

struct Repl {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next: u64,
}

impl Repl {
    fn start(dir: &Path) -> (Repl, J) {
        Repl::start_with(dir, &[])
    }

    fn start_with(dir: &Path, more: &[&str]) -> (Repl, J) {
        let mut child = Command::new(env!("CARGO_BIN_EXE_olang"))
            .args(["repl", "--serve"])
            .args(more)
            .current_dir(dir)
            .env_remove("OLANG_DENY")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn olang repl --serve");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let mut r = Repl {
            child,
            stdin,
            stdout,
            next: 1,
        };
        let hello = r.recv();
        (r, hello)
    }

    fn recv(&mut self) -> J {
        let mut line = String::new();
        self.stdout.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap_or_else(|e| panic!("not JSON ({e}): {line:?}"))
    }

    fn write(&mut self, mut m: J) -> u64 {
        let id = self.next;
        self.next += 1;
        m["id"] = json!(id);
        writeln!(self.stdin, "{}", m).unwrap();
        self.stdin.flush().unwrap();
        id
    }

    fn ask(&mut self, m: J) -> J {
        let id = self.write(m);
        let r = self.recv();
        assert_eq!(r["id"], json!(id), "a reply for another request: {r}");
        r
    }

    fn eval(&mut self, code: &str) -> J {
        self.ask(json!({ "op": "eval", "code": code }))
    }

    fn eval_at(&mut self, file: &Path, line: u64, code: &str) -> J {
        self.ask(json!({ "op": "eval", "code": code, "file": file, "line": line }))
    }

    /// Evaluate `code`, interrupt it after `wait`, and answer the
    /// evaluation's reply.
    fn interrupted(&mut self, m: J, wait: Duration) -> J {
        let id = self.write(m);
        std::thread::sleep(wait);
        let iid = self.write(json!({ "op": "interrupt" }));
        let mut eval = J::Null;
        let mut ack = J::Null;
        while eval.is_null() || ack.is_null() {
            let r = self.recv();
            if r["id"] == json!(iid) {
                ack = r;
            } else if r["id"] == json!(id) {
                eval = r;
            }
        }
        assert_eq!(
            ack["running"],
            json!(true),
            "the interrupt found nothing running: {ack}"
        );
        eval
    }
}

impl Drop for Repl {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let w = |name: &str, text: &str| std::fs::write(dir.path().join(name), text).unwrap();
    w(
        "olang.toml",
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n",
    );
    w("util.ol", "share fn twice(x) = x * 2\n");
    w(
        "calc.ol",
        r#"use util { twice }

let RATE = 3
let OPENED = fs.read_file("nothing-here")

fn scaled(x) = {
    let y = x * RATE
    twice(y)
}

fn boom(n) = inner(n + 1)

fn inner(z) = 1 / (z - z)

fn spin() = {
    let mut i = 0
    while true { i = i + 1 }
    i
}

fn sum_to(n) = {
    let mut i = 0
    let mut s = 0
    while i < n {
        s = s + i
        i = i + 1
    }
    s
}

fn forever(n) = forever(n + 1)

fn rows(n) = map(range(0, n), (i) => #{ "id": i, "name": "row " + to_string(i), "score": i * 7 % 13 })
"#,
    );
    dir
}

fn calc(dir: &Path) -> PathBuf {
    dir.join("calc.ol")
}

#[test]
fn the_handshake_says_the_protocol_and_the_project() {
    let dir = project();
    let (_r, hello) = Repl::start(dir.path());
    assert_eq!(hello["event"], json!("hello"));
    assert_eq!(hello["protocol"], json!(1));
    assert!(hello["olang"].as_str().is_some());
    let root = std::fs::canonicalize(dir.path()).unwrap();
    let said = std::fs::canonicalize(hello["root"].as_str().unwrap()).unwrap();
    assert_eq!(said, root);
}

#[test]
fn values_arrive_by_kind_with_their_type_time_and_tier() {
    let dir = project();
    let (mut r, _) = Repl::start(dir.path());
    let v = r.eval("1 + 2");
    assert_eq!(v["ok"], json!(true));
    assert_eq!(v["value"], json!({ "k": "int", "t": "Int", "s": "3" }));
    assert!(v["ms"].as_f64().is_some() && v["tier"].as_str().is_some());
    let s = r.eval("\"hello\"");
    assert_eq!(s["value"]["k"], json!("str"));
    let res = r.eval("Err(\"no\")");
    assert_eq!(res["value"]["k"], json!("result"));
    assert_eq!(res["value"]["ok"], json!(false));
    assert_eq!(res["value"]["v"]["s"], json!("no"));
    // a map: a handle and its entries, in its order
    let m = r.eval("#{ \"b\": [1, 2, 3], \"a\": 1.5 }");
    assert_eq!(m["value"]["k"], json!("map"));
    assert_eq!(m["value"]["entries"][0]["key"], json!("b"));
    let h = m["value"]["h"].as_u64().unwrap();
    let e = r.ask(json!({ "op": "expand", "h": h, "start": 0, "count": 10 }));
    assert_eq!(e["entries"][0]["key"], json!("b"));
    assert_eq!(e["entries"][1]["key"], json!("a"));
    assert_eq!(e["entries"][0]["v"]["series"]["hi"], json!(3.0));
    // a let's value is its binding's
    let l = r.eval("let answer = 6 * 7");
    assert_eq!(l["binding"], json!("answer"));
    assert_eq!(l["value"]["s"], json!("42"));
    // printed text comes with the evaluation, not on the protocol
    let p = r.eval("println(\"side\")\nanswer");
    assert_eq!(p["out"], json!("side\n"));
    assert_eq!(p["value"]["s"], json!("42"));
    // bytes that are a picture carry it
    let png = r.eval("bytes.from_list([137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 4, 0, 0, 0, 3, 8, 6, 0, 0, 0])");
    assert_eq!(png["value"]["k"], json!("bytes"));
    assert_eq!(
        png["value"]["img"],
        json!({ "format": "png", "w": 4, "h": 3 })
    );
}

#[test]
fn a_list_of_maps_is_a_table_sorted_and_read_by_range() {
    let dir = project();
    let (mut r, _) = Repl::start(dir.path());
    let t = r.eval_at(&calc(dir.path()), 1, "rows(100000)");
    assert_eq!(t["ok"], json!(true), "{t}");
    assert_eq!(t["value"]["n"], json!(100000));
    assert_eq!(t["value"]["table"]["cols"], json!(["id", "name", "score"]));
    let h = t["value"]["h"].as_u64().unwrap();
    let page = r.ask(json!({ "op": "expand", "h": h, "table": true, "start": 99998, "count": 5 }));
    assert_eq!(page["rows"].as_array().unwrap().len(), 2);
    assert_eq!(page["rows"][1]["c"][1], json!("row 99999"));
    let sorted = r.ask(json!({ "op": "expand", "h": h, "table": true, "start": 0, "count": 3, "sort": { "col": 2, "desc": true } }));
    assert_eq!(sorted["rows"][0]["c"][2], json!("12"));
    // released, the handle is gone
    let seq = t["eval"].as_u64().unwrap();
    let _ = r.ask(json!({ "op": "release", "eval": seq }));
    let gone = r.ask(json!({ "op": "expand", "h": h }));
    assert_eq!(gone["error"]["kind"], json!("gone"));
}

#[test]
fn a_file_scope_has_its_declarations_and_names_what_is_not_bound() {
    let dir = project();
    let (mut r, _) = Repl::start(dir.path());
    let f = calc(dir.path());
    // the file's functions, its `use`, and a `let` that calls nothing
    let v = r.eval_at(&f, 30, "scaled(5)");
    assert_eq!(v["value"]["s"], json!("30"), "{v}");
    // a line inside a function: its parameter is not bound here
    let p = r.eval_at(&f, 7, "let y = x * RATE");
    assert_eq!(p["ok"], json!(false));
    assert_eq!(p["error"]["kind"], json!("unbound"));
    assert_eq!(
        p["error"]["message"],
        json!("`x` is a parameter of `scaled` — not bound here")
    );
    assert_eq!(p["error"]["line"], json!(7));
    // a top-level let that calls something is left for its own line
    let o = r.eval_at(&f, 30, "OPENED");
    assert_eq!(o["error"]["kind"], json!("unbound"));
    assert!(
        o["error"]["message"]
            .as_str()
            .unwrap()
            .contains("evaluate its line (4) first")
    );
    // the session's own scope does not see the file's
    let other = r.eval("scaled(5)");
    assert_eq!(other["ok"], json!(false));
    // an error names its frames, each with its place
    let e = r.eval_at(&f, 30, "boom(1)");
    assert_eq!(e["error"]["message"], json!("Division by zero"));
    let frames = e["error"]["stack"].as_array().unwrap();
    let names: Vec<&str> = frames.iter().map(|x| x["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["boom", "inner"]);
    assert_eq!(frames[1]["line"], json!(13));
    assert!(frames[1]["file"].as_str().unwrap().ends_with("calc.ol"));
    // a parse error's line is the file's
    let bad = r.eval_at(&f, 12, "fn g(x) = x +");
    assert_eq!(bad["error"]["kind"], json!("parse"));
    assert_eq!(bad["error"]["line"], json!(12));
}

#[test]
fn a_functions_parameters_are_named_then_bound_for_one_evaluation() {
    let dir = project();
    let (mut r, hello) = Repl::start(dir.path());
    assert_eq!(
        hello["features"],
        json!([
            "bind", "pure", "cell", "caps", "profile", "distinct", "record", "html", "scene"
        ])
    );
    let f = calc(dir.path());
    // refused: the function and its parameters said
    let p = r.eval_at(&f, 7, "let y = x * RATE");
    assert_eq!(p["error"]["kind"], json!("unbound"));
    assert_eq!(p["error"]["fn"], json!("scaled"));
    assert_eq!(p["error"]["params"], json!(["x"]));
    assert_eq!(p["error"]["fn_line"], json!(6));
    // bound: each value an expression evaluated in the file's scope
    let b = r.ask(json!({ "op": "eval", "code": "let y = x * RATE", "file": f, "line": 7, "bind": { "x": "RATE + 1" } }));
    assert_eq!(b["ok"], json!(true), "{b}");
    assert_eq!(b["binding"], json!("y"));
    assert_eq!(b["value"]["s"], json!("12"));
    let l = r.ask(json!({ "op": "eval", "code": "twice(x) + len(xs)", "file": f, "line": 8, "bind": { "x": "3", "xs": "[1, 2, #{ \"a\": \"b\" }]" } }));
    assert_eq!(l["value"]["s"], json!("9"), "{l}");
    // taken back afterwards: `x` is not bound for the next evaluation
    let after = r.eval_at(&f, 30, "x");
    assert_eq!(after["ok"], json!(false));
    // a binding the scope had is put back
    let _ = r.eval_at(&f, 30, "let k = 1");
    let k = r.ask(
        json!({ "op": "eval", "code": "k * 10", "file": f, "line": 30, "bind": { "k": "5" } }),
    );
    assert_eq!(k["value"]["s"], json!("50"));
    assert_eq!(r.eval_at(&f, 30, "k")["value"]["s"], json!("1"));
    // a value that fails says which
    let bad =
        r.ask(json!({ "op": "eval", "code": "x", "file": f, "line": 7, "bind": { "x": "1 / 0" } }));
    assert_eq!(bad["ok"], json!(false));
    assert!(
        bad["error"]["message"]
            .as_str()
            .unwrap()
            .contains("the value given for `x` fails"),
        "{bad}"
    );
}

#[test]
fn a_pure_evaluation_runs_under_no_capability_and_says_which_it_wanted() {
    let dir = project();
    let (mut r, _) = Repl::start(dir.path());
    let f = calc(dir.path());
    let ok =
        r.ask(json!({ "op": "eval", "code": "scaled(5)", "file": f, "line": 30, "pure": true }));
    assert_eq!(ok["value"]["s"], json!("30"), "{ok}");
    let read = r.ask(json!({ "op": "eval", "code": "fs.read_file(\"calc.ol\")", "file": f, "line": 30, "pure": true }));
    assert_eq!(read["ok"], json!(false));
    assert_eq!(read["error"]["kind"], json!("impure"), "{read}");
    assert_eq!(read["error"]["cap"], json!("fs"));
    let env = r.ask(json!({ "op": "eval", "code": "os.get_env(\"HOME\")", "pure": true }));
    assert_eq!(env["error"]["cap"], json!("env"), "{env}");
    // the session's own grant is back: the same read runs
    let again = r.eval_at(&f, 30, "fs.exists(\"calc.ol\")");
    assert_eq!(again["value"]["s"], json!("true"), "{again}");
    // a function that reaches out deep inside, on a compiled tier too
    let _ = r.eval_at(
        &f,
        30,
        "fn peek(n) = if n == 0 => fs.exists(\"calc.ol\") else => peek(n - 1)",
    );
    let _warm = r.eval_at(&f, 30, "peek(3)");
    let deep =
        r.ask(json!({ "op": "eval", "code": "peek(3)", "file": f, "line": 30, "pure": true }));
    assert_eq!(deep["error"]["kind"], json!("impure"), "{deep}");
    // a plain error stays an error
    let e = r.ask(json!({ "op": "eval", "code": "boom(1)", "file": f, "line": 30, "pure": true }));
    assert_eq!(e["error"]["kind"], json!("runtime"));
}

#[test]
fn interrupt_stops_a_tight_loop_on_every_tier_and_the_session_lives() {
    let dir = project();
    let (mut r, _) = Repl::start(dir.path());
    let f = calc(dir.path());
    let wait = Duration::from_millis(400);
    // the tree-walker
    let a = r.interrupted(
        json!({ "op": "eval", "code": "let mut k = 0\nwhile true { k = k + 1 }" }),
        wait,
    );
    assert_eq!(a["error"]["kind"], json!("interrupted"), "{a}");
    // a function on the bytecode tier
    let b = r.interrupted(
        json!({ "op": "eval", "code": "spin()", "file": f, "line": 40 }),
        wait,
    );
    assert_eq!(b["error"]["kind"], json!("interrupted"), "{b}");
    // native code: compiled on a small call, then spun
    let warm = r.eval_at(&f, 40, "sum_to(10)");
    assert_eq!(warm["value"]["s"], json!("45"));
    let t0 = Instant::now();
    let c = r.interrupted(
        json!({ "op": "eval", "code": "sum_to(100000000000)", "file": f, "line": 40 }),
        wait,
    );
    assert_eq!(c["error"]["kind"], json!("interrupted"), "{c}");
    assert!(t0.elapsed() < Duration::from_secs(5));
    // a recursion
    let d = r.interrupted(
        json!({ "op": "eval", "code": "forever(0)", "file": f, "line": 40 }),
        wait,
    );
    assert_eq!(d["error"]["kind"], json!("interrupted"), "{d}");
    // the session goes on, its state intact
    let after = r.eval_at(&f, 40, "sum_to(10) + RATE");
    assert_eq!(after["value"]["s"], json!("48"));
    // an interrupt with nothing running is a no-op
    let idle = r.ask(json!({ "op": "interrupt" }));
    assert_eq!(idle["running"], json!(false));
    assert_eq!(r.eval("1")["value"]["s"], json!("1"));
}

#[test]
fn a_saved_module_reloads_keeping_state_and_a_broken_one_is_kept() {
    let dir = project();
    let (mut r, _) = Repl::start(dir.path());
    let f = calc(dir.path());
    let util = dir.path().join("util.ol");
    assert_eq!(r.eval_at(&f, 30, "scaled(5)")["value"]["s"], json!("30"));
    assert_eq!(r.eval_at(&f, 30, "let kept = 100")["ok"], json!(true));
    std::fs::write(&util, "share fn twice(x) = x * 3\n").unwrap();
    let ok = r.ask(json!({ "op": "reload", "path": util }));
    assert_eq!(ok["ok"], json!(true), "{ok}");
    // the new module, the scope's own binding kept (native code too)
    assert_eq!(
        r.eval_at(&f, 30, "scaled(5) + kept")["value"]["s"],
        json!("145")
    );
    std::fs::write(&util, "share fn twice(x) = x *\n").unwrap();
    let bad = r.ask(json!({ "op": "reload", "path": util }));
    assert_eq!(bad["ok"], json!(false));
    assert_eq!(bad["kept"], json!(true));
    assert_eq!(bad["error"]["kind"], json!("parse"));
    assert_eq!(r.eval_at(&f, 30, "scaled(5)")["value"]["s"], json!("45"));
    // the file's own declarations again
    let src = std::fs::read_to_string(&f).unwrap();
    std::fs::write(&util, "share fn twice(x) = x * 2\n").unwrap();
    std::fs::write(&f, src.replace("let RATE = 3", "let RATE = 10")).unwrap();
    assert_eq!(
        r.ask(json!({ "op": "reload", "path": util }))["ok"],
        json!(true)
    );
    assert_eq!(
        r.ask(json!({ "op": "reload", "path": f }))["ok"],
        json!(true)
    );
    assert_eq!(
        r.eval_at(&f, 30, "scaled(5) + kept")["value"]["s"],
        json!("200")
    );
}

#[test]
fn a_loom_view_renders_headless_to_a_picture() {
    // Loom is a sibling checkout; without it there is nothing to draw with
    let loom = Path::new(env!("CARGO_MANIFEST_DIR")).join("../loom");
    if !loom.join("lib/test.ol").exists() {
        eprintln!("no ../loom: skipped");
        return;
    }
    let dir = project();
    let (mut r, _) = Repl::start(dir.path());
    let v = r.eval("#{ \"role\": \"text\", \"key\": \"t\", \"props\": #{ \"text\": \"Hello\" }, \"children\": [] }");
    assert_eq!(v["value"]["k"], json!("view"));
    let h = v["value"]["h"].as_u64().unwrap();
    let pic = r.ask(json!({ "op": "render", "h": h, "dark": true, "scale": 2.0, "loom": std::fs::canonicalize(&loom).unwrap() }));
    assert_eq!(pic["ok"], json!(true), "{pic}");
    use base64::Engine;
    let png = base64::engine::general_purpose::STANDARD
        .decode(pic["png"].as_str().unwrap())
        .unwrap();
    assert!(png.starts_with(&[0x89, b'P', b'N', b'G']));
    let w = pic["width"].as_f64().unwrap();
    assert!(w > 20.0 && w < 200.0, "{w}");
    let pw = u32::from_be_bytes([png[16], png[17], png[18], png[19]]) as f64;
    assert_eq!(pw, w * 2.0);
}

#[test]
fn shutdown_ends_the_child() {
    let dir = project();
    let (mut r, _) = Repl::start(dir.path());
    let bye = r.ask(json!({ "op": "shutdown" }));
    assert_eq!(bye["ok"], json!(true));
    let t0 = Instant::now();
    loop {
        if let Ok(Some(status)) = r.child.try_wait() {
            assert!(status.success());
            break;
        }
        assert!(
            t0.elapsed() < Duration::from_secs(5),
            "the child is still running"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A notebook for a kernel, in a folder of its own inside `dir`:
/// a param, a grant that reads files and writes none, three cells.
fn notebook(dir: &Path) -> PathBuf {
    let nb = dir.join("nbs/sales.olmd");
    std::fs::create_dir_all(nb.parent().unwrap()).unwrap();
    std::fs::write(
        &nb,
        "---\ntitle: Kernel\nparams:\n  n: 2\ncapabilities:\n  fs: read\n---\n\n# A kernel\n\n```olang\nlet x = n * 10\n```\n\n```olang\nx + 1\n```\n\n```olang\nfn boom() = raise(\"no\")\nboom()\n```\n",
    )
    .unwrap();
    std::fs::write(nb.parent().unwrap().join("data.txt"), "beside it").unwrap();
    nb
}

#[test]
fn a_notebooks_kernel_binds_its_params_under_its_grant_in_its_folder() {
    let dir = tempfile::tempdir().unwrap();
    let nb = notebook(dir.path());
    // started from elsewhere: the notebook's folder is its working directory
    let (mut r, hello) = Repl::start_with(dir.path(), &["--notebook", "nbs/sales.olmd"]);
    assert!(
        hello["features"]
            .as_array()
            .unwrap()
            .contains(&json!("cell")),
        "{hello}"
    );
    let said = std::fs::canonicalize(hello["notebook"]["path"].as_str().unwrap()).unwrap();
    assert_eq!(said, std::fs::canonicalize(&nb).unwrap());
    assert_eq!(hello["notebook"]["title"], json!("Kernel"));
    assert_eq!(hello["notebook"]["params"], json!(["n"]));
    assert_eq!(hello["notebook"]["problems"], json!([]));
    // a cell reads the param; a later one reads the cell's name
    let a = r.ask(json!({ "op": "eval", "cell": "c1", "code": "let x = n * 10", "line": 12 }));
    // a cell ending in a let is `()`, and says what it defined
    assert_eq!(a["value"]["k"], json!("unit"), "{a}");
    assert_eq!(a["defines"], json!(["x"]), "{a}");
    assert_eq!(r.eval("x")["value"]["s"], json!("20"));
    let b = r.ask(json!({ "op": "eval", "cell": "c2", "code": "x + 1", "line": 16 }));
    assert_eq!(b["value"]["s"], json!("21"), "{b}");
    // a relative path is the notebook's folder's; the grant reads, never writes
    let read =
        r.ask(json!({ "op": "eval", "cell": "c4", "code": "unwrap(fs.read_file(\"data.txt\"))" }));
    assert_eq!(read["value"]["s"], json!("beside it"), "{read}");
    let wrote =
        r.ask(json!({ "op": "eval", "cell": "c4", "code": "fs.write_file(\"out.txt\", \"x\")" }));
    assert_eq!(wrote["ok"], json!(false), "{wrote}");
    assert!(
        wrote["error"]["message"]
            .as_str()
            .unwrap()
            .contains("capability 'fs' denied"),
        "{wrote}"
    );
    assert!(!nb.parent().unwrap().join("out.txt").exists());
    // the prompt (no cell, no file) is the notebook's scope too
    let p = r.eval("x * 2");
    assert_eq!(p["value"]["s"], json!("40"), "{p}");
}

#[test]
fn a_cell_evaluated_again_replaces_what_it_defined() {
    let dir = tempfile::tempdir().unwrap();
    notebook(dir.path());
    let (mut r, _) = Repl::start_with(dir.path(), &["--notebook", "nbs/sales.olmd"]);
    let a =
        r.ask(json!({ "op": "eval", "cell": "c1", "code": "let x = 1\nfn f() = 2", "line": 12 }));
    assert_eq!(a["ok"], json!(true), "{a}");
    assert_eq!(r.eval("x + f()")["value"]["s"], json!("3"));
    // the same cell now defines y: x and f are gone
    let again = r.ask(json!({ "op": "eval", "cell": "c1", "code": "let y = 5", "line": 12 }));
    assert_eq!(again["ok"], json!(true), "{again}");
    let gone = r.eval("x");
    assert_eq!(gone["ok"], json!(false), "{gone}");
    assert!(r.eval("f()")["ok"] == json!(false));
    assert_eq!(r.eval("y")["value"]["s"], json!("5"));
    // a cell id may be a number; another cell's names stay
    let other = r.ask(json!({ "op": "eval", "cell": 7, "code": "let z = y + 1", "line": 16 }));
    assert_eq!(other["ok"], json!(true), "{other}");
    assert_eq!(r.eval("z")["value"]["s"], json!("6"));
    let c1 = r.ask(json!({ "op": "eval", "cell": "c1", "code": "let y = 50", "line": 12 }));
    assert_eq!(c1["ok"], json!(true));
    assert_eq!(r.eval("z")["value"]["s"], json!("6"));
    // a cell that does not parse keeps what it defined
    let bad = r.ask(json!({ "op": "eval", "cell": "c1", "code": "let = ", "line": 12 }));
    assert_eq!(bad["error"]["kind"], json!("parse"));
    assert_eq!(bad["error"]["line"], json!(12), "{bad}");
    assert_eq!(r.eval("y")["value"]["s"], json!("50"));
}

#[test]
fn a_cells_errors_and_frames_name_the_notebooks_lines() {
    let dir = tempfile::tempdir().unwrap();
    let nb = notebook(dir.path());
    let (mut r, _) = Repl::start_with(dir.path(), &["--notebook", "nbs/sales.olmd"]);
    // a name a cell not run yet defines: that cell named
    let early = r.ask(json!({ "op": "eval", "cell": "c2", "code": "x + 1", "line": 16 }));
    assert!(
        early["error"]["message"]
            .as_str()
            .unwrap()
            .contains("`x` is defined at line 12, in a cell not run yet"),
        "{early}"
    );
    let e = r.ask(json!({ "op": "eval", "cell": "c3", "code": "fn boom() = raise(\"no\")\nboom()", "line": 20 }));
    assert_eq!(e["ok"], json!(false), "{e}");
    let err = &e["error"];
    let file = std::fs::canonicalize(err["file"].as_str().unwrap()).unwrap();
    assert_eq!(file, std::fs::canonicalize(&nb).unwrap(), "{e}");
    // the call that failed is on the cell's second line, the file's 21
    assert_eq!(err["line"], json!(21), "{e}");
    let frame = &err["stack"][0];
    assert_eq!(frame["name"], json!("boom"), "{e}");
    assert_eq!(frame["line"], json!(20), "{e}");
    let ff = std::fs::canonicalize(frame["file"].as_str().unwrap()).unwrap();
    assert_eq!(ff, std::fs::canonicalize(&nb).unwrap());
    // a parse error on the cell's second line is the file's line 21
    let p = r.ask(json!({ "op": "eval", "cell": "c3", "code": "1\nlet = 2", "line": 20 }));
    assert_eq!(p["error"]["line"], json!(21), "{p}");
}

#[test]
fn a_notebooks_kernel_refuses_what_is_no_notebook_and_a_grant_it_cannot_read() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("plain.ol"), "1\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_olang"))
        .args(["repl", "--serve", "--notebook", "plain.ol"])
        .current_dir(dir.path())
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("is no notebook"));
    // a capability the subset does not know: served, under no capability at all
    std::fs::write(
        dir.path().join("bad.olmd"),
        "---\ncapabilities:\n  gpu: true\n---\n\n```olang\n1\n```\n",
    )
    .unwrap();
    let (mut r, hello) = Repl::start_with(dir.path(), &["--notebook", "bad.olmd"]);
    assert_eq!(hello["notebook"]["refused"], json!(true), "{hello}");
    assert_eq!(
        hello["notebook"]["problems"][0]["line"],
        json!(3),
        "{hello}"
    );
    let env = r.ask(json!({ "op": "eval", "cell": "a", "code": "os.get_env(\"HOME\")" }));
    assert_eq!(env["ok"], json!(false), "{env}");
    assert_eq!(r.eval("1 + 1")["value"]["s"], json!("2"));
}

#[test]
fn a_cells_value_is_its_last_expression_and_it_says_what_it_defines() {
    let dir = tempfile::tempdir().unwrap();
    notebook(dir.path());
    let (mut r, _) = Repl::start_with(dir.path(), &["--notebook", "nbs/sales.olmd"]);
    let defs = r.ask(json!({ "op": "eval", "cell": "c1", "code": "fn fmt(x) = to_string(x)\nfn pct(x) = x * 100.0", "line": 12 }));
    assert_eq!(defs["value"]["k"], json!("unit"), "{defs}");
    assert_eq!(defs["defines"], json!(["fmt", "pct"]), "{defs}");
    let lets = r.ask(json!({ "op": "eval", "cell": "c2", "code": "let a = 1\nlet share = 0.2900157511888042", "line": 16 }));
    assert_eq!(lets["value"]["k"], json!("unit"), "{lets}");
    assert!(lets.get("binding").is_none(), "{lets}");
    assert_eq!(lets["defines"], json!(["a", "share"]));
    let expr =
        r.ask(json!({ "op": "eval", "cell": "c3", "code": "let b = a + 1\nb * 2", "line": 20 }));
    assert_eq!(expr["value"]["s"], json!("4"), "{expr}");
    assert_eq!(expr["defines"], json!(["b"]));
    // the prompt keeps its binding
    let p = r.eval("let c = 3");
    assert_eq!(p["binding"], json!("c"));
}

#[test]
fn a_frame_and_a_series_are_tables_with_typed_columns_read_by_range_and_sorted() {
    let dir = project();
    let (mut r, _) = Repl::start(dir.path());
    let f = r.eval(
        "ods.frame([[\"name\", ods.series([\"b\", \"a\", \"c\"])], [\"qty\", ods.series([3, (), 1])], [\"price\", ods.series([0.2900157511888042, 2.5, 10.0])]])",
    );
    assert_eq!(f["ok"], json!(true), "{f}");
    let v = &f["value"];
    assert_eq!(v["k"], json!("frame"));
    assert_eq!(v["t"], json!("Frame"));
    assert_eq!(v["n"], json!(3));
    assert_eq!(v["s"], json!("Frame[3 x 3]"));
    assert_eq!(v["table"]["cols"], json!(["name", "qty", "price"]));
    assert_eq!(v["table"]["types"], json!(["String", "Int", "Float"]));
    assert_eq!(v["rows"][0]["c"], json!(["b", "3", "0.290016"]));
    assert_eq!(v["rows"][1]["c"][1], J::Null);
    let h = v["h"].as_u64().unwrap();
    let page = r.ask(json!({ "op": "expand", "h": h, "table": true, "start": 1, "count": 5 }));
    assert_eq!(page["n"], json!(3));
    assert_eq!(page["rows"][1]["c"], json!(["c", "1", "10.0"]));
    // sorted by qty, the null last either way
    let up = r.ask(json!({ "op": "expand", "h": h, "table": true, "start": 0, "count": 3, "sort": { "col": 1 } }));
    assert_eq!(up["rows"][0]["c"][0], json!("c"));
    assert_eq!(up["rows"][2]["c"][1], J::Null);
    let down = r.ask(json!({ "op": "expand", "h": h, "table": true, "start": 0, "count": 3, "sort": { "col": 1, "desc": true } }));
    assert_eq!(down["rows"][0]["c"][0], json!("b"));
    assert_eq!(down["rows"][2]["c"][1], J::Null);
    // a Series: one column, named value
    let s = r.eval("ods.series([1.5, 2.5])");
    assert_eq!(s["value"]["k"], json!("series"), "{s}");
    assert_eq!(s["value"]["table"]["cols"], json!(["value"]));
    assert_eq!(s["value"]["table"]["types"], json!(["Float"]));
    assert_eq!(s["value"]["rows"][1]["c"], json!(["2.5"]));
    // a numeric Series says its summary and a histogram
    let st = &s["value"]["stats"];
    assert_eq!(st["count"], json!(2));
    assert_eq!(st["mean"], json!(2.0));
    assert_eq!(
        (st["min"].clone(), st["max"].clone()),
        (json!(1.5), json!(2.5))
    );
    assert_eq!(st["hist"].as_array().unwrap().len(), 24);
    assert_eq!(st["hist"][0], json!(1));
    assert_eq!(st["hist"][23], json!(1));
    assert!(
        r.eval("ods.series([\"a\"])")["value"]
            .get("stats")
            .is_none()
    );
}

#[test]
fn a_chart_says_it_is_svg_with_its_size_and_background() {
    let dir = project();
    let (mut r, _) = Repl::start(dir.path());
    let c = r.eval("plot.bar([\"a\", \"b\"], ods.series([1.0, 2.0]), #{ \"title\": \"t\" })");
    assert_eq!(c["ok"], json!(true), "{c}");
    let svg = &c["value"]["svg"];
    assert_eq!(svg["w"], json!(720.0), "{c}");
    assert_eq!(svg["h"], json!(440.0));
    assert_eq!(svg["bg"], json!("#fcfcfb"));
    assert!(svg["text"].as_str().unwrap().ends_with("</svg>"));
    // a string that is no SVG says nothing of it
    assert!(r.eval("\"<svg\"")["value"].get("svg").is_none());
}

#[test]
fn a_hosts_plot_theme_is_a_charts_when_it_names_none() {
    let dir = project();
    let run = |theme: Option<&str>, code: &str| {
        let mut c = Command::new(env!("CARGO_BIN_EXE_olang"));
        c.args(["eval", code]).current_dir(dir.path());
        match theme {
            Some(t) => c.env("OLANG_PLOT_THEME", t),
            None => c.env_remove("OLANG_PLOT_THEME"),
        };
        String::from_utf8_lossy(&c.output().unwrap().stdout).into_owned()
    };
    let chart = "plot.bar([\"a\"], ods.series([1.0]), #{})";
    assert!(run(None, chart).contains("#fcfcfb"));
    assert!(run(Some("dark"), chart).contains("#101720"));
    // a chart that names its theme keeps it
    assert!(
        run(
            Some("dark"),
            "plot.bar([\"a\"], ods.series([1.0]), #{ \"theme\": \"light\" })"
        )
        .contains("#fcfcfb")
    );
}

#[test]
fn a_cells_names_are_read_from_its_text_without_running_it_even_while_a_cell_runs() {
    let dir = tempfile::tempdir().unwrap();
    notebook(dir.path());
    let (mut r, hello) = Repl::start_with(dir.path(), &["--notebook", "nbs/sales.olmd"]);
    let ops = hello["ops"].as_array().unwrap();
    for op in ["deps", "drop", "vars", "reread"] {
        assert!(ops.contains(&json!(op)), "{op}: {hello}");
    }
    let d = r.ask(json!({ "op": "deps", "cells": [
        { "cell": "c1", "code": "let mine = rows(sales, region)\nlet total = sum(mine)", "line": 40 },
        { "cell": "c2", "code": "fn detects(size) = {\n    random.seed(seed + size)\n    lift\n}\nuse util { helper }\nhelper(detects(3))", "line": 10 },
        { "cell": "c3", "code": "let = 2", "line": 5 },
    ] }));
    assert_eq!(d["ok"], json!(true), "{d}");
    let c1 = &d["cells"][0];
    assert_eq!(c1["cell"], json!("c1"));
    assert_eq!(
        c1["defines"],
        json!([{ "name": "mine", "line": 40, "col": 1 }, { "name": "total", "line": 41, "col": 1 }]),
        "{d}"
    );
    // a builtin is no use; a name the cell defines before reading it neither
    assert_eq!(
        c1["uses"],
        json!([{ "name": "rows", "line": 40, "col": 12 }, { "name": "sales", "line": 40, "col": 17 }, { "name": "region", "line": 40, "col": 24 }]),
        "{d}"
    );
    // a function's body reads names too; a `use` binds one, said so
    let c2 = &d["cells"][1];
    let uses: Vec<&str> = c2["uses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|u| u["name"].as_str().unwrap())
        .collect();
    assert_eq!(uses, ["seed", "lift"], "{d}");
    assert_eq!(c2["uses"][0]["line"], json!(11));
    assert_eq!(
        c2["defines"][1],
        json!({ "name": "helper", "line": 14, "col": 1, "use": true }),
        "{d}"
    );
    // a cell that does not parse says where
    assert_eq!(d["cells"][2]["ok"], json!(false), "{d}");
    assert_eq!(d["cells"][2]["error"]["line"], json!(5), "{d}");
    // asked while a cell runs: answered at once, the cell still running
    let spin = r.write(json!({ "op": "eval", "cell": "c9", "code": "let mut s = 0\nwhile true { s = s + 1 }", "line": 30 }));
    std::thread::sleep(Duration::from_millis(200));
    let did =
        r.write(json!({ "op": "deps", "cells": [{ "cell": "c1", "code": "x + 1", "line": 16 }] }));
    let first = r.recv();
    assert_eq!(first["id"], json!(did), "{first}");
    assert_eq!(first["cells"][0]["uses"][0]["name"], json!("x"));
    let _ = r.write(json!({ "op": "interrupt" }));
    let mut stopped = J::Null;
    while stopped.is_null() {
        let m = r.recv();
        if m["id"] == json!(spin) {
            stopped = m;
        }
    }
    assert_eq!(stopped["error"]["kind"], json!("interrupted"), "{stopped}");
}

#[test]
fn every_evaluation_says_the_capabilities_it_reached() {
    let dir = tempfile::tempdir().unwrap();
    notebook(dir.path());
    let (mut r, hello) = Repl::start_with(dir.path(), &["--notebook", "nbs/sales.olmd"]);
    assert!(
        hello["features"]
            .as_array()
            .unwrap()
            .contains(&json!("caps")),
        "{hello}"
    );
    // a file read under the grant: fs, though allowed
    let read = r.ask(json!({ "op": "eval", "cell": "c1", "code": "let t = unwrap(fs.read_file(\"data.txt\"))", "line": 12 }));
    assert_eq!(read["ok"], json!(true), "{read}");
    assert_eq!(read["caps"], json!(["fs"]), "{read}");
    // computing, printing, the clock and randomness reach nothing
    let pure = r.ask(json!({ "op": "eval", "cell": "c2", "code": "random.seed(3)\nprintln(len(t))\nlet r = random.random()\ntime.now_ms() > 0", "line": 16 }));
    assert_eq!(pure["ok"], json!(true), "{pure}");
    assert_eq!(pure["caps"], json!([]), "{pure}");
    // a refusal is reached too, and on every tier a function calling it is
    let refused = r.ask(json!({ "op": "eval", "cell": "c3", "code": "fn put() = fs.write_file(\"out.txt\", \"x\")\nunwrap(put())", "line": 20 }));
    assert_eq!(refused["ok"], json!(false), "{refused}");
    assert_eq!(refused["caps"], json!(["fs"]), "{refused}");
    let env = r.ask(json!({ "op": "eval", "cell": "c4", "code": "fn home() = os.get_env(\"HOME\")\nlet hs = map(range(0, 300), (i) => home())\nlen(hs)", "line": 24 }));
    assert_eq!(env["caps"], json!(["env"]), "{env}");
    // the next evaluation starts afresh
    assert_eq!(r.eval("1 + 1")["caps"], json!([]));
}

#[test]
fn a_deleted_cells_names_are_dropped_and_the_notebooks_names_listed_with_their_shapes() {
    let dir = tempfile::tempdir().unwrap();
    notebook(dir.path());
    let (mut r, _) = Repl::start_with(dir.path(), &["--notebook", "nbs/sales.olmd"]);
    let a = r.ask(json!({ "op": "eval", "cell": "c1", "code": "let xs = [1, 2, 3]\nfn f(y) = y", "line": 12 }));
    assert_eq!(a["ok"], json!(true), "{a}");
    let b = r.ask(json!({ "op": "eval", "cell": "c2", "code": "let fr = ods.frame([[\"a\", ods.series([1, 2])], [\"b\", ods.series([\"x\", \"y\"])]])\nlet word = \"hello\"", "line": 16 }));
    assert_eq!(b["ok"], json!(true), "{b}");
    let v = r.ask(json!({ "op": "vars" }));
    assert_eq!(v["ok"], json!(true), "{v}");
    let vars = v["vars"].as_array().unwrap();
    let by = |n: &str| {
        vars.iter()
            .find(|x| x["name"] == json!(n))
            .cloned()
            .unwrap_or(J::Null)
    };
    // the param, then each cell's names with their cell
    assert_eq!(by("n")["param"], json!(true), "{v}");
    assert_eq!(by("n")["v"]["s"], json!("2"));
    assert_eq!(by("xs")["cell"], json!("c1"));
    assert_eq!(by("xs")["v"]["k"], json!("list"));
    assert_eq!(by("xs")["v"]["n"], json!(3));
    assert_eq!(by("f")["v"]["k"], json!("fn"));
    let fr = by("fr");
    assert_eq!(fr["cell"], json!("c2"));
    assert_eq!(fr["v"]["k"], json!("frame"));
    assert_eq!(fr["v"]["n"], json!(2));
    assert_eq!(fr["v"]["table"]["cols"], json!(["a", "b"]));
    assert!(fr["bytes"].as_u64().unwrap() >= 16, "{fr}");
    assert!(by("word")["bytes"].as_u64().unwrap() >= 5);
    // a frame's handle reads its rows
    let rows =
        r.ask(json!({ "op": "expand", "h": fr["v"]["h"], "table": true, "start": 0, "count": 10 }));
    assert_eq!(rows["rows"][1]["c"], json!(["2", "y"]), "{rows}");
    // a cell deleted: its names dropped, the others kept
    let d = r.ask(json!({ "op": "drop", "cell": "c1" }));
    assert_eq!(d["dropped"], json!(["f", "xs"]), "{d}");
    assert_eq!(r.eval("xs")["ok"], json!(false));
    assert_eq!(r.eval("word")["value"]["s"], json!("hello"));
    let after = r.ask(json!({ "op": "vars" }));
    assert!(
        !after["vars"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x["name"] == json!("xs")),
        "{after}"
    );
    // a cell never evaluated: nothing to drop
    assert_eq!(
        r.ask(json!({ "op": "drop", "cell": "c7" }))["dropped"],
        json!([])
    );
}

#[test]
fn a_front_matter_read_again_binds_its_changed_params_and_its_grant_in_place() {
    let dir = tempfile::tempdir().unwrap();
    let nb = notebook(dir.path());
    let (mut r, _) = Repl::start_with(dir.path(), &["--notebook", "nbs/sales.olmd"]);
    let a = r.ask(json!({ "op": "eval", "cell": "c1", "code": "let x = n * 10", "line": 12 }));
    assert_eq!(a["ok"], json!(true));
    // the editor's unsaved text: n changed, a param added; the grant the same
    let text = std::fs::read_to_string(&nb)
        .unwrap()
        .replace("  n: 2\n", "  n: 5\n  m: \"hi\"\n");
    let rr = r.ask(json!({ "op": "reread", "text": text }));
    assert_eq!(rr["ok"], json!(true), "{rr}");
    assert_eq!(rr["params"], json!(["n", "m"]), "{rr}");
    assert_eq!(rr["grant"], json!(false), "{rr}");
    assert_eq!(r.eval("n")["value"]["s"], json!("5"));
    assert_eq!(r.eval("m")["value"]["s"], json!("hi"));
    // what a cell defined stays until it runs again
    assert_eq!(r.eval("x")["value"]["s"], json!("20"));
    // the same text again: nothing changed
    assert_eq!(
        r.ask(json!({ "op": "reread", "text": text }))["params"],
        json!([])
    );
    // a param gone, the grant changed (fs off): applied at once
    let off = text
        .replace("  m: \"hi\"\n", "")
        .replace("fs: read", "fs: false");
    let r2 = r.ask(json!({ "op": "reread", "text": off }));
    assert_eq!(r2["removed"], json!(["m"]), "{r2}");
    assert_eq!(r2["grant"], json!(true), "{r2}");
    assert_eq!(r.eval("m")["ok"], json!(false));
    let read = r.ask(json!({ "op": "eval", "cell": "c4", "code": "fs.read_file(\"data.txt\")" }));
    assert_eq!(read["ok"], json!(false), "{read}");
    // with no text, the file on disk
    let r3 = r.ask(json!({ "op": "reread" }));
    assert_eq!(r3["params"], json!(["n"]), "{r3}");
    assert_eq!(r3["grant"], json!(true), "{r3}");
    assert_eq!(r.eval("n")["value"]["s"], json!("2"));
}

#[test]
fn every_frame_column_says_its_summary() {
    let dir = project();
    let (mut r, _) = Repl::start(dir.path());
    let f = r.eval(
        "ods.frame([[\"region\", ods.series([\"east\", \"west\", \"east\", (), \"north\", \"west\", \"east\"])], [\"amount\", ods.series([1.0, 3.0, (), 5.0, 1.0, 2.0, 3.0])], [\"ok\", ods.series([true, false, true, true, (), true, false])]])",
    );
    let t = &f["value"]["table"];
    assert!(t.get("sampled").is_none(), "{t}");
    let sum = t["sum"].as_array().unwrap();
    assert_eq!(sum.len(), 3);
    assert_eq!(
        sum[0],
        json!({ "nulls": 1, "distinct": 3, "top": [["east", 3], ["west", 2], ["north", 1]] })
    );
    assert_eq!(sum[1]["nulls"], json!(1));
    assert_eq!(sum[1]["distinct"], json!(4));
    assert_eq!(
        (sum[1]["min"].clone(), sum[1]["max"].clone()),
        (json!(1.0), json!(5.0))
    );
    assert_eq!(sum[1]["mean"], json!(2.5));
    let hist = sum[1]["hist"].as_array().unwrap();
    assert_eq!(hist.len(), 20);
    assert_eq!((hist[0].clone(), hist[19].clone()), (json!(2), json!(1)));
    assert_eq!(
        sum[2],
        json!({ "nulls": 1, "distinct": 2, "top": [["true", 4], ["false", 2]] })
    );
    // a column all null: no range, every bin empty
    let n = r.eval("ods.series([(), ()]) * 1.0");
    let s0 = &n["value"]["table"]["sum"][0];
    assert!(s0.get("min").is_none(), "{s0}");
    assert_eq!(s0["nulls"], json!(2));
    // a long frame is summarised over its first million rows, quickly
    let big = r.eval("let n = 1200000\nods.frame([[\"a\", ods.series(0..n)], [\"b\", ods.series(0..n) * 0.5], [\"c\", ods.cast(ods.series(0..n) > 600000, \"String\")], [\"d\", ods.series(0..n) > 5]])");
    assert_eq!(big["ok"], json!(true), "{big}");
    let t = &big["value"]["table"];
    assert_eq!(t["sampled"], json!(1000000));
    assert_eq!(t["sum"][0]["distinct"], json!(1000000));
    assert_eq!(t["sum"][0]["max"], json!(999999.0));
    assert_eq!(t["sum"][2]["distinct"], json!(2));
}

#[test]
fn a_frames_view_is_filtered_sorted_and_summarised_by_handle() {
    let dir = project();
    let (mut r, _) = Repl::start(dir.path());
    let f = r.eval(
        "ods.frame([[\"product\", ods.series([\"gadget\", \"Gadget\", \"widget\", \"gadgetry\", \"bolt\"])], [\"region\", ods.series([\"east\", \"west\", \"north\", \"west\", ()])], [\"amount\", ods.series([120.0, 300.0, 150.0, 90.0, 500.0])], [\"qty\", ods.series([1, (), 3, 4, 5])]])",
    );
    let h = f["value"]["h"].as_u64().unwrap();
    let view = r.ask(
        json!({ "op": "expand", "h": h, "table": true, "start": 0, "count": 10,
        "filters": [{ "col": 2, "range": [100.0, null] }, { "col": 1, "values": ["east", "west"] }],
        "sort": { "col": 2, "desc": true }, "summary": true }),
    );
    assert_eq!(view["ok"], json!(true), "{view}");
    assert_eq!(view["n"], json!(2));
    assert_eq!(view["total"], json!(5));
    assert_eq!(view["rows"][0]["i"], json!(1));
    assert_eq!(view["rows"][1]["i"], json!(0));
    assert_eq!(view["sum"][2]["min"], json!(120.0));
    assert_eq!(view["sum"][0]["distinct"], json!(2));
    // contains is case-sensitive, as ods.contains is; nulls kept or dropped
    let c = r.ask(json!({ "op": "expand", "h": h, "table": true, "count": 10,
        "filters": [{ "col": 0, "contains": "gad" }] }));
    let at: Vec<J> = c["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["i"].clone())
        .collect();
    assert_eq!(at, vec![json!(0), json!(3)], "{c}");
    let nulls = r.ask(json!({ "op": "expand", "h": h, "table": true, "count": 10,
        "filters": [{ "col": 3, "nulls": true }] }));
    assert_eq!(nulls["n"], json!(1));
    assert_eq!(nulls["rows"][0]["i"], json!(1));
    let kept = r.ask(json!({ "op": "expand", "h": h, "table": true, "count": 10,
        "filters": [{ "col": 1, "values": [null, "north"] }] }));
    assert_eq!(kept["n"], json!(2), "{kept}");
    // no filters: as before, with the total
    let all = r.ask(json!({ "op": "expand", "h": h, "table": true, "count": 2 }));
    assert_eq!(
        (all["n"].clone(), all["total"].clone()),
        (json!(5), json!(5))
    );
    assert!(all.get("sum").is_none());
    // a range on a text column is the protocol's error
    let bad = r.ask(json!({ "op": "expand", "h": h, "table": true,
        "filters": [{ "col": 0, "range": [1, 2] }] }));
    assert_eq!(bad["error"]["kind"], json!("protocol"), "{bad}");
    assert!(
        bad["error"]["message"]
            .as_str()
            .unwrap()
            .contains("`product`")
    );
}

#[test]
fn a_chart_says_its_description_decimated_and_zooms_by_range() {
    let dir = project();
    let (mut r, _) = Repl::start(dir.path());
    let c = r.eval("let x = ods.series(0..10000) * 0.001\nplot.xy([[\"wave\", \"line\", x, x * x], [\"dots\", \"scatter\", x, x]], #{})");
    assert_eq!(c["ok"], json!(true), "{c}");
    let v = &c["value"];
    assert!(v["h"].is_u64(), "a chart is always held: {v}");
    let chart = &v["chart"];
    assert_eq!(chart["type"], json!("xy"));
    let line = &chart["series"][0];
    assert!(line["x"].as_array().unwrap().len() <= 1500);
    assert_eq!(line["n"], json!(10000));
    assert_eq!(line["cut"], json!(true));
    // the line keeps its ends' extremes, in x order
    let xs: Vec<f64> = line["x"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_f64().unwrap())
        .collect();
    assert!(xs.windows(2).all(|w| w[0] <= w[1]));
    assert_eq!(
        line["y"].as_array().unwrap().last().unwrap(),
        &json!(9.999 * 9.999)
    );
    assert!(chart["series"][1]["x"].as_array().unwrap().len() <= 1500);
    // zoomed past what the reply carried
    let h = v["h"].as_u64().unwrap();
    let z = r.ask(json!({ "op": "expand", "h": h, "chart": { "x0": 1.5, "x1": 2.0 } }));
    let zl = &z["chart"]["series"][0];
    assert_eq!(zl["x"].as_array().unwrap().len(), 501, "{z}");
    assert!(zl.get("cut").is_none());
    assert_eq!(zl["x"][0], json!(1.5));
    // a short chart is whole, and still held
    let s = r.eval("plot.line(ods.series([1, 2]), ods.series([3, 4]), #{})");
    assert_eq!(s["value"]["chart"]["series"][0]["y"], json!([3.0, 4.0]));
    assert!(s["value"]["h"].is_u64());
    assert_eq!(s["value"]["svg"]["bg"], json!("#fcfcfb"));
}

#[test]
fn a_gg_plot_is_drawn_as_its_scene_and_zooms_by_rendering_again() {
    let dir = project();
    let (mut r, _) = Repl::start(dir.path());
    let c = r.eval("use gg\nlet x = ods.series(0..20000) * 0.001\nlet f = ods.frame([[\"x\", x], [\"y\", x * x]])\ngg.plot(f, gg.aes(x: \"x\", y: \"y\")) |> gg.point() |> gg.labs(title: \"sq\")");
    assert_eq!(c["ok"], json!(true), "{c}");
    let v = &c["value"];
    assert_eq!(v["t"], json!("Plot"));
    assert!(v["h"].is_u64(), "a plot is held: {v}");
    assert!(v["svg"]["text"].as_str().unwrap().starts_with("<svg"));
    let chart = &v["chart"];
    assert_eq!(chart["type"], json!("scene"), "{chart}");
    assert_eq!(chart["title"], json!("sq"));
    assert_eq!(chart["zoomable"], json!(true));
    assert_eq!(chart["n"], json!(20000));
    let pts = chart["ops"].as_array().unwrap().iter().find(|o| o["op"] == json!("points")).unwrap();
    // thinned for the screen, its tooltip values beside each point
    assert!(pts["x"].as_array().unwrap().len() < 20000, "{}", pts["x"].as_array().unwrap().len());
    assert_eq!(pts["tips"][0].as_array().unwrap().len(), pts["x"].as_array().unwrap().len());
    // a zoom renders again over the range, the y axis fitted to what shows
    let h = v["h"].as_u64().unwrap();
    let z = r.ask(json!({ "op": "expand", "h": h, "chart": { "x0": 1.0, "x1": 2.0, "panel": 0 } }));
    let p = &z["chart"]["panels"][0];
    assert_eq!(p["h_axis"]["lo"], json!(1.0), "{z}");
    assert_eq!(p["h_axis"]["hi"], json!(2.0));
    assert!(p["v_axis"]["hi"].as_f64().unwrap() < 5.0);
    // an error is said, the cell's value its words
    let e = r.eval("gg.plot(f, gg.aes(x: \"nope\")) |> gg.point()");
    assert!(e["value"]["s"].as_str().unwrap().contains("no such column"), "{e}");
}

#[test]
fn a_cells_control_is_said_set_by_the_host_and_kept_while_it_fits() {
    let dir = project();
    let (mut r, hello) = Repl::start(dir.path());
    assert!(hello["ops"].as_array().unwrap().contains(&json!("set")));
    let cell = |r: &mut Repl, id: &str, code: &str| {
        r.ask(json!({ "op": "eval", "cell": id, "code": code }))
    };
    let a = cell(
        &mut r,
        "c5",
        "use ui\nlet lift = ui.slider(0.08, 0.0, 0.3, 0.01)",
    );
    assert_eq!(a["ok"], json!(true), "{a}");
    let c = &a["control"];
    assert_eq!(c["kind"], json!("slider"), "{a}");
    assert_eq!(c["name"], json!("lift"));
    assert_eq!(c["value"]["s"], json!("0.08"));
    assert_eq!(c["default"]["s"], json!("0.08"));
    assert_eq!(
        (c["min"].clone(), c["max"].clone(), c["step"].clone()),
        (json!(0.0), json!(0.3), json!(0.01))
    );
    // the host sets it: bound in place, and kept when the cell runs again
    let s = r.ask(json!({ "op": "set", "cell": "c5", "value": "0.12" }));
    assert_eq!(
        s,
        json!({ "id": s["id"], "ok": true, "eval": s["eval"], "name": "lift", "value": { "k": "float", "t": "Float", "s": "0.12" } })
    );
    assert_eq!(r.eval("lift")["value"]["s"], json!("0.12"));
    let again = cell(
        &mut r,
        "c5",
        "use ui\nlet lift = ui.slider(0.08, 0.0, 0.3, 0.01)",
    );
    assert_eq!(again["control"]["value"]["s"], json!("0.12"), "{again}");
    assert_eq!(r.eval("lift")["value"]["s"], json!("0.12"));
    // out of range, or the wrong kind of value, is refused
    let out = r.ask(json!({ "op": "set", "cell": "c5", "value": "0.5" }));
    assert_eq!(out["error"]["kind"], json!("invalid"), "{out}");
    let bad = r.ask(json!({ "op": "set", "cell": "c5", "value": "\"x\"" }));
    assert_eq!(bad["error"]["kind"], json!("invalid"), "{bad}");
    // a set value reaches nothing
    let imp = r
        .ask(json!({ "op": "set", "cell": "c5", "value": "unwrap(fs.read_file(\"olang.toml\"))" }));
    assert_eq!(imp["ok"], json!(false), "{imp}");
    // a changed default forgets the host's value
    let changed = cell(
        &mut r,
        "c5",
        "use ui\nlet lift = ui.slider(0.1, 0.0, 0.3, 0.01)",
    );
    assert_eq!(changed["control"]["value"]["s"], json!("0.1"), "{changed}");
    let again = cell(
        &mut r,
        "c5",
        "use ui\nlet lift = ui.slider(0.1, 0.0, 0.3, 0.01)",
    );
    assert_eq!(again["control"]["value"]["s"], json!("0.1"));
    // so does a range it no longer fits
    r.ask(json!({ "op": "set", "cell": "c5", "value": "0.25" }));
    let narrower = cell(
        &mut r,
        "c5",
        "use ui\nlet lift = ui.slider(0.1, 0.0, 0.2, 0.01)",
    );
    assert_eq!(
        narrower["control"]["value"]["s"],
        json!("0.1"),
        "{narrower}"
    );
    // an Int slider takes an Int; a select one of its options
    let n = cell(&mut r, "c6", "use ui\nlet top_n = ui.slider(3, 1, 10, 1)");
    assert_eq!(n["control"]["min"], json!(1));
    let f = r.ask(json!({ "op": "set", "cell": "c6", "value": "2.5" }));
    assert_eq!(f["error"]["kind"], json!("invalid"), "{f}");
    let sel = cell(
        &mut r,
        "c7",
        "use ui\nlet region = ui.select(\"west\", [\"east\", \"west\"])",
    );
    assert_eq!(sel["control"]["options"][0]["s"], json!("east"), "{sel}");
    assert_eq!(
        r.ask(json!({ "op": "set", "cell": "c7", "value": "\"north\"" }))["error"]["kind"],
        json!("invalid")
    );
    assert_eq!(
        r.ask(json!({ "op": "set", "cell": "c7", "value": "\"east\"" }))["ok"],
        json!(true)
    );
    // drop forgets it
    r.ask(json!({ "op": "drop", "cell": "c7" }));
    let back = cell(
        &mut r,
        "c7",
        "use ui\nlet region = ui.select(\"west\", [\"east\", \"west\"])",
    );
    assert_eq!(back["control"]["value"]["s"], json!("west"), "{back}");
    // a brush carries its chart's description; set takes a range or ()
    let b = cell(
        &mut r,
        "c8",
        "use ui\nlet span = ui.brush(plot.line(ods.series([1, 2, 3]), ods.series([2, 4, 3]), #{}))",
    );
    assert_eq!(b["control"]["chart"]["type"], json!("xy"), "{b}");
    assert_eq!(b["control"]["value"]["k"], json!("unit"));
    let sb = r.ask(json!({ "op": "set", "cell": "c8", "value": "[2.0, 5.5]" }));
    assert_eq!(sb["ok"], json!(true), "{sb}");
    assert_eq!(
        r.ask(json!({ "op": "set", "cell": "c8", "value": "()" }))["ok"],
        json!(true)
    );
    // several names: the control is said with no name, and set refuses it
    let two = cell(&mut r, "c9", "use ui\nlet t = ui.toggle(true)\nlet u = 1");
    assert!(two["control"].get("name").is_none(), "{two}");
    assert_eq!(
        r.ask(json!({ "op": "set", "cell": "c9", "value": "false" }))["error"]["kind"],
        json!("invalid")
    );
    // no control: nothing said, and set is the protocol's error
    let plain = cell(&mut r, "c10", "let z = 1");
    assert!(plain.get("control").is_none());
    assert_eq!(
        r.ask(json!({ "op": "set", "cell": "c10", "value": "2" }))["error"]["kind"],
        json!("protocol")
    );
}

#[test]
fn a_param_that_is_a_controls_spec_binds_its_value_and_is_read_again() {
    let dir = tempfile::tempdir().unwrap();
    let nb = dir.path().join("p.olmd");
    let text = |v: u32| {
        format!(
            "---\nparams:\n  top_n: {{value: {}, min: 1, max: 10, step: 1}}\n  since: {{value: \"30 days ago\", options: [\"7 days ago\", \"30 days ago\"]}}\n---\n\n```olang\nlet t = top_n\n```\n",
            v
        )
    };
    std::fs::write(&nb, text(3)).unwrap();
    let (mut r, hello) = Repl::start_with(dir.path(), &["--notebook", "p.olmd"]);
    assert_eq!(hello["notebook"]["problems"], json!([]), "{hello}");
    assert_eq!(r.eval("top_n")["value"]["s"], json!("3"));
    assert_eq!(r.eval("since")["value"]["s"], json!("30 days ago"));
    let again = r.ask(json!({ "op": "reread", "text": text(7) }));
    assert_eq!(again["params"], json!(["top_n"]), "{again}");
    assert_eq!(r.eval("top_n")["value"]["s"], json!("7"));
    let out = r.ask(json!({ "op": "reread", "text": text(12) }));
    assert!(
        out["problems"][0]["message"]
            .as_str()
            .unwrap()
            .contains("outside its range"),
        "{out}"
    );
}

#[test]
fn a_columns_distinct_values_are_paged_commonest_first() {
    let dir = project();
    let (mut r, hello) = Repl::start(dir.path());
    assert!(
        hello["features"]
            .as_array()
            .unwrap()
            .contains(&json!("distinct")),
        "{hello}"
    );
    let f = r.eval(
        "ods.frame([[\"city\", ods.series(map(range(0, 40), (i) => \"c\" + to_string(i % 9)) + [(), ()])], [\"n\", ods.series(range(0, 42))]])",
    );
    let h = f["value"]["h"].as_u64().unwrap();
    // nine cities and the nulls: more than a summary's six
    let page =
        r.ask(json!({ "op": "expand", "h": h, "distinct": { "col": 0 }, "start": 0, "count": 4 }));
    assert_eq!(page["ok"], json!(true), "{page}");
    assert_eq!(page["n"], json!(10), "{page}");
    assert_eq!(page["values"].as_array().unwrap().len(), 4);
    // the commonest first, ties by their text
    assert_eq!(page["values"][0], json!(["c0", 5]), "{page}");
    assert_eq!(page["values"][3], json!(["c3", 5]), "{page}");
    let rest =
        r.ask(json!({ "op": "expand", "h": h, "distinct": { "col": 0 }, "start": 8, "count": 4 }));
    assert_eq!(rest["values"], json!([["c8", 4], [null, 2]]), "{rest}");
    // contains narrows them (a null holds no text)
    let some = r.ask(
        json!({ "op": "expand", "h": h, "distinct": { "col": 0, "contains": "c1" }, "count": 50 }),
    );
    assert_eq!(some["values"], json!([["c1", 5]]), "{some}");
    let bad = r.ask(json!({ "op": "expand", "h": h, "distinct": { "col": 7 } }));
    assert_eq!(bad["error"]["kind"], json!("protocol"), "{bad}");
}

#[test]
fn a_cells_run_profiled_says_its_profile_with_the_cells_lines() {
    let dir = tempfile::tempdir().unwrap();
    let nb = notebook(dir.path());
    let (mut r, hello) = Repl::start_with(dir.path(), &["--notebook", nb.to_str().unwrap()]);
    assert!(
        hello["features"]
            .as_array()
            .unwrap()
            .contains(&json!("profile")),
        "{hello}"
    );
    let code = "fn busy(n) = {\n    let mut s = 0\n    for i in 0..n { s = s + i % 7 }\n    s\n}\nbusy(300000)";
    let a = r.ask(json!({ "op": "eval", "cell": "c9", "code": code, "line": 30, "profile": true }));
    assert_eq!(a["ok"], json!(true), "{a}");
    let p = &a["profile"];
    assert!(p["samples"].as_u64().unwrap_or(0) > 0, "{p}");
    let busy = p["frames"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == json!("busy"))
        .unwrap_or_else(|| panic!("busy sampled: {p}"));
    // where the cell declared it: the notebook's line
    assert_eq!(busy["line"], json!(30), "{busy}");
    // an evaluation not asked says none
    let b = r.ask(json!({ "op": "eval", "cell": "c9", "code": "busy(10)", "line": 30 }));
    assert!(b.get("profile").is_none(), "{b}");
}

#[test]
fn a_recorded_run_replays_from_its_trace_and_says_a_difference() {
    let dir = tempfile::tempdir().unwrap();
    let nbdir = dir.path().join("nbs");
    std::fs::create_dir_all(&nbdir).unwrap();
    std::fs::write(nbdir.join("data.csv"), "a,b\n1,2\n3,4\n").unwrap();
    std::fs::write(
        nbdir.join("rec.olmd"),
        "---\ncapabilities:\n  fs: read\n---\n\n```olang\nlet f = unwrap(ods.read_csv_file(\"data.csv\"))\nlet t = unwrap(fs.read_file(\"data.csv\"))\n```\n\n```olang\nlet r = random.random()\nods.n_rows(f) + len(t)\n```\n",
    )
    .unwrap();
    let cells = [
        (
            "c1",
            "let f = unwrap(ods.read_csv_file(\"data.csv\"))\nlet t = unwrap(fs.read_file(\"data.csv\"))",
            7,
        ),
        ("c2", "let r = random.random()\nods.n_rows(f) + len(t)", 12),
    ];
    let trace = dir.path().join("run.olt");
    let (mut r, hello) = Repl::start_with(dir.path(), &["--notebook", "nbs/rec.olmd"]);
    assert!(
        hello["features"]
            .as_array()
            .unwrap()
            .contains(&json!("record")),
        "{hello}"
    );
    assert_eq!(r.ask(json!({ "op": "record" }))["recording"], json!(true));
    let mut said = Vec::new();
    for (c, code, line) in cells {
        let a = r.ask(json!({ "op": "eval", "cell": c, "code": code, "line": line }));
        assert_eq!(a["ok"], json!(true), "{a}");
        said.push(a["record"]["events"].as_u64().unwrap());
    }
    let random = r.eval("r")["value"]["s"].clone();
    assert_eq!(
        said,
        vec![2, 1],
        "a Frame read and a file read; one random draw"
    );
    let w = r.ask(json!({ "op": "record", "path": trace }));
    assert_eq!(w["ok"], json!(true), "{w}");
    assert_eq!(w["cells"], json!(2));
    drop(r);
    // anywhere: the data gone, the trace serves it
    std::fs::remove_file(nbdir.join("data.csv")).unwrap();
    let (mut k, _) = Repl::start_with(dir.path(), &["--notebook", "nbs/rec.olmd"]);
    let rp = k.ask(json!({ "op": "replay", "path": trace }));
    assert_eq!(rp["ok"], json!(true), "{rp}");
    for (c, code, line) in cells {
        let a = k.ask(json!({ "op": "eval", "cell": c, "code": code, "line": line }));
        assert_eq!(a["ok"], json!(true), "{a}");
        assert!(a["replay"]["diverged"].is_null(), "{a}");
        if c == "c2" {
            assert_eq!(a["value"]["s"], json!("14"), "{a}");
        }
    }
    assert_eq!(k.eval("r")["value"]["s"], random, "the same random draw");
    // a cell changed since: its replay says it diverged
    let d = k.ask(json!({ "op": "eval", "cell": "c2", "code": "let r = random.random() + random.random()\n1", "line": 12 }));
    assert!(
        d["replay"]["diverged"]
            .as_str()
            .unwrap_or("")
            .contains("diverged")
            || d["error"]["message"]
                .as_str()
                .unwrap_or("")
                .contains("diverged"),
        "{d}"
    );
    assert_eq!(
        k.ask(json!({ "op": "replay", "stop": true }))["was"],
        json!(true)
    );
}

#[test]
fn an_output_as_html_and_the_notebooks_page() {
    let dir = tempfile::tempdir().unwrap();
    notebook(dir.path());
    let (mut r, _) = Repl::start_with(dir.path(), &["--notebook", "nbs/sales.olmd"]);
    let a = r.ask(json!({ "op": "eval", "cell": "c1", "code": "use ui\nlet x = ui.slider(3, 0, 10, 1)", "line": 12, "html": true }));
    let html = a["html"].as_str().unwrap();
    assert!(
        html.contains("type=\"range\"") && html.contains("data-cell=\"c1\" data-name=\"x\""),
        "{html}"
    );
    let p = r.ask(
        json!({ "op": "page", "mode": "app", "doc": { "cells": [{ "line": 12, "reply": a }] } }),
    );
    let page = p["html"].as_str().unwrap();
    assert!(
        page.contains("<h1>Kernel</h1>") && page.contains("<script>"),
        "{page}"
    );
}

/// A brush is set as a slider is: the range an app's page dragged (as
/// olang source), or `()` for none; its output, drawn for the app, is a
/// layer over its chart a drag is read on.
#[test]
fn a_brush_is_set_by_the_host_and_drawn_for_dragging() {
    let dir = project();
    let (mut r, _) = Repl::start(dir.path());
    let c = r.ask(json!({ "op": "eval", "cell": "c5",
        "code": "let chart = plot.scatter(ods.series([1.0, 2.0, 3.0, 4.0]), ods.series([2.0, 1.0, 4.0, 3.0]), #{})" }));
    assert_eq!(c["ok"], json!(true), "{c}");
    let a = r.ask(json!({ "op": "eval", "cell": "c6", "html": true,
        "code": "use ui\nlet window = ui.brush(chart)" }));
    assert_eq!(a["ok"], json!(true), "{a}");
    assert_eq!(a["control"]["kind"], json!("brush"), "{a}");
    let html = a["html"].as_str().unwrap_or("");
    assert!(
        html.contains("<div class=\"brush\"")
            && html.contains("data-cell=\"c6\" data-name=\"window\"")
            && html.contains("\"plot\":{"),
        "{html}"
    );
    let s = r.ask(json!({ "op": "set", "cell": "c6", "value": "[1.5, 3.0]" }));
    assert_eq!(s["ok"], json!(true), "{s}");
    let n = r.eval("len(filter([1.0, 2.0, 3.0, 4.0], (x) => window != () && x >= window[0] && x <= window[1]))");
    assert_eq!(n["value"]["s"], json!("2"), "{n}");
    let again = r.ask(json!({ "op": "eval", "cell": "c6", "html": true,
        "code": "use ui\nlet window = ui.brush(chart)" }));
    assert!(
        again["html"]
            .as_str()
            .unwrap_or("")
            .contains("data-x0=\"1.5\" data-x1=\"3\""),
        "{again}"
    );
    let cleared = r.ask(json!({ "op": "set", "cell": "c6", "value": "()" }));
    assert_eq!(cleared["ok"], json!(true), "{cleared}");
    assert_eq!(r.eval("window")["value"]["s"], json!("()"));
}

/// `fields` says what a value held by name is made of — a Frame's columns
/// with their types, a map's keys, a record's fields, a list of maps'
/// columns, keys walked by `path` — and, given the text before an editor's
/// caret (`code`), what completes there: a column's place read as the
/// language server reads it, the receiver's chain followed from the values
/// held, a name not held from `known` (an editor's cached copy).
#[test]
fn a_values_fields_are_said_by_name_and_at_a_caret() {
    let dir = tempfile::tempdir().unwrap();
    notebook(dir.path());
    let (mut r, hello) = Repl::start_with(dir.path(), &["--notebook", "nbs/sales.olmd"]);
    assert!(
        hello["ops"].as_array().unwrap().contains(&json!("fields")),
        "{hello}"
    );
    let a = r.ask(json!({ "op": "eval", "cell": "c1", "line": 12, "code":
        "let sales = ods.read_csv(\"region,amount,qty\\neast,25.5,10\\nwest,3.0,2\\n\")\nlet cfg = #{ \"title\": \"Q1\", \"inner\": #{ \"depth\": 2 } }\nlet rows = [#{ \"a\": 1, \"b\": \"x\" }]" }));
    assert_eq!(a["ok"], json!(true), "{a}");
    let f = r.ask(json!({ "op": "fields", "name": "sales" }));
    assert_eq!(f["kind"], json!("frame"), "{f}");
    assert_eq!(
        f["fields"],
        json!([{ "name": "region", "type": "String" }, { "name": "amount", "type": "Float" }, { "name": "qty", "type": "Int" }])
    );
    let m = r.ask(json!({ "op": "fields", "name": "cfg" }));
    assert_eq!(m["kind"], json!("map"), "{m}");
    assert_eq!(m["fields"][0], json!({ "name": "title", "type": "String" }));
    let p = r.ask(json!({ "op": "fields", "name": "cfg", "path": ["inner"] }));
    assert_eq!(
        p["fields"],
        json!([{ "name": "depth", "type": "Int" }]),
        "{p}"
    );
    let t = r.ask(json!({ "op": "fields", "name": "rows" }));
    assert_eq!(t["kind"], json!("table"), "{t}");
    assert_eq!(t["fields"][1], json!({ "name": "b", "type": "String" }));
    let u = r.ask(json!({ "op": "fields", "name": "nobody" }));
    assert_eq!(u["ok"], json!(false));
    assert_eq!(u["error"]["kind"], json!("unbound"), "{u}");
    // at a caret: the live value's columns, a chain followed
    let at = |r: &mut Repl, code: &str, known: J| {
        let x = r.ask(json!({ "op": "fields", "code": code, "known": known }));
        assert_eq!(x["ok"], json!(true), "{x}");
        x
    };
    let x = at(&mut r, "ods.select(sales, [\"", J::Null);
    assert_eq!(x["context"], json!(true));
    assert_eq!(x["live"], json!(true));
    assert_eq!(x["fields"].as_array().unwrap().len(), 3, "{x}");
    let x = at(
        &mut r,
        "sales\n    |> ods.group_by(\"region\", [[\"total\", \"sum\", \"amount\"]])\n    |> ods.sort_by(\"to",
        J::Null,
    );
    assert_eq!(
        x["fields"],
        json!([{ "name": "region", "type": "String" }, { "name": "total", "type": "Float" }]),
        "{x}"
    );
    assert_eq!(x["typed"], json!("to"));
    let x = at(&mut r, "cfg.", J::Null);
    assert_eq!(x["want"], json!("keys"));
    assert_eq!(x["fields"][1]["name"], json!("inner"), "{x}");
    // a frame is no map: nothing after `sales.`; a module's functions are
    // no keys (the language server completes them)
    assert_eq!(at(&mut r, "sales.", J::Null)["fields"], json!([]));
    assert_eq!(at(&mut r, "ods.", J::Null)["fields"], json!([]));
    assert_eq!(
        r.ask(json!({ "op": "fields", "name": "ods" }))["kind"],
        json!("module")
    );
    // a name not held: an editor's cached copy
    let known =
        json!({ "cached": { "kind": "frame", "fields": [{ "name": "when", "type": "String" }] } });
    let x = at(&mut r, "cached[\"", known);
    assert_eq!(
        x["fields"],
        json!([{ "name": "when", "type": "String" }]),
        "{x}"
    );
    assert_eq!(x["live"], json!(false));
    // no place for a field
    assert_eq!(at(&mut r, "println(\"hi", J::Null)["context"], json!(false));
}

#[test]
fn a_session_stays_answerable_under_what_a_program_can_throw_at_it() {
    let dir = project();
    let (mut r, _) = Repl::start(dir.path());
    // a long sleep stops at the interrupt (one sleep of the whole time
    // could not be stopped: ⌘. did nothing for its length)
    let t0 = Instant::now();
    let s = r.interrupted(
        json!({ "op": "eval", "code": "time.sleep(60000)" }),
        Duration::from_millis(300),
    );
    assert_eq!(s["error"]["kind"], json!("interrupted"), "{s}");
    assert!(t0.elapsed() < Duration::from_secs(5));
    // a flood of printing comes back bounded: its start and its end, and
    // how much was left out between them
    let flood = r.eval("for i in range(0, 300000) { print(\"line \" + to_string(i)) }\n7");
    assert_eq!(flood["value"]["s"], json!("7"), "{}", &flood.to_string()[..300]);
    let out = flood["out"].as_str().unwrap();
    assert!(out.len() < 1_200_000, "{} bytes of output in one reply", out.len());
    assert!(out.starts_with("line 0line 1"), "{}", &out[..40]);
    assert!(out.ends_with("line 299999"), "{}", &out[out.len() - 40..]);
    assert!(out.contains("not kept"), "the cut is said");
    // a deep recursion's error names its frames, not a hundred thousand
    let deep = r.eval("fn down(n) = down(n + 1) + 1\ndown(0)");
    assert_eq!(deep["ok"], json!(false));
    let frames = deep["error"]["stack"].as_array().unwrap();
    assert!(frames.len() <= 64, "{} frames", frames.len());
    assert!(deep.to_string().len() < 100_000, "{} bytes", deep.to_string().len());
    // a cell holding itself is shown (its display waited on itself)
    let c = r.eval("let c = cell(0)\ncell.set(c, [c])\nc");
    assert_eq!(c["ok"], json!(true), "{c}");
    // and the session answers after all of it
    assert_eq!(r.eval("1 + 1")["value"]["s"], json!("2"));
}

#[test]
fn a_param_that_does_not_bind_is_named_with_why_and_a_far_line_is_no_hang() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("p.olmd"),
        "---\nparams:\n  top_n: {value: 12, min: 1, max: 10, step: 1}\n---\n\n```olang\nlet t = top_n\n```\n",
    )
    .unwrap();
    let (mut r, hello) = Repl::start_with(dir.path(), &["--notebook", "p.olmd"]);
    assert_eq!(hello["notebook"]["params"], json!([]), "{hello}");
    // a cell reading it says the param and why, not "a cell not run yet"
    let c = r.ask(json!({ "op": "eval", "cell": "c1", "code": "let t = top_n", "line": 7 }));
    let m = c["error"]["message"].as_str().unwrap();
    assert!(m.contains("the param `top_n` is not bound") && m.contains("outside its range"), "{c}");
    assert_eq!(c["error"]["kind"], json!("unbound"));
    // a line far past any file: answered at once, not padded line by line
    let t0 = Instant::now();
    let far = r.ask(json!({ "op": "eval", "code": "1 + 1", "line": 99_999_999_999u64 }));
    assert_eq!(far["value"]["s"], json!("2"), "{far}");
    let deps = r.ask(json!({ "op": "deps", "cells": [{ "id": "a", "code": "let q = 1", "line": 99_999_999_999u64 }] }));
    assert!(deps.to_string().contains("\"q\""), "{deps}");
    assert!(t0.elapsed() < Duration::from_secs(5), "{:?}", t0.elapsed());
}
