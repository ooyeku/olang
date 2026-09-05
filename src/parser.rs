use crate::ast::{
    Argument, BinaryOp, BitwiseOp, EnumVariant, ErrorTypeDecl, Expr, FieldValue, FunctionDecl,
    LetDecl, MapEntry, MatchArm, Parameter, Pattern, Program, ShareDecl, Statement, StructField,
    StructLiteral, TemplatePart, TestDecl, TypeAnnotation, TypeDecl, TypeDefinition, UnaryOp,
    UseDecl,
};
use pest::{Parser as PestParser, iterators::Pair, iterators::Pairs};
use pest_derive::Parser;
use std::sync::Arc as Rc;
use std::sync::Arc;
use thiserror::Error;

#[derive(Parser)]
#[grammar = "grammar.pest"]
pub struct OlangParser;

#[derive(Debug, Clone)]
pub struct PositionInfo {
    pub line: usize,
    pub column: usize,
    pub offset: usize,
    pub input_snippet: String,
}

impl PositionInfo {
    /// Create PositionInfo from a pest Pair. Uses the original input via pest::Position to build a snippet.
    /// Note: pest's line/column are 1-based; we keep this convention for display and caret alignment.
    pub fn from_pair(pair: &Pair<Rule>) -> Self {
        let pos = pair.as_span().start_pos();
        let (line, column) = pos.line_col();
        let offset = pos.pos();

        // Build a sanitized, single-line snippet with a caret under the column
        let error_line = sanitize_snippet(pos.line_of());
        let mut snippet = String::new();
        snippet.push_str(&format!("{:4} | {}\n", line, error_line));
        snippet.push_str(&format!(
            "{:4} | {}^",
            "",
            " ".repeat(column.saturating_sub(1))
        ));

        Self {
            line,
            column,
            offset,
            input_snippet: snippet,
        }
    }

    /// Create PositionInfo directly from a pest::Position
    pub fn from_position(pos: pest::Position) -> Self {
        let (line, column) = pos.line_col();
        let offset = pos.pos();
        let error_line = sanitize_snippet(pos.line_of());
        let mut snippet = String::new();
        snippet.push_str(&format!("{:4} | {}\n", line, error_line));
        snippet.push_str(&format!(
            "{:4} | {}^",
            "",
            " ".repeat(column.saturating_sub(1))
        ));

        Self {
            line,
            column,
            offset,
            input_snippet: snippet,
        }
    }
}

/// Very basic snippet sanitizer to avoid leaking sensitive content in logs.
/// - Truncates long lines
/// - Replaces control characters with spaces
fn sanitize_snippet(s: &str) -> String {
    let max_len = 200usize;
    let mut line = s
        .chars()
        .map(|c| {
            if c.is_control() && c != '\n' && c != '\t' {
                ' '
            } else {
                c
            }
        })
        .take(max_len)
        .collect::<String>();
    if s.chars().count() > max_len {
        line.push_str(" …");
    }
    line
}

#[derive(Error, Debug)]
pub enum ParseError {
    #[error("Pest parsing error: {0}")]
    Pest(#[from] pest::error::Error<Rule>),
    #[error("Invalid syntax at line {line}, column {column}: {message}\n{snippet}")]
    InvalidSyntaxWithPosition {
        message: String,
        line: usize,
        column: usize,
        snippet: Arc<str>,
    },
    #[error("Unexpected token at line {line}, column {column}: {token}\n{snippet}")]
    UnexpectedTokenWithPosition {
        token: String,
        line: usize,
        column: usize,
        snippet: Arc<str>,
    },
    #[error("Invalid syntax: {message}")]
    InvalidSyntax { message: String },
    #[error("Unexpected token: {token}")]
    UnexpectedToken { token: String },
}

/// Map a grammar rule to the phrase a person would use for it. The pest
/// error's "expected" list is written in grammar-internal vocabulary
/// (`mul_op`, `base_pattern`, `EOI`) that means nothing to someone whose
/// program just failed to parse; every rule a user can plausibly hit is
/// translated here, and the fallback prettifies rather than leaks.
fn humanize_rule(rule: Rule) -> &'static str {
    match rule {
        Rule::EOI => "the end of the file",
        Rule::statement => "a statement",
        Rule::expr | Rule::primary => "an expression",
        Rule::identifier => "a name",
        Rule::add_op
        | Rule::mul_op
        | Rule::bitwise_op
        | Rule::or_op
        | Rule::and_op
        | Rule::comp_op
        | Rule::range_op
        | Rule::pipe_op
        | Rule::postfix_op => "an operator",
        Rule::function_call => "a function call",
        Rule::block => "a block",
        Rule::string => "a string",
        Rule::number => "a number",
        _ => "",
    }
}

/// Turn a raw pest error into the same rich, located ParseError the
/// hand-written sub-parsers produce: plain-English message, line/column,
/// and a caret snippet — instead of leaking the parser crate's name and
/// grammar-rule identifiers to the user.
fn humanize_pest_error(e: pest::error::Error<Rule>, input: &str) -> ParseError {
    use pest::error::{ErrorVariant, LineColLocation};

    let (line, column) = match e.line_col {
        LineColLocation::Pos((l, c)) => (l, c),
        LineColLocation::Span((l, c), _) => (l, c),
    };

    let error_line = sanitize_snippet(input.lines().nth(line.saturating_sub(1)).unwrap_or(""));
    let mut snippet = String::new();
    snippet.push_str(&format!("{:4} | {}\n", line, error_line));
    snippet.push_str(&format!(
        "{:4} | {}^",
        "",
        " ".repeat(column.saturating_sub(1))
    ));

    let message = match &e.variant {
        ErrorVariant::ParsingError { positives, .. } => {
            // Collapse the rule list into deduplicated human phrases,
            // preserving first-seen order; unknown rules are prettified
            // (underscores to spaces) rather than shown raw.
            let mut phrases: Vec<String> = Vec::new();
            let mut prettied: Vec<String> = Vec::new();
            for r in positives {
                let h = humanize_rule(*r);
                if h.is_empty() {
                    let p = format!("{:?}", r).replace('_', " ");
                    if !prettied.contains(&p) {
                        prettied.push(p);
                    }
                } else if !phrases.iter().any(|x| x == h) {
                    phrases.push(h.to_string());
                }
            }
            phrases.extend(prettied);
            phrases.truncate(4);
            match phrases.len() {
                0 => "unexpected input here".to_string(),
                1 => format!("expected {}", phrases[0]),
                _ => {
                    let last = phrases.pop().unwrap();
                    format!("expected {} or {}", phrases.join(", "), last)
                }
            }
        }
        ErrorVariant::CustomError { message } => message.clone(),
    };

    // The nested-template mistake — a backtick inside `${...}` — ends the
    // template at the inner backtick, so the raw error lands "at an
    // unexpected position" with no mention of the cause. When the error
    // line up to the caret closes a template whose `${` never closed,
    // name the real problem.
    let raw_line = input.lines().nth(line.saturating_sub(1)).unwrap_or("");
    let message = if template_nesting_suspected(raw_line, column) {
        format!(
            "{} — a backtick inside `${{...}}` ends the template (templates \
             do not nest); bind the inner template to a name first",
            message
        )
    } else {
        message
    };

    ParseError::InvalidSyntaxWithPosition {
        message,
        line,
        column,
        snippet: snippet.into(),
    }
}

/// Does `line` up to `column` (1-based) contain a completed backtick
/// template whose `${` interpolation never closed? That shape means a
/// backtick inside the interpolation terminated the template early — the
/// classic nesting mistake. Double-quoted strings, char literals, and
/// `//` comments on the line are skipped so their backticks don't
/// confuse the pairing; the scan is line-local, so a template spanning
/// lines simply produces no hint.
fn template_nesting_suspected(line: &str, column: usize) -> bool {
    let prefix: String = line.chars().take(column.saturating_sub(1)).collect();
    let mut chars = prefix.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '"' => {
                let mut esc = false;
                for c in chars.by_ref() {
                    if esc {
                        esc = false;
                    } else if c == '\\' {
                        esc = true;
                    } else if c == '"' {
                        break;
                    }
                }
            }
            '\'' => {
                for c in chars.by_ref() {
                    if c == '\'' {
                        break;
                    }
                }
            }
            '/' if chars.peek() == Some(&'/') => return false,
            '`' => {
                // Collect this template's raw content up to its closing
                // backtick (backslash escapes the next character, as in
                // the grammar). Unterminated on this line → no verdict.
                let mut content = String::new();
                let mut closed = false;
                while let Some(c) = chars.next() {
                    if c == '\\' {
                        content.push(c);
                        if let Some(n) = chars.next() {
                            content.push(n);
                        }
                    } else if c == '`' {
                        closed = true;
                        break;
                    } else {
                        content.push(c);
                    }
                }
                if closed && template_has_unclosed_interpolation(&content) {
                    return true;
                }
                if !closed {
                    return false;
                }
            }
            _ => {}
        }
    }
    false
}

/// Does template raw content contain a `${` whose brace never closes
/// before the content ends? Mirrors `parse_template_content`'s brace and
/// string tracking.
fn template_has_unclosed_interpolation(content: &str) -> bool {
    let mut chars = content.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            chars.next();
            continue;
        }
        if ch == '$' && chars.peek() == Some(&'{') {
            chars.next();
            let mut depth = 1;
            let mut in_string = false;
            let mut esc = false;
            for c in chars.by_ref() {
                if in_string {
                    if esc {
                        esc = false;
                    } else if c == '\\' {
                        esc = true;
                    } else if c == '"' {
                        in_string = false;
                    }
                    continue;
                }
                match c {
                    '"' => in_string = true,
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
            }
            if depth > 0 {
                return true;
            }
        }
    }
    false
}

impl ParseError {
    pub fn invalid_syntax_at(message: String, position: PositionInfo) -> Self {
        Self::InvalidSyntaxWithPosition {
            message,
            line: position.line,
            column: position.column,
            snippet: position.input_snippet.into(),
        }
    }

    pub fn unexpected_token_at(token: String, position: PositionInfo) -> Self {
        Self::UnexpectedTokenWithPosition {
            token,
            line: position.line,
            column: position.column,
            snippet: position.input_snippet.into(),
        }
    }
}

pub struct Parser {
    suggestion_engine: ErrorSuggestionEngine,
    /// Which macro constructs the last `parse_raw` built. Recorded as the
    /// tree is built so `parse` can decide about expansion in O(1) —
    /// the AST-wide search it replaces serialized the whole program to
    /// JSON to look for three keys, a real cost in the browser on a
    /// large bundle whose text merely contains an `@` in a string.
    saw_macro_call: std::cell::Cell<bool>,
    saw_meta_fn: std::cell::Cell<bool>,
    saw_decorated: std::cell::Cell<bool>,
    /// How many `meta fn` declarations the tree holds, at any depth — the
    /// declarations-only shortcut applies only when all of them are top
    /// level (a nested one is a placement error the expander reports).
    meta_fn_count: std::cell::Cell<usize>,
}

impl Default for Parser {
    fn default() -> Self {
        Self::new()
    }
}

impl Parser {
    pub fn new() -> Self {
        Self {
            suggestion_engine: ErrorSuggestionEngine::new(),
            saw_macro_call: std::cell::Cell::new(false),
            saw_meta_fn: std::cell::Cell::new(false),
            saw_decorated: std::cell::Cell::new(false),
            meta_fn_count: std::cell::Cell::new(0),
        }
    }

    /// Parse with enhanced error reporting
    pub fn parse_with_suggestions(
        &self,
        input: &str,
    ) -> Result<Program, (ParseError, Vec<ErrorSuggestion>)> {
        match self.parse(input) {
            Ok(program) => Ok(program),
            Err(error) => {
                let suggestions = self
                    .suggestion_engine
                    .suggest_for_parse_error(&error, input);
                Err((error, suggestions))
            }
        }
    }

    /// Get suggestions for a parse error
    pub fn get_suggestions(&self, error: &ParseError, input: &str) -> Vec<ErrorSuggestion> {
        self.suggestion_engine.suggest_for_parse_error(error, input)
    }

    /// Convenience: returns suggestions for the given input without altering the error API.
    /// If parsing succeeds, returns an empty Vec.
    pub fn suggestions_for(&self, input: &str) -> Vec<ErrorSuggestion> {
        match self.parse(input) {
            Ok(_) => Vec::new(),
            Err(err) => self.suggestion_engine.suggest_for_parse_error(&err, input),
        }
    }

    /// Parse a program, expanding macros (docs/macros.md). This is the
    /// entry every execution path uses — files, the REPL, doc tests,
    /// `olang build` — so a program with `meta fn`/`@` in it behaves the
    /// same everywhere. Macro-free source (the overwhelmingly common
    /// case) pays a substring scan and nothing else.
    pub fn parse(&self, input: &str) -> Result<Program, ParseError> {
        let program = self.parse_raw(input)?;
        // The tree-build recorded which macro constructs exist; no `@`
        // site, no decorator, no `meta fn` means nothing to expand.
        let (calls, metas, decorated) = (
            self.saw_macro_call.get(),
            self.saw_meta_fn.get(),
            self.saw_decorated.get(),
        );
        if !(calls || metas || decorated) {
            return Ok(program);
        }
        // Declarations only — a library that ships macros, parsed as a
        // program that never invokes them (the bundled SDK modules in a
        // browser client): drop the declarations and skip the text
        // round trip, which would re-parse the whole program.
        if metas && !calls && !decorated {
            let top_level = program
                .statements
                .iter()
                .filter(|st| matches!(st.unwrapped(), Statement::MetaFnDecl { .. }))
                .count();
            if top_level == self.meta_fn_count.get() {
                let statements = program
                    .statements
                    .into_iter()
                    .filter(|st| !matches!(st.unwrapped(), Statement::MetaFnDecl { .. }))
                    .collect();
                return Ok(Program { statements });
            }
        }
        let expanded = crate::expand::expand_source(input)
            .map_err(|message| ParseError::InvalidSyntax { message })?;
        let program = self.parse_raw(&expanded)?;
        if self.saw_macro_call.get() || self.saw_meta_fn.get() || self.saw_decorated.get() {
            return Err(ParseError::InvalidSyntax {
                message: "macro expansion left unexpanded macro constructs (internal error)"
                    .to_string(),
            });
        }
        Ok(program)
    }

    /// Parse without macro expansion — the raw program as written. This
    /// is what `meta.parse` exposes (the Open AST shows source as the
    /// author wrote it, `@` sites and all) and what the expander itself
    /// uses between rounds.
    pub fn parse_raw(&self, input: &str) -> Result<Program, ParseError> {
        self.saw_macro_call.set(false);
        self.saw_meta_fn.set(false);
        self.saw_decorated.set(false);
        self.meta_fn_count.set(0);
        // A UTF-8 BOM (files from Windows editors) is invisible in every
        // editor but fails the grammar at 1:1 with a baffling caret at
        // nothing. Strip it before parsing.
        let input = input.strip_prefix('\u{feff}').unwrap_or(input);
        // A shebang line (#!/usr/bin/env olang) makes a script directly
        // executable; it is host metadata, not syntax. Mask it with the
        // same number of space BYTES rather than removing it, so every
        // span, line number, and byte offset downstream still matches
        // the file on disk exactly.
        let masked;
        let input = if input.starts_with("#!") {
            let line_end = input.find('\n').unwrap_or(input.len());
            masked = format!("{}{}", " ".repeat(line_end), &input[line_end..]);
            masked.as_str()
        } else {
            input
        };
        let parsed = <OlangParser as PestParser<Rule>>::parse(Rule::program, input)
            .map_err(|e| humanize_pest_error(e, input))?;

        let mut statements = Vec::new();
        for pair in parsed {
            if pair.as_rule() == Rule::program {
                for inner_pair in pair.into_inner() {
                    if inner_pair.as_rule() == Rule::statement {
                        let position_info = PositionInfo::from_pair(&inner_pair);
                        let stmt_inner = inner_pair.into_inner().next().ok_or_else(|| {
                            ParseError::invalid_syntax_at(
                                "Empty statement".to_string(),
                                position_info.clone(),
                            )
                        })?;
                        statements.push(Self::locate(
                            self.build_statement(stmt_inner)?,
                            position_info.line,
                            position_info.column,
                        ));
                    }
                }
            }
        }

        // A bare `share` is never a statement: the grammar accepts only
        // `share` followed by a declaration, so a lone `share` here means
        // the thing after it was not one — `share meta fn` is the case
        // that reaches users, since a meta fn is exported by being
        // declared in the module and travels with `use`. Without this
        // the identifier `share` survived to run time and failed there
        // as "Undefined variable: share", attributed to the importer's
        // `use` line.
        for statement in &statements {
            if let Statement::Located { stmt, line, column } = statement
                && let Statement::Expression(Expr::Identifier(name)) = stmt.as_ref()
                && name == "share"
            {
                let source_line = input
                    .lines()
                    .nth((*line as usize).saturating_sub(1))
                    .unwrap_or("");
                let hint = if source_line.contains("meta") {
                    " A meta fn cannot be shared: declare it in the module without `share`; \
                     it is exported to every `use` of the module by being declared there."
                } else {
                    ""
                };
                return Err(ParseError::InvalidSyntaxWithPosition {
                    message: format!(
                        "`share` must be followed by a declaration: `share fn`, `share let`, \
                         `share type`, `share trait`, `share impl`, or `share use`.{hint}"
                    ),
                    line: *line as usize,
                    column: *column as usize,
                    snippet: format!(
                        "{:4} | {}\n{:4} | {}^",
                        line,
                        sanitize_snippet(source_line),
                        "",
                        " ".repeat((*column as usize).saturating_sub(1))
                    )
                    .into(),
                });
            }
        }

        Ok(Program { statements })
    }

    /// Wrap a statement with the source position of the pair it was built
    /// from, so runtime errors can be attributed to lines. Every statement
    /// the parser produces goes through this.
    fn locate(stmt: Statement, line: usize, column: usize) -> Statement {
        Statement::Located {
            line: line as u32,
            column: column as u32,
            stmt: Box::new(stmt),
        }
    }

