//! `olang lsp` — the language server, speaking LSP over stdio.
//!
//! Everything is built on machinery the compiler already has, and the
//! documentation the editor shows is the same registry `:help` and the
//! book speak from:
//! - Diagnostics on open/change: parse errors, analyzer warnings,
//!   provable annotation violations — expansion-aware for macro files,
//!   with positions mapped back to the buffer.
//! - Completions, context-aware: after `mod.` the module's functions
//!   (with signatures and docs from the help registry); otherwise
//!   keywords, globals, modules, and this file's declarations.
//! - Hover for every documented name: builtins and stdlib functions
//!   from the help registry; user declarations (including `share`)
//!   with their real signature, the checker's type knowledge, and the
//!   author's `///` doc block — the editor speaks with the same voice
//!   as `:help`, which reads the identical convention.
//! - Go to definition (local and cross-module), references, rename,
//!   document highlight, document symbols (the outline), and signature
//!   help with active-parameter tracking.
//! - Whole-document formatting through the `olang fmt` engine.
//!
//! Positions cross the wire in UTF-16 code units — the protocol's
//! default — and are converted at every boundary, so a `π` or an emoji
//! earlier in a line never shifts a range. Internally everything is
//! character-indexed.
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

/// The fifteen reserved words, the eight contextual declaration words,
/// and the position-contextual particles — the language.md appendix,
/// exactly. `async`/`await`/`try`/`catch` sat in this list long after
/// they left the language, so the editor was completing keywords that
/// error; a test now diffs this list against the appendix's.
const KEYWORDS: &[&str] = &[
    // reserved
    "fn", "let", "if", "else", "match", "for", "while", "loop", "break", "continue", "return",
    "true", "false", "struct", "enum", // contextual declaration words
    "share", "error", "test", "type", "trait", "impl", "use", "meta",
    // position-contextual
    "mut", "par", "in", "spawn", "self",
];

/// Always-in-scope native modules, plus the embedded olang packages
/// (which need a `use` first — their completion detail says so).
const MODULES: &[&str] = &[
    "str",
    "col",
    "math",
    "json",
    "toml",
    "csv",
    "re",
    "dates",
    "time",
    "random",
    "crypto",
    "base64",
    "bytes",
    "fs",
    "os",
    "http",
    "db",
    "testing",
    "ods",
    "stats",
    "plot",
    "cell",
    "chan",
    "task",
    "proc",
    "caps",
    "meta",
    "bigint",
    "collections",
];
const USE_MODULES: &[&str] = &["cli", "term", "ui", "viz", "dash", "colx", "mathx"];

/// The one help registry, built once: the same entries `:help` renders.
fn help() -> &'static crate::help::HelpSystem {
    static HELP: std::sync::OnceLock<crate::help::HelpSystem> = std::sync::OnceLock::new();
    HELP.get_or_init(crate::help::HelpSystem::new)
}

// ── UTF-16 boundary layer ─────────────────────────────────────────────
// The protocol's positions are UTF-16 code units; the server's internal
// coordinates are character indices. Convert exactly at the wire.

/// Character index for a UTF-16 column on `line`, clamped to its end.
fn utf16_to_char_col(line: &str, utf16: u32) -> usize {
    let mut units = 0u32;
    for (i, ch) in line.chars().enumerate() {
        if units >= utf16 {
            return i;
        }
        units += ch.len_utf16() as u32;
    }
    line.chars().count()
}

/// UTF-16 column for a character index on `line`.
fn char_to_utf16_col(line: &str, chars: usize) -> u32 {
    line.chars().take(chars).map(|c| c.len_utf16() as u32).sum()
}

/// A wire Range on one line of `text`, from character columns.
fn utf16_range(text: &str, line: u32, start_char: usize, end_char: usize) -> Range {
    let l = text.lines().nth(line as usize).unwrap_or("");
    Range::new(
        Position::new(line, char_to_utf16_col(l, start_char)),
        Position::new(line, char_to_utf16_col(l, end_char)),
    )
}

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
        document_symbol_provider: Some(OneOf::Left(true)),
        references_provider: Some(OneOf::Left(true)),
        document_highlight_provider: Some(OneOf::Left(true)),
        rename_provider: Some(OneOf::Left(true)),
        inlay_hint_provider: Some(OneOf::Left(true)),
        code_action_provider: Some(lsp_types::CodeActionProviderCapability::Simple(true)),
        workspace_symbol_provider: Some(OneOf::Left(true)),
        signature_help_provider: Some(lsp_types::SignatureHelpOptions {
            trigger_characters: Some(vec!["(".to_string(), ",".to_string()]),
            retrigger_characters: None,
            work_done_progress_options: Default::default(),
        }),
        ..Default::default()
    };
    let init_params = connection.initialize(serde_json::to_value(capabilities)?)?;
    let init: InitializeParams = serde_json::from_value(init_params)?;
    #[allow(deprecated)]
    let root: Option<std::path::PathBuf> = init
        .workspace_folders
        .as_ref()
        .and_then(|f| f.first())
        .and_then(|f| f.uri.to_file_path().ok())
        .or_else(|| init.root_uri.as_ref().and_then(|u| u.to_file_path().ok()));

    main_loop(&connection, root)?;
    // The writer thread ends only when the sender side drops; drop the
    // connection before joining or the join never returns.
    drop(connection);
    io_threads.join()?;
    Ok(())
}

