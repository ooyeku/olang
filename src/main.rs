use clap::{Parser, Subcommand};
use colored::*;
use std::path::PathBuf;
use std::process;

use olang::{
    log::{Logger, init_logger},
    parallel::{initialize_parallelization, set_parallel_threshold},
    parser::{ErrorSuggestion, Parser as OlangParser, SuggestionSeverity},
    repl::Repl,
};

/// olang command-line interface.
///
/// The bare form runs a program: `olang script.ol [args...]`, or drops into
/// the REPL when no file is given. Everything else is a named command
/// (`olang inspect`, `olang build`, `olang check`, …) listed under Commands.
///
/// Design note: olang is file-first, like `python` and `node` — `olang
/// script.ol` runs a file directly. A word that isn't a known command and
/// isn't a flag is taken as a file to run, so `olang report.ol` and `olang
/// ./build` still run those files even though `build` is a command.
#[derive(Parser)]
#[command(name = "olang")]
#[command(version)]
#[command(
    about = "olang — the Open Language. Run programs, inspect binaries, and prove what code does."
)]
#[command(long_about = None)]
#[command(subcommand_help_heading = "Commands")]
#[command(
    override_usage = "olang [RUN OPTIONS] <FILE> [ARGS]...\n       olang <COMMAND> [ARGS]...\n       olang                       (starts the REPL)"
)]
#[command(after_help = "Examples:
  olang app.ol                 Run a program
  olang app.ol --port 8080     Run, passing --port 8080 to the program
  olang --watch app.ol         Re-run on every save
  olang --deny net app.ol      Run with the network capability withheld
  olang check src/             Type-check a directory
  olang build app.ol -o app    Compile to a self-contained binary
  olang inspect app --source   Print the source embedded in a built binary
  olang inspect app --caps     Show a built binary's capability grant
  olang record app.ol          Run and record inputs to app.olt
  olang replay app.olt         Re-run that recording bit-for-bit
  olang <command> --help       Full help for any command

Run options apply to `olang <file>` and `olang run <file>`. Every command
also honors the OLANG_DENY environment variable. Full reference: docs/tooling.md")]
struct Cli {
    /// Execute in batch mode (no REPL)
    #[arg(short, long, help_heading = "Run options")]
    batch: bool,

    /// Enable verbose output
    #[arg(short, long, help_heading = "Run options")]
    verbose: bool,

    /// Enable tracing for debugging
    #[arg(long, help_heading = "Run options")]
    trace: bool,

    /// Disable OVM and use the classic interpreter only
    #[arg(long, help_heading = "Run options")]
    no_ovm: bool,

    /// Show OVM performance statistics
    #[arg(long, help_heading = "Run options")]
    ovm_stats: bool,

    /// Re-run the file whenever any .ol file in its directory changes
    /// (place before the file: `olang --watch script.ol`)
    #[arg(long, help_heading = "Run options")]
    watch: bool,

    /// Enable parallel evaluation of independent expressions
    #[arg(long, help_heading = "Run options")]
    enable_parallel: bool,

    /// Set maximum parallelism for OVM/builtins (threads). Also respects the
    /// OVM_PARALLELISM environment variable.
    #[arg(long, value_name = "N", help_heading = "Run options")]
    ovm_parallelism: Option<usize>,

    /// Compile hot functions to OVM bytecode after N calls (default 50 when
    /// the flag is given without a value). Functions the tier cannot compile
    /// keep running on the interpreter.
    #[arg(long, value_name = "N", num_args = 0..=1, require_equals = true, default_missing_value = "50", help_heading = "Run options")]
    ovm_tier: Option<u32>,

    /// Deny capabilities for this run, on top of any manifest: a comma list
    /// of fs, fs-write, net, proc, db, env (e.g. --deny net,fs-write). Also
    /// honored from the OLANG_DENY environment variable.
    #[arg(long, value_name = "CAPS", help_heading = "Run options")]
    deny: Option<String>,

    /// Record this run's nondeterministic inputs to a portable .olt trace.
    /// Replay it bit-for-bit later with `olang replay <trace>`.
    #[arg(long, value_name = "TRACE.olt", help_heading = "Run options")]
    record: Option<String>,

    /// Report which capabilities the program actually exercised, then print a
    /// suggested least-privilege [capabilities] manifest. Runs on the
    /// interpreter tier so every effect is seen.
    #[arg(long, help_heading = "Run options")]
    trace_caps: bool,

    /// Maximum call depth before a clean "maximum call depth exceeded"
    /// error (default 100000). Both execution tiers enforce the same cap.
    #[arg(long, value_name = "N", help_heading = "Run options")]
    max_depth: Option<usize>,

    /// With --trace-caps, write the suggested [capabilities] block into the
    /// package's olang.toml instead of only printing it. Never overwrites an
    /// existing [capabilities] block.
    #[arg(long, help_heading = "Run options")]
    write: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

/// olang's named commands. The catch-all `External` variant is how the
/// file-first form (`olang script.ol`) reaches the runner: any leading word
/// that is neither a known command nor a flag is treated as a file path,
/// with the rest passed through as the program's argv.
#[derive(Subcommand)]
enum Commands {
    /// Run a program (the explicit form of `olang <file>`)
    Run {
        /// Program to execute
        #[arg(value_name = "FILE")]
        file: PathBuf,
        /// Arguments passed to the program, readable via os.args()
        #[arg(
            value_name = "ARGS",
            trailing_var_arg = true,
            allow_hyphen_values = true
        )]
        args: Vec<String>,
    },

    /// Start the interactive REPL (the explicit form of bare `olang`)
    Repl,

    /// Type-check programs without running them
    Check {
        /// Files or directories to check (default: current directory)
        #[arg(value_name = "PATH")]
        paths: Vec<PathBuf>,
        /// Also run project lints written as olang functions over the meta AST
        #[arg(long, value_name = "RULES.ol")]
        rules: Option<PathBuf>,
    },

    /// Run a program under the sampling profiler
    Profile {
        /// Program to execute
        #[arg(value_name = "FILE")]
        file: PathBuf,
        /// Sampling interval in microseconds (default 1000 — one per ms)
        #[arg(long, value_name = "US", default_value_t = 1000)]
        interval: u64,
        /// How many functions to list (default 20)
        #[arg(long, value_name = "N", default_value_t = 20)]
        top: usize,
        /// Arguments passed to the program, readable via os.args()
        #[arg(
            value_name = "ARGS",
            trailing_var_arg = true,
            allow_hyphen_values = true
        )]
        args: Vec<String>,
    },

    /// Format source files in place (or check formatting)
    Fmt {
        /// Files or directories to format (default: current directory)
        #[arg(value_name = "PATH")]
        paths: Vec<PathBuf>,
        /// Report which files would change and exit non-zero; write nothing
        #[arg(long)]
        check: bool,
    },

    /// Discover and run test blocks
    Test {
        /// File or directory to test (default: current directory)
        #[arg(value_name = "PATH")]
        path: Option<PathBuf>,
        /// Report line coverage
        #[arg(long)]
        coverage: bool,
        /// Report coverage and list each file's uncovered lines (implies --coverage)
        #[arg(long)]
        coverage_lines: bool,
    },

    /// Compile a program to a self-contained executable
    Build {
        /// Program to compile
        #[arg(value_name = "FILE")]
        file: PathBuf,
        /// Output path (default: the program's file stem)
        #[arg(short, long, value_name = "OUT")]
        output: Option<PathBuf>,
    },

    /// Inspect a built binary: embedded source, manifest, capabilities, provenance
    Inspect {
        /// The built binary to inspect
        #[arg(value_name = "BINARY")]
        binary: PathBuf,
        /// Print the embedded source
        #[arg(long)]
        source: bool,
        /// Print the embedded olang.toml manifest
        #[arg(long)]
        manifest: bool,
        /// Print the embedded olang.lock lockfile
        #[arg(long)]
        lockfile: bool,
        /// Print the resolved capability grant
        #[arg(long)]
        caps: bool,
        /// Re-verify every embedded checksum and exit non-zero on mismatch
        #[arg(long)]
        verify: bool,
        /// Prove the binary was built from the source tree in DIR
        #[arg(long, value_name = "DIR")]
        against: Option<PathBuf>,
        /// Extract embedded source/manifest/lockfile into DIR
        #[arg(short, long, value_name = "DIR")]
        output: Option<PathBuf>,
    },

    /// Show the capability grant a program or built binary carries
    Caps {
        /// A source file, package directory, or built binary (default: current directory)
        #[arg(value_name = "PATH")]
        path: Option<PathBuf>,
    },

    /// Run a program and record its nondeterministic inputs to a .olt trace
    Record {
        /// Program to run and record
        #[arg(value_name = "FILE")]
        file: PathBuf,
        /// Trace to write (default: the program's path with a .olt extension)
        #[arg(short, long, value_name = "TRACE.olt")]
        output: Option<PathBuf>,
        /// Arguments passed to the program
        #[arg(
            value_name = "ARGS",
            trailing_var_arg = true,
            allow_hyphen_values = true
        )]
        args: Vec<String>,
    },

    /// Replay a recorded .olt timeline bit-for-bit
    Replay {
        /// The recorded trace to replay
        #[arg(value_name = "TRACE.olt")]
        trace: PathBuf,
        /// Arguments passed to the replayed program
        #[arg(
            value_name = "ARGS",
            trailing_var_arg = true,
            allow_hyphen_values = true
        )]
        args: Vec<String>,
    },

    /// Generate HTML or Markdown API documentation
    Doc {
        /// Files or directories to document (default: current directory)
        #[arg(value_name = "PATH")]
        paths: Vec<PathBuf>,
        /// Output path (default: doc.html)
        #[arg(short, long, value_name = "OUT")]
        output: Option<PathBuf>,
        /// Emit Markdown instead of HTML
        #[arg(long)]
        markdown: bool,
    },

    /// Run benchmarks
    // The runner parses its own flags out of the forwarded arguments, so
    // clap knows nothing about them and prints no options. Spelling them
    // here keeps `--help` from being the one place they are invisible.
    #[command(long_about = "Run benchmarks\n\n\
        Flags are parsed by the benchmark runner itself, so they do not\n\
        appear under Options below:\n\n\
        \x20 --runs N            timed runs per benchmark (default 7)\n\
        \x20 --save FILE         store the medians as a baseline\n\
        \x20 --against FILE      compare against a saved baseline\n\
        \x20 --fail-on-regress   exit non-zero if anything got slower")]
    /// Show a file after macro expansion (docs/macros.md)
    ///
    /// Prints the program as the interpreter will actually run it: every
    /// `@` site replaced by what its `meta fn` returned, and the meta fns
    /// themselves blanked out. A macro-free file prints unchanged.
    Expand {
        /// The .ol file to expand
        file: PathBuf,
        /// Show only the lines that expansion changed, as removed/added
        /// hunks, instead of the whole expanded program
        #[arg(long)]
        diff: bool,
    },

    Bench {
        /// Arguments forwarded to the benchmark runner
        #[arg(
            value_name = "ARGS",
            trailing_var_arg = true,
            allow_hyphen_values = true
        )]
        args: Vec<String>,
    },

    /// Start the language server (LSP over stdio)
    Lsp {
        /// Accepted for editor-client compatibility: vscode-languageclient
        /// and other LSP clients append `--stdio` by convention. Stdio is
        /// the only transport, so the flag changes nothing — but rejecting
        /// it made the server exit before the first protocol byte, which
        /// is why the extension never worked: clap killed `olang lsp
        /// --stdio` with "unexpected argument" on every start.
        #[arg(long)]
        stdio: bool,
    },

    /// Run a program file — the file-first form `olang <file> [args]`
    #[command(external_subcommand)]
    External(Vec<String>),
}

