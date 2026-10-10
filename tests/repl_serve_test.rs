//! `olang repl --serve`: the REPL protocol over the real binary's stdio —
//! the handshake, values by kind with handles and `expand`, a file's
//! scope (its declarations, a parameter that is not bound, errors with
//! frames), what a program prints, interrupting a tight loop on every
//! tier without losing the session, a module reloaded (and a reload that
//! fails keeping the old one), a Loom view rendered headless, a
//! function's parameters bound for one evaluation (`bind`), and an
//! evaluation under no capability (`pure`) refused when it reaches out.

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
        let mut child = Command::new(env!("CARGO_BIN_EXE_olang"))
            .args(["repl", "--serve"])
            .current_dir(dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn olang repl --serve");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let mut r = Repl { child, stdin, stdout, next: 1 };
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
        assert_eq!(ack["running"], json!(true), "the interrupt found nothing running: {ack}");
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
    w("olang.toml", "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n");
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
    // a map: a handle and its entries, sorted
    let m = r.eval("#{ \"b\": [1, 2, 3], \"a\": 1.5 }");
    assert_eq!(m["value"]["k"], json!("map"));
    assert_eq!(m["value"]["entries"][0]["key"], json!("a"));
    let h = m["value"]["h"].as_u64().unwrap();
    let e = r.ask(json!({ "op": "expand", "h": h, "start": 1, "count": 10 }));
    assert_eq!(e["entries"][0]["key"], json!("b"));
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
    assert_eq!(png["value"]["img"], json!({ "format": "png", "w": 4, "h": 3 }));
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
    assert_eq!(p["error"]["message"], json!("`x` is a parameter of `scaled` — not bound here"));
    assert_eq!(p["error"]["line"], json!(7));
    // a top-level let that calls something is left for its own line
    let o = r.eval_at(&f, 30, "OPENED");
    assert_eq!(o["error"]["kind"], json!("unbound"));
    assert!(o["error"]["message"].as_str().unwrap().contains("evaluate its line (4) first"));
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
    assert_eq!(hello["features"], json!(["bind", "pure"]));
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
    let k = r.ask(json!({ "op": "eval", "code": "k * 10", "file": f, "line": 30, "bind": { "k": "5" } }));
    assert_eq!(k["value"]["s"], json!("50"));
    assert_eq!(r.eval_at(&f, 30, "k")["value"]["s"], json!("1"));
    // a value that fails says which
    let bad = r.ask(json!({ "op": "eval", "code": "x", "file": f, "line": 7, "bind": { "x": "1 / 0" } }));
    assert_eq!(bad["ok"], json!(false));
    assert!(bad["error"]["message"].as_str().unwrap().contains("the value given for `x` fails"), "{bad}");
}

#[test]
fn a_pure_evaluation_runs_under_no_capability_and_says_which_it_wanted() {
    let dir = project();
    let (mut r, _) = Repl::start(dir.path());
    let f = calc(dir.path());
    let ok = r.ask(json!({ "op": "eval", "code": "scaled(5)", "file": f, "line": 30, "pure": true }));
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
    let _ = r.eval_at(&f, 30, "fn peek(n) = if n == 0 => fs.exists(\"calc.ol\") else => peek(n - 1)");
    let _warm = r.eval_at(&f, 30, "peek(3)");
    let deep = r.ask(json!({ "op": "eval", "code": "peek(3)", "file": f, "line": 30, "pure": true }));
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
    let a = r.interrupted(json!({ "op": "eval", "code": "let mut k = 0\nwhile true { k = k + 1 }" }), wait);
    assert_eq!(a["error"]["kind"], json!("interrupted"), "{a}");
    // a function on the bytecode tier
    let b = r.interrupted(json!({ "op": "eval", "code": "spin()", "file": f, "line": 40 }), wait);
    assert_eq!(b["error"]["kind"], json!("interrupted"), "{b}");
    // native code: compiled on a small call, then spun
    let warm = r.eval_at(&f, 40, "sum_to(10)");
    assert_eq!(warm["value"]["s"], json!("45"));
    let t0 = Instant::now();
    let c = r.interrupted(json!({ "op": "eval", "code": "sum_to(100000000000)", "file": f, "line": 40 }), wait);
    assert_eq!(c["error"]["kind"], json!("interrupted"), "{c}");
    assert!(t0.elapsed() < Duration::from_secs(5));
    // a recursion
    let d = r.interrupted(json!({ "op": "eval", "code": "forever(0)", "file": f, "line": 40 }), wait);
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
    assert_eq!(r.eval_at(&f, 30, "scaled(5) + kept")["value"]["s"], json!("145"));
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
    assert_eq!(r.ask(json!({ "op": "reload", "path": util }))["ok"], json!(true));
    assert_eq!(r.ask(json!({ "op": "reload", "path": f }))["ok"], json!(true));
    assert_eq!(r.eval_at(&f, 30, "scaled(5) + kept")["value"]["s"], json!("200"));
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
    let png = base64::engine::general_purpose::STANDARD.decode(pic["png"].as_str().unwrap()).unwrap();
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
        assert!(t0.elapsed() < Duration::from_secs(5), "the child is still running");
        std::thread::sleep(Duration::from_millis(20));
    }
}
