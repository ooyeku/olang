use crate::ast::{
    Argument, BinaryOp, Expr, FunctionDecl, GenericTypeDefinition, LetDecl, Pattern, Program,
    Statement, TypeAnnotation, TypeContext, TypeDecl, TypeError, UnaryOp, Value,
};
use std::collections::{HashMap, HashSet};

/// Built-in type classes for constraints
#[derive(Debug, Clone, PartialEq)]
pub enum TypeClass {
    Numeric,    // Int, Float
    Comparable, // Int, Float, String, Bool
    Iterable,   // List, String, Range
    Equatable,  // All types except functions
    Hashable,   // Types that can be used as map keys
}

/// Type checker for Olang with enhanced type checking
pub struct TypeChecker {
    context: TypeContext,
    type_classes: HashMap<String, Vec<TypeClass>>,
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

        // Add built-in function types with improved signatures
        context.functions.insert(
            "println".to_string(),
            TypeAnnotation::Function {
                params: vec![], // Variadic - accepts any number of arguments
                return_type: Box::new(TypeAnnotation::Unit),
            },
        );

        context.functions.insert(
            "len".to_string(),
            TypeAnnotation::Function {
                params: vec![TypeAnnotation::Union {
                    types: vec![
                        TypeAnnotation::List(Box::new(TypeAnnotation::TypeVariable(
                            "T".to_string(),
                        ))),
                        TypeAnnotation::String,
                        TypeAnnotation::Map {
                            key_type: Box::new(TypeAnnotation::TypeVariable("K".to_string())),
                            value_type: Box::new(TypeAnnotation::TypeVariable("V".to_string())),
                        },
                    ],
                }],
                return_type: Box::new(TypeAnnotation::Int),
            },
        );

        context.functions.insert(
            "map".to_string(),
            TypeAnnotation::Function {
                params: vec![
                    TypeAnnotation::List(Box::new(TypeAnnotation::TypeVariable("T".to_string()))),
                    TypeAnnotation::Function {
                        params: vec![TypeAnnotation::TypeVariable("T".to_string())],
                        return_type: Box::new(TypeAnnotation::TypeVariable("U".to_string())),
                    },
                ],
                return_type: Box::new(TypeAnnotation::List(Box::new(
                    TypeAnnotation::TypeVariable("U".to_string()),
                ))),
            },
        );

        // Enhanced map functions with better type signatures
        context.functions.insert(
            "map_get".to_string(),
            TypeAnnotation::Function {
                params: vec![
                    TypeAnnotation::Map {
                        key_type: Box::new(TypeAnnotation::String),
                        value_type: Box::new(TypeAnnotation::TypeVariable("T".to_string())),
                    },
                    TypeAnnotation::String,
                ],
                return_type: Box::new(TypeAnnotation::Union {
                    types: vec![
                        TypeAnnotation::TypeVariable("T".to_string()),
                        TypeAnnotation::Unit,
                    ],
                }),
            },
        );

        context.functions.insert(
            "map_set".to_string(),
            TypeAnnotation::Function {
                params: vec![
                    TypeAnnotation::Map {
                        key_type: Box::new(TypeAnnotation::String),
                        value_type: Box::new(TypeAnnotation::TypeVariable("T".to_string())),
                    },
                    TypeAnnotation::String,
                    TypeAnnotation::TypeVariable("T".to_string()),
                ],
                return_type: Box::new(TypeAnnotation::Map {
                    key_type: Box::new(TypeAnnotation::String),
                    value_type: Box::new(TypeAnnotation::TypeVariable("T".to_string())),
                }),
            },
        );

        context.functions.insert(
            "map_has_key".to_string(),
            TypeAnnotation::Function {
                params: vec![
                    TypeAnnotation::Map {
                        key_type: Box::new(TypeAnnotation::String),
                        value_type: Box::new(TypeAnnotation::TypeVariable("T".to_string())),
                    },
                    TypeAnnotation::String,
                ],
                return_type: Box::new(TypeAnnotation::Bool),
            },
        );

        context.functions.insert(
            "map_keys".to_string(),
            TypeAnnotation::Function {
                params: vec![TypeAnnotation::Map {
                    key_type: Box::new(TypeAnnotation::String),
                    value_type: Box::new(TypeAnnotation::TypeVariable("T".to_string())),
                }],
                return_type: Box::new(TypeAnnotation::List(Box::new(TypeAnnotation::String))),
            },
        );

        context.functions.insert(
            "map_values".to_string(),
            TypeAnnotation::Function {
                params: vec![TypeAnnotation::Map {
                    key_type: Box::new(TypeAnnotation::String),
                    value_type: Box::new(TypeAnnotation::TypeVariable("T".to_string())),
                }],
                return_type: Box::new(TypeAnnotation::List(Box::new(
                    TypeAnnotation::TypeVariable("T".to_string()),
                ))),
            },
        );

        context.functions.insert(
            "map_len".to_string(),
            TypeAnnotation::Function {
                params: vec![TypeAnnotation::Map {
                    key_type: Box::new(TypeAnnotation::String),
                    value_type: Box::new(TypeAnnotation::TypeVariable("T".to_string())),
                }],
                return_type: Box::new(TypeAnnotation::Int),
            },
        );

        // Initialize type classes
        let mut type_classes = HashMap::new();

        // Numeric types
        type_classes.insert(
            "Int".to_string(),
            vec![
                TypeClass::Numeric,
                TypeClass::Comparable,
                TypeClass::Equatable,
                TypeClass::Hashable,
            ],
        );
        type_classes.insert(
            "Float".to_string(),
            vec![
                TypeClass::Numeric,
                TypeClass::Comparable,
                TypeClass::Equatable,
                TypeClass::Hashable,
            ],
        );

        // String types
        type_classes.insert(
            "String".to_string(),
            vec![
                TypeClass::Comparable,
                TypeClass::Equatable,
                TypeClass::Hashable,
                TypeClass::Iterable,
            ],
        );

        // Boolean types
        type_classes.insert(
            "Bool".to_string(),
            vec![
                TypeClass::Comparable,
                TypeClass::Equatable,
                TypeClass::Hashable,
            ],
        );

