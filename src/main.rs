use clap::Parser;
use colored::*;
use std::path::PathBuf;
use std::process;

use olang::{
    log::{Logger, init_logger},
    parallel::{initialize_parallelization, set_parallel_threshold},
    parser::{ErrorSuggestion, Parser as OlangParser, SuggestionSeverity},
    repl::Repl,
};

#[derive(Parser)]
#[command(name = "olang")]
#[command(about = "A minimal, expressive language with first-class functions and pipelines")]
#[command(version)]
struct Cli {
    /// Input file to execute (optional, starts REPL if not provided)
    #[arg(value_name = "FILE")]
    file: Option<PathBuf>,

    /// Execute in batch mode (no REPL)
    #[arg(short, long)]
    batch: bool,

    /// Enable verbose output
    #[arg(short, long)]
    verbose: bool,

    /// Enable tracing for debugging
    #[arg(long)]
    trace: bool,

    /// Disable OVM and use classic interpreter only
    #[arg(long)]
    no_ovm: bool,

    /// Show OVM performance statistics
    #[arg(long)]
    ovm_stats: bool,

    /// Rerun the file whenever any .ol file in its directory changes
    /// (place before the file: `olang --watch script.ol`)
    #[arg(long)]
    watch: bool,

    /// Enable parallel evaluation of independent expressions
    #[arg(long)]
    enable_parallel: bool,

    /// Set maximum parallelism for OVM/builtins (threads). Also respects OVM_PARALLELISM env var.
    #[arg(long, value_name = "N")]
    ovm_parallelism: Option<usize>,

    /// Compile hot functions to OVM bytecode after N calls (default 50 when
    /// the flag is given without a value). Functions the tier cannot compile
    /// keep running on the interpreter.
    #[arg(long, value_name = "N", num_args = 0..=1, require_equals = true, default_missing_value = "50")]
    ovm_tier: Option<u32>,

    /// Deny capabilities for this run, on top of any manifest: a comma
    /// list of fs, fs-write, net, proc, db, env (e.g. --deny net,fs-write).
    /// Also honored from the OLANG_DENY environment variable.
    #[arg(long, value_name = "CAPS")]
    deny: Option<String>,

    /// Arguments passed through to the program, readable via `os.args()`.
    /// Everything after the file name (or after `--`) is the script's argv.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    script_args: Vec<String>,
}

/// The interpreter recurses on the host stack, and its documented call-depth
/// limit (1000) needs more than the default 8 MB main-thread stack — without
/// this, a program recursing ~700 deep aborted the process instead of
/// reporting "Maximum call depth exceeded".
const INTERPRETER_STACK_SIZE: usize = 256 * 1024 * 1024;

fn main() {
    let exit_code = std::thread::Builder::new()
        .stack_size(INTERPRETER_STACK_SIZE)
        .spawn(run)
        .expect("failed to spawn interpreter thread")
        .join()
        .unwrap_or(1);
    process::exit(exit_code);
}