    fn build_statement(&self, pair: Pair<Rule>) -> Result<Statement, ParseError> {
        match pair.as_rule() {
            Rule::let_decl => Ok(Statement::LetDecl(self.build_let_decl(pair.into_inner())?)),
            // Macros: both carry byte spans so the expander
            // can splice over exactly the text this parse saw.
            Rule::meta_fn_decl => {
                self.saw_meta_fn.set(true);
                self.meta_fn_count.set(self.meta_fn_count.get() + 1);
                let span = (pair.as_span().start(), pair.as_span().end());
                let inner = pair
                    .into_inner()
                    .find(|p| p.as_rule() == Rule::function_decl)
                    .ok_or_else(|| ParseError::InvalidSyntax {
                        message: "meta fn: missing function declaration".to_string(),
                    })?;
                Ok(Statement::MetaFnDecl {
                    decl: self.build_function_decl(inner.into_inner())?,
                    span,
                })
            }
            Rule::decorated_decl => {
                self.saw_decorated.set(true);
                let span = (pair.as_span().start(), pair.as_span().end());
                let line = pair.as_span().start_pos().line_col().0 as u32;
                let mut decorators = Vec::new();
                let mut decl_src = String::new();
                for part in pair.into_inner() {
                    match part.as_rule() {
                        Rule::decorator => {
                            let dline = part.as_span().start_pos().line_col().0 as u32;
                            let name = part
                                .into_inner()
                                .next()
                                .map(|p| p.as_str().to_string())
                                .unwrap_or_default();
                            // Decorators are bare names (see the grammar);
                            // args_src stays for a future arguments design.
                            decorators.push(crate::ast::Decorator {
                                name,
                                args_src: Vec::new(),
                                line: dline,
                            });
                        }
                        Rule::type_decl | Rule::function_decl | Rule::let_decl => {
                            decl_src = part.as_str().to_string()
                        }
                        _ => {}
                    }
                }
                Ok(Statement::DecoratedDecl {
                    decorators,
                    decl_src,
                    span,
                    line,
                })
            }
            Rule::function_decl => Ok(Statement::FunctionDecl(
                self.build_function_decl(pair.into_inner())?,
            )),
            Rule::type_decl => Ok(Statement::TypeDecl(
                self.build_type_decl(pair.into_inner())?,
            )),
            Rule::error_type_decl => Ok(Statement::ErrorTypeDecl(
                self.build_error_type_decl(pair.into_inner())?,
            )),
            Rule::share_decl => Ok(Statement::ShareDecl(
                self.build_share_decl(pair.into_inner())?,
            )),
            Rule::use_decl => Ok(Statement::UseDecl(self.build_use_decl(pair.into_inner())?)),
            Rule::test_decl => Ok(Statement::TestDecl(
                self.build_test_decl(pair.into_inner())?,
            )),
            Rule::trait_decl => Ok(Statement::TraitDecl(
                self.build_trait_decl(pair.into_inner())?,
            )),
            Rule::impl_decl => Ok(Statement::ImplDecl(
                self.build_impl_decl(pair.into_inner())?,
            )),
            // An assertion is a statement form, so it reaches here from any
            // block — a loop body, an `if`, a function — not only from the
            // top level of a test block.
            Rule::assertion => Ok(Statement::Expression(
                self.build_assertion(pair.into_inner())?,
            )),
            Rule::expr => Ok(Statement::Expression(self.build_expr(pair.into_inner())?)),
            // `test_statement` wraps a plain `statement`, so unwrap one level
            // rather than rejecting (this blocked `let` inside test blocks)
            Rule::statement => {
                let inner = pair
                    .into_inner()
                    .next()
                    .ok_or_else(|| ParseError::InvalidSyntax {
                        message: "Empty statement".to_string(),
                    })?;
                self.build_statement(inner)
            }
            _ => Err(ParseError::invalid_syntax_at(
                format!("Invalid statement: {:?}", pair.as_rule()),
                PositionInfo::from_pair(&pair),
            )),
        }
    }

    fn build_error_type_decl(&self, pairs: Pairs<Rule>) -> Result<ErrorTypeDecl, ParseError> {
        let mut pairs = pairs.filter(|p| !Self::is_kw_pair(p.as_rule())).peekable();

        let name = pairs
            .next()
            .ok_or_else(|| ParseError::InvalidSyntax {
                message: "Missing error type name".to_string(),
            })?
            .as_str()
            .to_string();

        let variants = if let Some(pair) = pairs.next() {
            match pair.as_rule() {
                Rule::error_variant_list => self.build_error_variant_list(pair.into_inner())?,
                _ => Vec::new(),
            }
        } else {
            Vec::new()
        };

        Ok(ErrorTypeDecl { name, variants })
    }

    fn build_error_variant_list(
        &self,
        pairs: Pairs<Rule>,
    ) -> Result<Vec<crate::ast::ErrorVariant>, ParseError> {
        let mut variants = Vec::new();
        for pair in pairs {
            if pair.as_rule() == Rule::error_variant {
                variants.push(self.build_error_variant(pair.into_inner())?);
            }
        }
        Ok(variants)
    }

    fn build_error_variant(
        &self,
        mut pairs: Pairs<Rule>,
    ) -> Result<crate::ast::ErrorVariant, ParseError> {
        let name = pairs
            .next()
            .ok_or_else(|| ParseError::InvalidSyntax {
                message: "Missing error variant name".to_string(),
            })?
            .as_str()
            .to_string();

        // An optional payload after the colon: `()` (unit) keeps the variant
        // bare; `{ field: Type, ... }` records the payload fields, which
        // become the constructor's positional parameters in order.
        let fields = match pairs.next() {
            Some(type_pair) if type_pair.as_rule() == Rule::struct_field_list => {
                self.build_struct_field_list(type_pair.into_inner())?
            }
            _ => Vec::new(),
        };

        Ok(crate::ast::ErrorVariant { name, fields })
    }

    /// The guarded keyword atomics (`break_kw`, `let_kw`, ...) exist so
    /// `breaker` and `returns` lex as identifiers, but as named rules
    /// they emit pairs the older bare literals never did. Builders for
    /// the rules that consume keywords filter them out at entry; `mut`
    /// is deliberately absent — its pair is semantic (it marks the
    /// binding assignable) and is handled explicitly.
    fn is_kw_pair(rule: Rule) -> bool {
        matches!(
            rule,
            Rule::fn_kw
                | Rule::let_kw
                | Rule::if_kw
                | Rule::else_kw
                | Rule::match_kw
                | Rule::for_kw
                | Rule::in_kw
                | Rule::while_kw
                | Rule::loop_kw
                | Rule::break_kw
                | Rule::continue_kw
                | Rule::return_kw
                | Rule::struct_kw
                | Rule::enum_kw
                | Rule::share_kw
                | Rule::use_kw
                | Rule::type_kw
                | Rule::test_kw
                | Rule::trait_kw
                | Rule::impl_kw
                | Rule::error_kw
                | Rule::par_kw
        )
    }

    fn build_let_decl(&self, pairs: Pairs<Rule>) -> Result<LetDecl, ParseError> {
        let mut pairs = pairs.filter(|p| !Self::is_kw_pair(p.as_rule())).peekable();

        let mut pattern_pair = pairs.next().ok_or_else(|| ParseError::InvalidSyntax {
            message: "Missing pattern in let declaration".to_string(),
        })?;

        // `let mut x = ...`: `mut` marks the binding assignable. Without it
        // the binding is immutable and assigning to it is an error (a fresh
        // `let` still shadows).
        let mut mutable = false;
        if pattern_pair.as_rule() == Rule::mut_kw {
            mutable = true;
            pattern_pair = pairs.next().ok_or_else(|| ParseError::InvalidSyntax {
                message: "Missing pattern after `mut` in let declaration".to_string(),
            })?;
        }

        let (span_line, span_col) = pattern_pair.as_span().start_pos().line_col();
        let name_span = Some((span_line as u32, span_col as u32));
        let pattern = self.build_pattern(pattern_pair.into_inner())?;

        let mut type_annotation = None;
        let mut value = None;

        for pair in pairs {
            match pair.as_rule() {
                Rule::type_annotation => {
                    type_annotation = Some(self.build_type_annotation(pair.into_inner())?);
                }
                Rule::expr => {
                    value = Some(self.build_expr(pair.into_inner())?);
                }
                _ => {}
            }
        }

        Ok(LetDecl {
            pattern,
            type_annotation,
            value,
            name_span,
            mutable,
        })
    }

    fn build_expr(&self, mut pairs: Pairs<Rule>) -> Result<Expr, ParseError> {
        if let Some(pair) = pairs.next() {
            match pair.as_rule() {
                Rule::binary_expr => self.build_binary_expr(pair.into_inner()),
                Rule::match_expr => self.build_match_expr(pair.into_inner()),
                Rule::pipe_expr => self.build_pipe_expr(pair.into_inner()),
                Rule::call_expr => self.build_call_expr(pair.into_inner()),
                Rule::primary => self.build_primary(pair.into_inner()),
                Rule::for_loop => self.build_for_loop(pair.into_inner(), false),
                Rule::par_for_loop => self.build_for_loop(pair.into_inner(), true),
                Rule::while_loop => self.build_while_loop(pair.into_inner()),
                Rule::loop_expr => self.build_loop_expr(pair.into_inner()),
                Rule::break_expr => {
                    let value = match pair.into_inner().find(|p| !Self::is_kw_pair(p.as_rule())) {
                        Some(inner) => Some(Box::new(self.build_expr(inner.into_inner())?)),
                        None => None,
                    };
                    Ok(Expr::Break(value))
                }
                Rule::continue_expr => Ok(Expr::Continue),
                Rule::return_expr => {
                    let value = match pair.into_inner().find(|p| !Self::is_kw_pair(p.as_rule())) {
                        Some(inner) => Some(Box::new(self.build_expr(inner.into_inner())?)),
                        None => None,
                    };
                    Ok(Expr::Return(value))
                }
                Rule::assignment_expr => self.build_assignment(pair.into_inner()),
                _ => Err(ParseError::invalid_syntax_at(
                    format!("Unexpected expression rule: {:?}", pair.as_rule()),
                    PositionInfo::from_pair(&pair),
                )),
            }
        } else {
            Err(ParseError::InvalidSyntax {
                message: "Empty expression".to_string(),
            })
        }
    }

    fn build_binary_expr(&self, mut pairs: Pairs<Rule>) -> Result<Expr, ParseError> {
        let first_pair = pairs.next().ok_or_else(|| ParseError::InvalidSyntax {
            message: "Missing left operand in binary expression".to_string(),
        })?;
        let mut expr = self.build_and_expr(first_pair.into_inner())?;

        while let Some(op_pair) = pairs.next() {
            if let Some(right_pair) = pairs.next() {
                let op = match op_pair.as_str() {
                    "||" => BinaryOp::Or,
                    _ => break,
                };
                let right = self.build_and_expr(right_pair.into_inner())?;
                expr = Expr::BinaryOp {
                    left: Box::new(expr),
                    op,
                    right: Box::new(right),
                };
            } else {
                break;
            }
        }
        Ok(expr)
    }

    fn build_and_expr(&self, mut pairs: Pairs<Rule>) -> Result<Expr, ParseError> {
        let first_pair = pairs.next().ok_or_else(|| ParseError::InvalidSyntax {
            message: "Missing left operand in logical-and expression".to_string(),
        })?;
        let mut expr = self.build_comparison_expr(first_pair.into_inner())?;

        while let Some(op_pair) = pairs.next() {
            if let Some(right_pair) = pairs.next() {
                let op = match op_pair.as_str() {
                    "&&" => BinaryOp::And,
                    _ => break,
                };
                let right = self.build_comparison_expr(right_pair.into_inner())?;
                expr = Expr::BinaryOp {
                    left: Box::new(expr),
                    op,
                    right: Box::new(right),
                };
            } else {
                break;
            }
        }
        Ok(expr)
    }

    fn build_comparison_expr(&self, mut pairs: Pairs<Rule>) -> Result<Expr, ParseError> {
        let first_pair = pairs.next().ok_or_else(|| ParseError::InvalidSyntax {
            message: "Missing left operand in comparison expression".to_string(),
        })?;
        let mut expr = self.build_pipe_expr(first_pair.into_inner())?;

        while let Some(op_pair) = pairs.next() {
            if let Some(right_pair) = pairs.next() {
                let op = match op_pair.as_str() {
                    "==" => BinaryOp::Equal,
                    "!=" => BinaryOp::NotEqual,
                    "<" => BinaryOp::LessThan,
                    "<=" => BinaryOp::LessThanEqual,
                    ">" => BinaryOp::GreaterThan,
                    ">=" => BinaryOp::GreaterThanEqual,
                    _ => break,
                };
                let right = match right_pair.as_rule() {
                    Rule::cmp_rhs => {
                        let inner = right_pair.into_inner().next().ok_or_else(|| {
                            ParseError::InvalidSyntax {
                                message: "Missing right operand in comparison".to_string(),
                            }
                        })?;
                        match inner.as_rule() {
                            Rule::unit => Expr::Tuple(std::sync::Arc::new(Vec::new())),
                            _ => self.build_pipe_expr(inner.into_inner())?,
                        }
                    }
                    _ => self.build_pipe_expr(right_pair.into_inner())?,
                };
                expr = Expr::BinaryOp {
                    left: Box::new(expr),
                    op,
                    right: Box::new(right),
                };
            } else {
                break;
            }
        }
        Ok(expr)
    }

    fn build_additive_expr(&self, mut pairs: Pairs<Rule>) -> Result<Expr, ParseError> {
        let first_pair = pairs.next().ok_or_else(|| ParseError::InvalidSyntax {
            message: "Missing left operand in additive expression".to_string(),
        })?;
        let mut expr = self.build_multiplicative_expr(first_pair.into_inner())?;

        while let Some(op_pair) = pairs.next() {
            if let Some(right_pair) = pairs.next() {
                let op = match op_pair.as_str() {
                    "+" => BinaryOp::Add,
                    "-" => BinaryOp::Subtract,
                    _ => break,
                };
                let right = self.build_multiplicative_expr(right_pair.into_inner())?;
                expr = Expr::BinaryOp {
                    left: Box::new(expr),
                    op,
                    right: Box::new(right),
                };
            } else {
                break;
            }
        }
        Ok(expr)
    }

    fn build_multiplicative_expr(&self, mut pairs: Pairs<Rule>) -> Result<Expr, ParseError> {
        let first_pair = pairs.next().ok_or_else(|| ParseError::InvalidSyntax {
            message: "Missing left operand in multiplicative expression".to_string(),
        })?;
        let mut expr = self.build_bitwise_expr(first_pair.into_inner())?;

        while let Some(op_pair) = pairs.next() {
            if let Some(right_pair) = pairs.next() {
                let op = match op_pair.as_str() {
                    "*" => BinaryOp::Multiply,
                    "/" => BinaryOp::Divide,
                    "%" => BinaryOp::Modulo,
                    _ => break,
                };
                // Multiplicative operands sit directly above bitwise in the
                // ladder, so both sides build through build_bitwise_expr.
                let right = self.build_bitwise_expr(right_pair.into_inner())?;
                expr = Expr::BinaryOp {
                    left: Box::new(expr),
                    op,
                    right: Box::new(right),
                };
            } else {
                break;
            }
        }
        Ok(expr)
    }

    fn build_bitwise_expr(&self, mut pairs: Pairs<Rule>) -> Result<Expr, ParseError> {
        let first_pair = pairs.next().ok_or_else(|| ParseError::InvalidSyntax {
            message: "Missing left operand in bitwise expression".to_string(),
        })?;
        let mut expr = self.build_unary_expr(first_pair.into_inner())?;

        while let Some(op_pair) = pairs.next() {
            if let Some(right_pair) = pairs.next() {
                let op = match op_pair.as_str() {
                    "&" => BitwiseOp::And,
                    "|" => BitwiseOp::Or,
                    "^" => BitwiseOp::Xor,
                    "<<" => BitwiseOp::Shl,
                    ">>" => BitwiseOp::Shr,
                    _ => break,
                };
                let right = self.build_unary_expr(right_pair.into_inner())?;
                expr = Expr::BitwiseOp {
                    left: Box::new(expr),
                    op,
                    right: Box::new(right),
                };
            } else {
                break;
            }
        }
        Ok(expr)
    }

    fn build_pipe_expr(&self, mut pairs: Pairs<Rule>) -> Result<Expr, ParseError> {
        let first_pair = pairs.next().ok_or_else(|| ParseError::InvalidSyntax {
            message: "Missing left operand in pipeline expression".to_string(),
        })?;
        let mut left = self.build_range_expr(first_pair.into_inner())?;

        while let Some(op_pair) = pairs.next() {
            if op_pair.as_rule() == Rule::pipe_op {
                if let Some(right_pair) = pairs.next() {
                    let right = self.build_range_expr(right_pair.into_inner())?;
                    left = Expr::Pipeline {
                        left: Box::new(left),
                        right: Box::new(right),
                    };
                } else {
                    return Err(ParseError::InvalidSyntax {
                        message: "Missing right-hand side of pipeline expression".to_string(),
                    });
                }
            }
        }

        Ok(left)
    }

    fn build_range_expr(&self, mut pairs: Pairs<Rule>) -> Result<Expr, ParseError> {
        let first_pair = pairs.next().ok_or_else(|| ParseError::InvalidSyntax {
            message: "Missing left operand in range expression".to_string(),
        })?;
        // Range bounds are additive expressions: `0..n-1` is `0..(n-1)`.
        let mut left = self.build_additive_expr(first_pair.into_inner())?;

        if let Some(pair) = pairs.next() {
            let op_str = pair.as_str();
            if op_str == ".." || op_str == "..=" {
                let inclusive = op_str == "..=";
                let right_pair = pairs.next().ok_or_else(|| ParseError::InvalidSyntax {
                    message: "Missing right operand in range expression".to_string(),
                })?;
                let right = self.build_additive_expr(right_pair.into_inner())?;
                left = Expr::Range {
                    start: Box::new(left),
                    end: Box::new(right),
                    inclusive,
                };
            }
        }

        Ok(left)
    }

