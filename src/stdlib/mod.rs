use crate::ast::Value;
use std::collections::HashMap;
use std::sync::Arc;

pub mod base64;
pub mod cell;
pub mod chan;
pub mod collections;
pub mod crypto;
pub mod csv;
pub mod dates;
#[cfg(feature = "native")]
pub mod db;
pub mod dom;
pub mod embedded;
#[cfg(feature = "native")]
pub mod fs;
#[cfg(feature = "native")]
pub mod http;
pub mod json;
pub mod math;
pub mod meta;
#[cfg(feature = "native")]
pub mod os;
#[cfg(feature = "native")]
pub mod proc;
pub mod random;
pub mod regex_mod;
pub mod string;
pub mod task;
pub mod testing;
pub mod time;
pub mod toml_mod;

pub fn get_stdlib() -> HashMap<String, Value> {
    let mut stdlib = HashMap::new();
    stdlib.insert("base64".to_string(), base64::create_base64_module());
    stdlib.insert("meta".to_string(), meta::create_meta_module());
    stdlib.insert("col".to_string(), collections::create_collections_module());
    stdlib.insert("crypto".to_string(), crypto::create_crypto_module());
    #[cfg(feature = "native")]
    stdlib.insert("db".to_string(), db::create_db_module());
    stdlib.insert("csv".to_string(), csv::create_csv_module());
    stdlib.insert("dates".to_string(), dates::create_dates_module());
    #[cfg(feature = "native")]
    stdlib.insert("fs".to_string(), fs::create_fs_module());
    #[cfg(feature = "native")]
    stdlib.insert("http".to_string(), http::create_http_module());
    stdlib.insert("json".to_string(), json::create_json_module());
    stdlib.insert("toml".to_string(), toml_mod::create_toml_module());
    stdlib.insert("chan".to_string(), chan::create_chan_module());
    stdlib.insert("cell".to_string(), cell::create_cell_module());
    stdlib.insert("task".to_string(), task::create_task_module());
    stdlib.insert("math".to_string(), math::create_math_module());
    #[cfg(feature = "native")]
    stdlib.insert("os".to_string(), os::create_os_module());
    #[cfg(feature = "native")]
    stdlib.insert("proc".to_string(), proc::create_proc_module());
    stdlib.insert("random".to_string(), random::create_random_module());
    stdlib.insert("testing".to_string(), testing::create_testing_module());
    stdlib.insert("str".to_string(), string::create_string_module());
    stdlib.insert("re".to_string(), regex_mod::create_regex_module());
    stdlib.insert("time".to_string(), time::create_time_module());
    stdlib.insert("dom".to_string(), dom::create_dom_module());
    // OVM extension modules (ods, ...) contribute their namespaces through
    // the registry, so both tiers and the stdlib agree on one module set.
    for module in crate::native::registered_modules() {
        for (name, value) in module.namespaces() {
            stdlib.insert(name, value);
        }
    }
    stdlib
}

/// Standardized error utilities for stdlib modules
/// Misuse errors: a wrong argument count or type.
///
/// These **raise** rather than returning `Err(...)` as a value. The
/// distinction is the stdlib's governing convention as of 0.64:
///
/// - An operation that cannot fail returns its value directly.
/// - An operation that can fail for reasons the caller could reasonably
///   handle — a missing file, an absent environment variable, malformed
///   input — returns `Result`.
/// - An operation *called wrongly* raises, because no caller can sensibly
///   recover from its own bug, and a returned `Err` invites exactly that:
///   `unwrap_or(fs.read_file(), "")` would quietly swallow a typo'd call
///   the same way it swallows a missing file.
///
/// A Rust `Err` from a stdlib function becomes an olang runtime error at
/// the dispatch boundary, which is what makes these raise.
pub mod misuse {
    use super::Value;

    /// Wrong number of arguments. `expected` is rendered verbatim, so it
    /// can read "2", "1 or 2", or "at least 1".
    pub fn arity(function: &str, expected: &str, got: usize) -> Box<dyn std::error::Error> {
        format!("{function} expects {expected} argument(s), got {got}").into()
    }

    /// Wrong argument type, named by position or parameter name.
    pub fn arg_type(
        function: &str,
        which: &str,
        expected: &str,
        got: &Value,
    ) -> Box<dyn std::error::Error> {
        format!(
            "{function}: {which} must be {expected}, got {}",
            got.type_name()
        )
        .into()
    }

    /// An argument that is the right type but outside the allowed range.
    pub fn arg_value(function: &str, detail: &str) -> Box<dyn std::error::Error> {
        format!("{function}: {detail}").into()
    }
}

pub mod error_utils {
    use super::*;

    /// Create a standardized argument count error
    pub fn arity_error(function_name: &str, expected: usize, got: usize) -> Value {
        Value::Err(Box::new(Value::String(Arc::new(format!(
            "{} expects {} argument{}, got {}",
            function_name,
            expected,
            if expected == 1 { "" } else { "s" },
            got
        )))))
    }

    /// Create a standardized type error
    pub fn type_error(
        function_name: &str,
        argument_name: &str,
        expected_type: &str,
        actual_type: &str,
    ) -> Value {
        Value::Err(Box::new(Value::String(Arc::new(format!(
            "{}: {} must be {}, got {}",
            function_name, argument_name, expected_type, actual_type
        )))))
    }

    /// Create a standardized validation error
    pub fn validation_error(function_name: &str, message: &str) -> Value {
        Value::Err(Box::new(Value::String(Arc::new(format!(
            "{}: {}",
            function_name, message
        )))))
    }

    /// Create a standardized operation error
    pub fn operation_error(function_name: &str, operation: &str, details: &str) -> Value {
        Value::Err(Box::new(Value::String(Arc::new(format!(
            "{} failed to {}: {}",
            function_name, operation, details
        )))))
    }

    /// Create a standardized not found error
    pub fn not_found_error(function_name: &str, item_type: &str, identifier: &str) -> Value {
        Value::Err(Box::new(Value::String(Arc::new(format!(
            "{}: {} '{}' not found",
            function_name, item_type, identifier
        )))))
    }

    /// Create a standardized success result
    pub fn success(value: Value) -> Value {
        Value::Ok(Box::new(value))
    }

    /// Get the type name of a value for error messages
    pub fn get_type_name(value: &Value) -> &'static str {
        match value {
            Value::Integer(_) => "integer",
            Value::Float(_) => "float",
            Value::String(_) => "string",
            Value::Boolean(_) => "boolean",
            Value::List(_) => "list",
            Value::Tuple(_) => "tuple",
            Value::Struct { .. } => "struct",
            Value::Function(_) => "function",
            Value::Builtin(_) => "builtin function",
            Value::Range { .. } => "range",
            Value::Unit => "unit",
            Value::Ok(_) => "Ok",
            Value::Err(_) => "Err",
            Value::Enum { .. } => "enum",
            Value::EnumConstructor { .. } => "enum_constructor",
            Value::Map(_) => "map",
            Value::TypeInfo { .. } => "type",
            Value::Native(handle) => handle.0.type_name(),
        }
    }
}
