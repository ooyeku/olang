//! `olang lsp` — the language server, speaking LSP over stdio.
//!
//! v1 scope, built entirely on machinery the compiler already has:
//! - Diagnostics on open/change: parse errors (with the parser's own
//!   line/column info) and analyzer warnings (unused variables, attached
//!   at their declaration site by text search — the analyzer does not
//!   yet carry spans).
//! - Completions: keywords, global builtins, stdlib modules and their
//!   functions, plus `fn`/`type`/`let` names scanned from the document.
//! - Whole-document formatting through the `olang fmt` engine (AST-
//!   verified, whitespace-only).
//!
//! The server is intentionally stateless beyond an open-document map:
//! every edit re-parses whole files (olang files are small; the parser
//! is fast), which keeps correctness trivial.

use std::collections::HashMap;
use std::error::Error;

use lsp_server::{Connection, Message, Request, RequestId, Response};
use lsp_types::notification::{
    DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument, Notification as _,
    PublishDiagnostics,
};
use lsp_types::request::{Completion, Formatting, Request as _};
use lsp_types::{
    CompletionItem, CompletionItemKind, CompletionOptions, CompletionResponse, Diagnostic,
    DiagnosticSeverity, InitializeParams, OneOf, Position, PublishDiagnosticsParams, Range,
    ServerCapabilities, TextDocumentSyncCapability, TextDocumentSyncKind, TextEdit, Url as Uri,
};

use crate::analyze::Analyzer;
use crate::parser::{ParseError, Parser as OlangParser};

const KEYWORDS: &[&str] = &[
    "fn", "let", "mut", "type", "if", "else", "match", "for", "par", "while", "loop", "break",
    "continue", "return", "true", "false", "async", "await", "spawn", "try", "catch", "error",
    "share", "use", "struct", "enum", "test", "trait", "impl", "in",
];

const GLOBAL_BUILTINS: &[&str] = &[
    "print",
    "println",
    "show",
    "to_string",
    "typeof",
    "to_int",
    "to_float",
    "len",
    "range",
    "head",
    "tail",
    "take",
    "skip",
    "reverse",
    "sort",
    "contains",
    "concat",
    "cons",
    "chunk",
    "flatten",
    "enumerate",
    "zip",
    "group_by",
    "sum",
    "min",
    "max",
    "average",
    "clamp",
    "map",
    "filter",
    "par_map",
    "par_filter",
    "fold",
    "reduce",
    "find",
    "map_filtered",
    "split",
    "join",
    "starts_with",
    "ends_with",
    "map_get",
    "map_has_key",
    "map_keys",
    "map_values",
    "map_len",
    "map_set",
    "map_remove",
    "map_merge",
    "map_clear",
    "entries",
    "unwrap",
    "unwrap_or",
    "implements",
];

const MODULES: &[&str] = &[
    "str", "col", "math", "json", "toml", "csv", "re", "dates", "time", "random", "crypto",
    "base64", "fs", "os", "http", "db", "testing", "ods", "stats", "plot",
];

pub fn run() -> Result<(), Box<dyn Error + Sync + Send>> {
    let (connection, io_threads) = Connection::stdio();

    let capabilities = ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        completion_provider: Some(CompletionOptions {
            trigger_characters: Some(vec![".".to_string()]),
            ..Default::default()
        }),
        document_formatting_provider: Some(OneOf::Left(true)),
        hover_provider: Some(lsp_types::HoverProviderCapability::Simple(true)),
        definition_provider: Some(OneOf::Left(true)),
        ..Default::default()
    };
    let init_params = connection.initialize(serde_json::to_value(capabilities)?)?;
    let _init: InitializeParams = serde_json::from_value(init_params)?;

    main_loop(&connection)?;
    // The writer thread ends only when the sender side drops; drop the
    // connection before joining or the join never returns.
    drop(connection);
    io_threads.join()?;
    Ok(())
}

