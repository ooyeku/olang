use olang::{Interpreter, Parser};

// =============================================================================
// V0.13 FEATURE TESTS - COMPREHENSIVE COVERAGE
// =============================================================================

// =============================================================================
// FEATURE 1: Enhanced String Literals & Template Interpolation
// =============================================================================

#[test]
fn test_basic_template_interpolation() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    let source = r#"
        let name = "Alice";
        let age = 25;
        `Hello ${name}! You are ${age} years old.`
    "#;
    
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter.eval_program(program).expect("Failed to evaluate");

    assert_eq!(
        result,
        olang::ast::Value::String("Hello Alice! You are 25 years old.".to_string().into())
    );
}

#[test]
fn test_template_interpolation_with_math() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    let source = r#"
        let x = 10;
        let y = 5;
        `Math: ${x} + ${y} = ${x + y}, ${x} * ${y} = ${x * y}`
    "#;
    
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter.eval_program(program).expect("Failed to evaluate");

    assert_eq!(
        result,
        olang::ast::Value::String("Math: 10 + 5 = 15, 10 * 5 = 50".to_string().into())
    );
}

#[test]
fn test_template_interpolation_with_objects() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    let source = r#"
        let user = { name: "Bob", score: 1250 };
        `Player ${user.name} has ${user.score} points!`
    "#;
    
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter.eval_program(program).expect("Failed to evaluate");

    assert_eq!(
        result,
        olang::ast::Value::String("Player Bob has 1250 points!".to_string().into())
    );
}

#[test]
fn test_template_interpolation_nested_expressions() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    let source = r#"
        let items = [1, 2, 3];
        `List has ${len(items)} items: ${items[0]}, ${items[1]}, ${items[2]}`
    "#;
    
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter.eval_program(program).expect("Failed to evaluate");

    assert_eq!(
        result,
        olang::ast::Value::String("List has 3 items: 1, 2, 3".to_string().into())
    );
}

#[test]
fn test_hex_escape_sequences() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    let source = r#""Hex chars: \x41\x42\x43""#;
    
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter.eval_program(program).expect("Failed to evaluate");

    assert_eq!(
        result,
        olang::ast::Value::String("Hex chars: ABC".to_string().into())
    );
}

#[test]
fn test_unicode_escape_sequences() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    let source = r#""Unicode: \u{1F600} \u{03B1}\u{03B2}\u{03B3}""#;
    
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter.eval_program(program).expect("Failed to evaluate");

    assert_eq!(
        result,
        olang::ast::Value::String("Unicode: 😀 αβγ".to_string().into())
    );
}

#[test]
fn test_null_escape_sequence() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    let source = r#""Null char: \0 here""#;
    
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter.eval_program(program).expect("Failed to evaluate");

    let expected = format!("Null char: {} here", '\0');
    assert_eq!(
        result,
        olang::ast::Value::String(expected.into())
    );
}

#[test]
fn test_raw_strings() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    let source = r#"r"C:\Users\Alice\Documents\file.txt""#;
    
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter.eval_program(program).expect("Failed to evaluate");

    assert_eq!(
        result,
        olang::ast::Value::String(r"C:\Users\Alice\Documents\file.txt".to_string().into())
    );
}

#[test]
fn test_raw_strings_with_quotes() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    // The source should contain a raw string literal with escaped quotes and
    // newline characters. The previous version had an extra `"` before the
    // trailing `#` which broke the Rust string literal syntax.
    // Construct a raw string in the O language that contains escaped quotes
    // and a newline escape sequence. The outer Rust raw string allows us to
    // embed the double quotes without additional escaping.
    let source = r#"r"String with \"quotes\" and \n newlines""#;
    
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter.eval_program(program).expect("Failed to evaluate");

    assert_eq!(
        result,
        // The expected value after evaluation is the regular string with the
        // escaped characters interpreted by the O language runtime.
        olang::ast::Value::String(r#"String with \"quotes\" and \n newlines"#.to_string().into())
    );
}

#[test]
fn test_empty_template_interpolation() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    let source = r#"`Empty: ${""} End`"#;
    
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter.eval_program(program).expect("Failed to evaluate");

    assert_eq!(
        result,
        olang::ast::Value::String("Empty:  End".to_string().into())
    );
}

// =============================================================================
// FEATURE 2: Advanced Pattern Matching with Guards & Ranges
// =============================================================================

#[test]
fn test_integer_range_patterns() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    let source = r#"
        match 15 {
            1..=10 => "single digit",
            11..=99 => "double digit",
            100..=999 => "triple digit",
            _ => "large number"
        }
    "#;
    
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter.eval_program(program).expect("Failed to evaluate");

    assert_eq!(
        result,
        olang::ast::Value::String("double digit".to_string().into())
    );
}

#[test]
fn test_character_range_patterns() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    let source = r#"
        match 'm' {
            'a'..'z' => "lowercase",
            'A'..'Z' => "uppercase",
            '0'..'9' => "digit",
            _ => "other"
        }
    "#;
    
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter.eval_program(program).expect("Failed to evaluate");

    assert_eq!(
        result,
        olang::ast::Value::String("lowercase".to_string().into())
    );
}

