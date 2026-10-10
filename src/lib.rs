// InterpreterError is a large enum returned pervasively; boxing it is a
// worthwhile future refactor (smaller Results are faster), but it touches
// every eval signature — deferred rather than half-done. Tracked in
// CHANGELOG's unreleased notes.
#![allow(clippy::result_large_err)]

//! Olang - A minimal, expressive language with first-class functions and pipelines
//!
//! This crate provides the core implementation of the Olang programming language,
//! including parsing, AST construction, interpretation, and REPL functionality.

// First: the crate's print macros, which never panic on a closed stdout
// or stderr, shadow std's in every module below.
#[macro_use]
mod print_macros;
pub mod stdio;

pub mod analyze;
pub mod ast;
pub mod builtin;
pub mod boot_trace; // Where a program's start goes (`OLANG_BOOT_TRACE=1`)
pub mod caps;
pub mod clock;
pub mod vfs;
pub mod effects;
pub mod expand;
pub mod help;
pub mod home;
pub mod interpreter;
pub mod errtrace; // Where a caught error went: its frames, file and line each
pub mod interrupt; // Stopping a running evaluation (`olang repl --serve`)
pub mod log;
pub mod memory; // The accounting behind `runtime.memory()`
pub mod native;
pub mod ods;
pub mod olb; // The program image: a parsed Program as bytes
pub mod parse_cache; // Parsed modules kept between runs
pub mod compile_cache; // Compiled bytecode kept between runs
pub mod output;
pub mod ovm; // Bytecode execution tier
pub mod parallel;
pub mod parser;
pub mod pkg;
#[cfg(not(feature = "native"))]
pub mod playground;
pub mod profile; // Sampling profiler behind `olang profile`
pub mod profile_live; // A profile written while the program runs (`--profile-live`)
#[cfg(feature = "native")]
pub mod repl;
#[cfg(feature = "native")]
pub mod repl_commands; // the `:` commands described once, for the terminal and the protocol
#[cfg(feature = "native")]
pub mod repl_serve; // `olang repl --serve`: the REPL as a protocol
pub mod resolve;
#[cfg(feature = "native")]
pub mod runtime_wasm; // The browser runtime the binary embeds
pub mod scoping;
pub mod stdlib;
pub mod test_framework;
pub mod tier_stats; // Where each function ran: `--ovm-stats=json`, pins
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