/// The interpreter recurses on the host stack, and its documented call-depth
/// limit (1000) needs more than the default 8 MB main-thread stack — without
/// this, a program recursing ~700 deep aborted the process instead of
/// reporting "Maximum call depth exceeded".
const INTERPRETER_STACK_SIZE: usize = 256 * 1024 * 1024;

fn main() {
    let exit_code = std::thread::Builder::new()
        // Named, because it is the thread every program starts on and
        // diagnostics that name a thread (cell confinement) should call it
        // what the user would call it.
        .name("main".to_string())
        .stack_size(INTERPRETER_STACK_SIZE)
        .spawn(run)
        .expect("failed to spawn interpreter thread")
        .join()
        .unwrap_or(1);
    process::exit(exit_code);
}

fn run() -> i32 {
    // A binary produced by `olang build` carries its program appended
    // after the runtime. Run that and nothing else — checked before any
    // CLI parsing, so the bundled tool's own arguments reach it intact.
    if let Some(bundle) = embedded_program() {
        let logger = init_logger();
        let _ = initialize_parallelization(None);
        set_parallel_threshold(10_000);
        miette::set_panic_hook();
        return run_embedded(bundle, logger);
    }

    let mut cli = Cli::parse();

    // Initialize logger
    let logger = init_logger();

    // Initialize parallelization early for optimal performance
    match initialize_parallelization(cli.ovm_parallelism) {
        Err(e) => {
            if cli.verbose {
                logger.warn(
                    "main",
                    &format!("Failed to initialize parallel processing: {}", e),
                );
            }
        }
        _ => {
            if cli.verbose {
                logger.info(
            "main",
            &format!(
                "Multi-threading enabled: {} CPU cores detected, using aggressive parallelization",
                num_cpus::get()
            ),
        );
            }
        }
    }

    // Allow enabling/disabling parallel at runtime
    if cli.enable_parallel {
        // FIXME: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::set_var("OVM_ENABLE_PARALLEL", "1") };
    }
    if let Some(n) = cli.ovm_parallelism {
        // FIXME: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::set_var("OVM_PARALLELISM", n.to_string()) };
    }

    // Parallelize only where the work plausibly outweighs thread overhead.
    // Small lists are always cheaper sequentially.
    set_parallel_threshold(10_000);

    if cli.verbose {
        logger.info(
            "main",
            &format!(
                "Parallelization enabled for lists of 10,000+ items across {} cores",
                num_cpus::get()
            ),
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

    // Dispatch. The file-first form `olang <file> [args]` arrives as
    // `Commands::External` (any leading word that is neither a known command
    // nor a flag); every other command is named. No command — or `olang
    // repl` — starts the REPL. Taking `command` out leaves the run options on
    // `cli` for the runner to read.
    let command = cli.command.take();
    match command {
        None | Some(Commands::Repl) => {
            if let Err(e) = start_repl(cli.verbose, cli.no_ovm, logger) {
                logger.error("main", &format!("REPL error: {}", e));
                return 1;
            }
            0
        }

        Some(Commands::Run { file, args }) => {
            let record = cli.record.clone();
            run_program(&cli, file, args, record, logger)
        }

        Some(Commands::External(mut parts)) => {
            // The catch-all arm: `olang report.ol a b` -> ["report.ol","a","b"].
            // clap guarantees at least one element for an external subcommand.
            let file = PathBuf::from(parts.remove(0));
            let record = cli.record.clone();
            run_program(&cli, file, parts, record, logger)
        }

        Some(Commands::Test {
            path,
            coverage,
            coverage_lines,
        }) => {
            // Files under the runner get a bare argv — a program that branches
            // on os.args() takes its no-argument path.
            olang::stdlib::os::set_script_args(vec!["olang-test".to_string()]);
            let target = path.unwrap_or_else(|| PathBuf::from("."));
            olang::tools::test_runner::run(&target, coverage || coverage_lines, coverage_lines)
        }

        Some(Commands::Profile {
            file,
            interval,
            top,
            args,
        }) => {
            // The profiled run is an ordinary run: same tiers, same
            // capability grant, same argv. Only the shadow-stack flag
            // differs, so what the profile measures is what `olang run`
            // would have done.
            let record = cli.record.clone();
            let session = olang::profile::start(interval);
            let started = std::time::Instant::now();
            let code = run_program(&cli, file, args, record, logger);
            let elapsed = started.elapsed();
            println!("{}", session.finish(elapsed, top));
            code
        }

        Some(Commands::Check { mut paths, rules }) => {
            if paths.is_empty() {
                paths.push(PathBuf::from("."));
            }
            olang::tools::check::run(&paths, rules.as_deref())
        }

        Some(Commands::Fmt { mut paths, check }) => {
            if paths.is_empty() {
                paths.push(PathBuf::from("."));
            }
            olang::tools::fmt::run(&paths, check)
        }

        Some(Commands::Build { file, output }) => {
            match build_executable(&file, output.as_deref()) {
                Ok(out) => {
                    println!("built {}", out);
                    0
                }
                Err(e) => {
                    eprintln!("olang build: {}", e);
                    1
                }
            }
        }

        Some(Commands::Inspect {
            binary,
            source,
            manifest,
            lockfile,
            caps,
            verify,
            against,
            output,
        }) => inspect_binary(&InspectArgs {
            binary,
            source,
            manifest,
            lockfile,
            caps,
            verify,
            against,
            output,
        }),

        Some(Commands::Caps { path }) => {
            show_caps(path.unwrap_or_else(|| PathBuf::from(".")).as_path())
        }

        Some(Commands::Record { file, output, args }) => {
            // The trace path: -o if given, else the program's path with a
            // .olt extension, so the trace lands next to the program it
            // records rather than in the current directory.
            let trace = output
                .unwrap_or_else(|| file.with_extension("olt"))
                .to_string_lossy()
                .to_string();
            eprintln!("olang record: writing trace to {trace}");
            run_program(&cli, file, args, Some(trace), logger)
        }

        Some(Commands::Replay { trace, args }) => run_replay(&trace, &args, logger),

        Some(Commands::Doc {
            mut paths,
            output,
            markdown,
        }) => {
            if paths.is_empty() {
                paths.push(PathBuf::from("."));
            }
            let output = output.unwrap_or_else(|| PathBuf::from("doc.html"));
            olang::tools::doc::run(&paths, &output, markdown)
        }

        Some(Commands::Expand { file, diff }) => {
            let code = match std::fs::read_to_string(&file) {
                // Resolve `use`-imported macro libraries against the file's
                // own directory, exactly as `olang run`, `olang check`, and
                // the LSP do — `olang expand` is the command whose whole
                // point is showing the same program those consumers see, so
                // it must not resolve differently just because the CWD is
                // elsewhere.
                Ok(source) => {
                    match olang::expand::expand_source_mapped_with_dir(&source, file.parent())
                        .map(|e| e.text)
                    {
                        Ok(expanded) => {
                            if diff {
                                print_expansion_diff(&source, &expanded);
                            } else {
                                print!("{}", expanded);
                            }
                            0
                        }
                        Err(message) => {
                            eprintln!("olang expand: {}", message);
                            1
                        }
                    }
                }
                Err(e) => {
                    eprintln!("olang expand: cannot read {}: {}", file.display(), e);
                    2
                }
            };
            std::process::exit(code)
        }
        Some(Commands::Bench { args }) => olang::tools::bench::run(&args),

        Some(Commands::Lsp { stdio: _ }) => match olang::tools::lsp::run() {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("language server error: {e}");
                1
            }
        },
    }
}

/// Run a program file with the given run options (`olang <file>` and `olang
/// run <file>` both land here). `args` is the program's argv after the file.
fn run_program(
    cli: &Cli,
    file_path: PathBuf,
    args: Vec<String>,
    record: Option<String>,
    logger: &Logger,
) -> i32 {
    // Watch mode: run the script in a child process (so os.exit and crashes
    // end the run, not the watcher) and rerun when any .ol file in the
    // script's directory changes.
    if cli.watch {
        return watch_loop(&file_path, cli.deny.as_deref(), &args);
    }
    // Program's argv: the script path, then everything after it. Read via
    // os.args() inside the program.
    let mut argv = vec![file_path.to_string_lossy().to_string()];
    argv.extend(args);
    olang::stdlib::os::set_script_args(argv);

    // Capability restriction for this run: --deny and OLANG_DENY intersect
    // (both can only remove). A typo in either refuses to run rather than
    // running wide open.
    let deny = {
        let flag = match cli.deny.as_deref().map(olang::caps::parse_deny) {
            Some(Ok(c)) => Some(c),
            Some(Err(e)) => {
                eprintln!("olang: {}", e);
                return 2;
            }
            None => None,
        };
        let env = match std::env::var("OLANG_DENY")
            .ok()
            .as_deref()
            .map(olang::caps::parse_deny)
        {
            Some(Ok(c)) => Some(c),
            Some(Err(e)) => {
                eprintln!("olang (OLANG_DENY): {}", e);
                return 2;
            }
            None => None,
        };
        match (flag, env) {
            (Some(a), Some(b)) => Some(a.intersect(b)),
            (a, b) => a.or(b),
        }
    };

    // Recording builds a timeline over the program source (from `--record`
    // or the `olang record` command).
    let timeline = match &record {
        Some(out) => match std::fs::read_to_string(&file_path) {
            Ok(src) => Some(olang::timeline::Timeline::record(
                PathBuf::from(out),
                file_path.to_string_lossy().to_string(),
                src.clone(),
                sha256_hex(src.as_bytes()),
            )),
            Err(e) => {
                eprintln!("olang --record: cannot read {}: {}", file_path.display(), e);
                return 1;
            }
        },
        None => None,
    };

    if let Err(e) = execute_file(
        &file_path,
        cli.verbose,
        cli.no_ovm,
        cli.ovm_stats,
        cli.ovm_tier,
        cli.max_depth,
        deny,
        timeline,
        if cli.trace_caps {
            Some(cli.write)
        } else {
            None
        },
        logger,
    ) {
        logger.error("main", &format!("Error executing file: {}", e));
        return 1;
    }
    0
}

/// `olang --watch script.ol` -- the edit-run loop. Each run is a child
/// process of this same binary (without --watch): a crash, a panic, or
/// os.exit ends the run, never the watcher. Changes are detected by
/// polling the newest mtime of every .ol file at or below the script's
/// directory -- no dependency, and a save mid-run queues an immediate
/// rerun. Ctrl+C stops watcher and child together (same process group).
fn watch_loop(script: &std::path::Path, deny: Option<&str>, script_args: &[String]) -> i32 {
    use std::time::{Duration, SystemTime};
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("--watch: cannot find own executable: {}", e);
            return 1;
        }
    };
    let dir = script
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."))
        .to_path_buf();
    let latest_mtime = || -> SystemTime {
        let mut newest = SystemTime::UNIX_EPOCH;
        for f in olang::tools::discover_ol_files(&dir) {
            if let Ok(meta) = std::fs::metadata(&f)
                && let Ok(m) = meta.modified()
                && m > newest
            {
                newest = m;
            }
        }
        newest
    };
    loop {
        let baseline = latest_mtime();
        eprintln!("-- watch: running {} --", script.display());
        let mut cmd = std::process::Command::new(&exe);
        if let Some(d) = deny {
            cmd.arg("--deny").arg(d);
        }
        let status = cmd.arg(script).args(script_args).status();
        match status {
            Ok(s) => eprintln!(
                "-- watch: exited {} -- waiting for changes (Ctrl+C quits) --",
                s.code()
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "by signal".to_string())
            ),
            Err(e) => {
                eprintln!("--watch: failed to run {}: {}", script.display(), e);
                return 1;
            }
        }
        while latest_mtime() <= baseline {
            std::thread::sleep(Duration::from_millis(200));
        }
    }
}

