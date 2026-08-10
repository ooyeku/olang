//! Package-management subcommands: `otc pkg <init|add|remove|install|tree|publish>`.
//!
//! olang packages are source, so these commands mostly edit `olang.toml`,
//! resolve/fetch dependencies into `olang.lock`, and inspect the graph.

use clap::Subcommand;
use olang::pkg::lock::{LockedSource, Lockfile};
use olang::pkg::manifest::{Dependency, Manifest, PackageMeta};
use olang::pkg::registry::{Registry, Release};
use olang::pkg::{InstallOptions, install};
use semver::Version;
use std::path::{Path, PathBuf};

#[derive(Subcommand)]
pub enum PkgCommand {
    /// Create an olang.toml in the current directory
    Init {
        /// Package name (default: directory name)
        #[arg(long)]
        name: Option<String>,
    },
    /// Add a dependency to olang.toml
    Add {
        /// Dependency name
        name: String,
        /// Local path (--path ../thing)
        #[arg(long)]
        path: Option<String>,
        /// Git URL (--git https://...)
        #[arg(long)]
        git: Option<String>,
        /// Git tag / rev / branch for a git dependency
        #[arg(long)]
        tag: Option<String>,
        /// Registry version requirement (--version "^1.0")
        #[arg(long = "version")]
        version_req: Option<String>,
    },
    /// Remove a dependency from olang.toml
    Remove {
        /// Dependency name
        name: String,
    },
    /// Fetch dependencies, honoring olang.lock (resolving only what the
    /// lock doesn't cover)
    Install {
        /// Fail if the lockfile would change (for CI)
        #[arg(long)]
        frozen: bool,
    },
    /// Re-resolve all dependencies to the newest satisfying sources and
    /// rewrite olang.lock
    Update,
    /// Show the resolved dependency tree
    Tree,
    /// Publish a release into a registry index
    Publish {
        /// Registry index directory
        #[arg(long)]
        registry: String,
        /// Git URL the source is fetched from
        #[arg(long)]
        git: String,
        /// Exact commit for this release
        #[arg(long)]
        rev: String,
    },
}

impl PkgCommand {
    pub fn execute(&self, verbose: bool) -> anyhow::Result<()> {
        match self {
            PkgCommand::Init { name } => init(name.clone()),
            PkgCommand::Add {
                name,
                path,
                git,
                tag,
                version_req,
            } => add(name, path, git, tag, version_req),
            PkgCommand::Remove { name } => remove(name),
            PkgCommand::Install { frozen } => do_install(*frozen, false, verbose),
            PkgCommand::Update => do_install(false, true, verbose),
            PkgCommand::Tree => tree(),
            PkgCommand::Publish { registry, git, rev } => publish(registry, git, rev),
        }
    }
}

fn project_root() -> anyhow::Result<PathBuf> {
    let cwd = std::env::current_dir()?;
    Manifest::find_root(&cwd)
        .ok_or_else(|| anyhow::anyhow!("no olang.toml found (run 'otc pkg init' first)"))
}

fn registry_from_env() -> Option<PathBuf> {
    std::env::var("OLANG_REGISTRY").ok().map(PathBuf::from)
}

fn init(name: Option<String>) -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    if cwd.join("olang.toml").exists() {
        anyhow::bail!("olang.toml already exists");
    }
    let name = name.unwrap_or_else(|| {
        cwd.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "my-package".to_string())
    });
    let manifest = Manifest {
        package: PackageMeta {
            name: name.clone(),
            version: Version::new(0, 1, 0),
            description: None,
            authors: Vec::new(),
            license: None,
        },
        dependencies: Default::default(),
    };
    manifest.save(&cwd).map_err(|e| anyhow::anyhow!("{}", e))?;
    println!("Created olang.toml for '{}'", name);
    Ok(())
}

fn add(
    name: &str,
    path: &Option<String>,
    git: &Option<String>,
    tag: &Option<String>,
    version_req: &Option<String>,
) -> anyhow::Result<()> {
    let root = project_root()?;
    let mut manifest = Manifest::load(&root).map_err(|e| anyhow::anyhow!("{}", e))?;

    let dep = if let Some(p) = path {
        Dependency::Path { path: p.clone() }
    } else if let Some(g) = git {
        Dependency::Git {
            git: g.clone(),
            tag: tag.clone(),
            rev: None,
            branch: None,
        }
    } else if let Some(v) = version_req {
        Dependency::Registry(v.clone())
    } else {
        anyhow::bail!("specify one of --path, --git, or --version");
    };

    manifest.dependencies.insert(name.to_string(), dep);
    manifest.save(&root).map_err(|e| anyhow::anyhow!("{}", e))?;
    println!("Added dependency '{}'", name);
    Ok(())
}

