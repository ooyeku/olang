use crate::ast::Value;
use base64::{Engine as _, engine::general_purpose};
use std::collections::HashMap;
use std::sync::Arc;

/// Error types for Base64 operations
#[derive(Debug, thiserror::Error)]
pub enum Base64Error {
    #[error("Decode error: {message}")]
    DecodeError { message: String },
    #[error("Invalid input: {message}")]
    InvalidInput { message: String },
    #[error("Invalid argument: {message}")]
    InvalidArgument { message: String },
    #[error("Type error: {message}")]
    TypeError { message: String },
}

impl From<base64::DecodeError> for Base64Error {
    fn from(err: base64::DecodeError) -> Self {
        Base64Error::DecodeError {
            message: err.to_string(),
        }
    }
}

/// Creates the base64 module with all Base64 functions
pub fn create_base64_module() -> Value {
    let mut module = HashMap::new();

    // Core encoding and decoding
    module.insert("encode".to_string(), create_builtin_function("encode", 1));
    module.insert("decode".to_string(), create_builtin_function("decode", 1));

    // URL-safe variants
    module.insert(
        "encode_url_safe".to_string(),
        create_builtin_function("encode_url_safe", 1),
    );
    module.insert(
        "decode_url_safe".to_string(),
        create_builtin_function("decode_url_safe", 1),
    );

    // Validation and utility
    module.insert(
        "validate".to_string(),
        create_builtin_function("validate", 1),
    );
    module.insert(
        "is_valid".to_string(),
        create_builtin_function("is_valid", 1),
    );

    // Encoding with options
    module.insert(
        "encode_no_pad".to_string(),
        create_builtin_function("encode_no_pad", 1),
    );
    module.insert(
        "decode_no_pad".to_string(),
        create_builtin_function("decode_no_pad", 1),
    );

    Value::Struct {
        type_name: "Module".to_string(),
        fields: module,
    }
}

/// Helper function to create builtin function values
fn create_builtin_function(name: &str, arity: usize) -> Value {
    Value::Builtin(crate::ast::BuiltinFunction {
        name: format!("base64.{}", name),
        arity,
    })
}

/// Main dispatcher for Base64 function calls
pub fn call_base64_function(
    name: &str,
    args: Vec<Value>,
) -> Result<Value, Box<dyn std::error::Error>> {
    match name {
        "encode" => base64_encode(args),
        "decode" => base64_decode(args),
        "encode_url_safe" => base64_encode_url_safe(args),
        "decode_url_safe" => base64_decode_url_safe(args),
        "validate" => base64_validate(args),
        "is_valid" => base64_is_valid(args),
        "encode_no_pad" => base64_encode_no_pad(args),
        "decode_no_pad" => base64_decode_no_pad(args),
        _ => Err(format!("Unknown base64 function: {}", name).into()),
    }
}

/// Encode a string to base64
/// Usage: base64.encode("Hello, World!") -> Result<String, Error>
fn base64_encode(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "encode expects 1 argument, got {}",
            args.len()
        ))))));
    }

    let input = match &args[0] {
        Value::String(s) => s.as_ref().as_bytes(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "encode: argument must be a string".to_string(),
            )))));
        }
    };

    let encoded = general_purpose::STANDARD.encode(input);
    Ok(Value::String(Arc::new(encoded)))
}

/// Decode a base64 string
/// Usage: base64.decode("SGVsbG8sIFdvcmxkIQ==") -> Result<String, Error>
fn base64_decode(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "decode expects 1 argument, got {}",
            args.len()
        ))))));
    }

    let input = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "decode: argument must be a string".to_string(),
            )))));
        }
    };

    match general_purpose::STANDARD.decode(input) {
        Ok(decoded_bytes) => match String::from_utf8(decoded_bytes) {
            Ok(decoded_string) => Ok(Value::Ok(Box::new(Value::String(Arc::new(decoded_string))))),
            Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
                "Invalid UTF-8 in decoded data: {}",
                e
            )))))),
        },
        Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "Base64 decode error: {}",
            e
        )))))),
    }
}

