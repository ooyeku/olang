//! `meta` — the Open AST: olang programs reading olang programs, as data.
//!
//! olang stabilized its syntax early, on purpose. The prize on that
//! decision is that the *parse tree can be a stable public data format*:
//! `meta.parse(source)` hands a program back to olang code as ordinary
//! values — a list of statement maps, each tagged with a `"kind"`, that
//! you walk with the same `match`/`filter`/`map` you use on any data.
//!
//! This is the "open code" pillar of the openness campaign: linters,
//! codemods, import extractors, and complexity reports become olang
//! scripts instead of compiler changes. The `otc deps` command — "list a
//! file's imports" — is six lines of olang over `meta.parse`.
//!
//! Nodes are discriminated-union maps (`#{ "kind": "call", ... }`), the
//! shape olang already pattern-matches on. Statements carry `line`/
//! `column` when the parser positioned them. Expressions nest: a call's
//! `callee` and `args`, a `match`'s `arms`, a map's `entries`, a struct's
//! `fields`, and a template's `parts` are all themselves node maps. Every
//! sub-expression is emitted — the `expr_to_value` match is exhaustive with
//! no catch-all — so a tool that walks the node tree can never silently
//! miss a call hidden in a subtree (a "no bare unwrap" lint sees an unwrap
//! inside a `match` arm or `await`). What is *summarized*, not dropped, is
//! non-expression detail: patterns collapse to their bound names and type
//! annotations to source text. Faithful enough to *analyze* a program, not
//! to perfectly reconstruct one. A new `Expr` variant is a build error here
//! until it gets an arm — that compile-time check is the completeness
//! guarantee.

use crate::ast::{
    Argument, Expr, FieldValue, FunctionDecl, LetDecl, MatchArm, Parameter, Pattern, ShareDecl,
    Statement, TemplatePart, UseDecl, UseItem, Value,
};
use std::sync::Arc;

pub fn create_meta_module() -> Value {
    let mut module = crate::ast::ValueMap::default();
    module.insert("parse".to_string(), builtin("parse", 1));
    module.insert("parse_items".to_string(), builtin("parse_items", 1));
    module.insert("eval".to_string(), builtin("eval", 1));
    module.insert("lit".to_string(), builtin("lit", 1));
    module.insert("expand".to_string(), builtin("expand", 1));
    module.insert("encode".to_string(), builtin("encode", 1));
    module.insert("fresh".to_string(), builtin("fresh", 1));
    module.insert("exports".to_string(), builtin("exports", 1));
    module.insert("unresolved".to_string(), builtin("unresolved", 1));
    module.insert("resolve".to_string(), builtin("resolve", 1));
    module.insert("types".to_string(), builtin("types", 1));
    module.insert("fingerprint".to_string(), builtin("fingerprint", 1));
    module.insert("rename".to_string(), builtin("rename", 2));
    module.insert("tokens".to_string(), builtin("tokens", 1));
    module.insert("highlight".to_string(), builtin("highlight", 1));
    Value::Struct {
        type_name: "Module".into(),
        fields: std::sync::Arc::new(module).into(),
    }
}

fn builtin(name: &str, arity: usize) -> Value {
    Value::Builtin(crate::ast::BuiltinFunction {
        name: format!("meta.{}", name),
        arity,
    })
}

pub fn call_meta_function(
    name: &str,
    args: Vec<Value>,
) -> Result<Value, Box<dyn std::error::Error>> {
    match name {
        "parse" => meta_parse(args),
        "parse_items" => meta_parse_items(args),
        "eval" => meta_eval(args),
        "lit" => meta_lit(args),
        "expand" => meta_expand(args),
        "encode" => meta_encode(args),
        "fresh" => meta_fresh(args),
        "exports" => meta_exports(args),
        "unresolved" => meta_unresolved(args),
        "resolve" => meta_resolve(args),
        "types" => meta_types(args),
        "fingerprint" => meta_fingerprint(args),
        "rename" => meta_rename(args),
        "tokens" | "highlight" => {
            let src = match args.first() {
                Some(Value::String(s)) if args.len() <= 2 => s.clone(),
                Some(other) if args.len() <= 2 => {
                    return Err(format!(
                        "meta.{name}: the source must be a string, got {}",
                        other.type_name()
                    )
                    .into());
                }
                _ => {
                    return Err(format!(
                        "meta.{name} expects 1 or 2 arguments (source, lang), got {}",
                        args.len()
                    )
                    .into());
                }
            };
            // `#{from, state, packed, lang}`: packed Int lists in bytes,
            // from an offset with the lexer's state there (OL-454)
            if name == "tokens"
                && let Some(opt) = args.get(1).map(|v| v.as_options())
                && let Value::Map(opts) = opt.as_ref()
            {
                return tokens_with_options(&src, opts);
            }
            // the language: olang unless named
            let lang = match args.get(1) {
                None | Some(Value::Unit) => "olang",
                Some(Value::String(l)) => crate::stdlib::lexers::language(l).ok_or_else(|| {
                    format!(
                        "meta.{name}: no tokenizer for '{l}' (it knows {})",
                        crate::stdlib::lexers::LANGUAGES
                    )
                })?,
                Some(other) => {
                    return Err(format!(
                        "meta.{name}: the language must be a string like \"python\", got {}",
                        other.type_name()
                    )
                    .into());
                }
            };
            Ok(match (name, lang) {
                ("tokens", "olang") => crate::stdlib::tokens::tokens_value(&src),
                ("tokens", l) => crate::stdlib::tokens::tokens_value_in(&src, l),
                (_, "olang") => {
                    Value::String(std::sync::Arc::new(crate::stdlib::tokens::highlight(&src)))
                }
                (_, l) => Value::String(std::sync::Arc::new(crate::stdlib::tokens::highlight_in(
                    &src, l,
                ))),
            })
        }
        _ => Err(format!("Unknown meta function: {}", name).into()),
    }
}

/// `meta.tokens(source, #{"from", "state", "packed", "lang"})`: with
/// `packed: true`, `#{kind, start, end, kinds, state}` — three Int lists
/// in bytes (a kind is its index in `kinds`) and the lexer's state at the
/// end ("code", "string", "raw" or "template"); `from` (a byte offset)
/// and `state` (the state there, as a previous call answered it) relex
/// from a line instead of from the start.
fn tokens_with_options(
    src: &str,
    opts: &crate::ast::ValueMap,
) -> Result<Value, Box<dyn std::error::Error>> {
    use crate::stdlib::tokens::{LexState, packed_value, tokens_from, tokens_value_of};
    if let Some(k) = opts
        .keys()
        .find(|k| !["from", "state", "packed", "lang"].contains(&k.as_str()))
    {
        return Err(format!(
            "meta.tokens: unknown option \"{k}\" (it takes from, state, packed and lang)"
        )
        .into());
    }
    let from = match opts.get("from") {
        None | Some(Value::Unit) => 0,
        Some(Value::Integer(n)) if *n >= 0 && (*n as usize) <= src.len() => *n as usize,
        Some(Value::Integer(n)) => {
            return Err(format!(
                "meta.tokens: from {n} is outside the source (0 to {})",
                src.len()
            )
            .into());
        }
        Some(other) => {
            return Err(format!(
                "meta.tokens: from must be an Int, got {}",
                other.type_name()
            )
            .into());
        }
    };
    if !src.is_char_boundary(from) {
        return Err(format!("meta.tokens: from {from} is inside a character").into());
    }
    let state = match opts.get("state") {
        None | Some(Value::Unit) => LexState::Code,
        Some(Value::String(s)) => LexState::from_name(s).ok_or_else(|| {
            format!("meta.tokens: unknown state \"{s}\" (code, string, raw or template)")
        })?,
        Some(other) => {
            return Err(format!(
                "meta.tokens: state must be a string, got {}",
                other.type_name()
            )
            .into());
        }
    };
    let packed = match opts.get("packed") {
        None | Some(Value::Unit) => false,
        Some(Value::Boolean(b)) => *b,
        Some(other) => {
            return Err(format!(
                "meta.tokens: packed must be true or false, got {}",
                other.type_name()
            )
            .into());
        }
    };
    match opts.get("lang") {
        None | Some(Value::Unit) => {}
        Some(Value::String(l)) if crate::stdlib::lexers::language(l) == Some("olang") => {}
        Some(Value::String(l)) if from == 0 && state == LexState::Code && !packed => {
            let lang = crate::stdlib::lexers::language(l).ok_or_else(|| {
                format!(
                    "meta.tokens: no tokenizer for '{l}' (it knows {})",
                    crate::stdlib::lexers::LANGUAGES
                )
            })?;
            return Ok(crate::stdlib::tokens::tokens_value_in(src, lang));
        }
        Some(_) => {
            return Err("meta.tokens: from, state and packed are for olang source".into());
        }
    }
    let (toks, end) = tokens_from(src, from, state);
    Ok(if packed {
        packed_value(&toks, end)
    } else {
        tokens_value_of(src, toks)
    })
}

/// `meta.resolve(source)`: `Ok([#{name, start, end, ...}])`, every name in
/// the program in order, with what it refers to — `def: #{start, end}`
/// (where its binding is defined; `is_def` marks the defining place
/// itself), `builtin: true`, `module: true`, or `unresolved: true` —
/// by the walk the language server's references and rename take, so the
/// two agree. `Err` when the source does not parse.
#[cfg(feature = "native")]
fn meta_resolve(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let source = match args.first() {
        Some(Value::String(s)) if args.len() == 1 => s.clone(),
        _ => return Err("meta.resolve expects the source string".into()),
    };
    let Some(names) = crate::tools::scope_refs::names(&source) else {
        let message = match crate::parser::Parser::new().parse_raw(&source) {
            Err(e) => e.to_string(),
            Ok(_) => "the source does not parse".to_string(),
        };
        return Ok(Value::Err(Box::new(s(&message))));
    };
    let builtins = crate::builtin::BuiltinFunctions::new();
    let modules = crate::stdlib::native_members();
    let embedded = crate::stdlib::embedded::names();
    let out = names
        .into_iter()
        .map(|n| {
            let mut pairs = vec![
                ("name", s(&n.name)),
                ("start", Value::Integer(n.start as i64)),
                ("end", Value::Integer(n.end as i64)),
            ];
            match n.def {
                Some((a, b)) => {
                    pairs.push((
                        "def",
                        map(vec![
                            ("start", Value::Integer(a as i64)),
                            ("end", Value::Integer(b as i64)),
                        ]),
                    ));
                    if n.is_def {
                        pairs.push(("is_def", Value::Boolean(true)));
                    }
                    if n.top_level {
                        pairs.push(("top_level", Value::Boolean(true)));
                    }
                }
                None if modules.contains_key(&n.name) || embedded.contains(&n.name.as_str()) => {
                    pairs.push(("module", Value::Boolean(true)))
                }
                None if builtins.get_functions().contains_key(&n.name) => {
                    pairs.push(("builtin", Value::Boolean(true)))
                }
                None => pairs.push(("unresolved", Value::Boolean(true))),
            }
            map(pairs)
        })
        .collect();
    Ok(Value::Ok(Box::new(list(out))))
}