/// Enhanced error display for file execution
fn show_file_parse_error(
    error: &olang::parser::ParseError,
    file_path: &std::path::Path,
    source: &str,
) {
    eprintln!("\n{}", "═══ Parse Error ═══".bright_red().bold());
    eprintln!(
        "  {}: {}",
        "File".bright_blue().bold(),
        file_path.display().to_string().bright_white()
    );

    match error {
        olang::parser::ParseError::InvalidSyntaxWithPosition {
            message,
            line,
            column,
            snippet,
        } => {
            eprintln!(
                "  {}: {}",
                "Error".bright_red().bold(),
                message.bright_white()
            );
            eprintln!(
                "  {}: Line {}, Column {}",
                "Location".bright_yellow().bold(),
                line.to_string().bright_cyan(),
                column.to_string().bright_cyan()
            );

            if !snippet.trim().is_empty() {
                eprintln!("\n  {}", "Code Context:".bright_blue().bold());
                show_highlighted_snippet(snippet);
            }
        }
        olang::parser::ParseError::UnexpectedTokenWithPosition {
            token,
            line,
            column,
            snippet,
        } => {
            eprintln!(
                "  {}: Unexpected token '{}'",
                "Error".bright_red().bold(),
                token.bright_yellow().bold()
            );
            eprintln!(
                "  {}: Line {}, Column {}",
                "Location".bright_yellow().bold(),
                line.to_string().bright_cyan(),
                column.to_string().bright_cyan()
            );

            if !snippet.trim().is_empty() {
                eprintln!("\n  {}", "Code Context:".bright_blue().bold());
                show_highlighted_snippet(snippet);
            }
        }
        _ => {
            eprintln!("  {}: {}", "Error".bright_red().bold(), error);
        }
    }

    // Get suggestions
    let parser = OlangParser::new();
    let suggestions = parser.get_suggestions(error, source);

    if !suggestions.is_empty() {
        eprintln!("\n  {}", "Suggestions:".bright_cyan().bold());
        for suggestion in suggestions {
            show_suggestion(&suggestion);
        }
    }

    // Show help topics
    eprintln!("\n  {}", "Help:".bright_cyan().bold());
    eprintln!("    • Type {} for syntax help", "olang -h".bright_cyan());

    eprintln!();
}

/// Feature 9: Show classic interpreter errors with enhanced context and suggestions
fn show_classic_interpreter_error(
    error: &olang::interpreter::InterpreterError,
    file_path: &std::path::Path,
    interpreter: &olang::interpreter::Interpreter,
    location: Option<olang::ast::ErrorLocation>,
    source: &str,
) {
    eprintln!("\n{}", "═══ Execution Error ═══".bright_red().bold());

    let formatted_error = interpreter.format_error(error);

    if let Some(loc) = location {
        // Located: render the source line with a caret through miette so
        // the error points at where it happened, then the call stack.
        let offset = byte_offset_of(source, loc.line as usize, loc.column as usize);
        let mut diagnostic = miette::MietteDiagnostic::new(formatted_error.clone()).with_label(
            miette::LabeledSpan::at_offset(offset, "error occurred here"),
        );
        if let Some(hint) = &loc.hint {
            diagnostic = diagnostic.with_help(hint.clone());
        }
        let report = miette::Report::new(diagnostic).with_source_code(miette::NamedSource::new(
            file_path.display().to_string(),
            source.to_string(),
        ));
        eprintln!("{:?}", report);
        if !loc.call_stack.is_empty() {
            eprintln!("  {}", "Call stack (outermost first):".bright_blue().bold());
            for name in &loc.call_stack {
                eprintln!("    → {}", name.bright_white());
            }
            eprintln!();
        }
    } else {
        eprintln!(
            "  {}: {}",
            "File".bright_blue().bold(),
            file_path.display().to_string().bright_white()
        );
        eprintln!("\n{}", formatted_error);
        eprintln!();
    }
}

/// Byte offset of a 1-based (line, column) position in `source` — what
/// miette's span labels want. Clamped to the source length.
fn byte_offset_of(source: &str, line: usize, column: usize) -> usize {
    let mut offset = 0usize;
    for (i, l) in source.split('\n').enumerate() {
        if i + 1 == line {
            // Column is 1-based and counted in characters; walk the line.
            let col_bytes: usize = l
                .chars()
                .take(column.saturating_sub(1))
                .map(|c| c.len_utf8())
                .sum();
            return (offset + col_bytes).min(source.len());
        }
        offset += l.len() + 1;
    }
    offset.min(source.len())
}

