use crate::ast::Value;
use std::collections::HashMap;
use std::sync::Arc;

/// Error types for testing operations
#[derive(Debug, thiserror::Error)]
pub enum TestError {
    #[error("Assertion failed: {message}")]
    AssertionFailed { message: String },
    #[error("Test failed: {name} - {reason}")]
    TestFailed { name: String, reason: String },
    #[error("Invalid test function: {message}")]
    InvalidTestFunction { message: String },
    #[error("Test argument error: {message}")]
    ArgumentError { message: String },
}

/// Creates the testing module with all testing functions
pub fn create_testing_module() -> Value {
    let mut module = HashMap::new();

    // Basic assertion functions
    module.insert(
        "assert_eq".to_string(),
        create_builtin_function("assert_eq", 2),
    );
    module.insert(
        "assert_ne".to_string(),
        create_builtin_function("assert_ne", 2),
    );
    module.insert(
        "assert_true".to_string(),
        create_builtin_function("assert_true", 1),
    );
    module.insert(
        "assert_false".to_string(),
        create_builtin_function("assert_false", 1),
    );

    // Result assertion functions
    module.insert(
        "assert_ok".to_string(),
        create_builtin_function("assert_ok", 1),
    );
    module.insert(
        "assert_err".to_string(),
        create_builtin_function("assert_err", 1),
    );

    // Test control functions
    module.insert("fail".to_string(), create_builtin_function("fail", 1));
    module.insert(
        "run_test".to_string(),
        create_builtin_function("run_test", 2),
    );

    // Utility functions
    module.insert(
        "test_summary".to_string(),
        create_builtin_function("test_summary", 0),
    );
    module.insert(
        "reset_tests".to_string(),
        create_builtin_function("reset_tests", 0),
    );

    Value::Struct {
        type_name: "Module".to_string(),
        fields: module,
    }
}

/// Helper function to create builtin function values
fn create_builtin_function(name: &str, arity: usize) -> Value {
    Value::Builtin(crate::ast::BuiltinFunction {
        name: format!("testing.{}", name),
        arity,
    })
}

/// Main dispatcher for testing function calls
pub fn call_testing_function(
    name: &str,
    args: Vec<Value>,
) -> Result<Value, Box<dyn std::error::Error>> {
    match name {
        "assert_eq" => assert_eq(args),
        "assert_ne" => assert_ne(args),
        "assert_true" => assert_true(args),
        "assert_false" => assert_false(args),
        "assert_ok" => assert_ok(args),
        "assert_err" => assert_err(args),
        "fail" => fail_test(args),
        "run_test" => run_test(args),
        "test_summary" => test_summary(args),
        "reset_tests" => reset_tests(args),
        _ => Err(format!("Unknown testing function: {}", name).into()),
    }
}

/// Assert that two values are equal
/// Usage: testing.assert_eq(expected, actual) -> Result<Unit, Error>
fn assert_eq(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "assert_eq expects 2 arguments, got {}",
            args.len()
        ))))));
    }

    let expected = &args[0];
    let actual = &args[1];

    if values_equal(expected, actual) {
        Ok(Value::Ok(Box::new(Value::Unit)))
    } else {
        let error_msg = format!("Assertion failed: expected {} but got {}", expected, actual);
        Ok(Value::Err(Box::new(Value::String(Arc::new(error_msg)))))
    }
}

/// Assert that two values are not equal
/// Usage: testing.assert_ne(expected, actual) -> Result<Unit, Error>
fn assert_ne(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "assert_ne expects 2 arguments, got {}",
            args.len()
        ))))));
    }

    let expected = &args[0];
    let actual = &args[1];

    if !values_equal(expected, actual) {
        Ok(Value::Ok(Box::new(Value::Unit)))
    } else {
        let error_msg = format!(
            "Assertion failed: expected {} to not equal {}",
            expected, actual
        );
        Ok(Value::Err(Box::new(Value::String(Arc::new(error_msg)))))
    }
}

/// Assert that a condition is true
/// Usage: testing.assert_true(condition) -> Result<Unit, Error>
fn assert_true(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "assert_true expects 1 argument, got {}",
            args.len()
        ))))));
    }

    match &args[0] {
        Value::Boolean(true) => Ok(Value::Ok(Box::new(Value::Unit))),
        Value::Boolean(false) => {
            let error_msg = "Assertion failed: expected true but got false";
            Ok(Value::Err(Box::new(Value::String(Arc::new(
                error_msg.to_string(),
            )))))
        }
        other => {
            let error_msg = format!(
                "Assertion failed: assert_true requires a boolean, got {}",
                other.type_name()
            );
            Ok(Value::Err(Box::new(Value::String(Arc::new(error_msg)))))
        }
    }
}

