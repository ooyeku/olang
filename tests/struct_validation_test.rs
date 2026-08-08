//! Struct declarations are real: constructing a declared struct validates
//! the field-name set, and an undeclared struct-literal name is an error.
//! Field VALUES stay dynamic — olang declarations fix shape, not types.
//! 0.29 consolidation item C1.

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
fn field_values_are_not_type_checked() {
    // Dynamic typing is explicit: shape is validated, values are not.
    let src = r#"
type Point = struct { x: Int, y: Int }
let p = Point { x: "dynamic", y: 2 }
p.x
"#;
    match eval(src).unwrap() {
        Value::String(s) => assert_eq!(s.to_string(), "dynamic"),
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
