//! otc — the olang project tool.
//!
//! The division of labor is one sentence: everything that touches a
//! *file* lives in the `olang` binary (run, repl, test, fmt, check,
//! bench, profile); everything that touches a *project* or the machine's
//! olangs and libraries lives here. otc keeps the olang versions
//! (`otc use`, `otc toolchain`), the shelf of libraries (`otc shelf`),
//! and each project's dependencies (`otc add`, `list`, `upgrade`), with
//! the same verbs for all three (docs/packages.md).

// otc surfaces olang's interpreter Results; the large-error refactor is
// tracked in the olang crate (see olang/src/lib.rs).
#![allow(clippy::result_large_err)]
use clap::{Parser, Subcommand};
use std::process;

mod commands;

/// The help, grouped by what a person wants. Every subcommand is listed
/// here (a test checks it).
const HELP: &str = "\
{about-with-newline}
{usage-heading} {usage}

What runs here:
  status       What olang runs here and why, this project, the shelf, what to do next
               (also: otc alone)                    e.g. otc status

Olang versions:
  use          Install a version if needed and switch to it   e.g. otc use 1.0
  toolchain    List, add, remove, prune, link olang versions   e.g. otc toolchain list --available
  update       Install the newest release and switch to it     e.g. otc update

This project:
  new          Create a project                                e.g. otc new app
  add          Add a dependency: a folder, a shelf name, owner/repo   e.g. otc add ooyeku/geometry
  remove       Remove a dependency                             e.g. otc remove geometry
  list         Each dependency, the copy it got and why        e.g. otc list
  install      Resolve dependencies and write olang.lock       e.g. otc install --frozen
  outdated     What each dependency has and the newest allowed e.g. otc outdated
  upgrade      Move dependencies to the newest allowed         e.g. otc upgrade --dry-run
  bench        Run the project's benches                       e.g. otc bench

The shelf (libraries on this machine):
  shelf        List, add, use, upgrade, remove, prune libraries   e.g. otc shelf list

This machine:
  doctor       Check the installation and ~/.olang; --fix repairs   e.g. otc doctor --fix
  clean        Reclaim the cache, state, unused shelf copies   e.g. otc clean --all
  completions  A shell completion script                       e.g. otc completions zsh

Options:
{options}

`otc help <command>` says more about one. Every command takes --format json,
and every change --dry-run.
";

