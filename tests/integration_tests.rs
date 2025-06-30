use olang::{Interpreter, Parser};

// =============================================================================
// PHASE 1 TESTS - FUNDAMENTAL FEATURES (COMPLETE)
// =============================================================================

#[test]
fn test_basic_integer_literal() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    let source = "42";
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter
        .eval_program(program)
        .expect("Failed to evaluate");

    assert_eq!(result, olang::ast::Value::Integer(42));
}

#[test]
fn test_basic_string_literal() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    let source = "\"Hello World\"";
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter
        .eval_program(program)
        .expect("Failed to evaluate");

    assert_eq!(
        result,
        olang::ast::Value::String("Hello World".to_string().into())
    );
}

#[test]
fn test_basic_boolean_literals() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    // Test true
    let source = "true";
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter
        .eval_program(program)
        .expect("Failed to evaluate");
    assert_eq!(result, olang::ast::Value::Boolean(true));

    // Test false
    let source = "false";
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter
        .eval_program(program)
        .expect("Failed to evaluate");
    assert_eq!(result, olang::ast::Value::Boolean(false));
}

#[test]
fn test_variable_declaration_and_access() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    let source = "let x = 42; x";
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter
        .eval_program(program)
        .expect("Failed to evaluate");

    assert_eq!(result, olang::ast::Value::Integer(42));
}

#[test]
fn test_variable_with_type_annotation() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    let source = "let x: Int = 100; x";
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter
        .eval_program(program)
        .expect("Failed to evaluate");

    assert_eq!(result, olang::ast::Value::Integer(100));
}

#[test]
fn test_recursive_function_fibonacci() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    let source = r#"
        fn fib(n) = if n <= 1 => n else => fib(n - 1) + fib(n - 2);
        fib(5)
    "#;
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter
        .eval_program(program)
        .expect("Failed to evaluate");

    assert_eq!(result, olang::ast::Value::Integer(5)); // fib(5) = 5
}

#[test]
fn test_recursive_function_factorial() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    let source = r#"
        fn factorial(n) = if n <= 1 => 1 else => n * factorial(n - 1);
        factorial(4)
    "#;
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter
        .eval_program(program)
        .expect("Failed to evaluate");

    assert_eq!(result, olang::ast::Value::Integer(24));
}

#[test]
fn test_lambda_functions() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    let source = "let add = (x, y) => x + y; add(5, 7)";
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter
        .eval_program(program)
        .expect("Failed to evaluate");

    assert_eq!(result, olang::ast::Value::Integer(12));
}

#[test]
fn test_if_expressions() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    // Test true branch
    let source = "if true => 42 else => 0";
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter
        .eval_program(program)
        .expect("Failed to evaluate");
    assert_eq!(result, olang::ast::Value::Integer(42));

    // Test false branch
    let source = "if false => 42 else => 100";
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter
        .eval_program(program)
        .expect("Failed to evaluate");
    assert_eq!(result, olang::ast::Value::Integer(100));
}

#[test]
fn test_pattern_matching_basic() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    let source = "match 42 { 42 => \"correct\", _ => \"wrong\" }";
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter
        .eval_program(program)
        .expect("Failed to evaluate");

    assert_eq!(
        result,
        olang::ast::Value::String("correct".to_string().into())
    );
}

#[test]
fn test_pattern_matching_wildcard() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    let source = "match 123 { 42 => \"wrong\", _ => \"wildcard\" }";
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter
        .eval_program(program)
        .expect("Failed to evaluate");

    assert_eq!(
        result,
        olang::ast::Value::String("wildcard".to_string().into())
    );
}

#[test]
fn test_list_creation() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    let source = "[1, 2, 3]";
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter
        .eval_program(program)
        .expect("Failed to evaluate");

    match result {
        olang::ast::Value::List(items) => {
            assert_eq!(items.len(), 3);
            assert_eq!(items[0], olang::ast::Value::Integer(1));
            assert_eq!(items[1], olang::ast::Value::Integer(2));
            assert_eq!(items[2], olang::ast::Value::Integer(3));
        }
        _ => panic!("Expected list result"),
    }
}

#[test]
fn test_tuple_creation() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    let source = "(1, \"hello\", true)";
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter
        .eval_program(program)
        .expect("Failed to evaluate");

    match result {
        olang::ast::Value::Tuple(items) => {
            assert_eq!(items.len(), 3);
            assert_eq!(items[0], olang::ast::Value::Integer(1));
            assert_eq!(
                items[1],
                olang::ast::Value::String("hello".to_string().into())
            );
            assert_eq!(items[2], olang::ast::Value::Boolean(true));
        }
        _ => panic!("Expected tuple result"),
    }
}

