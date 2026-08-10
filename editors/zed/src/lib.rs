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
        let path = worktree
            .which("olang")
            .ok_or_else(|| "olang not found on PATH (install it, or add its dir to PATH)".to_string())?;
        Ok(zed::Command {
            command: path,
            args: vec!["lsp".to_string()],
            env: Default::default(),
        })
    }
}

zed::register_extension!(OlangExtension);
