//! olang: the language, its runtime and its tools, as a library.
//!
//! The `olang` binary is a thin layer over this crate. A program passes
//! through [`parser`] into the [`ast`], through [`expand`] for its macros,
//! and into the [`interpreter`]; [`ovm`] compiles its hot functions to
//! bytecode and native code. [`stdlib`] is the standard library, [`ods`]
//! the data stack, [`tools`] the developer commands (`check`, `fmt`,
//! `test`, `lsp` and the rest), and [`pkg`] the package manager.

// InterpreterError is large and returned almost everywhere. Boxing it would
// make every Result smaller, but it touches every eval signature, so the
// lint stays off until that change is made in one go.
#![allow(clippy::result_large_err)]

// First: the crate's print macros, which never panic on a closed stdout
// or stderr, shadow std's in every module below.
#[macro_use]
mod print_macros;
pub mod stdio;

pub mod analyze;
pub mod ast;
pub mod boot_trace; // Where a program's start goes (`OLANG_BOOT_TRACE=1`)
pub mod builtin;
pub mod caps;
pub mod clock;
pub mod compile_cache; // Compiled bytecode kept between runs
pub mod contain; // A panic turned into an error where the process must not end
pub mod controls; // Controls a host binds: `runtime.bind`, `ui.slider` and its kin
pub mod effects;
pub mod errtrace; // Where a caught error went: its frames, file and line each
pub mod expand;
pub mod help;
pub mod home;
pub mod interpreter;
pub mod interrupt; // Stopping a running evaluation (`olang repl --serve`)
pub mod log;
pub mod memory; // The accounting behind `runtime.memory()`
pub mod native;
pub mod native_sig; // Native builtins' parameter names, defaults and methods (L1, L2)
pub mod notebook; // A `.olmd` file read as a program
#[cfg(feature = "native")]
pub mod notebook_export; // A notebook's outputs as a page, an app's fragment, a pinned output
pub mod ods;
pub mod olb; // The program image: a parsed Program as bytes
pub mod output;
pub mod ovm; // Bytecode execution tier
pub mod parallel;
pub mod parse_cache; // Parsed modules kept between runs
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
pub mod tier_stats; // Where each function ran: `--ovm-stats=json`, pins
pub mod timeline;
pub mod tools;
pub mod version;
pub mod vfs;

// Re-export commonly used types
pub use ast::{Expr, Program, Value};
pub use interpreter::Interpreter;
pub use ovm::OvmValue;
pub use parser::Parser;
#[cfg(feature = "native")]
pub use repl::Repl;

// Make the version constant easily accessible (e.g., crate::VERSION)
pub use version::VERSION;

/// Result type for olang operations
pub type Result<T> = anyhow::Result<T>;

/// Error type for olang operations
pub type Error = anyhow::Error;
