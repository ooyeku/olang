//! par_map / par_filter: the parallel twins of map and filter.
//!
//! The contract under test: same results as the sequential builtins, in
//! order, for every input shape — with spawn's snapshot semantics (worker
//! clones never mutate the caller's environment) and first-in-order error
//! reporting. Each test runs both with and without the bytecode tier,
//! because workers carry their own tier and must agree either way.

use olang::ast::Value;
use olang::{Interpreter, Parser};

fn with_big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(f)
        .expect("spawn")
        .join()
        .expect("join")
}

fn eval(source: &str, tier_threshold: Option<u32>) -> Result<Value, String> {
    let source = source.to_string();
    with_big_stack(move || {
        let parser = Parser::new();
        let program = parser.parse(&source).map_err(|e| e.to_string())?;
        let mut interpreter = Interpreter::new();
        if let Some(threshold) = tier_threshold {
            interpreter.enable_bytecode_tier(threshold, false);
        }
        interpreter.eval_program(program).map_err(|e| e.to_string())
    })
}

/// The core differential: a program using par_map/par_filter must produce
/// exactly what the same program using map/filter produces — interpreted
/// and tiered.
fn assert_parallel_agrees(parallel_src: &str, sequential_src: &str) {
    for threshold in [None, Some(1)] {
        let par = eval(parallel_src, threshold);
        let seq = eval(sequential_src, threshold);
        assert_eq!(
            par, seq,
            "parallel and sequential disagree (tier: {:?})",
            threshold
        );
    }
}

#[test]
fn par_map_agrees_with_map_on_named_function() {
    assert_parallel_agrees(
        "fn double(x) = x * 2\npar_map([1, 2, 3, 4, 5], double)",
        "fn double(x) = x * 2\nmap([1, 2, 3, 4, 5], double)",
    );
}

#[test]
fn par_map_agrees_with_map_on_capturing_lambda() {
    assert_parallel_agrees(
        "let offset = 100\npar_map([1, 2, 3], (x) => x + offset)",
        "let offset = 100\nmap([1, 2, 3], (x) => x + offset)",
    );
}

#[test]
fn par_map_agrees_with_map_on_ranges() {
    assert_parallel_agrees(
        "sum(par_map(1..1000, (x) => x * x))",
        "sum(map(1..1000, (x) => x * x))",
    );
}

#[test]
fn par_map_agrees_on_structs_and_strings() {
    let par = r#"
type P = struct { name: String, score: Int }
let ps = [P { name: "a", score: 1 }, P { name: "b", score: 2 }]
par_map(ps, (p) => p.name + ":" + show(p.score * 10))
"#;
    let seq = par.replace("par_map", "map");
    assert_parallel_agrees(par, &seq);
}

#[test]
fn par_map_calls_functions_that_call_helpers() {
    // Workers must be able to resolve helper functions by name — the
    // declaration replay into the worker tier and the cloned environment
    // both have to carry them.
    assert_parallel_agrees(
        "fn helper(x) = x + 1\nfn f(x) = helper(x) * 2\npar_map([1, 2, 3, 4], f)",
        "fn helper(x) = x + 1\nfn f(x) = helper(x) * 2\nmap([1, 2, 3, 4], f)",
    );
}

#[test]
fn par_map_handles_empty_and_single() {
    assert_parallel_agrees("par_map([], (x) => x)", "map([], (x) => x)");
    assert_parallel_agrees("par_map([7], (x) => x * 3)", "map([7], (x) => x * 3)");
}

#[test]
fn par_map_large_list_matches() {
    assert_parallel_agrees(
        "sum(par_map(1..20000, (x) => (x * 17) % 11))",
        "sum(map(1..20000, (x) => (x * 17) % 11))",
    );
}

#[test]
fn par_filter_agrees_with_filter() {
    assert_parallel_agrees(
        "par_filter(1..100, (x) => x % 7 == 0)",
        "filter(1..100, (x) => x % 7 == 0)",
    );
}

