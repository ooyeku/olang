//! The starter libraries: curated, pure-olang libraries every shelf
//! ships with.
//!
//! A fresh shelf is not empty. The first time the shelf is touched it
//! is seeded with a small curated set of libraries — pure olang,
//! `///`-documented, each carrying its own test blocks — materialized
//! to real directories under the shelf's home and registered like
//! anything else. From there they are ordinary shelf citizens: `otc
//! add textkit` from any project, `otc lib remove textkit` when
//! unwanted, `otc lib restore textkit` to bring one back (or refresh
//! it to the running toolchain's copy).
//!
//! The sources live in this crate (`src/pkg/starter/*.ol`) and are
//! compiled in via `include_str!`, so seeding needs no network and no
//! repository checkout — the binary is the distribution.

use std::path::Path;

pub struct StarterLib {
    pub name: &'static str,
    /// One line for `otc lib list` and the docs.
    pub summary: &'static str,
    /// The library's `index.ol`.
    pub source: &'static str,
}

/// The curated set. Additions must be pure olang, `///`-documented,
/// and carry test blocks — `tests/shelf_starter_test.rs` enforces the
/// last by running them.
pub const STARTERS: &[StarterLib] = &[
    StarterLib {
        name: "textkit",
        summary: "text layout and humane formatting: pad, wrap, columns, table, money, human_bytes, human_duration, slugify",
        source: include_str!("starter/textkit.ol"),
    },
    StarterLib {
        name: "validate",
        summary: "declarative checks for map-shaped input: every problem reported, each naming its field",
        source: include_str!("starter/validate.ol"),
    },
    StarterLib {
        name: "markdown",
        summary: "a Markdown renderer: to_html with escaping safe for untrusted input",
        source: include_str!("starter/markdown.ol"),
    },
];

pub fn get(name: &str) -> Option<&'static StarterLib> {
    STARTERS.iter().find(|s| s.name == name)
}

/// Write `lib` out as a real library directory: its `index.ol` plus an
/// `olang.toml` naming it, versioned as the running toolchain. An
/// existing directory is overwritten — that is what makes `restore`
/// also mean "refresh to this toolchain's copy".
pub fn materialize(lib: &StarterLib, dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join("index.ol"), lib.source)?;
    std::fs::write(
        dir.join("olang.toml"),
        format!(
            "[package]\nname = \"{}\"\nversion = \"{}\"\ndescription = \"{}\"\n",
            lib.name,
            env!("CARGO_PKG_VERSION"),
            lib.summary
        ),
    )?;
    Ok(())
}