#[test]
fn test_range_pattern_boundaries() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    // Test inclusive range boundary
    let source = r#"
        match 10 {
            1..=10 => "in range",
            _ => "out of range"
        }
    "#;
    
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter.eval_program(program).expect("Failed to evaluate");

    assert_eq!(
        result,
        olang::ast::Value::String("in range".to_string().into())
    );
}

#[test]
fn test_guard_clauses_with_age() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    let source = r#"
        let user = { age: 25 };
        match user {
            { age } if age >= 65 => "senior",
            { age } if age >= 18 => "adult",
            { age } if age >= 13 => "teenager",
            _ => "child"
        }
    "#;
    
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter.eval_program(program).expect("Failed to evaluate");

    assert_eq!(
        result,
        olang::ast::Value::String("adult".to_string().into())
    );
}

#[test]
fn test_guard_clauses_with_lists() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    let source = r#"
        let data = [10, 5, 3];
        match data {
            [first, second, ...rest] if first > second => "descending",
            [first, second, ...rest] if first < second => "ascending", 
            [single] if single > 100 => "large single",
            [] => "empty",
            _ => "other"
        }
    "#;
    
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter.eval_program(program).expect("Failed to evaluate");

    assert_eq!(
        result,
        olang::ast::Value::String("descending".to_string().into())
    );
}

#[test]
fn test_complex_range_and_guard_combination() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    let source = r#"
        let score = 85;
        match score {
            90..=100 => "A grade",
            80..=89 if score >= 85 => "B+ grade",
            80..=89 => "B grade", 
            70..=79 => "C grade",
            _ => "Below C"
        }
    "#;
    
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter.eval_program(program).expect("Failed to evaluate");

    assert_eq!(
        result,
        olang::ast::Value::String("B+ grade".to_string().into())
    );
}

#[test]
fn test_or_patterns_with_results() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    let source = r#"
        match Ok(42) {
            Ok(_) | Some(_) => "success",
            Err(_) | None => "failure"
        }
    "#;
    
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter.eval_program(program).expect("Failed to evaluate");

    assert_eq!(
        result,
        olang::ast::Value::String("success".to_string().into())
    );
}

#[test]
fn test_nested_pattern_matching() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    let source = r#"
        let response = { status: "success", data: { value: 42 } };
        match response {
            { status: "success", data: { value } } if value > 40 => "high success",
            { status: "success", data: { value } } => "low success",
            { status: "error" } => "error occurred",
            _ => "unknown status"
        }
    "#;
    
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter.eval_program(program).expect("Failed to evaluate");

    assert_eq!(
        result,
        olang::ast::Value::String("high success".to_string().into())
    );
}

// =============================================================================
// FEATURE 3: Union Types & Enhanced Type System
// =============================================================================

#[test]
fn test_literal_type_patterns() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    let source = r#"
        match "pending" {
            "pending" => "⏳ Waiting",
            "running" => "🏃 In progress",
            "completed" => "✅ Done",
            "failed" => "❌ Error",
            _ => "Unknown status"
        }
    "#;
    
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter.eval_program(program).expect("Failed to evaluate");

    assert_eq!(
        result,
        olang::ast::Value::String("⏳ Waiting".to_string().into())
    );
}

#[test]
fn test_union_type_api_response_pattern() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    let source = r#"
        let response = { success: true, data: "User created" };
        match response {
            { success: true, data } => `Success: ${data}`,
            { success: false, error, code } => `Error ${code}: ${error}`,
            _ => "Invalid response format"
        }
    "#;
    
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter.eval_program(program).expect("Failed to evaluate");

    assert_eq!(
        result,
        olang::ast::Value::String("Success: User created".to_string().into())
    );
}

#[test]
fn test_union_type_error_response_pattern() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    let source = r#"
        let response = { success: false, error: "Validation failed", code: 400 };
        match response {
            { success: true, data } => `Success: ${data}`,
            { success: false, error, code } => `Error ${code}: ${error}`,
            _ => "Invalid response format"
        }
    "#;
    
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter.eval_program(program).expect("Failed to evaluate");

    assert_eq!(
        result,
        olang::ast::Value::String("Error 400: Validation failed".to_string().into())
    );
}

#[test]
fn test_http_method_union_types() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    let source = r#"
        fn handle_request(method) = {
            match method {
                "GET" => "Retrieving data",
                "POST" => "Creating resource",
                "PUT" => "Updating resource", 
                "DELETE" => "Removing resource",
                _ => "Unsupported method"
            }
        };
        handle_request("POST")
    "#;
    
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter.eval_program(program).expect("Failed to evaluate");

    assert_eq!(
        result,
        olang::ast::Value::String("Creating resource".to_string().into())
    );
}

#[test]
fn test_log_level_union_types() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    let source = r#"
        fn log_message(level, message) = {
            match level {
                "DEBUG" => `🐛 DEBUG: ${message}`,
                "INFO" => `ℹ️ INFO: ${message}`,
                "WARN" => `⚠️ WARN: ${message}`,
                "ERROR" => `❌ ERROR: ${message}`,
                _ => `? UNKNOWN: ${message}`
            }
        };
        log_message("WARN", "This is a warning")
    "#;
    
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter.eval_program(program).expect("Failed to evaluate");

    assert_eq!(
        result,
        olang::ast::Value::String("⚠️ WARN: This is a warning".to_string().into())
    );
}