fn run() -> i32 {
    // A binary produced by `olang build` carries its program appended
    // after the runtime. Run that and nothing else — checked before any
    // CLI parsing, so the bundled tool's own arguments reach it intact.
    if let Some(bundle) = embedded_program() {
        let logger = init_logger();
        let _ = initialize_parallelization(None);
        set_parallel_threshold(10_000);
        miette::set_panic_hook();
        return run_embedded(bundle, logger);
    }

    let mut cli = Cli::parse();

    // `olang run x.ol` — muscle memory from cargo/go/deno. There is no
    // `run` subcommand (the file is the first positional), so treat `run`
    // as an alias: shift to the next argument — unless a real file named
    // `run` exists, which stays runnable like any other file.
    if cli.file.as_deref() == Some(std::path::Path::new("run"))
        && !std::path::Path::new("run").exists()
    {
        if cli.script_args.is_empty() {
            eprintln!("olang: no file named 'run' and no file given — usage: olang <file.ol>");
            return 1;
        }
        cli.file = Some(PathBuf::from(cli.script_args.remove(0)));
    }

    // Initialize logger
    let logger = init_logger();

    // Initialize parallelization early for optimal performance
    match initialize_parallelization(cli.ovm_parallelism) {
        Err(e) => {
            if cli.verbose {
                logger.warn(
                    "main",
                    &format!("Failed to initialize parallel processing: {}", e),
                );
            }
        }
        _ => {
            if cli.verbose {
                logger.info(
            "main",
            &format!(
                "Multi-threading enabled: {} CPU cores detected, using aggressive parallelization",
                num_cpus::get()
            ),
        );
            }
        }
    }

    // Allow enabling/disabling parallel at runtime
    if cli.enable_parallel {
        // FIXME: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::set_var("OVM_ENABLE_PARALLEL", "1") };
    }
    if let Some(n) = cli.ovm_parallelism {
        // FIXME: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::set_var("OVM_PARALLELISM", n.to_string()) };
    }

    // Parallelize only where the work plausibly outweighs thread overhead.
    // Small lists are always cheaper sequentially.
    set_parallel_threshold(10_000);

    if cli.verbose {
        logger.info(
            "main",
            &format!(
                "Parallelization enabled for lists of 10,000+ items across {} cores",
                num_cpus::get()
            ),
        );
    }

    // Initialize tracing if requested
    if cli.trace {
        tracing_subscriber::fmt()
            .with_env_filter("olang=trace")
            .init();
    }

    // Initialize error reporting
    miette::set_panic_hook();

    // Tool commands: `olang test [path]`, `olang fmt [paths] [--check]`,
    // and `olang check [paths]`. The first positional dispatches; a file
    // literally named `test` or `fmt` is still runnable as `./test` or
    // `test.ol`.
    if let Some(ref file_path) = cli.file {
        match file_path.to_string_lossy().as_ref() {
            "test" => {
                // Files under the runner get a bare argv — a program that
                // branches on os.args() takes its no-argument path.
                olang::stdlib::os::set_script_args(vec!["olang-test".to_string()]);
                // `--coverage` reports line coverage; `--coverage-lines`
                // additionally lists each file's uncovered lines (and implies
                // `--coverage`). The path is the first non-flag argument.
                let show_missing = cli.script_args.iter().any(|a| a == "--coverage-lines");
                let coverage = show_missing || cli.script_args.iter().any(|a| a == "--coverage");
                let target = cli
                    .script_args
                    .iter()
                    .find(|a| !a.starts_with("--"))
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("."));
                return olang::tools::test_runner::run(&target, coverage, show_missing);
            }
            "lsp" => {
                return match olang::tools::lsp::run() {
                    Ok(()) => 0,
                    Err(e) => {
                        eprintln!("language server error: {e}");
                        1
                    }
                };
            }
            "bench" => {
                return olang::tools::bench::run(&cli.script_args);
            }
            "check" => {
                let mut paths: Vec<PathBuf> = cli.script_args.iter().map(PathBuf::from).collect();
                if paths.is_empty() {
                    paths.push(PathBuf::from("."));
                }
                return olang::tools::check::run(&paths);
            }
            "fmt" => {
                let check = cli.script_args.iter().any(|a| a == "--check");
                let mut paths: Vec<PathBuf> = cli
                    .script_args
                    .iter()
                    .filter(|a| *a != "--check")
                    .map(PathBuf::from)
                    .collect();
                if paths.is_empty() {
                    paths.push(PathBuf::from("."));
                }
                return olang::tools::fmt::run(&paths, check);
            }
            "build" => {
                return match build_executable(&cli.script_args) {
                    Ok(out) => {
                        println!("built {}", out);
                        0
                    }
                    Err(e) => {
                        eprintln!("olang build: {}", e);
                        1
                    }
                };
            }
            "inspect" => {
                return inspect_binary(&cli.script_args);
            }
            "doc" => {
                // olang doc [paths] [-o out.html] [--md]
                let mut output = PathBuf::from("doc.html");
                let mut markdown = false;
                let mut paths: Vec<PathBuf> = Vec::new();
                let mut it = cli.script_args.iter();
                while let Some(a) = it.next() {
                    match a.as_str() {
                        "--md" | "--markdown" => markdown = true,
                        "-o" | "--output" => {
                            if let Some(o) = it.next() {
                                output = PathBuf::from(o);
                            }
                        }
                        other => paths.push(PathBuf::from(other)),
                    }
                }
                if paths.is_empty() {
                    paths.push(PathBuf::from("."));
                }
                return olang::tools::doc::run(&paths, &output, markdown);
            }
            _ => {}
        }
    }

    if let Some(file_path) = cli.file {
        // Watch mode: run the script in a child process (so os.exit and
        // crashes end the run, not the watcher) and rerun when any .ol
        // file in the script's directory changes.
        if cli.watch {
            return watch_loop(&file_path, cli.deny.as_deref(), &cli.script_args);
        }
        // Program's argv: the script path, then everything after it. Read via
        // os.args() inside the program.
        let mut argv = vec![file_path.to_string_lossy().to_string()];
        argv.extend(cli.script_args.clone());
        olang::stdlib::os::set_script_args(argv);

        // Capability restriction for this run: --deny and OLANG_DENY
        // intersect (both can only remove). A typo in either refuses to
        // run rather than running wide open.
        let deny = {
            let flag = match cli.deny.as_deref().map(olang::caps::parse_deny) {
                Some(Ok(c)) => Some(c),
                Some(Err(e)) => {
                    eprintln!("olang: {}", e);
                    return 2;
                }
                None => None,
            };
            let env = match std::env::var("OLANG_DENY")
                .ok()
                .as_deref()
                .map(olang::caps::parse_deny)
            {
                Some(Ok(c)) => Some(c),
                Some(Err(e)) => {
                    eprintln!("olang (OLANG_DENY): {}", e);
                    return 2;
                }
                None => None,
            };
            match (flag, env) {
                (Some(a), Some(b)) => Some(a.intersect(b)),
                (a, b) => a.or(b),
            }
        };

        // Execute file in batch mode
        if let Err(e) = execute_file(
            &file_path,
            cli.verbose,
            cli.no_ovm,
            cli.ovm_stats,
            cli.ovm_tier,
            deny,
            logger,
        ) {
            logger.error("main", &format!("Error executing file: {}", e));
            return 1;
        }
    } else {
        // Start REPL
        if let Err(e) = start_repl(cli.verbose, cli.no_ovm, logger) {
            logger.error("main", &format!("REPL error: {}", e));
            return 1;
        }
    }

    0
}

/// `olang --watch script.ol` -- the edit-run loop. Each run is a child
/// process of this same binary (without --watch): a crash, a panic, or
/// os.exit ends the run, never the watcher. Changes are detected by
/// polling the newest mtime of every .ol file at or below the script's
/// directory -- no dependency, and a save mid-run queues an immediate
/// rerun. Ctrl+C stops watcher and child together (same process group).
fn watch_loop(script: &std::path::Path, deny: Option<&str>, script_args: &[String]) -> i32 {
    use std::time::{Duration, SystemTime};
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("--watch: cannot find own executable: {}", e);
            return 1;
        }
    };
    let dir = script
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."))
        .to_path_buf();
    let latest_mtime = || -> SystemTime {
        let mut newest = SystemTime::UNIX_EPOCH;
        for f in olang::tools::discover_ol_files(&dir) {
            if let Ok(meta) = std::fs::metadata(&f)
                && let Ok(m) = meta.modified()
                && m > newest
            {
                newest = m;
            }
        }
        newest
    };
    loop {
        let baseline = latest_mtime();
        eprintln!("-- watch: running {} --", script.display());
        let mut cmd = std::process::Command::new(&exe);
        if let Some(d) = deny {
            cmd.arg("--deny").arg(d);
        }
        let status = cmd.arg(script).args(script_args).status();
        match status {
            Ok(s) => eprintln!(
                "-- watch: exited {} -- waiting for changes (Ctrl+C quits) --",
                s.code()
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "by signal".to_string())
            ),
            Err(e) => {
                eprintln!("--watch: failed to run {}: {}", script.display(), e);
                return 1;
            }
        }
        while latest_mtime() <= baseline {
            std::thread::sleep(Duration::from_millis(200));
        }
    }
}

