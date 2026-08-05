//! The curated example programs must run — top to bottom, without error —
//! under the default execution model (interpreter + bytecode tier). This
//! keeps the showcase honest: a language change that breaks an example is a
//! CI failure, not a discovery a reader makes.

use olang::{Interpreter, Parser};

fn run_example(path: &str, source: &str) {
    // Run on a large-stack thread, matching how the `olang` binary executes
    // programs (main.rs spawns a 256 MB interpreter thread). The tree-walker
    // recurses in Rust per AST node, so deep olang recursion needs the same
    // headroom the real binary gives it — the default 2 MB test-thread stack
    // is far smaller than production.
    let path = path.to_string();
    let source = source.to_string();
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(move || {
            let parser = Parser::new();
            let program = parser
                .parse(&source)
                .unwrap_or_else(|e| panic!("{} failed to PARSE: {}", path, e));

            let mut interpreter = Interpreter::new();
            interpreter.enable_bytecode_tier(1, false);
            interpreter
                .eval_program(program)
                .unwrap_or_else(|e| panic!("{} failed to RUN: {}", path, e));
        })
        .expect("failed to spawn example thread")
        .join()
        .expect("example program panicked");
}

macro_rules! example_test {
    ($name:ident, $file:literal) => {
        #[test]
        fn $name() {
            run_example(
                concat!("examples/", $file),
                include_str!(concat!("../examples/", $file)),
            );
        }
    };
}

example_test!(language_tour, "01_language_tour.ol");
example_test!(data_pipeline, "02_data_pipeline.ol");
example_test!(algorithms, "03_algorithms.ol");
example_test!(stdlib_showcase, "04_stdlib_showcase.ol");
example_test!(text_processing, "05_text_processing.ol");
