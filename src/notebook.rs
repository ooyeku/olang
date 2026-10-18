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

/// What a pinned output block holds: `text` (a value's words, a string
/// bare), `table` (a table's columns aligned: names, types, a rule, its
/// first rows, its size), `chart` (the chart's SVG, its description
/// inside).
pub const OUTPUT_KINDS: &[&str] = &["text", "table", "chart"];

/// A pinned output: the block ```` ```output {cell: name, kind: …} ````
/// directly under the cell (blank lines between) whose last output it
/// keeps. olang reads it as prose: the program never sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct OutputBlock {
    /// The name of the cell it is the output of.
    pub cell: String,
    /// One of `OUTPUT_KINDS`.
    pub kind: String,
    /// The 1-based lines of its opening and closing fences.
    pub fence_line: u32,
    pub close_line: u32,
}

/// A front matter param: its name, its value as olang source, and the
/// line its `let` is placed on (the line of its key).
#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub name: String,
    pub value: String,
    pub line: u32,
    /// A control's spec, when the param is one (`{value, options}`,
    /// `{value, min, max, step}`): what a `--param` is held to.
    pub control: Option<ParamControl>,
}

/// A param that is a control: its options (as olang source), or its
/// bounds and step.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ParamControl {
    pub options: Option<Vec<String>>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub step: Option<f64>,
}

/// A literal's number, when it is one (`3`, `-2.5`).
fn literal_number(src: &str) -> Option<f64> {
    let t = src.trim();
    if t.starts_with('"') {
        return None;
    }
    t.parse::<f64>().ok()
}

/// Two literals' sources that say one value (`3` is `3.0`).
fn same_literal(a: &str, b: &str) -> bool {
    match (literal_number(a), literal_number(b)) {
        (Some(x), Some(y)) => x == y,
        _ => a.trim() == b.trim(),
    }
}

impl ParamControl {
    /// Whether `source` (an olang literal) is a value the control holds:
    /// one of its options, or a number within its bounds.
    pub fn check(&self, source: &str) -> Result<(), String> {
        if let Some(options) = &self.options
            && !options.iter().any(|o| same_literal(o, source))
        {
            return Err(format!(
                "{} is not one of its options ({})",
                source,
                options.join(", ")
            ));
        }
        if self.min.is_some() || self.max.is_some() {
            let Some(x) = literal_number(source) else {
                return Err(format!("{} is no number (the param has a range)", source));
            };
            if self.min.is_some_and(|lo| x < lo) || self.max.is_some_and(|hi| x > hi) {
                return Err(format!(
                    "{} is outside its range [{}, {}]",
                    source,
                    self.min
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "…".into()),
                    self.max
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "…".into())
                ));
            }
        }
        Ok(())
    }
}

/// A param's YAML value read as a control's spec: a map whose keys are
/// `value` and any of `options`, `min`, `max`, `step` — and no other. None
/// for any other value (a Map param stays one).
fn control_spec(node: &yaml::Node) -> Option<Result<(String, ParamControl), Diagnostic>> {
    let yaml::Value::Map(entries) = &node.value else {
        return None;
    };
    let keys = ["value", "options", "min", "max", "step"];
    if !entries.iter().any(|e| e.key == "value")
        || entries.iter().any(|e| !keys.contains(&e.key.as_str()))
        || entries.len() < 2
    {
        return None;
    }
    let get = |k: &str| entries.iter().find(|e| e.key == k);
    let value = get("value").expect("checked");
    let source = yaml::to_olang(&value.value.value);
    let num = |e: &yaml::Entry| match e.value.value {
        yaml::Value::Int(i) => Ok(i as f64),
        yaml::Value::Float(f) => Ok(f),
        _ => Err(Diagnostic::at(
            e.value.line,
            e.value.column,
            format!("a control param's `{}` is a number", e.key),
        )),
    };
    let mut ctl = ParamControl::default();
    if let Some(o) = get("options") {
        let yaml::Value::List(items) = &o.value.value else {
            return Some(Err(Diagnostic::at(
                o.value.line,
                o.value.column,
                "a control param's `options` is a list",
            )));
        };
        ctl.options = Some(items.iter().map(|n| yaml::to_olang(&n.value)).collect());
    }
    for (k, slot) in [
        ("min", &mut ctl.min),
        ("max", &mut ctl.max),
        ("step", &mut ctl.step),
    ] {
        if let Some(e) = get(k) {
            match num(e) {
                Ok(x) => *slot = Some(x),
                Err(d) => return Some(Err(d)),
            }
        }
    }
    if let Err(m) = ctl.check(&source) {
        return Some(Err(Diagnostic::at(
            value.value.line,
            value.value.column,
            format!("the param's value {}", m),
        )));
    }
    Some(Ok((source, ctl)))
}