fn main_loop(connection: &Connection) -> Result<(), Box<dyn Error + Sync + Send>> {
    // Open documents: uri -> current text.
    let mut docs: HashMap<Uri, String> = HashMap::new();

    for msg in &connection.receiver {
        match msg {
            Message::Request(req) => {
                if connection.handle_shutdown(&req)? {
                    return Ok(());
                }
                handle_request(connection, &docs, req)?;
            }
            Message::Notification(note) => match note.method.as_str() {
                DidOpenTextDocument::METHOD => {
                    let params: lsp_types::DidOpenTextDocumentParams =
                        serde_json::from_value(note.params)?;
                    let uri = params.text_document.uri;
                    let text = params.text_document.text;
                    publish(connection, &uri, &text)?;
                    docs.insert(uri, text);
                }
                DidChangeTextDocument::METHOD => {
                    let params: lsp_types::DidChangeTextDocumentParams =
                        serde_json::from_value(note.params)?;
                    // FULL sync: the last change carries the whole text.
                    if let Some(change) = params.content_changes.into_iter().last() {
                        let uri = params.text_document.uri;
                        publish(connection, &uri, &change.text)?;
                        docs.insert(uri, change.text);
                    }
                }
                DidCloseTextDocument::METHOD => {
                    let params: lsp_types::DidCloseTextDocumentParams =
                        serde_json::from_value(note.params)?;
                    docs.remove(&params.text_document.uri);
                    // Clear diagnostics for closed files.
                    send_diagnostics(connection, params.text_document.uri, Vec::new())?;
                }
                _ => {}
            },
            Message::Response(_) => {}
        }
    }
    Ok(())
}

fn handle_request(
    connection: &Connection,
    docs: &HashMap<Uri, String>,
    req: Request,
) -> Result<(), Box<dyn Error + Sync + Send>> {
    match req.method.as_str() {
        Completion::METHOD => {
            let (id, params): (RequestId, lsp_types::CompletionParams) =
                req.extract(Completion::METHOD)?;
            let text = docs
                .get(&params.text_document_position.text_document.uri)
                .map(String::as_str)
                .unwrap_or("");
            let items = completions(text);
            respond(connection, id, &CompletionResponse::Array(items))?;
        }
        lsp_types::request::HoverRequest::METHOD => {
            let (id, params): (RequestId, lsp_types::HoverParams) =
                req.extract(lsp_types::request::HoverRequest::METHOD)?;
            let pos = params.text_document_position_params;
            let dir = doc_dir(&pos.text_document.uri);
            let text = docs
                .get(&pos.text_document.uri)
                .map(String::as_str)
                .unwrap_or("");
            respond(connection, id, &hover(text, pos.position, dir.as_deref()))?;
        }
        lsp_types::request::GotoDefinition::METHOD => {
            let (id, params): (RequestId, lsp_types::GotoDefinitionParams) =
                req.extract(lsp_types::request::GotoDefinition::METHOD)?;
            let pos = params.text_document_position_params;
            let uri = pos.text_document.uri.clone();
            let dir = doc_dir(&uri);
            let text = docs.get(&uri).map(String::as_str).unwrap_or("");
            let loc = definition(text, pos.position, dir.as_deref()).and_then(|(file, range)| {
                let target = match file {
                    // Cross-file: the module file's own URI.
                    Some(path) => Uri::from_file_path(path).ok()?,
                    None => uri,
                };
                Some(lsp_types::GotoDefinitionResponse::Scalar(
                    lsp_types::Location { uri: target, range },
                ))
            });
            respond(connection, id, &loc)?;
        }
        Formatting::METHOD => {
            let (id, params): (RequestId, lsp_types::DocumentFormattingParams) =
                req.extract(Formatting::METHOD)?;
            let edits = docs
                .get(&params.text_document.uri)
                .map(|text| format_edits(text))
                .unwrap_or_default();
            respond(connection, id, &edits)?;
        }
        _ => {
            // Politely decline anything we did not advertise.
            let resp = Response::new_err(
                req.id,
                lsp_server::ErrorCode::MethodNotFound as i32,
                format!("unhandled method {}", req.method),
            );
            connection.sender.send(Message::Response(resp))?;
        }
    }
    Ok(())
}

fn respond<T: serde::Serialize>(
    connection: &Connection,
    id: RequestId,
    value: &T,
) -> Result<(), Box<dyn Error + Sync + Send>> {
    let resp = Response::new_ok(id, serde_json::to_value(value)?);
    connection.sender.send(Message::Response(resp))?;
    Ok(())
}

fn publish(
    connection: &Connection,
    uri: &Uri,
    text: &str,
) -> Result<(), Box<dyn Error + Sync + Send>> {
    let dir = doc_dir(uri);
    send_diagnostics(connection, uri.clone(), diagnostics(text, dir.as_deref()))
}

