//! A closure resolves its free top-level names in the file it was
//! written in, wherever it runs: handed to another module's function and
//! run there by `map`, `par_map`, `par_filter`, `spawn`, a channel's
//! sender, `col.max_by`, `cell.update`, `runtime.call_budget` or an
//! `http.serve` handler, on every tier.
//!
//! The compiled tier runs a callback a builtin calls back into on its
//! bridge interpreter, and `par_map` and `spawn` there clone that bridge
//! for their workers. The bridge was never told which file is the
//! program, so it took a closure the program wrote for some module's, and
//! read a name the program binds at its top level by name in the
//! function table: a module's like-named function (`routes`), silently.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

fn olang() -> &'static str {
    env!("CARGO_BIN_EXE_olang")
}

const TIERS: [&[&str]; 3] = [&["--no-ovm"], &["--ovm-tier=1"], &[]];

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("olang_home_scope_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("lib")).unwrap();
    dir
}

fn run_in(dir: &Path, args: &[&str]) -> (String, String) {
    let out = Command::new(olang())
        .args(args)
        .current_dir(dir)
        .env("OLANG_HOME", dir.join(".home"))
        .output()
        .expect("run olang");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

/// Write `files` (path, source) under a fresh folder and run `main.ol`
/// there on every tier: each prints `expected`.
fn agrees(tag: &str, files: &[(&str, &str)], expected: &str) {
    let dir = scratch(tag);
    for (path, source) in files {
        std::fs::write(dir.join(path), source).unwrap();
    }
    for flags in TIERS {
        let mut args: Vec<&str> = flags.to_vec();
        args.push("main.ol");
        let (out, err) = run_in(&dir, &args);
        assert_eq!(
            out.trim_end(),
            expected.trim_end(),
            "flags {flags:?}\nstderr: {err}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// The module that runs other files' closures. It binds every name the
/// program does — a function `routes`, a value `label`, a function
/// `fmtmod` where the program has a module alias, a function `helper` —
/// so a name looked up in the wrong file answers wrongly rather than not
/// at all.
const RUNNER: &str = r#"
share fn routes() = "runner's routes"
let label = "runner's label"
fn helper() = "runner's helper"
fn fmtmod() = "runner's fmtmod"
share fn run_seq(f) = map([1], (x) => f())
share fn run_par(f) = par_map([1], (x) => f())
share fn run_par_wide(f) = par_map(range(0, 3000), (x) => f())[2999]
share fn run_par_filter(f) = len(par_filter([1, 2], (x) => f() == "main's routes"))
share fn run_spawn(f) = unwrap(task.join(spawn f()))
share fn run_chan(f) = {
    let ch = chan.new()
    let t = spawn { chan.send(ch, f()); "sent" }
    task.join(t)
    unwrap(chan.recv(ch))
}
share fn run_max_by(f) = col.max_by(["a", "bb"], (x) => if f() == "main's routes" => len(x) else => 0 - len(x))
share fn run_cell(f) = {
    let c = cell(0)
    cell.update(c, (v) => f())
    cell.get(c)
}
share fn run_budget(f) = runtime.call_budget(f, [], 5000).value
share fn own_label() = par_map([1], (x) => label)
share fn adder() = (y) => label + y
"#;

const FMTMOD: &str = r#"share fn fmt(x) = "fmtmod's fmt""#;

#[test]
fn a_program_closure_reads_the_program_s_names_on_every_off_home_path() {
    let main = r#"
use lib.runner { run_seq, run_par, run_par_wide, run_par_filter, run_spawn, run_chan, run_max_by, run_cell, run_budget }
use lib.fmtmod {}
let routes = "main's routes"
let label = "main's label"
fn helper() = "main's helper"
let hook = () => routes
println(run_seq(hook))
println(run_par(hook))
println(run_par_wide(hook))
println(run_par_filter(hook))
println(run_spawn(hook))
println(run_chan(hook))
println(run_max_by(hook))
println(run_cell(hook))
println(run_budget(hook))
"#;
    agrees(
        "paths",
        &[
            ("main.ol", main),
            ("lib/runner.ol", RUNNER),
            ("lib/fmtmod.ol", FMTMOD),
        ],
        r#"["main's routes"]
["main's routes"]
main's routes
2
main's routes
main's routes
bb
main's routes
main's routes"#,
    );
}

#[test]
fn a_value_a_function_and_a_module_alias_each_resolve_at_home() {
    let main = r#"
use lib.runner { run_par, run_spawn, run_max_by }
use lib.fmtmod {}
let routes = "main's routes"
let label = "main's label"
fn helper() = "main's helper"
println(run_par(() => label))
println(run_par(() => helper()))
println(run_par(() => fmtmod.fmt(1)))
println(run_spawn(() => label + "/" + helper()))
println(run_max_by(() => routes))
fn named() = routes + "/" + label
println(run_par(named))
"#;
    agrees(
        "kinds",
        &[
            ("main.ol", main),
            ("lib/runner.ol", RUNNER),
            ("lib/fmtmod.ol", FMTMOD),
        ],
        r#"["main's label"]
["main's helper"]
["fmtmod's fmt"]
main's label/main's helper
bb
["main's routes/main's label"]"#,
    );
}

#[test]
fn a_local_shadows_and_a_returned_closure_keeps_its_home() {
    let main = r#"
use lib.runner { run_par, run_spawn, own_label, adder }
let routes = "main's routes"
let label = "main's label"
println(run_par(() => { let routes = "a local"; routes }))
fn shadow(routes) = run_par(() => routes)
println(shadow("a parameter"))
println(run_par(() => () => routes)[0]())
println(run_spawn(() => () => label)())
fn wrap(f) = () => routes + "/" + f()
println(run_par(wrap(() => label)))
println(run_par(() => run_par(() => label)))
println(own_label())
println(run_par(() => adder()("!")))
println(run_spawn(() => adder()("?")))
"#;
    agrees(
        "shapes",
        &[
            ("main.ol", main),
            ("lib/runner.ol", RUNNER),
            ("lib/fmtmod.ol", FMTMOD),
        ],
        r#"["a local"]
["a parameter"]
main's routes
main's label
["main's routes/main's label"]
[["main's label"]]
["runner's label"]
["runner's label!"]
runner's label?"#,
    );
}

#[test]
fn a_module_closure_run_by_another_module_reads_its_own_module() {
    // Neither side is the program: lib/a's closure, run by lib/b on a
    // worker, reads lib/a's `label`, though lib/b binds one too.
    let a = r#"
use lib.b { run_par }
let label = "a's label"
share fn go() = run_par(() => label)
"#;
    let b = r#"
let label = "b's label"
share fn label_fn() = label
share fn run_par(f) = par_map([1], (x) => f())
"#;
    let main = r#"
use lib.a { go }
let label = "main's label"
println(go())
"#;
    agrees(
        "two_modules",
        &[("main.ol", main), ("lib/a.ol", a), ("lib/b.ol", b)],
        r#"["a's label"]"#,
    );
}

struct KillOnDrop(Child);
impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn an_http_handler_a_module_serves_reads_the_program_s_names() {
    // Shuttle's shape: the module owns `http.serve` and a `routes`
    // function; the program hands it a hook that reads its own `routes`.
    let runner = r#"
share fn routes() = "runner's routes"
share fn serve(hook) = http.serve(0, (req) => hook(), #{ "workers": 2 })
"#;
    let main = r#"
use lib.runner { serve }
let routes = "main's routes"
serve(() => routes)
"#;
    let dir = scratch("http");
    std::fs::write(dir.join("main.ol"), main).unwrap();
    std::fs::write(dir.join("lib/runner.ol"), runner).unwrap();
    for flags in TIERS {
        let mut child = Command::new(olang())
            .args(flags.iter())
            .arg("main.ol")
            .current_dir(&dir)
            .env("OLANG_HOME", dir.join(".home"))
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn server");
        let mut reader = BufReader::new(child.stdout.take().expect("stdout"));
        let server = KillOnDrop(child);
        let mut line = String::new();
        let port = loop {
            line.clear();
            let n = reader.read_line(&mut line).expect("read server stdout");
            assert!(n > 0, "server exited before reporting its port ({flags:?})");
            if let Some(rest) = line.trim().strip_prefix("listening on http://127.0.0.1:") {
                break rest.parse::<u16>().expect("port");
            }
        };
        std::thread::spawn(move || {
            let _ = std::io::copy(&mut reader, &mut std::io::sink());
        });
        for _ in 0..3 {
            let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            stream
                .write_all(b"GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
                .unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).unwrap();
            assert!(
                response.ends_with("main's routes"),
                "flags {flags:?}: {response}"
            );
        }
        drop(server);
    }
    let _ = std::fs::remove_dir_all(&dir);
}
