//! The interpreter's error type and the human-facing error formatter.

use crate::ast::Value;
use thiserror::Error;

impl InterpreterError {
    /// A runtime error from a message that may already carry the
    /// "Runtime error: " prefix — a worker's error re-raised by the thread
    /// that joined it, a tier's message re-wrapped at the boundary. One
    /// prefix, added where the error is reported; never three.
    pub fn runtime(message: impl Into<String>) -> Self {
        let mut message: String = message.into();
        while let Some(rest) = message.strip_prefix("Runtime error: ") {
            message = rest.to_string();
        }
        InterpreterError::RuntimeError { message }
    }
}

#[derive(Error, Debug)]
pub enum InterpreterError {
    #[error("Undefined variable: {name}")]
    UndefinedVariable { name: String },
    #[error("Type error: {message}")]
    TypeError { message: String },
    #[error("Runtime error: {message}")]
    RuntimeError { message: String },
    #[error("Arity mismatch: expected {expected}, got {got}")]
    ArityMismatch { expected: usize, got: usize },
    #[error("Pattern match failed")]
    PatternMatchFailed,
    // Loop control flow signals — intercepted by the loop evaluators, an error
    // only if they escape to top level (i.e. used outside a loop). Break
    // carries `break value`'s value (Unit when bare).
    #[error("'break' used outside of a loop")]
    BreakSignal(Value),
    #[error("'continue' used outside of a loop")]
    ContinueSignal,
    // `return` unwinds to the enclosing function call, which returns the
    // carried value. An error only if it escapes to top level.
    #[error("'return' used outside of a function")]
    ReturnSignal(Value),
    // `?` on an Err: unwinds to the enclosing function call, which returns
    // the carried `Value::Err` to its caller. An error only if it escapes to
    // top level (i.e. `?` hit an Err outside any function).
    #[error("'?' propagated an Err outside of a function")]
    ErrPropagation(Value),

    // Feature 7: Circular dependency detection
    #[error("Circular dependency detected: {cycle_path}")]
    CircularDependencyDetected { cycle_path: String },

    // Feature 9: Enhanced multi-file error messages
    #[error("Module '{module_path}' not found")]
    ModuleNotFound {
        module_path: String,
        searched_paths: Vec<String>,
        available_modules: Vec<String>,
        suggestions: Vec<String>,
    },
    #[error("Function '{function_name}' not found in module '{module_path}'")]
    FunctionNotFoundInModule {
        function_name: String,
        module_path: String,
        available_functions: Vec<String>,
        suggestions: Vec<String>,
        file_path: Option<String>,
    },
    #[error("Import error in '{file_path}' at line {line}")]
    ImportError {
        file_path: String,
        line: usize,
        column: Option<usize>,
        import_path: String,
        reason: String,
        suggestions: Vec<String>,
    },
    #[error("Type mismatch in '{file_path}' from module '{module_path}'")]
    MultiFileTypeMismatch {
        file_path: String,
        module_path: String,
        expected_type: String,
        actual_type: String,
        function_name: Option<String>,
        suggestions: Vec<String>,
    },
    #[error("Dependency chain error: {chain:?}")]
    DependencyChainError {
        chain: Vec<String>,
        root_error: String,
        suggested_fix: String,
    },
}

/// Feature 9: Enhanced error message formatter for multi-file development
#[derive(Clone)]
pub struct IntuitiveErrorFormatter {
    pub use_colors: bool,
    pub show_suggestions: bool,
    pub show_context: bool,
    pub max_suggestions: usize,
    pub max_list_items: usize,
}

impl Default for IntuitiveErrorFormatter {
    fn default() -> Self {
        Self {
            use_colors: true,
            show_suggestions: true,
            show_context: true,
            max_suggestions: 5,
            max_list_items: 10,
        }
    }
}

