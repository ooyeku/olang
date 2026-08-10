//! Struct declarations are real: constructing a declared struct validates
//! the field-name set, an undeclared struct-literal name is an error, and a
//! field value whose runtime type does not match its declared annotation is
//! a type error. 0.29 consolidation item C1; field-type enforcement added
//! later.

use olang::{Interpreter, Parser, Value};

fn eval(src: &str) -> Result<Value, String> {
    let program = Parser::new().parse(src).map_err(|e| e.to_string())?;
    let mut interp = Interpreter::new();
    interp.eval_program(program).map_err(|e| e.to_string())
}

#[test]
fn declared_struct_with_exact_fields_constructs() {
    let src = r#"
type Point = struct { x: Int, y: Int }
let p = Point { x: 1, y: 2 }
p.x + p.y
"#;
    assert_eq!(eval(src).unwrap(), Value::Integer(3));
}

#[test]
fn missing_field_is_an_error_naming_it() {
    let src = r#"
type Point = struct { x: Int, y: Int }
Point { x: 1 }
"#;
    let err = eval(src).unwrap_err();
    assert!(err.contains("missing field 'y'"), "got: {err}");
}

#[test]
fn surprise_field_is_an_error_naming_it() {
    let src = r#"
type Point = struct { x: Int, y: Int }
Point { x: 1, y: 2, z: 3 }
"#;
    let err = eval(src).unwrap_err();
    assert!(err.contains("no field 'z'"), "got: {err}");
}

#[test]
fn undeclared_struct_type_is_an_error() {
    let err = eval("NeverDeclared { surprise: 42 }").unwrap_err();
    assert!(
        err.contains("unknown struct type 'NeverDeclared'"),
        "got: {err}"
    );
}

#[test]
fn anonymous_objects_remain_free_form() {
    let src = r#"
let o = { anything: 1, goes: "here" }
o.anything
"#;
    assert_eq!(eval(src).unwrap(), Value::Integer(1));
}

#[test]
fn mismatched_field_type_is_an_error() {
    // Declared field types are enforced: a value whose runtime type does not
    // match the field's annotation is a type error naming the field, the
    // struct, the expected type, and what was supplied.
    let src = r#"
type Point = struct { x: Int, y: Int }
Point { x: "dynamic", y: 2 }
"#;
    let err = eval(src).unwrap_err();
    assert!(
        err.contains("field 'x' of Point expects Int, got String"),
        "got: {err}"
    );
}

#[test]
fn matching_field_types_construct() {
    // The exact-type happy path still constructs.
    let src = r#"
type Mixed = struct { n: Int, f: Float, s: String, b: Bool }
let m = Mixed { n: 1, f: 2.5, s: "hi", b: true }
m.n
"#;
    assert_eq!(eval(src).unwrap(), Value::Integer(1));
}

#[test]
fn int_does_not_satisfy_a_float_field() {
    // Enforcement is strict: no Int->Float widening at construction.
    let src = r#"
type V = struct { x: Float }
V { x: 3 }
"#;
    let err = eval(src).unwrap_err();
    assert!(
        err.contains("field 'x' of V expects Float, got Int"),
        "got: {err}"
    );
}

#[test]
fn custom_struct_field_type_is_enforced() {
    // A field declared as another struct type must receive that struct.
    let good = r#"
type Point = struct { x: Int, y: Int }
type Line = struct { a: Point, b: Point }
let l = Line { a: Point { x: 0, y: 0 }, b: Point { x: 1, y: 1 } }
l.b.x
"#;
    assert_eq!(eval(good).unwrap(), Value::Integer(1));

    let bad = r#"
type Point = struct { x: Int, y: Int }
type Line = struct { a: Point, b: Point }
Line { a: 5, b: Point { x: 1, y: 1 } }
"#;
    let err = eval(bad).unwrap_err();
    assert!(
        err.contains("field 'a' of Line expects Point, got Int"),
        "got: {err}"
    );
}

#[test]
fn generic_field_parameter_is_not_checked() {
    // A field typed as one of the type's generic parameters has no runtime
    // identity to check against, so any value is accepted.
    let src = r#"
type Box<T> = struct { value: T }
let a = Box { value: 7 }
let b = Box { value: "seven" }
a.value
"#;
    assert_eq!(eval(src).unwrap(), Value::Integer(7));
}

#[test]
fn anonymous_object_fields_stay_free_form() {
    // Anonymous objects declare no field types, so nothing is enforced.
    let src = r#"
let o = { x: "anything", y: 2 }
o.x
"#;
    match eval(src).unwrap() {
        Value::String(s) => assert_eq!(s.to_string(), "anything"),
        other => panic!("expected string, got {:?}", other),
    }
}

#[test]
fn shared_struct_types_validate_in_the_importer() {
    // A struct declared in a module and imported validates at the
    // construction site in the importing file.
    let ws = std::env::temp_dir().join(format!("olang_structval_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&ws);
    std::fs::create_dir_all(ws.join("lib")).unwrap();
    std::fs::write(
        ws.join("olang.toml"),
        "[package]\nname = \"structval\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::write(
        ws.join("lib/shapes.ol"),
        "share type Rect = struct { w: Int, h: Int }\n",
    )
    .unwrap();

    let good = "use lib.shapes { Rect }\nlet r = Rect { w: 2, h: 3 }\nr.w * r.h";
    let bad = "use lib.shapes { Rect }\nRect { w: 2 }";

    let run = |src: &str| {
        let program = Parser::new().parse(src).map_err(|e| e.to_string())?;
        let mut interp = Interpreter::new();
        interp.set_current_file(&ws.join("main.ol"));
        interp.eval_program(program).map_err(|e| e.to_string())
    };
    assert_eq!(run(good).unwrap(), Value::Integer(6));
    let err = run(bad).unwrap_err();
    assert!(err.contains("missing field 'h'"), "got: {err}");
    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn pattern_matching_on_structs_is_unchanged() {
    let src = r#"
type Point = struct { x: Int, y: Int }
let p = Point { x: 5, y: 7 }
match p { Point { x, y } => x * y }
"#;
    assert_eq!(eval(src).unwrap(), Value::Integer(35));
}