/// Assert that a condition is false
/// Usage: testing.assert_false(condition) -> Result<Unit, Error>
fn assert_false(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "assert_false expects 1 argument, got {}",
            args.len()
        ))))));
    }

    match &args[0] {
        Value::Boolean(false) => Ok(Value::Ok(Box::new(Value::Unit))),
        Value::Boolean(true) => {
            let error_msg = "Assertion failed: expected false but got true";
            Ok(Value::Err(Box::new(Value::String(Arc::new(
                error_msg.to_string(),
            )))))
        }
        other => {
            let error_msg = format!(
                "Assertion failed: assert_false requires a boolean, got {}",
                other.type_name()
            );
            Ok(Value::Err(Box::new(Value::String(Arc::new(error_msg)))))
        }
    }
}

/// Assert that a Result value is Ok
/// Usage: testing.assert_ok(result) -> Result<Unit, Error>
fn assert_ok(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "assert_ok expects 1 argument, got {}",
            args.len()
        ))))));
    }

    match &args[0] {
        Value::Ok(_) => Ok(Value::Ok(Box::new(Value::Unit))),
        Value::Err(err) => {
            let error_msg = format!("Assertion failed: expected Ok but got Err({})", err);
            Ok(Value::Err(Box::new(Value::String(Arc::new(error_msg)))))
        }
        other => {
            let error_msg = format!(
                "Assertion failed: assert_ok requires a Result, got {}",
                other.type_name()
            );
            Ok(Value::Err(Box::new(Value::String(Arc::new(error_msg)))))
        }
    }
}

/// Assert that a Result value is Err
/// Usage: testing.assert_err(result) -> Result<Unit, Error>
fn assert_err(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "assert_err expects 1 argument, got {}",
            args.len()
        ))))));
    }

    match &args[0] {
        Value::Err(_) => Ok(Value::Ok(Box::new(Value::Unit))),
        Value::Ok(val) => {
            let error_msg = format!("Assertion failed: expected Err but got Ok({})", val);
            Ok(Value::Err(Box::new(Value::String(Arc::new(error_msg)))))
        }
        other => {
            let error_msg = format!(
                "Assertion failed: assert_err requires a Result, got {}",
                other.type_name()
            );
            Ok(Value::Err(Box::new(Value::String(Arc::new(error_msg)))))
        }
    }
}

/// Explicitly fail a test with a message
/// Usage: testing.fail("Test failed because...") -> Error
fn fail_test(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "fail expects 1 argument, got {}",
            args.len()
        ))))));
    }

    let message = match &args[0] {
        Value::String(s) => s.as_ref(),
        other => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
                "fail: message must be a string, got {}",
                other.type_name()
            ))))));
        }
    };

    let error_msg = format!("Test failed: {}", message);
    Ok(Value::Err(Box::new(Value::String(Arc::new(error_msg)))))
}

/// Run a test function with a name
/// Usage: testing.run_test("test_name", test_function) -> Result<String, Error>
fn run_test(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "run_test expects 2 arguments, got {}",
            args.len()
        ))))));
    }

    let test_name = match &args[0] {
        Value::String(s) => s.as_ref(),
        other => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
                "run_test: test name must be a string, got {}",
                other.type_name()
            ))))));
        }
    };

    // Stdlib builtins cannot call back into the interpreter to execute the
    // test function, so surface that honestly instead of reporting a false pass.
    Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
        "run_test: cannot execute test '{}' — use a `test \"{}\" {{ ... }}` block instead",
        test_name, test_name
    ))))))
}

/// Get a summary of test results
/// Usage: testing.test_summary() -> String
fn test_summary(_args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    // For now, return a simple summary
    // In a full implementation, this would track actual test statistics
    let summary = "Test Summary: 0 tests run, 0 passed, 0 failed";
    Ok(Value::String(Arc::new(summary.to_string())))
}

/// Reset test statistics
/// Usage: testing.reset_tests() -> Unit
fn reset_tests(_args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    // For now, just return unit
    // In a full implementation, this would reset test counters
    Ok(Value::Unit)
}

