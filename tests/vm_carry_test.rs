//! Messages between compiled code keep the VM's form (OL-430): a list a
//! compiled function sends arrives at a compiled receiver as the same
//! value, not converted out and back. Whoever else receives it (the
//! interpreter, or code the VM could not compile) gets an equal value.

use olang::ast::Value;
use olang::{Interpreter, Parser};

fn eval(source: &str, tier: bool) -> Result<Value, String> {
    let source = source.to_string();
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(move || {
            let program = Parser::new().parse(&source).map_err(|e| e.to_string())?;
            let mut interpreter = Interpreter::new();
            if tier {
                interpreter.enable_bytecode_tier(1, false);
            }
            interpreter.eval_program(program).map_err(|e| e.to_string())
        })
        .unwrap()
        .join()
        .unwrap()
}

fn shows(source: &str) -> String {
    let interpreted = eval(source, false);
    let tiered = eval(source, true);
    assert_eq!(tiered, interpreted, "the tiers disagree on:\n{source}");
    match &interpreted {
        Ok(v) => v.to_string(),
        Err(e) => panic!("{source}\nraised: {e}"),
    }
}

const MAKE: &str = "fn make(n) = {\n    let mut xs = []\n    let mut i = 0\n    while i < n { xs = xs + [i]; i = i + 1 }\n    xs\n}\n";

#[test]
fn a_message_between_compiled_functions_is_the_value_sent() {
    let src = format!(
        "{MAKE}fn trip(ch, xs) = {{\n    chan.send(ch, xs)\n    let got = unwrap(chan.recv(ch))\n    [same(got, xs), got == xs, len(got), got[99]]\n}}\nlet ch = chan.new()\nlet xs = make(100)\nlet mut r = []\nfor i in 0..3 {{ r = trip(ch, xs) }}\nr"
    );
    // On the interpreter `same` holds as well: a message is never copied.
    assert_eq!(shows(&src), "[true, true, 100, 99]");
}

#[test]
fn every_receiver_gets_an_equal_value() {
    // Compiled senders, an interpreted receiver at the top level; a
    // compiled receiver of what the interpreter sent; try_recv and
    // recv_timeout; floats, maps, tuples, structs, enums and results.
    let src = format!(
        "{MAKE}type P = struct {{ x: Int, y: Float }}\n\
         type E = enum {{ A(Int), B }}\n\
         fn send_all(ch) = {{\n    chan.send(ch, make(50))\n    chan.send(ch, map(make(40), (i) => i * 0.5))\n    chan.send(ch, #{{ \"a\": make(3), \"b\": (1, \"x\") }})\n    chan.send(ch, [P {{ x: 1, y: 2.5 }}, A(3), B, Ok([1]), Err(\"no\")])\n    4\n}}\n\
         fn take(ch) = [unwrap(chan.try_recv(ch)), unwrap(chan.recv_timeout(ch, 1000))]\n\
         let ch = chan.new()\n\
         let mut n = 0\n\
         for i in 0..2 {{ n = send_all(ch) }}\n\
         let first = [unwrap(chan.recv(ch)), unwrap(chan.recv(ch)), unwrap(chan.recv(ch)), unwrap(chan.recv(ch))]\n\
         let second = take(ch) + take(ch)\n\
         [first == second, len(first[0]), first[1][39], first[2].b, first[3][0].y, first[3][2]]"
    );
    assert_eq!(shows(&src), "[true, 50, 19.5, (1, \"x\"), 2.5, B]");
}

#[test]
fn a_closure_is_still_sent_converted() {
    // A closure's bytecode is its VM's own: it crosses the old way.
    let src = "fn adder(n) = (x) => x + n\n\
               fn trip(ch, n) = {\n    chan.send(ch, adder(n))\n    unwrap(chan.recv(ch))(1)\n}\n\
               let ch = chan.new()\n\
               let mut r = 0\n\
               for i in 0..3 { r = trip(ch, i) }\n\
               r";
    assert_eq!(shows(src), "3");
}

#[test]
fn a_cell_inside_a_compiled_send_is_refused() {
    let src = "fn trip(ch) = {\n    let c = cell(1)\n    chan.send(ch, [c, 2])\n}\nlet ch = chan.new()\nfor i in 0..3 { trip(ch) }";
    let e = eval(src, true).unwrap_err();
    assert!(e.contains("cannot be sent through a channel"), "{e}");
    assert_eq!(eval(src, false).unwrap_err(), e);
}

#[test]
fn a_million_ints_cross_between_compiled_functions_in_constant_time() {
    // Its own process: the timings are its own.
    let program = format!(
        "{MAKE}fn trip(ch, xs, reps) = {{\n    let mut t = 0\n    let mut r = 0\n    while r < reps {{\n        chan.send(ch, xs)\n        let got = unwrap(chan.recv(ch))\n        t = t + len(got)\n        r = r + 1\n    }}\n    t\n}}\n\
         let ch = chan.new()\n\
         let small = make(1000)\n\
         let big = make(1000000)\n\
         trip(ch, small, 5)\n\
         trip(ch, big, 2)\n\
         let t0 = time.monotonic_ms()\n\
         trip(ch, small, 50)\n\
         let t1 = time.monotonic_ms()\n\
         trip(ch, big, 50)\n\
         let t2 = time.monotonic_ms()\n\
         println(`${{(t1 - t0) * 1000.0 / 50}} ${{(t2 - t1) * 1000.0 / 50}}`)\n"
    );
    let dir = std::env::temp_dir().join(format!("olang-vm-carry-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("main.ol");
    std::fs::write(&file, program).unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_olang"))
        .arg(&file)
        .output()
        .expect("run olang");
    let _ = std::fs::remove_dir_all(&dir);
    let text = String::from_utf8_lossy(&out.stdout);
    let us: Vec<f64> = text
        .split_whitespace()
        .filter_map(|w| w.parse().ok())
        .collect();
    let [small, big] = us[..] else {
        panic!(
            "unexpected output: {text}\n{}",
            String::from_utf8_lossy(&out.stderr)
        )
    };
    // Converted, a million Ints took about 10 ms each way and back.
    assert!(big < 500.0 && big < small * 20.0 + 100.0, "{text}");
}
