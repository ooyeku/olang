use olang::interpreter::InterpreterError;
use olang::{Interpreter, Parser};

#[test]
fn test_simple_tuple_destructuring() {
    let mut interpreter = Interpreter::new();
    let parser = Parser::new();

    // Test basic tuple destructuring
    let input = r#"
        let (x, y) = (1, 2);
        x + y
    "#;

    let program = parser.parse(input).expect("Failed to parse");
    let result = interpreter
        .eval_program(program)
        .expect("Failed to evaluate");

    assert_eq!(result.to_string(), "3");
}

#[test]
fn test_nested_tuple_destructuring() {
    let mut interpreter = Interpreter::new();
    let parser = Parser::new();

    // Test nested tuple destructuring
    let input = r#"
        let ((a, b), c) = ((1, 2), 3);
        a + b + c
    "#;

    let program = parser.parse(input).expect("Failed to parse");
    let result = interpreter
        .eval_program(program)
        .expect("Failed to evaluate");

    assert_eq!(result.to_string(), "6");
}

#[test]
fn test_list_destructuring() {
    let mut interpreter = Interpreter::new();
    let parser = Parser::new();

    // Test basic list destructuring
    let input = r#"
        let [first, second] = [10, 20];
        first + second
    "#;

    let program = parser.parse(input).expect("Failed to parse");
    let result = interpreter
        .eval_program(program)
        .expect("Failed to evaluate");

    assert_eq!(result.to_string(), "30");
}

#[test]
fn test_list_destructuring_with_rest() {
    let mut interpreter = Interpreter::new();
    let parser = Parser::new();

    // Test list destructuring with rest pattern
    let input = r#"
        let [first, ...rest] = [1, 2, 3, 4, 5];
        first
    "#;

    let program = parser.parse(input).expect("Failed to parse");
    let result = interpreter
        .eval_program(program)
        .expect("Failed to evaluate");

    assert_eq!(result.to_string(), "1");
}

#[test]
fn test_mixed_destructuring() {
    let mut interpreter = Interpreter::new();
    let parser = Parser::new();

    // Test mixed destructuring patterns
    let input = r#"
        let (x, [y, z]) = (10, [20, 30]);
        x + y + z
    "#;

    let program = parser.parse(input).expect("Failed to parse");
    let result = interpreter
        .eval_program(program)
        .expect("Failed to evaluate");

    assert_eq!(result.to_string(), "60");
}

#[test]
fn test_wildcard_pattern() {
    let mut interpreter = Interpreter::new();
    let parser = Parser::new();

    // Test wildcard pattern
    let input = r#"
        let (x, _) = (42, "ignored");
        x
    "#;

    let program = parser.parse(input).expect("Failed to parse");
    let result = interpreter
        .eval_program(program)
        .expect("Failed to evaluate");

    assert_eq!(result.to_string(), "42");
}

#[test]
fn test_destructuring_with_type_annotation() {
    let mut interpreter = Interpreter::new();
    let parser = Parser::new();

    // Test destructuring with type annotation
    let input = r#"
        let (x, y): (Int, Int) = (100, 200);
        x * y
    "#;

    let program = parser.parse(input).expect("Failed to parse");
    let result = interpreter
        .eval_program(program)
        .expect("Failed to evaluate");

    assert_eq!(result.to_string(), "20000");
}

#[test]
fn test_destructuring_strings() {
    let mut interpreter = Interpreter::new();
    let parser = Parser::new();

    // Test destructuring with strings
    let input = r#"
        let (name, greeting) = ("Alice", "Hello");
        greeting + ", " + name
    "#;

    let program = parser.parse(input).expect("Failed to parse");
    let result = interpreter
        .eval_program(program)
        .expect("Failed to evaluate");

    assert_eq!(result.to_string(), "\"Hello, Alice\"");
}

#[test]
fn test_destructuring_booleans() {
    let mut interpreter = Interpreter::new();
    let parser = Parser::new();

    // Test destructuring with booleans
    let input = r#"
        let (flag1, flag2) = (true, false);
        flag1 && !flag2
    "#;

    let program = parser.parse(input).expect("Failed to parse");
    let result = interpreter
        .eval_program(program)
        .expect("Failed to evaluate");

    assert_eq!(result.to_string(), "true");
}

#[test]
fn test_backwards_compatibility() {
    let mut interpreter = Interpreter::new();
    let parser = Parser::new();

    // Test that simple identifier patterns still work (backwards compatibility)
    let input = r#"
        let x = 42;
        let y = "hello";
        let z = true;
        x + 10
    "#;

    let program = parser.parse(input).expect("Failed to parse");
    let result = interpreter
        .eval_program(program)
        .expect("Failed to evaluate");

    assert_eq!(result.to_string(), "52");
}

#[test]
fn test_destructuring_in_multiple_statements() {
    let mut interpreter = Interpreter::new();
    let parser = Parser::new();

    // Test multiple destructuring statements
    let input = r#"
        let (a, b) = (1, 2);
        let [c, d] = [3, 4];
        let (e, f) = (a + c, b + d);
        e * f
    "#;

    let program = parser.parse(input).expect("Failed to parse");
    let result = interpreter
        .eval_program(program)
        .expect("Failed to evaluate");

    assert_eq!(result.to_string(), "24");
}

#[test]
fn test_pattern_match_failure() {
    let mut interpreter = Interpreter::new();
    let parser = Parser::new();

    // Test pattern match failure with wrong tuple size
    let input = r#"
        let (x, y, z) = (1, 2);
        x
    "#;

    let program = parser.parse(input).expect("Failed to parse");
    let result = interpreter.eval_program(program);

    // Should fail with pattern match error
    assert!(result.is_err());
    if let Err(InterpreterError::PatternMatchFailed) = result {
        // Expected error
    } else {
        panic!("Expected PatternMatchFailed error, got: {:?}", result);
    }
}

#[test]
fn test_destructuring_complex_expressions() {
    let mut interpreter = Interpreter::new();
    let parser = Parser::new();

    // Test destructuring with complex expressions on the right side
    let input = r#"
        let (x, y) = (10 + 5, 20 * 2);
        x + y
    "#;

    let program = parser.parse(input).expect("Failed to parse");
    let result = interpreter
        .eval_program(program)
        .expect("Failed to evaluate");

    assert_eq!(result.to_string(), "55");
}
