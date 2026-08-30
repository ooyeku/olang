//! The all-parked stall detector (W6): a program whose every live
//! thread is blocked on an unbounded wait aborts with a report naming
//! the blocked sites, instead of hanging silently forever. The proof
//! lives in src/stdlib/chan.rs; these tests pin its two obligations —
//! genuine deadlocks die fast and loud, and anything that can still
//! make progress is never touched.

use std::process::Command;

fn run(source: &str) -> (i32, String) {
    let dir = std::env::temp_dir().join("olang_stall_tests");
    std::fs::create_dir_all(&dir).expect("mkdir");
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    source.hash(&mut h);
    let path = dir.join(format!("t{:x}.ol", h.finish()));
    std::fs::write(&path, source).expect("write");
    let out = Command::new(env!("CARGO_BIN_EXE_olang"))
        .arg("run")
        .arg(&path)
        .output()
        .expect("run");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.code().unwrap_or(-1), text)
}

#[test]
fn a_recv_nobody_can_satisfy_aborts_with_the_site() {
    let (code, text) = run("let c = chan.new()\nchan.recv(c)\nprintln(\"unreachable\")");
    assert_eq!(code, 101, "expected the stall abort, got:\n{text}");
    assert!(text.contains("deadlock"), "report missing:\n{text}");
    assert!(
        text.contains("chan.recv on channel #1"),
        "site not named:\n{text}"
    );
    assert!(!text.contains("unreachable"));
}

#[test]
fn a_join_on_a_parked_task_reports_both_sites() {
    let (code, text) = run("let c = chan.new()\n\
         let t = spawn chan.recv(c)\n\
         task.join(t)\n\
         println(\"unreachable\")");
    assert_eq!(code, 101, "expected the stall abort, got:\n{text}");
    assert!(text.contains("task.join on task 1"), "join site:\n{text}");
    assert!(
        text.contains("chan.recv on channel #1"),
        "recv site:\n{text}"
    );
}

#[test]
fn a_full_rendezvous_send_is_a_parked_site_too() {
    let (code, text) = run("let c = chan.bounded(0)\nchan.send(c, 1)\nprintln(\"unreachable\")");
    assert_eq!(code, 101, "expected the stall abort, got:\n{text}");
    assert!(
        text.contains("chan.send on full channel #1"),
        "send site:\n{text}"
    );
}

#[test]
fn a_late_sender_is_progress_not_deadlock() {
    // Main parks in recv for well over the detection window while the
    // worker sleeps (alive, not parked): the abort must not fire, and
    // the handoff must complete.
    let (code, text) = run("let c = chan.new()\n\
         let t = spawn { time.sleep(900) ; unwrap(chan.send(c, 42)) }\n\
         println(unwrap(chan.recv(c)))\n\
         task.join(t)\n\
         println(\"done\")");
    assert_eq!(code, 0, "no abort expected:\n{text}");
    assert!(text.contains("42") && text.contains("done"));
}

#[test]
fn rendezvous_handoff_under_the_polling_send() {
    // The bounded send is a try_send poll now; a real rendezvous
    // (receiver arrives late) must still hand off and finish clean.
    let (code, text) = run("let c = chan.bounded(0)\n\
         let t = spawn { time.sleep(400) ; unwrap(chan.recv(c)) }\n\
         unwrap(chan.send(c, 7))\n\
         println(task.join(t))\n\
         println(\"done\")");
    assert_eq!(code, 0, "no abort expected:\n{text}");
    assert!(text.contains("7") && text.contains("done"));
}

#[test]
fn introspection_names_tasks_channels_and_parked_sites() {
    let (code, text) = run("let c = chan.bounded(8)\n\
         unwrap(chan.send(c, 1))\n\
         unwrap(chan.send(c, 2))\n\
         let t1 = spawn { time.sleep(400) ; 7 }\n\
         let orphan = spawn chan.recv(chan.new())\n\
         time.sleep(150)\n\
         let s = chan.stat(c)\n\
         println(`queued ${map_get(s, \"queued\")} closed ${map_get(s, \"closed\")}`)\n\
         let tasks = task.list()\n\
         println(`tasks ${len(tasks)}`)\n\
         let p = task.parked()\n\
         println(`parked ${len(p)} on ${map_get(p[0], \"on\")}`)\n\
         println(task.join(t1))");
    assert_eq!(code, 0, "introspection run failed:\n{text}");
    assert!(text.contains("queued 2 closed false"), "chan.stat:\n{text}");
    assert!(text.contains("tasks 2"), "task.list:\n{text}");
    assert!(
        text.contains("parked 1 on chan.recv on channel #2"),
        "task.parked:\n{text}"
    );
    assert!(text.contains('7'));
}

#[test]
fn closing_the_channel_is_the_clean_way_out() {
    // The hint the report gives must actually work: close, then drain.
    let (code, text) = run("let c = chan.new()\n\
         unwrap(chan.send(c, 1))\n\
         chan.close(c)\n\
         println(unwrap(chan.recv(c)))\n\
         match chan.recv(c) { Ok(v) => println(v), Err(e) => println(`closed: ${e}`) }");
    assert_eq!(code, 0, "clean close failed:\n{text}");
    assert!(text.contains("closed: channel is closed"), "{text}");
}