impl IntuitiveErrorFormatter {
    /// Format an interpreter error with enhanced context and suggestions
    pub fn format_error(&self, error: &InterpreterError) -> String {
        match error {
            InterpreterError::ModuleNotFound {
                module_path,
                searched_paths,
                available_modules,
                suggestions,
            } => self.format_module_not_found_error(
                module_path,
                searched_paths,
                available_modules,
                suggestions,
            ),
            InterpreterError::FunctionNotFoundInModule {
                function_name,
                module_path,
                available_functions,
                suggestions,
                file_path,
            } => self.format_function_not_found_error(
                function_name,
                module_path,
                available_functions,
                suggestions,
                file_path,
            ),
            InterpreterError::ImportError {
                file_path,
                line,
                column,
                import_path,
                reason,
                suggestions,
            } => {
                self.format_import_error(file_path, *line, column, import_path, reason, suggestions)
            }
            InterpreterError::MultiFileTypeMismatch {
                file_path,
                module_path,
                expected_type,
                actual_type,
                function_name,
                suggestions,
            } => self.format_type_mismatch_error(
                file_path,
                module_path,
                expected_type,
                actual_type,
                function_name,
                suggestions,
            ),
            InterpreterError::DependencyChainError {
                chain,
                root_error,
                suggested_fix,
            } => self.format_dependency_chain_error(chain, root_error, suggested_fix),
            InterpreterError::UndefinedVariable { name } => self.format_undefined_variable(name),
            _ => {
                // Default formatting for other errors
                format!("{}", error)
            }
        }
    }

    /// An undefined variable whose name is a *contextual* keyword is almost
    /// always a malformed declaration: `error`, `share`, `test`, and `trait`
    /// are ordinary identifiers, so when their declaration form does not
    /// parse the parser falls back to reading the keyword as a variable —
    /// and the reader is told the keyword is undefined rather than that the
    /// declaration is malformed. Point them at the declaration form.
    fn format_undefined_variable(&self, name: &str) -> String {
        let base = format!("Undefined variable: {}", name);
        let form = match name {
            "error" => Some(
                "`error` declares an error type — `error Name { Variant, WithPayload: { field: Type } }`. \
                 A malformed one parses as this variable instead.",
            ),
            "share" => Some(
                "`share` exports a declaration — `share fn ...`, `share type ...`, or `share use ...`.",
            ),
            "test" => Some("`test` declares a test block — `test \"name\" { ... }`."),
            "trait" => Some("`trait` declares a trait — `trait Name { fn method(self) -> T }`."),
            _ => None,
        };
        match form {
            Some(hint) => format!("{base}\n\nHelp:\n  • Did you mean a declaration? {hint}"),
            None => base,
        }
    }

    fn format_module_not_found_error(
        &self,
        module_path: &str,
        searched_paths: &[String],
        available_modules: &[String],
        suggestions: &[String],
    ) -> String {
        let mut result = String::new();

        // Main error message
        result.push_str(&format!("Error: Cannot find module '{}'\n", module_path));

        // Show where we looked
        if !searched_paths.is_empty() {
            result.push_str("\nSearched in:\n");
            for path in searched_paths {
                let safe = self.sanitize_path(path);
                result.push_str(&format!("  • {}\n", safe));
            }
        }

        // Show suggestions if available
        if !suggestions.is_empty() {
            result.push_str("\nDid you mean:\n");
            for suggestion in suggestions.iter().take(self.max_suggestions) {
                result.push_str(&format!("  • {}\n", suggestion));
            }
        }

        // Show available modules if any
        if !available_modules.is_empty() {
            result.push_str("\nAvailable modules:\n");
            for module in available_modules.iter().take(self.max_list_items) {
                result.push_str(&format!("  • {}\n", module));
            }
            if available_modules.len() > self.max_list_items {
                result.push_str(&format!(
                    "  ... and {} more\n",
                    available_modules.len() - self.max_list_items
                ));
            }
        }

        // Helpful guidance
        result.push_str("\nHelp:\n");
        result.push_str("  • Check the module path spelling\n");
        result.push_str("  • A file in this project imports as `use lib.<name>` (lib/) or `use <name>` (same directory)\n");
        result.push_str("  • Registered shelf libraries import by name — `otc lib list` shows them, `otc lib add <path>` registers one\n");

        result
    }

