// otc surfaces olang's interpreter Results; the large-error refactor is
// tracked in the olang crate (see olang/src/lib.rs).
#![allow(clippy::result_large_err)]
use clap::{Parser, Subcommand};
use std::process;

mod commands;

#[derive(Parser)]
#[command(name = "otc")]
#[command(about = "olang toolchain — scaffolding, packages, and code analysis")]
#[command(version = olang::VERSION)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Enable verbose output
    #[arg(short, long)]
    verbose: bool,
}

// The surface is deliberately small: everything about *running* olang code
// (files, REPL, tests, fmt) lives in the `olang` binary. otc owns what the
// runtime doesn't: project scaffolding, the package manager, and static
// analysis over source trees.
#[derive(Subcommand)]
enum Commands {
    /// Create a new olang project
    New {
        /// Project name (also the directory to create)
        name: String,
    },
    /// Manage packages (olang.toml, olang.lock, registry)
    #[command(subcommand)]
    Pkg(commands::pkg::PkgCommand),
    /// Perform static analysis on an olang file
    Check {
        /// File to check (.ol)
        file: String,
    },
    /// Show a file's `use` dependencies
    Deps {
        /// File to analyze (.ol)
        file: String,
    },
    /// Find unused shared functions
    Unused {
        /// Directory to scan (default: current directory)
        #[arg(default_value = ".")]
        dir: String,
    },
    /// Run a program on the bytecode tier; --compare verifies it against the
    /// plain interpreter
    Ovm(commands::ovm::OvmCommand),
}

fn main() {
    let cli = Cli::parse();
    let verbose = cli.verbose;

    let result = match cli.command {
        Commands::New { name } => commands::new::execute(name, verbose),
        Commands::Pkg(cmd) => cmd.execute(verbose),
        Commands::Check { file } => commands::check::execute(file, verbose),
        Commands::Deps { file } => commands::deps::execute(file, verbose),
        Commands::Unused { dir } => commands::unused::execute(dir, verbose),
        Commands::Ovm(ovm_cmd) => ovm_cmd.execute(),
    };

    if let Err(e) = result {
        eprintln!("Error: {e}");
        process::exit(1);
    }
}
