use olang::ast::*;
use olang::type_checker::{TypeChecker, TypeClass};

// =============================================================================
// TYPE CHECKING CORNER CASES TESTS
// =============================================================================

#[test]
fn test_complex_union_type_compatibility() {
    let type_checker = TypeChecker::new();

    // Test nested union types
    let inner_union = TypeAnnotation::Union {
        types: vec![TypeAnnotation::Int, TypeAnnotation::Float],
    };
    let outer_union = TypeAnnotation::Union {
        types: vec![inner_union, TypeAnnotation::String],
    };

    // Int should be compatible with nested union
    assert!(type_checker.types_compatible(&TypeAnnotation::Int, &outer_union));

    // Float should be compatible with nested union
    assert!(type_checker.types_compatible(&TypeAnnotation::Float, &outer_union));

    // String should be compatible with outer union
    assert!(type_checker.types_compatible(&TypeAnnotation::String, &outer_union));

    // Bool should NOT be compatible
    assert!(!type_checker.types_compatible(&TypeAnnotation::Bool, &outer_union));
}

#[test]
fn test_intersection_type_edge_cases() {
    let type_checker = TypeChecker::new();

    // Test intersection with same type
    let same_type_intersection = TypeAnnotation::Intersection {
        types: vec![TypeAnnotation::Int, TypeAnnotation::Int],
    };

    // Int should be compatible with Int & Int
    assert!(type_checker.types_compatible(&TypeAnnotation::Int, &same_type_intersection));

    // Test impossible intersection
    let impossible_intersection = TypeAnnotation::Intersection {
        types: vec![TypeAnnotation::Int, TypeAnnotation::String],
    };

    // No type should be compatible with Int & String
    assert!(!type_checker.types_compatible(&TypeAnnotation::Int, &impossible_intersection));
    assert!(!type_checker.types_compatible(&TypeAnnotation::String, &impossible_intersection));
}

#[test]
fn test_literal_type_edge_cases() {
    let type_checker = TypeChecker::new();

    // Test literal type with actual value
    let literal_42 = TypeAnnotation::Literal {
        value: Box::new(Value::Integer(42)),
    };

    // Should be compatible with Int
    assert!(type_checker.types_compatible(&literal_42, &TypeAnnotation::Int));
    assert!(type_checker.types_compatible(&TypeAnnotation::Int, &literal_42));

    // Test string literal
    let literal_hello = TypeAnnotation::Literal {
        value: Box::new(Value::String("hello".to_string().into())),
    };

    // Should be compatible with String
    assert!(type_checker.types_compatible(&literal_hello, &TypeAnnotation::String));
    assert!(type_checker.types_compatible(&TypeAnnotation::String, &literal_hello));
}

#[test]
fn test_generic_type_compatibility() {
    let type_checker = TypeChecker::new();

    // Test generic list types
    let list_int = TypeAnnotation::Generic {
        base_type: "List".to_string(),
        type_args: vec![TypeAnnotation::Int],
    };

    let list_float = TypeAnnotation::Generic {
        base_type: "List".to_string(),
        type_args: vec![TypeAnnotation::Float],
    };

    // List<Int> should not be compatible with List<Float>
    assert!(!type_checker.types_compatible(&list_int, &list_float));

    // Test generic with multiple type arguments
    let map_string_int = TypeAnnotation::Generic {
        base_type: "Map".to_string(),
        type_args: vec![TypeAnnotation::String, TypeAnnotation::Int],
    };

    let map_string_float = TypeAnnotation::Generic {
        base_type: "Map".to_string(),
        type_args: vec![TypeAnnotation::String, TypeAnnotation::Float],
    };

    // Map<String, Int> should not be compatible with Map<String, Float>
    assert!(!type_checker.types_compatible(&map_string_int, &map_string_float));
}

#[test]
fn test_type_variable_constraints() {
    let type_checker = TypeChecker::new();

    // Test unconstrained type variable
    let type_var = TypeAnnotation::TypeVariable("T".to_string());

    // Should be compatible with any type
    assert!(type_checker.types_compatible(&type_var, &TypeAnnotation::Int));
    assert!(type_checker.types_compatible(&type_var, &TypeAnnotation::String));
    assert!(type_checker.types_compatible(&TypeAnnotation::Int, &type_var));

    // Test constrained type variable (simulated by using separate type checker)
    let constrained_checker = TypeChecker::new();

    // Should be compatible with any type for unconstrained variable
    assert!(constrained_checker.types_compatible(&type_var, &TypeAnnotation::Int));
    assert!(constrained_checker.types_compatible(&type_var, &TypeAnnotation::String));
}

