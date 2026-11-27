// Where cargo builds: one rule for `cargo xtask wasm`, which leaves the
// browser runtime there, and for the root crate's build.rs, which embeds
// it (build.rs includes this file). Each used to say `<repo>/target`,
// whatever CARGO_TARGET_DIR said: with it set, the build went to one
// folder and both read a stale runtime from the other (OL-640).

/// The target folder as cargo names it: `CARGO_TARGET_DIR` when it is
/// set and not empty, else `<root>/target`. A relative one is read from
/// `root`, the workspace's folder, which is where these builds are run.
pub fn target_dir(
    root: &std::path::Path,
    from_env: Option<std::ffi::OsString>,
) -> std::path::PathBuf {
    match from_env {
        Some(dir) if !dir.is_empty() => root.join(dir),
        _ => root.join("target"),
    }
}

#[cfg(test)]
mod tests {
    use super::target_dir;
    use std::path::{Path, PathBuf};

    #[test]
    fn the_target_folder_is_cargo_target_dir_when_it_is_set() {
        let root = Path::new("/work/olang");
        // unset, or set to nothing: the workspace's own
        assert_eq!(target_dir(root, None), PathBuf::from("/work/olang/target"));
        assert_eq!(
            target_dir(root, Some("".into())),
            PathBuf::from("/work/olang/target")
        );
        // a shared folder across worktrees, the case of the issue
        assert_eq!(
            target_dir(root, Some("/shared/target".into())),
            PathBuf::from("/shared/target")
        );
        // a relative one, from the workspace
        assert_eq!(
            target_dir(root, Some("out/t".into())),
            PathBuf::from("/work/olang/out/t")
        );
    }
}
