//! `olang fix guards | defaults | first-last | units`: each fixture's
//! rewrite is the one written down, a second run changes nothing, and
//! every rewritten form answers what its original answered — for `()`,
//! empty and ordinary inputs, and for a raise — on all three tiers: the
//! interpreter, the bytecode VM (each function compiled directly) and the
//! tiered interpreter at threshold 1 (bytecode promoted at the first call,
//! the JIT taking what it can).

use olang::ast::{Statement, Value};
use olang::ovm::FunctionId;
use olang::ovm::OvmValue;
use olang::ovm::bytecode::BytecodeVm;
use olang::tools::fix_idioms::{Target, fix};
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

/// `main()` on the bytecode VM, every function of the program compiled.
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

/// The three tiers' answers, which must agree (a raise with a raise).
fn on_every_tier(source: &str) -> Result<Value, String> {
    let interpreted = eval(source, None);
    let tiered = eval(source, Some(1));
    assert_eq!(tiered, interpreted, "the tiered run diverged\n{source}");
    let vm = on_vm(source);
    match (&interpreted, &vm) {
        (Ok(a), Ok(b)) => assert_eq!(a, b, "the VM diverged\n{source}"),
        (Err(_), Err(_)) => {}
        _ => panic!("the VM diverged: {interpreted:?} vs {vm:?}\n{source}"),
    }
    interpreted
}

/// `decl` rewritten by `target` (it must change), then called with each
/// input — `call` names the call, `{}` standing for the input — on every
/// tier, before and after: the same value, or a raise both times.
fn same_on(target: Target, decl: &str, call: &str, inputs: &[&str]) {
    let (fixed, report) = fix(decl, target).unwrap_or_else(|e| panic!("{decl}: {e}"));
    assert!(report.count() > 0, "nothing rewritten:\n{decl}");
    assert_ne!(fixed, decl);
    for input in inputs {
        let call = call.replace("{}", input);
        let before = on_every_tier(&format!("{decl}\nfn main() = {call}\nmain()\n"));
        let after = on_every_tier(&format!("{fixed}\nfn main() = {call}\nmain()\n"));
        match (&before, &after) {
            (Ok(a), Ok(b)) => assert_eq!(a, b, "{call}:\n{decl}\nbecame\n{fixed}"),
            (Err(_), Err(_)) => {}
            _ => panic!("{call}: {before:?} became {after:?}\n{decl}\nbecame\n{fixed}"),
        }
    }
}

const RECORDS: &[&str] = &[
    "()",
    "#{}",
    "#{ \"f\": () }",
    "#{ \"f\": 1 }",
    "#{ \"f\": \"s\" }",
    "#{ \"f\": #{ \"g\": 1 } }",
    "#{ \"f\": #{ \"g\": () } }",
    "{ f: 2 }",
    "{ f: { g: \"x\" } }",
    "5",
    "[1]",
    "\"text\"",
];