fn send_diagnostics(
    connection: &Connection,
    uri: Uri,
    diagnostics: Vec<Diagnostic>,
) -> Result<(), Box<dyn Error + Sync + Send>> {
    let params = PublishDiagnosticsParams {
        uri,
        diagnostics,
        version: None,
    };
    let note = lsp_server::Notification {
        method: PublishDiagnostics::METHOD.to_string(),
        params: serde_json::to_value(params)?,
    };
    connection.sender.send(Message::Notification(note))?;
    Ok(())
}

// ── cross-file modules ─────────────────────────────────────────────────

/// The directory of a document URI, when it is a real file.
fn doc_dir(uri: &Uri) -> Option<std::path::PathBuf> {
    uri.to_file_path()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
}

// ── diagnostics ────────────────────────────────────────────────────────

fn diagnostics(text: &str, doc_dir: Option<&std::path::Path>) -> Vec<Diagnostic> {
    let parser = OlangParser::new();
    // Syntax always comes from the RAW parse, so its positions point into
    // the buffer the editor shows. For a macro-bearing file, the semantic
    // half then runs on the EXPANDED program — where generated functions
    // exist, so calling them is not an error — and every resulting
    // position is translated back through the expansion's line map: a
    // diagnostic in untouched code lands on its original line with its
    // original column (identical content), and one inside generated code
    // lands on the `@` site with the macro named. Expansion is pure and
    // per-keystroke cheap; a failing macro becomes a diagnostic at its
    // site rather than silence.
    match parser.parse_raw(text) {
        Err(e) => vec![parse_error_diagnostic(text, &e)],
        Ok(program) if crate::expand::program_uses_macros(&program) => {
            match crate::expand::expand_source_mapped_with_dir(text, doc_dir) {
                Err(message) => vec![expansion_error_diagnostic(text, &message)],
                Ok(exp) => match parser.parse_raw(&exp.text) {
                    // Splice validation makes this near-impossible; if it
                    // happens, say so at the top rather than hiding it.
                    Err(e) => vec![Diagnostic {
                        range: Range::new(Position::new(0, 0), Position::new(0, 1)),
                        severity: Some(DiagnosticSeverity::ERROR),
                        source: Some("olang".to_string()),
                        message: format!("macro expansion produced text that does not parse: {e}"),
                        ..Default::default()
                    }],
                    Ok(expanded_program) => {
                        let mut out = program_diagnostics(&expanded_program, &exp.text, doc_dir);
                        for d in &mut out {
                            map_diagnostic(d, &exp.line_origins);
                        }
                        out.sort_by_key(|d| (d.range.start.line, d.range.start.character));
                        out.dedup_by(|a, b| a.range == b.range && a.message == b.message);
                        out
                    }
                },
            }
        }
        Ok(program) => program_diagnostics(&program, text, doc_dir),
    }
}

/// Translate one diagnostic's range from expanded-text coordinates back
/// to the buffer, through the expansion's line map.
fn map_diagnostic(d: &mut Diagnostic, origins: &[crate::expand::LineOrigin]) {
    let line = d.range.start.line as usize; // 0-based
    match origins.get(line) {
        Some(crate::expand::LineOrigin::Original(m)) => {
            let new_line = (*m as u32).saturating_sub(1);
            d.range.start.line = new_line;
            d.range.end.line = new_line;
        }
        Some(crate::expand::LineOrigin::Generated {
            macro_name,
            site_line,
        }) => {
            let new_line = (*site_line as u32).saturating_sub(1);
            d.range = Range::new(Position::new(new_line, 0), Position::new(new_line, 1));
            d.message = format!("in code generated by @{}: {}", macro_name, d.message);
        }
        None => {}
    }
}

/// A macro that failed to expand: the message carries "(line N)" from the
/// expander's site attribution — anchor the diagnostic there.
fn expansion_error_diagnostic(text: &str, message: &str) -> Diagnostic {
    let line = message
        .split("(line ")
        .nth(1)
        .and_then(|rest| {
            rest.chars()
                .take_while(|c| c.is_ascii_digit())
                .collect::<String>()
                .parse::<u32>()
                .ok()
        })
        .map(|l| l.saturating_sub(1))
        .unwrap_or_else(|| text.lines().position(|l| l.contains('@')).unwrap_or(0) as u32);
    Diagnostic {
        range: Range::new(Position::new(line, 0), Position::new(line, 1)),
        severity: Some(DiagnosticSeverity::ERROR),
        source: Some("olang".to_string()),
        message: message.to_string(),
        ..Default::default()
    }
}