/// Feature 9: Show interpreter errors with enhanced context and suggestions
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
                println!(
                    "    {}{}│ {}",
                    line_num.bright_black(),
                    " ".repeat(4 - line_num.len().min(4)),
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

                println!(
                    "    {}{}│ {}",
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
                if let Some(&next_ch) = chars.peek()
                    && ((next_ch == '=' && matches!(ch, '=' | '!' | '<' | '>'))
                        || (ch == '&' && next_ch == '&')
                        || (ch == '|' && next_ch == '|'))
                {
                    op.push(chars.next().unwrap());
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
        "let" | "fn" | "if" | "else" | "match" | "for" | "while" | "loop" | "break"
        | "continue" | "true" | "false" | "import" | "export" | "type" | "async" | "await"
        | "try" | "catch" => {
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
        SuggestionSeverity::Error => "ERROR",
        SuggestionSeverity::Warning => "WARNING",
        SuggestionSeverity::Hint => "HINT",
        SuggestionSeverity::Info => "INFO",
    };

    let severity_color = match suggestion.severity {
        SuggestionSeverity::Error => "red",
        SuggestionSeverity::Warning => "yellow",
        SuggestionSeverity::Hint => "cyan",
        SuggestionSeverity::Info => "blue",
    };

    println!(
        "    {} {}",
        severity_icon,
        suggestion.message.color(severity_color).bold()
    );

    if let Some(fix) = &suggestion.fix {
        println!("      {}: {}", "Fix".bright_green().bold(), fix);
    }

    if let Some(help) = &suggestion.help {
        println!("      {}: {}", "Help".bright_blue().bold(), help);
    }
}

/// Trailer marking an `olang build` executable. The program is appended
/// to the runtime as `[source bytes][source len: u64 LE][MAGIC: 8]`, so
/// a build never touches a linker — a byte copy and an append suffice.
/// Rung A bundle: raw olang source appended, parsed at startup.
const OLANG_BUNDLE_MAGIC: &[u8; 8] = b"oLaNgBnd";
/// Rung B bundle: the *parsed* AST appended alongside the source, so a
/// built tool deserializes its program at startup instead of re-parsing it
/// (deserialize is ~20× faster than the parser for a real program). The
/// source travels too, only so runtime error snippets still render.
const OLANG_AST_MAGIC: &[u8; 8] = b"oLaNgAsT";
/// Rung "transparent binary": [source][ast][meta][3 x u64 lens][magic].
const OLANG_META_MAGIC: &[u8; 8] = b"oLaNgMeT";
/// The highest meta format this build knows how to interpret. A bundle
/// claiming more is refused rather than guessed at: `format` selects
/// which bytes the digest covers, so reading a future layout under
/// today's rules would print "verified" about a check that never
/// examined what the newer format binds.
const OLANG_META_FORMAT: u32 = 3;

/// The transparency record a built binary carries alongside its program:
/// enough for `olang inspect` to reproduce, verify, and reason about the
/// artifact without any external context.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
struct BundleMeta {
    /// Meta format version, for forward compatibility.
    format: u32,
    /// The olang that built this binary.
    olang_version: String,
    /// The path the source was built from (basename context only).
    source_path: String,
    /// sha256 (hex) of the embedded source bytes alone. Kept for a
    /// human-legible per-file checksum and for reading pre-`digest` bundles.
    sha256: String,
    /// sha256 (hex) over the *whole* transparency payload. Format 3:
    /// source ‖ AST ‖ manifest ‖ lockfile — binds the executed AST, so a
    /// swapped program is caught. Format 2: source ‖ manifest ‖ lockfile
    /// (no AST; verified with `payload_digest_v2`). Empty on pre-digest
    /// bundles (fall back to `sha256`). The `format` field selects which.
    #[serde(default)]
    digest: String,
    /// The package's olang.toml, verbatim, when the source lived in one.
    #[serde(default)]
    manifest: Option<String>,
    /// The package's olang.lock, verbatim, when present.
    #[serde(default)]
    lockfile: Option<String>,
    /// The operating system the binary was built on (`std::env::consts::OS`,
    /// e.g. "macos", "linux"). Informational provenance — like
    /// `olang_version`, it is not part of the integrity digest. Empty when
    /// read from a binary built before this field existed. A native `olang
    /// build` binary is platform-specific; this tells you which platform if
    /// it turns up on another machine.
    #[serde(default)]
    built_os: String,
    /// The CPU architecture the binary was built on
    /// (`std::env::consts::ARCH`, e.g. "aarch64", "x86_64").
    #[serde(default)]
    built_arch: String,
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    format!("{:x}", h.finalize())
}

/// Length-framed sha256 over a sequence of byte parts: each part's length
/// (u64 LE) then its bytes, so no concatenation boundary is ambiguous and
/// an empty part hashes distinctly from an absent one.
fn digest_hex(parts: &[&[u8]]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    for part in parts {
        h.update((part.len() as u64).to_le_bytes());
        h.update(part);
    }
    format!("{:x}", h.finalize())
}

/// The transparency digest (format 3): sha256 over the whole executed
/// payload — source, the compiled **AST** (the bytes that actually run),
/// the manifest, and the lockfile. Binding the AST is what makes
/// `inspect --verify` catch a swapped program: the source is only *shown*,
/// the AST is what *executes*, so a checksum over source alone left the
/// running code unauthenticated.
fn payload_digest(
    source: &[u8],
    ast: &[u8],
    manifest: Option<&str>,
    lockfile: Option<&str>,
) -> String {
    digest_hex(&[
        source,
        ast,
        manifest.unwrap_or("").as_bytes(),
        lockfile.unwrap_or("").as_bytes(),
    ])
}

/// The pre-0.59-hardening digest layout (source ‖ manifest ‖ lockfile, no
/// AST), kept so `--verify` still validates format-2 bundles built before
/// the AST was bound in. New builds are format 3.
fn payload_digest_v2(source: &[u8], manifest: Option<&str>, lockfile: Option<&str>) -> String {
    digest_hex(&[
        source,
        manifest.unwrap_or("").as_bytes(),
        lockfile.unwrap_or("").as_bytes(),
    ])
}

/// Does the embedded source parse to exactly the AST the binary runs? A
/// built binary executes its embedded AST, not its source, so this is the
/// check that makes the printed source honest: the program a reviewer reads
/// is the program that runs. A tamperer who swaps the AST but keeps the
/// clean source fails here — forcing their code into the visible source.
fn source_matches_ast(source: &str, program: &olang::ast::Program) -> bool {
    matches!(OlangParser::new().parse(source), Ok(reparsed) if &reparsed == program)
}

/// A program bundled into an `olang build` executable.
enum Bundle {
    /// Rung A: just the source (older bundles, or a fallback).
    Source(String),
    /// Rung B: the pre-parsed AST, with the source kept for error rendering.
    /// `ast_bytes` is the raw on-disk AST JSON — retained (not re-serialized)
    /// so `inspect --verify` can hash exactly what executes; a re-serialize
    /// could differ byte-for-byte for the same program (serde is not
    /// canonical), which would make the digest lie.
    Ast {
        meta: Option<Box<BundleMeta>>,
        program: Box<olang::ast::Program>,
        source: String,
        ast_bytes: Vec<u8>,
    },
}

/// If this executable was produced by `olang build`, return the program
/// appended to it — the pre-parsed AST (rung B) when present, else the raw
/// source (rung A). `None` for the plain `olang` binary.
fn embedded_program() -> Option<Bundle> {
    let exe = std::env::current_exe().ok()?;
    read_bundle(&exe)
}

/// Read a bundled program from any file — the shared reader behind both
/// self-execution and `olang inspect <binary>`.
/// Is this a plausible sha256 hex digest?
fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Field-level validation of a bundle's transparency record.
///
/// `read_bundle` already checks the *frame* — lengths, overflow, bounds —
/// so the payload cannot drive a bad allocation. This checks the
/// *contents*, which the frame says nothing about. Returning None makes
/// the bundle unreadable, which is the correct outcome: a transparency
/// record that cannot be interpreted must not produce a verdict, because
/// the only verdict worse than "unverifiable" is a confident wrong one.
fn validate_meta(meta: &BundleMeta) -> Option<()> {
    // An unknown format is the important one. `format` decides which
    // bytes the digest covers, and `>= 3` would silently accept a
    // format 9 bundle as if it were 3.
    if meta.format == 0 || meta.format > OLANG_META_FORMAT {
        return None;
    }
    // Digests are checked by string comparison, so a non-hex value could
    // never match and would report a mismatch that is really a malformed
    // record. Empty is legitimate: pre-digest bundles carry none.
    if !meta.digest.is_empty() && !is_sha256_hex(&meta.digest) {
        return None;
    }
    if !meta.sha256.is_empty() && !is_sha256_hex(&meta.sha256) {
        return None;
    }
    Some(())
}

fn read_bundle(path: &std::path::Path) -> Option<Bundle> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).ok()?;
    let total = f.metadata().ok()?.len();
    if total < 16 {
        return None;
    }
    // The last 8 bytes are the format marker.
    f.seek(SeekFrom::End(-8)).ok()?;
    let mut magic = [0u8; 8];
    f.read_exact(&mut magic).ok()?;

    // Transparent binary: [source][ast][meta][src_len][ast_len][meta_len][MAGIC].
    if &magic == OLANG_META_MAGIC {
        if total < 32 {
            return None;
        }
        f.seek(SeekFrom::End(-32)).ok()?;
        let mut lens = [0u8; 24];
        f.read_exact(&mut lens).ok()?;
        let source_len = u64::from_le_bytes(lens[0..8].try_into().ok()?);
        let ast_len = u64::from_le_bytes(lens[8..16].try_into().ok()?);
        let meta_len = u64::from_le_bytes(lens[16..24].try_into().ok()?);
        // Checked arithmetic: the lengths are attacker-controlled on a
        // hostile file, and an overflowing sum could pass the bounds check
        // and then drive a multi-exabyte allocation below.
        let payload = source_len
            .checked_add(ast_len)
            .and_then(|s| s.checked_add(meta_len))?;
        if source_len == 0 || ast_len == 0 || meta_len == 0 || payload.checked_add(32)? > total {
            return None;
        }
        f.seek(SeekFrom::End(-(32 + payload as i64))).ok()?;
        let mut source_buf = vec![0u8; source_len as usize];
        f.read_exact(&mut source_buf).ok()?;
        let mut ast_buf = vec![0u8; ast_len as usize];
        f.read_exact(&mut ast_buf).ok()?;
        let mut meta_buf = vec![0u8; meta_len as usize];
        f.read_exact(&mut meta_buf).ok()?;
        let source = String::from_utf8(source_buf).ok()?;
        let program: olang::ast::Program = serde_json::from_slice(&ast_buf).ok()?;
        let meta: BundleMeta = serde_json::from_slice(&meta_buf).ok()?;
        // Validate the record before anything trusts a field of it. The
        // bytes came off disk and nothing has authenticated them yet:
        // the digest is checked *using* these fields, so a malformed
        // record must be refused here rather than steering the check.
        validate_meta(&meta)?;
        return Some(Bundle::Ast {
            meta: Some(Box::new(meta)),
            program: Box::new(program),
            source,
            ast_bytes: ast_buf,
        });
    }

    // Rung B: [source][ast_json][source_len u64][ast_len u64][AST_MAGIC].
    if &magic == OLANG_AST_MAGIC {
        if total < 24 {
            return None;
        }
        f.seek(SeekFrom::End(-24)).ok()?;
        let mut lens = [0u8; 16];
        f.read_exact(&mut lens).ok()?;
        let source_len = u64::from_le_bytes(lens[0..8].try_into().ok()?);
        let ast_len = u64::from_le_bytes(lens[8..16].try_into().ok()?);
        let footer_end = source_len
            .checked_add(ast_len)
            .and_then(|s| s.checked_add(24))?;
        if source_len == 0 || ast_len == 0 || footer_end > total {
            return None;
        }
        f.seek(SeekFrom::End(-(24 + ast_len as i64 + source_len as i64)))
            .ok()?;
        let mut source_buf = vec![0u8; source_len as usize];
        f.read_exact(&mut source_buf).ok()?;
        let mut ast_buf = vec![0u8; ast_len as usize];
        f.read_exact(&mut ast_buf).ok()?;
        let source = String::from_utf8(source_buf).ok()?;
        let program: olang::ast::Program = serde_json::from_slice(&ast_buf).ok()?;
        return Some(Bundle::Ast {
            meta: None,
            program: Box::new(program),
            source,
            ast_bytes: ast_buf,
        });
    }

    // Rung A: [source][source_len u64][BUNDLE_MAGIC].
    if &magic == OLANG_BUNDLE_MAGIC {
        f.seek(SeekFrom::End(-16)).ok()?;
        let mut footer = [0u8; 16];
        f.read_exact(&mut footer).ok()?;
        let src_len = u64::from_le_bytes(footer[0..8].try_into().ok()?);
        if src_len == 0 || src_len.checked_add(16)? > total {
            return None;
        }
        f.seek(SeekFrom::End(-(16 + src_len as i64))).ok()?;
        let mut buf = vec![0u8; src_len as usize];
        f.read_exact(&mut buf).ok()?;
        return Some(Bundle::Source(String::from_utf8(buf).ok()?));
    }

    None
}

