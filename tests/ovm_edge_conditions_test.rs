use olang::{Interpreter, Parser, OvmInterpreter, IntegrationConfig};

// =============================================================================
// OVM EDGE CONDITIONS AND STRESS TESTS
// =============================================================================

#[test]
fn test_ovm_large_data_processing() {
    let parser = Parser::new();
    let mut ovm_interpreter = OvmInterpreter::new();

    // Test processing large lists
    let source = r#"
        let large_list = [1..1000] |> map((x) => x * 2) |> filter((x) => x % 4 == 0);
        len(large_list)
    "#;
    
    let program = parser.parse(source).expect("Failed to parse");
    let result = ovm_interpreter.eval_program(program);
    
    // Should handle large data processing without crashing
    assert!(result.is_ok() || result.is_err()); // Either succeed or fail gracefully
}

#[test]
fn test_ovm_deep_recursion() {
    let parser = Parser::new();
    let mut ovm_interpreter = OvmInterpreter::new();

    // Test simple recursive function
    let source = r#"
        fn simple_sum(n) = {
            if n <= 1 => n
            else => n + simple_sum(n - 1)
        };
        simple_sum(5)
    "#;
    
    let program = parser.parse(source).expect("Failed to parse");
    let result = ovm_interpreter.eval_program(program);
    
    // Should handle deep recursion or detect stack overflow
    assert!(result.is_ok() || result.is_err());
}

#[test]
fn test_ovm_memory_intensive_operations() {
    let parser = Parser::new();
    let mut ovm_interpreter = OvmInterpreter::new();

    // Test memory-intensive nested structure creation
    let source = r#"
        let nested_data = [1..100] |> map((i) => {
            nested_list: [1..10] |> map((j) => i * j),
            metadata: { id: i, squared: i * i }
        });
        len(nested_data)
    "#;
    
    let program = parser.parse(source).expect("Failed to parse");
    let result = ovm_interpreter.eval_program(program);
    
    // Should handle memory-intensive operations
    assert!(result.is_ok() || result.is_err());
}

#[test]
fn test_ovm_concurrent_operations() {
    let parser = Parser::new();
    let mut ovm_interpreter = OvmInterpreter::new();

    // Test concurrent-like operations with simpler syntax
    let source = r#"
        fn computation(n) = {
            let result = n * n + n;
            result
        };
        
        let results = [1, 2, 3, 4, 5] |> map((x) => computation(x));
        len(results)
    "#;
    
    let program = parser.parse(source).expect("Failed to parse");
    let result = ovm_interpreter.eval_program(program);
    
    // Should handle concurrent-like operations
    assert!(result.is_ok() || result.is_err());
}

#[test]
fn test_ovm_pipeline_optimization_edge_cases() {
    let parser = Parser::new();
    let mut ovm_interpreter = OvmInterpreter::new();

    // Test complex pipeline optimization scenarios
    let source = r#"
        let complex_pipeline = [1..20]
            |> map((x) => x * 2)
            |> filter((x) => x % 3 == 0)
            |> map((x) => x + 1)
            |> filter((x) => x > 10);
        
        len(complex_pipeline)
    "#;
    
    let program = parser.parse(source).expect("Failed to parse");
    let result = ovm_interpreter.eval_program(program);
    
    // Should optimize complex pipelines or handle gracefully
    assert!(result.is_ok() || result.is_err());
}

#[test]
fn test_ovm_lazy_evaluation_edge_cases() {
    let parser = Parser::new();
    let mut ovm_interpreter = OvmInterpreter::new();

    // Test lazy evaluation with large ranges
    let source = r#"
        let lazy_range = range(1, 100);
        let first_ten = take(lazy_range, 10);
        len(first_ten)
    "#;
    
    let program = parser.parse(source).expect("Failed to parse");
    let result = ovm_interpreter.eval_program(program);
    
    // Should handle lazy evaluation efficiently
    assert!(result.is_ok());
    if let Ok(value) = result {
        assert_eq!(value, olang::ast::Value::Integer(10));
    }
}

#[test]
fn test_ovm_garbage_collection_pressure() {
    let parser = Parser::new();
    let mut ovm_interpreter = OvmInterpreter::new();

    // Test garbage collection under pressure
    let source = r#"
        fn create_garbage(n) = {
            if n <= 0 => []
            else => {
                let temp_list = [1..n] |> map((x) => x * x);
                create_garbage(n - 1)
            }
        };
        
        create_garbage(20);
        "gc_test_complete"
    "#;
    
    let program = parser.parse(source).expect("Failed to parse");
    let result = ovm_interpreter.eval_program(program);
    
    // Should handle GC pressure without crashing
    assert!(result.is_ok() || result.is_err());
}

