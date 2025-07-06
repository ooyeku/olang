use crate::ast::{Expr, MatchArm, Pattern, Program, Statement, Argument, TemplatePart};
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

                // Extract variable names from the pattern
                let pattern_variables = self.extract_pattern_variables(&let_decl.pattern);
                
                // Check for duplicate variables in current scope
                for var_name in &pattern_variables {
                    if self.scopes[self.current_scope].contains(var_name) {
                        return Err(AnalysisError::DuplicateVariable {
                            name: var_name.clone(),
                        });
                    }
                }

                // Add all variables from the pattern to current scope
                for var_name in pattern_variables {
                    self.scopes[self.current_scope].insert(var_name);
                }
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
            Expr::StructLiteral(struct_lit) => {
                // Analyze all field expressions in the struct literal
                for field in &struct_lit.fields {
                    self.analyze_expr(&field.value)?;
                }
                
                // Check for duplicate field names
                let mut field_names = HashSet::new();
                for field in &struct_lit.fields {
                    if !field_names.insert(&field.name) {
                        return Err(AnalysisError::DuplicateVariable {
                            name: field.name.clone(),
                        });
                    }
                }
                
                // In a full implementation, we would also:
                // - Check if the struct type exists
                // - Check if all required fields are present
                // - Check if any extra fields are provided
                // - Validate field types against the struct definition
                
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
                catch_var: _,
                catch_block,
            } => {
                self.analyze_expr(try_block)?;
                self.analyze_expr(catch_block)
            }
            Expr::Assignment { target, value } => {
                // Analyze the value expression first
                self.analyze_expr(value)?;
                
                // Check if the target variable exists in any scope
                let mut variable_exists = false;
                for scope in self.scopes.iter().rev() {
                    if scope.contains(target) {
                        variable_exists = true;
                        break;
                    }
                }
                
                if !variable_exists {
                    // Variable doesn't exist - this is an assignment to an undefined variable
                    // In some languages this would be an error, but in Olang it might be
                    // allowed to create variables through assignment
                    // For now, we'll add it to the current scope
                    self.scopes[self.current_scope].insert(target.clone());
                    
                    // Also add to variables map for tracking
                    self.variables.insert(
                        target.clone(),
                        VariableInfo {
                            name: target.clone(),
                            scope: self.current_scope,
                            is_mutable: true, // Variables created through assignment are mutable
                            usage_count: 1,   // Count the assignment as a use
                        },
                    );
                } else {
                    // Variable exists - update usage count
                    if let Some(var_info) = self.variables.get_mut(target) {
                        var_info.usage_count += 1;
                    }
                }
                
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
            
            // Enter a new scope for pattern variables
            self.enter_scope();
            
            // Extract and bind pattern variables
            let pattern_variables = self.extract_pattern_variables(&arm.pattern);
            for var_name in pattern_variables {
                self.scopes[self.current_scope].insert(var_name);
            }
            
            // Analyze the arm expression
            self.analyze_expr(&arm.expression)?;
            
            // Exit the scope
            self.exit_scope();
        }

        // Check pattern exhaustiveness
        let pattern_refs: Vec<Pattern> = patterns.iter().map(|p| (*p).clone()).collect();
        if !self.check_pattern_exhaustiveness(&pattern_refs)? {
            return Err(AnalysisError::NonExhaustivePatternMatch);
        }

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

    /// Extract all variable names from a pattern
    fn extract_pattern_variables(&self, pattern: &Pattern) -> Vec<String> {
        let mut variables = Vec::new();
        self.collect_pattern_variables(pattern, &mut variables);
        variables
    }

    /// Recursively collect variable names from a pattern
    fn collect_pattern_variables(&self, pattern: &Pattern, variables: &mut Vec<String>) {
        match pattern {
            Pattern::Identifier(name) => {
                variables.push(name.clone());
            }
            Pattern::Wildcard => {
                // No variables to collect
            }
            Pattern::Tuple(patterns) => {
                for pattern in patterns {
                    self.collect_pattern_variables(pattern, variables);
                }
            }
            Pattern::List { patterns, rest } => {
                for pattern in patterns {
                    self.collect_pattern_variables(pattern, variables);
                }
                if let Some(rest_name) = rest {
                    variables.push(rest_name.clone());
                }
            }
            Pattern::Struct { field_patterns, .. } => {
                for (_, field_pattern) in field_patterns {
                    self.collect_pattern_variables(field_pattern, variables);
                }
            }
            Pattern::AnonymousStruct { field_patterns } => {
                for (_, field_pattern) in field_patterns {
                    self.collect_pattern_variables(field_pattern, variables);
                }
            }
            Pattern::Or { alternatives } => {
                // For or patterns, collect variables from all alternatives
                // Note: In practice, all alternatives should bind the same variables
                for alternative in alternatives {
                    self.collect_pattern_variables(alternative, variables);
                }
            }
            Pattern::Ok(inner_pattern) => {
                self.collect_pattern_variables(inner_pattern, variables);
            }
            Pattern::Err(inner_pattern) => {
                self.collect_pattern_variables(inner_pattern, variables);
            }
            Pattern::Literal(_) => {
                // No variables to collect
            }
            Pattern::Range { .. } => {
                // No variables to collect
            }
            Pattern::EnumVariant { patterns, .. } => {
                for pattern in patterns {
                    self.collect_pattern_variables(pattern, variables);
                }
            }
            Pattern::Guarded { pattern, .. } => {
                self.collect_pattern_variables(pattern, variables);
            }
            Pattern::Rest(name) => {
                variables.push(name.clone());
            }
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

    pub fn detect_dead_code(&mut self, program: &Program) -> Vec<usize> {
        let mut reachable = HashSet::new();
        
        // All top-level statements are initially reachable
        for (index, statement) in program.statements.iter().enumerate() {
            reachable.insert(index);
            self.mark_statement_reachable(statement, &mut reachable);
        }
        
        // Find unreachable statements
        let mut dead_code = Vec::new();
        for (index, _) in program.statements.iter().enumerate() {
            if !reachable.contains(&index) {
                dead_code.push(index);
            }
        }
        
        dead_code
    }

    /// Mark a statement and its contained expressions as reachable
    fn mark_statement_reachable(&mut self, statement: &Statement, reachable: &mut HashSet<usize>) {
        match statement {
            Statement::Expression(expr) => {
                self.mark_expression_reachable(expr, reachable);
            }
            Statement::LetDecl(let_decl) => {
                if let Some(value) = &let_decl.value {
                    self.mark_expression_reachable(value, reachable);
                }
            }
            Statement::FunctionDecl(func_decl) => {
                self.mark_expression_reachable(&func_decl.body, reachable);
            }
            Statement::AsyncFunctionDecl(async_func_decl) => {
                self.mark_expression_reachable(&async_func_decl.body, reachable);
            }
            Statement::ExportDecl(export_decl) => {
                self.mark_expression_reachable(&export_decl.value, reachable);
            }
            Statement::TypeDecl(_) | Statement::ErrorTypeDecl(_) | Statement::ImportDecl(_) => {
                // These don't contain expressions that can be unreachable
            }
        }
    }

    /// Mark an expression and its sub-expressions as reachable
    fn mark_expression_reachable(&mut self, expr: &Expr, reachable: &mut HashSet<usize>) {
        match expr {
            Expr::Block(statements) => {
                let mut statements_reachable = true;
                for statement in statements {
                    if statements_reachable {
                        self.mark_statement_reachable(statement, reachable);
                    }
                    
                    // Check if this statement makes subsequent statements unreachable
                    if self.is_terminating_statement(statement) {
                        statements_reachable = false;
                    }
                }
            }
            Expr::If { condition, then_branch, else_branch } => {
                self.mark_expression_reachable(condition, reachable);
                self.mark_expression_reachable(then_branch, reachable);
                if let Some(else_branch) = else_branch {
                    self.mark_expression_reachable(else_branch, reachable);
                }
            }
            Expr::Match { value, arms } => {
                self.mark_expression_reachable(value, reachable);
                for arm in arms {
                    self.mark_expression_reachable(&arm.expression, reachable);
                }
            }
            Expr::Lambda { body, .. } => {
                self.mark_expression_reachable(body, reachable);
            }
            Expr::Call { callee, arguments } => {
                self.mark_expression_reachable(callee, reachable);
                for arg in arguments {
                    match arg {
                        Argument::Positional(expr) => self.mark_expression_reachable(expr, reachable),
                        Argument::Named { value, .. } => self.mark_expression_reachable(value, reachable),
                    }
                }
            }
            Expr::Pipeline { left, right } => {
                self.mark_expression_reachable(left, reachable);
                self.mark_expression_reachable(right, reachable);
            }
            Expr::BinaryOp { left, right, .. } => {
                self.mark_expression_reachable(left, reachable);
                self.mark_expression_reachable(right, reachable);
            }
            Expr::UnaryOp { operand, .. } => {
                self.mark_expression_reachable(operand, reachable);
            }
            Expr::List(items) => {
                for item in items.iter() {
                    self.mark_expression_reachable(item, reachable);
                }
            }
            Expr::Tuple(items) => {
                for item in items.iter() {
                    self.mark_expression_reachable(item, reachable);
                }
            }
            Expr::MapLiteral { entries } => {
                for entry in entries {
                    self.mark_expression_reachable(&entry.key, reachable);
                    self.mark_expression_reachable(&entry.value, reachable);
                }
            }
            Expr::Index { object, index } => {
                self.mark_expression_reachable(object, reachable);
                self.mark_expression_reachable(index, reachable);
            }
            Expr::FieldAccess { object, .. } => {
                self.mark_expression_reachable(object, reachable);
            }
            Expr::Assignment { value, .. } => {
                self.mark_expression_reachable(value, reachable);
            }
            Expr::TryCatch { try_block, catch_block, .. } => {
                self.mark_expression_reachable(try_block, reachable);
                self.mark_expression_reachable(catch_block, reachable);
            }
            Expr::Try(expr) => {
                self.mark_expression_reachable(expr, reachable);
            }
            Expr::ResultOk(expr) => {
                self.mark_expression_reachable(expr, reachable);
            }
            Expr::ResultErr(expr) => {
                self.mark_expression_reachable(expr, reachable);
            }
            Expr::ForLoop { iterable, body, .. } => {
                self.mark_expression_reachable(iterable, reachable);
                self.mark_expression_reachable(body, reachable);
            }
            Expr::WhileLoop { condition, body } => {
                self.mark_expression_reachable(condition, reachable);
                self.mark_expression_reachable(body, reachable);
            }
            Expr::Loop { body } => {
                self.mark_expression_reachable(body, reachable);
            }
            Expr::Range { start, end, .. } => {
                self.mark_expression_reachable(start, reachable);
                self.mark_expression_reachable(end, reachable);
            }
            Expr::StructLiteral(struct_lit) => {
                for field in &struct_lit.fields {
                    self.mark_expression_reachable(&field.value, reachable);
                }
            }
            Expr::AnonymousObject { fields } => {
                for field in fields {
                    self.mark_expression_reachable(&field.value, reachable);
                }
            }
            Expr::Async { body, .. } => {
                self.mark_expression_reachable(body, reachable);
            }
            Expr::Await { expression } => {
                self.mark_expression_reachable(expression, reachable);
            }
            Expr::Promise { value, delay, .. } => {
                self.mark_expression_reachable(value, reachable);
                if let Some(delay) = delay {
                    self.mark_expression_reachable(delay, reachable);
                }
            }
            Expr::All(promises) => {
                for promise in promises {
                    self.mark_expression_reachable(promise, reachable);
                }
            }
            Expr::Race(promises) => {
                for promise in promises {
                    self.mark_expression_reachable(promise, reachable);
                }
            }
            Expr::Spawn(expr) => {
                self.mark_expression_reachable(expr, reachable);
            }
            Expr::Spread(expr) => {
                self.mark_expression_reachable(expr, reachable);
            }
            Expr::Rest(expr) => {
                self.mark_expression_reachable(expr, reachable);
            }
            Expr::TemplateString { parts } => {
                for part in parts {
                    if let TemplatePart::Interpolation(expr) = part {
                        self.mark_expression_reachable(expr, reachable);
                    }
                }
            }
            Expr::BitwiseOp { left, right, .. } => {
                self.mark_expression_reachable(left, reachable);
                self.mark_expression_reachable(right, reachable);
            }
            // Terminal expressions that don't contain other expressions
            Expr::Integer(_) | Expr::Float(_) | Expr::String(_) | Expr::Boolean(_) |
            Expr::RawString(_) | Expr::Identifier(_) | Expr::Break | Expr::Continue => {
                // These don't contain sub-expressions
            }
        }
    }

    /// Check if a statement is terminating (makes subsequent statements unreachable)
    fn is_terminating_statement(&self, statement: &Statement) -> bool {
        match statement {
            Statement::Expression(expr) => self.is_terminating_expression(expr),
            _ => false,
        }
    }

    /// Check if an expression is terminating (doesn't return control flow)
    fn is_terminating_expression(&self, expr: &Expr) -> bool {
        match expr {
            Expr::Break | Expr::Continue => true,
            Expr::Block(statements) => {
                // A block is terminating if its last statement is terminating
                if let Some(last_stmt) = statements.last() {
                    self.is_terminating_statement(last_stmt)
                } else {
                    false
                }
            }
            Expr::If { then_branch, else_branch, .. } => {
                // If is terminating if both branches are terminating
                if let Some(else_branch) = else_branch {
                    self.is_terminating_expression(then_branch) && self.is_terminating_expression(else_branch)
                } else {
                    false
                }
            }
            Expr::Match { arms, .. } => {
                // Match is terminating if all arms are terminating
                arms.iter().all(|arm| self.is_terminating_expression(&arm.expression))
            }
            _ => false,
        }
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
                        // Safe since we verified len() == 1
                        if let Some(item_type) = item_types.into_iter().next() {
                            Ok(Some(format!("List[{}]", item_type)))
                        } else {
                            // Fallback in case of unexpected empty set
                            Ok(Some("List[Any]".to_string()))
                        }
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
    reachable: HashSet<usize>,
}

impl Default for DeadCodeDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl DeadCodeDetector {
    pub fn new() -> Self {
        Self {
            reachable: HashSet::new(),
        }
    }

    pub fn detect_dead_code(&mut self, program: &Program) -> Vec<usize> {
        self.reachable.clear();
        
        // All top-level statements are initially reachable
        for (index, statement) in program.statements.iter().enumerate() {
            self.reachable.insert(index);
            self.mark_statement_reachable(statement);
        }
        
        // Find unreachable statements
        let mut dead_code = Vec::new();
        for (index, _) in program.statements.iter().enumerate() {
            if !self.reachable.contains(&index) {
                dead_code.push(index);
            }
        }
        
        dead_code
    }

    /// Mark a statement and its contained expressions as reachable
    fn mark_statement_reachable(&mut self, statement: &Statement) {
        match statement {
            Statement::Expression(expr) => {
                self.mark_expression_reachable(expr);
            }
            Statement::LetDecl(let_decl) => {
                if let Some(value) = &let_decl.value {
                    self.mark_expression_reachable(value);
                }
            }
            Statement::FunctionDecl(func_decl) => {
                self.mark_expression_reachable(&func_decl.body);
            }
            Statement::AsyncFunctionDecl(async_func_decl) => {
                self.mark_expression_reachable(&async_func_decl.body);
            }
            Statement::ExportDecl(export_decl) => {
                self.mark_expression_reachable(&export_decl.value);
            }
            Statement::TypeDecl(_) | Statement::ErrorTypeDecl(_) | Statement::ImportDecl(_) => {
                // These don't contain expressions that can be unreachable
            }
        }
    }

    /// Mark an expression and its sub-expressions as reachable
    fn mark_expression_reachable(&mut self, expr: &Expr) {
        match expr {
            Expr::Block(statements) => {
                let mut statements_reachable = true;
                for statement in statements {
                    if statements_reachable {
                        self.mark_statement_reachable(statement);
                    }
                    
                    // Check if this statement makes subsequent statements unreachable
                    if self.is_terminating_statement(statement) {
                        statements_reachable = false;
                    }
                }
            }
            Expr::If { condition, then_branch, else_branch } => {
                self.mark_expression_reachable(condition);
                self.mark_expression_reachable(then_branch);
                if let Some(else_branch) = else_branch {
                    self.mark_expression_reachable(else_branch);
                }
            }
            Expr::Match { value, arms } => {
                self.mark_expression_reachable(value);
                for arm in arms {
                    self.mark_expression_reachable(&arm.expression);
                }
            }
            Expr::Lambda { body, .. } => {
                self.mark_expression_reachable(body);
            }
            Expr::Call { callee, arguments } => {
                self.mark_expression_reachable(callee);
                for arg in arguments {
                    match arg {
                        Argument::Positional(expr) => self.mark_expression_reachable(expr),
                        Argument::Named { value, .. } => self.mark_expression_reachable(value),
                    }
                }
            }
            Expr::Pipeline { left, right } => {
                self.mark_expression_reachable(left);
                self.mark_expression_reachable(right);
            }
            Expr::BinaryOp { left, right, .. } => {
                self.mark_expression_reachable(left);
                self.mark_expression_reachable(right);
            }
            Expr::UnaryOp { operand, .. } => {
                self.mark_expression_reachable(operand);
            }
            Expr::List(items) => {
                for item in items.iter() {
                    self.mark_expression_reachable(item);
                }
            }
            Expr::Tuple(items) => {
                for item in items.iter() {
                    self.mark_expression_reachable(item);
                }
            }
            Expr::MapLiteral { entries } => {
                for entry in entries {
                    self.mark_expression_reachable(&entry.key);
                    self.mark_expression_reachable(&entry.value);
                }
            }
            Expr::StructLiteral(struct_lit) => {
                for field in &struct_lit.fields {
                    self.mark_expression_reachable(&field.value);
                }
            }
            // Add other expression types as needed
            _ => {
                // For now, just mark as reachable without recursing
                // In a full implementation, we'd handle all expression types
            }
        }
    }

    /// Check if a statement is terminating (makes subsequent statements unreachable)
    fn is_terminating_statement(&self, statement: &Statement) -> bool {
        match statement {
            Statement::Expression(expr) => self.is_terminating_expression(expr),
            _ => false,
        }
    }

    /// Check if an expression is terminating (doesn't return control flow)
    fn is_terminating_expression(&self, expr: &Expr) -> bool {
        match expr {
            Expr::Break | Expr::Continue => true,
            Expr::Block(statements) => {
                // A block is terminating if its last statement is terminating
                if let Some(last_stmt) = statements.last() {
                    self.is_terminating_statement(last_stmt)
                } else {
                    false
                }
            }
            Expr::If { then_branch, else_branch, .. } => {
                // If is terminating if both branches are terminating
                if let Some(else_branch) = else_branch {
                    self.is_terminating_expression(then_branch) && self.is_terminating_expression(else_branch)
                } else {
                    false
                }
            }
            Expr::Match { arms, .. } => {
                // Match is terminating if all arms are terminating
                arms.iter().all(|arm| self.is_terminating_expression(&arm.expression))
            }
            _ => false,
        }
    }
}