#[cfg(not(feature = "native"))]
fn meta_resolve(_: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    Err("meta.resolve needs the language server's scope walk, which the browser runtime does not carry".into())
}

/// `meta.types(source)`: `Ok([#{name, signature, doc, name_start, line,
/// column}])`, each top-level declaration with the signature hover
/// shows for it (the checker's types where it knows them) and its `///`
/// doc — the language server's own code (`lsp::signatures`).
#[cfg(feature = "native")]
fn meta_types(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let source = match args.first() {
        Some(Value::String(s)) if args.len() == 1 => s.clone(),
        _ => return Err("meta.types expects the source string".into()),
    };
    let starts = line_starts_of(&source);
    let out = crate::tools::lsp::signatures(&source)
        .into_iter()
        .map(|d| {
            let (line, column) = d.span;
            let name_start = starts
                .get((line as usize).saturating_sub(1))
                .and_then(|ls| {
                    source[*ls..]
                        .char_indices()
                        .nth((column as usize).saturating_sub(1))
                        .map(|(i, _)| ls + i)
                })
                .unwrap_or(0);
            map(vec![
                ("name", s(&d.name)),
                ("signature", s(&d.code)),
                ("doc", s(&d.doc)),
                ("name_start", Value::Integer(name_start as i64)),
                ("line", Value::Integer(line as i64)),
                ("column", Value::Integer(column as i64)),
            ])
        })
        .collect();
    Ok(Value::Ok(Box::new(list(out))))
}

#[cfg(not(feature = "native"))]
fn meta_types(_: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    Err(
        "meta.types needs the language server's checker, which the browser runtime does not carry"
            .into(),
    )
}

// ── Fingerprints (OL-455) ───────────────────────────────────────────

/// Fields that say where a node is, not what it is.
const PLACE_KEYS: &[&str] = &["line", "column", "start", "end", "name_span"];

/// 64-bit FNV-1a, fed a node's canonical bytes. A fixed function, so a
/// fingerprint is the same in every process and every release that keeps
/// the node format (the values batch's `hash(v)` is per process).
struct Fnv(u64);