/// Enhanced error display for file execution
fn show_file_parse_error(
    error: &olang::parser::ParseError,
    file_path: &std::path::Path,
    source: &str,
) {
    eprintln!("\n{}", "═══ Parse Error ═══".bright_red().bold());
    eprintln!(
        "  {}: {}",
        "File".bright_blue().bold(),
        file_path.display().to_string().bright_white()
    );

    match error {
        olang::parser::ParseError::InvalidSyntaxWithPosition {
            message,
            line,
            column,
            snippet,
        } => {
            eprintln!(
                "  {}: {}",
                "Error".bright_red().bold(),
                message.bright_white()
            );
            eprintln!(
                "  {}: Line {}, Column {}",
                "Location".bright_yellow().bold(),
                line.to_string().bright_cyan(),
                column.to_string().bright_cyan()
            );

            if !snippet.trim().is_empty() {
                eprintln!("\n  {}", "Code Context:".bright_blue().bold());
                show_highlighted_snippet(snippet);
            }
        }
        olang::parser::ParseError::UnexpectedTokenWithPosition {
            token,
            line,
            column,
            snippet,
        } => {
            eprintln!(
                "  {}: Unexpected token '{}'",
                "Error".bright_red().bold(),
                token.bright_yellow().bold()
            );
            eprintln!(
                "  {}: Line {}, Column {}",
                "Location".bright_yellow().bold(),
                line.to_string().bright_cyan(),
                column.to_string().bright_cyan()
            );

            if !snippet.trim().is_empty() {
                eprintln!("\n  {}", "Code Context:".bright_blue().bold());
                show_highlighted_snippet(snippet);
            }
        }
        _ => {
            eprintln!("  {}: {}", "Error".bright_red().bold(), error);
        }
    }

    // Get suggestions
    let parser = OlangParser::new();
    let suggestions = parser.get_suggestions(error, source);

    if !suggestions.is_empty() {
        eprintln!("\n  {}", "Suggestions:".bright_cyan().bold());
        for suggestion in suggestions {
            show_suggestion(&suggestion);
        }
    }

    // Show help topics
    eprintln!("\n  {}", "Help:".bright_cyan().bold());
    eprintln!("    • Type {} for syntax help", "olang -h".bright_cyan());

    eprintln!();
}

/// Feature 9: Show classic interpreter errors with enhanced context and suggestions
fn show_classic_interpreter_error(
    error: &olang::interpreter::InterpreterError,
    file_path: &std::path::Path,
    interpreter: &olang::interpreter::Interpreter,
    location: Option<olang::ast::ErrorLocation>,
    source: &str,
) {
    eprintln!("\n{}", "═══ Execution Error ═══".bright_red().bold());

    let formatted_error = interpreter.format_error(error);

    if let Some(loc) = location {
        // Located: render the source line with a caret through miette so
        // the error points at where it happened, then the call stack.
        let offset = byte_offset_of(source, loc.line as usize, loc.column as usize);
        let mut diagnostic = miette::MietteDiagnostic::new(formatted_error.clone()).with_label(
            miette::LabeledSpan::at_offset(offset, "error occurred here"),
        );
        if let Some(hint) = &loc.hint {
            diagnostic = diagnostic.with_help(hint.clone());
        }
        let report = miette::Report::new(diagnostic).with_source_code(miette::NamedSource::new(
            file_path.display().to_string(),
            source.to_string(),
        ));
        eprintln!("{:?}", report);
        if !loc.call_stack.is_empty() {
            eprintln!("  {}", "Call stack (outermost first):".bright_blue().bold());
            for name in &loc.call_stack {
                eprintln!("    → {}", name.bright_white());
            }
            eprintln!();
        }
    } else {
        eprintln!(
            "  {}: {}",
            "File".bright_blue().bold(),
            file_path.display().to_string().bright_white()
        );
        eprintln!("\n{}", formatted_error);
        eprintln!();
    }
}

/// Byte offset of a 1-based (line, column) position in `source` — what
/// miette's span labels want. Clamped to the source length.
fn byte_offset_of(source: &str, line: usize, column: usize) -> usize {
    let mut offset = 0usize;
    for (i, l) in source.split('\n').enumerate() {
        if i + 1 == line {
            // Column is 1-based and counted in characters; walk the line.
            let col_bytes: usize = l
                .chars()
                .take(column.saturating_sub(1))
                .map(|c| c.len_utf8())
                .sum();
            return (offset + col_bytes).min(source.len());
        }
        offset += l.len() + 1;
    }
    offset.min(source.len())
}

/// Feature 9: Show interpreter errors with enhanced context and suggestions
fn show_highlighted_snippet(snippet: &str) {
    let lines: Vec<&str> = snippet.lines().collect();

    for line in lines {
        if line.trim().is_empty() {
            continue;
        }

        // Check if this line contains a caret pointer
        if line.contains("^") {
            // This is the error pointer line - highlight it specially
            let parts: Vec<&str> = line.split('|').collect();
            if parts.len() >= 2 {
                let line_num = parts[0].trim();
                let pointer = parts[1];
                println!(
                    "    {}{}│ {}",
                    line_num.bright_black(),
                    " ".repeat(4 - line_num.len().min(4)),
                    pointer.bright_red().bold()
                );
            }
        } else {
            // Regular code line - apply basic syntax highlighting
            let parts: Vec<&str> = line.split('|').collect();
            if parts.len() >= 2 {
                let line_num = parts[0].trim();
                let code = parts[1];

                // Basic syntax highlighting
                let highlighted_code = apply_basic_highlighting(code);

                println!(
                    "    {}{}│ {}",
                    line_num.bright_blue(),
                    " ".repeat(4 - line_num.len().min(4)),
                    highlighted_code
                );
            }
        }
    }
}