    fn build_call_expr(&self, mut pairs: Pairs<Rule>) -> Result<Expr, ParseError> {
        let first_pair = pairs.next().ok_or_else(|| ParseError::InvalidSyntax {
            message: "Missing primary expression in call expression".to_string(),
        })?;
        let mut expr = self.build_primary(first_pair.into_inner())?;

        // Process all operations in sequence
        let mut pending_args = Vec::new();

        for pair in pairs {
            match pair.as_rule() {
                Rule::primary => {
                    // Space-separated argument - collect for function call
                    let arg = self.build_primary(pair.into_inner())?;
                    pending_args.push(Argument::Positional(arg));
                }
                Rule::function_call => {
                    // Special case: Check if this is Ok(...) or Err(...) pattern
                    if let Expr::Identifier(ref name) = expr
                        && (name == "Ok" || name == "Err")
                        && pending_args.is_empty()
                    {
                        // This is a result expression, not a function call
                        let args = if let Some(arg_list_pair) = pair.into_inner().next() {
                            self.build_arg_list(arg_list_pair.into_inner())?
                        } else {
                            Vec::new()
                        };
                        if args.len() == 1 {
                            let arg = args.into_iter().next().ok_or_else(|| {
                                ParseError::InvalidSyntax {
                                    message: format!("{} expression missing argument", name),
                                }
                            })?;
                            let inner = match arg {
                                Argument::Positional(e) => e,
                                Argument::Named { .. } => {
                                    return Err(ParseError::InvalidSyntax {
                                        message: format!(
                                            "{} expressions cannot use named arguments",
                                            name
                                        ),
                                    });
                                }
                            };
                            // Keep looping so trailing postfix operations
                            // (`?`, `.field`, indexing) still apply —
                            // returning here silently dropped them
                            expr = match name.as_str() {
                                "Ok" => Expr::ResultOk(Box::new(inner)),
                                "Err" => Expr::ResultErr(Box::new(inner)),
                                _ => unreachable!(),
                            };
                            continue;
                        } else {
                            return Err(ParseError::InvalidSyntax {
                                message: format!(
                                    "{} expressions must have exactly one argument",
                                    name
                                ),
                            });
                        }
                    }

                    // Parenthesized arguments - create function call immediately
                    if !pending_args.is_empty() {
                        expr = Expr::Call {
                            callee: Box::new(expr),
                            arguments: pending_args,
                        };
                        pending_args = Vec::new();
                    }

                    // Parse arguments (if any)
                    let args = if let Some(arg_list_pair) = pair.into_inner().next() {
                        self.build_arg_list(arg_list_pair.into_inner())?
                    } else {
                        Vec::new()
                    };
                    expr = Expr::Call {
                        callee: Box::new(expr),
                        arguments: args,
                    };
                }
                Rule::arg_list => {
                    // This shouldn't happen anymore with the new grammar, but keep for compatibility
                    let args = self.build_arg_list(pair.into_inner())?;
                    expr = Expr::Call {
                        callee: Box::new(expr),
                        arguments: args,
                    };
                }
                Rule::identifier => {
                    // Field access - create field access immediately
                    if !pending_args.is_empty() {
                        expr = Expr::Call {
                            callee: Box::new(expr),
                            arguments: pending_args,
                        };
                        pending_args = Vec::new();
                    }
                    let field_name = pair.as_str().to_string();
                    expr = Expr::FieldAccess {
                        object: Box::new(expr),
                        field: field_name,
                    };
                }
                Rule::expr => {
                    // Bracket indexing - create index expression immediately
                    if !pending_args.is_empty() {
                        expr = Expr::Call {
                            callee: Box::new(expr),
                            arguments: pending_args,
                        };
                        pending_args = Vec::new();
                    }
                    let index_expr = self.build_expr(pair.into_inner())?;
                    expr = Expr::Index {
                        object: Box::new(expr),
                        index: Box::new(index_expr),
                    };
                }
                Rule::postfix_op => {
                    // Postfix operator - create Try expression
                    if !pending_args.is_empty() {
                        expr = Expr::Call {
                            callee: Box::new(expr),
                            arguments: pending_args,
                        };
                        pending_args = Vec::new();
                    }

                    // Currently only ? is supported
                    if pair.as_str() == "?" {
                        expr = Expr::Try(Box::new(expr));
                    } else {
                        return Err(ParseError::InvalidSyntax {
                            message: format!("Unknown postfix operator: {}", pair.as_str()),
                        });
                    }
                }
                _ => {}
            }
        }

        // Handle any remaining space-separated arguments
        if !pending_args.is_empty() {
            expr = Expr::Call {
                callee: Box::new(expr),
                arguments: pending_args,
            };
        }

        Ok(expr)
    }

    fn build_arg_list(&self, pairs: Pairs<Rule>) -> Result<Vec<Argument>, ParseError> {
        let mut args = Vec::new();
        for pair in pairs {
            match pair.as_rule() {
                Rule::argument => {
                    args.push(self.build_argument(pair.into_inner())?);
                }
                Rule::expr => {
                    // Legacy support for direct expressions
                    args.push(Argument::Positional(self.build_expr(pair.into_inner())?));
                }
                _ => {}
            }
        }
        Ok(args)
    }

    fn build_argument(&self, mut pairs: Pairs<Rule>) -> Result<Argument, ParseError> {
        let first_pair = pairs.next().ok_or_else(|| ParseError::InvalidSyntax {
            message: "Empty argument".to_string(),
        })?;

        match first_pair.as_rule() {
            Rule::named_arg => {
                let mut inner_pairs = first_pair.into_inner();
                let name = inner_pairs
                    .next()
                    .ok_or_else(|| ParseError::InvalidSyntax {
                        message: "Missing argument name".to_string(),
                    })?
                    .as_str()
                    .to_string();
                let value = inner_pairs
                    .next()
                    .ok_or_else(|| ParseError::InvalidSyntax {
                        message: "Missing argument value".to_string(),
                    })?;
                Ok(Argument::Named {
                    name,
                    value: self.build_expr(value.into_inner())?,
                })
            }
            Rule::positional_arg => {
                let expr_pair =
                    first_pair
                        .into_inner()
                        .next()
                        .ok_or_else(|| ParseError::InvalidSyntax {
                            message: "Missing positional argument expression".to_string(),
                        })?;
                Ok(Argument::Positional(
                    self.build_expr(expr_pair.into_inner())?,
                ))
            }
            _ => Err(ParseError::InvalidSyntax {
                message: format!("Invalid argument rule: {:?}", first_pair.as_rule()),
            }),
        }
    }

    fn build_primary(&self, mut pairs: Pairs<Rule>) -> Result<Expr, ParseError> {
        // Check for unary expressions
        if let Some(first) = pairs.peek()
            && first.as_rule() == Rule::unary_op
        {
            return self.build_unary_expr(pairs);
        }
        // Fallback to existing logic
        let pair = pairs.next().ok_or_else(|| ParseError::InvalidSyntax {
            message: "Empty primary expression".to_string(),
        })?;
        match pair.as_rule() {
            Rule::lambda => self.build_lambda(pair.into_inner()),
            Rule::spawn_expr => self.build_spawn_expr(pair.into_inner()),
            Rule::match_expr => self.build_match_expr(pair.into_inner()),
            Rule::if_expr => self.build_if_expr(pair.into_inner()),
            Rule::struct_literal => self.build_struct_literal(pair.into_inner()),
            Rule::anonymous_object => self.build_anonymous_object(pair.into_inner()),
            Rule::map_literal => self.build_map_literal(pair.into_inner()),
            Rule::literal => self.build_literal(pair.into_inner()),
            Rule::identifier => Ok(Expr::Identifier(pair.as_str().to_string())),
            Rule::macro_call => {
                self.saw_macro_call.set(true);
                let span = (pair.as_span().start(), pair.as_span().end());
                let line = pair.as_span().start_pos().line_col().0 as u32;
                let mut inner = pair.into_inner();
                let name = inner
                    .next()
                    .map(|p| p.as_str().to_string())
                    .unwrap_or_default();
                // The single arg_list child, if present; each argument is
                // carried as its source text (already validated by this
                // parse — the total-parse law).
                let args_src = inner
                    .flat_map(|p| p.into_inner())
                    .map(|a| a.as_str().trim().to_string())
                    .collect();
                Ok(Expr::MacroCall {
                    name,
                    args_src,
                    span,
                    line,
                })
            }
            Rule::block => self.build_block(pair.into_inner()),
            Rule::paren_expr => self.build_paren_expr(pair.into_inner()),
            Rule::expr => self.build_expr(pair.into_inner()),
            _ => Err(ParseError::InvalidSyntax {
                message: format!("Invalid primary rule: {:?}", pair.as_rule()),
            }),
        }
    }

    /// Build a parenthesized expression: a single inner expression is a
    /// grouping (unwrapped to that expression), two or more comma-separated
    /// expressions form a tuple. Grouping and tuples share one grammar rule so
    /// the inner expression list is parsed exactly once — see `paren_expr` in
    /// grammar.pest for why two separate `(`-prefixed alternatives caused
    /// exponential backtracking on nested parentheses.
    fn build_paren_expr(&self, pairs: Pairs<Rule>) -> Result<Expr, ParseError> {
        let mut items = Vec::new();
        for p in pairs {
            if p.as_rule() == Rule::expr {
                items.push(self.build_expr(p.into_inner())?);
            }
        }
        if items.len() == 1 {
            Ok(items.into_iter().next().unwrap())
        } else {
            Ok(Expr::Tuple(items.into()))
        }
    }

    fn build_unary_expr(&self, pairs: Pairs<Rule>) -> Result<Expr, ParseError> {
        let mut operators = Vec::new();
        let mut call_expr_pair = None;

        // Collect all unary operators first
        for pair in pairs {
            match pair.as_rule() {
                Rule::unary_op => {
                    let op = match pair.as_str() {
                        "-" => UnaryOp::Negate,
                        "!" => UnaryOp::Not,
                        _ => {
                            return Err(ParseError::InvalidSyntax {
                                message: format!("Unknown unary operator: {}", pair.as_str()),
                            });
                        }
                    };
                    operators.push(op);
                }
                Rule::call_expr => {
                    call_expr_pair = Some(pair);
                    break;
                }
                _ => {}
            }
        }

        // Parse the base expression
        let base_expr = if let Some(pair) = call_expr_pair {
            self.build_call_expr(pair.into_inner())?
        } else {
            return Err(ParseError::InvalidSyntax {
                message: "Missing call expression in unary expression".to_string(),
            });
        };

        // Apply unary operators from right to left
        let mut result = base_expr;
        for op in operators.into_iter().rev() {
            result = Expr::UnaryOp {
                op,
                operand: Box::new(result),
            };
        }

        Ok(result)
    }

    fn build_lambda(&self, mut pairs: Pairs<Rule>) -> Result<Expr, ParseError> {
        let mut parameters = Vec::new();
        let mut return_type = None;
        let mut body = None;

        // The param list is optional (`() => 3` has none), so the first pair
        // may already be the body — treat every pair uniformly rather than
        // special-casing the first and silently discarding it
        for pair in pairs.by_ref() {
            match pair.as_rule() {
                Rule::param_list => {
                    parameters = self.build_param_list(pair.into_inner())?;
                }
                Rule::type_annotation => {
                    return_type = Some(self.build_type_annotation(pair.into_inner())?);
                }
                Rule::block => {
                    body = Some(self.build_block(pair.into_inner())?);
                }
                Rule::expr => {
                    body = Some(self.build_expr(pair.into_inner())?);
                }
                _ => {}
            }
        }

        let body_expr = body.ok_or_else(|| ParseError::InvalidSyntax {
            message: "Missing lambda body".to_string(),
        })?;

        Ok(Expr::Lambda {
            parameters,
            return_type,
            body: Box::new(body_expr),
        })
    }

    fn build_spawn_expr(&self, mut pairs: Pairs<Rule>) -> Result<Expr, ParseError> {
        // The keyword is its own pair now (bounded so `spawn_cost` is a
        // name, not a spawn); the spawned call follows it.
        let expr = pairs
            .find(|p| p.as_rule() == Rule::call_expr)
            .ok_or_else(|| ParseError::InvalidSyntax {
                message: "Missing expression in spawn".to_string(),
            })?;

        let spawned_expr = self.build_call_expr(expr.into_inner())?;

        Ok(Expr::Spawn(Box::new(spawned_expr)))
    }

    fn build_param_list(&self, pairs: Pairs<Rule>) -> Result<Vec<Parameter>, ParseError> {
        let mut params = Vec::new();

        for pair in pairs {
            if pair.as_rule() == Rule::parameter {
                params.push(self.build_parameter(pair.into_inner())?);
            }
        }

        Ok(params)
    }

    fn build_parameter(&self, mut pairs: Pairs<Rule>) -> Result<Parameter, ParseError> {
        let name = pairs
            .next()
            .ok_or_else(|| ParseError::InvalidSyntax {
                message: "Missing parameter name".to_string(),
            })?
            .as_str()
            .to_string();

        let mut type_annotation = None;
        let mut default_value = None;

        // Parse optional type annotation and default value
        for pair in pairs {
            match pair.as_rule() {
                Rule::type_annotation => {
                    type_annotation = Some(self.build_type_annotation(pair.into_inner())?);
                }
                Rule::expr => {
                    default_value = Some(self.build_expr(pair.into_inner())?);
                }
                _ => {}
            }
        }

        Ok(Parameter {
            name,
            type_annotation,
            default_value,
        })
    }

    fn build_type_annotation(&self, mut pairs: Pairs<Rule>) -> Result<TypeAnnotation, ParseError> {
        let pair = pairs.next().ok_or_else(|| ParseError::InvalidSyntax {
            message: "Empty type annotation".to_string(),
        })?;

        match pair.as_rule() {
            Rule::union_type => {
                let inner = pair.into_inner();
                let mut types = Vec::new();
                for type_pair in inner {
                    types.push(self.build_type_annotation(type_pair.into_inner())?);
                }
                Ok(TypeAnnotation::Union { types })
            }
            Rule::intersection_type => {
                let inner = pair.into_inner();
                let mut types = Vec::new();
                for type_pair in inner {
                    types.push(self.build_type_annotation(type_pair.into_inner())?);
                }
                Ok(TypeAnnotation::Intersection { types })
            }
            Rule::literal_type => {
                let inner = pair
                    .into_inner()
                    .next()
                    .ok_or_else(|| ParseError::InvalidSyntax {
                        message: "Empty literal type".to_string(),
                    })?;
                let value = match inner.as_rule() {
                    Rule::string => crate::ast::Value::String(std::sync::Arc::new(
                        self.unquote_string(inner.as_str())?,
                    )),
                    Rule::integer => {
                        let v = inner.as_str().parse::<i64>().map_err(|_| {
                            ParseError::InvalidSyntax {
                                message: "Invalid integer in literal type".to_string(),
                            }
                        })?;
                        crate::ast::Value::Integer(v)
                    }
                    Rule::boolean => {
                        let v = inner.as_str().parse::<bool>().map_err(|_| {
                            ParseError::InvalidSyntax {
                                message: "Invalid boolean in literal type".to_string(),
                            }
                        })?;
                        crate::ast::Value::Boolean(v)
                    }
                    _ => {
                        return Err(ParseError::InvalidSyntax {
                            message: format!("Invalid literal type: {:?}", inner.as_rule()),
                        });
                    }
                };
                Ok(TypeAnnotation::Literal {
                    value: Box::new(value),
                })
            }
            Rule::basic_type => match pair.as_str() {
                "Int" => Ok(TypeAnnotation::Int),
                "Float" => Ok(TypeAnnotation::Float),
                "String" => Ok(TypeAnnotation::String),
                "Bool" => Ok(TypeAnnotation::Bool),
                "Map" => Ok(TypeAnnotation::Custom("Map".to_string())), // Fallback for standalone Map
                _ => Err(ParseError::InvalidSyntax {
                    message: format!("Unknown basic type: {}", pair.as_str()),
                }),
            },
            Rule::map_type => {
                let mut inner_pairs = pair.into_inner();
                // Skip "Map" literal
                inner_pairs.next();

                // Get the key type
                let key_type = inner_pairs
                    .next()
                    .ok_or_else(|| ParseError::InvalidSyntax {
                        message: "Missing key type for Map".to_string(),
                    })?;
                let key_annotation = self.build_type_annotation(key_type.into_inner())?;

                // Get the value type
                let value_type = inner_pairs
                    .next()
                    .ok_or_else(|| ParseError::InvalidSyntax {
                        message: "Missing value type for Map".to_string(),
                    })?;
                let value_annotation = self.build_type_annotation(value_type.into_inner())?;

                Ok(TypeAnnotation::Map {
                    key_type: Box::new(key_annotation),
                    value_type: Box::new(value_annotation),
                })
            }
            Rule::unit_type => Ok(TypeAnnotation::Unit),
            Rule::custom_type => Ok(TypeAnnotation::Custom(pair.as_str().to_string())),
            Rule::generic_type => {
                let mut inner_pairs = pair.into_inner();

                // Get the base type name
                let base_type = inner_pairs
                    .next()
                    .ok_or_else(|| ParseError::InvalidSyntax {
                        message: "Missing base type for generic type".to_string(),
                    })?
                    .as_str()
                    .to_string();

                // Parse type arguments
                let mut type_args = Vec::new();
                for arg_pair in inner_pairs {
                    if arg_pair.as_rule() == Rule::type_annotation {
                        type_args.push(self.build_type_annotation(arg_pair.into_inner())?);
                    }
                }

                Ok(TypeAnnotation::Generic {
                    base_type,
                    type_args,
                })
            }
            Rule::result_type => {
                let mut inner_pairs = pair.into_inner();

                // Get the ok type
                let ok_type = inner_pairs
                    .next()
                    .ok_or_else(|| ParseError::InvalidSyntax {
                        message: "Missing ok type for Result".to_string(),
                    })?;
                let ok_annotation = self.build_type_annotation(ok_type.into_inner())?;

                // Get the error type
                let err_type = inner_pairs
                    .next()
                    .ok_or_else(|| ParseError::InvalidSyntax {
                        message: "Missing error type for Result".to_string(),
                    })?;
                let err_annotation = self.build_type_annotation(err_type.into_inner())?;

                Ok(TypeAnnotation::Result {
                    ok_type: Box::new(ok_annotation),
                    err_type: Box::new(err_annotation),
                })
            }
            Rule::list_type => {
                let mut inner_pairs = pair.into_inner();
                let inner_type = inner_pairs
                    .next()
                    .ok_or_else(|| ParseError::InvalidSyntax {
                        message: "Missing inner type for list".to_string(),
                    })?;
                let inner_annotation = self.build_type_annotation(inner_type.into_inner())?;
                Ok(TypeAnnotation::List(Box::new(inner_annotation)))
            }
            Rule::tuple_type => {
                let inner_pairs = pair.into_inner();
                let mut types = Vec::new();

                // Parse all type annotations in the tuple
                for type_pair in inner_pairs {
                    if type_pair.as_rule() == Rule::type_annotation {
                        types.push(self.build_type_annotation(type_pair.into_inner())?);
                    }
                }

                if types.len() < 2 {
                    return Err(ParseError::InvalidSyntax {
                        message: "Tuple type must have at least 2 elements".to_string(),
                    });
                }

                Ok(TypeAnnotation::Tuple(types))
            }
            Rule::function_type => {
                let mut inner_pairs = pair.into_inner();
                let mut params = Vec::new();

                // The first part is either a single type annotation or a list of them in parens
                let first = inner_pairs
                    .next()
                    .ok_or_else(|| ParseError::InvalidSyntax {
                        message: "Missing parameter types in function type".to_string(),
                    })?;
                if first.as_rule() == Rule::type_annotation {
                    // Single parameter without parens (or the return type)
                    params.push(self.build_type_annotation(first.into_inner())?);
                } else {
                    // Parens with multiple parameters
                    for param_pair in first.into_inner() {
                        if param_pair.as_rule() == Rule::type_annotation {
                            params.push(self.build_type_annotation(param_pair.into_inner())?);
                        }
                    }
                }

                let return_type = inner_pairs
                    .next()
                    .ok_or_else(|| ParseError::InvalidSyntax {
                        message: "Missing return type in function type".to_string(),
                    })?;
                let return_annotation = self.build_type_annotation(return_type.into_inner())?;

                Ok(TypeAnnotation::Function {
                    params,
                    return_type: Box::new(return_annotation),
                })
            }
            Rule::anonymous_struct_type => {
                let fields = if let Some(field_list) = pair.into_inner().next() {
                    self.build_struct_field_list(field_list.into_inner())?
                } else {
                    Vec::new()
                };
                // For anonymous struct types, we create a synthetic Custom type
                // This is a simplified approach - in a full implementation, we might need
                // a separate TypeAnnotation variant for anonymous structs
                Ok(TypeAnnotation::Custom(format!(
                    "{{anonymous_struct_{}}}",
                    fields.len()
                )))
            }
            _ => Err(ParseError::InvalidSyntax {
                message: format!("Invalid type annotation rule: {:?}", pair.as_rule()),
            }),
        }
    }

