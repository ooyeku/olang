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
    module.insert("stdin".to_string(), create_builtin_function("stdin", 0));
    module.insert(
        "stdin_lines".to_string(),
        create_builtin_function("stdin_lines", 0),
    );
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

    // Terminal
    module.insert("is_tty".to_string(), create_builtin_function("is_tty", 0));
    module.insert("flush".to_string(), create_builtin_function("flush", 0));

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

    // Run an external program and capture its result
    module.insert("exec".to_string(), create_builtin_function("exec", 2));
    module.insert(
        "read_line".to_string(),
        create_builtin_function("read_line", 0),
    );

    // SIGINT (Ctrl-C) handling: install a handler that records the
    // interrupt instead of terminating, then poll it in a loop for a
    // graceful shutdown.
    module.insert(
        "on_interrupt".to_string(),
        create_builtin_function("on_interrupt", 0),
    );
    module.insert(
        "interrupted".to_string(),
        create_builtin_function("interrupted", 0),
    );
    module.insert(
        "on_shutdown".to_string(),
        create_builtin_function("on_shutdown", 1),
    );
    module.insert(
        "reset_interrupt".to_string(),
        create_builtin_function("reset_interrupt", 0),
    );

    Value::Struct {
        type_name: "Module".to_string(),
        fields: std::sync::Arc::new(module),
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
        "stdin" => os_stdin(args),
        "stdin_lines" => os_stdin_lines(args),
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
        "exec" => os_exec(args),
        "read_line" => os_read_line(args),
        "is_tty" => os_is_tty(args),
        "flush" => os_flush(args),
        "on_interrupt" => os_on_interrupt(args),
        "interrupted" => os_interrupted(args),
        // Intercepted by the interpreter (it needs the program to run the
        // handler on); reaching here means no interpreter did.
        "on_shutdown" => Err("os.on_shutdown needs the running program".into()),
        "reset_interrupt" => os_reset_interrupt(args),
        _ => Err(format!("Unknown os function: {}", name).into()),
    }
}

/// Get an environment variable
/// Usage: os.get_env("PATH") -> Result<String, Error>
fn os_get_env(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("os.get_env expects 1 argument, got {}", args.len()).into());
    }

    let var_name = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Err("os.get_env: argument must be a string".to_string().into());
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
        return Err(format!("os.set_env expects 2 arguments, got {}", args.len()).into());
    }

    let var_name = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Err("os.set_env: first argument must be a string"
                .to_string()
                .into());
        }
    };

    let var_value = match &args[1] {
        Value::String(s) => s.as_ref(),
        Value::Integer(i) => &i.to_string(),
        Value::Float(f) => &f.to_string(),
        Value::Boolean(b) => &b.to_string(),
        _ => {
            return Err(
                "os.set_env: second argument must be a string, number, or boolean"
                    .to_string()
                    .into(),
            );
        }
    };

    // SAFETY: the platform environment is process-global and libc-level
    // reads are unsynchronized; olang exposes mutation as an explicit
    // effect, and a program that calls os.set_env while worker threads
    // read the environment races exactly as the same C program would.
    unsafe { env::set_var(var_name, var_value) };
    Ok(Value::Unit)
}

/// Remove an environment variable
/// Usage: os.remove_env("MY_VAR") -> Result<Unit, Error>
fn os_remove_env(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("os.remove_env expects 1 argument, got {}", args.len()).into());
    }

    let var_name = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Err("os.remove_env: argument must be a string"
                .to_string()
                .into());
        }
    };

    // SAFETY: same contract as os.set_env above.
    unsafe { env::remove_var(var_name) };
    Ok(Value::Unit)
}

/// List all environment variables
/// Usage: os.list_env() -> Result<{String: String}, Error>
fn os_list_env(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if !args.is_empty() {
        return Err(format!("os.list_env expects 0 arguments, got {}", args.len()).into());
    }

    let mut env_vars = HashMap::new();
    for (key, value) in env::vars() {
        env_vars.insert(key, Value::String(Arc::new(value)));
    }

    Ok(Value::Struct {
        type_name: "EnvironmentVariables".to_string(),
        fields: std::sync::Arc::new(env_vars),
    })
}