// ── fixtures ────────────────────────────────────────────────────────
fn fixture(name: &str) -> String {
    let path = format!("{}/tests/fixtures/fix/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn fixture_holds(target: Target, name: &str) {
    let before = fixture(&format!("{name}.before.ol"));
    let after = fixture(&format!("{name}.after.ol"));
    let (once, _) = fix(&before, target).expect("fixes");
    assert_eq!(once, after, "{name}: the rewrite is not the fixture's");
    let (twice, report) = fix(&once, target).expect("fixes again");
    assert_eq!(twice, once, "{name}: a second run changed something");
    assert_eq!(report.count(), 0);
}

#[test]
fn the_fixtures_are_what_each_fix_writes_and_a_second_run_writes_nothing() {
    fixture_holds(Target::Guards, "guards");
    fixture_holds(Target::Defaults, "defaults");
    fixture_holds(Target::FirstLast, "first_last");
    fixture_holds(Target::FirstLast, "first_last_shadow");
    fixture_holds(Target::Units, "units");
    fixture_holds(Target::Units, "units_shadow");
}

#[test]
fn what_is_left_is_counted_by_reason() {
    let (_, r) = fix(&fixture("guards.before.ol"), Target::Guards).unwrap();
    let why: Vec<&str> = r.skipped.iter().map(|(_, w)| *w).collect();
    assert!(why.iter().any(|w| w.contains("used as a Bool")), "{why:?}");
    assert!(
        why.iter().any(|w| w.contains("may itself be ()")),
        "{why:?}"
    );
    assert!(
        why.iter().any(|w| w.contains("a call or an index")),
        "{why:?}"
    );
    assert!(why.iter().any(|w| w.contains("comment")), "{why:?}");
    let (_, r) = fix(&fixture("first_last.before.ol"), Target::FirstLast).unwrap();
    let why: Vec<&str> = r.skipped.iter().map(|(_, w)| *w).collect();
    assert!(
        why.iter().any(|w| w.contains("not proven non-empty")),
        "{why:?}"
    );
    assert!(why.iter().any(|w| w.contains("Bytes")), "{why:?}");
}

#[test]
fn a_file_whose_rewrite_would_read_differently_is_left() {
    // does not parse: refused, not touched
    assert!(
        fix(
            "fn f(x) = if x == () => () else => x.f\nlet = \n",
            Target::Guards
        )
        .is_err()
    );
    // a shebang and a BOM are kept
    let (out, _) = fix(
        "\u{feff}#!/usr/bin/env olang\nfn f(o) = if o.k == () => 0 else => o.k\n",
        Target::Defaults,
    )
    .unwrap();
    assert_eq!(out, "\u{feff}#!/usr/bin/env olang\nfn f(o) = o.k ?? 0\n");
}

// ── each form, on every tier ────────────────────────────────────────
#[test]
fn guards_answer_as_before() {
    for decl in [
        "fn f(x) = if x == () => () else => x.f.g",
        "fn f(x) = if x == () || x.f == () => () else => x.f.g",
        "fn f(x) = if x != () => x.f",
        "fn f(x) = if x != () && x.f != () => x.f.g else => ()",
        "fn f(x) = if x == () => () else => x",
        "fn f(x) = x != () && x.f == 1",
        "fn f(x) = x != () && x.f == \"s\"",
        "fn f(x) = x != () && x.f != () && x.f.g == 1",
        "fn f(x) = x != () && x.f != ()",
        "fn f(x) = x == () || x.f != 1",
        "fn f(x) = x == () || x.f == ()",
        "fn f(x) = x == () || x.f.g != \"x\"",
        "fn f(x) = if x != () && x.f == 1 => \"y\" else => \"n\"",
        "fn f(x) = true && x != () && x.f == 1",
        "fn f(x) = false || x == () || x.f != 2",
    ] {
        same_on(Target::Guards, decl, "f({})", RECORDS);
    }
}

#[test]
fn defaults_answer_as_before_and_evaluate_the_default_only_when_needed() {
    let values = [
        "()",
        "0",
        "1",
        "false",
        "\"\"",
        "[]",
        "#{ \"k\": () }",
        "#{ \"k\": 3 }",
        "{ k: 4 }",
        "#{}",
    ];
    for decl in [
        "fn f(x) = if x == () => 7 else => x",
        "fn f(x) = if x != () => x else => \"none\"",
        "fn f(x) = 1 + if x == () => 0 else => x",
        "fn f(x) = -if x == () => 0 else => x",
        "fn f(x) = if x == () => x == () else => x",
        "fn f(x) = if x == () => raise(\"no x\") else => x",
        "fn f(x) = if x == () => [1] else => x",
    ] {
        same_on(Target::Defaults, decl, "f({})", &values);
    }
    for decl in [
        "fn f(o) = if o.k == () => 7 else => o.k",
        "fn f(o) = if o.k != () => o.k else => raise(\"no k\")",
    ] {
        same_on(Target::Defaults, decl, "f({})", &values[6..]);
        same_on(Target::Defaults, decl, "f({})", &["()"]);
    }
    for (a, b) in [
        ("()", "()"),
        ("()", "2"),
        ("1", "()"),
        ("1", "2"),
        ("false", "()"),
    ] {
        let call = format!("f({a}, {b})");
        same_on(
            Target::Defaults,
            "fn f(a, b) = if a != () => a else if b != () => b else => 0",
            &call,
            &[""],
        );
        same_on(
            Target::Defaults,
            "fn f(a, b) = if a == () => (if b == () => 1 else => b) else => a",
            &call,
            &[""],
        );
    }
}

#[test]
fn first_and_last_answer_as_the_guarded_index_did() {
    let lists = ["[]", "[1]", "[1, 2]", "[(), 1]", "[1, ()]", "[9, 2]", "[2]"];
    for decl in [
        "fn f(xs: [Int]) = if len(xs) == 0 => () else => xs[len(xs) - 1]",
        "fn f(xs: [Int]) = if len(xs) > 0 => xs[0] else => ()",
        "fn f(xs: [Int]) = if len(xs) > 0 => xs[0]",
        "fn f(xs: [Int]) = if 0 < len(xs) => xs[0] else => ()",
        "fn f(xs: [Int]) = if len(xs) >= 1 => xs[0] else => ()",
        "fn f(xs: [Int]) = len(xs) > 0 && xs[len(xs) - 1] == 2",
        "fn f(xs: [Int]) = if len(xs) == 0 || xs[0] == 9 => 0 else => xs[0] + xs[len(xs) - 1]",
        "fn f(xs: [Int]) = if len(xs) == 0 => -1 else => xs[len(xs) - 1]",
        "fn f(xs: [Int]) = { let ys = xs + [5]; if len(ys) > 0 => ys[0] else => () }",
    ] {
        same_on(Target::FirstLast, decl, "f({})", &lists);
    }
    let strings = ["\"\"", "\"a\"", "\"abc\"", "\"héllo\""];
    for decl in [
        "fn f(s: String) = if len(s) == 0 => () else => s[len(s) - 1]",
        "fn f(s: String) = if len(s) == 0 => \"\" else => s[0] + s[len(s) - 1]",
        "fn f(s) = { let parts = str.split(s, \"/\"); if len(parts) == 0 => () else => parts[len(parts) - 1] }",
    ] {
        same_on(Target::FirstLast, decl, "f({})", &strings);
    }
}

#[test]
fn first_and_last_leave_what_might_be_bytes() {
    // `len` and `[i]` take Bytes, `last` refuses it: an unproven list is
    // left, for this is the input it would answer differently
    let decl = "fn f(xs) = if len(xs) == 0 => () else => xs[len(xs) - 1]";
    let (out, report) = fix(decl, Target::FirstLast).unwrap();
    assert_eq!(out, decl);
    assert_eq!(report.skipped.len(), 1);
    // (the bytes module is the interpreter's: the VM is not asked)
    let b = "bytes.from_string(\"ab\")";
    for tier in [None, Some(1)] {
        assert_eq!(
            eval(&format!("{decl}\nfn main() = f({b})\nmain()\n"), tier),
            Ok(Value::Integer(98))
        );
        assert!(eval(&format!("fn main() = last({b})\nmain()\n"), tier).is_err());
    }
}

#[test]
fn units_answer_as_before() {
    let numbers = ["2.5", "-2.5", "3", "-3", "0.49999", "1e300", "-0.0", "7.5"];
    for decl in [
        "fn f(x) = to_int(math.floor(x))",
        "fn f(x) = to_int(math.ceil(x))",
        "fn f(x) = to_int(math.round(x))",
        "fn f(x) = to_int(math.trunc(x))",
        "fn f(x) = to_int(math.round(x * 2.0)) / 2",
    ] {
        same_on(Target::Units, decl, "f({})", &numbers);
    }
}
