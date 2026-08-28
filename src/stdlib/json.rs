use crate::ast::Value;
use serde_json;
use std::collections::HashMap;
use std::sync::Arc;

/// Error types for JSON operations
#[derive(Debug, thiserror::Error)]
pub enum JsonError {
    #[error("Parse error: {message}")]
    ParseError { message: String },
    #[error("Invalid JSON: {message}")]
    InvalidJson { message: String },
    #[error("Invalid argument: {message}")]
    InvalidArgument { message: String },
    #[error("Type error: {message}")]
    TypeError { message: String },
    #[error("Key not found: {key}")]
    KeyNotFound { key: String },
}

impl From<serde_json::Error> for JsonError {
    fn from(err: serde_json::Error) -> Self {
        JsonError::ParseError {
            message: err.to_string(),
        }
    }
}

/// Creates the json module with all JSON functions
pub fn create_json_module() -> Value {
    let mut module = HashMap::new();

    // Core parsing and serialization
    module.insert("parse".to_string(), create_builtin_function("parse", 1));
    module.insert(
        "stringify".to_string(),
        create_builtin_function("stringify", 1),
    );
    module.insert(
        "prettify".to_string(),
        create_builtin_function("prettify", 1),
    );
    module.insert("minify".to_string(), create_builtin_function("minify", 1));

    // Validation and inspection
    module.insert(
        "validate".to_string(),
        create_builtin_function("validate", 1),
    );
    module.insert(
        "get_type".to_string(),
        create_builtin_function("get_type", 1),
    );

    // Object operations
    module.insert("has_key".to_string(), create_builtin_function("has_key", 2));
    module.insert(
        "get_keys".to_string(),
        create_builtin_function("get_keys", 1),
    );
    module.insert(
        "get_values".to_string(),
        create_builtin_function("get_values", 1),
    );
    module.insert("get".to_string(), create_builtin_function("get", 2));
    module.insert("set".to_string(), create_builtin_function("set", 3));
    module.insert("remove".to_string(), create_builtin_function("remove", 2));

    // Array operations
    module.insert(
        "array_get".to_string(),
        create_builtin_function("array_get", 2),
    );
    module.insert(
        "array_length".to_string(),
        create_builtin_function("array_length", 1),
    );
    module.insert(
        "array_push".to_string(),
        create_builtin_function("array_push", 2),
    );

    // Utility functions
    module.insert("merge".to_string(), create_builtin_function("merge", 2));
    module.insert(
        "deep_clone".to_string(),
        create_builtin_function("deep_clone", 1),
    );

    Value::Struct {
        type_name: "Module".to_string(),
        fields: std::sync::Arc::new(module),
    }
}

/// Helper function to create builtin function values
fn create_builtin_function(name: &str, arity: usize) -> Value {
    Value::Builtin(crate::ast::BuiltinFunction {
        name: format!("json.{}", name),
        arity,
    })
}

/// Main dispatcher for JSON function calls
pub fn call_json_function(
    name: &str,
    args: Vec<Value>,
) -> Result<Value, Box<dyn std::error::Error>> {
    match name {
        "parse" => json_parse(args),
        "stringify" => json_stringify(args),
        "prettify" => json_prettify(args),
        "minify" => json_minify(args),
        "validate" => json_validate(args),
        "get_type" => json_get_type(args),
        "has_key" => json_has_key(args),
        "get_keys" => json_get_keys(args),
        "get_values" => json_get_values(args),
        "get" => json_get(args),
        "set" => json_set(args),
        "remove" => json_remove(args),
        "array_get" => json_array_get(args),
        "array_length" => json_array_length(args),
        "array_push" => json_array_push(args),
        "merge" => json_merge(args),
        "deep_clone" => json_deep_clone(args),
        _ => Err(format!("Unknown json function: {}", name).into()),
    }
}

