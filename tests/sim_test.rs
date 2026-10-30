//! `sim` S1: a world runs an olang program on virtual time with seeded
//! scheduling of `spawn`, `task` and `chan` (plans/SIM.md, SIM-S1.md).
//!
//! What these hold: one seed is one run, event for event; different
//! seeds interleave differently; a known race is found by some seed and
//! replayed exactly by it (in process and through `olang sim`); sleeping
//! costs no real time; timeouts and deadlocks are the world's; and every
//! effect the world does not simulate is refused, never run for real.

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
 attempt(() => http.get("http://127.0.0.1:9/")),
 attempt(() => db.open(":memory:")),
 attempt(() => proc.spawn(["true"])),
 attempt(() => os.get_env("HOME")),
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
    assert!(s(&errors[2]).contains("http.get"));
    assert!(s(&errors[7]).contains("worlds do not nest"));
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
