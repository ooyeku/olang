use crate::ast::{
    Argument, BinaryOp, Expr, FunctionDecl, GenericTypeDefinition, LetDecl, Pattern, Program, Statement,
    TypeAnnotation, TypeContext, TypeDecl, TypeError, UnaryOp,
};
use std::collections::HashMap;

/// Type checker for Olang
pub struct TypeChecker {
    context: TypeContext,
}

impl Default for TypeChecker {
    fn default() -> Self {
        Self::new()
    }
}

impl TypeChecker {
    pub fn new() -> Self {
        let mut context = TypeContext {
            variables: HashMap::new(),
            functions: HashMap::new(),
            type_vars: HashMap::new(),
            next_type_var: 0,
            generic_types: HashMap::new(),
            type_parameters: Vec::new(),
        };

        // Add built-in function types
        context.functions.insert(
            "println".to_string(),
            TypeAnnotation::Function {
                params: vec![], // Variadic
                return_type: Box::new(TypeAnnotation::Unknown),
            },
        );
        context.functions.insert(
            "len".to_string(),
            TypeAnnotation::Function {
                params: vec![TypeAnnotation::List(Box::new(TypeAnnotation::Unknown))],
                return_type: Box::new(TypeAnnotation::Int),
            },
        );
        context.functions.insert(
            "map".to_string(),
            TypeAnnotation::Function {
                params: vec![
                    TypeAnnotation::List(Box::new(TypeAnnotation::Unknown)),
                    TypeAnnotation::Function {
                        params: vec![TypeAnnotation::Unknown],
                        return_type: Box::new(TypeAnnotation::Unknown),
                    },
                ],
                return_type: Box::new(TypeAnnotation::List(Box::new(TypeAnnotation::Unknown))),
            },
        );

        // Map functions
        context.functions.insert(
            "map_get".to_string(),
            TypeAnnotation::Function {
                params: vec![
                    TypeAnnotation::Map {
                        key_type: Box::new(TypeAnnotation::Unknown),
                        value_type: Box::new(TypeAnnotation::Unknown),
                    },
                    TypeAnnotation::Unknown, // Key type
                ],
                return_type: Box::new(TypeAnnotation::Unknown), // Value type
            },
        );

        context.functions.insert(
            "map_set".to_string(),
            TypeAnnotation::Function {
                params: vec![
                    TypeAnnotation::Map {
                        key_type: Box::new(TypeAnnotation::Unknown),
                        value_type: Box::new(TypeAnnotation::Unknown),
                    },
                    TypeAnnotation::Unknown, // Key type
                    TypeAnnotation::Unknown, // Value type
                ],
                return_type: Box::new(TypeAnnotation::Map {
                    key_type: Box::new(TypeAnnotation::Unknown),
                    value_type: Box::new(TypeAnnotation::Unknown),
                }),
            },
        );

        context.functions.insert(
            "map_has_key".to_string(),
            TypeAnnotation::Function {
                params: vec![
                    TypeAnnotation::Map {
                        key_type: Box::new(TypeAnnotation::Unknown),
                        value_type: Box::new(TypeAnnotation::Unknown),
                    },
                    TypeAnnotation::Unknown, // Key type
                ],
                return_type: Box::new(TypeAnnotation::Bool),
            },
        );

        context.functions.insert(
            "map_keys".to_string(),
            TypeAnnotation::Function {
                params: vec![TypeAnnotation::Map {
                    key_type: Box::new(TypeAnnotation::Unknown),
                    value_type: Box::new(TypeAnnotation::Unknown),
                }],
                return_type: Box::new(TypeAnnotation::List(Box::new(TypeAnnotation::String))),
            },
        );

        context.functions.insert(
            "map_values".to_string(),
            TypeAnnotation::Function {
                params: vec![TypeAnnotation::Map {
                    key_type: Box::new(TypeAnnotation::Unknown),
                    value_type: Box::new(TypeAnnotation::Unknown),
                }],
                return_type: Box::new(TypeAnnotation::List(Box::new(TypeAnnotation::Unknown))),
            },
        );

        context.functions.insert(
            "map_len".to_string(),
            TypeAnnotation::Function {
                params: vec![TypeAnnotation::Map {
                    key_type: Box::new(TypeAnnotation::Unknown),
                    value_type: Box::new(TypeAnnotation::Unknown),
                }],
                return_type: Box::new(TypeAnnotation::Int),
            },
        );

        Self { context }
    }

