use clap::Parser;
use std::path::PathBuf;
use std::process;

use olang::interpreter::Interpreter;
use olang::parser::Parser as OlangParser;
use olang::repl::Repl;

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
}

fn main() {
    let cli = Cli::parse();

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
        if let Err(e) = execute_file(&file_path, cli.verbose) {
            eprintln!("Error executing file: {}", e);
            process::exit(1);
        }
    } else {
        // Start REPL
        if let Err(e) = start_repl(cli.verbose) {
            eprintln!("REPL error: {}", e);
            process::exit(1);
        }
    }
}

fn execute_file(file_path: &PathBuf, verbose: bool) -> anyhow::Result<()> {
    let source = std::fs::read_to_string(file_path)?;
    let parser = OlangParser::new();
    let mut interpreter = Interpreter::new();

    match parser.parse(&source) {
        Ok(ast) => {
            let result = interpreter.eval_program(ast)?;
            if verbose {
                println!("Result: {:?}", result);
            }
            Ok(())
        }
        Err(e) => {
            eprintln!("Parse error: {}", e);
            Err(anyhow::anyhow!("Parse failed"))
        }
    }
}

fn start_repl(verbose: bool) -> anyhow::Result<()> {
    let mut repl = Repl::new(verbose)?;
    Ok(repl.run()?)
}