// =============================================================================
// COMBINED V0.13 FEATURES INTEGRATION TESTS
// =============================================================================

#[test]
fn test_user_management_system_comprehensive() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    let source = r#"
        fn validate_user(name, email, age, role) = {
            match (name, email, age, role) {
                (n, e, a, r) if n == "" => { success: false, message: "Name cannot be empty", code: 400 },
                (n, e, a, r) if a < 0 => { success: false, message: `Invalid age: ${a}`, code: 400 },
                (n, e, a, r) if a > 150 => { success: false, message: `Unrealistic age: ${a}`, code: 400 },
                (n, e, a, "admin") if a < 21 => { success: false, message: "Admin must be 21+", code: 403 },
                _ => { success: true, message: "Valid user", code: 200 }
            }
        };
        
        let validation = validate_user("Alice", "alice@example.com", 25, "admin");
        
        match validation {
            { success: true, message } => `✅ ${message}`,
            { success: false, message, code } => `❌ Error ${code}: ${message}`,
            _ => "Unknown validation result"
        }
    "#;
    
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter.eval_program(program).expect("Failed to evaluate");

    assert_eq!(
        result,
        olang::ast::Value::String("✅ Valid user".to_string().into())
    );
}

#[test]
fn test_data_processing_pipeline_v013() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    let source = r#"
        fn process_item(item) = {
            match item {
                { type: "user", data: { age } } if age >= 18 => `Adult user: ${data.name}`,
                { type: "user", data: { age } } => `Minor user: ${data.name}`,
                { type: "order", data: { total } } if total > 100 => `Large order: $${total}`,
                { type: "order", data: { total } } => `Small order: $${total}`,
                _ => "Unknown item type"
            }
        };
        
        let item = { type: "order", data: { total: 150, id: "ORD-001" } };
        process_item(item)
    "#;
    
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter.eval_program(program).expect("Failed to evaluate");

    assert_eq!(
        result,
        olang::ast::Value::String("Large order: $150".to_string().into())
    );
}

#[test]
fn test_configuration_system_v013() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    let source = r#"
        fn load_config(env) = {
            match env {
                "development" => { 
                    database_url: r"sqlite:///dev.db", 
                    debug: true,
                    port: 3000
                },
                "production" => { 
                    database_url: r"postgresql://prod.db",
                    debug: false, 
                    port: 80
                },
                _ => { 
                    database_url: r"sqlite:///test.db",
                    debug: true,
                    port: 8080
                } 
            }
        };
        
        let config = load_config("production");
        `Config loaded: Database=${config.database_url}, Debug=${config.debug}, Port=${config.port}`
    "#;
    
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter.eval_program(program).expect("Failed to evaluate");

    assert_eq!(
        result,
        olang::ast::Value::String("Config loaded: Database=postgresql://prod.db, Debug=false, Port=80".to_string().into())
    );
}

// =============================================================================
// EDGE CASES AND ERROR HANDLING
// =============================================================================

#[test]
fn test_template_interpolation_with_booleans() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    let source = r#"
        let active = true;
        let inactive = false;
        `Status: active=${active}, inactive=${inactive}`
    "#;
    
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter.eval_program(program).expect("Failed to evaluate");

    assert_eq!(
        result,
        olang::ast::Value::String("Status: active=true, inactive=false".to_string().into())
    );
}

#[test]
fn test_range_pattern_edge_cases() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    let source = r#"
        match 0 {
            0..=0 => "exactly zero",
            1..=1 => "exactly one",
            _ => "other"
        }
    "#;
    
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter.eval_program(program).expect("Failed to evaluate");

    assert_eq!(
        result,
        olang::ast::Value::String("exactly zero".to_string().into())
    );
}

#[test]
fn test_guard_clause_with_multiple_conditions() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    let source = r#"
        let user = { age: 25, premium: true, active: true };
        match user {
            { age, premium, active } if age >= 18 && premium && active => "premium adult",
            { age, active } if age >= 18 && active => "regular adult",
            { age } if age >= 18 => "inactive adult",
            _ => "minor or invalid"
        }
    "#;
    
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter.eval_program(program).expect("Failed to evaluate");

    assert_eq!(
        result,
        olang::ast::Value::String("premium adult".to_string().into())
    );
}

#[test]
fn test_mixed_string_types_in_template() {
    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    let source = r#"
        let regular = "regular string";
        let raw = r"raw\string\with\backslashes";
        let hex = "\x48\x65\x6C\x6C\x6F";
        `Mixed: ${regular} | ${raw} | ${hex}`
    "#;
    
    let program = parser.parse(source).expect("Failed to parse");
    let result = interpreter.eval_program(program).expect("Failed to evaluate");

    assert_eq!(
        result,
        olang::ast::Value::String("Mixed: regular string | raw\\string\\with\\backslashes | Hello".to_string().into())
    );
} 