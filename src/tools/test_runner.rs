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
use std::collections::{BTreeSet, HashMap};
use std::path::Path;

pub fn run(path: &Path, coverage: bool, show_missing: bool) -> i32 {
    let started = std::time::Instant::now();

    // Aggregated coverage across every test file: source path -> the set of
    // lines executed in it. Empty (and left untouched) unless `--coverage`.
    let mut coverage_hits: HashMap<String, BTreeSet<u32>> = HashMap::new();

    // A path the user named that doesn't exist is a mistake, not "no tests":
    // reporting success for a typo'd path lets CI pass over nothing.
    if !path.exists() {
        eprintln!("error: path not found: {}", path.display());
        return 1;
    }
    let files = super::discover_ol_files(path);

    let mut total_passed = 0usize;
    let mut total_failed = 0usize;
    let mut file_errors = 0usize;
    let mut parse_errors = 0usize;
    let mut test_files = 0usize;

    let parser = Parser::new();

    for file in &files {
        let source = match std::fs::read_to_string(file) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let program = match parser.parse(&source) {
            Ok(p) => p,
            // A file that fails to parse is a real error, not "no tests here".
            // Silently skipping it lets a syntax error in a test file pass CI.
            Err(e) => {
                parse_errors += 1;
                println!("  {} {}", "✗".red().bold(), file.display());
                println!("      {}", e.to_string().red());
                continue;
            }
        };
        let has_tests = program
            .statements
            .iter()
            .any(|s| matches!(s.unwrapped(), Statement::TestDecl(_)));
        if !has_tests {
            continue;
        }

        test_files += 1;
        println!("{}", file.display().to_string().bright_blue());

        let mut interpreter = Interpreter::new();
        interpreter.enable_test_mode();
        if coverage {
            // Coverage instruments the AST walk (a promoted function would
            // run past the hook unrecorded), so run on the interpreter tier
            // — the semantic oracle — instead of the bytecode tier.
            interpreter.enable_coverage();
        } else {
            // Run tests through the bytecode tier, exactly as `olang <file>`
            // does by default — so tests execute at production speed and
            // exercise the tier that ships (promotion threshold 1, default).
            interpreter.enable_bytecode_tier(1, false);
        }
        let absolute = file.canonicalize().unwrap_or_else(|_| file.to_path_buf());
        interpreter.set_current_file(&absolute);

        // Resolve the file's package dependencies, as `olang <file>` would.
        if let Some(root) = crate::pkg::manifest::Manifest::find_root(&absolute)
            && let Ok(map) = crate::pkg::install(&root, &crate::pkg::InstallOptions::default())
        {
            interpreter.set_dependency_map(map.into_iter().collect());
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

        // Fold this file's recorded lines into the run-wide tally. Keyed by
        // the file that owns each line, so a helper defined in one file and
        // exercised by a test in another lands under the file it lives in.
        if let Some(hits) = interpreter.take_coverage() {
            for (file, lines) in hits {
                coverage_hits.entry(file).or_default().extend(lines);
            }
        }

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
            // Name the statement that ran: a top-level effect the module
            // author did not expect `olang test` to execute (a `mount`, a
            // timer) is the usual cause, and "file error" alone sends them
            // looking at the test blocks instead.
            if let Some(loc) = interpreter.take_error_location() {
                let where_ = match &loc.file {
                    Some(f) => format!("{}:{}:{}", f, loc.line, loc.column),
                    None => format!("{}:{}:{}", file.display(), loc.line, loc.column),
                };
                println!(
                    "      {}",
                    format!(
                        "at {} — a top-level statement outside any test block; guard \
                         effects that only make sense when run (`if dom.available()`, a \
                         `main` the runner never calls), or move them below the tests",
                        where_
                    )
                    .dimmed()
                );
            }
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
        // Unparseable files still make the run fail, even with no test blocks.
        return if parse_errors > 0 { 1 } else { 0 };
    }
    let summary = format!(
        "  {} passed, {} failed  ({} test file{}, {}ms)",
        total_passed,
        total_failed + file_errors,
        test_files,
        if test_files == 1 { "" } else { "s" },
        elapsed
    );
    let failed = total_failed + file_errors + parse_errors > 0;
    if failed {
        println!("{}", summary.red().bold());
    } else {
        println!("{}", summary.green());
    }

    // Coverage is a report, not a gate: it never changes the exit code, so
    // a green suite with thin coverage still passes (and can be tightened
    // later). `--coverage-lines` additionally lists the uncovered lines.
    if coverage {
        super::coverage::print_report(&coverage_hits, show_missing);
    }

    if failed { 1 } else { 0 }
}
