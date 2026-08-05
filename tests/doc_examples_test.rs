//! Every olang code block in the documentation must work.
//!
//! Extracts fenced code blocks from README.md and docs/syntax.md and runs
//! them through the real parser and interpreter (with the bytecode tier, the
//! default execution model). Fence conventions:
//!
//! - ```olang          — must parse AND evaluate successfully
//! - ```olang no-run   — must parse; not evaluated (needs files, network,
//!                       or modules that don't exist in the test environment)
//! - anything else     — ignored (shell, text, REPL transcripts, ...)
//!
//! If an example needs context to run, the fix is to make the example
//! self-contained in the doc — readers copy-paste these.

use olang::{Interpreter, Parser};

struct DocBlock {
    file: &'static str,
    /// 1-based line where the block's code starts
    line: usize,
    code: String,
    no_run: bool,
}

fn extract_blocks(file: &'static str, content: &str) -> Vec<DocBlock> {
    let mut blocks = Vec::new();
    let mut in_block = false;
    let mut no_run = false;
    let mut start_line = 0;
    let mut code = String::new();

    for (idx, line) in content.lines().enumerate() {
        if let Some(info) = line.strip_prefix("```") {
            if in_block {
                blocks.push(DocBlock {
                    file,
                    line: start_line,
                    code: std::mem::take(&mut code),
                    no_run,
                });
                in_block = false;
            } else {
                let info = info.trim();
                let mut words = info.split_whitespace();
                if words.next() == Some("olang") {
                    in_block = true;
                    no_run = info.contains("no-run");
                    start_line = idx + 2;
                }
            }
        } else if in_block {
            code.push_str(line);
            code.push('\n');
        }
    }
    blocks
}

fn run_doc_file(file: &'static str, content: &str) {
    let blocks = extract_blocks(file, content);
    assert!(
        !blocks.is_empty(),
        "{}: no olang blocks found — extraction broken?",
        file
    );

    let parser = Parser::new();
    let mut failures = Vec::new();

    for block in &blocks {
        let program = match parser.parse(&block.code) {
            Ok(p) => p,
            Err(e) => {
                failures.push(format!(
                    "{}:{} does not PARSE: {}\n---\n{}---",
                    block.file, block.line, e, block.code
                ));
                continue;
            }
        };

        if block.no_run {
            continue;
        }

        // Fresh interpreter per block: examples must be self-contained
        let mut interpreter = Interpreter::new();
        interpreter.enable_bytecode_tier(1, false);
        if let Err(e) = interpreter.eval_program(program) {
            failures.push(format!(
                "{}:{} does not RUN: {}\n---\n{}---",
                block.file, block.line, e, block.code
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "{} documentation example(s) failed:\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}

#[test]
fn readme_examples_work() {
    run_doc_file("README.md", include_str!("../README.md"));
}

#[test]
fn syntax_reference_examples_work() {
    run_doc_file("docs/syntax.md", include_str!("../docs/syntax.md"));
}