/// Check if an environment variable exists
/// Usage: os.has_env("PATH") -> Result<Bool, Error>
fn os_has_env(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("os.has_env expects 1 argument, got {}", args.len()).into());
    }

    let var_name = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Err("os.has_env: argument must be a string".to_string().into());
        }
    };

    let exists = env::var(var_name).is_ok();
    Ok(Value::Boolean(exists))
}

/// Get system hostname
/// Usage: os.hostname() -> Result<String, Error>
fn os_hostname(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if !args.is_empty() {
        return Err(format!("os.hostname expects 0 arguments, got {}", args.len()).into());
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
        return Err(format!("os.username expects 0 arguments, got {}", args.len()).into());
    }

    let username = whoami::username();
    Ok(Value::String(Arc::new(username)))
}

/// Get operating system type
/// Usage: os.os_type() -> Result<String, Error>
fn os_os_type(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if !args.is_empty() {
        return Err(format!("os.os_type expects 0 arguments, got {}", args.len()).into());
    }

    let os_type = env::consts::OS;
    Ok(Value::String(Arc::new(os_type.to_string())))
}

/// Get system architecture
/// Usage: os.arch() -> Result<String, Error>
fn os_arch(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if !args.is_empty() {
        return Err(format!("os.arch expects 0 arguments, got {}", args.len()).into());
    }

    let arch = env::consts::ARCH;
    Ok(Value::String(Arc::new(arch.to_string())))
}

/// Get operating system family
/// Usage: os.family() -> Result<String, Error>
fn os_family(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if !args.is_empty() {
        return Err(format!("os.family expects 0 arguments, got {}", args.len()).into());
    }

    let family = env::consts::FAMILY;
    Ok(Value::String(Arc::new(family.to_string())))
}

/// Get current process ID
/// Usage: os.pid() -> Result<Int, Error>
fn os_pid(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if !args.is_empty() {
        return Err(format!("os.pid expects 0 arguments, got {}", args.len()).into());
    }

    let pid = process::id();
    Ok(Value::Integer(pid as i64))
}

/// Get command line arguments
/// Usage: os.args() -> Result<[String], Error>
/// The program's own arguments (argv[0] is the script path, the rest are the
/// arguments after it). Set by the CLI before execution; when unset (e.g. at
/// the REPL prompt) `os.args()` falls back to the process arguments. A
/// RwLock rather than a OnceLock so the REPL's `:run` can install the
/// file's argv for the duration of that run and restore the previous
/// value after — a script that keys off `os.args()[0]` (run_all.ol
/// resolves its whole example set from it) behaves the same under
/// `:run` as under `olang run`.
static SCRIPT_ARGS: std::sync::RwLock<Option<Vec<String>>> = std::sync::RwLock::new(None);

/// Install the program's argument vector, returning what it replaced.
pub fn set_script_args(args: Vec<String>) -> Option<Vec<String>> {
    match SCRIPT_ARGS.write() {
        Ok(mut slot) => slot.replace(args),
        Err(_) => None,
    }
}

/// Restore a previously replaced argument vector (None clears).
pub fn restore_script_args(previous: Option<Vec<String>>) {
    if let Ok(mut slot) = SCRIPT_ARGS.write() {
        *slot = previous;
    }
}

fn os_args(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if !args.is_empty() {
        return Err(format!("os.args expects 0 arguments, got {}", args.len()).into());
    }

    // The script's own argv when the CLI set it, otherwise the process args.
    let script_args = SCRIPT_ARGS.read().ok().and_then(|slot| slot.clone());
    let args: Vec<Value> = match script_args {
        Some(script_args) => script_args
            .iter()
            .map(|arg| Value::String(Arc::new(arg.clone())))
            .collect(),
        None => env::args()
            .map(|arg| Value::String(Arc::new(arg)))
            .collect(),
    };

    Ok(Value::List(args.into()))
}

