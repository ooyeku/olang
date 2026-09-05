//! String manipulation module (`str`).
//!
//! Convention: total operations (which never fail for a string argument)
//! return the bare value; only genuinely fallible ones (`parse_int`,
//! `parse_float`) return a `Result`. Wrong-type arguments are runtime
//! errors, not recoverable `Err` values — those are programming bugs.
//!
//! All indexing is by Unicode character, matching the interpreter's string
//! indexing and `len`, so multibyte text behaves intuitively.

use crate::ast::Value;
use std::collections::HashMap;
use std::sync::Arc;

/// Creates the string module.
pub fn create_string_module() -> Value {
    let mut module = HashMap::new();

    let unary = [
        "to_upper",
        "to_lower",
        "trim",
        "trim_start",
        "trim_end",
        "reverse",
        "chars",
        "graphemes",
        "lines",
        "words",
        "capitalize",
        "is_empty",
        "length",
        "parse_int",
        "parse_float",
        "char_code",
    ];
    for name in unary {
        module.insert(name.to_string(), create_builtin_function(name, 1));
    }

    let binary = [
        "contains",
        "starts_with",
        "ends_with",
        "split",
        "index_of",
        "last_index_of",
        "repeat",
        "count",
        "char_at",
        "fixed",
        "thousands",
    ];
    for name in binary {
        module.insert(name.to_string(), create_builtin_function(name, 2));
    }

    let ternary = [
        "replace",
        "replace_first",
        "substring",
        "pad_start",
        "pad_end",
    ];
    for name in ternary {
        module.insert(name.to_string(), create_builtin_function(name, 3));
    }

    // join(list, separator)
    module.insert("join".to_string(), create_builtin_function("join", 2));
    module.insert("fmt".to_string(), create_builtin_function("fmt", 1));

    Value::Struct {
        type_name: "Module".to_string(),
        fields: std::sync::Arc::new(module),
    }
}

fn create_builtin_function(name: &str, arity: usize) -> Value {
    Value::Builtin(crate::ast::BuiltinFunction {
        name: format!("str.{}", name),
        arity,
    })
}

/// Dispatcher for `str` functions.
pub fn call_string_function(
    name: &str,
    args: Vec<Value>,
) -> Result<Value, Box<dyn std::error::Error>> {
    match name {
        "to_upper" => str_to_upper(args),
        "to_lower" => str_to_lower(args),
        "trim" => str_trim(args, true, true),
        "trim_start" => str_trim(args, true, false),
        "trim_end" => str_trim(args, false, true),
        "reverse" => str_reverse(args),
        "chars" => str_chars(args),
        "graphemes" => str_graphemes(args),
        "lines" => str_lines(args),
        "words" => str_words(args),
        "capitalize" => str_capitalize(args),
        "is_empty" => str_is_empty(args),
        "length" => str_length(args),
        "parse_int" => str_parse_int(args),
        "parse_float" => str_parse_float(args),
        "contains" => str_contains(args),
        "starts_with" => str_starts_with(args),
        "ends_with" => str_ends_with(args),
        "split" => str_split(args),
        "index_of" => str_index_of(args),
        "last_index_of" => str_last_index_of(args),
        "repeat" => str_repeat(args),
        "count" => str_count(args),
        "char_at" => str_char_at(args),
        "char_code" => str_char_code(args),
        "replace" => str_replace(args, false),
        "replace_first" => str_replace(args, true),
        "substring" => str_substring(args),
        "pad_start" => str_pad(args, true),
        "pad_end" => str_pad(args, false),
        "fixed" => str_fixed(args),
        "thousands" => str_thousands(args),
        "join" => str_join(args),
        "fmt" => str_fmt(args),
        _ => Err(format!("Unknown str function: {}", name).into()),
    }
}

// ── fixed-decimal formatting ────────────────────────────────────────
// `to_string` prints the shortest round-tripping form — right for a
// value, wrong for a column: 12.5 and 12.50 must line up, 1234567.0
// wants separators, and a rounded negative must not read "-0.00".

/// The number argument of a formatting function, as f64 (Int or Float).
fn arg_number(args: &[Value], i: usize, func: &str) -> Result<f64, Box<dyn std::error::Error>> {
    match args.get(i) {
        Some(Value::Integer(n)) => Ok(*n as f64),
        Some(Value::Float(x)) => Ok(*x),
        Some(other) => {
            Err(format!("str.{}: expected a number, got {}", func, other.type_name()).into())
        }
        None => Err(format!("str.{}: expected a number", func).into()),
    }
}

fn arg_digits(args: &[Value], func: &str) -> Result<usize, Box<dyn std::error::Error>> {
    let d = arg_int(args, 1, func)?;
    if !(0..=20).contains(&d) {
        return Err(format!("str.{}: digits must be between 0 and 20, got {}", func, d).into());
    }
    Ok(d as usize)
}

