use crate::ast::{
    Argument, Expr, MatchArm, Pattern, Program, ShareDecl, Statement, TemplatePart, TestDecl,
    UseDecl, Value,
};
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
    #[error("Non-exhaustive pattern match: missing patterns {missing_patterns:?}")]
    NonExhaustivePatternMatchWithMissing { missing_patterns: Vec<String> },
    #[error("Unreachable code")]
    UnreachableCode,
    #[error("Type error: {message}")]
    TypeError { message: String },
}

pub struct Analyzer {
    variables: HashMap<String, VariableInfo>,
    current_scope: usize,
    scopes: Vec<HashSet<String>>,
    /// Declared enums: name -> set of variant names (for exhaustiveness checks)
    enum_definitions: HashMap<String, HashSet<String>>,
    /// Names pre-registered as builtins (excluded from unused-variable reports)
    builtin_names: HashSet<String>,
    /// Outer-scope tracking entries shadowed by each open scope (parallel to
    /// scopes[1..]), restored on scope exit. Without this, exiting a scope
    /// that shadowed an outer variable deleted the outer entry too, silently
    /// corrupting usage counts and unused-variable reports.
    shadowed: Vec<Vec<(String, VariableInfo)>>,
}

#[derive(Debug, Clone)]
pub struct VariableInfo {
    pub name: String,
    pub scope: usize,
    pub is_mutable: bool,
    pub usage_count: usize,
}

/// Comprehensive analysis report containing all analysis results
#[derive(Debug, Clone)]
pub struct AnalysisReport {
    pub unused_variables: Vec<String>,
    pub overwritten_variables: Vec<String>,
    pub dead_code: Vec<usize>,
    pub unreachable_statements: Vec<(usize, usize)>, // (block_index, statement_index)
    pub variable_usage: HashMap<String, usize>,
}

impl Default for Analyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl Analyzer {
    /// Create a new analyzer with builtin functions pre-loaded
    pub fn new() -> Self {
        let builtin_functions = BuiltinFunctions::new();
        let mut scopes = vec![HashSet::new()];
        let mut variables = HashMap::new();

        let module_names = crate::stdlib::get_stdlib().into_keys().chain(
            crate::stdlib::embedded::names()
                .into_iter()
                .map(String::from),
        );
        for name in builtin_functions
            .get_functions()
            .keys()
            .cloned()
            // The stdlib module namespaces resolve at runtime exactly
            // like builtins do; without them the analyzer reported
            // "Undefined variable: math" for math.sqrt.
            .chain(module_names)
        {
            scopes[0].insert(name.clone());
            variables.insert(
                name.clone(),
                VariableInfo {
                    name,
                    scope: 0,
                    is_mutable: false,
                    usage_count: 0, // Initially 0, will be incremented upon use
                },
            );
        }

        let builtin_names = scopes[0].clone();
        Self {
            variables,
            current_scope: 0,
            scopes,
            enum_definitions: HashMap::new(),
            builtin_names,
            shadowed: Vec::new(),
        }
    }

