//! The tier-differential fuzzer, committed.
//!
//! The stability chapter cites a 10,000-program differential campaign as
//! evidence that the bytecode tier agrees with the interpreter. The
//! original campaign ran from a throwaway harness; this file is the
//! permanent, reproducible replacement. The generator is fully
//! deterministic (seeded xorshift, no clock, no global RNG), so seed N
//! generates the same program on every machine forever — a divergence
//! report is a seed number.
//!
//! Two entry points:
//! - `tier_fuzz_smoke_corpus` — 150 seeds, always on, so the machinery
//!   itself is exercised by every test run.
//! - `tier_fuzz_full_campaign` — 10,000 seeds, `#[ignore]`d; run with
//!   `cargo test --test tier_fuzz_corpus_test -- --ignored` before a
//!   release or after touching the compiler or the interpreter.
//!
//! Agreement contract per seed: run `f(x)` for a spread of arguments
//! through the interpreter and through the bytecode VM.
//! - Both succeed → the values must be identical.
//! - The VM declines to compile (unsupported construct) → skip; declining
//!   is the tier's documented safe answer, and never a divergence.
//! - One side errors at runtime while the other succeeds → divergence.
//! - Both error → agreement (message equality is pinned by the dedicated
//!   error-reporting suites; the fuzzer's job is value agreement).

use olang::ast::{Statement, Value};
use olang::ovm::FunctionId;
use olang::ovm::OvmValue;
use olang::ovm::bytecode::BytecodeVm;
use olang::{Interpreter, Parser};

/// Deterministic PRNG: xorshift64*. No `rand` dependency, no seeding from
/// the environment — reproducibility is the whole point.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        // Avoid the all-zeros fixed point.
        Rng(seed.wrapping_mul(2685821657736338717).max(1))
    }
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(2685821657736338717)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// Generate an integer-valued expression over the variable `x`, depth-bounded.
/// The grammar stays inside the tier's supported subset on purpose: the goal
/// is value agreement on programs the tier accepts, not decline coverage
/// (declines are skipped, and `unsupported_features_fail_compilation` in the
/// differential suite covers refusal).
fn gen_expr(rng: &mut Rng, depth: u32) -> String {
    if depth == 0 {
        return match rng.below(3) {
            0 => format!("{}", rng.below(9) + 1),
            1 => "x".to_string(),
            _ => format!("{}", rng.below(5)),
        };
    }
    match rng.below(10) {
        0..=2 => format!(
            "({} {} {})",
            gen_expr(rng, depth - 1),
            ["+", "-", "*"][rng.below(3) as usize],
            gen_expr(rng, depth - 1)
        ),
        // Division and modulo with a denominator that cannot be zero.
        3 => format!(
            "({} / ({} % 7 + 1))",
            gen_expr(rng, depth - 1),
            gen_expr(rng, depth - 1)
        ),
        4 => format!(
            "({} % ({} % 9 + 1))",
            gen_expr(rng, depth - 1),
            gen_expr(rng, depth - 1)
        ),
        // Bitwise, with bounded shift distances.
        5 => format!(
            "({} {} {})",
            gen_expr(rng, depth - 1),
            ["&", "|", "^"][rng.below(3) as usize],
            gen_expr(rng, depth - 1)
        ),
        6 => format!("({} << {})", gen_expr(rng, depth - 1), rng.below(5)),
        7 => format!("({} >> {})", gen_expr(rng, depth - 1), rng.below(5)),
        // Branching on a comparison — `if` is an expression.
        8 => format!(
            "(if {} {} {} => {} else => {})",
            gen_expr(rng, depth - 1),
            ["<", "<=", ">", ">=", "==", "!="][rng.below(6) as usize],
            gen_expr(rng, depth - 1),
            gen_expr(rng, depth - 1),
            gen_expr(rng, depth - 1)
        ),
        // A while-loop accumulator in a block body.
        _ => format!(
            "{{ let mut acc = {} \n let mut i = 0 \n while i < {} {{ acc = acc + {} \n i = i + 1 }} \n acc }}",
            gen_expr(rng, depth - 1),
            rng.below(6) + 1,
            gen_expr(rng, depth - 1),
        ),
    }
}