    /// Type check a complete program
    pub fn check_program(&mut self, program: &Program) -> Result<(), Vec<TypeError>> {
        let mut errors = Vec::new();

        for statement in &program.statements {
            if let Err(error) = self.check_statement(statement) {
                errors.push(error);
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Type check a statement
    pub fn check_statement(&mut self, statement: &Statement) -> Result<TypeAnnotation, TypeError> {
        match statement {
            Statement::Expression(expr) => self.infer_type(expr),
            Statement::LetDecl(let_decl) => self.check_let_decl(let_decl),
            Statement::FunctionDecl(func_decl) => self.check_function_decl(func_decl),
            Statement::TypeDecl(type_decl) => self.check_type_decl(type_decl),
            Statement::ErrorTypeDecl(_) => {
                // Error type declarations don't produce values, just register types
                Ok(TypeAnnotation::Unknown)
            }
            Statement::ImportDecl(_) => Ok(TypeAnnotation::Unknown),
            Statement::ExportDecl(export_decl) => {
                let value_type = self.infer_type(&export_decl.value)?;
                self.context
                    .variables
                    .insert(export_decl.name.clone(), value_type.clone());
                Ok(value_type)
            }
            Statement::AsyncFunctionDecl(async_func_decl) => {
                // Check async function declaration (similar to regular function)
                let mut param_types = Vec::new();
                for param in &async_func_decl.parameters {
                    if let Some(param_type) = &param.type_annotation {
                        param_types.push(param_type.clone());
                    } else {
                        param_types.push(TypeAnnotation::Unknown);
                    }
                }

                // Return type should be Promise<T, E>
                let return_type =
                    async_func_decl
                        .return_type
                        .clone()
                        .unwrap_or(TypeAnnotation::Promise {
                            value_type: Box::new(TypeAnnotation::Unknown),
                            error_type: None,
                        });

                // Add function to context
                let func_type = TypeAnnotation::Function {
                    params: param_types.clone(),
                    return_type: Box::new(return_type.clone()),
                };

                self.context
                    .functions
                    .insert(async_func_decl.name.clone(), func_type.clone());

                // TODO: Type check the async function body
                // Create a new type checker context for the async function body
                let mut async_checker = self.clone();
                
                // Add type parameters to scope for generic async functions
                for type_param in &async_func_decl.type_params {
                    async_checker.context.type_parameters.push(type_param.clone());
                    async_checker.context.type_vars.insert(
                        type_param.clone(),
                        TypeAnnotation::TypeVariable(type_param.clone()),
                    );
                }
                
                // Add parameters to function body scope
                for (param, param_type) in async_func_decl.parameters.iter().zip(param_types.iter()) {
                    async_checker.context.variables.insert(param.name.clone(), param_type.clone());
                }
                
                // Type check the async function body
                let body_type = async_checker.infer_type(&async_func_decl.body)?;
                
                // For async functions, the body type should be compatible with the value type of the Promise
                // Extract the expected value type from the Promise return type
                let expected_body_type = match &return_type {
                    TypeAnnotation::Promise { value_type, .. } => {
                        *value_type.clone()
                    }
                    _ => {
                        // If return type is not a Promise, assume Unknown
                        TypeAnnotation::Unknown
                    }
                };
                
                // Check body type compatibility (if not unknown)
                if expected_body_type != TypeAnnotation::Unknown {
                    async_checker.check_type_compatibility(
                        &expected_body_type,
                        &body_type,
                        &format!("async function '{}' body", async_func_decl.name),
                    )?;
                }
                
                Ok(func_type)
            }
        }
    }

    /// Type check a type declaration
    fn check_type_decl(&mut self, type_decl: &TypeDecl) -> Result<TypeAnnotation, TypeError> {
        // Create a new generic type definition
        let generic_def = GenericTypeDefinition {
            name: type_decl.name.clone(),
            type_params: type_decl.type_params.clone(),
            definition: type_decl.definition.clone(),
        };

        // Add the generic type to the context
        self.context
            .generic_types
            .insert(type_decl.name.clone(), generic_def);

        // Return a custom type annotation
        Ok(TypeAnnotation::Custom(type_decl.name.clone()))
    }

    /// Type check a let declaration
    fn check_let_decl(&mut self, let_decl: &LetDecl) -> Result<TypeAnnotation, TypeError> {
        let inferred_type = if let Some(value) = &let_decl.value {
            self.infer_type(value)?
        } else {
            TypeAnnotation::Unknown
        };

        let final_type = if let Some(annotation) = &let_decl.type_annotation {
            // Check that the annotation matches the inferred type
            if let_decl.value.is_some() {
                self.check_type_compatibility(annotation, &inferred_type, "let declaration")?;
            }
            annotation.clone()
        } else {
            inferred_type
        };

        // Extract variable bindings from the pattern and bind them to appropriate types
        self.bind_pattern_variables(&let_decl.pattern, &final_type)?;

        Ok(final_type)
    }

    /// Bind variables from a pattern to their appropriate types
    fn bind_pattern_variables(&mut self, pattern: &Pattern, value_type: &TypeAnnotation) -> Result<(), TypeError> {
        match pattern {
            Pattern::Identifier(name) => {
                // Simple identifier pattern - bind the variable to the value type
                self.context.variables.insert(name.clone(), value_type.clone());
                Ok(())
            }
            Pattern::Wildcard => {
                // Wildcard pattern - no variables to bind
                Ok(())
            }
            Pattern::Tuple(patterns) => {
                // Tuple pattern - extract types from tuple type
                match value_type {
                    TypeAnnotation::Tuple(types) => {
                        if patterns.len() != types.len() {
                            return Err(TypeError::TypeMismatch {
                                expected: TypeAnnotation::Tuple(types.clone()),
                                found: TypeAnnotation::Tuple(vec![TypeAnnotation::Unknown; patterns.len()]),
                                location: "tuple pattern".to_string(),
                            });
                        }
                        for (pattern, pattern_type) in patterns.iter().zip(types.iter()) {
                            self.bind_pattern_variables(pattern, pattern_type)?;
                        }
                        Ok(())
                    }
                    TypeAnnotation::Unknown => {
                        // If type is unknown, bind all pattern variables to unknown
                        for pattern in patterns {
                            self.bind_pattern_variables(pattern, &TypeAnnotation::Unknown)?;
                        }
                        Ok(())
                    }
                    _ => Err(TypeError::TypeMismatch {
                        expected: TypeAnnotation::Tuple(vec![TypeAnnotation::Unknown; patterns.len()]),
                        found: value_type.clone(),
                        location: "tuple pattern".to_string(),
                    }),
                }
            }
            Pattern::List { patterns, rest } => {
                // List pattern - extract element type from list type
                match value_type {
                    TypeAnnotation::List(element_type) => {
                        for pattern in patterns {
                            self.bind_pattern_variables(pattern, element_type)?;
                        }
                        // If there's a rest pattern, bind it to the full list type
                        if let Some(ref rest_name) = rest {
                            self.context.variables.insert(rest_name.clone(), value_type.clone());
                        }
                        Ok(())
                    }
                    TypeAnnotation::Unknown => {
                        // If type is unknown, bind all pattern variables to unknown
                        for pattern in patterns {
                            self.bind_pattern_variables(pattern, &TypeAnnotation::Unknown)?;
                        }
                        if let Some(ref rest_name) = rest {
                            self.context.variables.insert(rest_name.clone(), TypeAnnotation::Unknown);
                        }
                        Ok(())
                    }
                    _ => Err(TypeError::TypeMismatch {
                        expected: TypeAnnotation::List(Box::new(TypeAnnotation::Unknown)),
                        found: value_type.clone(),
                        location: "list pattern".to_string(),
                    }),
                }
            }
            Pattern::Struct { type_name, field_patterns } => {
                // Struct pattern - for now, bind all fields to unknown
                // TODO: Implement proper struct type checking
                for (_, field_pattern) in field_patterns {
                    self.bind_pattern_variables(field_pattern, &TypeAnnotation::Unknown)?;
                }
                Ok(())
            }
            Pattern::AnonymousStruct { field_patterns } => {
                // Anonymous struct pattern - bind all fields to unknown
                // TODO: Implement proper anonymous struct type checking
                for (_, field_pattern) in field_patterns {
                    self.bind_pattern_variables(field_pattern, &TypeAnnotation::Unknown)?;
                }
                Ok(())
            }
            Pattern::Or { alternatives } => {
                // Or pattern - all alternatives should bind the same variables
                // For now, just use the first alternative
                if let Some(first_alt) = alternatives.first() {
                    self.bind_pattern_variables(first_alt, value_type)?;
                }
                Ok(())
            }
            Pattern::Ok(inner_pattern) => {
                // Ok pattern - extract the ok type from Result type
                match value_type {
                    TypeAnnotation::Result { ok_type, .. } => {
                        self.bind_pattern_variables(inner_pattern, ok_type)?;
                    }
                    TypeAnnotation::Unknown => {
                        self.bind_pattern_variables(inner_pattern, &TypeAnnotation::Unknown)?;
                    }
                    _ => {
                        return Err(TypeError::TypeMismatch {
                            expected: TypeAnnotation::Result {
                                ok_type: Box::new(TypeAnnotation::Unknown),
                                err_type: Box::new(TypeAnnotation::Unknown),
                            },
                            found: value_type.clone(),
                            location: "Ok pattern".to_string(),
                        });
                    }
                }
                Ok(())
            }
            Pattern::Err(inner_pattern) => {
                // Err pattern - extract the error type from Result type
                match value_type {
                    TypeAnnotation::Result { err_type, .. } => {
                        self.bind_pattern_variables(inner_pattern, err_type)?;
                    }
                    TypeAnnotation::Unknown => {
                        self.bind_pattern_variables(inner_pattern, &TypeAnnotation::Unknown)?;
                    }
                    _ => {
                        return Err(TypeError::TypeMismatch {
                            expected: TypeAnnotation::Result {
                                ok_type: Box::new(TypeAnnotation::Unknown),
                                err_type: Box::new(TypeAnnotation::Unknown),
                            },
                            found: value_type.clone(),
                            location: "Err pattern".to_string(),
                        });
                    }
                }
                Ok(())
            }
            Pattern::Literal(_) => {
                // Literal pattern - no variables to bind
                Ok(())
            }
            Pattern::Range { .. } => {
                // Range pattern - no variables to bind
                Ok(())
            }
            Pattern::EnumVariant { patterns, .. } => {
                // Enum variant pattern - bind all inner patterns to unknown for now
                // TODO: Implement proper enum type checking
                for pattern in patterns {
                    self.bind_pattern_variables(pattern, &TypeAnnotation::Unknown)?;
                }
                Ok(())
            }
            Pattern::Guarded { pattern, .. } => {
                // Guarded pattern - bind variables from the inner pattern
                self.bind_pattern_variables(pattern, value_type)?;
                Ok(())
            }
            Pattern::Rest(name) => {
                // Rest pattern - bind to the value type
                self.context.variables.insert(name.clone(), value_type.clone());
                Ok(())
            }
        }
    }

    /// Type check a function declaration
    fn check_function_decl(
        &mut self,
        func_decl: &FunctionDecl,
    ) -> Result<TypeAnnotation, TypeError> {
        // Handle generic functions
        let mut type_checker = self.clone();

        // Add type parameters to scope
        for type_param in &func_decl.type_params {
            type_checker
                .context
                .type_parameters
                .push(type_param.clone());
            // Add type variable to context
            type_checker.context.type_vars.insert(
                type_param.clone(),
                TypeAnnotation::TypeVariable(type_param.clone()),
            );
        }

        // Create function type
        let param_types: Vec<TypeAnnotation> = func_decl
            .parameters
            .iter()
            .map(|p| p.type_annotation.clone().unwrap_or(TypeAnnotation::Unknown))
            .collect();

        let return_type = func_decl
            .return_type
            .clone()
            .unwrap_or(TypeAnnotation::Unknown);

        let func_type = if func_decl.type_params.is_empty() {
            // Non-generic function
            TypeAnnotation::Function {
                params: param_types.clone(),
                return_type: Box::new(return_type.clone()),
            }
        } else {
            // Generic function - for now, we'll just use a regular function type
            // In a full implementation, we'd need a way to represent generic functions
            TypeAnnotation::Function {
                params: param_types.clone(),
                return_type: Box::new(return_type.clone()),
            }
        };

        // Add function to context
        self.context
            .functions
            .insert(func_decl.name.clone(), func_type.clone());

        // Add parameters to body scope
        for (param, param_type) in func_decl.parameters.iter().zip(param_types.iter()) {
            type_checker
                .context
                .variables
                .insert(param.name.clone(), param_type.clone());
        }

        let body_type = type_checker.infer_type(&func_decl.body)?;

        // Check return type compatibility
        if return_type != TypeAnnotation::Unknown {
            type_checker.check_type_compatibility(&return_type, &body_type, &func_decl.name)?;
        }

        Ok(func_type)
    }

    /// Infer the type of an expression
    pub fn infer_type(&mut self, expr: &Expr) -> Result<TypeAnnotation, TypeError> {
        match expr {
            Expr::Integer(_) => Ok(TypeAnnotation::Int),
            Expr::Float(_) => Ok(TypeAnnotation::Float),
            Expr::String(_) => Ok(TypeAnnotation::String),
            Expr::Boolean(_) => Ok(TypeAnnotation::Bool),

            Expr::List(items) => {
                if items.is_empty() {
                    Ok(TypeAnnotation::List(Box::new(TypeAnnotation::Unknown)))
                } else {
                    let first_type = self.infer_type(&items[0])?;
                    // Check all items have the same type
                    for item in items.iter().skip(1) {
                        let item_type = self.infer_type(item)?;
                        self.check_type_compatibility(&first_type, &item_type, "list element")?;
                    }
                    Ok(TypeAnnotation::List(Box::new(first_type)))
                }
            }

            Expr::Tuple(items) => {
                if items.is_empty() {
                    Ok(TypeAnnotation::Unit)
                } else {
                    let types: Result<Vec<_>, _> =
                        items.iter().map(|item| self.infer_type(item)).collect();
                    Ok(TypeAnnotation::Tuple(types?))
                }
            }

            Expr::MapLiteral { entries } => {
                if entries.is_empty() {
                    // Empty map - use unknown key/value types
                    Ok(TypeAnnotation::Map {
                        key_type: Box::new(TypeAnnotation::Unknown),
                        value_type: Box::new(TypeAnnotation::Unknown),
                    })
                } else {
                    // Infer key and value types from first entry
                    let first_entry = &entries[0];
                    let key_type = self.infer_type(&first_entry.key)?;
                    let value_type = self.infer_type(&first_entry.value)?;

                    // Check all entries have compatible types
                    for entry in entries.iter().skip(1) {
                        let entry_key_type = self.infer_type(&entry.key)?;
                        let entry_value_type = self.infer_type(&entry.value)?;
                        
                        self.check_type_compatibility(&key_type, &entry_key_type, "map key")?;
                        self.check_type_compatibility(&value_type, &entry_value_type, "map value")?;
                    }

                    Ok(TypeAnnotation::Map {
                        key_type: Box::new(key_type),
                        value_type: Box::new(value_type),
                    })
                }
            }

            Expr::Identifier(name) => self
                .context
                .variables
                .get(name)
                .cloned()
                .ok_or_else(|| TypeError::UnknownVariable { name: name.clone() }),

            Expr::BinaryOp { left, op, right } => {
                let left_type = self.infer_type(left)?;
                let right_type = self.infer_type(right)?;
                self.infer_binary_op_type(op, &left_type, &right_type)
            }

            Expr::UnaryOp { op, operand } => {
                let operand_type = self.infer_type(operand)?;
                self.infer_unary_op_type(op, &operand_type)
            }

            Expr::Call { callee, arguments } => {
                // Convert arguments to expressions for type checking
                let arg_exprs: Vec<Expr> = arguments
                    .iter()
                    .map(|arg| match arg {
                        Argument::Positional(expr) => expr.clone(),
                        Argument::Named { value, .. } => value.clone(),
                    })
                    .collect();
                self.infer_call_type(callee, &arg_exprs)
            }

            Expr::Lambda {
                parameters,
                body,
                return_type,
            } => {
                let param_types: Vec<TypeAnnotation> = parameters
                    .iter()
                    .map(|p| p.type_annotation.clone().unwrap_or(TypeAnnotation::Unknown))
                    .collect();

                // Create new scope for lambda body
                let mut lambda_checker = self.clone();
                for (param, param_type) in parameters.iter().zip(param_types.iter()) {
                    lambda_checker
                        .context
                        .variables
                        .insert(param.name.clone(), param_type.clone());
                }

                let inferred_body_type = lambda_checker.infer_type(body)?;

                let final_return_type = if let Some(annotated_return_type) = return_type {
                    self.check_type_compatibility(
                        annotated_return_type,
                        &inferred_body_type,
                        "lambda return type",
                    )?;
                    annotated_return_type.clone()
                } else {
                    inferred_body_type
                };

                Ok(TypeAnnotation::Function {
                    params: param_types,
                    return_type: Box::new(final_return_type),
                })
            }

            Expr::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let condition_type = self.infer_type(condition)?;
                self.check_type_compatibility(
                    &TypeAnnotation::Bool,
                    &condition_type,
                    "if condition",
                )?;

                let then_type = self.infer_type(then_branch)?;

                if let Some(else_expr) = else_branch {
                    let else_type = self.infer_type(else_expr)?;
                    self.check_type_compatibility(&then_type, &else_type, "if branches")?;
                    Ok(then_type)
                } else {
                    Ok(then_type)
                }
            }

            Expr::Block(statements) => {
                if statements.is_empty() {
                    Ok(TypeAnnotation::Unknown)
                } else {
                    let mut last_type = TypeAnnotation::Unknown;
                    for statement in statements {
                        last_type = self.check_statement(statement)?;
                    }
                    Ok(last_type)
                }
            }

            Expr::Index { object, index } => self.infer_index_type(object, index),

            _ => Ok(TypeAnnotation::Unknown), // For other expressions not yet implemented
        }
    }

    /// Infer type for binary operations
    fn infer_binary_op_type(
        &self,
        op: &BinaryOp,
        left: &TypeAnnotation,
        right: &TypeAnnotation,
    ) -> Result<TypeAnnotation, TypeError> {
        match op {
            BinaryOp::Add
            | BinaryOp::Subtract
            | BinaryOp::Multiply
            | BinaryOp::Divide
            | BinaryOp::Modulo => match (left, right) {
                (TypeAnnotation::Int, TypeAnnotation::Int) => Ok(TypeAnnotation::Int),
                (TypeAnnotation::Float, TypeAnnotation::Float) => Ok(TypeAnnotation::Float),
                (TypeAnnotation::Int, TypeAnnotation::Float)
                | (TypeAnnotation::Float, TypeAnnotation::Int) => Ok(TypeAnnotation::Float),
                (TypeAnnotation::String, TypeAnnotation::String) if matches!(op, BinaryOp::Add) => {
                    Ok(TypeAnnotation::String)
                }
                _ => Err(TypeError::InvalidOperation {
                    op: format!("{:?}", op),
                    left_type: left.clone(),
                    right_type: Some(right.clone()),
                }),
            },

            BinaryOp::Equal | BinaryOp::NotEqual => {
                // Can compare any types
                Ok(TypeAnnotation::Bool)
            }

            BinaryOp::LessThan
            | BinaryOp::LessThanEqual
            | BinaryOp::GreaterThan
            | BinaryOp::GreaterThanEqual => match (left, right) {
                (TypeAnnotation::Int, TypeAnnotation::Int)
                | (TypeAnnotation::Float, TypeAnnotation::Float)
                | (TypeAnnotation::Int, TypeAnnotation::Float)
                | (TypeAnnotation::Float, TypeAnnotation::Int)
                | (TypeAnnotation::String, TypeAnnotation::String) => Ok(TypeAnnotation::Bool),
                _ => Err(TypeError::InvalidOperation {
                    op: format!("{:?}", op),
                    left_type: left.clone(),
                    right_type: Some(right.clone()),
                }),
            },

            BinaryOp::And | BinaryOp::Or => match (left, right) {
                (TypeAnnotation::Bool, TypeAnnotation::Bool) => Ok(TypeAnnotation::Bool),
                _ => Err(TypeError::InvalidOperation {
                    op: format!("{:?}", op),
                    left_type: left.clone(),
                    right_type: Some(right.clone()),
                }),
            },
        }
    }

    /// Infer type for unary operations
    fn infer_unary_op_type(
        &self,
        op: &UnaryOp,
        operand: &TypeAnnotation,
    ) -> Result<TypeAnnotation, TypeError> {
        match op {
            UnaryOp::Negate => match operand {
                TypeAnnotation::Int => Ok(TypeAnnotation::Int),
                TypeAnnotation::Float => Ok(TypeAnnotation::Float),
                _ => Err(TypeError::InvalidOperation {
                    op: format!("{:?}", op),
                    left_type: operand.clone(),
                    right_type: None,
                }),
            },

            UnaryOp::Not => match operand {
                TypeAnnotation::Bool => Ok(TypeAnnotation::Bool),
                _ => Err(TypeError::InvalidOperation {
                    op: format!("{:?}", op),
                    left_type: operand.clone(),
                    right_type: None,
                }),
            },
        }
    }

    /// Infer type for function calls
    fn infer_call_type(
        &mut self,
        callee: &Expr,
        arguments: &[Expr],
    ) -> Result<TypeAnnotation, TypeError> {
        let callee_type = self.infer_type(callee)?;

        match callee_type {
            TypeAnnotation::Function {
                params,
                return_type,
            } => {
                // Check argument count (skip for variadic functions)
                if !params.is_empty() && params.len() != arguments.len() {
                    return Err(TypeError::ArityMismatch {
                        expected: params.len(),
                        found: arguments.len(),
                        function: "function".to_string(),
                    });
                }

                // Check argument types
                for (i, (param_type, arg)) in params.iter().zip(arguments.iter()).enumerate() {
                    let arg_type = self.infer_type(arg)?;
                    if param_type != &TypeAnnotation::Unknown {
                        self.check_type_compatibility(
                            param_type,
                            &arg_type,
                            &format!("argument {}", i + 1),
                        )?;
                    }
                }

                Ok(*return_type)
            }
            _ => Err(TypeError::InvalidOperation {
                op: "call".to_string(),
                left_type: callee_type,
                right_type: None,
            }),
        }
    }

    /// Check if two types are compatible
    fn check_type_compatibility(
        &self,
        expected: &TypeAnnotation,
        found: &TypeAnnotation,
        location: &str,
    ) -> Result<(), TypeError> {
        if self.types_compatible(expected, found) {
            Ok(())
        } else {
            Err(TypeError::TypeMismatch {
                expected: expected.clone(),
                found: found.clone(),
                location: location.to_string(),
            })
        }
    }

    /// Check if two types are compatible (including Unknown)
    fn types_compatible(&self, a: &TypeAnnotation, b: &TypeAnnotation) -> bool {
        match (a, b) {
            (TypeAnnotation::Unknown, _) | (_, TypeAnnotation::Unknown) => true,
            (TypeAnnotation::Int, TypeAnnotation::Int) => true,
            (TypeAnnotation::Float, TypeAnnotation::Float) => true,
            (TypeAnnotation::String, TypeAnnotation::String) => true,
            (TypeAnnotation::Bool, TypeAnnotation::Bool) => true,
            (TypeAnnotation::Unit, TypeAnnotation::Unit) => true,
            (TypeAnnotation::List(a_inner), TypeAnnotation::List(b_inner)) => {
                self.types_compatible(a_inner, b_inner)
            }
            (
                TypeAnnotation::Map {
                    key_type: a_key,
                    value_type: a_value,
                },
                TypeAnnotation::Map {
                    key_type: b_key,
                    value_type: b_value,
                },
            ) => {
                self.types_compatible(a_key, b_key) && self.types_compatible(a_value, b_value)
            }
            (TypeAnnotation::Tuple(a_types), TypeAnnotation::Tuple(b_types)) => {
                a_types.len() == b_types.len()
                    && a_types
                        .iter()
                        .zip(b_types.iter())
                        .all(|(a, b)| self.types_compatible(a, b))
            }
            // Unit type is compatible with empty tuple
            (TypeAnnotation::Unit, TypeAnnotation::Tuple(types)) => types.is_empty(),
            (TypeAnnotation::Tuple(types), TypeAnnotation::Unit) => types.is_empty(),
            // Generic types
            (
                TypeAnnotation::Generic {
                    base_type: b1,
                    type_args: args1,
                },
                TypeAnnotation::Generic {
                    base_type: b2,
                    type_args: args2,
                },
            ) => {
                b1 == b2
                    && args1.len() == args2.len()
                    && args1
                        .iter()
                        .zip(args2.iter())
                        .all(|(a, b)| self.types_compatible(a, b))
            }
            // Type variables
            (TypeAnnotation::TypeVariable(name), _) => {
                // Check if the type variable has a constraint
                if let Some(constraint) = self.context.type_vars.get(name) {
                    self.types_compatible(constraint, b)
                } else {
                    true // Unconstrained type variable is compatible with any type
                }
            }
            (_, TypeAnnotation::TypeVariable(name)) => {
                // Check if the type variable has a constraint
                if let Some(constraint) = self.context.type_vars.get(name) {
                    self.types_compatible(a, constraint)
                } else {
                    true // Unconstrained type variable is compatible with any type
                }
            }
            // Inferred types
            (TypeAnnotation::Inferred(name), _) => {
                if let Some(inferred) = self.context.type_vars.get(name) {
                    self.types_compatible(inferred, b)
                } else {
                    true // Uninferred type is compatible with any type
                }
            }
            (_, TypeAnnotation::Inferred(name)) => {
                if let Some(inferred) = self.context.type_vars.get(name) {
                    self.types_compatible(a, inferred)
                } else {
                    true // Uninferred type is compatible with any type
                }
            }
            // Custom types
            (TypeAnnotation::Custom(name1), TypeAnnotation::Custom(name2)) => name1 == name2,
            // Allow numeric coercion
            (TypeAnnotation::Int, TypeAnnotation::Float)
            | (TypeAnnotation::Float, TypeAnnotation::Int) => true,
            _ => false,
        }
    }

    /// Infer type for index expressions  
    fn infer_index_type(
        &mut self,
        object: &Expr,
        index: &Expr,
    ) -> Result<TypeAnnotation, TypeError> {
        let object_type = self.infer_type(object)?;
        let index_type = self.infer_type(index)?;

        // Check that index is an integer
        self.check_type_compatibility(&TypeAnnotation::Int, &index_type, "index")?;

        match object_type {
            TypeAnnotation::List(inner_type) => Ok(*inner_type),
            TypeAnnotation::Tuple(types) => {
                // For tuples, we can't statically determine the type without knowing the index
                // In a more advanced type system, we'd track constant indices
                if types.is_empty() {
                    Ok(TypeAnnotation::Unknown)
                } else {
                    // Return first type as approximation - in practice this needs constant folding
                    Ok(types[0].clone())
                }
            }
            TypeAnnotation::String => Ok(TypeAnnotation::String), // String indexing returns single character
            _ => Err(TypeError::InvalidOperation {
                op: "index".to_string(),
                left_type: object_type,
                right_type: Some(index_type),
            }),
        }
    }
}

impl Clone for TypeChecker {
    fn clone(&self) -> Self {
        Self {
            context: self.context.clone(),
        }
    }
}

impl Default for TypeContext {
    fn default() -> Self {
        Self::new()
    }
}

impl TypeContext {
    pub fn new() -> Self {
        Self {
            variables: HashMap::new(),
            functions: HashMap::new(),
            type_vars: HashMap::new(),
            next_type_var: 0,
            generic_types: HashMap::new(),
            type_parameters: Vec::new(),
        }
    }
}