#[test]
fn test_function_type_compatibility() {
    let type_checker = TypeChecker::new();

    // Test function types with same signature
    let func1 = TypeAnnotation::Function {
        params: vec![TypeAnnotation::Int, TypeAnnotation::String],
        return_type: Box::new(TypeAnnotation::Bool),
    };

    let func2 = TypeAnnotation::Function {
        params: vec![TypeAnnotation::Int, TypeAnnotation::String],
        return_type: Box::new(TypeAnnotation::Bool),
    };

    // Should be compatible
    assert!(type_checker.types_compatible(&func1, &func2));

    // Test function with different parameter types
    let func3 = TypeAnnotation::Function {
        params: vec![TypeAnnotation::Float, TypeAnnotation::String],
        return_type: Box::new(TypeAnnotation::Bool),
    };

    // Should not be compatible
    assert!(!type_checker.types_compatible(&func1, &func3));
}

#[test]
fn test_recursive_type_compatibility() {
    let type_checker = TypeChecker::new();

    // Test list of lists
    let list_of_lists = TypeAnnotation::List(Box::new(TypeAnnotation::List(Box::new(
        TypeAnnotation::Int,
    ))));

    // Should be compatible with itself
    assert!(type_checker.types_compatible(&list_of_lists, &list_of_lists));

    // Test tuple with recursive structure
    let complex_tuple = TypeAnnotation::Tuple(vec![
        TypeAnnotation::Int,
        TypeAnnotation::List(Box::new(TypeAnnotation::String)),
        TypeAnnotation::Map {
            key_type: Box::new(TypeAnnotation::String),
            value_type: Box::new(TypeAnnotation::Int),
        },
    ]);

    // Should be compatible with itself
    assert!(type_checker.types_compatible(&complex_tuple, &complex_tuple));
}

#[test]
fn test_result_type_compatibility() {
    let type_checker = TypeChecker::new();

    // Test Result types
    let result_int_string = TypeAnnotation::Result {
        ok_type: Box::new(TypeAnnotation::Int),
        err_type: Box::new(TypeAnnotation::String),
    };

    let result_int_bool = TypeAnnotation::Result {
        ok_type: Box::new(TypeAnnotation::Int),
        err_type: Box::new(TypeAnnotation::Bool),
    };

    // Different error types should not be compatible
    assert!(!type_checker.types_compatible(&result_int_string, &result_int_bool));

    // Test nested Result types
    let nested_result = TypeAnnotation::Result {
        ok_type: Box::new(result_int_string.clone()),
        err_type: Box::new(TypeAnnotation::String),
    };

    // Should be compatible with itself
    assert!(type_checker.types_compatible(&nested_result, &nested_result));
}

#[test]
fn test_promise_type_compatibility() {
    let type_checker = TypeChecker::new();

    // Test Promise types
    let promise_int = TypeAnnotation::Promise {
        value_type: Box::new(TypeAnnotation::Int),
        error_type: Some(Box::new(TypeAnnotation::String)),
    };

    let promise_float = TypeAnnotation::Promise {
        value_type: Box::new(TypeAnnotation::Float),
        error_type: Some(Box::new(TypeAnnotation::String)),
    };

    // Different value types should not be compatible
    assert!(!type_checker.types_compatible(&promise_int, &promise_float));

    // Test Promise without error type
    let promise_no_error = TypeAnnotation::Promise {
        value_type: Box::new(TypeAnnotation::Int),
        error_type: None,
    };

    // Should not be compatible with Promise that has error type
    assert!(!type_checker.types_compatible(&promise_int, &promise_no_error));
}

#[test]
fn test_custom_type_compatibility() {
    let type_checker = TypeChecker::new();

    // Test custom types
    let custom_user = TypeAnnotation::Custom("User".to_string());
    let custom_product = TypeAnnotation::Custom("Product".to_string());

    // Different custom types should not be compatible
    assert!(!type_checker.types_compatible(&custom_user, &custom_product));

    // Same custom type should be compatible
    assert!(type_checker.types_compatible(&custom_user, &custom_user));
}

