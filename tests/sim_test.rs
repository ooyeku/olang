//! `sim` S1: a world runs an olang program on virtual time with seeded
//! scheduling of `spawn`, `task` and `chan` (plans/SIM.md, SIM-S1.md).
//!
//! What these hold: one seed is one run, event for event; different
//! seeds interleave differently; a known race is found by some seed and
//! replayed exactly by it (in process and through `olang sim`); sleeping
//! costs no real time; timeouts and deadlocks are the world's; and every
//! effect the world does not simulate is refused, never run for real.
//!
//! S2 (the tests after `// ─── S2`): nodes, each its own interpreter,
//! talking `http` over a simulated network with latency, loss,
//! duplication and partitions; one seed is one world, event for event;
//! a partition-only bug is found by `olang sim --seeds` and replayed by
//! `--seed`.
//!
//! S5 (the tests after `// ─── S5`): a run's events as an `ods` Frame,
//! `olang sim view` drawing a run from a plan or a file and a seed, and
//! every world on the bytecode tier giving the interpreter's runs digest
//! for digest.
//!
//! S4 (the tests after `// ─── S4`): properties that pass and break,
//! the linearizability checker on known histories, the S2 and S3 bugs
//! caught as linearizability and durability violations, `--jobs` giving
//! the same results as one process, and the shrinker taking a noisy
//! failing world down to the fault that matters, replayed bit for bit.

use olang::Value;
use olang::interpreter::Interpreter;
use olang::parser::Parser;
use std::path::PathBuf;
use std::process::Command;

fn eval(src: &str) -> Value {
    let program = Parser::new().parse(src).expect("parse");
    Interpreter::new().eval_program(program).expect("eval")
}

fn s(v: &Value) -> String {
    match v {
        Value::String(s) => s.to_string(),
        other => other.to_string(),
    }
}

fn field(v: &Value, key: &str) -> Value {
    match v {
        Value::Map(m) => m.get(key).cloned().unwrap_or(Value::Unit),
        other => panic!("expected a map, got {other}"),
    }
}

fn list(v: &Value) -> Vec<Value> {
    match v {
        Value::List(items) => items.to_vec(),
        other => panic!("expected a list, got {other}"),
    }
}