fn apply_basic_highlighting(code: &str) -> String {
    let mut result = String::new();
    let mut chars = code.chars().peekable();
    let mut current_word = String::new();

    while let Some(ch) = chars.next() {
        match ch {
            // String literals
            '"' => {
                if !current_word.is_empty() {
                    result.push_str(&highlight_word(&current_word));
                    current_word.clear();
                }
                result.push_str(&format!("{}", "\"".bright_green()));

                // Consume the string content
                while let Some(str_ch) = chars.next() {
                    if str_ch == '"' {
                        result.push_str(&format!("{}", str_ch.to_string().bright_green()));
                        break;
                    } else if str_ch == '\\' {
                        result.push_str(&format!("{}", str_ch.to_string().bright_green()));
                        if let Some(escaped) = chars.next() {
                            result.push_str(&format!("{}", escaped.to_string().bright_green()));
                        }
                    } else {
                        result.push_str(&format!("{}", str_ch.to_string().bright_green()));
                    }
                }
            }
            // Numbers
            c if c.is_ascii_digit() => {
                current_word.push(c);

                // Look ahead to consume the full number
                while let Some(&next_ch) = chars.peek() {
                    if next_ch.is_ascii_digit() || next_ch == '.' || next_ch == '_' {
                        current_word.push(chars.next().unwrap());
                    } else {
                        break;
                    }
                }

                result.push_str(&format!("{}", current_word.bright_magenta()));
                current_word.clear();
            }
            // Identifiers and keywords
            c if c.is_alphabetic() || c == '_' => {
                current_word.push(c);
            }
            // Operators and punctuation
            '=' | '+' | '-' | '*' | '/' | '%' | '<' | '>' | '!' | '&' | '|' => {
                if !current_word.is_empty() {
                    result.push_str(&highlight_word(&current_word));
                    current_word.clear();
                }

                // Look ahead for compound operators
                let mut op = ch.to_string();
                if let Some(&next_ch) = chars.peek()
                    && ((next_ch == '=' && matches!(ch, '=' | '!' | '<' | '>'))
                        || (ch == '&' && next_ch == '&')
                        || (ch == '|' && next_ch == '|'))
                {
                    op.push(chars.next().unwrap());
                }
                result.push_str(&format!("{}", op.bright_yellow()));
            }
            // Brackets and parentheses
            '(' | ')' | '[' | ']' | '{' | '}' => {
                if !current_word.is_empty() {
                    result.push_str(&highlight_word(&current_word));
                    current_word.clear();
                }
                result.push_str(&format!("{}", ch.to_string().bright_cyan()));
            }
            // Other characters
            _ => {
                if !current_word.is_empty() {
                    result.push_str(&highlight_word(&current_word));
                    current_word.clear();
                }
                result.push(ch);
            }
        }
    }

    // Handle any remaining word
    if !current_word.is_empty() {
        result.push_str(&highlight_word(&current_word));
    }

    result
}

fn highlight_word(word: &str) -> String {
    match word {
        // Keywords
        "let" | "fn" | "if" | "else" | "match" | "for" | "while" | "loop" | "break"
        | "continue" | "true" | "false" | "import" | "export" | "type" | "async" | "await"
        | "try" | "catch" => {
            format!("{}", word.bright_blue().bold())
        }
        // Built-in functions
        "println" | "print" | "to_string" | "to_int" | "to_float" => {
            format!("{}", word.bright_green())
        }
        // Types
        "String" | "Int" | "Float" | "Bool" | "List" | "Map" => {
            format!("{}", word.bright_yellow())
        }
        // Default
        _ => word.to_string(),
    }
}

fn show_suggestion(suggestion: &ErrorSuggestion) {
    let severity_icon = match suggestion.severity {
        SuggestionSeverity::Error => "ERROR",
        SuggestionSeverity::Warning => "WARNING",
        SuggestionSeverity::Hint => "HINT",
        SuggestionSeverity::Info => "INFO",
    };

    let severity_color = match suggestion.severity {
        SuggestionSeverity::Error => "red",
        SuggestionSeverity::Warning => "yellow",
        SuggestionSeverity::Hint => "cyan",
        SuggestionSeverity::Info => "blue",
    };

    println!(
        "    {} {}",
        severity_icon,
        suggestion.message.color(severity_color).bold()
    );

    if let Some(fix) = &suggestion.fix {
        println!("      {}: {}", "Fix".bright_green().bold(), fix);
    }

    if let Some(help) = &suggestion.help {
        println!("      {}: {}", "Help".bright_blue().bold(), help);
    }
}

/// Trailer marking an `olang build` executable. The program is appended
/// to the runtime as `[source bytes][source len: u64 LE][MAGIC: 8]`, so
/// a build never touches a linker — a byte copy and an append suffice.
/// Rung A bundle: raw olang source appended, parsed at startup.
const OLANG_BUNDLE_MAGIC: &[u8; 8] = b"oLaNgBnd";
/// Rung B bundle: the *parsed* AST appended alongside the source, so a
/// built tool deserializes its program at startup instead of re-parsing it
/// (deserialize is ~20× faster than the parser for a real program). The
/// source travels too, only so runtime error snippets still render.
const OLANG_AST_MAGIC: &[u8; 8] = b"oLaNgAsT";
/// Rung "transparent binary": [source][ast][meta][3 x u64 lens][magic].
const OLANG_META_MAGIC: &[u8; 8] = b"oLaNgMeT";