/// `x` with exactly `digits` decimals, never in exponent form, and never
/// a negative zero ("-0.00" rounds to "0.00"). An Int with zero digits
/// keeps its exact digits (no f64 round trip).
fn fixed_text(v: &Value, digits: usize) -> String {
    if let (Value::Integer(n), 0) = (v, digits) {
        return n.to_string();
    }
    let x = match v {
        Value::Integer(n) => *n as f64,
        Value::Float(x) => *x,
        _ => 0.0,
    };
    let s = format!("{:.*}", digits, x);
    if s.starts_with('-') && s[1..].chars().all(|c| c == '0' || c == '.') {
        s[1..].to_string()
    } else {
        s
    }
}

/// Thousands separators in the integer part of a fixed-decimal text.
fn group_thousands(fixed: &str) -> String {
    let (sign, rest) = match fixed.strip_prefix('-') {
        Some(r) => ("-", r),
        None => ("", fixed),
    };
    let (int_part, frac) = match rest.find('.') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };
    let mut grouped = String::with_capacity(int_part.len() + int_part.len() / 3);
    for (i, c) in int_part.chars().enumerate() {
        if i > 0 && (int_part.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(c);
    }
    format!("{}{}{}", sign, grouped, frac)
}

/// str.fixed(x, digits): "1234.50".
fn str_fixed(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    arg_number(&args, 0, "fixed")?;
    let digits = arg_digits(&args, "fixed")?;
    Ok(ok_string(fixed_text(&args[0], digits)))
}

/// str.thousands(x, digits): "1,234.50".
fn str_thousands(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    arg_number(&args, 0, "thousands")?;
    let digits = arg_digits(&args, "thousands")?;
    Ok(ok_string(group_thousands(&fixed_text(&args[0], digits))))
}

// ── argument helpers ────────────────────────────────────────────────

fn arg_str<'a>(
    args: &'a [Value],
    i: usize,
    func: &str,
) -> Result<&'a str, Box<dyn std::error::Error>> {
    match args.get(i) {
        Some(Value::String(s)) => Ok(s.as_ref()),
        Some(other) => Err(format!(
            "str.{}: argument {} must be a string, got {}",
            func,
            i + 1,
            other.type_name()
        )
        .into()),
        None => Err(format!("str.{}: missing argument {}", func, i + 1).into()),
    }
}

fn arg_int(args: &[Value], i: usize, func: &str) -> Result<i64, Box<dyn std::error::Error>> {
    match args.get(i) {
        Some(Value::Integer(n)) => Ok(*n),
        Some(other) => Err(format!(
            "str.{}: argument {} must be an integer, got {}",
            func,
            i + 1,
            other.type_name()
        )
        .into()),
        None => Err(format!("str.{}: missing argument {}", func, i + 1).into()),
    }
}

fn ok_string(s: String) -> Value {
    Value::String(Arc::new(s))
}

// ── total operations (return bare values) ───────────────────────────

fn str_to_upper(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    Ok(ok_string(arg_str(&args, 0, "to_upper")?.to_uppercase()))
}

fn str_to_lower(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    Ok(ok_string(arg_str(&args, 0, "to_lower")?.to_lowercase()))
}

fn str_trim(args: Vec<Value>, start: bool, end: bool) -> Result<Value, Box<dyn std::error::Error>> {
    let s = arg_str(&args, 0, "trim")?;
    let trimmed = match (start, end) {
        (true, true) => s.trim(),
        (true, false) => s.trim_start(),
        (false, true) => s.trim_end(),
        (false, false) => s,
    };
    Ok(ok_string(trimmed.to_string()))
}

fn str_reverse(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    // By grapheme cluster, not by codepoint: reversing "\u{1F44B}\u{1F3FD}ab"
    // must keep the skin-tone modifier attached to its emoji. UAX #29
    // segmentation is what "the characters a reader sees" means.
    use unicode_segmentation::UnicodeSegmentation;
    Ok(ok_string(
        arg_str(&args, 0, "reverse")?
            .graphemes(true)
            .rev()
            .collect(),
    ))
}

/// The grapheme clusters of a string — the visible characters — as a
/// list of strings, segmented per UAX #29 (extended clusters). The
/// composable primitive for every visible-character question: count is
/// `len(str.graphemes(s))`, the nth is `str.graphemes(s)[n]`, a slice
/// is list slicing joined back with `str.join(gs, "")`.
fn str_graphemes(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    use unicode_segmentation::UnicodeSegmentation;
    let s = arg_str(&args, 0, "graphemes")?;
    let items: Vec<Value> = s
        .graphemes(true)
        .map(|g| Value::String(std::sync::Arc::new(g.to_string())))
        .collect();
    Ok(Value::List(items.into()))
}

