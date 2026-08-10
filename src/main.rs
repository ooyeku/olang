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

    // Tool commands: `olang test [path]` and `olang fmt [paths] [--check]`.
    // The first positional dispatches; a file literally named `test` or
    // `fmt` is still runnable as `./test` or `test.ol`.
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
            _ => {}
        }
    }

    if let Some(file_path) = cli.file {
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
    eprintln!(
        "    • Use {} for interactive mode with better error messages",
        "olang".bright_cyan()
    );
    eprintln!();
}

/// Feature 9: Show classic interpreter errors with enhanced context and suggestions
fn show_classic_interpreter_error(
    error: &olang::interpreter::InterpreterError,
    file_path: &std::path::Path,
    interpreter: &olang::interpreter::Interpreter,
) {
    eprintln!("\n{}", "═══ Execution Error ═══".bright_red().bold());
    eprintln!(
        "  {}: {}",
        "File".bright_blue().bold(),
        file_path.display().to_string().bright_white()
    );

    // Use the enhanced error formatter
    let formatted_error = interpreter.format_error(error);
    eprintln!("\n{}", formatted_error);

    eprintln!();
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

fn execute_file(
    file_path: &PathBuf,
    verbose: bool,
    no_ovm: bool,
    ovm_stats: bool,
    ovm_tier: Option<u32>,
    logger: &Logger,
) -> anyhow::Result<()> {
    let source = std::fs::read_to_string(file_path)?;
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

    match parser.parse(&source) {
        Ok(ast) => match interpreter.eval_program(ast) {
            Ok(result) => {
                if verbose {
                    logger.info("main", &format!("Result: {:?}", result));
                }
                if ovm_stats {
                    match interpreter.bytecode_tier_stats() {
                        Some(tier) => println!(
                            "Bytecode tier: {} promoted, {} rejected, {} bytecode calls, {} instructions",
                            tier.promoted,
                            tier.rejected,
                            tier.bytecode_calls,
                            tier.instructions_executed
                        ),
                        None => println!("Bytecode tier: disabled (--no-ovm)"),
                    }
                }
                Ok(())
            }
            Err(e) => {
                show_classic_interpreter_error(&e, file_path, &interpreter);
                Err(anyhow::anyhow!("Execution failed"))
            }
        },
        Err(e) => {
            show_file_parse_error(&e, file_path, &source);
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
