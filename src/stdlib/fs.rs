use crate::ast::Value;
use std::collections::HashMap;
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::sync::Arc;

/// Error types for filesystem operations
#[derive(Debug, thiserror::Error)]
pub enum FsError {
    #[error("File not found: {path}")]
    FileNotFound { path: String },
    #[error("Permission denied: {path}")]
    PermissionDenied { path: String },
    #[error("Directory not empty: {path}")]
    DirectoryNotEmpty { path: String },
    #[error("Path already exists: {path}")]
    PathExists { path: String },
    #[error("Invalid path: {path}")]
    InvalidPath { path: String },
    #[error("IO error: {message}")]
    IoError { message: String },
}

impl From<io::Error> for FsError {
    fn from(err: io::Error) -> Self {
        match err.kind() {
            io::ErrorKind::NotFound => FsError::IoError {
                message: format!("File or directory not found: {}", err),
            },
            io::ErrorKind::PermissionDenied => FsError::IoError {
                message: format!("Permission denied: {}", err),
            },
            io::ErrorKind::AlreadyExists => FsError::IoError {
                message: format!("File already exists: {}", err),
            },
            _ => FsError::IoError {
                message: err.to_string(),
            },
        }
    }
}

/// Creates the fs module with all filesystem functions
pub fn create_fs_module() -> Value {
    let mut module = HashMap::new();

    // Essential file operations
    module.insert(
        "read_file".to_string(),
        create_builtin_function("read_file", 1),
    );
    module.insert(
        "write_file".to_string(),
        create_builtin_function("write_file", 2),
    );
    module.insert(
        "append_file".to_string(),
        create_builtin_function("append_file", 2),
    );

    // File/directory existence and type checking
    module.insert("exists".to_string(), create_builtin_function("exists", 1));
    module.insert("is_file".to_string(), create_builtin_function("is_file", 1));
    module.insert("is_dir".to_string(), create_builtin_function("is_dir", 1));

    // Directory operations
    module.insert(
        "list_dir".to_string(),
        create_builtin_function("list_dir", 1),
    );
    module.insert(
        "create_dir".to_string(),
        create_builtin_function("create_dir", 1),
    );
    module.insert(
        "create_dir_all".to_string(),
        create_builtin_function("create_dir_all", 1),
    );
    module.insert(
        "remove_dir".to_string(),
        create_builtin_function("remove_dir", 1),
    );
    module.insert(
        "remove_dir_all".to_string(),
        create_builtin_function("remove_dir_all", 1),
    );

    // File manipulation
    module.insert(
        "remove_file".to_string(),
        create_builtin_function("remove_file", 1),
    );
    module.insert(
        "copy_file".to_string(),
        create_builtin_function("copy_file", 2),
    );
    module.insert(
        "move_file".to_string(),
        create_builtin_function("move_file", 2),
    );

    // File information
    module.insert(
        "file_size".to_string(),
        create_builtin_function("file_size", 1),
    );
    module.insert(
        "file_info".to_string(),
        create_builtin_function("file_info", 1),
    );

    Value::Struct {
        type_name: "Module".to_string(),
        fields: module,
    }
}

/// Helper function to create builtin function values
fn create_builtin_function(name: &str, arity: usize) -> Value {
    Value::Builtin(crate::ast::BuiltinFunction {
        name: format!("fs.{}", name),
        arity,
    })
}

/// Main dispatcher for fs function calls
pub fn call_fs_function(name: &str, args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    match name {
        "read_file" => read_file(args),
        "write_file" => write_file(args),
        "append_file" => append_file(args),
        "exists" => exists(args),
        "is_file" => is_file(args),
        "is_dir" => is_dir(args),
        "list_dir" => list_dir(args),
        "create_dir" => create_dir(args),
        "create_dir_all" => create_dir_all(args),
        "remove_dir" => remove_dir(args),
        "remove_dir_all" => remove_dir_all(args),
        "remove_file" => remove_file(args),
        "copy_file" => copy_file(args),
        "move_file" => move_file(args),
        "file_size" => file_size(args),
        "file_info" => file_info(args),
        _ => Err(format!("Unknown fs function: {}", name).into()),
    }
}

/// Read entire file contents as a string
/// Usage: fs.read_file("/path/to/file.txt") -> Result<String, Error>
fn read_file(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "read_file expects 1 argument, got {}",
            args.len()
        ))))));
    }

    let path_str = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "read_file: path must be a string".to_string(),
            )))))
        }
    };

    match fs::read_to_string(path_str) {
        Ok(contents) => Ok(Value::Ok(Box::new(Value::String(Arc::new(contents))))),
        Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "Failed to read file '{}': {}",
            path_str, e
        )))))),
    }
}