/// Parse a JSON string into an Olang value
/// Usage: json.parse("{\"name\": \"John\", \"age\": 30}") -> Result<Object, Error>
fn json_parse(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("parse expects 1 argument, got {}", args.len()).into());
    }

    let json_str = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Err("parse: argument must be a string".to_string().into());
        }
    };

    match serde_json::from_str::<serde_json::Value>(json_str) {
        Ok(json_value) => match json_to_olang_value(json_value) {
            Ok(olang_value) => Ok(Value::Ok(Box::new(olang_value))),
            Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(e))))),
        },
        Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "JSON parse error: {}",
            e
        )))))),
    }
}

/// Convert an Olang value to JSON string
/// Usage: json.stringify({"name": "John", "age": 30}) -> Result<String, Error>
fn json_stringify(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("stringify expects 1 argument, got {}", args.len()).into());
    }

    match olang_value_to_json(&args[0]) {
        Ok(json_value) => match serde_json::to_string(&json_value) {
            Ok(json_str) => Ok(Value::Ok(Box::new(Value::String(Arc::new(json_str))))),
            Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
                "JSON stringify error: {}",
                e
            )))))),
        },
        Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "Cannot convert to JSON: {}",
            e
        )))))),
    }
}

/// Pretty print JSON with indentation
/// Usage: json.prettify("{\"name\":\"John\",\"age\":30}") -> Result<String, Error>
fn json_prettify(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("prettify expects 1 argument, got {}", args.len()).into());
    }

    let json_str = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Err("prettify: argument must be a string".to_string().into());
        }
    };

    match serde_json::from_str::<serde_json::Value>(json_str) {
        Ok(json_value) => match serde_json::to_string_pretty(&json_value) {
            Ok(pretty_str) => Ok(Value::Ok(Box::new(Value::String(Arc::new(pretty_str))))),
            Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
                "JSON prettify error: {}",
                e
            )))))),
        },
        Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "JSON parse error: {}",
            e
        )))))),
    }
}

/// Minify JSON by removing whitespace
/// Usage: json.minify("{\n  \"name\": \"John\",\n  \"age\": 30\n}") -> Result<String, Error>
fn json_minify(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("minify expects 1 argument, got {}", args.len()).into());
    }

    let json_str = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Err("minify: argument must be a string".to_string().into());
        }
    };

    match serde_json::from_str::<serde_json::Value>(json_str) {
        Ok(json_value) => match serde_json::to_string(&json_value) {
            Ok(minified_str) => Ok(Value::Ok(Box::new(Value::String(Arc::new(minified_str))))),
            Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
                "JSON minify error: {}",
                e
            )))))),
        },
        Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "JSON parse error: {}",
            e
        )))))),
    }
}

/// Validate if a string is valid JSON
/// Usage: json.validate("{\"name\": \"John\"}") -> Bool
fn json_validate(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("validate expects 1 argument, got {}", args.len()).into());
    }

    let json_str = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => return Err("validate: argument must be a string".into()),
    };

    let is_valid = serde_json::from_str::<serde_json::Value>(json_str).is_ok();
    Ok(Value::Boolean(is_valid))
}

/// Get the type of a JSON value
/// Usage: json.get_type("{\"name\": \"John\"}") -> Result<String, Error>
fn json_get_type(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("get_type expects 1 argument, got {}", args.len()).into());
    }

    let json_str = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Err("get_type: argument must be a string".to_string().into());
        }
    };

    match serde_json::from_str::<serde_json::Value>(json_str) {
        Ok(json_value) => {
            let type_name = match json_value {
                serde_json::Value::Null => "null",
                serde_json::Value::Bool(_) => "boolean",
                serde_json::Value::Number(_) => "number",
                serde_json::Value::String(_) => "string",
                serde_json::Value::Array(_) => "array",
                serde_json::Value::Object(_) => "object",
            };
            Ok(Value::Ok(Box::new(Value::String(Arc::new(
                type_name.to_string(),
            )))))
        }
        Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "JSON parse error: {}",
            e
        )))))),
    }
}