    fn format_function_not_found_error(
        &self,
        function_name: &str,
        module_path: &str,
        available_functions: &[String],
        suggestions: &[String],
        file_path: &Option<String>,
    ) -> String {
        let mut result = String::new();

        // Main error message with context
        if let Some(path) = file_path.as_ref() {
            let safe_path = self.sanitize_path(path);
            result.push_str(&format!(
                "Error: Cannot find '{}' in {}\n",
                function_name, module_path
            ));
            result.push_str(&format!("  --> {}\n", safe_path));
        } else {
            result.push_str(&format!(
                "Error: Cannot find '{}' in {}\n",
                function_name, module_path
            ));
        }

        // Show import context
        result.push_str("\nIn import statement:\n");
        result.push_str(&format!("  use {} {{ {} }}\n", module_path, function_name));
        result.push_str(&format!(
            "             {}\n",
            "^".repeat(function_name.len())
        ));

        // Show suggestions
        if !suggestions.is_empty() {
            result.push_str("\nDid you mean:\n");
            for suggestion in suggestions.iter().take(self.max_suggestions) {
                result.push_str(&format!("  • {}\n", suggestion));
            }
        }

        // Show available functions
        if !available_functions.is_empty() {
            result.push_str(&format!("\nAvailable functions in {}:\n", module_path));
            for func in available_functions.iter().take(self.max_list_items) {
                result.push_str(&format!("  • {}\n", func));
            }
            if available_functions.len() > self.max_list_items {
                result.push_str(&format!(
                    "  ... and {} more\n",
                    available_functions.len() - self.max_list_items
                ));
            }
        }

        result.push_str("\nHelp:\n");
        result.push_str("  • Check the function name spelling\n");
        result.push_str("  • Ensure the function is marked with 'share' in the module\n");
        result.push_str(&format!(
            "  • Try: use {} {{ available_function_name }}\n",
            module_path
        ));

        result
    }

    fn format_import_error(
        &self,
        file_path: &str,
        line: usize,
        column: &Option<usize>,
        import_path: &str,
        reason: &str,
        suggestions: &[String],
    ) -> String {
        let mut result = String::new();
        let safe_path = self.sanitize_path(file_path);

        // Location information
        if let Some(col) = column {
            result.push_str("Error: Import failed\n");
            result.push_str(&format!("  --> {}:{}:{}\n", safe_path, line, col));
        } else {
            result.push_str("Error: Import failed\n");
            result.push_str(&format!("  --> {}:{}\n", safe_path, line));
        }

        result.push_str(&format!("\nFailed to import: {}\n", import_path));
        result.push_str(&format!("Reason: {}\n", reason));

        // Show suggestions
        if !suggestions.is_empty() {
            result.push_str("\nSuggestions:\n");
            for suggestion in suggestions.iter().take(self.max_suggestions) {
                result.push_str(&format!("  • {}\n", suggestion));
            }
        }

        result
    }

    fn format_type_mismatch_error(
        &self,
        file_path: &str,
        module_path: &str,
        expected_type: &str,
        actual_type: &str,
        function_name: &Option<String>,
        suggestions: &[String],
    ) -> String {
        let mut result = String::new();

        if let Some(func_name) = function_name {
            result.push_str(&format!(
                "Error: Type mismatch in function '{}'\n",
                func_name
            ));
        } else {
            result.push_str("Error: Type mismatch\n");
        }

        result.push_str(&format!(
            "  --> {} (from module {})\n",
            file_path, module_path
        ));
        result.push_str(&format!("\nExpected: {}\n", expected_type));
        result.push_str(&format!("Found:    {}\n", actual_type));

        if !suggestions.is_empty() {
            result.push_str("\nSuggestions:\n");
            for suggestion in suggestions {
                result.push_str(&format!("  • {}\n", suggestion));
            }
        }

        result
    }

    fn format_dependency_chain_error(
        &self,
        chain: &[String],
        root_error: &str,
        suggested_fix: &str,
    ) -> String {
        let mut result = String::new();

        result.push_str("Error: Dependency chain failure\n\n");
        result.push_str("Dependency chain:\n");
        for (i, module) in chain.iter().enumerate() {
            if i == chain.len() - 1 {
                result.push_str(&format!("  {} {} (error here)\n", "└─", module));
            } else {
                result.push_str(&format!(
                    "  {} {}\n",
                    if i == 0 { "┌─" } else { "├─" },
                    module
                ));
            }
        }

        result.push_str(&format!("\nRoot cause: {}\n", root_error));
        result.push_str(&format!("\nSuggested fix: {}\n", suggested_fix));

        result
    }

    /// Sanitize file paths to avoid leaking sensitive directories
    fn sanitize_path(&self, raw: &str) -> String {
        if let Ok(home) = std::env::var("HOME")
            && raw.starts_with(&home)
        {
            return raw.replacen(&home, "~", 1);
        }
        if let Ok(userprofile) = std::env::var("USERPROFILE")
            && raw.starts_with(&userprofile)
        {
            return raw.replacen(&userprofile, "~", 1);
        }
        raw.to_string()
    }

