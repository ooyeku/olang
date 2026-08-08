//! `olang test [path]` — discover files containing `test` blocks and run
//! them, one fresh interpreter per file, reporting every block's outcome.
//!
//! A "test file" is any `.ol` file with a top-level `test "name" { ... }`
//! block. The whole file executes top to bottom (so fixtures and helper
//! definitions work exactly as they do under `olang file.ol`); in runner
//! mode a failing block records its failure and execution continues, so one
//! red test doesn't hide the rest. Files inside a package get the package's
//! dependency map, so `use` resolves the same way it does when running.

use crate::ast::Statement;
use crate::{Interpreter, Parser};
use colored::*;
use std::path::Path;

pub fn run(path: &Path) -> i32 {
    let started = std::time::Instant::now();
    let files = super::discover_ol_files(path);

    let mut total_passed = 0usize;
    let mut total_failed = 0usize;
    let mut file_errors = 0usize;
    let mut test_files = 0usize;

    let parser = Parser::new();

    for file in &files {
        let source = match std::fs::read_to_string(file) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let program = match parser.parse(&source) {
            Ok(p) => p,
            // Unparseable files are simply not test files (the compiler of
            // record for parse errors is `olang <file>` itself).
            Err(_) => continue,
        };
        let has_tests = program
            .statements
            .iter()
            .any(|s| matches!(s, Statement::TestDecl(_)));
        if !has_tests {
            continue;
        }

        test_files += 1;
        println!("{}", file.display().to_string().bright_blue());

        let mut interpreter = Interpreter::new();
        interpreter.enable_test_mode();
        let absolute = file.canonicalize().unwrap_or_else(|_| file.to_path_buf());
        interpreter.set_current_file(&absolute);

        // Resolve the file's package dependencies, as `olang <file>` would.
        if let Some(root) = crate::pkg::manifest::Manifest::find_root(&absolute) {
            if let Ok(map) = crate::pkg::install(&root, &crate::pkg::InstallOptions::default()) {
                interpreter.set_dependency_map(map.into_iter().collect());
            }
        }

        // Run from the file's own directory so relative imports and file
        // reads resolve exactly as under `olang <file>` run from there.
        let saved_cwd = std::env::current_dir().ok();
        if let Some(dir) = absolute.parent() {
            let _ = std::env::set_current_dir(dir);
        }
        let run_error = interpreter.eval_program(program).err();
        if let Some(cwd) = saved_cwd {
            let _ = std::env::set_current_dir(cwd);
        }
        let results = interpreter.take_test_results();

        for outcome in &results {
            match &outcome.error {
                None => {
                    total_passed += 1;
                    println!("  {} {}", "✓".green(), outcome.name);
                }
                Some(msg) => {
                    total_failed += 1;
                    println!("  {} {}", "✗".red().bold(), outcome.name.bold());
                    println!("      {}", msg.red());
                }
            }
        }

        // An error outside any test block (setup code failed) is its own
        // failure — the file's tests can't be trusted to have all run.
        if let Some(e) = run_error {
            file_errors += 1;
            println!("  {} {}", "✗".red().bold(), "file error".bold());
            println!("      {}", e.to_string().red());
        }
    }

    let elapsed = started.elapsed().as_millis();
    println!("{}", "─".repeat(40));
    if test_files == 0 {
        println!(
            "  no test blocks found under {} ({} .ol files scanned)",
            path.display(),
            files.len()
        );
        return 0;
    }
    let summary = format!(
        "  {} passed, {} failed  ({} test file{}, {}ms)",
        total_passed,
        total_failed + file_errors,
        test_files,
        if test_files == 1 { "" } else { "s" },
        elapsed
    );
    if total_failed + file_errors > 0 {
        println!("{}", summary.red().bold());
        1
    } else {
        println!("{}", summary.green());
        0
    }
}
