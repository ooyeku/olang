use crate::ast::Value;
use std::collections::HashMap;
use std::env;
use std::process;
use std::sync::Arc;

/// Error types for OS operations
#[derive(Debug, thiserror::Error)]
pub enum OsError {
    #[error("Environment error: {message}")]
    EnvironmentError { message: String },
    #[error("Invalid argument: {message}")]
    InvalidArgument { message: String },
    #[error("Type error: {message}")]
    TypeError { message: String },
    #[error("Permission denied: {message}")]
    PermissionDenied { message: String },
    #[error("System error: {message}")]
    SystemError { message: String },
}

/// Creates the os module with all OS functions
pub fn create_os_module() -> Value {
    let mut module = HashMap::new();

    // Environment variables
    module.insert("get_env".to_string(), create_builtin_function("get_env", 1));
    module.insert("set_env".to_string(), create_builtin_function("set_env", 2));
    module.insert(
        "remove_env".to_string(),
        create_builtin_function("remove_env", 1),
    );
    module.insert(
        "list_env".to_string(),
        create_builtin_function("list_env", 0),
    );
    module.insert("has_env".to_string(), create_builtin_function("has_env", 1));

    // System information
    module.insert(
        "hostname".to_string(),
        create_builtin_function("hostname", 0),
    );
    module.insert(
        "username".to_string(),
        create_builtin_function("username", 0),
    );
    module.insert("os_type".to_string(), create_builtin_function("os_type", 0));
    module.insert("arch".to_string(), create_builtin_function("arch", 0));
    module.insert("family".to_string(), create_builtin_function("family", 0));

    // Process information
    module.insert("pid".to_string(), create_builtin_function("pid", 0));
    module.insert("args".to_string(), create_builtin_function("args", 0));
    module.insert(
        "exe_path".to_string(),
        create_builtin_function("exe_path", 0),
    );

    // Working directory
    module.insert("cwd".to_string(), create_builtin_function("cwd", 0));
    module.insert("chdir".to_string(), create_builtin_function("chdir", 1));

    // Path operations
    module.insert(
        "path_separator".to_string(),
        create_builtin_function("path_separator", 0),
    );
    module.insert(
        "home_dir".to_string(),
        create_builtin_function("home_dir", 0),
    );
    module.insert(
        "temp_dir".to_string(),
        create_builtin_function("temp_dir", 0),
    );

    // System exit
    module.insert("exit".to_string(), create_builtin_function("exit", 1));

    Value::Struct {
        type_name: "Module".to_string(),
        fields: module,
    }
}

/// Helper function to create builtin function values
fn create_builtin_function(name: &str, arity: usize) -> Value {
    Value::Builtin(crate::ast::BuiltinFunction {
        name: format!("os.{}", name),
        arity,
    })
}

/// Main dispatcher for OS function calls
pub fn call_os_function(name: &str, args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    match name {
        "get_env" => os_get_env(args),
        "set_env" => os_set_env(args),
        "remove_env" => os_remove_env(args),
        "list_env" => os_list_env(args),
        "has_env" => os_has_env(args),
        "hostname" => os_hostname(args),
        "username" => os_username(args),
        "os_type" => os_os_type(args),
        "arch" => os_arch(args),
        "family" => os_family(args),
        "pid" => os_pid(args),
        "args" => os_args(args),
        "exe_path" => os_exe_path(args),
        "cwd" => os_cwd(args),
        "chdir" => os_chdir(args),
        "path_separator" => os_path_separator(args),
        "home_dir" => os_home_dir(args),
        "temp_dir" => os_temp_dir(args),
        "exit" => os_exit(args),
        _ => Err(format!("Unknown os function: {}", name).into()),
    }
}

/// Get an environment variable
/// Usage: os.get_env("PATH") -> Result<String, Error>
fn os_get_env(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "get_env expects 1 argument, got {}",
            args.len()
        ))))));
    }

    let var_name = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "get_env: argument must be a string".to_string(),
            )))))
        }
    };

    match env::var(var_name) {
        Ok(value) => Ok(Value::Ok(Box::new(Value::String(Arc::new(value))))),
        Err(env::VarError::NotPresent) => Ok(Value::Err(Box::new(Value::String(Arc::new(
            format!("Environment variable '{}' not found", var_name),
        ))))),
        Err(env::VarError::NotUnicode(_)) => {
            Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
                "Environment variable '{}' contains invalid Unicode",
                var_name
            ))))))
        }
    }
}