/// Get executable path
/// Usage: os.exe_path() -> Result<String, Error>
fn os_exe_path(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if !args.is_empty() {
        return Err(format!("os.exe_path expects 0 arguments, got {}", args.len()).into());
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
        return Err(format!("os.cwd expects 0 arguments, got {}", args.len()).into());
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
        return Err(format!("os.chdir expects 1 argument, got {}", args.len()).into());
    }

    let path = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Err("os.chdir: argument must be a string".to_string().into());
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

/// Run an external program to completion, capturing its output.
/// Usage: os.exec("olang", ["script.ol", "arg"])
///   -> Result<{ code: Int, stdout: String, stderr: String }, Error>
/// The program's exit code is captured (or -1 if it was killed by a signal);
/// stdout/stderr are returned as strings. An Err is returned only when the
/// program could not be started at all (e.g. it was not found).
fn os_exec(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 && args.len() != 3 {
        return Err(format!(
            "os.exec expects 2 or 3 arguments (program, args, options?), got {}",
            args.len()
        )
        .into());
    }

    let program = match &args[0] {
        Value::String(s) => s.as_ref().clone(),
        _ => {
            return Err("os.exec: first argument (program) must be a string"
                .to_string()
                .into());
        }
    };

    let arg_list = match &args[1] {
        Value::List(items) => items,
        _ => {
            return Err("os.exec: second argument (args) must be a list of strings"
                .to_string()
                .into());
        }
    };

    let mut cmd_args: Vec<String> = Vec::with_capacity(arg_list.len());
    for item in arg_list.iter() {
        match item {
            Value::String(s) => cmd_args.push(s.as_ref().clone()),
            other => {
                return Err(format!(
                    "os.exec: argument list must contain only strings, found {}",
                    other.type_name()
                )
                .into());
            }
        }
    }

    // Optional third argument: #{ "cwd": ..., "stdin": ..., "env": #{...} }.
    let mut cwd: Option<String> = None;
    let mut stdin_data: Option<String> = None;
    let mut env_vars: Vec<(String, String)> = Vec::new();
    if let Some(options) = args.get(2) {
        let fields = match options {
            Value::Map(m) => m.as_ref().clone(),
            Value::Struct { fields, .. } => fields.as_ref().clone(),
            _ => {
                return Err("os.exec: options must be a map or object"
                    .to_string()
                    .into());
            }
        };
        for (key, value) in &fields {
            match (key.as_str(), value) {
                ("cwd", Value::String(s)) => cwd = Some(s.as_ref().clone()),
                ("stdin", Value::String(s)) => stdin_data = Some(s.as_ref().clone()),
                ("env", Value::Map(m)) => {
                    for (k, v) in m.iter() {
                        match v {
                            Value::String(s) => env_vars.push((k.clone(), s.as_ref().clone())),
                            other => {
                                return Err(format!(
                                    "os.exec: env values must be strings, got {}",
                                    other.type_name()
                                )
                                .into());
                            }
                        }
                    }
                }
                (other_key, _) => {
                    return Err(format!(
                        "os.exec: unknown or mistyped option '{}' (supported: cwd, stdin, env)",
                        other_key
                    )
                    .into());
                }
            }
        }
    }

    let mut command = process::Command::new(&program);
    command.args(&cmd_args);
    if let Some(dir) = &cwd {
        command.current_dir(dir);
    }
    for (k, v) in &env_vars {
        command.env(k, v);
    }

    // With stdin data the child is spawned piped and fed before collecting.
    let output_result = if let Some(input) = stdin_data {
        command
            .stdin(process::Stdio::piped())
            .stdout(process::Stdio::piped())
            .stderr(process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                use std::io::Write;
                if let Some(mut pipe) = child.stdin.take() {
                    pipe.write_all(input.as_bytes())?;
                }
                child.wait_with_output()
            })
    } else {
        command.output()
    };

    match output_result {
        Ok(output) => {
            let code = output.status.code().unwrap_or(-1) as i64;
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            let mut fields = HashMap::new();
            fields.insert("code".to_string(), Value::Integer(code));
            fields.insert("stdout".to_string(), Value::String(Arc::new(stdout)));
            fields.insert("stderr".to_string(), Value::String(Arc::new(stderr)));
            Ok(Value::Ok(Box::new(Value::Struct {
                type_name: "ExecResult".to_string(),
                fields: std::sync::Arc::new(fields),
            })))
        }
        Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "os.exec: failed to run '{}': {}",
            program, e
        )))))),
    }
}

