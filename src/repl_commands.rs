//! The REPL's `:` commands, described once.
//!
//! The terminal REPL (`src/repl.rs`) and the editor protocol
//! (`olang repl --serve`, `src/repl_serve.rs`) run the same command code
//! — `Repl::handle_command`. This module is what both know *about* the
//! commands: each one's name, aliases, usage, summary, what its
//! arguments are (for completion), its group (for help), and whether an
//! editor answers it with its own interface rather than the session's
//! (`client`: clearing a panel, a history list, multi-line input,
//! ending the session).
//!
//! It also holds what the protocol needs to turn the terminal's words
//! into an editor's: the nearest command to a mistyped one, completion
//! of a command line, and ANSI-styled text split into styled runs.

/// What a command found besides its text, for an editor to show richly
/// (the terminal has only the text; `Repl::note` keeps these headless).
#[derive(Debug, Clone)]
pub enum Note {
    /// Something the editor does with its own interface: `clear` (the
    /// panel), `clear_history`, `quit` (the session ends), `multiline`.
    Effect(&'static str),
    /// A value, with what it is (`value`, a variable's name, `bindings`).
    Value { label: String, value: crate::ast::Value },
    /// A type, as `:type` says it.
    Type(String),
    /// How long an evaluation took, ms.
    Time(f64),
    /// A benchmark's runs, ms each.
    Bench(Vec<f64>),
    /// A richer place the editor may have: `profiler`, `tiers`.
    Open(&'static str),
    /// A tutorial, to be taken step by step.
    Tutorial(crate::help::Tutorial),
    /// The command reference belongs with the answer (`:help`).
    Commands,
    /// The working folder changed.
    Cwd(String),
}

/// What a command's arguments are, for completion and help.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arg {
    /// No arguments.
    None,
    /// olang code: an expression or a statement.
    Code,
    /// A variable of the session.
    Var,
    /// A function's name.
    Fn,
    /// A help topic: a function, a module, a category, a subcommand.
    Topic,
    /// A file or folder.
    Path,
    /// A folder.
    Dir,
    /// An olang file.
    File,
    /// A shell command line (paths complete).
    Shell,
    /// A tutorial's name.
    Tutorial,
    /// One of these words.
    Words(&'static [&'static str]),
}

impl Arg {
    /// The kind's name in the protocol (`commands`).
    pub fn name(&self) -> &'static str {
        match self {
            Arg::None => "none",
            Arg::Code => "code",
            Arg::Var => "var",
            Arg::Fn => "fn",
            Arg::Topic => "topic",
            Arg::Path => "path",
            Arg::Dir => "dir",
            Arg::File => "file",
            Arg::Shell => "shell",
            Arg::Tutorial => "tutorial",
            Arg::Words(_) => "words",
        }
    }
}

/// One command.
#[derive(Debug, Clone, Copy)]
pub struct CommandSpec {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub usage: &'static str,
    pub summary: &'static str,
    pub arg: Arg,
    pub group: &'static str,
    /// An editor answers it with its own interface (the session's answer
    /// is the terminal's, for a client that has none).
    pub client: bool,
    /// In the terminal REPL's TAB completion and its "did you mean".
    pub tab: bool,
}

const fn c(
    name: &'static str,
    aliases: &'static [&'static str],
    usage: &'static str,
    summary: &'static str,
    arg: Arg,
    group: &'static str,
    client: bool,
    tab: bool,
) -> CommandSpec {
    CommandSpec { name, aliases, usage, summary, arg, group, client, tab }
}

