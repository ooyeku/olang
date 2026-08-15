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
//! `callee` and `args` are themselves node maps. The representation is
//! faithful for the common shapes a tool inspects (declarations, calls,
//! identifiers, literals, operators, control flow) and summarizes the
//! deep interior (patterns collapse to their bound names, type
//! annotations to their source text) — enough to *analyze* a program, not
//! to perfectly reconstruct one.

use crate::ast::{
    Argument, Expr, FunctionDecl, LetDecl, Parameter, Pattern, ShareDecl, Statement, UseDecl,
    UseItem, Value,
};
use std::collections::HashMap;

pub fn create_meta_module() -> Value {
    let mut module = HashMap::new();
    module.insert("parse".to_string(), builtin("parse", 1));
    Value::Struct {
        type_name: "Module".to_string(),
        fields: module,
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
        _ => Err(format!("Unknown meta function: {}", name).into()),
    }
}

/// `meta.parse(source)` → Ok(list of statement node maps) | Err(message).
fn meta_parse(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let source = match args.first() {
        Some(Value::String(s)) => s.as_str().to_string(),
        _ => return Err("meta.parse: expected a source string".into()),
    };
    match crate::parser::Parser::new().parse(&source) {
        Ok(program) => {
            let nodes: Vec<Value> = program.statements.iter().map(stmt_to_value).collect();
            Ok(Value::Ok(Box::new(list(nodes))))
        }
        Err(e) => Ok(Value::Err(Box::new(s(&format!("{}", e))))),
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
        Statement::AsyncFunctionDecl(d) => map(vec![
            ("kind", s("async_fn")),
            ("name", s(&d.name)),
            ("params", params_to_value(&d.parameters)),
            ("body", expr_to_value(&d.body)),
        ]),
        Statement::TypeDecl(d) => map(vec![
            ("kind", s("type")),
            ("name", s(&d.name)),
            ("type_params", names(&d.type_params)),
        ]),
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
        Expr::TryCatch {
            try_block,
            catch_block,
            ..
        } => map(vec![
            ("kind", s("try_catch")),
            ("body", expr_to_value(try_block)),
            ("handler", expr_to_value(catch_block)),
        ]),
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
        Expr::StructLiteral(sl) => map(vec![("kind", s("struct")), ("type", s(&sl.type_name))]),
        Expr::AnonymousObject { .. } => map(vec![("kind", s("object"))]),
        Expr::TemplateString { .. } => map(vec![("kind", s("template"))]),
        Expr::RawString(text) => map(vec![("kind", s("str")), ("value", s(text))]),
        Expr::BitwiseOp { left, right, .. } => map(vec![
            ("kind", s("binop")),
            ("left", expr_to_value(left)),
            ("right", expr_to_value(right)),
        ]),
        Expr::Spawn(inner) => map(vec![("kind", s("spawn")), ("value", expr_to_value(inner))]),
        Expr::Async { body, .. } => map(vec![("kind", s("async")), ("value", expr_to_value(body))]),
        // The async/promise/assertion interior and any future variant:
        // named generically, not detailed. A tool sees a node it can skip
        // rather than a hole; this keeps `meta.parse` total.
        _ => map(vec![("kind", s("expr"))]),
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