#[test]
fn test_type_class_constraints() {
    let type_checker = TypeChecker::new();

    // Test Numeric constraint
    assert!(type_checker.check_type_constraint(&TypeAnnotation::Int, &TypeClass::Numeric));
    assert!(type_checker.check_type_constraint(&TypeAnnotation::Float, &TypeClass::Numeric));
    assert!(!type_checker.check_type_constraint(&TypeAnnotation::String, &TypeClass::Numeric));

    // Test Comparable constraint
    assert!(type_checker.check_type_constraint(&TypeAnnotation::Int, &TypeClass::Comparable));
    assert!(type_checker.check_type_constraint(&TypeAnnotation::String, &TypeClass::Comparable));
    assert!(type_checker.check_type_constraint(&TypeAnnotation::Bool, &TypeClass::Comparable));

    // Test Iterable constraint
    assert!(type_checker.check_type_constraint(
        &TypeAnnotation::List(Box::new(TypeAnnotation::Int)),
        &TypeClass::Iterable
    ));
    assert!(type_checker.check_type_constraint(&TypeAnnotation::String, &TypeClass::Iterable));
    assert!(!type_checker.check_type_constraint(&TypeAnnotation::Int, &TypeClass::Iterable));

    // Test Hashable constraint
    assert!(type_checker.check_type_constraint(&TypeAnnotation::Int, &TypeClass::Hashable));
    assert!(type_checker.check_type_constraint(&TypeAnnotation::String, &TypeClass::Hashable));
    assert!(!type_checker.check_type_constraint(
        &TypeAnnotation::List(Box::new(TypeAnnotation::Int)),
        &TypeClass::Hashable
    ));
}

#[test]
fn test_union_type_creation_and_simplification() {
    let type_checker = TypeChecker::new();

    // Test union type creation with duplicates
    let union_with_duplicates = type_checker.create_union_type(vec![
        TypeAnnotation::Int,
        TypeAnnotation::String,
        TypeAnnotation::Int, // Duplicate
        TypeAnnotation::Float,
    ]);

    // Should remove duplicates
    match union_with_duplicates {
        TypeAnnotation::Union { types } => {
            assert_eq!(types.len(), 3); // Should have 3 unique types
        }
        _ => panic!("Expected Union type"),
    }

    // Test union type creation with single type
    let single_type_union = type_checker.create_union_type(vec![TypeAnnotation::Int]);

    // Should simplify to single type
    assert_eq!(single_type_union, TypeAnnotation::Int);

    // Test empty union
    let empty_union = type_checker.create_union_type(vec![]);
    assert_eq!(empty_union, TypeAnnotation::Unknown);
}

#[test]
fn test_intersection_type_creation_and_simplification() {
    let type_checker = TypeChecker::new();

    // Test intersection type creation with duplicates
    let intersection_with_duplicates = type_checker.create_intersection_type(vec![
        TypeAnnotation::Int,
        TypeAnnotation::Int, // Duplicate
        TypeAnnotation::Float,
    ]);

    // Should remove duplicates
    match intersection_with_duplicates {
        TypeAnnotation::Intersection { types } => {
            assert_eq!(types.len(), 2); // Should have 2 unique types
        }
        _ => panic!("Expected Intersection type"),
    }

    // Test intersection type creation with single type
    let single_type_intersection = type_checker.create_intersection_type(vec![TypeAnnotation::Int]);

    // Should simplify to single type
    assert_eq!(single_type_intersection, TypeAnnotation::Int);

    // Test empty intersection
    let empty_intersection = type_checker.create_intersection_type(vec![]);
    assert_eq!(empty_intersection, TypeAnnotation::Unknown);
}

#[test]
fn test_complex_nested_types() {
    let type_checker = TypeChecker::new();

    // Test deeply nested type structure
    let complex_type = TypeAnnotation::Function {
        params: vec![
            TypeAnnotation::List(Box::new(TypeAnnotation::Union {
                types: vec![TypeAnnotation::Int, TypeAnnotation::String],
            })),
            TypeAnnotation::Map {
                key_type: Box::new(TypeAnnotation::String),
                value_type: Box::new(TypeAnnotation::Result {
                    ok_type: Box::new(TypeAnnotation::Float),
                    err_type: Box::new(TypeAnnotation::String),
                }),
            },
        ],
        return_type: Box::new(TypeAnnotation::Promise {
            value_type: Box::new(TypeAnnotation::List(Box::new(TypeAnnotation::Int))),
            error_type: Some(Box::new(TypeAnnotation::String)),
        }),
    };

    // Should be compatible with itself
    assert!(type_checker.types_compatible(&complex_type, &complex_type));

    // Test with slightly different nested structure
    let similar_type = TypeAnnotation::Function {
        params: vec![
            TypeAnnotation::List(Box::new(TypeAnnotation::Union {
                types: vec![TypeAnnotation::Int, TypeAnnotation::Bool], // Different union
            })),
            TypeAnnotation::Map {
                key_type: Box::new(TypeAnnotation::String),
                value_type: Box::new(TypeAnnotation::Result {
                    ok_type: Box::new(TypeAnnotation::Float),
                    err_type: Box::new(TypeAnnotation::String),
                }),
            },
        ],
        return_type: Box::new(TypeAnnotation::Promise {
            value_type: Box::new(TypeAnnotation::List(Box::new(TypeAnnotation::Int))),
            error_type: Some(Box::new(TypeAnnotation::String)),
        }),
    };

    // Should not be compatible due to different union types
    assert!(!type_checker.types_compatible(&complex_type, &similar_type));
}