/// Run a bundled program. Its argv is the whole process argv, so the
/// tool's own flags reach `os.args()` / `cli.args()` exactly as they
/// would for a normal script. A rung-B bundle runs its pre-parsed AST
/// directly; a rung-A bundle parses its source first.
fn run_embedded(bundle: Bundle, logger: &Logger) -> i32 {
    olang::stdlib::os::set_script_args(std::env::args().collect());
    let path = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("program"));
    // A built binary honors the capabilities its embedded manifest
    // declares (base grant; per-dependency attenuation is a project-run
    // concern — a bundle is a single source). OLANG_DENY restricts
    // further from the outside; a bundle's own argv belongs to the tool.
    let deny = std::env::var("OLANG_DENY")
        .ok()
        .and_then(|list| olang::caps::parse_deny(&list).ok());
    let result = match bundle {
        Bundle::Ast {
            meta,
            program,
            source,
            ..
        } => {
            let manifest_caps = meta
                .as_ref()
                .and_then(|m| m.manifest.as_deref())
                .and_then(|text| olang::pkg::manifest::Manifest::from_toml(text).ok())
                .and_then(|m| m.capabilities)
                .and_then(|c| c.base.resolve().ok());
            let effective = match (manifest_caps, deny) {
                (Some(a), Some(b)) => Some(a.intersect(b)),
                (Some(a), None) => Some(a),
                (None, d) => d,
            };
            execute_program(
                *program, &source, &path, false, false, false, None, None, effective, None, None,
                logger, None,
            )
        }
        Bundle::Source(source) => execute_source(
            &source, &path, false, false, false, None, None, deny, None, None, logger,
        ),
    };
    match result {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

/// `olang inspect <binary>` — read the transparency record out of a built
/// executable. An olang binary is open by construction: it carries its
/// exact source, its manifest and lockfile, and a checksum — this command
/// is the reader. Flags:
///   --source            print the embedded source
///   --manifest          print the embedded olang.toml
///   --lockfile          print the embedded olang.lock
///   --caps              print the resolved capability grant
///   --verify            recompute the payload digest; nonzero on mismatch
///   --against DIR       prove the binary was built from the source in DIR
///   -o DIR              extract source/manifest/lockfile into DIR
/// Typed arguments for `olang inspect` (mirrors `Commands::Inspect`).
struct InspectArgs {
    binary: PathBuf,
    source: bool,
    manifest: bool,
    lockfile: bool,
    caps: bool,
    verify: bool,
    against: Option<PathBuf>,
    output: Option<PathBuf>,
}

fn inspect_binary(args: &InspectArgs) -> i32 {
    let show_source = args.source;
    let show_manifest = args.manifest;
    let show_lockfile = args.lockfile;
    let show_caps = args.caps;
    let verify = args.verify;
    let out_dir = args.output.as_deref();
    let against = args.against.as_deref();
    let target = args.binary.to_string_lossy().to_string();
    let path = args.binary.as_path();
    let Some(bundle) = read_bundle(path) else {
        eprintln!(
            "olang inspect: {} carries no olang bundle (not built with `olang build`?)",
            target
        );
        return 1;
    };
    let (source, meta) = match &bundle {
        Bundle::Ast { meta, source, .. } => (source.clone(), meta.clone()),
        Bundle::Source(source) => (source.clone(), None),
    };
    // The executed program and its raw on-disk AST bytes, for the integrity
    // checks. A rung-A source bundle has neither (its source IS what runs).
    let (program, ast_bytes): (Option<&olang::ast::Program>, &[u8]) = match &bundle {
        Bundle::Ast {
            program, ast_bytes, ..
        } => (Some(program.as_ref()), ast_bytes.as_slice()),
        Bundle::Source(_) => (None, &[]),
    };

    // Focused outputs print raw and exit, so they compose with pipes.
    if show_source {
        print!("{}", source);
        return 0;
    }
    if show_manifest {
        match meta.as_ref().and_then(|m| m.manifest.as_ref()) {
            Some(text) => {
                print!("{}", text);
                return 0;
            }
            None => {
                eprintln!("olang inspect: no manifest embedded");
                return 1;
            }
        }
    }
    if show_lockfile {
        match meta.as_ref().and_then(|m| m.lockfile.as_ref()) {
            Some(text) => {
                print!("{}", text);
                return 0;
            }
            None => {
                eprintln!("olang inspect: no lockfile embedded");
                return 1;
            }
        }
    }

    let caps_config = meta
        .as_ref()
        .and_then(|m| m.manifest.as_deref())
        .and_then(|text| olang::pkg::manifest::Manifest::from_toml(text).ok())
        .and_then(|m| m.capabilities);
    if show_caps {
        match &caps_config {
            Some(config) => match config.base.resolve() {
                Ok(caps) => {
                    println!("{}", caps.summary());
                    for (name, spec) in &config.dependencies {
                        match spec.resolve() {
                            Ok(dep_caps) => println!(
                                "dependency {}: {}",
                                name,
                                dep_caps.intersect(caps).summary()
                            ),
                            Err(e) => println!("dependency {}: invalid ({})", name, e),
                        }
                    }
                    return 0;
                }
                Err(e) => {
                    eprintln!("olang inspect: {}", e);
                    return 1;
                }
            },
            None => {
                println!(
                    "{} (no manifest: full capability)",
                    olang::caps::Caps::default().summary()
                );
                return 0;
            }
        }
    }

    // `--against DIR`: does this binary come from *that* source tree?
    // `--verify` proves a binary is internally consistent; this proves it
    // is the binary a given checkout builds — the provenance question.
    // (For a git ref, check it out first, then point --against at it.)
    if let Some(dir) = against {
        return inspect_against(dir, &source, meta.as_deref(), program);
    }

    if let Some(dir) = out_dir {
        if let Err(e) = std::fs::create_dir_all(dir) {
            eprintln!("olang inspect: {}: {}", dir.display(), e);
            return 1;
        }
        let source_name = meta
            .as_ref()
            .map(|m| {
                std::path::Path::new(&m.source_path)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "program.ol".to_string())
            })
            .unwrap_or_else(|| "program.ol".to_string());
        let mut wrote = vec![source_name.clone()];
        if std::fs::write(dir.join(&source_name), &source).is_err() {
            eprintln!("olang inspect: cannot write {}", source_name);
            return 1;
        }
        if let Some(m) = &meta {
            if let Some(text) = &m.manifest {
                let _ = std::fs::write(dir.join("olang.toml"), text);
                wrote.push("olang.toml".into());
            }
            if let Some(text) = &m.lockfile {
                let _ = std::fs::write(dir.join("olang.lock"), text);
                wrote.push("olang.lock".into());
            }
        }
        println!("extracted {} into {}", wrote.join(", "), dir.display());
        return 0;
    }

    // The default: a human summary. --verify folds in as the verdict.
    println!("olang bundle: {}", target);
    match &meta {
        Some(m) => {
            println!("  built with:   olang {}", m.olang_version);
            println!("  built from:   {}", m.source_path);
            // Build host platform: a native binary only runs on a matching
            // OS/arch, so this flags a binary that turned up off-platform.
            // Empty on binaries built before the field existed.
            if !m.built_os.is_empty() || !m.built_arch.is_empty() {
                let os = if m.built_os.is_empty() {
                    "unknown"
                } else {
                    &m.built_os
                };
                let arch = if m.built_arch.is_empty() {
                    "unknown"
                } else {
                    &m.built_arch
                };
                let here = os == std::env::consts::OS && arch == std::env::consts::ARCH;
                println!(
                    "  built on:     {}/{}{}",
                    os,
                    arch,
                    if here {
                        "  [this platform]"
                    } else {
                        "  [foreign — native binaries are platform-specific]"
                    }
                );
            }
            println!(
                "  source:       {} bytes  (--source to print)",
                source.len()
            );
            let actual_src = sha256_hex(source.as_bytes());
            let src_ok = actual_src == m.sha256;
            println!(
                "  sha256:       {}  [{}]",
                m.sha256,
                if src_ok { "verified" } else { "MISMATCH" }
            );
            if !src_ok {
                println!("  actual:       {}", actual_src);
            }
            // The payload digest is the authoritative integrity check.
            // Format 3 binds the AST (what runs); format 2 covers only
            // source+manifest+lockfile. Pre-digest bundles carry none — the
            // source sha is the only verdict there.
            let digest_ok = if m.digest.is_empty() {
                true
            } else {
                let (actual, covers) = if m.format >= 3 {
                    (
                        payload_digest(
                            source.as_bytes(),
                            ast_bytes,
                            m.manifest.as_deref(),
                            m.lockfile.as_deref(),
                        ),
                        "source + AST + manifest + lockfile",
                    )
                } else {
                    (
                        payload_digest_v2(
                            source.as_bytes(),
                            m.manifest.as_deref(),
                            m.lockfile.as_deref(),
                        ),
                        "source + manifest + lockfile (format 2: AST unbound)",
                    )
                };
                let ok = actual == m.digest;
                println!(
                    "  digest:       {}  [{}]",
                    m.digest,
                    if ok { "verified" } else { "MISMATCH" }
                );
                if !ok {
                    println!("  actual:       {}", actual);
                    println!("  digest covers {}", covers);
                }
                ok
            };
            // Source faithfulness: the embedded source must parse to the AST
            // that runs. This is what makes `--source` honest — a binary
            // whose AST was swapped while the source was kept clean fails
            // here, even if the attacker recomputed the digest.
            let faithful = match program {
                Some(p) => {
                    let ok = source_matches_ast(&source, p);
                    println!(
                        "  program:      {}",
                        if ok {
                            "source parses to the embedded AST  [faithful]"
                        } else {
                            "source does NOT match the executed AST  [DIVERGES]"
                        }
                    );
                    ok
                }
                None => true,
            };
            let ok = src_ok && digest_ok && faithful;
            println!(
                "  manifest:     {}",
                if m.manifest.is_some() {
                    "embedded (--manifest to print)"
                } else {
                    "none"
                }
            );
            println!(
                "  lockfile:     {}",
                if m.lockfile.is_some() {
                    "embedded (--lockfile to print)"
                } else {
                    "none"
                }
            );
            match &caps_config {
                Some(config) => match config.base.resolve() {
                    Ok(caps) => println!("  capabilities: {}", caps.summary()),
                    Err(e) => println!("  capabilities: invalid ({})", e),
                },
                None => println!("  capabilities: full (no [capabilities] manifest)"),
            }
            if verify && !ok {
                return 1;
            }
        }
        None => {
            println!("  format:       pre-transparency bundle (rung A/B)");
            println!("  source:       {} bytes (--source to print)", source.len());
            println!("  sha256:       none recorded — rebuild with this olang to add one");
            if verify {
                eprintln!("olang inspect: nothing to verify in a pre-transparency bundle");
                return 1;
            }
        }
    }
    0
}