/// Set an environment variable
/// Usage: os.set_env("MY_VAR", "value") -> Result<Unit, Error>
fn os_set_env(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "set_env expects 2 arguments, got {}",
            args.len()
        ))))));
    }

    let var_name = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "set_env: first argument must be a string".to_string(),
            )))))
        }
    };

    let var_value = match &args[1] {
        Value::String(s) => s.as_ref(),
        Value::Integer(i) => &i.to_string(),
        Value::Float(f) => &f.to_string(),
        Value::Boolean(b) => &b.to_string(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "set_env: second argument must be a string, number, or boolean".to_string(),
            )))))
        }
    };

    env::set_var(var_name, var_value);
    Ok(Value::Ok(Box::new(Value::Unit)))
}

/// Remove an environment variable
/// Usage: os.remove_env("MY_VAR") -> Result<Unit, Error>
fn os_remove_env(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "remove_env expects 1 argument, got {}",
            args.len()
        ))))));
    }

    let var_name = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "remove_env: argument must be a string".to_string(),
            )))))
        }
    };

    env::remove_var(var_name);
    Ok(Value::Ok(Box::new(Value::Unit)))
}

/// List all environment variables
/// Usage: os.list_env() -> Result<{String: String}, Error>
fn os_list_env(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if !args.is_empty() {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "list_env expects 0 arguments, got {}",
            args.len()
        ))))));
    }

    let mut env_vars = HashMap::new();
    for (key, value) in env::vars() {
        env_vars.insert(key, Value::String(Arc::new(value)));
    }

    Ok(Value::Ok(Box::new(Value::Struct {
        type_name: "EnvironmentVariables".to_string(),
        fields: env_vars,
    })))
}

/// Check if an environment variable exists
/// Usage: os.has_env("PATH") -> Result<Bool, Error>
fn os_has_env(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "has_env expects 1 argument, got {}",
            args.len()
        ))))));
    }

    let var_name = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "has_env: argument must be a string".to_string(),
            )))))
        }
    };

    let exists = env::var(var_name).is_ok();
    Ok(Value::Ok(Box::new(Value::Boolean(exists))))
}

/// Get system hostname
/// Usage: os.hostname() -> Result<String, Error>
fn os_hostname(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if !args.is_empty() {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "hostname expects 0 arguments, got {}",
            args.len()
        ))))));
    }

    match hostname::get() {
        Ok(hostname) => {
            let hostname_str = hostname.to_string_lossy().to_string();
            Ok(Value::Ok(Box::new(Value::String(Arc::new(hostname_str)))))
        }
        Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "Failed to get hostname: {}",
            e
        )))))),
    }
}

/// Get current username
/// Usage: os.username() -> Result<String, Error>
fn os_username(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if !args.is_empty() {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "username expects 0 arguments, got {}",
            args.len()
        ))))));
    }

    let username = whoami::username();
    Ok(Value::Ok(Box::new(Value::String(Arc::new(username)))))
}

/// Get operating system type
/// Usage: os.os_type() -> Result<String, Error>
fn os_os_type(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if !args.is_empty() {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "os_type expects 0 arguments, got {}",
            args.len()
        ))))));
    }

    let os_type = env::consts::OS;
    Ok(Value::Ok(Box::new(Value::String(Arc::new(
        os_type.to_string(),
    )))))
}

/// Get system architecture
/// Usage: os.arch() -> Result<String, Error>
fn os_arch(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if !args.is_empty() {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "arch expects 0 arguments, got {}",
            args.len()
        ))))));
    }

    let arch = env::consts::ARCH;
    Ok(Value::Ok(Box::new(Value::String(Arc::new(
        arch.to_string(),
    )))))
}

