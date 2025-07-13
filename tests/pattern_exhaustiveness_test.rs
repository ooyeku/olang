use olang::analyze::Analyzer;
use olang::ast::{Pattern, Value};

#[test]
fn test_empty_patterns_not_exhaustive() {
    let analyzer = Analyzer::new();
    let patterns = vec![];
    
    let result = analyzer.check_pattern_exhaustiveness(&patterns).unwrap();
    assert!(!result, "Empty patterns should not be exhaustive");
}

#[test]
fn test_wildcard_pattern_exhaustive() {
    let analyzer = Analyzer::new();
    let patterns = vec![Pattern::Wildcard];
    
    let result = analyzer.check_pattern_exhaustiveness(&patterns).unwrap();
    assert!(result, "Wildcard pattern should be exhaustive");
}

#[test]
fn test_variable_pattern_exhaustive() {
    let analyzer = Analyzer::new();
    let patterns = vec![Pattern::Identifier("x".to_string())];
    
    let result = analyzer.check_pattern_exhaustiveness(&patterns).unwrap();
    assert!(result, "Variable pattern should be exhaustive");
}

#[test]
fn test_boolean_patterns_exhaustive() {
    let analyzer = Analyzer::new();
    let patterns = vec![
        Pattern::Literal(Value::Boolean(true)),
        Pattern::Literal(Value::Boolean(false)),
    ];
    
    let result = analyzer.check_pattern_exhaustiveness(&patterns).unwrap();
    assert!(result, "Both true and false patterns should be exhaustive");
}

#[test]
fn test_boolean_patterns_not_exhaustive_missing_true() {
    let analyzer = Analyzer::new();
    let patterns = vec![Pattern::Literal(Value::Boolean(false))];
    
    let result = analyzer.check_pattern_exhaustiveness(&patterns).unwrap();
    assert!(!result, "Only false pattern should not be exhaustive");
    
    let missing = analyzer.get_missing_patterns(&patterns);
    assert!(missing.contains(&"true".to_string()), "Should report missing true pattern");
}

#[test]
fn test_boolean_patterns_not_exhaustive_missing_false() {
    let analyzer = Analyzer::new();
    let patterns = vec![Pattern::Literal(Value::Boolean(true))];
    
    let result = analyzer.check_pattern_exhaustiveness(&patterns).unwrap();
    assert!(!result, "Only true pattern should not be exhaustive");
    
    let missing = analyzer.get_missing_patterns(&patterns);
    assert!(missing.contains(&"false".to_string()), "Should report missing false pattern");
}

#[test]
fn test_result_patterns_exhaustive() {
    let analyzer = Analyzer::new();
    let patterns = vec![
        Pattern::Ok(Box::new(Pattern::Wildcard)),
        Pattern::Err(Box::new(Pattern::Wildcard)),
    ];
    
    let result = analyzer.check_pattern_exhaustiveness(&patterns).unwrap();
    assert!(result, "Both Ok and Err patterns should be exhaustive");
}

#[test]
fn test_result_patterns_not_exhaustive_missing_ok() {
    let analyzer = Analyzer::new();
    let patterns = vec![Pattern::Err(Box::new(Pattern::Wildcard))];
    
    let result = analyzer.check_pattern_exhaustiveness(&patterns).unwrap();
    assert!(!result, "Only Err pattern should not be exhaustive");
    
    let missing = analyzer.get_missing_patterns(&patterns);
    assert!(missing.contains(&"Ok(_)".to_string()), "Should report missing Ok pattern");
}

#[test]
fn test_result_patterns_not_exhaustive_missing_err() {
    let analyzer = Analyzer::new();
    let patterns = vec![Pattern::Ok(Box::new(Pattern::Wildcard))];
    
    let result = analyzer.check_pattern_exhaustiveness(&patterns).unwrap();
    assert!(!result, "Only Ok pattern should not be exhaustive");
    
    let missing = analyzer.get_missing_patterns(&patterns);
    assert!(missing.contains(&"Err(_)".to_string()), "Should report missing Err pattern");
}

#[test]
fn test_literal_patterns_not_exhaustive() {
    let analyzer = Analyzer::new();
    let patterns = vec![
        Pattern::Literal(Value::Integer(1)),
        Pattern::Literal(Value::Integer(2)),
    ];
    
    let result = analyzer.check_pattern_exhaustiveness(&patterns).unwrap();
    assert!(!result, "Literal patterns without wildcard should not be exhaustive");
    
    let missing = analyzer.get_missing_patterns(&patterns);
    assert!(missing.contains(&"_ (wildcard pattern)".to_string()), "Should report missing wildcard pattern");
}

