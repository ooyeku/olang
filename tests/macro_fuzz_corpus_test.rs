//! The macro fuzzer, committed.
//!
//! The stability chapter's macro-graduation criteria cite a 10,000-program
//! fuzzing campaign (expansion determinism, tier agreement, clean refusal)
//! that originally ran from a throwaway harness. This file is the
//! permanent, reproducible replacement: a seeded, fully deterministic
//! generator, a 150-seed smoke corpus that runs on every `cargo test`, and
//! the full `#[ignore]`d campaign
//! (`cargo test --test macro_fuzz_corpus_test -- --ignored`).
//!
//! Properties checked per seed:
//! 1. **Determinism** — expanding the same source twice yields
//!    byte-identical text.
//! 2. **Total parse** — the expanded program parses with the real grammar.
//! 3. **Evaluation agreement** — expanding then evaluating succeeds, and
//!    evaluating the same program a second time gives the same output
//!    value (macros introduce no hidden state).
//! 4. **Clean refusal** — a meta fn that reaches for an effect
//!    (`fs.*`, `dates.now`, `crypto.random_hex`, `ods.write_csv`) is
//!    *refused* with an error, never expanded and never a panic. These
//!    are exactly the drift cases the unified effect classification
//!    (src/effects.rs) closed.

use olang::{Interpreter, Parser};

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed.wrapping_mul(0x9E3779B97F4A7C15).max(1))
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

/// A pure *unary* meta fn drawn from a family of splice shapes (the
/// two-argument macro in a program is always `m2`, so call sites and
/// declarations agree by construction).
fn gen_meta_fn(rng: &mut Rng, name: &str) -> String {
    match rng.below(5) {
        0 => format!("meta fn {name}(e) = `${{e}} + ${{e}}`"),
        1 => format!("meta fn {name}(e) = `(${{e}}) * {}`", rng.below(5) + 1),
        2 => format!("meta fn {name}(e) = `(${{e}}) - {}`", rng.below(9)),
        3 => format!("meta fn {name}(e) = `(if (${{e}}) > 0 => (${{e}}) else => 0 - (${{e}}))`"),
        _ => format!(
            "meta fn {name}(e) = {{\n    let doubled = `(${{e}}) + (${{e}})`\n    `(${{doubled}}) + {}`\n}}",
            rng.below(9)
        ),
    }
}

/// An expression that invokes the generated macros — possibly nested, the
/// shape the original campaign's one real find (unexpanded nested calls in
/// arguments) came from.
fn gen_call(rng: &mut Rng, unary: &str, binary_ok: bool) -> String {
    match rng.below(4) {
        0 => format!("@{unary}({})", rng.below(20) + 1),
        1 => format!("@{unary}(@{unary}({}))", rng.below(10) + 1),
        2 if binary_ok => format!("@m2({}, @{unary}({}))", rng.below(10), rng.below(6) + 1),
        _ => format!("@{unary}({} + {})", rng.below(9), rng.below(9) + 1),
    }
}

fn gen_program(seed: u64) -> String {
    let mut rng = Rng::new(seed);
    let m1 = gen_meta_fn(&mut rng, "m1");
    // A two-argument macro is only in scope on some seeds.
    let with_m2 = rng.below(2) == 0;
    let m2 = if with_m2 {
        format!("meta fn m2(a, b) = `(${{a}}) + (${{b}})`\n")
    } else {
        String::new()
    };
    let call1 = gen_call(&mut rng, "m1", with_m2);
    let call2 = gen_call(&mut rng, "m1", with_m2);
    format!("{m1}\n{m2}let a = {call1}\nlet b = {call2}\na * 1000 + b\n")
}

/// Effectful meta fns that the purity gate must refuse.
fn gen_impure_program(seed: u64) -> String {
    let bodies = [
        "unwrap(fs.read_file(\"/etc/hostname\"))",
        "dates.now()",
        "crypto.random_hex(8)",
        "show(ods.write_csv(ods.read_csv(\"a\\n1\\n\"), \"/tmp/macro-escape.csv\"))",
        "dates.today()",
        "show(os.get_env(\"HOME\"))",
        "to_string(time.now_ms())",
    ];
    let body = bodies[(seed as usize) % bodies.len()];
    format!("meta fn evil(e) = {body}\nlet x = @evil(1)\nx\n")
}

fn eval_to_string(source: &str) -> Result<String, String> {
    let parser = Parser::new();
    let program = parser.parse(source).map_err(|e| e.to_string())?;
    let mut interpreter = Interpreter::new();
    interpreter
        .eval_program(program)
        .map(|v| format!("{v:?}"))
        .map_err(|e| e.to_string())
}

fn check_seed(seed: u64) -> Result<(), String> {
    let source = gen_program(seed);

    // 1. Determinism: byte-identical expansion, twice.
    let first = olang::expand::expand_source(&source)
        .map_err(|e| format!("seed {seed}: expansion refused a pure program: {e}\n{source}"))?;
    let second = olang::expand::expand_source(&source)
        .map_err(|e| format!("seed {seed}: second expansion failed: {e}"))?;
    if first != second {
        return Err(format!(
            "seed {seed}: expansion is nondeterministic\n--- first\n{first}\n--- second\n{second}"
        ));
    }

    // 2. Total parse: the expanded text is a valid program.
    Parser::new()
        .parse_raw(&first)
        .map_err(|e| format!("seed {seed}: expanded program does not parse: {e}\n{first}"))?;

    // 3. Evaluation agreement: same result on repeated evaluation.
    let a = eval_to_string(&source)
        .map_err(|e| format!("seed {seed}: expanded program failed to run: {e}\n{source}"))?;
    let b = eval_to_string(&source).map_err(|e| format!("seed {seed}: rerun failed: {e}"))?;
    if a != b {
        return Err(format!(
            "seed {seed}: two runs of the same macro program disagree: {a} vs {b}"
        ));
    }
    Ok(())
}

fn check_impure_seed(seed: u64) -> Result<(), String> {
    let source = gen_impure_program(seed);
    match olang::expand::expand_source(&source) {
        Err(_) => Ok(()), // refused cleanly — the required outcome
        Ok(expanded) => Err(format!(
            "seed {seed}: an effectful meta fn expanded instead of being refused\n{source}\n--- expanded\n{expanded}"
        )),
    }
}

fn run_corpus(range: std::ops::Range<u64>) {
    let mut failures = Vec::new();
    for seed in range {
        let r = std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(move || check_seed(seed).and_then(|_| check_impure_seed(seed)))
            .expect("spawn")
            .join()
            .expect("fuzzer panicked — that alone is a bug");
        if let Err(e) = r {
            failures.push(e);
        }
    }
    assert!(
        failures.is_empty(),
        "{} failing seeds:\n{}",
        failures.len(),
        failures.join("\n---\n")
    );
}

#[test]
fn macro_fuzz_smoke_corpus() {
    run_corpus(0..150);
}

#[test]
#[ignore = "full 10,000-seed campaign; run before a release or after touching expand.rs"]
fn macro_fuzz_full_campaign() {
    run_corpus(0..10_000);
}