/// Every command, in the order the terminal's completion lists them
/// (those with `tab`), the others after.
pub const COMMANDS: &[CommandSpec] = &[
    c(":help", &[], ":help [topic]", "Help: the overview, a function, a module, a category; list, examples, syntax, tutorials, search <query>, contextual", Arg::Topic, "Help", false, true),
    c(":quit", &["quit"], ":quit", "End the session", Arg::None, "Session", true, true),
    c(":env", &[], ":env [--full]", "The core functions and the session's bindings", Arg::Words(&["--full"]), "Values", false, true),
    c(":clear", &[], ":clear [env|history]", "Clear the screen; env: forget the session's bindings; history: forget the history", Arg::Words(&["env", "history"]), "Session", true, true),
    c(":ovm", &[], ":ovm [status|<function>]", "The bytecode tier: what was compiled, what ran where, why a function was refused", Arg::Fn, "Performance", false, true),
    c(":version", &[], ":version", "The olang version", Arg::None, "Session", false, true),
    c(":history", &[], ":history [<count>|search <pattern>]", "The inputs of the session, the last <count>, or those containing <pattern>", Arg::Words(&["search"]), "Session", true, true),
    c(":type", &[], ":type <expression>", "The type of a value, deeply (List<Int>)", Arg::Code, "Values", false, true),
    c(":time", &[], ":time <expression>", "Evaluate and say how long it took", Arg::Code, "Performance", false, true),
    c(":memory", &[], ":memory", "How memory is managed and the session's bindings", Arg::None, "Performance", false, true),
    c(":stats", &[], ":stats", "The bytecode tier's counts: promoted, rejected, calls", Arg::None, "Performance", false, true),
    c(":parallel", &[], ":parallel [status|enable|disable|threshold <n>]", "Automatic parallelism: its state, on or off, the list size it starts at", Arg::Words(&["status", "enable", "disable", "threshold"]), "Performance", false, true),
    c(":run", &[], ":run <file>", "Run an olang file in the session", Arg::File, "Files and shell", false, true),
    c(":debug", &[], ":debug [on|off|<expression>]", "Debug mode, or step through an expression (its time, the watched variables it changed)", Arg::Code, "Debugging", false, true),
    c(":watch", &[], ":watch [<variable>]", "Watch a variable (again: stop); with none, those watched", Arg::Var, "Debugging", false, true),
    c(":inspect", &[], ":inspect <variable>", "A variable in detail: type, value, length, fields, parameters", Arg::Var, "Debugging", false, true),
    c(":trace", &[], ":trace [<function>]", "Trace a function (again: stop); with none, those traced", Arg::Fn, "Debugging", false, true),
    c(":set", &[], ":set <variable> <value>", "Bind a variable to a value's result", Arg::Var, "Debugging", false, true),
    c(":stack", &[], ":stack", "The debugger's call stack", Arg::None, "Debugging", false, true),
    c(":profile", &[], ":profile <code>", "Run code under the sampling profiler: time by tier, functions, hottest paths", Arg::Code, "Performance", false, true),
    c(":config", &[], ":config [<setting> <value>]", "The REPL's settings: prompt, show_types, debug_mode, time_commands", Arg::Words(&["prompt", "show_types", "debug_mode", "time_commands"]), "Session", false, true),
    c(":benchmark", &[], ":benchmark <expression>", "Evaluate five times: each run, the average, least and most", Arg::Code, "Performance", false, true),
    c(":search", &[], ":search <query> [category:<c>] [returns:<t>] [limit:<n>]", "Search the documentation, with filters", Arg::Topic, "Help", false, true),
    c(":tutorial", &[], ":tutorial [<name>]", "The tutorials, or one", Arg::Tutorial, "Help", false, true),
    c(":tutorial_run", &[], ":tutorial_run <name>", "A tutorial step by step: run each step's code, or your own", Arg::Tutorial, "Help", false, true),
    c(":contextual_help", &[], ":contextual_help", "Suggestions from what you did recently", Arg::None, "Help", false, true),
    c(":help_advanced", &[], ":help_advanced", "Search, tutorials and contextual help explained", Arg::None, "Help", false, true),
    c(":sh", &["!"], ":sh <command>", "Run a command through your shell (also !<command>)", Arg::Shell, "Files and shell", false, true),
    c(":cd", &[], ":cd [<folder>]", "Change the working folder (home with none); its package loads", Arg::Dir, "Files and shell", false, true),
    c(":pwd", &[], ":pwd", "The working folder", Arg::None, "Files and shell", false, true),
    c(":ls", &[], ":ls [<args>]", "List files (ls)", Arg::Path, "Files and shell", false, true),
    c(":pkg", &[], ":pkg [load <path>]", "Load the working folder's package again, or a package at a path, for `use`", Arg::Words(&["load"]), "Packages", false, false),
    c(":use", &[], ":use <module>", "The statement `use <module>` (the colon is forgiven)", Arg::Code, "Values", false, false),
    c(":let", &[], ":let <name> = <value>", "The statement `let …` (the colon is forgiven)", Arg::Code, "Values", false, false),
    c(":fn", &[], ":fn <name>(<params>) = <body>", "The statement `fn …` (the colon is forgiven)", Arg::Code, "Values", false, false),
    c(":!", &[], ":!<n>", "Evaluate input <n> of the history again", Arg::None, "Session", true, false),
    c(":ml", &[], ":ml", "Multi-line input: lines collect until :end evaluates them, :cancel abandons", Arg::None, "Input", true, false),
    c(":end", &[], ":end", "Evaluate the multi-line input", Arg::None, "Input", true, false),
    c(":cancel", &[], ":cancel", "Abandon the multi-line input", Arg::None, "Input", true, false),
];

