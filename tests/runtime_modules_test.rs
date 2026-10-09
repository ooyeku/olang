//! Modules loaded at run time, as a host (olang Studio) loads its
//! plugins: `runtime.load_module(path, grant)` reads a file into the
//! running program as a module held to its own grant — attributed, as a
//! dependency's is, by the folder its code lives in — finer than a
//! dependency's (folders for `fs`, programs for `proc`) and never the
//! host's (gui, tty, loading code); reloading reads it afresh and a
//! failed reload keeps the module and grant before it;
//! `runtime.call_budget` stops a runaway call on its own thread only and
//! catches its failures with their frames; `runtime.last_error()` keeps
//! where a caught error went. Runs the real binary, as a user would.

use std::path::{Path, PathBuf};
use std::process::Command;

fn olang() -> &'static str {
    env!("CARGO_BIN_EXE_olang")
}

fn workspace(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("olang_rtmod_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::canonicalize(&dir).unwrap()
}

fn write(path: &Path, s: &str) {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p).unwrap();
    }
    std::fs::write(path, s).unwrap();
}

/// Run `host.ol` in `ws`; its stdout (the run must succeed).
fn run(ws: &Path, host: &str) -> String {
    write(&ws.join("host.ol"), host);
    let out = Command::new(olang())
        .current_dir(ws)
        .arg("host.ol")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        out.status.success(),
        "host.ol failed:\nstdout: {}\nstderr: {}",
        stdout,
        String::from_utf8_lossy(&out.stderr)
    );
    stdout
}

#[test]
fn a_module_loads_with_its_own_imports_and_reports_its_grant() {
    let ws = workspace("load");
    write(
        &ws.join("plug/hello/plugin.ol"),
        "use helper { twice }\nshare fn greet(who) = \"hello \" + who + \" \" + to_string(twice(21))\n",
    );
    write(&ws.join("plug/hello/helper.ol"), "share fn twice(n) = n * 2\n");
    let out = run(
        &ws,
        r#"let m = unwrap(runtime.load_module("plug/hello/plugin.ol", #{ "name": "hi", "fs": "read", "fs_roots": ["."], "proc": ["olang"] }))
println(m.greet("you"))
let g = runtime.module_grant("plug/hello")
println(map_get(g, "name") + " " + show(map_get(g, "fs")) + " " + show(map_get(g, "proc")) + " " + show(map_get(g, "net")))
println(show(runtime.module_grant("plug/elsewhere")))
println(show(runtime.unload_module("plug/hello")) + " " + show(runtime.module_grant("plug/hello")))
"#,
    );
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "hello you 42");
    assert_eq!(lines[1], "hi read [\"olang\"] false");
    assert_eq!(lines[2], "()");
    assert_eq!(lines[3], "true ()");
}

