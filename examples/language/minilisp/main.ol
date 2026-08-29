// minilisp — a small Lisp, interpreted by olang.
//
// An interpreter interpreting an interpreter: the reader turns source text
// into LVal enum trees, the evaluator walks them with maps as environments
// and Err values as the only error channel. Closures capture their
// environment by value; a defined lambda becomes self-aware at apply time
// (call-time self-binding), which is exactly how olang's own interpreter
// handles named functions one level up.
//
// This is deliberately the workload class that stresses the bytecode tier
// hardest: deep mutual recursion over enum trees, pattern matching in every
// function, functional map threading, and higher-order Lisp code running on
// higher-order olang code.

use lib.reader { read_program }
use lib.eval { run }

fn run_src(src) = match read_program(src) {
    Ok(forms) => match run(forms) { Ok(v) => show(v), Err(e) => "error: " + e },
    Err(e) => "parse error: " + e
}

println("═══ minilisp ═══")
println("")

let programs = [
    ("arithmetic", "(* (+ 1 2) (- 10 3))"),
    ("factorial", "(define fact (lambda (n) (if (< n 2) 1 (* n (fact (- n 1)))))) (fact 12)"),
    ("fibonacci", "(define fib (lambda (n) (if (< n 2) n (+ (fib (- n 1)) (fib (- n 2)))))) (fib 15)"),
    ("higher-order", "(define twice (lambda (f x) (f (f x)))) (twice (lambda (n) (* n 3)) 7)"),
    ("lisp map", "(define map (lambda (f xs) (if (null? xs) (list) (cons (f (car xs)) (map f (cdr xs)))))) (map (lambda (n) (* n n)) (list 1 2 3 4 5))"),
    ("closures", "(define make-adder (lambda (n) (lambda (x) (+ x n)))) (define add5 (make-adder 5)) (add5 37)"),
    ("error value", "(car (list))")
]

for entry in programs {
    let (label, src) = entry
    println("  " + str.pad_end(label, 14, " ") + run_src(src))
}

// ── benchmark: fib(17) through two layers of interpretation ──

let bench_src = "(define fib (lambda (n) (if (< n 2) n (+ (fib (- n 1)) (fib (- n 2)))))) (fib 17)"
let t0 = time.monotonic_ms()
let bench_result = run_src(bench_src)
let t1 = time.monotonic_ms()
println("")
println("  lisp fib(17): " + bench_result + "  in " + show(t1 - t0) + "ms")

test "minilisp evaluates correctly" {
    assert_eq(run_src("(* (+ 1 2) (- 10 3))"), "LVal.LNum(21)")
    assert_eq(run_src("(define fact (lambda (n) (if (< n 2) 1 (* n (fact (- n 1)))))) (fact 12)"), "LVal.LNum(479001600)")
    assert_eq(run_src("(define twice (lambda (f x) (f (f x)))) (twice (lambda (n) (* n 3)) 7)"), "LVal.LNum(63)")
    assert_eq(run_src("(define make-adder (lambda (n) (lambda (x) (+ x n)))) ((make-adder 5) 37)"), "LVal.LNum(42)")
    assert_eq(run_src("(car (list))"), "error: car: empty list")
    assert_eq(run_src("(nope 1)"), "error: unbound symbol: nope")
}
