//! `fs.scan` and `fs.search`: a tree walked as a checkout is — its
//! `.gitignore`s respected, hidden entries and named folders skipped — on
//! worker threads, and its files searched for a literal or a regular
//! expression in parallel. An editor lists a workspace of thousands of
//! files and searches a quarter of a million lines with these; olang code
//! walking a tree a `list_dir` at a time cannot.

use crate::ast::{Value, ValueMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// What a walk keeps: the options both builtins take.
struct Walk {
    /// Folder names never entered (`target`, `node_modules`, …).
    skip: Vec<String>,
    /// File extensions kept (without the dot); empty keeps every file.
    exts: Vec<String>,
    /// Hidden entries (a name starting with `.`) walked too.
    hidden: bool,
    /// `.gitignore` (and `.git/info/exclude`) respected, in any folder.
    gitignore: bool,
    /// At most this many files.
    limit: usize,
    /// At most this deep below the root (`None`: any depth).
    max_depth: Option<usize>,
}

impl Default for Walk {
    fn default() -> Self {
        Walk { skip: Vec::new(), exts: Vec::new(), hidden: false, gitignore: true, limit: 100_000, max_depth: None }
    }
}

fn err(msg: String) -> Value {
    Value::Err(Box::new(Value::String(Arc::new(msg))))
}

fn fields_of(v: &Value) -> Option<ValueMap> {
    match v {
        Value::Map(m) => Some(m.as_ref().clone()),
        Value::Struct { fields, .. } => Some(fields.as_ref().clone()),
        Value::Unit => Some(ValueMap::default()),
        _ => None,
    }
}

fn strings(v: &Value, who: &str, key: &str) -> Result<Vec<String>, Value> {
    match v {
        Value::List(xs) => xs
            .iter()
            .map(|x| match x {
                Value::String(s) => Ok(s.as_ref().clone()),
                other => Err(err(format!("{}: {} must be a list of strings, got a {}", who, key, other.type_name()))),
            })
            .collect(),
        Value::Unit => Ok(Vec::new()),
        other => Err(err(format!("{}: {} must be a list of strings, got {}", who, key, other.type_name()))),
    }
}

fn count(v: &Value, who: &str, key: &str) -> Result<usize, Value> {
    match v {
        Value::Integer(n) if *n >= 0 => Ok(*n as usize),
        other => Err(err(format!("{}: {} must be a count, got {}", who, key, other.type_name()))),
    }
}

fn flag(v: &Value, who: &str, key: &str) -> Result<bool, Value> {
    match v {
        Value::Boolean(b) => Ok(*b),
        other => Err(err(format!("{}: {} must be true or false, got {}", who, key, other.type_name()))),
    }
}

/// The walk's options from `opts`; the keys left for the caller in `rest`.
fn walk_opts(opts: &ValueMap, who: &str, extra: &[&str]) -> Result<Walk, Value> {
    let mut w = Walk::default();
    for (k, v) in opts {
        match k.as_str() {
            "skip" => w.skip = strings(v, who, "skip")?,
            "exts" => w.exts = strings(v, who, "exts")?.into_iter().map(|e| e.trim_start_matches('.').to_lowercase()).collect(),
            "hidden" => w.hidden = flag(v, who, "hidden")?,
            "gitignore" => w.gitignore = flag(v, who, "gitignore")?,
            "limit" if !extra.contains(&"limit") => w.limit = count(v, who, "limit")?,
            "files_limit" => w.limit = count(v, who, "files_limit")?,
            "max_depth" => w.max_depth = Some(count(v, who, "max_depth")?),
            other if extra.contains(&other) => {}
            other => {
                return Err(err(format!(
                    "{}: unknown option '{}' (supported: skip, exts, hidden, gitignore, {}max_depth{}{})",
                    who,
                    other,
                    if extra.contains(&"limit") { "files_limit, " } else { "limit, " },
                    if extra.is_empty() { "" } else { ", " },
                    extra.join(", ")
                )))
            }
        }
    }
    Ok(w)
}

/// The files under `root` the walk keeps, relative to it with `/`
/// between folders, sorted.
fn walk_files(root: &Path, w: &Walk) -> Vec<String> {
    let mut b = ignore::WalkBuilder::new(root);
    b.hidden(!w.hidden)
        .parents(w.gitignore)
        .git_ignore(w.gitignore)
        .git_exclude(w.gitignore)
        .git_global(false)
        .ignore(false)
        .require_git(false)
        .follow_links(false)
        .max_depth(w.max_depth)
        .threads(num_cpus::get().clamp(1, 8));
    let skip = Arc::new(w.skip.clone());
    b.filter_entry(move |e| {
        let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
        !(is_dir && e.depth() > 0 && e.file_name().to_str().map(|n| skip.iter().any(|s| s == n)).unwrap_or(false))
    });
    let found: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let exts = Arc::new(w.exts.clone());
    let root_buf: Arc<PathBuf> = Arc::new(root.to_path_buf());
    let limit = w.limit;
    b.build_parallel().run(|| {
        let found = found.clone();
        let exts = exts.clone();
        let root_buf = root_buf.clone();
        Box::new(move |entry| {
            let e = match entry {
                Ok(e) => e,
                Err(_) => return ignore::WalkState::Continue,
            };
            if !e.file_type().map(|t| t.is_file()).unwrap_or(false) {
                return ignore::WalkState::Continue;
            }
            let path = e.path();
            if !exts.is_empty() {
                let ext = path.extension().and_then(|x| x.to_str()).map(|x| x.to_lowercase()).unwrap_or_default();
                if !exts.iter().any(|x| *x == ext) {
                    return ignore::WalkState::Continue;
                }
            }
            let rel = match path.strip_prefix(root_buf.as_ref()) {
                Ok(r) => r,
                Err(_) => return ignore::WalkState::Continue,
            };
            let rel = match rel.to_str() {
                Some(s) => s.replace(std::path::MAIN_SEPARATOR, "/"),
                None => return ignore::WalkState::Continue,
            };
            let mut f = found.lock().unwrap();
            if f.len() >= limit {
                return ignore::WalkState::Quit;
            }
            f.push(rel);
            ignore::WalkState::Continue
        })
    });
    let mut out = Arc::try_unwrap(found).map(|m| m.into_inner().unwrap()).unwrap_or_else(|a| a.lock().unwrap().clone());
    out.sort();
    out.truncate(limit);
    out
}

fn root_arg(args: &[Value], who: &str) -> Result<PathBuf, Value> {
    match args.first() {
        Some(Value::String(s)) => {
            let p = PathBuf::from(s.as_ref());
            if p.is_dir() {
                Ok(p)
            } else {
                Err(err(format!("{}: '{}' is not a folder", who, s)))
            }
        }
        Some(other) => Err(err(format!("{}: the root must be a string path, got {}", who, other.type_name()))),
        None => Err(err(format!("{}: a root folder is required", who))),
    }
}

/// `fs.scan(root, opts = #{})`: every file under `root` a checkout keeps
/// (its `.gitignore`s respected at every level, hidden entries and the
/// folders named in `skip` left out), as paths relative to `root`, sorted.
/// `opts`: `skip` (folder names), `exts` (extensions kept), `hidden`,
/// `gitignore` (default true), `limit` (files, default 100,000),
/// `max_depth`.
pub fn scan(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.is_empty() || args.len() > 2 {
        return Err(format!("scan expects 1 or 2 arguments, got {}", args.len()).into());
    }
    let root = match root_arg(&args, "fs.scan") {
        Ok(r) => r,
        Err(e) => return Ok(e),
    };
    let opts = match args.get(1).map(fields_of) {
        None => ValueMap::default(),
        Some(Some(m)) => m,
        Some(None) => return Ok(err("fs.scan: options must be a map".to_string())),
    };
    let w = match walk_opts(&opts, "fs.scan", &[]) {
        Ok(w) => w,
        Err(e) => return Ok(e),
    };
    let files = walk_files(&root, &w);
    let list: Vec<Value> = files.into_iter().map(|p| Value::String(Arc::new(p))).collect();
    Ok(Value::Ok(Box::new(Value::List(list.into()))))
}

/// How a line is searched.
enum Needle {
    /// A literal: lower-cased when the case is ignored.
    Literal { text: String, case: bool },
    #[cfg(feature = "regex-module")]
    Regex(regex::Regex),
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The matches in one line: byte ranges.
fn line_matches(line: &str, needle: &Needle, word: bool) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    match needle {
        Needle::Literal { text, case } => {
            if text.is_empty() {
                return out;
            }
            // a case-blind search over the lower-cased line keeps byte
            // offsets only where lower-casing keeps lengths (ASCII); other
            // lines are searched character by character
            let hay: std::borrow::Cow<str> = if *case { line.into() } else if line.is_ascii() { line.to_ascii_lowercase().into() } else { line.into() };
            if !*case && !line.is_ascii() {
                let lc: Vec<(usize, char)> = line.char_indices().collect();
                let nc: Vec<char> = text.chars().collect();
                let mut i = 0;
                while i + nc.len() <= lc.len() {
                    if (0..nc.len()).all(|k| lc[i + k].1.to_lowercase().eq(nc[k].to_lowercase())) {
                        let a = lc[i].0;
                        let b = if i + nc.len() < lc.len() { lc[i + nc.len()].0 } else { line.len() };
                        out.push((a, b));
                        i += nc.len().max(1);
                    } else {
                        i += 1;
                    }
                }
            } else {
                let mut from = 0;
                while let Some(at) = hay[from..].find(text.as_str()) {
                    let a = from + at;
                    out.push((a, a + text.len()));
                    from = a + text.len().max(1);
                    if from >= hay.len() {
                        break;
                    }
                }
            }
        }
        #[cfg(feature = "regex-module")]
        Needle::Regex(rx) => {
            for m in rx.find_iter(line) {
                if m.end() > m.start() {
                    out.push((m.start(), m.end()));
                }
            }
        }
    }
    if word {
        out.retain(|(a, b)| {
            let before = line[..*a].chars().next_back();
            let after = line[*b..].chars().next();
            !before.map(is_word).unwrap_or(false) && !after.map(is_word).unwrap_or(false)
        });
    }
    out
}

/// What a match is replaced with: the template as it is for a literal;
/// a regular expression's groups (`$1`, `${name}`, `$$` a dollar) taken
/// from the match at `at` in `line`.
fn replaced_with(line: &str, at: usize, needle: &Needle, template: &str) -> String {
    match needle {
        Needle::Literal { .. } => template.to_string(),
        #[cfg(feature = "regex-module")]
        Needle::Regex(rx) => match rx.captures_at(line, at) {
            Some(caps) => {
                let mut out = String::new();
                caps.expand(template, &mut out);
                out
            }
            None => template.to_string(),
        },
    }
}

/// One file's matches: `(line, start, end, text, with)` with columns in
/// characters, at most `cap`; `with` what the match becomes when
/// `replace` is given.
fn search_file(path: &Path, needle: &Needle, word: bool, cap: usize, replace: Option<&str>) -> Vec<(usize, usize, usize, String, Option<String>)> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(_) => return Vec::new(),
    };
    // a binary file (a NUL early on) is not searched
    if bytes[..bytes.len().min(8192)].contains(&0) {
        return Vec::new();
    }
    let text = match std::str::from_utf8(&bytes) {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };
    // a quick test of the whole file first: most files hold no match
    if let Needle::Literal { text: t, case: true } = needle {
        if !text.contains(t.as_str()) {
            return Vec::new();
        }
    }
    #[cfg(feature = "regex-module")]
    if let Needle::Regex(rx) = needle {
        if !rx.is_match(text) {
            return Vec::new();
        }
    }
    let mut out = Vec::new();
    for (i, line) in text.split('\n').enumerate() {
        let line = line.strip_suffix('\r').unwrap_or(line);
        for (a, b) in line_matches(line, needle, word) {
            let ca = line[..a].chars().count();
            let cb = ca + line[a..b].chars().count();
            // the line as it is shown: at most 400 characters
            let shown: String = if line.len() > 400 { line.chars().take(400).collect() } else { line.to_string() };
            let with = replace.map(|tpl| replaced_with(line, a, needle, tpl));
            out.push((i, ca, cb, shown, with));
            if out.len() >= cap {
                return out;
            }
        }
    }
    out
}