/// The lost-update protocol as a function, so a program can run it in
/// many worlds.
const RACE: &str = r#"
fn server(requests) = {
    let mut value = 0
    let mut serving = true
    while serving {
        match chan.recv(requests) {
            Ok(msg) => {
                if msg.req.op == "get" => chan.send(msg.reply, value)
                else if msg.req.op == "set" => {
                    value = msg.req.value
                    chan.send(msg.reply, value)
                }
                else => {
                    chan.send(msg.reply, value)
                    serving = false
                }
            }
            Err(e) => { serving = false }
        }
    }
    value
}
fn client(requests) = {
    let seen = unwrap(chan.ask(requests, #{ "op": "get" }))
    unwrap(chan.ask(requests, #{ "op": "set", "value": seen + 1 }))
}
fn race() = {
    let requests = chan.new()
    spawn server(requests)
    let a = spawn client(requests)
    let b = spawn client(requests)
    unwrap(task.join(a))
    unwrap(task.join(b))
    let total = unwrap(chan.ask(requests, #{ "op": "stop" }))
    assert_eq(total, 2, "lost update: counter is " + to_string(total))
    total
}
"#;

#[test]
fn one_seed_is_one_run_event_for_event() {
    let v = eval(&format!(
        "{RACE}\nlet w = sim.world(#{{ \"seed\": 9 }})\n[w.run_fn(race), w.run_fn(race), sim.world(#{{ \"seed\": 9 }}).run_fn(race)]"
    ));
    let runs = list(&v);
    let events: Vec<Value> = runs.iter().map(|r| field(r, "events")).collect();
    assert!(list(&events[0]).len() > 10, "a run has events");
    assert_eq!(events[0], events[1], "the same world, run twice");
    assert_eq!(
        events[0], events[2],
        "a world made again with the same seed"
    );
    assert_eq!(field(&runs[0], "outcome"), field(&runs[2], "outcome"));
}

#[test]
fn different_seeds_interleave_differently_and_some_seed_finds_the_race() {
    let v = eval(&format!(
        "{RACE}\nrange(1, 41) |> map((seed) => sim.world(#{{ \"seed\": seed }}).run_fn(race))"
    ));
    let runs = list(&v);
    let mut distinct: Vec<Value> = Vec::new();
    for r in &runs {
        let ev = field(r, "events");
        if !distinct.contains(&ev) {
            distinct.push(ev);
        }
    }
    assert!(
        distinct.len() >= 3,
        "40 seeds gave only {} interleaving(s)",
        distinct.len()
    );
    let failing: Vec<i64> = runs
        .iter()
        .filter(|r| s(&field(r, "outcome")) == "failed")
        .map(|r| match field(r, "seed") {
            Value::Integer(n) => n,
            _ => unreachable!(),
        })
        .collect();
    let passing = runs
        .iter()
        .filter(|r| s(&field(r, "outcome")) == "done")
        .count();
    assert!(
        !failing.is_empty(),
        "no seed among the first 40 found the race"
    );
    assert!(
        passing > 0,
        "every seed lost an update: no race, just a bug"
    );
    let failure = runs
        .iter()
        .find(|r| s(&field(r, "outcome")) == "failed")
        .unwrap();
    assert!(s(&field(failure, "error")).contains("lost update: counter is 1"));

    // The failing seed replays the failure exactly.
    let seed = failing[0];
    let again = eval(&format!(
        "{RACE}\nsim.world(#{{ \"seed\": {seed} }}).run_fn(race)"
    ));
    assert_eq!(s(&field(&again, "outcome")), "failed");
    assert_eq!(field(&again, "events"), field(failure, "events"));
}

#[test]
fn an_hour_of_sleep_takes_milliseconds() {
    let started = std::time::Instant::now();
    let v = eval(
        r#"
let run = sim.world(#{ "seed": 1, "start": 1000 }).run_fn(() => {
    let before = time.now_ms()
    let sleeper = spawn { time.sleep(sim.minutes(30)); time.monotonic_ms() }
    time.sleep(sim.hours(1))
    [before, time.now_ms(), time.monotonic_ms(), unwrap(task.join(sleeper)), dates.utc_now()]
})
[run.outcome, run.value, run.now]
"#,
    );
    let parts = list(&v);
    assert_eq!(s(&parts[0]), "done");
    assert_eq!(
        parts[1],
        Value::List(
            vec![
                Value::Integer(1000),
                Value::Integer(1000 + 3_600_000),
                Value::Integer(3_600_000),
                Value::Integer(1_800_000),
                Value::String("1970-01-01T01:00:01+00:00".to_string().into()),
            ]
            .into()
        )
    );
    assert_eq!(parts[2], Value::Integer(3_600_000));
    assert!(
        started.elapsed().as_secs_f64() < 5.0,
        "an hour of virtual sleep took {:?}",
        started.elapsed()
    );
}

#[test]
fn timeouts_fire_on_virtual_time() {
    let v = eval(
        r#"
let run = sim.world(#{ "seed": 4 }).run_fn(() => {
    let c = chan.new()
    let slow = spawn { time.sleep(500); chan.send(c, "late"); 7 }
    let first = chan.recv_timeout(c, 100)
    let at_first = time.monotonic_ms()
    let joined_early = task.join_timeout(slow, 50)
    let second = chan.recv_timeout(c, #{ "timeout_ms": 10000 })
    let at_second = time.monotonic_ms()
    [first, at_first, joined_early, second, at_second, task.join_timeout(slow, 1)]
})
run.value
"#,
    );
    let err = |m: &str| Value::Err(Box::new(Value::String(m.to_string().into())));
    let ok = |v: Value| Value::Ok(Box::new(v));
    assert_eq!(
        list(&v),
        vec![
            err("timed out"),
            Value::Integer(100),
            err("timed out"),
            ok(Value::String("late".to_string().into())),
            Value::Integer(500),
            ok(ok(Value::Integer(7))),
        ]
    );
}

#[test]
fn a_deadlock_is_reported_with_every_blocked_site() {
    let v = eval(
        r#"
let run = sim.world(#{ "seed": 2 }).run_fn(() => {
    let a = chan.new()
    let b = chan.new()
    let left = spawn { unwrap(chan.recv(a)); chan.send(b, 1) }
    let right = spawn { unwrap(chan.recv(b)); chan.send(a, 1) }
    task.join(left)
})
[run.outcome, run.error, len(run.events)]
"#,
    );
    let parts = list(&v);
    assert_eq!(s(&parts[0]), "deadlock");
    let why = s(&parts[1]);
    assert!(why.contains("task 0: task.join on task 1"), "{why}");
    assert!(why.contains("task 1: chan.recv on channel #"), "{why}");
    assert!(why.contains("task 2: chan.recv on channel #"), "{why}");
}

#[test]
fn a_deadline_stops_the_run_in_virtual_time() {
    let v = eval(
        r#"
let run = sim.world(#{ "seed": 3 }).run_fn(() => { loop { time.sleep(1000) } }, #{ "for": sim.hours(2) })
[run.outcome, run.now]
"#,
    );
    assert_eq!(
        list(&v),
        vec![
            Value::String("deadline".to_string().into()),
            Value::Integer(7_200_000)
        ]
    );
}

#[test]
fn a_rendezvous_send_waits_for_its_receiver() {
    let v = eval(
        r#"
sim.world(#{ "seed": 5 }).run_fn(() => {
    let c = chan.bounded(0)
    let t = spawn { chan.send(c, 5); time.monotonic_ms() }
    time.sleep(70)
    [unwrap(chan.recv(c)), unwrap(task.join(t))]
}).value
"#,
    );
    assert_eq!(list(&v), vec![Value::Integer(5), Value::Integer(70)]);
}

#[test]
fn randomness_comes_from_the_seed() {
    let v = eval(
        r#"
fn draws() = {
    let t = spawn [random.randint(0, 1000000), crypto.random_hex(8)]
    [random.random(), random.randint(0, 1000000), unwrap(task.join(t))]
}
[sim.world(#{ "seed": 1 }).run_fn(draws).value,
 sim.world(#{ "seed": 1 }).run_fn(draws).value,
 sim.world(#{ "seed": 2 }).run_fn(draws).value]
"#,
    );
    let runs = list(&v);
    assert_eq!(runs[0], runs[1]);
    assert_ne!(runs[0], runs[2]);
}

#[test]
fn unsimulated_effects_are_refused_and_never_reach_the_real_world() {
    let marker = std::env::temp_dir().join(format!("olang_sim_refused_{}", std::process::id()));
    let _ = std::fs::remove_file(&marker);
    let path = marker.to_string_lossy().replace('\\', "/");
    let v = eval(&format!(
        r#"
fn attempt(f) = sim.world(#{{ "seed": 1 }}).run_fn(f).error
[attempt(() => fs.watch("{path}")),
 attempt(() => fs.scan("{path}")),
 attempt(() => os.username()),
 attempt(() => proc.spawn(["true"])),
 attempt(() => os.exec("true", [])),
 attempt(() => {{ let t = spawn fs.trash("{path}"); unwrap(task.join(t)) }}),
 attempt(() => sim.world(#{{}}))]
"#
    ));
    let errors = list(&v);
    for (i, e) in errors.iter().enumerate() {
        let e = s(e);
        assert!(e.contains("refused inside a world"), "attempt {i}: {e}");
    }
    assert!(s(&errors[0]).contains("fs.watch"));
    assert!(s(&errors[6]).contains("worlds do not nest"));
    // A node's environment is its own (empty unless given): the real one
    // is never read, and setting it sets the node's.
    let home = eval(
        r#"sim.world(#{ "seed": 1 }).run_fn(() => {
    let before = [os.get_env("HOME"), os.has_env("PATH")]
    os.set_env("HOME", "/nowhere")
    before + [os.get_env("HOME")]
}).value"#,
    );
    assert_eq!(
        home.to_string(),
        r#"[Err("Environment variable 'HOME' not found"), false, Ok("/nowhere")]"#
    );
    assert_ne!(std::env::var("HOME").ok().as_deref(), Some("/nowhere"));
    // Files and databases a world writes are on its own disk: the same
    // paths outside are never touched.
    let inside = eval(&format!(
        r#"sim.world(#{{ "seed": 1 }}).run_fn(() => {{
    // the same folders as outside, made on the world's disk
    unwrap(fs.create_dir_all(fs.dirname("{path}")))
    unwrap(fs.write_file("{path}", "x"))
    let conn = unwrap(db.open("{path}.db"))
    unwrap(db.execute(conn, "CREATE TABLE t (v)", []))
    [unwrap(fs.read_file("{path}")), fs.exists("{path}.db")]
}}).value"#
    ));
    assert_eq!(inside.to_string(), r#"["x", true]"#);
    assert!(
        !marker.exists(),
        "a world's write reached the real filesystem"
    );
    assert!(!std::path::Path::new(&format!("{path}.db")).exists());
}

#[test]
fn the_world_ends_cleanly_and_the_program_goes_on() {
    // Tasks still blocked when the main program returns stop with the
    // world; the real clock is back afterwards.
    let v = eval(
        r#"
let run = sim.world(#{ "seed": 8 }).run_fn(() => {
    let c = chan.new()
    spawn { chan.recv(c) }
    spawn { loop { time.sleep(10) } }
    "main is done"
})
[run.outcome, run.value, run.tasks, run.failed_tasks, time.now_ms() > 1767225600000 + 86400000]
"#,
    );
    assert_eq!(
        list(&v),
        vec![
            Value::String("done".to_string().into()),
            Value::String("main is done".to_string().into()),
            Value::Integer(3),
            Value::Integer(0),
            Value::Boolean(true),
        ]
    );
}

#[test]
fn a_failing_task_is_an_event_and_its_join_an_err() {
    let v = eval(
        r#"
let run = sim.world(#{ "seed": 1 }).run_fn(() => {
    let t = spawn raise("boom")
    task.join(t)
})
[run.outcome, run.failed_tasks, is_err(run.value), run.events |> filter((e) => e.kind == "fail") |> map((e) => e.detail)]
"#,
    );
    let parts = list(&v);
    assert_eq!(s(&parts[0]), "done");
    assert_eq!(parts[1], Value::Integer(1));
    assert_eq!(parts[2], Value::Boolean(true));
    assert!(s(&parts[3]).contains("boom"));
}

// ─── olang sim ───────────────────────────────────────────────────────

fn olang() -> &'static str {
    env!("CARGO_BIN_EXE_olang")
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/sim")
        .join(name)
}

fn sim(args: &[&str]) -> (i32, String, String) {
    let out = Command::new(olang())
        .arg("sim")
        .args(args)
        .output()
        .expect("run olang sim");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).to_string(),
        (String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr))
            .to_string(),
    )
}

#[test]
fn olang_sim_finds_the_race_by_seed_and_the_seed_reproduces_it() {
    let file = fixture("lost_update.ol");
    let file = file.to_str().unwrap();
    let (code, _, err) = sim(&[file, "--seeds", "50"]);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("lost update"), "{err}");
    let seed: u64 = err
        .lines()
        .find_map(|l| l.split("--seed ").nth(1))
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or_else(|| panic!("no reproduce line in:\n{err}"));
    assert!(err.contains(&format!("seed {seed}: failed")), "{err}");

    // `--seed S` reproduces it, event for event, every time.
    let (code1, out1, err1) = sim(&[file, "--seed", &seed.to_string(), "--events"]);
    let (code2, out2, err2) = sim(&[file, "--seed", &seed.to_string(), "--events"]);
    assert_eq!(code1, 1, "{err1}");
    assert!(
        err1.contains("lost update: two increments, counter is 1"),
        "{err1}"
    );
    assert_eq!((code1, out1, err1), (code2, out2, err2));
}

#[test]
fn olang_sim_passes_a_seed_that_does_not_lose_the_update() {
    let file = fixture("lost_update.ol");
    let file = file.to_str().unwrap();
    let passing = (1..50)
        .map(|seed| (seed, sim(&[file, "--seed", &seed.to_string()])))
        .find(|(_, (code, _, _))| *code == 0)
        .expect("some seed keeps both increments");
    let (_, (_, out, err)) = passing;
    assert!(out.contains("counter = 2"), "{out}");
    assert!(err.contains("done at"), "{err}");
}

#[test]
fn olang_sim_reports_a_deadlock_with_its_seed() {
    let file = fixture("deadlock.ol");
    let (code, _, err) = sim(&[file.to_str().unwrap(), "--seed", "4"]);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("seed 4: deadlock"), "{err}");
    assert!(err.contains("chan.recv on channel #"), "{err}");
}

