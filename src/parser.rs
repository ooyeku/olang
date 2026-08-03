use crate::ast::{
    Argument, AsyncFunctionDecl, BinaryOp, EnumVariant, ErrorTypeDecl, Expr, FieldValue,
    FunctionDecl, LetDecl, MapEntry, MatchArm, Parameter, Pattern, Program, PromiseType,
    Statement, StructField, StructLiteral, TypeAnnotation, TypeDecl, TypeDefinition,
    BitwiseOp, UnaryOp, TemplatePart,
    ShareDecl, UseDecl, TestDecl,
};
use pest::{iterators::Pair, iterators::Pairs, Parser as PestParser};
use pest_derive::Parser;
use thiserror::Error;
use std::sync::Arc as Rc;
use std::sync::Arc;

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
        snippet.push_str(&format!("{:4} | {}^", "", " ".repeat(column.saturating_sub(1))));

        Self { line, column, offset, input_snippet: snippet }
    }

    /// Create PositionInfo directly from a pest::Position
    pub fn from_position(pos: pest::Position) -> Self {
        let (line, column) = pos.line_col();
        let offset = pos.pos();
        let error_line = sanitize_snippet(pos.line_of());
        let mut snippet = String::new();
        snippet.push_str(&format!("{:4} | {}\n", line, error_line));
        snippet.push_str(&format!("{:4} | {}^", "", " ".repeat(column.saturating_sub(1))));

        Self { line, column, offset, input_snippet: snippet }
    }
}