/// Check if JSON object has a specific key
/// Usage: json.has_key("{\"name\": \"John\"}", "name") -> Result<Bool, Error>
fn json_has_key(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Err(format!("has_key expects 2 arguments, got {}", args.len()).into());
    }

    let json_str = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Err("has_key: first argument must be a string"
                .to_string()
                .into());
        }
    };

    let key = match &args[1] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Err("has_key: second argument must be a string"
                .to_string()
                .into());
        }
    };

    match serde_json::from_str::<serde_json::Value>(json_str) {
        Ok(json_value) => {
            if let serde_json::Value::Object(obj) = json_value {
                Ok(Value::Ok(Box::new(Value::Boolean(obj.contains_key(key)))))
            } else {
                Ok(Value::Err(Box::new(Value::String(Arc::new(
                    "has_key: JSON value is not an object".to_string(),
                )))))
            }
        }
        Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "JSON parse error: {}",
            e
        )))))),
    }
}

/// Get all keys from a JSON object
/// Usage: json.get_keys("{\"name\": \"John\", \"age\": 30}") -> Result<List<String>, Error>
fn json_get_keys(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("get_keys expects 1 argument, got {}", args.len()).into());
    }

    let json_str = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Err("get_keys: argument must be a string".to_string().into());
        }
    };

    match serde_json::from_str::<serde_json::Value>(json_str) {
        Ok(json_value) => {
            if let serde_json::Value::Object(obj) = json_value {
                let keys: Vec<Value> = obj
                    .keys()
                    .map(|k| Value::String(Arc::new(k.to_string())))
                    .collect();
                Ok(Value::Ok(Box::new(Value::List(keys.into()))))
            } else {
                Ok(Value::Err(Box::new(Value::String(Arc::new(
                    "get_keys: JSON value is not an object".to_string(),
                )))))
            }
        }
        Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "JSON parse error: {}",
            e
        )))))),
    }
}

/// Get all values from a JSON object
/// Usage: json.get_values("{\"name\": \"John\", \"age\": 30}") -> Result<List<Value>, Error>
fn json_get_values(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("get_values expects 1 argument, got {}", args.len()).into());
    }

    let json_str = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Err("get_values: argument must be a string".to_string().into());
        }
    };

    match serde_json::from_str::<serde_json::Value>(json_str) {
        Ok(json_value) => {
            if let serde_json::Value::Object(obj) = json_value {
                let values = match obj
                    .values()
                    .map(|v| json_to_olang_value(v.clone()))
                    .collect::<Result<Vec<Value>, _>>()
                {
                    Ok(v) => v,
                    Err(e) => return Ok(Value::Err(Box::new(Value::String(Arc::new(e))))),
                };
                Ok(Value::Ok(Box::new(Value::List(values.into()))))
            } else {
                Ok(Value::Err(Box::new(Value::String(Arc::new(
                    "get_values: JSON value is not an object".to_string(),
                )))))
            }
        }
        Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "JSON parse error: {}",
            e
        )))))),
    }
}

/// Get a value from JSON object by key
/// Usage: json.get("{\"name\": \"John\"}", "name") -> Result<Value, Error>
fn json_get(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Err(format!("get expects 2 arguments, got {}", args.len()).into());
    }

    let json_str = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Err("get: first argument must be a string".to_string().into());
        }
    };

    let key = match &args[1] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Err("get: second argument must be a string".to_string().into());
        }
    };

    match serde_json::from_str::<serde_json::Value>(json_str) {
        Ok(json_value) => {
            if let serde_json::Value::Object(obj) = json_value {
                if let Some(value) = obj.get(key) {
                    let olang_value = match json_to_olang_value(value.clone()) {
                        Ok(v) => v,
                        Err(e) => return Ok(Value::Err(Box::new(Value::String(Arc::new(e))))),
                    };
                    Ok(Value::Ok(Box::new(olang_value)))
                } else {
                    Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
                        "Key '{}' not found in JSON object",
                        key
                    ))))))
                }
            } else {
                Ok(Value::Err(Box::new(Value::String(Arc::new(
                    "get: JSON value is not an object".to_string(),
                )))))
            }
        }
        Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "JSON parse error: {}",
            e
        )))))),
    }
}