#[test]
fn test_or_pattern_with_boolean_exhaustive() {
    let analyzer = Analyzer::new();
    let patterns = vec![
        Pattern::Or {
            alternatives: vec![
                Pattern::Literal(Value::Boolean(true)),
                Pattern::Literal(Value::Boolean(false)),
            ],
        },
    ];
    
    let result = analyzer.check_pattern_exhaustiveness(&patterns).unwrap();
    assert!(result, "Or pattern covering both boolean values should be exhaustive");
}

#[test]
fn test_or_pattern_with_catch_all_exhaustive() {
    let analyzer = Analyzer::new();
    let patterns = vec![
        Pattern::Or {
            alternatives: vec![
                Pattern::Literal(Value::Integer(1)),
                Pattern::Wildcard,
            ],
        },
    ];
    
    let result = analyzer.check_pattern_exhaustiveness(&patterns).unwrap();
    assert!(result, "Or pattern with wildcard should be exhaustive");
}

#[test]
fn test_guarded_patterns_not_exhaustive() {
    let analyzer = Analyzer::new();
    let patterns = vec![
        Pattern::Guarded {
            pattern: Box::new(Pattern::Wildcard),
            guard: Box::new(olang::ast::Expr::Boolean(true)),
        },
    ];
    
    let result = analyzer.check_pattern_exhaustiveness(&patterns).unwrap();
    assert!(!result, "Guarded patterns should not be considered exhaustive");
}

#[test]
fn test_tuple_patterns_not_exhaustive() {
    let analyzer = Analyzer::new();
    let patterns = vec![
        Pattern::Tuple(vec![
            Pattern::Literal(Value::Integer(1)),
            Pattern::Literal(Value::Integer(2)),
        ]),
    ];
    
    let result = analyzer.check_pattern_exhaustiveness(&patterns).unwrap();
    assert!(!result, "Tuple patterns without wildcard should not be exhaustive");
    
    let missing = analyzer.get_missing_patterns(&patterns);
    assert!(missing.contains(&"_ (wildcard pattern)".to_string()), "Should report missing wildcard pattern");
}

#[test]
fn test_list_patterns_not_exhaustive() {
    let analyzer = Analyzer::new();
    let patterns = vec![
        Pattern::List {
            patterns: vec![Pattern::Literal(Value::Integer(1))],
            rest: None,
        },
    ];
    
    let result = analyzer.check_pattern_exhaustiveness(&patterns).unwrap();
    assert!(!result, "List patterns without wildcard should not be exhaustive");
    
    let missing = analyzer.get_missing_patterns(&patterns);
    assert!(missing.contains(&"_ (wildcard pattern)".to_string()), "Should report missing wildcard pattern");
}

#[test]
fn test_enum_patterns_not_exhaustive() {
    let analyzer = Analyzer::new();
    let patterns = vec![
        Pattern::EnumVariant {
            variant_name: "Red".to_string(),
            patterns: vec![],
        },
    ];
    
    let result = analyzer.check_pattern_exhaustiveness(&patterns).unwrap();
    assert!(!result, "Enum patterns without all variants should not be exhaustive");
    
    let missing = analyzer.get_missing_patterns(&patterns);
    assert!(missing.contains(&"_ (wildcard pattern or other enum variants)".to_string()), "Should report missing enum variants");
}

#[test]
fn test_mixed_patterns_not_exhaustive() {
    let analyzer = Analyzer::new();
    let patterns = vec![
        Pattern::Literal(Value::Boolean(true)),
        Pattern::Ok(Box::new(Pattern::Wildcard)),
        Pattern::Literal(Value::Integer(1)),
    ];
    
    let result = analyzer.check_pattern_exhaustiveness(&patterns).unwrap();
    assert!(!result, "Mixed patterns without wildcard should not be exhaustive");
    
    let missing = analyzer.get_missing_patterns(&patterns);
    assert!(missing.contains(&"_ (wildcard pattern)".to_string()), "Should report missing wildcard pattern");
}

#[test]
fn test_struct_patterns_not_exhaustive() {
    let analyzer = Analyzer::new();
    let patterns = vec![
        Pattern::Struct {
            type_name: "Point".to_string(),
            field_patterns: vec![
                ("x".to_string(), Pattern::Literal(Value::Integer(0))),
                ("y".to_string(), Pattern::Literal(Value::Integer(0))),
            ],
        },
    ];
    
    let result = analyzer.check_pattern_exhaustiveness(&patterns).unwrap();
    assert!(!result, "Struct patterns without wildcard should not be exhaustive");
    
    let missing = analyzer.get_missing_patterns(&patterns);
    assert!(missing.contains(&"_ (wildcard pattern)".to_string()), "Should report missing wildcard pattern");
}