/// Write string contents to a file
/// Usage: fs.write_file("/path/to/file.txt", "contents") -> Result<Unit, Error>
fn write_file(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "write_file expects 2 arguments, got {}",
            args.len()
        ))))));
    }

    let path_str = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "write_file: path must be a string".to_string(),
            )))))
        }
    };

    let contents = match &args[1] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "write_file: contents must be a string".to_string(),
            )))))
        }
    };

    match fs::write(path_str, contents) {
        Ok(()) => Ok(Value::Ok(Box::new(Value::Unit))),
        Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "Failed to write file '{}': {}",
            path_str, e
        )))))),
    }
}

/// Append string contents to a file
/// Usage: fs.append_file("/path/to/file.txt", "more contents") -> Result<Unit, Error>
fn append_file(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "append_file expects 2 arguments, got {}",
            args.len()
        ))))));
    }

    let path_str = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "append_file: path must be a string".to_string(),
            )))))
        }
    };

    let contents = match &args[1] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "append_file: contents must be a string".to_string(),
            )))))
        }
    };

    match fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path_str)
    {
        Ok(mut file) => match file.write_all(contents.as_bytes()) {
            Ok(()) => Ok(Value::Ok(Box::new(Value::Unit))),
            Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
                "Failed to append to file '{}': {}",
                path_str, e
            )))))),
        },
        Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "Failed to open file '{}' for appending: {}",
            path_str, e
        )))))),
    }
}

/// Check if a file or directory exists
/// Usage: fs.exists("/path/to/file") -> Bool
fn exists(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("exists expects 1 argument, got {}", args.len()).into());
    }

    let path_str = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => return Err("exists: path must be a string".into()),
    };

    Ok(Value::Boolean(Path::new(path_str).exists()))
}

/// Check if path is a file
/// Usage: fs.is_file("/path/to/file") -> Bool
fn is_file(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("is_file expects 1 argument, got {}", args.len()).into());
    }

    let path_str = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => return Err("is_file: path must be a string".into()),
    };

    Ok(Value::Boolean(Path::new(path_str).is_file()))
}

/// Check if path is a directory
/// Usage: fs.is_dir("/path/to/dir") -> Bool
fn is_dir(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("is_dir expects 1 argument, got {}", args.len()).into());
    }

    let path_str = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => return Err("is_dir: path must be a string".into()),
    };

    Ok(Value::Boolean(Path::new(path_str).is_dir()))
}

/// List directory contents
/// Usage: fs.list_dir("/path/to/dir") -> Result<List<String>, Error>
fn list_dir(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "list_dir expects 1 argument, got {}",
            args.len()
        ))))));
    }

    let path_str = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "list_dir: path must be a string".to_string(),
            )))))
        }
    };

    match fs::read_dir(path_str) {
        Ok(entries) => {
            let mut files = Vec::new();
            for entry in entries {
                match entry {
                    Ok(entry) => {
                        if let Some(name) = entry.file_name().to_str() {
                            files.push(Value::String(Arc::new(name.to_string())));
                        }
                    }
                    Err(e) => {
                        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
                            "Error reading directory entry: {}",
                            e
                        ))))));
                    }
                }
            }
            Ok(Value::Ok(Box::new(Value::List(files.into()))))
        }
        Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "Failed to read directory '{}': {}",
            path_str, e
        )))))),
    }
}

/// Create a directory
/// Usage: fs.create_dir("/path/to/new/dir") -> Result<Unit, Error>
fn create_dir(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "create_dir expects 1 argument, got {}",
            args.len()
        ))))));
    }

    let path_str = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "create_dir: path must be a string".to_string(),
            )))))
        }
    };

    match fs::create_dir(path_str) {
        Ok(()) => Ok(Value::Ok(Box::new(Value::Unit))),
        Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "Failed to create directory '{}': {}",
            path_str, e
        )))))),
    }
}

/// Create a directory and all parent directories
/// Usage: fs.create_dir_all("/path/to/new/dir") -> Result<Unit, Error>
fn create_dir_all(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "create_dir_all expects 1 argument, got {}",
            args.len()
        ))))));
    }

    let path_str = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "create_dir_all: path must be a string".to_string(),
            )))))
        }
    };

    match fs::create_dir_all(path_str) {
        Ok(()) => Ok(Value::Ok(Box::new(Value::Unit))),
        Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "Failed to create directory tree '{}': {}",
            path_str, e
        )))))),
    }
}

/// Remove an empty directory
/// Usage: fs.remove_dir("/path/to/dir") -> Result<Unit, Error>
fn remove_dir(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "remove_dir expects 1 argument, got {}",
            args.len()
        ))))));
    }

    let path_str = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "remove_dir: path must be a string".to_string(),
            )))))
        }
    };

    match fs::remove_dir(path_str) {
        Ok(()) => Ok(Value::Ok(Box::new(Value::Unit))),
        Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "Failed to remove directory '{}': {}",
            path_str, e
        )))))),
    }
}