fn main_loop(
    connection: &Connection,
    root: Option<std::path::PathBuf>,
) -> Result<(), Box<dyn Error + Sync + Send>> {
    // Open documents: uri -> current text.
    let mut docs: HashMap<Uri, String> = HashMap::new();

    for msg in &connection.receiver {
        match msg {
            Message::Request(req) => {
                if connection.handle_shutdown(&req)? {
                    return Ok(());
                }
                handle_request(connection, &docs, req, root.as_deref())?;
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
    root: Option<&std::path::Path>,
) -> Result<(), Box<dyn Error + Sync + Send>> {
    match req.method.as_str() {
        Completion::METHOD => {
            let (id, params): (RequestId, lsp_types::CompletionParams) =
                req.extract(Completion::METHOD)?;
            let text = docs
                .get(&params.text_document_position.text_document.uri)
                .map(String::as_str)
                .unwrap_or("");
            let items = completions(text, params.text_document_position.position);
            respond(connection, id, &CompletionResponse::Array(items))?;
        }
        lsp_types::request::DocumentSymbolRequest::METHOD => {
            let (id, params): (RequestId, lsp_types::DocumentSymbolParams) =
                req.extract(lsp_types::request::DocumentSymbolRequest::METHOD)?;
            let text = docs
                .get(&params.text_document.uri)
                .map(String::as_str)
                .unwrap_or("");
            respond(
                connection,
                id,
                &lsp_types::DocumentSymbolResponse::Nested(document_symbols(text)),
            )?;
        }
        lsp_types::request::References::METHOD => {
            let (id, params): (RequestId, lsp_types::ReferenceParams) =
                req.extract(lsp_types::request::References::METHOD)?;
            let uri = params.text_document_position.text_document.uri.clone();
            let text = docs.get(&uri).map(String::as_str).unwrap_or("");
            let locs: Option<Vec<lsp_types::Location>> =
                word_at(text, params.text_document_position.position).map(|(word, _)| {
                    occurrences(text, &word)
                        .into_iter()
                        .map(|range| lsp_types::Location {
                            uri: uri.clone(),
                            range,
                        })
                        .collect()
                });
            respond(connection, id, &locs)?;
        }
        lsp_types::request::DocumentHighlightRequest::METHOD => {
            let (id, params): (RequestId, lsp_types::DocumentHighlightParams) =
                req.extract(lsp_types::request::DocumentHighlightRequest::METHOD)?;
            let pos = params.text_document_position_params;
            let text = docs
                .get(&pos.text_document.uri)
                .map(String::as_str)
                .unwrap_or("");
            let highlights: Option<Vec<lsp_types::DocumentHighlight>> = word_at(text, pos.position)
                .map(|(word, _)| {
                    occurrences(text, &word)
                        .into_iter()
                        .map(|range| lsp_types::DocumentHighlight { range, kind: None })
                        .collect()
                });
            respond(connection, id, &highlights)?;
        }
        lsp_types::request::Rename::METHOD => {
            let (id, params): (RequestId, lsp_types::RenameParams) =
                req.extract(lsp_types::request::Rename::METHOD)?;
            let uri = params.text_document_position.text_document.uri.clone();
            let text = docs.get(&uri).map(String::as_str).unwrap_or("");
            match rename(
                text,
                params.text_document_position.position,
                &params.new_name,
            ) {
                Ok(edits) => {
                    let mut changes = std::collections::HashMap::new();
                    changes.insert(uri, edits);
                    respond(
                        connection,
                        id,
                        &lsp_types::WorkspaceEdit {
                            changes: Some(changes),
                            ..Default::default()
                        },
                    )?;
                }
                Err(message) => {
                    let resp = Response::new_err(
                        id,
                        lsp_server::ErrorCode::InvalidRequest as i32,
                        message,
                    );
                    connection.sender.send(Message::Response(resp))?;
                }
            }
        }
        lsp_types::request::SignatureHelpRequest::METHOD => {
            let (id, params): (RequestId, lsp_types::SignatureHelpParams) =
                req.extract(lsp_types::request::SignatureHelpRequest::METHOD)?;
            let pos = params.text_document_position_params;
            let text = docs
                .get(&pos.text_document.uri)
                .map(String::as_str)
                .unwrap_or("");
            respond(connection, id, &signature_help(text, pos.position))?;
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
        lsp_types::request::InlayHintRequest::METHOD => {
            let (id, params): (RequestId, lsp_types::InlayHintParams) =
                req.extract(lsp_types::request::InlayHintRequest::METHOD)?;
            let hints = docs
                .get(&params.text_document.uri)
                .map(|text| inlay_hints(text, params.range))
                .unwrap_or_default();
            respond(connection, id, &hints)?;
        }
        lsp_types::request::CodeActionRequest::METHOD => {
            let (id, params): (RequestId, lsp_types::CodeActionParams) =
                req.extract(lsp_types::request::CodeActionRequest::METHOD)?;
            let actions = docs
                .get(&params.text_document.uri)
                .map(|text| {
                    code_actions(
                        text,
                        &params.text_document.uri,
                        params.range,
                        &params.context.diagnostics,
                    )
                })
                .unwrap_or_default();
            respond(connection, id, &actions)?;
        }
        lsp_types::request::WorkspaceSymbolRequest::METHOD => {
            let (id, params): (RequestId, lsp_types::WorkspaceSymbolParams) =
                req.extract(lsp_types::request::WorkspaceSymbolRequest::METHOD)?;
            let symbols = workspace_symbols(root, &params.query);
            respond(connection, id, &symbols)?;
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
    // The scoping pass first: assigning to an immutable or undeclared
    // binding fails at runtime before a single statement runs, so the
    // editor must say so — and its message is what the `let mut` quick
    // fix reads.
    let mut scope_diags: Vec<Diagnostic> = {
        let mut known = crate::scoping::Predefined::new();
        crate::scoping::validate_program(program, &mut known)
            .into_iter()
            .map(|e| Diagnostic {
                range: Range::new(
                    Position::new(e.line.saturating_sub(1), e.column.saturating_sub(1)),
                    Position::new(e.line.saturating_sub(1), e.column.saturating_sub(1) + 1),
                ),
                severity: Some(DiagnosticSeverity::ERROR),
                source: Some("olang".to_string()),
                message: e.message,
                ..Default::default()
            })
            .collect()
    };
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
                    scope_diags.push(Diagnostic {
                        range,
                        severity: Some(DiagnosticSeverity::ERROR),
                        source: Some("olang".to_string()),
                        message: msg,
                        ..Default::default()
                    });
                    scope_diags
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
                                .map(|(n, _, span)| span_range(text, *span, n.chars().count()))
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
                            let col = d.column.saturating_sub(1) as usize;
                            let severity = if d.warning {
                                DiagnosticSeverity::WARNING
                            } else {
                                DiagnosticSeverity::ERROR
                            };
                            Diagnostic {
                                range: utf16_range(text, line, col, col + 1),
                                severity: Some(severity),
                                source: Some("olang".to_string()),
                                message: d.message,
                                ..Default::default()
                            }
                        }),
                    );
                    out.append(&mut scope_diags);
                    // The scoping pass and the checker overlap on
                    // assignment errors; one report per finding.
                    out.sort_by_key(|d| (d.range.start.line, d.range.start.character));
                    out.dedup_by(|a, b| a.message == b.message && a.range == b.range);
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
    // LSP is 0-based; the parser is 1-based, in characters. Highlight to
    // end of line, converted to UTF-16 at the wire.
    let l = line.saturating_sub(1) as u32;
    let c = column.saturating_sub(1);
    let line_len = text
        .lines()
        .nth(l as usize)
        .map(|s| s.chars().count())
        .unwrap_or(c + 1);
    Diagnostic {
        range: utf16_range(text, l, c, line_len.max(c + 1)),
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

/// 0-based wire range of the first standalone occurrence of `name`.
fn find_identifier(text: &str, name: &str) -> Option<Range> {
    occurrences(text, name).into_iter().next()
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
                        let col_chars = line[..kw + prefix.len()].chars().count();
                        return Some(utf16_range(
                            text,
                            ln as u32,
                            col_chars,
                            col_chars + name.chars().count(),
                        ));
                    }
                }
            }
        }
    }
    find_identifier(text, name)
}

