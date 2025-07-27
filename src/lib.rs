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
pub mod ovm; // Olang Virtual Machine
pub mod ovm_integration; // OVM Integration Layer
pub mod ovm_repl; // Enhanced REPL with OVM support
pub mod parallel;
pub mod parser;
pub mod repl;
pub mod stdlib;
pub mod test_framework;
pub mod type_checker;
pub mod version;

// Internal lazy evaluation module (not public API)
pub(crate) mod internal;

// Re-export commonly used types
pub use ast::{Expr, Program, Value};
pub use interpreter::Interpreter;
pub use ovm::{OlangVirtualMachine, OvmConfig, OvmValue};
pub use ovm_integration::{ExecutionStats, IntegrationConfig, OvmInterpreter};
pub use ovm_repl::{ExecutionMode, OvmRepl};
pub use parser::Parser;
pub use repl::Repl;
pub use type_checker::{TypeChecker, TypeClass};

// Make the version constant easily accessible (e.g., crate::VERSION)
pub use version::VERSION;

/// Result type for Olang operations
pub type Result<T> = anyhow::Result<T>;

/// Error type for Olang operations
pub type Error = anyhow::Error;
