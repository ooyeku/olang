//! The library shelf: `otc lib add | list | remove | restore`.
//!
//! Registering a library once is what makes `otc add <name>` work from
//! any project without paths. The shelf itself is a user-level name →
//! directory map (see `olang::pkg::shelf`); these commands are its
//! whole interface. A brand-new shelf arrives stocked with the starter
//! libraries; `remove` takes one off like any other library, `restore`
//! brings one back (or refreshes it to the running toolchain's copy).

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
    /// Re-shelve starter libraries: a removed one comes back, a present
    /// one refreshes to this toolchain's copy
    Restore {
        /// A starter library's name; omit to restore all of them
        name: Option<String>,
    },
}

impl LibCommand {
    pub fn execute(&self) -> anyhow::Result<()> {
        match self {
            LibCommand::Add { path, name } => add(path, name.as_deref()),
            LibCommand::List => list(),
            LibCommand::Remove { name } => remove(name),
            LibCommand::Restore { name } => restore(name.as_deref()),
        }
    }
}

fn add(path: &str, name: Option<&str>) -> anyhow::Result<()> {
    let mut shelf = Shelf::load_or_seed().map_err(|e| anyhow::anyhow!("{}", e))?;
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
    let shelf = Shelf::load_or_seed().map_err(|e| anyhow::anyhow!("{}", e))?;
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
            "  (missing — directory is gone; re-register, remove, or restore)"
        } else if Manifest::load(dir).is_err() && !dir.join("index.ol").exists() {
            "  (no longer looks like a library)"
        } else if shelf.owns(dir) && olang::pkg::starter::get(name).is_some() {
            "  (starter)"
        } else {
            ""
        };
        println!("  {:<width$}  {}{}", name, dir.display(), status);
    }
    Ok(())
}

fn remove(name: &str) -> anyhow::Result<()> {
    let mut shelf = Shelf::load_or_seed().map_err(|e| anyhow::anyhow!("{}", e))?;
    let Some(dir) = shelf.libraries.remove(name) else {
        anyhow::bail!(
            "'{}' is not on your shelf (otc lib list shows what is)",
            name
        );
    };
    // A starter's directory is shelf-owned — materialized by seeding or
    // restore — so removal deletes it too. A user directory the shelf
    // merely points at is never touched.
    if shelf.owns(&dir) && olang::pkg::starter::get(name).is_some() {
        let _ = std::fs::remove_dir_all(&dir);
    }
    shelf.save().map_err(|e| anyhow::anyhow!("{}", e))?;
    println!("Removed '{}' from your shelf", name);
    if olang::pkg::starter::get(name).is_some() {
        println!(
            "It is a starter library:  otc lib restore {}  brings it back",
            name
        );
    }
    Ok(())
}

fn restore(name: Option<&str>) -> anyhow::Result<()> {
    let mut shelf = Shelf::load_or_seed().map_err(|e| anyhow::anyhow!("{}", e))?;
    match name {
        Some(one) => {
            let dir = shelf
                .restore_starter(one)
                .map_err(|e| anyhow::anyhow!("{}", e))?;
            shelf.save().map_err(|e| anyhow::anyhow!("{}", e))?;
            println!("Restored '{}' → {}", one, dir.display());
        }
        None => {
            for lib in olang::pkg::starter::STARTERS {
                let dir = shelf
                    .restore_starter(lib.name)
                    .map_err(|e| anyhow::anyhow!("{}", e))?;
                println!("Restored '{}' → {}", lib.name, dir.display());
            }
            shelf.save().map_err(|e| anyhow::anyhow!("{}", e))?;
        }
    }
    Ok(())
}
