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
[attempt(() => fs.write_file("{path}", "x")),
 attempt(() => fs.read_file("{path}")),
 attempt(() => db.open(":memory:")),
 attempt(() => proc.spawn(["true"])),
 attempt(() => os.set_env("HOME", "/tmp")),
 attempt(() => {{ let t = spawn fs.write_file("{path}", "y"); unwrap(task.join(t)) }}),
 attempt(() => sim.world(#{{}}))]
"#
    ));
    let errors = list(&v);
    for (i, e) in errors.iter().enumerate() {
        let e = s(e);
        assert!(e.contains("refused inside a world"), "attempt {i}: {e}");
    }
    assert!(s(&errors[0]).contains("fs.write_file"));
    assert!(s(&errors[6]).contains("worlds do not nest"));
    // A node's environment is its own (empty unless given): the real one
    // is never read.
    let home = eval(
        r#"sim.world(#{ "seed": 1 }).run_fn(() => [os.get_env("HOME"), os.has_env("PATH")]).value"#,
    );
    assert_eq!(
        home.to_string(),
        r#"[Err("Environment variable 'HOME' not found"), false]"#
    );
    assert!(
        !marker.exists(),
        "a refused write reached the real filesystem"
    );
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
        String::from_utf8_lossy(&out.stderr).to_string(),
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
