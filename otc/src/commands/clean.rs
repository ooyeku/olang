//! `otc clean` — reclaim the prunable parts of `~/.olang`.
//!
//! `cache/` holds git dependency clones and the releases list (re-fetched
//! on demand) and `state/` holds warm hints and other regenerable runtime
//! data; both are safe to delete at any time. `--shelf` prunes the shelf's
//! copies nothing uses (`otc shelf prune --yes`). The toolchains and the
//! shims are never clean's business (`otc toolchain prune`).

use super::doctor::{dir_size, human};
use super::{Changes, ctx, fail};
use crate::say;
use anyhow::Result;

pub fn run(cache: bool, state: bool, shelf: bool, all: bool) -> Result<()> {
    let Some(home) = olang::home::root() else {
        return Err(fail(
            "no home directory could be determined",
            "export OLANG_HOME=<a folder> (or set HOME)",
        ));
    };
    let do_cache = cache || all || (!cache && !state && !shelf);
    let do_state = state || all;
    let do_shelf = shelf || all;

    let mut changes = Changes::new("clean");
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
            say!("  {} — already clean", t.display());
            continue;
        }
        let size = dir_size(t);
        changes.push("remove", t.display().to_string(), human(size));
        if !ctx().dry_run {
            std::fs::remove_dir_all(t)?;
        }
        reclaimed += size;
    }
    if do_shelf {
        reclaimed += super::shelf::prune(true, false)?;
    }
    if ctx().dry_run {
        say!("{} would be reclaimed.", human(reclaimed));
    } else {
        say!("reclaimed {} total.", human(reclaimed));
    }
    changes.finish(serde_json::json!({ "reclaimed": reclaimed }));
    Ok(())
}