/// The transparency record a built binary carries alongside its program:
/// enough for `olang inspect` to reproduce, verify, and reason about the
/// artifact without any external context.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
struct BundleMeta {
    /// Meta format version, for forward compatibility.
    format: u32,
    /// The olang that built this binary.
    olang_version: String,
    /// The path the source was built from (basename context only).
    source_path: String,
    /// sha256 (hex) of the embedded source bytes.
    sha256: String,
    /// The package's olang.toml, verbatim, when the source lived in one.
    #[serde(default)]
    manifest: Option<String>,
    /// The package's olang.lock, verbatim, when present.
    #[serde(default)]
    lockfile: Option<String>,
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    format!("{:x}", h.finalize())
}

/// A program bundled into an `olang build` executable.
enum Bundle {
    /// Rung A: just the source (older bundles, or a fallback).
    Source(String),
    /// Rung B: the pre-parsed AST, with the source kept for error rendering.
    Ast {
        meta: Option<Box<BundleMeta>>,
        program: Box<olang::ast::Program>,
        source: String,
    },
}

/// If this executable was produced by `olang build`, return the program
/// appended to it — the pre-parsed AST (rung B) when present, else the raw
/// source (rung A). `None` for the plain `olang` binary.
fn embedded_program() -> Option<Bundle> {
    let exe = std::env::current_exe().ok()?;
    read_bundle(&exe)
}

/// Read a bundled program from any file — the shared reader behind both
/// self-execution and `olang inspect <binary>`.
fn read_bundle(path: &std::path::Path) -> Option<Bundle> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).ok()?;
    let total = f.metadata().ok()?.len();
    if total < 16 {
        return None;
    }
    // The last 8 bytes are the format marker.
    f.seek(SeekFrom::End(-8)).ok()?;
    let mut magic = [0u8; 8];
    f.read_exact(&mut magic).ok()?;

    // Transparent binary: [source][ast][meta][src_len][ast_len][meta_len][MAGIC].
    if &magic == OLANG_META_MAGIC {
        if total < 32 {
            return None;
        }
        f.seek(SeekFrom::End(-32)).ok()?;
        let mut lens = [0u8; 24];
        f.read_exact(&mut lens).ok()?;
        let source_len = u64::from_le_bytes(lens[0..8].try_into().ok()?);
        let ast_len = u64::from_le_bytes(lens[8..16].try_into().ok()?);
        let meta_len = u64::from_le_bytes(lens[16..24].try_into().ok()?);
        let payload = source_len + ast_len + meta_len;
        if source_len == 0 || ast_len == 0 || meta_len == 0 || payload + 32 > total {
            return None;
        }
        f.seek(SeekFrom::End(-(32 + payload as i64))).ok()?;
        let mut source_buf = vec![0u8; source_len as usize];
        f.read_exact(&mut source_buf).ok()?;
        let mut ast_buf = vec![0u8; ast_len as usize];
        f.read_exact(&mut ast_buf).ok()?;
        let mut meta_buf = vec![0u8; meta_len as usize];
        f.read_exact(&mut meta_buf).ok()?;
        let source = String::from_utf8(source_buf).ok()?;
        let program: olang::ast::Program = serde_json::from_slice(&ast_buf).ok()?;
        let meta: BundleMeta = serde_json::from_slice(&meta_buf).ok()?;
        return Some(Bundle::Ast {
            meta: Some(Box::new(meta)),
            program: Box::new(program),
            source,
        });
    }

    // Rung B: [source][ast_json][source_len u64][ast_len u64][AST_MAGIC].
    if &magic == OLANG_AST_MAGIC {
        if total < 24 {
            return None;
        }
        f.seek(SeekFrom::End(-24)).ok()?;
        let mut lens = [0u8; 16];
        f.read_exact(&mut lens).ok()?;
        let source_len = u64::from_le_bytes(lens[0..8].try_into().ok()?);
        let ast_len = u64::from_le_bytes(lens[8..16].try_into().ok()?);
        if source_len == 0 || ast_len == 0 || source_len + ast_len + 24 > total {
            return None;
        }
        f.seek(SeekFrom::End(-(24 + ast_len as i64 + source_len as i64)))
            .ok()?;
        let mut source_buf = vec![0u8; source_len as usize];
        f.read_exact(&mut source_buf).ok()?;
        let mut ast_buf = vec![0u8; ast_len as usize];
        f.read_exact(&mut ast_buf).ok()?;
        let source = String::from_utf8(source_buf).ok()?;
        let program: olang::ast::Program = serde_json::from_slice(&ast_buf).ok()?;
        return Some(Bundle::Ast {
            meta: None,
            program: Box::new(program),
            source,
        });
    }

    // Rung A: [source][source_len u64][BUNDLE_MAGIC].
    if &magic == OLANG_BUNDLE_MAGIC {
        f.seek(SeekFrom::End(-16)).ok()?;
        let mut footer = [0u8; 16];
        f.read_exact(&mut footer).ok()?;
        let src_len = u64::from_le_bytes(footer[0..8].try_into().ok()?);
        if src_len == 0 || src_len + 16 > total {
            return None;
        }
        f.seek(SeekFrom::End(-(16 + src_len as i64))).ok()?;
        let mut buf = vec![0u8; src_len as usize];
        f.read_exact(&mut buf).ok()?;
        return Some(Bundle::Source(String::from_utf8(buf).ok()?));
    }

    None
}