/// The semantic half of `diagnostics`, over an already-parsed program and
/// the text its positions refer to (the buffer, or the expanded program).
fn program_diagnostics(
    program: &crate::ast::Program,
    text: &str,
    doc_dir: Option<&std::path::Path>,
) -> Vec<Diagnostic> {
    {
        {
            let mut analyzer = Analyzer::new();
            match analyzer.analyze_comprehensive(program) {
                Err(e) => {
                    // Semantic error without a span: attach at the named
                    // identifier's first occurrence where one exists.
                    let msg = e.to_string();
                    let range = name_in_error(&msg)
                        .and_then(|name| find_identifier(text, &name))
                        .unwrap_or_else(|| Range::new(Position::new(0, 0), Position::new(0, 0)));
                    vec![Diagnostic {
                        range,
                        severity: Some(DiagnosticSeverity::ERROR),
                        source: Some("olang".to_string()),
                        message: msg,
                        ..Default::default()
                    }]
                }
                Ok(report) => {
                    let decls = declarations(text);
                    let mut out: Vec<Diagnostic> = report
                        .unused_variables
                        .iter()
                        .filter_map(|name| {
                            let range = decls
                                .iter()
                                .find(|(n, _, _)| n == name)
                                .map(|(n, _, span)| span_range(*span, n.len()))
                                .or_else(|| find_declaration(text, name))?;
                            Some(Diagnostic {
                                range,
                                severity: Some(DiagnosticSeverity::WARNING),
                                source: Some("olang".to_string()),
                                message: format!("unused variable: {}", name),
                                ..Default::default()
                            })
                        })
                        .collect();
                    // Provable annotation violations: the runtime would
                    // reject these, so surface them as errors pre-run.
                    out.extend(
                        {
                            let modules = crate::tools::check::module_programs(text, doc_dir);
                            let context: Vec<&crate::ast::Program> =
                                modules.iter().map(|(_, _, p)| p).collect();
                            crate::tools::check::check_program_with_context(&context, program)
                        }
                        .into_iter()
                        .map(|d| {
                            let line = d.line.saturating_sub(1);
                            let col = d.column.saturating_sub(1);
                            let severity = if d.warning {
                                DiagnosticSeverity::WARNING
                            } else {
                                DiagnosticSeverity::ERROR
                            };
                            Diagnostic {
                                range: Range::new(
                                    Position::new(line, col),
                                    Position::new(line, col + 1),
                                ),
                                severity: Some(severity),
                                source: Some("olang".to_string()),
                                message: d.message,
                                ..Default::default()
                            }
                        }),
                    );
                    out
                }
            }
        }
    }
}

fn parse_error_diagnostic(text: &str, e: &ParseError) -> Diagnostic {
    let (line, column, message) = match e {
        ParseError::Pest(pe) => {
            let (l, c) = match pe.line_col {
                pest::error::LineColLocation::Pos((l, c)) => (l, c),
                pest::error::LineColLocation::Span((l, c), _) => (l, c),
            };
            (l, c, format!("syntax error: {}", pe.variant.message()))
        }
        ParseError::InvalidSyntaxWithPosition {
            message,
            line,
            column,
            ..
        }
        | ParseError::UnexpectedTokenWithPosition {
            token: message,
            line,
            column,
            ..
        } => (*line, *column, message.clone()),
        other => (1, 1, other.to_string()),
    };
    // LSP is 0-based; the parser is 1-based. Highlight to end of line.
    let l = line.saturating_sub(1) as u32;
    let c = column.saturating_sub(1) as u32;
    let line_len = text
        .lines()
        .nth(l as usize)
        .map(|s| s.chars().count() as u32)
        .unwrap_or(c + 1);
    Diagnostic {
        range: Range::new(Position::new(l, c), Position::new(l, line_len.max(c + 1))),
        severity: Some(DiagnosticSeverity::ERROR),
        source: Some("olang".to_string()),
        message,
        ..Default::default()
    }
}