/// The terminal REPL's TAB-completed commands, in its order.
pub fn tab_names() -> Vec<&'static str> {
    COMMANDS.iter().filter(|c| c.tab).map(|c| c.name).collect()
}

/// The command a line's first word names (by name or alias; `:!3` is
/// `:!`, `!ls` is `:sh`).
pub fn find(word: &str) -> Option<&'static CommandSpec> {
    if word.starts_with(":!") {
        return COMMANDS.iter().find(|c| c.name == ":!");
    }
    if word.starts_with('!') {
        return COMMANDS.iter().find(|c| c.name == ":sh");
    }
    COMMANDS.iter().find(|c| c.name == word || c.aliases.contains(&word))
}

/// The nearest of `candidates` to a mistyped `word`, as the terminal
/// REPL chooses it: within a third of the word's length (1 to 3 edits).
pub fn nearest<'a>(word: &str, candidates: &[&'a str]) -> Option<&'a str> {
    let max_distance = (word.chars().count() / 3).clamp(1, 3);
    candidates
        .iter()
        .map(|c| (crate::interpreter::IntuitiveErrorFormatter::levenshtein_distance(word, c), *c))
        .filter(|(d, _)| *d <= max_distance)
        .min()
        .map(|(_, c)| c)
}

/// The nearest of every command (and its aliases) to `word`.
pub fn suggest(word: &str) -> Option<&'static str> {
    let names: Vec<&'static str> = COMMANDS.iter().filter(|c| c.name != ":!").map(|c| c.name).collect();
    nearest(word, &names)
}

// ── styled text ──────────────────────────────────────────────────────

/// A piece of a line in one style: `color` a terminal colour's name
/// (`red`, `green`, `yellow`, `blue`, `magenta`, `cyan`, `white`,
/// `gray`), bold, dim.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Run {
    pub text: String,
    pub color: Option<&'static str>,
    pub bold: bool,
    pub dim: bool,
}

/// Text with ANSI styling as lines of styled runs (other escapes are
/// dropped: the screen clear, cursor moves).
pub fn styled_lines(text: &str) -> Vec<Vec<Run>> {
    let mut lines: Vec<Vec<Run>> = vec![Vec::new()];
    let mut cur = Run::default();
    let mut chars = text.chars().peekable();
    let flush = |lines: &mut Vec<Vec<Run>>, cur: &mut Run| {
        if !cur.text.is_empty() {
            let t = std::mem::take(&mut cur.text);
            lines.last_mut().unwrap().push(Run { text: t, ..cur.clone() });
        }
    };
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                let mut params = String::new();
                let mut fin = ' ';
                for c2 in chars.by_ref() {
                    if c2.is_ascii_alphabetic() {
                        fin = c2;
                        break;
                    }
                    params.push(c2);
                }
                if fin == 'm' {
                    flush(&mut lines, &mut cur);
                    let codes: Vec<u32> = if params.is_empty() { vec![0] } else { params.split(';').filter_map(|p| p.parse().ok()).collect() };
                    let mut i = 0;
                    while i < codes.len() {
                        match codes[i] {
                            0 => {
                                cur.color = None;
                                cur.bold = false;
                                cur.dim = false;
                            }
                            1 => cur.bold = true,
                            2 => cur.dim = true,
                            22 => {
                                cur.bold = false;
                                cur.dim = false;
                            }
                            39 => cur.color = None,
                            30 | 90 => cur.color = Some("gray"),
                            31 | 91 => cur.color = Some("red"),
                            32 | 92 => cur.color = Some("green"),
                            33 | 93 => cur.color = Some("yellow"),
                            34 | 94 => cur.color = Some("blue"),
                            35 | 95 => cur.color = Some("magenta"),
                            36 | 96 => cur.color = Some("cyan"),
                            37 | 97 => cur.color = Some("white"),
                            38 => {
                                // 38;5;n and 38;2;r;g;b: no named colour
                                i += if codes.get(i + 1) == Some(&5) { 2 } else { 4 };
                            }
                            _ => {}
                        }
                        i += 1;
                    }
                }
            }
            continue;
        }
        if ch == '\n' {
            flush(&mut lines, &mut cur);
            lines.push(Vec::new());
            continue;
        }
        if ch == '\r' {
            continue;
        }
        cur.text.push(ch);
    }
    flush(&mut lines, &mut cur);
    // a final newline ends the last line rather than starting one
    if lines.len() > 1 && lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    lines
}

