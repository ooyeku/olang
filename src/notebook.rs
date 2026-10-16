//! Notebooks: a `.olmd` file read as a program (docs/notebooks.md).
//!
//! A notebook is markdown. A fenced block whose info string begins with
//! `olang` is a code cell; everything else is prose. The program is the
//! cells' code with every other line made empty, so line N of the program
//! is line N of the file and a cell's columns are the file's: every
//! diagnostic, error frame and test location points into the file as
//! written, with no map between the two.
//!
//! The front matter (YAML between `---` lines at the top) gives the
//! notebook's `params`, bound as top-level `let`s on the front matter's
//! own lines, and its `capabilities`, the run's grant. The YAML read here
//! is a strict subset, documented in docs/notebooks.md: block maps and
//! lists, flow `[…]` and `{…}` on one line, plain, single- and
//! double-quoted scalars, and comments. Anything else is a diagnostic,
//! never a guess.

use crate::caps::{Caps, FsCap};
use std::path::Path;
use std::sync::Mutex;

/// The file extension of a notebook.
pub const EXTENSION: &str = "olmd";

/// Whether `path` names a notebook (its extension is `.olmd`).
pub fn is_notebook(path: &Path) -> bool {
    path.extension().is_some_and(|e| e == EXTENSION)
}

/// A problem with the notebook's structure, front matter or options, at a
/// 1-based line and character column of the file.
#[derive(Debug, Clone, PartialEq)]
pub struct Diagnostic {
    pub line: u32,
    pub column: u32,
    pub message: String,
}

impl Diagnostic {
    fn at(line: u32, column: u32, message: impl Into<String>) -> Self {
        Diagnostic {
            line,
            column,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}: {}", self.line, self.column, self.message)
    }
}

/// What the program is read for. `olang check` (and `fmt`'s gate) reads
/// the `no-run` cells as code, so they are checked; a run and `olang test`
/// leave them out, so they never run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Run,
    Check,
}

/// A cell's options, from the record after `olang` in its info string:
/// ```` ```olang {name: load, hide: code, run: manual, pin: true} ````.
/// Kept for the notebook pane (Studio); the program does not read them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CellOptions {
    /// The cell's id for links and runs.
    pub name: Option<String>,
    /// `code`, `output` or `all`: what is hidden when drawn and exported.
    pub hide: Option<String>,
    /// `auto` (the default for a pure cell) or `manual`.
    pub run: Option<String>,
    /// Keep the cell's last output in the file.
    pub pin: bool,
    /// `markdown`: a String result is drawn as markdown.
    pub as_: Option<String>,
}

/// One olang block of the file.
#[derive(Debug, Clone)]
pub struct Cell {
    /// The 1-based line of the opening fence.
    pub fence_line: u32,
    /// The 1-based line of the closing fence; for an unclosed fence, one
    /// past the file's last line.
    pub close_line: u32,
    /// ```` ```olang no-run ````: shown and checked, never run.
    pub no_run: bool,
    pub options: CellOptions,
}

impl Cell {
    /// The 1-based lines of the cell's code: `first..=last` (empty when
    /// `first > last`).
    pub fn code_lines(&self) -> std::ops::RangeInclusive<u32> {
        self.fence_line + 1..=self.close_line - 1
    }
}

/// A front matter param: its name, its value as olang source, and the
/// line its `let` is placed on (the line of its key).
#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub name: String,
    pub value: String,
    pub line: u32,
}

/// A notebook's structure: its cells, its front matter, and what is wrong
/// with them.
#[derive(Debug, Clone, Default)]
pub struct Notebook {
    pub cells: Vec<Cell>,
    /// The lines of the front matter's opening and closing `---`.
    pub front_matter: Option<(u32, u32)>,
    pub title: Option<String>,
    /// The line of `params:`, when there is one.
    pub params_line: Option<u32>,
    pub params: Vec<Param>,
    /// The front matter's grant; None when it has no `capabilities`.
    pub capabilities: Option<Caps>,
    pub diagnostics: Vec<Diagnostic>,
    /// A fence that is never closed: the structure past it is unknown, so
    /// `olang fmt` leaves the file alone.
    pub unclosed: bool,
    /// The front matter's grant could not be read (a bad `capabilities`,
    /// or front matter that is no YAML of the subset): whatever runs the
    /// notebook anyway (the REPL's kernel) runs it under no capability.
    pub capabilities_refused: bool,
}

/// A notebook made into a program.
#[derive(Debug, Clone, Default)]
pub struct Program {
    /// The program's source: the file's lines, every one that is not a
    /// cell's code empty, and the params' `let`s on the front matter's.
    pub source: String,
    /// The front matter's grant, if it has one.
    pub capabilities: Option<Caps>,
    /// Problems with the notebook as written; a run refuses on any.
    pub diagnostics: Vec<Diagnostic>,
    /// The lines of the front matter's `---`s, where the params' `let`s
    /// are: a param no cell reads is no unused name.
    pub front_matter: Option<(u32, u32)>,
    /// The front matter's grant could not be read (`Notebook`'s).
    pub capabilities_refused: bool,
}

// ── --param ─────────────────────────────────────────────────────────

static OVERRIDES: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());

/// `--param NAME=VALUE` for this process's notebooks: each overrides the
/// default of the param it names. Set once by the command line, before
/// any notebook is read.
pub fn set_param_overrides(overrides: Vec<(String, String)>) {
    if let Ok(mut o) = OVERRIDES.lock() {
        *o = overrides;
    }
}

fn param_overrides() -> Vec<(String, String)> {
    OVERRIDES.lock().map(|o| o.clone()).unwrap_or_default()
}

/// Split `NAME=VALUE`, as `--param` takes it.
pub fn parse_param_flag(text: &str) -> Result<(String, String), String> {
    match text.split_once('=') {
        Some((name, value)) if !name.trim().is_empty() => {
            Ok((name.trim().to_string(), value.to_string()))
        }
        _ => Err(format!("--param expects NAME=VALUE, got `{}`", text)),
    }
}

/// A `--param` value as olang source: the text itself when it parses as
/// an olang literal (`5`, `2.5`, `true`, `"east"`, `[1, 2]`, `#{ "a": 1 }`,
/// `()`), otherwise the text as a String.
pub fn param_value_source(text: &str) -> String {
    let trimmed = text.trim();
    if !trimmed.is_empty()
        && let Ok(program) = crate::parser::Parser::new().parse_raw(trimmed)
        && let [statement] = program.statements.as_slice()
        && let crate::ast::Statement::Expression(e) = statement.unwrapped()
        && is_literal(e)
    {
        return trimmed.to_string();
    }
    olang_string(text)
}

fn is_literal(e: &crate::ast::Expr) -> bool {
    use crate::ast::{Expr, UnaryOp};
    match e {
        Expr::Integer(_) | Expr::Float(_) | Expr::String(_) | Expr::RawString(_) => true,
        Expr::Boolean(_) => true,
        Expr::List(items) => items.iter().all(is_literal),
        Expr::Tuple(items) => items.iter().all(is_literal),
        Expr::MapLiteral { entries } => entries
            .iter()
            .all(|en| is_literal(&en.key) && is_literal(&en.value)),
        Expr::UnaryOp {
            op: UnaryOp::Negate,
            operand,
        } => matches!(**operand, Expr::Integer(_) | Expr::Float(_)),
        _ => false,
    }
}

