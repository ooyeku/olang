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
pub mod builtin;
pub mod caps;
pub mod clock;
pub mod effects;
pub mod expand;
pub mod help;
pub mod interpreter;
pub mod log;
pub mod native;
pub mod ods;
pub mod output;
pub mod ovm; // Bytecode execution tier
pub mod parallel;
pub mod parser;
pub mod pkg;
#[cfg(not(feature = "native"))]
pub mod playground;
#[cfg(feature = "native")]
pub mod repl;
pub mod resolve;
pub mod scoping;
pub mod stdlib;
pub mod test_framework;
pub mod timeline;
pub mod tools;
pub mod version;

// Re-export commonly used types
pub use ast::{Expr, Program, Value};
pub use interpreter::Interpreter;
pub use ovm::OvmValue;
pub use parser::Parser;
#[cfg(feature = "native")]
pub use repl::Repl;

// Make the version constant easily accessible (e.g., crate::VERSION)
pub use version::VERSION;

/// Result type for Olang operations
pub type Result<T> = anyhow::Result<T>;

/// Error type for Olang operations
pub type Error = anyhow::Error;