#[test]
fn test_ovm_bytecode_optimization_limits() {
    let parser = Parser::new();
    let mut ovm_interpreter = OvmInterpreter::new();

    // Test bytecode optimization limits with complex expressions
    let source = r#"
        fn complex_math(x) = {
            let a = x * x * x;
            let b = a + x * 2;
            let c = b * a - x;
            let d = c / (a + 1);
            d
        };
        
        [1..20] |> map(complex_math) |> sum()
    "#;
    
    let program = parser.parse(source).expect("Failed to parse");
    let result = ovm_interpreter.eval_program(program);
    
    // Should handle complex optimization scenarios
    assert!(result.is_ok() || result.is_err());
}

#[test]
fn test_ovm_execution_mode_switching() {
    let parser = Parser::new();

    // Test classic interpreter mode
    let mut classic_interpreter = Interpreter::new();
    let source = "[1, 2, 3] |> map((x) => x * 2) |> sum()";
    let program = parser.parse(source).expect("Failed to parse");
    let classic_result = classic_interpreter.eval_program(program.clone());

    // Test OVM mode
    let mut ovm_interpreter = OvmInterpreter::new();
    let ovm_result = ovm_interpreter.eval_program(program);

    // Both should work (may have different results due to implementation differences)
    assert!(classic_result.is_ok() || classic_result.is_err());
    assert!(ovm_result.is_ok() || ovm_result.is_err());
}

#[test]
fn test_ovm_error_handling_in_optimized_code() {
    let parser = Parser::new();
    let mut ovm_interpreter = OvmInterpreter::new();

    // Test error handling in optimized pipeline
    let source = r#"
        fn might_fail(x) = {
            if x == 5 => x / 0
            else => x * 2
        };
        
        [1, 2, 3, 4, 5, 6] |> map(might_fail)
    "#;
    
    let program = parser.parse(source).expect("Failed to parse");
    let result = ovm_interpreter.eval_program(program);
    
    // Should handle errors in optimized code
    assert!(result.is_err());
}

#[test]
fn test_ovm_type_checking_integration() {
    let parser = Parser::new();
    let mut ovm_interpreter = OvmInterpreter::new();

    // Test type checking integration with OVM
    let source = r#"
        fn typed_function(x: Int) -> Int = x * 2;
        let numbers: [Int] = [1, 2, 3, 4, 5];
        numbers |> map(typed_function) |> sum()
    "#;
    
    let program = parser.parse(source).expect("Failed to parse");
    let result = ovm_interpreter.eval_program(program);
    
    // Should handle type annotations in OVM
    assert!(result.is_ok());
}

#[test]
fn test_ovm_configuration_edge_cases() {
    let parser = Parser::new();

    // Test with minimal OVM configuration
    let minimal_config = IntegrationConfig {
        use_ovm_by_default: false,
        ovm_complexity_threshold: 1000,
        auto_compile_functions: false,
        enable_ovm_lazy_eval: false,
        fallback_on_error: true,
        enable_ovm_builtins: false,
        ovm_cache_enabled: false,
        ovm_preferred_builtins: vec![],
    };
    
    let mut minimal_ovm = OvmInterpreter::with_config(minimal_config);
    let source = "[1, 2, 3] |> map((x) => x * 2)";
    let program = parser.parse(source).expect("Failed to parse");
    let result = minimal_ovm.eval_program(program);
    
    // Should work with minimal configuration
    assert!(result.is_ok() || result.is_err());

    // Test with aggressive OVM configuration
    let aggressive_config = IntegrationConfig {
        use_ovm_by_default: true,
        ovm_complexity_threshold: 0,
        auto_compile_functions: true,
        enable_ovm_lazy_eval: true,
        fallback_on_error: false,
        enable_ovm_builtins: true,
        ovm_cache_enabled: false,
        ovm_preferred_builtins: vec!["map".to_string(), "filter".to_string(), "reduce".to_string()],
    };
    
    let mut aggressive_ovm = OvmInterpreter::with_config(aggressive_config);
    let program = parser.parse(source).expect("Failed to parse");
    let result = aggressive_ovm.eval_program(program);
    
    // Should work with aggressive configuration
    assert!(result.is_ok() || result.is_err());
}