fn str_chars(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let s = arg_str(&args, 0, "chars")?;
    let list: Vec<Value> = s.chars().map(|c| ok_string(c.to_string())).collect();
    Ok(Value::List(list.into()))
}

fn str_lines(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let s = arg_str(&args, 0, "lines")?;
    let list: Vec<Value> = s.lines().map(|l| ok_string(l.to_string())).collect();
    Ok(Value::List(list.into()))
}

fn str_words(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let s = arg_str(&args, 0, "words")?;
    let list: Vec<Value> = s
        .split_whitespace()
        .map(|w| ok_string(w.to_string()))
        .collect();
    Ok(Value::List(list.into()))
}

fn str_capitalize(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let s = arg_str(&args, 0, "capitalize")?;
    let mut chars = s.chars();
    let capped = match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    };
    Ok(ok_string(capped))
}

fn str_is_empty(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    Ok(Value::Boolean(arg_str(&args, 0, "is_empty")?.is_empty()))
}

fn str_length(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    Ok(Value::Integer(
        arg_str(&args, 0, "length")?.chars().count() as i64
    ))
}

fn str_contains(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let s = arg_str(&args, 0, "contains")?;
    let sub = arg_str(&args, 1, "contains")?;
    Ok(Value::Boolean(s.contains(sub)))
}

fn str_starts_with(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let s = arg_str(&args, 0, "starts_with")?;
    let sub = arg_str(&args, 1, "starts_with")?;
    Ok(Value::Boolean(s.starts_with(sub)))
}

fn str_ends_with(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let s = arg_str(&args, 0, "ends_with")?;
    let sub = arg_str(&args, 1, "ends_with")?;
    Ok(Value::Boolean(s.ends_with(sub)))
}

fn str_split(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let s = arg_str(&args, 0, "split")?;
    let sep = arg_str(&args, 1, "split")?;
    let parts: Vec<Value> = if sep.is_empty() {
        s.chars().map(|c| ok_string(c.to_string())).collect()
    } else {
        s.split(sep).map(|p| ok_string(p.to_string())).collect()
    };
    Ok(Value::List(parts.into()))
}

/// Character index of the first byte-match, or Unit when absent. Reported
/// in characters so it composes with char_at and substring.
///
/// Unit, not `-1`: absence in olang is Unit (the same value a missing map
/// key yields), and a `-1` sentinel was actively dangerous here because
/// indexing counts negative positions from the end — `s[index_of(s, x)]`
/// on a miss silently read the *last* character. Changed in 0.68 as part
/// of settling the absence convention; test with `idx != ()`.
fn str_index_of(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let s = arg_str(&args, 0, "index_of")?;
    let sub = arg_str(&args, 1, "index_of")?;
    Ok(match s.find(sub) {
        Some(byte_idx) => Value::Integer(s[..byte_idx].chars().count() as i64),
        None => Value::Unit,
    })
}

fn str_last_index_of(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let s = arg_str(&args, 0, "last_index_of")?;
    let sub = arg_str(&args, 1, "last_index_of")?;
    Ok(match s.rfind(sub) {
        Some(byte_idx) => Value::Integer(s[..byte_idx].chars().count() as i64),
        None => Value::Unit,
    })
}

fn str_repeat(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let s = arg_str(&args, 0, "repeat")?;
    let n = arg_int(&args, 1, "repeat")?;
    if n < 0 {
        return Err("str.repeat: count cannot be negative".into());
    }
    Ok(ok_string(s.repeat(n as usize)))
}

fn str_count(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let s = arg_str(&args, 0, "count")?;
    let sub = arg_str(&args, 1, "count")?;
    let n = if sub.is_empty() {
        0
    } else {
        s.matches(sub).count()
    };
    Ok(Value::Integer(n as i64))
}

fn str_char_at(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let s = arg_str(&args, 0, "char_at")?;
    let i = arg_int(&args, 1, "char_at")?;
    let ch = if i < 0 {
        None
    } else {
        s.chars().nth(i as usize)
    };
    // Out of range yields an empty string, so callers can probe without
    // guarding — total operation
    Ok(ok_string(ch.map(|c| c.to_string()).unwrap_or_default()))
}

/// The Unicode code point of a string's first character, as an Int —
/// the primitive under any olang-written string hash (the embedded
/// `table` module's, for one). An empty string returns Unit, the
/// absence value, so callers can probe without guarding.
fn str_char_code(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let s = arg_str(&args, 0, "char_code")?;
    Ok(match s.chars().next() {
        Some(c) => Value::Integer(c as i64),
        None => Value::Unit,
    })
}