/// Set a value in JSON object
/// Usage: json.set("{\"name\": \"John\"}", "age", 30) -> Result<String, Error>
fn json_set(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 3 {
        return Err(format!("set expects 3 arguments, got {}", args.len()).into());
    }

    let json_str = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Err("set: first argument must be a string".to_string().into());
        }
    };

    let key = match &args[1] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Err("set: second argument must be a string".to_string().into());
        }
    };

    match serde_json::from_str::<serde_json::Value>(json_str) {
        Ok(mut json_value) => {
            if let serde_json::Value::Object(ref mut obj) = json_value {
                match olang_value_to_json(&args[2]) {
                    Ok(value_json) => {
                        obj.insert(key.to_string(), value_json);
                        match serde_json::to_string(&json_value) {
                            Ok(result_str) => {
                                Ok(Value::Ok(Box::new(Value::String(Arc::new(result_str)))))
                            }
                            Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
                                "JSON stringify error: {}",
                                e
                            )))))),
                        }
                    }
                    Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
                        "Cannot convert value to JSON: {}",
                        e
                    )))))),
                }
            } else {
                Ok(Value::Err(Box::new(Value::String(Arc::new(
                    "set: JSON value is not an object".to_string(),
                )))))
            }
        }
        Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "JSON parse error: {}",
            e
        )))))),
    }
}

/// Remove a key from JSON object
/// Usage: json.remove("{\"name\": \"John\", \"age\": 30}", "age") -> Result<String, Error>
fn json_remove(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Err(format!("remove expects 2 arguments, got {}", args.len()).into());
    }

    let json_str = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Err("remove: first argument must be a string".to_string().into());
        }
    };

    let key = match &args[1] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Err("remove: second argument must be a string"
                .to_string()
                .into());
        }
    };

    match serde_json::from_str::<serde_json::Value>(json_str) {
        Ok(mut json_value) => {
            if let serde_json::Value::Object(ref mut obj) = json_value {
                obj.remove(key);
                match serde_json::to_string(&json_value) {
                    Ok(result_str) => Ok(Value::Ok(Box::new(Value::String(Arc::new(result_str))))),
                    Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
                        "JSON stringify error: {}",
                        e
                    )))))),
                }
            } else {
                Ok(Value::Err(Box::new(Value::String(Arc::new(
                    "remove: JSON value is not an object".to_string(),
                )))))
            }
        }
        Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "JSON parse error: {}",
            e
        )))))),
    }
}

/// Get element from JSON array by index
/// Usage: json.array_get("[1, 2, 3]", 1) -> Result<Value, Error>
fn json_array_get(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Err(format!("array_get expects 2 arguments, got {}", args.len()).into());
    }

    let json_str = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Err("array_get: first argument must be a string"
                .to_string()
                .into());
        }
    };

    let index = match &args[1] {
        Value::Integer(i) => *i as usize,
        _ => {
            return Err("array_get: second argument must be an integer"
                .to_string()
                .into());
        }
    };

    match serde_json::from_str::<serde_json::Value>(json_str) {
        Ok(json_value) => {
            if let serde_json::Value::Array(arr) = json_value {
                if index < arr.len() {
                    let olang_value = match json_to_olang_value(arr[index].clone()) {
                        Ok(v) => v,
                        Err(e) => return Ok(Value::Err(Box::new(Value::String(Arc::new(e))))),
                    };
                    Ok(Value::Ok(Box::new(olang_value)))
                } else {
                    Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
                        "Array index {} out of bounds (length: {})",
                        index,
                        arr.len()
                    ))))))
                }
            } else {
                Ok(Value::Err(Box::new(Value::String(Arc::new(
                    "array_get: JSON value is not an array".to_string(),
                )))))
            }
        }
        Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "JSON parse error: {}",
            e
        )))))),
    }
}

