//! The byte-offset and one-pass `str` functions (stdlib/text.rs) and
//! the text readers beside them. Every case runs three ways and must
//! agree: the interpreter, the bytecode VM (`main` compiled directly,
//! which must succeed) and the tiered interpreter at threshold 1.

#![allow(dead_code)]

use olang::ast::{Statement, Value};
use olang::ovm::FunctionId;
use olang::ovm::OvmValue;
use olang::ovm::bytecode::BytecodeVm;
use olang::{Interpreter, Parser};

fn with_big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(f)
        .expect("spawn")
        .join()
        .expect("join")
}

fn eval(source: &str, tier: Option<u32>) -> Result<Value, String> {
    let source = source.to_string();
    with_big_stack(move || {
        let program = Parser::new().parse(&source).map_err(|e| e.to_string())?;
        let mut interpreter = Interpreter::new();
        if let Some(threshold) = tier {
            interpreter.enable_bytecode_tier(threshold, false);
        }
        interpreter.eval_program(program).map_err(|e| e.to_string())
    })
}

/// `main()` compiled on the bytecode VM with every function of the
/// program; refusing to compile is a failure of the test.
fn on_vm(source: &str) -> Result<Value, String> {
    let source = source.to_string();
    with_big_stack(move || {
        let program = Parser::new().parse(&source).map_err(|e| e.to_string())?;
        let mut vm = BytecodeVm::new();
        let mut ids = Vec::new();
        let mut main = None;
        for statement in &program.statements {
            if let Statement::FunctionDecl(func) = statement.unwrapped() {
                let id = FunctionId::new();
                vm.register_function(func.name.clone(), id);
                if func.name == "main" {
                    main = Some(id);
                }
                ids.push((id, func.clone()));
            }
        }
        for (id, func) in &ids {
            if let Err(e) = vm.compile_function(*id, func) {
                panic!("`{}` refused by the bytecode tier: {}", func.name, e);
            }
        }
        let main = main.expect("a main function");
        vm.execute(main, &[] as &[OvmValue])
            .map_err(|e| e.to_string())
            .and_then(|v| v.into_ast().map_err(|e| format!("{:?}", e)))
    })
}

/// The interpreter's answer, which the tiered run must give too, message
/// for message. For programs that need the interpreter's environment
/// (modules, declared types, traits) to compile.
fn agree_tiered(source: &str) -> Result<Value, String> {
    let interpreted = eval(source, None);
    let tiered = eval(source, Some(1));
    assert_eq!(
        tiered, interpreted,
        "tiered diverged from the interpreter\n{source}"
    );
    interpreted
}

/// The interpreter's answer, which the VM and the tiered run must give
/// too. The program ends with a call to `main()`.
fn agree(source: &str) -> Result<Value, String> {
    let interpreted = agree_tiered(source);
    // The VM run is of `main` alone; its error text may name less
    // context, so errors are compared as errors.
    let vm = on_vm(source);
    match (&interpreted, &vm) {
        (Ok(a), Ok(b)) => assert_eq!(a, b, "the VM diverged\n{source}"),
        (Err(_), Err(_)) => {}
        _ => panic!("the VM diverged: {interpreted:?} vs {vm:?}\n{source}"),
    }
    interpreted
}

fn s(text: &str) -> Value {
    Value::String(std::sync::Arc::new(text.to_string()))
}


fn ok(source: &str) -> Value {
    agree(source).unwrap_or_else(|e| panic!("{e}\n{source}"))
}

fn ints(xs: &[i64]) -> Value {
    Value::List(std::sync::Arc::new(xs.iter().map(|x| Value::Integer(*x)).collect()))
}

fn strs(xs: &[&str]) -> Value {
    Value::List(std::sync::Arc::new(xs.iter().map(|x| s(x)).collect()))
}

// ── OL-432: byte offsets ────────────────────────────────────────────
#[test]
fn byte_offsets_convert_and_slice() {
    let src = r#"
fn main() = {
    let t = "naïve café"
    [str.byte_len(t), str.length(t), str.char_to_byte(t, 3), str.byte_to_char(t, 4),
     str.char_to_byte(t, 99), str.byte_to_char(t, 99), str.utf16_len("a😀b"), str.byte_len("")]
}
main()
"#;
    assert_eq!(ok(src), ints(&[12, 10, 4, 3, 12, 10, 4, 0]));
    let src = r#"
fn main() = [str.slice_bytes("naïve", 0, 4), str.slice_bytes("naïve", 4, 99), str.slice_bytes("abc", 2, 1)]
main()
"#;
    assert_eq!(ok(src), strs(&["naï", "ve", ""]));
}

#[test]
fn a_byte_offset_inside_a_character_raises() {
    let err = agree(r#"
fn main() = str.slice_bytes("naïve", 0, 3)
main()
"#)
    .unwrap_err();
    assert!(err.contains("byte 3 is inside the character \"ï\""), "{err}");
    let err = agree("fn main() = str.byte_to_char(\"é\", 1)\nmain()\n").unwrap_err();
    assert!(err.contains("inside the character"), "{err}");
    let err = agree("fn main() = str.slice_bytes(\"abc\", -1, 2)\nmain()\n").unwrap_err();
    assert!(err.contains("negative"), "{err}");
}

#[test]
fn chunks_cut_between_characters_and_prefer_line_ends() {
    let src = r#"
fn main() = [str.chunks("ab\ncd\nef", 5, #{"prefer": "\n"}), str.chunks("ab\ncd\nefgh", 5, #{"prefer": "\n"}), str.chunks("ééé", 3), str.chunks("😀x", 2), str.chunks("", 4)]
main()
"#;
    let v = ok(src);
    assert_eq!(
        v,
        Value::List(std::sync::Arc::new(vec![
            strs(&["ab\n", "cd\nef"]),
            strs(&["ab\n", "cd\n", "efgh"]),
            strs(&["é", "é", "é"]),
            strs(&["😀", "x"]),
            strs(&[]),
        ]))
    );
    let err = agree("fn main() = str.chunks(\"abc\", 0)\nmain()\n").unwrap_err();
    assert!(err.contains("at least 1"), "{err}");
    let err = agree("fn main() = str.chunks(\"abc\", 2, #{\"perfer\": \"x\"})\nmain()\n").unwrap_err();
    assert!(err.contains("unknown option"), "{err}");
}

#[test]
fn chunks_join_back_to_the_text() {
    let src = r#"
fn widest(ps, i, most) = if i == len(ps) => most else => widest(ps, i + 1, max(most, str.byte_len(ps[i])))
fn main() = {
    let t = str.repeat("línea uno\nline two 😀\n", 500)
    let pieces = str.chunks(t, 64, #{"prefer": "\n"})
    let small = widest(pieces, 0, 0) <= 64
    [str.join(pieces, "") == t, small, len(pieces) > 100]
}
main()
"#;
    assert_eq!(
        ok(src),
        Value::List(std::sync::Arc::new(vec![Value::Boolean(true); 3]))
    );
}