/// Text with its ANSI escapes taken out (every other character kept).
pub fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for c2 in chars.by_ref() {
                    if c2.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(ch);
    }
    out
}

// ── completion ───────────────────────────────────────────────────────

/// One completion: what is inserted, a word on what it is.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub label: String,
    pub kind: &'static str,
    pub detail: String,
}

/// The names completion offers, from the session.
#[derive(Default)]
pub struct Names {
    /// The session's bindings: (name, a short type).
    pub vars: Vec<(String, String)>,
    /// Functions of the session (user functions).
    pub fns: Vec<String>,
    /// Builtins, stdlib `module.fn` names and modules.
    pub library: Vec<String>,
    /// Help topics beyond the library names.
    pub topics: Vec<String>,
    /// Tutorials' names.
    pub tutorials: Vec<String>,
    /// The working folder (paths complete against it).
    pub cwd: std::path::PathBuf,
}

/// Whether the cursor is inside an unclosed double-quoted string.
pub fn in_string(before: &str) -> bool {
    let mut on = false;
    let mut esc = false;
    for ch in before.chars() {
        if esc {
            esc = false;
        } else if ch == '\\' {
            esc = true;
        } else if ch == '"' {
            on = !on;
        }
    }
    on
}

fn word_start(before: &str, extra: &[char]) -> usize {
    before
        .rfind(|c: char| !(c.is_alphanumeric() || c == '_' || c == '.' || extra.contains(&c)))
        .map(|i| i + before[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1))
        .unwrap_or(0)
}

/// Complete `line` at byte `pos`: `(start, items)` — the items replace
/// `line[start..pos]`.
pub fn complete(line: &str, pos: usize, names: &Names) -> (usize, Vec<Item>) {
    let pos = pos.min(line.len());
    let before = &line[..pos];
    let trimmed = before.trim_start();
    let lead = before.len() - trimmed.len();
    // the command word
    if trimmed.starts_with(':') && !trimmed.contains(char::is_whitespace) {
        let items = COMMANDS
            .iter()
            .filter(|c| c.name != ":!" && c.name.starts_with(trimmed))
            .map(|c| Item { label: c.name.to_string(), kind: "command", detail: c.summary.to_string() })
            .collect();
        return (lead, items);
    }
    if let Some(rest) = trimmed.strip_prefix('!') {
        return complete_path(before, lead + 1 + (rest.len() - rest.trim_start().len()), names, PathKind::Any);
    }
    if trimmed.starts_with(':') {
        let first_end = trimmed.find(char::is_whitespace).unwrap_or(trimmed.len());
        let word = &trimmed[..first_end];
        let args = &trimmed[first_end..];
        let Some(cmd) = find(word) else { return (pos, Vec::new()) };
        let arg_index = args.split_whitespace().count() - usize::from(!args.ends_with(char::is_whitespace) && !args.trim().is_empty());
        return match cmd.arg {
            Arg::None => (pos, Vec::new()),
            Arg::Path | Arg::Shell => complete_path(before, path_start(before), names, PathKind::Any),
            Arg::Dir => complete_path(before, path_start(before), names, PathKind::Dir),
            Arg::File => complete_path(before, path_start(before), names, PathKind::Olang),
            Arg::Words(ws) => {
                if cmd.name == ":pkg" && arg_index >= 1 {
                    return complete_path(before, path_start(before), names, PathKind::Dir);
                }
                if cmd.name == ":config" && arg_index >= 1 {
                    return words(before, &["true", "false"], "value");
                }
                if arg_index >= 1 {
                    return (pos, Vec::new());
                }
                words(before, ws, "option")
            }
            Arg::Tutorial => {
                let ts: Vec<&str> = names.tutorials.iter().map(|s| s.as_str()).collect();
                words(before, &ts, "tutorial")
            }
            Arg::Var => {
                if cmd.name == ":set" && arg_index >= 1 {
                    return complete_code(line, pos, names);
                }
                let start = word_start(before, &[]);
                let w = &before[start..];
                let items = names
                    .vars
                    .iter()
                    .filter(|(n, _)| n.starts_with(w) && n != w)
                    .map(|(n, t)| Item { label: n.clone(), kind: "variable", detail: t.clone() })
                    .collect();
                (start, items)
            }
            Arg::Fn => {
                let start = word_start(before, &[]);
                let w = &before[start..];
                let mut items: Vec<Item> = Vec::new();
                if cmd.name == ":ovm" && "status".starts_with(w) && w != "status" {
                    items.push(Item { label: "status".into(), kind: "option", detail: String::new() });
                }
                let mut fns: Vec<&String> = names.fns.iter().filter(|n| n.starts_with(w) && *n != w).collect();
                fns.sort();
                items.extend(fns.into_iter().map(|n| Item { label: n.clone(), kind: "function", detail: String::new() }));
                (start, items)
            }
            Arg::Topic => {
                let start = word_start(before, &[]);
                let w = &before[start..];
                let mut items: Vec<Item> = Vec::new();
                if arg_index == 0 && cmd.name == ":help" {
                    for t in ["list", "examples", "syntax", "tutorials", "tutorial", "search", "contextual"] {
                        if t.starts_with(w) && t != w {
                            items.push(Item { label: t.into(), kind: "option", detail: String::new() });
                        }
                    }
                }
                let mut lib: Vec<&String> = names.library.iter().chain(names.topics.iter()).chain(names.fns.iter()).filter(|n| !w.is_empty() && n.starts_with(w) && *n != w).collect();
                lib.sort();
                lib.dedup();
                items.extend(lib.into_iter().take(200).map(|n| Item { label: n.clone(), kind: if n.contains('.') { "function" } else { "topic" }, detail: String::new() }));
                (start, items)
            }
            Arg::Code => {
                if cmd.name == ":debug" && arg_index == 0 {
                    let (s, mut items) = words(before, &["on", "off"], "option");
                    let (_, more) = complete_code(line, pos, names);
                    items.extend(more);
                    return (s, items);
                }
                complete_code(line, pos, names)
            }
        };
    }
    complete_code(line, pos, names)
}