// ── completions ────────────────────────────────────────────────────────

/// Context-aware completion. After `mod.` the items are that module's
/// functions, with signatures and documentation from the help registry —
/// the same entries `:help` prints. Anywhere else: keywords, global
/// builtins (from the registry, so nothing rotted survives here),
/// modules, and this file's own declarations.
fn completions(text: &str, pos: Position) -> Vec<CompletionItem> {
    // The module receiver, if the cursor sits right after `name.`.
    let line = text.lines().nth(pos.line as usize).unwrap_or("");
    // On a `use` line, complete importable module names: the embedded
    // olang packages, the shelf, and the local lib/ files.
    {
        let cursor = utf16_to_char_col(line, pos.character);
        let head: String = line.chars().take(cursor).collect();
        let t = head.trim_start();
        let after_use = t.strip_prefix("share ").unwrap_or(t).strip_prefix("use ");
        if let Some(rest) = after_use
            && rest
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '.')
        {
            return use_completions();
        }
    }
    let cursor = utf16_to_char_col(line, pos.character);
    let chars: Vec<char> = line.chars().collect();
    let mut i = cursor;
    // Skip back over the partial word being typed.
    while i > 0 && (chars[i - 1].is_alphanumeric() || chars[i - 1] == '_') {
        i -= 1;
    }
    if i > 0 && chars[i - 1] == '.' {
        let mut j = i - 1;
        while j > 0 && (chars[j - 1].is_alphanumeric() || chars[j - 1] == '_') {
            j -= 1;
        }
        let receiver: String = chars[j..i - 1].iter().collect();
        if MODULES.contains(&receiver.as_str()) || USE_MODULES.contains(&receiver.as_str()) {
            return module_completions(&receiver);
        }
    }

    let mut items: Vec<CompletionItem> = Vec::new();
    for kw in KEYWORDS {
        items.push(CompletionItem {
            label: kw.to_string(),
            kind: Some(CompletionItemKind::KEYWORD),
            detail: Some("keyword".to_string()),
            ..Default::default()
        });
    }
    // Global builtins: every registry entry without a module prefix.
    let mut names = help().get_function_names();
    names.sort();
    for name in names {
        if name.contains('.') {
            continue;
        }
        let doc = help().get_function(&name);
        items.push(CompletionItem {
            label: name.clone(),
            kind: Some(CompletionItemKind::FUNCTION),
            detail: doc.map(|d| d.syntax.clone()),
            documentation: doc.map(doc_markup),
            ..Default::default()
        });
    }
    for m in MODULES {
        items.push(CompletionItem {
            label: m.to_string(),
            kind: Some(CompletionItemKind::MODULE),
            detail: Some("stdlib module".to_string()),
            ..Default::default()
        });
    }
    for m in USE_MODULES {
        items.push(CompletionItem {
            label: m.to_string(),
            kind: Some(CompletionItemKind::MODULE),
            detail: Some(format!("embedded package (use {m})")),
            ..Default::default()
        });
    }
    let mut seen = std::collections::HashSet::new();
    for (name, detail, _) in declarations(text) {
        if seen.insert(name.clone()) {
            let kind = if detail.starts_with("fn ") {
                CompletionItemKind::FUNCTION
            } else if detail.starts_with("type ") {
                CompletionItemKind::STRUCT
            } else {
                CompletionItemKind::VARIABLE
            };
            items.push(CompletionItem {
                label: name,
                kind: Some(kind),
                detail: Some(detail),
                ..Default::default()
            });
        }
    }
    items
}

