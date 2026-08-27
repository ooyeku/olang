//! Zed extension: launches the language server that ships inside the
//! olang binary (`olang lsp`). Zed compiles this to wasm on install.
use zed_extension_api as zed;

struct OlangExtension;

impl zed::Extension for OlangExtension {
    fn new() -> Self {
        OlangExtension
    }

    fn language_server_command(
        &mut self,
        _id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> zed::Result<zed::Command> {
        // PATH first, then the well-known install locations. A
        // GUI-launched Zed inherits launchd's minimal PATH, which holds
        // none of the places olang installs to — the same silent failure
        // the VS Code client had.
        let path = worktree.which("olang").or_else(|| {
            let home = std::env::var("HOME").ok()?;
            // The real binary first; ~/.olang/olang is a legacy setup
            // wrapper that only forwards to ~/.cargo/bin anyway.
            [
                format!("{home}/.cargo/bin/olang"),
                "/usr/local/bin/olang".to_string(),
                "/opt/homebrew/bin/olang".to_string(),
                format!("{home}/.olang/olang"),
            ]
            .into_iter()
            .find(|p| std::fs::metadata(p).is_ok())
        });
        let path = path.ok_or_else(|| {
            "olang not found on PATH or in ~/.cargo/bin, /usr/local/bin, /opt/homebrew/bin,              ~/.olang — install olang, or add its directory to PATH"
                .to_string()
        })?;
        Ok(zed::Command {
            command: path,
            args: vec!["lsp".to_string()],
            env: Default::default(),
        })
    }
}

zed::register_extension!(OlangExtension);