#[derive(Parser)]
#[command(name = "otc")]
#[command(about = "otc, olang's toolchain: olang versions, the shelf of libraries, and projects")]
#[command(version = olang::VERSION)]
#[command(help_template = HELP)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Print more about what otc is doing
    #[arg(short, long, global = true)]
    verbose: bool,

    /// `text`, or `json`: one object on stdout, in the shape docs/packages.md documents
    #[arg(long, value_name = "FORMAT", default_value = "text", global = true)]
    format: String,

    /// Say what would change, and change nothing
    #[arg(long, global = true)]
    dry_run: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// What olang runs here and why, the project here, the shelf, and at
    /// most three suggested commands.
    ///
    /// Example: otc status
    Status,
    /// Create a new olang project.
    ///
    /// Example: otc new app --lib
    New {
        /// Project name (also the directory to create)
        name: String,
        /// A library package: index.ol at the root, usable as a dependency
        #[arg(long)]
        lib: bool,
        /// A full-stack web app on the web SDK: JSON API + wasm frontend,
        /// one process, three files (main.ol, lib/pages.ol, client.ol)
        #[arg(long)]
        web: bool,
        /// The same web app written directly on the stdlib, without the
        /// SDK, to show what the SDK does for you
        #[arg(long = "web-bare")]
        web_bare: bool,
    },
    /// Add a dependency: a folder (../my-lib), a library on your shelf or
    /// in the registry (geometry, geometry@1.2), or a GitHub repository
    /// (owner/repo[@tag|@branch|@commit]). Says which it understood, shows
    /// the capabilities it asks for, and creates olang.toml if there is none.
    ///
    /// Example: otc add ooyeku/geometry@v1.2.0
    Add {
        /// A folder, a name[@version] from the shelf or the registry, or owner/repo[@ref]
        spec: String,
        /// Treat the argument as a folder even if it looks like a name
        #[arg(long)]
        path: Option<String>,
        /// Replace an existing dependency's source
        #[arg(long)]
        force: bool,
    },
    /// Remove a dependency (the shelf keeps its copies).
    ///
    /// Example: otc remove geometry
    Remove {
        /// Dependency name
        name: String,
    },
    /// Each dependency: the version it got, where it lives, and the rule
    /// that chose it (locked, asked, default, local, remote).
    ///
    /// Example: otc list --format json
    List,
    /// Resolve the dependencies and write olang.lock.
    ///
    /// Example: otc install --frozen
    Install {
        /// Fail if the lockfile would change (for CI)
        #[arg(long)]
        frozen: bool,
        /// Re-resolve everything to the newest satisfying sources
        #[arg(long)]
        update: bool,
    },
    /// What each dependency has, the newest its requirement allows, and
    /// the newest there is (reads GitHub for GitHub libraries).
    ///
    /// Example: otc outdated
    Outdated {
        /// Only what the shelf holds: no network
        #[arg(long)]
        offline: bool,
    },
    /// Move dependencies to the newest their requirement allows, saying
    /// each change first; a failure leaves olang.toml and olang.lock as
    /// they were.
    ///
    /// Example: otc upgrade geometry --dry-run
    Upgrade {
        /// Only these dependencies (default: all)
        names: Vec<String>,
        /// Go past the requirement and edit olang.toml
        #[arg(long)]
        major: bool,
        /// Go ahead when an upgrade asks for a new capability
        #[arg(long)]
        yes: bool,
    },
    /// Install an olang version if needed and make it the default: a
    /// version, a series (1.0), latest, stable, rc, a linked build's name,
    /// or - for the one before.
    ///
    /// Example: otc use rc
    Use {
        version: String,
        /// Pin this project (olang.toml and olang.lock) instead
        #[arg(long)]
        project: bool,
        /// With --project: allow a linked build's name in olang.lock
        #[arg(long)]
        local: bool,
    },
    /// The olang versions on this machine (alone: list them).
    ///
    /// Example: otc toolchain list --available
    Toolchain {
        #[command(subcommand)]
        cmd: Option<commands::update::ToolchainCommand>,
    },
    /// The shelf: the libraries on this machine, every version of each
    /// (alone: list them).
    ///
    /// Example: otc shelf list
    Shelf {
        #[command(subcommand)]
        cmd: Option<commands::shelf::ShelfCommand>,
    },
    /// List the registry's libraries whose name or description holds
    /// every word (all of them with no words)
    Search {
        /// Words to look for
        words: Vec<String>,
    },
    /// Check the library in this folder and write its release into a
    /// checkout of the registry's index (--dry-run: check everything, show
    /// the entry, write nothing). Commits and sends nothing
    Publish {
        /// The checkout of the index to write into (default: the folder
        /// OLANG_REGISTRY names)
        #[arg(long, value_name = "FOLDER")]
        index: Option<String>,
        /// Where the release is fetched from (default: the repository's origin)
        #[arg(long, value_name = "ADDRESS")]
        git: Option<String>,
        /// Rewrite a version that is already in the index
        #[arg(long)]
        force: bool,
        /// Skip `olang check` and `olang test`
        #[arg(long)]
        no_test: bool,
    },
    /// Withdraw a published release (name@version): no new install
    /// chooses it, and a lock that names it still installs
    Withdraw {
        /// The release, as name@version
        release: String,
        /// Why, shown to whoever still installs it
        #[arg(long)]
        reason: Option<String>,
        /// Put a withdrawn release back
        #[arg(long)]
        restore: bool,
        /// The checkout of the index to write into (default: the folder
        /// OLANG_REGISTRY names)
        #[arg(long, value_name = "FOLDER")]
        index: Option<String>,
    },
    /// Run the project's benches (bench/*.ol): scaling curves, verified
    /// answers, peak memory, baselines.
    ///
    /// Example: otc bench --runs 9
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
    /// Check the installation and ~/.olang for problems; --fix applies
    /// the safe repairs and moves an old layout to this olang's.
    ///
    /// Example: otc doctor --fix
    Doctor {
        /// Apply the safe repairs
        #[arg(long)]
        fix: bool,
    },
    /// Reclaim the prunable parts of ~/.olang (the cache by default).
    ///
    /// Example: otc clean --all --dry-run
    Clean {
        /// The dependency cache and the releases list (the default)
        #[arg(long)]
        cache: bool,
        /// Regenerable runtime state (warm hints, REPL history, known projects)
        #[arg(long)]
        state: bool,
        /// The shelf's copies nothing uses
        #[arg(long)]
        shelf: bool,
        /// Everything prunable
        #[arg(long)]
        all: bool,
    },
    /// Install the newest release and make it the default (`otc use latest`).
    ///
    /// Example: otc update --check
    Update {
        /// Only say what the newest is
        #[arg(long)]
        check: bool,
    },
    /// Print a shell completion script (zsh, bash, fish, elvish).
    ///
    /// Example: otc completions zsh
    Completions {
        /// The shell to generate for
        shell: clap_complete::Shell,
    },
}

impl Commands {
    /// The word an error's fallback `next:` is chosen by.
    fn word(&self) -> &'static str {
        match self {
            Commands::Status => "status",
            Commands::New { .. } => "new",
            Commands::Add { .. } => "add",
            Commands::Remove { .. } => "remove",
            Commands::List => "list",
            Commands::Install { .. } => "install",
            Commands::Outdated { .. } => "outdated",
            Commands::Upgrade { .. } => "upgrade",
            Commands::Use { .. } => "use",
            Commands::Toolchain { .. } => "toolchain",
            Commands::Shelf { .. } => "shelf",
            Commands::Bench { .. } => "bench",
            Commands::Doctor { .. } => "doctor",
            Commands::Clean { .. } => "clean",
            Commands::Update { .. } => "update",
            Commands::Completions { .. } => "completions",
        }
    }
}