fn remove(name: &str) -> anyhow::Result<()> {
    let root = project_root()?;
    let mut manifest = Manifest::load(&root).map_err(|e| anyhow::anyhow!("{}", e))?;
    if manifest.dependencies.remove(name).is_none() {
        anyhow::bail!("'{}' is not a dependency", name);
    }
    manifest.save(&root).map_err(|e| anyhow::anyhow!("{}", e))?;
    println!("Removed dependency '{}'", name);
    Ok(())
}

fn do_install(frozen: bool, refresh: bool, verbose: bool) -> anyhow::Result<()> {
    let root = project_root()?;
    let lock_before = std::fs::read_to_string(root.join("olang.lock")).ok();
    let opts = InstallOptions {
        frozen,
        registry: registry_from_env(),
        refresh,
    };
    let map = install(&root, &opts).map_err(|e| anyhow::anyhow!("{}", e))?;
    println!(
        "Resolved {} dependenc{}",
        map.len(),
        if map.len() == 1 { "y" } else { "ies" }
    );
    if verbose {
        for (name, dir) in &map {
            println!("  {} -> {}", name, dir.display());
        }
    }
    let lock_after = std::fs::read_to_string(root.join("olang.lock")).ok();
    if lock_before == lock_after {
        println!("olang.lock unchanged");
    } else {
        println!("Wrote olang.lock");
    }
    Ok(())
}

fn tree() -> anyhow::Result<()> {
    let root = project_root()?;
    let manifest = Manifest::load(&root).map_err(|e| anyhow::anyhow!("{}", e))?;
    let lock = Lockfile::load(&root).map_err(|e| anyhow::anyhow!("{}", e))?;

    println!("{} {}", manifest.package.name, manifest.package.version);
    let direct: Vec<&String> = manifest.dependencies.keys().collect();
    for (i, name) in direct.iter().enumerate() {
        let last = i + 1 == direct.len();
        let branch = if last { "└──" } else { "├──" };
        let locked = lock.package.get(*name);
        let source = locked
            .map(|p| describe_source(&p.source))
            .unwrap_or_else(|| "(unresolved — run 'otc pkg install')".to_string());
        let version = locked
            .and_then(|p| p.version.clone())
            .map(|v| format!(" {}", v))
            .unwrap_or_default();
        println!("{} {}{} {}", branch, name, version, source);
        // one level of transitive deps
        if let Some(p) = locked {
            let child_prefix = if last { "    " } else { "│   " };
            for (j, child) in p.dependencies.iter().enumerate() {
                let clast = j + 1 == p.dependencies.len();
                let cbranch = if clast { "└──" } else { "├──" };
                println!("{}{} {}", child_prefix, cbranch, child);
            }
        }
    }
    Ok(())
}

fn describe_source(source: &LockedSource) -> String {
    match source {
        LockedSource::Path { path } => format!("(path {})", path),
        LockedSource::Git { git, rev, .. } => {
            format!("(git {} @ {})", git, &rev[..rev.len().min(8)])
        }
        LockedSource::Registry { .. } => "(registry)".to_string(),
    }
}

fn publish(registry: &str, git: &str, rev: &str) -> anyhow::Result<()> {
    let root = project_root()?;
    let manifest = Manifest::load(&root).map_err(|e| anyhow::anyhow!("{}", e))?;
    let checksum = olang::pkg::cache::checksum_dir(&root).ok();

    // A release's registry dependencies are the manifest's registry deps.
    let dependencies = manifest
        .dependencies
        .iter()
        .filter_map(|(n, d)| d.version_req().map(|r| (n.clone(), r.to_string())))
        .collect();

    let reg = Registry::at(Path::new(registry));
    reg.publish(
        &manifest.package.name,
        Release {
            version: manifest.package.version.clone(),
            git: git.to_string(),
            rev: rev.to_string(),
            checksum,
            dependencies,
        },
    )
    .map_err(|e| anyhow::anyhow!("{}", e))?;
    println!(
        "Published {}@{} to {}",
        manifest.package.name, manifest.package.version, registry
    );
    Ok(())
}