#[test]
fn test_type_inference_edge_cases() {
    let mut type_checker = TypeChecker::new();

    // Test empty list type inference through expression
    let empty_list_expr = Expr::List([].into());
    let list_type = type_checker.infer_type(&empty_list_expr);

    assert!(list_type.is_ok());
    match list_type.unwrap() {
        TypeAnnotation::List(inner) => {
            assert_eq!(*inner, TypeAnnotation::Unknown);
        }
        _ => panic!("Expected List type"),
    }

    // Test heterogeneous list type inference through expression
    let mixed_list_expr = Expr::List(
        [
            Expr::Integer(42),
            Expr::String("hello".to_string().into()),
            Expr::Boolean(true),
        ]
        .into(),
    );

    let mixed_type = type_checker.infer_type(&mixed_list_expr);
    assert!(mixed_type.is_ok());

    match mixed_type.unwrap() {
        TypeAnnotation::List(inner) => {
            // Should create a union type for mixed elements or use Unknown
            // Type inference may simplify to Unknown for mixed types
            assert!(!matches!(*inner, TypeAnnotation::Int)); // Should not be just Int
        }
        _ => panic!("Expected List type"),
    }
}

#[test]
fn test_map_type_inference_edge_cases() {
    let mut type_checker = TypeChecker::new();

    // Test empty map type inference through expression
    let empty_map_expr = Expr::MapLiteral { entries: vec![] };
    let map_type = type_checker.infer_type(&empty_map_expr);

    assert!(map_type.is_ok());
    match map_type.unwrap() {
        TypeAnnotation::Map {
            key_type,
            value_type,
        } => {
            assert_eq!(*key_type, TypeAnnotation::String);
            assert_eq!(*value_type, TypeAnnotation::Unknown);
        }
        _ => panic!("Expected Map type"),
    }

    // Test map with heterogeneous values through expression
    let mixed_map_expr = Expr::MapLiteral {
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

    let mixed_map_type = type_checker.infer_type(&mixed_map_expr);
    assert!(mixed_map_type.is_ok());

    match mixed_map_type.unwrap() {
        TypeAnnotation::Map {
            key_type,
            value_type,
        } => {
            assert_eq!(*key_type, TypeAnnotation::String);
            // Type inference may create union or simplify to Unknown
            assert!(!matches!(*value_type, TypeAnnotation::Int)); // Should not be just Int
        }
        _ => panic!("Expected Map type"),
    }
}

#[test]
fn test_range_type_compatibility() {
    let type_checker = TypeChecker::new();

    // Test range types
    let int_range = TypeAnnotation::Range {
        start: Box::new(TypeAnnotation::Int),
        end: Box::new(TypeAnnotation::Int),
        inclusive: true,
    };

    let float_range = TypeAnnotation::Range {
        start: Box::new(TypeAnnotation::Float),
        end: Box::new(TypeAnnotation::Float),
        inclusive: true,
    };

    // Different range types should not be compatible
    assert!(!type_checker.types_compatible(&int_range, &float_range));

    // Same range types should be compatible
    assert!(type_checker.types_compatible(&int_range, &int_range));

    // Test range with different inclusiveness
    let exclusive_range = TypeAnnotation::Range {
        start: Box::new(TypeAnnotation::Int),
        end: Box::new(TypeAnnotation::Int),
        inclusive: false,
    };

    // Should be compatible regardless of inclusiveness
    assert!(type_checker.types_compatible(&int_range, &exclusive_range));
}

#[test]
fn test_constraint_violation_detection() {
    let type_checker = TypeChecker::new();

    // Test union type with constraint
    let numeric_union = TypeAnnotation::Union {
        types: vec![TypeAnnotation::Int, TypeAnnotation::Float],
    };

    // Should satisfy Numeric constraint
    assert!(type_checker.check_type_constraint(&numeric_union, &TypeClass::Numeric));

    // Test union with non-numeric types
    let mixed_union = TypeAnnotation::Union {
        types: vec![TypeAnnotation::Int, TypeAnnotation::String],
    };

    // Should NOT satisfy Numeric constraint
    assert!(!type_checker.check_type_constraint(&mixed_union, &TypeClass::Numeric));

    // Test intersection type with constraint
    let numeric_intersection = TypeAnnotation::Intersection {
        types: vec![TypeAnnotation::Int, TypeAnnotation::Int],
    };

    // Should satisfy Numeric constraint
    assert!(type_checker.check_type_constraint(&numeric_intersection, &TypeClass::Numeric));
}