    /// Create a new analyzer without builtin functions (for testing or custom environments)
    pub fn new_empty() -> Self {
        Self {
            variables: HashMap::new(),
            current_scope: 0,
            scopes: vec![HashSet::new()],
            enum_definitions: HashMap::new(),
            builtin_names: HashSet::new(),
            shadowed: Vec::new(),
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
            Statement::Located { stmt, .. } => self.analyze_statement(stmt),
            Statement::TraitDecl(_) | Statement::ImplDecl(_) => Ok(()),
            // Macros are gone before analysis normally runs; a raw parse
            // (LSP, meta.parse) treats them as opaque.
            Statement::MetaFnDecl { .. } | Statement::DecoratedDecl { .. } => Ok(()),
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

                // Add all variables from the pattern to current scope, with a
                // tracking entry — without one, usage counting silently
                // no-ops for every let-bound variable and they can never
                // appear in unused-variable reports
                for var_name in pattern_variables {
                    self.declare_in_scope(var_name.clone());
                    self.variables.insert(
                        var_name.clone(),
                        VariableInfo {
                            name: var_name,
                            scope: self.current_scope,
                            is_mutable: true,
                            usage_count: 0,
                        },
                    );
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
                self.declare_in_scope(func_decl.name.clone());

                // Add function to variables map
                self.variables.insert(
                    func_decl.name.clone(),
                    VariableInfo {
                        name: func_decl.name.clone(),
                        scope: self.current_scope,
                        is_mutable: false,
                        usage_count: 0,
                    },
                );

                // Analyze function body with parameters in scope
                self.enter_scope();

                // Add parameters to function scope
                for param in &func_decl.parameters {
                    if self.scopes[self.current_scope].contains(&param.name) {
                        return Err(AnalysisError::DuplicateVariable {
                            name: param.name.clone(),
                        });
                    }
                    self.declare_in_scope(param.name.clone());
                    self.variables.insert(
                        param.name.clone(),
                        VariableInfo {
                            name: param.name.clone(),
                            scope: self.current_scope,
                            is_mutable: false,
                            usage_count: 0,
                        },
                    );
                }

                // Analyze function body
                self.analyze_expr(&func_decl.body)?;

                self.exit_scope();
                Ok(())
            }
            Statement::TypeDecl(type_decl) => {
                // Add type to current scope
                if self.scopes[self.current_scope].contains(&type_decl.name) {
                    return Err(AnalysisError::DuplicateVariable {
                        name: type_decl.name.clone(),
                    });
                }
                self.declare_in_scope(type_decl.name.clone());

                // Add type to variables map (types are treated as constants)
                self.variables.insert(
                    type_decl.name.clone(),
                    VariableInfo {
                        name: type_decl.name.clone(),
                        scope: self.current_scope,
                        is_mutable: false,
                        usage_count: 0,
                    },
                );

                // Analyze type definition based on its structure
                // For now, we'll do basic validation that can be extended as needed
                match &type_decl.definition {
                    crate::ast::TypeDefinition::Struct { fields } => {
                        let mut field_names = HashSet::new();
                        for field in fields {
                            // Check for duplicate field names
                            if !field_names.insert(&field.name) {
                                return Err(AnalysisError::DuplicateVariable {
                                    name: format!(
                                        "Field '{}' in type '{}'",
                                        field.name, type_decl.name
                                    ),
                                });
                            }

                            // In a full implementation, we could analyze the field type
                            // to ensure it's valid and accessible
                        }
                    }
                    crate::ast::TypeDefinition::Enum { variants } => {
                        let mut variant_names = HashSet::new();
                        for variant in variants {
                            // Check for duplicate variant names
                            if !variant_names.insert(variant.name.clone()) {
                                return Err(AnalysisError::DuplicateVariable {
                                    name: format!(
                                        "Variant '{}' in enum '{}'",
                                        variant.name, type_decl.name
                                    ),
                                });
                            }
                        }
                        // Record for exhaustiveness checking
                        self.enum_definitions
                            .insert(type_decl.name.clone(), variant_names);
                    }
                    _ => {
                        // Handle other type definitions (Union, Alias, etc.)
                        // For now, we don't perform specific validation on these
                    }
                }

                Ok(())
            }
            Statement::ErrorTypeDecl(error_type_decl) => {
                // Add error type to current scope
                if self.scopes[self.current_scope].contains(&error_type_decl.name) {
                    return Err(AnalysisError::DuplicateVariable {
                        name: error_type_decl.name.clone(),
                    });
                }
                self.declare_in_scope(error_type_decl.name.clone());

                // Add error type to variables map
                self.variables.insert(
                    error_type_decl.name.clone(),
                    VariableInfo {
                        name: error_type_decl.name.clone(),
                        scope: self.current_scope,
                        is_mutable: false,
                        usage_count: 0,
                    },
                );

                // Analyze error variants for duplicate names
                let mut field_names = HashSet::new();
                for variant in &error_type_decl.variants {
                    // Check for duplicate variant names
                    if !field_names.insert(&variant.name) {
                        return Err(AnalysisError::DuplicateVariable {
                            name: format!(
                                "Variant '{}' in error type '{}'",
                                variant.name, error_type_decl.name
                            ),
                        });
                    }
                }

                Ok(())
            }
            Statement::ShareDecl(share_decl) => {
                // Analyze the shared declaration
                self.analyze_share_decl(share_decl)
            }
            Statement::UseDecl(use_decl) => {
                // Analyze the use declaration
                self.analyze_use_decl(use_decl)
            }
            Statement::TestDecl(test_decl) => {
                // Analyze the test declaration
                self.analyze_test_decl(test_decl)
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
            Expr::Call { callee, arguments } => {
                // Analyze the function being called
                self.analyze_expr(callee)?;

                // Analyze all arguments
                for arg in arguments {
                    match arg {
                        Argument::Positional(expr) => self.analyze_expr(expr)?,
                        Argument::Named { value, .. } => self.analyze_expr(value)?,
                    }
                }

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
                    self.declare_in_scope(param.name.clone());
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
                    self.declare_in_scope(target.clone());

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
            Expr::MapLiteral { entries } => {
                for entry in entries {
                    self.analyze_expr(&entry.key)?;
                    self.analyze_expr(&entry.value)?;
                }
                Ok(())
            }
            Expr::AnonymousObject { fields } => {
                // Check for duplicate field names
                let mut field_names = HashSet::new();
                for field in fields {
                    if !field_names.insert(&field.name) {
                        return Err(AnalysisError::DuplicateVariable {
                            name: format!("Duplicate field '{}' in anonymous object", field.name),
                        });
                    }
                    self.analyze_expr(&field.value)?;
                }
                Ok(())
            }
            Expr::ForLoop {
                variable,
                iterable,
                body,
            }
            | Expr::ParForLoop {
                variable,
                iterable,
                body,
            } => {
                // Analyze the iterable expression
                self.analyze_expr(iterable)?;

                // Enter new scope for loop variable
                self.enter_scope();

                // Add loop variable to scope
                self.declare_in_scope(variable.clone());
                self.variables.insert(
                    variable.clone(),
                    VariableInfo {
                        name: variable.clone(),
                        scope: self.current_scope,
                        is_mutable: false,
                        usage_count: 0,
                    },
                );

                // Analyze loop body
                self.analyze_expr(body)?;

                self.exit_scope();
                Ok(())
            }
            Expr::WhileLoop { condition, body } => {
                self.analyze_expr(condition)?;
                self.analyze_expr(body)?;
                Ok(())
            }
            Expr::Loop { body } => {
                self.analyze_expr(body)?;
                Ok(())
            }
            Expr::Break(value) => {
                if let Some(v) = value {
                    self.analyze_expr(v)?;
                }
                Ok(())
            }
            Expr::Continue => Ok(()),
            Expr::Return(value) => {
                if let Some(v) = value {
                    self.analyze_expr(v)?;
                }
                Ok(())
            }
            Expr::Spawn(expr) => {
                self.analyze_expr(expr)?;
                Ok(())
            }
            Expr::Spread(expr) => {
                self.analyze_expr(expr)?;
                Ok(())
            }
            Expr::Rest(expr) => {
                self.analyze_expr(expr)?;
                Ok(())
            }
            Expr::TemplateString { parts } => {
                for part in parts {
                    if let TemplatePart::Interpolation(expr) = part {
                        self.analyze_expr(expr)?;
                    }
                }
                Ok(())
            }
            Expr::BitwiseOp { left, right, .. } => {
                self.analyze_expr(left)?;
                self.analyze_expr(right)?;
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
                self.declare_in_scope(var_name);
            }

            // Analyze the arm expression
            self.analyze_expr(&arm.expression)?;

            // Exit the scope
            self.exit_scope();
        }

        // Check pattern exhaustiveness
        let pattern_refs: Vec<Pattern> = patterns.iter().map(|p| (*p).clone()).collect();
        if !self.check_pattern_exhaustiveness(&pattern_refs)? {
            // Get missing patterns for better error messages
            let missing_patterns = self.get_missing_patterns(&pattern_refs);
            if !missing_patterns.is_empty() {
                return Err(AnalysisError::NonExhaustivePatternMatchWithMissing {
                    missing_patterns,
                });
            } else {
                return Err(AnalysisError::NonExhaustivePatternMatch);
            }
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
        self.shadowed.push(Vec::new());
    }

    fn exit_scope(&mut self) {
        if self.current_scope > 0 {
            // Remove variables that belong to this scope from the variables map
            if let Some(scope_set) = self.scopes.get(self.current_scope) {
                let names_to_remove: Vec<String> = scope_set.iter().cloned().collect();
                for name in names_to_remove {
                    self.variables.remove(&name);
                }
            }
            // Restore outer-scope entries this scope shadowed
            if let Some(restored) = self.shadowed.pop() {
                for (name, info) in restored {
                    self.variables.insert(name, info);
                }
            }
            // Pop the scope and update current_scope index
            self.scopes.pop();
            self.current_scope -= 1;
        }
    }

    /// Insert a name into the current scope's set, first saving any
    /// outer-scope tracking entry it shadows so exit_scope can restore it.
    fn declare_in_scope(&mut self, name: String) {
        if self.current_scope > 0
            && !self.scopes[self.current_scope].contains(&name)
            && let Some(existing) = self.variables.get(&name)
        {
            self.shadowed[self.current_scope - 1].push((name.clone(), existing.clone()));
        }
        self.scopes[self.current_scope].insert(name);
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
            // Pre-registered builtins are not user variables and shouldn't
            // be reported as unused
            .filter(|(name, info)| info.usage_count == 0 && !self.builtin_names.contains(*name))
            .map(|(name, _)| name)
            .collect()
    }

    pub fn get_undefined_variables(&self) -> Vec<String> {
        // Collect all undefined variables encountered during analysis
        // This would be populated during analysis if we tracked undefined refs
        Vec::new()
    }

    pub fn get_overwritten_variables(&self) -> Vec<String> {
        // Find variables that are defined but immediately overwritten
        let mut overwritten = Vec::new();
        for (name, info) in &self.variables {
            if info.usage_count == 0 && info.scope > 0 {
                // Check if there's another variable with the same name in outer scope
                let mut found_outer = false;
                for scope_idx in 0..info.scope {
                    if let Some(scope) = self.scopes.get(scope_idx)
                        && scope.contains(name)
                    {
                        found_outer = true;
                        break;
                    }
                }
                if found_outer {
                    overwritten.push(name.clone());
                }
            }
        }
        overwritten
    }

    pub fn get_variable_usage_stats(&self) -> HashMap<String, usize> {
        self.variables
            .iter()
            .map(|(name, info)| (name.clone(), info.usage_count))
            .collect()
    }

    pub fn check_unreachable_after_statement(&self, statements: &[Statement]) -> Vec<usize> {
        let mut unreachable = Vec::new();
        let mut found_terminating = false;

        for (index, statement) in statements.iter().enumerate() {
            if found_terminating {
                unreachable.push(index);
            }

            if self.is_terminating_statement(statement) {
                found_terminating = true;
            }
        }

        unreachable
    }

    pub fn check_pattern_exhaustiveness(
        &self,
        patterns: &[Pattern],
    ) -> Result<bool, AnalysisError> {
        // Empty patterns are never exhaustive
        if patterns.is_empty() {
            return Ok(false);
        }

        // Check for catch-all patterns (wildcards and variables)
        if self.has_catch_all_pattern(patterns) {
            return Ok(true);
        }

        // Analyze patterns based on their structure
        let pattern_analysis = self.analyze_pattern_structure(patterns)?;

        // Check exhaustiveness based on pattern analysis
        match pattern_analysis {
            PatternAnalysis::Boolean(has_true, has_false) => Ok(has_true && has_false),
            PatternAnalysis::Result(has_ok, has_err) => Ok(has_ok && has_err),
            PatternAnalysis::Literals(_literal_values) => {
                // For literals, we can't determine exhaustiveness without type information
                // This is a limitation - in a full implementation, we'd need type context
                Ok(false)
            }
            PatternAnalysis::Mixed => {
                // Mixed patterns without catch-all are not exhaustive
                Ok(false)
            }
            PatternAnalysis::Enum(covered_variants) => {
                // Exhaustive only if some declared enum's variants are all
                // covered. Without a matching declaration, be conservative:
                // the old "2+ variants covered" heuristic accepted matches
                // that miss variants (runtime PatternMatchFailed).
                let exhaustive = self.enum_definitions.values().any(|variants| {
                    covered_variants.iter().all(|v| variants.contains(v))
                        && variants.iter().all(|v| covered_variants.contains(v))
                });
                Ok(exhaustive)
            }
            PatternAnalysis::Tuple(_arity) => {
                // Tuple patterns are exhaustive only if they have comprehensive coverage
                // A single tuple pattern like (x, y) is not exhaustive because it only covers
                // one specific pattern, not all possible tuples
                // For now, we consider them NOT exhaustive unless there's a wildcard
                Ok(false)
            }
            PatternAnalysis::List => {
                // List patterns are complex to check exhaustively
                // For now, we consider them exhaustive if they have comprehensive coverage
                // A full implementation would check for patterns covering different lengths
                let has_empty_list = patterns.iter().any(
                    |p| matches!(p, Pattern::List { patterns: inner, .. } if inner.is_empty()),
                );
                let has_general_list = patterns.iter().any(
                    |p| matches!(p, Pattern::List { patterns: inner, .. } if !inner.is_empty()),
                );
                let has_rest_pattern = patterns
                    .iter()
                    .any(|p| matches!(p, Pattern::List { rest: Some(_), .. }));

                Ok(has_empty_list && (has_general_list || has_rest_pattern))
            }
        }
    }

    /// Check if patterns contain a catch-all pattern (wildcard or variable)
    fn has_catch_all_pattern(&self, patterns: &[Pattern]) -> bool {
        for pattern in patterns {
            if self.is_catch_all_pattern(pattern) {
                return true;
            }
        }
        false
    }

    /// Check if a single pattern is catch-all (matches everything)
    fn is_catch_all_pattern(&self, pattern: &Pattern) -> bool {
        match pattern {
            Pattern::Wildcard => true,
            Pattern::Identifier(_) => true, // Variables match everything
            Pattern::Or { alternatives } => {
                // Or pattern is catch-all if any alternative is catch-all
                alternatives
                    .iter()
                    .any(|alt| self.is_catch_all_pattern(alt))
            }
            Pattern::Guarded { .. } => {
                // Guarded patterns are not catch-all (guard might fail)
                false
            }
            _ => false,
        }
    }

    /// Analyze the structure of patterns to determine exhaustiveness strategy
    fn analyze_pattern_structure(
        &self,
        patterns: &[Pattern],
    ) -> Result<PatternAnalysis, AnalysisError> {
        let mut has_boolean = false;
        let mut has_true = false;
        let mut has_false = false;
        let mut has_result = false;
        let mut has_ok = false;
        let mut has_err = false;
        let mut literal_values = Vec::new();
        let mut enum_variants = std::collections::HashSet::new();
        let mut tuple_arities = std::collections::HashSet::new();
        let mut has_list = false;

        for pattern in patterns {
            match pattern {
                Pattern::Literal(value) => match value {
                    Value::Boolean(true) => {
                        has_boolean = true;
                        has_true = true;
                    }
                    Value::Boolean(false) => {
                        has_boolean = true;
                        has_false = true;
                    }
                    _ => {
                        literal_values.push(value.clone());
                    }
                },
                Pattern::Ok(_) => {
                    has_result = true;
                    has_ok = true;
                }
                Pattern::Err(_) => {
                    has_result = true;
                    has_err = true;
                }
                Pattern::EnumVariant { variant_name, .. } => {
                    enum_variants.insert(variant_name.clone());
                }
                Pattern::Tuple(sub_patterns) => {
                    tuple_arities.insert(sub_patterns.len());
                }
                Pattern::List { .. } => {
                    has_list = true;
                }
                Pattern::Or { alternatives } => {
                    // Recursively analyze or pattern alternatives
                    let sub_analysis = self.analyze_pattern_structure(alternatives)?;
                    match sub_analysis {
                        PatternAnalysis::Boolean(sub_true, sub_false) => {
                            has_boolean = true;
                            has_true = has_true || sub_true;
                            has_false = has_false || sub_false;
                        }
                        PatternAnalysis::Result(sub_ok, sub_err) => {
                            has_result = true;
                            has_ok = has_ok || sub_ok;
                            has_err = has_err || sub_err;
                        }
                        PatternAnalysis::Literals(sub_literals) => {
                            literal_values.extend(sub_literals);
                        }
                        PatternAnalysis::Enum(sub_variants) => {
                            enum_variants.extend(sub_variants);
                        }
                        PatternAnalysis::Tuple(sub_arity) => {
                            tuple_arities.insert(sub_arity);
                        }
                        PatternAnalysis::List => {
                            has_list = true;
                        }
                        _ => {}
                    }
                }
                Pattern::Guarded { pattern, .. } => {
                    // A guard can fail at runtime, so a guarded pattern must
                    // NOT be credited toward coverage (crediting it made
                    // `Ok(v) if v > 0 => .., Err(e) => ..` pass as exhaustive
                    // while `Ok(0)` matched no arm). Record the shape of the
                    // inner pattern only so the analysis category is right.
                    let inner_analysis =
                        self.analyze_pattern_structure(&[pattern.as_ref().clone()])?;
                    match inner_analysis {
                        PatternAnalysis::Boolean(..) => {
                            has_boolean = true;
                        }
                        PatternAnalysis::Result(..) => {
                            has_result = true;
                        }
                        PatternAnalysis::Literals(_) => {}
                        PatternAnalysis::Enum(_) => {}
                        PatternAnalysis::Tuple(sub_arity) => {
                            tuple_arities.insert(sub_arity);
                        }
                        PatternAnalysis::List => {
                            has_list = true;
                        }
                        PatternAnalysis::Mixed => {}
                    }
                }
                _ => {
                    // Other patterns like Range, Struct, etc.
                    // For now, consider them as mixed patterns
                }
            }
        }

        // Determine the primary pattern type
        if has_boolean && !has_result && literal_values.is_empty() && enum_variants.is_empty() {
            Ok(PatternAnalysis::Boolean(has_true, has_false))
        } else if has_result
            && !has_boolean
            && literal_values.is_empty()
            && enum_variants.is_empty()
        {
            Ok(PatternAnalysis::Result(has_ok, has_err))
        } else if !literal_values.is_empty()
            && !has_boolean
            && !has_result
            && enum_variants.is_empty()
        {
            Ok(PatternAnalysis::Literals(literal_values))
        } else if !enum_variants.is_empty()
            && !has_boolean
            && !has_result
            && literal_values.is_empty()
        {
            Ok(PatternAnalysis::Enum(enum_variants))
        } else if tuple_arities.len() == 1
            && !has_boolean
            && !has_result
            && literal_values.is_empty()
            && enum_variants.is_empty()
        {
            Ok(PatternAnalysis::Tuple(
                tuple_arities.into_iter().next().unwrap(),
            ))
        } else if has_list
            && !has_boolean
            && !has_result
            && literal_values.is_empty()
            && enum_variants.is_empty()
        {
            Ok(PatternAnalysis::List)
        } else {
            Ok(PatternAnalysis::Mixed)
        }
    }

    /// Get missing patterns for better error messages
    pub fn get_missing_patterns(&self, patterns: &[Pattern]) -> Vec<String> {
        let mut missing = Vec::new();

        // Check for catch-all patterns first
        if self.has_catch_all_pattern(patterns) {
            return missing; // No missing patterns if there's a catch-all
        }

        // Analyze pattern structure to find missing patterns
        if let Ok(analysis) = self.analyze_pattern_structure(patterns) {
            match analysis {
                PatternAnalysis::Boolean(has_true, has_false) => {
                    if !has_true {
                        missing.push("true".to_string());
                    }
                    if !has_false {
                        missing.push("false".to_string());
                    }
                }
                PatternAnalysis::Result(has_ok, has_err) => {
                    if !has_ok {
                        missing.push("Ok(_)".to_string());
                    }
                    if !has_err {
                        missing.push("Err(_)".to_string());
                    }
                }
                PatternAnalysis::Literals(_) => {
                    missing.push("_ (wildcard pattern)".to_string());
                }
                PatternAnalysis::Enum(_) => {
                    missing.push("_ (wildcard pattern or other enum variants)".to_string());
                }
                PatternAnalysis::Tuple(_) => {
                    missing.push("_ (wildcard pattern)".to_string());
                }
                PatternAnalysis::List => {
                    missing.push("_ (wildcard pattern)".to_string());
                }
                PatternAnalysis::Mixed => {
                    missing.push("_ (wildcard pattern)".to_string());
                }
            }
        }

        missing
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
            Statement::MetaFnDecl { .. } | Statement::DecoratedDecl { .. } => {}
            Statement::Located { stmt, .. } => {
                self.mark_statement_reachable(stmt, reachable);
            }
            Statement::TraitDecl(_) | Statement::ImplDecl(_) => {}
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
            Statement::ShareDecl(share_decl) => {
                self.mark_share_decl_reachable(share_decl, reachable);
            }
            Statement::UseDecl(_) => {
                // Use declarations don't contain expressions to mark
            }
            Statement::TestDecl(test_decl) => {
                // Mark all expressions in test body as reachable
                for statement in &test_decl.body {
                    self.mark_statement_reachable(statement, reachable);
                }
            }
            Statement::TypeDecl(_) | Statement::ErrorTypeDecl(_) => {
                // Type declarations don't contain expressions to mark
            }
        }
    }

    /// Mark an expression and its sub-expressions as reachable
    fn mark_expression_reachable(&mut self, expr: &Expr, reachable: &mut HashSet<usize>) {
        match expr {
            Expr::MacroCall { .. } => {}
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
            Expr::If {
                condition,
                then_branch,
                else_branch,
            } => {
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
                        Argument::Positional(expr) => {
                            self.mark_expression_reachable(expr, reachable)
                        }
                        Argument::Named { value, .. } => {
                            self.mark_expression_reachable(value, reachable)
                        }
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
            Expr::Try(expr) => {
                self.mark_expression_reachable(expr, reachable);
            }
            Expr::ResultOk(expr) => {
                self.mark_expression_reachable(expr, reachable);
            }
            Expr::ResultErr(expr) => {
                self.mark_expression_reachable(expr, reachable);
            }
            Expr::ForLoop { iterable, body, .. } | Expr::ParForLoop { iterable, body, .. } => {
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
            // Test assertions
            Expr::AssertEq {
                actual, expected, ..
            } => {
                self.mark_expression_reachable(actual, reachable);
                self.mark_expression_reachable(expected, reachable);
            }
            Expr::AssertNe {
                actual, expected, ..
            } => {
                self.mark_expression_reachable(actual, reachable);
                self.mark_expression_reachable(expected, reachable);
            }
            Expr::Assert { condition, .. } => {
                self.mark_expression_reachable(condition, reachable);
            }
            Expr::AssertTrue { expression, .. } => {
                self.mark_expression_reachable(expression, reachable);
            }
            Expr::AssertFalse { expression, .. } => {
                self.mark_expression_reachable(expression, reachable);
            }

            // Terminal expressions that don't contain other expressions
            Expr::Integer(_)
            | Expr::Float(_)
            | Expr::String(_)
            | Expr::Boolean(_)
            | Expr::RawString(_)
            | Expr::Identifier(_)
            | Expr::Break(None)
            | Expr::Continue
            | Expr::Return(None) => {
                // These don't contain sub-expressions
            }
            Expr::Break(Some(value)) | Expr::Return(Some(value)) => {
                self.mark_expression_reachable(value, reachable);
            }
            // Resolved forms only appear in declaration-resolved function
            // bodies, which the analyzer never sees (it runs on parse
            // output) — handled for exhaustiveness
            Expr::LocalRef { .. } => {}
            Expr::LocalAssign { value, .. } => {
                self.mark_expression_reachable(value, reachable);
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
            Expr::Break(_) | Expr::Continue | Expr::Return(_) => true,
            Expr::Block(statements) => {
                // A block is terminating if its last statement is terminating
                if let Some(last_stmt) = statements.last() {
                    self.is_terminating_statement(last_stmt)
                } else {
                    false
                }
            }
            Expr::If {
                then_branch,
                else_branch,
                ..
            } => {
                // If is terminating if both branches are terminating
                else_branch.as_ref().is_some_and(|else_branch| {
                    self.is_terminating_expression(then_branch)
                        && self.is_terminating_expression(else_branch)
                })
            }
            Expr::Match { arms, .. } => {
                // Match is terminating if all arms are terminating
                arms.iter()
                    .all(|arm| self.is_terminating_expression(&arm.expression))
            }
            _ => false,
        }
    }

    /// Perform comprehensive analysis and return a report
    pub fn analyze_comprehensive(
        &mut self,
        program: &Program,
    ) -> Result<AnalysisReport, AnalysisError> {
        // First, do the standard analysis
        self.analyze_program(program)?;

        // Collect various analysis results
        let unused_variables = self.get_unused_variables().into_iter().cloned().collect();
        let overwritten_variables = self.get_overwritten_variables();
        let dead_code = self.detect_dead_code(program);
        let variable_usage = self.get_variable_usage_stats();

        // Check for unreachable code in blocks
        let mut unreachable_statements = Vec::new();
        for (stmt_idx, statement) in program.statements.iter().enumerate() {
            if let Statement::Expression(Expr::Block(statements)) = statement {
                let unreachable_in_block = self.check_unreachable_after_statement(statements);
                for unreachable_idx in unreachable_in_block {
                    unreachable_statements.push((stmt_idx, unreachable_idx));
                }
            }
        }

        Ok(AnalysisReport {
            unused_variables,
            overwritten_variables,
            dead_code,
            unreachable_statements,
            variable_usage,
        })
    }

    /// Analyze share declarations
    fn analyze_share_decl(&mut self, share_decl: &ShareDecl) -> Result<(), AnalysisError> {
        match share_decl {
            ShareDecl::Trait(_) | ShareDecl::Impl(_) => Ok(()),
            ShareDecl::Function(func_decl) => {
                // Add function to current scope
                if self.scopes[self.current_scope].contains(&func_decl.name) {
                    return Err(AnalysisError::DuplicateVariable {
                        name: func_decl.name.clone(),
                    });
                }
                self.declare_in_scope(func_decl.name.clone());

                // Add function to variables map
                self.variables.insert(
                    func_decl.name.clone(),
                    VariableInfo {
                        name: func_decl.name.clone(),
                        scope: self.current_scope,
                        is_mutable: false,
                        usage_count: 0,
                    },
                );

                // Analyze function body with parameters in scope
                self.enter_scope();

                // Add parameters to function scope
                for param in &func_decl.parameters {
                    if self.scopes[self.current_scope].contains(&param.name) {
                        return Err(AnalysisError::DuplicateVariable {
                            name: param.name.clone(),
                        });
                    }
                    self.declare_in_scope(param.name.clone());
                    self.variables.insert(
                        param.name.clone(),
                        VariableInfo {
                            name: param.name.clone(),
                            scope: self.current_scope,
                            is_mutable: false,
                            usage_count: 0,
                        },
                    );
                }

                // Analyze function body
                self.analyze_expr(&func_decl.body)?;

                self.exit_scope();
                Ok(())
            }
            ShareDecl::Let(let_decl) => {
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

                // Add all variables from the pattern to current scope, with a
                // tracking entry — without one, usage counting silently
                // no-ops for every let-bound variable and they can never
                // appear in unused-variable reports
                for var_name in pattern_variables {
                    self.declare_in_scope(var_name.clone());
                    self.variables.insert(
                        var_name.clone(),
                        VariableInfo {
                            name: var_name,
                            scope: self.current_scope,
                            is_mutable: true,
                            usage_count: 0,
                        },
                    );
                }
                Ok(())
            }
            ShareDecl::Type(type_decl) => {
                // Add type to current scope
                if self.scopes[self.current_scope].contains(&type_decl.name) {
                    return Err(AnalysisError::DuplicateVariable {
                        name: type_decl.name.clone(),
                    });
                }
                self.declare_in_scope(type_decl.name.clone());

                // Add type to variables map
                self.variables.insert(
                    type_decl.name.clone(),
                    VariableInfo {
                        name: type_decl.name.clone(),
                        scope: self.current_scope,
                        is_mutable: false,
                        usage_count: 0,
                    },
                );
                Ok(())
            }
            ShareDecl::Use(use_decl) => {
                // Analyze transitive sharing like a regular use declaration
                self.analyze_use_decl(use_decl)
            }
        }
    }

    /// Analyze use declarations for module dependencies and validation
    fn analyze_use_decl(&mut self, use_decl: &UseDecl) -> Result<(), AnalysisError> {
        // Check for valid module path format
        if use_decl.path.is_empty() {
            return Err(AnalysisError::TypeError {
                message: "Empty module path in use declaration".to_string(),
            });
        }

        // Check for relative path traversal (security concern)
        let module_path = use_decl.path.join(".");
        if module_path.contains("..") {
            return Err(AnalysisError::TypeError {
                message: "Path traversal not allowed in module imports".to_string(),
            });
        }

        // Track imported symbols in current scope
        for item in &use_decl.items {
            match item {
                crate::ast::UseItem::Specific(name)
                | crate::ast::UseItem::Aliased { alias: name, .. } => {
                    if name.is_empty() {
                        return Err(AnalysisError::TypeError {
                            message: "Empty import item name".to_string(),
                        });
                    }

                    // Check for duplicate imports in same scope
                    if self.scopes[self.current_scope].contains(name) {
                        return Err(AnalysisError::DuplicateVariable { name: name.clone() });
                    }

                    // Add imported symbol to current scope
                    self.declare_in_scope(name.clone());

                    // Track in variables map
                    self.variables.insert(
                        name.clone(),
                        VariableInfo {
                            name: name.clone(),
                            scope: self.current_scope,
                            is_mutable: false,
                            usage_count: 0,
                        },
                    );
                }
                crate::ast::UseItem::Wildcard => {
                    // For wildcard imports, we can't track specific symbols at analysis time
                    // since we don't know what will be imported until runtime
                    // This is acceptable since wildcard imports bring everything into scope
                }
            }
        }

        Ok(())
    }

    /// Analyze test declarations for proper structure and dependencies
    fn analyze_test_decl(&mut self, test_decl: &TestDecl) -> Result<(), AnalysisError> {
        // Create new scope for test
        self.enter_scope();

        // Analyze all statements in test body
        for statement in &test_decl.body {
            self.analyze_statement(statement)?;
        }

        self.exit_scope();
        Ok(())
    }

    fn mark_share_decl_reachable(
        &mut self,
        share_decl: &ShareDecl,
        reachable: &mut HashSet<usize>,
    ) {
        match share_decl {
            ShareDecl::Trait(_) | ShareDecl::Impl(_) => {}
            ShareDecl::Function(func_decl) => {
                self.mark_expression_reachable(&func_decl.body, reachable);
            }
            ShareDecl::Let(let_decl) => {
                if let Some(ref value) = let_decl.value {
                    self.mark_expression_reachable(value, reachable);
                }
            }
            ShareDecl::Type(_) => {
                // Type declarations don't contain expressions
            }
            ShareDecl::Use(_) => {
                // Use declarations don't contain expressions to mark
            }
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
            // A call's type would need the callee's return annotation
            // resolved through scope; the checker verifies annotations at
            // call boundaries instead of inferring expression types here.
            // Lambdas likewise carry no inferred type: None means "not
            // provable", never "wrong".
            Expr::Call { .. } | Expr::Lambda { .. } => Ok(None),
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
            Statement::Located { stmt, .. } => self.mark_statement_reachable(stmt),
            Statement::TraitDecl(_) | Statement::ImplDecl(_) => {}
            Statement::MetaFnDecl { .. } | Statement::DecoratedDecl { .. } => {}
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
            Statement::ShareDecl(share_decl) => {
                // Handle share declarations by marking their expressions as reachable
                match share_decl {
                    ShareDecl::Trait(_) | ShareDecl::Impl(_) => {}
                    ShareDecl::Function(func_decl) => {
                        self.mark_expression_reachable(&func_decl.body);
                    }
                    ShareDecl::Let(let_decl) => {
                        if let Some(ref value) = let_decl.value {
                            self.mark_expression_reachable(value);
                        }
                    }
                    ShareDecl::Type(_) => {
                        // Type declarations don't contain expressions
                    }
                    ShareDecl::Use(_) => {
                        // Use declarations don't contain expressions to mark
                    }
                }
            }
            Statement::UseDecl(_) => {
                // Use declarations don't contain expressions to mark
            }
            Statement::TestDecl(test_decl) => {
                // Mark all expressions in test body as reachable
                for statement in &test_decl.body {
                    self.mark_statement_reachable(statement);
                }
            }
            Statement::TypeDecl(_) | Statement::ErrorTypeDecl(_) => {
                // Type declarations don't contain expressions to mark
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
            Expr::If {
                condition,
                then_branch,
                else_branch,
            } => {
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
            Expr::Break(_) | Expr::Continue | Expr::Return(_) => true,
            Expr::Block(statements) => {
                // A block is terminating if its last statement is terminating
                if let Some(last_stmt) = statements.last() {
                    self.is_terminating_statement(last_stmt)
                } else {
                    false
                }
            }
            Expr::If {
                then_branch,
                else_branch,
                ..
            } => {
                // If is terminating if both branches are terminating
                else_branch.as_ref().is_some_and(|else_branch| {
                    self.is_terminating_expression(then_branch)
                        && self.is_terminating_expression(else_branch)
                })
            }
            Expr::Match { arms, .. } => {
                // Match is terminating if all arms are terminating
                arms.iter()
                    .all(|arm| self.is_terminating_expression(&arm.expression))
            }
            _ => false,
        }
    }
}

/// Analysis result for different pattern types
#[derive(Debug, Clone)]
enum PatternAnalysis {
    /// Boolean patterns (true/false coverage)
    Boolean(bool, bool), // (has_true, has_false)
    /// Result patterns (Ok/Err coverage)
    Result(bool, bool), // (has_ok, has_err)
    /// Literal patterns
    Literals(Vec<Value>),
    /// Enum patterns
    Enum(std::collections::HashSet<String>),
    /// Tuple patterns with fixed arity
    Tuple(usize),
    /// List patterns
    List,
    /// Mixed pattern types
    Mixed,
}

#[cfg(test)]
mod shadowing_tests {
    use super::*;
    use crate::parser::Parser;

    /// Exiting a scope that shadowed an outer variable must restore the
    /// outer entry, not delete it — deletion silently corrupted usage
    /// counts and unused-variable reports.
    #[test]
    fn shadowing_scope_restores_outer_variable_tracking() {
        let parser = Parser::new();
        // `x` is declared at top level and shadowed by the function
        // parameter; after analysis the top-level `x` must still be tracked.
        let program = parser
            .parse("let x = 1\nfn f(x) = x + 1\nf(2)\nx")
            .expect("parse");

        let mut analyzer = Analyzer::new_empty();
        analyzer.analyze_program(&program).expect("analyze");

        assert!(
            analyzer.variables.contains_key("x"),
            "outer x should survive the shadowing function scope"
        );
        assert_eq!(
            analyzer.variables["x"].scope, 0,
            "the surviving entry should be the top-level one"
        );
    }

    #[test]
    fn shadowing_builtin_name_restores_builtin_entry() {
        let parser = Parser::new();
        let program = parser
            .parse("fn apply(map) = map + 1\napply(1)")
            .expect("parse");

        let mut analyzer = Analyzer::new();
        analyzer.analyze_program(&program).expect("analyze");

        assert!(
            analyzer.variables.contains_key("map"),
            "builtin map's tracking entry should survive being shadowed"
        );
        assert_eq!(analyzer.variables["map"].scope, 0);
    }
}