fn words(before: &str, ws: &[&str], kind: &'static str) -> (usize, Vec<Item>) {
    let start = word_start(before, &['-']);
    let w = &before[start..];
    (start, ws.iter().filter(|x| x.starts_with(w) && **x != w).map(|x| Item { label: x.to_string(), kind, detail: String::new() }).collect())
}

/// Code: paths inside a string, else the names the session knows.
pub fn complete_code(line: &str, pos: usize, names: &Names) -> (usize, Vec<Item>) {
    let pos = pos.min(line.len());
    let before = &line[..pos];
    if in_string(before) {
        let q = before.rfind('"').map(|i| i + 1).unwrap_or(0);
        return complete_path(before, q, names, PathKind::Any);
    }
    let start = word_start(before, &[]);
    let w = &before[start..];
    if w.is_empty() {
        return (start, Vec::new());
    }
    let mut items: Vec<Item> = names
        .vars
        .iter()
        .filter(|(n, _)| n.starts_with(w) && n != w)
        .map(|(n, t)| Item { label: n.clone(), kind: if t.starts_with("fn") || t == "Function" { "function" } else { "variable" }, detail: t.clone() })
        .collect();
    let mut lib: Vec<&String> = names.library.iter().filter(|n| n.starts_with(w) && *n != w).collect();
    lib.sort();
    lib.dedup();
    // after `module.` only the module's members; else no dotted names
    let dotted = w.contains('.');
    items.extend(
        lib.into_iter()
            .filter(|n| dotted || !n.contains('.'))
            .take(200)
            .map(|n| Item { label: n.clone(), kind: "function", detail: String::new() }),
    );
    (start, items)
}

#[derive(Clone, Copy, PartialEq)]
enum PathKind {
    Any,
    Dir,
    Olang,
}

// Where the path being typed starts: after the last space outside quotes.
fn path_start(before: &str) -> usize {
    let mut start = 0;
    let mut quote = false;
    for (i, ch) in before.char_indices() {
        if ch == '"' || ch == '\'' {
            quote = !quote;
            start = i + 1;
        } else if ch.is_whitespace() && !quote {
            start = i + 1;
        }
    }
    start
}

