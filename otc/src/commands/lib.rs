//! The library shelf: `otc lib add | list | remove`.
//!
//! Registering a library once is what makes `otc add <name>` work from
//! any project without paths. The shelf itself is a user-level name →
//! directory map (see `olang::pkg::shelf`); these commands are its whole
//! interface.

use clap::Subcommand;
use olang::pkg::manifest::Manifest;
use olang::pkg::shelf::Shelf;

#[derive(Subcommand)]
pub enum LibCommand {
    /// Register a local library on your shelf, so any project can
    /// `otc add <name>` it
    Add {
        /// Directory of the library (must contain index.ol or olang.toml)
        path: String,
        /// Register under this name (default: the library's package name)
        #[arg(long)]
        name: Option<String>,
    },
    /// List the libraries on your shelf
    List,
    /// Remove a library from your shelf (projects already depending on
    /// it keep their lockfile pin until their next install)
    Remove {
        /// Registered name
        name: String,
    },
}

impl LibCommand {
    pub fn execute(&self) -> anyhow::Result<()> {
        match self {
            LibCommand::Add { path, name } => add(path, name.as_deref()),
            LibCommand::List => list(),
            LibCommand::Remove { name } => remove(name),
        }
    }
}

fn add(path: &str, name: Option<&str>) -> anyhow::Result<()> {
    let mut shelf = Shelf::load().map_err(|e| anyhow::anyhow!("{}", e))?;
    let registered = shelf
        .add(std::path::Path::new(path), name)
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    let dir = shelf.resolve(&registered).cloned().expect("just added");
    shelf.save().map_err(|e| anyhow::anyhow!("{}", e))?;
    println!("Shelved '{}' → {}", registered, dir.display());
    println!("Use it from any project:  otc add {}", registered);
    Ok(())
}

fn list() -> anyhow::Result<()> {
    let shelf = Shelf::load().map_err(|e| anyhow::anyhow!("{}", e))?;
    if shelf.libraries.is_empty() {
        println!("Your shelf is empty. Register a library:  otc lib add <path>");
        return Ok(());
    }
    let width = shelf
        .libraries
        .keys()
        .map(|n| n.chars().count())
        .max()
        .unwrap_or(4);
    for (name, dir) in &shelf.libraries {
        // Report drift rather than hiding it: a moved directory is the
        // one shelf problem a user actually hits.
        let status = if !dir.exists() {
            "  (missing — directory is gone; re-register or remove)"
        } else if Manifest::load(dir).is_err() && !dir.join("index.ol").exists() {
            "  (no longer looks like a library)"
        } else {
            ""
        };
        println!("  {:<width$}  {}{}", name, dir.display(), status);
    }
    Ok(())
}

fn remove(name: &str) -> anyhow::Result<()> {
    let mut shelf = Shelf::load().map_err(|e| anyhow::anyhow!("{}", e))?;
    if shelf.libraries.remove(name).is_none() {
        anyhow::bail!(
            "'{}' is not on your shelf (otc lib list shows what is)",
            name
        );
    }
    shelf.save().map_err(|e| anyhow::anyhow!("{}", e))?;
    println!("Removed '{}' from your shelf", name);
    Ok(())
}
