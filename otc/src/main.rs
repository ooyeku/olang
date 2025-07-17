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
    /// Show file dependencies
    Deps {
        /// File to analyze dependencies for (.ol)
        file: String,
    },
    /// Find unused shared functions
    Unused {
        /// Directory to scan for unused functions (default: current directory)
        #[arg(short, long, default_value = ".")]
        dir: String,
    },
    /// Visualize project structure
    Tree {
        /// Directory to visualize (default: current directory)
        #[arg(short, long, default_value = ".")]
        dir: String,
    },
    /// Suggest file reorganization
    Organize {
        /// Directory to analyze for reorganization (default: current directory)
        #[arg(short, long, default_value = ".")]
        dir: String,
    },
    /// Move a shared function between files
    #[command(name = "move-fn")]
    MoveFn {
        /// Function name to move
        function: String,
        /// Source file path
        from: String,
        /// Target file path
        to: String,
    },
    /// Rename a function across all usage sites
    #[command(name = "rename-fn")]
    RenameFn {
        /// Current function name
        old_name: String,
        /// New function name
        new_name: String,
        /// Directory to search for usages (default: current directory)
        #[arg(short, long, default_value = ".")]
        dir: String,
    },
    /// Extract functions into a new file
    #[command(name = "extract-file")]
    ExtractFile {
        /// Functions to extract (comma-separated)
        functions: String,
        /// Source file path
        from: String,
        /// Target file path (new file to create)
        to: String,
    },
    /// Merge two files together
    #[command(name = "merge-files")]
    MergeFiles {
        /// First file to merge
        file1: String,
        /// Second file to merge
        file2: String,
        /// Output file path
        output: String,
    },
    /// Fix import statements after refactoring
    #[command(name = "fix-imports")]
    FixImports {
        /// Directory to scan and fix imports (default: current directory)
        #[arg(short, long, default_value = ".")]
        dir: String,
    },
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
        Commands::Deps { file } => commands::deps::execute(file, verbose),
        Commands::Unused { dir } => commands::unused::execute(dir, verbose),
        Commands::Tree { dir } => commands::tree::execute(dir, verbose),
        Commands::Organize { dir } => commands::organize::execute(dir, verbose),
        Commands::MoveFn { function, from, to } => commands::refactor::move_function(function, from, to, verbose),
        Commands::RenameFn { old_name, new_name, dir } => commands::refactor::rename_function(old_name, new_name, dir, verbose),
        Commands::ExtractFile { functions, from, to } => commands::refactor::extract_file(functions, from, to, verbose),
        Commands::MergeFiles { file1, file2, output } => commands::refactor::merge_files(file1, file2, output, verbose),
        Commands::FixImports { dir } => commands::refactor::fix_imports(dir, verbose),
        Commands::Version => commands::version::execute(verbose),
    };

    if let Err(e) = result {
        eprintln!("Error: {e}");
        process::exit(1);
    }
}