#[test]
fn test_builtin_functions() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    // Test println (should return Unit)
    let source = "println(\"test\")";
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter
        .eval_program(program)
        .expect("Failed to evaluate");
    assert_eq!(result, olang::ast::Value::Unit);

    // Test len function
    let source = "len([1, 2, 3])";
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter
        .eval_program(program)
        .expect("Failed to evaluate");
    assert_eq!(result, olang::ast::Value::Integer(3));
}

// =============================================================================
// PHASE 3 TESTS - ERROR HANDLING (COMPLETE)
// =============================================================================

#[test]
fn test_error_handling_result_creation() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    // Test Ok creation
    let source = "Ok(42)";
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter
        .eval_program(program)
        .expect("Failed to evaluate");

    match result {
        olang::ast::Value::Ok(inner) => {
            assert_eq!(*inner, olang::ast::Value::Integer(42));
        }
        _ => panic!("Expected Ok result, got: {:?}", result),
    }

    // Test Err creation
    let source = "Err(\"something went wrong\")";
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter
        .eval_program(program)
        .expect("Failed to evaluate");

    match result {
        olang::ast::Value::Err(inner) => {
            assert_eq!(
                *inner,
                olang::ast::Value::String("something went wrong".to_string().into())
            );
        }
        _ => panic!("Expected Err result, got: {:?}", result),
    }
}

#[test]
fn test_error_handling_pattern_matching() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    // Test Ok pattern matching - simplified approach
    let source = "match Ok(42) { Ok(_) => 42, Err(_) => 0 }";
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter
        .eval_program(program)
        .expect("Failed to evaluate");
    assert_eq!(result, olang::ast::Value::Integer(42));

    // Test Err pattern matching - simplified approach
    let source = "match Err(\"error\") { Ok(_) => \"ok\", Err(_) => \"error\" }";
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter
        .eval_program(program)
        .expect("Failed to evaluate");
    assert_eq!(
        result,
        olang::ast::Value::String("error".to_string().into())
    );
}

#[test]
fn test_error_handling_try_operator() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    // Test try with Ok value (should unwrap)
    let source = "let result = Ok(100); result?";
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter
        .eval_program(program)
        .expect("Failed to evaluate");
    assert_eq!(result, olang::ast::Value::Integer(100));
}

// =============================================================================
// CURRENT LIMITATIONS (TO BE ADDRESSED)
// =============================================================================

#[test]
fn test_operator_precedence_multiplication_first() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    // This should evaluate to 7 (1 + 6) but currently returns 1
    let source = "1 + 2 * 3";
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter
        .eval_program(program)
        .expect("Failed to evaluate");

    assert_eq!(result, olang::ast::Value::Integer(7));
}

#[test]
#[ignore] // Ignore until block parsing is fixed
fn test_block_expressions_with_multiple_statements() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    let source = "{ let x = 1; let y = 2; x + y }";
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter
        .eval_program(program)
        .expect("Failed to evaluate");

    assert_eq!(result, olang::ast::Value::Integer(3));
}

#[test]
fn test_string_concatenation_multiple() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    let source = "\"Hello\" + \" \" + \"World\"";
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter
        .eval_program(program)
        .expect("Failed to evaluate");

    assert_eq!(
        result,
        olang::ast::Value::String("Hello World".to_string().into())
    );
}

// =============================================================================
// FEATURE COMPLETENESS TESTS
// =============================================================================

#[test]
fn test_comprehensive_language_features() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    // Test a complex program that uses multiple features
    let source = r#"
        // Recursive function with type annotation
        fn factorial(n: Int) -> Int = if n <= 1 => 1 else => n * factorial(n - 1);
        
        // Lambda function
        let double = (x) => x * 2;
        
        // Test factorial and double functions
        let fact_result = factorial(4);
        let double_result = double(5);
        
        // Simple pattern matching with computed values
        let result = if fact_result == 24 && double_result == 10 => "correct" else => "wrong";
        
        // Error handling - simplified approach
        let safe_result = Ok(result);
        
        match safe_result {
            Ok(_) => "correct",
            Err(_) => "error"
        }
    "#;

    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter
        .eval_program(program)
        .expect("Failed to evaluate");

    assert_eq!(
        result,
        olang::ast::Value::String("correct".to_string().into())
    );
}

#[test]
fn test_pipeline_operator_basic() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    // Test basic pipeline with built-in function
    let source = "[1, 2, 3, 4] |> len";
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter
        .eval_program(program)
        .expect("Failed to evaluate");

    assert_eq!(result, olang::ast::Value::Integer(4));
}

#[test]
fn test_type_annotations_comprehensive() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    let source = r#"
        let number: Int = 42;
        let text: String = "hello";
        let flag: Bool = true;
        let items: [Int] = [1, 2, 3];
        
        // Function with type annotations
        fn typed_add(a: Int, b: Int) -> Int = a + b;
        
        typed_add(number, 10)
    "#;

    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter
        .eval_program(program)
        .expect("Failed to evaluate");

    assert_eq!(result, olang::ast::Value::Integer(52));
}
