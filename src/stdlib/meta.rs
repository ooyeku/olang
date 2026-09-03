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
use std::collections::HashMap;

pub fn create_meta_module() -> Value {
    let mut module = HashMap::new();
    module.insert("parse".to_string(), builtin("parse", 1));
    module.insert("eval".to_string(), builtin("eval", 1));
    module.insert("lit".to_string(), builtin("lit", 1));
    module.insert("expand".to_string(), builtin("expand", 1));
    module.insert("encode".to_string(), builtin("encode", 1));
    module.insert("fresh".to_string(), builtin("fresh", 1));
    Value::Struct {
        type_name: "Module".to_string(),
        fields: std::sync::Arc::new(module),
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
        "eval" => meta_eval(args),
        "lit" => meta_lit(args),
        "expand" => meta_expand(args),
        "encode" => meta_encode(args),
        "fresh" => meta_fresh(args),
        _ => Err(format!("Unknown meta function: {}", name).into()),
    }
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
    match crate::parser::Parser::new().parse_raw(&source) {
        Ok(program) => {
            let nodes: Vec<Value> = program.statements.iter().map(stmt_to_value).collect();
            Ok(Value::Ok(Box::new(list(nodes))))
        }
        Err(e) => Ok(Value::Err(Box::new(s(&format!("{}", e))))),
    }
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
    match crate::parser::Parser::new().parse(&source) {
        Err(e) => Ok(Value::Err(Box::new(s(&format!("{}", e))))),
        Ok(program) => match crate::olb::encode(&program) {
            Ok(bytes) => Ok(Value::Ok(Box::new(crate::stdlib::bytes::to_value(bytes)))),
            Err(e) => Ok(Value::Err(Box::new(s(&e)))),
        },
    }
}

// ── Value builders ──────────────────────────────────────────────────
fn s(text: &str) -> Value {
    Value::String(std::sync::Arc::new(text.to_string()))
}
fn list(items: Vec<Value>) -> Value {
    Value::List(std::sync::Arc::new(items))
}
fn map(pairs: Vec<(&str, Value)>) -> Value {
    let mut m = HashMap::new();
    for (k, v) in pairs {
        m.insert(k.to_string(), v);
    }
    Value::Map(std::sync::Arc::new(m))
}
fn names(v: &[String]) -> Value {
    list(v.iter().map(|n| s(n)).collect())
}

// ── Statements ──────────────────────────────────────────────────────
fn stmt_to_value(stmt: &Statement) -> Value {
    match stmt {
        Statement::MetaFnDecl { decl, .. } => {
            let mut m_v = match stmt_to_value(&Statement::FunctionDecl(decl.clone())) {
                Value::Map(inner) => (*inner).clone(),
                _ => std::collections::HashMap::new(),
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
            // Attach position to the inner node.
            let inner = stmt_to_value(stmt);
            if let Value::Map(m) = &inner {
                let mut m = (**m).clone();
                m.insert("line".to_string(), Value::Integer(*line as i64));
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
                                        ("type", s(&f.field_type.display_source())),
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
            if let Value::Map(m) = use_to_value(u) {
                let mut m = (*m).clone();
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
            ("start", expr_to_value(start)),
            ("end", expr_to_value(end)),
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
        ("pattern", s(&pattern_name(&a.pattern))),
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
    match callee {
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
    match args.get(1) {
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
            // Deterministic order, so expansion output is reproducible.
            let mut keys: Vec<_> = entries.keys().collect();
            keys.sort();
            let parts: Result<Vec<_>, _> = keys
                .iter()
                .map(|k| {
                    render_literal(entries.get(*k).expect("key exists"))
                        .map(|rendered| format!("{}: {}", render_key(k), rendered))
                })
                .collect();
            format!("#{{{}}}", parts?.join(", "))
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
    Ok(s(&format!("{}_m{}", prefix, n)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str) -> Vec<Value> {
        match meta_parse(vec![s(src)]).unwrap() {
            Value::Ok(inner) => match *inner {
                Value::List(items) => (*items).clone(),
                _ => panic!("expected a list"),
            },
            other => panic!("expected Ok(list), got {:?}", other),
        }
    }

    fn get<'a>(node: &'a Value, key: &str) -> &'a Value {
        match node {
            Value::Map(m) => m.get(key).unwrap_or(&Value::Unit),
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