/// Run a bundled program. Its argv is the whole process argv, so the
/// tool's own flags reach `os.args()` / `cli.args()` exactly as they
/// would for a normal script. A rung-B bundle runs its pre-parsed AST
/// directly; a rung-A bundle parses its source first.
fn run_embedded(bundle: Bundle, logger: &Logger) -> i32 {
    olang::stdlib::os::set_script_args(std::env::args().collect());
    let path = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("program"));
    // A built binary honors the capabilities its embedded manifest
    // declares (base grant; per-dependency attenuation is a project-run
    // concern — a bundle is a single source). OLANG_DENY restricts
    // further from the outside; a bundle's own argv belongs to the tool.
    let deny = std::env::var("OLANG_DENY")
        .ok()
        .and_then(|list| olang::caps::parse_deny(&list).ok());
    let result = match bundle {
        Bundle::Ast {
            meta,
            program,
            source,
        } => {
            let manifest_caps = meta
                .as_ref()
                .and_then(|m| m.manifest.as_deref())
                .and_then(|text| olang::pkg::manifest::Manifest::from_toml(text).ok())
                .and_then(|m| m.capabilities)
                .and_then(|c| c.base.resolve().ok());
            let effective = match (manifest_caps, deny) {
                (Some(a), Some(b)) => Some(a.intersect(b)),
                (Some(a), None) => Some(a),
                (None, d) => d,
            };
            execute_program(
                *program, &source, &path, false, false, false, None, effective, logger,
            )
        }
        Bundle::Source(source) => {
            execute_source(&source, &path, false, false, false, None, deny, logger)
        }
    };
    match result {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

/// `olang inspect <binary>` — read the transparency record out of a built
/// executable. An olang binary is open by construction: it carries its
/// exact source, its manifest and lockfile, and a checksum — this command
/// is the reader. Flags:
///   --source            print the embedded source
///   --manifest          print the embedded olang.toml
///   --lockfile          print the embedded olang.lock
///   --caps              print the resolved capability grant
///   --verify            recompute the source checksum; nonzero on mismatch
///   -o DIR              extract source/manifest/lockfile into DIR
fn inspect_binary(args: &[String]) -> i32 {
    let mut target: Option<String> = None;
    let mut show_source = false;
    let mut show_manifest = false;
    let mut show_lockfile = false;
    let mut show_caps = false;
    let mut verify = false;
    let mut out_dir: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--source" => show_source = true,
            "--manifest" => show_manifest = true,
            "--lockfile" => show_lockfile = true,
            "--caps" => show_caps = true,
            "--verify" => verify = true,
            "-o" | "--output" => {
                i += 1;
                out_dir = args.get(i).cloned();
                if out_dir.is_none() {
                    eprintln!("olang inspect: -o needs a directory");
                    return 2;
                }
            }
            other if other.starts_with('-') => {
                eprintln!("olang inspect: unknown flag {}", other);
                return 2;
            }
            other => {
                if target.is_none() {
                    target = Some(other.to_string());
                }
            }
        }
        i += 1;
    }
    let Some(target) = target else {
        eprintln!(
            "usage: olang inspect <binary> [--source|--manifest|--lockfile|--caps|--verify|-o DIR]"
        );
        return 2;
    };
    let path = std::path::Path::new(&target);
    let Some(bundle) = read_bundle(path) else {
        eprintln!(
            "olang inspect: {} carries no olang bundle (not built with `olang build`?)",
            target
        );
        return 1;
    };
    let (source, meta) = match &bundle {
        Bundle::Ast { meta, source, .. } => (source.clone(), meta.clone()),
        Bundle::Source(source) => (source.clone(), None),
    };

    // Focused outputs print raw and exit, so they compose with pipes.
    if show_source {
        print!("{}", source);
        return 0;
    }
    if show_manifest {
        match meta.as_ref().and_then(|m| m.manifest.as_ref()) {
            Some(text) => {
                print!("{}", text);
                return 0;
            }
            None => {
                eprintln!("olang inspect: no manifest embedded");
                return 1;
            }
        }
    }
    if show_lockfile {
        match meta.as_ref().and_then(|m| m.lockfile.as_ref()) {
            Some(text) => {
                print!("{}", text);
                return 0;
            }
            None => {
                eprintln!("olang inspect: no lockfile embedded");
                return 1;
            }
        }
    }

    let caps_config = meta
        .as_ref()
        .and_then(|m| m.manifest.as_deref())
        .and_then(|text| olang::pkg::manifest::Manifest::from_toml(text).ok())
        .and_then(|m| m.capabilities);
    if show_caps {
        match &caps_config {
            Some(config) => match config.base.resolve() {
                Ok(caps) => {
                    println!("{}", caps.summary());
                    for (name, spec) in &config.dependencies {
                        match spec.resolve() {
                            Ok(dep_caps) => println!(
                                "dependency {}: {}",
                                name,
                                dep_caps.intersect(caps).summary()
                            ),
                            Err(e) => println!("dependency {}: invalid ({})", name, e),
                        }
                    }
                    return 0;
                }
                Err(e) => {
                    eprintln!("olang inspect: {}", e);
                    return 1;
                }
            },
            None => {
                println!(
                    "{} (no manifest: full capability)",
                    olang::caps::Caps::default().summary()
                );
                return 0;
            }
        }
    }

    if let Some(dir) = out_dir {
        let dir = std::path::Path::new(&dir);
        if let Err(e) = std::fs::create_dir_all(dir) {
            eprintln!("olang inspect: {}: {}", dir.display(), e);
            return 1;
        }
        let source_name = meta
            .as_ref()
            .map(|m| {
                std::path::Path::new(&m.source_path)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "program.ol".to_string())
            })
            .unwrap_or_else(|| "program.ol".to_string());
        let mut wrote = vec![source_name.clone()];
        if std::fs::write(dir.join(&source_name), &source).is_err() {
            eprintln!("olang inspect: cannot write {}", source_name);
            return 1;
        }
        if let Some(m) = &meta {
            if let Some(text) = &m.manifest {
                let _ = std::fs::write(dir.join("olang.toml"), text);
                wrote.push("olang.toml".into());
            }
            if let Some(text) = &m.lockfile {
                let _ = std::fs::write(dir.join("olang.lock"), text);
                wrote.push("olang.lock".into());
            }
        }
        println!("extracted {} into {}", wrote.join(", "), dir.display());
        return 0;
    }

    // The default: a human summary. --verify folds in as the verdict.
    println!("olang bundle: {}", target);
    match &meta {
        Some(m) => {
            println!("  built with:   olang {}", m.olang_version);
            println!("  built from:   {}", m.source_path);
            println!("  source:       {} bytes", source.len());
            let actual = sha256_hex(source.as_bytes());
            let ok = actual == m.sha256;
            println!(
                "  sha256:       {}  [{}]",
                m.sha256,
                if ok { "verified" } else { "MISMATCH" }
            );
            if !ok {
                println!("  actual:       {}", actual);
            }
            println!(
                "  manifest:     {}",
                if m.manifest.is_some() {
                    "embedded (--manifest to print)"
                } else {
                    "none"
                }
            );
            println!(
                "  lockfile:     {}",
                if m.lockfile.is_some() {
                    "embedded (--lockfile to print)"
                } else {
                    "none"
                }
            );
            match &caps_config {
                Some(config) => match config.base.resolve() {
                    Ok(caps) => println!("  capabilities: {}", caps.summary()),
                    Err(e) => println!("  capabilities: invalid ({})", e),
                },
                None => println!("  capabilities: full (no [capabilities] manifest)"),
            }
            if verify && !ok {
                return 1;
            }
        }
        None => {
            println!("  format:       pre-transparency bundle (rung A/B)");
            println!("  source:       {} bytes (--source to print)", source.len());
            println!("  sha256:       none recorded — rebuild with this olang to add one");
            if verify {
                eprintln!("olang inspect: nothing to verify in a pre-transparency bundle");
                return 1;
            }
        }
    }
    0
}

