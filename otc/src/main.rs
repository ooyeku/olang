use clap::{Parser, Subcommand};
use std::process;

mod commands;
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
    /// Run an Olang file
    Run {
        /// File to run (.rap). If omitted, starts REPL in batch mode (stdin)
        file: Option<String>,
    },
    /// Perform static analysis on an Olang file
    Check {
        /// File to check (.rap)
        file: Option<String>,
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

    let result = match cli.command {
        Commands::Run { file } => commands::run::execute(file, verbose),
        Commands::Check { file } => commands::check::execute(file, verbose),
        Commands::Repl => commands::repl::execute(verbose),
        Commands::Ovm(ovm_cmd) => ovm_cmd.execute(),
        Commands::Version => commands::version::execute(verbose),
    };

    if let Err(e) = result {
        eprintln!("Error: {e}");
        process::exit(1);
    }
}