/// Remove a directory and all its contents
/// Usage: fs.remove_dir_all("/path/to/dir") -> Result<Unit, Error>
fn remove_dir_all(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "remove_dir_all expects 1 argument, got {}",
            args.len()
        ))))));
    }

    let path_str = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "remove_dir_all: path must be a string".to_string(),
            )))))
        }
    };

    match fs::remove_dir_all(path_str) {
        Ok(()) => Ok(Value::Ok(Box::new(Value::Unit))),
        Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "Failed to remove directory tree '{}': {}",
            path_str, e
        )))))),
    }
}

/// Remove a file
/// Usage: fs.remove_file("/path/to/file") -> Result<Unit, Error>
fn remove_file(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "remove_file expects 1 argument, got {}",
            args.len()
        ))))));
    }

    let path_str = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "remove_file: path must be a string".to_string(),
            )))))
        }
    };

    match fs::remove_file(path_str) {
        Ok(()) => Ok(Value::Ok(Box::new(Value::Unit))),
        Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "Failed to remove file '{}': {}",
            path_str, e
        )))))),
    }
}

/// Copy a file
/// Usage: fs.copy_file("/source/path", "/dest/path") -> Result<Unit, Error>
fn copy_file(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "copy_file expects 2 arguments, got {}",
            args.len()
        ))))));
    }

    let src_str = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "copy_file: source path must be a string".to_string(),
            )))))
        }
    };

    let dest_str = match &args[1] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "copy_file: destination path must be a string".to_string(),
            )))))
        }
    };

    match fs::copy(src_str, dest_str) {
        Ok(_) => Ok(Value::Ok(Box::new(Value::Unit))),
        Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "Failed to copy '{}' to '{}': {}",
            src_str, dest_str, e
        )))))),
    }
}

/// Move/rename a file
/// Usage: fs.move_file("/old/path", "/new/path") -> Result<Unit, Error>
fn move_file(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "move_file expects 2 arguments, got {}",
            args.len()
        ))))));
    }

    let src_str = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "move_file: source path must be a string".to_string(),
            )))))
        }
    };

    let dest_str = match &args[1] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "move_file: destination path must be a string".to_string(),
            )))))
        }
    };

    match fs::rename(src_str, dest_str) {
        Ok(()) => Ok(Value::Ok(Box::new(Value::Unit))),
        Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "Failed to move '{}' to '{}': {}",
            src_str, dest_str, e
        )))))),
    }
}

/// Get file size in bytes
/// Usage: fs.file_size("/path/to/file") -> Result<Int, Error>
fn file_size(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "file_size expects 1 argument, got {}",
            args.len()
        ))))));
    }

    let path_str = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "file_size: path must be a string".to_string(),
            )))))
        }
    };

    match fs::metadata(path_str) {
        Ok(metadata) => Ok(Value::Ok(Box::new(Value::Integer(metadata.len() as i64)))),
        Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "Failed to get file size for '{}': {}",
            path_str, e
        )))))),
    }
}