/// Get length of JSON array
/// Usage: json.array_length("[1, 2, 3]") -> Result<Int, Error>
fn json_array_length(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("array_length expects 1 argument, got {}", args.len()).into());
    }

    let json_str = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Err("array_length: argument must be a string".to_string().into());
        }
    };

    match serde_json::from_str::<serde_json::Value>(json_str) {
        Ok(json_value) => {
            if let serde_json::Value::Array(arr) = json_value {
                Ok(Value::Ok(Box::new(Value::Integer(arr.len() as i64))))
            } else {
                Ok(Value::Err(Box::new(Value::String(Arc::new(
                    "array_length: JSON value is not an array".to_string(),
                )))))
            }
        }
        Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "JSON parse error: {}",
            e
        )))))),
    }
}

/// Push element to JSON array
/// Usage: json.array_push("[1, 2]", 3) -> Result<String, Error>
fn json_array_push(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Err(format!("array_push expects 2 arguments, got {}", args.len()).into());
    }

    let json_str = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Err("array_push: first argument must be a string"
                .to_string()
                .into());
        }
    };

    match serde_json::from_str::<serde_json::Value>(json_str) {
        Ok(mut json_value) => {
            if let serde_json::Value::Array(ref mut arr) = json_value {
                match olang_value_to_json(&args[1]) {
                    Ok(value_json) => {
                        arr.push(value_json);
                        match serde_json::to_string(&json_value) {
                            Ok(result_str) => {
                                Ok(Value::Ok(Box::new(Value::String(Arc::new(result_str)))))
                            }
                            Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
                                "JSON stringify error: {}",
                                e
                            )))))),
                        }
                    }
                    Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
                        "Cannot convert value to JSON: {}",
                        e
                    )))))),
                }
            } else {
                Ok(Value::Err(Box::new(Value::String(Arc::new(
                    "array_push: JSON value is not an array".to_string(),
                )))))
            }
        }
        Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "JSON parse error: {}",
            e
        )))))),
    }
}

/// Merge two JSON objects
/// Usage: json.merge("{\"a\": 1}", "{\"b\": 2}") -> Result<String, Error>
fn json_merge(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Err(format!("merge expects 2 arguments, got {}", args.len()).into());
    }

    let json1_str = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Err("merge: first argument must be a string".to_string().into());
        }
    };

    let json2_str = match &args[1] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Err("merge: second argument must be a string".to_string().into());
        }
    };

    match (
        serde_json::from_str::<serde_json::Value>(json1_str),
        serde_json::from_str::<serde_json::Value>(json2_str),
    ) {
        (Ok(mut json1), Ok(json2)) => {
            if let (serde_json::Value::Object(obj1), serde_json::Value::Object(obj2)) =
                (&mut json1, json2)
            {
                for (key, value) in obj2 {
                    obj1.insert(key, value);
                }
                match serde_json::to_string(&json1) {
                    Ok(result_str) => Ok(Value::Ok(Box::new(Value::String(Arc::new(result_str))))),
                    Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
                        "JSON stringify error: {}",
                        e
                    )))))),
                }
            } else {
                Ok(Value::Err(Box::new(Value::String(Arc::new(
                    "merge: both arguments must be JSON objects".to_string(),
                )))))
            }
        }
        (Err(e), _) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "JSON parse error in first argument: {}",
            e
        )))))),
        (_, Err(e)) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "JSON parse error in second argument: {}",
            e
        )))))),
    }
}

/// Deep clone a JSON value
/// Usage: json.deep_clone("{\"name\": \"John\"}") -> Result<String, Error>
fn json_deep_clone(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("deep_clone expects 1 argument, got {}", args.len()).into());
    }

    let json_str = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Err("deep_clone: argument must be a string".to_string().into());
        }
    };

    match serde_json::from_str::<serde_json::Value>(json_str) {
        Ok(json_value) => match serde_json::to_string(&json_value) {
            Ok(cloned_str) => Ok(Value::Ok(Box::new(Value::String(Arc::new(cloned_str))))),
            Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
                "JSON stringify error: {}",
                e
            )))))),
        },
        Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "JSON parse error: {}",
            e
        )))))),
    }
}

