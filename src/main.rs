use clap::Parser;
use std::path::PathBuf;
use std::process;
use colored::*;

use olang::{
    log::{init_logger, Logger},
    ovm_integration::{IntegrationConfig, OvmInterpreter},
    parallel::{initialize_parallelization, set_parallel_threshold},
    parser::{Parser as OlangParser, ErrorSuggestion, SuggestionSeverity},
    repl::Repl,
    OvmConfig,
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
}

fn main() {
    let cli = Cli::parse();
    
    // Initialize logger
    let logger = init_logger();

    // Initialize parallelization early for optimal performance
    if let Err(e) = initialize_parallelization(None) {
        if cli.verbose {
            logger.warn("main", &format!("Failed to initialize parallel processing: {}", e));
        }
    } else if cli.verbose {
        logger.info(
            "main",
            &format!("Multi-threading enabled: {} CPU cores detected, using aggressive parallelization", num_cpus::get())
        );
    }

    // Set a very aggressive parallel threshold for maximum multi-threading by default
    // Parallelize even small lists (10+ items) to utilize all CPU cores
    set_parallel_threshold(10);

    if cli.verbose {
        logger.info(
            "main",
            &format!("Automatic parallelization: Lists with 10+ items will use all {} cores", num_cpus::get())
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

    if let Some(file_path) = cli.file {
        // Execute file in batch mode
        if let Err(e) = execute_file(&file_path, cli.verbose, cli.no_ovm, cli.ovm_stats, logger) {
            logger.error("main", &format!("Error executing file: {}", e));
            process::exit(1);
        }
    } else {
        // Start REPL
        if let Err(e) = start_repl(cli.verbose, cli.no_ovm, logger) {
            logger.error("main", &format!("REPL error: {}", e));
            process::exit(1);
        }
    }
}

/// Enhanced error display for file execution
fn show_file_parse_error(error: &olang::parser::ParseError, file_path: &PathBuf, source: &str) {
    println!("\n{}", "═══ Parse Error ═══".bright_red().bold());
    println!("  {}: {}", "File".bright_blue().bold(), file_path.display().to_string().bright_white());
    
    match error {
        olang::parser::ParseError::InvalidSyntaxWithPosition { message, line, column, snippet } => {
            println!("  {}: {}", "Error".bright_red().bold(), message.bright_white());
            println!("  {}: Line {}, Column {}", "Location".bright_yellow().bold(), 
                line.to_string().bright_cyan(), column.to_string().bright_cyan());
            
            if !snippet.trim().is_empty() {
                println!("\n  {}", "📝 Code Context:".bright_blue().bold());
                show_highlighted_snippet(snippet);
            }
        }
        olang::parser::ParseError::UnexpectedTokenWithPosition { token, line, column, snippet } => {
            println!("  {}: Unexpected token '{}'", "Error".bright_red().bold(), token.bright_yellow().bold());
            println!("  {}: Line {}, Column {}", "Location".bright_yellow().bold(), 
                line.to_string().bright_cyan(), column.to_string().bright_cyan());
            
            if !snippet.trim().is_empty() {
                println!("\n  {}", "📝 Code Context:".bright_blue().bold());
                show_highlighted_snippet(snippet);
            }
        }
        _ => {
            println!("  {}: {}", "Error".bright_red().bold(), error);
        }
    }
    
    // Get suggestions
    let parser = OlangParser::new();
    let suggestions = parser.get_suggestions(error, source);
    
    if !suggestions.is_empty() {
        println!("\n  {}", "💡 Suggestions:".bright_cyan().bold());
        for suggestion in suggestions {
            show_suggestion(&suggestion);
        }
    }
    
    // Show help topics
    println!("\n  {}", "📚 Help:".bright_cyan().bold());
    println!("    • Type {} for syntax help", "olang -h".bright_cyan());
    println!("    • Use {} for interactive mode with better error messages", "olang".bright_cyan());
    println!();
}

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
                println!("    {}{}│{}{}", 
                    line_num.bright_black(),
                    " ".repeat(4 - line_num.len().min(4)),
                    " ",
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
                
                println!("    {}{}│ {}", 
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
                if let Some(&next_ch) = chars.peek() {
                    if (ch == '=' && next_ch == '=') ||
                       (ch == '!' && next_ch == '=') ||
                       (ch == '<' && next_ch == '=') ||
                       (ch == '>' && next_ch == '=') ||
                       (ch == '&' && next_ch == '&') ||
                       (ch == '|' && next_ch == '|') {
                        op.push(chars.next().unwrap());
                    }
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
        "let" | "fn" | "if" | "else" | "match" | "for" | "while" | "loop" | "break" | "continue" |
        "true" | "false" | "import" | "export" | "type" | "async" | "await" | "try" | "catch" => {
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
        SuggestionSeverity::Error => "❌",
        SuggestionSeverity::Warning => "⚠️",
        SuggestionSeverity::Hint => "💡",
        SuggestionSeverity::Info => "ℹ️",
    };
    
    let severity_color = match suggestion.severity {
        SuggestionSeverity::Error => "red",
        SuggestionSeverity::Warning => "yellow",
        SuggestionSeverity::Hint => "cyan",
        SuggestionSeverity::Info => "blue",
    };
    
    println!("    {} {}", severity_icon, suggestion.message.color(severity_color).bold());
    
    if let Some(fix) = &suggestion.fix {
        println!("      {}: {}", "Fix".bright_green().bold(), fix);
    }
    
    if let Some(help) = &suggestion.help {
        println!("      {}: {}", "Help".bright_blue().bold(), help);
    }
}

fn execute_file(file_path: &PathBuf, verbose: bool, no_ovm: bool, ovm_stats: bool, logger: &Logger) -> anyhow::Result<()> {
    let source = std::fs::read_to_string(file_path)?;
    let parser = OlangParser::new();
    
    if no_ovm {
        // Use classic interpreter
        let mut interpreter = olang::interpreter::Interpreter::new();
        match parser.parse(&source) {
            Ok(ast) => {
                let result = interpreter.eval_program(ast)?;
                if verbose {
                    logger.info("main", &format!("Result: {:?}", result));
                }
                Ok(())
            }
            Err(e) => {
                show_file_parse_error(&e, file_path, &source);
                Err(anyhow::anyhow!("Parse failed"))
            }
        }
    } else {
        // Use OVM integration
        let config = IntegrationConfig::default();
        let mut ovm_interpreter = OvmInterpreter::with_config(config);
        
        // Initialize OVM
        if let Err(e) = ovm_interpreter.initialize_ovm_default() {
            if verbose {
                logger.warn("main", &format!("OVM initialization failed, falling back to classic: {}", e));
            }
        }
        
        match parser.parse(&source) {
            Ok(ast) => {
                match ovm_interpreter.eval_program(ast) {
                    Ok(result) => {
                        if verbose {
                            logger.info("main", &format!("Execution result: {:?}", result));
                        }
                        
                        if ovm_stats {
                            let stats = ovm_interpreter.get_stats();
                            logger.info("main", "OVM Performance Statistics:");
                            logger.info("main", &format!("  Classic executions: {}", stats.classic_executions));
                            logger.info("main", &format!("  OVM executions: {}", stats.ovm_executions));
                            logger.info("main", &format!("  Fallback executions: {}", stats.fallback_executions));
                            logger.info("main", &format!("  Compilations: {}", stats.compilation_count));
                            logger.info("main", &format!("  Average classic time: {:.2}ms", stats.average_classic_time_ms));
                            logger.info("main", &format!("  Average OVM time: {:.2}ms", stats.average_ovm_time_ms));
                        }
                        
                        Ok(())
                    }
                    Err(e) => {
                        logger.error("main", &format!("Execution error: {}", e));
                        Err(anyhow::anyhow!("Execution failed"))
                    }
                }
            }
            Err(e) => {
                show_file_parse_error(&e, file_path, &source);
                Err(anyhow::anyhow!("Parse failed"))
            }
        }
    }
}

fn start_repl(verbose: bool, no_ovm: bool, logger: &Logger) -> anyhow::Result<()> {
    if no_ovm {
        // Use classic REPL
        let mut repl = Repl::new(verbose)?;
        Ok(repl.run()?)
    } else {
        // Use OVM-enhanced REPL
        let integration_config = IntegrationConfig {
            use_ovm_by_default: true,
            ovm_complexity_threshold: 1,
            auto_compile_functions: true,
            enable_ovm_lazy_eval: true,
            fallback_on_error: true,
            enable_ovm_builtins: true,
            ovm_preferred_builtins: vec![
                "len".to_string(),
                "typeof".to_string(),
                "to_string".to_string(),
                "sum".to_string(),
                "average".to_string(),
                "min".to_string(),
                "max".to_string(),
                "reverse".to_string(),
                "sort".to_string(),
                "contains".to_string(),
            ],
        };

        let mut interpreter = OvmInterpreter::with_config(integration_config);
        
        // Initialize OVM
        let ovm_config = OvmConfig::default();
        interpreter.initialize_ovm(ovm_config)?;

        if verbose {
            logger.info("main", "OVM REPL initialized with JIT compilation enabled");
        }

        // For now, use classic REPL but with OVM interpreter
        // TODO: Create OVM-enhanced REPL
        let mut repl = Repl::new(verbose)?;
        Ok(repl.run()?)
    }
}