    fn build_match_expr(&self, pairs: Pairs<Rule>) -> Result<Expr, ParseError> {
        let mut pairs = pairs.filter(|p| !Self::is_kw_pair(p.as_rule())).peekable();

        let value = if let Some(pair) = pairs.next() {
            if pair.as_rule() == Rule::expr {
                self.build_expr(pair.into_inner())?
            } else {
                return Err(ParseError::InvalidSyntax {
                    message: "Missing match value".to_string(),
                });
            }
        } else {
            return Err(ParseError::InvalidSyntax {
                message: "Missing match value".to_string(),
            });
        };

        let mut arms = Vec::new();
        for pair in pairs {
            if pair.as_rule() == Rule::match_arm {
                arms.push(self.build_match_arm(pair.into_inner())?);
            }
        }

        Ok(Expr::Match {
            value: Box::new(value),
            arms,
        })
    }

    fn build_match_arm(&self, pairs: Pairs<Rule>) -> Result<MatchArm, ParseError> {
        let mut pairs = pairs.filter(|p| !Self::is_kw_pair(p.as_rule())).peekable();

        let pattern = if let Some(pair) = pairs.next() {
            self.build_pattern(pair.into_inner())?
        } else {
            return Err(ParseError::InvalidSyntax {
                message: "Missing pattern in match arm".to_string(),
            });
        };

        let mut guard = None;
        let mut expression = None;

        // Check for optional guard clause and main expression
        while let Some(pair) = pairs.next() {
            if pair.as_rule() == Rule::expr {
                if guard.is_none() && pairs.peek().is_some() {
                    // This is a guard expression (there's another expr following)
                    guard = Some(Box::new(self.build_expr(pair.into_inner())?));
                } else {
                    // This is the main expression
                    expression = Some(self.build_expr(pair.into_inner())?);
                    break;
                }
            }
        }

        let expression = expression.ok_or_else(|| ParseError::InvalidSyntax {
            message: "Missing expression in match arm".to_string(),
        })?;

        Ok(MatchArm {
            pattern,
            guard,
            expression,
        })
    }