/// Helper function to compare values for equality
fn values_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Integer(a), Value::Integer(b)) => a == b,
        (Value::Float(a), Value::Float(b)) => floats_equal(*a, *b),
        (Value::String(a), Value::String(b)) => a == b,
        (Value::Boolean(a), Value::Boolean(b)) => a == b,
        (Value::Unit, Value::Unit) => true,
        (Value::List(a), Value::List(b)) => {
            if a.len() != b.len() {
                return false;
            }
            a.iter().zip(b.iter()).all(|(x, y)| values_equal(x, y))
        }
        (Value::Tuple(a), Value::Tuple(b)) => {
            if a.len() != b.len() {
                return false;
            }
            a.iter().zip(b.iter()).all(|(x, y)| values_equal(x, y))
        }
        (Value::Ok(a), Value::Ok(b)) => values_equal(a, b),
        (Value::Err(a), Value::Err(b)) => values_equal(a, b),
        // Allow numeric type coercion
        (Value::Integer(a), Value::Float(b)) => floats_equal(*a as f64, *b),
        (Value::Float(a), Value::Integer(b)) => floats_equal(*a, *b as f64),
        _ => false,
    }
}

/// Compare floats with a relative tolerance so small-magnitude values are not
/// spuriously equal (an absolute EPSILON made 1e-20 == 2e-20 "pass").
fn floats_equal(a: f64, b: f64) -> bool {
    if a == b {
        return true;
    }
    let diff = (a - b).abs();
    let scale = a.abs().max(b.abs());
    diff <= scale * f64::EPSILON * 4.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn string_val(s: &str) -> Value {
        Value::String(Arc::new(s.to_string()))
    }

    fn int_val(n: i64) -> Value {
        Value::Integer(n)
    }

    fn bool_val(b: bool) -> Value {
        Value::Boolean(b)
    }

    fn assert_ok_result(result: &Value) -> &Value {
        match result {
            Value::Ok(inner) => inner,
            Value::Err(err) => {
                panic!("Expected Ok, got Err: {:?}", err)
            }
            other => {
                panic!("Expected Result, got: {:?}", other)
            }
        }
    }

    fn assert_err_result(result: &Value) -> &Value {
        match result {
            Value::Err(inner) => inner,
            Value::Ok(val) => {
                panic!("Expected Err, got Ok: {:?}", val)
            }
            other => {
                panic!("Expected Result, got: {:?}", other)
            }
        }
    }

    #[test]
    fn test_testing_module_creation() {
        let module = create_testing_module();

        if let Value::Struct { type_name, fields } = module {
            assert_eq!(type_name, "Module");

            // Check that all expected functions are present
            let expected_functions = [
                "assert_eq",
                "assert_ne",
                "assert_true",
                "assert_false",
                "assert_ok",
                "assert_err",
                "fail",
                "run_test",
                "test_summary",
                "reset_tests",
            ];

            for func_name in &expected_functions {
                assert!(
                    fields.contains_key(*func_name),
                    "Missing function: {}",
                    func_name
                );
            }

            assert_eq!(fields.len(), expected_functions.len());
        } else {
            panic!("Expected struct module, got: {:?}", module);
        }
    }

    #[test]
    fn test_assert_eq_success() {
        let result = assert_eq(vec![int_val(42), int_val(42)]).unwrap();
        assert_ok_result(&result);
    }

    #[test]
    fn test_assert_eq_failure() {
        let result = assert_eq(vec![int_val(42), int_val(24)]).unwrap();
        let error = assert_err_result(&result);

        if let Value::String(msg) = error {
            assert!(msg.contains("Assertion failed"));
            assert!(msg.contains("expected 42 but got 24"));
        } else {
            panic!("Expected string error message, got: {:?}", error);
        }
    }

    #[test]
    fn test_assert_ne_success() {
        let result = assert_ne(vec![int_val(42), int_val(24)]).unwrap();
        assert_ok_result(&result);
    }

    #[test]
    fn test_assert_ne_failure() {
        let result = assert_ne(vec![int_val(42), int_val(42)]).unwrap();
        let error = assert_err_result(&result);

        if let Value::String(msg) = error {
            assert!(msg.contains("Assertion failed"));
            assert!(msg.contains("not equal"));
        } else {
            panic!("Expected string error message, got: {:?}", error);
        }
    }

    #[test]
    fn test_assert_true_success() {
        let result = assert_true(vec![bool_val(true)]).unwrap();
        assert_ok_result(&result);
    }

    #[test]
    fn test_assert_true_failure() {
        let result = assert_true(vec![bool_val(false)]).unwrap();
        let error = assert_err_result(&result);

        if let Value::String(msg) = error {
            assert!(msg.contains("expected true but got false"));
        } else {
            panic!("Expected string error message, got: {:?}", error);
        }
    }

    #[test]
    fn test_assert_false_success() {
        let result = assert_false(vec![bool_val(false)]).unwrap();
        assert_ok_result(&result);
    }

    #[test]
    fn test_assert_false_failure() {
        let result = assert_false(vec![bool_val(true)]).unwrap();
        let error = assert_err_result(&result);

        if let Value::String(msg) = error {
            assert!(msg.contains("expected false but got true"));
        } else {
            panic!("Expected string error message, got: {:?}", error);
        }
    }

    #[test]
    fn test_assert_ok_success() {
        let ok_value = Value::Ok(Box::new(int_val(42)));
        let result = assert_ok(vec![ok_value]).unwrap();
        assert_ok_result(&result);
    }

    #[test]
    fn test_assert_ok_failure() {
        let err_value = Value::Err(Box::new(string_val("error")));
        let result = assert_ok(vec![err_value]).unwrap();
        let error = assert_err_result(&result);

        if let Value::String(msg) = error {
            assert!(msg.contains("expected Ok but got Err"));
        } else {
            panic!("Expected string error message, got: {:?}", error);
        }
    }

    #[test]
    fn test_assert_err_success() {
        let err_value = Value::Err(Box::new(string_val("error")));
        let result = assert_err(vec![err_value]).unwrap();
        assert_ok_result(&result);
    }

    #[test]
    fn test_assert_err_failure() {
        let ok_value = Value::Ok(Box::new(int_val(42)));
        let result = assert_err(vec![ok_value]).unwrap();
        let error = assert_err_result(&result);

        if let Value::String(msg) = error {
            assert!(msg.contains("expected Err but got Ok"));
        } else {
            panic!("Expected string error message, got: {:?}", error);
        }
    }

    #[test]
    fn test_fail_test() {
        let result = fail_test(vec![string_val("Custom failure message")]).unwrap();
        let error = assert_err_result(&result);

        if let Value::String(msg) = error {
            assert!(msg.contains("Test failed: Custom failure message"));
        } else {
            panic!("Expected string error message, got: {:?}", error);
        }
    }

    #[test]
    fn test_run_test_basic() {
        let result = run_test(vec![
            string_val("my_test"),
            Value::Unit, // Placeholder for test function
        ])
        .unwrap();

        // run_test cannot execute the function from a stdlib builtin, so it
        // must report an error rather than a false pass
        let err_msg = assert_err_result(&result);
        if let Value::String(msg) = err_msg {
            assert!(msg.contains("cannot execute test 'my_test'"));
        } else {
            panic!("Expected string error message, got: {:?}", err_msg);
        }
    }

    #[test]
    fn test_values_equal() {
        // Basic equality
        assert!(values_equal(&int_val(42), &int_val(42)));
        assert!(values_equal(&string_val("hello"), &string_val("hello")));
        assert!(values_equal(&bool_val(true), &bool_val(true)));

        // Numeric coercion
        assert!(values_equal(&int_val(42), &Value::Float(42.0)));
        assert!(values_equal(&Value::Float(42.0), &int_val(42)));

        // Lists
        let list1 = Value::List(vec![int_val(1), int_val(2)].into());
        let list2 = Value::List(vec![int_val(1), int_val(2)].into());
        assert!(values_equal(&list1, &list2));

        // Inequality
        assert!(!values_equal(&int_val(42), &int_val(24)));
        assert!(!values_equal(&string_val("hello"), &string_val("world")));
    }

    #[test]
    fn test_function_dispatcher() {
        // Test that all functions can be called through the dispatcher
        let test_cases = vec![
            ("assert_eq", vec![int_val(1), int_val(1)]),
            ("assert_ne", vec![int_val(1), int_val(2)]),
            ("assert_true", vec![bool_val(true)]),
            ("assert_false", vec![bool_val(false)]),
            ("fail", vec![string_val("test")]),
            ("test_summary", vec![]),
            ("reset_tests", vec![]),
        ];

        for (func_name, args) in test_cases {
            let result = call_testing_function(func_name, args);
            assert!(
                result.is_ok(),
                "Function {} failed: {:?}",
                func_name,
                result
            );
        }
    }

    #[test]
    fn test_invalid_function() {
        let result = call_testing_function("invalid_function", vec![]);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Unknown testing function")
        );
    }
}
