// InterpreterError is a large enum returned pervasively; boxing it is a
// worthwhile future refactor (smaller Results are faster), but it touches
// every eval signature — deferred rather than half-done. Tracked in
// CHANGELOG's unreleased notes.
#![allow(clippy::result_large_err)]

//! Olang - A minimal, expressive language with first-class functions and pipelines
//!
//! This crate provides the core implementation of the Olang programming language,
//! including parsing, AST construction, interpretation, and REPL functionality.

pub mod analyze;
pub mod ast;
pub mod async_runtime;
pub mod builtin;
pub mod help;
pub mod interpreter;
pub mod log;
pub mod ovm; // Bytecode execution tier
pub mod parallel;
pub mod parser;
pub mod pkg;
pub mod repl;
pub mod resolve;
pub mod stdlib;
pub mod test_framework;
pub mod tools;
pub mod type_checker;
pub mod version;

// Re-export commonly used types
pub use ast::{Expr, Program, Value};
pub use interpreter::Interpreter;
pub use ovm::OvmValue;
pub use parser::Parser;
pub use repl::Repl;
pub use type_checker::{TypeChecker, TypeClass};

// Make the version constant easily accessible (e.g., crate::VERSION)
pub use version::VERSION;

/// Result type for Olang operations
pub type Result<T> = anyhow::Result<T>;

/// Error type for Olang operations
pub type Error = anyhow::Error;