/// A notebook's structure: its cells, its front matter, and what is wrong
/// with them.
#[derive(Debug, Clone, Default)]
pub struct Notebook {
    pub cells: Vec<Cell>,
    /// The pinned outputs, in the file's order.
    pub outputs: Vec<OutputBlock>,
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

/// `--for`'s parameter sets, each a list of `(NAME, VALUE)` as `--param`
/// takes them: `NAME=A,B,C` (every value of one param or control, split at
/// the commas outside quotes and brackets), or a file of sets — `.json` (a
/// list of objects), `.yaml`/`.yml` (a list of maps, front matter's YAML
/// subset) or `.csv` (a header of names, a row a set).
pub fn param_sets(spec: &str) -> Result<Vec<Vec<(String, String)>>, String> {
    let word = |w: &str| {
        !w.is_empty()
            && w.chars().all(|c| c.is_alphanumeric() || c == '_')
            && !w.starts_with(|c: char| c.is_ascii_digit())
    };
    if let Some((name, values)) = spec.split_once('=')
        && word(name.trim())
    {
        let vals = split_top(values);
        if vals.iter().all(|v| v.trim().is_empty()) {
            return Err(format!("--for {}: no values (NAME=A,B,C)", name.trim()));
        }
        return Ok(vals
            .into_iter()
            .map(|v| vec![(name.trim().to_string(), v.trim().to_string())])
            .collect());
    }
    let path = Path::new(spec);
    let text = std::fs::read_to_string(path).map_err(|e| {
        format!(
            "--for {}: neither NAME=A,B,C nor a file of sets ({})",
            spec, e
        )
    })?;
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let sets: Vec<Vec<(String, String)>> = match ext.as_str() {
        "json" => {
            let v: serde_json::Value =
                serde_json::from_str(&text).map_err(|e| format!("--for {}: {}", spec, e))?;
            let arr = v
                .as_array()
                .ok_or_else(|| format!("--for {}: a list of objects, one a set", spec))?;
            arr.iter()
                .map(|o| {
                    o.as_object()
                        .map(|m| m.iter().map(|(k, x)| (k.clone(), json_source(x))).collect())
                        .ok_or_else(|| {
                            format!("--for {}: each set is an object of names to values", spec)
                        })
                })
                .collect::<Result<_, _>>()?
        }
        "yaml" | "yml" => {
            let lines = lines_of(&text);
            let node = yaml::parse(&lines, 1)
                .map_err(|d| format!("--for {}:{}: {}", spec, d.line, d.message))?;
            let Some(yaml::Node {
                value: yaml::Value::List(items),
                ..
            }) = node
            else {
                return Err(format!("--for {}: a list of maps, one a set", spec));
            };
            items
                .iter()
                .map(|n| match &n.value {
                    yaml::Value::Map(es) => Ok(es
                        .iter()
                        .map(|e| (e.key.clone(), yaml::to_olang(&e.value.value)))
                        .collect()),
                    _ => Err(format!(
                        "--for {}:{}: each set is a map of names to values",
                        spec, n.line
                    )),
                })
                .collect::<Result<_, _>>()?
        }
        "csv" => {
            let mut rows = csv::ReaderBuilder::new()
                .has_headers(true)
                .from_reader(text.as_bytes());
            let names: Vec<String> = rows
                .headers()
                .map_err(|e| format!("--for {}: {}", spec, e))?
                .iter()
                .map(|h| h.trim().to_string())
                .collect();
            rows.records()
                .map(|r| {
                    r.map(|r| {
                        names
                            .iter()
                            .cloned()
                            .zip(r.iter().map(|c| c.trim().to_string()))
                            .collect()
                    })
                    .map_err(|e| format!("--for {}: {}", spec, e))
                })
                .collect::<Result<_, _>>()?
        }
        _ => {
            return Err(format!(
                "--for {}: a file of sets is .json, .yaml or .csv",
                spec
            ));
        }
    };
    if sets.is_empty() {
        return Err(format!("--for {}: no sets", spec));
    }
    for set in &sets {
        if let Some((n, _)) = set.iter().find(|(n, _)| !word(n)) {
            return Err(format!("--for {}: `{}` is no param's name", spec, n));
        }
    }
    Ok(sets)
}

/// `text` split at its commas outside quotes and brackets.
fn split_top(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    let mut esc = false;
    for c in text.chars() {
        if let Some(q) = quote {
            cur.push(c);
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '"' | '\'' => {
                quote = Some(c);
                cur.push(c);
            }
            '[' | '{' | '(' => {
                depth += 1;
                cur.push(c);
            }
            ']' | '}' | ')' => {
                depth -= 1;
                cur.push(c);
            }
            ',' if depth == 0 => out.push(std::mem::take(&mut cur)),
            c => cur.push(c),
        }
    }
    out.push(cur);
    out
}

/// A JSON value as olang source (a string quoted, an object a map).
fn json_source(v: &serde_json::Value) -> String {
    use serde_json::Value as J;
    match v {
        J::Null => "()".to_string(),
        J::String(s) => olang_string(s),
        J::Array(a) => format!(
            "[{}]",
            a.iter().map(json_source).collect::<Vec<_>>().join(", ")
        ),
        J::Object(o) => format!(
            "#{{ {} }}",
            o.iter()
                .map(|(k, x)| format!("{}: {}", olang_string(k), json_source(x)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        other => other.to_string(),
    }
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
    /// ```` ```output {cell: name, kind: …} ````: a pinned output (its
    /// cell and kind), or None when its options are wrong (a diagnostic).
    Output(Option<(String, String)>),
    Other,
}

/// Read an info string; `at` is its byte offset in `line` (for columns).
fn classify(line: &str, at: usize, line_no: u32, diags: &mut Vec<Diagnostic>) -> Kind {
    let raw = &line[at..];
    let lead = raw.len() - raw.trim_start().len();
    let info = raw.trim();
    // a pinned output: `output` and its record (a bare `output` block is
    // prose, as markdown writes a program's output)
    if let Some(rest) = info.strip_prefix("output")
        && rest.trim_start().starts_with('{')
    {
        let at_rec = at + lead + 6 + (rest.len() - rest.trim_start().len());
        return match parse_output_options(line, at_rec, line_no) {
            Ok(o) => Kind::Output(Some(o)),
            Err(d) => {
                diags.push(d);
                Kind::Output(None)
            }
        };
    }
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

/// One entry of an options record: its key, its value, and the byte
/// offsets of each in the line (for columns).
struct RecordEntry {
    key: String,
    key_at: usize,
    value: OptValue,
    value_at: usize,
}

/// The record at byte `at` of `line`: `{key: value, …}`, a value a bare
/// word, a double-quoted string, `true` or `false`; a key given twice is
/// an error. `what` names the record in messages (`the cell's options`).
fn read_record(
    line: &str,
    at: usize,
    line_no: u32,
    what: &str,
) -> Result<Vec<RecordEntry>, Diagnostic> {
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
    let mut entries: Vec<RecordEntry> = Vec::new();
    loop {
        p = skip_ws(p);
        if p >= bytes.len() {
            return Err(err(
                at,
                format!("{} record is not closed: `}}` is missing", what),
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
                format!(
                    "{} record is `{{key: value, …}}`: a key is expected here",
                    what
                ),
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
        if entries.iter().any(|e| e.key == key) {
            return Err(err(key_at, format!("the option `{}` is given twice", key)));
        }
        entries.push(RecordEntry {
            key,
            key_at,
            value,
            value_at,
        });
        p = skip_ws(p);
        if p < bytes.len() && bytes[p] == b',' {
            p += 1;
        } else if p < bytes.len() && bytes[p] != b'}' {
            return Err(err(p, format!("`,` or `}}` is expected in {}", what)));
        }
    }
    let trailing = skip_ws(p);
    if trailing < bytes.len() {
        return Err(err(trailing, format!("nothing may follow {} record", what)));
    }
    Ok(entries)
}

impl RecordEntry {
    /// The value's text (a word or a string), or None for a boolean.
    fn text(&self) -> Option<String> {
        match &self.value {
            OptValue::Word(w) | OptValue::Str(w) => Some(w.clone()),
            OptValue::Bool(_) => None,
        }
    }

    /// The value, held to one of `allowed`.
    fn one_of(&self, line: &str, line_no: u32, allowed: &[&str]) -> Result<String, Diagnostic> {
        match self.text() {
            Some(t) if allowed.contains(&t.as_str()) => Ok(t),
            _ => Err(Diagnostic::at(
                line_no,
                col(line, self.value_at),
                format!(
                    "`{}` is one of {}",
                    self.key,
                    allowed
                        .iter()
                        .map(|a| format!("`{}`", a))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            )),
        }
    }
}

/// The options record at byte `at` of `line`: every key validated; the
/// first problem is the answer.
fn parse_options(line: &str, at: usize, line_no: u32) -> Result<CellOptions, Diagnostic> {
    let err = |p: usize, m: String| Diagnostic::at(line_no, col(line, p), m);
    let mut options = CellOptions::default();
    for e in read_record(line, at, line_no, "the cell's options")? {
        match e.key.as_str() {
            "name" => match e.text() {
                Some(t) if !t.is_empty() => options.name = Some(t),
                _ => {
                    return Err(err(e.value_at, "`name` is a word or a string".into()));
                }
            },
            "hide" => options.hide = Some(e.one_of(line, line_no, &["code", "output", "all"])?),
            "run" => options.run = Some(e.one_of(line, line_no, &["auto", "manual"])?),
            "as" => options.as_ = Some(e.one_of(line, line_no, &["markdown"])?),
            "pin" => match e.value {
                OptValue::Bool(b) => options.pin = b,
                _ => return Err(err(e.value_at, "`pin` is `true` or `false`".into())),
            },
            other => {
                return Err(err(
                    e.key_at,
                    format!(
                        "a cell has no option `{}` (its options: name, hide, run, pin, as)",
                        other
                    ),
                ));
            }
        }
    }
    Ok(options)
}

/// A pinned output's options (```` ```output {cell: total, kind: table} ````):
/// the cell it is the output of, and its kind.
fn parse_output_options(
    line: &str,
    at: usize,
    line_no: u32,
) -> Result<(String, String), Diagnostic> {
    let err = |p: usize, m: String| Diagnostic::at(line_no, col(line, p), m);
    let mut cell = None;
    let mut kind = "text".to_string();
    for e in read_record(line, at, line_no, "the output block's options")? {
        match e.key.as_str() {
            "cell" => match e.text() {
                Some(t) if !t.is_empty() => cell = Some(t),
                _ => {
                    return Err(err(
                        e.value_at,
                        "`cell` is the name of the cell above".into(),
                    ));
                }
            },
            "kind" => kind = e.one_of(line, line_no, OUTPUT_KINDS)?,
            other => {
                return Err(err(
                    e.key_at,
                    format!(
                        "an output block has no option `{}` (its options: cell, kind)",
                        other
                    ),
                ));
            }
        }
    }
    match cell {
        Some(c) => Ok((c, kind)),
        None => Err(err(
            at,
            "an output block names its cell: ```output {cell: <the cell's name>}".into(),
        )),
    }
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
        let kind = classify(line, info_at, line_no, &mut nb.diagnostics);
        if let Kind::Output(Some((cell, okind))) = &kind {
            nb.outputs.push(OutputBlock {
                cell: cell.clone(),
                kind: okind.clone(),
                fence_line: line_no,
                close_line,
            });
        }
        if let Kind::Cell(no_run, options) = kind {
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
    pinned_rules(&lines, &mut nb);
    nb
}

/// A pinned output's place: directly under its cell (blank lines
/// between), which names it and is pinned; a pinned cell has a name (its
/// block says it).
fn pinned_rules(lines: &[&str], nb: &mut Notebook) {
    for c in &nb.cells {
        if c.options.pin && c.options.name.is_none() {
            nb.diagnostics.push(Diagnostic::at(
                c.fence_line,
                1,
                "a pinned cell needs a name: its output block names it ({name: …, pin: true})",
            ));
        }
    }
    for o in &nb.outputs {
        // the nearest block above, past blank lines, must be a cell's close
        let mut k = o.fence_line as usize - 1; // 0-based index of the fence
        while k > 0 && lines[k - 1].trim().is_empty() {
            k -= 1;
        }
        let above = nb.cells.iter().find(|c| c.close_line as usize == k);
        let problem = match above {
            None => Some(format!(
                "this output block is `{}`'s, but it does not stand under a cell: it goes directly under its cell",
                o.cell
            )),
            Some(c) if c.options.name.as_deref() != Some(o.cell.as_str()) => {
                Some(match &c.options.name {
                    Some(n) => format!(
                        "this output block says `{}`, but the cell above it is `{}`",
                        o.cell, n
                    ),
                    None => format!(
                        "this output block says `{}`, but the cell above it has no name",
                        o.cell
                    ),
                })
            }
            Some(c) if !c.options.pin => Some(format!(
                "this output block keeps `{}`'s output, but the cell is not pinned (`pin: true`): pin it or remove the block",
                o.cell
            )),
            _ => None,
        };
        if let Some(m) = problem {
            nb.diagnostics.push(Diagnostic::at(o.fence_line, 1, m));
        }
    }
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
        // a control's spec binds its value (held to its options or range)
        let (value, control) = match control_spec(&p.value) {
            None => (yaml::to_olang(&p.value.value), None),
            Some(Ok((v, c))) => (v, Some(c)),
            Some(Err(d)) => {
                nb.diagnostics.push(d);
                continue;
            }
        };
        nb.params.push(Param {
            name: p.key.clone(),
            value,
            line: p.line,
            control,
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
    // the control cells a `--param` may name (a run's: `runtime.preset`)
    let control_cells: Vec<(String, u32)> = if mode == Mode::Run
        && overrides
            .iter()
            .any(|(n, _)| !nb.params.iter().any(|p| &p.name == n))
    {
        nb.cells
            .iter()
            .filter(|c| !c.no_run)
            .filter_map(|c| control_cell_name(&cell_code(&lines, c)).map(|n| (n, c.fence_line)))
            .collect()
    } else {
        Vec::new()
    };
    for (name, value) in overrides {
        match params.iter_mut().find(|p| &p.name == name) {
            Some(p) => {
                let v = param_value_source(value);
                match p.control.as_ref().map(|c| c.check(&v)) {
                    Some(Err(m)) => diagnostics.push(Diagnostic::at(
                        p.line,
                        1,
                        format!("--param {}: {}", name, m),
                    )),
                    _ => p.value = v,
                }
            }
            // a control cell's name: its control takes the value as it binds
            // (`runtime.preset` on the cell's fence line, empty in the program)
            None if control_cells.iter().any(|(n, _)| n == name) => {
                let fence = control_cells
                    .iter()
                    .find(|(n, _)| n == name)
                    .map(|c| c.1)
                    .unwrap_or(1);
                out[fence as usize - 1] = format!(
                    "runtime.preset({}, {})",
                    param_value_source(value),
                    olang_string(name)
                );
            }
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

/// The lines (1-based, the file's) where each cell's last top-level
/// statement starts: the cell's value, shown under it, never a value
/// dropped (`olang check`'s `no_effect` reads the notebook as one
/// program, where only the last cell's last statement is its value).
pub fn cell_value_lines(text: &str) -> std::collections::HashSet<u32> {
    let nb = parse(text);
    let lines = lines_of(text);
    let parser = crate::parser::Parser::new();
    let mut out = std::collections::HashSet::new();
    for c in &nb.cells {
        let Ok(program) = parser.parse_raw(&cell_code(&lines, c)) else {
            continue;
        };
        if let Some(crate::ast::Statement::Located { line, .. }) = program.statements.last() {
            out.insert(c.fence_line + line);
        }
    }
    out
}

/// A cell's code: its lines between its fences.
fn cell_code(lines: &[&str], c: &Cell) -> String {
    let from = c.fence_line as usize;
    let to = (c.close_line as usize - 1).min(lines.len());
    lines[from.min(to)..to].join("\n")
}

/// The one name a control cell defines (`let region = ui.select(…)`):
/// a cell whose top-level statements define one name (its `use`s aside)
/// with a `ui` control's call (`ui.slider`, `ui.select`, `ui.date`,
/// `ui.text`, `ui.toggle`, `ui.brush`).
pub fn control_cell_name(code: &str) -> Option<String> {
    let calls = ["slider", "select", "date", "text", "toggle", "brush"];
    if !calls.iter().any(|k| code.contains(&format!("ui.{}(", k))) {
        return None;
    }
    let program = crate::parser::Parser::new().parse(code).ok()?;
    let mut names: Vec<String> = Vec::new();
    for st in &program.statements {
        for n in defined_names(st, false) {
            if !names.contains(&n) {
                names.push(n);
            }
        }
    }
    (names.len() == 1).then(|| names.remove(0))
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

/// What one cell defines and uses, read from its text without running it
/// (the kernel's `deps`): the graph an editor builds a notebook's runs on.
#[derive(Debug, Clone, PartialEq)]
pub struct CellDeps {
    /// Each top-level name the cell defines, in order: (name, line,
    /// column, whether a `use` binds it). A name a `use` binds is the
    /// cell's for the cells that read it, and no clash with another
    /// cell's `use` of it.
    pub defines: Vec<(String, u32, u32, bool)>,
    /// Each name the cell reads that none of its own statements defines
    /// (a builtin is no such name), in order of first use: (name, line,
    /// column of its first use as a word).
    pub uses: Vec<(String, u32, u32)>,
}

/// The defined and used names of a cell whose code begins on `line`
/// (1-based) of its notebook — every line and column the file's. Uses
/// are the checker's resolution (`Analyzer`), as the notebook's rules
/// read them; a name read inside a function the cell declares is a use.
pub fn cell_deps(code: &str, line: u32) -> Result<CellDeps, crate::parser::ParseError> {
    use crate::ast::Statement;
    let padded = format!("{}{}", "\n".repeat(line.max(1) as usize - 1), code);
    let program = crate::parser::Parser::new().parse_raw(&padded)?;
    let mut defines: Vec<(String, u32, u32, bool)> = Vec::new();
    for s in &program.statements {
        let (l, c) = match s {
            Statement::Located { line, column, .. } => (*line, *column),
            _ => (line, 1),
        };
        let own = defined_names(s, false);
        for name in defined_names(s, true) {
            if defines.iter().any(|d| d.0 == name) {
                continue;
            }
            let by_use = !own.contains(&name);
            defines.push((name, l, c, by_use));
        }
    }
    let mut analyzer = crate::analyze::Analyzer::new();
    let _ = analyzer.analyze_program(&program);
    let lines: Vec<&str> = padded.lines().collect();
    let last = lines.len().max(1) as u32;
    let mut uses: Vec<(String, u32, u32)> = Vec::new();
    for name in analyzer.get_undefined_variables() {
        if uses.iter().any(|u| u.0 == name) || defines.iter().any(|d| d.0 == name) {
            continue;
        }
        let (l, c) = first_use(&lines, line.max(1)..=last, &name).unwrap_or((line.max(1), 1));
        uses.push((name, l, c));
    }
    Ok(CellDeps { defines, uses })
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

/// The fence a block's body needs: three backticks, or one more than the
/// longest run of backticks at the start of any of its lines.
pub fn fence_for(body: &str) -> String {
    let longest = body
        .split('\n')
        .map(|l| {
            let t = l.trim_start_matches(' ');
            t.len() - t.trim_start_matches('`').len()
        })
        .max()
        .unwrap_or(0);
    "`".repeat(longest.max(2) + 1)
}

/// `text` with the pinned outputs given written: each `(cell, output)`
/// replaces the output block under the cell of that name (or adds one
/// under it, after a blank line), `None` removes it; every other line is
/// kept byte for byte. An output is `(kind, body)`.
pub fn with_pins(text: &str, pins: &[(String, Option<(String, String)>)]) -> String {
    let nb = parse(text);
    let raw: Vec<&str> = text.split('\n').collect();
    // (from, to, replacement): 0-based line ranges to replace, in order
    let mut edits: Vec<(usize, usize, Vec<String>)> = Vec::new();
    for (name, out) in pins {
        let Some(cell) = nb
            .cells
            .iter()
            .find(|c| c.options.name.as_deref() == Some(name.as_str()))
        else {
            continue;
        };
        let close = cell.close_line as usize; // 1-based close == 0-based next line
        let block = nb.outputs.iter().find(|o| {
            let mut k = o.fence_line as usize - 1;
            while k > 0 && raw[k - 1].trim().is_empty() {
                k -= 1;
            }
            k == close
        });
        let lines_of_block = |kind: &str, body: &str| -> Vec<String> {
            let fence = fence_for(body);
            let mut v = vec![format!(
                "{}output {{cell: {}, kind: {}}}",
                fence,
                cell_name_text(name),
                kind
            )];
            v.extend(body.trim_end_matches('\n').split('\n').map(str::to_string));
            v.push(fence);
            v
        };
        match (block, out) {
            (Some(b), Some((kind, body))) => {
                edits.push((
                    b.fence_line as usize - 1,
                    b.close_line as usize,
                    lines_of_block(kind, body),
                ));
            }
            (Some(b), None) => {
                // the block and the blank lines between it and its cell
                edits.push((close, b.close_line as usize, Vec::new()));
            }
            (None, Some((kind, body))) => {
                let mut v = vec![String::new()];
                v.extend(lines_of_block(kind, body));
                edits.push((close, close, v));
            }
            (None, None) => {}
        }
    }
    edits.sort_by_key(|e| e.0);
    let mut out: Vec<String> = Vec::with_capacity(raw.len());
    let mut at = 0usize;
    for (from, to, rep) in edits {
        if from < at {
            continue;
        }
        while at < from && at < raw.len() {
            out.push(raw[at].to_string());
            at += 1;
        }
        out.extend(rep);
        at = to.min(raw.len());
    }
    while at < raw.len() {
        out.push(raw[at].to_string());
        at += 1;
    }
    out.join("\n")
}

/// A cell's name as an option's value: bare when it is a word, else quoted.
fn cell_name_text(name: &str) -> String {
    if !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
        && !matches!(name, "true" | "false")
    {
        name.to_string()
    } else {
        format!("\"{}\"", name.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

/// Each pinned output's body, by its cell's name: the lines between its
/// fences.
pub fn pinned_outputs(text: &str) -> Vec<(OutputBlock, String)> {
    let nb = parse(text);
    let raw = lines_of(text);
    nb.outputs
        .iter()
        .map(|o| {
            let from = o.fence_line as usize; // 0-based first body line
            let to = (o.close_line as usize - 1).min(raw.len());
            (o.clone(), raw[from.min(to)..to].join("\n"))
        })
        .collect()
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

    #[test]
    fn a_param_may_be_a_controls_spec() {
        let text = "---\nparams:\n  since: {value: \"30 days ago\", options: [\"7 days ago\", \"30 days ago\", \"90 days ago\"]}\n  top_n: {value: 3, min: 1, max: 10, step: 1}\n  limits: {low: 1, value: 2}\n  only: {value: 5}\n---\n```olang\nlet a = [since, top_n, limits, only]\n```\n";
        let nb = parse(text);
        assert!(nb.diagnostics.is_empty(), "{:?}", nb.diagnostics);
        assert_eq!(nb.params[0].value, "\"30 days ago\"");
        assert_eq!(
            nb.params[0]
                .control
                .as_ref()
                .unwrap()
                .options
                .as_ref()
                .unwrap()
                .len(),
            3
        );
        assert_eq!(nb.params[1].value, "3");
        assert_eq!(nb.params[1].control.as_ref().unwrap().max, Some(10.0));
        // any other map stays a Map param, a lone value too
        assert!(nb.params[2].control.is_none() && nb.params[2].value.starts_with("#{"));
        assert!(nb.params[3].control.is_none());
        // --param is held to the control
        let ok = program_with(
            text,
            Mode::Run,
            &[
                ("top_n".into(), "7".into()),
                ("since".into(), "7 days ago".into()),
            ],
        );
        assert!(ok.diagnostics.is_empty(), "{:?}", ok.diagnostics);
        assert!(
            ok.source.contains("let top_n = 7") && ok.source.contains("let since = \"7 days ago\"")
        );
        let bad = program_with(
            text,
            Mode::Run,
            &[
                ("top_n".into(), "11".into()),
                ("since".into(), "a year ago".into()),
            ],
        );
        assert_eq!(bad.diagnostics.len(), 2, "{:?}", bad.diagnostics);
        assert!(
            bad.diagnostics.iter().any(|d| d
                .message
                .contains("--param top_n: 11 is outside its range [1, 10]")),
            "{:?}",
            bad.diagnostics
        );
        assert!(bad.diagnostics.iter().any(|d| {
            d.message
                .contains("--param since: \"a year ago\" is not one of its options")
        }));
        // a value outside its own spec is a diagnostic at the value
        let wrong = parse(
            "---\nparams:\n  n: {value: 12, min: 1, max: 10}\n  r: {value: north, options: [east, west]}\n---\n",
        );
        assert_eq!(wrong.diagnostics.len(), 2, "{:?}", wrong.diagnostics);
        assert!(
            wrong.diagnostics[0]
                .message
                .contains("the param's value 12 is outside its range")
        );
        assert_eq!(wrong.diagnostics[0].line, 3);
        assert!(wrong.params.is_empty());
    }

    #[test]
    fn a_pinned_output_stands_under_its_cell() {
        let text = "# T\n\n```olang {name: total, pin: true}\nlet total = 3\ntotal\n```\n\n```output {cell: total}\n3\n```\n\nprose\n";
        let nb = parse(text);
        assert!(nb.diagnostics.is_empty(), "{:?}", nb.diagnostics);
        assert_eq!(nb.outputs.len(), 1);
        assert_eq!(nb.outputs[0].cell, "total");
        assert_eq!(nb.outputs[0].kind, "text");
        assert_eq!(
            (nb.outputs[0].fence_line, nb.outputs[0].close_line),
            (8, 10)
        );
        // the program never sees it: its lines are empty
        let p = program_with(text, Mode::Run, &[]);
        assert!(p.diagnostics.is_empty(), "{:?}", p.diagnostics);
        assert_eq!(p.source.split('\n').nth(8), Some(""));
        assert_eq!(pinned_outputs(text)[0].1, "3");
        // a bare `output` block is prose, as markdown writes a program's output
        assert!(parse("```output\nhello\n```\n").diagnostics.is_empty());
    }

    #[test]
    fn a_pinned_output_out_of_place_is_a_diagnostic() {
        let msgs = |t: &str| {
            parse(t)
                .diagnostics
                .into_iter()
                .map(|d| (d.line, d.message))
                .collect::<Vec<_>>()
        };
        let unnamed = msgs("```olang {pin: true}\n1\n```\n");
        assert!(
            unnamed[0].1.contains("a pinned cell needs a name"),
            "{:?}",
            unnamed
        );
        let stray = msgs("prose\n\n```output {cell: x}\n1\n```\n");
        assert!(
            stray[0].1.contains("does not stand under a cell"),
            "{:?}",
            stray
        );
        let other = msgs("```olang {name: y, pin: true}\n1\n```\n```output {cell: x}\n1\n```\n");
        assert_eq!(other[0].0, 4);
        assert!(
            other[0].1.contains("the cell above it is `y`"),
            "{:?}",
            other
        );
        let unpinned = msgs("```olang {name: x}\n1\n```\n```output {cell: x}\n1\n```\n");
        assert!(unpinned[0].1.contains("not pinned"), "{:?}", unpinned);
        let nocell =
            msgs("```olang {name: x, pin: true}\n1\n```\n```output {kind: text}\n1\n```\n");
        assert!(nocell[0].1.contains("names its cell"), "{:?}", nocell);
        let kind = msgs(
            "```olang {name: x, pin: true}\n1\n```\n```output {cell: x, kind: movie}\n1\n```\n",
        );
        assert!(kind[0].1.contains("`kind` is one of"), "{:?}", kind);
    }

    #[test]
    fn pins_are_written_replaced_and_removed() {
        let text = "```olang {name: a, pin: true}\n1\n```\n\nprose\n\n```olang {name: b, pin: true}\n\"x\"\n```\n";
        let one = with_pins(
            text,
            &[
                ("a".into(), Some(("text".into(), "1".into()))),
                ("b".into(), Some(("text".into(), "```\nfenced".into()))),
            ],
        );
        assert_eq!(
            one,
            "```olang {name: a, pin: true}\n1\n```\n\n```output {cell: a, kind: text}\n1\n```\n\nprose\n\n```olang {name: b, pin: true}\n\"x\"\n```\n\n````output {cell: b, kind: text}\n```\nfenced\n````\n"
        );
        assert!(
            parse(&one).diagnostics.is_empty(),
            "{:?}",
            parse(&one).diagnostics
        );
        let two = with_pins(
            &one,
            &[("a".into(), Some(("table".into(), "x  y\n1  2".into())))],
        );
        assert!(
            two.contains("```output {cell: a, kind: table}\nx  y\n1  2\n```\n\nprose"),
            "{}",
            two
        );
        let gone = with_pins(&two, &[("a".into(), None), ("b".into(), None)]);
        assert_eq!(gone, text);
        assert_eq!(with_pins(text, &[("nobody".into(), None)]), text);
    }

    #[test]
    fn a_param_may_name_a_control_cell() {
        let text = "```olang {name: region}\nuse ui\nlet region = ui.select(\"east\", [\"east\", \"west\"])\n```\n\n```olang\nlet x = region\n```\n";
        let p = program_with(text, Mode::Run, &[("region".into(), "west".into())]);
        assert!(p.diagnostics.is_empty(), "{:?}", p.diagnostics);
        assert_eq!(
            p.source.split('\n').next(),
            Some("runtime.preset(\"west\", \"region\")")
        );
        assert_eq!(
            control_cell_name("use ui\nlet lift = ui.slider(0.08, 0.0, 0.3, 0.01)").as_deref(),
            Some("lift")
        );
        assert_eq!(
            control_cell_name("let a = ui.slider(1, 0, 2, 1)\nlet b = 2"),
            None
        );
        let none = program_with(text, Mode::Run, &[("nobody".into(), "1".into())]);
        assert!(
            none.diagnostics[0].message.contains("no such param")
                || none.diagnostics[0].message.contains("declares no params"),
            "{:?}",
            none.diagnostics
        );
    }

    #[test]
    fn param_sets_from_a_list_or_a_file() {
        let sets = param_sets("region=east,west,\"a, b\",[1, 2]").unwrap();
        assert_eq!(sets.len(), 4);
        assert_eq!(
            sets[2],
            vec![("region".to_string(), "\"a, b\"".to_string())]
        );
        assert_eq!(sets[3][0].1, "[1, 2]");
        let dir = std::env::temp_dir().join(format!("olang-for-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let j = dir.join("sets.json");
        std::fs::write(
            &j,
            r#"[{"region": "east", "top_n": 2}, {"region": "west", "top_n": 5}]"#,
        )
        .unwrap();
        let js = param_sets(j.to_str().unwrap()).unwrap();
        assert_eq!(
            js[1],
            vec![
                ("region".to_string(), "\"west\"".to_string()),
                ("top_n".to_string(), "5".to_string())
            ]
        );
        let y = dir.join("sets.yaml");
        std::fs::write(
            &y,
            "- region: east\n  top_n: 2\n- region: north\n  top_n: 4\n",
        )
        .unwrap();
        assert_eq!(
            param_sets(y.to_str().unwrap()).unwrap()[1][0],
            ("region".to_string(), "\"north\"".to_string())
        );
        let c = dir.join("sets.csv");
        std::fs::write(&c, "region,top_n\neast,1\nsouth,3\n").unwrap();
        assert_eq!(
            param_sets(c.to_str().unwrap()).unwrap()[1],
            vec![
                ("region".to_string(), "south".to_string()),
                ("top_n".to_string(), "3".to_string())
            ]
        );
        assert!(param_sets("nothing-here.toml").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
