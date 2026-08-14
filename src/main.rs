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
    if let Some(source) = embedded_program() {
        let logger = init_logger();
        let _ = initialize_parallelization(None);
        set_parallel_threshold(10_000);
        miette::set_panic_hook();
        return run_embedded(&source, logger);
    }

    let cli = Cli::parse();

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
                let target = cli
                    .script_args
                    .first()
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("."));
                return olang::tools::test_runner::run(&target);
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
            _ => {}
        }
    }

    if let Some(file_path) = cli.file {
        // Watch mode: run the script in a child process (so os.exit and
        // crashes end the run, not the watcher) and rerun when any .ol
        // file in the script's directory changes.
        if cli.watch {
            return watch_loop(&file_path, &cli.script_args);
        }
        // Program's argv: the script path, then everything after it. Read via
        // os.args() inside the program.
        let mut argv = vec![file_path.to_string_lossy().to_string()];
        argv.extend(cli.script_args.clone());
        olang::stdlib::os::set_script_args(argv);

        // Execute file in batch mode
        if let Err(e) = execute_file(
            &file_path,
            cli.verbose,
            cli.no_ovm,
            cli.ovm_stats,
            cli.ovm_tier,
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
fn watch_loop(script: &std::path::Path, script_args: &[String]) -> i32 {
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
        let status = std::process::Command::new(&exe)
            .arg(script)
            .args(script_args)
            .status();
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
const OLANG_BUNDLE_MAGIC: &[u8; 8] = b"oLaNgBnd";

/// If this executable was produced by `olang build`, return the olang
/// source appended to it. `None` for the plain `olang` binary.
fn embedded_program() -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let exe = std::env::current_exe().ok()?;
    let mut f = std::fs::File::open(&exe).ok()?;
    let total = f.metadata().ok()?.len();
    if total < 16 {
        return None;
    }
    f.seek(SeekFrom::End(-16)).ok()?;
    let mut footer = [0u8; 16];
    f.read_exact(&mut footer).ok()?;
    if &footer[8..16] != OLANG_BUNDLE_MAGIC {
        return None;
    }
    let src_len = u64::from_le_bytes(footer[0..8].try_into().ok()?);
    if src_len == 0 || src_len + 16 > total {
        return None;
    }
    f.seek(SeekFrom::End(-(16 + src_len as i64))).ok()?;
    let mut buf = vec![0u8; src_len as usize];
    f.read_exact(&mut buf).ok()?;
    String::from_utf8(buf).ok()
}

/// Run a bundled program. Its argv is the whole process argv, so the
/// tool's own flags reach `os.args()` / `cli.args()` exactly as they
/// would for a normal script.
fn run_embedded(source: &str, logger: &Logger) -> i32 {
    olang::stdlib::os::set_script_args(std::env::args().collect());
    let path = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("program"));
    match execute_source(source, &path, false, false, false, None, logger) {
        Ok(()) => 0,
        Err(_) => 1,
    }
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
    OlangParser::new()
        .parse(&src)
        .map_err(|e| anyhow::anyhow!("{} does not parse:\n{}", source_path, e))?;

    let output = output.unwrap_or_else(|| {
        PathBuf::from(&source_path)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "program".to_string())
    });

    let exe = std::env::current_exe()?;
    std::fs::copy(&exe, &output)?;
    {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().append(true).open(&output)?;
        f.write_all(src.as_bytes())?;
        f.write_all(&(src.len() as u64).to_le_bytes())?;
        f.write_all(OLANG_BUNDLE_MAGIC)?;
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

fn execute_file(
    file_path: &PathBuf,
    verbose: bool,
    no_ovm: bool,
    ovm_stats: bool,
    ovm_tier: Option<u32>,
    logger: &Logger,
) -> anyhow::Result<()> {
    let source = std::fs::read_to_string(file_path)?;
    execute_source(
        &source, file_path, verbose, no_ovm, ovm_stats, ovm_tier, logger,
    )
}

/// Run a program from source already in hand (a file's contents, or the
/// program bundled into an `olang build` executable). `file_path` names
/// it for error messages and cwd-relative module resolution.
fn execute_source(
    source: &str,
    file_path: &PathBuf,
    verbose: bool,
    no_ovm: bool,
    ovm_stats: bool,
    ovm_tier: Option<u32>,
    logger: &Logger,
) -> anyhow::Result<()> {
    let parser = OlangParser::new();

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
                if let Ok(manifest) = olang::pkg::manifest::Manifest::load(&root) {
                    map.entry(manifest.package.name)
                        .or_insert_with(|| root.clone());
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

    match parser.parse(source) {
        Ok(ast) => match interpreter.eval_program(ast) {
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
        },
        Err(e) => {
            show_file_parse_error(&e, file_path, source);
            Err(anyhow::anyhow!("Parse failed"))
        }
    }
}
fn start_repl(verbose: bool, no_ovm: bool, _logger: &Logger) -> anyhow::Result<()> {
    // One REPL: the interpreter with the bytecode tier enabled by default;
    // --no-ovm gives the pure tree-walker.
    let mut repl = Repl::with_tier(verbose, !no_ovm)?;
    Ok(repl.run()?)
}
