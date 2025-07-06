use olang::ast::*;
use olang::parser::Parser;
use olang::type_checker::{TypeChecker, TypeClass};

#[test]
fn test_union_type_compatibility() {
    let mut type_checker = TypeChecker::new();
    
    // Test union type creation
    let union_type = TypeAnnotation::Union {
        types: vec![TypeAnnotation::Int, TypeAnnotation::String],
    };
    
    // Int should be compatible with Int|String
    assert!(type_checker.types_compatible(&TypeAnnotation::Int, &union_type));
    
    // String should be compatible with Int|String
    assert!(type_checker.types_compatible(&TypeAnnotation::String, &union_type));
    
    // Bool should NOT be compatible with Int|String
    assert!(!type_checker.types_compatible(&TypeAnnotation::Bool, &union_type));
}

#[test]
fn test_intersection_type_compatibility() {
    let mut type_checker = TypeChecker::new();
    
    // Test intersection type creation (theoretical example)
    let intersection_type = TypeAnnotation::Intersection {
        types: vec![TypeAnnotation::Int, TypeAnnotation::Int], // Simplified for testing
    };
    
    // Int should be compatible with Int&Int
    assert!(type_checker.types_compatible(&TypeAnnotation::Int, &intersection_type));
    
    // String should NOT be compatible with Int&Int
    assert!(!type_checker.types_compatible(&TypeAnnotation::String, &intersection_type));
}

#[test]
fn test_enhanced_list_type_inference() {
    let parser = Parser::new();
    let mut type_checker = TypeChecker::new();
    
    // Test mixed type list creates union type
    let mixed_list = Expr::List(vec![
        Expr::Integer(42),
        Expr::String("hello".to_string().into()),
        Expr::Boolean(true),
    ].into());
    
    let result = type_checker.infer_type(&mixed_list);
    assert!(result.is_ok());
    
    let inferred_type = result.unwrap();
    match inferred_type {
        TypeAnnotation::List(element_type) => {
            match *element_type {
                TypeAnnotation::Union { types } => {
                    assert_eq!(types.len(), 3);
                    assert!(types.contains(&TypeAnnotation::Int));
                    assert!(types.contains(&TypeAnnotation::String));
                    assert!(types.contains(&TypeAnnotation::Bool));
                }
                _ => panic!("Expected Union type for mixed list elements"),
            }
        }
        _ => panic!("Expected List type"),
    }
}

#[test]
fn test_enhanced_map_type_inference() {
    let mut type_checker = TypeChecker::new();
    
    // Test mixed value map creates union type
    let mixed_map = Expr::MapLiteral {
        entries: vec![
            MapEntry {
                key: Expr::String("key1".to_string().into()),
                value: Expr::Integer(42),
            },
            MapEntry {
                key: Expr::String("key2".to_string().into()),
                value: Expr::String("value".to_string().into()),
            },
        ],
    };
    
    let result = type_checker.infer_type(&mixed_map);
    assert!(result.is_ok());
    
    let inferred_type = result.unwrap();
    match inferred_type {
        TypeAnnotation::Map { key_type, value_type } => {
            assert_eq!(*key_type, TypeAnnotation::String);
            match *value_type {
                TypeAnnotation::Union { types } => {
                    assert_eq!(types.len(), 2);
                    assert!(types.contains(&TypeAnnotation::Int));
                    assert!(types.contains(&TypeAnnotation::String));
                }
                _ => panic!("Expected Union type for mixed map values"),
            }
        }
        _ => panic!("Expected Map type"),
    }
}

#[test]
fn test_type_constraint_checking() {
    let type_checker = TypeChecker::new();
    
    // Test numeric constraint
    assert!(type_checker.check_type_constraint(&TypeAnnotation::Int, &TypeClass::Numeric));
    assert!(type_checker.check_type_constraint(&TypeAnnotation::Float, &TypeClass::Numeric));
    assert!(!type_checker.check_type_constraint(&TypeAnnotation::String, &TypeClass::Numeric));
    
    // Test comparable constraint
    assert!(type_checker.check_type_constraint(&TypeAnnotation::Int, &TypeClass::Comparable));
    assert!(type_checker.check_type_constraint(&TypeAnnotation::String, &TypeClass::Comparable));
    assert!(type_checker.check_type_constraint(&TypeAnnotation::Bool, &TypeClass::Comparable));
    
    // Test hashable constraint
    assert!(type_checker.check_type_constraint(&TypeAnnotation::Int, &TypeClass::Hashable));
    assert!(type_checker.check_type_constraint(&TypeAnnotation::String, &TypeClass::Hashable));
    assert!(type_checker.check_type_constraint(&TypeAnnotation::Bool, &TypeClass::Hashable));
    
    // Test iterable constraint
    assert!(type_checker.check_type_constraint(&TypeAnnotation::List(Box::new(TypeAnnotation::Int)), &TypeClass::Iterable));
    assert!(type_checker.check_type_constraint(&TypeAnnotation::String, &TypeClass::Iterable));
    assert!(!type_checker.check_type_constraint(&TypeAnnotation::Int, &TypeClass::Iterable));
}

