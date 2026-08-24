//! otc — the olang project tool.
//!
//! The division of labor is one sentence: everything that touches a
//! *file* lives in the `olang` binary (run, repl, test, fmt, check,
//! bench, profile); everything that touches a *project* lives here.
//! otc scaffolds projects, manages their local dependencies, and keeps
//! the user's shelf of local libraries.

// otc surfaces olang's interpreter Results; the large-error refactor is
// tracked in the olang crate (see olang/src/lib.rs).
#![allow(clippy::result_large_err)]
use clap::{Parser, Subcommand};
use std::process;

mod commands;

#[derive(Parser)]
#[command(name = "otc")]
#[command(about = "olang project tool — scaffolding and local package management")]
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
    /// Create a new olang project
    New {
        /// Project name (also the directory to create)
        name: String,
        /// A library package: index.ol at the root, usable as a dependency
        #[arg(long)]
        lib: bool,
        /// A full-stack web app: JSON API + wasm frontend, one process
        #[arg(long)]
        web: bool,
    },
    /// Add a dependency: a path (otc add ../my-lib) or a name from your
    /// shelf (otc add my-lib). Creates olang.toml if the project has none
    Add {
        /// Path to a library, or the name of a shelved one
        spec: String,
        /// Treat the argument as a path even if it looks like a bare name
        #[arg(long)]
        path: Option<String>,
        /// Replace an existing dependency's source
        #[arg(long)]
        force: bool,
    },
    /// Remove a dependency
    Remove {
        /// Dependency name
        name: String,
    },
    /// Show this project's dependencies and where each one lives
    List,
    /// Fetch dependencies and write olang.lock
    Install {
        /// Fail if the lockfile would change (for CI)
        #[arg(long)]
        frozen: bool,
        /// Re-resolve everything to the newest satisfying sources
        #[arg(long)]
        update: bool,
    },
    /// Your local library shelf: register libraries once, add them by
    /// name from any project
    #[command(subcommand)]
    Lib(commands::lib::LibCommand),
}

fn main() {
    let cli = Cli::parse();
    let verbose = cli.verbose;

    let result = match cli.command {
        Commands::New { name, lib, web } => commands::new::execute(name, lib, web, verbose),
        Commands::Add { spec, path, force } => {
            commands::project::add(&spec, path.as_deref(), force)
        }
        Commands::Remove { name } => commands::project::remove(&name),
        Commands::List => commands::project::list(),
        Commands::Install { frozen, update } => {
            commands::project::do_install(frozen, update, verbose)
        }
        Commands::Lib(cmd) => cmd.execute(),
    };

    if let Err(e) = result {
        eprintln!("Error: {e}");
        process::exit(1);
    }
}
