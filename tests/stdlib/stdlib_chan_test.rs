//! `chan` — message passing between `spawn`ed tasks. Channels are
//! multi-producer multi-consumer queues of olang values; everything
//! fallible speaks Result, and closing is cooperative: sends fail after
//! close, but queued messages still drain before recv reports it.

use olang::Value;
use olang::interpreter::Interpreter;
use olang::parser::Parser;

fn eval(src: &str) -> Value {
    let program = Parser::new().parse(src).expect("parse");
    Interpreter::new().eval_program(program).expect("eval")
}

#[test]
fn send_and_recv_buffer_in_order() {
    let v = eval(
        r#"
let c = chan.new()
chan.send(c, 1)
chan.send(c, 2)
chan.send(c, 3)
unwrap(chan.recv(c)) * 100 + unwrap(chan.recv(c)) * 10 + unwrap(chan.recv(c))
"#,
    );
    assert_eq!(v, Value::Integer(123));
}

#[test]
fn try_recv_reports_empty_without_blocking() {
    let v = eval(
        r#"
let c = chan.new()
let empty = chan.try_recv(c)
unwrap(chan.send(c, 5))
let one = chan.try_recv(c)
chan.close(c)
[empty, one, chan.try_recv(c)]
"#,
    );
    // nothing yet is Ok(()); a closed, drained channel the named sentinel
    assert_eq!(
        v,
        Value::List(
            vec![
                Value::Ok(Box::new(Value::Unit)),
                Value::Ok(Box::new(Value::Integer(5))),
                Value::Err(Box::new(Value::String(
                    "channel is closed".to_string().into()
                )))
            ]
            .into()
        )
    );
}

#[test]
fn close_fails_sends_but_drains_queued_messages() {
    let v = eval(
        r#"
let c = chan.new()
chan.send(c, 7)
chan.close(c)
let after_close = match chan.send(c, 8) {
    Ok(v) => "sent",
    Err(e) => e
}
let drained = unwrap(chan.recv(c))
let then = match chan.recv(c) {
    Ok(v) => "value",
    Err(e) => e
}
after_close + " " + to_string(drained) + " " + then
"#,
    );
    assert_eq!(
        v,
        Value::String("channel is closed 7 channel is closed".to_string().into())
    );
}

#[test]
fn close_is_idempotent() {
    let v = eval(
        r#"
let c = chan.new()
chan.close(c)
chan.close(c)
match chan.recv(c) {
    Ok(v) => "value",
    Err(e) => e
}
"#,
    );
    assert_eq!(v, Value::String("channel is closed".to_string().into()));
}

#[test]
fn recv_timeout_times_out_on_a_quiet_channel() {
    let v = eval(
        r#"
let c = chan.new()
match chan.recv_timeout(c, #{ "timeout_ms": 30 }) {
    Ok(v) => "value",
    Err(e) => e
}
"#,
    );
    assert_eq!(v, Value::String("timed out".to_string().into()));
}

#[test]
fn spawned_producer_feeds_a_consuming_loop() {
    let v = eval(
        r#"
fn producer(ch, n) = {
    let mut i = 1
    while i <= n {
        chan.send(ch, i)
        i = i + 1
    }
    chan.close(ch)
    n
}
let pipe = chan.new()
let worker = spawn producer(pipe, 5)
let mut total = 0
let mut going = true
while going {
    match chan.recv(pipe) {
        Ok(v) => { total = total + v }
        Err(e) => { going = false }
    }
}
total * 10 + unwrap(task.join(worker))
"#,
    );
    assert_eq!(v, Value::Integer(155));
}

#[test]
fn request_reply_between_tasks() {
    // A worker that doubles what it receives and replies on a second
    // channel — the classic two-channel request/reply shape.
    let v = eval(
        r#"
fn worker(req, rep) = {
    let mut going = true
    while going {
        match chan.recv(req) {
            Ok(v) => { chan.send(rep, v * 2) }
            Err(e) => { going = false }
        }
    }
    chan.close(rep)
    0
}
let req = chan.new()
let rep = chan.new()
let worker = spawn worker(req, rep)
chan.send(req, 10)
chan.send(req, 20)
let a = unwrap(chan.recv(rep))
let b = unwrap(chan.recv(rep))
chan.close(req)
unwrap(task.join(worker))
a + b
"#,
    );
    assert_eq!(v, Value::Integer(60));
}

#[test]
fn rendezvous_channels_hand_off_across_threads() {
    // capacity 0: a send blocks until a receiver takes the value, so the
    // spawned sender and the main thread meet at the handoff.
    let v = eval(
        r#"
fn sender(ch) = {
    chan.send(ch, 42)
    chan.close(ch)
    0
}
let c = chan.bounded(0)
let worker = spawn sender(c)
let got = unwrap(chan.recv(c))
unwrap(task.join(worker))
got
"#,
    );
    assert_eq!(v, Value::Integer(42));
}