fn module_completions(module: &str) -> Vec<CompletionItem> {
    let mut names = help().functions_in_module(module);
    names.sort();
    names
        .into_iter()
        .map(|qualified| {
            let doc = help().get_function(&qualified);
            let label = qualified
                .rsplit_once('.')
                .map(|(_, f)| f.to_string())
                .unwrap_or(qualified.clone());
            CompletionItem {
                label,
                kind: Some(CompletionItemKind::FUNCTION),
                detail: doc.map(|d| d.syntax.clone()),
                documentation: doc.map(doc_markup),
                ..Default::default()
            }
        })
        .collect()
}

fn doc_markup(d: &crate::help::FunctionDoc) -> lsp_types::Documentation {
    lsp_types::Documentation::MarkupContent(lsp_types::MarkupContent {
        kind: lsp_types::MarkupKind::Markdown,
        value: format!(
            "```olang\n{} -> {}\n```\n\n{}",
            d.syntax, d.return_type, d.description
        ),
    })
}

// ── declarations by span (the spans rung) ──────────────────────────────

/// A top-level declaration's name, kind label, and 1-based span. When
/// the file does not parse — which is most moments in a live editor,
/// mid-keystroke — falls back to a line scan, so hover, definition, and
/// completion keep working while the user types.
fn declarations(text: &str) -> Vec<(String, String, (u32, u32))> {
    let parser = OlangParser::new();
    let Ok(program) = parser.parse_raw(text) else {
        return scan_declarations(text);
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
            Statement::MetaFnDecl { decl, .. } => {
                if let Some(span) = decl.name_span {
                    let params: Vec<&str> =
                        decl.parameters.iter().map(|p| p.name.as_str()).collect();
                    out.push((
                        decl.name.clone(),
                        format!("meta fn {}({})", decl.name, params.join(", ")),
                        span,
                    ));
                }
            }
            Statement::ShareDecl(sd) => {
                use crate::ast::ShareDecl;
                match sd {
                    ShareDecl::Function(f) => {
                        if let Some(span) = f.name_span {
                            let params: Vec<&str> =
                                f.parameters.iter().map(|p| p.name.as_str()).collect();
                            out.push((
                                f.name.clone(),
                                format!("share fn {}({})", f.name, params.join(", ")),
                                span,
                            ));
                        }
                    }
                    ShareDecl::Let(l) => {
                        if let (Some(span), Pattern::Identifier(name)) = (l.name_span, &l.pattern) {
                            out.push((name.clone(), format!("share let {}", name), span));
                        }
                    }
                    ShareDecl::Type(t) => {
                        if let Some(span) = t.name_span {
                            out.push((t.name.clone(), format!("share type {}", t.name), span));
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    out
}

/// Declarations by text scan — the mid-edit fallback. 1-based spans,
/// like the parser's.
fn scan_declarations(text: &str) -> Vec<(String, String, (u32, u32))> {
    let mut out = Vec::new();
    for (ln, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        let indent = line.chars().count() - trimmed.chars().count();
        for (prefix, label) in [
            ("meta fn ", "meta fn"),
            ("share fn ", "share fn"),
            ("share let ", "share let"),
            ("share type ", "share type"),
            ("fn ", "fn"),
            ("type ", "type"),
            ("let mut ", "let"),
            ("let ", "let"),
        ] {
            if let Some(rest) = trimmed.strip_prefix(prefix) {
                let name: String = rest
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                if !name.is_empty() {
                    let col = indent + prefix.chars().count();
                    out.push((
                        name.clone(),
                        format!("{label} {name}"),
                        (ln as u32 + 1, col as u32 + 1),
                    ));
                }
                break;
            }
        }
    }
    out
}

/// The document outline: every top-level declaration, plus test blocks.
fn document_symbols(text: &str) -> Vec<lsp_types::DocumentSymbol> {
    let mut out: Vec<lsp_types::DocumentSymbol> = Vec::new();
    let mut push = |name: String, detail: String, kind: lsp_types::SymbolKind, range: Range| {
        #[allow(deprecated)]
        out.push(lsp_types::DocumentSymbol {
            name,
            detail: Some(detail),
            kind,
            tags: None,
            deprecated: None,
            range,
            selection_range: range,
            children: None,
        });
    };
    for (name, detail, span) in declarations(text) {
        let kind = if detail.starts_with("fn ") || detail.starts_with("meta fn ") {
            lsp_types::SymbolKind::FUNCTION
        } else if detail.starts_with("type ") {
            lsp_types::SymbolKind::STRUCT
        } else {
            lsp_types::SymbolKind::VARIABLE
        };
        push(
            name.clone(),
            detail,
            kind,
            span_range(text, span, name.chars().count()),
        );
    }
    // Test blocks, by text scan — like the declarations fallback, this
    // keeps the outline complete while the file is mid-edit.
    for (ln, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("test ")
            && let Some(name) = rest.trim_start().strip_prefix('"')
            && let Some((name, _)) = name.split_once('"')
        {
            let range = utf16_range(text, ln as u32, 0, 4);
            push(
                format!("test \"{name}\""),
                "test block".to_string(),
                lsp_types::SymbolKind::EVENT,
                range,
            );
        }
    }
    out
}

/// The identifier under the cursor (wire position), and its qualified
/// form when preceded by `module.` — `("trim", Some("str.trim"))`.
fn word_at(text: &str, pos: Position) -> Option<(String, Option<String>)> {
    let line = text.lines().nth(pos.line as usize)?;
    let chars: Vec<char> = line.chars().collect();
    let cursor = utf16_to_char_col(line, pos.character);
    let mut start = cursor.min(chars.len());
    while start > 0 && (chars[start - 1].is_alphanumeric() || chars[start - 1] == '_') {
        start -= 1;
    }
    let mut end = start;
    while end < chars.len() && (chars[end].is_alphanumeric() || chars[end] == '_') {
        end += 1;
    }
    if end == start {
        return None;
    }
    let word: String = chars[start..end].iter().collect();
    let qualified = (start > 0 && chars[start - 1] == '.').then(|| {
        let mut j = start - 1;
        while j > 0 && (chars[j - 1].is_alphanumeric() || chars[j - 1] == '_') {
            j -= 1;
        }
        let receiver: String = chars[j..start - 1].iter().collect();
        format!("{receiver}.{word}")
    });
    Some((word, qualified))
}

/// Every standalone occurrence of `name` in `text`, as wire ranges.
/// Word-boundary exact, so `count` never matches `counter`.
fn occurrences(text: &str, name: &str) -> Vec<Range> {
    let mut out = Vec::new();
    for (ln, line) in text.lines().enumerate() {
        let chars: Vec<char> = line.chars().collect();
        let target: Vec<char> = name.chars().collect();
        let mut i = 0;
        while i + target.len() <= chars.len() {
            if chars[i..i + target.len()] == target[..] {
                let before_ok = i == 0 || !(chars[i - 1].is_alphanumeric() || chars[i - 1] == '_');
                let after = chars.get(i + target.len());
                let after_ok = !after.is_some_and(|c| c.is_alphanumeric() || *c == '_');
                if before_ok && after_ok {
                    out.push(utf16_range(text, ln as u32, i, i + target.len()));
                    i += target.len();
                    continue;
                }
            }
            i += 1;
        }
    }
    out
}

fn span_range(text: &str, span: (u32, u32), name_len: usize) -> Range {
    // Parser spans are 1-based (line, char-column).
    let (line, col) = (span.0.saturating_sub(1), span.1.saturating_sub(1) as usize);
    utf16_range(text, line, col, col + name_len)
}

fn hover(text: &str, pos: Position, doc_dir: Option<&std::path::Path>) -> Option<lsp_types::Hover> {
    let (word, qualified) = word_at(text, pos)?;
    // Registry entries first for qualified names (`str.trim`), then local
    // declarations, then bare registry names, then imported modules.
    if let Some(q) = &qualified
        && let Some(d) = help().get_function(q)
    {
        return Some(help_hover(d));
    }
    if let Some((name, detail, _)) = declarations(text).into_iter().find(|(n, _, _)| *n == word) {
        // The checker's view carries annotation types but degrades an
        // unannotated `fn double(x)` to `fn double` — keep whichever
        // string says more.
        let typed = OlangParser::new()
            .parse_raw(text)
            .ok()
            .and_then(|program| crate::tools::check::hover_types(&program).remove(&name));
        let sig = match typed {
            Some(t) if t.len() >= detail.len() => t,
            _ => detail,
        };
        return Some(decl_hover(text, &name, sig, None));
    }
    if let Some(d) = help().get_function(&word) {
        return Some(help_hover(d));
    }
    // Imported names hover with their module's signature.
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
    Some(decl_hover(&src, &word, detail, Some(&from)))
}

/// A user declaration's hover: the real signature, and the author's
/// `///` doc block when one exists — the editor speaks with the same
/// voice as `:help`, which reads the identical convention.
fn decl_hover(source: &str, name: &str, sig: String, from: Option<&str>) -> lsp_types::Hover {
    let item = crate::tools::doc::extract(source)
        .items
        .into_iter()
        .find(|i| i.name == name);
    let shared = item.as_ref().map(|i| i.shared).unwrap_or(false);
    // In a file that does not currently parse, the declaration scanner
    // degrades to bare names; the doc extractor's signature is
    // text-level and keeps the parameter list — prefer it when richer.
    let sig = match &item {
        Some(i) if !sig.contains('(') && i.signature.contains('(') => {
            format!("{} {}", i.kind, i.signature)
        }
        _ => sig,
    };
    let mut code = String::new();
    if shared && !sig.starts_with("share ") {
        code.push_str("share ");
    }
    code.push_str(&sig);
    if let Some(from) = from {
        code.push_str("    // from ");
        code.push_str(from);
    }
    match item.map(|i| i.doc).filter(|d| !d.is_empty()) {
        Some(doc) => lsp_types::Hover {
            contents: lsp_types::HoverContents::Markup(lsp_types::MarkupContent {
                kind: lsp_types::MarkupKind::Markdown,
                value: format!("```olang\n{}\n```\n\n{}", code, doc),
            }),
            range: None,
        },
        None => code_hover(code),
    }
}

/// Parameter-name inlay hints at call sites. The parameter lists come
/// from this file's own declarations and the help registry; arguments
/// are split at top-level commas per line, so a multi-line call hints
/// the arguments on the line the call starts on. An argument that
/// already reads as the parameter's name is not hinted.
fn inlay_hints(text: &str, range: Range) -> Vec<lsp_types::InlayHint> {
    let mut params_of: HashMap<String, Vec<String>> = HashMap::new();
    for (name, detail, _) in declarations(text) {
        if let Some(open) = detail.find('(') {
            let inner = detail[open + 1..].trim_end_matches(')');
            let ps: Vec<String> = inner
                .split(',')
                .map(|p| p.split(':').next().unwrap_or("").trim().to_string())
                .filter(|p| !p.is_empty())
                .collect();
            if !ps.is_empty() {
                params_of.insert(name, ps);
            }
        }
    }
    let registry_params = |name: &str| -> Option<Vec<String>> {
        let d = help().get_function(name)?;
        let ps: Vec<String> = d
            .parameters
            .iter()
            .map(|p| {
                p.split(&[':', ' '][..])
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_string()
            })
            .filter(|p| !p.is_empty() && p.chars().all(|c| c.is_alphanumeric() || c == '_'))
            .collect();
        if ps.is_empty() { None } else { Some(ps) }
    };

    let mut hints = Vec::new();
    for (ln, line) in text.lines().enumerate() {
        let ln = ln as u32;
        if ln < range.start.line || ln > range.end.line {
            continue;
        }
        let chars: Vec<char> = line.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            // A comment ends the scannable part of the line.
            if chars[i] == '/' && chars.get(i + 1) == Some(&'/') {
                break;
            }
            // Skip string-ish content wholesale.
            if chars[i] == '"' || chars[i] == '`' {
                let quote = chars[i];
                i += 1;
                while i < chars.len() {
                    if chars[i] == '\\' {
                        i += 2;
                        continue;
                    }
                    if chars[i] == quote {
                        break;
                    }
                    i += 1;
                }
                i += 1;
                continue;
            }
            if !(chars[i].is_ascii_lowercase() || chars[i] == '_') {
                i += 1;
                continue;
            }
            // A word — possibly dotted — possibly a call.
            let start = i;
            while i < chars.len()
                && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '.')
            {
                i += 1;
            }
            if chars.get(i) != Some(&'(') {
                continue;
            }
            let callee: String = chars[start..i].iter().collect();
            let ps = params_of
                .get(callee.rsplit('.').next().unwrap_or(&callee))
                .cloned()
                .or_else(|| registry_params(&callee));
            let Some(ps) = ps else {
                continue;
            };
            // A single-parameter call reads fine bare — hints there are
            // noise (`math.sqrt(number: x)`).
            if ps.len() < 2 {
                continue;
            }
            // Split the arguments at top-level commas, this line only.
            let open = i;
            let mut depth = 1i32;
            let mut j = open + 1;
            let mut arg_starts = vec![j];
            while j < chars.len() && depth > 0 {
                match chars[j] {
                    '(' | '[' | '{' => depth += 1,
                    ')' | ']' | '}' => depth -= 1,
                    '"' | '`' => {
                        let q = chars[j];
                        j += 1;
                        while j < chars.len() && chars[j] != q {
                            if chars[j] == '\\' {
                                j += 1;
                            }
                            j += 1;
                        }
                    }
                    ',' if depth == 1 => arg_starts.push(j + 1),
                    _ => {}
                }
                j += 1;
            }
            let end = if depth == 0 { j - 1 } else { chars.len() };
            for (k, &a) in arg_starts.iter().enumerate() {
                let Some(p) = ps.get(k) else { break };
                let mut a = a;
                while a < end && chars[a] == ' ' {
                    a += 1;
                }
                if a >= end {
                    break;
                }
                // Don't hint an argument that IS the parameter name, or
                // a lambda (the arrow says everything already).
                let arg_end = arg_starts.get(k + 1).map(|n| n - 1).unwrap_or(end);
                let arg_text: String = chars[a..arg_end.min(chars.len())].iter().collect();
                let arg_text = arg_text.trim();
                if arg_text == p || arg_text.starts_with('(') && arg_text.contains("=>") {
                    continue;
                }
                hints.push(lsp_types::InlayHint {
                    position: Position {
                        line: ln,
                        character: char_to_utf16_col(line, a),
                    },
                    label: lsp_types::InlayHintLabel::String(format!("{}:", p)),
                    kind: Some(lsp_types::InlayHintKind::PARAMETER),
                    text_edits: None,
                    tooltip: None,
                    padding_left: None,
                    padding_right: Some(true),
                    data: None,
                });
            }
        }
    }
    hints
}

/// Code actions: document a declaration, and make a binding mutable
/// when the scoping pass said the assignment needs `mut`.
fn code_actions(
    text: &str,
    uri: &Uri,
    range: Range,
    diagnostics: &[Diagnostic],
) -> Vec<lsp_types::CodeActionOrCommand> {
    let mut out = Vec::new();
    let lines: Vec<&str> = text.lines().collect();

    // "Add /// documentation" on an undocumented declaration line.
    let ln = range.start.line as usize;
    if let Some(line) = lines.get(ln) {
        let t = line.trim_start();
        let is_decl = ["fn ", "share fn ", "meta fn ", "type ", "share type "]
            .iter()
            .any(|p| t.starts_with(p));
        let documented = ln > 0
            && lines
                .get(ln - 1)
                .map(|p| p.trim_start().starts_with("///"))
                .unwrap_or(false);
        if is_decl && !documented {
            let indent: String = line.chars().take_while(|c| *c == ' ').collect();
            let edit = TextEdit {
                range: Range {
                    start: Position {
                        line: ln as u32,
                        character: 0,
                    },
                    end: Position {
                        line: ln as u32,
                        character: 0,
                    },
                },
                new_text: format!("{}/// \n", indent),
            };
            out.push(action(uri, "Add /// documentation", edit, None));
        }
    }

    // "Declare with `let mut`" from the scoping diagnostic.
    for d in diagnostics {
        let msg = &d.message;
        let Some(rest) = msg.strip_prefix("cannot assign to '") else {
            continue;
        };
        let Some(name) = rest.split('\'').next() else {
            continue;
        };
        if !msg.contains("not declared mutable") {
            continue;
        }
        // The nearest `let NAME` above the assignment.
        let assign_line = d.range.start.line as usize;
        for back in (0..=assign_line.min(lines.len().saturating_sub(1))).rev() {
            let line = lines[back];
            let t = line.trim_start();
            let pat = format!("let {}", name);
            if t.starts_with(&pat) && !t.starts_with(&format!("let mut {}", name)) {
                let col = (line.len() - t.len() + 4) as u32; // after "let "
                let edit = TextEdit {
                    range: Range {
                        start: Position {
                            line: back as u32,
                            character: col,
                        },
                        end: Position {
                            line: back as u32,
                            character: col,
                        },
                    },
                    new_text: "mut ".to_string(),
                };
                out.push(action(
                    uri,
                    &format!("Declare '{}' with `let mut`", name),
                    edit,
                    Some(d.clone()),
                ));
                break;
            }
        }
    }
    out
}

fn action(
    uri: &Uri,
    title: &str,
    edit: TextEdit,
    diagnostic: Option<Diagnostic>,
) -> lsp_types::CodeActionOrCommand {
    let mut changes = HashMap::new();
    changes.insert(uri.clone(), vec![edit]);
    lsp_types::CodeActionOrCommand::CodeAction(lsp_types::CodeAction {
        title: title.to_string(),
        kind: Some(lsp_types::CodeActionKind::QUICKFIX),
        diagnostics: diagnostic.map(|d| vec![d]),
        edit: Some(lsp_types::WorkspaceEdit {
            changes: Some(changes),
            ..Default::default()
        }),
        command: None,
        is_preferred: Some(true),
        disabled: None,
        data: None,
    })
}

/// Project-wide symbol search: every declaration in every `.ol` file
/// under the workspace root whose name contains the query.
fn workspace_symbols(
    root: Option<&std::path::Path>,
    query: &str,
) -> Vec<lsp_types::SymbolInformation> {
    let Some(root) = root else {
        return Vec::new();
    };
    let q = query.to_lowercase();
    let mut out = Vec::new();
    for file in super::discover_ol_files(root) {
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        let Ok(uri) = Uri::from_file_path(&file) else {
            continue;
        };
        for (name, detail, span) in declarations(&text) {
            if !q.is_empty() && !name.to_lowercase().contains(&q) {
                continue;
            }
            let kind = if detail.contains("fn ") {
                lsp_types::SymbolKind::FUNCTION
            } else if detail.starts_with("type") || detail.contains("type ") {
                lsp_types::SymbolKind::STRUCT
            } else {
                lsp_types::SymbolKind::VARIABLE
            };
            #[allow(deprecated)]
            out.push(lsp_types::SymbolInformation {
                name: name.clone(),
                kind,
                tags: None,
                deprecated: None,
                location: lsp_types::Location {
                    uri: uri.clone(),
                    range: span_range(&text, span, name.chars().count()),
                },
                container_name: file.file_name().map(|f| f.to_string_lossy().into_owned()),
            });
            if out.len() >= 128 {
                return out;
            }
        }
    }
    out
}

/// Everything `use` can name here: embedded stdlib packages, the
/// user's shelf, and `lib.<stem>` for the files beside the buffer.
fn use_completions() -> Vec<CompletionItem> {
    let mut items: Vec<CompletionItem> = Vec::new();
    for name in crate::stdlib::embedded::names() {
        items.push(CompletionItem {
            label: name.to_string(),
            kind: Some(CompletionItemKind::MODULE),
            detail: Some("embedded olang package".to_string()),
            ..Default::default()
        });
    }
    if let Ok(shelf) = crate::pkg::shelf::Shelf::load() {
        for name in shelf.libraries.keys() {
            items.push(CompletionItem {
                label: name.clone(),
                kind: Some(CompletionItemKind::MODULE),
                detail: Some("from your shelf".to_string()),
                ..Default::default()
            });
        }
    }
    if let Ok(entries) = std::fs::read_dir("lib") {
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().map(|x| x == "ol").unwrap_or(false)
                && let Some(stem) = p.file_stem()
            {
                items.push(CompletionItem {
                    label: format!("lib.{}", stem.to_string_lossy()),
                    kind: Some(CompletionItemKind::MODULE),
                    detail: Some("local module".to_string()),
                    ..Default::default()
                });
            }
        }
    }
    items.sort_by(|a, b| a.label.cmp(&b.label));
    items
}

fn code_hover(detail: String) -> lsp_types::Hover {
    lsp_types::Hover {
        contents: lsp_types::HoverContents::Scalar(lsp_types::MarkedString::LanguageString(
            lsp_types::LanguageString {
                language: "olang".to_string(),
                value: detail,
            },
        )),
        range: None,
    }
}

fn help_hover(d: &crate::help::FunctionDoc) -> lsp_types::Hover {
    lsp_types::Hover {
        contents: lsp_types::HoverContents::Markup(lsp_types::MarkupContent {
            kind: lsp_types::MarkupKind::Markdown,
            value: format!(
                "```olang\n{} -> {}\n```\n\n{}",
                d.syntax, d.return_type, d.description
            ),
        }),
        range: None,
    }
}

fn definition(
    text: &str,
    pos: Position,
    doc_dir: Option<&std::path::Path>,
) -> Option<(Option<std::path::PathBuf>, Range)> {
    let (word, _) = word_at(text, pos)?;
    if let Some((n, _, span)) = declarations(text).into_iter().find(|(n, _, _)| *n == word) {
        return Some((None, span_range(text, span, n.chars().count())));
    }
    // Not declared here: jump into the module that declares it. The range
    // converts against the MODULE's text — its lines, its columns.
    for (path, src, _) in crate::tools::check::module_programs(text, doc_dir) {
        if let Some((n, _, span)) = declarations(&src).into_iter().find(|(n, _, _)| *n == word) {
            return Some((Some(path), span_range(&src, span, n.chars().count())));
        }
    }
    None
}

/// Rename: every standalone occurrence in this file. Refused for names
/// that are not the user's to change — keywords, builtins, stdlib — and
/// for invalid identifiers, with a message saying why.
fn rename(text: &str, pos: Position, new_name: &str) -> Result<Vec<TextEdit>, String> {
    let Some((word, qualified)) = word_at(text, pos) else {
        return Err("nothing to rename here".to_string());
    };
    if qualified.is_some() || help().get_function(&word).is_some() {
        return Err(format!(
            "'{word}' is a standard-library name — it cannot be renamed"
        ));
    }
    if KEYWORDS.contains(&word.as_str()) {
        return Err(format!("'{word}' is a keyword"));
    }
    let valid = new_name
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic())
        && new_name.chars().all(|c| c.is_alphanumeric() || c == '_')
        && !new_name.starts_with('_');
    if !valid {
        return Err(format!(
            "'{new_name}' is not a valid olang identifier (letter first, then letters, digits, _)"
        ));
    }
    let edits: Vec<TextEdit> = occurrences(text, &word)
        .into_iter()
        .map(|range| TextEdit {
            range,
            new_text: new_name.to_string(),
        })
        .collect();
    if edits.is_empty() {
        return Err(format!("'{word}' does not occur in this file"));
    }
    Ok(edits)
}

// ── signature help ─────────────────────────────────────────────────────

/// The callee and active-parameter index for the innermost unclosed call
/// at the cursor, then its signature from a local declaration or the
/// registry.
fn signature_help(text: &str, pos: Position) -> Option<lsp_types::SignatureHelp> {
    let line = text.lines().nth(pos.line as usize)?;
    let chars: Vec<char> = line.chars().collect();
    let cursor = utf16_to_char_col(line, pos.character).min(chars.len());

    // Walk back to the innermost unmatched '(' on this line, counting
    // top-level commas for the active parameter.
    let mut depth = 0i32;
    let mut commas = 0u32;
    let mut open = None;
    let mut in_str = false;
    for i in (0..cursor).rev() {
        let c = chars[i];
        if in_str {
            if c == '"' {
                in_str = false;
            }
            continue;
        }
        match c {
            '"' => in_str = true,
            ')' | ']' => depth += 1,
            '[' => depth -= 1,
            ',' if depth == 0 => commas += 1,
            '(' => {
                if depth == 0 {
                    open = Some(i);
                    break;
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    let open = open?;
    // The callee name (possibly module-qualified) just before the paren.
    let mut end = open;
    while end > 0 && chars[end - 1] == ' ' {
        end -= 1;
    }
    let mut start = end;
    while start > 0
        && (chars[start - 1].is_alphanumeric()
            || chars[start - 1] == '_'
            || chars[start - 1] == '.')
    {
        start -= 1;
    }
    if start == end {
        return None;
    }
    let callee: String = chars[start..end].iter().collect();

    let (label, doc) = if let Some(d) = help().get_function(&callee) {
        (
            format!("{} -> {}", d.syntax, d.return_type),
            Some(d.description.clone()),
        )
    } else {
        let bare = callee.rsplit('.').next().unwrap_or(&callee);
        let (_, detail, _) = declarations(text).into_iter().find(|(n, _, _)| n == bare)?;
        (detail, None)
    };

    // Parameters from the label's parenthesized list.
    let params: Vec<lsp_types::ParameterInformation> = label
        .split_once('(')
        .and_then(|(_, rest)| rest.rsplit_once(')'))
        .map(|(inner, _)| {
            inner
                .split(',')
                .map(|p| p.trim())
                .filter(|p| !p.is_empty())
                .map(|p| lsp_types::ParameterInformation {
                    label: lsp_types::ParameterLabel::Simple(p.to_string()),
                    documentation: None,
                })
                .collect()
        })
        .unwrap_or_default();

    Some(lsp_types::SignatureHelp {
        signatures: vec![lsp_types::SignatureInformation {
            label,
            documentation: doc.map(|d| {
                lsp_types::Documentation::MarkupContent(lsp_types::MarkupContent {
                    kind: lsp_types::MarkupKind::Markdown,
                    value: d,
                })
            }),
            parameters: Some(params),
            active_parameter: Some(commas),
        }],
        active_signature: Some(0),
        active_parameter: Some(commas),
    })
}

// ── formatting ─────────────────────────────────────────────────────────

fn format_edits(text: &str) -> Vec<TextEdit> {
    let formatted = crate::tools::fmt::format_source(text);
    if formatted == text {
        return Vec::new();
    }
    // Replace the whole document: end position is the true end of text,
    // not one line past it.
    let line_count = text.split('\n').count() as u32;
    let last = text.split('\n').next_back().unwrap_or("");
    vec![TextEdit {
        range: Range::new(
            Position::new(0, 0),
            Position::new(
                line_count.saturating_sub(1),
                char_to_utf16_col(last, last.chars().count()),
            ),
        ),
        new_text: formatted,
    }]
}
