//! Stopping one thread's work: budgets per thread (OL-494) and
//! `task.interrupt` (OL-493); joins and full sends that wake instead of
//! polling (OL-498); a Rust panic kept to its task or boundary (OL-499);
//! the stall detector leaving waits on outside sources alone (OL-512);
//! macOS QoS classes for tasks (OL-507).
//! Every program
//! here runs twice, on the default tiers (bytecode VM and native code)
//! and on the interpreter alone (`--no-ovm`), and must print the same
//! lines on both. Runs the real binary, as a user would.

use std::path::PathBuf;
use std::process::Command;

fn olang() -> &'static str {
    env!("CARGO_BIN_EXE_olang")
}

fn workspace(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("olang_stops_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Run `source` on both tiers; its stdout, the same on each.
fn run_both(tag: &str, source: &str) -> String {
    run_both_env(tag, source, &[])
}

/// `run_both`, with environment variables set.
fn run_both_env(tag: &str, source: &str, env: &[(&str, &str)]) -> String {
    let ws = workspace(tag);
    std::fs::write(ws.join("main.ol"), source).unwrap();
    let mut outs = Vec::new();
    for flags in [&[][..], &["--no-ovm"][..]] {
        let out = Command::new(olang())
            .current_dir(&ws)
            .args(flags)
            .arg("main.ol")
            .envs(env.iter().copied())
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
        assert!(
            out.status.success(),
            "main.ol failed ({flags:?}):\nstdout: {stdout}\nstderr: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        outs.push(stdout);
    }
    assert_eq!(outs[0], outs[1], "the tiers disagree");
    outs.remove(0)
}

const HELPERS: &str = r#"
fn spin() = { let mut n = 0; while true { n = n + 1 }; n }
fn count(k) = { let mut n = 0; let mut i = 0; while i < k { n = n + i; i = i + 1 }; n }
fn ms_of(r) = map_get(r, "us") / 1000
"#;

#[test]
fn each_thread_s_budget_trips_on_its_own_deadline() {
    // One shared stack watched only its top: task A's 50 ms budget ran
    // for 1,515 ms while task B held a 1,500 ms one.
    let out = run_both(
        "own_deadline",
        &format!(
            r#"{HELPERS}
let b = spawn runtime.call_budget(spin, [], 1500)
time.sleep(20)
let a = runtime.call_budget(spin, [], 50)
println("a over=" + show(map_get(a, "over")) + " short=" + show(ms_of(a) < 400))
let br = unwrap(task.join(b))
println("b over=" + show(map_get(br, "over")) + " long=" + show(ms_of(br) >= 1400))
"#
        ),
    );
    assert_eq!(out, "a over=true short=true\nb over=true long=true\n");
}

#[test]
fn code_compiled_before_the_first_budget_still_stops() {
    // A warmed count() under a 50 ms budget ran for 2,055 ms: native
    // code compiled before any budget had no poll.
    let out = run_both(
        "warm",
        &format!(
            r#"{HELPERS}
let _warm = count(100000)
let w = runtime.call_budget(count, [10000000000], 50)
println("over=" + show(map_get(w, "over")) + " short=" + show(ms_of(w) < 400))
println(count(1000))
"#
        ),
    );
    assert_eq!(out, "over=true short=true\n499500\n");
}

#[test]
fn waits_honour_a_budget() {
    // call_budget(() => chan.recv(never), [], 50) returned after 3,004 ms.
    let out = run_both(
        "waits",
        &format!(
            r#"{HELPERS}
let never = chan.new()
let r = runtime.call_budget(() => chan.recv(never), [], 50)
println("recv " + show(map_get(r, "over")) + " " + show(ms_of(r) < 400) + " " + map_get(r, "error"))
let rt = runtime.call_budget(() => chan.recv_timeout(never, 5000), [], 50)
println("recv_timeout " + show(map_get(rt, "over")) + " " + show(ms_of(rt) < 400))
let s = runtime.call_budget(() => time.sleep(5000), [], 50)
println("sleep " + show(map_get(s, "over")) + " " + show(ms_of(s) < 400))
let full = chan.bounded(0)
let fs = runtime.call_budget(() => chan.send(full, 1), [], 50)
println("send " + show(map_get(fs, "over")) + " " + show(ms_of(fs) < 400))
let svc = chan.new()
let ask = runtime.call_budget(() => chan.ask(svc, 1), [], 50)
println("ask " + show(map_get(ask, "over")) + " " + show(ms_of(ask) < 400))
// the thread runs on normally after each budget
println(chan.recv_timeout(never, 10))
"#
        ),
    );
    assert_eq!(
        out,
        "recv true true stopped: over its time budget of 50 ms\n\
         recv_timeout true true\n\
         sleep true true\n\
         send true true\n\
         ask true true\n\
         Err(\"timed out\")\n"
    );
}

#[test]
fn a_step_budget_counts_on_every_tier() {
    // meta.eval's max_steps used to keep the evaluated code on the
    // interpreter, the only tier that counted steps.
    let out = run_both(
        "steps",
        r#"
let src = "fn spin() = { let mut n = 0; while true { n = n + 1 }; n }\nspin()"
println(meta.eval(src, #{ "max_steps": 100000 }))
let ok = "fn count(k) = { let mut n = 0; let mut i = 0; while i < k { n = n + i; i = i + 1 }; n }\ncount(1000)"
println(meta.eval(ok, #{ "max_steps": 100000 }))
println(meta.eval(src, #{ "timeout_ms": 50 }))
fn spin2() = { let mut n = 0; while true { n = n + 1 }; n }
let t = spawn runtime.call_budget(spin2, [], 100)
println(map_get(unwrap(task.join(t)), "over"))
"#,
    );
    assert_eq!(
        out,
        "Err(\"Runtime error: budget exceeded: the evaluation used up its max_steps\")\n\
         Ok(499500)\n\
         Err(\"Runtime error: budget exceeded: the evaluation ran past its timeout_ms (50 ms)\")\n\
         true\n"
    );
}

#[test]
fn an_interrupt_stops_a_task_on_every_tier_with_its_reason() {
    // olang had no task cancellation: the only interrupt API aimed at the
    // calling thread.
    let out = run_both(
        "interrupt",
        &format!(
            r#"{HELPERS}
let t = spawn spin()
time.sleep(30)
let t0 = time.monotonic()
println(task.interrupt(t, "restart"))
println(task.join(t))
println("quick " + show(time.monotonic() - t0 < 300))
let _w = count(1000)
let n = spawn count(100000000000)
time.sleep(30)
task.interrupt(n)
println(task.join(n))
let done = spawn count(3)
time.sleep(30)
println(task.interrupt(done))
"#
        ),
    );
    assert_eq!(
        out,
        concat!(
            "true\n",
            "Err(\"Runtime error: interrupted: restart\")\n",
            "quick true\n",
            "Err(\"Runtime error: interrupted\")\n",
            "false\n"
        )
    );
}

#[test]
fn an_interrupted_task_stops_waiting_at_once() {
    let out = run_both(
        "interrupt_waits",
        &format!(
            r#"{HELPERS}
let never = chan.new()
let svc = chan.new()
let full = chan.bounded(0)
let inner = spawn spin()
let p = unwrap(proc.spawn(["sleep", "30"]))
let waits = [
    spawn chan.recv(never),
    spawn chan.recv_timeout(never, 60000),
    spawn time.sleep(60000),
    spawn task.join(inner),
    spawn task.join_timeout(inner, 60000),
    spawn chan.send(full, 1),
    spawn chan.ask(svc, 1),
    spawn proc.read_line(p),
    spawn proc.wait(p),
]
time.sleep(30)
let t0 = time.monotonic()
for w in waits {{ task.interrupt(w, "stop") }}
for w in waits {{ println(task.join(w)) }}
println("quick " + show(time.monotonic() - t0 < 500))
task.interrupt(inner)
println(task.join(inner))
proc.kill(p)
"#
        ),
    );
    let stopped = "Err(\"Runtime error: interrupted: stop\")\n".repeat(9);
    assert_eq!(
        out,
        format!("{stopped}quick true\nErr(\"Runtime error: interrupted\")\n")
    );
}

#[test]
fn an_interrupt_is_asked_once_so_attempt_can_catch_it() {
    let out = run_both(
        "interrupt_once",
        &format!(
            r#"{HELPERS}
fn careful() = {{
    let first = attempt(() => spin())
    let mut k = 0
    while k < 1000 {{ k = k + 1 }}
    show(first) + " then " + show(k)
}}
let c = spawn careful()
time.sleep(20)
task.interrupt(c, "once")
println(task.join(c))
"#
        ),
    );
    assert_eq!(out, "Ok(\"Err(\\\"interrupted: once\\\") then 1000\")\n");
}

#[test]
fn a_task_in_a_system_call_raises_once_the_call_returns() {
    let out = run_both(
        "interrupt_syscall",
        r#"
fn stuck() = {
    let r = os.exec(["sleep", "1"])
    "after " + show(r)
}
let t = spawn stuck()
time.sleep(50)
println(task.interrupt(t, "go"))
println(map(task.list(), (row) => row.state))
println(task.join(t))
"#,
    );
    assert_eq!(
        out,
        "true\n[\"interrupting\"]\nErr(\"Runtime error: interrupted: go\")\n"
    );
}

#[test]
fn a_join_and_a_full_send_wake_without_polling() {
    // Both slept 1 ms between looks: a spawn plus a join took 1.3 ms
    // serially, and backpressure added about 1 ms per send. The bounds
    // here are loose, for a loaded machine; quiet, they are 28-45 µs and
    // 5 µs.
    let out = run_both(
        "wakeups",
        r#"
fn one(x) = x + 1
let _w = task.join(spawn one(1))
let t0 = time.monotonic()
for i in range(0, 500) { let _ = task.join(spawn one(i)) }
let per_join = (time.monotonic() - t0) * 1000.0 / 500.0
let c = chan.bounded(1)
let consumer = spawn { let mut k = 0; while k < 2000 { let _ = chan.recv(c); k = k + 1 }; k }
let t1 = time.monotonic()
for i in range(0, 2000) { chan.send(c, i) }
println(task.join(consumer))
let per_send = (time.monotonic() - t1) * 1000.0 / 2000.0
println("join quick " + show(per_join < 500.0))
println("send quick " + show(per_send < 300.0))
fn later() = { time.sleep(10); 7 }
let t = spawn later()
let t2 = time.monotonic()
println(task.join_timeout(t, 5000))
println("join_timeout quick " + show(time.monotonic() - t2 < 200))
"#,
    );
    assert_eq!(
        out,
        "Ok(2000)\njoin quick true\nsend quick true\nOk(Ok(7))\njoin_timeout quick true\n"
    );
}

#[test]
fn a_rust_panic_is_its_task_s_or_boundary_s_err() {
    // The panic hook aborted the process unless the panic happened inside
    // contain::catch, which only lsp and gui used: one builtin bug took
    // the whole program (and OlangStudio) down. The sites below panic
    // once each on purpose (OLANG_TEST_PANIC).
    let out = run_both_env(
        "panics",
        r#"
fn lister() = len(task.list())
println(task.join(spawn lister()))
// inside a function, so the VM's own attempt runs it on the tiers
fn guarded() = attempt(() => task.parked())
println(guarded())
let r = runtime.call_budget(() => chan.stat(chan.new()), [], 1000)
println(show(map_get(r, "ok")) + " " + map_get(r, "error"))
fn count(k) = { let mut n = 0; let mut i = 0; while i < k { n = n + i; i = i + 1 }; n }
println(attempt(() => count(10)))
println(guarded())
println(len(task.list()))
"#,
        &[("OLANG_TEST_PANIC", "task.list,task.parked,chan.stat")],
    );
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 6, "{out}");
    assert!(
        lines[0]
            .starts_with("Err(\"internal error: task olang-spawn-1: a panic injected at task.list"),
        "{out}"
    );
    assert!(
        lines[1].starts_with("Err(\"internal error: attempt: a panic injected at task.parked"),
        "{out}"
    );
    assert!(
        lines[2].starts_with(
            "false internal error: runtime.call_budget: a panic injected at chan.stat"
        ),
        "{out}"
    );
    assert_eq!(lines[3], "Ok(45)", "the program runs on");
    assert_eq!(lines[4], "Ok([])", "the same attempt runs normally after");
    assert_eq!(lines[5], "0", "the registry is usable after the panic");
}

#[test]
fn an_idle_scheduler_stays_parked_for_ten_seconds() {
    // Workers wait on the run queue, a handler waits on its reply inside
    // its lane's deadline, the timer task waits on its control channel
    // with a timeout, main waits on the workers. Nothing is ready for ten
    // seconds; then the timer closes the queue. The stall detector must
    // not call that a deadlock.
    let ws = workspace("idle");
    std::fs::write(
        ws.join("main.ol"),
        r#"
let runq = chan.new()
let control = chan.new()
let replies = chan.new()
fn worker(q) = {
    let mut n = 0
    let mut going = true
    while going { match chan.recv(q) { Ok(job) => { n = n + 1 }, Err(_) => { going = false } } }
    n
}
fn handler(r) = map_get(runtime.call_budget(() => chan.recv(r), [], 12000), "over")
fn timer(ctl, q) = {
    let start = time.monotonic_ms()
    let mut going = true
    while going {
        match chan.recv_timeout(ctl, 1000) {
            Ok(_) => { going = false }
            Err(_) => { if time.monotonic_ms() - start > 10000 => { chan.close(q); going = false } else => () }
        }
    }
    "timer done"
}
let ws = map(range(0, 4), (i) => spawn worker(runq))
let h = spawn handler(replies)
let t = spawn timer(control, runq)
println(map(ws, task.join))
println(task.join(t))
chan.send(replies, "late")
println(task.join(h))
"#,
    )
    .unwrap();
    let started = std::time::Instant::now();
    let out = Command::new(olang())
        .current_dir(&ws)
        .arg("main.ol")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "stdout: {stdout}\nstderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        stdout,
        "[Ok(0), Ok(0), Ok(0), Ok(0)]\nOk(\"timer done\")\nOk(false)\n"
    );
    assert!(started.elapsed() >= std::time::Duration::from_secs(10));
}

#[test]
fn a_wait_inside_a_budget_runs_to_its_deadline() {
    // The deadline is a wake that will come: the stall detector aborted
    // this at 0.5 s, every thread being parked.
    let out = run_both(
        "budget_wait",
        r#"
let never = chan.new()
let r = runtime.call_budget(() => chan.recv(never), [], 1500)
println(show(map_get(r, "over")) + " " + show(map_get(r, "us") > 1400000))
println(meta.eval("chan.recv(chan.new())", #{ "timeout_ms": 1200 }))
"#,
    );
    assert_eq!(
        out,
        "true true\nErr(\"Runtime error: budget exceeded: the evaluation ran past its timeout_ms (1200 ms)\")\n"
    );
}

#[test]
fn a_wait_on_a_channel_fed_from_outside_is_no_deadlock() {
    // gui.events() is fed by the platform's thread, outside the program's
    // threads: a receive on it may still be answered.
    let ws = workspace("outside_feed");
    std::fs::write(
        ws.join("main.ol"),
        "let ev = gui.events()\nprintln(\"waiting\")\nprintln(chan.recv(ev))\n",
    )
    .unwrap();
    let mut child = Command::new(olang())
        .current_dir(&ws)
        .arg("main.ol")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(1500));
    let still = child.try_wait().unwrap();
    let _ = child.kill();
    let out = child.wait_with_output().unwrap();
    assert!(
        still.is_none(),
        "it ended: {still:?}\nstderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "waiting\n");
}

#[test]
fn qos_classes_are_set_and_read() {
    let out = run_both(
        "qos",
        r#"
println(task.qos())
println(task.set_qos("utility"))
println(task.qos())
let t = spawn task.qos()
println(task.join(t))
fn idle(c) = { let _ = chan.recv(c); task.qos() }
let c = chan.new()
let w = spawn idle(c)
time.sleep(20)
println(task.set_qos(w, "background"))
time.sleep(20)
chan.send(c, 1)
println(task.join(w))
println(attempt(() => task.set_qos("fast")))
"#,
    );
    let expect = if cfg!(target_os = "macos") {
        "default\nutility\nutility\nOk(\"utility\")\nbackground\nOk(\"background\")\n"
    } else {
        "default\nutility\ndefault\nOk(\"default\")\nbackground\nOk(\"default\")\n"
    };
    assert!(out.starts_with(expect), "{out}");
    assert!(out.contains("no QoS class \\\"fast\\\""), "{out}");
}

#[cfg(target_os = "macos")]
#[test]
fn a_background_task_is_scheduled_below_an_interactive_one_under_load() {
    // Twice as many busy tasks as cores, half of them background and half
    // user-interactive: the interactive half gets most of the CPU.
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(8);
    let ws = workspace("qos_load");
    std::fs::write(
        ws.join("main.ol"),
        format!(
            r#"
fn busy(class, until) = {{
    task.set_qos(class)
    let mut n = 0
    while time.monotonic_ms() < until {{
        let mut i = 0
        while i < 10000 {{ i = i + 1 }}
        n = n + 1
    }}
    n
}}
let until = time.monotonic_ms() + 1500
let bg = map(range(0, {cores}), (i) => spawn busy("background", until))
let fg = map(range(0, {cores}), (i) => spawn busy("user-interactive", until))
let sum = (ts) => fold(map(ts, (t) => unwrap(task.join(t))), 0, (a, b) => a + b)
let b = sum(bg)
let f = sum(fg)
println("interactive " + show(f) + " background " + show(b))
println(f > b * 2)
"#
        ),
    )
    .unwrap();
    let out = Command::new(olang())
        .current_dir(&ws)
        .arg("main.ol")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(stdout.ends_with("true\n"), "{stdout}");
}