    /// Calculate Levenshtein distance for "did you mean" suggestions
    #[allow(clippy::needless_range_loop)] // matrix DP is clearest indexed
    pub fn levenshtein_distance(a: &str, b: &str) -> usize {
        // Compare by Unicode scalar values, not bytes. Sizing the matrix by
        // byte length (a.len()) while indexing by character (chars().nth())
        // produced wrong distances for any multibyte identifier — the tail
        // rows compared None == None as a zero-cost match — and the repeated
        // nth() calls were quadratic. Collect the chars once and index them.
        let a: Vec<char> = a.chars().collect();
        let b: Vec<char> = b.chars().collect();
        let len_a = a.len();
        let len_b = b.len();

        if len_a == 0 {
            return len_b;
        }
        if len_b == 0 {
            return len_a;
        }

        let mut matrix = vec![vec![0; len_b + 1]; len_a + 1];

        for i in 0..=len_a {
            matrix[i][0] = i;
        }
        for j in 0..=len_b {
            matrix[0][j] = j;
        }

        for i in 1..=len_a {
            for j in 1..=len_b {
                let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
                matrix[i][j] = (matrix[i - 1][j] + 1)
                    .min(matrix[i][j - 1] + 1)
                    .min(matrix[i - 1][j - 1] + cost);
            }
        }

        matrix[len_a][len_b]
    }

    /// Generate "did you mean" suggestions based on available options
    pub fn generate_suggestions(&self, target: &str, available: &[String]) -> Vec<String> {
        let mut suggestions: Vec<(String, usize)> = available
            .iter()
            .map(|option| (option.clone(), Self::levenshtein_distance(target, option)))
            .filter(|(_, distance)| *distance <= 3 && *distance > 0) // Only suggest if reasonably close
            .collect();

        suggestions.sort_by_key(|(_, distance)| *distance);
        suggestions
            .into_iter()
            .take(self.max_suggestions)
            .map(|(suggestion, _)| suggestion)
            .collect()
    }
}

#[cfg(test)]
mod undefined_variable_hints {
    use super::IntuitiveErrorFormatter;
    use crate::interpreter::errors::InterpreterError;

    fn message(name: &str) -> String {
        IntuitiveErrorFormatter::default().format_error(&InterpreterError::UndefinedVariable {
            name: name.to_string(),
        })
    }

    #[test]
    fn a_contextual_keyword_points_at_its_declaration() {
        // A malformed `error`/`share`/`test`/`trait` declaration parses as a
        // reference to the keyword-as-variable, so "undefined variable"
        // misdirects. Each names its declaration form instead.
        for kw in ["error", "share", "test", "trait"] {
            let m = message(kw);
            assert!(
                m.contains("Did you mean a declaration?"),
                "{kw} should hint at its declaration: {m}"
            );
        }
    }

    #[test]
    fn an_ordinary_undefined_variable_gets_no_declaration_hint() {
        let m = message("foo");
        assert!(m.contains("Undefined variable: foo"));
        assert!(
            !m.contains("Did you mean a declaration?"),
            "a plain name must not be mistaken for a keyword: {m}"
        );
    }
}

#[cfg(test)]
mod levenshtein_tests {
    use super::IntuitiveErrorFormatter;

    #[test]
    fn ascii_distances_are_correct() {
        assert_eq!(
            IntuitiveErrorFormatter::levenshtein_distance("kitten", "sitting"),
            3
        );
        assert_eq!(
            IntuitiveErrorFormatter::levenshtein_distance("compute", "compute"),
            0
        );
        assert_eq!(IntuitiveErrorFormatter::levenshtein_distance("", "abc"), 3);
    }

    #[test]
    fn multibyte_identifiers_measure_by_character_not_byte() {
        // "café" is 5 bytes but 4 chars. Byte-sized dimensions with
        // char-indexed comparison gave wrong distances; these are the
        // true character edit distances.
        assert_eq!(
            IntuitiveErrorFormatter::levenshtein_distance("café", "cafe"),
            1
        );
        assert_eq!(
            IntuitiveErrorFormatter::levenshtein_distance("café", "café"),
            0
        );
        assert_eq!(
            IntuitiveErrorFormatter::levenshtein_distance("naïve", "naive"),
            1
        );
        assert_eq!(
            IntuitiveErrorFormatter::levenshtein_distance("αβγ", "αβδ"),
            1
        );
    }
}