/// Read one line from stdin: Ok(line) without the trailing newline, or
/// Err("eof") when the stream ends. The missing primitive for interactive
/// programs — REPLs, shells, prompts — and for reading piped input line
/// by line.
/// Usage: os.read_line() -> Result<String, Error>
/// `os.stdin()` — read all of standard input to end-of-file as one
/// string. The pipe-friendly primitive: `cat log | olang analyze.ol`.
fn os_stdin(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if !args.is_empty() {
        return Err(format!("os.stdin expects 0 arguments, got {}", args.len()).into());
    }
    let mut buf = String::new();
    match std::io::Read::read_to_string(&mut std::io::stdin().lock(), &mut buf) {
        Ok(_) => Ok(Value::Ok(Box::new(Value::String(Arc::new(buf))))),
        Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "os.stdin: {}",
            e
        )))))),
    }
}

/// `os.stdin_lines()` — all of standard input as a list of lines, line
/// endings stripped (both \n and \r\n).
fn os_stdin_lines(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if !args.is_empty() {
        return Err(format!("os.stdin_lines expects 0 arguments, got {}", args.len()).into());
    }
    let mut buf = String::new();
    match std::io::Read::read_to_string(&mut std::io::stdin().lock(), &mut buf) {
        Ok(_) => {
            let lines: Vec<Value> = buf
                .lines()
                .map(|l| Value::String(Arc::new(l.to_string())))
                .collect();
            Ok(Value::Ok(Box::new(Value::List(lines.into()))))
        }
        Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "os.stdin_lines: {}",
            e
        )))))),
    }
}

fn os_read_line(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if !args.is_empty() {
        return Err(format!("os.read_line expects no arguments, got {}", args.len()).into());
    }
    let mut line = String::new();
    match std::io::BufRead::read_line(&mut std::io::stdin().lock(), &mut line) {
        Ok(0) => Ok(Value::Err(Box::new(Value::String(Arc::new(
            "eof".to_string(),
        ))))),
        Ok(_) => {
            if line.ends_with('\n') {
                line.pop();
                if line.ends_with('\r') {
                    line.pop();
                }
            }
            Ok(Value::Ok(Box::new(Value::String(Arc::new(line)))))
        }
        Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "os.read_line: {}",
            e
        )))))),
    }
}

/// Get path separator for the current OS
/// Usage: os.path_separator() -> Result<String, Error>
fn os_path_separator(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if !args.is_empty() {
        return Err(format!("os.path_separator expects 0 arguments, got {}", args.len()).into());
    }

    let separator = std::path::MAIN_SEPARATOR.to_string();
    Ok(Value::String(Arc::new(separator)))
}

/// Get home directory
/// Usage: os.home_dir() -> Result<String, Error>
fn os_home_dir(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if !args.is_empty() {
        return Err(format!("os.home_dir expects 0 arguments, got {}", args.len()).into());
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
        return Err(format!("os.temp_dir expects 0 arguments, got {}", args.len()).into());
    }

    let temp_dir = env::temp_dir();
    Ok(Value::String(Arc::new(
        temp_dir.to_string_lossy().to_string(),
    )))
}