/// Convert serde_json::Value to Olang Value.
///
/// Numeric fidelity is the contract (docs/stability.md): integers within
/// i64 arrive losslessly as Int, JSON decimals arrive as the nearest
/// Float (the standard IEEE reading), and an integer OUTSIDE i64 is an
/// error rather than a silently-lossy Float — `99999999999999999999` used
/// to come back as `1e20` with no sign anything was lost.
pub(crate) fn json_to_olang_value(json_value: serde_json::Value) -> Result<Value, String> {
    Ok(match json_value {
        serde_json::Value::Null => Value::Unit,
        serde_json::Value::Bool(b) => Value::Boolean(b),
        serde_json::Value::Number(n) => {
            // arbitrary_precision keeps the source text, so the JSON
            // grammar itself decides the reading: a `.`/`e`/`E` means the
            // author wrote a float (nearest finite f64, overflow is an
            // error, never inf); bare digits mean an integer (i64 or an
            // error, never a silently-lossy float).
            let text = n.to_string();
            if text.contains(['.', 'e', 'E']) {
                match n.as_f64() {
                    Some(f) if f.is_finite() => Value::Float(f),
                    _ => {
                        return Err(format!(
                            "JSON number {} does not fit Float (f64): its \
                             magnitude overflows",
                            text
                        ));
                    }
                }
            } else if let Some(i) = n.as_i64() {
                Value::Integer(i)
            } else {
                return Err(format!(
                    "JSON integer {} does not fit Int (64-bit signed); refusing \
                     the lossy Float reading. Transport oversized integers as \
                     JSON strings (bigint can hold them)",
                    text
                ));
            }
        }
        serde_json::Value::String(s) => Value::String(Arc::new(s)),
        serde_json::Value::Array(arr) => {
            let values: Vec<Value> = arr
                .into_iter()
                .map(json_to_olang_value)
                .collect::<Result<Vec<_>, _>>()?;
            Value::List(values.into())
        }
        serde_json::Value::Object(obj) => {
            let mut fields = HashMap::new();
            for (key, value) in obj {
                fields.insert(key, json_to_olang_value(value)?);
            }
            Value::Struct {
                type_name: "JsonObject".to_string(),
                fields: std::sync::Arc::new(fields),
            }
        }
    })
}

/// Convert Olang Value to serde_json::Value
pub(crate) fn olang_value_to_json(value: &Value) -> Result<serde_json::Value, JsonError> {
    match value {
        Value::Unit => Ok(serde_json::Value::Null),
        Value::Boolean(b) => Ok(serde_json::Value::Bool(*b)),
        Value::Integer(i) => Ok(serde_json::Value::Number(serde_json::Number::from(*i))),
        Value::Float(f) => {
            if let Some(num) = serde_json::Number::from_f64(*f) {
                Ok(serde_json::Value::Number(num))
            } else {
                Err(JsonError::TypeError {
                    message: format!("Invalid float value: {}", f),
                })
            }
        }
        Value::String(s) => Ok(serde_json::Value::String(s.as_ref().clone())),
        Value::List(list) => {
            let mut arr = Vec::new();
            for item in list.iter() {
                arr.push(olang_value_to_json(item)?);
            }
            Ok(serde_json::Value::Array(arr))
        }
        Value::Struct { fields, .. } => {
            let mut obj = serde_json::Map::new();
            for (key, value) in fields.iter() {
                obj.insert(key.clone(), olang_value_to_json(value)?);
            }
            Ok(serde_json::Value::Object(obj))
        }
        // Maps are JSON objects too — `#{ "k": v }` and db rows serialize
        // exactly like structs and anonymous objects.
        Value::Map(map) => {
            let mut obj = serde_json::Map::new();
            for (key, value) in map.iter() {
                obj.insert(key.clone(), olang_value_to_json(value)?);
            }
            Ok(serde_json::Value::Object(obj))
        }
        Value::Tuple(tuple) => {
            let mut arr = Vec::new();
            for item in tuple.iter() {
                arr.push(olang_value_to_json(item)?);
            }
            Ok(serde_json::Value::Array(arr))
        }
        _ => Err(JsonError::TypeError {
            message: format!("Cannot convert {:?} to JSON", value),
        }),
    }
}
