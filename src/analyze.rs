use crate::ast::{Expr, MatchArm, Pattern, Program, Statement};
use crate::builtin::BuiltinFunctions;
use std::collections::{HashMap, HashSet};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AnalysisError {
    #[error("Undefined variable: {name}")]
    UndefinedVariable { name: String },
    #[error("Duplicate variable: {name}")]
    DuplicateVariable { name: String },
    #[error("Non-exhaustive pattern match")]
    NonExhaustivePatternMatch,
    #[error("Unreachable code")]
    UnreachableCode,
    #[error("Type error: {message}")]
    TypeError { message: String },
}

pub struct Analyzer {
    variables: HashMap<String, VariableInfo>,
    current_scope: usize,
    scopes: Vec<HashSet<String>>,
}

#[derive(Debug, Clone)]
pub struct VariableInfo {
    pub name: String,
    pub scope: usize,
    pub is_mutable: bool,
    pub usage_count: usize,
}

impl Default for Analyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl Analyzer {
    pub fn new() -> Self {
        let builtin_functions = BuiltinFunctions::new();
        let mut scopes = vec![HashSet::new()];
        let mut variables = HashMap::new();

        for name in builtin_functions.get_functions().keys() {
            scopes[0].insert(name.clone());
            variables.insert(
                name.clone(),
                VariableInfo {
                    name: name.clone(),
                    scope: 0,
                    is_mutable: false,
                    usage_count: 0, // Initially 0, will be incremented upon use
                },
            );
        }

        Self {
            variables,
            current_scope: 0,
            scopes,
        }
    }

    pub fn analyze_program(&mut self, program: &Program) -> Result<(), AnalysisError> {
        for statement in &program.statements {
            self.analyze_statement(statement)?;
        }
        Ok(())
    }

    fn analyze_statement(&mut self, statement: &Statement) -> Result<(), AnalysisError> {
        match statement {
            Statement::Expression(expr) => self.analyze_expr(expr),
            Statement::LetDecl(let_decl) => {
                // Analyze the value expression if present
                if let Some(value) = &let_decl.value {
                    self.analyze_expr(value)?;
                }

                // Check for duplicate variable in current scope
                if self.scopes[self.current_scope].contains(&let_decl.name) {
                    return Err(AnalysisError::DuplicateVariable {
                        name: let_decl.name.clone(),
                    });
                }

                // Add variable to current scope
                self.scopes[self.current_scope].insert(let_decl.name.clone());
                Ok(())
            }
            Statement::FunctionDecl(func_decl) => {
                // Add function to current scope
                if self.scopes[self.current_scope].contains(&func_decl.name) {
                    return Err(AnalysisError::DuplicateVariable {
                        name: func_decl.name.clone(),
                    });
                }
                self.scopes[self.current_scope].insert(func_decl.name.clone());

                // TODO: analyze function body with parameters in scope
                Ok(())
            }
            Statement::TypeDecl(_) => {
                // TODO: Implement type declaration analysis
                Ok(())
            }
            Statement::ErrorTypeDecl(_) => {
                // TODO: Implement error type declaration analysis
                Ok(())
            }
            Statement::ImportDecl(_) => {
                // TODO: Implement import analysis
                Ok(())
            }
            Statement::ExportDecl(export_decl) => {
                // Analyze the export value
                self.analyze_expr(&export_decl.value)
            }
            Statement::AsyncFunctionDecl(async_func_decl) => {
                // Add async function to current scope
                if self.scopes[self.current_scope].contains(&async_func_decl.name) {
                    return Err(AnalysisError::DuplicateVariable {
                        name: async_func_decl.name.clone(),
                    });
                }
                self.scopes[self.current_scope].insert(async_func_decl.name.clone());

                // TODO: analyze async function body with parameters in scope
                Ok(())
            }
        }
    }