#[test]
fn a_reload_reads_the_file_again_and_a_failed_one_keeps_what_was() {
    let ws = workspace("reload");
    write(&ws.join("plug/a/plugin.ol"), "share fn on(s, n) = s + n\n");
    write(&ws.join("plug/b/plugin.ol"), "share fn on(s, n) = s * n\n");
    let out = run(
        &ws,
        r#"let a = unwrap(runtime.load_module("plug/a/plugin.ol", #{}))
let b = unwrap(runtime.load_module("plug/b/plugin.ol", #{}))
println(show(a.on(3, 4)) + " " + show(b.on(3, 4)))
let _w = fs.write_file("plug/a/plugin.ol", "share fn on(s, n) = s - n\n")
let a2 = unwrap(runtime.load_module("plug/a/plugin.ol", #{}))
println(show(a2.on(3, 4)) + " " + show(a.on(3, 4)) + " " + show(b.on(3, 4)))
let _w2 = fs.write_file("plug/a/plugin.ol", "share fn on(s, n) = s +\n")
let bad = runtime.load_module("plug/a/plugin.ol", #{ "net": true })
println(if is_err(bad) => "refused" else => "loaded")
println(show(map_get(runtime.module_grant("plug/a"), "net")))
println(show(a2.on(3, 4)))
"#,
    );
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "7 12", "two modules' same-named functions stay their own");
    assert_eq!(lines[1], "-1 7 12", "the reload runs the new body; the old value is unchanged");
    assert_eq!(lines[2], "refused");
    assert_eq!(lines[3], "false", "a failed reload keeps the grant before it");
    assert_eq!(lines[4], "-1", "and the module before it still runs");
}

#[test]
fn a_module_is_held_to_its_folders_its_programs_and_never_the_host() {
    let ws = workspace("grants");
    write(&ws.join("proj/in.txt"), "inside");
    write(&ws.join("outside.txt"), "outside");
    write(
        &ws.join("plug/p/plugin.ol"),
        r#"share fn read(p) = fs.read_file(p)
share fn write(p) = fs.write_file(p, "x")
share fn run(prog) = os.exec(prog, ["hi"])
share fn net() = http.get("http://127.0.0.1:9/")
share fn window() = gui.available()
share fn load() = runtime.load_module("plugin.ol", #{ "fs": true })
share fn env() = os.get_env("HOME")
share fn quit() = os.exit(3)
share fn asked() = caps.granted()
"#,
    );
    let out = run(
        &ws,
        r#"let m = unwrap(runtime.load_module("plug/p/plugin.ol", #{ "name": "p", "fs": "read", "fs_roots": ["proj"], "proc": ["echo"] }))
fn said(r) = if map_get(r, "ok") => "ok" else => map_get(r, "error")
println(said(runtime.call_budget(m.read, ["proj/in.txt"], 1000)))
println(said(runtime.call_budget(m.read, ["outside.txt"], 1000)))
println(said(runtime.call_budget(m.read, ["proj/../outside.txt"], 1000)))
println(said(runtime.call_budget(m.write, ["proj/new.txt"], 1000)))
println(said(runtime.call_budget(m.run, ["echo"], 1000)))
println(said(runtime.call_budget(m.run, ["ls"], 1000)))
println(said(runtime.call_budget(m.net, [], 1000)))
println(said(runtime.call_budget(m.window, [], 1000)))
println(said(runtime.call_budget(m.load, [], 1000)))
println(said(runtime.call_budget(m.env, [], 1000)))
println(said(runtime.call_budget(m.quit, [], 1000)))
println(show(map_get(map_get(runtime.call_budget(m.asked, [], 1000), "value"), "net")))
println(show(runtime.module_permits("plug/p", "proc.spawn", [["echo", "x"]])))
println(show(runtime.module_permits("plug/p", "proc.spawn", [["olang", "x"]])))
println(show(runtime.module_permits("plug/p", "http.get", ["http://x"])))
println(show(is_ok(fs.read_file("outside.txt"))))
"#,
    );
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "ok");
    assert!(lines[1].starts_with("capability 'fs' denied: fs.read_file reaches"), "{}", lines[1]);
    assert!(lines[1].contains("outside what module 'p' is granted"), "{}", lines[1]);
    assert!(lines[2].starts_with("capability 'fs' denied"), "a path's .. is resolved: {}", lines[2]);
    assert!(lines[3].starts_with("capability 'fs' denied: fs.write_file requires it, and module 'p'"), "{}", lines[3]);
    assert_eq!(lines[4], "ok");
    assert!(lines[5].contains("os.exec runs ls, and module 'p' may only run echo"), "{}", lines[5]);
    assert!(lines[6].starts_with("capability 'net' denied"), "{}", lines[6]);
    assert!(lines[7].starts_with("capability 'host' denied: gui.available is the host's"), "{}", lines[7]);
    assert!(lines[8].starts_with("capability 'host' denied: runtime.load_module"), "{}", lines[8]);
    assert!(lines[9].starts_with("capability 'env' denied"), "{}", lines[9]);
    assert!(lines[10].starts_with("capability 'proc' denied: os.exit ends the host"), "{}", lines[10]);
    assert_eq!(lines[11], "false", "caps.granted() answers the module's grant");
    assert_eq!(lines[12], "Ok(())");
    assert!(lines[13].starts_with("Err(\"capability 'proc' denied: proc.spawn runs olang"), "{}", lines[13]);
    assert!(lines[14].starts_with("Err(\"capability 'net' denied"), "{}", lines[14]);
    assert_eq!(lines[15], "true", "the host itself is not held to the module's grant");
}

#[test]
fn a_module_within_a_manifest_gets_no_more_than_the_app() {
    let ws = workspace("manifest");
    write(
        &ws.join("olang.toml"),
        "[package]\nname = \"host\"\nversion = \"1.0.0\"\n\n[capabilities]\nnet = false\n",
    );
    write(&ws.join("plug/p/plugin.ol"), "share fn net() = http.get(\"http://127.0.0.1:9/\")\n");
    let out = run(
        &ws,
        r#"let m = unwrap(runtime.load_module("plug/p/plugin.ol", #{ "net": true }))
let r = runtime.call_budget(m.net, [], 1000)
println(map_get(r, "error"))
"#,
    );
    assert!(out.starts_with("capability 'net' denied: http.get requires it, and module 'p'"), "{}", out);
}

#[test]
fn a_budget_stops_a_runaway_call_and_nothing_else() {
    let ws = workspace("budget");
    write(
        &ws.join("plug/r/plugin.ol"),
        "share fn spin() = {\n    let mut n = 0\n    while true { n = n + 1 }\n    n\n}\nshare fn quick(x) = x + 1\nshare fn caught() = attempt(() => spin())\n",
    );
    let out = run(
        &ws,
        r#"fn count_to(k) = {
    let mut n = 0
    while n < k { n = n + 1 }
    n
}
let m = unwrap(runtime.load_module("plug/r/plugin.ol", #{}))
let worker = spawn count_to(30000000)
let r = runtime.call_budget(m.spin, [], 40)
println(show(map_get(r, "ok")) + " " + show(map_get(r, "over")) + " " + map_get(r, "error"))
println(show(map_get(r, "us") >= 40000) + " " + show(map_get(r, "us") < 2000000))
println(show(task.join(worker)))
let q = runtime.call_budget(m.quick, [1], 40)
println(show(map_get(q, "ok")) + " " + show(map_get(q, "value")) + " " + show(map_get(q, "over")))
let c = runtime.call_budget(m.caught, [], 40)
println(show(map_get(c, "over")))
println(show(count_to(1000)))
"#,
    );
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "false true stopped: over its time budget of 40 ms");
    assert_eq!(lines[1], "true true");
    assert_eq!(lines[2], "30000000", "a task on another thread ran on through the trip");
    assert_eq!(lines[3], "true 2 false");
    assert_eq!(lines[4], "true", "a call that catches the stop meets it again and still ends");
    assert_eq!(lines[5], "1000", "the host runs on after a trip");
}

#[test]
fn a_caught_error_keeps_its_frames_with_their_lines() {
    let ws = workspace("trace");
    write(
        &ws.join("plug/t/plugin.ol"),
        "share fn boom(x) = str.trim(x)\nshare fn deeper(x) = {\n    let y = boom(x)\n    y\n}\n",
    );
    let out = run(
        &ws,
        r#"fn outer(m) = {
    let v = m.deeper(())
    v
}
let m = unwrap(runtime.load_module("plug/t/plugin.ol", #{}))
let r = runtime.call_budget(m.deeper, [()], 1000)
for f in map_get(r, "frames") { println(fs.basename(show(map_get(f, "file"))) + ":" + show(map_get(f, "line")) + " " + show(map_get(f, "fn"))) }
println("--")
let a = attempt(() => outer(m))
let le = runtime.last_error()
println(map_get(le, "message"))
for f in map_get(le, "frames") { println(fs.basename(show(map_get(f, "file"))) + ":" + show(map_get(f, "line")) + " " + show(map_get(f, "fn"))) }
println(runtime.shape(#{ "n": 1, "items": ["a"], "s": Ok(#{ "k": 2.0 }) }))
"#,
    );
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "plugin.ol:0 boom", "an expression body: named, no line");
    assert_eq!(lines[1], "plugin.ol:3 deeper");
    assert_eq!(lines[2], "--");
    assert_eq!(lines[3], "str.trim: argument 1 must be a string, got Unit");
    assert_eq!(lines[4], "plugin.ol:0 boom");
    assert_eq!(lines[5], "plugin.ol:3 deeper");
    assert_eq!(lines[6], "host.ol:2 outer");
    assert_eq!(lines[7], "host.ol:0 <lambda>");
    assert_eq!(lines[8], "{items:[String],n:Int,s:Ok({k:Float})}");
}