/// `s` as a double-quoted olang string.
fn olang_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || c == '\u{7f}' => {
                out.push_str(&format!("\\u{{{:x}}}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

// ── the file ────────────────────────────────────────────────────────

/// The file's lines, a trailing `\r` taken off each. `split('\n')`, so a
/// file ending in a newline has an empty last line and the program keeps
/// the file's line count exactly.
fn lines_of(text: &str) -> Vec<&str> {
    text.split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .collect()
}

/// The 1-based character column of byte offset `at` in `line`.
fn col(line: &str, at: usize) -> u32 {
    line.get(..at).map(|s| s.chars().count()).unwrap_or(0) as u32 + 1
}

/// An opening fence: up to three spaces, then three or more backticks or
/// tildes, then the info string (which, after backticks, has none).
/// Answers (fence character, its count, the info string's byte offset).
fn fence_open(line: &str) -> Option<(char, usize, usize)> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    if indent > 3 {
        return None;
    }
    let rest = &line[indent..];
    let ch = rest.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let count = rest.len() - rest.trim_start_matches(ch).len();
    if count < 3 {
        return None;
    }
    let info = &rest[count..];
    if ch == '`' && info.contains('`') {
        return None;
    }
    Some((ch, count, indent + count))
}

/// A closing fence for one opened with `count` of `ch`.
fn fence_close(line: &str, ch: char, count: usize) -> bool {
    let indent = line.len() - line.trim_start_matches(' ').len();
    if indent > 3 {
        return false;
    }
    let rest = &line[indent..];
    let n = rest.len() - rest.trim_start_matches(ch).len();
    n >= count && rest[n..].trim().is_empty()
}

/// What an info string makes a block: an olang cell (with whether it is
/// `no-run`, and its options), or anything else.
enum Kind {
    Cell(bool, CellOptions),
    Other,
}

/// Read an info string; `at` is its byte offset in `line` (for columns).
fn classify(line: &str, at: usize, line_no: u32, diags: &mut Vec<Diagnostic>) -> Kind {
    let raw = &line[at..];
    let lead = raw.len() - raw.trim_start().len();
    let info = raw.trim();
    let Some(rest) = info.strip_prefix("olang") else {
        return Kind::Other;
    };
    if !(rest.is_empty() || rest.starts_with(char::is_whitespace) || rest.starts_with('{')) {
        return Kind::Other;
    }
    let mut pos = at + lead + 5;
    let mut rest_t = rest.trim_start();
    pos += rest.len() - rest_t.len();
    let mut no_run = false;
    if let Some(after) = rest_t.strip_prefix("no-run")
        && (after.is_empty() || after.starts_with(char::is_whitespace) || after.starts_with('{'))
    {
        no_run = true;
        let t = after.trim_start();
        pos += 6 + (after.len() - t.len());
        rest_t = t;
    }
    if rest_t.is_empty() {
        return Kind::Cell(no_run, CellOptions::default());
    }
    if !rest_t.starts_with('{') {
        let word = rest_t.split_whitespace().next().unwrap_or(rest_t);
        diags.push(Diagnostic::at(
            line_no,
            col(line, pos),
            format!(
                "`{}` after `olang`: a cell's info string is `olang`, then `no-run` or an options record such as {{name: load}}",
                word
            ),
        ));
        return Kind::Cell(no_run, CellOptions::default());
    }
    match parse_options(line, pos, line_no) {
        Ok(options) => Kind::Cell(no_run, options),
        Err(d) => {
            diags.push(d);
            Kind::Cell(no_run, CellOptions::default())
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum OptValue {
    Word(String),
    Str(String),
    Bool(bool),
}

/// The options record at byte `at` of `line`: `{key: value, …}`, a value
/// a bare word, a double-quoted string, `true` or `false`. Every key is
/// validated; the first problem is the answer.
fn parse_options(line: &str, at: usize, line_no: u32) -> Result<CellOptions, Diagnostic> {
    let bytes = line.as_bytes();
    let err = |p: usize, m: String| Diagnostic::at(line_no, col(line, p), m);
    let skip_ws = |mut p: usize| {
        while p < bytes.len() && (bytes[p] == b' ' || bytes[p] == b'\t') {
            p += 1;
        }
        p
    };
    let word_end = |mut p: usize| {
        while p < bytes.len()
            && (bytes[p].is_ascii_alphanumeric() || matches!(bytes[p], b'_' | b'-' | b'.'))
        {
            p += 1;
        }
        p
    };
    let mut p = at + 1; // past `{`
    let mut seen: Vec<String> = Vec::new();
    let mut options = CellOptions::default();
    loop {
        p = skip_ws(p);
        if p >= bytes.len() {
            return Err(err(
                at,
                "the cell's options record is not closed: `}` is missing".into(),
            ));
        }
        if bytes[p] == b'}' {
            p += 1;
            break;
        }
        // key
        let key_at = p;
        let ke = word_end(p);
        if ke == p || !(bytes[p].is_ascii_alphabetic() || bytes[p] == b'_') {
            return Err(err(
                p,
                "a cell's options record is `{key: value, …}`: a key is expected here".into(),
            ));
        }
        let key = line[p..ke].to_string();
        p = skip_ws(ke);
        if p >= bytes.len() || bytes[p] != b':' {
            return Err(err(
                p,
                format!("`:` is expected after the option `{}`", key),
            ));
        }
        p = skip_ws(p + 1);
        // value
        let value_at = p;
        let value = if p < bytes.len() && bytes[p] == b'"' {
            let mut s = String::new();
            let mut q = p + 1;
            let mut closed = false;
            while q < line.len() {
                let c = line[q..].chars().next().unwrap_or('"');
                if c == '\\' {
                    let n = line[q + 1..].chars().next();
                    match n {
                        Some(e @ ('"' | '\\')) => {
                            s.push(e);
                            q += 2;
                        }
                        _ => {
                            return Err(err(
                                q,
                                "only \\\" and \\\\ escape in an option's string".into(),
                            ));
                        }
                    }
                    continue;
                }
                if c == '"' {
                    closed = true;
                    q += 1;
                    break;
                }
                s.push(c);
                q += c.len_utf8();
            }
            if !closed {
                return Err(err(value_at, "the option's string is not closed".into()));
            }
            p = q;
            OptValue::Str(s)
        } else {
            let ve = word_end(p);
            if ve == p {
                return Err(err(p, format!("the option `{}` has no value", key)));
            }
            let w = &line[p..ve];
            p = ve;
            match w {
                "true" => OptValue::Bool(true),
                "false" => OptValue::Bool(false),
                w => OptValue::Word(w.to_string()),
            }
        };
        if seen.contains(&key) {
            return Err(err(key_at, format!("the option `{}` is given twice", key)));
        }
        seen.push(key.clone());
        let text = match &value {
            OptValue::Word(w) | OptValue::Str(w) => Some(w.clone()),
            OptValue::Bool(_) => None,
        };
        let one_of = |allowed: &[&str]| -> Result<String, Diagnostic> {
            match &text {
                Some(t) if allowed.contains(&t.as_str()) => Ok(t.clone()),
                _ => Err(err(
                    value_at,
                    format!(
                        "`{}` is one of {}",
                        key,
                        allowed
                            .iter()
                            .map(|a| format!("`{}`", a))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                )),
            }
        };
        match key.as_str() {
            "name" => match &text {
                Some(t) if !t.is_empty() => options.name = Some(t.clone()),
                _ => {
                    return Err(err(value_at, "`name` is a word or a string".into()));
                }
            },
            "hide" => options.hide = Some(one_of(&["code", "output", "all"])?),
            "run" => options.run = Some(one_of(&["auto", "manual"])?),
            "as" => options.as_ = Some(one_of(&["markdown"])?),
            "pin" => match value {
                OptValue::Bool(b) => options.pin = b,
                _ => return Err(err(value_at, "`pin` is `true` or `false`".into())),
            },
            other => {
                return Err(err(
                    key_at,
                    format!(
                        "a cell has no option `{}` (its options: name, hide, run, pin, as)",
                        other
                    ),
                ));
            }
        }
        p = skip_ws(p);
        if p < bytes.len() && bytes[p] == b',' {
            p += 1;
        } else if p < bytes.len() && bytes[p] != b'}' {
            return Err(err(
                p,
                "`,` or `}` is expected in the cell's options".into(),
            ));
        }
    }
    let trailing = skip_ws(p);
    if trailing < bytes.len() {
        return Err(err(
            trailing,
            "nothing may follow the cell's options record".into(),
        ));
    }
    Ok(options)
}

/// Read a notebook's structure: its front matter, its cells and their
/// options. Never fails; what is wrong is in `diagnostics`.
pub fn parse(text: &str) -> Notebook {
    let lines = lines_of(text);
    let mut nb = Notebook::default();
    let mut i = 0usize;
    if lines.first().is_some_and(|l| l.trim_end() == "---") {
        match lines
            .iter()
            .enumerate()
            .skip(1)
            .find(|(_, l)| matches!(l.trim_end(), "---" | "..."))
        {
            Some((close, _)) => {
                nb.front_matter = Some((1, close as u32 + 1));
                read_front_matter(&lines[1..close], 2, &mut nb);
                i = close + 1;
            }
            None => {
                nb.diagnostics.push(Diagnostic::at(
                    1,
                    1,
                    "the front matter opened here is never closed: it ends at a line `---`",
                ));
                nb.capabilities_refused = true;
                i = 1;
            }
        }
    }
    let mut names: Vec<(String, u32)> = Vec::new();
    while i < lines.len() {
        let line = lines[i];
        let Some((ch, count, info_at)) = fence_open(line) else {
            i += 1;
            continue;
        };
        let line_no = i as u32 + 1;
        let close = (i + 1..lines.len()).find(|&k| fence_close(lines[k], ch, count));
        let close_line = match close {
            Some(k) => k as u32 + 1,
            None => {
                nb.unclosed = true;
                nb.diagnostics.push(Diagnostic::at(
                    line_no,
                    col(line, info_at - count),
                    format!(
                        "this fence is never closed: a line of {} or more {} ends it",
                        count,
                        if ch == '`' { "backticks" } else { "tildes" }
                    ),
                ));
                lines.len() as u32 + 1
            }
        };
        if let Kind::Cell(no_run, options) = classify(line, info_at, line_no, &mut nb.diagnostics) {
            if let Some(name) = &options.name {
                if let Some((_, first)) = names.iter().find(|(n, _)| n == name) {
                    nb.diagnostics.push(Diagnostic::at(
                        line_no,
                        1,
                        format!(
                            "two cells are named `{}`: this one and the cell at line {}",
                            name, first
                        ),
                    ));
                } else {
                    names.push((name.clone(), line_no));
                }
            }
            nb.cells.push(Cell {
                fence_line: line_no,
                close_line,
                no_run,
                options,
            });
        }
        i = close.map(|k| k + 1).unwrap_or(lines.len());
    }
    nb
}

/// The front matter's keys: `title`, `params`, `capabilities`; others
/// are allowed and ignored.
fn read_front_matter(lines: &[&str], first_line: u32, nb: &mut Notebook) {
    let doc = match yaml::parse(lines, first_line) {
        Ok(doc) => doc,
        Err(d) => {
            nb.diagnostics.push(d);
            nb.capabilities_refused = true;
            return;
        }
    };
    let entries = match doc {
        None => return,
        Some(yaml::Node {
            value: yaml::Value::Map(entries),
            ..
        }) => entries,
        Some(node) => {
            nb.diagnostics.push(Diagnostic::at(
                node.line,
                node.column,
                "the front matter is a map of keys (title, params, capabilities)",
            ));
            nb.capabilities_refused = true;
            return;
        }
    };
    for entry in entries {
        match entry.key.as_str() {
            "title" => {
                if let yaml::Value::Str(s) = &entry.value.value {
                    nb.title = Some(s.clone());
                }
            }
            "params" => {
                nb.params_line = Some(entry.line);
                read_params(&entry, nb);
            }
            "capabilities" => match read_capabilities(&entry.value) {
                Ok(caps) => nb.capabilities = Some(caps),
                Err(d) => {
                    nb.diagnostics.push(d);
                    nb.capabilities_refused = true;
                }
            },
            _ => {}
        }
    }
}

fn read_params(entry: &yaml::Entry, nb: &mut Notebook) {
    let entries = match &entry.value.value {
        yaml::Value::Map(entries) => entries,
        yaml::Value::Null => return,
        _ => {
            nb.diagnostics.push(Diagnostic::at(
                entry.value.line,
                entry.value.column,
                "`params` is a map of each param's name to its default",
            ));
            return;
        }
    };
    let parser = crate::parser::Parser::new();
    for p in entries {
        // A param is a name a `let` can bind: an identifier, not a keyword.
        let bindable = p
            .key
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && p.key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            && parser
                .parse_raw(&format!("let {} = 1", p.key))
                .ok()
                .is_some_and(|prog| {
                    matches!(prog.statements.as_slice(), [s] if matches!(
                        s.unwrapped(),
                        crate::ast::Statement::LetDecl(d)
                            if matches!(&d.pattern, crate::ast::Pattern::Identifier(n) if *n == p.key)
                    ))
                });
        if !bindable {
            nb.diagnostics.push(Diagnostic::at(
                p.line,
                p.column,
                format!(
                    "the param `{}` is not a name olang can bind (a letter or `_`, then letters, digits or `_`; not a keyword)",
                    p.key
                ),
            ));
            continue;
        }
        nb.params.push(Param {
            name: p.key.clone(),
            value: yaml::to_olang(&p.value.value),
            line: p.line,
        });
    }
}

/// The grant in olang.toml's `[capabilities]` vocabulary: `fs` is `true`,
/// `false` or `read`; `net`, `proc`, `db`, `env` are `true` or `false`.
/// What is not named stays allowed, as in the manifest. Fail-closed: an
/// unknown key or value is an error, never ignored.
fn read_capabilities(node: &yaml::Node) -> Result<Caps, Diagnostic> {
    let entries = match &node.value {
        yaml::Value::Map(entries) => entries,
        yaml::Value::Null => return Ok(Caps::default()),
        _ => {
            return Err(Diagnostic::at(
                node.line,
                node.column,
                "`capabilities` is a map: fs, net, proc, db, env",
            ));
        }
    };
    let mut caps = Caps::default();
    for e in entries {
        let v = &e.value;
        let bad = |what: &str| {
            Diagnostic::at(
                v.line,
                v.column,
                format!(
                    "capabilities: {} is {}; an unknown value is refused, never ignored",
                    e.key, what
                ),
            )
        };
        match e.key.as_str() {
            "fs" => {
                caps.fs = match &v.value {
                    yaml::Value::Bool(true) => FsCap::Full,
                    yaml::Value::Bool(false) => FsCap::None,
                    yaml::Value::Str(s) if s == "read" => FsCap::Read,
                    _ => return Err(bad("true, false or \"read\"")),
                }
            }
            "net" | "proc" | "db" | "env" => {
                let yaml::Value::Bool(b) = v.value else {
                    return Err(bad("true or false"));
                };
                match e.key.as_str() {
                    "net" => caps.net = b,
                    "proc" => caps.proc = b,
                    "db" => caps.db = b,
                    _ => caps.env = b,
                }
            }
            other => {
                return Err(Diagnostic::at(
                    e.line,
                    e.column,
                    format!(
                        "capabilities has no capability `{}` (known: fs, net, proc, db, env); an unknown capability is refused, never ignored",
                        other
                    ),
                ));
            }
        }
    }
    Ok(caps)
}

/// The notebook in `text` as a program, `--param` overrides taken from
/// the command line (`set_param_overrides`).
pub fn program(text: &str, mode: Mode) -> Program {
    program_with(text, mode, &param_overrides())
}

/// The notebook in `text` as a program, with these `--param` overrides.
pub fn program_with(text: &str, mode: Mode, overrides: &[(String, String)]) -> Program {
    let nb = parse(text);
    let lines = lines_of(text);
    let mut out: Vec<String> = vec![String::new(); lines.len()];
    for cell in &nb.cells {
        if cell.no_run && mode == Mode::Run {
            continue;
        }
        for l in cell.code_lines() {
            if let Some(src) = lines.get(l as usize - 1) {
                out[l as usize - 1] = (*src).to_string();
            }
        }
    }
    let mut diagnostics = nb.diagnostics.clone();
    // Params: each a `let` on its key's line, the line's others after `;`.
    let mut params = nb.params.clone();
    for (name, value) in overrides {
        match params.iter_mut().find(|p| &p.name == name) {
            Some(p) => p.value = param_value_source(value),
            None => {
                let declared: Vec<&str> = nb.params.iter().map(|p| p.name.as_str()).collect();
                diagnostics.push(Diagnostic::at(
                    nb.params_line.unwrap_or(1),
                    1,
                    if declared.is_empty() {
                        format!(
                            "--param {}: the notebook declares no params (front matter `params:`)",
                            name
                        )
                    } else {
                        format!(
                            "--param {}: the notebook declares no such param (its params: {})",
                            name,
                            declared.join(", ")
                        )
                    },
                ));
            }
        }
    }
    for p in &params {
        let slot = &mut out[p.line as usize - 1];
        if !slot.is_empty() {
            slot.push_str("; ");
        }
        slot.push_str(&format!("let {} = {}", p.name, p.value));
    }
    let source = out.join("\n");
    if !params.is_empty() {
        diagnostics.extend(param_conflicts(&source, &params, nb.front_matter));
    }
    // a notebook's own rules, held by the check (a run reads the program
    // as the language does)
    if mode == Mode::Check {
        diagnostics.extend(rules(&nb, &lines, &source));
    }
    Program {
        source,
        capabilities: nb.capabilities,
        diagnostics,
        front_matter: nb.front_matter,
        capabilities_refused: nb.capabilities_refused,
    }
}

// ── one name, one cell; top to bottom ───────────────────────────────

/// The top-level names a statement defines: a `let`'s bound names, a
/// function's, a type's (an enum's variants with it), an error type's, a
/// trait's; with `uses`, the names a `use` binds (each item, or the
/// module's last segment).
pub fn defined_names(st: &crate::ast::Statement, uses: bool) -> Vec<String> {
    use crate::ast::{ShareDecl, Statement, TypeDefinition, UseItem};
    let mut out = Vec::new();
    let use_names = |u: &crate::ast::UseDecl, out: &mut Vec<String>| {
        if u.items.is_empty()
            && let Some(last) = u.path.last()
        {
            out.push(last.clone());
        }
        for item in &u.items {
            match item {
                UseItem::Specific(n) => out.push(n.clone()),
                UseItem::Aliased { alias, .. } => out.push(alias.clone()),
                UseItem::Wildcard => {}
            }
        }
    };
    let type_names = |t: &crate::ast::TypeDecl, out: &mut Vec<String>| {
        out.push(t.name.clone());
        if let TypeDefinition::Enum { variants } = &t.definition {
            out.extend(variants.iter().map(|v| v.name.clone()));
        }
    };
    match st.unwrapped() {
        Statement::LetDecl(d) | Statement::ShareDecl(ShareDecl::Let(d)) => {
            pattern_names(&d.pattern, &mut out)
        }
        Statement::FunctionDecl(f) | Statement::ShareDecl(ShareDecl::Function(f)) => {
            out.push(f.name.clone())
        }
        Statement::TypeDecl(t) | Statement::ShareDecl(ShareDecl::Type(t)) => {
            type_names(t, &mut out)
        }
        Statement::ErrorTypeDecl(e) => out.push(e.name.clone()),
        Statement::TraitDecl(t) | Statement::ShareDecl(ShareDecl::Trait(t)) => {
            out.push(t.name.clone())
        }
        Statement::UseDecl(u) | Statement::ShareDecl(ShareDecl::Use(u)) if uses => {
            use_names(u, &mut out)
        }
        _ => {}
    }
    out
}

/// A notebook's rules, which `olang check` holds an `.olmd` to (a `.ol`
/// file keeps the language's own): **one name, one cell** — a top-level
/// name defined in two cells is an error at each, naming the other — and
/// **top to bottom** — a cell that uses a name a later cell defines
/// (functions included, which the language hoists) is an error at the
/// use, naming the later cell's line. A program that does not parse is
/// left to the checker's parse error.
fn rules(nb: &Notebook, lines: &[&str], source: &str) -> Vec<Diagnostic> {
    use crate::ast::Statement;
    let parser = crate::parser::Parser::new();
    let Ok(program) = parser.parse_raw(source) else {
        return Vec::new();
    };
    let cell_of = |line: u32| nb.cells.iter().position(|c| c.code_lines().contains(&line));
    // (name, cell, line, column, by a `use`)
    let mut defs: Vec<(String, usize, u32, u32, bool)> = Vec::new();
    for s in &program.statements {
        let (line, column) = match s {
            Statement::Located { line, column, .. } => (*line, *column),
            _ => continue,
        };
        let Some(cell) = cell_of(line) else { continue };
        let all = defined_names(s, true);
        let own = defined_names(s, false);
        for name in all {
            let by_use = !own.contains(&name);
            defs.push((name, cell, line, column, by_use));
        }
    }
    let mut out = Vec::new();
    // one name, one cell (a `use` of the same name in two cells is no clash:
    // each cell imports what it needs)
    for (name, cell, line, column, by_use) in &defs {
        if *by_use {
            continue;
        }
        if let Some((_, _, other, ..)) = defs
            .iter()
            .find(|(n, c, _, _, u)| n == name && c != cell && !u)
        {
            out.push(Diagnostic::at(
                *line,
                *column,
                format!(
                    "`{}` is defined in two cells, here and at line {}: a name has one cell in a notebook",
                    name, other
                ),
            ));
        }
    }
    // top to bottom: each cell's uses (the names it reads that no earlier
    // statement of its own defines), against the names later cells define
    for (i, cell) in nb.cells.iter().enumerate() {
        let range = cell.code_lines();
        let cell_source: String = (1..=lines.len() as u32)
            .map(|l| {
                if range.contains(&l) {
                    lines[l as usize - 1]
                } else {
                    ""
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        let Ok(cell_program) = parser.parse_raw(&cell_source) else {
            continue;
        };
        let mut analyzer = crate::analyze::Analyzer::new();
        let _ = analyzer.analyze_program(&cell_program);
        for name in analyzer.get_undefined_variables() {
            // defined above (or in this cell): no matter here
            if defs.iter().any(|(n, c, ..)| *n == name && *c <= i) {
                continue;
            }
            let Some((_, _, later, ..)) = defs.iter().find(|(n, c, ..)| *n == name && *c > i)
            else {
                continue;
            };
            let (line, column) =
                first_use(lines, range.clone(), &name).unwrap_or((*range.start(), 1));
            out.push(Diagnostic::at(
                line,
                column,
                format!(
                    "`{}` is defined below, in a later cell (line {}): a cell uses only the names defined above it",
                    name, later
                ),
            ));
        }
    }
    out.sort_by_key(|d| (d.line, d.column));
    out
}

/// Where `name` is first read in `lines` (1-based, inclusive range): the
/// first time it stands as a word of its own outside a string or a
/// comment, not a member (`x.name`) nor a key (`name:`).
fn first_use(
    lines: &[&str],
    range: std::ops::RangeInclusive<u32>,
    name: &str,
) -> Option<(u32, u32)> {
    let word = |c: char| c.is_alphanumeric() || c == '_';
    for l in range {
        let Some(text) = lines.get(l as usize - 1) else {
            break;
        };
        let chars: Vec<char> = text.chars().collect();
        let target: Vec<char> = name.chars().collect();
        let mut in_str: Option<char> = None;
        let mut k = 0;
        while k < chars.len() {
            let c = chars[k];
            if let Some(q) = in_str {
                if c == '\\' {
                    k += 2;
                    continue;
                }
                if c == q {
                    in_str = None;
                }
                k += 1;
                continue;
            }
            if c == '"' || c == '`' {
                in_str = Some(c);
                k += 1;
                continue;
            }
            if c == '/' && chars.get(k + 1) == Some(&'/') {
                break;
            }
            if chars[k..].starts_with(&target)
                && (k == 0 || !word(chars[k - 1]))
                && chars.get(k + target.len()).is_none_or(|n| !word(*n))
                && (k == 0 || chars[k - 1] != '.')
            {
                return Some((l, k as u32 + 1));
            }
            k += 1;
        }
    }
    None
}

/// A cell that defines a param's name again (a top-level `let`, `fn` or
/// `type`): an error, as a function declared twice is. A program that
/// does not parse has its parse error reported where it is read.
fn param_conflicts(source: &str, params: &[Param], front: Option<(u32, u32)>) -> Vec<Diagnostic> {
    use crate::ast::{ShareDecl, Statement};
    let Ok(program) = crate::parser::Parser::new().parse_raw(source) else {
        return Vec::new();
    };
    let front_end = front.map(|f| f.1).unwrap_or(0);
    let mut out = Vec::new();
    for s in &program.statements {
        let (line, column) = match s {
            Statement::Located { line, column, .. } => (*line, *column),
            _ => continue,
        };
        if line <= front_end {
            continue;
        }
        let mut names = Vec::new();
        let mut collect = |st: &Statement| match st {
            Statement::LetDecl(d) => pattern_names(&d.pattern, &mut names),
            Statement::FunctionDecl(f) => names.push(f.name.clone()),
            Statement::TypeDecl(t) => names.push(t.name.clone()),
            Statement::ShareDecl(ShareDecl::Let(d)) => pattern_names(&d.pattern, &mut names),
            Statement::ShareDecl(ShareDecl::Function(f)) => names.push(f.name.clone()),
            Statement::ShareDecl(ShareDecl::Type(t)) => names.push(t.name.clone()),
            _ => {}
        };
        collect(s.unwrapped());
        for name in names {
            if let Some(p) = params.iter().find(|p| p.name == name) {
                out.push(Diagnostic::at(
                    line,
                    column,
                    format!(
                        "`{}` is a param of the notebook (front matter, line {}): a cell may read it but not define it again",
                        name, p.line
                    ),
                ));
            }
        }
    }
    out
}

fn pattern_names(p: &crate::ast::Pattern, out: &mut Vec<String>) {
    use crate::ast::Pattern;
    match p {
        Pattern::Identifier(n) => out.push(n.clone()),
        Pattern::Tuple(ps) => ps.iter().for_each(|p| pattern_names(p, out)),
        Pattern::List { patterns, rest } => {
            patterns.iter().for_each(|p| pattern_names(p, out));
            if let Some(r) = rest {
                out.push(r.clone());
            }
        }
        _ => {}
    }
}

/// The program olang reads from `path`: a `.ol` file's text as it is, a
/// notebook's as `program` makes it.
pub fn read(path: &Path, mode: Mode) -> std::io::Result<Program> {
    let text = std::fs::read_to_string(path)?;
    Ok(if is_notebook(path) {
        program(&text, mode)
    } else {
        Program {
            source: text,
            ..Program::default()
        }
    })
}

/// A notebook's problems as `file:line:column: error: message` lines, the
/// form a run and `olang test` print them in.
pub fn report(path: &Path, diagnostics: &[Diagnostic]) -> String {
    diagnostics
        .iter()
        .map(|d| {
            format!(
                "{}:{}:{}: error: {}",
                path.display(),
                d.line,
                d.column,
                d.message
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// `olang fmt` on a notebook: each olang cell's code formatted as a file
/// of its own, every other line (prose, fences, front matter) byte for
/// byte. None when a fence is never closed, where the cells are unknown.
pub fn format(text: &str) -> Option<String> {
    let nb = parse(text);
    if nb.unclosed {
        return None;
    }
    let raw: Vec<&str> = text.split('\n').collect();
    let mut out: Vec<String> = Vec::with_capacity(raw.len());
    let mut at = 0usize; // 0-based index of the next raw line to copy
    for cell in &nb.cells {
        let first = cell.fence_line as usize; // 0-based index of the first code line
        let close = cell.close_line as usize - 1; // 0-based index of the closing fence
        while at < first {
            out.push(raw[at].to_string());
            at += 1;
        }
        let code: Vec<&str> = raw[first..close]
            .iter()
            .map(|l| l.strip_suffix('\r').unwrap_or(l))
            .collect();
        if code.iter().any(|l| !l.trim().is_empty()) {
            let formatted = crate::tools::fmt::format_source(&(code.join("\n") + "\n"));
            out.extend(
                formatted
                    .trim_end_matches('\n')
                    .split('\n')
                    .map(str::to_string),
            );
        }
        at = close;
    }
    while at < raw.len() {
        out.push(raw[at].to_string());
        at += 1;
    }
    Some(out.join("\n"))
}

// ── the front matter's YAML ─────────────────────────────────────────

/// The strict subset of YAML a notebook's front matter is read in:
///
/// - block maps (`key: value`) and block lists (`- item`), nested by
///   indentation in spaces;
/// - flow lists `[a, b]` and flow maps `{a: 1}`, each on one line;
/// - plain scalars (`null`/`~`, `true`/`false`, decimal integers and
///   floats, anything else a string), `"double-quoted"` strings with
///   backslash escapes, `'single-quoted'` strings (`''` a quote);
/// - `#` comments.
///
/// Anchors, aliases, tags, block scalars (`|`, `>`), multi-line flow
/// collections, several documents and tabs for indentation are errors.
pub mod yaml {
    use super::Diagnostic;

    #[derive(Debug, Clone, PartialEq)]
    pub enum Value {
        Null,
        Bool(bool),
        Int(i64),
        Float(f64),
        Str(String),
        List(Vec<Node>),
        Map(Vec<Entry>),
    }

    /// A value and where it starts (1-based line and column).
    #[derive(Debug, Clone, PartialEq)]
    pub struct Node {
        pub value: Value,
        pub line: u32,
        pub column: u32,
    }

    /// A map's entry: its key, where the key is, and its value.
    #[derive(Debug, Clone, PartialEq)]
    pub struct Entry {
        pub key: String,
        pub line: u32,
        pub column: u32,
        pub value: Node,
    }

    struct Line {
        no: u32,
        indent: usize,
        /// The line past its indentation, its comment taken off.
        text: String,
    }

    /// Parse the lines of a document; `first_line` is the file's line of
    /// the first. None for a document with nothing in it.
    pub fn parse(lines: &[&str], first_line: u32) -> Result<Option<Node>, Diagnostic> {
        let mut ls = Vec::new();
        for (k, raw) in lines.iter().enumerate() {
            let no = first_line + k as u32;
            let body = raw.trim_start_matches(' ');
            if body.starts_with('\t') {
                return Err(Diagnostic::at(
                    no,
                    1,
                    "a tab indents this line: YAML indents with spaces",
                ));
            }
            let text = strip_comment(body).trim_end();
            if text.is_empty() {
                continue;
            }
            if text == "---" || text == "..." {
                return Err(Diagnostic::at(
                    no,
                    1,
                    "one YAML document only in the front matter",
                ));
            }
            ls.push(Line {
                no,
                indent: raw.len() - body.len(),
                text: text.to_string(),
            });
        }
        if ls.is_empty() {
            return Ok(None);
        }
        let mut i = 0;
        let indent = ls[0].indent;
        let node = block(&mut ls, &mut i, indent)?;
        if i < ls.len() {
            return Err(Diagnostic::at(
                ls[i].no,
                ls[i].indent as u32 + 1,
                "this line's indentation does not match the lines above it",
            ));
        }
        Ok(Some(node))
    }

    fn strip_comment(s: &str) -> &str {
        let mut quote: Option<char> = None;
        let mut prev_space = true;
        let mut escaped = false;
        for (i, c) in s.char_indices() {
            match quote {
                Some('"') if escaped => escaped = false,
                Some('"') if c == '\\' => escaped = true,
                Some(q) if c == q => quote = None,
                Some(_) => {}
                None if c == '#' && prev_space => return &s[..i],
                None if (c == '"' || c == '\'') && prev_space_or_flow(s, i) => quote = Some(c),
                None => {}
            }
            prev_space = c == ' ' || c == '\t';
        }
        s
    }

    /// A quote opens a quoted scalar only where a scalar may start.
    fn prev_space_or_flow(s: &str, i: usize) -> bool {
        match s[..i].chars().last() {
            None => true,
            Some(c) => matches!(c, ' ' | '\t' | '[' | '{' | ',' | ':' | '-'),
        }
    }

    fn is_item(text: &str) -> bool {
        text == "-" || text.starts_with("- ")
    }

    fn block(ls: &mut [Line], i: &mut usize, indent: usize) -> Result<Node, Diagnostic> {
        if is_item(&ls[*i].text) {
            seq(ls, i, indent)
        } else {
            map(ls, i, indent)
        }
    }

    fn map(ls: &mut [Line], i: &mut usize, indent: usize) -> Result<Node, Diagnostic> {
        let (line, column) = (ls[*i].no, indent as u32 + 1);
        let mut entries: Vec<Entry> = Vec::new();
        while *i < ls.len() && ls[*i].indent == indent && !is_item(&ls[*i].text) {
            let no = ls[*i].no;
            let text = ls[*i].text.clone();
            let (key, rest_at) = split_key(&text)
                .ok_or_else(|| Diagnostic::at(no, column, "`key: value` is expected here"))?;
            if entries.iter().any(|e| e.key == key) {
                return Err(Diagnostic::at(
                    no,
                    column,
                    format!("the key `{}` is given twice", key),
                ));
            }
            let rest = text[rest_at..].trim();
            let rest_col = indent as u32
                + 1
                + text[..rest_at].chars().count() as u32
                + (text[rest_at..].len() - text[rest_at..].trim_start().len()) as u32;
            *i += 1;
            let value = if rest.is_empty() {
                if *i < ls.len() && ls[*i].indent > indent {
                    let inner = ls[*i].indent;
                    block(ls, i, inner)?
                } else if *i < ls.len() && ls[*i].indent == indent && is_item(&ls[*i].text) {
                    seq(ls, i, indent)?
                } else {
                    Node {
                        value: Value::Null,
                        line: no,
                        column: rest_col,
                    }
                }
            } else {
                inline(rest, no, rest_col)?
            };
            entries.push(Entry {
                key,
                line: no,
                column,
                value,
            });
            if *i < ls.len() && ls[*i].indent > indent {
                return Err(Diagnostic::at(
                    ls[*i].no,
                    ls[*i].indent as u32 + 1,
                    "this line is indented under a key that already has a value",
                ));
            }
        }
        Ok(Node {
            value: Value::Map(entries),
            line,
            column,
        })
    }

    fn seq(ls: &mut [Line], i: &mut usize, indent: usize) -> Result<Node, Diagnostic> {
        let (line, column) = (ls[*i].no, indent as u32 + 1);
        let mut items = Vec::new();
        while *i < ls.len() && ls[*i].indent == indent && is_item(&ls[*i].text) {
            let no = ls[*i].no;
            let text = ls[*i].text.clone();
            let after = &text[1..];
            let rest = after.trim_start();
            let offset = 1 + after.len() - rest.len();
            let rest_col = indent as u32 + 1 + offset as u32;
            if rest.is_empty() {
                *i += 1;
                let value = if *i < ls.len() && ls[*i].indent > indent {
                    let inner = ls[*i].indent;
                    block(ls, i, inner)?
                } else {
                    Node {
                        value: Value::Null,
                        line: no,
                        column: rest_col,
                    }
                };
                items.push(value);
            } else if !rest.starts_with(['[', '{']) && split_key(rest).is_some() {
                // `- key: value`: a map whose first line starts after `- `
                ls[*i].indent = indent + offset;
                ls[*i].text = rest.to_string();
                let inner = ls[*i].indent;
                items.push(map(ls, i, inner)?);
            } else {
                *i += 1;
                items.push(inline(rest, no, rest_col)?);
            }
            if *i < ls.len() && ls[*i].indent > indent {
                return Err(Diagnostic::at(
                    ls[*i].no,
                    ls[*i].indent as u32 + 1,
                    "this line is indented under a list item that already has a value",
                ));
            }
        }
        Ok(Node {
            value: Value::List(items),
            line,
            column,
        })
    }

    /// `key: rest` → (key, the byte offset of rest). A key is a plain
    /// word or a quoted string; `:` ends it when a space or the line's
    /// end follows.
    fn split_key(text: &str) -> Option<(String, usize)> {
        if text.starts_with(['"', '\'']) {
            let (key, end) = quoted(text, 0).ok()?;
            let after = &text[end..];
            let colon = after.len() - after.trim_start().len();
            let rest = &after[colon..];
            let tail = rest.strip_prefix(':')?;
            if !(tail.is_empty() || tail.starts_with(' ')) {
                return None;
            }
            return Some((key, end + colon + 1));
        }
        let mut at = None;
        for (k, c) in text.char_indices() {
            if c == ':' {
                let next = text[k + 1..].chars().next();
                if next.is_none() || next == Some(' ') {
                    at = Some(k);
                    break;
                }
            }
        }
        let k = at?;
        let key = text[..k].trim();
        if key.is_empty() || key.starts_with(['[', '{', '&', '*', '!', '|', '>', '?']) {
            return None;
        }
        Some((key.to_string(), k + 1))
    }

    /// A value written on the line after its key or `- `.
    fn inline(text: &str, line: u32, column: u32) -> Result<Node, Diagnostic> {
        let err = |m: &str| Diagnostic::at(line, column, m.to_string());
        match text.chars().next() {
            Some('|') | Some('>') => {
                return Err(err(
                    "block scalars (`|`, `>`) are not in the front matter's YAML: write the string quoted on one line",
                ));
            }
            Some('&') | Some('*') => {
                return Err(err(
                    "anchors and aliases are not in the front matter's YAML",
                ));
            }
            Some('!') => return Err(err("tags are not in the front matter's YAML")),
            Some('[') | Some('{') | Some('"') | Some('\'') => {
                let mut p = Flow {
                    s: text,
                    at: 0,
                    line,
                    column,
                };
                let node = p.value()?;
                p.ws();
                if p.at < text.len() {
                    return Err(Diagnostic::at(
                        line,
                        column + text[..p.at].chars().count() as u32,
                        "nothing may follow this value on its line",
                    ));
                }
                return Ok(node);
            }
            _ => {}
        }
        Ok(Node {
            value: plain(text, line, column)?,
            line,
            column,
        })
    }

    /// A plain scalar resolved: null, a Bool, an Int, a Float, or a String.
    fn plain(s: &str, line: u32, column: u32) -> Result<Value, Diagnostic> {
        let s = s.trim();
        Ok(match s {
            "" | "~" | "null" | "Null" | "NULL" => Value::Null,
            "true" | "True" | "TRUE" => Value::Bool(true),
            "false" | "False" | "FALSE" => Value::Bool(false),
            _ if is_int(s) => match s.parse::<i64>() {
                Ok(n) => Value::Int(n),
                Err(_) => {
                    return Err(Diagnostic::at(
                        line,
                        column,
                        format!("{} is past an Int's range", s),
                    ));
                }
            },
            _ if is_float(s) => match s.parse::<f64>() {
                Ok(f) if f.is_finite() => Value::Float(f),
                _ => {
                    return Err(Diagnostic::at(
                        line,
                        column,
                        format!("{} is past a Float's range", s),
                    ));
                }
            },
            _ => Value::Str(s.to_string()),
        })
    }

    fn is_int(s: &str) -> bool {
        let d = s.strip_prefix(['-', '+']).unwrap_or(s);
        !d.is_empty() && d.chars().all(|c| c.is_ascii_digit())
    }

    fn is_float(s: &str) -> bool {
        let d = s.strip_prefix(['-', '+']).unwrap_or(s);
        let (mant, exp) = match d.find(['e', 'E']) {
            Some(k) => (&d[..k], Some(&d[k + 1..])),
            None => (d, None),
        };
        let mant_ok = match mant.split_once('.') {
            Some((a, b)) => {
                (!a.is_empty() || !b.is_empty())
                    && a.chars().all(|c| c.is_ascii_digit())
                    && b.chars().all(|c| c.is_ascii_digit())
            }
            None => exp.is_some() && !mant.is_empty() && mant.chars().all(|c| c.is_ascii_digit()),
        };
        let exp_ok = exp.is_none_or(|e| {
            let e = e.strip_prefix(['-', '+']).unwrap_or(e);
            !e.is_empty() && e.chars().all(|c| c.is_ascii_digit())
        });
        mant_ok && exp_ok
    }

    /// A quoted scalar at byte `at` of `s`: (its text, the byte past it).
    fn quoted(s: &str, at: usize) -> Result<(String, usize), String> {
        let q = s[at..].chars().next().ok_or("a string is expected")?;
        let mut out = String::new();
        let mut k = at + 1;
        while k < s.len() {
            let c = s[k..].chars().next().unwrap_or(q);
            if q == '\'' && c == '\'' {
                if s[k + 1..].starts_with('\'') {
                    out.push('\'');
                    k += 2;
                    continue;
                }
                return Ok((out, k + 1));
            }
            if q == '"' && c == '"' {
                return Ok((out, k + 1));
            }
            if q == '"' && c == '\\' {
                let e = s[k + 1..].chars().next().ok_or("a string ends in `\\`")?;
                k += 1 + e.len_utf8();
                match e {
                    '"' => out.push('"'),
                    '\\' => out.push('\\'),
                    '/' => out.push('/'),
                    'n' => out.push('\n'),
                    't' => out.push('\t'),
                    'r' => out.push('\r'),
                    '0' => out.push('\0'),
                    'b' => out.push('\u{8}'),
                    'f' => out.push('\u{c}'),
                    ' ' => out.push(' '),
                    'x' | 'u' | 'U' => {
                        let n = match e {
                            'x' => 2,
                            'u' => 4,
                            _ => 8,
                        };
                        let hex = s.get(k..k + n).ok_or("a short \\x/\\u escape")?;
                        let code = u32::from_str_radix(hex, 16)
                            .ok()
                            .and_then(char::from_u32)
                            .ok_or_else(|| format!("`\\{}{}` is no character", e, hex))?;
                        out.push(code);
                        k += n;
                    }
                    other => return Err(format!("`\\{}` is no escape YAML knows", other)),
                }
                continue;
            }
            out.push(c);
            k += c.len_utf8();
        }
        Err("this string is not closed on its line".into())
    }

    /// A flow value: `[…]`, `{…}`, a quoted or a plain scalar.
    struct Flow<'a> {
        s: &'a str,
        at: usize,
        line: u32,
        column: u32,
    }

    impl Flow<'_> {
        fn col(&self) -> u32 {
            self.column + self.s[..self.at].chars().count() as u32
        }
        fn err(&self, m: impl Into<String>) -> Diagnostic {
            Diagnostic::at(self.line, self.col(), m.into())
        }
        fn ws(&mut self) {
            while self.at < self.s.len() && self.s.as_bytes()[self.at] == b' ' {
                self.at += 1;
            }
        }
        fn peek(&self) -> Option<char> {
            self.s[self.at..].chars().next()
        }
        fn value(&mut self) -> Result<Node, Diagnostic> {
            self.ws();
            let (line, column) = (self.line, self.col());
            let node = |value| Node {
                value,
                line,
                column,
            };
            match self.peek() {
                None => Ok(node(Value::Null)),
                Some('[') => {
                    self.at += 1;
                    let mut items = Vec::new();
                    loop {
                        self.ws();
                        match self.peek() {
                            None => {
                                return Err(self.err(
                                    "this `[` is not closed on its line (a flow list is one line)",
                                ));
                            }
                            Some(']') => {
                                self.at += 1;
                                break;
                            }
                            _ => {}
                        }
                        items.push(self.value()?);
                        self.ws();
                        match self.peek() {
                            Some(',') => self.at += 1,
                            Some(']') => {}
                            _ => return Err(self.err("`,` or `]` is expected")),
                        }
                    }
                    Ok(node(Value::List(items)))
                }
                Some('{') => {
                    self.at += 1;
                    let mut entries: Vec<Entry> = Vec::new();
                    loop {
                        self.ws();
                        match self.peek() {
                            None => {
                                return Err(self.err(
                                    "this `{` is not closed on its line (a flow map is one line)",
                                ));
                            }
                            Some('}') => {
                                self.at += 1;
                                break;
                            }
                            _ => {}
                        }
                        let key_col = self.col();
                        let key = match self.peek() {
                            Some('"') | Some('\'') => {
                                let (k, end) = quoted(self.s, self.at).map_err(|m| self.err(m))?;
                                self.at = end;
                                k
                            }
                            _ => {
                                let start = self.at;
                                while let Some(c) = self.peek() {
                                    if matches!(c, ':' | ',' | '}' | '[' | ']' | '{') {
                                        break;
                                    }
                                    self.at += c.len_utf8();
                                }
                                self.s[start..self.at].trim().to_string()
                            }
                        };
                        self.ws();
                        if self.peek() != Some(':') || key.is_empty() {
                            return Err(self.err("`key: value` is expected in a flow map"));
                        }
                        self.at += 1;
                        if entries.iter().any(|e| e.key == key) {
                            return Err(Diagnostic::at(
                                self.line,
                                key_col,
                                format!("the key `{}` is given twice", key),
                            ));
                        }
                        let value = self.value()?;
                        entries.push(Entry {
                            key,
                            line: self.line,
                            column: key_col,
                            value,
                        });
                        self.ws();
                        match self.peek() {
                            Some(',') => self.at += 1,
                            Some('}') => {}
                            _ => return Err(self.err("`,` or `}` is expected")),
                        }
                    }
                    Ok(node(Value::Map(entries)))
                }
                Some('"') | Some('\'') => {
                    let (s, end) = quoted(self.s, self.at).map_err(|m| self.err(m))?;
                    self.at = end;
                    Ok(node(Value::Str(s)))
                }
                Some('&' | '*' | '!' | '|' | '>') => Err(self.err(
                    "anchors, aliases, tags and block scalars are not in the front matter's YAML",
                )),
                Some(_) => {
                    let start = self.at;
                    while let Some(c) = self.peek() {
                        if matches!(c, ',' | ']' | '}') {
                            break;
                        }
                        self.at += c.len_utf8();
                    }
                    let text = &self.s[start..self.at];
                    Ok(node(plain(text, line, column)?))
                }
            }
        }
    }

    /// A value as olang source: `()`, a Bool, an Int, a Float, a String,
    /// a List, a Map with String keys.
    pub fn to_olang(v: &Value) -> String {
        match v {
            Value::Null => "()".to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Int(n) => n.to_string(),
            Value::Float(f) => format!("{:?}", f),
            Value::Str(s) => super::olang_string(s),
            Value::List(items) => format!(
                "[{}]",
                items
                    .iter()
                    .map(|n| to_olang(&n.value))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Value::Map(entries) if entries.is_empty() => "#{}".to_string(),
            Value::Map(entries) => format!(
                "#{{ {} }}",
                entries
                    .iter()
                    .map(|e| format!(
                        "{}: {}",
                        super::olang_string(&e.key),
                        to_olang(&e.value.value)
                    ))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = "---\ntitle: Sales\nparams:\n  region: \"east\"\n  n: 3\ncapabilities:\n  fs: \"read\"\n  net: false\n---\n\n# Sales\n\n```olang {name: load}\nlet a = region\n```\n\nprose\n\n~~~~olang no-run\nlet b = 2\n~~~~\n";

    #[test]
    fn the_program_keeps_every_line() {
        let p = program_with(EXAMPLE, Mode::Run, &[]);
        assert!(p.diagnostics.is_empty(), "{:?}", p.diagnostics);
        let lines: Vec<&str> = p.source.split('\n').collect();
        assert_eq!(lines.len(), EXAMPLE.split('\n').count());
        assert_eq!(lines[3], "let region = \"east\"");
        assert_eq!(lines[4], "let n = 3");
        assert_eq!(lines[13], "let a = region");
        assert_eq!(lines[19], "", "no-run is left out of a run");
        let c = program_with(EXAMPLE, Mode::Check, &[]);
        assert_eq!(c.source.split('\n').nth(19), Some("let b = 2"));
        let caps = p.capabilities.unwrap();
        assert_eq!(caps.fs, FsCap::Read);
        assert!(!caps.net && caps.proc);
    }

    #[test]
    fn overrides_parse_as_literals_or_strings() {
        let p = program_with(
            EXAMPLE,
            Mode::Run,
            &[("region".into(), "west".into()), ("n".into(), "7".into())],
        );
        let lines: Vec<&str> = p.source.split('\n').collect();
        assert_eq!(lines[3], "let region = \"west\"");
        assert_eq!(lines[4], "let n = 7");
        let bad = program_with(EXAMPLE, Mode::Run, &[("nope".into(), "1".into())]);
        assert_eq!(bad.diagnostics.len(), 1);
        assert_eq!(bad.diagnostics[0].line, 3);
        assert_eq!(param_value_source("[1, -2.5, \"x\"]"), "[1, -2.5, \"x\"]");
        assert_eq!(param_value_source("2026-01-01"), "\"2026-01-01\"");
        assert_eq!(param_value_source("a\"b"), "\"a\\\"b\"");
    }

    #[test]
    fn options_are_validated() {
        let nb = parse("```olang {name: a, hide: code, run: manual, pin: true}\n```\n");
        assert!(nb.diagnostics.is_empty(), "{:?}", nb.diagnostics);
        let o = &nb.cells[0].options;
        assert_eq!(o.name.as_deref(), Some("a"));
        assert!(o.pin);
        let bad = parse("x\n```olang {nme: a}\n```\n");
        assert_eq!(bad.diagnostics[0].line, 2);
        assert_eq!(bad.diagnostics[0].column, 11);
        assert!(bad.diagnostics[0].message.contains("no option `nme`"));
        for src in [
            "```olang {name: a\n```\n",
            "```olang {hide: everything}\n```\n",
            "```olang {pin: yes}\n```\n",
            "```olang {name: a, name: b}\n```\n",
            "```olang {name: a} extra\n```\n",
            "```olang fast\n```\n",
        ] {
            assert_eq!(parse(src).diagnostics.len(), 1, "{}", src);
        }
        assert!(parse("```olangx\n```\n").cells.is_empty());
    }

    #[test]
    fn fences_follow_commonmark() {
        let nb = parse("````olang\n```\nlet x = 1\n````\n");
        assert_eq!(nb.cells.len(), 1);
        assert_eq!(nb.cells[0].close_line, 4);
        let nb = parse("    ```olang\nlet x = 1\n");
        assert!(nb.cells.is_empty(), "four spaces make an indented block");
        let nb = parse("text\n```olang\nlet x = 1\n");
        assert!(nb.unclosed);
        assert_eq!(nb.diagnostics[0].line, 2);
        let nb = parse("~~~olang\nlet x = 1\n```\n~~~\n");
        assert_eq!(nb.cells[0].close_line, 4);
    }

    #[test]
    fn yaml_subset() {
        let doc = yaml::parse(
            &[
                "a: [1, 2.5, 'it''s', \"q\\n\"]",
                "b:",
                "  - x: 1",
                "    y: true",
                "  - ~",
                "c: {k: v, n: -3}  # comment",
                "d: plain text, with comma",
            ],
            1,
        )
        .unwrap()
        .unwrap();
        let yaml::Value::Map(e) = &doc.value else {
            panic!()
        };
        assert_eq!(
            yaml::to_olang(&e[0].value.value),
            "[1, 2.5, \"it's\", \"q\\n\"]"
        );
        assert_eq!(
            yaml::to_olang(&e[1].value.value),
            "[#{ \"x\": 1, \"y\": true }, ()]"
        );
        assert_eq!(
            yaml::to_olang(&e[2].value.value),
            "#{ \"k\": \"v\", \"n\": -3 }"
        );
        assert_eq!(
            yaml::to_olang(&e[3].value.value),
            "\"plain text, with comma\""
        );
        for bad in [
            &["a: |"][..],
            &["a: &x 1"],
            &["a: 1", "  b: 2"],
            &["a: [1,"],
            &["a: 1", "a: 2"],
        ] {
            assert!(yaml::parse(bad, 1).is_err(), "{:?}", bad);
        }
    }

    #[test]
    fn capabilities_fail_closed() {
        for bad in [
            "---\ncapabilities:\n  gpu: true\n---\n",
            "---\ncapabilities:\n  fs: write\n---\n",
            "---\ncapabilities:\n  net: \"no\"\n---\n",
            "---\ncapabilities: all\n---\n",
        ] {
            let nb = parse(bad);
            assert_eq!(nb.diagnostics.len(), 1, "{}", bad);
            assert!(nb.capabilities.is_none());
        }
    }

    #[test]
    fn format_leaves_prose_alone() {
        let src = "---\ntitle:   x  \n---\nSome   prose   \n```olang {name: a}\nlet x =   1   \n\n\n\nlet y = 2\n```\n  trailing  \n";
        let out = format(src).unwrap();
        assert_eq!(
            out,
            "---\ntitle:   x  \n---\nSome   prose   \n```olang {name: a}\nlet x = 1\n\nlet y = 2\n```\n  trailing  \n"
        );
    }

    #[test]
    fn a_cell_may_not_define_a_param() {
        let p = program_with(
            "---\nparams:\n  region: east\n---\n```olang\nlet region = 1\n```\n",
            Mode::Run,
            &[],
        );
        assert_eq!(p.diagnostics.len(), 1);
        assert_eq!(p.diagnostics[0].line, 6);
    }
}