/// Exit the program with a status code
/// Usage: os.exit(0) -> Never returns
fn os_exit(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("os.exit expects 1 argument, got {}", args.len()).into());
    }

    let exit_code = match &args[0] {
        Value::Integer(i) => *i as i32,
        _ => {
            return Err("os.exit: argument must be an integer".to_string().into());
        }
    };

    process::exit(exit_code);
}

/// Whether standard output is a terminal (a TTY) rather than a pipe or
/// file — the signal a program uses to decide whether ANSI styling
/// will render. `Ok(true)` when interactive, `Ok(false)` otherwise
/// (including under wasm, which has no terminal).
fn os_is_tty(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if !args.is_empty() {
        return Err(format!("os.is_tty expects 0 arguments, got {}", args.len()).into());
    }
    use std::io::IsTerminal;
    Ok(Value::Boolean(std::io::stdout().is_terminal()))
}

/// Flush standard output. `print` without a newline is buffered, so a
/// progress bar or spinner redrawn with `\r` needs an explicit flush
/// to appear. Returns `Ok(Unit)`.
fn os_flush(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if !args.is_empty() {
        return Err(format!("os.flush expects 0 arguments, got {}", args.len()).into());
    }
    use std::io::Write;
    let _ = std::io::stdout().flush();
    Ok(Value::Unit)
}

/// Set once an interrupt (SIGINT / Ctrl-C) has been seen, and cleared by
/// `reset_interrupt`. The handler only flips this flag, so a long-running
/// loop can notice it and shut down cleanly instead of being killed.
static INTERRUPTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// The OS handler can be installed only once per process; remember that we
/// have, so `on_interrupt` is idempotent.
static HANDLER_INSTALLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Shutdown handlers registered by `os.on_shutdown`: each runs on its own
/// thread against a thread-safe clone of the registering interpreter when
/// SIGINT or SIGTERM arrives, with the signal's name ("INT" or "TERM" is
/// not distinguishable through the handler crate, so "shutdown").
type ShutdownHandler = (crate::interpreter::Interpreter, Value);
static SHUTDOWN_HANDLERS: std::sync::Mutex<Vec<ShutdownHandler>> =
    std::sync::Mutex::new(Vec::new());

/// The one process-wide signal handler: records the interrupt (for
/// `os.interrupted()`) and runs every registered shutdown handler.
fn install_signal_handler() -> Result<(), String> {
    use std::sync::atomic::Ordering;
    if HANDLER_INSTALLED.swap(true, Ordering::SeqCst) {
        return Ok(());
    }
    let installed = ctrlc::set_handler(|| {
        INTERRUPTED.store(true, Ordering::SeqCst);
        // A shutdown is one-shot: the handlers are taken, each with the
        // interpreter clone it registered, and run on their own threads.
        let handlers: Vec<ShutdownHandler> = SHUTDOWN_HANDLERS
            .lock()
            .map(|mut h| std::mem::take(&mut *h))
            .unwrap_or_default();
        for (mut interp, handler) in handlers {
            let _ = std::thread::Builder::new()
                .name("olang-shutdown".to_string())
                .spawn(move || {
                    if let Err(e) = interp.call_function(
                        handler,
                        vec![Value::String(Arc::new("shutdown".to_string()))],
                    ) {
                        eprintln!("shutdown handler failed: {}", e);
                    }
                });
        }
    });
    if let Err(e) = installed {
        HANDLER_INSTALLED.store(false, Ordering::SeqCst);
        return Err(e.to_string());
    }
    Ok(())
}

/// `os.on_shutdown(handler)`, the interpreter-level half: register the
/// handler with a thread-safe clone of the program to run it on.
pub fn register_shutdown_handler(
    interpreter: crate::interpreter::Interpreter,
    handler: Value,
) -> Result<(), String> {
    install_signal_handler()?;
    SHUTDOWN_HANDLERS
        .lock()
        .map_err(|_| "shutdown handlers poisoned".to_string())?
        .push((interpreter, handler));
    Ok(())
}