/// Get detailed file information
/// Usage: fs.file_info("/path/to/file") -> Result<Struct, Error>
fn file_info(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "file_info expects 1 argument, got {}",
            args.len()
        ))))));
    }

    let path_str = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "file_info: path must be a string".to_string(),
            )))))
        }
    };

    match fs::metadata(path_str) {
        Ok(metadata) => {
            let mut info = HashMap::new();
            info.insert("size".to_string(), Value::Integer(metadata.len() as i64));
            info.insert("is_file".to_string(), Value::Boolean(metadata.is_file()));
            info.insert("is_dir".to_string(), Value::Boolean(metadata.is_dir()));
            info.insert(
                "readonly".to_string(),
                Value::Boolean(metadata.permissions().readonly()),
            );

            Ok(Value::Ok(Box::new(Value::Struct {
                type_name: "FileInfo".to_string(),
                fields: info,
            })))
        }
        Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "Failed to get file info for '{}': {}",
            path_str, e
        )))))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::Value;
    use std::fs;

    use std::sync::Arc;
    use tempfile::{tempdir, NamedTempFile};

    // Helper functions to create test values
    fn string_val(s: &str) -> Value {
        Value::String(Arc::new(s.to_string()))
    }

    fn int_val(n: i64) -> Value {
        Value::Integer(n)
    }

    fn bool_val(b: bool) -> Value {
        Value::Boolean(b)
    }

    // Helper to assert Result<T, E> success
    fn assert_ok(result: &Value) -> &Value {
        match result {
            Value::Ok(inner) => inner.as_ref(),
            _ => panic!("Expected Ok result, got: {:?}", result),
        }
    }

    // Helper to assert Result<T, E> error
    fn assert_err(result: &Value) -> &Value {
        match result {
            Value::Err(inner) => inner.as_ref(),
            _ => panic!("Expected Err result, got: {:?}", result),
        }
    }

    // Helper to extract string from Value::String
    fn extract_string(value: &Value) -> &str {
        match value {
            Value::String(s) => s.as_ref(),
            _ => panic!("Expected string value, got: {:?}", value),
        }
    }

    // Helper to extract integer from Value::Integer
    fn extract_int(value: &Value) -> i64 {
        match value {
            Value::Integer(i) => *i,
            _ => panic!("Expected integer value, got: {:?}", value),
        }
    }

    // Helper to extract boolean from Value::Boolean
    fn extract_bool(value: &Value) -> bool {
        match value {
            Value::Boolean(b) => *b,
            _ => panic!("Expected boolean value, got: {:?}", value),
        }
    }

    #[test]
    fn test_fs_module_creation() {
        let module = create_fs_module();

        if let Value::Struct { type_name, fields } = module {
            assert_eq!(type_name, "Module");

            // Check that all expected functions are present
            let expected_functions = vec![
                "read_file",
                "write_file",
                "append_file",
                "exists",
                "is_file",
                "is_dir",
                "list_dir",
                "create_dir",
                "create_dir_all",
                "remove_dir",
                "remove_dir_all",
                "remove_file",
                "copy_file",
                "move_file",
                "file_size",
                "file_info",
            ];

            for func_name in expected_functions {
                assert!(
                    fields.contains_key(func_name),
                    "Missing function: {}",
                    func_name
                );

                if let Value::Builtin(builtin) = &fields[func_name] {
                    assert_eq!(builtin.name, format!("fs.{}", func_name));
                } else {
                    panic!("Expected builtin function for {}", func_name);
                }
            }
        } else {
            panic!("Expected struct for fs module");
        }
    }

    #[test]
    fn test_write_and_read_file() {
        let temp_file = NamedTempFile::new().unwrap();
        let file_path = temp_file.path().to_str().unwrap();

        // Test writing to file
        let content = "Hello, world!\nThis is a test file.";
        let write_result = write_file(vec![string_val(file_path), string_val(content)]).unwrap();
        assert_ok(&write_result);

        // Test reading from file
        let read_result = read_file(vec![string_val(file_path)]).unwrap();
        let read_content = assert_ok(&read_result);
        assert_eq!(extract_string(read_content), content);
    }

    #[test]
    fn test_append_file() {
        let temp_file = NamedTempFile::new().unwrap();
        let file_path = temp_file.path().to_str().unwrap();

        // Write initial content
        let initial_content = "Line 1\n";
        let write_result =
            write_file(vec![string_val(file_path), string_val(initial_content)]).unwrap();
        assert_ok(&write_result);

        // Append more content
        let append_content = "Line 2\nLine 3\n";
        let append_result =
            append_file(vec![string_val(file_path), string_val(append_content)]).unwrap();
        assert_ok(&append_result);

        // Read and verify
        let read_result = read_file(vec![string_val(file_path)]).unwrap();
        let final_content = assert_ok(&read_result);
        assert_eq!(extract_string(final_content), "Line 1\nLine 2\nLine 3\n");
    }

    #[test]
    fn test_append_file_creates_new() {
        let temp_dir = tempdir().unwrap();
        let file_path = temp_dir.path().join("new_file.txt");
        let file_path_str = file_path.to_str().unwrap();

        // Append to non-existent file (should create it)
        let content = "Created by append";
        let append_result =
            append_file(vec![string_val(file_path_str), string_val(content)]).unwrap();
        assert_ok(&append_result);

        // Verify file was created and has correct content
        let read_result = read_file(vec![string_val(file_path_str)]).unwrap();
        let read_content = assert_ok(&read_result);
        assert_eq!(extract_string(read_content), content);
    }

    #[test]
    fn test_file_operations_error_handling() {
        // Test reading non-existent file
        let read_result = read_file(vec![string_val("/nonexistent/file.txt")]).unwrap();
        assert_err(&read_result);

        // Test writing to invalid path (directory doesn't exist)
        let write_result = write_file(vec![
            string_val("/nonexistent/dir/file.txt"),
            string_val("content"),
        ])
        .unwrap();
        assert_err(&write_result);

        // Test invalid argument types
        let read_result = read_file(vec![int_val(42)]).unwrap();
        assert_err(&read_result);

        let write_result = write_file(vec![string_val("file.txt"), int_val(42)]).unwrap();
        assert_err(&write_result);

        // Test wrong number of arguments
        let read_result = read_file(vec![]).unwrap();
        assert_err(&read_result);

        let write_result = write_file(vec![string_val("file.txt")]).unwrap();
        assert_err(&write_result);
    }

    #[test]
    fn test_exists_is_file_is_dir() {
        let temp_dir = tempdir().unwrap();
        let dir_path = temp_dir.path().to_str().unwrap();

        let temp_file = NamedTempFile::new_in(&temp_dir).unwrap();
        let file_path = temp_file.path().to_str().unwrap();

        // Test exists
        assert!(extract_bool(&exists(vec![string_val(dir_path)]).unwrap()));
        assert!(extract_bool(&exists(vec![string_val(file_path)]).unwrap()));
        assert!(!extract_bool(
            &exists(vec![string_val("/nonexistent/path")]).unwrap()
        ));

        // Test is_dir
        assert!(extract_bool(&is_dir(vec![string_val(dir_path)]).unwrap()));
        assert!(!extract_bool(&is_dir(vec![string_val(file_path)]).unwrap()));
        assert!(!extract_bool(
            &is_dir(vec![string_val("/nonexistent/path")]).unwrap()
        ));

        // Test is_file
        assert!(!extract_bool(&is_file(vec![string_val(dir_path)]).unwrap()));
        assert!(extract_bool(&is_file(vec![string_val(file_path)]).unwrap()));
        assert!(!extract_bool(
            &is_file(vec![string_val("/nonexistent/path")]).unwrap()
        ));

        // Test error conditions
        assert!(exists(vec![int_val(42)]).is_err());
        assert!(is_file(vec![]).is_err());
        assert!(is_dir(vec![string_val("a"), string_val("b")]).is_err());
    }

    #[test]
    fn test_directory_operations() {
        let temp_dir = tempdir().unwrap();
        let test_dir = temp_dir.path().join("test_directory");
        let test_dir_str = test_dir.to_str().unwrap();

        // Test create_dir
        let create_result = create_dir(vec![string_val(test_dir_str)]).unwrap();
        assert_ok(&create_result);
        assert!(test_dir.exists());
        assert!(test_dir.is_dir());

        // Test remove_dir
        let remove_result = remove_dir(vec![string_val(test_dir_str)]).unwrap();
        assert_ok(&remove_result);
        assert!(!test_dir.exists());
    }

    #[test]
    fn test_create_dir_all() {
        let temp_dir = tempdir().unwrap();
        let nested_dir = temp_dir.path().join("level1").join("level2").join("level3");
        let nested_dir_str = nested_dir.to_str().unwrap();

        // Test create_dir_all (creates parent directories)
        let create_result = create_dir_all(vec![string_val(nested_dir_str)]).unwrap();
        assert_ok(&create_result);
        assert!(nested_dir.exists());
        assert!(nested_dir.is_dir());

        // Test that parent directories were also created
        assert!(nested_dir.parent().unwrap().exists());
        assert!(nested_dir.parent().unwrap().parent().unwrap().exists());
    }

    #[test]
    fn test_remove_dir_all() {
        let temp_dir = tempdir().unwrap();
        let test_dir = temp_dir.path().join("test_remove_all");
        let test_dir_str = test_dir.to_str().unwrap();

        // Create directory with nested structure and files
        fs::create_dir_all(&test_dir).unwrap();
        fs::create_dir_all(test_dir.join("subdir")).unwrap();
        fs::write(test_dir.join("file1.txt"), "content1").unwrap();
        fs::write(test_dir.join("subdir").join("file2.txt"), "content2").unwrap();

        // Test remove_dir_all
        let remove_result = remove_dir_all(vec![string_val(test_dir_str)]).unwrap();
        assert_ok(&remove_result);
        assert!(!test_dir.exists());
    }

    #[test]
    fn test_list_dir() {
        let temp_dir = tempdir().unwrap();

        // Create some test files and directories
        let file1 = temp_dir.path().join("file1.txt");
        let file2 = temp_dir.path().join("file2.txt");
        let subdir = temp_dir.path().join("subdir");

        fs::write(&file1, "content1").unwrap();
        fs::write(&file2, "content2").unwrap();
        fs::create_dir(&subdir).unwrap();

        // Test list_dir
        let list_result = list_dir(vec![string_val(temp_dir.path().to_str().unwrap())]).unwrap();
        let file_list = assert_ok(&list_result);

        if let Value::List(entries) = file_list {
            assert_eq!(entries.len(), 3);

            // Convert to strings for easier testing
            let mut names: Vec<String> = entries
                .iter()
                .map(|v| extract_string(v).to_string())
                .collect();
            names.sort();

            assert_eq!(names, vec!["file1.txt", "file2.txt", "subdir"]);
        } else {
            panic!("Expected list result");
        }

        // Test error conditions
        let list_result = list_dir(vec![string_val("/nonexistent/directory")]).unwrap();
        assert_err(&list_result);

        let list_result = list_dir(vec![int_val(42)]).unwrap();
        assert_err(&list_result);
    }

    #[test]
    fn test_file_manipulation() {
        let temp_dir = tempdir().unwrap();

        // Create source file
        let source_file = temp_dir.path().join("source.txt");
        let source_str = source_file.to_str().unwrap();
        let content = "Test file content";
        fs::write(&source_file, content).unwrap();

        // Test copy_file
        let dest_file = temp_dir.path().join("dest.txt");
        let dest_str = dest_file.to_str().unwrap();

        let copy_result = copy_file(vec![string_val(source_str), string_val(dest_str)]).unwrap();
        assert_ok(&copy_result);
        assert!(dest_file.exists());
        assert_eq!(fs::read_to_string(&dest_file).unwrap(), content);

        // Test move_file
        let moved_file = temp_dir.path().join("moved.txt");
        let moved_str = moved_file.to_str().unwrap();

        let move_result = move_file(vec![string_val(dest_str), string_val(moved_str)]).unwrap();
        assert_ok(&move_result);
        assert!(!dest_file.exists());
        assert!(moved_file.exists());
        assert_eq!(fs::read_to_string(&moved_file).unwrap(), content);

        // Test remove_file
        let remove_result = remove_file(vec![string_val(moved_str)]).unwrap();
        assert_ok(&remove_result);
        assert!(!moved_file.exists());
    }

    #[test]
    fn test_file_manipulation_errors() {
        // Test copy non-existent file
        let copy_result = copy_file(vec![
            string_val("/nonexistent/source.txt"),
            string_val("/tmp/dest.txt"),
        ])
        .unwrap();
        assert_err(&copy_result);

        // Test move non-existent file
        let move_result = move_file(vec![
            string_val("/nonexistent/source.txt"),
            string_val("/tmp/dest.txt"),
        ])
        .unwrap();
        assert_err(&move_result);

        // Test remove non-existent file
        let remove_result = remove_file(vec![string_val("/nonexistent/file.txt")]).unwrap();
        assert_err(&remove_result);

        // Test invalid argument types
        let copy_result = copy_file(vec![int_val(42), string_val("dest.txt")]).unwrap();
        assert_err(&copy_result);

        let move_result = move_file(vec![string_val("source.txt"), int_val(42)]).unwrap();
        assert_err(&move_result);

        // Test wrong number of arguments
        let copy_result = copy_file(vec![string_val("file.txt")]).unwrap();
        assert_err(&copy_result);

        let move_result = move_file(vec![]).unwrap();
        assert_err(&move_result);
    }

    #[test]
    fn test_file_size() {
        let temp_file = NamedTempFile::new().unwrap();
        let file_path = temp_file.path().to_str().unwrap();

        // Write content of known size
        let content = "Hello, world!"; // 13 bytes
        fs::write(temp_file.path(), content).unwrap();

        // Test file_size
        let size_result = file_size(vec![string_val(file_path)]).unwrap();
        let size_value = assert_ok(&size_result);
        assert_eq!(extract_int(size_value), 13);

        // Test with empty file
        fs::write(temp_file.path(), "").unwrap();
        let size_result = file_size(vec![string_val(file_path)]).unwrap();
        let size_value = assert_ok(&size_result);
        assert_eq!(extract_int(size_value), 0);

        // Test error conditions
        let size_result = file_size(vec![string_val("/nonexistent/file.txt")]).unwrap();
        assert_err(&size_result);

        let size_result = file_size(vec![int_val(42)]).unwrap();
        assert_err(&size_result);
    }

    #[test]
    fn test_file_info() {
        let temp_file = NamedTempFile::new().unwrap();
        let file_path = temp_file.path().to_str().unwrap();

        // Write content
        let content = "Test content for file info";
        fs::write(temp_file.path(), content).unwrap();

        // Test file_info
        let info_result = file_info(vec![string_val(file_path)]).unwrap();
        let info_value = assert_ok(&info_result);

        if let Value::Struct { type_name, fields } = info_value {
            assert_eq!(type_name, "FileInfo");

            // Check required fields
            assert!(fields.contains_key("size"));
            assert!(fields.contains_key("is_file"));
            assert!(fields.contains_key("is_dir"));
            assert!(fields.contains_key("readonly"));

            // Verify values
            assert_eq!(extract_int(&fields["size"]), content.len() as i64);
            assert!(extract_bool(&fields["is_file"]));
            assert!(!extract_bool(&fields["is_dir"]));
            // readonly can be true or false depending on platform
        } else {
            panic!("Expected FileInfo struct");
        }

        // Test with directory
        let temp_dir = tempdir().unwrap();
        let dir_path = temp_dir.path().to_str().unwrap();

        let info_result = file_info(vec![string_val(dir_path)]).unwrap();
        let info_value = assert_ok(&info_result);

        if let Value::Struct { fields, .. } = info_value {
            assert!(!extract_bool(&fields["is_file"]));
            assert!(extract_bool(&fields["is_dir"]));
        } else {
            panic!("Expected FileInfo struct");
        }

        // Test error conditions
        let info_result = file_info(vec![string_val("/nonexistent/file.txt")]).unwrap();
        assert_err(&info_result);

        let info_result = file_info(vec![int_val(42)]).unwrap();
        assert_err(&info_result);
    }

    #[test]
    fn test_directory_operation_errors() {
        // Test create_dir with invalid path
        let create_result = create_dir(vec![string_val("/nonexistent/parent/dir")]).unwrap();
        assert_err(&create_result);

        // Test remove_dir with non-existent directory
        let remove_result = remove_dir(vec![string_val("/nonexistent/dir")]).unwrap();
        assert_err(&remove_result);

        // Test remove_dir with non-empty directory
        let temp_dir = tempdir().unwrap();
        let test_dir = temp_dir.path().join("non_empty");
        fs::create_dir(&test_dir).unwrap();
        fs::write(test_dir.join("file.txt"), "content").unwrap();

        let remove_result = remove_dir(vec![string_val(test_dir.to_str().unwrap())]).unwrap();
        assert_err(&remove_result);

        // Test invalid argument types
        let create_result = create_dir(vec![int_val(42)]).unwrap();
        assert_err(&create_result);

        let remove_result = remove_dir_all(vec![bool_val(true)]).unwrap();
        assert_err(&remove_result);

        // Test wrong number of arguments
        let create_result = create_dir(vec![]).unwrap();
        assert_err(&create_result);

        let remove_result = remove_dir(vec![string_val("a"), string_val("b")]).unwrap();
        assert_err(&remove_result);
    }

    #[test]
    fn test_function_dispatcher() {
        let temp_file = NamedTempFile::new().unwrap();
        let file_path = temp_file.path().to_str().unwrap();

        // Test successful function calls through dispatcher
        let result = call_fs_function(
            "write_file",
            vec![string_val(file_path), string_val("test content")],
        )
        .unwrap();
        assert_ok(&result);

        let result = call_fs_function("exists", vec![string_val(file_path)]).unwrap();
        assert!(extract_bool(&result));

        // Test unknown function
        assert!(call_fs_function("unknown_func", vec![]).is_err());

        // Test with error conditions
        let result = call_fs_function("read_file", vec![string_val("/nonexistent")]).unwrap();
        assert_err(&result);
    }

    #[test]
    fn test_integration_file_lifecycle() {
        let temp_dir = tempdir().unwrap();
        let file_path = temp_dir.path().join("lifecycle_test.txt");
        let file_path_str = file_path.to_str().unwrap();

        // Initially file should not exist
        assert!(!extract_bool(
            &exists(vec![string_val(file_path_str)]).unwrap()
        ));

        // Create file by writing to it
        let write_result = write_file(vec![
            string_val(file_path_str),
            string_val("Initial content"),
        ])
        .unwrap();
        assert_ok(&write_result);

        // Now file should exist and be a file
        assert!(extract_bool(
            &exists(vec![string_val(file_path_str)]).unwrap()
        ));
        assert!(extract_bool(
            &is_file(vec![string_val(file_path_str)]).unwrap()
        ));
        assert!(!extract_bool(
            &is_dir(vec![string_val(file_path_str)]).unwrap()
        ));

        // Read content back
        let read_result = read_file(vec![string_val(file_path_str)]).unwrap();
        let content = assert_ok(&read_result);
        assert_eq!(extract_string(content), "Initial content");

        // Append more content
        let append_result = append_file(vec![
            string_val(file_path_str),
            string_val("\nAppended line"),
        ])
        .unwrap();
        assert_ok(&append_result);

        // Verify appended content
        let read_result = read_file(vec![string_val(file_path_str)]).unwrap();
        let content = assert_ok(&read_result);
        assert_eq!(extract_string(content), "Initial content\nAppended line");

        // Check file size
        let size_result = file_size(vec![string_val(file_path_str)]).unwrap();
        let size = assert_ok(&size_result);
        assert_eq!(
            extract_int(size),
            "Initial content\nAppended line".len() as i64
        );

        // Copy file
        let copy_path = temp_dir.path().join("copy_test.txt");
        let copy_path_str = copy_path.to_str().unwrap();

        let copy_result =
            copy_file(vec![string_val(file_path_str), string_val(copy_path_str)]).unwrap();
        assert_ok(&copy_result);

        // Verify copy exists and has same content
        assert!(extract_bool(
            &exists(vec![string_val(copy_path_str)]).unwrap()
        ));
        let copy_read_result = read_file(vec![string_val(copy_path_str)]).unwrap();
        let copy_content = assert_ok(&copy_read_result);
        assert_eq!(
            extract_string(copy_content),
            "Initial content\nAppended line"
        );

        // Move file
        let moved_path = temp_dir.path().join("moved_test.txt");
        let moved_path_str = moved_path.to_str().unwrap();

        let move_result =
            move_file(vec![string_val(copy_path_str), string_val(moved_path_str)]).unwrap();
        assert_ok(&move_result);

        // Verify original copy is gone and moved file exists
        assert!(!extract_bool(
            &exists(vec![string_val(copy_path_str)]).unwrap()
        ));
        assert!(extract_bool(
            &exists(vec![string_val(moved_path_str)]).unwrap()
        ));

        // Remove files
        let remove_result = remove_file(vec![string_val(file_path_str)]).unwrap();
        assert_ok(&remove_result);
        assert!(!extract_bool(
            &exists(vec![string_val(file_path_str)]).unwrap()
        ));

        let remove_result = remove_file(vec![string_val(moved_path_str)]).unwrap();
        assert_ok(&remove_result);
        assert!(!extract_bool(
            &exists(vec![string_val(moved_path_str)]).unwrap()
        ));
    }

    #[test]
    fn test_integration_directory_lifecycle() {
        let temp_dir = tempdir().unwrap();
        let test_dir = temp_dir.path().join("dir_lifecycle_test");
        let test_dir_str = test_dir.to_str().unwrap();

        // Initially directory should not exist
        assert!(!extract_bool(
            &exists(vec![string_val(test_dir_str)]).unwrap()
        ));

        // Create directory
        let create_result = create_dir(vec![string_val(test_dir_str)]).unwrap();
        assert_ok(&create_result);

        // Now directory should exist and be a directory
        assert!(extract_bool(
            &exists(vec![string_val(test_dir_str)]).unwrap()
        ));
        assert!(extract_bool(
            &is_dir(vec![string_val(test_dir_str)]).unwrap()
        ));
        assert!(!extract_bool(
            &is_file(vec![string_val(test_dir_str)]).unwrap()
        ));

        // List empty directory
        let list_result = list_dir(vec![string_val(test_dir_str)]).unwrap();
        let entries = assert_ok(&list_result);
        if let Value::List(list) = entries {
            assert_eq!(list.len(), 0);
        }

        // Create files in directory
        let file1_path = test_dir.join("file1.txt");
        let file2_path = test_dir.join("file2.txt");

        fs::write(&file1_path, "content1").unwrap();
        fs::write(&file2_path, "content2").unwrap();

        // List directory with files
        let list_result = list_dir(vec![string_val(test_dir_str)]).unwrap();
        let entries = assert_ok(&list_result);
        if let Value::List(list) = entries {
            assert_eq!(list.len(), 2);
        }

        // Try to remove non-empty directory (should fail)
        let remove_result = remove_dir(vec![string_val(test_dir_str)]).unwrap();
        assert_err(&remove_result);

        // Remove directory with all contents
        let remove_all_result = remove_dir_all(vec![string_val(test_dir_str)]).unwrap();
        assert_ok(&remove_all_result);

        // Directory should no longer exist
        assert!(!extract_bool(
            &exists(vec![string_val(test_dir_str)]).unwrap()
        ));
    }

    #[test]
    fn test_error_messages_are_informative() {
        // Test that error messages contain useful information

        // File not found error
        let read_result = read_file(vec![string_val("/clearly/nonexistent/file.txt")]).unwrap();
        let error_msg = assert_err(&read_result);
        let error_str = extract_string(error_msg);
        assert!(error_str.contains("Failed to read file"));
        assert!(error_str.contains("/clearly/nonexistent/file.txt"));

        // Invalid argument type error
        let write_result = write_file(vec![string_val("file.txt"), int_val(42)]).unwrap();
        let error_msg = assert_err(&write_result);
        let error_str = extract_string(error_msg);
        assert!(error_str.contains("contents must be a string"));

        // Wrong number of arguments error
        let copy_result = copy_file(vec![string_val("only_one_arg")]).unwrap();
        let error_msg = assert_err(&copy_result);
        let error_str = extract_string(error_msg);
        assert!(error_str.contains("copy_file expects 2 arguments"));
    }
}