/// `olang build <program.ol> [-o output]` — copy this runtime and append
/// the program, producing a self-contained executable with no external
/// olang install required. The source is parse-checked first, so a
/// broken program is never shipped. Bundles a single source file: a
/// program that `use`s local files should be a package built with all
/// its sources inlined, or restrict itself to stdlib and the embedded
/// packages (cli, term, ui, viz, dash, …), which travel in the runtime.
fn build_executable(args: &[String]) -> anyhow::Result<String> {
    let mut source_path: Option<String> = None;
    let mut output: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-o" | "--output" => {
                i += 1;
                output = Some(
                    args.get(i)
                        .cloned()
                        .ok_or_else(|| anyhow::anyhow!("-o needs an output path"))?,
                );
            }
            other if other.starts_with('-') => {
                return Err(anyhow::anyhow!("unknown flag: {}", other));
            }
            other => {
                if source_path.is_none() {
                    source_path = Some(other.to_string());
                }
            }
        }
        i += 1;
    }
    let source_path = source_path
        .ok_or_else(|| anyhow::anyhow!("usage: olang build <program.ol> [-o output]"))?;
    let src = std::fs::read_to_string(&source_path)?;
    // Parse-check up front (a broken program is never shipped) and keep the
    // AST: the built binary embeds it so startup skips the parser (rung B).
    let program = OlangParser::new()
        .parse(&src)
        .map_err(|e| anyhow::anyhow!("{} does not parse:\n{}", source_path, e))?;
    let ast_json =
        serde_json::to_vec(&program).map_err(|e| anyhow::anyhow!("serialize AST: {}", e))?;

    let output = output.unwrap_or_else(|| {
        PathBuf::from(&source_path)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "program".to_string())
    });

    // The transparency record: checksum, provenance, and — when the
    // source lives in a package — its manifest and lockfile, verbatim.
    // Every built binary carries its own source and its own paper trail;
    // `olang inspect` reads them back out.
    let source_abs =
        std::fs::canonicalize(&source_path).unwrap_or_else(|_| PathBuf::from(&source_path));
    let (manifest_text, lockfile_text) =
        match olang::pkg::manifest::Manifest::find_root(&source_abs) {
            Some(root) => (
                std::fs::read_to_string(root.join("olang.toml")).ok(),
                std::fs::read_to_string(root.join("olang.lock")).ok(),
            ),
            None => (None, None),
        };
    // A manifest with capabilities must parse — a build never ships a
    // grant it cannot enforce.
    if let Some(text) = &manifest_text {
        let parsed: olang::pkg::manifest::Manifest =
            olang::pkg::manifest::Manifest::from_toml(text)
                .map_err(|e| anyhow::anyhow!("olang.toml: {}", e))?;
        if let Some(config) = &parsed.capabilities {
            config
                .base
                .resolve()
                .map_err(|e| anyhow::anyhow!("olang.toml: {}", e))?;
        }
    }
    let meta = BundleMeta {
        format: 1,
        olang_version: olang::VERSION.to_string(),
        source_path: source_path.clone(),
        sha256: sha256_hex(src.as_bytes()),
        manifest: manifest_text,
        lockfile: lockfile_text,
    };
    let meta_json =
        serde_json::to_vec(&meta).map_err(|e| anyhow::anyhow!("serialize meta: {}", e))?;

    let exe = std::env::current_exe()?;
    std::fs::copy(&exe, &output)?;
    {
        use std::io::Write;
        // Transparent-binary footer:
        // [source][ast][meta][src_len][ast_len][meta_len][META_MAGIC].
        // The source travels as a feature, not a debugging convenience:
        // an olang binary is open by construction.
        let mut f = std::fs::OpenOptions::new().append(true).open(&output)?;
        f.write_all(src.as_bytes())?;
        f.write_all(&ast_json)?;
        f.write_all(&meta_json)?;
        f.write_all(&(src.len() as u64).to_le_bytes())?;
        f.write_all(&(ast_json.len() as u64).to_le_bytes())?;
        f.write_all(&(meta_json.len() as u64).to_le_bytes())?;
        f.write_all(OLANG_META_MAGIC)?;
        f.flush()?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&output)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&output, perms)?;
    }
    Ok(output)
}

#[allow(clippy::too_many_arguments)]
fn execute_file(
    file_path: &PathBuf,
    verbose: bool,
    no_ovm: bool,
    ovm_stats: bool,
    ovm_tier: Option<u32>,
    deny: Option<olang::caps::Caps>,
    logger: &Logger,
) -> anyhow::Result<()> {
    let source = std::fs::read_to_string(file_path)
        .map_err(|e| anyhow::anyhow!("cannot read '{}': {}", file_path.display(), e))?;
    execute_source(
        &source, file_path, verbose, no_ovm, ovm_stats, ovm_tier, deny, logger,
    )
}