/// Get operating system family
/// Usage: os.family() -> Result<String, Error>
fn os_family(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if !args.is_empty() {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "family expects 0 arguments, got {}",
            args.len()
        ))))));
    }

    let family = env::consts::FAMILY;
    Ok(Value::Ok(Box::new(Value::String(Arc::new(
        family.to_string(),
    )))))
}

/// Get current process ID
/// Usage: os.pid() -> Result<Int, Error>
fn os_pid(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if !args.is_empty() {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "pid expects 0 arguments, got {}",
            args.len()
        ))))));
    }

    let pid = process::id();
    Ok(Value::Ok(Box::new(Value::Integer(pid as i64))))
}

/// Get command line arguments
/// Usage: os.args() -> Result<[String], Error>
fn os_args(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if !args.is_empty() {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "args expects 0 arguments, got {}",
            args.len()
        ))))));
    }

    let args: Vec<Value> = env::args()
        .map(|arg| Value::String(Arc::new(arg)))
        .collect();

    Ok(Value::Ok(Box::new(Value::List(args.into()))))
}

/// Get executable path
/// Usage: os.exe_path() -> Result<String, Error>
fn os_exe_path(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if !args.is_empty() {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "exe_path expects 0 arguments, got {}",
            args.len()
        ))))));
    }

    match env::current_exe() {
        Ok(path) => Ok(Value::Ok(Box::new(Value::String(Arc::new(
            path.to_string_lossy().to_string(),
        ))))),
        Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "Failed to get executable path: {}",
            e
        )))))),
    }
}

/// Get current working directory
/// Usage: os.cwd() -> Result<String, Error>
fn os_cwd(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if !args.is_empty() {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "cwd expects 0 arguments, got {}",
            args.len()
        ))))));
    }

    match env::current_dir() {
        Ok(path) => Ok(Value::Ok(Box::new(Value::String(Arc::new(
            path.to_string_lossy().to_string(),
        ))))),
        Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "Failed to get current directory: {}",
            e
        )))))),
    }
}

/// Change current working directory
/// Usage: os.chdir("/path/to/dir") -> Result<Unit, Error>
fn os_chdir(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "chdir expects 1 argument, got {}",
            args.len()
        ))))));
    }

    let path = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "chdir: argument must be a string".to_string(),
            )))))
        }
    };

    match env::set_current_dir(path) {
        Ok(()) => Ok(Value::Ok(Box::new(Value::Unit))),
        Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "Failed to change directory: {}",
            e
        )))))),
    }
}

/// Get path separator for the current OS
/// Usage: os.path_separator() -> Result<String, Error>
fn os_path_separator(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if !args.is_empty() {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "path_separator expects 0 arguments, got {}",
            args.len()
        ))))));
    }

    let separator = std::path::MAIN_SEPARATOR.to_string();
    Ok(Value::Ok(Box::new(Value::String(Arc::new(separator)))))
}

/// Get home directory
/// Usage: os.home_dir() -> Result<String, Error>
fn os_home_dir(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if !args.is_empty() {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "home_dir expects 0 arguments, got {}",
            args.len()
        ))))));
    }

    match dirs::home_dir() {
        Some(path) => Ok(Value::Ok(Box::new(Value::String(Arc::new(
            path.to_string_lossy().to_string(),
        ))))),
        None => Ok(Value::Err(Box::new(Value::String(Arc::new(
            "Failed to get home directory".to_string(),
        ))))),
    }
}

/// Get temporary directory
/// Usage: os.temp_dir() -> Result<String, Error>
fn os_temp_dir(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if !args.is_empty() {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "temp_dir expects 0 arguments, got {}",
            args.len()
        ))))));
    }

    let temp_dir = env::temp_dir();
    Ok(Value::Ok(Box::new(Value::String(Arc::new(
        temp_dir.to_string_lossy().to_string(),
    )))))
}