fn str_replace(args: Vec<Value>, first_only: bool) -> Result<Value, Box<dyn std::error::Error>> {
    let func = if first_only {
        "replace_first"
    } else {
        "replace"
    };
    let s = arg_str(&args, 0, func)?;
    let from = arg_str(&args, 1, func)?;
    let to = arg_str(&args, 2, func)?;
    let result = if first_only {
        s.replacen(from, to, 1)
    } else {
        s.replace(from, to)
    };
    Ok(ok_string(result))
}

/// substring(s, start, end) over character indices, clamped to bounds so it
/// never fails — a total operation.
fn str_substring(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let s = arg_str(&args, 0, "substring")?;
    let chars: Vec<char> = s.chars().collect();
    let len = chars.len() as i64;
    let start = arg_int(&args, 1, "substring")?.clamp(0, len) as usize;
    let end = arg_int(&args, 2, "substring")?.clamp(0, len) as usize;
    let slice: String = if start < end {
        chars[start..end].iter().collect()
    } else {
        String::new()
    };
    Ok(ok_string(slice))
}

fn str_pad(args: Vec<Value>, start: bool) -> Result<Value, Box<dyn std::error::Error>> {
    let func = if start { "pad_start" } else { "pad_end" };
    let s = arg_str(&args, 0, func)?;
    let target = arg_int(&args, 1, func)?;
    let pad = arg_str(&args, 2, func)?;
    let current = s.chars().count() as i64;
    if target <= current || pad.is_empty() {
        return Ok(ok_string(s.to_string()));
    }
    let needed = (target - current) as usize;
    let filler: String = pad.chars().cycle().take(needed).collect();
    let result = if start {
        filler + s
    } else {
        s.to_string() + &filler
    };
    Ok(ok_string(result))
}

/// Fill `{}` placeholders with the remaining arguments, rendered in display
/// form (strings bare, like `show`). `{{` and `}}` escape literal braces.
/// The placeholder and argument counts must agree.
/// Usage: str.fmt("{} of {}", 3, "hearts") -> "3 of hearts"
fn str_fmt(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let template = match args.first() {
        Some(Value::String(s)) => s.as_ref().clone(),
        Some(other) => {
            return Err(format!(
                "str.fmt: first argument must be a format string, got {}",
                other.type_name()
            )
            .into());
        }
        None => return Err("str.fmt: missing format string".into()),
    };

    let mut out = String::with_capacity(template.len());
    let mut used = 0;
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                out.push('{');
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                out.push('}');
            }
            '{' if chars.peek() == Some(&'}') => {
                chars.next();
                let value = args.get(used + 1).ok_or_else(|| {
                    format!(
                        "str.fmt: format string has more {{}} placeholders than arguments ({})",
                        args.len() - 1
                    )
                })?;
                match value {
                    Value::String(s) => out.push_str(s),
                    other => out.push_str(&other.to_string()),
                }
                used += 1;
            }
            other => out.push(other),
        }
    }

    if used != args.len() - 1 {
        return Err(format!(
            "str.fmt: {} argument(s) given but {} {{}} placeholder(s) in the format string",
            args.len() - 1,
            used
        )
        .into());
    }

    Ok(Value::String(std::sync::Arc::new(out)))
}

fn str_join(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let list = match args.first() {
        Some(Value::List(items)) => items,
        Some(other) => {
            return Err(format!(
                "str.join: first argument must be a list, got {}",
                other.type_name()
            )
            .into());
        }
        None => return Err("str.join: missing argument 1".into()),
    };
    let sep = arg_str(&args, 1, "join")?;
    let parts: Result<Vec<String>, Box<dyn std::error::Error>> = list
        .iter()
        .map(|v| match v {
            Value::String(s) => Ok(s.to_string()),
            other => Err(format!(
                "str.join: list must contain strings, got {}",
                other.type_name()
            )
            .into()),
        })
        .collect();
    Ok(ok_string(parts?.join(sep)))
}

// ── fallible operations (return Result) ─────────────────────────────

fn str_parse_int(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let s = arg_str(&args, 0, "parse_int")?.trim();
    match s.parse::<i64>() {
        Ok(n) => Ok(Value::Ok(Box::new(Value::Integer(n)))),
        Err(_) => Ok(Value::Err(Box::new(ok_string(format!(
            "cannot parse '{}' as an integer",
            s
        ))))),
    }
}

fn str_parse_float(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let s = arg_str(&args, 0, "parse_float")?.trim();
    match s.parse::<f64>() {
        // A Float is never inf or NaN, so text whose value would be one
        // does not parse: "1e999" is out of range, not infinity.
        Ok(f) if !f.is_finite() => Ok(Value::Err(Box::new(ok_string(format!(
            "'{}' is out of range for a float",
            s
        ))))),
        Ok(f) => Ok(Value::Ok(Box::new(Value::Float(f)))),
        Err(_) => Ok(Value::Err(Box::new(ok_string(format!(
            "cannot parse '{}' as a float",
            s
        ))))),
    }
}