fn complete_path(before: &str, start: usize, names: &Names, kind: PathKind) -> (usize, Vec<Item>) {
    let start = path_start(&before[..]).max(start).min(before.len());
    let typed = &before[start..];
    let (dir_part, file_part) = match typed.rfind('/') {
        Some(i) => (&typed[..=i], &typed[i + 1..]),
        None => ("", typed),
    };
    let base = if dir_part.starts_with('~') {
        let home = std::env::var("HOME").unwrap_or_default();
        std::path::PathBuf::from(dir_part.replacen('~', &home, 1))
    } else if dir_part.starts_with('/') {
        std::path::PathBuf::from(dir_part)
    } else {
        names.cwd.join(dir_part)
    };
    let Ok(rd) = std::fs::read_dir(&base) else { return (start, Vec::new()) };
    let mut items: Vec<Item> = Vec::new();
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') && !file_part.starts_with('.') {
            continue;
        }
        if !name.starts_with(file_part) {
            continue;
        }
        let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false)
            || (e.file_type().map(|t| t.is_symlink()).unwrap_or(false) && e.path().is_dir());
        if kind == PathKind::Dir && !is_dir {
            continue;
        }
        if kind == PathKind::Olang && !is_dir && !name.ends_with(".ol") {
            continue;
        }
        let label = format!("{}{}{}", dir_part, name, if is_dir { "/" } else { "" });
        items.push(Item { label, kind: if is_dir { "folder" } else { "file" }, detail: String::new() });
    }
    items.sort_by(|a, b| (a.kind != "folder", a.label.to_lowercase()).cmp(&(b.kind != "folder", b.label.to_lowercase())));
    items.truncate(200);
    (start, items)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_once_and_the_terminal_list_in_its_order() {
        let mut seen = std::collections::HashSet::new();
        for c in COMMANDS {
            assert!(seen.insert(c.name), "{} twice", c.name);
        }
        assert_eq!(tab_names().len(), 31);
        assert_eq!(tab_names()[0], ":help");
        assert_eq!(find("quit").unwrap().name, ":quit");
        assert_eq!(find(":!3").unwrap().name, ":!");
        assert_eq!(find("!ls").unwrap().name, ":sh");
    }

    #[test]
    fn the_nearest_command() {
        assert_eq!(suggest(":halp"), Some(":help"));
        assert_eq!(suggest(":tme"), Some(":time"));
        assert_eq!(suggest(":zzzzzzzz"), None);
    }

    #[test]
    fn ansi_into_runs() {
        let l = styled_lines("\x1b[1;36mHi\x1b[0m there\nnext\n");
        assert_eq!(l.len(), 2);
        assert_eq!(l[0][0], Run { text: "Hi".into(), color: Some("cyan"), bold: true, dim: false });
        assert_eq!(l[0][1].text, " there");
        assert_eq!(strip_ansi("\x1b[2J\x1b[1;1Ha\x1b[31mb"), "ab");
    }

    #[test]
    fn completes_commands_and_their_arguments() {
        let dir = std::env::temp_dir().join(format!("olang_rc_{}", std::process::id()));
        let _ = std::fs::create_dir_all(dir.join("src"));
        let _ = std::fs::write(dir.join("main.ol"), "");
        let _ = std::fs::write(dir.join("notes.txt"), "");
        let names = Names {
            vars: vec![("total".into(), "Int".into())],
            fns: vec!["twice".into()],
            library: vec!["len".into(), "str.trim".into(), "str".into()],
            tutorials: vec!["basics".into()],
            cwd: dir.clone(),
            ..Default::default()
        };
        let labels = |r: (usize, Vec<Item>)| r.1.into_iter().map(|i| i.label).collect::<Vec<_>>();
        assert!(labels(complete(":ti", 3, &names)).contains(&":time".to_string()));
        assert_eq!(labels(complete(":cd ", 4, &names)), vec!["src/"]);
        assert_eq!(labels(complete(":run ", 5, &names)), vec!["src/", "main.ol"]);
        assert_eq!(complete(":run ma", 7, &names).0, 5);
        assert_eq!(labels(complete(":inspect to", 11, &names)), vec!["total"]);
        assert_eq!(labels(complete(":trace tw", 9, &names)), vec!["twice"]);
        assert_eq!(labels(complete(":tutorial b", 11, &names)), vec!["basics"]);
        assert_eq!(labels(complete(":parallel th", 12, &names)), vec!["threshold"]);
        assert_eq!(labels(complete("str.t", 5, &names)), vec!["str.trim"]);
        assert_eq!(labels(complete(":type tot", 9, &names)), vec!["total"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