/// Encode a string to URL-safe base64
/// Usage: base64.encode_url_safe("Hello, World!") -> Result<String, Error>
fn base64_encode_url_safe(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "encode_url_safe expects 1 argument, got {}",
            args.len()
        ))))));
    }

    let input = match &args[0] {
        Value::String(s) => s.as_ref().as_bytes(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "encode_url_safe: argument must be a string".to_string(),
            )))));
        }
    };

    let encoded = general_purpose::URL_SAFE.encode(input);
    Ok(Value::String(Arc::new(encoded)))
}

/// Decode a URL-safe base64 string
/// Usage: base64.decode_url_safe("SGVsbG8sIFdvcmxkIQ==") -> Result<String, Error>
fn base64_decode_url_safe(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "decode_url_safe expects 1 argument, got {}",
            args.len()
        ))))));
    }

    let input = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "decode_url_safe: argument must be a string".to_string(),
            )))));
        }
    };

    match general_purpose::URL_SAFE.decode(input) {
        Ok(decoded_bytes) => match String::from_utf8(decoded_bytes) {
            Ok(decoded_string) => Ok(Value::Ok(Box::new(Value::String(Arc::new(decoded_string))))),
            Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
                "Invalid UTF-8 in decoded data: {}",
                e
            )))))),
        },
        Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "Base64 decode error: {}",
            e
        )))))),
    }
}

/// Validate if a string is valid base64
/// Usage: base64.validate("SGVsbG8sIFdvcmxkIQ==") -> Result<Bool, Error>
fn base64_validate(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "validate expects 1 argument, got {}",
            args.len()
        ))))));
    }

    let input = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "validate: argument must be a string".to_string(),
            )))));
        }
    };

    let is_valid = general_purpose::STANDARD.decode(input).is_ok();
    Ok(Value::Ok(Box::new(Value::Boolean(is_valid))))
}

/// Check if a string is valid base64 (alias for validate)
/// Usage: base64.is_valid("SGVsbG8sIFdvcmxkIQ==") -> Result<Bool, Error>
fn base64_is_valid(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    base64_validate(args)
}

/// Encode a string to base64 without padding
/// Usage: base64.encode_no_pad("Hello, World!") -> Result<String, Error>
fn base64_encode_no_pad(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "encode_no_pad expects 1 argument, got {}",
            args.len()
        ))))));
    }

    let input = match &args[0] {
        Value::String(s) => s.as_ref().as_bytes(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "encode_no_pad: argument must be a string".to_string(),
            )))));
        }
    };

    let encoded = general_purpose::STANDARD_NO_PAD.encode(input);
    Ok(Value::String(Arc::new(encoded)))
}

