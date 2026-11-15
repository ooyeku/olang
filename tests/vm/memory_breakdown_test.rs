//! `runtime.memory()` splits the heap into what the loaded program holds
//! (`program`), what the runtime's own code holds once the program runs
//! (`code`: functions compiled to bytecode, their machine code, bodies
//! built on first read, bridge interpreters) and the rest (`values`).
//!
//! `values` was the heap minus the program alone (OL-418), so everything
//! the tiers compiled after load counted as the program's values: about
//! 130 MB of an editor's "values" was compiled code, which sent two
//! investigations looking in the wrong place.

use std::process::Command;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};

fn run(source: &str, args: &[&str]) -> String {
    let dir = std::env::temp_dir().join(format!("olang_mem_split_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let tag: String = args
        .join("")
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect();
    let file = dir.join(format!("m_{tag}.ol"));
    std::fs::write(&file, source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_olang"))
        .args(args)
        .arg(&file)
        .env("OLANG_COMPILE_CACHE", "0")
        .output()
        .expect("run olang");
    assert!(
        out.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

const SHOW_MEM: &str = "fn show_mem() = { let m = runtime.memory(); println(to_string(m.heap) + \" \" + to_string(m.program) + \" \" + to_string(m.code) + \" \" + to_string(m.values)) }\n";

/// How much `code` must grow, at the least, for each of the 400
/// functions called: a compiled function, or a body built, is kilobytes;
/// a floor far under that keeps the check about where the bytes are
/// counted, not about how many a compile takes.
const PER_FUNCTION_FLOOR: i64 = 256;

/// A program of 400 functions, each called enough to compile, that then
/// builds a list of 100,000 strings (about 7.8 MB). It prints, before
/// the calls, after them and after the list, `heap program code values`.
fn source() -> String {
    let mut src = String::new();
    for i in 0..400 {
        src.push_str(&format!(
            "fn f{i}(x) = {{\n  let mut acc = x\n  for k in 0..3 {{ acc = acc + k * {i} + len(to_string(acc)) }}\n  if acc % 2 == 0 => acc / 2 else => acc * 3 + 1\n}}\n"
        ));
    }
    src.push_str(SHOW_MEM);
    src.push_str("show_mem()\nlet mut total = 0\nfor r in 0..3 {\n");
    for i in 0..400 {
        src.push_str(&format!("  total = total + f{i}(r)\n"));
    }
    src.push_str("}\n");
    src.push_str(
        "show_mem()\n\
         let mut xs = []\n\
         for i in 0..100000 { xs = xs + [to_string(i)] }\n\
         show_mem()\n\
         println(len(xs) + total * 0)\n",
    );
    src
}

fn figures(line: &str) -> [i64; 4] {
    let v: Vec<i64> = line
        .split_whitespace()
        .map(|n| n.parse().unwrap())
        .collect();
    [v[0], v[1], v[2], v[3]]
}

/// The three lines' figures, each checked to add up to the heap.
fn three(out: &str) -> [[i64; 4]; 3] {
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 4, "{out}");
    let f = [figures(lines[0]), figures(lines[1]), figures(lines[2])];
    for x in f {
        assert_eq!(
            x[1] + x[2] + x[3],
            x[0],
            "program + code + values = heap: {out}"
        );
    }
    f
}

/// What a run shows, whichever tier: the calls grow `code` by more than
/// the floor a function, and the list grows `values` and leaves `code`.
fn split_holds(out: &str) {
    let [start, called, listed] = three(out);
    assert!(
        called[2] - start[2] > 400 * PER_FUNCTION_FLOOR,
        "the 400 functions' code is counted under code: {out}"
    );
    assert!(
        listed[3] - called[3] > 5 << 20,
        "values hold the list: {out}"
    );
    assert!(
        (listed[2] - called[2]).abs() < 256 << 10,
        "code did not grow with the list: {out}"
    );
}

#[test]
fn compiled_code_is_counted_apart_from_the_programs_values() {
    split_holds(&run(&source(), &["--ovm-tier=1"]));
    split_holds(&run(&source(), &[]));
}

#[test]
fn without_the_tier_code_is_the_bodies_built_on_first_call() {
    split_holds(&run(&source(), &["--no-ovm"]));
}

// ── budgets (OL-606) ────────────────────────────────────────────────
//
// What a long-running program holds, held to budgets about a quarter
// above what was measured when each was set (the commits say the
// numbers): a loaded program against its source (OL-607), footprint
// against heap (OL-608), idle services and an idle task (OL-510), http
// workers under a light load (OL-172). Ignored by default; the release
// check runs them: `cargo test --test memory_breakdown_test -- --ignored`.

fn workspace(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("olang_budget_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Kill the child even when an assertion panics.
struct KillOnDrop(Child);
impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Start `main.ol` in `dir` and read its stdout up to a line `ready`:
/// the lines before it, and the running child.
fn start(dir: &Path, env: &[(&str, &str)]) -> (KillOnDrop, Vec<String>) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_olang"));
    cmd.arg("main.ol")
        .current_dir(dir)
        .env("OLANG_COMPILE_CACHE", "0")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().expect("run olang");
    let stdout = child.stdout.take().unwrap();
    let mut reader = BufReader::new(stdout);
    let mut lines = Vec::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap() == 0 {
            let mut err = String::new();
            let _ = child.stderr.take().unwrap().read_to_string(&mut err);
            panic!("the program ended before it was ready: {lines:?}\n{err}");
        }
        let line = line.trim().to_string();
        if line == "ready" {
            break;
        }
        lines.push(line);
    }
    std::thread::spawn(move || {
        let _ = std::io::copy(&mut reader, &mut std::io::sink());
    });
    (KillOnDrop(child), lines)
}

/// The figure after `label` on the line that starts with it.
fn figure(lines: &[String], label: &str) -> i64 {
    lines
        .iter()
        .find_map(|l| l.strip_prefix(label))
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("no `{label}` figure in {lines:?}"))
}

// ── the loaded program (OL-607) ─────────────────────────────────────

/// Twenty modules of a hundred shared functions each, a shared constant
/// every tenth (which ends a run of declarations): the shape of a
/// library. 338 KB of source.
fn library(dir: &Path) -> usize {
    std::fs::create_dir_all(dir.join("lib")).unwrap();
    let mut total = 0;
    for m in 0..20 {
        let mut src = String::new();
        for i in 0..100 {
            if i % 10 == 0 {
                src.push_str(&format!(
                    "share let K{i} = #{{ \"name\": \"m{m}_{i}\", \"limit\": {}, \"tags\": [\"a\", \"b\"] }}\n",
                    i * 7
                ));
            }
            src.push_str(&format!(
                "share fn f{i}(x, y = {i}) = {{\n  let mut acc = x + y\n  for k in 0..3 {{ acc = acc + k * {i} + len(to_string(acc)) }}\n  if acc % 2 == 0 => acc / 2 else => acc * 3 + 1\n}}\n"
            ));
        }
        total += src.len();
        std::fs::write(dir.join(format!("lib/m{m}.ol")), src).unwrap();
    }
    total
}

const SHOW: &str = "let m = runtime.memory()\n\
println(\"heap \" + to_string(m.heap))\n\
println(\"program \" + to_string(m.program))\n\
println(\"values \" + to_string(m.values))\n\
println(\"ready\")\n\
time.sleep(60000)\n";

/// The program's bytes for each byte of its source: 25.6 on rc.9 (8.64 MB
/// for 338 KB), 23.9 when this was set (OL-607).
const PROGRAM_PER_SOURCE: f64 = 30.0;

#[test]
#[ignore = "a memory budget: run with --ignored (the release check)"]
fn a_loaded_library_holds_under_its_budget_per_byte_of_source() {
    let dir = workspace("program");
    let source = library(&dir);
    let mut main: String = (0..20).map(|m| format!("use lib.m{m}\n")).collect();
    main.push_str(SHOW);
    std::fs::write(dir.join("main.ol"), main).unwrap();
    let (_child, lines) = start(&dir, &[]);
    let program = figure(&lines, "program ") as f64;
    let per_byte = program / source as f64;
    assert!(
        per_byte <= PROGRAM_PER_SOURCE,
        "the loaded library holds {program} bytes for {source} of source: {per_byte:.1} a byte, \
         past the budget of {PROGRAM_PER_SOURCE}"
    );
}

// ── footprint against heap (OL-608, OL-206) ─────────────────────────

/// The process's physical footprint, as `footprint` and Activity
/// Monitor count it.
#[cfg(target_os = "macos")]
fn footprint(pid: u32) -> u64 {
    let mut info: libc::rusage_info_v2 = unsafe { std::mem::zeroed() };
    let rc = unsafe {
        libc::proc_pid_rusage(
            pid as i32,
            libc::RUSAGE_INFO_V2,
            &mut info as *mut _ as *mut libc::rusage_info_t,
        )
    };
    assert_eq!(rc, 0, "proc_pid_rusage");
    info.ri_phys_footprint
}

/// The footprint past the heap, for each byte of heap, after the library
/// above loaded: rc.9 on the system allocator kept 17 MB for an 8.6 MB
/// heap. The process's fixed part (the binary's own data, stacks) is
/// allowed on top: an empty script is about 3 MB.
/// Measured when this was set: 17.4 MB for an 8.5 MB heap.
#[cfg(target_os = "macos")]
const FOOTPRINT_PER_HEAP: f64 = 1.6;
#[cfg(target_os = "macos")]
const FOOTPRINT_FIXED: u64 = 6 << 20;

#[cfg(target_os = "macos")]
#[test]
#[ignore = "a memory budget: run with --ignored (the release check)"]
fn a_process_footprint_follows_its_heap() {
    let dir = workspace("footprint");
    library(&dir);
    let mut main: String = (0..20).map(|m| format!("use lib.m{m}\n")).collect();
    main.push_str(SHOW);
    std::fs::write(dir.join("main.ol"), main).unwrap();
    let (child, lines) = start(&dir, &[]);
    // what the allocator gives back, it gives back in a moment
    std::thread::sleep(std::time::Duration::from_millis(500));
    let heap = figure(&lines, "heap ") as u64;
    let fp = footprint(child.0.id());
    let budget = (heap as f64 * FOOTPRINT_PER_HEAP) as u64 + FOOTPRINT_FIXED;
    assert!(
        fp <= budget,
        "footprint {fp} for a heap of {heap}: past the budget of {budget}"
    );
}

// ── idle services and tasks (OL-510) ────────────────────────────────

/// Ten thousand idle services, each as a service is in a library: a
/// name, its state, an empty mailbox and its handlers. OL-510's budget:
/// 10,000 in under 20 MB.
#[test]
#[ignore = "a memory budget: run with --ignored (the release check)"]
fn ten_thousand_idle_services_fit_in_twenty_megabytes() {
    let dir = workspace("services");
    std::fs::write(
        dir.join("main.ol"),
        "fn on_ping(state, msg) = (state, \"pong\")\n\
         fn on_add(state, msg) = ({ ...state, count: state.count + msg.n }, ())\n\
         let handlers = #{ \"ping\": on_ping, \"add\": on_add }\n\
         fn service(i) = { id: i, name: \"svc-\" + to_string(i), state: { count: 0, since: i }, mailbox: [], handlers: handlers }\n\
         let before = runtime.memory().values\n\
         let mut services = []\n\
         for i in 0..10000 { services = services + [service(i)] }\n\
         let after = runtime.memory().values\n\
         println(\"services \" + to_string(len(services)))\n\
         println(\"bytes \" + to_string(after - before))\n\
         println(\"ready\")\n",
    )
    .unwrap();
    let (_child, lines) = start(&dir, &[]);
    assert_eq!(figure(&lines, "services "), 10_000);
    let bytes = figure(&lines, "bytes ");
    assert!(
        bytes < 20 << 20,
        "10,000 idle services hold {bytes} bytes: past OL-510's 20 MB"
    );
}

/// What an idle spawned task holds on the heap, in a program that loaded
/// the library above: its thread's interpreter and what waiting costs. A
/// task is a thread today, and each carries a clone of the interpreter;
/// the clone shares the module tables until it changes one. 98 KB a task
/// before they were shared, 60 KB when this was set (with `use shuttle`:
/// 200 KB and 69 KB).
const IDLE_TASK_BYTES: i64 = 75_000;

#[test]
#[ignore = "a memory budget: run with --ignored (the release check)"]
fn an_idle_task_holds_under_its_budget() {
    let dir = workspace("tasks");
    library(&dir);
    let uses: String = (0..20).map(|m| format!("use lib.m{m}\n")).collect();
    std::fs::write(
        dir.join("main.ol"),
        uses + "let c = chan.new()\n\
         fn wait_for(r) = chan.recv(r)\n\
         let before = runtime.memory().heap\n\
         let tasks = map(range(0, 100), (i) => spawn wait_for(c))\n\
         time.sleep(300)\n\
         let after = runtime.memory().heap\n\
         println(\"per_task \" + to_string((after - before) / 100))\n\
         println(\"ready\")\n\
         for t in tasks { let _s = chan.send(c, 0) }\n\
         for t in tasks { let _j = task.join(t) }\n",
    )
    .unwrap();
    let (_child, lines) = start(&dir, &[]);
    let per_task = figure(&lines, "per_task ");
    assert!(
        per_task <= IDLE_TASK_BYTES,
        "an idle task holds {per_task} bytes: past its budget of {IDLE_TASK_BYTES}"
    );
}

// ── http workers (OL-172) ───────────────────────────────────────────

/// One request at a time warms one worker, not all of them: a worker
/// compiles the application for itself, so each warm worker is a copy.
#[test]
#[ignore = "a memory budget: run with --ignored (the release check)"]
fn one_request_at_a_time_warms_one_http_worker() {
    let dir = workspace("http");
    let mut src = String::new();
    for i in 0..100 {
        src.push_str(&format!(
            "fn f{i}(x) = {{ let mut acc = x\n  for k in 0..3 {{ acc = acc + k * {i} + len(to_string(acc)) }}\n  if acc % 2 == 0 => acc / 2 else => acc * 3 + 1 }}\n"
        ));
    }
    let calls: Vec<String> = (0..100).map(|i| format!("f{i}(n)")).collect();
    src.push_str(&format!("fn work(n) = {}\n", calls.join(" + ")));
    src.push_str(
        "fn warm() = {\n\
           let ws = filter(runtime.memory().tasks, (t) => starts_with(t.name, \"olang-http\"))\n\
           to_string(len(filter(ws, (t) => t.bytes > 262144))) + \" of \" + to_string(len(ws))\n\
         }\n\
         fn handle(req) = if req.path == \"/warm\" => warm() else => to_string(work(len(req.path)))\n\
         println(\"ready\")\n\
         http.serve(0, handle, #{ \"workers\": 8 })\n",
    );
    std::fs::write(dir.join("main.ol"), src).unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_olang"));
    cmd.arg("main.ol")
        .current_dir(&dir)
        .env("OLANG_COMPILE_CACHE", "0")
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = cmd.spawn().expect("run olang");
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    let child = KillOnDrop(child);
    let port = loop {
        let mut line = String::new();
        assert!(reader.read_line(&mut line).unwrap() > 0, "the server ended");
        if let Some(rest) = line.trim().strip_prefix("listening on http://127.0.0.1:") {
            break rest.parse::<u16>().unwrap();
        }
    };
    std::thread::spawn(move || {
        let _ = std::io::copy(&mut reader, &mut std::io::sink());
    });
    let get = |path: &str| -> String {
        let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        write!(s, "GET {path} HTTP/1.1\r\nHost: t\r\nConnection: close\r\n\r\n").unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        out.split("\r\n\r\n").nth(1).unwrap_or("").to_string()
    };
    for i in 0..60 {
        get(&format!("/r{i}"));
    }
    let warm = get("/warm");
    drop(child);
    let n: usize = warm.split(' ').next().unwrap().parse().unwrap();
    assert!(
        n <= 2,
        "{warm} workers hold more than 256 KB after 60 requests one at a time"
    );
}