fn name_in_error(msg: &str) -> Option<String> {
    msg.rsplit_once(": ")
        .map(|(_, name)| name.trim().to_string())
        .filter(|n| n.chars().all(|c| c.is_alphanumeric() || c == '_'))
}

/// 0-based range of the first standalone occurrence of `name`.
fn find_identifier(text: &str, name: &str) -> Option<Range> {
    for (ln, line) in text.lines().enumerate() {
        let mut start = 0;
        while let Some(pos) = line[start..].find(name) {
            let at = start + pos;
            let before_ok = at == 0
                || !line[..at]
                    .chars()
                    .next_back()
                    .is_some_and(|c| c.is_alphanumeric() || c == '_');
            let after = line[at + name.len()..].chars().next();
            let after_ok = !after.is_some_and(|c| c.is_alphanumeric() || c == '_');
            if before_ok && after_ok {
                return Some(Range::new(
                    Position::new(ln as u32, at as u32),
                    Position::new(ln as u32, (at + name.len()) as u32),
                ));
            }
            start = at + name.len().max(1);
        }
    }
    None
}

/// The declaration site of `name` (`let [mut] name`, `fn name`), falling
/// back to any standalone occurrence.
fn find_declaration(text: &str, name: &str) -> Option<Range> {
    for (ln, line) in text.lines().enumerate() {
        for prefix in ["let mut ", "let ", "fn "] {
            if let Some(kw) = line.find(prefix) {
                let after = &line[kw + prefix.len()..];
                if let Some(rest) = after.strip_prefix(name) {
                    let boundary = rest.chars().next();
                    if !boundary.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                        let col = kw + prefix.len();
                        return Some(Range::new(
                            Position::new(ln as u32, col as u32),
                            Position::new(ln as u32, (col + name.len()) as u32),
                        ));
                    }
                }
            }
        }
    }
    find_identifier(text, name)
}

// ── completions ────────────────────────────────────────────────────────

fn completions(text: &str) -> Vec<CompletionItem> {
    let mut items: Vec<CompletionItem> = Vec::new();
    let mut push = |label: &str, kind: CompletionItemKind, detail: &str| {
        items.push(CompletionItem {
            label: label.to_string(),
            kind: Some(kind),
            detail: Some(detail.to_string()),
            ..Default::default()
        });
    };

    for kw in KEYWORDS {
        push(kw, CompletionItemKind::KEYWORD, "keyword");
    }
    for b in GLOBAL_BUILTINS {
        push(b, CompletionItemKind::FUNCTION, "builtin");
    }
    for m in MODULES {
        push(m, CompletionItemKind::MODULE, "stdlib module");
    }

    // Document symbols: fn/type/let names from the current text.
    let mut seen = std::collections::HashSet::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        for (prefix, kind, detail) in [
            ("fn ", CompletionItemKind::FUNCTION, "function (this file)"),
            ("type ", CompletionItemKind::STRUCT, "type (this file)"),
            ("let mut ", CompletionItemKind::VARIABLE, "variable"),
            ("let ", CompletionItemKind::VARIABLE, "variable"),
        ] {
            if let Some(rest) = trimmed.strip_prefix(prefix) {
                let name: String = rest
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                if !name.is_empty() && seen.insert(name.clone()) {
                    push(&name, kind, detail);
                }
                break;
            }
        }
    }
    items
}

// ── declarations by span (the spans rung) ──────────────────────────────

/// A top-level declaration's name, kind label, and 1-based span.
fn declarations(text: &str) -> Vec<(String, String, (u32, u32))> {
    let parser = OlangParser::new();
    let Ok(program) = parser.parse_raw(text) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for stmt in &program.statements {
        use crate::ast::{Pattern, Statement};
        let stmt = stmt.unwrapped();
        match stmt {
            Statement::FunctionDecl(f) => {
                if let Some(span) = f.name_span {
                    let params: Vec<&str> = f.parameters.iter().map(|p| p.name.as_str()).collect();
                    out.push((
                        f.name.clone(),
                        format!("fn {}({})", f.name, params.join(", ")),
                        span,
                    ));
                }
            }
            Statement::TypeDecl(t) => {
                if let Some(span) = t.name_span {
                    out.push((t.name.clone(), format!("type {}", t.name), span));
                }
            }
            Statement::LetDecl(l) => {
                if let (Some(span), Pattern::Identifier(name)) = (l.name_span, &l.pattern) {
                    out.push((name.clone(), format!("let {}", name), span));
                }
            }
            _ => {}
        }
    }
    out
}