    fn analyze_expr(&mut self, expr: &Expr) -> Result<(), AnalysisError> {
        match expr {
            Expr::Integer(_) | Expr::Float(_) | Expr::String(_) | Expr::Boolean(_) => Ok(()),
            Expr::List(items) => {
                for item in items.iter() {
                    self.analyze_expr(item)?;
                }
                Ok(())
            }
            Expr::Tuple(items) => {
                for item in items.iter() {
                    self.analyze_expr(item)?;
                }
                Ok(())
            }
            Expr::Identifier(name) => self.check_variable_usage(name),
            Expr::Call {
                callee: _,
                arguments: _,
            } => {
                // self.analyze_expr(callee)?;
                // for arg in arguments {
                //     self.analyze_expr(arg)?;
                // }
                Ok(())
            }
            Expr::Lambda {
                parameters, body, ..
            } => {
                self.enter_scope();
                for param in parameters {
                    // Add parameter to current scope
                    if self.scopes[self.current_scope].contains(&param.name) {
                        return Err(AnalysisError::DuplicateVariable {
                            name: param.name.clone(),
                        });
                    }
                    self.scopes[self.current_scope].insert(param.name.clone());
                }
                self.analyze_expr(body)?;
                self.exit_scope();
                Ok(())
            }
            Expr::Pipeline { left, right } => {
                self.analyze_expr(left)?;
                self.analyze_expr(right)?;
                Ok(())
            }
            Expr::Match { value, arms } => {
                self.analyze_expr(value)?;
                self.analyze_match_arms(arms)?;
                Ok(())
            }
            Expr::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.analyze_expr(condition)?;
                self.analyze_expr(then_branch)?;
                if let Some(else_expr) = else_branch {
                    self.analyze_expr(else_expr)?;
                }
                Ok(())
            }
            Expr::Block(statements) => {
                self.enter_scope();
                for statement in statements {
                    self.analyze_statement(statement)?;
                }
                self.exit_scope();
                Ok(())
            }
            Expr::BinaryOp { left, op: _, right } => {
                self.analyze_expr(left)?;
                self.analyze_expr(right)?;
                Ok(())
            }
            Expr::UnaryOp { op: _, operand } => {
                self.analyze_expr(operand)?;
                Ok(())
            }
            Expr::Range {
                start,
                end,
                inclusive: _,
            } => {
                self.analyze_expr(start)?;
                self.analyze_expr(end)?;
                Ok(())
            }
            Expr::StructLiteral(_) => {
                // TODO: Implement struct literal analysis
                Ok(())
            }
            Expr::FieldAccess { object, .. } => {
                // Analyze the object being accessed
                self.analyze_expr(object)
            }
            Expr::ResultOk(expr) => self.analyze_expr(expr),
            Expr::ResultErr(expr) => self.analyze_expr(expr),
            Expr::Try(expr) => self.analyze_expr(expr),
            Expr::TryCatch {
                try_block,
                error_var: _,
                catch_block,
            } => {
                self.analyze_expr(try_block)?;
                self.analyze_expr(catch_block)
            }
            Expr::Assignment { name: _, value } => {
                self.analyze_expr(value)?;
                // TODO: Check if variable exists for assignment vs declaration
                Ok(())
            }
            Expr::Index { object, index } => {
                self.analyze_expr(object)?;
                self.analyze_expr(index)?;
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn analyze_match_arms(&mut self, arms: &[MatchArm]) -> Result<(), AnalysisError> {
        if arms.is_empty() {
            return Err(AnalysisError::NonExhaustivePatternMatch);
        }

        let mut patterns = Vec::new();
        for arm in arms {
            patterns.push(&arm.pattern);
            self.analyze_expr(&arm.expression)?;
        }

        // TODO: Implement pattern exhaustiveness checking
        // For now, just check that we have at least one arm
        Ok(())
    }

    fn check_variable_usage(&mut self, name: &str) -> Result<(), AnalysisError> {
        // Check if variable is defined in any scope
        for scope in self.scopes.iter().rev() {
            if scope.contains(name) {
                // Update usage count
                if let Some(var_info) = self.variables.get_mut(name) {
                    var_info.usage_count += 1;
                }
                return Ok(());
            }
        }

        Err(AnalysisError::UndefinedVariable {
            name: name.to_string(),
        })
    }

    fn enter_scope(&mut self) {
        self.current_scope += 1;
        self.scopes.push(HashSet::new());
    }

    fn exit_scope(&mut self) {
        if self.current_scope > 0 {
            self.scopes.pop();
            self.current_scope -= 1;
        }
    }

    pub fn get_unused_variables(&self) -> Vec<&String> {
        self.variables
            .iter()
            .filter(|(_, info)| info.usage_count == 0)
            .map(|(name, _)| name)
            .collect()
    }

    pub fn get_undefined_variables(&self) -> Vec<String> {
        // This would be populated during analysis
        Vec::new()
    }

    pub fn check_pattern_exhaustiveness(
        &self,
        patterns: &[Pattern],
    ) -> Result<bool, AnalysisError> {
        // TODO: Implement proper pattern exhaustiveness checking
        // For now, just return true if we have at least one pattern
        Ok(!patterns.is_empty())
    }

    pub fn detect_dead_code(&mut self, _program: &Program) -> Vec<usize> {
        // TODO: Implement dead code detection
        // This would track which statements are reachable
        Vec::new()
    }
}

// Type inference (basic implementation)
pub struct TypeInferrer {
    type_env: HashMap<String, String>,
}

impl Default for TypeInferrer {
    fn default() -> Self {
        Self::new()
    }
}

impl TypeInferrer {
    pub fn new() -> Self {
        Self {
            type_env: HashMap::new(),
        }
    }

    pub fn infer_type(&mut self, expr: &Expr) -> Result<Option<String>, AnalysisError> {
        match expr {
            Expr::Integer(_) => Ok(Some("Int".to_string())),
            Expr::Float(_) => Ok(Some("Float".to_string())),
            Expr::String(_) => Ok(Some("String".to_string())),
            Expr::Boolean(_) => Ok(Some("Bool".to_string())),
            Expr::List(items) => {
                if items.is_empty() {
                    Ok(Some("List[Any]".to_string()))
                } else {
                    let mut item_types = HashSet::new();
                    for item in items.iter() {
                        if let Some(item_type) = self.infer_type(item)? {
                            item_types.insert(item_type);
                        }
                    }
                    if item_types.len() == 1 {
                        let item_type = item_types.into_iter().next().unwrap();
                        Ok(Some(format!("List[{}]", item_type)))
                    } else {
                        Ok(Some("List[Any]".to_string()))
                    }
                }
            }
            Expr::Tuple(items) => {
                let mut types = Vec::new();
                for item in items.iter() {
                    if let Some(item_type) = self.infer_type(item)? {
                        types.push(item_type);
                    } else {
                        types.push("Any".to_string());
                    }
                }
                Ok(Some(format!("Tuple[{}]", types.join(", "))))
            }
            Expr::Identifier(name) => Ok(self.type_env.get(name).cloned()),
            Expr::Call { .. } => {
                // TODO: Implement function type inference
                Ok(None)
            }
            Expr::Lambda { .. } => {
                // TODO: Implement lambda type inference
                Ok(None)
            }
            Expr::Range { .. } => {
                // Ranges always produce List[Int]
                Ok(Some("List[Int]".to_string()))
            }
            _ => Ok(None),
        }
    }
}

// Dead code detection
pub struct DeadCodeDetector {
    _reachable: HashSet<usize>,
}

impl Default for DeadCodeDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl DeadCodeDetector {
    pub fn new() -> Self {
        Self {
            _reachable: HashSet::new(),
        }
    }

    pub fn detect_dead_code(&mut self, _program: &Program) -> Vec<usize> {
        // TODO: Implement dead code detection
        // This would track which statements are reachable
        Vec::new()
    }
}