#[test]
fn test_enhanced_error_messages() {
    let parser = Parser::new();
    let mut type_checker = TypeChecker::new();
    
    // Test type mismatch with suggestion
    let result = type_checker.check_type_compatibility(
        &TypeAnnotation::Int,
        &TypeAnnotation::Float,
        "test location"
    );
    
    assert!(result.is_err());
    
    let error = result.unwrap_err();
    match error {
        TypeError::TypeMismatch { location, .. } => {
            assert!(location.contains("suggestion"));
            assert!(location.contains("integer literal"));
        }
        _ => panic!("Expected TypeMismatch error"),
    }
}

#[test]
fn test_literal_type_compatibility() {
    let mut type_checker = TypeChecker::new();
    
    // Test literal type compatibility
    let literal_type = TypeAnnotation::Literal {
        value: Box::new(Value::Integer(42)),
    };
    
    // Literal should be compatible with its base type
    assert!(type_checker.types_compatible(&literal_type, &TypeAnnotation::Int));
    assert!(type_checker.types_compatible(&TypeAnnotation::Int, &literal_type));
    
    // Literal should NOT be compatible with other types
    assert!(!type_checker.types_compatible(&literal_type, &TypeAnnotation::String));
}

#[test]
fn test_result_type_compatibility() {
    let mut type_checker = TypeChecker::new();
    
    let result_type1 = TypeAnnotation::Result {
        ok_type: Box::new(TypeAnnotation::Int),
        err_type: Box::new(TypeAnnotation::String),
    };
    
    let result_type2 = TypeAnnotation::Result {
        ok_type: Box::new(TypeAnnotation::Int),
        err_type: Box::new(TypeAnnotation::String),
    };
    
    let result_type3 = TypeAnnotation::Result {
        ok_type: Box::new(TypeAnnotation::Float),
        err_type: Box::new(TypeAnnotation::String),
    };
    
    // Same Result types should be compatible
    assert!(type_checker.types_compatible(&result_type1, &result_type2));
    
    // Different Result types should not be compatible
    assert!(!type_checker.types_compatible(&result_type1, &result_type3));
}

#[test]
fn test_promise_type_compatibility() {
    let mut type_checker = TypeChecker::new();
    
    let promise_type1 = TypeAnnotation::Promise {
        value_type: Box::new(TypeAnnotation::Int),
        error_type: Some(Box::new(TypeAnnotation::String)),
    };
    
    let promise_type2 = TypeAnnotation::Promise {
        value_type: Box::new(TypeAnnotation::Int),
        error_type: Some(Box::new(TypeAnnotation::String)),
    };
    
    let promise_type3 = TypeAnnotation::Promise {
        value_type: Box::new(TypeAnnotation::Float),
        error_type: Some(Box::new(TypeAnnotation::String)),
    };
    
    // Same Promise types should be compatible
    assert!(type_checker.types_compatible(&promise_type1, &promise_type2));
    
    // Different Promise types should not be compatible
    assert!(!type_checker.types_compatible(&promise_type1, &promise_type3));
}

#[test]
fn test_function_type_compatibility() {
    let mut type_checker = TypeChecker::new();
    
    let func_type1 = TypeAnnotation::Function {
        params: vec![TypeAnnotation::Int, TypeAnnotation::String],
        return_type: Box::new(TypeAnnotation::Bool),
    };
    
    let func_type2 = TypeAnnotation::Function {
        params: vec![TypeAnnotation::Int, TypeAnnotation::String],
        return_type: Box::new(TypeAnnotation::Bool),
    };
    
    let func_type3 = TypeAnnotation::Function {
        params: vec![TypeAnnotation::Float, TypeAnnotation::String],
        return_type: Box::new(TypeAnnotation::Bool),
    };
    
    // Same function types should be compatible
    assert!(type_checker.types_compatible(&func_type1, &func_type2));
    
    // Different function types should not be compatible
    assert!(!type_checker.types_compatible(&func_type1, &func_type3));
}