/// `os.on_interrupt()` — install a Ctrl-C (SIGINT) handler that records
/// the interrupt instead of terminating the process, and clear any prior
/// interrupt. After this, `os.interrupted()` reports whether Ctrl-C has
/// been pressed, so a server or long-running loop can drain and exit
/// gracefully. Idempotent. Returns `Ok(Unit)`.
fn os_on_interrupt(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if !args.is_empty() {
        return Err(format!("os.on_interrupt expects 0 arguments, got {}", args.len()).into());
    }
    use std::sync::atomic::Ordering;
    if let Err(e) = install_signal_handler() {
        // Environmental, not misuse — e.g. another handler was installed
        // outside olang. The caller can carry on without interrupt
        // trapping, so this stays a Result rather than raising.
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "os.on_interrupt: could not install handler: {}",
            e
        ))))));
    }
    INTERRUPTED.store(false, Ordering::SeqCst);
    Ok(Value::Ok(Box::new(Value::Unit)))
}

/// `os.interrupted()` — whether Ctrl-C has been pressed since the last
/// `on_interrupt`/`reset_interrupt`. The poll a graceful loop checks:
/// `while os.interrupted() == false { ... }`.
fn os_interrupted(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if !args.is_empty() {
        return Err(format!("os.interrupted expects 0 arguments, got {}", args.len()).into());
    }
    Ok(Value::Boolean(
        INTERRUPTED.load(std::sync::atomic::Ordering::SeqCst),
    ))
}

