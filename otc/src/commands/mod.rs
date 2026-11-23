pub mod bench;
pub mod clean;
pub mod doctor;
pub mod new;
pub mod project;
pub mod registry;
pub mod releases;
pub mod shelf;
pub mod status;
pub mod toolchains;
pub mod update;
pub mod web;

use std::sync::OnceLock;

/// The global flags every command honors.
#[derive(Debug, Clone, Copy, Default)]
pub struct Ctx {
    /// `--format json`: one JSON object on stdout; words go to stderr.
    pub json: bool,
    /// `--dry-run`: say what would change, change nothing.
    pub dry_run: bool,
}

static CTX: OnceLock<Ctx> = OnceLock::new();

pub fn set_ctx(ctx: Ctx) {
    let _ = CTX.set(ctx);
}

pub fn ctx() -> Ctx {
    CTX.get().copied().unwrap_or_default()
}

/// Words for the person: stdout in text mode, stderr under `--format
/// json` (so stdout stays one JSON object).
#[macro_export]
macro_rules! say {
    () => { $crate::commands::say_line(String::new()) };
    ($($t:tt)*) => { $crate::commands::say_line(format!($($t)*)) };
}

pub fn say_line(line: String) {
    if ctx().json {
        eprintln!("{line}");
    } else {
        println!("{line}");
    }
}

/// Print a JSON answer (only under `--format json`).
pub fn emit(value: serde_json::Value) {
    if ctx().json {
        println!("{value}");
    }
}

/// An error that names the command to run next.
#[derive(Debug)]
pub struct Next {
    pub message: String,
    pub next: String,
}

impl std::fmt::Display for Next {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for Next {}

/// An error with the command that fixes it or shows more.
pub fn fail(message: impl Into<String>, next: impl Into<String>) -> anyhow::Error {
    anyhow::Error::new(Next {
        message: message.into(),
        next: next.into(),
    })
}

/// The next command an error names, if it names one; otherwise the one
/// that shows the state of things for `command`.
pub fn next_of(err: &anyhow::Error, command: &str) -> String {
    if let Some(n) = err.downcast_ref::<Next>() {
        return n.next.clone();
    }
    // an error that already names its command in backticks
    let text = err.to_string();
    if let Some(start) = text.rfind("`otc ").or_else(|| text.rfind("`olang "))
        && let Some(len) = text[start + 1..].find('`')
    {
        return text[start + 1..start + 1 + len].to_string();
    }
    match command {
        "toolchain" | "use" | "update" => "otc toolchain list".to_string(),
        "shelf" => "otc shelf list".to_string(),
        "add" | "remove" | "list" | "install" | "upgrade" | "outdated" => "otc list".to_string(),
        "doctor" | "clean" => "otc doctor".to_string(),
        _ => "otc status".to_string(),
    }
}

/// One change a command made (or, with `--dry-run`, would make).
pub struct Change {
    pub action: &'static str,
    pub what: String,
    pub detail: String,
}

/// What a command changed, said as it goes and answered as JSON at the
/// end: `#{ format: 1, kind: "change", command, dry_run, changes, now }`.
pub struct Changes {
    pub command: &'static str,
    pub list: Vec<Change>,
}

impl Changes {
    pub fn new(command: &'static str) -> Self {
        Changes {
            command,
            list: Vec::new(),
        }
    }

    /// Record a change and say it: `would <action>` under `--dry-run`.
    pub fn push(
        &mut self,
        action: &'static str,
        what: impl Into<String>,
        detail: impl Into<String>,
    ) {
        let (what, detail) = (what.into(), detail.into());
        let verb = if ctx().dry_run {
            format!("would {action}")
        } else {
            past(action).to_string()
        };
        if detail.is_empty() {
            say!("{verb} {what}");
        } else {
            say!("{verb} {what}: {detail}");
        }
        self.list.push(Change {
            action,
            what,
            detail,
        });
    }

    /// The JSON answer, with `now` saying what is in use after.
    pub fn finish(self, now: serde_json::Value) {
        if ctx().dry_run && !ctx().json {
            if self.list.is_empty() {
                say!("(dry run: nothing would change)");
            } else {
                say!("(dry run: nothing was changed)");
            }
        }
        emit(serde_json::json!({
            "format": 1,
            "kind": "change",
            "command": self.command,
            "dry_run": ctx().dry_run,
            "changes": self.list.iter().map(|c| serde_json::json!({
                "action": c.action, "what": c.what, "detail": c.detail,
            })).collect::<Vec<_>>(),
            "now": now,
        }));
    }
}

fn past(action: &str) -> &str {
    match action {
        "install" => "installed",
        "switch" => "switched to",
        "remove" => "removed",
        "link" => "linked",
        "unlink" => "unlinked",
        "pin" => "pinned",
        "add" => "added",
        "fetch" => "fetched",
        "upgrade" => "upgraded",
        "forget" => "forgot",
        "default" => "set the default of",
        "write" => "wrote",
        "move" => "moved",
        "keep" => "kept",
        other => other,
    }
}
