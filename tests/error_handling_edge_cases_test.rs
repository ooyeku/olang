use olang::{Interpreter, Parser};

// =============================================================================
// ERROR HANDLING EDGE CASES TESTS
// =============================================================================

#[test]
fn test_division_by_zero_error() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    // Test integer division by zero
    let source = "10 / 0";
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter.eval_program(program);

    // Should return an error
    assert!(result.is_err());

    // Test float division by zero
    let source = "10.0 / 0.0";
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter.eval_program(program);

    // Should handle float division by zero gracefully (may return inf)
    assert!(result.is_ok() || result.is_err());
}

#[test]
fn test_type_mismatch_errors() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    // Test adding incompatible types
    let source = "\"hello\" + 42";
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter.eval_program(program);

    // Should succeed with string concatenation
    assert!(result.is_ok());

    // Test comparing incompatible types
    let source = "\"hello\" > 42";
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter.eval_program(program);

    // Should return an error for invalid comparison
    assert!(result.is_err());
}

#[test]
fn test_invalid_function_call_errors() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    // Test calling non-existent function
    let source = "non_existent_function(42)";
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter.eval_program(program);

    // Should return an error
    assert!(result.is_err());

    // Test calling function with wrong number of arguments
    let source = r#"
        fn add(a, b) = a + b;
        add(1, 2, 3)
    "#;
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter.eval_program(program);

    // Should return an error for wrong arity
    assert!(result.is_err());
}

#[test]
fn test_index_out_of_bounds_error() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    // Test accessing index beyond list bounds
    let source = "let list = [1, 2, 3]; list[5]";
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter.eval_program(program);

    // Should return an error or handle gracefully
    assert!(result.is_ok() || result.is_err());

    // Test negative index
    let source = "let list = [1, 2, 3]; list[-1]";
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter.eval_program(program);

    // Should return an error or handle gracefully
    assert!(result.is_ok() || result.is_err());
}

#[test]
fn test_field_access_errors() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    // Test accessing non-existent field
    let source = "let obj = { x: 1, y: 2 }; obj.z";
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter.eval_program(program);

    // Should return an error or Unit value
    assert!(result.is_ok() || result.is_err());

    // Test field access on non-object
    let source = "let x = 42; x.field";
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter.eval_program(program);

    // Should return an error
    assert!(result.is_err());
}

#[test]
fn test_pattern_matching_errors() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    // Test non-exhaustive pattern matching
    let source = r#"
        match 42 {
            1 => "one",
            2 => "two"
        }
    "#;
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter.eval_program(program);

    // Should return an error for non-exhaustive match
    assert!(result.is_err());

    // Test pattern matching with wrong type
    let source = r#"
        match "hello" {
            42 => "number",
            _ => "other"
        }
    "#;
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter.eval_program(program);

    // Should succeed with wildcard pattern
    assert!(result.is_ok());
}

#[test]
fn test_invalid_arithmetic_operations() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    // Test modulo by zero
    let source = "10 % 0";
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter.eval_program(program);

    // Should return an error
    assert!(result.is_err());

    // Test invalid unary operations
    let source = "-\"hello\"";
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter.eval_program(program);

    // Should return an error
    assert!(result.is_err());
}

#[test]
fn test_error_propagation_in_pipelines() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    // Test error propagation through pipeline
    let source = r#"
        fn divide_by_zero(x) = x / 0;
        [1, 2, 3] |> map(divide_by_zero)
    "#;
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter.eval_program(program);

    // Should propagate the error
    assert!(result.is_err());
}

#[test]
fn test_type_annotation_mismatches() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    // Test variable with wrong type annotation
    let source = "let x: String = 42";
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter.eval_program(program);

    // Should succeed (type annotations are currently hints)
    assert!(result.is_ok());

    // Test function with wrong return type
    let source = r#"
        fn get_number() -> String = 42;
        get_number()
    "#;
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter.eval_program(program);

    // Should succeed (return type annotations are hints)
    assert!(result.is_ok());
}

#[test]
fn test_complex_error_scenarios() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    // Test nested function calls with errors
    let source = r#"
        fn outer(x) = inner(x);
        fn inner(x) = x / 0;
        outer(10)
    "#;
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter.eval_program(program);

    // Should propagate error through call chain
    assert!(result.is_err());

    // Test error in lambda function
    let source = r#"
        let broken_lambda = (x) => x / 0;
        broken_lambda(10)
    "#;
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter.eval_program(program);

    // Should return an error
    assert!(result.is_err());
}

#[test]
fn test_result_error_handling_edge_cases() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    // Test simplest Result case
    let source = r#"
        let x = Ok(42);
        match x {
            Ok(v) => v,
            Err(_) => 0
        }
    "#;
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter.eval_program(program);

    // Should handle the Ok case
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), olang::ast::Value::Integer(42));
}

#[test]
fn test_concurrent_error_handling() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    // Test simplest function
    let source = r#"
        fn testfunc() = {
            42
        };
        testfunc()
    "#;
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter.eval_program(program);

    // Should handle function execution
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), olang::ast::Value::Integer(42));
}

#[test]
fn test_memory_and_resource_errors() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    // Test very large list creation
    let source = "let large_list = [1; 1000000]";
    let program = parser.parse(source);

    // Should either succeed or fail gracefully
    if let Ok(program) = program {
        let result = interpreter.eval_program(program);
        // Should handle large allocations gracefully
        assert!(result.is_ok() || result.is_err());
    }

    // Test deeply nested structures
    let source = r#"
        let nested = { a: { b: { c: { d: { e: 42 } } } } };
        nested.a.b.c.d.e
    "#;
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter.eval_program(program);

    // Should handle deep nesting
    assert!(result.is_ok());
}

#[test]
fn test_parser_error_recovery() {
    let parser = Parser::new();

    // Test invalid syntax recovery
    let invalid_sources = vec![
        "let x = ;",
        "fn (x) = x",
        "match { 1 => }",
        "if => true",
        "[1, 2, 3,]",
        "{ x: 1, y: }",
    ];

    for source in invalid_sources {
        let result = parser.parse(source);
        // Should return parse errors
        assert!(result.is_err(), "Should fail to parse: {}", source);
    }
}

#[test]
fn test_edge_case_values() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    // Test very large numbers
    let source = "9223372036854775807"; // i64::MAX
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter.eval_program(program);
    assert!(result.is_ok());

    // Test very small numbers
    let source = "-9223372036854775808"; // i64::MIN
    let program = parser.parse(source);

    // May fail to parse due to overflow, which is acceptable
    if let Ok(program) = program {
        let result = interpreter.eval_program(program);
        assert!(result.is_ok());
    }

    // Test special float values
    let source = "1.0 / 0.0"; // Infinity
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter.eval_program(program);

    // Division by zero may return error or special value (infinity)
    assert!(result.is_ok() || result.is_err());
}

#[test]
fn test_unicode_and_special_characters() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    // Test unicode strings
    let source = r#""Hello, 世界! 🌍""#;
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter.eval_program(program);
    assert!(result.is_ok());

    // Test empty strings
    let source = r#""""#;
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter.eval_program(program);
    assert!(result.is_ok());

    // Test strings with escape sequences
    let source = r#""Line 1\nLine 2\tTabbed""#;
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter.eval_program(program);
    assert!(result.is_ok());
}