/// The identifier under the cursor (0-based LSP position).
fn word_at(text: &str, pos: Position) -> Option<String> {
    let line = text.lines().nth(pos.line as usize)?;
    let chars: Vec<char> = line.chars().collect();
    let mut start = (pos.character as usize).min(chars.len());
    while start > 0 && (chars[start - 1].is_alphanumeric() || chars[start - 1] == '_') {
        start -= 1;
    }
    let mut end = start;
    while end < chars.len() && (chars[end].is_alphanumeric() || chars[end] == '_') {
        end += 1;
    }
    (end > start).then(|| chars[start..end].iter().collect())
}

fn span_range(span: (u32, u32), name_len: usize) -> Range {
    let (line, col) = (span.0.saturating_sub(1), span.1.saturating_sub(1));
    Range::new(
        Position::new(line, col),
        Position::new(line, col + name_len as u32),
    )
}

fn hover(text: &str, pos: Position, doc_dir: Option<&std::path::Path>) -> Option<lsp_types::Hover> {
    let word = word_at(text, pos)?;
    let local = declarations(text).into_iter().find(|(n, _, _)| *n == word);
    // Imported names hover with their module's signature.
    let (name, detail) = match local {
        Some((n, d, _)) => (n, d),
        None => {
            let (path, src, prog) = crate::tools::check::module_programs(text, doc_dir)
                .into_iter()
                .find(|(_, src, _)| declarations(src).iter().any(|(n, _, _)| *n == word))?;
            let detail = crate::tools::check::hover_types(&prog)
                .remove(&word)
                .or_else(|| {
                    declarations(&src)
                        .into_iter()
                        .find(|(n, _, _)| *n == word)
                        .map(|(_, d, _)| d)
                })?;
            let from = path
                .file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_default();
            (word.clone(), format!("{}    // from {}", detail, from))
        }
    };
    // Upgrade the declaration text with the checker's type knowledge —
    // annotated signatures rendered in full, unannotated lets with their
    // inferred types when the checker knows one.
    let detail = OlangParser::new()
        .parse_raw(text)
        .ok()
        .and_then(|program| crate::tools::check::hover_types(&program).remove(&name))
        .unwrap_or(detail);
    Some(lsp_types::Hover {
        contents: lsp_types::HoverContents::Scalar(lsp_types::MarkedString::LanguageString(
            lsp_types::LanguageString {
                language: "olang".to_string(),
                value: detail,
            },
        )),
        range: Some(span_range_at_cursor(text, pos, &name)),
    })
}

fn span_range_at_cursor(text: &str, pos: Position, name: &str) -> Range {
    // Highlight the word under the cursor itself.
    let line = pos.line;
    let col = pos.character.saturating_sub(
        word_at(text, pos)
            .map(|w| w.len() as u32)
            .unwrap_or(0)
            .min(pos.character),
    );
    Range::new(
        Position::new(line, col),
        Position::new(line, col + name.len() as u32),
    )
}

fn definition(
    text: &str,
    pos: Position,
    doc_dir: Option<&std::path::Path>,
) -> Option<(Option<std::path::PathBuf>, Range)> {
    let word = word_at(text, pos)?;
    if let Some((n, _, span)) = declarations(text).into_iter().find(|(n, _, _)| *n == word) {
        return Some((None, span_range(span, n.len())));
    }
    // Not declared here: jump into the module that declares it.
    for (path, src, _) in crate::tools::check::module_programs(text, doc_dir) {
        if let Some((n, _, span)) = declarations(&src).into_iter().find(|(n, _, _)| *n == word) {
            return Some((Some(path), span_range(span, n.len())));
        }
    }
    None
}

// ── formatting ─────────────────────────────────────────────────────────

fn format_edits(text: &str) -> Vec<TextEdit> {
    let formatted = crate::tools::fmt::format_source(text);
    if formatted == text {
        return Vec::new();
    }
    let last_line = text.lines().count() as u32;
    vec![TextEdit {
        range: Range::new(Position::new(0, 0), Position::new(last_line + 1, 0)),
        new_text: formatted,
    }]
}