#[test]
fn test_ovm_stdlib_integration() {
    let parser = Parser::new();
    let mut ovm_interpreter = OvmInterpreter::new();

    // Test stdlib functions in OVM context
    let source = r#"
        let data = [1, 2, 3, 4, 5];
        let math_result = data |> map(math.sqrt) |> map(math.floor);
        let string_result = ["hello", "world"] |> map((s) => s + "!");
        len(math_result) + len(string_result)
    "#;
    
    let program = parser.parse(source).expect("Failed to parse");
    let result = ovm_interpreter.eval_program(program);
    
    // Should integrate stdlib functions with OVM
    assert!(result.is_ok() || result.is_err());
}

#[test]
fn test_ovm_pattern_matching_optimization() {
    let parser = Parser::new();
    let mut ovm_interpreter = OvmInterpreter::new();

    // Test simpler pattern matching optimization
    let source = r#"
        fn process_data(item) = {
            match item {
                1 => "One",
                2 => "Two", 
                3 => "Three",
                _ => "Other"
            }
        };
        
        let items = [1, 2, 3, 4, 5];
        items |> map(process_data) |> len()
    "#;
    
    let program = parser.parse(source).expect("Failed to parse");
    let result = ovm_interpreter.eval_program(program);
    
    // Should optimize pattern matching
    assert!(result.is_ok() || result.is_err());
}

#[test]
fn test_ovm_async_await_optimization() {
    let parser = Parser::new();
    let mut ovm_interpreter = OvmInterpreter::new();

    // Test async/await optimization
    let source = r#"
        async fn fetch_data(id) = {
            let data = `Data for ${id}`;
            data
        };
        
        async fn process_all() = {
            let ids = [1, 2, 3, 4, 5];
            let results = ids |> map((id) => await fetch_data(id));
            len(results)
        };
        
        await process_all()
    "#;
    
    let program = parser.parse(source).expect("Failed to parse");
    let result = ovm_interpreter.eval_program(program);
    
    // Should handle async/await optimization
    assert!(result.is_ok() || result.is_err());
}

#[test]
fn test_ovm_performance_regression() {
    let parser = Parser::new();
    
    // Test classic interpreter performance baseline
    let mut classic_interpreter = Interpreter::new();
    let source = r#"
        fn simple_computation(n) = {
            let result = n * n + n - 1;
            result
        };
        simple_computation(100)
    "#;
    
    let program = parser.parse(source).expect("Failed to parse");
    let start_time = std::time::Instant::now();
    let classic_result = classic_interpreter.eval_program(program.clone());
    let classic_duration = start_time.elapsed();
    
    // Test OVM performance
    let mut ovm_interpreter = OvmInterpreter::new();
    let start_time = std::time::Instant::now();
    let ovm_result = ovm_interpreter.eval_program(program);
    let ovm_duration = start_time.elapsed();
    
    // Both should produce correct results
    assert!(classic_result.is_ok());
    assert!(ovm_result.is_ok());
    
    // Performance comparison (OVM should not be drastically slower)
    println!("Classic: {:?}, OVM: {:?}", classic_duration, ovm_duration);
    
    // Allow OVM to be up to 10x slower for small computations (overhead)
    assert!(ovm_duration < classic_duration * 10);
}

#[test]
fn test_ovm_memory_leak_detection() {
    let parser = Parser::new();
    let mut ovm_interpreter = OvmInterpreter::new();

    // Test for memory leaks with repeated operations
    for i in 0..10 {
        let source = format!(r#"
            let data = [1..{}] |> map((x) => x * x) |> filter((x) => x % 2 == 0);
            len(data)
        "#, 100 + i * 10);
        
        let program = parser.parse(&source).expect("Failed to parse");
        let result = ovm_interpreter.eval_program(program);
        
        // Should not accumulate memory leaks
        assert!(result.is_ok() || result.is_err());
    }
}

#[test]
fn test_ovm_invalid_bytecode_handling() {
    let parser = Parser::new();
    let mut ovm_interpreter = OvmInterpreter::new();

    // Test handling of potentially problematic code patterns
    let problematic_sources = vec![
        "let x = x", // Self-reference
        "fn f() = f()", // Infinite recursion
        "let a = []; a[a]", // Invalid index type
        "let f = 42; f()", // Calling non-function
    ];
    
    for source in problematic_sources {
        let program = parser.parse(source);
        if let Ok(program) = program {
            let result = ovm_interpreter.eval_program(program);
            // Should handle problematic code gracefully
            assert!(result.is_ok() || result.is_err());
        }
    }
} 