/// `fs.search(root, query, opts = #{})`: every line under `root` holding
/// `query`, searched on worker threads: `Ok([#{ path, line, col, end, text }])`
/// (`path` relative to `root`, `line` from 0, `col`/`end` in characters,
/// `text` the line), by path then line. The files are walked as `fs.scan`
/// walks them, or are `opts.files` (paths relative to `root`). `opts`:
/// `regex`, `case` (match case; default false), `word` (whole words),
/// `limit` (matches, default 2,000), `per_file` (default 200),
/// `replace` (a template: each match also says `with`, what it becomes —
/// a regular expression's groups expanded, `$1`, `${name}`, `$$`), and
/// `fs.scan`'s `skip`, `exts`, `hidden`, `gitignore`, `max_depth`.
pub fn search(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() < 2 || args.len() > 3 {
        return Err(format!("search expects 2 or 3 arguments, got {}", args.len()).into());
    }
    let root = match root_arg(&args, "fs.search") {
        Ok(r) => r,
        Err(e) => return Ok(e),
    };
    let query = match &args[1] {
        Value::String(s) => s.as_ref().clone(),
        other => return Ok(err(format!("fs.search: the query must be a string, got {}", other.type_name()))),
    };
    let opts = match args.get(2).map(fields_of) {
        None => ValueMap::default(),
        Some(Some(m)) => m,
        Some(None) => return Ok(err("fs.search: options must be a map".to_string())),
    };
    let extra = ["regex", "case", "word", "limit", "per_file", "files", "replace"];
    // (the walk's own `limit` is `files_limit` here: `limit` counts matches)
    let w = match walk_opts(&opts, "fs.search", &extra) {
        Ok(w) => w,
        Err(e) => return Ok(e),
    };
    let get = |k: &str| opts.get(k);
    let regex_on = match get("regex").map(|v| flag(v, "fs.search", "regex")) {
        Some(Err(e)) => return Ok(e),
        Some(Ok(b)) => b,
        None => false,
    };
    let case = match get("case").map(|v| flag(v, "fs.search", "case")) {
        Some(Err(e)) => return Ok(e),
        Some(Ok(b)) => b,
        None => false,
    };
    let word = match get("word").map(|v| flag(v, "fs.search", "word")) {
        Some(Err(e)) => return Ok(e),
        Some(Ok(b)) => b,
        None => false,
    };
    let limit = match get("limit").map(|v| count(v, "fs.search", "limit")) {
        Some(Err(e)) => return Ok(e),
        Some(Ok(n)) => n,
        None => 2000,
    };
    let per_file = match get("per_file").map(|v| count(v, "fs.search", "per_file")) {
        Some(Err(e)) => return Ok(e),
        Some(Ok(n)) => n.max(1),
        None => 200,
    };
    let replace: Option<String> = match get("replace") {
        None | Some(Value::Unit) => None,
        Some(Value::String(s)) => Some(s.as_ref().clone()),
        Some(other) => return Ok(err(format!("fs.search: \"replace\" is a string, got {}", other.type_name()))),
    };
    if query.is_empty() {
        return Ok(Value::Ok(Box::new(Value::List(Vec::new().into()))));
    }
    let needle = if regex_on {
        #[cfg(feature = "regex-module")]
        {
            match regex::RegexBuilder::new(&query).case_insensitive(!case).multi_line(true).build() {
                Ok(rx) => Needle::Regex(rx),
                Err(e) => return Ok(err(format!("fs.search: bad regular expression: {}", e))),
            }
        }
        #[cfg(not(feature = "regex-module"))]
        {
            return Ok(err("fs.search: regular expressions are not in this build".to_string()));
        }
    } else {
        Needle::Literal { text: if case { query.clone() } else { query.to_lowercase() }, case }
    };
    let files: Vec<String> = match get("files") {
        Some(v) => match strings(v, "fs.search", "files") {
            Ok(fs) => fs,
            Err(e) => return Ok(e),
        },
        None => walk_files(&root, &w),
    };
    use rayon::prelude::*;
    let per: Vec<(usize, Vec<(usize, usize, usize, String, Option<String>)>)> = files
        .par_iter()
        .enumerate()
        .map(|(i, rel)| (i, search_file(&root.join(rel), &needle, word, per_file, replace.as_deref())))
        .filter(|(_, ms)| !ms.is_empty())
        .collect();
    let mut out: Vec<Value> = Vec::new();
    'files: for (i, ms) in per {
        let rel = Arc::new(files[i].clone());
        for (line, a, b, text, with) in ms {
            if out.len() >= limit {
                break 'files;
            }
            let mut m = ValueMap::default();
            m.insert("path".to_string(), Value::String(rel.clone()));
            m.insert("line".to_string(), Value::Integer(line as i64));
            m.insert("col".to_string(), Value::Integer(a as i64));
            m.insert("end".to_string(), Value::Integer(b as i64));
            m.insert("text".to_string(), Value::String(Arc::new(text)));
            if let Some(w) = with {
                m.insert("with".to_string(), Value::String(Arc::new(w)));
            }
            out.push(Value::Map(Arc::new(m)));
        }
    }
    Ok(Value::Ok(Box::new(Value::List(out.into()))))
}