/// Very basic snippet sanitizer to avoid leaking sensitive content in logs.
/// - Truncates long lines
/// - Replaces control characters with spaces
fn sanitize_snippet(s: &str) -> String {
    let max_len = 200usize;
    let mut line = s.chars()
        .map(|c| if c.is_control() && c != '\n' && c != '\t' { ' ' } else { c })
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
        }
    }
    
    /// Parse with enhanced error reporting
    pub fn parse_with_suggestions(&self, input: &str) -> Result<Program, (ParseError, Vec<ErrorSuggestion>)> {
        match self.parse(input) {
            Ok(program) => Ok(program),
            Err(error) => {
                let suggestions = self.suggestion_engine.suggest_for_parse_error(&error, input);
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

    pub fn parse(&self, input: &str) -> Result<Program, ParseError> {
        let parsed = <OlangParser as PestParser<Rule>>::parse(Rule::program, input)?;

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
                        statements.push(self.build_statement(stmt_inner)?);
                    }
                }
            }
        }

        Ok(Program { statements })
    }

    fn build_statement(&self, pair: Pair<Rule>) -> Result<Statement, ParseError> {
        match pair.as_rule() {
            Rule::let_decl => Ok(Statement::LetDecl(self.build_let_decl(pair.into_inner())?)),
            Rule::function_decl => Ok(Statement::FunctionDecl(
                self.build_function_decl(pair.into_inner())?,
            )),
            Rule::async_function_decl => Ok(Statement::AsyncFunctionDecl(
                self.build_async_function_decl(pair.into_inner())?,
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
            Rule::use_decl => Ok(Statement::UseDecl(
                self.build_use_decl(pair.into_inner())?,
            )),
            Rule::test_decl => Ok(Statement::TestDecl(
                self.build_test_decl(pair.into_inner())?,
            )),
            Rule::expr => Ok(Statement::Expression(self.build_expr(pair.into_inner())?)),
            _ => Err(ParseError::invalid_syntax_at(
                format!("Invalid statement: {:?}", pair.as_rule()),
                PositionInfo::from_pair(&pair),
            )),
        }
    }

    fn build_error_type_decl(&self, mut pairs: Pairs<Rule>) -> Result<ErrorTypeDecl, ParseError> {
        let name = pairs
            .next()
            .ok_or_else(|| ParseError::InvalidSyntax {
                message: "Missing error type name".to_string(),
            })?
            .as_str()
            .to_string();

        let fields = if let Some(pair) = pairs.next() {
            match pair.as_rule() {
                Rule::struct_field_list => self.build_struct_field_list(pair.into_inner())?,
                Rule::error_variant_list => self.build_error_variant_list(pair.into_inner())?,
                _ => Vec::new(),
            }
        } else {
            Vec::new()
        };

        Ok(ErrorTypeDecl { name, fields })
    }

    fn build_error_variant_list(&self, pairs: Pairs<Rule>) -> Result<Vec<StructField>, ParseError> {
        let mut fields = Vec::new();
        for pair in pairs {
            if pair.as_rule() == Rule::error_variant {
                fields.extend(self.build_error_variant(pair.into_inner())?);
            }
        }
        Ok(fields)
    }

    fn build_error_variant(&self, mut pairs: Pairs<Rule>) -> Result<Vec<StructField>, ParseError> {
        let variant_name = pairs
            .next()
            .ok_or_else(|| ParseError::InvalidSyntax {
                message: "Missing error variant name".to_string(),
            })?
            .as_str()
            .to_string();

        // Check if there's a type specification after the colon
        if let Some(type_pair) = pairs.next() {
            match type_pair.as_rule() {
                Rule::unit_type => {
                    // Unit type variant (e.g., DivideByZero: ())
                    Ok(vec![StructField {
                        name: variant_name,
                        field_type: TypeAnnotation::Unit,
                    }])
                }
                Rule::struct_field_list => {
                    // Struct-like variant (e.g., NegativeSqrt: { value: Float })
                    let _inner_fields = self.build_struct_field_list(type_pair.into_inner())?;
                    // For now, flatten the fields with the variant name as prefix
                    // In a proper implementation, we'd have nested structure
                    let custom_type = format!("{}Variant", variant_name);
                    Ok(vec![StructField {
                        name: variant_name,
                        // Use a custom type to represent the struct variant
                        field_type: TypeAnnotation::Custom(custom_type),
                    }])
                }
                _ => Err(ParseError::InvalidSyntax {
                    message: format!("Invalid error variant type for {}", variant_name),
                }),
            }
        } else {
            // No type specified, treat as unit variant
            Ok(vec![StructField {
                name: variant_name,
                field_type: TypeAnnotation::Unit,
            }])
        }
    }

    fn build_let_decl(&self, mut pairs: Pairs<Rule>) -> Result<LetDecl, ParseError> {
        let pattern_pair = pairs
            .next()
            .ok_or_else(|| ParseError::InvalidSyntax {
                message: "Missing pattern in let declaration".to_string(),
            })?;
        
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
                Rule::for_loop => self.build_for_loop(pair.into_inner()),
                Rule::while_loop => self.build_while_loop(pair.into_inner()),
                Rule::loop_expr => self.build_loop_expr(pair.into_inner()),
                Rule::break_expr => Ok(Expr::Break),
                Rule::continue_expr => Ok(Expr::Continue),
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
        let mut expr = self.build_comparison_expr(first_pair.into_inner())?;

        while let Some(op_pair) = pairs.next() {
            if let Some(right_pair) = pairs.next() {
                let op = match op_pair.as_str() {
                    "&&" => BinaryOp::And,
                    "||" => BinaryOp::Or,
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
        let mut expr = self.build_additive_expr(first_pair.into_inner())?;

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
                let right = self.build_additive_expr(right_pair.into_inner())?;
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
        let mut expr = self.build_pipe_expr(first_pair.into_inner())?;

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
                let right = self.build_pipe_expr(right_pair.into_inner())?;
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
        let mut left = self.build_unary_expr(first_pair.into_inner())?;

        if let Some(pair) = pairs.next() {
            let op_str = pair.as_str();
            if op_str == ".." || op_str == "..=" {
                let inclusive = op_str == "..=";
                let right_pair = pairs.next().ok_or_else(|| ParseError::InvalidSyntax {
                    message: "Missing right operand in range expression".to_string(),
                })?;
                let right = self.build_unary_expr(right_pair.into_inner())?;
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
                    if let Expr::Identifier(ref name) = expr {
                        if (name == "Ok" || name == "Err") && pending_args.is_empty() {
                            // This is a result expression, not a function call
                            let args = if let Some(arg_list_pair) = pair.into_inner().next() {
                                self.build_arg_list(arg_list_pair.into_inner())?
                            } else {
                                Vec::new()
                            };
                            if args.len() == 1 {
                                let arg = args.into_iter().next().ok_or_else(|| ParseError::InvalidSyntax {
                                    message: format!("{} expression missing argument", name),
                                })?;
                                let inner = match arg {
                                    Argument::Positional(e) => e,
                                    Argument::Named { .. } => {
                                        return Err(ParseError::InvalidSyntax {
                                            message: format!("{} expressions cannot use named arguments", name),
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
                let name = inner_pairs.next().ok_or_else(|| ParseError::InvalidSyntax {
                    message: "Missing argument name".to_string(),
                })?.as_str().to_string();
                let value = inner_pairs.next().ok_or_else(|| ParseError::InvalidSyntax {
                    message: "Missing argument value".to_string(),
                })?;
                Ok(Argument::Named {
                    name,
                    value: self.build_expr(value.into_inner())?,
                })
            }
            Rule::positional_arg => {
                let expr_pair = first_pair.into_inner().next().ok_or_else(|| ParseError::InvalidSyntax {
                    message: "Missing positional argument expression".to_string(),
                })?;
                Ok(Argument::Positional(self.build_expr(expr_pair.into_inner())?))
            }
            _ => Err(ParseError::InvalidSyntax {
                message: format!("Invalid argument rule: {:?}", first_pair.as_rule()),
            }),
        }
    }

    fn build_expr_list(&self, pairs: Pairs<Rule>) -> Result<Vec<Expr>, ParseError> {
        let mut expressions = Vec::new();
        for pair in pairs {
            match pair.as_rule() {
                Rule::expr => {
                    expressions.push(self.build_expr(pair.into_inner())?);
                }
                Rule::argument => {
                    // For expression lists, we only want the positional value
                    let arg = self.build_argument(pair.into_inner())?;
                    match arg {
                        Argument::Positional(expr) => expressions.push(expr),
                        Argument::Named { value, .. } => expressions.push(value),
                    }
                }
                _ => {}
            }
        }
        Ok(expressions)
    }

    fn build_primary(&self, mut pairs: Pairs<Rule>) -> Result<Expr, ParseError> {
        // Check for unary expressions
        if let Some(first) = pairs.peek() {
            if first.as_rule() == Rule::unary_op {
                return self.build_unary_expr(pairs);
            }
        }
        // Fallback to existing logic
        let pair = pairs.next().ok_or_else(|| ParseError::InvalidSyntax {
            message: "Empty primary expression".to_string(),
        })?;
        match pair.as_rule() {
            Rule::lambda => self.build_lambda(pair.into_inner()),
            Rule::async_expr => self.build_async_expr(pair.into_inner()),
            Rule::await_expr => self.build_await_expr(pair.into_inner()),
            Rule::promise_expr => self.build_promise_expr(pair.into_inner()),
            Rule::all_expr => self.build_all_expr(pair.into_inner()),
            Rule::race_expr => self.build_race_expr(pair.into_inner()),
            Rule::spawn_expr => self.build_spawn_expr(pair.into_inner()),
            Rule::match_expr => self.build_match_expr(pair.into_inner()),
            Rule::if_expr => self.build_if_expr(pair.into_inner()),
            Rule::try_catch_expr => self.build_try_catch_expr(pair.into_inner()),
            Rule::struct_literal => self.build_struct_literal(pair.into_inner()),
            Rule::anonymous_object => self.build_anonymous_object(pair.into_inner()),
            Rule::map_literal => self.build_map_literal(pair.into_inner()),
            Rule::literal => self.build_literal(pair.into_inner()),
            Rule::identifier => Ok(Expr::Identifier(pair.as_str().to_string())),
            Rule::block => self.build_block(pair.into_inner()),
            Rule::expr => self.build_expr(pair.into_inner()),
            _ => Err(ParseError::InvalidSyntax {
                message: format!("Invalid primary rule: {:?}", pair.as_rule()),
            }),
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
                            })
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

        let first_pair = pairs.next().ok_or_else(|| ParseError::InvalidSyntax {
            message: "Missing parameter list in lambda".to_string(),
        })?;
        if first_pair.as_rule() == Rule::param_list {
            parameters = self.build_param_list(first_pair.into_inner())?;
        }

        for pair in pairs {
            match pair.as_rule() {
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

    fn build_async_expr(&self, mut pairs: Pairs<Rule>) -> Result<Expr, ParseError> {
        let mut parameters = Vec::new();
        let mut return_type = None;
        let mut body = None;

        let first_pair = pairs.next().ok_or_else(|| ParseError::InvalidSyntax {
            message: "Missing parameter list in async expression".to_string(),
        })?;
        if first_pair.as_rule() == Rule::param_list {
            parameters = self.build_param_list(first_pair.into_inner())?;
        }

        for pair in pairs {
            match pair.as_rule() {
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
            message: "Missing function body".to_string(),
        })?;

        Ok(Expr::Async {
            parameters,
            return_type,
            body: Box::new(body_expr),
        })
    }

    fn build_await_expr(&self, mut pairs: Pairs<Rule>) -> Result<Expr, ParseError> {
        let expr = pairs.next().ok_or_else(|| ParseError::InvalidSyntax {
            message: "Missing expression in await".to_string(),
        })?;

        let awaited_expr = self.build_call_expr(expr.into_inner())?;

        Ok(Expr::Await {
            expression: Box::new(awaited_expr),
        })
    }

    fn build_promise_expr(&self, mut pairs: Pairs<Rule>) -> Result<Expr, ParseError> {
        let _promise_literal = pairs.next(); // Skip "Promise"
        let method = pairs.next().ok_or_else(|| ParseError::InvalidSyntax {
            message: "Missing promise method".to_string(),
        })?;

        let promise_type = match method.as_str() {
            "resolve" => PromiseType::Resolve,
            "reject" => PromiseType::Reject,
            "delay" => PromiseType::Delay,
            _ => {
                return Err(ParseError::InvalidSyntax {
                    message: format!("Unknown promise method: {}", method.as_str()),
                })
            }
        };

        let value_expr = pairs.next().ok_or_else(|| ParseError::InvalidSyntax {
            message: "Missing value expression in promise".to_string(),
        })?;
        let value = self.build_expr(value_expr.into_inner())?;

        let delay = if promise_type == PromiseType::Delay {
            if let Some(delay_expr) = pairs.next() {
                Some(Box::new(self.build_expr(delay_expr.into_inner())?))
            } else {
                None
            }
        } else {
            None
        };

        Ok(Expr::Promise {
            promise_type,
            value: Box::new(value),
            delay,
        })
    }

    fn build_all_expr(&self, mut pairs: Pairs<Rule>) -> Result<Expr, ParseError> {
        let _promise_literal = pairs.next(); // Skip "Promise"
        let _all_literal = pairs.next(); // Skip "all"

        let mut expressions = Vec::new();
        for pair in pairs {
            if pair.as_rule() == Rule::arg_list {
                expressions = self.build_expr_list(pair.into_inner())?;
                break;
            }
        }

        Ok(Expr::All(expressions))
    }

    fn build_race_expr(&self, mut pairs: Pairs<Rule>) -> Result<Expr, ParseError> {
        let _promise_literal = pairs.next(); // Skip "Promise"
        let _race_literal = pairs.next(); // Skip "race"

        let mut expressions = Vec::new();
        for pair in pairs {
            if pair.as_rule() == Rule::arg_list {
                expressions = self.build_expr_list(pair.into_inner())?;
                break;
            }
        }

        Ok(Expr::Race(expressions))
    }

    fn build_spawn_expr(&self, mut pairs: Pairs<Rule>) -> Result<Expr, ParseError> {
        let expr = pairs.next().ok_or_else(|| ParseError::InvalidSyntax {
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
        while let Some(pair) = pairs.next() {
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
                let mut inner = pair.into_inner();
                let mut types = Vec::new();
                while let Some(type_pair) = inner.next() {
                    types.push(self.build_type_annotation(type_pair.into_inner())?);
                }
                Ok(TypeAnnotation::Union { types })
            }
            Rule::intersection_type => {
                let mut inner = pair.into_inner();
                let mut types = Vec::new();
                while let Some(type_pair) = inner.next() {
                    types.push(self.build_type_annotation(type_pair.into_inner())?);
                }
                Ok(TypeAnnotation::Intersection { types })
            }
            Rule::literal_type => {
                let inner = pair.into_inner().next().ok_or_else(|| ParseError::InvalidSyntax {
                    message: "Empty literal type".to_string(),
                })?;
                let value = match inner.as_rule() {
                    Rule::string => crate::ast::Value::String(std::sync::Arc::new(self.unquote_string(inner.as_str())?)),
                    Rule::integer => {
                        let v = inner.as_str().parse::<i64>().map_err(|_| ParseError::InvalidSyntax {
                            message: "Invalid integer in literal type".to_string(),
                        })?;
                        crate::ast::Value::Integer(v)
                    }
                    Rule::boolean => {
                        let v = inner.as_str().parse::<bool>().map_err(|_| ParseError::InvalidSyntax {
                            message: "Invalid boolean in literal type".to_string(),
                        })?;
                        crate::ast::Value::Boolean(v)
                    }
                    _ => {
                        return Err(ParseError::InvalidSyntax {
                            message: format!("Invalid literal type: {:?}", inner.as_rule()),
                        })
                    }
                };
                Ok(TypeAnnotation::Literal { value: Box::new(value) })
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
            },
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
                let first = inner_pairs.next().ok_or_else(|| ParseError::InvalidSyntax {
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

                let return_type = inner_pairs.next().ok_or_else(|| ParseError::InvalidSyntax {
                    message: "Missing return type in function type".to_string(),
                })?;
                let return_annotation = self.build_type_annotation(return_type.into_inner())?;

                Ok(TypeAnnotation::Function {
                    params,
                    return_type: Box::new(return_annotation),
                })
            }
            Rule::promise_type => {
                let mut inner_pairs = pair.into_inner();

                // Get the value type
                let value_type = inner_pairs
                    .next()
                    .ok_or_else(|| ParseError::InvalidSyntax {
                        message: "Missing value type for Promise".to_string(),
                    })?;
                let value_annotation = self.build_type_annotation(value_type.into_inner())?;

                // Get the optional error type
                let error_type = if let Some(err_type) = inner_pairs.next() {
                    Some(Box::new(self.build_type_annotation(err_type.into_inner())?))
                } else {
                    None
                };

                Ok(TypeAnnotation::Promise {
                    value_type: Box::new(value_annotation),
                    error_type,
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
                Ok(TypeAnnotation::Custom(format!("{{anonymous_struct_{}}}", fields.len())))
            }
            _ => Err(ParseError::InvalidSyntax {
                message: format!("Invalid type annotation rule: {:?}", pair.as_rule()),
            }),
        }
    }

    fn build_match_expr(&self, mut pairs: Pairs<Rule>) -> Result<Expr, ParseError> {
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

    fn build_match_arm(&self, mut pairs: Pairs<Rule>) -> Result<MatchArm, ParseError> {
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
                let start_pattern = match start_pair.as_rule() {
                    Rule::integer => {
                        let value = start_pair.as_str().parse::<i64>().map_err(|_| ParseError::InvalidSyntax {
                            message: "Invalid integer in range pattern".to_string(),
                        })?;
                        Pattern::Literal(crate::ast::Value::Integer(value))
                    }
                    Rule::char_literal => {
                        let char_str = start_pair.as_str().trim_matches('\'');
                        let value = char_str.chars().next().ok_or_else(|| ParseError::InvalidSyntax {
                            message: "Invalid character in range pattern".to_string(),
                        })?;
                        Pattern::Literal(crate::ast::Value::String(value.to_string().into()))
                    }
                    _ => return Err(ParseError::InvalidSyntax {
                        message: format!("Invalid start type in range pattern '{}': {:?}", pattern_str, start_pair.as_rule()),
                    })
                };
                
                let end_pattern = match end_pair.as_rule() {
                    Rule::integer => {
                        let value = end_pair.as_str().parse::<i64>().map_err(|_| ParseError::InvalidSyntax {
                            message: "Invalid integer in range pattern".to_string(),
                        })?;
                        Pattern::Literal(crate::ast::Value::Integer(value))
                    }
                    Rule::char_literal => {
                        let char_str = end_pair.as_str().trim_matches('\'');
                        let value = char_str.chars().next().ok_or_else(|| ParseError::InvalidSyntax {
                            message: "Invalid character in range pattern".to_string(),
                        })?;
                        Pattern::Literal(crate::ast::Value::String(value.to_string().into()))
                    }
                    _ => return Err(ParseError::InvalidSyntax {
                        message: format!("Invalid end type in range pattern '{}': {:?}", pattern_str, end_pair.as_rule()),
                    })
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
                    alternatives.into_iter().next()
                        .ok_or_else(|| ParseError::InvalidSyntax {
                            message: "Internal error: expected one alternative in or-pattern".to_string(),
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
                    match p.as_rule() {
                        Rule::list_pattern_inner => {
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
                        _ => {}
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
                                        message: "Missing field name in anonymous struct pattern".to_string(),
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

                Ok(Pattern::AnonymousStruct {
                    field_patterns,
                })
            }
            Rule::base_pattern => {
                // Handle base_pattern by recursing into its inner content
                let inner = pair.into_inner().next().ok_or_else(|| ParseError::InvalidSyntax {
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
                let start_pattern = match start_pair.as_rule() {
                    Rule::integer => {
                        let value = start_pair.as_str().parse::<i64>().map_err(|_| ParseError::InvalidSyntax {
                            message: "Invalid integer in range pattern".to_string(),
                        })?;
                        Pattern::Literal(crate::ast::Value::Integer(value))
                    }
                    Rule::char_literal => {
                        let char_str = start_pair.as_str().trim_matches('\'');
                        let value = char_str.chars().next().ok_or_else(|| ParseError::InvalidSyntax {
                            message: "Invalid character in range pattern".to_string(),
                        })?;
                        Pattern::Literal(crate::ast::Value::String(value.to_string().into()))
                    }
                    _ => return Err(ParseError::InvalidSyntax {
                        message: format!("Invalid start type in range pattern '{}': {:?}", pattern_str, start_pair.as_rule()),
                    })
                };
                
                let end_pattern = match end_pair.as_rule() {
                    Rule::integer => {
                        let value = end_pair.as_str().parse::<i64>().map_err(|_| ParseError::InvalidSyntax {
                            message: "Invalid integer in range pattern".to_string(),
                        })?;
                        Pattern::Literal(crate::ast::Value::Integer(value))
                    }
                    Rule::char_literal => {
                        let char_str = end_pair.as_str().trim_matches('\'');
                        let value = char_str.chars().next().ok_or_else(|| ParseError::InvalidSyntax {
                            message: "Invalid character in range pattern".to_string(),
                        })?;
                        Pattern::Literal(crate::ast::Value::String(value.to_string().into()))
                    }
                    _ => return Err(ParseError::InvalidSyntax {
                        message: format!("Invalid end type in range pattern '{}': {:?}", pattern_str, end_pair.as_rule()),
                    })
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
                    alternatives.into_iter().next()
                        .ok_or_else(|| ParseError::InvalidSyntax {
                            message: "Internal error: expected one alternative in or-pattern".to_string(),
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
                    match p.as_rule() {
                        Rule::list_pattern_inner => {
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
                        _ => {}
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
                                        message: "Missing field name in anonymous struct pattern".to_string(),
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

                Ok(Pattern::AnonymousStruct {
                    field_patterns,
                })
            }
            Rule::base_pattern => {
                // Handle base_pattern by recursing into its inner content
                let inner = pair.into_inner().next().ok_or_else(|| ParseError::InvalidSyntax {
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
            Rule::integer => {
                // Remove numeric separators
                let s = pair.as_str().replace('_', "");
                let value = s.parse::<i64>().map_err(|_| ParseError::invalid_syntax_at(
                    "Invalid integer literal".to_string(),
                    PositionInfo::from_pair(&pair),
                ))?;
                Ok(Expr::Integer(value))
            }
            Rule::float => {
                let s = pair.as_str().replace('_', "");
                let value = s.parse::<f64>().map_err(|_| ParseError::invalid_syntax_at(
                    "Invalid float literal".to_string(),
                    PositionInfo::from_pair(&pair),
                ))?;
                Ok(Expr::Float(value))
            }
            Rule::binary => {
                let raw = pair.as_str().replace('_', "");
                let (sign, digits) = if raw.starts_with("-0b") {
                    (-1i64, &raw[3..])
                } else if raw.starts_with("+0b") {
                    (1i64, &raw[3..])
                } else {
                    (1i64, &raw[2..])
                };
                let parsed = i64::from_str_radix(digits, 2).map_err(|_| ParseError::invalid_syntax_at(
                    "Invalid binary literal".to_string(),
                    PositionInfo::from_pair(&pair),
                ))?;
                Ok(Expr::Integer(sign * parsed))
            }
            Rule::octal => {
                let raw = pair.as_str().replace('_', "");
                let (sign, digits) = if raw.starts_with("-0o") {
                    (-1i64, &raw[3..])
                } else if raw.starts_with("+0o") {
                    (1i64, &raw[3..])
                } else {
                    (1i64, &raw[2..])
                };
                let parsed = i64::from_str_radix(digits, 8).map_err(|_| ParseError::invalid_syntax_at(
                    "Invalid octal literal".to_string(),
                    PositionInfo::from_pair(&pair),
                ))?;
                Ok(Expr::Integer(sign * parsed))
            }
            Rule::hex => {
                let raw = pair.as_str().replace('_', "");
                let (sign, digits) = if raw.starts_with("-0x") {
                    (-1i64, &raw[3..])
                } else if raw.starts_with("+0x") {
                    (1i64, &raw[3..])
                } else {
                    (1i64, &raw[2..])
                };
                let parsed = i64::from_str_radix(digits, 16).map_err(|_| ParseError::invalid_syntax_at(
                    "Invalid hex literal".to_string(),
                    PositionInfo::from_pair(&pair),
                ))?;
                Ok(Expr::Integer(sign * parsed))
            }
            Rule::string => {
                let full_str = pair.as_str();
                // Extract content between quotes, handling edge cases
                if full_str.len() >= 2 && full_str.starts_with('"') && full_str.ends_with('"') {
                    let raw_value = &full_str[1..full_str.len()-1];
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
                let value = char_str.chars().next().ok_or_else(|| ParseError::invalid_syntax_at(
                    "Invalid character literal".to_string(),
                    PositionInfo::from_pair(&pair),
                ))?;
                Ok(Expr::String(value.to_string().into()))
            }
            Rule::template_string => {
                self.build_template_string(pair.into_inner())
            }
            Rule::boolean => {
                let value = pair.as_str().parse::<bool>().map_err(|_| ParseError::invalid_syntax_at(
                    "Invalid boolean literal".to_string(),
                    PositionInfo::from_pair(&pair),
                ))?;
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
            Rule::tuple => {
                let mut items = Vec::new();
                for p in pair.into_inner() {
                    if p.as_rule() == Rule::expr {
                        items.push(self.build_expr(p.into_inner())?);
                    }
                }
                Ok(Expr::Tuple(items.into()))
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
                let inner = pair
                    .into_inner()
                    .next()
                    .ok_or_else(|| ParseError::InvalidSyntax {
                        message: "Empty statement in block".to_string(),
                    })?;
                statements.push(self.build_statement(inner)?);
            }
        }

        Ok(Expr::Block(statements))
    }

    fn build_if_expr(&self, mut pairs: Pairs<Rule>) -> Result<Expr, ParseError> {
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
                    })
                }
            }
        } else {
            return Err(ParseError::InvalidSyntax {
                message: "Missing then branch in if expression".to_string(),
            });
        };

        let else_branch = if let Some(pair) = pairs.next() {
            match pair.as_rule() {
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

    fn build_function_decl(&self, mut pairs: Pairs<Rule>) -> Result<FunctionDecl, ParseError> {
        let name = pairs
            .next()
            .ok_or_else(|| ParseError::InvalidSyntax {
                message: "Missing function name".to_string(),
            })?
            .as_str()
            .to_string();

        let mut type_params = Vec::new();
        let mut parameters = Vec::new();
        let mut return_type = None;
        let mut body_expr = None;

        for pair in pairs {
            match pair.as_rule() {
                Rule::type_params => {
                    // Parse type parameters
                    for type_param in pair.into_inner() {
                        if type_param.as_rule() == Rule::identifier {
                            type_params.push(type_param.as_str().to_string());
                        }
                    }
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
            type_params,
            parameters,
            return_type,
            body,
        })
    }

    fn build_async_function_decl(
        &self,
        mut pairs: Pairs<Rule>,
    ) -> Result<AsyncFunctionDecl, ParseError> {
        let name = pairs
            .next()
            .ok_or_else(|| ParseError::InvalidSyntax {
                message: "Missing function name".to_string(),
            })?
            .as_str()
            .to_string();

        let mut type_params = Vec::new();
        let mut parameters = Vec::new();
        let mut return_type = None;
        let mut body = None;

        for pair in pairs {
            match pair.as_rule() {
                Rule::type_params => {
                    for type_param in pair.into_inner() {
                        if type_param.as_rule() == Rule::identifier {
                            type_params.push(type_param.as_str().to_string());
                        }
                    }
                }
                Rule::param_list => {
                    parameters = self.build_param_list(pair.into_inner())?;
                }
                Rule::type_annotation => {
                    return_type = Some(self.build_type_annotation(pair.into_inner())?);
                }
                Rule::expr => {
                    body = Some(self.build_expr(pair.into_inner())?);
                }
                _ => {}
            }
        }

        let body_expr = body.ok_or_else(|| ParseError::InvalidSyntax {
            message: "Missing async function body".to_string(),
        })?;

        Ok(AsyncFunctionDecl {
            name,
            type_params,
            parameters,
            return_type,
            body: body_expr,
        })
    }

    fn build_type_decl(&self, mut pairs: Pairs<Rule>) -> Result<TypeDecl, ParseError> {
        let name = pairs
            .next()
            .ok_or_else(|| ParseError::InvalidSyntax {
                message: "Missing type name".to_string(),
            })?
            .as_str()
            .to_string();

        let mut type_params = Vec::new();
        let mut definition = None;

        for pair in pairs {
            match pair.as_rule() {
                Rule::type_params => {
                    // Parse type parameters
                    for type_param in pair.into_inner() {
                        if type_param.as_rule() == Rule::identifier {
                            type_params.push(type_param.as_str().to_string());
                        }
                    }
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
                let fields = if let Some(field_list) = pair.into_inner().next() {
                    self.build_struct_field_list(field_list.into_inner())?
                } else {
                    Vec::new()
                };
                Ok(TypeDefinition::Struct { fields })
            }
            Rule::enum_def => {
                let variants = if let Some(variant_list) = pair.into_inner().next() {
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

    fn build_for_loop(&self, mut pairs: Pairs<Rule>) -> Result<Expr, ParseError> {
        // Expected order: identifier, expr (iterable), block (body)
        let var_name_pair = pairs.next().ok_or_else(|| ParseError::InvalidSyntax {
            message: "Missing loop variable".to_string(),
        })?;
        let variable = var_name_pair.as_str().to_string();

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
                })
            }
        };

        Ok(Expr::ForLoop {
            variable,
            iterable: Box::new(iterable_expr),
            body: Box::new(body_expr),
        })
    }

    fn build_while_loop(&self, mut pairs: Pairs<Rule>) -> Result<Expr, ParseError> {
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
                })
            }
        };

        Ok(Expr::WhileLoop {
            condition: Box::new(condition_expr),
            body: Box::new(body_expr),
        })
    }

    fn build_loop_expr(&self, mut pairs: Pairs<Rule>) -> Result<Expr, ParseError> {
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
                })
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

    fn build_try_catch_expr(&self, mut pairs: Pairs<Rule>) -> Result<Expr, ParseError> {
        let try_block = pairs.next().ok_or_else(|| ParseError::InvalidSyntax {
            message: "Missing try block".to_string(),
        })?;
        let catch_var = pairs.next().ok_or_else(|| ParseError::InvalidSyntax {
            message: "Missing catch variable".to_string(),
        })?;
        let catch_block = pairs.next().ok_or_else(|| ParseError::InvalidSyntax {
            message: "Missing catch block".to_string(),
        })?;

        Ok(Expr::TryCatch {
            try_block: Box::new(self.build_block(try_block.into_inner())?),
            catch_var: catch_var.as_str().to_string(),
            catch_block: Box::new(self.build_block(catch_block.into_inner())?),
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

                while let Some(ch) = chars.next() {
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
                        message: "Unclosed template interpolation".to_string(),
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

    fn parse_expression_from_string(&self, expr_str: &str) -> Result<Expr, ParseError> {
        // Use Pest to parse just the expression
        let trimmed = expr_str.trim();
        let pairs = OlangParser::parse(Rule::expr, trimmed)
            .map_err(ParseError::Pest)?;

        let expr_pair = pairs.into_iter().next().ok_or_else(|| ParseError::InvalidSyntax {
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
                                Some(digit) if digit.is_ascii_hexdigit() => {
                                    hex_digits.push(digit)
                                }
                                _ => {
                                    return Err(ParseError::InvalidSyntax {
                                        message: "Invalid hex escape sequence: expected 2 hex digits".to_string(),
                                    })
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
                            
                            while let Some(ch) = chars.next() {
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
                                        message: "Invalid character in Unicode escape sequence".to_string(),
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
                                    _ => {
                                        return Err(ParseError::InvalidSyntax {
                                            message: "Invalid unicode escape sequence: expected 4 hex digits".to_string(),
                                        })
                                    }
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

    fn build_share_decl(&self, mut pairs: Pairs<Rule>) -> Result<ShareDecl, ParseError> {
        let inner_pair = pairs.next().ok_or_else(|| ParseError::InvalidSyntax {
            message: "Missing declaration in share".to_string(),
        })?;

        match inner_pair.as_rule() {
            Rule::function_decl => Ok(ShareDecl::Function(self.build_function_decl(inner_pair.into_inner())?)),
            Rule::let_decl => Ok(ShareDecl::Let(self.build_let_decl(inner_pair.into_inner())?)),
            Rule::type_decl => Ok(ShareDecl::Type(self.build_type_decl(inner_pair.into_inner())?)),
            Rule::use_decl => Ok(ShareDecl::Use(self.build_use_decl(inner_pair.into_inner())?)),
            _ => Err(ParseError::InvalidSyntax {
                message: "Invalid declaration in share".to_string(),
            }),
        }
    }

    fn build_use_decl(&self, mut pairs: Pairs<Rule>) -> Result<UseDecl, ParseError> {
        let path_pair = pairs.next().ok_or_else(|| ParseError::InvalidSyntax {
            message: "Missing path in use".to_string(),
        })?;
        let mut path = Vec::new();
        for part in path_pair.into_inner() {
            if part.as_rule() == Rule::identifier {
                path.push(part.as_str().to_string());
            }
        }

        let list_pair = pairs.next().ok_or_else(|| ParseError::InvalidSyntax {
            message: "Missing item list in use".to_string(),
        })?;
        let mut items = Vec::new();
        for item in list_pair.into_inner() {
            if item.as_rule() == Rule::use_item {
                let inner = item.into_inner().next().unwrap();
                match inner.as_rule() {
                    Rule::identifier => {
                        items.push(crate::ast::UseItem::Specific(inner.as_str().to_string()));
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

    fn build_test_decl(&self, mut pairs: Pairs<Rule>) -> Result<TestDecl, ParseError> {
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
                let inner = statement_pair.into_inner().next().unwrap();
                if inner.as_rule() == Rule::assertion {
                    // Handle assertion as expression
                    let assertion_expr = self.build_assertion(inner.into_inner())?;
                    body.push(Statement::Expression(assertion_expr));
                } else {
                    // Handle regular statement
                    body.push(self.build_statement(inner)?);
                }
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
            },
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
            },
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
            },
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
            },
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
            },
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

impl ErrorSuggestionEngine {
    pub fn new() -> Self {
        Self {}
    }
    
    pub fn suggest_for_parse_error(&self, error: &ParseError, input: &str) -> Vec<ErrorSuggestion> {
        match error {
            ParseError::Pest(pest_error) => {
                self.suggest_for_pest_error(pest_error, input)
            }
            ParseError::InvalidSyntaxWithPosition { message, line, column, .. } => {
                self.suggest_for_invalid_syntax(message, *line, *column, input)
            }
            ParseError::UnexpectedTokenWithPosition { token, line, column, .. } => {
                self.suggest_for_unexpected_token(token, *line, *column, input)
            }
            ParseError::InvalidSyntax { message } => {
                self.suggest_for_generic_syntax_error(message, input)
            }
            ParseError::UnexpectedToken { token } => {
                self.suggest_for_generic_token_error(token, input)
            }
        }
    }
    
    fn suggest_for_pest_error(&self, pest_error: &pest::error::Error<Rule>, _input: &str) -> Vec<ErrorSuggestion> {
        let mut suggestions = Vec::new();
        
        // Analyze the pest error for common patterns
        let error_msg = format!("{}", pest_error);
        
        // Check for common syntax issues
        if error_msg.contains("expected") {
            if error_msg.contains("expected `)`") {
                suggestions.push(ErrorSuggestion {
                    message: "Missing closing parenthesis".to_string(),
                    fix: Some("Add `)` to close the parenthesis".to_string()),
                    help: Some("Check that all opening parentheses have matching closing ones".to_string()),
                    severity: SuggestionSeverity::Error,
                });
            }
            
            if error_msg.contains("expected `}`") {
                suggestions.push(ErrorSuggestion {
                    message: "Missing closing brace".to_string(),
                    fix: Some("Add `}` to close the brace".to_string()),
                    help: Some("Check that all opening braces have matching closing ones".to_string()),
                    severity: SuggestionSeverity::Error,
                });
            }
            
            if error_msg.contains("expected `]`") {
                suggestions.push(ErrorSuggestion {
                    message: "Missing closing bracket".to_string(),
                    fix: Some("Add `]` to close the bracket".to_string()),
                    help: Some("Check that all opening brackets have matching closing ones".to_string()),
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
                fix: Some("Check function syntax: `fn name(params) = body` or `fn name(params) { body }`".to_string()),
                help: Some("Functions can be declared with expression bodies (=) or block bodies ({ })".to_string()),
                severity: SuggestionSeverity::Hint,
            });
        }
        
        // Check for let declaration errors
        if error_msg.contains("let") {
            suggestions.push(ErrorSuggestion {
                message: "Let declaration syntax error".to_string(),
                fix: Some("Check let syntax: `let name = value` or `let pattern = value`".to_string()),
                help: Some("Let declarations support pattern matching and type annotations".to_string()),
                severity: SuggestionSeverity::Hint,
            });
        }
        
        suggestions
    }
    
    fn suggest_for_invalid_syntax(&self, message: &str, line: usize, column: usize, input: &str) -> Vec<ErrorSuggestion> {
        let mut suggestions = Vec::new();
        
        // Get the line content for analysis
        let lines: Vec<&str> = input.lines().collect();
        let line_content = if line > 0 && line <= lines.len() {
            lines[line - 1]
        } else {
            ""
        };
        
        // Check for common typos and mistakes
        if message.contains("integer") {
            suggestions.push(ErrorSuggestion {
                message: "Invalid integer format".to_string(),
                fix: Some("Use decimal (123), binary (0b1010), octal (0o123), or hex (0xFF) format".to_string()),
                help: Some("Integer literals support underscores for readability: 1_000_000".to_string()),
                severity: SuggestionSeverity::Error,
            });
        }
        
        if message.contains("float") {
            suggestions.push(ErrorSuggestion {
                message: "Invalid float format".to_string(),
                fix: Some("Use decimal point format: 3.14, 2.0, or scientific notation: 1e10".to_string()),
                help: Some("Float literals require a decimal point or scientific notation".to_string()),
                severity: SuggestionSeverity::Error,
            });
        }
        
        if message.contains("string") {
            suggestions.push(ErrorSuggestion {
                message: "Invalid string format".to_string(),
                fix: Some("Enclose strings in double quotes: \"hello world\"".to_string()),
                help: Some("Use raw strings for literals with backslashes: r\"C:\\path\\to\\file\"".to_string()),
                severity: SuggestionSeverity::Error,
            });
        }
        
        // Check for bracket mismatches
        if self.has_unmatched_brackets(line_content) {
            suggestions.push(ErrorSuggestion {
                message: "Unmatched brackets detected".to_string(),
                fix: Some("Check that all brackets, parentheses, and braces are properly matched".to_string()),
                help: Some("Use an editor with bracket matching to help identify issues".to_string()),
                severity: SuggestionSeverity::Error,
            });
        }
        
        // Check for assignment vs equality
        if line_content.contains("=") && !line_content.contains("==") && !line_content.contains("let") {
            suggestions.push(ErrorSuggestion {
                message: "Possible assignment in expression context".to_string(),
                fix: Some("Use `==` for equality comparison, `=` only for let declarations".to_string()),
                help: Some("Assignments are only allowed in let declarations and function parameters".to_string()),
                severity: SuggestionSeverity::Hint,
            });
        }
        
        // Check character at column position for specific suggestions
        if column > 0 && column <= line_content.len() {
            let char_at_pos = line_content.chars().nth(column - 1);
            if let Some(ch) = char_at_pos {
                match ch {
                    ';' => {
                        suggestions.push(ErrorSuggestion {
                            message: "Semicolons are not used in Olang".to_string(),
                            fix: Some("Remove the semicolon".to_string()),
                            help: Some("Olang uses newlines and expression-based syntax".to_string()),
                            severity: SuggestionSeverity::Error,
                        });
                    }
                    _ => {}
                }
            }
        }
        
        suggestions
    }
    
    fn suggest_for_unexpected_token(&self, token: &str, _line: usize, _column: usize, _input: &str) -> Vec<ErrorSuggestion> {
        let mut suggestions = Vec::new();
        
        // Common token-specific suggestions
        match token {
            ";" => {
                suggestions.push(ErrorSuggestion {
                    message: "Semicolons are not used in Olang".to_string(),
                    fix: Some("Remove the semicolon".to_string()),
                    help: Some("Olang uses newlines and expression-based syntax, not semicolons".to_string()),
                    severity: SuggestionSeverity::Error,
                });
            }
            "=" => {
                suggestions.push(ErrorSuggestion {
                    message: "Unexpected assignment operator".to_string(),
                    fix: Some("Use `==` for comparison or `let` for variable declarations".to_string()),
                    help: Some("Single `=` is only used in let declarations and function parameters".to_string()),
                    severity: SuggestionSeverity::Error,
                });
            }
            ")" | "}" | "]" => {
                suggestions.push(ErrorSuggestion {
                    message: "Unexpected closing bracket".to_string(),
                    fix: Some("Check for missing opening bracket or extra closing bracket".to_string()),
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
    
    fn suggest_for_generic_syntax_error(&self, _message: &str, input: &str) -> Vec<ErrorSuggestion> {
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
                fix: Some("Use `println(...)` or string interpolation `\"Hello {name}\"`".to_string()),
                help: Some("Olang uses `println` and string interpolation instead of printf".to_string()),
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
        assert!(result.is_ok(), "Failed to parse string with escape sequence: {:?}", result);
        
        // Test unicode escapes
        let input = r#""Hello\u0041""#;
        let result = parser.parse(input);
        assert!(result.is_ok(), "Failed to parse string with unicode escape: {:?}", result);
        
        // Test escaped quotes
        let input = r#""He said \"Hello\"""#;
        let result = parser.parse(input);
        assert!(result.is_ok(), "Failed to parse string with escaped quotes: {:?}", result);
    }

    #[test]
    fn test_named_arguments_parsing() {
        let parser = Parser::new();

        // Test function call with named arguments
        let input = "greet(name: \"Alice\", age: 25)";
        let result = parser.parse(input);
        assert!(result.is_ok(), "Failed to parse named arguments: {:?}", result);

        if let Ok(program) = result {
            assert_eq!(program.statements.len(), 1);
            if let Statement::Expression(Expr::Call { arguments, .. }) = &program.statements[0] {
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
        assert!(result.is_ok(), "Failed to parse mixed arguments: {:?}", result);

        if let Ok(program) = result {
            assert_eq!(program.statements.len(), 1);
            if let Statement::Expression(Expr::Call { arguments, .. }) = &program.statements[0] {
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
        assert!(result.is_ok(), "Failed to parse positional arguments: {:?}", result);

        if let Ok(program) = result {
            assert_eq!(program.statements.len(), 1);
            if let Statement::Expression(Expr::Call { arguments, .. }) = &program.statements[0] {
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
        assert!(result.is_ok(), "Failed to parse complex named arguments: {:?}", result);

        if let Ok(program) = result {
            assert_eq!(program.statements.len(), 1);
            if let Statement::Expression(Expr::Call { arguments, .. }) = &program.statements[0] {
                assert_eq!(arguments.len(), 3);
                
                // Check all arguments are named with proper names
                let names: Vec<&str> = arguments.iter().map(|arg| {
                    match arg {
                        Argument::Named { name, .. } => name.as_str(),
                        _ => panic!("Expected named argument"),
                    }
                }).collect();
                
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
        assert!(result.is_ok(), "Failed to parse nested calls with named arguments: {:?}", result);

        if let Ok(program) = result {
            assert_eq!(program.statements.len(), 1);
            if let Statement::Expression(Expr::Call { arguments, .. }) = &program.statements[0] {
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
}