fn main() {
    let cli = Cli::parse();
    let verbose = cli.verbose;
    let json = match cli.format.as_str() {
        "text" => false,
        "json" => true,
        other => {
            eprintln!("error: --format is `text` or `json`, not `{other}`");
            eprintln!(
                "next: otc {} --format json",
                std::env::args().nth(1).unwrap_or_default()
            );
            process::exit(2);
        }
    };
    commands::set_ctx(commands::Ctx {
        json,
        dry_run: cli.dry_run,
    });
    let command = cli.command.unwrap_or(Commands::Status);
    let word = command.word();

    let result = match command {
        Commands::Status => commands::status::run(),
        Commands::New {
            name,
            lib,
            web,
            web_bare,
        } => commands::new::execute(name, lib, web, web_bare, verbose),
        Commands::Add { spec, path, force } => {
            commands::project::add(&spec, path.as_deref(), force)
        }
        Commands::Remove { name } => commands::project::remove(&name),
        Commands::List => commands::project::list(),
        Commands::Install { frozen, update } => {
            commands::project::do_install(frozen, update, verbose)
        }
        Commands::Outdated { offline } => commands::project::outdated(offline),
        Commands::Upgrade { names, major, yes } => commands::project::upgrade(&names, major, yes),
        Commands::Use {
            version,
            project,
            local,
        } => commands::update::use_version(&version, project, local),
        Commands::Shelf { cmd } => cmd
            .unwrap_or(commands::shelf::ShelfCommand::List { name: None })
            .execute(),
        Commands::Search { words } => commands::registry::search(&words),
        Commands::Publish {
            index,
            git,
            force,
            no_test,
        } => commands::registry::publish(commands::registry::PublishArgs {
            index,
            git,
            dry_run: commands::ctx().dry_run,
            force,
            no_test,
        }),
        Commands::Withdraw {
            release,
            reason,
            restore,
            index,
        } => commands::registry::withdraw(&release, reason, restore, index.as_deref()),
        Commands::Doctor { fix } => commands::doctor::run(fix),
        Commands::Clean {
            cache,
            state,
            shelf,
            all,
        } => commands::clean::run(cache, state, shelf, all),
        Commands::Update { check } => commands::update::update(check),
        Commands::Toolchain { cmd } => {
            commands::update::toolchain(cmd.unwrap_or(commands::update::ToolchainCommand::List {
                available: false,
                stable: false,
                offline: false,
            }))
        }
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
        let next = commands::next_of(&e, word);
        if json {
            println!(
                "{}",
                serde_json::json!({
                    "format": 1, "kind": "error", "command": word,
                    "error": format!("{e:#}"), "next": next,
                })
            );
        } else {
            eprintln!("error: {e:#}");
            eprintln!("next: {next}");
        }
        process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn the_grouped_help_names_every_command() {
        for sub in Cli::command().get_subcommands() {
            let name = sub.get_name();
            assert!(
                HELP.contains(&format!("\n  {name} ")),
                "the help does not list `{name}`"
            );
            let about = sub
                .get_long_about()
                .map(|s| s.to_string())
                .unwrap_or_default();
            assert!(
                about.contains("Example: otc"),
                "`otc {name}` has no example"
            );
            for inner in sub.get_subcommands() {
                let about = inner
                    .get_long_about()
                    .map(|s| s.to_string())
                    .unwrap_or_default();
                assert!(
                    about.contains("Example: otc"),
                    "`otc {name} {}` has no example",
                    inner.get_name()
                );
            }
        }
    }

    /// Every `otc …` command in docs/packages.md is
    /// one otc reads: a renamed or removed verb fails here.
    #[test]
    fn every_command_in_the_docs_parses() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let mut checked = 0;
        for doc in ["docs/packages.md"] {
            let text = std::fs::read_to_string(root.join(doc)).unwrap();
            let mut in_block = false;
            for line in text.lines() {
                if line.trim_start().starts_with("```") {
                    in_block = !in_block;
                    continue;
                }
                let l = line.trim();
                if !in_block || !l.starts_with("otc ") {
                    continue;
                }
                // the command, without its comment and placeholders
                let cmd = l
                    .split("  ")
                    .next()
                    .unwrap_or(l)
                    .split(" #")
                    .next()
                    .unwrap_or(l);
                let words: Vec<String> = cmd
                    .split_whitespace()
                    .map(|w| {
                        w.replace(['<', '>', '[', ']'], "")
                            .replace('|', "-")
                            .replace("…", "")
                    })
                    .filter(|w| !w.is_empty())
                    .collect();
                if words.iter().any(|w| w.contains('/') && w.starts_with('-')) {
                    continue;
                }
                let parsed = Cli::try_parse_from(&words);
                if let Err(e) = &parsed
                    && !matches!(
                        e.kind(),
                        clap::error::ErrorKind::DisplayHelp
                            | clap::error::ErrorKind::DisplayVersion
                    )
                {
                    panic!("{doc}: `{cmd}` does not parse: {e}");
                }
                checked += 1;
            }
        }
        assert!(
            checked > 30,
            "only {checked} commands were found in the docs"
        );
    }
}