/// Exit the program with a status code
/// Usage: os.exit(0) -> Never returns
fn os_exit(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "exit expects 1 argument, got {}",
            args.len()
        ))))));
    }

    let exit_code = match &args[0] {
        Value::Integer(i) => *i as i32,
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "exit: argument must be an integer".to_string(),
            )))))
        }
    };

    process::exit(exit_code);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    // Helper functions for creating test values
    fn string_val(s: &str) -> Value {
        Value::String(Arc::new(s.to_string()))
    }

    fn int_val(n: i64) -> Value {
        Value::Integer(n)
    }

    fn bool_val(b: bool) -> Value {
        Value::Boolean(b)
    }

    fn float_val(f: f64) -> Value {
        Value::Float(f)
    }

    // Helper function to assert Ok result and extract inner value
    fn assert_ok(result: &Value) -> &Value {
        match result {
            Value::Ok(inner) => inner.as_ref(),
            _ => panic!("Expected Ok result, got: {:?}", result),
        }
    }

    // Helper function to assert Err result and extract error message
    fn assert_err(result: &Value) -> &Value {
        match result {
            Value::Err(inner) => inner.as_ref(),
            _ => panic!("Expected Err result, got: {:?}", result),
        }
    }

    // Helper function to extract string from Value
    fn extract_string(value: &Value) -> &str {
        match value {
            Value::String(s) => s.as_ref(),
            _ => panic!("Expected string value, got: {:?}", value),
        }
    }

    // Helper function to extract boolean from Value
    fn extract_bool(value: &Value) -> bool {
        match value {
            Value::Boolean(b) => *b,
            _ => panic!("Expected boolean value, got: {:?}", value),
        }
    }

    // Helper function to extract integer from Value
    fn extract_int(value: &Value) -> i64 {
        match value {
            Value::Integer(i) => *i,
            _ => panic!("Expected integer value, got: {:?}", value),
        }
    }

    // Helper function to extract list from Value
    fn extract_list(value: &Value) -> &[Value] {
        match value {
            Value::List(list) => list.as_ref(),
            _ => panic!("Expected list value, got: {:?}", value),
        }
    }

    // Helper function to extract struct fields from Value
    fn extract_struct_fields(value: &Value) -> &HashMap<String, Value> {
        match value {
            Value::Struct { fields, .. } => fields,
            _ => panic!("Expected struct value, got: {:?}", value),
        }
    }

    #[test]
    fn test_os_module_creation() {
        let module = create_os_module();

        if let Value::Struct { type_name, fields } = module {
            assert_eq!(type_name, "Module");

            // Check that all expected functions are present with correct arities
            let expected_functions = vec![
                ("get_env", 1),
                ("set_env", 2),
                ("remove_env", 1),
                ("list_env", 0),
                ("has_env", 1),
                ("hostname", 0),
                ("username", 0),
                ("os_type", 0),
                ("arch", 0),
                ("family", 0),
                ("pid", 0),
                ("args", 0),
                ("exe_path", 0),
                ("cwd", 0),
                ("chdir", 1),
                ("path_separator", 0),
                ("home_dir", 0),
                ("temp_dir", 0),
                ("exit", 1),
            ];

            for (func_name, expected_arity) in expected_functions {
                assert!(
                    fields.contains_key(func_name),
                    "Missing function: {}",
                    func_name
                );

                if let Value::Builtin(builtin) = &fields[func_name] {
                    assert_eq!(builtin.name, format!("os.{}", func_name));
                    assert_eq!(builtin.arity, expected_arity);
                } else {
                    panic!("Expected builtin function for {}", func_name);
                }
            }
        } else {
            panic!("Expected struct for os module");
        }
    }

    #[test]
    fn test_environment_variable_operations() {
        let test_var = "OLANG_TEST_VAR";
        let test_value = "test_value_123";

        // Clean up any existing test variable
        env::remove_var(test_var);

        // Test has_env for non-existent variable
        let result = os_has_env(vec![string_val(test_var)]).unwrap();
        let exists = assert_ok(&result);
        assert!(!extract_bool(exists), "Variable should not exist initially");

        // Test get_env for non-existent variable
        let result = os_get_env(vec![string_val(test_var)]).unwrap();
        assert_err(&result); // Should return error for non-existent variable

        // Test set_env
        let result = os_set_env(vec![string_val(test_var), string_val(test_value)]).unwrap();
        assert_ok(&result);

        // Test has_env for existing variable
        let result = os_has_env(vec![string_val(test_var)]).unwrap();
        let exists = assert_ok(&result);
        assert!(extract_bool(exists), "Variable should exist after setting");

        // Test get_env for existing variable
        let result = os_get_env(vec![string_val(test_var)]).unwrap();
        let value = assert_ok(&result);
        assert_eq!(extract_string(value), test_value);

        // Test remove_env
        let result = os_remove_env(vec![string_val(test_var)]).unwrap();
        assert_ok(&result);

        // Test has_env after removal
        let result = os_has_env(vec![string_val(test_var)]).unwrap();
        let exists = assert_ok(&result);
        assert!(
            !extract_bool(exists),
            "Variable should not exist after removal"
        );

        // Clean up
        env::remove_var(test_var);
    }

    #[test]
    fn test_set_env_with_different_types() {
        let test_var = "OLANG_TEST_TYPE_VAR";

        // Test with string
        let result = os_set_env(vec![string_val(test_var), string_val("string_value")]).unwrap();
        assert_ok(&result);
        assert_eq!(env::var(test_var).unwrap(), "string_value");

        // Test with integer
        let result = os_set_env(vec![string_val(test_var), int_val(42)]).unwrap();
        assert_ok(&result);
        assert_eq!(env::var(test_var).unwrap(), "42");

        // Test with float
        let result = os_set_env(vec![string_val(test_var), float_val(3.14)]).unwrap();
        assert_ok(&result);
        assert_eq!(env::var(test_var).unwrap(), "3.14");

        // Test with boolean
        let result = os_set_env(vec![string_val(test_var), bool_val(true)]).unwrap();
        assert_ok(&result);
        assert_eq!(env::var(test_var).unwrap(), "true");

        // Clean up
        env::remove_var(test_var);
    }

    #[test]
    fn test_list_env() {
        let result = os_list_env(vec![]).unwrap();
        let env_struct = assert_ok(&result);
        let fields = extract_struct_fields(env_struct);

        // Should contain at least some environment variables
        assert!(!fields.is_empty(), "Environment should have some variables");

        // Check that PATH exists (should exist on all systems)
        assert!(
            fields.contains_key("PATH"),
            "PATH should exist in environment"
        );

        // Verify all values are strings
        for (key, value) in fields {
            match value {
                Value::String(_) => {} // Expected
                _ => panic!(
                    "All env values should be strings, but {} has type {:?}",
                    key, value
                ),
            }
        }
    }

    #[test]
    fn test_system_information_functions() {
        // Test os_type
        let result = os_os_type(vec![]).unwrap();
        let os_type = assert_ok(&result);
        let os_str = extract_string(os_type);
        assert!(!os_str.is_empty(), "OS type should not be empty");
        // Should be one of the known OS types
        assert!(
            ["linux", "macos", "windows", "freebsd", "openbsd", "netbsd"].contains(&os_str),
            "Unknown OS type: {}",
            os_str
        );

        // Test arch
        let result = os_arch(vec![]).unwrap();
        let arch = assert_ok(&result);
        let arch_str = extract_string(arch);
        assert!(!arch_str.is_empty(), "Architecture should not be empty");

        // Test family
        let result = os_family(vec![]).unwrap();
        let family = assert_ok(&result);
        let family_str = extract_string(family);
        assert!(!family_str.is_empty(), "OS family should not be empty");
        // Should be one of the known families
        assert!(
            ["unix", "windows"].contains(&family_str),
            "Unknown OS family: {}",
            family_str
        );

        // Test hostname
        let result = os_hostname(vec![]).unwrap();
        let hostname = assert_ok(&result);
        let hostname_str = extract_string(hostname);
        assert!(!hostname_str.is_empty(), "Hostname should not be empty");

        // Test username
        let result = os_username(vec![]).unwrap();
        let username = assert_ok(&result);
        let username_str = extract_string(username);
        assert!(!username_str.is_empty(), "Username should not be empty");
    }

    #[test]
    fn test_process_information_functions() {
        // Test pid
        let result = os_pid(vec![]).unwrap();
        let pid = assert_ok(&result);
        let pid_val = extract_int(pid);
        assert!(pid_val > 0, "PID should be positive");

        // Test args
        let result = os_args(vec![]).unwrap();
        let args = assert_ok(&result);
        let args_list = extract_list(args);
        assert!(
            !args_list.is_empty(),
            "Args should contain at least the program name"
        );

        // First argument should be the program name/path
        if let Value::String(first_arg) = &args_list[0] {
            assert!(!first_arg.is_empty(), "First argument should not be empty");
        } else {
            panic!("First argument should be a string");
        }

        // Test exe_path
        let result = os_exe_path(vec![]).unwrap();
        let exe_path = assert_ok(&result);
        let exe_path_str = extract_string(exe_path);
        assert!(
            !exe_path_str.is_empty(),
            "Executable path should not be empty"
        );
    }

    #[test]
    fn test_working_directory_operations() {
        // Get current directory
        let result = os_cwd(vec![]).unwrap();
        let original_cwd = assert_ok(&result);
        let original_path = extract_string(original_cwd);
        assert!(
            !original_path.is_empty(),
            "Current directory should not be empty"
        );

        // Test changing to a valid directory (use temp dir which should exist)
        let temp_result = os_temp_dir(vec![]).unwrap();
        let temp_dir = assert_ok(&temp_result);
        let temp_path = extract_string(temp_dir);

        let chdir_result = os_chdir(vec![string_val(temp_path)]).unwrap();
        assert_ok(&chdir_result);

        // Verify the change
        let new_cwd_result = os_cwd(vec![]).unwrap();
        let new_cwd = assert_ok(&new_cwd_result);
        let new_path = extract_string(new_cwd);

        // On some systems, paths might be resolved differently, so just check it's not empty
        assert!(
            !new_path.is_empty(),
            "New current directory should not be empty"
        );

        // Change back to original directory
        let restore_result = os_chdir(vec![string_val(original_path)]).unwrap();
        assert_ok(&restore_result);
    }

    #[test]
    fn test_path_operations() {
        // Test path_separator
        let result = os_path_separator(vec![]).unwrap();
        let separator = assert_ok(&result);
        let sep_str = extract_string(separator);
        assert!(!sep_str.is_empty(), "Path separator should not be empty");
        // Should be either / or \ depending on OS
        assert!(
            sep_str == "/" || sep_str == "\\",
            "Path separator should be / or \\, got: {}",
            sep_str
        );

        // Test home_dir
        let result = os_home_dir(vec![]).unwrap();
        let home_dir = assert_ok(&result);
        let home_path = extract_string(home_dir);
        assert!(!home_path.is_empty(), "Home directory should not be empty");

        // Test temp_dir
        let result = os_temp_dir(vec![]).unwrap();
        let temp_dir = assert_ok(&result);
        let temp_path = extract_string(temp_dir);
        assert!(!temp_path.is_empty(), "Temp directory should not be empty");
    }

    #[test]
    fn test_argument_validation() {
        // Test functions that expect 0 arguments
        let zero_arg_functions = vec![
            "list_env",
            "hostname",
            "username",
            "os_type",
            "arch",
            "family",
            "pid",
            "args",
            "exe_path",
            "cwd",
            "path_separator",
            "home_dir",
            "temp_dir",
        ];

        for func_name in zero_arg_functions {
            let result = call_os_function(func_name, vec![string_val("extra")]);
            match result {
                Ok(Value::Err(_)) => {} // Expected error
                Ok(other) => panic!(
                    "Expected error for {} with extra arg, got: {:?}",
                    func_name, other
                ),
                Err(e) => panic!("Unexpected error for {}: {}", func_name, e),
            }
        }

        // Test functions that expect 1 argument
        let result = os_get_env(vec![]).unwrap();
        assert_err(&result);

        let result = os_get_env(vec![string_val("TEST"), string_val("EXTRA")]).unwrap();
        assert_err(&result);

        // Test functions that expect 2 arguments
        let result = os_set_env(vec![string_val("TEST")]).unwrap();
        assert_err(&result);

        let result = os_set_env(vec![
            string_val("TEST"),
            string_val("VALUE"),
            string_val("EXTRA"),
        ])
        .unwrap();
        assert_err(&result);
    }

    #[test]
    fn test_type_validation() {
        // Test get_env with non-string argument
        let result = os_get_env(vec![int_val(42)]).unwrap();
        assert_err(&result);

        // Test set_env with non-string first argument
        let result = os_set_env(vec![int_val(42), string_val("value")]).unwrap();
        assert_err(&result);

        // Test set_env with unsupported type for second argument
        let result = os_set_env(vec![string_val("TEST"), Value::List(vec![].into())]).unwrap();
        assert_err(&result);

        // Test has_env with non-string argument
        let result = os_has_env(vec![bool_val(true)]).unwrap();
        assert_err(&result);

        // Test remove_env with non-string argument
        let result = os_remove_env(vec![float_val(3.14)]).unwrap();
        assert_err(&result);

        // Test chdir with non-string argument
        let result = os_chdir(vec![int_val(123)]).unwrap();
        assert_err(&result);

        // Test exit with non-integer argument
        // Note: We can't actually test exit because it would terminate the test process
        // But we can test the argument validation
        let result = os_exit(vec![string_val("not_a_number")]).unwrap();
        assert_err(&result);
    }

    #[test]
    fn test_error_conditions() {
        // Test chdir with invalid path
        let result = os_chdir(vec![string_val(
            "/nonexistent/invalid/path/that/should/not/exist",
        )])
        .unwrap();
        assert_err(&result);

        // Test get_env with non-existent variable
        let result = os_get_env(vec![string_val("OLANG_NONEXISTENT_VAR_12345")]).unwrap();
        assert_err(&result);
    }

    #[test]
    fn test_function_dispatcher() {
        // Test valid function calls through dispatcher
        let result = call_os_function("os_type", vec![]).unwrap();
        let os_type = assert_ok(&result);
        assert!(!extract_string(os_type).is_empty());

        let result = call_os_function("pid", vec![]).unwrap();
        let pid = assert_ok(&result);
        assert!(extract_int(pid) > 0);

        // Test unknown function
        let result = call_os_function("unknown_function", vec![]);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Unknown os function"));
    }

    #[test]
    fn test_environment_variable_edge_cases() {
        let test_var = "OLANG_EDGE_TEST_VAR";

        // Test with empty string value
        env::remove_var(test_var);
        let result = os_set_env(vec![string_val(test_var), string_val("")]).unwrap();
        assert_ok(&result);

        let result = os_get_env(vec![string_val(test_var)]).unwrap();
        let value = assert_ok(&result);
        assert_eq!(extract_string(value), "");

        // Test with special characters in value
        let special_value = "test with spaces & special chars: !@#$%^&*()";
        let result = os_set_env(vec![string_val(test_var), string_val(special_value)]).unwrap();
        assert_ok(&result);

        let result = os_get_env(vec![string_val(test_var)]).unwrap();
        let value = assert_ok(&result);
        assert_eq!(extract_string(value), special_value);

        // Clean up
        env::remove_var(test_var);
    }

    #[test]
    fn test_system_constants_are_valid() {
        // Test that system constants return expected values
        let result = os_os_type(vec![]).unwrap();
        let os_type = extract_string(assert_ok(&result));

        let result = os_family(vec![]).unwrap();
        let family = extract_string(assert_ok(&result));

        // Verify consistency between os_type and family
        match family {
            "unix" => assert!(
                ["linux", "macos", "freebsd", "openbsd", "netbsd"].contains(&os_type),
                "Unix family should have unix-like OS type, got: {}",
                os_type
            ),
            "windows" => assert_eq!(
                os_type, "windows",
                "Windows family should have windows OS type"
            ),
            _ => panic!("Unknown OS family: {}", family),
        }
    }

    #[test]
    fn test_path_separator_consistency() {
        let result = os_path_separator(vec![]).unwrap();
        let separator = extract_string(assert_ok(&result));

        let result = os_family(vec![]).unwrap();
        let family = extract_string(assert_ok(&result));

        // Verify path separator matches OS family
        match family {
            "unix" => assert_eq!(
                separator, "/",
                "Unix systems should use / as path separator"
            ),
            "windows" => assert_eq!(separator, "\\", "Windows should use \\ as path separator"),
            _ => panic!("Unknown OS family for path separator test: {}", family),
        }
    }
}
