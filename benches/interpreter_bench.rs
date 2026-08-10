//! Interpreter benchmarks: representative olang programs measuring the
//! classic tree-walking evaluator. Run with `cargo bench`.
//!
//! These are the yardstick for evaluator/VM work — any tiering or
//! optimization change should move these numbers, and none may change
//! program results (see the differential tests in tests/).

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use olang::{Interpreter, Parser};
use std::time::Duration;

/// Parse once, evaluate per-iteration on a shared interpreter.
fn bench_program(c: &mut Criterion, name: &str, source: &str) {
    let parser = Parser::new();
    let program = parser
        .parse(source)
        .unwrap_or_else(|e| panic!("bench '{}' failed to parse: {}", name, e));
    let mut interpreter = Interpreter::new();

    c.bench_function(name, |b| {
        b.iter(|| {
            let result = interpreter.eval_program(black_box(program.clone()));
            black_box(result).expect("bench program errored");
        })
    });
}

fn arithmetic_loop(c: &mut Criterion) {
    bench_program(
        c,
        "arithmetic_while_loop",
        r#"
let total = 0
let i = 0
while i < 1000 {
    total = total + i * 2 - 1
    i = i + 1
}
total
"#,
    );
}

fn for_range_accumulate(c: &mut Criterion) {
    bench_program(
        c,
        "for_range_accumulate",
        r#"
let total = 0
for i in 0..1000 {
    total = total + i
}
total
"#,
    );
}

fn recursive_fib(c: &mut Criterion) {
    bench_program(
        c,
        "recursive_fib_13",
        r#"
fn fib(n) = if n < 2 => n else => fib(n - 1) + fib(n - 2)
fib(13)
"#,
    );
}

fn function_call_loop(c: &mut Criterion) {
    bench_program(
        c,
        "function_call_loop",
        r#"
fn add3(a, b, c) = a + b + c
let total = 0
let i = 0
while i < 500 {
    total = add3(total, i, 1)
    i = i + 1
}
total
"#,
    );
}

fn list_pipeline(c: &mut Criterion) {
    bench_program(
        c,
        "list_map_filter_fold",
        r#"
let result = range(0, 500)
    |> map((x) => x * 3)
    |> filter((x) => x % 2 == 0)
    |> fold(0, (acc, x) => acc + x)
result
"#,
    );
}

fn string_building(c: &mut Criterion) {
    bench_program(
        c,
        "string_concat_loop",
        r#"
let s = ""
let i = 0
while i < 150 {
    s = s + "chunk" + to_string(i)
    i = i + 1
}
len(s)
"#,
    );
}

fn match_dispatch(c: &mut Criterion) {
    bench_program(
        c,
        "match_dispatch_loop",
        r#"
fn classify(n) = match n % 4 {
    0 => "zero",
    1 => "one",
    2 => "two",
    _ => "three"
}
let count = 0
let i = 0
while i < 500 {
    let c = classify(i)
    count = count + len(c)
    i = i + 1
}
count
"#,
    );
}

fn closure_heavy(c: &mut Criterion) {
    bench_program(
        c,
        "closure_capture_calls",
        r#"
fn make_adder(n) = (x) => x + n
let add5 = make_adder(5)
let total = 0
let i = 0
while i < 500 {
    total = add5(total)
    i = i + 1
}
total
"#,
    );
}

fn nested_data(c: &mut Criterion) {
    bench_program(
        c,
        "nested_list_indexing",
        r#"
let grid = map(range(0, 30), (r) => map(range(0, 30), (c) => r * c))
let total = 0
for row in grid {
    total = total + sum(row)
}
total
"#,
    );
}

fn template_strings(c: &mut Criterion) {
    bench_program(
        c,
        "template_interpolation_loop",
        r#"
let parts = map(range(0, 200), (i) => `item ${i} squared is ${i * i}`)
len(parts)
"#,
    );
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .sample_size(20)
        .warm_up_time(Duration::from_millis(500))
        .measurement_time(Duration::from_secs(3));
    targets =
    arithmetic_loop,
    for_range_accumulate,
    recursive_fib,
    function_call_loop,
    list_pipeline,
    string_building,
    match_dispatch,
    closure_heavy,
    nested_data,
    template_strings
}
criterion_main!(benches);
