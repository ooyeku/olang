//! Regular expression module (`re`), backed by the `regex` crate.
//!
//! Every operation but `is_valid` can fail on a malformed pattern, so it
//! returns a `Result` — the olang programmer decides how to handle a bad
//! pattern rather than the program aborting. `is_valid` is total (it answers
//! the question directly) so it returns a bare boolean.

use crate::ast::Value;
use regex::Regex;
use std::sync::Arc;

/// Creates the regex module.
pub fn create_regex_module() -> Value {
    let mut module = crate::ast::ValueMap::default();

    module.insert(
        "is_valid".to_string(),
        create_builtin_function("is_valid", 1),
    );
    module.insert(
        "is_match".to_string(),
        create_builtin_function("is_match", 2),
    );
    module.insert("find".to_string(), create_builtin_function("find", 2));
    module.insert(
        "find_all".to_string(),
        create_builtin_function("find_all", 2),
    );
    module.insert(
        "find_spans".to_string(),
        create_builtin_function("find_spans", 2),
    );
    module.insert(
        "captures".to_string(),
        create_builtin_function("captures", 2),
    );
    module.insert("split".to_string(), create_builtin_function("split", 2));
    module.insert("replace".to_string(), create_builtin_function("replace", 3));
    module.insert(
        "replace_first".to_string(),
        create_builtin_function("replace_first", 3),
    );

    Value::Struct {
        type_name: "Module".to_string(),
        fields: std::sync::Arc::new(module),
    }
}

fn create_builtin_function(name: &str, arity: usize) -> Value {
    Value::Builtin(crate::ast::BuiltinFunction {
        name: format!("re.{}", name),
        arity,
    })
}

/// Dispatcher for `re` functions.
/// The most arguments each function takes (stdlib.md rule 3: a call
/// with more raises, rather than dropping them). Kept equal to the
/// functions' help (`help_and_max_args_agree`, OL-334).
pub(crate) const MAX_ARGS: &[(&str, usize)] = &[
    ("is_valid", 1),
    ("is_match", 2),
    ("find_spans", 2),
    ("find", 2),
    ("find_all", 2),
    ("captures", 2),
    ("split", 2),
    ("replace", 3),
    ("replace_first", 3),
];

pub fn call_regex_function(
    name: &str,
    args: Vec<Value>,
) -> Result<Value, Box<dyn std::error::Error>> {
    if let Some(e) = super::too_many_args("re", name, args.len(), MAX_ARGS) {
        return Err(e.into());
    }
    match name {
        "is_valid" => re_is_valid(args),
        "is_match" => re_is_match(args),
        "find_spans" => re_find_spans(args),
        "find" => re_find(args),
        "find_all" => re_find_all(args),
        "captures" => re_captures(args),
        "split" => re_split(args),
        "replace" => re_replace(args, "replace", true),
        "replace_first" => re_replace(args, "replace_first", false),
        _ => Err(format!("Unknown re function: {}", name).into()),
    }
}

fn arg_str<'a>(
    args: &'a [Value],
    i: usize,
    func: &str,
) -> Result<&'a str, Box<dyn std::error::Error>> {
    match args.get(i) {
        Some(Value::String(s)) => Ok(s.as_ref()),
        Some(other) => Err(format!(
            "re.{}: argument {} must be a string, got {}",
            func,
            i + 1,
            other.type_name()
        )
        .into()),
        None => Err(format!("re.{}: missing argument {}", func, i + 1).into()),
    }
}

fn ok(v: Value) -> Value {
    Value::Ok(Box::new(v))
}

fn err(msg: String) -> Value {
    Value::Err(Box::new(Value::String(Arc::new(msg))))
}

/// Patterns compiled on this thread, by their text: a `re` call compiled
/// its pattern every time (~28 µs for a character class against 0.1 µs
/// for `str.contains`; Heddle's painter asked thousands of times a
/// frame). A compiled `Regex` is shared by reference, so a hit is a
/// clone of a pointer. Bounded: past `PATTERNS_KEPT` the cache starts
/// over (a program building patterns from data does not grow it).
const PATTERNS_KEPT: usize = 256;

thread_local! {
    static PATTERNS: std::cell::RefCell<rustc_hash::FxHashMap<String, Regex>> =
        std::cell::RefCell::new(rustc_hash::FxHashMap::default());
}

/// Compile a pattern (or take it compiled), mapping a failure to an olang
/// `Err` rather than a runtime abort.
fn compile(pattern: &str) -> Result<Regex, Value> {
    if let Some(r) = PATTERNS.with(|p| p.borrow().get(pattern).cloned()) {
        return Ok(r);
    }
    let r = Regex::new(pattern).map_err(|e| err(format!("invalid pattern: {}", e)))?;
    PATTERNS.with(|p| {
        let mut p = p.borrow_mut();
        if p.len() >= PATTERNS_KEPT {
            p.clear();
        }
        p.insert(pattern.to_string(), r.clone());
    });
    Ok(r)
}

fn string(s: &str) -> Value {
    Value::String(Arc::new(s.to_string()))
}

// ── operations ──────────────────────────────────────────────────────
/// Total: does the pattern compile at all?
fn re_is_valid(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let pattern = arg_str(&args, 0, "is_valid")?;
    Ok(Value::Boolean(compile(pattern).is_ok()))
}

