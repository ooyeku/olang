//! Freeing a deep value costs heap, not stack.
//!
//! Drop glue frees nested values by recursion, one frame per level, so a
//! long enough list built from an enum overflowed the stack when it was
//! freed — a process abort, not an error, and in an http worker's 32 MB
//! stack at a fraction of the depth the main thread survived. Both value
//! representations now free their nested containers with a work list.
//! Each test here builds a chain far deeper than the thread it drops on
//! could recurse through.

use std::collections::HashMap;
use std::sync::Arc;

use olang::ast::{EnumVariantData, Value};
use olang::interpreter::Interpreter;
use olang::ovm::OvmValue;
use olang::ovm::value::{EnumData, EnumObject, ResultObject, StructObject, ValueData};
use olang::parser::Parser;

const DEPTH: usize = 1_000_000;

/// Runs `f` on a thread whose stack a recursive drop of `DEPTH` levels
/// would overflow many times over.
fn on_small_stack(f: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap();
}

fn chain(link: impl Fn(Value) -> Value) -> Value {
    (0..DEPTH).fold(Value::Unit, |acc, _| link(acc))
}

fn ovm_chain(link: impl Fn(OvmValue) -> OvmValue) -> OvmValue {
    (0..DEPTH).fold(
        OvmValue {
            data: ValueData::Unit,
        },
        |acc, _| link(acc),
    )
}

#[test]
fn every_interpreter_container_frees_a_deep_chain() {
    on_small_stack(|| {
        drop(chain(|next| {
            Value::enum_of(
                "L".into(),
                "Cons".into(),
                EnumVariantData::Tuple(vec![Value::Integer(1), next]),
            )
        }));
        drop(chain(|next| {
            Value::enum_of(
                "L".into(),
                "Cons".into(),
                EnumVariantData::Struct(olang::ast::ValueMap::from_iter([(
                    "next".to_string(),
                    next,
                )])),
            )
        }));
        drop(chain(|next| {
            Value::List(Arc::new(vec![Value::Integer(1), next]))
        }));
        drop(chain(|next| Value::Tuple(Arc::new(vec![next]))));
        drop(chain(|next| {
            Value::Map(Arc::new(olang::ast::ValueMap::from_iter([(
                "next".to_string(),
                next,
            )])))
        }));
        drop(chain(|next| Value::Struct {
            type_name: "N".into(),
            fields: Arc::new(olang::ast::ValueMap::from_iter([(
                "next".to_string(),
                next,
            )])),
        }));
        drop(chain(|next| Value::Ok(Box::new(next))));
        drop(chain(|next| Value::Err(Box::new(next))));
    });
}

#[test]
fn every_vm_container_frees_a_deep_chain() {
    on_small_stack(|| {
        drop(ovm_chain(|next| OvmValue {
            data: ValueData::List(Arc::new(vec![next])),
        }));
        drop(ovm_chain(|next| OvmValue {
            data: ValueData::Tuple(Arc::new(vec![next])),
        }));
        drop(ovm_chain(|next| {
            let mut map = olang::ovm::value::OvmMap::default();
            map.insert("next".into(), next);
            OvmValue {
                data: ValueData::Map(Arc::new(map)),
            }
        }));
        drop(ovm_chain(|next| OvmValue {
            data: ValueData::Struct(Arc::new(StructObject::from_pairs(
                "N",
                vec![("next".into(), next)],
            ))),
        }));
        drop(ovm_chain(|next| OvmValue {
            data: ValueData::Enum(Arc::new(EnumObject {
                type_name: "L".into(),
                variant_name: "Cons".into(),
                data: EnumData::Tuple(vec![next]),
            })),
        }));
        drop(ovm_chain(|next| OvmValue {
            data: ValueData::Result(Arc::new(ResultObject {
                ok: Some(next),
                err: None,
            })),
        }));
    });
}

#[test]
fn a_shared_child_survives_its_parent() {
    on_small_stack(|| {
        let shared = chain(|next| Value::List(Arc::new(vec![next])));
        let parent = Value::List(Arc::new(vec![shared.clone()]));
        drop(parent);
        let Value::List(items) = &shared else {
            panic!("list")
        };
        assert_eq!(items.len(), 1);
        drop(shared);
    });
}