fn gen_program(seed: u64) -> String {
    let mut rng = Rng::new(seed);
    let body = gen_expr(&mut rng, 3);
    format!("fn f(x) = {}", body)
}

fn interpreter_result(source: &str, args: &[Value]) -> Result<Value, String> {
    let parser = Parser::new();
    let program = parser.parse(source).map_err(|e| e.to_string())?;
    let mut interpreter = Interpreter::new();
    interpreter
        .eval_program(program)
        .map_err(|e| e.to_string())?;
    let callee = interpreter
        .get_user_variables()
        .get("f")
        .cloned()
        .cloned()
        .ok_or_else(|| "function 'f' not defined".to_string())?;
    interpreter
        .call_function(callee, args.to_vec())
        .map_err(|e| e.to_string())
}

/// Ok(None) = the VM declined to compile (skip); Ok(Some(result)) otherwise.
fn bytecode_result(source: &str, args: &[Value]) -> Result<Option<Result<Value, String>>, String> {
    let parser = Parser::new();
    let program = parser.parse(source).map_err(|e| e.to_string())?;
    let mut vm = BytecodeVm::new();
    let mut target_id = None;
    let mut ids = Vec::new();
    for statement in &program.statements {
        if let Statement::FunctionDecl(func) = statement.unwrapped() {
            let id = FunctionId::new();
            vm.register_function(func.name.clone(), id);
            ids.push((id, func.clone()));
            if func.name == "f" {
                target_id = Some(id);
            }
        }
    }
    for (id, func) in &ids {
        if vm.compile_function(*id, func).is_err() {
            return Ok(None); // decline, not divergence
        }
    }
    let target_id = target_id.ok_or_else(|| "function 'f' not defined".to_string())?;
    let ovm_args: Vec<OvmValue> = args.iter().map(|v| OvmValue::from_ast(v.clone())).collect();
    Ok(Some(
        vm.execute(target_id, &ovm_args)
            .map_err(|e| e.to_string())
            .and_then(|v| v.to_ast().map_err(|e| format!("{:?}", e))),
    ))
}

fn check_seed(seed: u64) -> Result<(), String> {
    let source = gen_program(seed);
    for arg in [0i64, 1, 2, 7, 59] {
        let args = [Value::Integer(arg)];
        let interp = interpreter_result(&source, &args);
        let vm = match bytecode_result(&source, &args) {
            Ok(Some(r)) => r,
            Ok(None) => return Ok(()), // tier declined this program
            Err(parse) => {
                return Err(format!(
                    "seed {seed}: generated program failed to parse for the VM harness: {parse}\n{source}"
                ));
            }
        };
        match (&interp, &vm) {
            (Ok(a), Ok(b)) if a == b => {}
            (Err(_), Err(_)) => {} // both raised; error-text parity is pinned elsewhere
            _ => {
                return Err(format!(
                    "seed {seed}, f({arg}): tiers diverge\n  interpreter: {interp:?}\n  bytecode:    {vm:?}\n{source}"
                ));
            }
        }
    }
    Ok(())
}

fn run_corpus(range: std::ops::Range<u64>) {
    let mut failures = Vec::new();
    for seed in range {
        if let Err(e) = std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(move || check_seed(seed))
            .expect("spawn")
            .join()
            .expect("generator panicked — that alone is a bug")
        {
            failures.push(e);
        }
    }
    assert!(
        failures.is_empty(),
        "{} divergent seeds:\n{}",
        failures.len(),
        failures.join("\n---\n")
    );
}

#[test]
fn tier_fuzz_smoke_corpus() {
    run_corpus(0..150);
}

#[test]
#[ignore = "full 10,000-seed campaign; run before a release or after compiler/interpreter changes"]
fn tier_fuzz_full_campaign() {
    run_corpus(0..10_000);
}