/// `os.reset_interrupt()` — clear the interrupt flag, so a supervisor can
/// arm for the next Ctrl-C after handling one. Returns `Ok(Unit)`.
fn os_reset_interrupt(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if !args.is_empty() {
        return Err(format!("os.reset_interrupt expects 0 arguments, got {}", args.len()).into());
    }
    INTERRUPTED.store(false, std::sync::atomic::Ordering::SeqCst);
    Ok(Value::Unit)
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
            Value::Ok(inner) => inner,
            Value::Err(e) => panic!("Expected Ok result, got Err: {:?}", e),
            // 0.64: infallible functions return the value directly.
            bare => bare,
        }
    }

    /// Assert a call reports failure, either way it can now.
    ///
    /// 0.64 split the two: misuse (bad arity or type) **raises**, which is
    /// a Rust `Err`; environmental failure still returns `Ok(Value::Err)`.
    /// Tests that only care *that* the call failed use this; tests that
    /// care *which* assert on the specific shape.
    #[allow(dead_code)]
    fn assert_fails(result: Result<Value, Box<dyn std::error::Error>>) {
        match result {
            Err(_) => {}
            Ok(Value::Err(_)) => {}
            Ok(other) => panic!("expected a failure, got: {:?}", other),
        }
    }

    // Helper function to assert Err result and extract error message
    fn assert_err(result: &Value) -> &Value {
        match result {
            Value::Err(inner) => inner,
            _ => {
                panic!("Expected Err result, got: {:?}", result)
            }
        }
    }

    // Helper function to extract string from Value
    fn extract_string(value: &Value) -> &str {
        match value {
            Value::String(s) => s,
            _ => {
                panic!("Expected string value, got: {:?}", value)
            }
        }
    }

    // Helper function to extract boolean from Value
    fn extract_bool(value: &Value) -> bool {
        match value {
            Value::Boolean(b) => *b,
            _ => {
                panic!("Expected boolean value, got: {:?}", value)
            }
        }
    }

    // Helper function to extract integer from Value
    fn extract_int(value: &Value) -> i64 {
        match value {
            Value::Integer(i) => *i,
            _ => {
                panic!("Expected integer value, got: {:?}", value)
            }
        }
    }

    // Helper function to extract list from Value
    fn extract_list(value: &Value) -> &[Value] {
        match value {
            Value::List(items) => items,
            _ => {
                panic!("Expected list value, got: {:?}", value)
            }
        }
    }

    // Helper function to extract struct fields from Value
    fn extract_struct_fields(value: &Value) -> &HashMap<String, Value> {
        match value {
            Value::Struct { fields, .. } => fields,
            _ => {
                panic!("Expected struct value, got: {:?}", value)
            }
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
                    panic!(
                        "Expected builtin function for {}, got: {:?}",
                        func_name, fields[func_name]
                    );
                }
            }
        } else {
            panic!("Expected struct for os module, got: {:?}", module);
        }
    }

    #[test]
    fn test_environment_variable_operations() {
        let test_var = "OLANG_TEST_VAR";
        let test_value = "test_value_123";

        // Clean up any existing test variable
        // SAFETY: test-only env mutation on a test-unique variable name.
        unsafe { env::remove_var(test_var) };

        // Test has_env for non-existent variable
        let result = os_has_env(vec![string_val(test_var)]).unwrap();
        let exists = assert_ok(&result);
        assert!(!extract_bool(exists), "Variable should not exist initially");

        // Test get_env for non-existent variable
        assert_fails(os_get_env(vec![string_val(test_var)])); // Should return error for non-existent variable

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
        // SAFETY: test-only env mutation on a test-unique variable name.
        unsafe { env::remove_var(test_var) };
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
        let result = os_set_env(vec![string_val(test_var), float_val(2.5)]).unwrap();
        assert_ok(&result);
        assert_eq!(env::var(test_var).unwrap(), "2.5");

        // Test with boolean
        let result = os_set_env(vec![string_val(test_var), bool_val(true)]).unwrap();
        assert_ok(&result);
        assert_eq!(env::var(test_var).unwrap(), "true");

        // Clean up
        // SAFETY: test-only env mutation on a test-unique variable name.
        unsafe { env::remove_var(test_var) };
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
            panic!("First argument should be a string, got: {:?}", args_list[0]);
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
            // Extra arguments are misuse, so these raise rather than
            // returning an Err the caller might unwrap_or past.
            let result = call_os_function(func_name, vec![string_val("extra")]);
            assert!(
                result.is_err(),
                "{} with an extra argument should raise, got: {:?}",
                func_name,
                result
            );
        }

        // Test functions that expect 1 argument
        assert_fails(os_get_env(vec![]));

        assert_fails(os_get_env(vec![string_val("TEST"), string_val("EXTRA")]));

        // Test functions that expect 2 arguments
        assert_fails(os_set_env(vec![string_val("TEST")]));

        assert_fails(os_set_env(vec![
            string_val("TEST"),
            string_val("VALUE"),
            string_val("EXTRA"),
        ]));
    }

    #[test]
    fn test_type_validation() {
        // Test get_env with non-string argument
        assert_fails(os_get_env(vec![int_val(42)]));

        // Test set_env with non-string first argument
        assert_fails(os_set_env(vec![int_val(42), string_val("value")]));

        // Test set_env with unsupported type for second argument
        assert_fails(os_set_env(vec![
            string_val("TEST"),
            Value::List(vec![].into()),
        ]));

        // Test has_env with non-string argument
        assert_fails(os_has_env(vec![bool_val(true)]));

        // Test remove_env with non-string argument
        assert_fails(os_remove_env(vec![float_val(2.5)]));

        // Test chdir with non-string argument
        assert_fails(os_chdir(vec![int_val(123)]));

        // Test exit with non-integer argument
        // Note: We can't actually test exit because it would terminate the test process
        // But we can test the argument validation
        assert_fails(os_exit(vec![string_val("not_a_number")]));
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
        assert_fails(os_get_env(vec![string_val("OLANG_NONEXISTENT_VAR_12345")]));
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
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Unknown os function")
        );
    }

    #[test]
    fn test_environment_variable_edge_cases() {
        let test_var = "OLANG_EDGE_TEST_VAR";

        // Test with empty string value
        // SAFETY: test-only env mutation on a test-unique variable name.
        unsafe { env::remove_var(test_var) };
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
        // SAFETY: test-only env mutation on a test-unique variable name.
        unsafe { env::remove_var(test_var) };
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
