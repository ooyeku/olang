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

/// How a run is narrowed and reported: `only` keeps the blocks whose name
/// contains it, `times` prints each block's milliseconds and the slowest
/// at the end, `deps` runs the blocks of the project's dependencies too
/// (by default only the project's own files' blocks run).
#[derive(Default)]
pub struct Options {
    pub only: Option<String>,
    pub times: bool,
    pub deps: bool,
    /// `--format json`: the run as line-delimited JSON events
    /// (`super::test_events`) instead of the human report.
    pub json: bool,
    /// `--record-args` (with `--format json`): after each file, the last
    /// call its test blocks made to each of the project's functions, its
    /// arguments as olang source (an `args` event each).
    pub record_args: bool,
    /// `--fail-if-none`: a run in which no test block ran exits 1 — no
    /// file has one, or `--only` matched none — so CI cannot pass over
    /// nothing.
    pub fail_if_none: bool,
}

/// The project a file belongs to — its package root, canonical; None
/// outside any package, where every file on disk is the project's — and
/// the canonical directories its dependencies resolved to. A dependency
/// that contains the root itself is no dependency of it.
fn project_scope(
    root: Option<&Path>,
    deps: Option<&crate::pkg::DependencyMap>,
) -> (Option<std::path::PathBuf>, Vec<std::path::PathBuf>) {
    let root = root.map(|r| r.canonicalize().unwrap_or_else(|_| r.to_path_buf()));
    let dirs = deps
        .map(|m| {
            m.values()
                .filter_map(|d| d.canonicalize().ok())
                .filter(|d| root.as_ref().is_none_or(|r| !r.starts_with(d)))
                .collect()
        })
        .unwrap_or_default();
    (root, dirs)
}

/// A file as the JSON events name it: its canonical path.
fn abs_name(file: &Path) -> String {
    file.canonicalize()
        .unwrap_or_else(|_| file.to_path_buf())
        .display()
        .to_string()
}

/// A file's program parsed for the runner. The file's directory anchors
/// its macro expansion: a nested package's file resolves its own `use
/// lib.x` against its package, not the project the runner was started
/// from. A notebook's own problems (`notebook::Program::diagnostics`)
/// fail it as a parse error does.
fn parse_program(
    parser: &Parser,
    file: &Path,
    program: crate::notebook::Program,
) -> Result<crate::ast::Program, String> {
    if !program.diagnostics.is_empty() {
        return Err(crate::notebook::report(file, &program.diagnostics));
    }
    parser
        .parse_with_dir(&program.source, file.parent())
        .map_err(|e| e.to_string())
}

pub fn run(path: &Path, coverage: bool, show_missing: bool) -> i32 {
    run_with(path, coverage, show_missing, &Options::default())
}

pub fn run_with(path: &Path, coverage: bool, show_missing: bool, options: &Options) -> i32 {
    run_paths(&[path.to_path_buf()], coverage, show_missing, options)
}

