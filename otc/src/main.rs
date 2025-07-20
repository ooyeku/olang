use clap::{Parser, Subcommand};
use std::process;

mod commands;
mod config;
mod git_package;
mod global;
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
        /// Directory to search for tests (default: current directory)
        #[arg(short, long, default_value = ".")]
        dir: String,
        /// Watch for file changes and re-run tests
        #[arg(short, long)]
        watch: bool,
        /// Number of parallel test threads
        #[arg(short, long)]
        threads: Option<usize>,
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
    /// Install packages (project-local or global)
    Install {
        /// Package URL to install (if omitted, installs from olang.toml)
        url: Option<String>,
        /// Install globally (available to all projects)
        #[arg(short, long)]
        global: bool,
        /// Use only cached packages (no network)
        #[arg(long)]
        offline: bool,
    },
    /// List installed packages
    List {
        /// List global packages
        #[arg(short, long)]
        global: bool,
        /// Show cached packages
        #[arg(long)]
        cached: bool,
    },
    /// Update packages to latest versions
    Update {
        /// Update global packages
        #[arg(short, long)]
        global: bool,
    },
    /// Remove a package
    Remove {
        /// Package name to remove
        name: String,
        /// Remove from global packages
        #[arg(short, long)]
        global: bool,
    },
    /// Clean unused cached packages
    Clean,
    /// Show package information
    Info {
        /// Package name or URL
        package: String,
        /// Show global package info
        #[arg(short, long)]
        global: bool,
    },
    /// Global configuration management
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// Cache management
    Cache {
        #[command(subcommand)]
        action: CacheAction,
    },
    /// OTC self-management
    #[command(name = "self")]
    SelfCmd {
        #[command(subcommand)]
        action: SelfAction,
    },
    /// Show system information
    #[command(name = "system-info")]
    SystemInfo,
    /// Check for common issues and fix them
    Doctor,
    /// Print version information
    Version,
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Set a configuration value
    Set {
        /// Configuration key (e.g., cache.max_size_gb)
        key: String,
        /// Configuration value
        value: String,
    },
    /// Get a configuration value
    Get {
        /// Configuration key
        key: String,
    },
    /// List all configuration values
    List,
}

#[derive(Subcommand)]
enum CacheAction {
    /// Show cache status
    Status,
    /// Clean unused cached packages
    Clean,
    /// Clear all cached packages
    Clear,
    /// Repair cache integrity
    Repair,
}

#[derive(Subcommand)]
enum SelfAction {
    /// Update OTC to latest version
    Update,
    /// Migrate from older OTC versions
    Migrate,
    /// Clean legacy installation artifacts
    #[command(name = "clean-legacy")]
    CleanLegacy,
    /// Uninstall OTC and all data
    Uninstall,
}

fn main() {
    // Set up basic error reporting
    std::panic::set_hook(Box::new(|info| {
        eprintln!("Application panicked: {}", info);
    }));

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
        Commands::Test { filter, dir, watch, threads } => commands::test::execute(filter, dir, watch, threads, verbose),
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
        
        // Git Package Management Commands
        Commands::Install { url, global, offline: _ } => {
            if global {
                if let Some(url) = url {
                    commands::global::install_global(url, verbose)
                } else {
                    Err(anyhow::anyhow!("URL required for global package installation"))
                }
            } else {
                // Use the new Simple Git Package System
                commands::install::execute(url, verbose)
            }
        },
        Commands::List { global, cached: _ } => {
            if global {
                commands::global::list_global(verbose)
            } else {
                // Use the new Simple Git Package System
                commands::install::list(verbose)
            }
        },
        Commands::Update { global } => {
            if global {
                Err(anyhow::anyhow!("Global package update not yet implemented"))
            } else {
                // Use the new Simple Git Package System
                commands::install::update(verbose)
            }
        },
        Commands::Remove { name, global } => {
            if global {
                commands::global::remove_global(name, verbose)
            } else {
                // Use the new Simple Git Package System
                commands::install::remove(name, verbose)
            }
        },
        Commands::Clean => {
            // Use the new Simple Git Package System
            commands::install::clean(verbose)
        },
        Commands::Info { package, global } => {
            if global {
                commands::global::info_global(package, verbose)
            } else {
                // Use the new Simple Git Package System for URL info
                commands::install::info(package, verbose)
            }
        },
        
        // Configuration Management Commands
        Commands::Config { action } => {
            match action {
                ConfigAction::Set { key, value } => commands::global::config_set(key, value, verbose),
                ConfigAction::Get { key } => commands::global::config_get(key, verbose),
                ConfigAction::List => commands::global::config_list(verbose),
            }
        },
        
        // Cache Management Commands
        Commands::Cache { action } => {
            match action {
                CacheAction::Status => commands::global::cache_status(verbose),
                CacheAction::Clean => commands::global::cache_clean(verbose),
                CacheAction::Clear => commands::global::cache_clear(verbose),
                CacheAction::Repair => commands::global::cache_repair(verbose),
            }
        },
        
        // Self Management Commands
        Commands::SelfCmd { action } => {
            match action {
                SelfAction::Update => commands::global::update_self(verbose),
                SelfAction::Migrate => commands::global::migrate(verbose),
                SelfAction::CleanLegacy => commands::global::clean_legacy(verbose),
                SelfAction::Uninstall => commands::global::uninstall_self(verbose),
            }
        },
        
        // System Commands
        Commands::SystemInfo => commands::global::system_info(verbose),
        Commands::Doctor => commands::global::doctor(verbose),
        Commands::Version => commands::version::execute(verbose),
    };

    if let Err(e) = result {
        eprintln!("Error: {e}");
        process::exit(1);
    }
}
