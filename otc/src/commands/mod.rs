pub mod bench;
pub mod clean;
pub mod doctor;
pub mod lib;
pub mod new;
pub mod project;
pub mod update;
pub mod web;

/// A `--format` value: `text` (false) or `json` (true), anything else
/// refused naming the command.
pub fn json_format(command: &str, format: &str) -> anyhow::Result<bool> {
    match format {
        "text" => Ok(false),
        "json" => Ok(true),
        other => anyhow::bail!("{command}: --format is `text` or `json`, not `{other}`"),
    }
}
