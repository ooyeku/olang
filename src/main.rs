use clap::Parser;
use std::path::PathBuf;
use std::process;

use olang::ovm_integration::{IntegrationConfig, OvmInterpreter};
use olang::ovm::OvmConfig;
use olang::parallel::{initialize_parallelization, set_parallel_threshold};
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

    /// Disable OVM and use classic interpreter only
    #[arg(long)]
    no_ovm: bool,

    /// Show OVM performance statistics
    #[arg(long)]
    ovm_stats: bool,
}

fn main() {
    let cli = Cli::parse();

    // Initialize parallelization early for optimal performance
    if let Err(e) = initialize_parallelization(None) {
        if cli.verbose {
            eprintln!("Warning: Failed to initialize parallel processing: {}", e);
        }
    } else if cli.verbose {
        println!(
            "✅ Multi-threading enabled: {} CPU cores detected, using aggressive parallelization",
            num_cpus::get()
        );
    }

    // Set a very aggressive parallel threshold for maximum multi-threading by default
    // Parallelize even small lists (10+ items) to utilize all CPU cores
    set_parallel_threshold(10);

    if cli.verbose {
        println!(
            "🚀 Automatic parallelization: Lists with 10+ items will use all {} cores",
            num_cpus::get()
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
        if let Err(e) = execute_file(&file_path, cli.verbose, cli.no_ovm, cli.ovm_stats) {
            eprintln!("Error executing file: {}", e);
            process::exit(1);
        }
    } else {
        // Start REPL
        if let Err(e) = start_repl(cli.verbose, cli.no_ovm) {
            eprintln!("REPL error: {}", e);
            process::exit(1);
        }
    }
}

fn execute_file(file_path: &PathBuf, verbose: bool, no_ovm: bool, ovm_stats: bool) -> anyhow::Result<()> {
    let source = std::fs::read_to_string(file_path)?;
    let parser = OlangParser::new();
    
    if no_ovm {
        // Use classic interpreter
        let mut interpreter = olang::interpreter::Interpreter::new();
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
    } else {
        // Use OVM interpreter with JIT compilation
        let integration_config = IntegrationConfig {
            use_ovm_by_default: true,
            ovm_complexity_threshold: 5,
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
        
        // Initialize OVM with default configuration
        let ovm_config = OvmConfig::default();
        interpreter.initialize_ovm(ovm_config)?;

        if verbose {
            println!("🚀 OVM initialized with JIT compilation enabled");
        }

        match parser.parse(&source) {
            Ok(ast) => {
                let result = interpreter.eval_program(ast)?;
                if verbose {
                    println!("Result: {:?}", result);
                }
                
                if ovm_stats {
                    let stats = interpreter.get_stats();
                    println!("\n📊 OVM Performance Statistics:");
                    println!("  Classic executions: {}", stats.classic_executions);
                    println!("  OVM executions: {}", stats.ovm_executions);
                    println!("  Fallback executions: {}", stats.fallback_executions);
                    println!("  Compilations: {}", stats.compilation_count);
                    println!("  Average classic time: {:.2}ms", stats.average_classic_time_ms);
                    println!("  Average OVM time: {:.2}ms", stats.average_ovm_time_ms);
                }
                
                Ok(())
            }
            Err(e) => {
                eprintln!("Parse error: {}", e);
                Err(anyhow::anyhow!("Parse failed"))
            }
        }
    }
}

fn start_repl(verbose: bool, no_ovm: bool) -> anyhow::Result<()> {
    if no_ovm {
        // Use classic REPL
        let mut repl = Repl::new(verbose)?;
        Ok(repl.run()?)
    } else {
        // Use OVM-enhanced REPL
        let integration_config = IntegrationConfig {
            use_ovm_by_default: true,
            ovm_complexity_threshold: 5,
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
            println!("🚀 OVM REPL initialized with JIT compilation enabled");
        }

        // For now, use classic REPL but with OVM interpreter
        // TODO: Create OVM-enhanced REPL
        let mut repl = Repl::new(verbose)?;
        Ok(repl.run()?)
    }
}
