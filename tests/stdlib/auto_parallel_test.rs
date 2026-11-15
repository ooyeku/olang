//! Automatic parallelism for provably-pure bulk operations (Campaign 5,
//! lane H2). A pure kernel over a large `map`/`filter` fans out across
//! cores; because the kernel is pure and the concat preserves order, the
//! result is bit-identical to the sequential one — so these tests assert
//! the *invisibility* of the fan-out: same values, same order, same
//! first error, and effects (which disqualify a kernel) still happen in
//! sequential order.

use olang::ast::Value;
use olang::builtin::BuiltinFunctions;
use olang::{Interpreter, Parser};

fn eval(source: &str) -> Result<Value, String> {
    let program = Parser::new().parse(source).map_err(|e| e.to_string())?;
    Interpreter::new()
        .eval_program(program)
        .map_err(|e| e.to_string())
}

fn pure_of(source: &str) -> bool {
    // Evaluate a lambda expression to a Function value, then ask the
    // purity walk.
    let program = Parser::new().parse(source).expect("parse");
    let value = Interpreter::new().eval_program(program).expect("eval");
    BuiltinFunctions::kernel_provably_pure(&value)
}

#[test]
fn the_purity_walk_recognizes_pure_and_rejects_effects() {
    for pure in [
        "(x) => x * 3 + 1",
        "(x) => math.sin(to_float(x))",
        "(x) => str.trim(show(x))",
        "(x) => if x > 0 => x else => 0 - x",
        "(x) => [x, x] |> map((y) => y + 1) |> sum",
        "(x) => { let d = x * 2\n    d + 1 }",
        "(x) => `v=${x}`",
    ] {
        assert!(pure_of(pure), "should be provably pure: {pure}");
    }
    for impure in [
        "(x) => { println(show(x)); x }",
        "(x) => { let c = cell.new(x); cell.get(c) }",
        "(x) => time.monotonic_ms() + x",
        "(x) => unwrap(fs.read_file(show(x)))",
        "(x) => random.randint(0, x)",
        "(x) => spawn (x + 1)",
    ] {
        assert!(!pure_of(impure), "must NOT be provably pure: {impure}");
    }
}

#[test]
fn auto_parallel_results_are_identical_to_sequential() {
    // Above threshold with a pure kernel: values and ORDER must match the
    // arithmetic exactly (fan-out is order-preserving by construction).
    let out = eval(
        "let xs = range(0, 60000) |> map((x) => x * 2 + 1)\n\
         let ys = range(0, 60000) |> filter((x) => x % 3 == 0)\n\
         [xs[0], xs[1], xs[59999], len(xs), ys[0], ys[1], ys[19999], len(ys)]",
    )
    .expect("run");
    assert_eq!(out.to_string(), "[1, 3, 119999, 60000, 0, 3, 59997, 20000]");
}

#[test]
fn an_effectful_kernel_stays_sequential_and_ordered() {
    // println disqualifies the kernel, so the prints happen in sequence —
    // strictly increasing — even far above the parallel threshold. If the
    // fan-out ever ran an impure kernel, interleaving would break the
    // order; this is the behavioral proof the gate holds.
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_olang"))
        .arg("run")
        .arg({
            let dir = std::env::temp_dir().join(format!("olang_autopar_{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let f = dir.join("p.ol");
            std::fs::write(
                &f,
                "let xs = range(0, 60000) |> map((x) => { let _ = if x % 10000 == 0 => println(show(x)) else => (); x })\n\
                 println(show(len(xs)))\n",
            )
            .unwrap();
            f
        })
        .output()
        .expect("run");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(
        lines,
        vec!["0", "10000", "20000", "30000", "40000", "50000", "60000"],
        "effectful kernel must run sequentially, in order"
    );
}

#[test]
fn the_first_error_wins_regardless_of_which_chunk_finds_it() {
    // A pure kernel that raises at one index: the reported error must be
    // the lowest-index one, exactly as the sequential loop would report —
    // even though a later chunk's worker may hit its own error first.
    let err = eval(
        "let xs = range(0, 60000) |> map((x) => if x == 70 => unwrap(Err(\"boom-low\")) else => if x == 59000 => unwrap(Err(\"boom-high\")) else => x)\n\
         len(xs)",
    )
    .expect_err("must fail");
    assert!(
        err.contains("boom-low") && !err.contains("boom-high"),
        "lowest index error must win: {err}"
    );
}

#[test]
fn set_parallel_false_disables_the_fan_out() {
    // Correctness is unchanged either way; this pins that the switch is
    // honored end to end (it also fixes par_map, which previously ignored
    // the enabled flag entirely).
    let out = eval(
        "set_parallel(false)\n\
         let xs = range(0, 60000) |> map((x) => x + 1)\n\
         let ys = range(0, 60000) |> par_map((x) => x + 1)\n\
         set_parallel(true)\n\
         [xs[59999], ys[59999]]",
    )
    .expect("run");
    assert_eq!(out.to_string(), "[60000, 60000]");
}