/// `olang inspect <binary> --against <dir>` — prove the binary was built
/// from the source tree in `dir`. Diffs the embedded source, manifest, and
/// lockfile against the files on disk and reports each. Exit 0 iff every
/// embedded artifact byte-matches its on-disk counterpart.
fn inspect_against(
    dir: &std::path::Path,
    source: &str,
    meta: Option<&BundleMeta>,
    program: Option<&olang::ast::Program>,
) -> i32 {
    if !dir.is_dir() {
        eprintln!(
            "olang inspect --against: {} is not a directory",
            dir.display()
        );
        return 2;
    }
    let Some(meta) = meta else {
        eprintln!(
            "olang inspect --against: pre-transparency bundle carries no source path — rebuild with this olang first"
        );
        return 1;
    };
    println!("olang inspect: comparing against {}", dir.display());

    // One artifact's verdict. `matched` is None when there is nothing to
    // compare (not embedded, or absent on both sides); Some(false) is a
    // real mismatch and fails the check.
    let mut all_ok = true;
    let mut any_compared = false;
    let mut compare = |label: &str, embedded: Option<&str>, path: std::path::PathBuf| {
        match embedded {
            None => {} // not carried by this bundle — nothing to assert
            Some(want) => match std::fs::read_to_string(&path) {
                Ok(have) if have == want => {
                    any_compared = true;
                    println!("  {:<12} match   ({})", label, path.display());
                }
                Ok(_) => {
                    any_compared = true;
                    all_ok = false;
                    println!("  {:<12} DIFFERS ({})", label, path.display());
                }
                Err(_) => {
                    any_compared = true;
                    all_ok = false;
                    println!("  {:<12} MISSING ({})", label, path.display());
                }
            },
        }
    };

    // The source lives at its recorded path relative to the checkout, with
    // a basename fallback — for a build invoked with an absolute path (a
    // relative join would ignore `dir` entirely) or from elsewhere in the
    // tree.
    let src_rel = std::path::Path::new(&meta.source_path);
    let basename = src_rel
        .file_name()
        .unwrap_or(std::ffi::OsStr::new("program.ol"));
    let src_path = {
        let at_rel = dir.join(src_rel);
        if !src_rel.is_absolute() && at_rel.is_file() {
            at_rel
        } else {
            dir.join(basename)
        }
    };
    compare("source", Some(source), src_path);
    compare(
        "olang.toml",
        meta.manifest.as_deref(),
        dir.join("olang.toml"),
    );
    compare(
        "olang.lock",
        meta.lockfile.as_deref(),
        dir.join("olang.lock"),
    );

    // Provenance is only meaningful if the binary actually runs the source
    // we just diffed: the embedded AST must be what that source parses to.
    // Without this, a matching source proves nothing — the executed AST
    // could have been swapped.
    if let Some(p) = program {
        any_compared = true;
        if source_matches_ast(source, p) {
            println!(
                "  {:<12} match   (executed AST == parse(source))",
                "program"
            );
        } else {
            all_ok = false;
            println!(
                "  {:<12} DIVERGES (binary runs an AST that is not this source)",
                "program"
            );
        }
    }

    if !any_compared {
        eprintln!(
            "olang inspect --against: nothing to compare — is {} the right source tree?",
            dir.display()
        );
        return 1;
    }
    if all_ok {
        println!("  → this binary matches the source tree");
        0
    } else {
        println!("  → this binary does NOT match the source tree");
        1
    }
}

/// `olang build <program.ol> [-o output]` — copy this runtime and append
/// the program, producing a self-contained executable with no external
/// olang install required. The source is parse-checked first, so a
/// broken program is never shipped. Bundles a single source file: a
/// program that `use`s local files should be a package built with all
/// its sources inlined, or restrict itself to stdlib and the embedded
/// packages (cli, term, ui, viz, dash, …), which travel in the runtime.
fn build_executable(
    source_path: &std::path::Path,
    output_arg: Option<&std::path::Path>,
) -> anyhow::Result<String> {
    let src = std::fs::read_to_string(source_path)
        .map_err(|e| anyhow::anyhow!("cannot read {}: {}", source_path.display(), e))?;
    // Parse-check up front (a broken program is never shipped) and keep the
    // AST: the built binary embeds it so startup skips the parser (rung B).
    let program = OlangParser::new()
        .parse(&src)
        .map_err(|e| anyhow::anyhow!("{} does not parse:\n{}", source_path.display(), e))?;
    let ast_json =
        serde_json::to_vec(&program).map_err(|e| anyhow::anyhow!("serialize AST: {}", e))?;

    let output = match output_arg {
        Some(p) => p.to_string_lossy().to_string(),
        None => source_path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "program".to_string()),
    };

    // The transparency record: checksum, provenance, and — when the
    // source lives in a package — its manifest and lockfile, verbatim.
    // Every built binary carries its own source and its own paper trail;
    // `olang inspect` reads them back out.
    let source_abs =
        std::fs::canonicalize(source_path).unwrap_or_else(|_| source_path.to_path_buf());
    let (manifest_text, lockfile_text) =
        match olang::pkg::manifest::Manifest::find_root(&source_abs) {
            Some(root) => (
                std::fs::read_to_string(root.join("olang.toml")).ok(),
                std::fs::read_to_string(root.join("olang.lock")).ok(),
            ),
            None => (None, None),
        };
    // A manifest with capabilities must parse — a build never ships a
    // grant it cannot enforce.
    if let Some(text) = &manifest_text {
        let parsed: olang::pkg::manifest::Manifest =
            olang::pkg::manifest::Manifest::from_toml(text)
                .map_err(|e| anyhow::anyhow!("olang.toml: {}", e))?;
        if let Some(config) = &parsed.capabilities {
            config
                .base
                .resolve()
                .map_err(|e| anyhow::anyhow!("olang.toml: {}", e))?;
        }
    }
    let meta = BundleMeta {
        format: 3,
        olang_version: olang::VERSION.to_string(),
        source_path: source_path.to_string_lossy().to_string(),
        sha256: sha256_hex(src.as_bytes()),
        // The digest binds the AST — the bytes that actually execute — so a
        // swapped program cannot pass `--verify` behind an intact source.
        digest: payload_digest(
            src.as_bytes(),
            &ast_json,
            manifest_text.as_deref(),
            lockfile_text.as_deref(),
        ),
        manifest: manifest_text,
        lockfile: lockfile_text,
        // The build host's platform: a native binary only runs on a matching
        // OS/arch, so `olang inspect` can flag one that wandered off-platform.
        built_os: std::env::consts::OS.to_string(),
        built_arch: std::env::consts::ARCH.to_string(),
    };
    let meta_json =
        serde_json::to_vec(&meta).map_err(|e| anyhow::anyhow!("serialize meta: {}", e))?;

    let exe = std::env::current_exe()?;
    std::fs::copy(&exe, &output)?;
    {
        use std::io::Write;
        // Transparent-binary footer:
        // [source][ast][meta][src_len][ast_len][meta_len][META_MAGIC].
        // The source travels as a feature, not a debugging convenience:
        // an olang binary is open by construction.
        let mut f = std::fs::OpenOptions::new().append(true).open(&output)?;
        f.write_all(src.as_bytes())?;
        f.write_all(&ast_json)?;
        f.write_all(&meta_json)?;
        f.write_all(&(src.len() as u64).to_le_bytes())?;
        f.write_all(&(ast_json.len() as u64).to_le_bytes())?;
        f.write_all(&(meta_json.len() as u64).to_le_bytes())?;
        f.write_all(OLANG_META_MAGIC)?;
        f.flush()?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&output)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&output, perms)?;
    }
    Ok(output)
}