#[test]
fn par_filter_drops_non_boolean_predicate_results_like_filter() {
    // filter keeps only Boolean(true); anything else drops. Same rule.
    assert_parallel_agrees(
        "par_filter([1, 2, 3], (x) => if x == 2 => true else => 0)",
        "filter([1, 2, 3], (x) => if x == 2 => true else => 0)",
    );
}

#[test]
fn par_map_reports_the_first_error_in_list_order() {
    // Elements 5 AND 7 both fail; the reported error must be element 5's,
    // exactly as the sequential builtin would report it.
    let par = r#"
fn f(x) = if x == 5 => head([]) else => if x == 7 => head([]) else => x
par_map(1..10, f)
"#;
    let seq = par.replace("par_map", "map");
    for threshold in [None, Some(1)] {
        let par_err = eval(par, threshold).expect_err("par_map should error");
        let seq_err = eval(&seq, threshold).expect_err("map should error");
        assert_eq!(par_err, seq_err, "error parity (tier: {:?})", threshold);
    }
}

#[test]
fn par_map_rejects_non_list_input_with_its_own_name() {
    let err = eval("par_map(1, (x) => x)", None).expect_err("should error");
    assert!(
        err.contains("par_map: first argument must be a list or range"),
        "got: {}",
        err
    );
    let err = eval("par_filter(1, (x) => x)", None).expect_err("should error");
    assert!(
        err.contains("par_filter: first argument must be a list or range"),
        "got: {}",
        err
    );
}

#[test]
fn par_map_arity_is_checked() {
    assert!(eval("par_map([1])", None).is_err());
    assert!(eval("par_map([1], (x) => x, 3)", None).is_err());
}

#[test]
fn par_map_reads_enclosing_state_but_cannot_write_it() {
    // The documented difference from `map`: the function runs against
    // worker clones (spawn semantics). Reading captured state is fine and
    // sees the caller's values...
    let src = r#"
let factor = 10
let out = par_map([1, 2, 3], (x) => x * factor)
show(out)
"#;
    for threshold in [None, Some(1)] {
        let result = eval(src, threshold).expect("should evaluate");
        assert_eq!(
            result,
            Value::String(std::sync::Arc::new("[10, 20, 30]".to_string())),
            "tier: {:?}",
            threshold
        );
    }
    // ...but writing it is refused before the program runs, since the
    // write could only ever land on a worker's snapshot.
    let e = eval(
        "let mut hits = 0\npar_map([1, 2], (x) => { hits = hits + 1; x })\n",
        None,
    )
    .expect_err("assigning captured state must be refused");
    assert!(e.contains("captured from an enclosing scope"), "{e}");
}

#[test]
fn par_map_workers_cannot_reach_a_cell_in_the_calling_thread() {
    // A cell is the sanctioned way to hold mutable state, and confinement
    // is what stops it from becoming the shared-mutable-state hole that
    // par_map's whole lock-free design depends on not existing.
    let e = eval(
        "let hits = cell(0)\npar_map([1, 2], (x) => cell.update(hits, (n) => n + 1))\n",
        None,
    )
    .expect_err("a worker must not reach the caller's cell");
    assert!(e.contains("cell escaped its thread"), "{e}");
}

#[test]
fn par_map_composes_with_pipelines_and_nested_use() {
    assert_parallel_agrees(
        "1..50 |> par_map((x) => x * 2) |> par_filter((x) => x % 3 == 0) |> sum",
        "1..50 |> map((x) => x * 2) |> filter((x) => x % 3 == 0) |> sum",
    );
}

#[test]
fn function_containing_par_map_stays_correct_under_the_tier() {
    // par_map is not in the VM's builtin allow-list, so a function calling
    // it must be refused (fail-closed) and stay interpreted — never
    // miscompiled. Calling it twice crosses the promotion threshold.
    let src = r#"
fn work(xs) = par_map(xs, (x) => x + 1)
sum(work([1, 2, 3])) + sum(work([4, 5, 6]))
"#;
    assert_eq!(eval(src, Some(1)), eval(src, None));
}
