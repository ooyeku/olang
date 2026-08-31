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
    /// Run the project's benches (bench/*.ol): scaling curves, verified
    /// answers, peak memory, baselines
    Bench {
        /// Only benches whose name contains one of these
        filter: Vec<String>,
        /// Timed repetitions per point (default 5)
        #[arg(long, default_value_t = 5)]
        runs: usize,
        /// Override every bench's sizes ("1000,4000,16000")
        #[arg(long)]
        sizes: Option<String>,
        /// Save medians as a baseline JSON
        #[arg(long, value_name = "FILE")]
        save: Option<String>,
        /// Compare against a saved baseline
        #[arg(long, value_name = "FILE")]
        against: Option<String>,
        /// Exit non-zero when any point regresses past the noise floor
        #[arg(long)]
        fail_on_regress: bool,
        /// Rerun the slowest point under `olang profile`
        #[arg(long)]
        profile: bool,
    },
    /// Audit the installation against the ~/.olang contract; --fix
    /// applies the safe repairs
    Doctor {
        /// Apply the safe repairs (legacy files, history migration)
        #[arg(long)]
        fix: bool,
    },
    /// Reclaim the prunable parts of ~/.olang (cache by default)
    Clean {
        /// The dependency cache (the default when no flag is given)
        #[arg(long)]
        cache: bool,
        /// Regenerable runtime state (warm hints, REPL history)
        #[arg(long)]
        state: bool,
        /// Everything prunable
        #[arg(long)]
        all: bool,
    },
    /// Install the latest release and make it the default toolchain
    Update {
        /// Only report the current and latest versions
        #[arg(long)]
        check: bool,
    },
    /// Manage side-by-side toolchain installs
    #[command(subcommand)]
    Toolchain(commands::update::ToolchainCommand),
    /// Print a shell completion script (zsh, bash, fish, elvish)
    Completions {
        /// The shell to generate for
        shell: clap_complete::Shell,
    },
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
        Commands::Doctor { fix } => commands::doctor::run(fix),
        Commands::Clean { cache, state, all } => commands::clean::run(cache, state, all),
        Commands::Update { check } => commands::update::update(check),
        Commands::Toolchain(cmd) => commands::update::toolchain(cmd),
        Commands::Completions { shell } => {
            use clap::CommandFactory;
            clap_complete::generate(shell, &mut Cli::command(), "otc", &mut std::io::stdout());
            Ok(())
        }
        Commands::Bench {
            filter,
            runs,
            sizes,
            save,
            against,
            fail_on_regress,
            profile,
        } => commands::bench::execute(commands::bench::BenchArgs {
            filter,
            runs,
            sizes,
            save,
            against,
            fail_on_regress,
            profile,
        }),
    };

    if let Err(e) = result {
        eprintln!("Error: {e}");
        process::exit(1);
    }
}