#[test]
fn olang_sim_runs_an_hour_of_virtual_time_in_a_moment() {
    let file = fixture("virtual_time.ol");
    let started = std::time::Instant::now();
    let (code, out, err) = sim(&[file.to_str().unwrap(), "--seed", "1"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("60 ticks in 3600000 ms"), "{out}");
    assert!(err.contains("done at 1h 00m 00.000s virtual"), "{err}");
    assert!(started.elapsed().as_secs_f64() < 10.0);
}

#[test]
fn olang_sim_stops_at_its_deadline() {
    let file = fixture("virtual_time.ol");
    let (code, _, err) = sim(&[file.to_str().unwrap(), "--seed", "1", "--for", "30m"]);
    assert_eq!(code, 0, "{err}");
    assert!(err.contains("deadline at 30m 00.000s virtual"), "{err}");
}

// ─── S2: nodes and the network ───────────────────────────────────────

/// Evaluate `src` as if it were a file in tests/fixtures/sim (so
/// `w.node("kv", ...)` and `use kv_client` resolve there).
fn eval_here(src: &str) -> Value {
    let program = Parser::new().parse(src).expect("parse");
    let mut i = Interpreter::new();
    i.set_current_file(&fixture("inline.ol"));
    i.eval_program(program).expect("eval")
}

fn int(v: &Value) -> i64 {
    match v {
        Value::Integer(n) => *n,
        other => panic!("expected an Int, got {other}"),
    }
}

fn kinds(run: &Value) -> Vec<String> {
    list(&field(run, "events"))
        .iter()
        .map(|e| s(&field(e, "kind")))
        .collect()
}

#[test]
fn a_client_node_talks_to_a_server_node_over_the_network() {
    let run = eval_here(
        r#"
let w = sim.world(#{ "seed": 5 })
let kv = w.node("kv", ["olang", "kv_server.ol", "7500"])
w.client("c1", () => {
    let body = (op) => json.stringify(op) |> unwrap
    let a = unwrap(http.post(kv.url + ":7500/rpc", body(#{ "op": "add", "key": "x", "by": 2, "id": "a" })))
    let again = unwrap(http.post("http://kv:7500/rpc", body(#{ "op": "add", "key": "x", "by": 2, "id": "a" })))
    let b = unwrap(http.post("http://kv:7500/rpc", body(#{ "op": "add", "key": "x", "by": 3, "id": "b" })))
    let missing = unwrap(http.get("http://kv:7500/nothing"))
    [a.status, unwrap(json.parse(a.body)).value, unwrap(json.parse(again.body)).value,
     unwrap(json.parse(b.body)).value, missing.status, a.headers["content-type"], time.monotonic_ms() > 0]
})
w.run(sim.minutes(1))
"#,
    );
    assert_eq!(s(&field(&run, "outcome")), "done", "{run}");
    let got = field(&field(&run, "value"), "c1");
    assert_eq!(
        got.to_string(),
        r#"[200, 2, 2, 5, 404, "application/json", true]"#
    );
    let history = list(&field(&run, "history"));
    assert_eq!(history.len(), 4);
    assert!(history.iter().all(|h| s(&field(h, "outcome")) == "ok"));
    let k = kinds(&run);
    for kind in ["listen", "http", "deliver", "handle", "respond", "reply"] {
        assert!(k.iter().any(|x| x == kind), "no {kind} event in {k:?}");
    }
    assert_eq!(
        list(&field(&run, "nodes")),
        vec![
            Value::String("world".to_string().into()),
            Value::String("kv".to_string().into()),
            Value::String("c1".to_string().into())
        ]
    );
}

#[test]
fn timeouts_refusals_and_unknown_hosts_are_values_on_virtual_time() {
    let run = eval_here(
        r#"
let w = sim.world(#{ "seed": 2 })
w.node("kv", "kv_server.ol")
w.client("c1", () => {
    let cut = http.get("http://kv:7500/rpc", #{ "timeout_ms": 5000 })
    let at = time.monotonic_ms()
    let nobody = http.get("http://kv:9999/")
    let nowhere = http.get("http://elsewhere/")
    [cut, at, nobody, nowhere]
})
w.partition(["kv"], ["c1"])
w.run(sim.minutes(1))
"#,
    );
    assert_eq!(s(&field(&run, "outcome")), "done", "{run}");
    let parts = list(&field(&field(&run, "value"), "c1"));
    assert!(
        parts[0].to_string().contains("timed out after 5000 ms"),
        "{}",
        parts[0]
    );
    assert_eq!(int(&parts[1]), 5000);
    // the partition holds for the whole run: the refusal never comes back
    assert!(parts[2].to_string().contains("timed out"), "{}", parts[2]);
    assert!(
        parts[3].to_string().contains("could not resolve host"),
        "{}",
        parts[3]
    );
    let net = field(&run, "net");
    assert!(int(&field(&net, "cut")) >= 2);
    assert_eq!(int(&field(&net, "timeouts")), 2);
}

#[test]
fn a_refused_connection_comes_back_after_a_round_trip() {
    let run = eval_here(
        r#"
let w = sim.world(#{ "seed": 2, "net": #{ "latency": 25 } })
w.node("kv", "kv_server.ol")
w.client("c1", () => [http.get("http://kv:9999/"), time.monotonic_ms()])
w.run(sim.minutes(1))
"#,
    );
    let parts = list(&field(&field(&run, "value"), "c1"));
    assert!(
        parts[0].to_string().contains("connection refused"),
        "{}",
        parts[0]
    );
    assert_eq!(int(&parts[1]), 50, "there and back at 25 ms a leg");
}

#[test]
fn a_node_has_its_own_args_env_and_clock_skew() {
    let run = eval_here(
        r#"
let w = sim.world(#{ "seed": 1, "start": 1000 })
w.node("a", ["olang", "node_env.ol", "x", "y"], #{ "env": #{ "ROLE": "primary" }, "skew": 250 })
w.node("b", "node_env.ol")
w.run()
"#,
    );
    assert_eq!(s(&field(&run, "outcome")), "done", "{run}");
    let prints: Vec<(String, String)> = list(&field(&run, "events"))
        .iter()
        .filter(|e| s(&field(e, "kind")) == "print")
        .map(|e| (s(&field(e, "node")), s(&field(e, "detail"))))
        .collect();
    assert!(
        prints.contains(&(
            "a".into(),
            r#"["node_env.ol", "x", "y"] primary 1250"#.into()
        )),
        "{prints:?}"
    );
    assert!(
        prints.contains(&("b".into(), r#"["node_env.ol"] none 1000"#.into())),
        "{prints:?}"
    );
}

#[test]
fn a_crash_loses_memory_and_a_restart_starts_fresh() {
    let run = eval_here(
        r#"
let w = sim.world(#{ "seed": 3 })
w.node("kv", "kv_server.ol")
fn call(op) = match http.post("http://kv:7500/rpc", unwrap(json.stringify(op)), #{ "timeout_ms": 1000 }) {
    Ok(r) => unwrap(json.parse(r.body)).value
    Err(e) => "down"
}
w.client("c1", () => {
    let before = call(#{ "op": "add", "key": "k", "by": 7, "id": "1" })
    time.sleep(sim.seconds(15))
    let during = call(#{ "op": "get", "key": "k" })
    time.sleep(sim.seconds(10))
    let after = call(#{ "op": "get", "key": "k" })
    [before, during, after]
})
w.at(sim.seconds(10), () => w.crash("kv", #{ "restart_after": sim.seconds(8) }))
w.run(sim.minutes(1))
"#,
    );
    assert_eq!(s(&field(&run, "outcome")), "done", "{run}");
    assert_eq!(
        field(&field(&run, "value"), "c1").to_string(),
        r#"[7, "down", 0]"#
    );
    let k = kinds(&run);
    assert!(k.contains(&"crash".to_string()) && k.contains(&"restart".to_string()));
    assert_eq!(k.iter().filter(|x| *x == "listen").count(), 2);
}

#[test]
fn a_client_that_raises_fails_the_run() {
    let run = eval_here(
        r#"
let w = sim.world(#{ "seed": 3 })
w.node("kv", "kv_server.ol")
w.client("c1", () => { time.sleep(100); assert_eq(1, 2, "the check failed") })
w.run(sim.hours(1))
"#,
    );
    assert_eq!(s(&field(&run, "outcome")), "failed");
    assert!(s(&field(&run, "error")).contains("client c1"), "{run}");
    assert!(int(&field(&run, "now")) < 1000);
}

#[test]
fn http_in_a_one_program_world_talks_to_itself() {
    let v = eval(
        r#"
sim.world(#{ "seed": 1 }).run_fn(() => {
    spawn http.serve(8080, (req) => "hello " + req.query.name)
    let r = unwrap(http.get("http://localhost:8080/?name=sim"))
    let refused = http.get("http://127.0.0.1:9/")
    http.shutdown()
    [r.body, is_err(refused)]
}).value
"#,
    );
    assert_eq!(v.to_string(), r#"["hello sim", true]"#);
}

fn builder_report(file: &str, seed: u64) -> olang::sim::Report {
    olang::sim::cli::run_builder(&fixture(file), seed).expect("runs")
}

#[test]
fn one_seed_is_one_world_of_nodes_event_for_event() {
    let a = builder_report("kv_world.ol", 3);
    let b = builder_report("kv_world.ol", 3);
    let c = builder_report("kv_world.ol", 4);
    assert_eq!(a.outcome, "done", "{:?}", a.error);
    assert!(
        a.events.len() > 1000,
        "an hour of traffic: {}",
        a.events.len()
    );
    assert_eq!(a.events, b.events, "the same seed, the same world");
    assert_eq!(a.history, b.history);
    assert_ne!(a.events, c.events, "another seed, another world");
}

#[test]
fn loss_and_partitions_make_retries_and_careful_clients_survive_them() {
    let mut timeouts = 0;
    let mut cut = 0;
    for seed in 1..=3 {
        let r = builder_report("kv_world.ol", seed);
        assert_eq!(r.outcome, "done", "seed {seed}: {:?}", r.error);
        assert!(r.net.lost > 0, "seed {seed}: nothing lost");
        timeouts += r.net.timeouts;
        cut += r.net.cut;
        assert!(r.now >= 3_600_000, "a simulated hour");
    }
    assert!(timeouts > 0, "no request ever timed out");
    assert!(cut > 0, "no partition ever cut a message");
}

#[test]
fn olang_sim_finds_the_partition_bug_and_its_seed_replays_it() {
    let file = fixture("kv_partition_bug.ol");
    let file = file.to_str().unwrap();
    let (code, _, err) = sim(&[file, "--seeds", "30"]);
    assert_eq!(code, 1, "no seed found the bug:\n{err}");
    assert!(err.contains("adds, the client made"), "{err}");
    let passed: u64 = err
        .lines()
        .find_map(|l| {
            l.split(" passed")
                .next()?
                .rsplit("· ")
                .next()?
                .trim()
                .parse()
                .ok()
        })
        .unwrap_or(0);
    assert!(
        passed > 0,
        "every seed failed: not a partition-only bug\n{err}"
    );
    let seed: u64 = err
        .lines()
        .find_map(|l| l.split("--seed ").nth(1))
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or_else(|| panic!("no reproduce line in:\n{err}"));
    let (code1, out1, err1) = sim(&[file, "--seed", &seed.to_string(), "--events"]);
    let (code2, out2, err2) = sim(&[file, "--seed", &seed.to_string(), "--events"]);
    assert_eq!(code1, 1, "{err1}");
    assert!(
        err1.contains("partition"),
        "the replay shows the partition:\n{err1}"
    );
    assert!(err1.contains("adds, the client made"), "{err1}");
    assert_eq!((code1, out1, err1), (code2, out2, err2));
}

#[test]
fn olang_sim_runs_a_world_of_nodes() {
    let file = fixture("kv_world.ol");
    let (code, out, err) = sim(&[file.to_str().unwrap(), "--seed", "2"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("done:"), "{out}");
    assert!(out.contains("timed out and were retried"), "{out}");
}

// ─── S3: the disk and crashes ────────────────────────────────────────

fn faults_of(r: &olang::sim::Report, kind: &str) -> usize {
    r.faults.iter().filter(|f| f.kind == kind).count()
}

#[test]
fn a_node_reads_and_writes_its_own_disk() {
    let v = eval(
        r#"
sim.world(#{ "seed": 1 }).run_fn(() => {
    unwrap(fs.create_dir_all("/srv/data"))
    unwrap(fs.write_file("/srv/data/a.txt", "one"))
    unwrap(fs.append_file("/srv/data/a.txt", " two"))
    unwrap(fs.write_bytes("/srv/data/b.bin", bytes.from_string("bin")))
    unwrap(fs.sync("/srv/data/a.txt"))
    unwrap(fs.move_file("/srv/data/b.bin", "/srv/data/c.bin"))
    unwrap(fs.sync("/srv/data"))
    [unwrap(fs.read_file("/srv/data/a.txt")),
     unwrap(fs.list_dir("/srv/data")),
     unwrap(fs.walk("/srv")),
     unwrap(fs.glob("/srv/**/*.txt")),
     unwrap(fs.file_size("/srv/data/c.bin")),
     fs.exists("/srv/data/b.bin"),
     is_err(fs.read_file("/nope")),
     is_err(fs.remove_dir("/srv")),
     unwrap(fs.abs_path("x/../y"))]
}).value
"#,
    );
    assert_eq!(
        v.to_string(),
        r#"["one two", ["a.txt", "c.bin"], ["/srv/data/a.txt", "/srv/data/c.bin"], ["/srv/data/a.txt"], 3, false, true, true, "/y"]"#
    );
}

#[test]
fn a_crash_keeps_what_was_synced_and_loses_the_rest() {
    let run = eval_here(
        r#"
let w = sim.world(#{ "seed": 2 })
w.client("n", () => {
    let boot = match fs.read_file("/boot") { Ok(t) => unwrap(str.parse_int(t)) + 1, Err(e) => 1 }
    unwrap(fs.write_file("/boot", to_string(boot)))
    unwrap(fs.sync("/boot"))
    if boot == 1 => {
        unwrap(fs.write_file("/kept", "synced"))
        unwrap(fs.sync("/kept"))
        unwrap(fs.write_file("/kept", "overwritten, never synced"))
        unwrap(fs.write_file("/lost", "never synced"))
        time.sleep(sim.minutes(1))
        "the crash never came"
    } else => [unwrap(fs.read_file("/kept")), fs.exists("/lost"), boot]
})
w.at(sim.seconds(10), () => w.crash("n", #{ "restart_after": sim.seconds(1) }))
w.run(sim.minutes(5))
"#,
    );
    assert_eq!(s(&field(&run, "outcome")), "done", "{run}");
    assert_eq!(
        field(&field(&run, "value"), "n").to_string(),
        r#"["synced", false, 2]"#
    );
    let faults = list(&field(&run, "faults"));
    assert_eq!(faults.len(), 1);
    assert!(
        s(&field(&faults[0], "detail")).contains("unsynced write(s) lost"),
        "{}",
        faults[0]
    );
}

#[test]
fn a_full_disk_and_a_slow_one_are_faults_on_virtual_time() {
    let run = eval_here(
        r#"
let w = sim.world(#{ "seed": 2 })
w.client("full", () => {
    let mut n = 0
    while is_ok(fs.append_file("/log", "0123456789")) { n = n + 1 }
    [n, fs.append_file("/log", "x")]
}, #{ "disk": sim.disk(#{ "full_at": 100 }) })
w.client("slow", () => {
    unwrap(fs.write_file("/f", "x"))
    let t0 = time.monotonic_ms()
    unwrap(fs.sync("/f"))
    let one = time.monotonic_ms() - t0
    w.slow_disk("slow", 100)
    let t1 = time.monotonic_ms()
    unwrap(fs.sync("/f"))
    [one, time.monotonic_ms() - t1]
})
w.run(sim.minutes(1))
"#,
    );
    assert_eq!(s(&field(&run, "outcome")), "done", "{run}");
    let value = field(&run, "value");
    assert_eq!(
        field(&value, "full").to_string(),
        r#"[10, Err("Failed to append to file '/log': No space left on device (os error 28)")]"#
    );
    let slow = list(&field(&value, "slow"));
    let (one, hundred) = (int(&slow[0]), int(&slow[1]));
    assert!((1..=3).contains(&one), "a sync takes 1–3 ms: {one}");
    assert!(
        (100..=300).contains(&hundred),
        "a hundred times slower: {hundred}"
    );
    let kinds: Vec<String> = list(&field(&run, "faults"))
        .iter()
        .map(|f| s(&field(f, "kind")))
        .collect();
    assert!(
        kinds.contains(&"disk_full".to_string()) && kinds.contains(&"slow_disk".to_string()),
        "{kinds:?}"
    );
}

#[test]
fn the_log_store_that_syncs_before_it_acks_survives_every_crash() {
    let mut crashes = 0;
    let mut lost = 0;
    for seed in 1..=4 {
        let r = builder_report("log_safe.ol", seed);
        assert_eq!(r.outcome, "done", "seed {seed}: {:?}", r.error);
        crashes += faults_of(&r, "crash");
        lost += r
            .disks
            .iter()
            .find(|(n, _)| n == "store")
            .map(|(_, d)| d.lost)
            .unwrap_or(0);
    }
    assert!(crashes >= 8, "the power went {crashes} times");
    assert!(
        lost > 0,
        "a crash lost unsynced writes (puts never acknowledged)"
    );
}

#[test]
fn olang_sim_finds_the_log_store_that_acks_before_it_syncs_and_replays_it() {
    let file = fixture("log_bug.ol");
    let file = file.to_str().unwrap();
    let (code, _, err) = sim(&[file, "--seeds", "8"]);
    assert_eq!(code, 1, "no seed found the bug:\n{err}");
    assert!(err.contains("lost an acknowledged write"), "{err}");
    let seed: u64 = err
        .lines()
        .find_map(|l| l.split("--seed ").nth(1))
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or_else(|| panic!("no reproduce line in:\n{err}"));
    let (code1, out1, err1) = sim(&[file, "--seed", &seed.to_string(), "--events"]);
    let (code2, out2, err2) = sim(&[file, "--seed", &seed.to_string(), "--events"]);
    assert_eq!(code1, 1, "{err1}");
    assert!(
        err1.contains("crash"),
        "the replay shows the crash:\n{err1}"
    );
    assert!(err1.contains("unsynced write(s) lost"), "{err1}");
    assert_eq!((code1, out1, err1), (code2, out2, err2));
}

#[test]
fn sqlite_keeps_every_commit_through_crashes_mid_commit_on_an_honest_disk() {
    let mut points = 0;
    for seed in 1..=3 {
        let r = builder_report("sqlite_honest.ol", seed);
        assert_eq!(r.outcome, "done", "seed {seed}: {:?}", r.error);
        // crashes the seed placed inside SQLite's own writes and syncs
        points += r
            .faults
            .iter()
            .filter(|f| {
                f.kind == "crash" && (f.detail.contains("xWrite") || f.detail.contains("xSync"))
            })
            .count();
    }
    assert!(points > 0, "no crash fell inside SQLite");
}

#[test]
fn sqlite_loses_acknowledged_commits_when_fsync_lies() {
    let mut failed = 0;
    for seed in 1..=4 {
        let r = builder_report("sqlite_lies.ol", seed);
        if r.failed() {
            failed += 1;
            let e = r.error.clone().unwrap_or_default();
            assert!(e.contains("lost an acknowledged write"), "seed {seed}: {e}");
        }
        assert!(faults_of(&r, "fsync_lie") > 0);
    }
    assert!(failed > 0, "a lying fsync never lost a commit");
}

#[test]
fn one_seed_is_one_world_with_disks_and_crashes_event_for_event() {
    let a = builder_report("log_safe.ol", 7);
    let b = builder_report("log_safe.ol", 7);
    let c = builder_report("log_safe.ol", 8);
    assert_eq!(a.outcome, "done", "{:?}", a.error);
    assert!(a.events.iter().any(|e| e.kind == "fsync"));
    assert_eq!(a.events, b.events, "the same seed, the same world");
    assert_eq!(a.faults, b.faults);
    assert_eq!(a.disks, b.disks);
    assert_ne!(a.events, c.events, "another seed, another world");
    let d = builder_report("sqlite_honest.ol", 2);
    let e = builder_report("sqlite_honest.ol", 2);
    assert_eq!(
        d.events, e.events,
        "SQLite's writes, syncs and crash points too"
    );
    assert_eq!(d.faults, e.faults);
}

#[test]
fn a_node_process_has_its_own_env_stdin_shutdown_handlers_and_exit() {
    let run = eval_here(
        r#"
let w = sim.world(#{ "seed": 4 })
w.node("svc", "life_server.ol", #{ "stdin": "hello\n", "env": #{ "MODE": "a" } })
fn state() = match http.get("http://svc:7700/state", #{ "timeout_ms": 500 }) {
    Ok(r) => { let s = unwrap(json.parse(r.body)); [s.first, s.mode, s.extra, s.boots, s.bye] },
    Err(e) => "down"
}
w.client("c", () => {
    time.sleep(100)
    let first = state()
    // SIGTERM: the handler runs (it writes /bye and stops the server),
    // and the node comes back a second later
    w.stop("svc", #{ "restart_after": 1000 })
    time.sleep(2000)
    let after_stop = state()
    let cleared = unwrap(http.post("http://svc:7700/clear", "")).status
    // the power: no handler runs, so /bye stays gone
    w.crash("svc", #{ "restart_after": 1000 })
    time.sleep(2000)
    let after_crash = state()
    // a line on stdin ends the process with os.exit(0)
    w.write_stdin("svc", "quit\n")
    time.sleep(100)
    [first, after_stop, cleared, after_crash, state()]
})
w.run(sim.minutes(1))
"#,
    );
    assert_eq!(s(&field(&run, "outcome")), "done", "{run}");
    assert_eq!(
        field(&field(&run, "value"), "c").to_string(),
        r#"[["hello", "a", "boot 1", 1, false], ["hello", "a", "boot 2", 2, true], 200, ["hello", "a", "boot 3", 3, false], "down"]"#
    );
    let k = kinds(&run);
    for kind in ["stop", "exit", "crash", "restart", "stdin", "on_shutdown"] {
        assert!(k.contains(&kind.to_string()), "no {kind} event: {k:?}");
    }
}

#[test]
fn os_exit_with_a_failing_status_fails_the_run() {
    let run = eval_here(
        r#"
let w = sim.world(#{ "seed": 1 })
w.client("c", () => { time.sleep(10); os.exit(3) })
w.run(sim.minutes(1))
"#,
    );
    assert_eq!(s(&field(&run, "outcome")), "failed");
    assert!(
        s(&field(&run, "error")).contains("exited with status 3"),
        "{run}"
    );
}

#[test]
fn sim_maybe_is_false_outside_a_world_and_seeded_inside_one() {
    let v = eval(
        r#"
fn fired(seed) = sim.world(#{ "seed": seed }).run_fn(() =>
    len(filter(range(0, 200), (i) => sim.maybe("a") || sim.maybe("b") || sim.maybe("c")))).value
[sim.maybe("a"), fired(1) == fired(1), map(range(1, 30), fired)]
"#,
    );
    let parts = list(&v);
    assert_eq!(parts[0].to_string(), "false");
    assert_eq!(parts[1].to_string(), "true");
    let counts: Vec<i64> = list(&parts[2]).iter().map(int).collect();
    assert!(
        counts.contains(&0),
        "some world turns every point off: {counts:?}"
    );
    assert!(
        counts.iter().any(|c| *c > 20),
        "some world fires them: {counts:?}"
    );
}

#[test]
fn a_node_program_resolves_its_package_dependencies() {
    let run = eval_here(
        r#"
let w = sim.world(#{ "seed": 1 })
w.node("p", "pkgnode/main.ol", #{ "env": #{ "WHO": "sim" } })
w.run(sim.minutes(1))
"#,
    );
    assert_eq!(s(&field(&run, "outcome")), "done", "{run}");
    let prints: Vec<String> = list(&field(&run, "events"))
        .iter()
        .filter(|e| s(&field(e, "kind")) == "print")
        .map(|e| s(&field(e, "detail")))
        .collect();
    assert_eq!(prints, vec!["hello, sim".to_string()]);
    assert!(
        !fixture("pkgnode/olang.lock").exists(),
        "a world wrote a lock"
    );
}

#[test]
fn a_connection_from_outside_the_world_is_refused() {
    let v = eval(
        r#"
let conn = unwrap(db.open(":memory:"))
let inside = sim.world(#{ "seed": 1 }).run_fn(() => db.query(conn, "SELECT 1 AS x", [])).error
[inside, unwrap(db.query(conn, "SELECT 1 AS x", []))[0].x]
"#,
    );
    let parts = list(&v);
    assert!(s(&parts[0]).contains("opened outside it"), "{}", parts[0]);
    assert_eq!(parts[1].to_string(), "1");
}

// ─── S3, the stretch: olang-track as a node ──────────────────────────

/// The olang-track checkout beside this one, when this machine has it
/// and the shelf its server reads (its packages are files on the node's
/// disk, copied from there by the fixture).
fn olang_track() -> Option<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../olang-track");
    let shelf = std::env::var("HOME")
        .map(|h| PathBuf::from(h).join(".olang/shelf.toml"))
        .ok()?;
    (dir.join("lib/server.ol").is_file() && shelf.is_file()).then_some(dir)
}

fn track_sim(dir: &std::path::Path, crash: &str, args: &[&str]) -> (i32, String) {
    let out = Command::new(olang())
        .arg("sim")
        .arg(fixture("track_world.ol"))
        .args(args)
        .env("OLANG_TRACK_DIR", dir)
        .env("CRASH", crash)
        .env("MINUTES", "2")
        .env("EVERY", "20")
        .output()
        .expect("run olang sim");
    (
        out.status.code().unwrap_or(-1),
        (String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr))
            .to_string(),
    )
}

#[test]
fn olang_track_keeps_every_acknowledged_issue_through_kills_and_stops() {
    let Some(dir) = olang_track() else {
        eprintln!("skipped: no olang-track checkout (and shelf) beside this one");
        return;
    };
    let (code, err) = track_sim(&dir, "kill", &["--seeds", "3"]);
    assert_eq!(code, 0, "{err}");
    assert!(err.contains("3 passed"), "{err}");
}

#[test]
fn olang_track_runs_the_same_on_one_job_and_three() {
    // A real application in one process, seed after seed, gives what a
    // fresh worker gives: no process-wide counter (a connection's id, a
    // channel's) reaches a run.
    let Some(dir) = olang_track() else {
        eprintln!("skipped: no olang-track checkout (and shelf) beside this one");
        return;
    };
    let (c1, e1) = track_sim(&dir, "power", &["--seeds", "3", "--jobs", "1"]);
    let (c3, e3) = track_sim(&dir, "power", &["--seeds", "3", "--jobs", "3"]);
    assert_eq!((c1, c3), (0, 0), "{e1}\n{e3}");
    assert_eq!(digest_of(&e1), digest_of(&e3), "{e1}\n{e3}");
}

#[test]
fn olang_track_keeps_acknowledged_issues_through_power_cuts() {
    // S3 found WAL at synchronous=NORMAL losing the issues acknowledged
    // since the last checkpoint to a power cut; olang-track now syncs
    // every commit (synchronous=FULL, its commit cc4fd13), and power cuts
    // lose nothing acknowledged. Run across two worker processes.
    let Some(dir) = olang_track() else {
        eprintln!("skipped: no olang-track checkout (and shelf) beside this one");
        return;
    };
    let (code, err) = track_sim(&dir, "power", &["--seeds", "2", "--jobs", "2"]);
    assert_eq!(code, 0, "{err}");
    assert!(err.contains("2 passed"), "{err}");
}

// ─── S4: properties, search and shrinking ────────────────────────────

/// A history-driven world: `n` writes, each recorded as invoked and, `gap`
/// ms later, completed, with `gap` ms between them. (Without the pause
/// between, a write would complete and the next be invoked within one
/// millisecond, and no check would ever see none open: properties are
/// checked at instants.)
const PROPS: &str = r#"
fn work(n, gap) = {
    for i in range(0, n) {
        sim.record("ops", #{ "type": "invoke", "f": "write", "value": i })
        time.sleep(gap)
        sim.record("ops", #{ "type": "ok" })
        time.sleep(gap)
    }
    n
}
fn pending(s) = {
    let h = s.history("ops")
    len(filter(h, (e) => e.type == "invoke")) - len(filter(h, (e) => e.type == "ok"))
}
"#;

#[test]
fn always_never_and_eventually_hold_and_break_with_the_time_and_step() {
    let v = eval(&format!(
        r#"{PROPS}
let w = sim.world(#{{ "seed": 3 }})
let ok = w.run_fn(() => work(5, 100), #{{ "props": [
    sim.always("at most one open", (s) => pending(s) <= 1),
    sim.never("time runs backwards", (s) => s.now < 0),
    sim.eventually("every write completes", 150, (s) => pending(s) == 0)
] }})
let always = w.run_fn(() => work(5, 100), #{{ "props": [sim.always("fewer than 3 writes", (s) => len(s.history("ops")) < 6)] }})
let never = w.run_fn(() => work(5, 100), [sim.never("a third write", (s) => len(s.history("ops")) >= 5)])
let slow = w.run_fn(() => work(3, 1000), #{{ "props": [sim.eventually("every write completes", 150, (s) => pending(s) == 0)] }})
let raises = w.run_fn(() => work(2, 10), #{{ "props": [sim.always("bad", (s) => 1 / 0 == 1)] }})
let effect = w.run_fn(() => work(2, 10), #{{ "props": [sim.always("peeks", (s) => time.now_ms() > 0)] }})
[ok.outcome, ok.violation, always.violation, never.violation, slow.violation, raises.violation.detail, effect.violation.detail]
"#
    ));
    let out = list(&v);
    assert_eq!(s(&out[0]), "done", "{out:?}");
    assert_eq!(out[1], Value::Unit);
    let always = &out[2];
    assert_eq!(s(&field(always, "property")), "fewer than 3 writes");
    assert_eq!(s(&field(always, "kind")), "always");
    assert_eq!(int(&field(always, "t")), 500, "the third write's completion, at 500 ms");
    assert!(int(&field(always, "step")) > 0);
    let never = &out[3];
    assert_eq!(s(&field(never, "property")), "a third write");
    assert_eq!(int(&field(never, "t")), 400, "the third write's invoke");
    let slow = &out[4];
    assert_eq!(s(&field(slow, "kind")), "eventually");
    assert_eq!(
        int(&field(slow, "t")),
        151,
        "reported at the stretch's deadline, though nothing happens then"
    );
    assert!(s(&out[5]).contains("its predicate raised"), "{}", s(&out[5]));
    assert!(s(&out[6]).contains("cannot call time.now_ms"), "{}", s(&out[6]));
}

#[test]
fn properties_given_to_w_run_stop_it_and_sim_check_runs_it_again_watched() {
    let v = eval_here(
        r#"
use log_props { acked_puts_are_durable }
use log_world { run_store }
let run = run_store("ack_then_sync", 1)
let found = attempt(() => sim.check(run, [acked_puts_are_durable()]))
let fine = sim.check(run_store("sync_then_ack", 1), [acked_puts_are_durable()])
[run.outcome, run.handle != (), len(run.history("puts")) > 10, found, fine.outcome]
"#,
    );
    let out = list(&v);
    assert_eq!(s(&out[0]), "failed", "unwatched, the writer's own check fails it later");
    assert_eq!(out[1], Value::Boolean(true));
    assert_eq!(out[2], Value::Boolean(true), "run.history(name) answers the history");
    let found = s(&out[3]);
    assert!(
        found.contains("sim.check: property")
            && found.contains("an acknowledged put is on the store's disk")
            && found.contains("broke at 0."),
        "{found}"
    );
    assert_eq!(s(&out[4]), "done");
}

#[test]
fn a_predicate_reads_a_node_disk_as_a_power_cut_would_leave_it() {
    // The predicate raises with what it saw once the client has written:
    // the violation's detail carries it.
    let v = eval_here(
        r#"
let w = sim.world(#{ "seed": 1 })
w.client("n", () => {
    unwrap(fs.write_file("/a", "synced"))
    unwrap(fs.sync("/a"))
    unwrap(fs.write_file("/a", "pending"))
    unwrap(fs.write_file("/b", "never synced"))
    time.sleep(10)
})
let run = w.run(#{ "props": [sim.always("snapshot", (s) => {
    let d = s.disk("n")
    let views = [d.read("/a"), d.current("/a"), d.read("/b"), d.exists("/b"), d.current("/b"), d.exists("/a")]
    if views[1] == "pending" => { assert(false, to_string(views)); true } else => true
})] })
[run.violation.t, run.violation.detail]
"#,
    );
    let out = list(&v);
    let t = int(&out[0]);
    assert!((1..=3).contains(&t), "the instant the sync (1–3 ms) returns: {t}");
    let detail = s(&out[1]);
    assert!(
        detail.contains(r#"["synced", "pending", (), false, "never synced", true]"#),
        "{detail}"
    );
}

#[test]
fn a_predicate_queries_the_durable_image_of_a_sqlite_database() {
    // Acknowledged puts must be rows of the database as a power cut would
    // leave it: true on an honest disk, false when fsync lies.
    let src = |lies: &str| {
        format!(
            r#"
use log_client {{ writer }}
fn acked(s) = fold(s.history("puts"), #{{}}, (acc, e) =>
    if e.type == "ok" => map_set(acc, e.value[0], e.value[1]) else => acc)
fn durable(s) = match s.disk("store").query("/data/kv.db", "SELECT key, version FROM kv WHERE version > ?", [0]) {{
    Ok(rows) => fold(rows, #{{}}, (acc, r) => map_set(acc, r.key, r.version)),
    Err(e) => #{{}}
}}
let w = sim.world(#{{ "seed": 2, "net": #{{ "latency": (1, 20) }} }})
w.node("store", ["olang", "sqlite_store.ol"], #{{ "disk": sim.disk(#{{ "fsync_lies": {lies} }}) }})
w.client("w1", () => writer("http://store:7600", "k1", sim.seconds(20)))
let run = w.run(#{{ "for": sim.seconds(30), "props": [sim.always("acked puts are durable rows", (s) => {{
    let d = durable(s)
    let a = acked(s)
    all(map_keys(a), (k) => (map_get(d, k) ?? 0) >= map_get(a, k))
}})] }})
[run.outcome, len(run.history("puts"))]
"#
        )
    };
    let honest = list(&eval_here(&src("0")));
    assert_eq!(s(&honest[0]), "done", "{honest:?}");
    assert!(int(&honest[1]) > 20, "{honest:?}");
    let lying = list(&eval_here(&src("1.0")));
    assert_eq!(s(&lying[0]), "violated", "{lying:?}");
}

#[test]
fn the_linearizability_checker_is_right_on_known_histories() {
    let v = eval(
        r#"
fn e(t, p, f, v) = #{ "type": t, "process": p, "f": f, "value": v }
fn ok(p, v) = #{ "type": "ok", "process": p, "value": v }
fn verdict(h, m) = sim.linearize(h, m).verdict
// a write overlapping two reads that see old, then new: fine
let fine = [e("invoke", "w", "write", 1), e("invoke", "a", "read", ()), ok("a", 0),
    e("invoke", "b", "read", ()), ok("b", 1), ok("w", ())]
// new, then old, after the first read finished: no order explains it
let flip = [e("invoke", "w", "write", 1), e("invoke", "a", "read", ()), ok("a", 1),
    e("invoke", "b", "read", ()), ok("b", 0), ok("w", ())]
// a timed-out write may or may not have happened, but not both
let maybe = [e("invoke", "w", "write", 5), #{ "type": "info", "process": "w" },
    e("invoke", "a", "read", ()), ok("a", 5)]
let both = maybe + [e("invoke", "a", "read", ()), ok("a", 0)]
// two compare-and-sets from 0 both succeeding
let cas = [e("invoke", "a", "cas", [0, 1]), e("invoke", "b", "cas", [0, 2]), ok("a", ()), ok("b", ())]
// a model in olang: a counter whose `inc` answers the new value
let counter = #{ "init": 0, "step": (st, op) =>
    if op.f == "inc" => (if !op.known || op.output == st + 1 => Ok(st + 1) else => Err("wrong count"))
    else => Err("unknown op") }
let incs = [e("invoke", "a", "inc", ()), e("invoke", "b", "inc", ()), ok("b", 1), ok("a", 2)]
let twice = [e("invoke", "a", "inc", ()), ok("a", 1), e("invoke", "b", "inc", ()), ok("b", 3)]
[verdict(fine, sim.register(0)), verdict(flip, sim.register(0)), verdict(maybe, sim.register(0)),
 verdict(both, sim.register(0)), verdict(cas, sim.register(0)),
 verdict(incs, counter), verdict(twice, counter),
 sim.linearize(flip, sim.register(0), #{ "max_steps": 2 }).verdict]
"#,
    );
    let got: Vec<String> = list(&v).iter().map(s).collect();
    assert_eq!(
        got,
        [
            "linearizable",
            "violation",
            "linearizable",
            "violation",
            "violation",
            "linearizable",
            "violation",
            "unknown"
        ]
    );
}

#[test]
fn the_kv_partition_bug_is_a_linearizability_violation() {
    let file = fixture("kv_linear.ol");
    let (code, _, err) = sim(&[file.to_str().unwrap(), "--seeds", "16", "--jobs", "4"]);
    assert_eq!(code, 1, "no seed broke linearizability:\n{err}");
    assert!(
        err.contains("\"every add counts once\" broke at") && err.contains("no order of the"),
        "{err}"
    );
    // with clients that retry under the same id, every seed is linearizable
    let out = Command::new(olang())
        .arg("sim")
        .arg(&file)
        .args(["--seeds", "8", "--jobs", "4"])
        .env("KV_BUGGY", "0")
        .output()
        .expect("run olang sim");
    let err =
        String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "{err}");
    assert!(err.contains("8 passed"), "{err}");
}

#[test]
fn the_log_bug_is_a_durability_violation_and_the_safe_store_holds() {
    let (code, _, err) = sim(&[fixture("log_durable_bug.ol").to_str().unwrap(), "--seeds", "4"]);
    assert_eq!(code, 1, "{err}");
    assert_eq!(
        err.matches("\"an acknowledged put is on the store's disk\" broke at").count(),
        4,
        "{err}"
    );
    let (code, _, err) = sim(&[fixture("log_durable_safe.ol").to_str().unwrap(), "--seeds", "4", "--jobs", "2"]);
    assert_eq!(code, 0, "{err}");
    assert!(err.contains("4 passed"), "{err}");
}

fn digest_of(err: &str) -> String {
    err.lines()
        .find_map(|l| l.strip_prefix("digest: "))
        .unwrap_or_else(|| panic!("no digest in:\n{err}"))
        .to_string()
}

#[test]
fn one_job_and_four_jobs_give_the_same_results() {
    for (file, seeds) in [("kv_linear.ol", "8"), ("log_bug.ol", "8"), ("sqlite_lies.ol", "4")] {
        let path = fixture(file);
        let path = path.to_str().unwrap();
        let (c1, _, e1) = sim(&[path, "--seeds", seeds, "--jobs", "1"]);
        let (c4, _, e4) = sim(&[path, "--seeds", seeds, "--jobs", "4"]);
        assert_eq!(c1, c4, "{file}");
        assert_eq!(digest_of(&e1), digest_of(&e4), "{file}:\n{e1}\n{e4}");
        let failures = |e: &str| -> Vec<String> {
            e.lines().filter(|l| l.starts_with("seed ")).map(String::from).collect()
        };
        assert_eq!(failures(&e1), failures(&e4), "{file}");
        assert!(e4.contains("4 job(s)"), "{e4}");
    }
}

#[test]
fn a_recorded_run_comes_back_exactly_from_its_plan() {
    for file in ["log_bug.ol", "kv_partition_bug.ol", "sqlite_lies.ol"] {
        let path = fixture(file);
        let (first, plans) =
            olang::sim::cli::run_builder_forced(&path, 5, None).expect("runs");
        assert!(first.plannable_faults() > 0, "{file}: no faults");
        let (again, _) =
            olang::sim::cli::run_builder_forced(&path, 5, Some(plans.clone())).expect("runs");
        assert_eq!(first.digest(), again.digest(), "{file}");
        assert_eq!(first.events, again.events, "{file}");
        // with no faults at all, another run
        let mut none = plans.clone();
        for p in &mut none {
            p.faults.clear();
        }
        let (quiet, _) = olang::sim::cli::run_builder_forced(&path, 5, Some(none)).expect("runs");
        assert_eq!(quiet.plannable_faults(), 0, "{file}");
        assert_ne!(quiet.digest(), first.digest(), "{file}");
    }
}

#[test]
fn the_shrinker_takes_a_noisy_world_down_to_its_one_fault_and_it_replays() {
    let dir = std::env::temp_dir().join(format!("olang-sim-shrink-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = fixture("log_noisy.ol");
    let run = |args: &[&str]| {
        let out = Command::new(olang())
            .arg("sim")
            .args(args)
            .current_dir(&dir)
            .output()
            .expect("run olang sim");
        (
            out.status.code().unwrap_or(-1),
            (String::from_utf8_lossy(&out.stdout).to_string()
                + &String::from_utf8_lossy(&out.stderr))
                .to_string(),
        )
    };
    let (code, err) = run(&[file.to_str().unwrap(), "--seed", "1", "--shrink", "--jobs", "2"]);
    assert_eq!(code, 1, "{err}");
    let line = err
        .lines()
        .find(|l| l.starts_with("shrunk: "))
        .unwrap_or_else(|| panic!("no shrink line:\n{err}"));
    let before: usize = line["shrunk: ".len()..]
        .split(' ')
        .next()
        .and_then(|n| n.parse().ok())
        .unwrap();
    assert!(before >= 20, "a noisy world: {line}");
    assert!(line.contains("→ 1 ·"), "{line}");
    assert!(line.contains("left out: spare"), "{line}");
    assert!(err.contains("crash      store"), "the fault left is the store's power cut:\n{err}");
    let plan = dir.join(".sim/1.simplan");
    assert!(plan.is_file(), "{err}");
    // the plan replays bit for bit, every time
    for _ in 0..2 {
        let (code, err) = run(&["--replay", plan.to_str().unwrap()]);
        assert_eq!(code, 1, "{err}");
        assert!(err.contains("replayed") && err.contains("bit for bit"), "{err}");
        assert!(err.contains("lost an acknowledged write"), "{err}");
    }
    // a shrunk plan shrinks no further
    let (code, err) = run(&["shrink", plan.to_str().unwrap()]);
    assert_eq!(code, 0, "{err}");
    assert!(err.contains("shrunk: 1 fault → 1"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn olang_test_runs_sim_blocks_over_a_few_seeds() {
    let file = fixture("sim_blocks_test.ol");
    let out = Command::new(olang())
        .arg("test")
        .arg(&file)
        .output()
        .expect("run olang test");
    let text = String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "{text}");
    assert!(text.contains("3 passed, 0 failed"), "{text}");
    // a block that fails on one seed names it; --seeds widens the search
    let dir = std::env::temp_dir().join(format!("olang-sim-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let t = dir.join("seeded_test.ol");
    std::fs::write(
        &t,
        "test \"not seed 5\" {\n    let run = sim.world(#{}).run_fn(() => sim.seed())\n    assert(run.value != 5, \"seed five\")\n}\n",
    )
    .unwrap();
    let run = |extra: &[&str]| {
        let out = Command::new(olang()).arg("test").arg(&t).args(extra).output().unwrap();
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr),
        )
    };
    let (code, text) = run(&[]);
    assert_eq!(code, Some(0), "three seeds do not reach 5: {text}");
    let (code, text) = run(&["--seeds", "6"]);
    assert_eq!(code, Some(1), "{text}");
    assert!(text.contains("sim seed 5: "), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

// ─── S5: seeing a run, and the bytecode tier ─────────────────────────

#[test]
fn run_events_is_a_frame_of_every_event_for_analysis() {
    let v = eval(&format!(
        r#"{RACE}
let run = sim.world(#{{ "seed": 3 }}).run_fn(race)
let f = run.events()
let sends = ods.filter(f, f["kind"] == "send")
let by = ods.to_records(ods.sort_by(f.group_by("kind").agg(#{{ "n": ods.count() }}), "kind", false))
let listed = len(filter(run.events, (e) => e.kind == "send"))
[ods.n_rows(f), len(run.events), ods.n_rows(sends), listed, ods.columns(f), by, typeof(f)]
"#
    ));
    let parts = list(&v);
    assert_eq!(parts[0], parts[1], "a row for every event: {v}");
    assert!(int(&parts[0]) > 10, "{v}");
    assert_eq!(parts[2], parts[3], "filter agrees with the list: {v}");
    assert_eq!(
        parts[4].to_string(),
        r#"["seq", "t", "node", "task", "kind", "detail", "peer", "sent", "inbound", "fault"]"#
    );
    // the counts by kind add up to every event
    let total: i64 = list(&parts[5]).iter().map(|r| int(&field(r, "n"))).sum();
    assert_eq!(total, int(&parts[0]), "{v}");
    assert!(s(&parts[6]).contains("Frame"), "{v}");
}

#[test]
fn a_frame_of_a_world_of_nodes_carries_messages_and_faults() {
    let v = eval_here(
        r#"
let w = sim.world(#{ "seed": 5 })
w.node("kv", ["olang", "kv_server.ol", "7500"])
w.client("c1", () => {
    let body = (op) => json.stringify(op) |> unwrap
    for i in range(0, 5) {
        let r = http.post("http://kv:7500/rpc", body(#{ "op": "add", "key": "x", "by": 1, "id": to_string(i) }), #{ "timeout_ms": 500 })
        time.sleep(200)
    }
})
w.at(300, () => { w.partition(["kv"], ["c1"]); time.sleep(400); w.heal() })
let run = w.run(sim.minutes(1))
let f = run.events()
let arrivals = ods.filter(f, f["kind"] == "deliver")
let parts = ods.to_records(ods.filter(f, f["kind"] == "partition"))
let peers = ods.to_list(ods.unique(arrivals["peer"]))
let late = ods.to_records(ods.filter(arrivals, arrivals["t"] < arrivals["sent"]))
[ods.n_rows(arrivals) > 0, peers, len(late), parts[0].fault, parts[0].node,
 ods.to_list(arrivals["inbound"]) |> all((b) => b), run.outcome]
"#,
    );
    let parts = list(&v);
    assert_eq!(parts[0], Value::Boolean(true), "{v}");
    let mut peers: Vec<String> = list(&parts[1]).iter().map(s).collect();
    peers.sort();
    assert_eq!(peers, vec!["c1", "kv"], "{v}");
    assert_eq!(int(&parts[2]), 0, "nothing arrives before it was sent: {v}");
    assert_eq!(
        parts[3],
        Value::Boolean(true),
        "a partition is a fault: {v}"
    );
    assert_eq!(parts[5], Value::Boolean(true), "{v}");
    assert_eq!(s(&parts[6]), "done", "{v}");
}

#[test]
fn the_view_model_has_a_lane_per_node_arrows_faults_and_down_stretches() {
    let r = builder_report("log_noisy.ol", 1);
    let m = olang::sim::view::model(&r, "log_noisy.ol · seed 1");
    let lanes: Vec<&str> = m["lanes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l.as_str().unwrap())
        .collect();
    for node in ["store", "spare", "w1", "w2", "w3"] {
        assert!(lanes.contains(&node), "{lanes:?}");
    }
    let events = m["events"].as_array().unwrap();
    assert_eq!(events.len(), r.events.len());
    let class = |c: &str| events.iter().filter(|e| e[7] == c).count();
    assert!(class("msg") > 100, "messages are arrows");
    assert!(class("fault") > 10, "faults are marks");
    // an arrow goes from its sender's lane at its send to its receiver's
    for e in events.iter().filter(|e| e[7] == "msg") {
        assert!(e[5].as_i64().unwrap() >= 0 && e[6].as_i64().unwrap() <= e[0].as_i64().unwrap());
    }
    // the spare's crashes are stretches it was down, until each restart
    let spare = lanes.iter().position(|l| *l == "spare").unwrap() as i64;
    let down = m["down"].as_array().unwrap();
    assert!(
        down.iter().filter(|d| d[0] == spare).count() >= 3,
        "{down:?}"
    );
    for d in down {
        assert!(d[1].as_u64() <= d[2].as_u64(), "{d:?}");
    }
    // the partitions are spans over the lanes they cut apart
    let cuts = m["cuts"].as_array().unwrap();
    assert!(!cuts.is_empty());
    assert_eq!(
        cuts[0][1].as_u64().unwrap() - cuts[0][0].as_u64().unwrap(),
        1000
    );
    assert_eq!(m["failed"], true);
}

#[test]
fn olang_sim_view_draws_a_run_from_a_file_and_seed_and_from_a_saved_plan() {
    let dir = std::env::temp_dir().join(format!("olang-sim-view-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let html = dir.join("kv.html");
    let json = dir.join("kv.json");
    let (code, _, err) = sim(&[
        "view",
        fixture("kv_partition_bug.ol").to_str().unwrap(),
        "--seed",
        "5",
        "--html",
        html.to_str().unwrap(),
        "--json",
        json.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "{err}");
    let page = std::fs::read_to_string(&html).unwrap();
    assert!(page.starts_with("<!doctype html>"), "a page");
    assert!(page.contains("<title>kv_partition_bug.ol · seed 5</title>"));
    // one file: nothing is fetched from anywhere (the run's own data, in
    // between, may name URLs: the requests it made)
    let head = page.split("const DATA = ").next().unwrap();
    let code_part = page.split(";\n(() => {").nth(1).expect("the page's code");
    for outside in ["http://", "https://", "src=\"", "<link", "import("] {
        assert!(
            !head.contains(outside) && !code_part.contains(outside),
            "the page loads {outside}"
        );
    }
    assert!(!page.contains("__DATA__") && !page.contains("__TITLE__"));
    let model: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&json).unwrap()).unwrap();
    assert_eq!(model["seed"], 5);
    assert_eq!(model["failed"], true);
    assert_eq!(
        model["lanes"],
        serde_json::json!(["world", "kv", "c1", "c2", "c3", "c4", "hook-1"])
    );
    // the same run as `olang sim --seed 5` reports
    let report = builder_report("kv_partition_bug.ol", 5);
    assert_eq!(model["digest"], format!("{:016x}", report.digest()));
    // the page holds the model as it is
    assert!(page.contains(&format!("\"digest\":\"{:016x}\"", report.digest())));

    // from a saved plan: the shrunk run, replayed bit for bit
    let (code, _, err) = Command::new(olang())
        .current_dir(&dir)
        .args([
            "sim",
            fixture("log_bug.ol").to_str().unwrap(),
            "--seed",
            "3",
            "--shrink",
        ])
        .output()
        .map(|o| {
            (
                o.status.code().unwrap_or(-1),
                String::new(),
                (String::from_utf8_lossy(&o.stdout).to_string()
                    + &String::from_utf8_lossy(&o.stderr))
                    .to_string(),
            )
        })
        .unwrap();
    assert_eq!(code, 1, "{err}");
    let plan = dir.join(".sim/3.simplan");
    assert!(plan.is_file(), "{err}");
    let out = dir.join("plan.json");
    let (code, _, err) = sim(&[
        "view",
        plan.to_str().unwrap(),
        "--json",
        out.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "{err}");
    assert!(!err.contains("diverged"), "{err}");
    let model: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
    let saved: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&plan).unwrap()).unwrap();
    assert_eq!(
        model["digest"], saved["digest"],
        "the plan's run, bit for bit"
    );
    assert_eq!(model["title"], "log_bug.ol · seed 3");
    let kinds: Vec<&str> = model["kinds"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| k.as_str().unwrap())
        .collect();
    let crash = kinds
        .iter()
        .position(|k| *k == "crash")
        .expect("the plan's crash");
    assert!(
        model["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e[3] == crash && e[7] == "fault"),
        "the crash is drawn as a fault"
    );
    // asking for nothing to view says how
    let (code, _, err) = sim(&["view"]);
    assert_eq!(code, 2);
    assert!(err.contains("olang sim view"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The sweep digest of `file` over `seeds` seeds, on one tier (the
/// report is on stdout, diagnostics on stderr).
fn tier_digest(file: &str, seeds: &str, tier: &str) -> String {
    let out = Command::new(olang())
        .args(["sim", fixture(file).to_str().unwrap(), "--seeds", seeds])
        .env("OLANG_SIM_TIER", tier)
        .output()
        .expect("run olang sim");
    let err = (String::from_utf8_lossy(&out.stdout).to_string()
        + &String::from_utf8_lossy(&out.stderr))
        .to_string();
    digest_of(&err)
}

#[test]
fn every_world_runs_the_same_on_the_bytecode_tier_as_on_the_interpreter() {
    // S1 programs, S2 networks, S3 disks and SQLite, S4 properties: the
    // same seeds give the same runs, digest for digest (every event,
    // fault, the outcome, the time and the steps).
    for (file, seeds) in [
        ("lost_update.ol", "12"),
        ("deadlock.ol", "3"),
        ("virtual_time.ol", "3"),
        ("kv_partition_bug.ol", "6"),
        ("kv_linear.ol", "3"),
        ("log_bug.ol", "4"),
        ("log_noisy.ol", "3"),
        ("log_durable_safe.ol", "2"),
        ("sqlite_lies.ol", "3"),
        ("node_env.ol", "2"),
    ] {
        let interp = tier_digest(file, seeds, "interp");
        let bytecode = tier_digest(file, seeds, "bytecode");
        assert_eq!(interp, bytecode, "{file}: the tiers disagree");
    }
}

#[test]
fn the_tier_a_world_runs_on_reaches_the_workers() {
    // `--jobs` workers run on the tier their parent was asked for
    let run = |tier: &str| {
        let out = Command::new(olang())
            .args([
                "sim",
                fixture("kv_partition_bug.ol").to_str().unwrap(),
                "--seeds",
                "4",
                "--jobs",
                "2",
            ])
            .env("OLANG_SIM_TIER", tier)
            .output()
            .unwrap();
        digest_of(
            &(String::from_utf8_lossy(&out.stdout).to_string()
                + &String::from_utf8_lossy(&out.stderr)),
        )
    };
    assert_eq!(run("interp"), run("bytecode"));
    assert_eq!(
        run("bytecode"),
        tier_digest("kv_partition_bug.ol", "4", "bytecode")
    );
}

#[test]
fn a_function_compiled_before_a_world_still_meets_the_world() {
    // `now` and `fetch` are hot (compiled to bytecode) before the world
    // starts; inside it their builtin calls are the world's: virtual
    // time, and a network with no such host. A function that reaches for
    // the operating system is refused, compiled or not.
    let v = eval(
        r#"
fn now() = time.monotonic_ms()
fn fetch(url) = http.get(url, #{ "timeout_ms": 50 })
fn peek() = os.username()
for i in range(0, 200) { let _ = now() }
let run = sim.world(#{ "seed": 1 }).run_fn(() => {
    time.sleep(5000)
    let t = now()
    let r = fetch("http://nowhere/")
    let p = match attempt(() => peek()) { Ok(x) => "read", Err(e) => e }
    [t, is_err(r), p]
})
run.value
"#,
    );
    let parts = list(&v);
    assert_eq!(int(&parts[0]), 5000, "virtual time: {v}");
    assert_eq!(parts[1], Value::Boolean(true), "{v}");
    assert!(
        s(&parts[2]).contains("refused inside a world"),
        "refused: {v}"
    );
}

#[test]
fn every_sim_and_peg_member_has_help() {
    let help = olang::help::HelpSystem::new();
    let mut missing = Vec::new();
    let module = olang::sim::surface::create_sim_module();
    let Value::Struct { fields, .. } = &module else {
        panic!("sim is a module")
    };
    for (name, value) in fields.iter() {
        match value {
            Value::Struct { fields: inner, .. } => {
                for method in inner.keys() {
                    missing.push(format!("sim.{name}.{method}"));
                }
            }
            _ => missing.push(format!("sim.{name}")),
        }
    }
    for f in ["grammar", "parse", "check", "examples", "dump"] {
        missing.push(format!("peg.{f}"));
    }
    missing.retain(|n| help.get_function(n).is_none());
    assert!(missing.is_empty(), "no :help for {missing:?}");
}

#[test]
fn olang_sim_reports_on_stdout_and_refuses_events_over_many_seeds() {
    let file = fixture("kv_partition_bug.ol");
    let out = Command::new(olang())
        .args(["sim", file.to_str().unwrap(), "--seeds", "2"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("digest: ") && stdout.contains("2 seed(s)"),
        "{stdout}"
    );
    let (code, _, err) = sim(&[file.to_str().unwrap(), "--seeds", "2", "--events"]);
    assert_eq!(code, 2);
    assert!(err.contains("--events prints one run's events"), "{err}");
    // a refusal reads as a sentence
    let v = eval("sim.world(#{}).run_fn(() => proc.spawn(\"ls\", [])).error");
    assert!(
        s(&v).contains("a world does not simulate processes yet"),
        "{v}"
    );
}