fn re_is_match(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let pattern = arg_str(&args, 0, "is_match")?;
    let text = arg_str(&args, 1, "is_match")?;
    let re = match compile(pattern) {
        Ok(re) => re,
        Err(e) => return Ok(e),
    };
    Ok(ok(Value::Boolean(re.is_match(text))))
}

/// First match as `Ok(string)`, or `Ok(())` when there is no match — the
/// pattern was valid, so this is a success that found nothing.
fn re_find(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let pattern = arg_str(&args, 0, "find")?;
    let text = arg_str(&args, 1, "find")?;
    let re = match compile(pattern) {
        Ok(re) => re,
        Err(e) => return Ok(e),
    };
    Ok(ok(re
        .find(text)
        .map(|m| string(m.as_str()))
        .unwrap_or(Value::Unit)))
}

fn re_find_all(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let pattern = arg_str(&args, 0, "find_all")?;
    let text = arg_str(&args, 1, "find_all")?;
    let re = match compile(pattern) {
        Ok(re) => re,
        Err(e) => return Ok(e),
    };
    let matches: Vec<Value> = re.find_iter(text).map(|m| string(m.as_str())).collect();
    Ok(ok(Value::List(matches.into())))
}

/// Where every match is: `(start, end)` character offsets, in order —
/// what an editor's find highlights (a match's text alone does not say
/// where it was).
fn re_find_spans(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let pattern = arg_str(&args, 0, "find_spans")?;
    let text = arg_str(&args, 1, "find_spans")?;
    let re = match compile(pattern) {
        Ok(re) => re,
        Err(e) => return Ok(e),
    };
    let ascii = text.is_ascii();
    let mut out = Vec::new();
    // byte offsets to characters, walking forward once
    let (mut at_b, mut at_c) = (0usize, 0usize);
    let mut chars = |b: usize| {
        if ascii {
            return b;
        }
        at_c += text[at_b..b].chars().count();
        at_b = b;
        at_c
    };
    for m in re.find_iter(text) {
        let a = chars(m.start());
        let b = chars(m.end());
        out.push(Value::Tuple(Arc::new(vec![
            Value::Integer(a as i64),
            Value::Integer(b as i64),
        ])));
    }
    Ok(ok(Value::List(out.into())))
}

/// Capture groups of the first match: index 0 is the whole match, then each
/// group, `()` for a group that took no part in the match. `Ok(())` when
/// there is no match.
fn re_captures(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let pattern = arg_str(&args, 0, "captures")?;
    let text = arg_str(&args, 1, "captures")?;
    let re = match compile(pattern) {
        Ok(re) => re,
        Err(e) => return Ok(e),
    };
    Ok(ok(match re.captures(text) {
        Some(caps) => Value::List(
            caps.iter()
                .map(|g| g.map(|m| string(m.as_str())).unwrap_or(Value::Unit))
                .collect::<Vec<_>>()
                .into(),
        ),
        None => Value::Unit,
    }))
}

fn re_split(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let pattern = arg_str(&args, 0, "split")?;
    let text = arg_str(&args, 1, "split")?;
    let re = match compile(pattern) {
        Ok(re) => re,
        Err(e) => return Ok(e),
    };
    let parts: Vec<Value> = re.split(text).map(string).collect();
    Ok(ok(Value::List(parts.into())))
}

/// A `$name` in a replacement that names no group of the pattern: the
/// regex reads the longest name after `$`, so `$1_n` is a group called
/// `1_n`, which would put nothing there and lose the text silently
/// (OL-341). Said, with the braces that mean what was likely meant.
fn missing_group(re: &Regex, rep: &str) -> Option<String> {
    let b = rep.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'$' {
            i += 1;
            continue;
        }
        // `$$` is a literal `$`; `${name}` is braced and taken as written
        if i + 1 < b.len() && (b[i + 1] == b'$' || b[i + 1] == b'{') {
            i += 2;
            continue;
        }
        let start = i + 1;
        let mut end = start;
        while end < b.len() && (b[end].is_ascii_alphanumeric() || b[end] == b'_') {
            end += 1;
        }
        if end > start {
            let name = &rep[start..end];
            let known = match name.parse::<usize>() {
                Ok(n) => n < re.captures_len(),
                Err(_) => re.capture_names().flatten().any(|g| g == name),
            };
            if !known {
                let digits: String = name.chars().take_while(|c| c.is_ascii_digit()).collect();
                let hint = if !digits.is_empty() && digits.len() < name.len() {
                    format!(
                        "; for group {digits} then `{}`, write `${{{digits}}}{}`",
                        &name[digits.len()..],
                        &name[digits.len()..]
                    )
                } else {
                    String::new()
                };
                return Some(format!(
                    "`${name}` in the replacement names no group of the pattern{hint}"
                ));
            }
        }
        i = end.max(i + 1);
    }
    None
}

fn re_replace(
    args: Vec<Value>,
    func: &str,
    all: bool,
) -> Result<Value, Box<dyn std::error::Error>> {
    let pattern = arg_str(&args, 0, func)?;
    let text = arg_str(&args, 1, func)?;
    let replacement = arg_str(&args, 2, func)?;
    let re = match compile(pattern) {
        Ok(re) => re,
        Err(e) => return Ok(e),
    };
    if let Some(why) = missing_group(&re, replacement) {
        return Ok(err(format!("{func}: {why}")));
    }
    let result = if all {
        re.replace_all(text, replacement).into_owned()
    } else {
        re.replace(text, replacement).into_owned()
    };
    Ok(ok(string(&result)))
}