#[test]
fn test_union_type_simplification() {
    let type_checker = TypeChecker::new();
    
    // Test empty union
    let empty_union = type_checker.create_union_type(vec![]);
    assert_eq!(empty_union, TypeAnnotation::Unknown);
    
    // Test single type union
    let single_union = type_checker.create_union_type(vec![TypeAnnotation::Int]);
    assert_eq!(single_union, TypeAnnotation::Int);
    
    // Test multiple type union
    let multi_union = type_checker.create_union_type(vec![
        TypeAnnotation::Int,
        TypeAnnotation::String,
        TypeAnnotation::Bool,
    ]);
    match multi_union {
        TypeAnnotation::Union { types } => {
            assert_eq!(types.len(), 3);
            assert!(types.contains(&TypeAnnotation::Int));
            assert!(types.contains(&TypeAnnotation::String));
            assert!(types.contains(&TypeAnnotation::Bool));
        }
        _ => panic!("Expected Union type"),
    }
}

#[test]
fn test_intersection_type_simplification() {
    let type_checker = TypeChecker::new();
    
    // Test empty intersection
    let empty_intersection = type_checker.create_intersection_type(vec![]);
    assert_eq!(empty_intersection, TypeAnnotation::Unknown);
    
    // Test single type intersection
    let single_intersection = type_checker.create_intersection_type(vec![TypeAnnotation::Int]);
    assert_eq!(single_intersection, TypeAnnotation::Int);
    
    // Test multiple type intersection
    let multi_intersection = type_checker.create_intersection_type(vec![
        TypeAnnotation::Int,
        TypeAnnotation::Int, // Duplicate should be removed
    ]);
    assert_eq!(multi_intersection, TypeAnnotation::Int);
}

#[test]
fn test_enhanced_builtin_function_types() {
    let type_checker = TypeChecker::new();
    
    // Test enhanced len function type
    let len_type = type_checker.get_context().functions.get("len").unwrap();
    match len_type {
        TypeAnnotation::Function { params, return_type } => {
            assert_eq!(params.len(), 1);
            assert_eq!(return_type.as_ref(), &TypeAnnotation::Int);
            
            // First parameter should be a union of iterable types
            match &params[0] {
                TypeAnnotation::Union { types } => {
                    assert!(types.len() >= 2); // At least List and String
                }
                _ => panic!("Expected Union type for len parameter"),
            }
        }
        _ => panic!("Expected Function type for len"),
    }
}

#[test]
fn test_map_key_hashable_constraint() {
    let mut type_checker = TypeChecker::new();
    
    // Test valid map with hashable keys
    let valid_map = Expr::MapLiteral {
        entries: vec![
            MapEntry {
                key: Expr::String("key1".to_string().into()),
                value: Expr::Integer(42),
            },
            MapEntry {
                key: Expr::Integer(123),
                value: Expr::String("value".to_string().into()),
            },
        ],
    };
    
    let result = type_checker.infer_type(&valid_map);
    assert!(result.is_ok());
    
    // Test invalid map with non-hashable keys (function)
    let invalid_map = Expr::MapLiteral {
        entries: vec![
            MapEntry {
                key: Expr::Lambda {
                    parameters: vec![],
                    body: Box::new(Expr::Integer(42)),
                    return_type: None,
                },
                value: Expr::Integer(42),
            },
        ],
    };
    
    let result = type_checker.infer_type(&invalid_map);
    assert!(result.is_err());
    
    match result.unwrap_err() {
        TypeError::InvalidOperation { op, .. } => {
            assert_eq!(op, "map key");
        }
        _ => panic!("Expected InvalidOperation error for non-hashable key"),
    }
}

#[test]
fn test_comprehensive_type_system_integration() {
    let parser = Parser::new();
    let mut type_checker = TypeChecker::new();
    
    // Test a complex expression using multiple enhanced features
    let complex_expr = Expr::Call {
        callee: Box::new(Expr::Identifier("map".to_string())),
        arguments: vec![
            Argument::Positional(Expr::List(vec![
                Expr::Integer(1),
                Expr::Integer(2),
                Expr::Integer(3),
            ].into())),
            Argument::Positional(Expr::Lambda {
                parameters: vec![Parameter {
                    name: "x".to_string(),
                    type_annotation: Some(TypeAnnotation::Int),
                    default_value: None,
                }],
                body: Box::new(Expr::BinaryOp {
                    left: Box::new(Expr::Identifier("x".to_string())),
                    op: BinaryOp::Multiply,
                    right: Box::new(Expr::Integer(2)),
                }),
                return_type: None,
            }),
        ],
    };
    
    let result = type_checker.infer_type(&complex_expr);
    assert!(result.is_ok());
    
    let inferred_type = result.unwrap();
    match inferred_type {
        TypeAnnotation::List(element_type) => {
            match *element_type {
                TypeAnnotation::TypeVariable(_) => {
                    // Should be inferred as Int through type variable substitution
                    // This is acceptable for now
                }
                TypeAnnotation::Int => {
                    // Perfect - fully inferred
                }
                _ => panic!("Expected Int or TypeVariable for list element type"),
            }
        }
        _ => panic!("Expected List type for map result"),
    }
} 