/// `olang caps [path]` — show the capability grant a program is *declared*
/// to have, read from its `olang.toml` (base grant plus each dependency's
/// attenuation). This is the static counterpart to the dynamic
/// `--trace-caps` profiler: "what is this allowed to do" versus "what does
/// it actually use". `path` is a package directory or a file inside one
/// (default: the current directory); for a built binary it defers to
/// `inspect --caps`.
fn show_caps(path: &std::path::Path) -> i32 {
    // A built binary carries its own manifest — reuse the inspect reader.
    if path.is_file() && read_bundle(path).is_some() {
        return inspect_binary(&InspectArgs {
            binary: path.to_path_buf(),
            source: false,
            manifest: false,
            lockfile: false,
            caps: true,
            verify: false,
            against: None,
            output: None,
        });
    }

    let abs = std::fs::canonicalize(path)
        .unwrap_or_else(|_| std::env::current_dir().unwrap_or_default().join(path));
    let full = olang::caps::Caps::default().summary();
    let Some(root) = olang::pkg::manifest::Manifest::find_root(&abs) else {
        println!("{}  (no olang.toml — full capability)", full);
        return 0;
    };
    let manifest = match olang::pkg::manifest::Manifest::load(&root) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("olang caps: {}", e);
            return 1;
        }
    };
    let Some(config) = manifest.capabilities else {
        println!(
            "{}  (no [capabilities] in {} — full capability)",
            full,
            root.join("olang.toml").display()
        );
        return 0;
    };
    match config.base.resolve() {
        Ok(caps) => {
            println!("package {}: {}", manifest.package.name, caps.summary());
            for (name, spec) in &config.dependencies {
                match spec.resolve() {
                    Ok(dep) => {
                        println!("  dependency {}: {}", name, dep.intersect(caps).summary())
                    }
                    Err(e) => println!("  dependency {}: invalid ({})", name, e),
                }
            }
            0
        }
        Err(e) => {
            eprintln!("olang caps: {}", e);
            1
        }
    }
}