    fn build_pattern(&self, mut pairs: Pairs<Rule>) -> Result<Pattern, ParseError> {
        let pair = pairs.next().ok_or_else(|| ParseError::InvalidSyntax {
            message: "Empty pattern".to_string(),
        })?;

        match pair.as_rule() {
            Rule::range_pattern => {
                let pattern_str = pair.as_str();
                let mut inner = pair.into_inner();
                let start_pair = inner.next().ok_or_else(|| ParseError::InvalidSyntax {
                    message: "Missing start in range pattern".to_string(),
                })?;
                let operator_pair = inner.next().ok_or_else(|| ParseError::InvalidSyntax {
                    message: "Missing operator in range pattern".to_string(),
                })?;
                let end_pair = inner.next().ok_or_else(|| ParseError::InvalidSyntax {
                    message: "Missing end in range pattern".to_string(),
                })?;

                let inclusive = operator_pair.as_str() == "..=";

                // Handle both integer and char patterns
                let start_pattern =
                    match start_pair.as_rule() {
                        Rule::integer => {
                            let value = start_pair.as_str().parse::<i64>().map_err(|_| {
                                ParseError::InvalidSyntax {
                                    message: "Invalid integer in range pattern".to_string(),
                                }
                            })?;
                            Pattern::Literal(crate::ast::Value::Integer(value))
                        }
                        Rule::char_literal => {
                            let char_str = start_pair.as_str().trim_matches('\'');
                            let value = char_str.chars().next().ok_or_else(|| {
                                ParseError::InvalidSyntax {
                                    message: "Invalid character in range pattern".to_string(),
                                }
                            })?;
                            Pattern::Literal(crate::ast::Value::String(value.to_string().into()))
                        }
                        _ => {
                            return Err(ParseError::InvalidSyntax {
                                message: format!(
                                    "Invalid start type in range pattern '{}': {:?}",
                                    pattern_str,
                                    start_pair.as_rule()
                                ),
                            });
                        }
                    };

                let end_pattern =
                    match end_pair.as_rule() {
                        Rule::integer => {
                            let value = end_pair.as_str().parse::<i64>().map_err(|_| {
                                ParseError::InvalidSyntax {
                                    message: "Invalid integer in range pattern".to_string(),
                                }
                            })?;
                            Pattern::Literal(crate::ast::Value::Integer(value))
                        }
                        Rule::char_literal => {
                            let char_str = end_pair.as_str().trim_matches('\'');
                            let value = char_str.chars().next().ok_or_else(|| {
                                ParseError::InvalidSyntax {
                                    message: "Invalid character in range pattern".to_string(),
                                }
                            })?;
                            Pattern::Literal(crate::ast::Value::String(value.to_string().into()))
                        }
                        _ => {
                            return Err(ParseError::InvalidSyntax {
                                message: format!(
                                    "Invalid end type in range pattern '{}': {:?}",
                                    pattern_str,
                                    end_pair.as_rule()
                                ),
                            });
                        }
                    };

                Ok(Pattern::Range {
                    start: Box::new(start_pattern),
                    end: Box::new(end_pattern),
                    inclusive,
                })
            }
            Rule::or_pattern => {
                let mut inner = pair.into_inner();
                let first = inner.next().ok_or_else(|| ParseError::InvalidSyntax {
                    message: "Empty or-pattern".to_string(),
                })?;
                let first_pattern = self.build_base_pattern(first)?;

                let mut alternatives = vec![first_pattern];
                for alt_pair in inner {
                    alternatives.push(self.build_base_pattern(alt_pair)?);
                }

                if alternatives.len() == 1 {
                    // Safe unwrap since we verified len() == 1
                    alternatives
                        .into_iter()
                        .next()
                        .ok_or_else(|| ParseError::InvalidSyntax {
                            message: "Internal error: expected one alternative in or-pattern"
                                .to_string(),
                        })
                } else {
                    Ok(Pattern::Or { alternatives })
                }
            }
            Rule::result_pattern => {
                // Handle Ok(...) and Err(...) patterns
                let full_str = pair.as_str(); // Get string before moving
                let inner_pairs = pair.into_inner();

                // The first pair should tell us the variant
                // We need to iterate through and find the pattern, then determine variant differently
                let pairs_vec: Vec<_> = inner_pairs.collect();

                // Let's reconstruct what we need by examining what we have
                let mut variant = None;
                let mut inner_pattern = None;

                for p in pairs_vec.iter() {
                    match p.as_rule() {
                        Rule::pattern => {
                            inner_pattern = Some(self.build_pattern(p.clone().into_inner())?);
                        }
                        _ => {
                            // This might be a literal token, let's check the string
                            let token_str = p.as_str();
                            if token_str == "Ok" {
                                variant = Some("Ok");
                            } else if token_str == "Err" {
                                variant = Some("Err");
                            }
                        }
                    }
                }

                // If we didn't find variant from tokens, try the original string
                if variant.is_none() {
                    if full_str.starts_with("Ok(") {
                        variant = Some("Ok");
                    } else if full_str.starts_with("Err(") {
                        variant = Some("Err");
                    }
                }

                let variant = variant.ok_or_else(|| ParseError::InvalidSyntax {
                    message: "Could not determine result pattern variant (Ok or Err)".to_string(),
                })?;

                let inner_pattern = inner_pattern.ok_or_else(|| ParseError::InvalidSyntax {
                    message: "Missing inner pattern in result pattern".to_string(),
                })?;

                match variant {
                    "Ok" => Ok(Pattern::Ok(Box::new(inner_pattern))),
                    "Err" => Ok(Pattern::Err(Box::new(inner_pattern))),
                    _ => unreachable!(),
                }
            }
            Rule::identifier => {
                let name = pair.as_str().to_string();
                Ok(Pattern::Identifier(name))
            }
            Rule::pattern_wildcard => Ok(Pattern::Wildcard),
            // `()` as a pattern: the Unit value, matched literally.
            Rule::unit => Ok(Pattern::Literal(crate::ast::Value::Unit)),
            Rule::integer => {
                let value =
                    pair.as_str()
                        .parse::<i64>()
                        .map_err(|_| ParseError::InvalidSyntax {
                            message: "Invalid integer in pattern".to_string(),
                        })?;
                Ok(Pattern::Literal(crate::ast::Value::Integer(value)))
            }
            Rule::string => {
                let value = self.unquote_string(pair.as_str())?;
                Ok(Pattern::Literal(crate::ast::Value::String(value.into())))
            }
            Rule::boolean => {
                let value =
                    pair.as_str()
                        .parse::<bool>()
                        .map_err(|_| ParseError::InvalidSyntax {
                            message: "Invalid boolean in pattern".to_string(),
                        })?;
                Ok(Pattern::Literal(crate::ast::Value::Boolean(value)))
            }
            Rule::rest_pattern => {
                let mut inner = pair.into_inner();
                let identifier = inner.next().ok_or_else(|| ParseError::InvalidSyntax {
                    message: "Missing identifier in rest pattern".to_string(),
                })?;
                Ok(Pattern::Rest(identifier.as_str().to_string()))
            }
            Rule::tuple_pattern => {
                let mut patterns = Vec::new();
                for p in pair.into_inner() {
                    if p.as_rule() == Rule::pattern {
                        patterns.push(self.build_pattern(p.into_inner())?);
                    }
                }
                Ok(Pattern::Tuple(patterns))
            }
            Rule::list_pattern => {
                let mut patterns = Vec::new();
                let mut rest = None;

                for p in pair.into_inner() {
                    if p.as_rule() == Rule::list_pattern_inner {
                        for inner_p in p.into_inner() {
                            match inner_p.as_rule() {
                                Rule::pattern => {
                                    patterns.push(self.build_pattern(inner_p.into_inner())?);
                                }
                                Rule::rest_pattern => {
                                    let mut rest_inner = inner_p.into_inner();
                                    if let Some(identifier) = rest_inner.next() {
                                        rest = Some(identifier.as_str().to_string());
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }

                Ok(Pattern::List { patterns, rest })
            }
            Rule::enum_variant_pattern => {
                let mut inner_pairs = pair.into_inner();
                let variant_name = inner_pairs
                    .next()
                    .ok_or_else(|| ParseError::InvalidSyntax {
                        message: "Missing variant name in enum pattern".to_string(),
                    })?
                    .as_str()
                    .to_string();

                let mut patterns = Vec::new();
                for p in inner_pairs {
                    if p.as_rule() == Rule::pattern {
                        patterns.push(self.build_pattern(p.into_inner())?);
                    }
                }

                Ok(Pattern::EnumVariant {
                    variant_name,
                    patterns,
                })
            }
            Rule::struct_pattern => {
                let mut inner_pairs = pair.into_inner();
                let type_name = inner_pairs
                    .next()
                    .ok_or_else(|| ParseError::InvalidSyntax {
                        message: "Missing type name in struct pattern".to_string(),
                    })?
                    .as_str()
                    .to_string();

                let mut field_patterns = Vec::new();
                for field_pair in inner_pairs {
                    if field_pair.as_rule() == Rule::struct_pattern_fields {
                        for field_inner in field_pair.into_inner() {
                            if field_inner.as_rule() == Rule::struct_pattern_field {
                                let mut field_inner_pairs = field_inner.into_inner();
                                let field_name = field_inner_pairs
                                    .next()
                                    .ok_or_else(|| ParseError::InvalidSyntax {
                                        message: "Missing field name in struct pattern".to_string(),
                                    })?
                                    .as_str()
                                    .to_string();

                                if let Some(pattern_pair) = field_inner_pairs.next() {
                                    // Long form: field_name: pattern
                                    let field_pattern =
                                        self.build_pattern(pattern_pair.into_inner())?;
                                    field_patterns.push((field_name.clone(), field_pattern));
                                } else {
                                    // Shorthand: field_name (equivalent to field_name: field_name)
                                    field_patterns.push((
                                        field_name.clone(),
                                        Pattern::Identifier(field_name),
                                    ));
                                }
                            }
                        }
                    }
                }

                Ok(Pattern::Struct {
                    type_name,
                    field_patterns,
                })
            }
            Rule::anonymous_struct_pattern => {
                let mut field_patterns = Vec::new();
                for field_pair in pair.into_inner() {
                    if field_pair.as_rule() == Rule::struct_pattern_fields {
                        for field_inner in field_pair.into_inner() {
                            if field_inner.as_rule() == Rule::struct_pattern_field {
                                let mut field_inner_pairs = field_inner.into_inner();
                                let field_name = field_inner_pairs
                                    .next()
                                    .ok_or_else(|| ParseError::InvalidSyntax {
                                        message: "Missing field name in anonymous struct pattern"
                                            .to_string(),
                                    })?
                                    .as_str()
                                    .to_string();

                                if let Some(pattern_pair) = field_inner_pairs.next() {
                                    // Long form: field_name: pattern
                                    let field_pattern =
                                        self.build_pattern(pattern_pair.into_inner())?;
                                    field_patterns.push((field_name.clone(), field_pattern));
                                } else {
                                    // Shorthand: field_name (equivalent to field_name: field_name)
                                    field_patterns.push((
                                        field_name.clone(),
                                        Pattern::Identifier(field_name),
                                    ));
                                }
                            }
                        }
                    }
                }

                Ok(Pattern::AnonymousStruct { field_patterns })
            }
            Rule::base_pattern => {
                // Handle base_pattern by recursing into its inner content
                let inner = pair
                    .into_inner()
                    .next()
                    .ok_or_else(|| ParseError::InvalidSyntax {
                        message: "Empty base pattern".to_string(),
                    })?;
                self.build_base_pattern(inner)
            }
            _ => Err(ParseError::InvalidSyntax {
                message: format!("Invalid pattern rule: {:?}", pair.as_rule()),
            }),
        }
    }

    fn build_base_pattern(&self, pair: Pair<Rule>) -> Result<Pattern, ParseError> {
        match pair.as_rule() {
            Rule::range_pattern => {
                let pattern_str = pair.as_str();
                let mut inner = pair.into_inner();
                let start_pair = inner.next().ok_or_else(|| ParseError::InvalidSyntax {
                    message: "Missing start in range pattern".to_string(),
                })?;
                let operator_pair = inner.next().ok_or_else(|| ParseError::InvalidSyntax {
                    message: "Missing operator in range pattern".to_string(),
                })?;
                let end_pair = inner.next().ok_or_else(|| ParseError::InvalidSyntax {
                    message: "Missing end in range pattern".to_string(),
                })?;

                let inclusive = operator_pair.as_str() == "..=";

                // Handle both integer and char patterns
                let start_pattern =
                    match start_pair.as_rule() {
                        Rule::integer => {
                            let value = start_pair.as_str().parse::<i64>().map_err(|_| {
                                ParseError::InvalidSyntax {
                                    message: "Invalid integer in range pattern".to_string(),
                                }
                            })?;
                            Pattern::Literal(crate::ast::Value::Integer(value))
                        }
                        Rule::char_literal => {
                            let char_str = start_pair.as_str().trim_matches('\'');
                            let value = char_str.chars().next().ok_or_else(|| {
                                ParseError::InvalidSyntax {
                                    message: "Invalid character in range pattern".to_string(),
                                }
                            })?;
                            Pattern::Literal(crate::ast::Value::String(value.to_string().into()))
                        }
                        _ => {
                            return Err(ParseError::InvalidSyntax {
                                message: format!(
                                    "Invalid start type in range pattern '{}': {:?}",
                                    pattern_str,
                                    start_pair.as_rule()
                                ),
                            });
                        }
                    };

                let end_pattern =
                    match end_pair.as_rule() {
                        Rule::integer => {
                            let value = end_pair.as_str().parse::<i64>().map_err(|_| {
                                ParseError::InvalidSyntax {
                                    message: "Invalid integer in range pattern".to_string(),
                                }
                            })?;
                            Pattern::Literal(crate::ast::Value::Integer(value))
                        }
                        Rule::char_literal => {
                            let char_str = end_pair.as_str().trim_matches('\'');
                            let value = char_str.chars().next().ok_or_else(|| {
                                ParseError::InvalidSyntax {
                                    message: "Invalid character in range pattern".to_string(),
                                }
                            })?;
                            Pattern::Literal(crate::ast::Value::String(value.to_string().into()))
                        }
                        _ => {
                            return Err(ParseError::InvalidSyntax {
                                message: format!(
                                    "Invalid end type in range pattern '{}': {:?}",
                                    pattern_str,
                                    end_pair.as_rule()
                                ),
                            });
                        }
                    };

                Ok(Pattern::Range {
                    start: Box::new(start_pattern),
                    end: Box::new(end_pattern),
                    inclusive,
                })
            }
            Rule::or_pattern => {
                let mut inner = pair.into_inner();
                let first = inner.next().ok_or_else(|| ParseError::InvalidSyntax {
                    message: "Empty or-pattern".to_string(),
                })?;
                let first_pattern = self.build_base_pattern(first)?;

                let mut alternatives = vec![first_pattern];
                for alt_pair in inner {
                    alternatives.push(self.build_base_pattern(alt_pair)?);
                }

                if alternatives.len() == 1 {
                    // Safe unwrap since we verified len() == 1
                    alternatives
                        .into_iter()
                        .next()
                        .ok_or_else(|| ParseError::InvalidSyntax {
                            message: "Internal error: expected one alternative in or-pattern"
                                .to_string(),
                        })
                } else {
                    Ok(Pattern::Or { alternatives })
                }
            }
            Rule::result_pattern => {
                // Handle Ok(...) and Err(...) patterns
                let full_str = pair.as_str(); // Get string before moving
                let inner_pairs = pair.into_inner();

                // The first pair should tell us the variant
                // We need to iterate through and find the pattern, then determine variant differently
                let pairs_vec: Vec<_> = inner_pairs.collect();

                // Let's reconstruct what we need by examining what we have
                let mut variant = None;
                let mut inner_pattern = None;

                for p in pairs_vec.iter() {
                    match p.as_rule() {
                        Rule::pattern => {
                            inner_pattern = Some(self.build_pattern(p.clone().into_inner())?);
                        }
                        _ => {
                            // This might be a literal token, let's check the string
                            let token_str = p.as_str();
                            if token_str == "Ok" {
                                variant = Some("Ok");
                            } else if token_str == "Err" {
                                variant = Some("Err");
                            }
                        }
                    }
                }

                // If we didn't find variant from tokens, try the original string
                if variant.is_none() {
                    if full_str.starts_with("Ok(") {
                        variant = Some("Ok");
                    } else if full_str.starts_with("Err(") {
                        variant = Some("Err");
                    }
                }

                let variant = variant.ok_or_else(|| ParseError::InvalidSyntax {
                    message: "Could not determine result pattern variant (Ok or Err)".to_string(),
                })?;

                let inner_pattern = inner_pattern.ok_or_else(|| ParseError::InvalidSyntax {
                    message: "Missing inner pattern in result pattern".to_string(),
                })?;

                match variant {
                    "Ok" => Ok(Pattern::Ok(Box::new(inner_pattern))),
                    "Err" => Ok(Pattern::Err(Box::new(inner_pattern))),
                    _ => unreachable!(),
                }
            }
            Rule::identifier => {
                let name = pair.as_str().to_string();
                Ok(Pattern::Identifier(name))
            }
            Rule::pattern_wildcard => Ok(Pattern::Wildcard),
            // `()` as a pattern: the Unit value, matched literally.
            Rule::unit => Ok(Pattern::Literal(crate::ast::Value::Unit)),
            Rule::integer => {
                let value =
                    pair.as_str()
                        .parse::<i64>()
                        .map_err(|_| ParseError::InvalidSyntax {
                            message: "Invalid integer in pattern".to_string(),
                        })?;
                Ok(Pattern::Literal(crate::ast::Value::Integer(value)))
            }
            Rule::string => {
                let value = self.unquote_string(pair.as_str())?;
                Ok(Pattern::Literal(crate::ast::Value::String(value.into())))
            }
            Rule::boolean => {
                let value =
                    pair.as_str()
                        .parse::<bool>()
                        .map_err(|_| ParseError::InvalidSyntax {
                            message: "Invalid boolean in pattern".to_string(),
                        })?;
                Ok(Pattern::Literal(crate::ast::Value::Boolean(value)))
            }
            Rule::list_pattern => {
                let mut patterns = Vec::new();
                let mut rest = None;

                for p in pair.into_inner() {
                    if p.as_rule() == Rule::list_pattern_inner {
                        for inner_p in p.into_inner() {
                            match inner_p.as_rule() {
                                Rule::pattern => {
                                    patterns.push(self.build_pattern(inner_p.into_inner())?);
                                }
                                Rule::rest_pattern => {
                                    let mut rest_inner = inner_p.into_inner();
                                    if let Some(identifier) = rest_inner.next() {
                                        rest = Some(identifier.as_str().to_string());
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }

                Ok(Pattern::List { patterns, rest })
            }
            Rule::rest_pattern => {
                let mut inner = pair.into_inner();
                let identifier = inner.next().ok_or_else(|| ParseError::InvalidSyntax {
                    message: "Missing identifier in rest pattern".to_string(),
                })?;
                Ok(Pattern::Rest(identifier.as_str().to_string()))
            }
            Rule::tuple_pattern => {
                let mut patterns = Vec::new();
                for p in pair.into_inner() {
                    if p.as_rule() == Rule::pattern {
                        patterns.push(self.build_pattern(p.into_inner())?);
                    }
                }
                Ok(Pattern::Tuple(patterns))
            }
            Rule::enum_variant_pattern => {
                let mut inner_pairs = pair.into_inner();
                let variant_name = inner_pairs
                    .next()
                    .ok_or_else(|| ParseError::InvalidSyntax {
                        message: "Missing variant name in enum pattern".to_string(),
                    })?
                    .as_str()
                    .to_string();

                let mut patterns = Vec::new();
                for p in inner_pairs {
                    if p.as_rule() == Rule::pattern {
                        patterns.push(self.build_pattern(p.into_inner())?);
                    }
                }

                Ok(Pattern::EnumVariant {
                    variant_name,
                    patterns,
                })
            }
            Rule::struct_pattern => {
                let mut inner_pairs = pair.into_inner();
                let type_name = inner_pairs
                    .next()
                    .ok_or_else(|| ParseError::InvalidSyntax {
                        message: "Missing type name in struct pattern".to_string(),
                    })?
                    .as_str()
                    .to_string();

                let mut field_patterns = Vec::new();
                for field_pair in inner_pairs {
                    if field_pair.as_rule() == Rule::struct_pattern_fields {
                        for field_inner in field_pair.into_inner() {
                            if field_inner.as_rule() == Rule::struct_pattern_field {
                                let mut field_inner_pairs = field_inner.into_inner();
                                let field_name = field_inner_pairs
                                    .next()
                                    .ok_or_else(|| ParseError::InvalidSyntax {
                                        message: "Missing field name in struct pattern".to_string(),
                                    })?
                                    .as_str()
                                    .to_string();

                                if let Some(pattern_pair) = field_inner_pairs.next() {
                                    // Long form: field_name: pattern
                                    let field_pattern =
                                        self.build_pattern(pattern_pair.into_inner())?;
                                    field_patterns.push((field_name.clone(), field_pattern));
                                } else {
                                    // Shorthand: field_name (equivalent to field_name: field_name)
                                    field_patterns.push((
                                        field_name.clone(),
                                        Pattern::Identifier(field_name),
                                    ));
                                }
                            }
                        }
                    }
                }

                Ok(Pattern::Struct {
                    type_name,
                    field_patterns,
                })
            }
            Rule::anonymous_struct_pattern => {
                let mut field_patterns = Vec::new();
                for field_pair in pair.into_inner() {
                    if field_pair.as_rule() == Rule::struct_pattern_fields {
                        for field_inner in field_pair.into_inner() {
                            if field_inner.as_rule() == Rule::struct_pattern_field {
                                let mut field_inner_pairs = field_inner.into_inner();
                                let field_name = field_inner_pairs
                                    .next()
                                    .ok_or_else(|| ParseError::InvalidSyntax {
                                        message: "Missing field name in anonymous struct pattern"
                                            .to_string(),
                                    })?
                                    .as_str()
                                    .to_string();

                                if let Some(pattern_pair) = field_inner_pairs.next() {
                                    // Long form: field_name: pattern
                                    let field_pattern =
                                        self.build_pattern(pattern_pair.into_inner())?;
                                    field_patterns.push((field_name.clone(), field_pattern));
                                } else {
                                    // Shorthand: field_name (equivalent to field_name: field_name)
                                    field_patterns.push((
                                        field_name.clone(),
                                        Pattern::Identifier(field_name),
                                    ));
                                }
                            }
                        }
                    }
                }

                Ok(Pattern::AnonymousStruct { field_patterns })
            }
            Rule::base_pattern => {
                // Handle base_pattern by recursing into its inner content
                let inner = pair
                    .into_inner()
                    .next()
                    .ok_or_else(|| ParseError::InvalidSyntax {
                        message: "Empty base pattern".to_string(),
                    })?;
                self.build_base_pattern(inner)
            }
            _ => Err(ParseError::InvalidSyntax {
                message: format!("Invalid pattern rule: {:?}", pair.as_rule()),
            }),
        }
    }

    fn build_literal(&self, mut pairs: Pairs<Rule>) -> Result<Expr, ParseError> {
        let pair = pairs.next().ok_or_else(|| ParseError::InvalidSyntax {
            message: "Empty literal".to_string(),
        })?;

        match pair.as_rule() {
            // `()` is the empty tuple, which is what Unit *is* — so it
            // needs no Expr variant of its own, and every tier that
            // already handles tuples handles it.
            Rule::unit => Ok(Expr::Tuple(std::sync::Arc::new(Vec::new()))),
            Rule::integer => {
                // Remove numeric separators
                let s = pair.as_str().replace('_', "");
                let value = s.parse::<i64>().map_err(|_| {
                    ParseError::invalid_syntax_at(
                        "Invalid integer literal".to_string(),
                        PositionInfo::from_pair(&pair),
                    )
                })?;
                Ok(Expr::Integer(value))
            }
            Rule::float => {
                let s = pair.as_str().replace('_', "");
                let value = s.parse::<f64>().map_err(|_| {
                    ParseError::invalid_syntax_at(
                        "Invalid float literal".to_string(),
                        PositionInfo::from_pair(&pair),
                    )
                })?;
                // A Float is never inf: a literal beyond f64's range is a
                // syntax error, not infinity.
                if !value.is_finite() {
                    return Err(ParseError::invalid_syntax_at(
                        format!(
                            "Float literal {} is out of range (the largest float is about 1.8e308)",
                            s
                        ),
                        PositionInfo::from_pair(&pair),
                    ));
                }
                Ok(Expr::Float(value))
            }
            Rule::binary => {
                let raw = pair.as_str().replace('_', "");
                let (sign, digits) = if let Some(rest) = raw.strip_prefix("-0b") {
                    (-1i64, rest)
                } else if let Some(rest) = raw.strip_prefix("+0b") {
                    (1i64, rest)
                } else {
                    (1i64, &raw[2..])
                };
                let parsed = i64::from_str_radix(digits, 2).map_err(|_| {
                    ParseError::invalid_syntax_at(
                        "Invalid binary literal".to_string(),
                        PositionInfo::from_pair(&pair),
                    )
                })?;
                Ok(Expr::Integer(sign * parsed))
            }
            Rule::octal => {
                let raw = pair.as_str().replace('_', "");
                let (sign, digits) = if let Some(rest) = raw.strip_prefix("-0o") {
                    (-1i64, rest)
                } else if let Some(rest) = raw.strip_prefix("+0o") {
                    (1i64, rest)
                } else {
                    (1i64, &raw[2..])
                };
                let parsed = i64::from_str_radix(digits, 8).map_err(|_| {
                    ParseError::invalid_syntax_at(
                        "Invalid octal literal".to_string(),
                        PositionInfo::from_pair(&pair),
                    )
                })?;
                Ok(Expr::Integer(sign * parsed))
            }
            Rule::hex => {
                let raw = pair.as_str().replace('_', "");
                let (sign, digits) = if let Some(rest) = raw.strip_prefix("-0x") {
                    (-1i64, rest)
                } else if let Some(rest) = raw.strip_prefix("+0x") {
                    (1i64, rest)
                } else {
                    (1i64, &raw[2..])
                };
                let parsed = i64::from_str_radix(digits, 16).map_err(|_| {
                    ParseError::invalid_syntax_at(
                        "Invalid hex literal".to_string(),
                        PositionInfo::from_pair(&pair),
                    )
                })?;
                Ok(Expr::Integer(sign * parsed))
            }
            Rule::string => {
                let full_str = pair.as_str();
                // Extract content between quotes, handling edge cases
                if full_str.len() >= 2 && full_str.starts_with('"') && full_str.ends_with('"') {
                    let raw_value = &full_str[1..full_str.len() - 1];
                    let value = self.process_string_escapes(raw_value)?;
                    Ok(Expr::String(value.into()))
                } else {
                    Err(ParseError::invalid_syntax_at(
                        "Malformed string literal".to_string(),
                        PositionInfo::from_pair(&pair),
                    ))
                }
            }
            Rule::raw_string => {
                let raw_value = pair.as_str();
                // Remove r" and ending ", but do not process escapes
                let value = &raw_value[2..raw_value.len() - 1];
                Ok(Expr::RawString(Rc::new(value.to_string())))
            }
            Rule::char_literal => {
                let char_str = pair.as_str().trim_matches('\'');
                let value = char_str.chars().next().ok_or_else(|| {
                    ParseError::invalid_syntax_at(
                        "Invalid character literal".to_string(),
                        PositionInfo::from_pair(&pair),
                    )
                })?;
                Ok(Expr::String(value.to_string().into()))
            }
            Rule::template_string => self.build_template_string(pair.into_inner()),
            Rule::boolean => {
                let value = pair.as_str().parse::<bool>().map_err(|_| {
                    ParseError::invalid_syntax_at(
                        "Invalid boolean literal".to_string(),
                        PositionInfo::from_pair(&pair),
                    )
                })?;
                Ok(Expr::Boolean(value))
            }
            Rule::list => {
                let mut items = Vec::new();
                for p in pair.into_inner() {
                    if p.as_rule() == Rule::expr {
                        items.push(self.build_expr(p.into_inner())?);
                    }
                }
                Ok(Expr::List(items.into()))
            }
            _ => Err(ParseError::invalid_syntax_at(
                format!("Invalid literal rule: {:?}", pair.as_rule()),
                PositionInfo::from_pair(&pair),
            )),
        }
    }

    fn build_block(&self, pairs: Pairs<Rule>) -> Result<Expr, ParseError> {
        let mut statements = Vec::new();

        for pair in pairs {
            if pair.as_rule() == Rule::statement {
                let (line, column) = pair.as_span().start_pos().line_col();
                let inner = pair
                    .into_inner()
                    .next()
                    .ok_or_else(|| ParseError::InvalidSyntax {
                        message: "Empty statement in block".to_string(),
                    })?;
                statements.push(Self::locate(self.build_statement(inner)?, line, column));
            }
        }

        Ok(Expr::Block(statements))
    }

    fn build_if_expr(&self, pairs: Pairs<Rule>) -> Result<Expr, ParseError> {
        let mut pairs = pairs.filter(|p| !Self::is_kw_pair(p.as_rule())).peekable();

        let condition_pair = pairs.next().ok_or_else(|| ParseError::InvalidSyntax {
            message: "Missing condition in if expression".to_string(),
        })?;
        let condition = self.build_expr(condition_pair.into_inner())?;

        let then_branch = if let Some(pair) = pairs.next() {
            match pair.as_rule() {
                Rule::block => self.build_block(pair.into_inner())?,
                Rule::expr => self.build_expr(pair.into_inner())?,
                _ => {
                    return Err(ParseError::InvalidSyntax {
                        message: "Invalid then branch in if expression".to_string(),
                    });
                }
            }
        } else {
            return Err(ParseError::InvalidSyntax {
                message: "Missing then branch in if expression".to_string(),
            });
        };

        let else_branch = if let Some(pair) = pairs.next() {
            match pair.as_rule() {
                // `else if ...` — the else branch is itself a full if-expression,
                // so the chain nests without requiring `else => if ...`.
                Rule::if_expr => Some(Box::new(self.build_if_expr(pair.into_inner())?)),
                Rule::block => Some(Box::new(self.build_block(pair.into_inner())?)),
                Rule::expr => Some(Box::new(self.build_expr(pair.into_inner())?)),
                _ => None,
            }
        } else {
            None
        };

        Ok(Expr::If {
            condition: Box::new(condition),
            then_branch: Box::new(then_branch),
            else_branch,
        })
    }

    fn build_trait_decl(&self, pairs: Pairs<Rule>) -> Result<crate::ast::TraitDecl, ParseError> {
        let mut pairs = pairs.filter(|p| !Self::is_kw_pair(p.as_rule())).peekable();

        let name = pairs
            .next()
            .ok_or_else(|| ParseError::InvalidSyntax {
                message: "Missing trait name".to_string(),
            })?
            .as_str()
            .to_string();

        let mut methods = Vec::new();
        for pair in pairs {
            if pair.as_rule() == Rule::trait_method {
                methods.push(self.build_trait_method(pair.into_inner())?);
            }
        }
        Ok(crate::ast::TraitDecl { name, methods })
    }

    fn build_trait_method(
        &self,
        pairs: Pairs<Rule>,
    ) -> Result<crate::ast::TraitMethod, ParseError> {
        let mut pairs = pairs.filter(|p| !Self::is_kw_pair(p.as_rule())).peekable();

        let name = pairs
            .next()
            .ok_or_else(|| ParseError::InvalidSyntax {
                message: "Missing trait method name".to_string(),
            })?
            .as_str()
            .to_string();

        let mut parameters = Vec::new();
        let mut default_body = None;
        for pair in pairs {
            match pair.as_rule() {
                Rule::param_list => parameters = self.build_param_list(pair.into_inner())?,
                Rule::expr => default_body = Some(self.build_expr(pair.into_inner())?),
                // return-type annotation is accepted but not retained at runtime
                _ => {}
            }
        }
        Ok(crate::ast::TraitMethod {
            name,
            parameters,
            default_body,
        })
    }

    fn build_impl_decl(&self, pairs: Pairs<Rule>) -> Result<crate::ast::ImplDecl, ParseError> {
        let mut pairs = pairs.filter(|p| !Self::is_kw_pair(p.as_rule())).peekable();

        let trait_name = pairs
            .next()
            .ok_or_else(|| ParseError::InvalidSyntax {
                message: "Missing trait name in impl".to_string(),
            })?
            .as_str()
            .to_string();
        let type_name = pairs
            .next()
            .ok_or_else(|| ParseError::InvalidSyntax {
                message: "Missing type name in impl".to_string(),
            })?
            .as_str()
            .to_string();

        let mut methods = Vec::new();
        for pair in pairs {
            if pair.as_rule() == Rule::function_decl {
                methods.push(self.build_function_decl(pair.into_inner())?);
            }
        }
        Ok(crate::ast::ImplDecl {
            trait_name,
            type_name,
            methods,
        })
    }

    /// Parse a `type_params` pair into (names, bounds). Each `type_param` is
    /// `identifier (: trait (+ trait)*)?`; bounds are (param name, traits).
    fn parse_type_params(&self, pair: Pair<Rule>) -> (Vec<String>, Vec<(String, Vec<String>)>) {
        let mut names = Vec::new();
        let mut bounds = Vec::new();
        for type_param in pair.into_inner() {
            if type_param.as_rule() != Rule::type_param {
                continue;
            }
            let mut idents = type_param
                .into_inner()
                .filter(|p| p.as_rule() == Rule::identifier);
            if let Some(name_pair) = idents.next() {
                let name = name_pair.as_str().to_string();
                let traits: Vec<String> = idents.map(|p| p.as_str().to_string()).collect();
                if !traits.is_empty() {
                    bounds.push((name.clone(), traits));
                }
                names.push(name);
            }
        }
        (names, bounds)
    }

    fn build_function_decl(&self, pairs: Pairs<Rule>) -> Result<FunctionDecl, ParseError> {
        let mut pairs = pairs.filter(|p| !Self::is_kw_pair(p.as_rule())).peekable();

        let name_pair = pairs.next().ok_or_else(|| ParseError::InvalidSyntax {
            message: "Missing name".to_string(),
        })?;
        let (nl, nc) = name_pair.as_span().start_pos().line_col();
        let name_span = Some((nl as u32, nc as u32));
        let name = name_pair.as_str().to_string();

        let mut type_params = Vec::new();
        let mut type_param_bounds = Vec::new();
        let mut parameters = Vec::new();
        let mut return_type = None;
        let mut body_expr = None;

        for pair in pairs {
            match pair.as_rule() {
                Rule::type_params => {
                    let (names, bounds) = self.parse_type_params(pair);
                    type_params = names;
                    type_param_bounds = bounds;
                }
                Rule::param_list => {
                    parameters = self.build_param_list(pair.into_inner())?;
                }
                Rule::type_annotation => {
                    return_type = Some(self.build_type_annotation(pair.into_inner())?);
                }
                Rule::expr => {
                    body_expr = Some(self.build_expr(pair.into_inner())?);
                }
                _ => {}
            }
        }

        let body = body_expr.ok_or_else(|| ParseError::InvalidSyntax {
            message: "Missing function body".to_string(),
        })?;

        Ok(FunctionDecl {
            name,
            name_span,
            type_params,
            type_param_bounds,
            parameters,
            return_type,
            body,
        })
    }

    fn build_type_decl(&self, pairs: Pairs<Rule>) -> Result<TypeDecl, ParseError> {
        let mut pairs = pairs.filter(|p| !Self::is_kw_pair(p.as_rule())).peekable();

        let name_pair = pairs.next().ok_or_else(|| ParseError::InvalidSyntax {
            message: "Missing name".to_string(),
        })?;
        let (nl, nc) = name_pair.as_span().start_pos().line_col();
        let name_span = Some((nl as u32, nc as u32));
        let name = name_pair.as_str().to_string();

        let mut type_params = Vec::new();
        let mut definition = None;

        for pair in pairs {
            match pair.as_rule() {
                Rule::type_params => {
                    let (names, _bounds) = self.parse_type_params(pair);
                    type_params = names;
                }
                Rule::type_definition => {
                    definition = Some(self.build_type_definition(pair.into_inner())?);
                }
                _ => {}
            }
        }

        let definition = definition.ok_or_else(|| ParseError::InvalidSyntax {
            message: "Missing type definition".to_string(),
        })?;

        Ok(TypeDecl {
            name,
            name_span,
            type_params,
            definition,
        })
    }

    fn build_type_definition(&self, mut pairs: Pairs<Rule>) -> Result<TypeDefinition, ParseError> {
        let pair = pairs.next().ok_or_else(|| ParseError::InvalidSyntax {
            message: "Empty type definition".to_string(),
        })?;

        match pair.as_rule() {
            Rule::union_type_def => {
                let mut types = Vec::new();
                for type_pair in pair.into_inner() {
                    if type_pair.as_rule() == Rule::type_annotation {
                        types.push(self.build_type_annotation(type_pair.into_inner())?);
                    }
                }
                Ok(TypeDefinition::Union { types })
            }
            Rule::struct_def => {
                let fields = if let Some(field_list) = pair
                    .into_inner()
                    .find(|p| p.as_rule() == Rule::struct_field_list)
                {
                    self.build_struct_field_list(field_list.into_inner())?
                } else {
                    Vec::new()
                };
                Ok(TypeDefinition::Struct { fields })
            }
            Rule::enum_def => {
                let variants = if let Some(variant_list) = pair
                    .into_inner()
                    .find(|p| p.as_rule() == Rule::enum_variant_list)
                {
                    self.build_enum_variant_list(variant_list.into_inner())?
                } else {
                    Vec::new()
                };
                Ok(TypeDefinition::Enum { variants })
            }
            _ => Err(ParseError::InvalidSyntax {
                message: format!("Invalid type definition: {:?}", pair.as_rule()),
            }),
        }
    }

    fn build_struct_field_list(&self, pairs: Pairs<Rule>) -> Result<Vec<StructField>, ParseError> {
        let mut fields = Vec::new();
        for pair in pairs {
            if pair.as_rule() == Rule::struct_field {
                fields.push(self.build_struct_field(pair.into_inner())?);
            }
        }
        Ok(fields)
    }

    fn build_struct_field(&self, mut pairs: Pairs<Rule>) -> Result<StructField, ParseError> {
        let name = pairs
            .next()
            .ok_or_else(|| ParseError::InvalidSyntax {
                message: "Missing field name".to_string(),
            })?
            .as_str()
            .to_string();

        let field_type = pairs.next().ok_or_else(|| ParseError::InvalidSyntax {
            message: "Missing field type".to_string(),
        })?;

        let type_annotation = self.build_type_annotation(field_type.into_inner())?;

        Ok(StructField {
            name,
            field_type: type_annotation,
        })
    }

    fn build_enum_variant_list(&self, pairs: Pairs<Rule>) -> Result<Vec<EnumVariant>, ParseError> {
        let mut variants = Vec::new();
        for pair in pairs {
            if pair.as_rule() == Rule::enum_variant {
                variants.push(self.build_enum_variant(pair.into_inner())?);
            }
        }
        Ok(variants)
    }

    fn build_enum_variant(&self, mut pairs: Pairs<Rule>) -> Result<EnumVariant, ParseError> {
        let name = pairs
            .next()
            .ok_or_else(|| ParseError::InvalidSyntax {
                message: "Missing variant name".to_string(),
            })?
            .as_str()
            .to_string();

        let data = if let Some(pair) = pairs.next() {
            match pair.as_rule() {
                Rule::type_annotation => {
                    // Tuple-style variant: collect all type annotations
                    let mut types = vec![self.build_type_annotation(pair.into_inner())?];
                    for remaining_pair in pairs {
                        if remaining_pair.as_rule() == Rule::type_annotation {
                            types.push(self.build_type_annotation(remaining_pair.into_inner())?);
                        }
                    }
                    Some(types)
                }
                Rule::struct_field_list => {
                    // Struct-style variant: convert struct fields to type annotations
                    let fields = self.build_struct_field_list(pair.into_inner())?;
                    let types: Vec<TypeAnnotation> =
                        fields.into_iter().map(|f| f.field_type).collect();
                    Some(types)
                }
                _ => {
                    // Unknown rule - treat as no data
                    None
                }
            }
        } else {
            // No additional data - unit variant
            None
        };

        Ok(EnumVariant { name, data })
    }

    fn build_struct_literal(&self, mut pairs: Pairs<Rule>) -> Result<Expr, ParseError> {
        let type_name = pairs
            .next()
            .ok_or_else(|| ParseError::InvalidSyntax {
                message: "Missing struct type name".to_string(),
            })?
            .as_str()
            .to_string();

        let mut fields = Vec::new();
        for pair in pairs {
            if pair.as_rule() == Rule::field_value_list {
                for field_pair in pair.into_inner() {
                    if field_pair.as_rule() == Rule::field_value {
                        fields.push(self.build_field_value(field_pair.into_inner())?);
                    }
                }
            } else if pair.as_rule() == Rule::field_value {
                fields.push(self.build_field_value(pair.into_inner())?);
            }
        }

        Ok(Expr::StructLiteral(StructLiteral { type_name, fields }))
    }

    fn build_field_value(&self, mut pairs: Pairs<Rule>) -> Result<FieldValue, ParseError> {
        let name = pairs
            .next()
            .ok_or_else(|| ParseError::InvalidSyntax {
                message: "Missing field name".to_string(),
            })?
            .as_str()
            .to_string();

        let value = pairs.next().ok_or_else(|| ParseError::InvalidSyntax {
            message: "Missing field value".to_string(),
        })?;

        let expr = self.build_expr(value.into_inner())?;

        Ok(FieldValue { name, value: expr })
    }

    fn build_anonymous_object(&self, pairs: Pairs<Rule>) -> Result<Expr, ParseError> {
        let mut fields = Vec::new();
        for pair in pairs {
            if pair.as_rule() == Rule::field_value_list {
                for field_pair in pair.into_inner() {
                    if field_pair.as_rule() == Rule::field_value {
                        fields.push(self.build_field_value(field_pair.into_inner())?);
                    }
                }
            } else if pair.as_rule() == Rule::field_value {
                fields.push(self.build_field_value(pair.into_inner())?);
            }
        }

        Ok(Expr::AnonymousObject { fields })
    }

    fn build_map_literal(&self, pairs: Pairs<Rule>) -> Result<Expr, ParseError> {
        let mut entries = Vec::new();
        for pair in pairs {
            if pair.as_rule() == Rule::map_entry_list {
                for entry_pair in pair.into_inner() {
                    if entry_pair.as_rule() == Rule::map_entry {
                        entries.push(self.build_map_entry(entry_pair.into_inner())?);
                    }
                }
            } else if pair.as_rule() == Rule::map_entry {
                entries.push(self.build_map_entry(pair.into_inner())?);
            }
        }

        Ok(Expr::MapLiteral { entries })
    }

    fn build_map_entry(&self, mut pairs: Pairs<Rule>) -> Result<MapEntry, ParseError> {
        let key = pairs.next().ok_or_else(|| ParseError::InvalidSyntax {
            message: "Missing key in map entry".to_string(),
        })?;
        let value = pairs.next().ok_or_else(|| ParseError::InvalidSyntax {
            message: "Missing value in map entry".to_string(),
        })?;

        Ok(MapEntry {
            key: self.build_expr(key.into_inner())?,
            value: self.build_expr(value.into_inner())?,
        })
    }

    fn build_for_loop(&self, pairs: Pairs<Rule>, parallel: bool) -> Result<Expr, ParseError> {
        let mut pairs = pairs.filter(|p| !Self::is_kw_pair(p.as_rule())).peekable();

        // Expected order: binding (identifier or tuple of identifiers),
        // expr (iterable), block (body)
        let binding_pair = pairs.next().ok_or_else(|| ParseError::InvalidSyntax {
            message: "Missing loop variable".to_string(),
        })?;

        // A tuple binding desugars: `for (a, b) in e { body }` becomes a loop
        // over a hidden variable whose body first destructures it with
        // `let (a, b) = <hidden>`. Both tiers then see plain, existing forms.
        let tuple_names: Option<Vec<String>> = if binding_pair.as_rule() == Rule::for_binding_tuple
        {
            Some(
                binding_pair
                    .clone()
                    .into_inner()
                    .map(|p| p.as_str().to_string())
                    .collect(),
            )
        } else {
            None
        };
        let variable = if tuple_names.is_some() {
            "__for_item".to_string()
        } else {
            binding_pair.as_str().to_string()
        };

        let iterable_pair = pairs.next().ok_or_else(|| ParseError::InvalidSyntax {
            message: "Missing iterable expression in for loop".to_string(),
        })?;
        let iterable_expr = self.build_expr(iterable_pair.into_inner())?;

        let body_pair = pairs.next().ok_or_else(|| ParseError::InvalidSyntax {
            message: "Missing body in for loop".to_string(),
        })?;
        let body_expr = match body_pair.as_rule() {
            Rule::block => self.build_block(body_pair.into_inner())?,
            Rule::expr => self.build_expr(body_pair.into_inner())?,
            _ => {
                return Err(ParseError::InvalidSyntax {
                    message: "Invalid body in for loop".to_string(),
                });
            }
        };

        let body_expr = match tuple_names {
            None => body_expr,
            Some(names) => Expr::Block(vec![
                crate::ast::Statement::LetDecl(crate::ast::LetDecl {
                    name_span: None,
                    // `for (a, b) in ...` destructures the loop item: the
                    // names are loop bindings, immutable like the loop
                    // variable itself.
                    mutable: false,
                    pattern: crate::ast::Pattern::Tuple(
                        names
                            .into_iter()
                            .map(crate::ast::Pattern::Identifier)
                            .collect(),
                    ),
                    type_annotation: None,
                    value: Some(Expr::Identifier(variable.clone())),
                }),
                crate::ast::Statement::Expression(body_expr),
            ]),
        };

        Ok(if parallel {
            Expr::ParForLoop {
                variable,
                iterable: Box::new(iterable_expr),
                body: Box::new(body_expr),
            }
        } else {
            Expr::ForLoop {
                variable,
                iterable: Box::new(iterable_expr),
                body: Box::new(body_expr),
            }
        })
    }

    fn build_while_loop(&self, pairs: Pairs<Rule>) -> Result<Expr, ParseError> {
        let mut pairs = pairs.filter(|p| !Self::is_kw_pair(p.as_rule())).peekable();

        // Expected order: expr (condition), block (body)
        let cond_pair = pairs.next().ok_or_else(|| ParseError::InvalidSyntax {
            message: "Missing condition in while loop".to_string(),
        })?;
        let condition_expr = self.build_expr(cond_pair.into_inner())?;

        let body_pair = pairs.next().ok_or_else(|| ParseError::InvalidSyntax {
            message: "Missing body in while loop".to_string(),
        })?;
        let body_expr = match body_pair.as_rule() {
            Rule::block => self.build_block(body_pair.into_inner())?,
            Rule::expr => self.build_expr(body_pair.into_inner())?,
            _ => {
                return Err(ParseError::InvalidSyntax {
                    message: "Invalid body in while loop".to_string(),
                });
            }
        };

        Ok(Expr::WhileLoop {
            condition: Box::new(condition_expr),
            body: Box::new(body_expr),
        })
    }

    fn build_loop_expr(&self, pairs: Pairs<Rule>) -> Result<Expr, ParseError> {
        let mut pairs = pairs.filter(|p| !Self::is_kw_pair(p.as_rule())).peekable();

        // Expected: single block
        let body_pair = pairs.next().ok_or_else(|| ParseError::InvalidSyntax {
            message: "Missing body in loop expression".to_string(),
        })?;
        let body_expr = match body_pair.as_rule() {
            Rule::block => self.build_block(body_pair.into_inner())?,
            Rule::expr => self.build_expr(body_pair.into_inner())?,
            _ => {
                return Err(ParseError::InvalidSyntax {
                    message: "Invalid body in loop expression".to_string(),
                });
            }
        };

        Ok(Expr::Loop {
            body: Box::new(body_expr),
        })
    }

    fn build_assignment(&self, mut pairs: Pairs<Rule>) -> Result<Expr, ParseError> {
        let identifier = pairs.next().ok_or_else(|| ParseError::InvalidSyntax {
            message: "Missing identifier in assignment".to_string(),
        })?;
        let value = pairs.next().ok_or_else(|| ParseError::InvalidSyntax {
            message: "Missing value in assignment".to_string(),
        })?;

        Ok(Expr::Assignment {
            target: identifier.as_str().to_string(),
            value: Box::new(self.build_expr(value.into_inner())?),
        })
    }

    fn build_template_string(&self, pairs: Pairs<Rule>) -> Result<Expr, ParseError> {
        // Get the raw content between backticks
        let raw_content = pairs
            .into_iter()
            .find(|p| p.as_rule() == Rule::template_raw_content)
            .ok_or_else(|| ParseError::InvalidSyntax {
                message: "Missing template content".to_string(),
            })?
            .as_str();

        // Manually parse the template content to preserve all spaces
        self.parse_template_content(raw_content)
    }

    fn parse_template_content(&self, content: &str) -> Result<Expr, ParseError> {
        let mut parts = Vec::new();
        let mut current_literal = String::new();
        let mut chars = content.chars().peekable();

        while let Some(ch) = chars.next() {
            // The three template escapes: \` \$ \\ produce the bare
            // character (an escaped $ never starts an interpolation). Any
            // other backslash pair passes through untouched — `\n` stays
            // two characters, as template literals have always behaved.
            if ch == '\\' {
                match chars.peek() {
                    Some('`') | Some('$') | Some('\\') => {
                        current_literal.push(chars.next().expect("peeked"));
                    }
                    _ => current_literal.push('\\'),
                }
                continue;
            }
            if ch == '$' && chars.peek() == Some(&'{') {
                // Found interpolation start
                chars.next(); // consume '{'

                // Save any accumulated literal text
                if !current_literal.is_empty() {
                    parts.push(TemplatePart::Literal(current_literal.clone()));
                    current_literal.clear();
                }

                // Extract the expression inside ${}, ignoring braces that
                // appear inside string literals (e.g. `${ "}" }`)
                let mut expr_content = String::new();
                let mut brace_count = 1;
                let mut in_string = false;
                let mut prev_escape = false;

                for ch in chars.by_ref() {
                    if in_string {
                        if prev_escape {
                            prev_escape = false;
                        } else if ch == '\\' {
                            prev_escape = true;
                        } else if ch == '"' {
                            in_string = false;
                        }
                        expr_content.push(ch);
                        continue;
                    }
                    match ch {
                        '"' => in_string = true,
                        '{' => brace_count += 1,
                        '}' => {
                            brace_count -= 1;
                            if brace_count == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    expr_content.push(ch);
                }

                if brace_count > 0 {
                    return Err(ParseError::InvalidSyntax {
                        message: "Unclosed template interpolation — if the `${...}` \
                                  contains a backtick, note that templates do not \
                                  nest; bind the inner template to a name first"
                            .to_string(),
                    });
                }

                // Parse the expression content
                let expr = self.parse_expression_from_string(&expr_content)?;
                parts.push(TemplatePart::Interpolation(Box::new(expr)));
            } else {
                // Regular character - add to literal
                current_literal.push(ch);
            }
        }

        // Add any remaining literal text
        if !current_literal.is_empty() {
            parts.push(TemplatePart::Literal(current_literal));
        }

        Ok(Expr::TemplateString { parts })
    }

    /// The source spans of every template literal in `source`:
    /// `(line, column, raw_content)`, both 1-based, positioned at the
    /// first character after the opening backtick. Check-time lints need
    /// the source *spelling* — the parsed AST cannot distinguish `\n`
    /// from the deliberate `\\n` (both yield the same literal) — and the
    /// grammar keeps `template_raw_content` findable for exactly this.
    /// A source that does not parse yields no spans.
    pub fn template_literal_spans(source: &str) -> Vec<(u32, u32, String)> {
        // Shebang lines are masked with spaces exactly as `parse` does,
        // keeping every position identical to the file on disk.
        let masked;
        let source = if source.starts_with("#!") {
            let line_end = source.find('\n').unwrap_or(source.len());
            masked = format!("{}{}", " ".repeat(line_end), &source[line_end..]);
            masked.as_str()
        } else {
            source
        };
        let Ok(pairs) = <OlangParser as PestParser<Rule>>::parse(Rule::program, source) else {
            return Vec::new();
        };
        pairs
            .flatten()
            .filter(|p| p.as_rule() == Rule::template_raw_content)
            .map(|p| {
                let (line, col) = p.as_span().start_pos().line_col();
                (line as u32, col as u32, p.as_str().to_string())
            })
            .collect()
    }

    fn parse_expression_from_string(&self, expr_str: &str) -> Result<Expr, ParseError> {
        // Use Pest to parse just the expression
        let trimmed = expr_str.trim();
        let pairs = OlangParser::parse(Rule::expr, trimmed).map_err(ParseError::Pest)?;

        let expr_pair = pairs
            .into_iter()
            .next()
            .ok_or_else(|| ParseError::InvalidSyntax {
                message: "Empty expression in template interpolation".to_string(),
            })?;

        // The parse isn't anchored with EOI, so reject leftover input instead
        // of silently discarding it (`${1 + }` used to parse as just `1`)
        if expr_pair.as_span().end() != trimmed.len() {
            return Err(ParseError::InvalidSyntax {
                message: format!(
                    "Invalid expression in template interpolation: '{}'",
                    trimmed
                ),
            });
        }

        self.build_expr(expr_pair.into_inner())
    }

    /// Strip exactly one pair of surrounding quotes and process escapes.
    /// (`trim_matches('"')` also eats an escaped closing quote, and skipping
    /// escape processing made `"\n"` patterns match a literal backslash-n.)
    fn unquote_string(&self, s: &str) -> Result<String, ParseError> {
        if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
            self.process_string_escapes(&s[1..s.len() - 1])
        } else {
            Err(ParseError::InvalidSyntax {
                message: "Malformed string literal".to_string(),
            })
        }
    }

    fn process_string_escapes(&self, input: &str) -> Result<String, ParseError> {
        let mut result = String::new();
        let mut chars = input.chars().peekable();

        while let Some(ch) = chars.next() {
            if ch == '\\' {
                match chars.next() {
                    Some('"') => result.push('"'),
                    Some('\\') => result.push('\\'),
                    Some('/') => result.push('/'),
                    Some('b') => result.push('\u{0008}'), // backspace
                    Some('f') => result.push('\u{000C}'), // form feed
                    Some('n') => result.push('\n'),
                    Some('r') => result.push('\r'),
                    Some('t') => result.push('\t'),
                    Some('0') => result.push('\0'), // null character
                    Some('x') => {
                        // Hex escape sequence \xHH
                        let mut hex_digits = String::new();
                        for _ in 0..2 {
                            match chars.next() {
                                Some(digit) if digit.is_ascii_hexdigit() => hex_digits.push(digit),
                                _ => {
                                    return Err(ParseError::InvalidSyntax {
                                        message:
                                            "Invalid hex escape sequence: expected 2 hex digits"
                                                .to_string(),
                                    });
                                }
                            }
                        }
                        if let Ok(byte_value) = u8::from_str_radix(&hex_digits, 16) {
                            result.push(byte_value as char);
                        } else {
                            return Err(ParseError::InvalidSyntax {
                                message: "Invalid hex escape sequence".to_string(),
                            });
                        }
                    }
                    Some('u') => {
                        // Check for variable-length Unicode escape \u{H+}
                        if chars.peek() == Some(&'{') {
                            chars.next(); // consume '{'
                            let mut unicode_digits = String::new();
                            let mut brace_count = 1;

                            for ch in chars.by_ref() {
                                if ch == '{' {
                                    brace_count += 1;
                                } else if ch == '}' {
                                    brace_count -= 1;
                                    if brace_count == 0 {
                                        break;
                                    }
                                } else if ch.is_ascii_hexdigit() {
                                    unicode_digits.push(ch);
                                } else {
                                    return Err(ParseError::InvalidSyntax {
                                        message: "Invalid character in Unicode escape sequence"
                                            .to_string(),
                                    });
                                }
                            }

                            if brace_count != 0 {
                                return Err(ParseError::InvalidSyntax {
                                    message: "Unterminated Unicode escape sequence".to_string(),
                                });
                            }

                            if unicode_digits.is_empty() {
                                return Err(ParseError::InvalidSyntax {
                                    message: "Empty Unicode escape sequence".to_string(),
                                });
                            }

                            if let Ok(code_point) = u32::from_str_radix(&unicode_digits, 16) {
                                if let Some(unicode_char) = char::from_u32(code_point) {
                                    result.push(unicode_char);
                                } else {
                                    return Err(ParseError::InvalidSyntax {
                                        message: "Invalid unicode code point".to_string(),
                                    });
                                }
                            } else {
                                return Err(ParseError::InvalidSyntax {
                                    message: "Invalid Unicode escape sequence".to_string(),
                                });
                            }
                        } else {
                            // Fixed-length Unicode escape sequence \uXXXX
                            let mut unicode_digits = String::new();
                            for _ in 0..4 {
                                match chars.next() {
                                    Some(digit) if digit.is_ascii_hexdigit() => {
                                        unicode_digits.push(digit)
                                    }
                                    _ => return Err(ParseError::InvalidSyntax {
                                        message:
                                            "Invalid unicode escape sequence: expected 4 hex digits"
                                                .to_string(),
                                    }),
                                }
                            }
                            if let Ok(code_point) = u32::from_str_radix(&unicode_digits, 16) {
                                if let Some(unicode_char) = char::from_u32(code_point) {
                                    result.push(unicode_char);
                                } else {
                                    return Err(ParseError::InvalidSyntax {
                                        message: "Invalid unicode code point".to_string(),
                                    });
                                }
                            } else {
                                return Err(ParseError::InvalidSyntax {
                                    message: "Invalid unicode escape sequence".to_string(),
                                });
                            }
                        }
                    }
                    Some(other) => {
                        return Err(ParseError::InvalidSyntax {
                            message: format!("Invalid escape sequence: \\{}", other),
                        });
                    }
                    None => {
                        return Err(ParseError::InvalidSyntax {
                            message: "Incomplete escape sequence".to_string(),
                        });
                    }
                }
            } else {
                result.push(ch);
            }
        }

        Ok(result)
    }

    fn build_share_decl(&self, pairs: Pairs<Rule>) -> Result<ShareDecl, ParseError> {
        let mut pairs = pairs.filter(|p| !Self::is_kw_pair(p.as_rule())).peekable();

        let inner_pair = pairs.next().ok_or_else(|| ParseError::InvalidSyntax {
            message: "Missing declaration in share".to_string(),
        })?;

        match inner_pair.as_rule() {
            Rule::function_decl => Ok(ShareDecl::Function(
                self.build_function_decl(inner_pair.into_inner())?,
            )),
            Rule::let_decl => Ok(ShareDecl::Let(
                self.build_let_decl(inner_pair.into_inner())?,
            )),
            Rule::type_decl => Ok(ShareDecl::Type(
                self.build_type_decl(inner_pair.into_inner())?,
            )),
            Rule::use_decl => Ok(ShareDecl::Use(
                self.build_use_decl(inner_pair.into_inner())?,
            )),
            Rule::trait_decl => Ok(ShareDecl::Trait(
                self.build_trait_decl(inner_pair.into_inner())?,
            )),
            Rule::impl_decl => Ok(ShareDecl::Impl(
                self.build_impl_decl(inner_pair.into_inner())?,
            )),
            _ => Err(ParseError::InvalidSyntax {
                message: "Invalid declaration in share".to_string(),
            }),
        }
    }

    fn build_use_decl(&self, pairs: Pairs<Rule>) -> Result<UseDecl, ParseError> {
        let mut pairs = pairs.filter(|p| !Self::is_kw_pair(p.as_rule())).peekable();

        let path_pair = pairs.next().ok_or_else(|| ParseError::InvalidSyntax {
            message: "Missing path in use".to_string(),
        })?;
        let mut path = Vec::new();
        for part in path_pair.into_inner() {
            if part.as_rule() == Rule::identifier {
                path.push(part.as_str().to_string());
            }
        }

        // No `{ ... }` means "import everything the package shares" — a bare
        // `use foo` is sugar for `use foo { * }`.
        let list_pair = match pairs.next() {
            Some(p) => p,
            None => {
                return Ok(UseDecl {
                    path,
                    items: vec![crate::ast::UseItem::Wildcard],
                });
            }
        };
        let mut items = Vec::new();
        for item in list_pair.into_inner() {
            if item.as_rule() == Rule::use_item {
                let inner = item.into_inner().next().unwrap();
                match inner.as_rule() {
                    Rule::identifier => {
                        items.push(crate::ast::UseItem::Specific(inner.as_str().to_string()));
                    }
                    Rule::use_alias => {
                        let mut ids = inner
                            .into_inner()
                            .filter(|p| p.as_rule() == Rule::identifier);
                        let name = ids.next().unwrap().as_str().to_string();
                        let alias = ids.next().unwrap().as_str().to_string();
                        items.push(crate::ast::UseItem::Aliased { name, alias });
                    }
                    Rule::wildcard => {
                        items.push(crate::ast::UseItem::Wildcard);
                    }
                    _ => {
                        return Err(ParseError::InvalidSyntax {
                            message: format!("Invalid use item: {:?}", inner.as_rule()),
                        });
                    }
                }
            }
        }

        Ok(UseDecl { path, items })
    }

    fn build_test_decl(&self, pairs: Pairs<Rule>) -> Result<TestDecl, ParseError> {
        let mut pairs = pairs.filter(|p| !Self::is_kw_pair(p.as_rule())).peekable();

        // Get test name from string literal
        let name_pair = pairs.next().ok_or_else(|| ParseError::InvalidSyntax {
            message: "Missing test name".to_string(),
        })?;
        let name = self.unquote_string(name_pair.as_str())?;

        // Get test block with statements
        let block_pair = pairs.next().ok_or_else(|| ParseError::InvalidSyntax {
            message: "Missing test block".to_string(),
        })?;

        let mut body = Vec::new();
        for statement_pair in block_pair.into_inner() {
            if statement_pair.as_rule() == Rule::test_statement {
                // `build_statement` handles assertions like any other
                // statement form now, so there is nothing special left to
                // do at a test block's top level.
                let inner = statement_pair.into_inner().next().unwrap();
                body.push(self.build_statement(inner)?);
            }
        }

        Ok(TestDecl { name, body })
    }

    fn build_assertion(&self, mut pairs: Pairs<Rule>) -> Result<Expr, ParseError> {
        let assertion_pair = pairs.next().ok_or_else(|| ParseError::InvalidSyntax {
            message: "Missing assertion type".to_string(),
        })?;

        match assertion_pair.as_rule() {
            Rule::assert_eq => {
                let mut inner = assertion_pair.into_inner();
                let actual = self.build_expr(inner.next().unwrap().into_inner())?;
                let expected = self.build_expr(inner.next().unwrap().into_inner())?;
                let message = if let Some(msg_pair) = inner.next() {
                    Some(self.unquote_string(msg_pair.as_str())?)
                } else {
                    None
                };
                Ok(Expr::AssertEq {
                    actual: Box::new(actual),
                    expected: Box::new(expected),
                    message,
                })
            }
            Rule::assert_ne => {
                let mut inner = assertion_pair.into_inner();
                let actual = self.build_expr(inner.next().unwrap().into_inner())?;
                let expected = self.build_expr(inner.next().unwrap().into_inner())?;
                let message = if let Some(msg_pair) = inner.next() {
                    Some(self.unquote_string(msg_pair.as_str())?)
                } else {
                    None
                };
                Ok(Expr::AssertNe {
                    actual: Box::new(actual),
                    expected: Box::new(expected),
                    message,
                })
            }
            Rule::assert => {
                let mut inner = assertion_pair.into_inner();
                let condition = self.build_expr(inner.next().unwrap().into_inner())?;
                let message = if let Some(msg_pair) = inner.next() {
                    Some(self.unquote_string(msg_pair.as_str())?)
                } else {
                    None
                };
                Ok(Expr::Assert {
                    condition: Box::new(condition),
                    message,
                })
            }
            Rule::assert_true => {
                let mut inner = assertion_pair.into_inner();
                let expression = self.build_expr(inner.next().unwrap().into_inner())?;
                let message = if let Some(msg_pair) = inner.next() {
                    Some(self.unquote_string(msg_pair.as_str())?)
                } else {
                    None
                };
                Ok(Expr::AssertTrue {
                    expression: Box::new(expression),
                    message,
                })
            }
            Rule::assert_false => {
                let mut inner = assertion_pair.into_inner();
                let expression = self.build_expr(inner.next().unwrap().into_inner())?;
                let message = if let Some(msg_pair) = inner.next() {
                    Some(self.unquote_string(msg_pair.as_str())?)
                } else {
                    None
                };
                Ok(Expr::AssertFalse {
                    expression: Box::new(expression),
                    message,
                })
            }
            _ => Err(ParseError::invalid_syntax_at(
                format!("Unknown assertion type: {:?}", assertion_pair.as_rule()),
                PositionInfo::from_pair(&assertion_pair),
            )),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ErrorSuggestion {
    pub message: String,
    pub fix: Option<String>,
    pub help: Option<String>,
    pub severity: SuggestionSeverity,
}

#[derive(Debug, Clone)]
pub enum SuggestionSeverity {
    Error,
    Warning,
    Hint,
    Info,
}

pub struct ErrorSuggestionEngine {
    // Common patterns and their suggestions
}

impl Default for ErrorSuggestionEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl ErrorSuggestionEngine {
    pub fn new() -> Self {
        Self {}
    }

    pub fn suggest_for_parse_error(&self, error: &ParseError, input: &str) -> Vec<ErrorSuggestion> {
        match error {
            ParseError::Pest(pest_error) => self.suggest_for_pest_error(pest_error, input),
            ParseError::InvalidSyntaxWithPosition {
                message,
                line,
                column,
                ..
            } => self.suggest_for_invalid_syntax(message, *line, *column, input),
            ParseError::UnexpectedTokenWithPosition {
                token,
                line,
                column,
                ..
            } => self.suggest_for_unexpected_token(token, *line, *column, input),
            ParseError::InvalidSyntax { message } => {
                self.suggest_for_generic_syntax_error(message, input)
            }
            ParseError::UnexpectedToken { token } => {
                self.suggest_for_generic_token_error(token, input)
            }
        }
    }

    fn suggest_for_pest_error(
        &self,
        pest_error: &pest::error::Error<Rule>,
        _input: &str,
    ) -> Vec<ErrorSuggestion> {
        let mut suggestions = Vec::new();

        // Analyze the pest error for common patterns
        let error_msg = format!("{}", pest_error);

        Self::suggest_let_mut_typo(pest_error.line(), &mut suggestions);

        // Check for common syntax issues
        if error_msg.contains("expected") {
            if error_msg.contains("expected `)`") {
                suggestions.push(ErrorSuggestion {
                    message: "Missing closing parenthesis".to_string(),
                    fix: Some("Add `)` to close the parenthesis".to_string()),
                    help: Some(
                        "Check that all opening parentheses have matching closing ones".to_string(),
                    ),
                    severity: SuggestionSeverity::Error,
                });
            }

            if error_msg.contains("expected `}`") {
                suggestions.push(ErrorSuggestion {
                    message: "Missing closing brace".to_string(),
                    fix: Some("Add `}` to close the brace".to_string()),
                    help: Some(
                        "Check that all opening braces have matching closing ones".to_string(),
                    ),
                    severity: SuggestionSeverity::Error,
                });
            }

            if error_msg.contains("expected `]`") {
                suggestions.push(ErrorSuggestion {
                    message: "Missing closing bracket".to_string(),
                    fix: Some("Add `]` to close the bracket".to_string()),
                    help: Some(
                        "Check that all opening brackets have matching closing ones".to_string(),
                    ),
                    severity: SuggestionSeverity::Error,
                });
            }

            if error_msg.contains("expected `\"`") {
                suggestions.push(ErrorSuggestion {
                    message: "Missing closing quote".to_string(),
                    fix: Some("Add `\"` to close the string".to_string()),
                    help: Some("String literals must be enclosed in double quotes".to_string()),
                    severity: SuggestionSeverity::Error,
                });
            }
        }

        // Check for function-related errors
        if error_msg.contains("fn") {
            suggestions.push(ErrorSuggestion {
                message: "Function declaration syntax error".to_string(),
                fix: Some(
                    "Check function syntax: `fn name(params) = body` or `fn name(params) { body }`"
                        .to_string(),
                ),
                help: Some(
                    "Functions can be declared with expression bodies (=) or block bodies ({ })"
                        .to_string(),
                ),
                severity: SuggestionSeverity::Hint,
            });
        }

        // Check for let declaration errors
        if error_msg.contains("let") {
            suggestions.push(ErrorSuggestion {
                message: "Let declaration syntax error".to_string(),
                fix: Some(
                    "Check let syntax: `let name = value` or `let pattern = value`".to_string(),
                ),
                help: Some(
                    "Let declarations support pattern matching and type annotations".to_string(),
                ),
                severity: SuggestionSeverity::Hint,
            });
        }

        suggestions
    }

    /// `let m f = ...` — a mistyped `mut` reads as a binding name
    /// followed by a stray identifier, and the generic "expected the end
    /// of the file" points at the wrong problem. Recognize the shape on
    /// the offending line and name the likely fix.
    fn suggest_let_mut_typo(line: &str, suggestions: &mut Vec<ErrorSuggestion>) {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("let ") {
            let mut words = rest.split_whitespace();
            if let (Some(first), Some(second)) = (words.next(), words.next())
                && first != "mut"
                // An annotated binding (`let xs: List<Int> = ...`) is not
                // a mistyped `mut`; the second word is its type.
                && !first.ends_with(':')
                && !second.starts_with(':')
                && second != "="
                && !second.starts_with('=')
                && second
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_alphabetic() || c == '_')
            {
                suggestions.push(ErrorSuggestion {
                    message: format!(
                        "`let {} {}` binds `{}` and then trips on `{}`",
                        first, second, first, second
                    ),
                    fix: Some(format!("Did you mean `let mut {}`?", second)),
                    help: Some(
                        "A binding takes one name; `let mut name = ...` declares it reassignable"
                            .to_string(),
                    ),
                    severity: SuggestionSeverity::Error,
                });
            }
        }
    }

    /// The innermost delimiter still open at the error position, with the
    /// line/column where it was opened. String literals (with escapes) and
    /// `//` comments are skipped so a bracket inside either never counts.
    fn unclosed_opener_before(
        input: &str,
        err_line: usize,
        err_column: usize,
    ) -> Option<(char, usize, usize)> {
        let mut stack: Vec<(char, usize, usize)> = Vec::new();
        let mut line = 1usize;
        let mut col = 1usize;
        let mut chars = input.chars().peekable();
        while let Some(c) = chars.next() {
            if line > err_line || (line == err_line && col >= err_column) {
                break;
            }
            match c {
                '\n' => {
                    line += 1;
                    col = 1;
                    continue;
                }
                '"' => {
                    // consume the string body, honoring escapes
                    col += 1;
                    while let Some(&s) = chars.peek() {
                        chars.next();
                        col += 1;
                        match s {
                            '\\' => {
                                if chars.next().is_some() {
                                    col += 1;
                                }
                            }
                            '"' => break,
                            '\n' => {
                                line += 1;
                                col = 1;
                            }
                            _ => {}
                        }
                    }
                    continue;
                }
                '/' if chars.peek() == Some(&'/') => {
                    // comment: skip to end of line
                    while let Some(&s) = chars.peek() {
                        if s == '\n' {
                            break;
                        }
                        chars.next();
                        col += 1;
                    }
                    col += 1;
                    continue;
                }
                '(' | '[' | '{' => stack.push((c, line, col)),
                ')' | ']' | '}' => {
                    let wants = match c {
                        ')' => '(',
                        ']' => '[',
                        _ => '{',
                    };
                    if stack.last().map(|(o, _, _)| *o) == Some(wants) {
                        stack.pop();
                    }
                }
                _ => {}
            }
            col += 1;
        }
        stack.pop()
    }

    /// When the parse position sits under an unclosed delimiter, say WHERE
    /// it was opened — "expected an operator" three lines later teaches
    /// nothing on its own.
    fn suggest_unclosed_opener(
        line: usize,
        column: usize,
        input: &str,
        suggestions: &mut Vec<ErrorSuggestion>,
    ) -> bool {
        if let Some((opener, oline, ocol)) = Self::unclosed_opener_before(input, line, column) {
            let closer = match opener {
                '(' => ')',
                '[' => ']',
                _ => '}',
            };
            suggestions.push(ErrorSuggestion {
                message: format!(
                    "Unclosed `{}` opened at line {}, column {}",
                    opener, oline, ocol
                ),
                fix: Some(format!("Add the matching `{}`", closer)),
                help: Some(format!(
                    "Everything after that `{}` is still inside it — the parser \
                     trips where the missing `{}` first matters, not where it \
                     belongs",
                    opener, closer
                )),
                severity: SuggestionSeverity::Error,
            });
            return true;
        }
        false
    }

    fn suggest_for_invalid_syntax(
        &self,
        message: &str,
        line: usize,
        column: usize,
        input: &str,
    ) -> Vec<ErrorSuggestion> {
        let mut suggestions = Vec::new();

        // Get the line content for analysis
        let lines: Vec<&str> = input.lines().collect();
        let line_content = if line > 0 && line <= lines.len() {
            lines[line - 1]
        } else {
            ""
        };

        Self::suggest_let_mut_typo(line_content, &mut suggestions);
        Self::suggest_unclosed_opener(line, column, input, &mut suggestions);

        // Check for common typos and mistakes
        if message.contains("integer") {
            suggestions.push(ErrorSuggestion {
                message: "Invalid integer format".to_string(),
                fix: Some(
                    "Use decimal (123), binary (0b1010), octal (0o123), or hex (0xFF) format"
                        .to_string(),
                ),
                help: Some(
                    "Integer literals support underscores for readability: 1_000_000".to_string(),
                ),
                severity: SuggestionSeverity::Error,
            });
        }

        if message.contains("float") {
            suggestions.push(ErrorSuggestion {
                message: "Invalid float format".to_string(),
                fix: Some(
                    "Use decimal point format: 3.14, 2.0, or scientific notation: 1e10".to_string(),
                ),
                help: Some(
                    "Float literals require a decimal point or scientific notation".to_string(),
                ),
                severity: SuggestionSeverity::Error,
            });
        }

        if message.contains("string") {
            suggestions.push(ErrorSuggestion {
                message: "Invalid string format".to_string(),
                fix: Some("Enclose strings in double quotes: \"hello world\"".to_string()),
                help: Some(
                    "Use raw strings for literals with backslashes: r\"C:\\path\\to\\file\""
                        .to_string(),
                ),
                severity: SuggestionSeverity::Error,
            });
        }

        // C-style `if cond { ... }` — the single most common habit from
        // other languages. Only `if`/`else` need the arrow; `while` and
        // `for` take braces directly, so the check is scoped to those two.
        let trimmed = line_content.trim_start();
        if (trimmed.starts_with("if ")
            || trimmed.starts_with("} else")
            || trimmed.starts_with("else"))
            && !line_content.contains("=>")
            && line_content.trim_end().ends_with('{')
        {
            let fixed = format!("{}=> {{", line_content.trim_end().trim_end_matches('{'));
            suggestions.push(ErrorSuggestion {
                message: "`if` takes an arrow before its branch".to_string(),
                fix: Some(format!("Write `{}`", fixed.trim_start())),
                help: Some(
                    "olang branches are `if cond => expr else => expr`; a block is an \
                     expression, so `if cond => { ... }` works too. Only `if`/`else` \
                     use `=>` — `while` and `for` take braces directly"
                        .to_string(),
                ),
                severity: SuggestionSeverity::Error,
            });
            return suggestions;
        }

        // Check for bracket mismatches
        if self.has_unmatched_brackets(line_content) {
            suggestions.push(ErrorSuggestion {
                message: "Unmatched brackets detected".to_string(),
                fix: Some(
                    "Check that all brackets, parentheses, and braces are properly matched"
                        .to_string(),
                ),
                help: Some(
                    "Use an editor with bracket matching to help identify issues".to_string(),
                ),
                severity: SuggestionSeverity::Error,
            });
        }

        // Check for assignment vs equality
        if line_content.contains("=")
            && !line_content.contains("==")
            && !line_content.contains("let")
        {
            suggestions.push(ErrorSuggestion {
                message: "Possible assignment in expression context".to_string(),
                fix: Some(
                    "Use `==` for equality comparison, `=` only for let declarations".to_string(),
                ),
                help: Some(
                    "Assignments are only allowed in let declarations and function parameters"
                        .to_string(),
                ),
                severity: SuggestionSeverity::Hint,
            });
        }

        // Check character at column position for specific suggestions
        if column > 0 && column <= line_content.len() {
            let char_at_pos = line_content.chars().nth(column - 1);
            if let Some(ch) = char_at_pos
                && ch == ';'
            {
                suggestions.push(ErrorSuggestion {
                    message: "Semicolons are not used in Olang".to_string(),
                    fix: Some("Remove the semicolon".to_string()),
                    help: Some("Olang uses newlines and expression-based syntax".to_string()),
                    severity: SuggestionSeverity::Error,
                });
            }
        }

        suggestions
    }

    fn suggest_for_unexpected_token(
        &self,
        token: &str,
        line: usize,
        column: usize,
        input: &str,
    ) -> Vec<ErrorSuggestion> {
        let mut suggestions = Vec::new();

        Self::suggest_unclosed_opener(line, column, input, &mut suggestions);

        // Common token-specific suggestions
        match token {
            ";" => {
                suggestions.push(ErrorSuggestion {
                    message: "Semicolons are not used in Olang".to_string(),
                    fix: Some("Remove the semicolon".to_string()),
                    help: Some(
                        "Olang uses newlines and expression-based syntax, not semicolons"
                            .to_string(),
                    ),
                    severity: SuggestionSeverity::Error,
                });
            }
            "=" => {
                suggestions.push(ErrorSuggestion {
                    message: "Unexpected assignment operator".to_string(),
                    fix: Some(
                        "Use `==` for comparison or `let` for variable declarations".to_string(),
                    ),
                    help: Some(
                        "Single `=` is only used in let declarations and function parameters"
                            .to_string(),
                    ),
                    severity: SuggestionSeverity::Error,
                });
            }
            ")" | "}" | "]" => {
                suggestions.push(ErrorSuggestion {
                    message: "Unexpected closing bracket".to_string(),
                    fix: Some(
                        "Check for missing opening bracket or extra closing bracket".to_string(),
                    ),
                    help: Some("Make sure all brackets are properly paired".to_string()),
                    severity: SuggestionSeverity::Error,
                });
            }
            _ => {
                // Generic token suggestions
                suggestions.push(ErrorSuggestion {
                    message: format!("Unexpected token: {}", token),
                    fix: Some("Check the syntax around this token".to_string()),
                    help: Some("Refer to the language documentation for valid syntax".to_string()),
                    severity: SuggestionSeverity::Error,
                });
            }
        }

        suggestions
    }

    fn suggest_for_generic_syntax_error(
        &self,
        _message: &str,
        input: &str,
    ) -> Vec<ErrorSuggestion> {
        let mut suggestions = Vec::new();

        // Analyze the input for common patterns
        if input.contains("console.log") {
            suggestions.push(ErrorSuggestion {
                message: "JavaScript-style console.log detected".to_string(),
                fix: Some("Use `println(...)` instead of `console.log(...)`".to_string()),
                help: Some("Olang uses `println` for output, not `console.log`".to_string()),
                severity: SuggestionSeverity::Hint,
            });
        }

        if input.contains("printf") {
            suggestions.push(ErrorSuggestion {
                message: "C-style printf detected".to_string(),
                fix: Some(
                    "Use `println(...)` or string interpolation `\"Hello {name}\"`".to_string(),
                ),
                help: Some(
                    "Olang uses `println` and string interpolation instead of printf".to_string(),
                ),
                severity: SuggestionSeverity::Hint,
            });
        }

        if input.contains("print(") && !input.contains("println(") {
            suggestions.push(ErrorSuggestion {
                message: "Use println for output".to_string(),
                fix: Some("Use `println(...)` instead of `print(...)`".to_string()),
                help: Some("Olang's print function is called `println`".to_string()),
                severity: SuggestionSeverity::Hint,
            });
        }

        suggestions
    }

    fn suggest_for_generic_token_error(&self, token: &str, _input: &str) -> Vec<ErrorSuggestion> {
        // Similar to suggest_for_unexpected_token but without position info
        vec![ErrorSuggestion {
            message: format!("Unexpected token: {}", token),
            fix: Some("Check the syntax around this token".to_string()),
            help: Some("Refer to the language documentation for valid syntax".to_string()),
            severity: SuggestionSeverity::Error,
        }]
    }

    fn has_unmatched_brackets(&self, line: &str) -> bool {
        let mut paren_count = 0;
        let mut brace_count = 0;
        let mut bracket_count = 0;
        let mut in_string = false;
        let mut escape_next = false;

        for ch in line.chars() {
            if escape_next {
                escape_next = false;
                continue;
            }

            match ch {
                '"' if !in_string => in_string = true,
                '"' if in_string => in_string = false,
                '\\' if in_string => escape_next = true,
                '(' if !in_string => paren_count += 1,
                ')' if !in_string => paren_count -= 1,
                '{' if !in_string => brace_count += 1,
                '}' if !in_string => brace_count -= 1,
                '[' if !in_string => bracket_count += 1,
                ']' if !in_string => bracket_count -= 1,
                _ => {}
            }
        }

        paren_count != 0 || brace_count != 0 || bracket_count != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_enhanced_string_escapes() {
        let parser = Parser::new();

        // Test basic string parsing
        let input = r#""Hello\nWorld""#;
        let result = parser.parse(input);
        assert!(
            result.is_ok(),
            "Failed to parse string with escape sequence: {:?}",
            result
        );

        // Test unicode escapes
        let input = r#""Hello\u0041""#;
        let result = parser.parse(input);
        assert!(
            result.is_ok(),
            "Failed to parse string with unicode escape: {:?}",
            result
        );

        // Test escaped quotes
        let input = r#""He said \"Hello\"""#;
        let result = parser.parse(input);
        assert!(
            result.is_ok(),
            "Failed to parse string with escaped quotes: {:?}",
            result
        );
    }

    #[test]
    fn test_named_arguments_parsing() {
        let parser = Parser::new();

        // Test function call with named arguments
        let input = "greet(name: \"Alice\", age: 25)";
        let result = parser.parse(input);
        assert!(
            result.is_ok(),
            "Failed to parse named arguments: {:?}",
            result
        );

        if let Ok(program) = result {
            assert_eq!(program.statements.len(), 1);
            if let Statement::Expression(Expr::Call { arguments, .. }) =
                program.statements[0].unwrapped()
            {
                assert_eq!(arguments.len(), 2);

                // Check first argument is named
                if let Argument::Named { name, .. } = &arguments[0] {
                    assert_eq!(name, "name");
                } else {
                    panic!("Expected named argument, got: {:?}", arguments[0]);
                }

                // Check second argument is named
                if let Argument::Named { name, .. } = &arguments[1] {
                    assert_eq!(name, "age");
                } else {
                    panic!("Expected named argument, got: {:?}", arguments[1]);
                }
            } else {
                panic!("Expected function call, got: {:?}", program.statements[0]);
            }
        }
    }

    #[test]
    fn test_mixed_arguments_parsing() {
        let parser = Parser::new();

        // Test function call with mixed positional and named arguments
        let input = "connect(\"localhost\", port: 8080, timeout: 30)";
        let result = parser.parse(input);
        assert!(
            result.is_ok(),
            "Failed to parse mixed arguments: {:?}",
            result
        );

        if let Ok(program) = result {
            assert_eq!(program.statements.len(), 1);
            if let Statement::Expression(Expr::Call { arguments, .. }) =
                program.statements[0].unwrapped()
            {
                assert_eq!(arguments.len(), 3);

                // Check first argument is positional
                assert!(matches!(arguments[0], Argument::Positional(_)));

                // Check second argument is named
                if let Argument::Named { name, .. } = &arguments[1] {
                    assert_eq!(name, "port");
                } else {
                    panic!("Expected named argument, got: {:?}", arguments[1]);
                }

                // Check third argument is named
                if let Argument::Named { name, .. } = &arguments[2] {
                    assert_eq!(name, "timeout");
                } else {
                    panic!("Expected named argument, got: {:?}", arguments[2]);
                }
            } else {
                panic!("Expected function call, got: {:?}", program.statements[0]);
            }
        }
    }

    #[test]
    fn test_positional_arguments_still_work() {
        let parser = Parser::new();

        // Test function call with only positional arguments
        let input = "add(1, 2, 3)";
        let result = parser.parse(input);
        assert!(
            result.is_ok(),
            "Failed to parse positional arguments: {:?}",
            result
        );

        if let Ok(program) = result {
            assert_eq!(program.statements.len(), 1);
            if let Statement::Expression(Expr::Call { arguments, .. }) =
                program.statements[0].unwrapped()
            {
                assert_eq!(arguments.len(), 3);

                // All arguments should be positional
                for arg in arguments {
                    assert!(matches!(arg, Argument::Positional(_)));
                }
            } else {
                panic!("Expected function call, got: {:?}", program.statements[0]);
            }
        }
    }

    #[test]
    fn test_complex_named_argument_values() {
        let parser = Parser::new();

        // Test named arguments with complex expressions
        let input = "process(data: [1, 2, 3], transform: (x) => x * 2, config: {debug: true})";
        let result = parser.parse(input);
        assert!(
            result.is_ok(),
            "Failed to parse complex named arguments: {:?}",
            result
        );

        if let Ok(program) = result {
            assert_eq!(program.statements.len(), 1);
            if let Statement::Expression(Expr::Call { arguments, .. }) =
                program.statements[0].unwrapped()
            {
                assert_eq!(arguments.len(), 3);

                // Check all arguments are named with proper names
                let names: Vec<&str> = arguments
                    .iter()
                    .map(|arg| match arg {
                        Argument::Named { name, .. } => name.as_str(),
                        _ => panic!("Expected named argument"),
                    })
                    .collect();

                assert_eq!(names, vec!["data", "transform", "config"]);
            } else {
                panic!("Expected function call, got: {:?}", program.statements[0]);
            }
        }
    }

    #[test]
    fn test_nested_function_calls_with_named_args() {
        let parser = Parser::new();

        // Test nested function calls with named arguments
        let input = "outer(inner(value: 42), name: \"test\")";
        let result = parser.parse(input);
        assert!(
            result.is_ok(),
            "Failed to parse nested calls with named arguments: {:?}",
            result
        );

        if let Ok(program) = result {
            assert_eq!(program.statements.len(), 1);
            if let Statement::Expression(Expr::Call { arguments, .. }) =
                program.statements[0].unwrapped()
            {
                assert_eq!(arguments.len(), 2);

                // First argument should be positional (the inner call)
                assert!(matches!(arguments[0], Argument::Positional(_)));

                // Second argument should be named
                if let Argument::Named { name, .. } = &arguments[1] {
                    assert_eq!(name, "name");
                } else {
                    panic!("Expected named argument, got: {:?}", arguments[1]);
                }
            } else {
                panic!("Expected function call, got: {:?}", program.statements[0]);
            }
        }
    }

    // Regression: deeply nested parentheses once triggered catastrophic
    // exponential-time backtracking because grouping `(e)` and the `tuple`
    // literal were two separate `(`-prefixed alternatives in `primary`. A plain
    // `((((…))))` made the tuple branch parse the inner expression, fail at the
    // missing comma, then the grouping branch re-parse it — doubling the work at
    // every nesting level, so ~25 levels hung indefinitely. `paren_expr` now
    // parses the inner expression exactly once. Parsing must be roughly linear;
    // a generous wall-clock bound turns any regression back into a hang into a
    // failing test rather than a stuck suite.
    #[test]
    fn test_deeply_nested_parens_parse_in_linear_time() {
        use std::time::{Duration, Instant};

        // Recursive-descent AST building needs a deep stack for 100 nested
        // levels; a debug test thread's default 2 MiB stack overflows on the
        // recursion alone (unrelated to the backtracking bug this guards). Run
        // on a generously sized stack so the test is deterministic in both debug
        // and release, then let the time bound catch any exponential regression.
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                let parser = Parser::new();
                for depth in [25usize, 100] {
                    let input = format!("{}1{}", "(".repeat(depth), ")".repeat(depth));
                    let start = Instant::now();
                    let result = parser.parse(&input);
                    let elapsed = start.elapsed();
                    assert!(
                        result.is_ok(),
                        "Failed to parse {depth}-deep parenthesized expression: {result:?}",
                    );
                    assert!(
                        elapsed < Duration::from_secs(2),
                        "Parsing {depth}-deep parens took {elapsed:?}; expected roughly linear \
                         time (exponential backtracking has regressed)",
                    );
                }
            })
            .expect("spawn parser thread")
            .join()
            .expect("deeply-nested-paren parsing panicked or overflowed its stack");
    }

    // Guard the semantics the exponential-backtracking fix had to preserve: one
    // inner expression is a grouping (unwrapped), two or more is a tuple, and
    // nesting composes both.
    #[test]
    fn test_paren_grouping_vs_tuple_semantics() {
        let parser = Parser::new();

        let grouping = parser.parse("(2 + 3) * 4").expect("grouping should parse");
        assert!(
            matches!(
                grouping.statements[0].unwrapped(),
                Statement::Expression(Expr::BinaryOp { .. })
            ),
            "grouping should unwrap to its inner expression, got: {:?}",
            grouping.statements[0]
        );

        let tuple = parser.parse("(1, 2, 3)").expect("tuple should parse");
        if let Statement::Expression(Expr::Tuple(items)) = tuple.statements[0].unwrapped() {
            assert_eq!(items.len(), 3, "expected a 3-element tuple");
        } else {
            panic!("expected a tuple, got: {:?}", tuple.statements[0]);
        }

        let nested = parser
            .parse("((1, 2), (3, 4))")
            .expect("nested tuple should parse");
        if let Statement::Expression(Expr::Tuple(items)) = nested.statements[0].unwrapped() {
            assert_eq!(items.len(), 2, "outer tuple should have 2 elements");
            assert!(
                matches!(&items[0], Expr::Tuple(inner) if inner.len() == 2),
                "first element should itself be a 2-tuple, got: {:?}",
                items[0]
            );
        } else {
            panic!("expected a nested tuple, got: {:?}", nested.statements[0]);
        }

        assert!(
            parser.parse("(x) => x + 1").is_ok(),
            "single-parameter lambda should still parse"
        );
        assert!(
            parser.parse("(a, b) => a * b").is_ok(),
            "multi-parameter lambda should still parse"
        );
    }
}
