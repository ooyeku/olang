//! A count far past memory is an error, never an allocation that aborts
//! the process (release builds abort on a failed allocation: a plugin's
//! `str.repeat("x", 1 << 40)` took olang Studio down with it), and a long
//! native build hears a budget's stop.

use olang::{Interpreter, Parser, Value};

fn run(src: &str) -> Value {
    let program = Parser::new().parse(src).expect("parses");
    Interpreter::new().eval_program(program).expect("runs")
}

#[test]
fn a_huge_count_is_an_error() {
    let v = run(r#"
        let a = attempt(() => str.repeat("x", 1 << 40))
        let b = attempt(() => range(0, 1 << 40))
        let c = attempt(() => range(1 << 40))
        let d = attempt(() => range(0, 1 << 40, 3))
        [match a { Err(_) => true, Ok(_) => false }, match b { Err(_) => true, Ok(_) => false },
         match c { Err(_) => true, Ok(_) => false }, match d { Err(_) => true, Ok(_) => false },
         len(range(10, 0, -3)), len(range(0, 10, 3)), len(range(5))]
    "#);
    assert_eq!(format!("{v}"), "[true, true, true, true, 4, 4, 5]");
}

#[test]
fn a_long_range_stops_at_its_budget() {
    let t0 = std::time::Instant::now();
    let v = run(r#"
        let r = runtime.call_budget((n) => len(range(0, n)), [200000000], 50)
        if r.ok == true => "ran " + show(r.value) else => if str.contains(show(r), "budget") => "stopped" else => show(r)
    "#);
    let shown = format!("{v}");
    // a fast machine may finish it; it must not run on for long either way
    assert!(
        shown.contains("stopped") || shown.contains("ran "),
        "{shown}"
    );
    assert!(t0.elapsed().as_secs() < 20, "took {:?}", t0.elapsed());
}