/// `--trace-caps --write`: fold the suggested least-privilege
/// `[capabilities]` block into the package's olang.toml. Safe by
/// construction: it never overwrites an existing `[capabilities]` block
/// (a hand-tuned grant is left alone), and it prints the block instead of
/// writing when the program is not in a package.
fn write_caps_manifest(
    script_path: &std::path::Path,
    used: &std::collections::BTreeSet<olang::caps::CapUse>,
) {
    let block = olang::caps::suggested_block(used);
    let Some(root) = olang::pkg::manifest::Manifest::find_root(script_path) else {
        eprintln!("olang --write: not in a package (no olang.toml found); block not written");
        return;
    };
    let toml_path = root.join("olang.toml");
    let existing = std::fs::read_to_string(&toml_path).unwrap_or_default();
    if existing
        .lines()
        .any(|l| l.trim_start().starts_with("[capabilities"))
    {
        eprintln!(
            "olang --write: {} already has a [capabilities] block; left unchanged",
            toml_path.display()
        );
        return;
    }
    // Append as a new top-level table — valid regardless of what precedes.
    let sep = if existing.is_empty() || existing.ends_with('\n') {
        ""
    } else {
        "\n"
    };
    let updated = format!("{}{}\n{}", existing, sep, block);
    match std::fs::write(&toml_path, updated) {
        Ok(()) => eprintln!("olang: wrote [capabilities] to {}", toml_path.display()),
        Err(e) => eprintln!(
            "olang --write: could not write {}: {}",
            toml_path.display(),
            e
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn execute_file(
    file_path: &PathBuf,
    verbose: bool,
    no_ovm: bool,
    ovm_stats: bool,
    ovm_tier: Option<u32>,
    max_depth: Option<usize>,
    deny: Option<olang::caps::Caps>,
    timeline: Option<olang::timeline::Timeline>,
    trace_caps: Option<bool>,
    logger: &Logger,
) -> anyhow::Result<()> {
    let source = std::fs::read_to_string(file_path)
        .map_err(|e| anyhow::anyhow!("cannot read '{}': {}", file_path.display(), e))?;
    execute_source(
        &source, file_path, verbose, no_ovm, ovm_stats, ovm_tier, max_depth, deny, timeline,
        trace_caps, logger,
    )
}

/// Print only what expansion changed: consecutive differing lines as one
/// hunk, original lines prefixed `-`, expanded lines `+`. Line-based and
/// deliberately simple — expansion preserves line count everywhere except
/// decorator splices and multi-line expression output, so hunks stay
/// small and aligned in practice.
fn print_expansion_diff(original: &str, expanded: &str) {
    let a: Vec<&str> = original.lines().collect();
    let b: Vec<&str> = expanded.lines().collect();
    let n = a.len().max(b.len());
    let mut i = 0;
    while i < n {
        if a.get(i) == b.get(i) {
            i += 1;
            continue;
        }
        let start = i;
        while i < n && a.get(i) != b.get(i) {
            i += 1;
        }
        println!("@@ line {} @@", start + 1);
        for line in a.iter().take(i.min(a.len())).skip(start) {
            println!("- {}", line);
        }
        for line in b.iter().take(i.min(b.len())).skip(start) {
            println!("+ {}", line);
        }
    }
}

/// Run a program from source already in hand (a file's contents, or the
/// program bundled into an `olang build` executable). `file_path` names
/// it for error messages and cwd-relative module resolution.
#[allow(clippy::too_many_arguments)]
fn execute_source(
    source: &str,
    file_path: &PathBuf,
    verbose: bool,
    no_ovm: bool,
    ovm_stats: bool,
    ovm_tier: Option<u32>,
    max_depth: Option<usize>,
    deny: Option<olang::caps::Caps>,
    timeline: Option<olang::timeline::Timeline>,
    trace_caps: Option<bool>,
    logger: &Logger,
) -> anyhow::Result<()> {
    // Macro-bearing programs run as their *expanded* text, so every span
    // a runtime error carries points into text we can actually show.
    // Expand here, hand the expanded source to both the parser and the
    // error display, and say so once — otherwise an error inside
    // generated code would print context from the file on disk with the
    // wrong lines under it.
    let original_source = source;
    let expanded_holder;
    let mut line_map: Option<Vec<olang::expand::LineOrigin>> = None;
    let mut did_expand = false;
    let source = if source.contains('@') || olang::expand::has_meta_fn_token(source) {
        match olang::expand::expand_source_mapped_with_dir(source, file_path.parent()) {
            Ok(expansion) if expansion.text != source => {
                did_expand = true;
                line_map = Some(expansion.line_origins);
                expanded_holder = expansion.text;
                expanded_holder.as_str()
            }
            Ok(same) => {
                expanded_holder = same.text;
                expanded_holder.as_str()
            }
            Err(message) => {
                eprintln!("olang: {}", message);
                return Err(anyhow::anyhow!("Macro expansion failed"));
            }
        }
    } else {
        source
    };
    let program = match OlangParser::new().parse(source) {
        Ok(program) => program,
        Err(e) => {
            show_file_parse_error(&e, file_path, source);
            if did_expand {
                eprintln!(
                    "note: this program contains macros — line numbers and code context \
                     refer to the expanded program (`olang expand {}`)",
                    file_path.display()
                );
            }
            return Err(anyhow::anyhow!("Parse failed"));
        }
    };
    // Runtime errors translate through the line map back into the file
    // the author is looking at, so the note is only needed on the parse
    // path above, where no map exists yet.
    let _ = did_expand;
    execute_program(
        program,
        source,
        file_path,
        verbose,
        no_ovm,
        ovm_stats,
        ovm_tier,
        max_depth,
        deny,
        timeline,
        trace_caps,
        logger,
        line_map.as_deref().map(|m| (original_source, m)),
    )
}

/// Run an already-parsed program. `source` is retained only for error
/// snippets (parse is already done). This is the shared tail of running a
/// file and running a bundle whose AST was embedded by `olang build`, so a
/// built tool never re-parses at startup.
#[allow(clippy::too_many_arguments)]
fn execute_program(
    program: olang::ast::Program,
    source: &str,
    file_path: &PathBuf,
    verbose: bool,
    no_ovm: bool,
    ovm_stats: bool,
    ovm_tier: Option<u32>,
    max_depth: Option<usize>,
    deny: Option<olang::caps::Caps>,
    timeline: Option<olang::timeline::Timeline>,
    trace_caps: Option<bool>,
    logger: &Logger,
    // When the program ran as expanded macro output: the ORIGINAL source
    // and the expansion's line map, so a runtime error can point into
    // the file the author wrote instead of the text the runtime ran.
    expansion: Option<(&str, &[olang::expand::LineOrigin])>,
) -> anyhow::Result<()> {
    // Get absolute path for module resolution
    let absolute_path = if file_path.is_absolute() {
        file_path.clone()
    } else {
        std::env::current_dir().unwrap_or_default().join(file_path)
    };

    // One execution model: the interpreter with the bytecode tier enabled,
    // promoting eligible functions on their first call. --no-ovm disables
    // the tier for a pure tree-walk (debugging / semantics reference);
    // --ovm-tier=N raises the promotion threshold.
    let mut interpreter = olang::interpreter::Interpreter::new();

    if !no_ovm {
        let threshold = ovm_tier.unwrap_or(1);
        interpreter.enable_bytecode_tier(threshold, verbose);
    }

    if let Some(depth) = max_depth {
        interpreter.set_max_call_depth(depth);
    }

    // Set file context for proper module resolution
    interpreter.set_current_file(&absolute_path);

    let mut caps_installed = false;
    // If the file lives in a package (an olang.toml is found by walking up),
    // resolve its dependencies and hand the interpreter the dependency map so
    // `use <dep>` paths resolve. Path and cached-git deps are cheap; a missing
    // dependency surfaces when the `use` is evaluated, not here.
    if let Some(root) = olang::pkg::manifest::Manifest::find_root(&absolute_path) {
        let opts = olang::pkg::InstallOptions {
            registry: std::env::var("OLANG_REGISTRY")
                .ok()
                .map(std::path::PathBuf::from),
            ..Default::default()
        };
        match olang::pkg::install(&root, &opts) {
            Ok(map) => {
                let mut map: std::collections::HashMap<_, _> = map.into_iter().collect();
                // A package is referable by its own name from within itself,
                // so a single-file package can `use <own_name> { ... }`.
                let manifest = olang::pkg::manifest::Manifest::load(&root).ok();
                if let Some(m) = &manifest {
                    map.entry(m.package.name.clone())
                        .or_insert_with(|| root.clone());
                }
                // Capabilities: the manifest's grant, attenuated per
                // dependency, intersected with any --deny/OLANG_DENY.
                // A bad capabilities block refuses to run — an
                // unenforceable grant must never silently run wide open.
                let config = manifest.as_ref().and_then(|m| m.capabilities.clone());
                if let Some(config) = config {
                    match olang::caps::CapTable::build(&config, &map) {
                        Ok(mut table) => {
                            if let Some(d) = deny {
                                table.app = table.app.intersect(d);
                                for entry in &mut table.deps {
                                    entry.2 = entry.2.intersect(d);
                                }
                            }
                            interpreter.set_capabilities(table);
                            caps_installed = true;
                        }
                        Err(e) => return Err(anyhow::anyhow!("{}", e)),
                    }
                }
                interpreter.set_dependency_map(map);
            }
            Err(e) => {
                if verbose {
                    logger.warn("main", &format!("package resolution: {}", e));
                }
            }
        }
    }

    // A --deny/OLANG_DENY restriction applies to every run, package or
    // not — a bare script gets a deny-only table.
    if !caps_installed && let Some(d) = deny {
        interpreter.set_capabilities(olang::caps::CapTable {
            app: d,
            deps: Vec::new(),
        });
    }

    // The Open Timeline: record or replay this run's nondeterministic
    // inputs. Attaching a timeline forces the interpreter tier.
    let recording = timeline
        .as_ref()
        .map(|t| t.mode() == olang::timeline::Mode::Record)
        .unwrap_or(false);
    if let Some(t) = timeline {
        interpreter.set_timeline(t);
    }

    // --trace-caps: record which capabilities the program exercises, so the
    // author can write a least-privilege manifest instead of guessing. Like
    // the gate, it observes at the dispatch choke point (interpreter tier).
    if trace_caps.is_some() {
        interpreter.enable_caps_trace();
    }

    // Whatever happens, a recorded trace is written — a crashed run is
    // exactly the run you want to replay.
    let write_trace = |interp: &mut olang::interpreter::Interpreter, logger: &Logger| {
        if let Some(t) = interp.take_timeline() {
            match t.finish() {
                Ok(Some(path)) => eprintln!(
                    "olang: recorded {} event(s) to {}",
                    t.total_events(),
                    path.display()
                ),
                Ok(None) => {}
                Err(e) => logger.warn("main", &format!("could not write trace: {}", e)),
            }
        }
    };

    // Print the capability profile after the run — a crashed run still
    // reports what it touched before it died. With --write, also fold the
    // suggested block into the package's olang.toml.
    let write_manifest = trace_caps == Some(true);
    let report_caps = |interp: &mut olang::interpreter::Interpreter| {
        if let Some(used) = interp.take_caps_trace() {
            print!("{}", olang::caps::trace_report(&used));
            if write_manifest {
                write_caps_manifest(&absolute_path, &used);
            }
        }
    };

    match interpreter.eval_program(program) {
        Ok(result) => {
            if verbose {
                logger.info("main", &format!("Result: {:?}", result));
            }
            report_caps(&mut interpreter);
            if ovm_stats {
                match interpreter.bytecode_tier_stats() {
                    Some(tier) => println!(
                        "Bytecode tier: {} promoted, {} rejected, {} bytecode calls, {} instructions, {} native calls",
                        tier.promoted,
                        tier.rejected,
                        tier.bytecode_calls,
                        tier.instructions_executed,
                        tier.jit_native_calls
                    ),
                    None => println!("Bytecode tier: disabled (--no-ovm)"),
                }
            }
            if recording {
                write_trace(&mut interpreter, logger);
            }
            Ok(())
        }
        Err(e) => {
            if recording {
                write_trace(&mut interpreter, logger);
            }
            report_caps(&mut interpreter);
            let mut location = interpreter.take_error_location();
            let mut display_source = source;
            // Source-map the error through the expansion, when there is
            // one: a line of untouched source points at its original
            // line (identical content, exact column); a generated line
            // points at the @ site that produced it, with the macro
            // named and `olang expand` offered for the generated text.
            if let (Some((original, map)), Some(loc)) = (expansion, location.as_mut()) {
                match map.get(loc.line as usize - 1) {
                    Some(olang::expand::LineOrigin::Original(m)) => {
                        loc.line = *m as u32;
                        display_source = original;
                    }
                    Some(olang::expand::LineOrigin::Generated {
                        macro_name,
                        site_line,
                    }) => {
                        let expanded_line = loc.line;
                        loc.line = *site_line as u32;
                        loc.column = 1;
                        let note = format!(
                            "the error is inside code generated by @{} at this site \
                             (expanded line {} — run `olang expand {}` to see it)",
                            macro_name,
                            expanded_line,
                            file_path.display()
                        );
                        loc.hint = Some(match loc.hint.take() {
                            Some(h) => format!("{note}\n{h}"),
                            None => note,
                        });
                        display_source = original;
                    }
                    None => {}
                }
            }
            show_classic_interpreter_error(&e, file_path, &interpreter, location, display_source);
            Err(anyhow::anyhow!("Execution failed"))
        }
    }
}

/// `olang replay <trace.olt>` — re-run the program embedded in a trace,
/// serving every recorded nondeterministic call from the log. A clean
/// finish means the run was fully determined by the recorded inputs; a
/// divergence means the program changed or has uncaptured nondeterminism.
fn run_replay(trace_path: &std::path::Path, args: &[String], logger: &Logger) -> i32 {
    let trace = match olang::timeline::Timeline::load_trace(trace_path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("olang replay: {}", e);
            return 1;
        }
    };
    // Warn (do not refuse) if the source changed since recording: a
    // divergence will pinpoint where, which is more useful than a hard stop.
    let live_sha = sha256_hex(trace.source.as_bytes());
    if live_sha != trace.program_sha256 {
        logger.warn(
            "replay",
            "the trace's recorded checksum does not match its embedded source",
        );
    }
    // The replayed program's argv is its own; recorded os.args() results
    // replay from the log regardless, so this only shapes any live reads.
    let mut argv = vec![trace.program_path.clone()];
    argv.extend(args.iter().cloned());
    olang::stdlib::os::set_script_args(argv);

    let program = match OlangParser::new().parse(&trace.source) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("olang replay: the trace's source no longer parses: {}", e);
            return 1;
        }
    };
    let events = trace.events.len();
    let timeline = olang::timeline::Timeline::replay(trace.clone());
    let path = PathBuf::from(&trace.program_path);
    match execute_program(
        program,
        &trace.source,
        &path,
        false,
        false,
        false,
        None,
        None,
        None,
        Some(timeline),
        None,
        logger,
        None,
    ) {
        Ok(()) => {
            eprintln!(
                "olang replay: clean — {} recorded event(s) reproduced",
                events
            );
            0
        }
        Err(_) => 1,
    }
}

fn start_repl(verbose: bool, no_ovm: bool, _logger: &Logger) -> anyhow::Result<()> {
    // One REPL: the interpreter with the bytecode tier enabled by default;
    // --no-ovm gives the pure tree-walker.
    let mut repl = Repl::with_tier(verbose, !no_ovm)?;
    Ok(repl.run()?)
}
