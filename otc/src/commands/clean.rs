//! `otc clean` — reclaim the prunable parts of `~/.olang`.
//!
//! `cache/` holds git dependency clones (re-fetched on demand) and
//! `state/` holds warm hints and other regenerable runtime data; both
//! are safe to delete at any time. The shelf, the toolchains, and the
//! shims are never touched by this command — toolchains are removed
//! with `otc toolchain remove`, shelf entries with `otc lib remove`.

use super::doctor::{dir_size, human};
use anyhow::Result;

pub fn run(cache: bool, state: bool, all: bool) -> Result<()> {
    let Some(home) = olang::home::root() else {
        anyhow::bail!("no home directory could be determined");
    };
    let do_cache = cache || all || (!cache && !state);
    let do_state = state || all;

    let mut reclaimed = 0u64;
    let mut targets = Vec::new();
    if do_cache {
        targets.push(home.join("cache"));
    }
    if do_state {
        targets.push(home.join("state"));
    }
    for t in &targets {
        if !t.exists() {
            println!("  {} — already clean", t.display());
            continue;
        }
        let size = dir_size(t);
        std::fs::remove_dir_all(t)?;
        println!("  {} — reclaimed {}", t.display(), human(size));
        reclaimed += size;
    }
    println!("reclaimed {} total.", human(reclaimed));
    Ok(())
}