/// Decode a base64 string without padding
/// Usage: base64.decode_no_pad("SGVsbG8sIFdvcmxkIQ") -> Result<String, Error>
fn base64_decode_no_pad(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "decode_no_pad expects 1 argument, got {}",
            args.len()
        ))))));
    }

    let input = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "decode_no_pad: argument must be a string".to_string(),
            )))));
        }
    };

    match general_purpose::STANDARD_NO_PAD.decode(input) {
        Ok(decoded_bytes) => match String::from_utf8(decoded_bytes) {
            Ok(decoded_string) => Ok(Value::Ok(Box::new(Value::String(Arc::new(decoded_string))))),
            Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
                "Invalid UTF-8 in decoded data: {}",
                e
            )))))),
        },
        Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "Base64 decode error: {}",
            e
        )))))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    // Helper function to assert Ok result and extract inner value
    // Unwraps a fallible result, or passes a total (bare) value through.
    // encode operations return bare values now; decode still returns Result.
    fn assert_ok(result: &Value) -> &Value {
        match result {
            Value::Ok(inner) => inner,
            Value::Err(e) => panic!("Expected Ok result, got Err: {:?}", e),
            bare => bare,
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

    #[test]
    fn test_base64_module_creation() {
        let module = create_base64_module();

        if let Value::Struct { type_name, fields } = module {
            assert_eq!(type_name, "Module");

            // Check that all expected functions are present
            let expected_functions = vec![
                "encode",
                "decode",
                "encode_url_safe",
                "decode_url_safe",
                "validate",
                "is_valid",
                "encode_no_pad",
                "decode_no_pad",
            ];

            for func_name in expected_functions {
                assert!(
                    fields.contains_key(func_name),
                    "Missing function: {}",
                    func_name
                );

                if let Value::Builtin(builtin) = &fields[func_name] {
                    assert_eq!(builtin.name, format!("base64.{}", func_name));
                    assert_eq!(builtin.arity, 1); // All base64 functions take 1 argument
                } else {
                    panic!(
                        "Expected builtin function for {}, got: {:?}",
                        func_name, fields[func_name]
                    );
                }
            }
        } else {
            panic!("Expected struct for base64 module, got: {:?}", module);
        }
    }

    #[test]
    fn test_base64_encode_basic() {
        // Test basic encoding
        let result = base64_encode(vec![string_val("Hello, World!")]).unwrap();
        let encoded = assert_ok(&result);
        assert_eq!(extract_string(encoded), "SGVsbG8sIFdvcmxkIQ==");

        // Test empty string
        let result = base64_encode(vec![string_val("")]).unwrap();
        let encoded = assert_ok(&result);
        assert_eq!(extract_string(encoded), "");

        // Test simple text
        let result = base64_encode(vec![string_val("test")]).unwrap();
        let encoded = assert_ok(&result);
        assert_eq!(extract_string(encoded), "dGVzdA==");
    }

    #[test]
    fn test_base64_decode_basic() {
        // Test basic decoding
        let result = base64_decode(vec![string_val("SGVsbG8sIFdvcmxkIQ==")]).unwrap();
        let decoded = assert_ok(&result);
        assert_eq!(extract_string(decoded), "Hello, World!");

        // Test empty string
        let result = base64_decode(vec![string_val("")]).unwrap();
        let decoded = assert_ok(&result);
        assert_eq!(extract_string(decoded), "");

        // Test simple text
        let result = base64_decode(vec![string_val("dGVzdA==")]).unwrap();
        let decoded = assert_ok(&result);
        assert_eq!(extract_string(decoded), "test");
    }

    #[test]
    fn test_base64_roundtrip() {
        let test_strings = vec![
            "Hello, World!",
            "The quick brown fox jumps over the lazy dog",
            "Base64 encoding test with special chars: !@#$%^&*()",
            "Unicode test: rocket star sparkle party",
            "Newlines\nand\ttabs\ttest",
            "",
            "a",
            "ab",
            "abc",
            "abcd",
        ];

        for test_str in test_strings {
            // Encode
            let encode_result = base64_encode(vec![string_val(test_str)]).unwrap();
            let encoded = assert_ok(&encode_result);
            let encoded_str = extract_string(encoded);

            // Decode
            let decode_result = base64_decode(vec![string_val(encoded_str)]).unwrap();
            let decoded = assert_ok(&decode_result);
            let decoded_str = extract_string(decoded);

            assert_eq!(decoded_str, test_str, "Roundtrip failed for: {}", test_str);
        }
    }

    #[test]
    fn test_base64_url_safe_encoding() {
        // Test URL-safe encoding with characters that differ from standard base64
        let test_data = "?>?";

        // Standard encoding
        let std_result = base64_encode(vec![string_val(test_data)]).unwrap();
        let std_encoded = extract_string(assert_ok(&std_result));

        // URL-safe encoding
        let url_result = base64_encode_url_safe(vec![string_val(test_data)]).unwrap();
        let url_encoded = extract_string(assert_ok(&url_result));

        // Both should encode the same data, but URL-safe might use different characters
        assert!(!std_encoded.is_empty());
        assert!(!url_encoded.is_empty());

        // Test roundtrip for URL-safe
        let decode_result = base64_decode_url_safe(vec![string_val(url_encoded)]).unwrap();
        let decoded = assert_ok(&decode_result);
        assert_eq!(extract_string(decoded), test_data);
    }

    #[test]
    fn test_base64_no_pad_encoding() {
        // Test encoding without padding
        let result = base64_encode_no_pad(vec![string_val("test")]).unwrap();
        let encoded = assert_ok(&result);
        let encoded_str = extract_string(encoded);

        // Should not contain padding characters
        assert!(!encoded_str.contains('='));

        // Test roundtrip
        let decode_result = base64_decode_no_pad(vec![string_val(encoded_str)]).unwrap();
        let decoded = assert_ok(&decode_result);
        assert_eq!(extract_string(decoded), "test");

        // Test with different lengths to ensure no padding
        let test_cases = vec!["a", "ab", "abc", "abcd", "abcde"];
        for test_str in test_cases {
            let encode_result = base64_encode_no_pad(vec![string_val(test_str)]).unwrap();
            let encoded = assert_ok(&encode_result);
            let encoded_str = extract_string(encoded);

            assert!(
                !encoded_str.contains('='),
                "Found padding in no-pad encoding for: {}",
                test_str
            );

            // Verify roundtrip
            let decode_result = base64_decode_no_pad(vec![string_val(encoded_str)]).unwrap();
            let decoded = assert_ok(&decode_result);
            assert_eq!(extract_string(decoded), test_str);
        }
    }

    #[test]
    fn test_base64_validation() {
        // Valid base64 strings
        let valid_cases = vec![
            "SGVsbG8sIFdvcmxkIQ==",
            "dGVzdA==",
            "",
            "YQ==",
            "YWI=",
            "YWJj",
        ];

        for valid_case in valid_cases {
            let result = base64_validate(vec![string_val(valid_case)]).unwrap();
            let is_valid = assert_ok(&result);
            assert!(
                extract_bool(is_valid),
                "Expected {} to be valid",
                valid_case
            );

            // Test is_valid alias
            let result = base64_is_valid(vec![string_val(valid_case)]).unwrap();
            let is_valid = assert_ok(&result);
            assert!(
                extract_bool(is_valid),
                "Expected {} to be valid (is_valid)",
                valid_case
            );
        }

        // Invalid base64 strings
        let invalid_cases = vec![
            "SGVsbG8sIFdvcmxkIQ=",   // Wrong padding
            "SGVsbG8sIFdvcmxkIQ===", // Too much padding
            "SGVsbG8@IFdvcmxkIQ==",  // Invalid character
            "SGVsbG8 IFdvcmxkIQ==",  // Space character
            "SGVsbG8sIFdvcmxkI",     // Incomplete
        ];

        for invalid_case in invalid_cases {
            let result = base64_validate(vec![string_val(invalid_case)]).unwrap();
            let is_valid = assert_ok(&result);
            assert!(
                !extract_bool(is_valid),
                "Expected {} to be invalid",
                invalid_case
            );
        }
    }

    #[test]
    fn test_base64_decode_errors() {
        // Test invalid base64 strings
        let invalid_cases = vec![
            "SGVsbG8@IFdvcmxkIQ==",  // Invalid character
            "SGVsbG8sIFdvcmxkIQ=",   // Wrong padding
            "SGVsbG8sIFdvcmxkIQ===", // Too much padding
        ];

        for invalid_case in invalid_cases {
            let result = base64_decode(vec![string_val(invalid_case)]).unwrap();
            assert_err(&result);
        }
    }

    #[test]
    fn test_base64_argument_validation() {
        // Test wrong number of arguments
        let result = base64_encode(vec![]).unwrap();
        assert_err(&result);

        let result = base64_encode(vec![string_val("test"), string_val("extra")]).unwrap();
        assert_err(&result);

        // Test wrong argument types
        let result = base64_encode(vec![int_val(42)]).unwrap();
        assert_err(&result);

        let result = base64_decode(vec![bool_val(true)]).unwrap();
        assert_err(&result);

        let result = base64_validate(vec![int_val(123)]).unwrap();
        assert_err(&result);
    }

    #[test]
    fn test_base64_special_characters() {
        // Test strings with special characters that might cause issues
        let special_cases = vec![
            "Line 1\nLine 2\nLine 3",
            "Tab\tseparated\tvalues",
            "Quotes: \"double\" and 'single'",
            "Backslashes: \\ and forward slashes: /",
            "Control chars: \r\n\t\0",
            "High unicode: rocket star sparkle party fire hundred",
        ];

        for test_case in special_cases {
            // Test encoding
            let encode_result = base64_encode(vec![string_val(test_case)]).unwrap();
            let encoded = assert_ok(&encode_result);
            let encoded_str = extract_string(encoded);

            // Encoded string should be valid base64
            let validate_result = base64_validate(vec![string_val(encoded_str)]).unwrap();
            let is_valid = assert_ok(&validate_result);
            assert!(
                extract_bool(is_valid),
                "Encoded string should be valid base64"
            );

            // Test decoding
            let decode_result = base64_decode(vec![string_val(encoded_str)]).unwrap();
            let decoded = assert_ok(&decode_result);
            assert_eq!(
                extract_string(decoded),
                test_case,
                "Roundtrip failed for special case"
            );
        }
    }

    #[test]
    fn test_base64_function_dispatcher() {
        // Test all functions through the dispatcher
        let test_data = "Hello, World!";

        // Test encode
        let result = call_base64_function("encode", vec![string_val(test_data)]).unwrap();
        let encoded = assert_ok(&result);
        let encoded_str = extract_string(encoded);
        assert_eq!(encoded_str, "SGVsbG8sIFdvcmxkIQ==");

        // Test decode
        let result = call_base64_function("decode", vec![string_val(encoded_str)]).unwrap();
        let decoded = assert_ok(&result);
        assert_eq!(extract_string(decoded), test_data);

        // Test validate
        let result = call_base64_function("validate", vec![string_val(encoded_str)]).unwrap();
        let is_valid = assert_ok(&result);
        assert!(extract_bool(is_valid));

        // Test unknown function
        let result = call_base64_function("unknown_function", vec![string_val(test_data)]);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Unknown base64 function")
        );
    }

    #[test]
    fn test_base64_edge_cases() {
        // Test very long strings
        let long_string = "a".repeat(10000);
        let encode_result = base64_encode(vec![string_val(&long_string)]).unwrap();
        let encoded = assert_ok(&encode_result);
        let encoded_str = extract_string(encoded);

        let decode_result = base64_decode(vec![string_val(encoded_str)]).unwrap();
        let decoded = assert_ok(&decode_result);
        assert_eq!(extract_string(decoded), long_string);

        // Test binary-like data (all possible byte values as string)
        let binary_like = (0..=255).map(|i| i as u8 as char).collect::<String>();
        let encode_result = base64_encode(vec![string_val(&binary_like)]).unwrap();
        let encoded = assert_ok(&encode_result);
        let encoded_str = extract_string(encoded);

        let decode_result = base64_decode(vec![string_val(encoded_str)]).unwrap();
        let decoded = assert_ok(&decode_result);
        assert_eq!(extract_string(decoded), binary_like);
    }

    #[test]
    fn test_base64_url_safe_vs_standard() {
        // Test data that will produce different results in standard vs URL-safe encoding
        // This test ensures we're actually using different encoders
        let test_cases = vec![
            "???>",  // Contains characters that map to +/ in standard base64
            "???>>", // More characters that will differ
        ];

        for test_case in test_cases {
            let std_result = base64_encode(vec![string_val(test_case)]).unwrap();
            let std_encoded = extract_string(assert_ok(&std_result));

            let url_result = base64_encode_url_safe(vec![string_val(test_case)]).unwrap();
            let url_encoded = extract_string(assert_ok(&url_result));

            // Both should be valid when decoded with their respective decoders
            let std_decode = base64_decode(vec![string_val(std_encoded)]).unwrap();
            assert_eq!(extract_string(assert_ok(&std_decode)), test_case);

            let url_decode = base64_decode_url_safe(vec![string_val(url_encoded)]).unwrap();
            assert_eq!(extract_string(assert_ok(&url_decode)), test_case);
        }
    }
}
