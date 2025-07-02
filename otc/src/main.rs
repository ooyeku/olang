use clap::{Parser, Subcommand};
use std::process;

mod commands;
mod config;
mod utils;

#[derive(Parser)]
#[command(name = "otc")]
#[command(about = "Olang Toolchain - Command-line tools for the Olang language")]
#[command(version = olang::VERSION)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Enable verbose output
    #[arg(short, long)]
    verbose: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Create a new Olang project
    New {
        /// Project name
        name: String,
        /// Initialize as library (default: application)
        #[arg(short, long)]
        lib: bool,
        /// Project template (default, web, cli, library)
        #[arg(short, long, default_value = "default")]
        template: String,
    },
    /// Build the current project
    Build {
        /// Release build (optimized)
        #[arg(short, long)]
        release: bool,
    },
    /// Run an Olang file
    Run {
        /// File to run (.ol). If omitted, starts REPL in batch mode (stdin)
        file: Option<String>,
    },
    /// Perform static analysis on an Olang file
    Check {
        /// File to check (.ol)
        file: Option<String>,
    },
    /// Run project tests
    Test {
        /// Run only tests matching this pattern
        #[arg(short, long)]
        filter: Option<String>,
    },
    /// Start an interactive REPL session
    Repl,
    /// Execute with Olang Virtual Machine (high-performance)
    #[command(about = "Execute Olang programs using the OVM for enhanced performance")]
    Ovm(commands::ovm::OvmCommand),
    /// Print version information
    Version,
}

fn main() {
    // Fancy panic / error handler
    miette::set_panic_hook();

    let cli = Cli::parse();
    let verbose = cli.verbose;

    // Initialize parallelization for optimal performance
    if let Err(e) = olang::parallel::initialize_parallelization(None) {
        if verbose {
            eprintln!("Warning: Failed to initialize parallel processing: {}", e);
        }
    } else if verbose {
        println!(
            "Parallel processing initialized with {} threads",
            num_cpus::get()
        );
    }

    // Set a very aggressive parallel threshold for maximum multi-threading by default
    olang::parallel::set_parallel_threshold(10);

    let result = match cli.command {
        Commands::New { name, lib, template } => commands::new::execute(name, lib, template, verbose),
        Commands::Build { release } => commands::new::build_project(release, verbose),
        Commands::Run { file } => commands::run::execute(file, verbose),
        Commands::Check { file } => commands::check::execute(file, verbose),
        Commands::Test { filter } => commands::new::test_project(filter, verbose),
        Commands::Repl => commands::repl::execute(verbose),
        Commands::Ovm(ovm_cmd) => ovm_cmd.execute(),
        Commands::Version => commands::version::execute(verbose),
    };

    if let Err(e) = result {
        eprintln!("Error: {e}");
        process::exit(1);
    }
}
