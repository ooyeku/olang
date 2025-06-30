use crate::ast::{
    AsyncFunctionDecl, BinaryOp, EnumVariant, ErrorTypeDecl, ExportDecl, Expr, FieldValue,
    FunctionDecl, ImportDecl, LetDecl, MatchArm, Parameter, Pattern, Program, PromiseType,
    Statement, StructField, StructLiteral, TypeAnnotation, TypeDecl, TypeDefinition,
    BitwiseOp, UnaryOp,
};
use pest::{iterators::Pair, iterators::Pairs, Parser as PestParser};
use pest_derive::Parser;
use thiserror::Error;
use std::rc::Rc;

#[derive(Parser)]
#[grammar = "grammar.pest"]
pub struct OlangParser;

#[derive(Error, Debug)]
pub enum ParseError {
    #[error("Pest parsing error: {0}")]
    Pest(#[from] pest::error::Error<Rule>),
    #[error("Invalid syntax: {message}")]
    InvalidSyntax { message: String },
    #[error("Unexpected token: {token}")]
    UnexpectedToken { token: String },
}

pub struct Parser {
    // Parser state can be added here if needed
}

impl Default for Parser {
    fn default() -> Self {
        Self::new()
    }
}

impl Parser {
    pub fn new() -> Self {
        Self {}
    }

