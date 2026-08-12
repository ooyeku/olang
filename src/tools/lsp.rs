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
    "str", "col", "math", "json", "csv", "re", "dates", "time", "random", "crypto", "base64", "fs",
    "os", "http", "db", "testing", "ods", "stats", "plot",
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
            let text = docs
                .get(&pos.text_document.uri)
                .map(String::as_str)
                .unwrap_or("");
            respond(connection, id, &hover(text, pos.position))?;
        }
        lsp_types::request::GotoDefinition::METHOD => {
            let (id, params): (RequestId, lsp_types::GotoDefinitionParams) =
                req.extract(lsp_types::request::GotoDefinition::METHOD)?;
            let pos = params.text_document_position_params;
            let uri = pos.text_document.uri.clone();
            let text = docs.get(&uri).map(String::as_str).unwrap_or("");
            let loc = definition(text, pos.position).map(|range| {
                lsp_types::GotoDefinitionResponse::Scalar(lsp_types::Location { uri, range })
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
    send_diagnostics(connection, uri.clone(), diagnostics(text))
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

// ── diagnostics ────────────────────────────────────────────────────────

fn diagnostics(text: &str) -> Vec<Diagnostic> {
    let parser = OlangParser::new();
    match parser.parse(text) {
        Err(e) => vec![parse_error_diagnostic(text, &e)],
        Ok(program) => {
            let mut analyzer = Analyzer::new();
            match analyzer.analyze_comprehensive(&program) {
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
                        crate::tools::check::check_program(&program)
                            .into_iter()
                            .map(|d| {
                                let line = d.line.saturating_sub(1);
                                let col = d.column.saturating_sub(1);
                                Diagnostic {
                                    range: Range::new(
                                        Position::new(line, col),
                                        Position::new(line, col + 1),
                                    ),
                                    severity: Some(DiagnosticSeverity::ERROR),
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
    let Ok(program) = parser.parse(text) else {
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

fn hover(text: &str, pos: Position) -> Option<lsp_types::Hover> {
    let word = word_at(text, pos)?;
    let (name, detail, _span) = declarations(text)
        .into_iter()
        .find(|(n, _, _)| *n == word)?;
    // Upgrade the declaration text with the checker's type knowledge —
    // annotated signatures rendered in full, unannotated lets with their
    // inferred types when the checker knows one.
    let detail = OlangParser::new()
        .parse(text)
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

fn definition(text: &str, pos: Position) -> Option<Range> {
    let word = word_at(text, pos)?;
    declarations(text)
        .into_iter()
        .find(|(n, _, _)| *n == word)
        .map(|(n, _, span)| span_range(span, n.len()))
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
