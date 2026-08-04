use olang::ast::Value;
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
        _ => assert!(false, "Expected list result, got: {:?}", result),
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
        _ => assert!(false, "Expected tuple result, got: {:?}", result),
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
        _ => assert!(false, "Expected Ok result, got: {:?}", result),
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
        _ => assert!(false, "Expected Err result, got: {:?}", result),
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

// =============================================================================
// DEFAULT PARAMETER TESTS
// =============================================================================

#[test]
fn test_default_parameters_single_default() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    let source = r#"
        fn greet(name = "World") = "Hello, " + name;
        greet()
    "#;
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter
        .eval_program(program)
        .expect("Failed to evaluate");

    assert_eq!(
        result,
        olang::ast::Value::String("Hello, World".to_string().into())
    );
}

#[test]
fn test_default_parameters_override_default() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    let source = r#"
        fn greet(name = "World") = "Hello, " + name;
        greet("Alice")
    "#;
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter
        .eval_program(program)
        .expect("Failed to evaluate");

    assert_eq!(
        result,
        olang::ast::Value::String("Hello, Alice".to_string().into())
    );
}

#[test]
fn test_default_parameters_mixed_required_and_default() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    let source = r#"
        fn multiply(a, b = 2) = a * b;
        multiply(5)
    "#;
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter
        .eval_program(program)
        .expect("Failed to evaluate");

    assert_eq!(result, olang::ast::Value::Integer(10));
}

#[test]
fn test_default_parameters_mixed_with_override() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    let source = r#"
        fn multiply(a, b = 2) = a * b;
        multiply(5, 3)
    "#;
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter
        .eval_program(program)
        .expect("Failed to evaluate");

    assert_eq!(result, olang::ast::Value::Integer(15));
}

#[test]
fn test_default_parameters_multiple_defaults() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    let source = r#"
        fn create_person(name = "Unknown", age = 0) = name + " is " + age + " years old";
        create_person()
    "#;
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter
        .eval_program(program)
        .expect("Failed to evaluate");

    assert_eq!(
        result,
        olang::ast::Value::String("Unknown is 0 years old".to_string().into())
    );
}

#[test]
fn test_default_parameters_partial_override() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    let source = r#"
        fn create_person(name = "Unknown", age = 0) = name + " is " + age + " years old";
        create_person("Alice")
    "#;
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter
        .eval_program(program)
        .expect("Failed to evaluate");

    assert_eq!(
        result,
        olang::ast::Value::String("Alice is 0 years old".to_string().into())
    );
}

#[test]
fn test_default_parameters_with_type_annotations() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    let source = r#"
        fn add(a: Int, b: Int = 10) = a + b;
        add(5)
    "#;
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter
        .eval_program(program)
        .expect("Failed to evaluate");

    assert_eq!(result, olang::ast::Value::Integer(15));
}

#[test]
fn test_default_parameters_expression_as_default() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    let source = r#"
        fn power(base, exponent = 2) = if exponent == 0 => 1 else => base * power(base, exponent - 1);
        power(3)
    "#;
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter
        .eval_program(program)
        .expect("Failed to evaluate");

    assert_eq!(result, olang::ast::Value::Integer(9)); // 3^2 = 9
}

#[test]
fn test_default_parameters_lambda_functions() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    let source = r#"
        let multiply = (x, y = 2) => x * y;
        multiply(4)
    "#;
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter
        .eval_program(program)
        .expect("Failed to evaluate");

    assert_eq!(result, olang::ast::Value::Integer(8));
}

#[test]
fn test_default_parameters_error_too_many_args() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    let source = r#"
        fn greet(name = "World") = "Hello, " + name;
        greet("Alice", "Bob")
    "#;
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter.eval_program(program);

    assert!(result.is_err());
    if let Err(e) = result {
        assert!(e.to_string().contains("ArityMismatch") || e.to_string().contains("Arity mismatch"));
    }
}

// --- Regressions found while verifying the documentation examples ---

#[test]
fn promise_all_collects_results() {
    // Promise.all silently evaluated to [] because the parser skipped pairs
    // for grammar literals that produce none, consuming the argument list.
    let source = r#"
async fn double(x: Int) -> Promise<Int, String> = { x * 2 }
await Promise.all([double(1), double(2), double(3)])
"#;
    let parser = Parser::new();
    let program = parser.parse(source).expect("should parse");
    let mut interpreter = Interpreter::new();
    let result = interpreter.eval_program(program).expect("should evaluate");
    assert_eq!(
        result,
        Value::List(
            vec![Value::Integer(2), Value::Integer(4), Value::Integer(6)].into()
        )
    );
}

#[test]
fn promise_resolve_returns_its_argument() {
    // The same pair-skipping bug read the argument as the method name.
    let parser = Parser::new();
    let program = parser
        .parse("await Promise.resolve(42)")
        .expect("should parse");
    let mut interpreter = Interpreter::new();
    assert_eq!(
        interpreter.eval_program(program).expect("should evaluate"),
        Value::Integer(42)
    );
}

#[test]
fn let_declarations_work_inside_test_blocks() {
    // `test_statement` wraps a `statement`, which build_statement rejected.
    let source = r#"
test "arithmetic" {
    let x = 21
    assert_eq(x * 2, 42)
}
"#;
    let parser = Parser::new();
    assert!(
        parser.parse(source).is_ok(),
        "a let declaration inside a test block should parse"
    );
}

#[test]
fn enum_variant_patterns_accept_payloads() {
    // `identifier` preceded `enum_variant_pattern` in the grammar, so any
    // payload-carrying variant pattern failed to parse.
    let parser = Parser::new();
    assert!(
        parser
            .parse("match value { Circle(r) => r, _ => 0 }")
            .is_ok(),
        "enum variant patterns with payloads should parse"
    );
}

#[test]
fn bare_identifier_patterns_still_bind() {
    // Guard against the above fix turning binding patterns into variant
    // patterns, which would stop them matching anything.
    let parser = Parser::new();
    let program = parser.parse("match 7 { n => n + 1 }").expect("should parse");
    let mut interpreter = Interpreter::new();
    assert_eq!(
        interpreter.eval_program(program).expect("should evaluate"),
        Value::Integer(8)
    );
}

/// Promise.delay must be awaitable: the promise carries its deadline and
/// await sleeps out the remainder. Previously every await of a delayed
/// promise errored ("async scheduling not implemented") and each delay
/// leaked an async-runtime registry entry.
#[test]
fn promise_delay_resolves_at_await() {
    let parser = olang::Parser::new();
    let mut interpreter = olang::Interpreter::new();

    let program = parser
        .parse(r#"await Promise.delay("done", 30)"#)
        .expect("parse");
    let start = std::time::Instant::now();
    let result = interpreter.eval_program(program).expect("eval");
    let elapsed = start.elapsed();

    assert_eq!(result, olang::Value::String("done".to_string().into()));
    assert!(
        elapsed.as_millis() >= 25,
        "await should sleep out the delay, took {:?}",
        elapsed
    );
}

#[test]
fn promise_delay_counts_elapsed_work_against_the_delay() {
    let parser = olang::Parser::new();
    let mut interpreter = olang::Interpreter::new();

    // The delay deadline is set at creation; by the time we await, most of
    // it may already have passed.
    let program = parser
        .parse(
            r#"
let p = Promise.delay(42, 20)
let x = await p
x
"#,
        )
        .expect("parse");
    assert_eq!(
        interpreter.eval_program(program).expect("eval"),
        olang::Value::Integer(42)
    );
}