#[test]
fn a_program_built_list_frees_on_a_small_stack() {
    let src = "
        type L = enum { Cons(Int, L), Nil }
        let acc = cell(Nil)
        for i in 0..200000 { cell.set(acc, Cons(i, cell.get(acc))) }
        cell.get(acc)
    ";
    let program = Parser::new().parse(src).expect("parse");
    let value = Interpreter::new().eval_program(program).expect("eval");
    on_small_stack(move || drop(value));
}

// --- Crossing the tier boundary ---
//
// A value entering or leaving the bytecode tier is walked as deep as it
// is: `round_trips` asks whether it may cross, `from_ast` and `to_ast`
// convert it. Each recursed a frame per level and overflowed the stack
// for a deep enough value — the crash freeing had, one step earlier. They
// now grow the stack as they go, as the interpreter's own evaluation does.

#[test]
fn a_deep_value_crosses_the_tier_boundary_both_ways() {
    on_small_stack(|| {
        let enum_chain = chain(|next| {
            Value::enum_of(
                "L".into(),
                "Cons".into(),
                EnumVariantData::Tuple(vec![Value::Integer(1), next]),
            )
        });
        let list_chain = chain(|next| Value::List(Arc::new(vec![Value::Integer(1), next])));
        for value in [enum_chain, list_chain] {
            assert!(olang::ovm::bytecode::BytecodeVm::round_trips(&value));
            let back = OvmValue::from_ast(value.clone()).to_ast().expect("to_ast");
            assert!(matches!(
                (&value, &back),
                (Value::Enum(_), Value::Enum(_)) | (Value::List(_), Value::List(_))
            ));
        }
    });
}

// --- Printing and serializing ---
//
// `show` (the value's Display) and `json.stringify` walk a value as deep
// as it is too, and overflowed the stack for a million-deep list. They
// grow the stack as they go; stringify no longer builds a serde_json
// tree, which serde_json prints and frees by recursion of its own.

/// `json.stringify`'s answer: whether it was `Ok`, and the string inside.
fn stringify(value: Value) -> (bool, String) {
    let result =
        olang::stdlib::json::call_json_function("stringify", vec![value]).expect("stringify");
    let ok = matches!(result, Value::Ok(_));
    match result.into_payload() {
        Some(Value::String(ref s)) => (ok, s.to_string()),
        other => panic!("not a result of a string: {other:?}"),
    }
}

#[test]
fn a_deep_value_prints_and_serializes() {
    on_small_stack(|| {
        let list = chain(|next| Value::List(Arc::new(vec![Value::Integer(1), next])));
        let text = list.to_string();
        assert!(text.starts_with("[1, [1, [1, "));
        let (ok, json) = stringify(list);
        assert!(ok, "{json}");
        assert!(json.starts_with("[1,[1,[1,") && json.contains("[1,null]]]"));
        assert_eq!(json.matches('[').count(), DEPTH);

        let enum_chain = chain(|next| {
            Value::enum_of(
                "L".into(),
                "Cons".into(),
                EnumVariantData::Tuple(vec![Value::Integer(1), next]),
            )
        });
        assert!(enum_chain.to_string().starts_with("L.Cons(1, L.Cons(1, "));
        // An enum has no JSON form; the error names it in a short preview
        // rather than printing a million levels of it.
        let (ok, message) = stringify(enum_chain);
        assert!(!ok, "an enum serialized");
        assert!(
            message.contains("Cannot convert L.Cons(1, L.Cons(1, "),
            "{message}"
        );
        assert!(message.len() < 200, "{message}");
    });
}

#[test]
fn stringify_refuses_a_float_json_cannot_hold() {
    for f in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let (ok, message) = stringify(Value::List(Arc::new(vec![Value::Float(f)])));
        assert!(!ok, "{f} serialized");
        assert_eq!(
            message,
            format!("Cannot convert to JSON: Type error: Invalid float value: {f}")
        );
    }
}