    pub fn parse(&self, input: &str) -> Result<Program, ParseError> {
        let parsed = OlangParser::parse(Rule::program, input)?;

        let mut statements = Vec::new();
        for pair in parsed {
            if pair.as_rule() == Rule::program {
                for inner_pair in pair.into_inner() {
                    if inner_pair.as_rule() == Rule::statement {
                        let stmt_inner = inner_pair.into_inner().next().ok_or_else(|| {
                            ParseError::InvalidSyntax {
                                message: "Empty statement".to_string(),
                            }
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
            Rule::import_decl => Ok(Statement::ImportDecl(
                self.build_import_decl(pair.into_inner())?,
            )),
            Rule::export_decl => Ok(Statement::ExportDecl(
                self.build_export_decl(pair.into_inner())?,
            )),
            Rule::expr => Ok(Statement::Expression(self.build_expr(pair.into_inner())?)),
            _ => Err(ParseError::InvalidSyntax {
                message: format!("Invalid statement: {:?}", pair.as_rule()),
            }),
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
        let name = pairs
            .next()
            .ok_or_else(|| ParseError::InvalidSyntax {
                message: "Missing identifier in let declaration".to_string(),
            })?
            .as_str()
            .to_string();

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
            name,
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
                _ => Err(ParseError::InvalidSyntax {
                    message: format!("Unexpected expression rule: {:?}", pair.as_rule()),
                }),
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
        let mut left = self.build_call_expr(first_pair.into_inner())?;

        if let Some(pair) = pairs.next() {
            let op_str = pair.as_str();
            if op_str == ".." || op_str == "..=" {
                let inclusive = op_str == "..=";
                let right_pair = pairs.next().ok_or_else(|| ParseError::InvalidSyntax {
                    message: "Missing right operand in range expression".to_string(),
                })?;
                let right = self.build_call_expr(right_pair.into_inner())?;
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
                    pending_args.push(arg);
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
                                return match name.as_str() {
                                    "Ok" => Ok(Expr::ResultOk(Box::new(arg))),
                                    "Err" => Ok(Expr::ResultErr(Box::new(arg))),
                                    _ => unreachable!(),
                                };
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

    fn build_arg_list(&self, pairs: Pairs<Rule>) -> Result<Vec<Expr>, ParseError> {
        let mut args = Vec::new();
        for pair in pairs {
            if pair.as_rule() == Rule::expr {
                args.push(self.build_expr(pair.into_inner())?);
            }
        }
        Ok(args)
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
            Rule::literal => self.build_literal(pair.into_inner()),
            Rule::identifier => Ok(Expr::Identifier(pair.as_str().to_string())),
            Rule::block => self.build_block(pair.into_inner()),
            Rule::expr => self.build_expr(pair.into_inner()),
            _ => Err(ParseError::InvalidSyntax {
                message: format!("Invalid primary rule: {:?}", pair.as_rule()),
            }),
        }
    }

    fn build_unary_expr(&self, mut pairs: Pairs<Rule>) -> Result<Expr, ParseError> {
        let op_pair = pairs.next().ok_or_else(|| ParseError::InvalidSyntax {
            message: "Missing unary operator".to_string(),
        })?;
        let operand_pair = pairs.next().ok_or_else(|| ParseError::InvalidSyntax {
            message: "Missing operand for unary operator".to_string(),
        })?;
        let operand = self.build_primary(operand_pair.into_inner())?;
        let op = match op_pair.as_str() {
            "-" => UnaryOp::Negate,
            "!" => UnaryOp::Not,
            _ => {
                return Err(ParseError::InvalidSyntax {
                    message: format!("Unknown unary operator: {}", op_pair.as_str()),
                })
            }
        };
        Ok(Expr::UnaryOp {
            op,
            operand: Box::new(operand),
        })
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
            message: "Missing async expression body".to_string(),
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
                expressions = self.build_arg_list(pair.into_inner())?;
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
                expressions = self.build_arg_list(pair.into_inner())?;
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

        let type_annotation = if let Some(pair) = pairs.next() {
            if pair.as_rule() == Rule::type_annotation {
                Some(self.build_type_annotation(pair.into_inner())?)
            } else {
                None
            }
        } else {
            None
        };

        Ok(Parameter {
            name,
            type_annotation,
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
                    Rule::string => crate::ast::Value::String(std::sync::Arc::new(inner.as_str().trim_matches('"').to_string())),
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
                _ => Err(ParseError::InvalidSyntax {
                    message: format!("Unknown basic type: {}", pair.as_str()),
                }),
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

        let expression = if let Some(pair) = pairs.next() {
            if pair.as_rule() == Rule::expr {
                self.build_expr(pair.into_inner())?
            } else {
                return Err(ParseError::InvalidSyntax {
                    message: "Missing expression in match arm".to_string(),
                });
            }
        } else {
            return Err(ParseError::InvalidSyntax {
                message: "Missing expression in match arm".to_string(),
            });
        };

        Ok(MatchArm {
            pattern,
            expression,
        })
    }

    fn build_pattern(&self, mut pairs: Pairs<Rule>) -> Result<Pattern, ParseError> {
        let pair = pairs.next().ok_or_else(|| ParseError::InvalidSyntax {
            message: "Empty pattern".to_string(),
        })?;

        match pair.as_rule() {
            Rule::range_pattern => {
                let mut inner = pair.into_inner();
                let start = inner.next().ok_or_else(|| ParseError::InvalidSyntax {
                    message: "Missing start in range pattern".to_string(),
                })?;
                let end = inner.next().ok_or_else(|| ParseError::InvalidSyntax {
                    message: "Missing end in range pattern".to_string(),
                })?;
                
                Ok(Pattern::Range {
                    start: Box::new(self.build_pattern(start.into_inner())?),
                    end: Box::new(self.build_pattern(end.into_inner())?),
                    inclusive: true, // Simplified for now
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
                    Ok(alternatives.into_iter().next().unwrap())
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
            Rule::wildcard => Ok(Pattern::Wildcard),
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
                let value = pair.as_str().trim_matches('"').to_string();
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
                for p in pair.into_inner() {
                    if p.as_rule() == Rule::pattern {
                        patterns.push(self.build_pattern(p.into_inner())?);
                    }
                }
                Ok(Pattern::List(patterns))
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
            _ => Err(ParseError::InvalidSyntax {
                message: format!("Invalid pattern rule: {:?}", pair.as_rule()),
            }),
        }
    }

    fn build_base_pattern(&self, pair: Pair<Rule>) -> Result<Pattern, ParseError> {
        match pair.as_rule() {
            Rule::range_pattern => {
                let mut inner = pair.into_inner();
                let start = inner.next().ok_or_else(|| ParseError::InvalidSyntax {
                    message: "Missing start in range pattern".to_string(),
                })?;
                let end = inner.next().ok_or_else(|| ParseError::InvalidSyntax {
                    message: "Missing end in range pattern".to_string(),
                })?;
                
                Ok(Pattern::Range {
                    start: Box::new(self.build_pattern(start.into_inner())?),
                    end: Box::new(self.build_pattern(end.into_inner())?),
                    inclusive: true, // Simplified for now
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
                    Ok(alternatives.into_iter().next().unwrap())
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
            Rule::wildcard => Ok(Pattern::Wildcard),
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
                let value = pair.as_str().trim_matches('"').to_string();
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
                for p in pair.into_inner() {
                    if p.as_rule() == Rule::pattern {
                        patterns.push(self.build_pattern(p.into_inner())?);
                    }
                }
                Ok(Pattern::List(patterns))
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
                let value = s.parse::<i64>().map_err(|_| ParseError::InvalidSyntax {
                    message: "Invalid integer literal".to_string(),
                })?;
                Ok(Expr::Integer(value))
            }
            Rule::float => {
                let s = pair.as_str().replace('_', "");
                let value = s.parse::<f64>().map_err(|_| ParseError::InvalidSyntax {
                    message: "Invalid float literal".to_string(),
                })?;
                Ok(Expr::Float(value))
            }
            Rule::binary => {
                let s = pair.as_str().replace('_', "").replace("0b", "");
                let value = i64::from_str_radix(&s, 2).map_err(|_| ParseError::InvalidSyntax {
                    message: "Invalid binary literal".to_string(),
                })?;
                Ok(Expr::Integer(value))
            }
            Rule::octal => {
                let s = pair.as_str().replace('_', "").replace("0o", "");
                let value = i64::from_str_radix(&s, 8).map_err(|_| ParseError::InvalidSyntax {
                    message: "Invalid octal literal".to_string(),
                })?;
                Ok(Expr::Integer(value))
            }
            Rule::hex => {
                let s = pair.as_str().replace('_', "").replace("0x", "");
                let value = i64::from_str_radix(&s, 16).map_err(|_| ParseError::InvalidSyntax {
                    message: "Invalid hex literal".to_string(),
                })?;
                Ok(Expr::Integer(value))
            }
            Rule::string => {
                let raw_value = pair.as_str().trim_matches('"');
                let value = self.process_string_escapes(raw_value)?;
                Ok(Expr::String(value.into()))
            }
            Rule::raw_string => {
                let raw_value = pair.as_str();
                // Remove r" and ending ", but do not process escapes
                let value = &raw_value[2..raw_value.len() - 1];
                Ok(Expr::RawString(Rc::new(value.to_string())))
            }

            Rule::boolean => {
                let value = pair.as_str().parse::<bool>().map_err(|_| ParseError::InvalidSyntax {
                    message: "Invalid boolean literal".to_string(),
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
            Rule::tuple => {
                let mut items = Vec::new();
                for p in pair.into_inner() {
                    if p.as_rule() == Rule::expr {
                        items.push(self.build_expr(p.into_inner())?);
                    }
                }
                Ok(Expr::Tuple(items.into()))
            }
            _ => Err(ParseError::InvalidSyntax {
                message: format!("Invalid literal rule: {:?}", pair.as_rule()),
            }),
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

        let body = body.ok_or_else(|| ParseError::InvalidSyntax {
            message: "Missing function body".to_string(),
        })?;

        Ok(AsyncFunctionDecl {
            name,
            type_params,
            parameters,
            return_type,
            body,
        })
    }

    fn build_import_decl(&self, pairs: Pairs<Rule>) -> Result<ImportDecl, ParseError> {
        let mut module_path = String::new();
        let mut items = None;

        for pair in pairs {
            match pair.as_rule() {
                Rule::import_items => {
                    let items_inner = pair.into_inner();
                    let mut item_list = Vec::new();
                    for item_pair in items_inner {
                        if item_pair.as_rule() == Rule::identifier {
                            item_list.push(item_pair.as_str().to_string());
                        }
                    }
                    items = if item_list.is_empty() {
                        None
                    } else {
                        Some(item_list)
                    };
                }
                Rule::string => {
                    module_path = pair.as_str().trim_matches('"').to_string();
                }
                _ => {}
            }
        }

        Ok(ImportDecl { module_path, items })
    }

    fn build_export_decl(&self, mut pairs: Pairs<Rule>) -> Result<ExportDecl, ParseError> {
        let name = pairs
            .next()
            .ok_or_else(|| ParseError::InvalidSyntax {
                message: "Missing export name".to_string(),
            })?
            .as_str()
            .to_string();

        let value = pairs.next().ok_or_else(|| ParseError::InvalidSyntax {
            message: "Missing export value".to_string(),
        })?;

        let expr = self.build_expr(value.into_inner())?;

        Ok(ExportDecl { name, value: expr })
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_enhanced_string_escapes() {
        let parser = Parser::new();
        
        // Test basic escapes
        assert_eq!(parser.process_string_escapes("\\n\\t\\r").unwrap(), "\n\t\r");
        assert_eq!(parser.process_string_escapes("\\\"\\\\").unwrap(), "\"\\");
        
        // Test null character
        assert_eq!(parser.process_string_escapes("\\0").unwrap(), "\0");
        
        // Test hex escapes
        assert_eq!(parser.process_string_escapes("\\x41").unwrap(), "A");
        assert_eq!(parser.process_string_escapes("\\x61").unwrap(), "a");
        assert_eq!(parser.process_string_escapes("\\x20").unwrap(), " ");
        
        // Test fixed-length Unicode escapes
        assert_eq!(parser.process_string_escapes("\\u0041").unwrap(), "A");
        assert_eq!(parser.process_string_escapes("\\u0061").unwrap(), "a");
        assert_eq!(parser.process_string_escapes("\\u0020").unwrap(), " ");
        
        // Test variable-length Unicode escapes
        assert_eq!(parser.process_string_escapes("\\u{41}").unwrap(), "A");
        assert_eq!(parser.process_string_escapes("\\u{61}").unwrap(), "a");
        assert_eq!(parser.process_string_escapes("\\u{20}").unwrap(), " ");
        assert_eq!(parser.process_string_escapes("\\u{1F600}").unwrap(), "😀");
        
        // Test mixed escapes
        assert_eq!(parser.process_string_escapes("Hello\\nWorld\\u{1F600}").unwrap(), "Hello\nWorld😀");
        
        // Test error cases
        assert!(parser.process_string_escapes("\\x").is_err());
        assert!(parser.process_string_escapes("\\x1").is_err());
        assert!(parser.process_string_escapes("\\u").is_err());
        assert!(parser.process_string_escapes("\\u{").is_err());
        assert!(parser.process_string_escapes("\\u{}").is_err());
        assert!(parser.process_string_escapes("\\u{invalid}").is_err());
    }
}