/// Run a program from source already in hand (a file's contents, or the
/// program bundled into an `olang build` executable). `file_path` names
/// it for error messages and cwd-relative module resolution.
#[allow(clippy::too_many_arguments)]
fn execute_source(
    source: &str,
    file_path: &PathBuf,
    verbose: bool,
    no_ovm: bool,
    ovm_stats: bool,
    ovm_tier: Option<u32>,
    deny: Option<olang::caps::Caps>,
    logger: &Logger,
) -> anyhow::Result<()> {
    let program = match OlangParser::new().parse(source) {
        Ok(program) => program,
        Err(e) => {
            show_file_parse_error(&e, file_path, source);
            return Err(anyhow::anyhow!("Parse failed"));
        }
    };
    execute_program(
        program, source, file_path, verbose, no_ovm, ovm_stats, ovm_tier, deny, logger,
    )
}

/// Run an already-parsed program. `source` is retained only for error
/// snippets (parse is already done). This is the shared tail of running a
/// file and running a bundle whose AST was embedded by `olang build`, so a
/// built tool never re-parses at startup.
#[allow(clippy::too_many_arguments)]
fn execute_program(
    program: olang::ast::Program,
    source: &str,
    file_path: &PathBuf,
    verbose: bool,
    no_ovm: bool,
    ovm_stats: bool,
    ovm_tier: Option<u32>,
    deny: Option<olang::caps::Caps>,
    logger: &Logger,
) -> anyhow::Result<()> {
    // Get absolute path for module resolution
    let absolute_path = if file_path.is_absolute() {
        file_path.clone()
    } else {
        std::env::current_dir().unwrap_or_default().join(file_path)
    };

    // One execution model: the interpreter with the bytecode tier enabled,
    // promoting eligible functions on their first call. --no-ovm disables
    // the tier for a pure tree-walk (debugging / semantics reference);
    // --ovm-tier=N raises the promotion threshold.
    let mut interpreter = olang::interpreter::Interpreter::new();

    if !no_ovm {
        let threshold = ovm_tier.unwrap_or(1);
        interpreter.enable_bytecode_tier(threshold, verbose);
    }

    // Set file context for proper module resolution
    interpreter.set_current_file(&absolute_path);

    let mut caps_installed = false;
    // If the file lives in a package (an olang.toml is found by walking up),
    // resolve its dependencies and hand the interpreter the dependency map so
    // `use <dep>` paths resolve. Path and cached-git deps are cheap; a missing
    // dependency surfaces when the `use` is evaluated, not here.
    if let Some(root) = olang::pkg::manifest::Manifest::find_root(&absolute_path) {
        let opts = olang::pkg::InstallOptions {
            registry: std::env::var("OLANG_REGISTRY")
                .ok()
                .map(std::path::PathBuf::from),
            ..Default::default()
        };
        match olang::pkg::install(&root, &opts) {
            Ok(map) => {
                let mut map: std::collections::HashMap<_, _> = map.into_iter().collect();
                // A package is referable by its own name from within itself,
                // so a single-file package can `use <own_name> { ... }`.
                let manifest = olang::pkg::manifest::Manifest::load(&root).ok();
                if let Some(m) = &manifest {
                    map.entry(m.package.name.clone())
                        .or_insert_with(|| root.clone());
                }
                // Capabilities: the manifest's grant, attenuated per
                // dependency, intersected with any --deny/OLANG_DENY.
                // A bad capabilities block refuses to run — an
                // unenforceable grant must never silently run wide open.
                let config = manifest.as_ref().and_then(|m| m.capabilities.clone());
                if let Some(config) = config {
                    match olang::caps::CapTable::build(&config, &map) {
                        Ok(mut table) => {
                            if let Some(d) = deny {
                                table.app = table.app.intersect(d);
                                for entry in &mut table.deps {
                                    entry.2 = entry.2.intersect(d);
                                }
                            }
                            interpreter.set_capabilities(table);
                            caps_installed = true;
                        }
                        Err(e) => return Err(anyhow::anyhow!("{}", e)),
                    }
                }
                interpreter.set_dependency_map(map);
            }
            Err(e) => {
                if verbose {
                    logger.warn("main", &format!("package resolution: {}", e));
                }
            }
        }
    }

    // A --deny/OLANG_DENY restriction applies to every run, package or
    // not — a bare script gets a deny-only table.
    if !caps_installed && let Some(d) = deny {
        interpreter.set_capabilities(olang::caps::CapTable {
            app: d,
            deps: Vec::new(),
        });
    }

    match interpreter.eval_program(program) {
        Ok(result) => {
            if verbose {
                logger.info("main", &format!("Result: {:?}", result));
            }
            if ovm_stats {
                match interpreter.bytecode_tier_stats() {
                    Some(tier) => println!(
                        "Bytecode tier: {} promoted, {} rejected, {} bytecode calls, {} instructions, {} native calls",
                        tier.promoted,
                        tier.rejected,
                        tier.bytecode_calls,
                        tier.instructions_executed,
                        tier.jit_native_calls
                    ),
                    None => println!("Bytecode tier: disabled (--no-ovm)"),
                }
            }
            Ok(())
        }
        Err(e) => {
            let location = interpreter.take_error_location();
            show_classic_interpreter_error(&e, file_path, &interpreter, location, source);
            Err(anyhow::anyhow!("Execution failed"))
        }
    }
}
fn start_repl(verbose: bool, no_ovm: bool, _logger: &Logger) -> anyhow::Result<()> {
    // One REPL: the interpreter with the bytecode tier enabled by default;
    // --no-ovm gives the pure tree-walker.
    let mut repl = Repl::with_tier(verbose, !no_ovm)?;
    Ok(repl.run()?)
}