#[test]
fn heap_values_cross_the_channel_intact() {
    let v = eval(
        r#"
type P = struct { x: Int, y: Int }
let c = chan.new()
chan.send(c, [1, 2, 3])
chan.send(c, #{ "k": "v" })
chan.send(c, P { x: 3, y: 4 })
let xs = unwrap(chan.recv(c))
let m = unwrap(chan.recv(c))
let p = unwrap(chan.recv(c))
to_string(xs[2]) + " " + map_get(m, "k") + " " + to_string(p.x + p.y)
"#,
    );
    assert_eq!(v, Value::String("3 v 7".to_string().into()));
}

#[test]
fn misuse_errors_speak_plainly() {
    let program = Parser::new().parse("chan.send(5, 1)\n").expect("parse");
    let err = Interpreter::new()
        .eval_program(program)
        .expect_err("not a channel");
    assert!(
        err.to_string()
            .contains("chan: expected a channel (from chan.new)"),
        "got: {}",
        err
    );
}

/// A program that opens and drops many channels must not grow memory —
/// the channel handle owns its channel by Arc, so the last drop frees it.
/// (Regression: channels used to be pinned in a process-wide registry that
/// `close` never removed, leaking every channel ever created.)
#[test]
fn channels_are_reclaimed_when_the_handle_drops() {
    use olang::{Interpreter, Parser};
    // 5000 channels, each opened, used, closed, and dropped. Correctness
    // proxy for the leak fix: this completes and the checksum is right;
    // the memory behaviour is verified by the soak harness in the repo.
    let src = "\
let mut total = 0
for i in range(0, 5000) {
    let c = chan.bounded(2)
    unwrap(chan.send(c, i))
    total = total + unwrap(chan.recv(c))
    chan.close(c)
}
total
";
    let program = Parser::new().parse(src).expect("parse");
    let mut interp = Interpreter::new();
    interp.enable_bytecode_tier(1, false);
    let result = interp.eval_program(program).expect("run");
    // sum 0..5000
    assert_eq!(format!("{}", result), "12497500");
}

/// A program that waits for its `os.on_shutdown` handler to send is not a
/// deadlock: the handler is a wake from outside, which a signal runs on a
/// thread of its own. The stall detector aborted it (exit 101) half a
/// second after it started (OL-363); a channel nothing can send on is
/// still reported.
#[cfg(unix)]
#[test]
fn a_program_waiting_on_its_shutdown_handler_is_not_a_deadlock() {
    use std::process::{Command, Stdio};
    let dir = std::env::temp_dir().join(format!("olang_ol363_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("s.ol"),
        "let stop = chan.new()\nos.on_shutdown((why) => { chan.send(stop, why) })\nprintln(\"waiting\")\nprintln(show(chan.recv(stop)))\nprintln(\"clean exit\")\n",
    )
    .unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_olang"))
        .arg("s.ol")
        .current_dir(&dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // well past the detector's half second
    std::thread::sleep(std::time::Duration::from_millis(1500));
    let killed = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(killed.success());
    let out = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        stdout,
        "waiting\nOk(\"shutdown\")\nclean exit\n",
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success());

    std::fs::write(
        dir.join("d.ol"),
        "let c = chan.new()\nprintln(chan.recv(c))\n",
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_olang"))
        .arg("d.ol")
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&out.stderr).contains("deadlock"));
}

// OL-374: a sender parked on a full bounded (or rendezvous) channel wakes
// with the close instead of waiting for a receiver, and its value is not
// queued behind the close.
#[test]
fn close_releases_a_sender_blocked_on_a_full_channel() {
    let v = eval(
        r#"
let b = chan.bounded(1)
unwrap(chan.send(b, 1))
let s = spawn chan.send(b, 2)
time.sleep(100)
chan.close(b)
let sent = task.join_timeout(s, 2000)
let first = chan.recv(b)
let after = chan.recv(b)

let r = chan.bounded(0)
let s2 = spawn chan.send(r, "x")
time.sleep(100)
chan.close(r)
let sent2 = task.join_timeout(s2, 2000)
let after2 = chan.recv(r)
show([sent, first, after, sent2, after2])
"#,
    );
    assert_eq!(
        v,
        Value::String(
            r#"[Ok(Ok(Err("channel is closed"))), Ok(1), Err("channel is closed"), Ok(Ok(Err("channel is closed"))), Err("channel is closed")]"#
                .to_string()
                .into()
        )
    );
}