        // List types
        type_classes.insert(
            "List".to_string(),
            vec![TypeClass::Iterable, TypeClass::Equatable],
        );

        Self {
            context,
            type_classes,
        }
    }

    /// Get the type context for inspection (mainly for testing)
    pub fn get_context(&self) -> &TypeContext {
        &self.context
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
            Statement::TraitDecl(_) | Statement::ImplDecl(_) => Ok(TypeAnnotation::Unknown),
            Statement::Expression(expr) => self.infer_type(expr),
            Statement::LetDecl(let_decl) => self.check_let_decl(let_decl),
            Statement::FunctionDecl(func_decl) => self.check_function_decl(func_decl),
            Statement::TypeDecl(type_decl) => self.check_type_decl(type_decl),
            Statement::ErrorTypeDecl(_) => {
                // Error type declarations don't produce values, just register types
                Ok(TypeAnnotation::Unknown)
            }
            Statement::ShareDecl(share_decl) => {
                match share_decl {
                    crate::ast::ShareDecl::Trait(_) | crate::ast::ShareDecl::Impl(_) => {
                        Ok(crate::ast::TypeAnnotation::Unknown)
                    }
                    crate::ast::ShareDecl::Function(func_decl) => {
                        self.check_function_decl(func_decl)
                    }
                    crate::ast::ShareDecl::Let(let_decl) => {
                        if let Some(ref value) = let_decl.value {
                            let value_type = self.infer_type(value)?;
                            if let crate::ast::Pattern::Identifier(name) = &let_decl.pattern {
                                self.context
                                    .variables
                                    .insert(name.clone(), value_type.clone());
                            }
                            Ok(value_type)
                        } else {
                            Ok(TypeAnnotation::Unknown)
                        }
                    }
                    crate::ast::ShareDecl::Type(_type_decl) => {
                        // Type declarations don't have runtime values
                        Ok(TypeAnnotation::Unknown)
                    }
                    crate::ast::ShareDecl::Use(_use_decl) => {
                        // Transitive sharing doesn't produce types directly
                        Ok(TypeAnnotation::Unknown)
                    }
                }
            }
            Statement::UseDecl(_use_decl) => {
                // Use declarations import symbols but don't produce types directly
                Ok(TypeAnnotation::Unknown)
            }
            Statement::TestDecl(test_decl) => {
                // Type check test declarations - all statements in test body should be valid
                for statement in &test_decl.body {
                    self.check_statement(statement)?;
                }
                // Test declarations don't produce types directly
                Ok(TypeAnnotation::Unknown)
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

                // Type check the async function body with parameters in scope
                // Create a new type checker context for the async function body
                let mut async_checker = self.clone();

                // Add type parameters to scope for generic async functions
                for type_param in &async_func_decl.type_params {
                    async_checker
                        .context
                        .type_parameters
                        .push(type_param.clone());
                    async_checker.context.type_vars.insert(
                        type_param.clone(),
                        TypeAnnotation::TypeVariable(type_param.clone()),
                    );
                }

                // Add parameters to function body scope
                for (param, param_type) in async_func_decl.parameters.iter().zip(param_types.iter())
                {
                    async_checker
                        .context
                        .variables
                        .insert(param.name.clone(), param_type.clone());
                }

                // Type check the async function body
                let body_type = async_checker.infer_type(&async_func_decl.body)?;

                // For async functions, the body type should be compatible with the value type of the Promise
                // Extract the expected value type from the Promise return type
                let expected_body_type = match &return_type {
                    TypeAnnotation::Promise { value_type, .. } => *value_type.clone(),
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
    fn bind_pattern_variables(
        &mut self,
        pattern: &Pattern,
        value_type: &TypeAnnotation,
    ) -> Result<(), TypeError> {
        match pattern {
            Pattern::Identifier(name) => {
                // Simple identifier pattern - bind the variable to the value type
                self.context
                    .variables
                    .insert(name.clone(), value_type.clone());
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
                                found: TypeAnnotation::Tuple(vec![
                                    TypeAnnotation::Unknown;
                                    patterns.len()
                                ]),
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
                        expected: TypeAnnotation::Tuple(vec![
                            TypeAnnotation::Unknown;
                            patterns.len()
                        ]),
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
                        if let Some(rest_name) = rest {
                            self.context
                                .variables
                                .insert(rest_name.clone(), value_type.clone());
                        }
                        Ok(())
                    }
                    TypeAnnotation::Unknown => {
                        // If type is unknown, bind all pattern variables to unknown
                        for pattern in patterns {
                            self.bind_pattern_variables(pattern, &TypeAnnotation::Unknown)?;
                        }
                        if let Some(rest_name) = rest {
                            self.context
                                .variables
                                .insert(rest_name.clone(), TypeAnnotation::Unknown);
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
            Pattern::Struct {
                type_name,
                field_patterns,
            } => {
                // Enhanced struct pattern type checking
                match value_type {
                    TypeAnnotation::Custom(struct_type_name) if struct_type_name == type_name => {
                        // Type names match - bind field patterns to appropriate types
                        for (_field_name, field_pattern) in field_patterns {
                            // For now, bind to unknown since we don't have struct field type info
                            // In full implementation, would look up field types from type definitions
                            self.bind_pattern_variables(field_pattern, &TypeAnnotation::Unknown)?;
                        }
                        Ok(())
                    }
                    TypeAnnotation::Custom(_) => {
                        // Type name mismatch
                        Err(TypeError::TypeMismatch {
                            expected: TypeAnnotation::Custom(type_name.clone()),
                            found: value_type.clone(),
                            location: "struct pattern".to_string(),
                        })
                    }
                    TypeAnnotation::Unknown => {
                        // If type is unknown, bind all field patterns to unknown
                        for (_, field_pattern) in field_patterns {
                            self.bind_pattern_variables(field_pattern, &TypeAnnotation::Unknown)?;
                        }
                        Ok(())
                    }
                    _ => Err(TypeError::TypeMismatch {
                        expected: TypeAnnotation::Custom(type_name.clone()),
                        found: value_type.clone(),
                        location: "struct pattern".to_string(),
                    }),
                }
            }
            Pattern::AnonymousStruct { field_patterns } => {
                // Enhanced anonymous struct pattern type checking
                match value_type {
                    TypeAnnotation::Map {
                        value_type: map_value_type,
                        ..
                    } => {
                        // Anonymous struct pattern against map - bind fields to map value type
                        for (_, field_pattern) in field_patterns {
                            self.bind_pattern_variables(field_pattern, map_value_type)?;
                        }
                        Ok(())
                    }
                    TypeAnnotation::Custom(_) => {
                        // Anonymous struct pattern against custom type - bind to unknown
                        for (_, field_pattern) in field_patterns {
                            self.bind_pattern_variables(field_pattern, &TypeAnnotation::Unknown)?;
                        }
                        Ok(())
                    }
                    TypeAnnotation::Unknown => {
                        // If type is unknown, bind all field patterns to unknown
                        for (_, field_pattern) in field_patterns {
                            self.bind_pattern_variables(field_pattern, &TypeAnnotation::Unknown)?;
                        }
                        Ok(())
                    }
                    _ => Err(TypeError::TypeMismatch {
                        expected: TypeAnnotation::Map {
                            key_type: Box::new(TypeAnnotation::String),
                            value_type: Box::new(TypeAnnotation::Unknown),
                        },
                        found: value_type.clone(),
                        location: "anonymous struct pattern".to_string(),
                    }),
                }
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
            Pattern::EnumVariant {
                patterns,
                variant_name,
            } => {
                // Enhanced enum variant pattern type checking
                match value_type {
                    TypeAnnotation::Custom(enum_type_name) => {
                        // Look up the enum type definition
                        if let Some(type_def) = self.context.generic_types.get(enum_type_name) {
                            match &type_def.definition {
                                crate::ast::TypeDefinition::Enum { variants } => {
                                    // Find the variant by name
                                    if let Some(variant) =
                                        variants.iter().find(|v| v.name == *variant_name)
                                    {
                                        // Check if the variant has data and if patterns match
                                        match (&variant.data, patterns.len()) {
                                            (None, 0) => {
                                                // Unit variant with no patterns - OK
                                                Ok(())
                                            }
                                            (None, _) => {
                                                // Unit variant with patterns - error
                                                Err(TypeError::InvalidOperation {
                                                    op: format!(
                                                        "variant '{}' is a unit variant but patterns were provided",
                                                        variant_name
                                                    ),
                                                    left_type: value_type.clone(),
                                                    right_type: None,
                                                })
                                            }
                                            (Some(variant_types), pattern_count) => {
                                                // Tuple variant - check pattern count matches
                                                if variant_types.len() != pattern_count {
                                                    return Err(TypeError::ArityMismatch {
                                                        expected: variant_types.len(),
                                                        found: pattern_count,
                                                        function: format!(
                                                            "enum variant '{}'",
                                                            variant_name
                                                        ),
                                                    });
                                                }

                                                // Clone variant types to avoid borrow checker issues
                                                let variant_types = variant_types.clone();

                                                // Bind each pattern to its corresponding variant type
                                                for (pattern, variant_type) in
                                                    patterns.iter().zip(variant_types.iter())
                                                {
                                                    self.bind_pattern_variables(
                                                        pattern,
                                                        variant_type,
                                                    )?;
                                                }
                                                Ok(())
                                            }
                                        }
                                    } else {
                                        Err(TypeError::InvalidOperation {
                                            op: format!(
                                                "variant '{}' not found in enum '{}'",
                                                variant_name, enum_type_name
                                            ),
                                            left_type: value_type.clone(),
                                            right_type: None,
                                        })
                                    }
                                }
                                _ => Err(TypeError::InvalidOperation {
                                    op: format!("type '{}' is not an enum", enum_type_name),
                                    left_type: value_type.clone(),
                                    right_type: None,
                                }),
                            }
                        } else {
                            // Enum type not found - fall back to unknown
                            for pattern in patterns {
                                self.bind_pattern_variables(pattern, &TypeAnnotation::Unknown)?;
                            }
                            Ok(())
                        }
                    }
                    TypeAnnotation::Union { types: _ } => {
                        // Union types might contain enum variants
                        // For now, bind all inner patterns to unknown
                        for pattern in patterns {
                            self.bind_pattern_variables(pattern, &TypeAnnotation::Unknown)?;
                        }
                        Ok(())
                    }
                    TypeAnnotation::Unknown => {
                        // If type is unknown, bind all patterns to unknown
                        for pattern in patterns {
                            self.bind_pattern_variables(pattern, &TypeAnnotation::Unknown)?;
                        }
                        Ok(())
                    }
                    _ => {
                        // For non-enum types, this might be an error, but for now be permissive
                        for pattern in patterns {
                            self.bind_pattern_variables(pattern, &TypeAnnotation::Unknown)?;
                        }
                        Ok(())
                    }
                }
            }
            Pattern::Guarded { pattern, .. } => {
                // Guarded pattern - bind variables from the inner pattern
                self.bind_pattern_variables(pattern, value_type)?;
                Ok(())
            }
            Pattern::Rest(name) => {
                // Rest pattern - bind to the value type
                self.context
                    .variables
                    .insert(name.clone(), value_type.clone());
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

        // Also register it in the body-scope checker (cloned before this
        // point) so recursive calls resolve
        type_checker
            .context
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
                // Use enhanced list literal type inference
                self.infer_list_literal_type(items)
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
                // Use enhanced map literal type inference
                self.infer_map_literal_type(entries)
            }

            Expr::Identifier(name) => {
                // First try variables, then functions
                if let Some(var_type) = self.context.variables.get(name) {
                    Ok(var_type.clone())
                } else if let Some(func_type) = self.context.functions.get(name) {
                    Ok(func_type.clone())
                } else {
                    Err(TypeError::UnknownVariable { name: name.clone() })
                }
            }

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

            // Struct and object literals
            Expr::StructLiteral(struct_literal) => {
                // Enhanced struct literal type checking
                self.infer_struct_literal_type(struct_literal)
            }

            Expr::AnonymousObject { fields } => {
                // Enhanced anonymous object type checking
                self.infer_anonymous_object_type(fields)
            }

            // Result types
            Expr::ResultOk(expr) => {
                let inner_type = self.infer_type(expr)?;
                Ok(TypeAnnotation::Result {
                    ok_type: Box::new(inner_type),
                    err_type: Box::new(TypeAnnotation::Unknown),
                })
            }

            Expr::ResultErr(expr) => {
                let inner_type = self.infer_type(expr)?;
                Ok(TypeAnnotation::Result {
                    ok_type: Box::new(TypeAnnotation::Unknown),
                    err_type: Box::new(inner_type),
                })
            }

            // Range expressions
            Expr::Range { start, end, .. } => {
                let start_type = self.infer_type(start)?;
                let end_type = self.infer_type(end)?;

                // Ensure both start and end are integers
                self.check_type_compatibility(&TypeAnnotation::Int, &start_type, "range start")?;
                self.check_type_compatibility(&TypeAnnotation::Int, &end_type, "range end")?;

                Ok(TypeAnnotation::Range {
                    start: Box::new(start_type),
                    end: Box::new(end_type),
                    inclusive: true, // Default to inclusive
                })
            }

            // Field access
            Expr::FieldAccess { object, field } => {
                let object_type = self.infer_type(object)?;
                self.infer_field_access_type(&object_type, field)
            }

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
    pub fn check_type_compatibility(
        &self,
        expected: &TypeAnnotation,
        found: &TypeAnnotation,
        location: &str,
    ) -> Result<(), TypeError> {
        if self.types_compatible(expected, found) {
            Ok(())
        } else {
            Err(self.create_enhanced_error(expected, found, location))
        }
    }

    /// Check if two types are compatible (including Unknown)
    pub fn types_compatible(&self, a: &TypeAnnotation, b: &TypeAnnotation) -> bool {
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
            // Range types
            (
                TypeAnnotation::Range {
                    start: start1,
                    end: end1,
                    inclusive: _,
                },
                TypeAnnotation::Range {
                    start: start2,
                    end: end2,
                    inclusive: _,
                },
            ) => self.types_compatible(start1, start2) && self.types_compatible(end1, end2),
            (
                TypeAnnotation::Map {
                    key_type: a_key,
                    value_type: a_value,
                },
                TypeAnnotation::Map {
                    key_type: b_key,
                    value_type: b_value,
                },
            ) => self.types_compatible(a_key, b_key) && self.types_compatible(a_value, b_value),
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
            // Union types - two union types are compatible if they have the same set of types
            (TypeAnnotation::Union { types: types1 }, TypeAnnotation::Union { types: types2 }) => {
                types1.len() == types2.len()
                    && types1
                        .iter()
                        .all(|t1| types2.iter().any(|t2| self.types_compatible(t1, t2)))
                    && types2
                        .iter()
                        .all(|t2| types1.iter().any(|t1| self.types_compatible(t1, t2)))
            }
            // Union types - a is compatible with union if it's compatible with any member
            (a, TypeAnnotation::Union { types }) => types
                .iter()
                .any(|union_type| self.types_compatible(a, union_type)),
            (TypeAnnotation::Union { types }, b) => types
                .iter()
                .any(|union_type| self.types_compatible(union_type, b)),
            // Intersection types - a is compatible with intersection if it's compatible with all members
            (a, TypeAnnotation::Intersection { types }) => types
                .iter()
                .all(|intersection_type| self.types_compatible(a, intersection_type)),
            (TypeAnnotation::Intersection { types }, b) => types
                .iter()
                .all(|intersection_type| self.types_compatible(intersection_type, b)),
            // Literal types
            (TypeAnnotation::Literal { value: val1 }, TypeAnnotation::Literal { value: val2 }) => {
                self.values_equal(val1, val2)
            }
            // Literal types are compatible with their base types
            (TypeAnnotation::Literal { value }, type_ann) => {
                let base_type = self.value_to_type(value);
                self.types_compatible(&base_type, type_ann)
            }
            (type_ann, TypeAnnotation::Literal { value }) => {
                let base_type = self.value_to_type(value);
                self.types_compatible(type_ann, &base_type)
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
            // Result types
            (
                TypeAnnotation::Result {
                    ok_type: ok1,
                    err_type: err1,
                },
                TypeAnnotation::Result {
                    ok_type: ok2,
                    err_type: err2,
                },
            ) => self.types_compatible(ok1, ok2) && self.types_compatible(err1, err2),
            // Promise types
            (
                TypeAnnotation::Promise {
                    value_type: val1,
                    error_type: err1,
                },
                TypeAnnotation::Promise {
                    value_type: val2,
                    error_type: err2,
                },
            ) => {
                self.types_compatible(val1, val2)
                    && match (err1, err2) {
                        (Some(e1), Some(e2)) => self.types_compatible(e1, e2),
                        (None, None) => true,
                        _ => false,
                    }
            }
            // Function types
            (
                TypeAnnotation::Function {
                    params: p1,
                    return_type: r1,
                },
                TypeAnnotation::Function {
                    params: p2,
                    return_type: r2,
                },
            ) => {
                p1.len() == p2.len()
                    && p1
                        .iter()
                        .zip(p2.iter())
                        .all(|(a, b)| self.types_compatible(a, b))
                    && self.types_compatible(r1, r2)
            }
            // Allow numeric coercion only for basic arithmetic operations
            // For strict type checking in complex types, we don't allow coercion
            (TypeAnnotation::Int, TypeAnnotation::Float)
            | (TypeAnnotation::Float, TypeAnnotation::Int) => false,
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

    /// Check if two values are equal for literal type comparison
    fn values_equal(&self, val1: &Value, val2: &Value) -> bool {
        match (val1, val2) {
            (Value::Integer(i1), Value::Integer(i2)) => i1 == i2,
            (Value::Float(f1), Value::Float(f2)) => (f1 - f2).abs() < f64::EPSILON,
            (Value::String(s1), Value::String(s2)) => s1 == s2,
            (Value::Boolean(b1), Value::Boolean(b2)) => b1 == b2,
            (Value::Unit, Value::Unit) => true,
            _ => false,
        }
    }

    /// Convert a value to its corresponding type
    fn value_to_type(&self, value: &Value) -> TypeAnnotation {
        match value {
            Value::Integer(_) => TypeAnnotation::Int,
            Value::Float(_) => TypeAnnotation::Float,
            Value::String(_) => TypeAnnotation::String,
            Value::Boolean(_) => TypeAnnotation::Bool,
            Value::Unit => TypeAnnotation::Unit,
            Value::List(values) => {
                if values.is_empty() {
                    TypeAnnotation::List(Box::new(TypeAnnotation::Unknown))
                } else {
                    let first_type = self.value_to_type(&values[0]);
                    TypeAnnotation::List(Box::new(first_type))
                }
            }
            Value::Tuple(values) => {
                let types: Vec<TypeAnnotation> =
                    values.iter().map(|v| self.value_to_type(v)).collect();
                TypeAnnotation::Tuple(types)
            }
            Value::Map(_) => TypeAnnotation::Map {
                key_type: Box::new(TypeAnnotation::String),
                value_type: Box::new(TypeAnnotation::Unknown),
            },
            Value::Struct { type_name, .. } => TypeAnnotation::Custom(type_name.clone()),
            Value::Function(_) => TypeAnnotation::Function {
                params: vec![],
                return_type: Box::new(TypeAnnotation::Unknown),
            },
            Value::Ok(inner) => TypeAnnotation::Result {
                ok_type: Box::new(self.value_to_type(inner)),
                err_type: Box::new(TypeAnnotation::Unknown),
            },
            Value::Err(inner) => TypeAnnotation::Result {
                ok_type: Box::new(TypeAnnotation::Unknown),
                err_type: Box::new(self.value_to_type(inner)),
            },
            _ => TypeAnnotation::Unknown,
        }
    }

    /// Check if a type satisfies a type constraint
    pub fn check_type_constraint(&self, type_ann: &TypeAnnotation, constraint: &TypeClass) -> bool {
        match type_ann {
            TypeAnnotation::Int => matches!(
                constraint,
                TypeClass::Numeric
                    | TypeClass::Comparable
                    | TypeClass::Equatable
                    | TypeClass::Hashable
            ),
            TypeAnnotation::Float => matches!(
                constraint,
                TypeClass::Numeric
                    | TypeClass::Comparable
                    | TypeClass::Equatable
                    | TypeClass::Hashable
            ),
            TypeAnnotation::String => matches!(
                constraint,
                TypeClass::Comparable
                    | TypeClass::Equatable
                    | TypeClass::Hashable
                    | TypeClass::Iterable
            ),
            TypeAnnotation::Bool => matches!(
                constraint,
                TypeClass::Comparable | TypeClass::Equatable | TypeClass::Hashable
            ),
            TypeAnnotation::List(_) => {
                matches!(constraint, TypeClass::Iterable | TypeClass::Equatable)
            }
            TypeAnnotation::Map { .. } => matches!(constraint, TypeClass::Equatable),
            TypeAnnotation::Range { .. } => matches!(constraint, TypeClass::Iterable),
            TypeAnnotation::Union { types } => types
                .iter()
                .all(|t| self.check_type_constraint(t, constraint)),
            TypeAnnotation::Intersection { types } => types
                .iter()
                .any(|t| self.check_type_constraint(t, constraint)),
            TypeAnnotation::TypeVariable(name) => {
                if let Some(types) = self.type_classes.get(name) {
                    types.contains(constraint)
                } else {
                    false
                }
            }
            // Unknown types satisfy any constraint (gradual typing) — rejecting
            // them made valid code like `fn f(k) { {k: 1} }` fail the map-key
            // Hashable check
            TypeAnnotation::Unknown => true,
            _ => false,
        }
    }

    /// Create a union type from multiple types, simplifying if possible
    pub fn create_union_type(&self, types: Vec<TypeAnnotation>) -> TypeAnnotation {
        if types.is_empty() {
            return TypeAnnotation::Unknown;
        }

        if types.len() == 1 {
            return types[0].clone();
        }

        // Remove duplicates and simplify
        let mut unique_types = Vec::new();
        let mut seen = HashSet::new();

        for type_ann in types {
            let type_str = format!("{:?}", type_ann);
            if !seen.contains(&type_str) {
                seen.insert(type_str);
                unique_types.push(type_ann);
            }
        }

        if unique_types.len() == 1 {
            unique_types[0].clone()
        } else {
            TypeAnnotation::Union {
                types: unique_types,
            }
        }
    }

    /// Create an intersection type from multiple types, simplifying if possible
    pub fn create_intersection_type(&self, types: Vec<TypeAnnotation>) -> TypeAnnotation {
        if types.is_empty() {
            return TypeAnnotation::Unknown;
        }

        if types.len() == 1 {
            return types[0].clone();
        }

        // Remove duplicates and simplify
        let mut unique_types = Vec::new();
        let mut seen = HashSet::new();

        for type_ann in types {
            let type_str = format!("{:?}", type_ann);
            if !seen.contains(&type_str) {
                seen.insert(type_str);
                unique_types.push(type_ann);
            }
        }

        if unique_types.len() == 1 {
            unique_types[0].clone()
        } else {
            TypeAnnotation::Intersection {
                types: unique_types,
            }
        }
    }

    /// Generate better error messages with suggestions
    fn create_enhanced_error(
        &self,
        expected: &TypeAnnotation,
        found: &TypeAnnotation,
        location: &str,
    ) -> TypeError {
        let suggestion = self.suggest_type_fix(expected, found);

        TypeError::TypeMismatch {
            expected: expected.clone(),
            found: found.clone(),
            location: if suggestion.is_empty() {
                location.to_string()
            } else {
                format!("{} (suggestion: {})", location, suggestion)
            },
        }
    }

    /// Suggest fixes for common type errors
    fn suggest_type_fix(&self, expected: &TypeAnnotation, found: &TypeAnnotation) -> String {
        match (expected, found) {
            (TypeAnnotation::Int, TypeAnnotation::Float) => {
                "try using an integer literal or cast to int".to_string()
            }
            (TypeAnnotation::Float, TypeAnnotation::Int) => {
                "try using a float literal or cast to float".to_string()
            }
            (TypeAnnotation::String, TypeAnnotation::Int) => {
                "try using string interpolation or .to_string()".to_string()
            }
            (TypeAnnotation::List(_), TypeAnnotation::Tuple(_)) => {
                "try using list syntax [a, b, c] instead of tuple syntax (a, b, c)".to_string()
            }
            (TypeAnnotation::Tuple(_), TypeAnnotation::List(_)) => {
                "try using tuple syntax (a, b, c) instead of list syntax [a, b, c]".to_string()
            }
            (TypeAnnotation::Function { .. }, _) => "try calling the function with ()".to_string(),
            (TypeAnnotation::Union { types }, found_type) => {
                let compatible_types: Vec<String> = types
                    .iter()
                    .filter(|t| self.types_compatible(t, found_type))
                    .map(|t| format!("{:?}", t))
                    .collect();
                if !compatible_types.is_empty() {
                    format!(
                        "found type is compatible with: {}",
                        compatible_types.join(", ")
                    )
                } else {
                    format!(
                        "expected one of: {}",
                        types
                            .iter()
                            .map(|t| format!("{:?}", t))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                }
            }
            _ => String::new(),
        }
    }

    /// Enhanced type inference for map literals
    fn infer_map_literal_type(
        &mut self,
        entries: &[crate::ast::MapEntry],
    ) -> Result<TypeAnnotation, TypeError> {
        if entries.is_empty() {
            return Ok(TypeAnnotation::Map {
                key_type: Box::new(TypeAnnotation::String),
                value_type: Box::new(TypeAnnotation::Unknown),
            });
        }

        let mut value_types = Vec::new();

        for entry in entries {
            let key_type = self.infer_type(&entry.key)?;
            let value_type = self.infer_type(&entry.value)?;

            // Check that key is hashable
            if !self.check_type_constraint(&key_type, &TypeClass::Hashable) {
                return Err(TypeError::InvalidOperation {
                    op: "map key".to_string(),
                    left_type: key_type,
                    right_type: None,
                });
            }

            value_types.push(value_type);
        }

        // Create union type for values if they're different
        let value_type = if value_types.len() == 1 {
            value_types[0].clone()
        } else {
            self.create_union_type(value_types)
        };

        Ok(TypeAnnotation::Map {
            key_type: Box::new(TypeAnnotation::String),
            value_type: Box::new(value_type),
        })
    }

    /// Enhanced type inference for list literals
    fn infer_list_literal_type(&mut self, elements: &[Expr]) -> Result<TypeAnnotation, TypeError> {
        if elements.is_empty() {
            return Ok(TypeAnnotation::List(Box::new(TypeAnnotation::Unknown)));
        }

        let mut element_types = Vec::new();

        for element in elements {
            let element_type = self.infer_type(element)?;
            element_types.push(element_type);
        }

        // Create union type for elements if they're different
        let element_type = if element_types.len() == 1 {
            element_types[0].clone()
        } else {
            self.create_union_type(element_types)
        };

        Ok(TypeAnnotation::List(Box::new(element_type)))
    }

    /// Enhanced type inference for struct literals
    fn infer_struct_literal_type(
        &mut self,
        struct_literal: &crate::ast::StructLiteral,
    ) -> Result<TypeAnnotation, TypeError> {
        let type_name = &struct_literal.type_name;

        // Look up the struct type definition
        if let Some(type_def) = self.context.generic_types.get(type_name) {
            match &type_def.definition {
                crate::ast::TypeDefinition::Struct {
                    fields: struct_fields,
                } => {
                    // Check that all required fields are present
                    for struct_field in struct_fields {
                        if !struct_literal
                            .fields
                            .iter()
                            .any(|f| f.name == struct_field.name)
                        {
                            return Err(TypeError::InvalidOperation {
                                op: format!(
                                    "missing field '{}' in struct literal for type '{}'",
                                    struct_field.name, type_name
                                ),
                                left_type: TypeAnnotation::Unknown,
                                right_type: None,
                            });
                        }
                    }

                    // Clone struct fields to avoid borrow checker issues
                    let struct_fields = struct_fields.clone();

                    // Check that provided fields exist and have correct types
                    for field in &struct_literal.fields {
                        if let Some(struct_field) =
                            struct_fields.iter().find(|f| f.name == field.name)
                        {
                            let field_type = self.infer_type(&field.value)?;
                            if !self.types_compatible(&field_type, &struct_field.field_type) {
                                return Err(TypeError::TypeMismatch {
                                    expected: struct_field.field_type.clone(),
                                    found: field_type,
                                    location: format!(
                                        "field '{}' in struct '{}'",
                                        field.name, type_name
                                    ),
                                });
                            }
                        } else {
                            return Err(TypeError::InvalidOperation {
                                op: format!(
                                    "unknown field '{}' in struct literal for type '{}'",
                                    field.name, type_name
                                ),
                                left_type: TypeAnnotation::Unknown,
                                right_type: None,
                            });
                        }
                    }

                    // All validation passed, return the custom type
                    Ok(TypeAnnotation::Custom(type_name.clone()))
                }
                _ => Err(TypeError::InvalidOperation {
                    op: format!("type '{}' is not a struct", type_name),
                    left_type: TypeAnnotation::Unknown,
                    right_type: None,
                }),
            }
        } else {
            // If type not found, fall back to creating a representation for anonymous struct
            let mut field_types = Vec::new();

            for field in &struct_literal.fields {
                let field_type = self.infer_type(&field.value)?;
                field_types.push((field.name.clone(), field_type));
            }

            let field_types_str = field_types
                .iter()
                .map(|(name, type_ann)| format!("{}: {:?}", name, type_ann))
                .collect::<Vec<_>>()
                .join(", ");

            Ok(TypeAnnotation::Custom(format!("{{ {} }}", field_types_str)))
        }
    }

    /// Enhanced type inference for anonymous objects
    fn infer_anonymous_object_type(
        &mut self,
        fields: &[crate::ast::FieldValue],
    ) -> Result<TypeAnnotation, TypeError> {
        let mut field_types = Vec::new();

        for field in fields {
            let field_type = self.infer_type(&field.value)?;
            field_types.push((field.name.clone(), field_type));
        }

        let field_types: Vec<(String, TypeAnnotation)> = field_types;
        let field_types_str = field_types
            .iter()
            .map(|(name, type_ann)| format!("{}: {:?}", name, type_ann))
            .collect::<Vec<_>>()
            .join(", ");

        Ok(TypeAnnotation::Custom(format!("{{ {} }}", field_types_str)))
    }

    /// Infer type for field access
    fn infer_field_access_type(
        &mut self,
        object_type: &TypeAnnotation,
        field: &str,
    ) -> Result<TypeAnnotation, TypeError> {
        match object_type {
            TypeAnnotation::Map {
                key_type,
                value_type,
            } => {
                if **key_type == TypeAnnotation::String {
                    Ok((**value_type).clone())
                } else {
                    Err(TypeError::InvalidOperation {
                        op: "field access on non-string key map".to_string(),
                        left_type: object_type.clone(),
                        right_type: None,
                    })
                }
            }
            TypeAnnotation::Custom(type_name) => {
                // Look up the custom type definition in our context
                if let Some(type_def) = self.context.generic_types.get(type_name) {
                    match &type_def.definition {
                        crate::ast::TypeDefinition::Struct { fields } => {
                            // Search for the field in the struct
                            for struct_field in fields {
                                if struct_field.name == field {
                                    return Ok(struct_field.field_type.clone());
                                }
                            }
                            // Field not found in struct
                            Err(TypeError::InvalidOperation {
                                op: format!(
                                    "field '{}' not found in struct '{}'",
                                    field, type_name
                                ),
                                left_type: object_type.clone(),
                                right_type: None,
                            })
                        }
                        crate::ast::TypeDefinition::Enum { variants: _ } => {
                            // For enums, field access typically doesn't make sense
                            // unless it's a method call or associated constant
                            Err(TypeError::InvalidOperation {
                                op: format!(
                                    "field access on enum type '{}' is not supported",
                                    type_name
                                ),
                                left_type: object_type.clone(),
                                right_type: None,
                            })
                        }
                        crate::ast::TypeDefinition::Union { types: _ } => {
                            // For union types, field access is complex and depends on the specific variant
                            // For now, we'll return an error as this requires more sophisticated handling
                            Err(TypeError::InvalidOperation {
                                op: format!(
                                    "field access on union type '{}' is not supported",
                                    type_name
                                ),
                                left_type: object_type.clone(),
                                right_type: None,
                            })
                        }
                    }
                } else {
                    // Custom type not found in context
                    Err(TypeError::InvalidOperation {
                        op: format!("unknown custom type '{}'", type_name),
                        left_type: object_type.clone(),
                        right_type: None,
                    })
                }
            }
            _ => Err(TypeError::InvalidOperation {
                op: "field access".to_string(),
                left_type: object_type.clone(),
                right_type: None,
            }),
        }
    }
}

impl Clone for TypeChecker {
    fn clone(&self) -> Self {
        Self {
            context: self.context.clone(),
            type_classes: self.type_classes.clone(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::*;

    #[test]
    fn test_struct_literal_type_inference() {
        let mut type_checker = TypeChecker::new();

        // Create a struct literal: { name: "Alice", age: 30 }
        let struct_literal = StructLiteral {
            type_name: "Person".to_string(),
            fields: vec![
                FieldValue {
                    name: "name".to_string(),
                    value: Expr::String("Alice".to_string().into()),
                },
                FieldValue {
                    name: "age".to_string(),
                    value: Expr::Integer(30),
                },
            ],
        };

        let result = type_checker.infer_struct_literal_type(&struct_literal);
        assert!(result.is_ok());

        let inferred_type = result.unwrap();
        match inferred_type {
            TypeAnnotation::Custom(type_name) => {
                assert!(type_name.contains("name: String"));
                assert!(type_name.contains("age: Int"));
            }
            _ => panic!("Expected custom type annotation"),
        }
    }

    #[test]
    fn test_anonymous_object_type_inference() {
        let mut type_checker = TypeChecker::new();

        // Create anonymous object fields: { x: 10, y: 20.5 }
        let fields = vec![
            FieldValue {
                name: "x".to_string(),
                value: Expr::Integer(10),
            },
            FieldValue {
                name: "y".to_string(),
                value: Expr::Float(20.5),
            },
        ];

        let result = type_checker.infer_anonymous_object_type(&fields);
        assert!(result.is_ok());

        let inferred_type = result.unwrap();
        match inferred_type {
            TypeAnnotation::Custom(type_name) => {
                assert!(type_name.contains("x: Int"));
                assert!(type_name.contains("y: Float"));
            }
            _ => panic!("Expected custom type annotation"),
        }
    }

    #[test]
    fn test_field_access_type_inference() {
        let mut type_checker = TypeChecker::new();

        // Test field access on map type
        let map_type = TypeAnnotation::Map {
            key_type: Box::new(TypeAnnotation::String),
            value_type: Box::new(TypeAnnotation::Int),
        };

        let result = type_checker.infer_field_access_type(&map_type, "field");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), TypeAnnotation::Int);

        // Test field access on unknown custom type should return error
        let custom_type = TypeAnnotation::Custom("MyStruct".to_string());
        let result = type_checker.infer_field_access_type(&custom_type, "field");
        assert!(result.is_err());

        // Test field access on unsupported type
        let int_type = TypeAnnotation::Int;
        let result = type_checker.infer_field_access_type(&int_type, "field");
        assert!(result.is_err());
    }

    #[test]
    fn test_enhanced_pattern_matching_struct() {
        let mut type_checker = TypeChecker::new();

        // Test struct pattern matching
        let struct_pattern = Pattern::Struct {
            type_name: "Person".to_string(),
            field_patterns: vec![
                ("name".to_string(), Pattern::Identifier("n".to_string())),
                ("age".to_string(), Pattern::Identifier("a".to_string())),
            ],
        };

        let custom_type = TypeAnnotation::Custom("Person".to_string());
        let result = type_checker.bind_pattern_variables(&struct_pattern, &custom_type);
        assert!(result.is_ok());

        // Check that variables were bound
        assert!(type_checker.context.variables.contains_key("n"));
        assert!(type_checker.context.variables.contains_key("a"));

        // Test type mismatch
        let wrong_type = TypeAnnotation::Custom("Animal".to_string());
        let result = type_checker.bind_pattern_variables(&struct_pattern, &wrong_type);
        assert!(result.is_err());
    }

    #[test]
    fn test_enhanced_pattern_matching_anonymous_struct() {
        let mut type_checker = TypeChecker::new();

        // Test anonymous struct pattern matching
        let anonymous_pattern = Pattern::AnonymousStruct {
            field_patterns: vec![
                ("x".to_string(), Pattern::Identifier("x_val".to_string())),
                ("y".to_string(), Pattern::Identifier("y_val".to_string())),
            ],
        };

        let map_type = TypeAnnotation::Map {
            key_type: Box::new(TypeAnnotation::String),
            value_type: Box::new(TypeAnnotation::Int),
        };

        let result = type_checker.bind_pattern_variables(&anonymous_pattern, &map_type);
        assert!(result.is_ok());

        // Check that variables were bound with correct types
        assert!(type_checker.context.variables.contains_key("x_val"));
        assert!(type_checker.context.variables.contains_key("y_val"));
        assert_eq!(
            type_checker.context.variables.get("x_val"),
            Some(&TypeAnnotation::Int)
        );
        assert_eq!(
            type_checker.context.variables.get("y_val"),
            Some(&TypeAnnotation::Int)
        );
    }

    #[test]
    fn test_enhanced_pattern_matching_enum() {
        let mut type_checker = TypeChecker::new();

        // Test enum variant pattern matching
        let enum_pattern = Pattern::EnumVariant {
            variant_name: "Some".to_string(),
            patterns: vec![Pattern::Identifier("value".to_string())],
        };

        let option_type = TypeAnnotation::Custom("Option".to_string());
        let result = type_checker.bind_pattern_variables(&enum_pattern, &option_type);
        assert!(result.is_ok());

        // Check that variable was bound
        assert!(type_checker.context.variables.contains_key("value"));

        // Test with union type
        let union_type = TypeAnnotation::Union {
            types: vec![TypeAnnotation::Int, TypeAnnotation::String],
        };
        let result = type_checker.bind_pattern_variables(&enum_pattern, &union_type);
        assert!(result.is_ok());
    }

    #[test]
    fn test_result_type_inference() {
        let mut type_checker = TypeChecker::new();

        // Test Result Ok type inference
        let ok_expr = Expr::ResultOk(Box::new(Expr::Integer(42)));
        let result = type_checker.infer_type(&ok_expr);
        assert!(result.is_ok());

        match result.unwrap() {
            TypeAnnotation::Result { ok_type, err_type } => {
                assert_eq!(*ok_type, TypeAnnotation::Int);
                assert_eq!(*err_type, TypeAnnotation::Unknown);
            }
            _ => panic!("Expected Result type annotation"),
        }

        // Test Result Err type inference
        let err_expr = Expr::ResultErr(Box::new(Expr::String("error".to_string().into())));
        let result = type_checker.infer_type(&err_expr);
        assert!(result.is_ok());

        match result.unwrap() {
            TypeAnnotation::Result { ok_type, err_type } => {
                assert_eq!(*ok_type, TypeAnnotation::Unknown);
                assert_eq!(*err_type, TypeAnnotation::String);
            }
            _ => panic!("Expected Result type annotation"),
        }
    }

    #[test]
    fn test_range_type_inference() {
        let mut type_checker = TypeChecker::new();

        // Test range type inference
        let range_expr = Expr::Range {
            start: Box::new(Expr::Integer(1)),
            end: Box::new(Expr::Integer(10)),
            inclusive: true,
        };

        let result = type_checker.infer_type(&range_expr);
        assert!(result.is_ok());

        match result.unwrap() {
            TypeAnnotation::Range {
                start,
                end,
                inclusive,
            } => {
                assert_eq!(*start, TypeAnnotation::Int);
                assert_eq!(*end, TypeAnnotation::Int);
                assert!(inclusive);
            }
            _ => panic!("Expected Range type annotation"),
        }
    }

    #[test]
    fn test_field_access_expression_type_inference() {
        let mut type_checker = TypeChecker::new();

        // Test field access on map
        let field_access = Expr::FieldAccess {
            object: Box::new(Expr::Identifier("map_var".to_string())),
            field: "key".to_string(),
        };

        // Set up map variable
        type_checker.context.variables.insert(
            "map_var".to_string(),
            TypeAnnotation::Map {
                key_type: Box::new(TypeAnnotation::String),
                value_type: Box::new(TypeAnnotation::Float),
            },
        );

        let result = type_checker.infer_type(&field_access);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), TypeAnnotation::Float);
    }

    #[test]
    fn test_complex_nested_expressions() {
        let mut type_checker = TypeChecker::new();

        // Test nested struct literal with field access
        let nested_expr = Expr::StructLiteral(StructLiteral {
            type_name: "Point".to_string(),
            fields: vec![
                FieldValue {
                    name: "x".to_string(),
                    value: Expr::BinaryOp {
                        left: Box::new(Expr::Integer(10)),
                        op: BinaryOp::Add,
                        right: Box::new(Expr::Integer(5)),
                    },
                },
                FieldValue {
                    name: "y".to_string(),
                    value: Expr::Float(2.5),
                },
            ],
        });

        let result = type_checker.infer_type(&nested_expr);
        assert!(result.is_ok());

        match result.unwrap() {
            TypeAnnotation::Custom(type_name) => {
                assert!(type_name.contains("x: Int"));
                assert!(type_name.contains("y: Float"));
            }
            _ => panic!("Expected custom type annotation"),
        }
    }
}