impl Fnv {
    fn new() -> Self {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
    fn bytes(&mut self, b: &[u8]) {
        for x in b {
            self.0 ^= *x as u64;
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
    }
    fn tag(&mut self, t: u8, len: usize) {
        self.bytes(&[t]);
        self.bytes(&(len as u64).to_le_bytes());
    }
    fn value(&mut self, v: &Value, skip: &[&str]) {
        match v {
            Value::Unit => self.tag(0, 0),
            Value::Boolean(b) => self.tag(1, *b as usize),
            Value::Integer(n) => {
                self.tag(2, 8);
                self.bytes(&n.to_le_bytes());
            }
            Value::Float(f) => {
                self.tag(3, 8);
                self.bytes(&f.to_bits().to_le_bytes());
            }
            Value::String(s) => {
                self.tag(4, s.len());
                self.bytes(s.as_bytes());
            }
            Value::List(items) => {
                self.tag(5, items.len());
                items.iter().for_each(|x| self.value(x, &[]));
            }
            Value::Map(m) => {
                let keys: Vec<_> = m
                    .iter()
                    .filter(|(k, _)| {
                        !PLACE_KEYS.contains(&k.as_str()) && !skip.contains(&k.as_str())
                    })
                    .collect();
                self.tag(6, keys.len());
                for (k, x) in keys {
                    self.tag(7, k.len());
                    self.bytes(k.as_bytes());
                    self.value(x, &[]);
                }
            }
            other => {
                let shown = other.to_string();
                self.tag(8, shown.len());
                self.bytes(shown.as_bytes());
            }
        }
    }
    fn hex(&self) -> String {
        format!("{:016x}", self.0)
    }
}

/// `meta.fingerprint(item)`: `#{sig, body}` for one item (its source, or
/// its node from `meta.parse`), each a 16-digit hex hash of the node with
/// every position left out, so moving the item, or adding a line above
/// it, changes neither. `sig` is the declaration without its body (a
/// `fn`'s name, parameters, types and `share`; a `let`'s binding and
/// type), `body` the body alone (a `let`'s value; for a `type`, a `use` or
/// any other item, the whole of it): a change inside a body leaves `sig`.
fn meta_fingerprint(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let node = match args.first() {
        Some(Value::String(src)) if args.len() == 1 => {
            let program = match crate::parser::Parser::new().parse_raw(src) {
                Ok(p) => p,
                Err(e) => return Ok(Value::Err(Box::new(s(&e.to_string())))),
            };
            match program.statements.as_slice() {
                [one] => stmt_to_value(one),
                [] => return Err("meta.fingerprint: the source holds no item".into()),
                more => {
                    return Err(format!(
                        "meta.fingerprint: the source holds {} items, not one (meta.parse_items splits a file)",
                        more.len()
                    )
                    .into());
                }
            }
        }
        Some(v @ Value::Map(_)) if args.len() == 1 => v.clone(),
        _ => {
            return Err(
                "meta.fingerprint expects one item: its source, or its node from meta.parse".into(),
            );
        }
    };
    let Value::Map(m) = &node else { unreachable!() };
    let body_key = match m.get("kind") {
        Some(Value::String(k)) if matches!(k.as_str(), "fn" | "meta_fn" | "test") => Some("body"),
        Some(Value::String(k)) if k.as_str() == "let" => Some("value"),
        _ => None,
    };
    let mut sig = Fnv::new();
    let mut body = Fnv::new();
    match body_key {
        Some(key) => {
            sig.value(&node, &[key]);
            body.value(m.get(key).unwrap_or(&Value::Unit), &[]);
        }
        None => {
            sig.value(&node, &[]);
            body.value(&node, &[]);
        }
    }
    Ok(Value::Ok(Box::new(map(vec![
        ("sig", s(&sig.hex())),
        ("body", s(&body.hex())),
    ]))))
}

/// `meta.unresolved(source)` → Ok(list of names) | Err(message): every
/// name the program uses that nothing in it defines or imports and the
/// language does not provide, in order of first use: names undefined
/// when the module runs, in a browser bundle as natively.
fn meta_unresolved(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let source = match args.first() {
        Some(Value::String(s)) => s.as_str().to_string(),
        other => {
            return Err(format!(
                "meta.unresolved expects source text, got {}",
                other
                    .map(|v| v.type_name())
                    .unwrap_or_else(|| "nothing".to_string())
            )
            .into());
        }
    };
    match crate::parser::Parser::new().parse(&source) {
        Err(e) => Ok(Value::Err(Box::new(Value::String(std::sync::Arc::new(
            e.to_string(),
        ))))),
        Ok(program) => {
            let names: Vec<Value> = crate::analyze::Analyzer::unresolved_names(&program)
                .into_iter()
                .map(|n| Value::String(std::sync::Arc::new(n)))
                .collect();
            Ok(Value::Ok(Box::new(Value::List(std::sync::Arc::new(names)))))
        }
    }
}

/// `meta.rename(source, names)` → Ok(text) | Err(message): `source`
/// with each name that is a key of the map `names` renamed to its value
/// wherever it binds or refers to a value (`Parser::rename_identifiers`
/// says where it does not). What a splicer uses to keep a module's
/// private names its own.
fn meta_rename(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Err(super::misuse::arity("meta.rename", "2", args.len()));
    }
    let source = match &args[0] {
        Value::String(s) => s.as_str().to_string(),
        other => {
            return Err(super::misuse::arg_type(
                "meta.rename",
                "source",
                "a String",
                other,
            ));
        }
    };
    let entries: Vec<(String, Value)> = match &args[1] {
        Value::Map(m) => m.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        Value::Struct { type_name, fields } if type_name != "Module" => {
            fields.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
        }
        other => {
            return Err(super::misuse::arg_type(
                "meta.rename",
                "names",
                "a Map",
                other,
            ));
        }
    };
    let mut renames = std::collections::HashMap::new();
    for (from, to) in entries {
        match to {
            Value::String(ref t) => {
                renames.insert(from, t.as_str().to_string());
            }
            other => {
                return Err(super::misuse::arg_type(
                    "meta.rename",
                    "each new name",
                    "a String",
                    &other,
                ));
            }
        }
    }
    Ok(
        match crate::parser::Parser::rename_identifiers(&source, &renames) {
            Ok(text) => Value::Ok(Box::new(Value::String(std::sync::Arc::new(text)))),
            Err(e) => Value::Err(Box::new(Value::String(std::sync::Arc::new(e)))),
        },
    )
}

/// `meta.parse(source)` → Ok(list of statement node maps) | Err(message).
fn meta_parse(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let source = match args.first() {
        Some(Value::String(s)) => s.as_str().to_string(),
        // The overwhelmingly common miss: the string is still inside the
        // Result a file read returned. Name the fix, not just the rule —
        // and head off the to_string(f) "conversion", which stringifies
        // the Ok(...) wrapper into unparseable pseudo-source.
        Some(Value::Ok(inner)) if matches!(inner.as_ref(), Value::String(_)) => {
            return Err(
                "meta.parse: expected a source string, got Ok(String) — unwrap the read first: meta.parse(unwrap(f))"
                    .into(),
            );
        }
        other => {
            return Err(format!(
                "meta.parse: expected a source string, got {}",
                other
                    .map(|v| v.type_name())
                    .unwrap_or_else(|| "no argument".to_string())
            )
            .into());
        }
    };
    if args.len() > 2 {
        return Err(format!("meta.parse expects 1 or 2 arguments, got {}", args.len()).into());
    }
    let opt = args.get(1).map(|v| v.as_options());
    let opts = match opt.as_deref() {
        None | Some(Value::Unit) => None,
        Some(Value::Map(opts)) => Some(opts),
        Some(other) => {
            return Err(format!(
                "meta.parse: the options must be a map, got {}",
                other.type_name()
            )
            .into());
        }
    };
    let Some(opts) = opts else {
        return Ok(match parse_spanned(&source) {
            Ok(nodes) => Value::Ok(Box::new(list(nodes))),
            Err(e) => Value::Err(Box::new(s(&e))),
        });
    };
    if let Some(k) = opts
        .keys()
        .find(|k| !["recover", "byte_offset", "line_offset"].contains(&k.as_str()))
    {
        return Err(format!(
            "meta.parse: unknown option \"{k}\" (it takes recover, byte_offset and line_offset)"
        )
        .into());
    }
    // An item parsed on its own (`meta.parse_items` found it): its spans
    // and lines in the whole file's coordinates.
    let offset = |key: &str| -> Result<usize, String> {
        match opts.get(key) {
            None | Some(Value::Unit) => Ok(0),
            Some(Value::Integer(n)) if *n >= 0 => Ok(*n as usize),
            Some(other) => Err(format!(
                "meta.parse: option {key} must be an Int of at least 0, got {other}"
            )),
        }
    };
    let (shift, line_shift) = (offset("byte_offset")?, offset("line_offset")?);
    let recover = match opts.get("recover") {
        None | Some(Value::Unit) => false,
        Some(Value::Boolean(b)) => *b,
        Some(other) => {
            return Err(format!(
                "meta.parse: option recover must be true or false, got {}",
                other.type_name()
            )
            .into());
        }
    };
    if recover {
        return Ok(parse_recovering(&source, shift, line_shift));
    }
    Ok(match parse_nodes(&source, shift, line_shift) {
        Ok(nodes) => Value::Ok(Box::new(list(nodes))),
        Err(e) => Value::Err(Box::new(s(&e.to_string()))),
    })
}

// ── Items one at a time, and recovery (OL-435, OL-436) ──────────────

/// The declaration a top-level item opens with, and its name: read from
/// its first words, without parsing it.
fn item_head(text: &str) -> (&'static str, Option<String>) {
    let mut rest = text;
    // A doc comment and decorators belong to the item under them.
    loop {
        let t = rest.trim_start();
        if t.starts_with("//") {
            rest = t.split_once('\n').map_or("", |(_, r)| r);
        } else if t.starts_with('@') && !t.starts_with("@{") {
            let line = t.split_once('\n').map_or(t, |(l, _)| l);
            // `@derive(...)` on its own line decorates what follows; an
            // `@macro(...)` that is the whole item is an expression.
            match t.split_once('\n') {
                Some((_, r)) if !r.trim_start().is_empty() && !line.contains('=') => rest = r,
                _ => return ("expr", None),
            }
        } else {
            rest = t;
            break;
        }
    }
    let word = |s: &str| -> Option<String> {
        let s = s.trim_start();
        let end = s
            .find(|c: char| !(c.is_alphanumeric() || c == '_'))
            .unwrap_or(s.len());
        (end > 0).then(|| s[..end].to_string())
    };
    let after = |kw: &str| rest.strip_prefix(kw).filter(|r| r.starts_with([' ', '\t']));
    for (kw, kind) in [
        ("fn", "fn"),
        ("let", "let"),
        ("type", "type"),
        ("error", "error"),
        ("trait", "trait"),
        ("impl", "impl"),
        ("test", "test"),
        ("use", "use"),
    ] {
        if let Some(r) = after(kw) {
            let r = r.trim_start();
            let r = r.strip_prefix("mut ").unwrap_or(r);
            let name = if kind == "test" {
                r.strip_prefix('"')
                    .and_then(|t| t.split_once('"'))
                    .map(|(n, _)| n.to_string())
            } else if kind == "use" {
                Some(
                    r.split(|c: char| c.is_whitespace() || c == '{')
                        .next()
                        .unwrap_or("")
                        .to_string(),
                )
            } else {
                word(r)
            };
            return (kind, name);
        }
    }
    if let Some(r) = after("share") {
        let (kind, name) = item_head(r);
        return (if kind == "expr" { "share" } else { kind }, name);
    }
    if let Some(r) = after("meta") {
        return ("meta_fn", r.trim_start().strip_prefix("fn").and_then(word));
    }
    ("expr", None)
}

/// The top-level items of `source`, without a full parse: each item's
/// byte range (they tile the file, a doc comment going with the item
/// under it), its kind and name, and its first line. Found by the scan
/// the parser splits large files with, which also restarts at a line that
/// opens with a declaration keyword, so a file mid-edit (an unclosed
/// bracket) still shows the items after the break.
pub(crate) fn item_ranges(source: &str) -> Vec<(usize, usize, &'static str, Option<String>)> {
    let starts = crate::parser::item_starts(source, true);
    let mut bounds: Vec<usize> = Vec::with_capacity(starts.len() + 1);
    bounds.push(0);
    bounds.extend(starts.into_iter().filter(|s| *s > 0));
    bounds.push(source.len());
    // Comment-only pieces join the item after them.
    let mut out: Vec<(usize, usize, &'static str, Option<String>)> = Vec::new();
    let mut from = 0;
    for w in bounds.windows(2) {
        let (a, b) = (w[0], w[1]);
        let text = &source[from..b];
        let only_comments = source[a..b]
            .lines()
            .all(|l| l.trim().is_empty() || l.trim_start().starts_with("//"));
        if only_comments && b < source.len() {
            continue;
        }
        if text.trim().is_empty() {
            if let Some(last) = out.last_mut() {
                last.1 = b;
            }
            from = b;
            continue;
        }
        let (kind, name) = item_head(text);
        out.push((from, b, kind, name));
        from = b;
    }
    out
}

fn line_of(line_starts: &[usize], offset: usize) -> usize {
    line_starts.partition_point(|s| *s <= offset)
}

/// `meta.parse_items(source)`: `[#{kind, name, start, end, line}]`, one per
/// top-level item (see `item_ranges`): what an index reparses one of.
fn meta_parse_items(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let source = match args.first() {
        Some(Value::String(s)) if args.len() == 1 => s.clone(),
        _ => return Err("meta.parse_items expects the source string".into()),
    };
    let starts = line_starts_of(&source);
    let items = item_ranges(&source)
        .into_iter()
        .map(|(a, b, kind, name)| {
            map(vec![
                ("kind", s(kind)),
                ("name", name.map(|n| s(&n)).unwrap_or(Value::Unit)),
                ("start", Value::Integer(a as i64)),
                ("end", Value::Integer(b as i64)),
                ("line", Value::Integer(line_of(&starts, a) as i64)),
            ])
        })
        .collect();
    Ok(list(items))
}

/// One parse error as a node: its message and where it is, in the whole
/// file's coordinates.
fn error_node(e: &crate::parser::ParseError, text: &str, shift: usize, line_shift: usize) -> Value {
    let message = format!("{}", e);
    let (line, column) = e.line_column().unwrap_or((1, 1));
    let starts = line_starts_of(text);
    let at = starts
        .get(line.saturating_sub(1))
        .map(|ls| {
            ls + text[*ls..]
                .char_indices()
                .nth(column.saturating_sub(1))
                .map_or(text.len() - ls, |(i, _)| i)
        })
        .unwrap_or(0)
        .min(text.len());
    map(vec![
        ("kind", s("error")),
        ("message", s(&message)),
        ("line", Value::Integer((line + line_shift) as i64)),
        ("column", Value::Integer(column as i64)),
        ("at", Value::Integer((at + shift) as i64)),
        ("start", Value::Integer(shift as i64)),
        ("end", Value::Integer((shift + text.len()) as i64)),
    ])
}

/// The nodes of `text` parsed with spans, shifted to where it sits in its
/// file: `Ok(nodes)` or the parse error.
fn parse_nodes(
    text: &str,
    shift: usize,
    line_shift: usize,
) -> Result<Vec<Value>, crate::parser::ParseError> {
    let parsed = crate::parser::Parser::new().parse_with_spans(text)?;
    let ctx = SpanContext {
        statements: parsed.statement_spans,
        line_starts: line_starts_of(text),
        source: Arc::new(text.to_string()),
        shift: (shift + parsed.offset as usize) as i64,
        line_shift: line_shift as i64,
    };
    Ok(spanned_nodes(&parsed.program, ctx))
}

/// `meta.parse(source, #{"recover": true})`: `#{items, errors}`. The whole
/// file when it parses; otherwise each top-level item parsed on its own,
/// so a file mid-edit keeps every item but the broken one, and each
/// error says where it is.
fn parse_recovering(source: &str, shift: usize, line_shift: usize) -> Value {
    if let Ok(nodes) = parse_nodes(source, shift, line_shift) {
        return map(vec![("items", list(nodes)), ("errors", list(Vec::new()))]);
    }
    let starts = line_starts_of(source);
    let mut items = Vec::new();
    let mut errors = Vec::new();
    for (a, b, _, _) in item_ranges(source) {
        let text = &source[a..b];
        let item_lines = line_shift + line_of(&starts, a) - 1;
        match parse_nodes(text, shift + a, item_lines) {
            Ok(nodes) => items.extend(nodes),
            Err(e) => errors.push(error_node(&e, text, shift + a, item_lines)),
        }
    }
    map(vec![("items", list(items)), ("errors", list(errors))])
}

/// What `meta.parse(source)` answers, from Rust: the nodes with their
/// spans, or the parse error's message.
pub fn parse_spanned(source: &str) -> Result<Vec<Value>, String> {
    let parsed = crate::parser::Parser::new()
        .parse_with_spans(source)
        .map_err(|e| e.to_string())?;
    let ctx = SpanContext {
        statements: parsed.statement_spans,
        line_starts: line_starts_of(source),
        source: Arc::new(source.to_string()),
        shift: parsed.offset as i64,
        line_shift: 0,
    };
    Ok(spanned_nodes(&parsed.program, ctx))
}

/// The byte offset of each line of `text`.
pub(crate) fn line_starts_of(text: &str) -> Vec<usize> {
    std::iter::once(0)
        .chain(memchr::memchr_iter(b'\n', text.as_bytes()).map(|i| i + 1))
        .collect()
}

/// `meta.encode(source)`: the program as an image (src/olb.rs) — parsed
/// with its macros expanded, then serialized behind the version header.
/// `Ok(bytes)`, or `Err(message)` for a syntax error.
fn meta_encode(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let source = match args.first() {
        Some(Value::String(src)) => src.as_str().to_string(),
        Some(Value::Ok(inner)) if matches!(inner.as_ref(), Value::String(_)) => {
            return Err(
                "meta.encode: expected a source string, got Ok(String) — unwrap the read first: meta.encode(unwrap(f))"
                    .into(),
            );
        }
        other => {
            return Err(format!(
                "meta.encode: expected a source string, got {}",
                other
                    .map(|v| v.type_name())
                    .unwrap_or_else(|| "no argument".to_string())
            )
            .into());
        }
    };
    // Options: `#{ "hot": ["view", "update"] }` names the functions the
    // loading runtime should compile at declaration rather than at their
    // first call — the functions a page runs once, at boot, whose one
    // call would otherwise be the tree-walker's.
    let mut hot: Vec<String> = Vec::new();
    match args.get(1).map(|v| v.as_options()).as_deref() {
        None => {}
        Some(Value::Map(options)) => {
            for (key, value) in options.iter() {
                match (key.as_str(), value) {
                    ("hot", Value::List(names)) => {
                        for name in names.iter() {
                            match name {
                                Value::String(n) => hot.push(n.as_str().to_string()),
                                other => {
                                    return Err(format!(
                                        "meta.encode: hot must be a list of function names (Strings), got {}",
                                        other.type_name()
                                    )
                                    .into());
                                }
                            }
                        }
                    }
                    ("hot", other) => {
                        return Err(format!(
                            "meta.encode: hot must be a list of function names, got {}",
                            other.type_name()
                        )
                        .into());
                    }
                    (other, _) => {
                        return Err(format!(
                            "meta.encode: unknown option '{}' — the option is hot",
                            other
                        )
                        .into());
                    }
                }
            }
        }
        Some(other) => {
            return Err(format!(
                "meta.encode: options must be a map, got {}",
                other.type_name()
            )
            .into());
        }
    }
    match crate::parser::Parser::new().parse(&source) {
        Err(e) => Ok(Value::Err(Box::new(s(&format!("{}", e))))),
        Ok(program) => match crate::olb::encode_with(&program, &hot) {
            Ok(bytes) => Ok(Value::Ok(Box::new(crate::stdlib::bytes::to_value(bytes)))),
            Err(e) => Ok(Value::Err(Box::new(s(&e)))),
        },
    }
}

/// The program's top-level statements as the node maps `meta.parse`
/// hands programs — the same shapes, so a tool in Rust and a tool in
/// olang read one format.
pub fn program_nodes(program: &crate::ast::Program) -> Vec<Value> {
    program.statements.iter().map(stmt_to_value).collect()
}

/// One statement as `program_nodes` gives it.
pub fn stmt_node(stmt: &Statement) -> Value {
    stmt_to_value(stmt)
}

// ── Value builders ──────────────────────────────────────────────────
fn s(text: &str) -> Value {
    Value::String(std::sync::Arc::new(text.to_string()))
}
fn list(items: Vec<Value>) -> Value {
    Value::List(std::sync::Arc::new(items))
}
fn map(pairs: Vec<(&str, Value)>) -> Value {
    let mut m = crate::ast::ValueMap::default();
    for (k, v) in pairs {
        m.insert(k.to_string(), v);
    }
    Value::Map(std::sync::Arc::new(m))
}
fn names(v: &[String]) -> Value {
    list(v.iter().map(|n| s(n)).collect())
}

// ── Spans (OL-434) ──────────────────────────────────────────────────
//
// `meta.parse` parses with spans (`Parser::parse_with_spans`): every
// expression comes wrapped in `Expr::At` with its byte range, and each
// statement's range is in a table keyed by its line and column. The
// converter below reads them through this context, which also holds the
// shift an item parsed on its own adds to put its spans in the whole
// file's coordinates (`#{"byte_offset", "line_offset"}`).

pub(crate) struct SpanContext {
    pub statements: rustc_hash::FxHashMap<(u32, u32), (u32, u32)>,
    /// Byte offset of each line of the parsed text.
    pub line_starts: Vec<usize>,
    pub source: std::sync::Arc<String>,
    /// Added to every byte offset (a BOM, an item's place in its file).
    pub shift: i64,
    /// Added to every line.
    pub line_shift: i64,
}

impl SpanContext {
    /// The byte range of a declaration's name, from the 1-based line and
    /// character column the AST keeps for it.
    fn name_span(&self, stmt: &Statement) -> Option<Value> {
        let (pos, name) = match stmt.unwrapped() {
            Statement::FunctionDecl(d) | Statement::MetaFnDecl { decl: d, .. } => {
                (d.name_span?, d.name.clone())
            }
            Statement::ShareDecl(ShareDecl::Function(d)) => (d.name_span?, d.name.clone()),
            Statement::LetDecl(d) | Statement::ShareDecl(ShareDecl::Let(d)) => {
                (d.name_span?, pattern_name(&d.pattern))
            }
            Statement::TypeDecl(d) => (d.name_span?, d.name.clone()),
            _ => return None,
        };
        let (line, column) = pos;
        let line_start = *self.line_starts.get((line as usize).checked_sub(1)?)?;
        let text = self.source.get(line_start..)?;
        let col = text
            .char_indices()
            .nth((column as usize).checked_sub(1)?)
            .map(|(i, _)| i)?;
        let start = line_start + col;
        let len = if self.source[start..].starts_with(name.as_str()) {
            name.len()
        } else {
            self.source[start..]
                .find(|c: char| !(c.is_alphanumeric() || c == '_'))
                .unwrap_or(self.source.len() - start)
        };
        Some(map(vec![
            ("start", Value::Integer(start as i64 + self.shift)),
            ("end", Value::Integer((start + len) as i64 + self.shift)),
        ]))
    }
}

thread_local! {
    static SPANS: std::cell::RefCell<Option<SpanContext>> = const { std::cell::RefCell::new(None) };
}

/// `node` with `start` and `end`, unless it has its own.
fn with_span(mut node: Value, start: usize, end: usize) -> Value {
    let shift = SPANS.with(|c| c.borrow().as_ref().map_or(0, |c| c.shift));
    if let Value::Map(m) = &mut node
        && !m.contains_key("start")
    {
        // A node just built is held only here: no copy.
        let m = Arc::make_mut(m);
        m.insert("start".to_string(), Value::Integer(start as i64 + shift));
        m.insert("end".to_string(), Value::Integer(end as i64 + shift));
    }
    node
}

/// Convert `program` with `ctx` in place for the spans it holds.
pub(crate) fn spanned_nodes(program: &crate::ast::Program, ctx: SpanContext) -> Vec<Value> {
    SPANS.with(|c| *c.borrow_mut() = Some(ctx));
    let nodes = program
        .statements
        .iter()
        .map(|st| inherit_spans(stmt_to_value(st), None))
        .collect();
    SPANS.with(|c| *c.borrow_mut() = None);
    nodes
}

/// The node kinds that are parts of a construct rather than statements or
/// expressions: they carry no span (their expressions do).
const UNSPANNED: &[&str] = &[
    "arm",
    "named_arg",
    "entry",
    "field_value",
    "pattern",
    "type",
];

/// Give every statement and expression node a span: one the parser built
/// rather than read (a desugaring's call, the `()` a block that ends in a
/// `let` gains, a destructured parameter's `let`, a statement inside a
/// template's `${}`) takes the span of the nearest node around it, the
/// construct it was made from.
fn inherit_spans(mut node: Value, around: Option<(Value, Value)>) -> Value {
    match &mut node {
        Value::List(items) => {
            let items = Arc::make_mut(items);
            for item in items.iter_mut() {
                let taken = std::mem::replace(item, Value::Unit);
                *item = inherit_spans(taken, around.clone());
            }
        }
        Value::Map(m) => {
            let kind = match m.get("kind") {
                Some(Value::String(k)) => Some(k.clone()),
                _ => None,
            };
            let spanned = kind
                .as_ref()
                .is_some_and(|k| !UNSPANNED.contains(&k.as_str()));
            let m = Arc::make_mut(m);
            if spanned
                && !m.contains_key("start")
                && let Some((a, b)) = &around
            {
                m.insert("start".to_string(), a.clone());
                m.insert("end".to_string(), b.clone());
            }
            let here = match (m.get("start"), m.get("end")) {
                (Some(a @ Value::Integer(_)), Some(b @ Value::Integer(_))) if spanned => {
                    Some((a.clone(), b.clone()))
                }
                _ => around,
            };
            for (k, v) in m.iter_mut() {
                if k == "name_span" || ((k == "start" || k == "end") && spanned) {
                    continue;
                }
                let taken = std::mem::replace(v, Value::Unit);
                *v = inherit_spans(taken, here.clone());
            }
        }
        _ => {}
    }
    node
}

// ── Statements ──────────────────────────────────────────────────────
fn stmt_to_value(stmt: &Statement) -> Value {
    match stmt {
        Statement::MetaFnDecl { decl, .. } => {
            let mut m_v = match &stmt_to_value(&Statement::FunctionDecl(decl.clone())) {
                Value::Map(inner) => inner.as_ref().clone(),
                _ => crate::ast::ValueMap::default(),
            };
            m_v.insert("kind".to_string(), s("meta_fn"));
            Value::Map(std::sync::Arc::new(m_v))
        }
        Statement::DecoratedDecl {
            decorators,
            decl_src,
            ..
        } => map(vec![
            ("kind", s("decorated_type")),
            (
                "decorators",
                Value::List(std::sync::Arc::new(
                    decorators
                        .iter()
                        .map(|d| {
                            map(vec![
                                ("name", s(&d.name)),
                                (
                                    "args",
                                    Value::List(std::sync::Arc::new(
                                        d.args_src.iter().map(|a| s(a)).collect(),
                                    )),
                                ),
                            ])
                        })
                        .collect(),
                )),
            ),
            ("decl", s(decl_src)),
        ]),
        Statement::Located { line, column, stmt } => {
            // Attach position to the inner node: its line and column, and
            // in a spans parse its byte range and its name's.
            let inner = stmt_to_value(stmt);
            if let Value::Map(m) = &inner {
                let mut m = (**m).clone();
                SPANS.with(|c| {
                    let c = c.borrow();
                    let Some(c) = c.as_ref() else { return };
                    m.insert(
                        "line".to_string(),
                        Value::Integer(*line as i64 + c.line_shift),
                    );
                    if let Some((a, b)) = c.statements.get(&(*line, *column)) {
                        m.insert("start".to_string(), Value::Integer(*a as i64 + c.shift));
                        m.insert("end".to_string(), Value::Integer(*b as i64 + c.shift));
                    }
                    if let Some(name_span) = c.name_span(stmt) {
                        m.insert("name_span".to_string(), name_span);
                    }
                });
                m.entry("line".to_string())
                    .or_insert(Value::Integer(*line as i64));
                m.insert("column".to_string(), Value::Integer(*column as i64));
                return Value::Map(std::sync::Arc::new(m));
            }
            inner
        }
        Statement::Expression(e) => map(vec![("kind", s("expr")), ("value", expr_to_value(e))]),
        Statement::LetDecl(d) => let_to_value(d, false),
        Statement::FunctionDecl(d) => fn_to_value(d, false),
        Statement::TypeDecl(d) => {
            // The definition rides along: a derive macro reading a type's
            // fields is the whole reason to parse a type declaration.
            let mut entries = vec![
                ("kind", s("type")),
                ("name", s(&d.name)),
                ("type_params", names(&d.type_params)),
            ];
            match &d.definition {
                crate::ast::TypeDefinition::Struct { fields } => {
                    entries.push(("definition", s("struct")));
                    entries.push((
                        "fields",
                        list(
                            fields
                                .iter()
                                .map(|f| {
                                    map(vec![
                                        ("name", s(&f.name)),
                                        // `type` stays the source text a
                                        // derive macro compares; the node
                                        // rides beside it.
                                        ("type", s(&f.field_type.display_source())),
                                        ("type_node", type_to_value(&f.field_type)),
                                    ])
                                })
                                .collect(),
                        ),
                    ));
                }
                crate::ast::TypeDefinition::Enum { variants } => {
                    entries.push(("definition", s("enum")));
                    entries.push((
                        "variants",
                        list(variants.iter().map(|v| s(&v.name)).collect()),
                    ));
                }
                crate::ast::TypeDefinition::Union { types } => {
                    entries.push(("definition", s("union")));
                    entries.push((
                        "types",
                        list(types.iter().map(|t| s(&t.display_source())).collect()),
                    ));
                }
                crate::ast::TypeDefinition::Alias { target } => {
                    // The aliased annotation as a node: a record alias's
                    // `fields` are what a shape rule reads.
                    entries.push(("definition", s("alias")));
                    entries.push(("type", type_to_value(target)));
                }
            }
            map(entries)
        }
        Statement::ErrorTypeDecl(d) => map(vec![("kind", s("error")), ("name", s(&d.name))]),
        Statement::ShareDecl(d) => share_to_value(d),
        Statement::UseDecl(d) => use_to_value(d),
        Statement::TestDecl(d) => map(vec![
            ("kind", s("test")),
            ("name", s(&d.name)),
            ("body", list(d.body.iter().map(stmt_to_value).collect())),
        ]),
        Statement::TraitDecl(d) => map(vec![("kind", s("trait")), ("name", s(&d.name))]),
        Statement::ImplDecl(d) => map(vec![
            ("kind", s("impl")),
            ("trait", s(&d.trait_name)),
            ("type", s(&d.type_name)),
        ]),
    }
}

fn let_to_value(d: &LetDecl, shared: bool) -> Value {
    let mut pairs = vec![
        ("kind", s("let")),
        ("name", s(&pattern_name(&d.pattern))),
        ("pattern", pattern_to_value(&d.pattern)),
        (
            "type",
            d.type_annotation
                .as_ref()
                .map(type_to_value)
                .unwrap_or(Value::Unit),
        ),
        ("shared", Value::Boolean(shared)),
        // `let mut` — a lint that reasons about mutability needs this.
        ("mutable", Value::Boolean(d.mutable)),
    ];
    if let Some(v) = &d.value {
        pairs.push(("value", expr_to_value(v)));
    }
    map(pairs)
}

fn fn_to_value(d: &FunctionDecl, shared: bool) -> Value {
    map(vec![
        ("kind", s("fn")),
        ("name", s(&d.name)),
        ("params", params_to_value(&d.parameters)),
        // The parameters with their annotations, and the return type:
        // what a shape rule reads.
        ("parameters", parameters_to_value(&d.parameters)),
        (
            "return_type",
            d.return_type
                .as_ref()
                .map(type_to_value)
                .unwrap_or(Value::Unit),
        ),
        ("type_params", names(&d.type_params)),
        ("shared", Value::Boolean(shared)),
        ("body", expr_to_value(&d.body)),
    ])
}

fn share_to_value(d: &ShareDecl) -> Value {
    match d {
        ShareDecl::Function(f) => fn_to_value(f, true),
        ShareDecl::Let(l) => let_to_value(l, true),
        ShareDecl::Type(t) => map(vec![
            ("kind", s("type")),
            ("name", s(&t.name)),
            ("shared", Value::Boolean(true)),
        ]),
        ShareDecl::Use(u) => {
            // A re-export: a use marked shared.
            if let Value::Map(m) = &use_to_value(u) {
                let mut m = m.as_ref().clone();
                m.insert("shared".to_string(), Value::Boolean(true));
                return Value::Map(std::sync::Arc::new(m));
            }
            use_to_value(u)
        }
        ShareDecl::Trait(t) => map(vec![
            ("kind", s("trait")),
            ("name", s(&t.name)),
            ("shared", Value::Boolean(true)),
        ]),
        ShareDecl::Impl(i) => map(vec![
            ("kind", s("impl")),
            ("trait", s(&i.trait_name)),
            ("type", s(&i.type_name)),
            ("shared", Value::Boolean(true)),
        ]),
    }
}

fn use_to_value(d: &UseDecl) -> Value {
    let items: Vec<Value> = d
        .items
        .iter()
        .map(|i| match i {
            UseItem::Specific(n) => s(n),
            UseItem::Aliased { name, alias } => s(&format!("{} as {}", name, alias)),
            UseItem::Wildcard => s("*"),
        })
        .collect();
    map(vec![
        ("kind", s("use")),
        ("path", s(&d.path.join("."))),
        ("items", list(items)),
    ])
}

fn params_to_value(params: &[Parameter]) -> Value {
    list(params.iter().map(|p| s(&p.name)).collect())
}

/// Each parameter as a node: its name, its annotation as a type node
/// (Unit when unannotated), and whether it has a default.
fn parameters_to_value(params: &[Parameter]) -> Value {
    list(
        params
            .iter()
            .map(|p| {
                map(vec![
                    ("name", s(&p.name)),
                    (
                        "type",
                        p.type_annotation
                            .as_ref()
                            .map(type_to_value)
                            .unwrap_or(Value::Unit),
                    ),
                    ("has_default", Value::Boolean(p.default_value.is_some())),
                ])
            })
            .collect(),
    )
}

/// A type annotation as a node: `kind: "type"`, its source text, and a
/// `form` with the structure a rule needs — `record` carries `fields`
/// (name and type), `list` an `element`, `map` a `key` and `value`,
/// `tuple`/`union` `types`, `result` `ok` and `err`, `function` `params`
/// and `returns`, `generic` a `base` and `args`, `named` and `basic` a
/// `name`, `literal` a `value`.
fn type_to_value(t: &crate::ast::TypeAnnotation) -> Value {
    use crate::ast::TypeAnnotation as T;
    let mut pairs = vec![("kind", s("type")), ("text", s(&t.display_source()))];
    match t {
        T::Int | T::Float | T::Bool | T::String | T::Unit => {
            pairs.push(("form", s("basic")));
            pairs.push(("name", s(&t.display_source())));
        }
        T::List(e) => {
            pairs.push(("form", s("list")));
            pairs.push(("element", type_to_value(e)));
        }
        T::Map {
            key_type,
            value_type,
        } => {
            pairs.push(("form", s("map")));
            pairs.push(("key", type_to_value(key_type)));
            pairs.push(("value", type_to_value(value_type)));
        }
        T::Tuple(ts) => {
            pairs.push(("form", s("tuple")));
            pairs.push(("types", list(ts.iter().map(type_to_value).collect())));
        }
        T::Result { ok_type, err_type } => {
            pairs.push(("form", s("result")));
            pairs.push(("ok", type_to_value(ok_type)));
            pairs.push(("err", type_to_value(err_type)));
        }
        T::Function {
            params,
            return_type,
        } => {
            pairs.push(("form", s("function")));
            pairs.push(("params", list(params.iter().map(type_to_value).collect())));
            pairs.push(("returns", type_to_value(return_type)));
        }
        T::Record { fields } => {
            pairs.push(("form", s("record")));
            pairs.push((
                "fields",
                list(
                    fields
                        .iter()
                        .map(|f| {
                            map(vec![
                                ("name", s(&f.name)),
                                ("type", type_to_value(&f.field_type)),
                            ])
                        })
                        .collect(),
                ),
            ));
        }
        T::Union { types } => {
            pairs.push(("form", s("union")));
            pairs.push(("types", list(types.iter().map(type_to_value).collect())));
        }
        T::Generic {
            base_type,
            type_args,
        } => {
            pairs.push(("form", s("generic")));
            pairs.push(("base", s(base_type)));
            pairs.push(("args", list(type_args.iter().map(type_to_value).collect())));
        }
        T::Literal { value } => {
            pairs.push(("form", s("literal")));
            pairs.push(("value", value.as_ref().clone()));
        }
        T::Custom(name) | T::TypeVariable(name) => {
            pairs.push(("form", s("named")));
            pairs.push(("name", s(name)));
        }
        _ => pairs.push(("form", s("other"))),
    }
    map(pairs)
}

/// A match (or let) pattern as a node: `kind: "pattern"`, its bound
/// names as `text`, and a `form` with the structure an exhaustiveness
/// rule needs — `enum` carries the `variant` and its sub-`patterns`,
/// `literal` a `value`, `ident` and `rest` a `name`, `struct` its `type`
/// and `fields` (name and pattern), `record` its `fields`, `list` its
/// `items` and `rest`, `tuple` its `items`, `ok`/`err` an `inner`,
/// `range` `start`/`end`/`inclusive`, `or` its `alternatives`, and
/// `guarded` the `pattern` with its `guard` expression.
fn pattern_to_value(p: &Pattern) -> Value {
    let mut pairs = vec![("kind", s("pattern")), ("text", s(&pattern_text(p)))];
    match p {
        Pattern::Identifier(n) => {
            pairs.push(("form", s("ident")));
            pairs.push(("name", s(n)));
        }
        Pattern::Wildcard => pairs.push(("form", s("wildcard"))),
        Pattern::Literal(v) => {
            pairs.push(("form", s("literal")));
            pairs.push(("value", v.clone()));
        }
        Pattern::EnumVariant {
            variant_name,
            patterns,
        } => {
            pairs.push(("form", s("enum")));
            pairs.push(("variant", s(variant_name)));
            pairs.push((
                "patterns",
                list(patterns.iter().map(pattern_to_value).collect()),
            ));
        }
        Pattern::Struct {
            type_name,
            field_patterns,
        } => {
            pairs.push(("form", s("struct")));
            pairs.push(("type", s(type_name)));
            pairs.push(("fields", field_patterns_to_value(field_patterns)));
        }
        Pattern::AnonymousStruct { field_patterns } => {
            pairs.push(("form", s("record")));
            pairs.push(("fields", field_patterns_to_value(field_patterns)));
        }
        Pattern::List { patterns, rest } => {
            pairs.push(("form", s("list")));
            pairs.push((
                "items",
                list(patterns.iter().map(pattern_to_value).collect()),
            ));
            pairs.push(("rest", rest.as_ref().map(|r| s(r)).unwrap_or(Value::Unit)));
        }
        Pattern::Tuple(ps) => {
            pairs.push(("form", s("tuple")));
            pairs.push(("items", list(ps.iter().map(pattern_to_value).collect())));
        }
        Pattern::Ok(inner) => {
            pairs.push(("form", s("ok")));
            pairs.push(("inner", pattern_to_value(inner)));
        }
        Pattern::Err(inner) => {
            pairs.push(("form", s("err")));
            pairs.push(("inner", pattern_to_value(inner)));
        }
        Pattern::Range {
            start,
            end,
            inclusive,
        } => {
            pairs.push(("form", s("range")));
            // `from` and `to`: `start` and `end` are a node's byte span.
            pairs.push(("from", pattern_to_value(start)));
            pairs.push(("to", pattern_to_value(end)));
            pairs.push(("inclusive", Value::Boolean(*inclusive)));
        }
        Pattern::Or { alternatives } => {
            pairs.push(("form", s("or")));
            pairs.push((
                "alternatives",
                list(alternatives.iter().map(pattern_to_value).collect()),
            ));
        }
        Pattern::Guarded { pattern, guard } => {
            pairs.push(("form", s("guarded")));
            pairs.push(("pattern", pattern_to_value(pattern)));
            pairs.push(("guard", expr_to_value(guard)));
        }
        Pattern::Rest(n) => {
            pairs.push(("form", s("rest")));
            pairs.push(("name", s(n)));
        }
    }
    map(pairs)
}

/// A pattern rendered as source, every form included (`pattern_name`
/// keeps the bound-names summary a `let` node's `name` is).
fn pattern_text(p: &Pattern) -> String {
    let joined = |ps: &[Pattern]| ps.iter().map(pattern_text).collect::<Vec<_>>().join(", ");
    match p {
        Pattern::Identifier(n) | Pattern::Rest(n) => n.clone(),
        Pattern::Wildcard => "_".to_string(),
        Pattern::Literal(v) => format!("{}", v),
        Pattern::EnumVariant {
            variant_name,
            patterns,
        } if patterns.is_empty() => variant_name.clone(),
        Pattern::EnumVariant {
            variant_name,
            patterns,
        } => format!("{}({})", variant_name, joined(patterns)),
        Pattern::Struct {
            type_name,
            field_patterns,
        } => format!(
            "{} {{ {} }}",
            type_name,
            field_patterns
                .iter()
                .map(|(n, p)| format!("{}: {}", n, pattern_text(p)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Pattern::AnonymousStruct { field_patterns } => format!(
            "{{ {} }}",
            field_patterns
                .iter()
                .map(|(n, p)| format!("{}: {}", n, pattern_text(p)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Pattern::List { patterns, rest } => {
            let mut parts: Vec<String> = patterns.iter().map(pattern_text).collect();
            if let Some(r) = rest {
                parts.push(format!("...{}", r));
            }
            format!("[{}]", parts.join(", "))
        }
        Pattern::Tuple(ps) => format!("({})", joined(ps)),
        Pattern::Ok(inner) => format!("Ok({})", pattern_text(inner)),
        Pattern::Err(inner) => format!("Err({})", pattern_text(inner)),
        Pattern::Range {
            start,
            end,
            inclusive,
        } => format!(
            "{}{}{}",
            pattern_text(start),
            if *inclusive { "..=" } else { ".." },
            pattern_text(end)
        ),
        Pattern::Or { alternatives } => alternatives
            .iter()
            .map(pattern_text)
            .collect::<Vec<_>>()
            .join(" | "),
        Pattern::Guarded { pattern, .. } => format!("{} if …", pattern_text(pattern)),
    }
}

fn field_patterns_to_value(fields: &[(String, Pattern)]) -> Value {
    list(
        fields
            .iter()
            .map(|(name, p)| map(vec![("name", s(name)), ("pattern", pattern_to_value(p))]))
            .collect(),
    )
}

fn pattern_name(p: &Pattern) -> String {
    match p {
        Pattern::Identifier(n) => n.clone(),
        Pattern::Wildcard => "_".to_string(),
        Pattern::Tuple(ps) => format!(
            "({})",
            ps.iter().map(pattern_name).collect::<Vec<_>>().join(", ")
        ),
        Pattern::List { patterns, rest } => {
            let mut parts: Vec<String> = patterns.iter().map(pattern_name).collect();
            if let Some(r) = rest {
                parts.push(format!("...{}", r));
            }
            format!("[{}]", parts.join(", "))
        }
        _ => "_".to_string(),
    }
}

// ── Expressions ─────────────────────────────────────────────────────
fn expr_to_value(e: &Expr) -> Value {
    match e {
        // A spans parse's range: the node inside carries it. The node's
        // own (innermost) range wins over a wrapper's of the same node.
        Expr::At { start, end, expr } => {
            let inner = expr_to_value(expr);
            with_span(inner, *start as usize, *end as usize)
        }
        Expr::MacroCall { name, args_src, .. } => map(vec![
            ("kind", s("macro_call")),
            ("name", s(name)),
            (
                "args",
                Value::List(std::sync::Arc::new(args_src.iter().map(|a| s(a)).collect())),
            ),
        ]),
        Expr::Integer(n) => map(vec![("kind", s("int")), ("value", Value::Integer(*n))]),
        Expr::Float(f) => map(vec![("kind", s("float")), ("value", Value::Float(*f))]),
        Expr::String(text) => map(vec![("kind", s("str")), ("value", s(text))]),
        Expr::Boolean(b) => map(vec![("kind", s("bool")), ("value", Value::Boolean(*b))]),
        Expr::Identifier(n) => map(vec![("kind", s("ident")), ("name", s(n))]),
        Expr::LocalRef { name, .. } => map(vec![("kind", s("ident")), ("name", s(name))]),
        // The forms the parser builds as calls of hidden builtins are
        // nodes of their own: a literal with spreads, and `xs[i] = v`.
        Expr::Call { callee, arguments } if matches!(callee.bare(), Expr::Identifier(n) if n == crate::parser::SPREAD_BUILTIN) => {
            spread_to_value(arguments)
        }
        Expr::Assignment { target, value } if index_assign_parts(value).is_some() => {
            let (field, index, element) = index_assign_parts(value).expect("checked");
            let mut pairs = vec![
                ("kind", s("index_assign")),
                ("target", s(target)),
                ("index", expr_to_value(index)),
                ("value", expr_to_value(element)),
            ];
            if let Some(field) = field {
                pairs.push(("field", s(field)));
            }
            map(pairs)
        }
        Expr::Call { callee, arguments } => map(vec![
            ("kind", s("call")),
            ("callee", expr_to_value(callee)),
            ("target", s(&call_target(callee))),
            ("args", list(arguments.iter().map(arg_to_value).collect())),
        ]),
        Expr::FieldAccess { object, field } => map(vec![
            ("kind", s("field")),
            ("object", expr_to_value(object)),
            ("field", s(field)),
        ]),
        Expr::BinaryOp { left, op, right } => map(vec![
            ("kind", s("binop")),
            ("op", s(op.symbol())),
            ("left", expr_to_value(left)),
            ("right", expr_to_value(right)),
        ]),
        Expr::UnaryOp { op, operand } => map(vec![
            ("kind", s("unary")),
            ("op", s(op.symbol())),
            ("operand", expr_to_value(operand)),
        ]),
        Expr::If {
            condition,
            then_branch,
            else_branch,
        } => {
            let mut pairs = vec![
                ("kind", s("if")),
                ("condition", expr_to_value(condition)),
                ("then", expr_to_value(then_branch)),
            ];
            if let Some(e) = else_branch {
                pairs.push(("else", expr_to_value(e)));
            }
            map(pairs)
        }
        Expr::Match { value, arms } => map(vec![
            ("kind", s("match")),
            ("value", expr_to_value(value)),
            ("arm_count", Value::Integer(arms.len() as i64)),
            ("arms", list(arms.iter().map(arm_to_value).collect())),
        ]),
        Expr::Lambda {
            parameters, body, ..
        } => map(vec![
            ("kind", s("lambda")),
            ("params", params_to_value(parameters)),
            ("parameters", parameters_to_value(parameters)),
            ("body", expr_to_value(body)),
        ]),
        Expr::Pipeline { left, right } => map(vec![
            ("kind", s("pipeline")),
            ("left", expr_to_value(left)),
            ("right", expr_to_value(right)),
        ]),
        Expr::Block(stmts) => map(vec![
            ("kind", s("block")),
            ("body", list(stmts.iter().map(stmt_to_value).collect())),
        ]),
        Expr::List(items) => map(vec![
            ("kind", s("list")),
            ("items", list(items.iter().map(expr_to_value).collect())),
        ]),
        Expr::Tuple(items) => map(vec![
            ("kind", s("tuple")),
            ("items", list(items.iter().map(expr_to_value).collect())),
        ]),
        Expr::MapLiteral { entries } => map(vec![
            ("kind", s("map")),
            ("entry_count", Value::Integer(entries.len() as i64)),
            (
                "entries",
                list(
                    entries
                        .iter()
                        .map(|e| {
                            map(vec![
                                ("kind", s("entry")),
                                ("key", expr_to_value(&e.key)),
                                ("value", expr_to_value(&e.value)),
                            ])
                        })
                        .collect(),
                ),
            ),
        ]),
        Expr::Index { object, index } => map(vec![
            ("kind", s("index")),
            ("object", expr_to_value(object)),
            ("index", expr_to_value(index)),
        ]),
        Expr::Assignment { target, value } => map(vec![
            ("kind", s("assign")),
            ("target", s(target)),
            ("value", expr_to_value(value)),
        ]),
        Expr::LocalAssign { name, value, .. } => map(vec![
            ("kind", s("assign")),
            ("target", s(name)),
            ("value", expr_to_value(value)),
        ]),
        Expr::Range {
            start,
            end,
            inclusive,
        } => map(vec![
            ("kind", s("range")),
            // `from` and `to`: `start` and `end` are a node's byte span.
            ("from", expr_to_value(start)),
            ("to", expr_to_value(end)),
            ("inclusive", Value::Boolean(*inclusive)),
        ]),
        Expr::ResultOk(inner) => map(vec![("kind", s("ok")), ("value", expr_to_value(inner))]),
        Expr::ResultErr(inner) => map(vec![("kind", s("err")), ("value", expr_to_value(inner))]),
        Expr::Try(inner) => map(vec![("kind", s("try")), ("value", expr_to_value(inner))]),
        Expr::ForLoop {
            variable,
            iterable,
            body,
        } => map(vec![
            ("kind", s("for")),
            ("var", s(variable)),
            ("iterable", expr_to_value(iterable)),
            ("body", expr_to_value(body)),
        ]),
        Expr::ParForLoop {
            variable,
            iterable,
            body,
        } => map(vec![
            ("kind", s("par_for")),
            ("var", s(variable)),
            ("iterable", expr_to_value(iterable)),
            ("body", expr_to_value(body)),
        ]),
        Expr::WhileLoop { condition, body } => map(vec![
            ("kind", s("while")),
            ("condition", expr_to_value(condition)),
            ("body", expr_to_value(body)),
        ]),
        Expr::Loop { body } => map(vec![("kind", s("loop")), ("body", expr_to_value(body))]),
        Expr::Break(v) => {
            let mut pairs = vec![("kind", s("break"))];
            if let Some(e) = v {
                pairs.push(("value", expr_to_value(e)));
            }
            map(pairs)
        }
        Expr::Continue => map(vec![("kind", s("continue"))]),
        Expr::Return(v) => {
            let mut pairs = vec![("kind", s("return"))];
            if let Some(e) = v {
                pairs.push(("value", expr_to_value(e)));
            }
            map(pairs)
        }
        Expr::StructLiteral(sl) => map(vec![
            ("kind", s("struct")),
            ("type", s(&sl.type_name)),
            ("fields", fields_to_value(&sl.fields)),
        ]),
        Expr::AnonymousObject { fields } => map(vec![
            ("kind", s("object")),
            ("fields", fields_to_value(fields)),
        ]),
        Expr::TemplateString { parts } => map(vec![
            ("kind", s("template")),
            (
                "parts",
                list(
                    parts
                        .iter()
                        .map(|p| match p {
                            TemplatePart::Literal(t) => {
                                map(vec![("kind", s("str")), ("value", s(t))])
                            }
                            TemplatePart::Interpolation(e) => expr_to_value(e),
                        })
                        .collect(),
                ),
            ),
        ]),
        Expr::RawString(text) => map(vec![("kind", s("str")), ("value", s(text))]),
        Expr::BitwiseOp { left, right, .. } => map(vec![
            ("kind", s("binop")),
            ("left", expr_to_value(left)),
            ("right", expr_to_value(right)),
        ]),
        // Concurrency and assertion interiors carry sub-expressions where
        // calls hide (`spawn risky()`, `assert_eq(f(), g())`), so they are
        // walked, not collapsed — a lint over the node tree must see them.
        Expr::Spawn(inner) => map(vec![("kind", s("spawn")), ("value", expr_to_value(inner))]),
        Expr::Spread(inner) => map(vec![("kind", s("spread")), ("value", expr_to_value(inner))]),
        Expr::Rest(inner) => map(vec![("kind", s("rest")), ("value", expr_to_value(inner))]),
        Expr::AssertEq {
            actual, expected, ..
        } => map(vec![
            ("kind", s("assert_eq")),
            ("left", expr_to_value(actual)),
            ("right", expr_to_value(expected)),
        ]),
        Expr::AssertNe {
            actual, expected, ..
        } => map(vec![
            ("kind", s("assert_ne")),
            ("left", expr_to_value(actual)),
            ("right", expr_to_value(expected)),
        ]),
        Expr::Assert { condition, .. } => map(vec![
            ("kind", s("assert")),
            ("value", expr_to_value(condition)),
        ]),
        Expr::AssertTrue { expression, .. } => map(vec![
            ("kind", s("assert_true")),
            ("value", expr_to_value(expression)),
        ]),
        Expr::AssertFalse { expression, .. } => map(vec![
            ("kind", s("assert_false")),
            ("value", expr_to_value(expression)),
        ]), // No catch-all: the match is exhaustive, so the *compiler* guarantees
            // every expression variant is converted with its children — a lint
            // over the node tree can never silently miss a call hidden in a
            // subtree. A new `Expr` variant is a build error here until it gets
            // an explicit arm, which is the intended completeness check.
    }
}

/// A literal with spreads (`[...xs, 1]`, `#{ ...m, "k": v }`, `{ ...o }`):
/// `of` is the literal's kind (`list`, `map`, `object`), and `parts`
/// lists, in source order, each run of written entries as a literal node
/// of that kind and each spread as a `spread` node. (A spread of a literal
/// of the same kind reads as a run of its entries, which it is.)
fn spread_to_value(arguments: &[Argument]) -> Value {
    let arg = |a: &Argument| match a {
        Argument::Positional(e) | Argument::Named { value: e, .. } => e.clone(),
    };
    let bare_arg = |a: &Argument| arg(a).into_bare();
    // The first argument is the empty literal the parts are laid over.
    let args = arguments.iter().skip(1).map(arg);
    let of = match arguments.first().map(bare_arg) {
        Some(Expr::List(_)) => "list",
        Some(Expr::MapLiteral { .. }) => "map",
        _ => "object",
    };
    let parts = args
        .map(|e| {
            let same_kind = matches!(
                (e.bare(), of),
                (Expr::List(_), "list")
                    | (Expr::MapLiteral { .. }, "map")
                    | (Expr::AnonymousObject { .. }, "object")
            );
            if same_kind {
                expr_to_value(&e)
            } else {
                map(vec![("kind", s("spread")), ("value", expr_to_value(&e))])
            }
        })
        .collect();
    map(vec![
        ("kind", s("spread_literal")),
        ("of", s(of)),
        ("parts", list(parts)),
    ])
}

/// `xs[i] = v` (`Assignment` of `__index_set(xs, i, v)`) and `m.k[i] = v`
/// (`__index_set_in(m, "k", i, v)`): the field, the index and the value.
fn index_assign_parts(value: &Expr) -> Option<(Option<&str>, &Expr, &Expr)> {
    let Expr::Call { callee, arguments } = value.bare() else {
        return None;
    };
    let Expr::Identifier(name) = callee.bare() else {
        return None;
    };
    let positional = |i: usize| match arguments.get(i) {
        Some(Argument::Positional(e)) => Some(e),
        _ => None,
    };
    if name == crate::parser::INDEX_SET_BUILTIN {
        Some((None, positional(1)?, positional(2)?))
    } else if name == crate::parser::INDEX_SET_IN_BUILTIN {
        match positional(1)?.bare() {
            Expr::String(field) => Some((Some(field.as_str()), positional(2)?, positional(3)?)),
            _ => None,
        }
    } else {
        None
    }
}

fn arg_to_value(a: &Argument) -> Value {
    match a {
        Argument::Positional(e) => expr_to_value(e),
        Argument::Named { name, value } => map(vec![
            ("kind", s("named_arg")),
            ("name", s(name)),
            ("value", expr_to_value(value)),
        ]),
    }
}

/// One `match` arm as a node: the bound-name form of its pattern, its body
/// expression, and its guard when present — so calls inside arm bodies and
/// guards are visible to analysis.
fn arm_to_value(a: &MatchArm) -> Value {
    let mut pairs = vec![
        ("kind", s("arm")),
        // The pattern as a node (its `text` is the bound-names summary
        // this field used to be).
        ("pattern", pattern_to_value(&a.pattern)),
        ("body", expr_to_value(&a.expression)),
    ];
    if let Some(g) = &a.guard {
        pairs.push(("guard", expr_to_value(g)));
    }
    map(pairs)
}

/// Struct-literal / anonymous-object fields as nodes, each with its value
/// expression walked — so a call in a field initializer is not hidden.
fn fields_to_value(fields: &[FieldValue]) -> Value {
    list(
        fields
            .iter()
            .map(|f| {
                map(vec![
                    ("kind", s("field_value")),
                    ("name", s(&f.name)),
                    ("value", expr_to_value(&f.value)),
                ])
            })
            .collect(),
    )
}

/// The dotted call target for a `Call` callee, when it is a plain name or
/// a field chain (`fs.read_file`, `unwrap`) — the field tools look at most.
/// Empty when the callee is a more complex expression.
fn call_target(callee: &Expr) -> String {
    match callee.bare() {
        Expr::Identifier(n) => n.clone(),
        Expr::LocalRef { name, .. } => name.clone(),
        Expr::FieldAccess { object, field } => {
            let base = call_target(object);
            if base.is_empty() {
                field.clone()
            } else {
                format!("{}.{}", base, field)
            }
        }
        _ => String::new(),
    }
}

/// `meta.eval(source)` — the expansion-time form: evaluate olang source
/// in a fresh, pure interpreter and return the program's final value.
/// The evaluation runs in meta mode: no filesystem, network, processes,
/// clock, or randomness — the same sandbox meta fns themselves run in,
/// so a value computed here is a deterministic function of the source.
/// Parse and runtime failures come back as `Err(message)`, since
/// malformed source is a condition the caller can handle.
///
/// Outside macro expansion, builtin dispatch routes string arguments to
/// `Interpreter::eval_source_at_runtime` instead — real evaluation with
/// effects, judged by the run's own capability table. This function
/// still validates every call's argument shape: non-string arguments
/// fall through to the error arms below on both paths.
/// The optional second argument of `meta.eval`: a budget for untrusted
/// or user-authored source. `max_steps` bounds loop iterations and calls
/// (the deterministic bound); `timeout_ms` is wall clock. A sandbox
/// needs one: `while true { }` passes every static check, and without a
/// bound it is a hung thread. Returns `(max_steps, timeout)`.
pub fn eval_budget(
    args: &[Value],
) -> Result<(Option<u64>, Option<std::time::Duration>), Box<dyn std::error::Error>> {
    let mut max_steps = None;
    let mut timeout = None;
    match args.get(1).map(|v| v.as_options()).as_deref() {
        None => {}
        Some(Value::Map(options)) => {
            for (key, value) in options.iter() {
                match (key.as_str(), value) {
                    ("max_steps", Value::Integer(n)) if *n >= 0 => max_steps = Some(*n as u64),
                    ("timeout_ms", Value::Integer(n)) if *n >= 0 => {
                        timeout = Some(std::time::Duration::from_millis(*n as u64))
                    }
                    ("max_steps" | "timeout_ms", other) => {
                        return Err(format!(
                            "meta.eval: {} must be a non-negative Int, got {}",
                            key,
                            other.type_name()
                        )
                        .into());
                    }
                    (other, _) => {
                        return Err(format!(
                            "meta.eval: unknown option '{}' — the options are max_steps and timeout_ms",
                            other
                        )
                        .into());
                    }
                }
            }
        }
        Some(other) => {
            return Err(format!(
                "meta.eval: options must be a map like #{{ \"max_steps\": 100000 }}, got {}",
                other.type_name()
            )
            .into());
        }
    }
    Ok((max_steps, timeout))
}

fn meta_eval(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let source = match args.first() {
        Some(Value::String(text)) => text.as_str().to_string(),
        Some(Value::Ok(inner)) if matches!(inner.as_ref(), Value::String(_)) => {
            return Err(
                "meta.eval: expected a source string, got Ok(String) — unwrap the read first: meta.eval(unwrap(f))"
                    .into(),
            );
        }
        // The other common miss: feeding meta.parse output back in.
        // Nodes are the analysis format — patterns and types are
        // summarized, so they cannot be turned back into a program.
        Some(Value::List(_)) => {
            return Err(
                "meta.eval: expected a source string, got a list of parse nodes — meta.parse output is for analysis and cannot be evaluated. Keep the source text and eval that: meta.eval(src)"
                    .into(),
            );
        }
        Some(Value::Ok(inner)) if matches!(inner.as_ref(), Value::List(_)) => {
            return Err(
                "meta.eval: expected a source string, got meta.parse output — parse nodes are for analysis and cannot be evaluated. Eval the source text itself: meta.eval(src)"
                    .into(),
            );
        }
        other => {
            return Err(format!(
                "meta.eval: expected a source string, got {}",
                other.map(|v| v.type_name()).unwrap_or_default()
            )
            .into());
        }
    };
    let (max_steps, timeout) = eval_budget(&args)?;
    let program = match crate::parser::Parser::new().parse(&source) {
        Ok(p) => p,
        Err(e) => return Ok(Value::Err(Box::new(s(&format!("{}", e))))),
    };
    let mut interp = crate::interpreter::Interpreter::new();
    interp.set_meta_mode(true);
    interp.set_eval_budget(max_steps, timeout);
    match interp.eval_program(program) {
        Ok(v) => Ok(Value::Ok(Box::new(v))),
        Err(e) => Ok(Value::Err(Box::new(s(&format!("{}", e))))),
    }
}

/// `meta.expand(source)` — the program after macro expansion, as text:
/// `Ok(expanded)` with every `meta fn` gone and every `@` site replaced,
/// or `Err(message)`. What `olang expand FILE` prints, as a value, so a
/// server can hand a browser the expanded bundle and the wasm parser
/// reads it once instead of running the expansion rounds itself.
fn meta_expand(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let source = match args.first() {
        Some(Value::String(text)) => text.as_str(),
        other => {
            return Err(format!(
                "meta.expand: expected a source string, got {}",
                other.map(|v| v.type_name()).unwrap_or_default()
            )
            .into());
        }
    };
    Ok(match crate::expand::expand_source(source) {
        Ok(text) => Value::Ok(Box::new(s(&text))),
        Err(message) => Value::Err(Box::new(s(&message))),
    })
}

/// `meta.lit(value)` — render a value as olang source that evaluates
/// back to it. The generation half of `@bake`: evaluate at expansion
/// time, splice the result as a literal. Strings are escaped, floats
/// keep their `.0`, and containers recurse. A value with no literal
/// form (a function, a native handle) is an error, because there is no
/// honest source text for it.
fn meta_lit(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let value = args.first().ok_or("meta.lit: expected a value")?;
    Ok(s(&render_literal(value)?))
}

fn render_literal(v: &Value) -> Result<String, Box<dyn std::error::Error>> {
    Ok(match v {
        Value::Integer(n) => n.to_string(),
        Value::Float(f) => crate::ast::format_float(*f),
        Value::Boolean(b) => b.to_string(),
        Value::Unit => "()".to_string(),
        Value::String(text) => {
            let mut out = String::with_capacity(text.len() + 2);
            out.push('"');
            for c in text.chars() {
                match c {
                    '"' => out.push_str("\\\""),
                    '\\' => out.push_str("\\\\"),
                    '\n' => out.push_str("\\n"),
                    '\r' => out.push_str("\\r"),
                    '\t' => out.push_str("\\t"),
                    other => out.push(other),
                }
            }
            out.push('"');
            out
        }
        Value::List(items) => {
            let parts: Result<Vec<_>, _> = items.iter().map(render_literal).collect();
            format!("[{}]", parts?.join(", "))
        }
        Value::Tuple(items) => {
            let parts: Result<Vec<_>, _> = items.iter().map(render_literal).collect();
            format!("({})", parts?.join(", "))
        }
        Value::Map(entries) => {
            // The map's own order: the literal makes the same map again.
            let keys: Vec<_> = entries.keys().collect();
            let parts: Result<Vec<_>, _> = keys
                .iter()
                .map(|k| {
                    render_literal(entries.get(*k).expect("key exists"))
                        .map(|rendered| format!("{}: {}", render_key(k), rendered))
                })
                .collect();
            format!("#{{{}}}", parts?.join(", "))
        }
        // An anonymous object as its literal, `{ a: 1, b: "x" }`, its
        // fields in order (Heddle's recordings converted objects to maps
        // first). Its fields are identifiers, as its literal wrote them.
        Value::Struct { type_name, fields } if type_name == "Object" => {
            let keys: Vec<_> = fields.keys().collect();
            let parts: Result<Vec<_>, Box<dyn std::error::Error>> = keys
                .iter()
                .map(|k| {
                    let ident = k
                        .chars()
                        .next()
                        .is_some_and(|c| c.is_alphabetic() || c == '_')
                        && k.chars().all(|c| c.is_alphanumeric() || c == '_');
                    if !ident {
                        return Err(
                            format!("meta.lit: an object's field {k:?} is not a name").into()
                        );
                    }
                    Ok(format!(
                        "{}: {}",
                        k,
                        render_literal(fields.get(*k).expect("key exists"))?
                    ))
                })
                .collect();
            format!("{{ {} }}", parts?.join(", "))
        }
        Value::Ok(inner) => format!("Ok({})", render_literal(inner)?),
        Value::Err(inner) => format!("Err({})", render_literal(inner)?),
        other => {
            return Err(format!(
                "meta.lit: a {} has no literal source form",
                other.type_name()
            )
            .into());
        }
    })
}

fn render_key(k: &str) -> String {
    let mut out = String::with_capacity(k.len() + 2);
    out.push('"');
    for c in k.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

// The `meta.fresh` counter: thread-local, reset at the start of every
// expansion (`expand::expand_source`). Expansion is single-threaded by
// construction (meta mode refuses spawn and the parallel builtins), so a
// thread-local is exactly per-expansion state — same source always
// yields the same names, and concurrent expansions on other threads
// (parallel tests, parallel builds) cannot race each other's counters.
thread_local! {
    static FRESH_COUNTER: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

pub fn reset_fresh_counter() {
    FRESH_COUNTER.with(|c| c.set(0));
}

/// `meta.fresh(prefix)` — a name no program writes by hand, for macro
/// temporaries that must not collide with call-site bindings. The
/// counter is reset per expansion; sites expand in text order, so a
/// given program yields the same names on every run and every re-parse.
fn meta_fresh(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let prefix = match args.first() {
        Some(Value::String(text)) => text.as_str().to_string(),
        other => {
            return Err(format!(
                "meta.fresh: expected a name prefix as a String, got {}",
                other.map(|v| v.type_name()).unwrap_or_default()
            )
            .into());
        }
    };
    let n = FRESH_COUNTER.with(|c| {
        let n = c.get();
        c.set(n + 1);
        n
    });
    Ok(s(&format!("{}__m{}", prefix, n)))
}

// The `meta.exports` table: the literal top-level bindings of every
// module the expanding file imports, by import path, installed by
// `expand::expand_impl` for the duration of one expansion. Thread-local
// for the same reason the fresh counter is: expansion is single-threaded
// and per-thread state is exactly per-expansion state.
thread_local! {
    static EXPORTS: std::cell::RefCell<Option<crate::ast::ValueMap>> =
        const { std::cell::RefCell::new(None) };
}

pub fn set_exports(table: crate::ast::ValueMap) {
    EXPORTS.with(|e| *e.borrow_mut() = Some(table));
}

/// Clears the exports table when the expansion that installed it ends,
/// whichever way it ends.
pub struct ExportsGuard;

impl ExportsGuard {
    pub fn new() -> Self {
        EXPORTS.with(|e| *e.borrow_mut() = Some(crate::ast::ValueMap::default()));
        ExportsGuard
    }
}

impl Default for ExportsGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for ExportsGuard {
    fn drop(&mut self) {
        EXPORTS.with(|e| *e.borrow_mut() = None);
    }
}

/// `meta.exports(path)` — the literal values the module at `path` (as
/// the expanding file imports it: `"lib.decl"`, `"shuttle"`) binds at
/// its top level, as a map from name to value. `Err` when the module is
/// not one the file imports, and outside expansion, where there is no
/// importing file to read from.
fn meta_exports(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let path = match args.first() {
        Some(Value::String(text)) => text.replace('/', "."),
        other => {
            return Err(format!(
                "meta.exports: expected a module path as a String, got {}",
                other.map(|v| v.type_name()).unwrap_or_default()
            )
            .into());
        }
    };
    EXPORTS.with(|e| match e.borrow().as_ref() {
        None => Ok(Value::Err(Box::new(s(
            "meta.exports is answered at expansion time, inside a meta fn: at runtime a module's bindings are reached by importing them",
        )))),
        Some(table) => match table.get(&path) {
            Some(v) => Ok(Value::Ok(Box::new(v.clone()))),
            None => {
                let mut known: Vec<&str> = table.keys().map(|k| k.as_str()).collect();
                known.sort_unstable();
                Ok(Value::Err(Box::new(s(&format!(
                    "meta.exports: '{}' is not a module this file imports{}",
                    path,
                    if known.is_empty() {
                        String::new()
                    } else {
                        format!(" (imported: {})", known.join(", "))
                    }
                )))))
            }
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str) -> Vec<Value> {
        match &meta_parse(vec![s(src)]).unwrap() {
            Value::Ok(inner) => match inner.as_ref() {
                Value::List(items) => items.as_ref().clone(),
                _ => panic!("expected a list"),
            },
            other => panic!("expected Ok(list), got {:?}", other),
        }
    }

    fn get<'a>(node: &'a Value, key: &str) -> &'a Value {
        match node {
            Value::Map(m) => {
                static UNIT: Value = Value::Unit;
                m.get(key).unwrap_or(&UNIT)
            }
            _ => panic!("not a map"),
        }
    }

    fn text(v: &Value) -> String {
        match v {
            Value::String(s) => s.to_string(),
            _ => panic!("not a string: {:?}", v),
        }
    }

    #[test]
    fn top_level_declarations_are_tagged_maps() {
        let p = parse("use geometry { area, circle }\nshare fn f(x) = x + 1\nlet n = 42\n");
        assert_eq!(p.len(), 3);
        assert_eq!(text(get(&p[0], "kind")), "use");
        assert_eq!(text(get(&p[0], "path")), "geometry");
        assert_eq!(text(get(&p[1], "kind")), "fn");
        assert_eq!(text(get(&p[1], "name")), "f");
        assert_eq!(*get(&p[1], "shared"), Value::Boolean(true));
        assert_eq!(text(get(&p[2], "kind")), "let");
        assert_eq!(text(get(&p[2], "name")), "n");
    }

    #[test]
    fn statements_carry_source_position() {
        let p = parse("let a = 1\nlet b = 2\n");
        assert_eq!(*get(&p[0], "line"), Value::Integer(1));
        assert_eq!(*get(&p[1], "line"), Value::Integer(2));
    }

    #[test]
    fn calls_expose_their_dotted_target() {
        let p = parse("fn f() = fs.read_file(\"x\")\n");
        let body = get(&p[0], "body");
        assert_eq!(text(get(body, "kind")), "call");
        assert_eq!(text(get(body, "target")), "fs.read_file");
        // a bare call target too
        let p2 = parse("fn g() = unwrap(h())\n");
        assert_eq!(text(get(get(&p2[0], "body"), "target")), "unwrap");
    }

    #[test]
    fn conversion_is_total_and_errors_are_ordinary() {
        // A program exercising many node kinds must not panic.
        let src = "\
fn big(xs) = {
    let s = xs |> map((x) => x * 2) |> fold(0, (a, b) => a + b)
    for i in 0..10 { println(show(i)) }
    match s { 0 => \"zero\", _ => \"nonzero\" }
    if s > 0 => Ok(s) else => Err(\"neg\")
}
type Shape = enum { Circle(Float), Rect(Float, Float) }
";
        let p = parse(src);
        assert!(!p.is_empty());
        // a syntax error round-trips as Err, never a crash
        match meta_parse(vec![s("fn (")]).unwrap() {
            Value::Err(_) => {}
            other => panic!("expected Err, got {:?}", other),
        }
    }
}