/// `olang test a.ol tests/a_test.ol`: several files (or directories) in
/// one run — an editor's "this file and its tests" — each file once.
pub fn run_paths(
    paths: &[std::path::PathBuf],
    coverage: bool,
    show_missing: bool,
    options: &Options,
) -> i32 {
    let started = std::time::Instant::now();
    let path: &Path = match paths.first() {
        Some(p) => p,
        None => Path::new("."),
    };
    // A named file runs its own blocks; a directory runs every file in it
    // (an imported module's blocks once, as before).
    let own_blocks = paths.iter().all(|p| p.is_file());
    let mut slowest: Vec<(f64, String, String)> = Vec::new();

    // Aggregated coverage across every test file: source path -> the set of
    // lines executed in it. Empty (and left untouched) unless `--coverage`.
    let mut coverage_hits: HashMap<String, BTreeSet<u32>> = HashMap::new();

    // A path the user named that doesn't exist is a mistake, not "no tests":
    // reporting success for a typo'd path lets CI pass over nothing.
    if let Some(missing) = paths.iter().find(|p| !p.exists()) {
        eprintln!("error: path not found: {}", missing.display());
        return 2;
    }
    let mut files = super::discover_sources(path);
    for more in paths.iter().skip(1) {
        for f in super::discover_sources(more) {
            let c = f.canonicalize().ok();
            if !files
                .iter()
                .any(|g| g == &f || (c.is_some() && g.canonicalize().ok() == c))
            {
                files.push(f);
            }
        }
    }
    // A dependency vendored inside the project (a path dependency below
    // its root) is walked past too, unless `--deps` asks for it.
    if !options.deps
        && let Ok(abs) = path.canonicalize()
        && let Some(root) = crate::pkg::manifest::Manifest::find_root(&abs)
        && let Ok(map) = crate::pkg::install(&root, &crate::pkg::InstallOptions::default())
    {
        let (_, dep_dirs) = project_scope(Some(&root), Some(&map));
        if !dep_dirs.is_empty() {
            files.retain(|f| {
                f.canonicalize()
                    .map(|c| !dep_dirs.iter().any(|d| c.starts_with(d)))
                    .unwrap_or(true)
            });
        }
    }

    let mut total_passed = 0usize;
    let mut total_failed = 0usize;
    let mut file_errors = 0usize;
    let mut parse_errors = 0usize;
    let mut test_files = 0usize;
    let json = options.json;

    let parser = Parser::new();
    // `--format json` says up front which files hold blocks and how many:
    // the files are parsed once, here, and the walk below reuses them.
    let mut parsed: HashMap<std::path::PathBuf, Result<crate::ast::Program, String>> =
        HashMap::new();
    if json {
        super::test_events::enable();
        super::test_events::set_aliases(paths);
        let mut with_tests = Vec::new();
        let mut count = 0usize;
        for file in &files {
            let Ok(source) = crate::notebook::read(file, crate::notebook::Mode::Run) else {
                continue;
            };
            let program = parse_program(&parser, file, source);
            if let Ok(p) = &program {
                let blocks = p
                    .statements
                    .iter()
                    .filter_map(|s| match s.unwrapped() {
                        Statement::TestDecl(t) => Some(t),
                        _ => None,
                    })
                    .filter(|t| options.only.as_deref().is_none_or(|o| t.name.contains(o)))
                    .count();
                let any = p
                    .statements
                    .iter()
                    .any(|s| matches!(s.unwrapped(), Statement::TestDecl(_)));
                if any {
                    with_tests.push(abs_name(file));
                    count += blocks;
                }
            } else {
                with_tests.push(abs_name(file));
            }
            parsed.insert(file.clone(), program);
        }
        super::test_events::run_started(&with_tests, count, options.only.as_deref());
    }
    // Every file the walk will visit: a module among them runs its blocks
    // as a file of its own, not again when another file imports it.
    let entries: std::sync::Arc<std::collections::HashSet<std::path::PathBuf>> =
        std::sync::Arc::new(files.iter().filter_map(|f| f.canonicalize().ok()).collect());

    for file in &files {
        let pre = parsed.remove(file);
        let program = if let Some(p) = pre {
            p
        } else {
            let source = match crate::notebook::read(file, crate::notebook::Mode::Run) {
                Ok(s) => s,
                Err(_) => continue,
            };
            parse_program(&parser, file, source)
        };
        let program = match program {
            Ok(p) => p,
            // A file that fails to parse is a real error, not "no tests here".
            // Silently skipping it lets a syntax error in a test file pass CI.
            Err(e) => {
                parse_errors += 1;
                if json {
                    let fail = super::test_events::Failure {
                        message: e,
                        file: None,
                        line: None,
                        col: None,
                        frames: Vec::new(),
                        stmt: None,
                    };
                    super::test_events::file_done(&abs_name(file), 0, 0, Some(fail), "parse");
                } else {
                    println!("  {} {}", "✗".red().bold(), file.display());
                    println!("      {}", e.red());
                }
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
        // Under `--only` a file is named when one of its blocks ran.
        let mut named = options.only.is_none() || json;
        if named && !json {
            println!("{}", file.display().to_string().bright_blue());
        }

        let mut interpreter = Interpreter::new();
        interpreter.enable_test_mode();
        interpreter.narrow_tests(options.only.clone(), own_blocks);
        interpreter.set_test_entries(entries.clone());
        // Programs can tell: the web SDK's `dispatch` runs handlers on a
        // task thread under `olang test`, so a captured cell fails in the
        // test that exercises it instead of in production.
        unsafe { std::env::set_var("OLANG_TEST", "1") };
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
        if json && options.record_args {
            interpreter.enable_arg_recording();
        }

        // Resolve the file's package dependencies, as `olang <file>` would.
        let package_root = crate::pkg::manifest::Manifest::find_root(&absolute);
        let deps_map = package_root.as_deref().and_then(|root| {
            crate::pkg::install(root, &crate::pkg::InstallOptions::default()).ok()
        });
        if !options.deps {
            let (root, dirs) = project_scope(package_root.as_deref(), deps_map.as_ref());
            interpreter.set_test_scope(root, dirs);
        }
        let deny = std::env::var("OLANG_DENY")
            .ok()
            .and_then(|list| crate::caps::parse_deny(&list).ok());
        // A notebook's front matter grant, taken with OLANG_DENY's: both
        // only remove, and the manifest's applies below.
        let notebook_caps = crate::notebook::is_notebook(file)
            .then(|| crate::notebook::read(file, crate::notebook::Mode::Run).ok())
            .flatten()
            .and_then(|p| p.capabilities);
        let deny = match (deny, notebook_caps) {
            (Some(d), Some(c)) => Some(d.intersect(c)),
            (d, c) => d.or(c),
        };
        let mut caps_installed = false;
        if let Some(map) = deps_map {
            let mut map: std::collections::HashMap<_, _> = map.into_iter().collect();
            // A package is known by its own name from within itself, as
            // under `olang <file>` (`use own_name`, `asset.read(own_name, …)`).
            if let Some(root) = package_root.as_deref()
                && let Ok(m) = crate::pkg::manifest::Manifest::load(root)
            {
                map.entry(m.package.name.clone())
                    .or_insert_with(|| root.to_path_buf());
            }
            // The manifest's grant, attenuated per dependency, as
            // `olang <file>` applies it: a test runs under the capabilities
            // the program runs under (and OLANG_DENY on top).
            let config = package_root.as_deref().and_then(|root| {
                crate::pkg::manifest::Manifest::load_for_capabilities(root)
                    .ok()
                    .flatten()
                    .and_then(|m| m.capabilities)
            });
            if let Some(config) = config
                && let Ok(mut table) = crate::caps::CapTable::build(&config, &map)
            {
                if let Some(d) = deny {
                    table.app = table.app.intersect(d);
                    for entry in &mut table.deps {
                        entry.2 = entry.2.intersect(d);
                    }
                }
                interpreter.set_capabilities(table);
                caps_installed = true;
            }
            interpreter.set_dependency_map(map);
        }
        if !caps_installed && let Some(d) = deny {
            interpreter.set_capabilities(crate::caps::CapTable {
                app: d,
                deps: Vec::new(),
                fixed: false,
            });
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
        if json && options.record_args {
            for call in interpreter.take_test_calls() {
                super::test_events::args(&call);
            }
        }

        // Fold this file's recorded lines into the run-wide tally. Keyed by
        // the file that owns each line, so a helper defined in one file and
        // exercised by a test in another lands under the file it lives in.
        if let Some(hits) = interpreter.take_coverage() {
            for (file, lines) in hits {
                coverage_hits.entry(file).or_default().extend(lines);
            }
        }

        let (mut file_passed, mut file_failed) = (0usize, 0usize);
        for outcome in &results {
            if outcome.error.is_none() {
                file_passed += 1
            } else {
                file_failed += 1
            }
            if json {
                total_passed += usize::from(outcome.error.is_none());
                total_failed += usize::from(outcome.error.is_some());
                continue;
            }
            if !named {
                named = true;
                println!("{}", file.display().to_string().bright_blue());
            }
            let took = if options.times {
                format!("  {}", format!("{:.0} ms", outcome.ms).dimmed())
            } else {
                String::new()
            };
            if options.times {
                slowest.push((outcome.ms, outcome.name.clone(), file.display().to_string()));
            }
            match &outcome.error {
                None => {
                    total_passed += 1;
                    println!("  {} {}{}", "✓".green(), outcome.name, took);
                }
                Some(msg) => {
                    total_failed += 1;
                    println!("  {} {}{}", "✗".red().bold(), outcome.name.bold(), took);
                    println!("      {}", msg.red());
                }
            }
        }

        // An error outside any test block (setup code failed) is its own
        // failure — the file's tests can't be trusted to have all run.
        if json {
            let fail = run_error.as_ref().map(|e| {
                let loc = interpreter.take_error_location();
                super::test_events::Failure {
                    message: e.to_string(),
                    file: loc.as_ref().and_then(|l| l.file.clone()),
                    line: loc.as_ref().map(|l| l.line),
                    col: loc.as_ref().map(|l| l.column),
                    frames: loc.map(|l| l.call_stack).unwrap_or_default(),
                    stmt: None,
                }
            });
            file_errors += usize::from(fail.is_some());
            super::test_events::file_done(
                &absolute.display().to_string(),
                file_passed,
                file_failed,
                fail,
                "error",
            );
        } else if let Some(e) = run_error {
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
    let none_ran = options.fail_if_none && total_passed + total_failed == 0;
    if none_ran && !json {
        eprintln!("olang test --fail-if-none: no test block ran");
    }
    if json {
        let failed = total_failed + file_errors + parse_errors > 0 || none_ran;
        let code = if failed { 1 } else { 0 };
        super::test_events::finished(
            total_passed,
            total_failed + file_errors + parse_errors,
            test_files,
            elapsed,
            code,
        );
        return code;
    }
    println!("{}", "─".repeat(40));
    if test_files == 0 {
        println!(
            "  no test blocks found under {} ({} file{} scanned)",
            path.display(),
            files.len(),
            if files.len() == 1 { "" } else { "s" }
        );
        // Unparseable files still make the run fail, even with no test blocks.
        return if parse_errors > 0 || none_ran { 1 } else { 0 };
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
    if let Some(only) = &options.only
        && total_passed + total_failed == 0
    {
        println!("  no test block's name contains {:?}", only);
    }
    if options.times && slowest.len() > 1 {
        slowest.sort_by(|a, b| b.0.total_cmp(&a.0));
        println!("  slowest:");
        for (ms, name, file) in slowest.iter().take(5) {
            println!("    {:>7.0} ms  {}  {}", ms, name, file.dimmed());
        }
    }

    // Coverage is a report, not a gate: it never changes the exit code, so
    // a green suite with thin coverage still passes (and can be tightened
    // later). `--coverage-lines` additionally lists the uncovered lines.
    if coverage {
        super::coverage::print_report(&coverage_hits, show_missing);
    }

    if failed || none_ran { 1 } else { 0 }
}