#[test]
fn test_range_patterns_not_exhaustive() {
    let analyzer = Analyzer::new();
    let patterns = vec![
        Pattern::Range {
            start: Box::new(Pattern::Literal(Value::Integer(1))),
            end: Box::new(Pattern::Literal(Value::Integer(10))),
            inclusive: true,
        },
    ];
    
    let result = analyzer.check_pattern_exhaustiveness(&patterns).unwrap();
    assert!(!result, "Range patterns without wildcard should not be exhaustive");
    
    let missing = analyzer.get_missing_patterns(&patterns);
    assert!(missing.contains(&"_ (wildcard pattern)".to_string()), "Should report missing wildcard pattern");
}

#[test]
fn test_comprehensive_boolean_or_pattern_exhaustive() {
    let analyzer = Analyzer::new();
    let patterns = vec![
        Pattern::Literal(Value::Boolean(true)),
        Pattern::Or {
            alternatives: vec![
                Pattern::Literal(Value::Boolean(false)),
            ],
        },
    ];
    
    let result = analyzer.check_pattern_exhaustiveness(&patterns).unwrap();
    assert!(result, "Boolean patterns covering both true and false should be exhaustive");
}

#[test]
fn test_comprehensive_result_or_pattern_exhaustive() {
    let analyzer = Analyzer::new();
    let patterns = vec![
        Pattern::Ok(Box::new(Pattern::Wildcard)),
        Pattern::Or {
            alternatives: vec![
                Pattern::Err(Box::new(Pattern::Wildcard)),
            ],
        },
    ];
    
    let result = analyzer.check_pattern_exhaustiveness(&patterns).unwrap();
    assert!(result, "Result patterns covering both Ok and Err should be exhaustive");
}

#[test]
fn test_nested_or_patterns_exhaustive() {
    let analyzer = Analyzer::new();
    let patterns = vec![
        Pattern::Or {
            alternatives: vec![
                Pattern::Literal(Value::Boolean(true)),
                Pattern::Or {
                    alternatives: vec![
                        Pattern::Literal(Value::Boolean(false)),
                    ],
                },
            ],
        },
    ];
    
    let result = analyzer.check_pattern_exhaustiveness(&patterns).unwrap();
    assert!(result, "Nested or patterns covering both boolean values should be exhaustive");
}

#[test]
fn test_rest_patterns_not_exhaustive() {
    let analyzer = Analyzer::new();
    let patterns = vec![
        Pattern::Rest("rest".to_string()),
    ];
    
    let result = analyzer.check_pattern_exhaustiveness(&patterns).unwrap();
    assert!(!result, "Rest patterns alone should not be exhaustive");
    
    let missing = analyzer.get_missing_patterns(&patterns);
    assert!(missing.contains(&"_ (wildcard pattern)".to_string()), "Should report missing wildcard pattern");
}

#[test]
fn test_missing_patterns_with_catch_all() {
    let analyzer = Analyzer::new();
    let patterns = vec![
        Pattern::Literal(Value::Integer(1)),
        Pattern::Wildcard,
    ];
    
    let missing = analyzer.get_missing_patterns(&patterns);
    assert!(missing.is_empty(), "Should report no missing patterns when there's a catch-all");
}

#[test]
fn test_analyze_pattern_structure_boolean() {
    let analyzer = Analyzer::new();
    let patterns = vec![
        Pattern::Literal(Value::Boolean(true)),
        Pattern::Literal(Value::Boolean(false)),
    ];
    
    let result = analyzer.check_pattern_exhaustiveness(&patterns).unwrap();
    assert!(result, "Boolean pattern structure should be correctly analyzed");
}

#[test]
fn test_analyze_pattern_structure_result() {
    let analyzer = Analyzer::new();
    let patterns = vec![
        Pattern::Ok(Box::new(Pattern::Wildcard)),
        Pattern::Err(Box::new(Pattern::Wildcard)),
    ];
    
    let result = analyzer.check_pattern_exhaustiveness(&patterns).unwrap();
    assert!(result, "Result pattern structure should be correctly analyzed");
}

#[test]
fn test_analyze_pattern_structure_mixed() {
    let analyzer = Analyzer::new();
    let patterns = vec![
        Pattern::Literal(Value::Boolean(true)),
        Pattern::Ok(Box::new(Pattern::Wildcard)),
    ];
    
    let result = analyzer.check_pattern_exhaustiveness(&patterns).unwrap();
    assert!(!result, "Mixed pattern structure should not be exhaustive");
    
    let missing = analyzer.get_missing_patterns(&patterns);
    assert!(missing.contains(&"_ (wildcard pattern)".to_string()), "Should report missing wildcard pattern for mixed patterns");
} 