//! Trait bounds (`fn f<T: Show>(x: T)`): runtime-enforced contracts checked
//! at the call boundary against argument types, plus the `implements` builtin.

use olang::{Interpreter, Parser, Value};

fn eval(src: &str) -> Value {
    let parser = Parser::new();
    let program = parser.parse(src).expect("parse");
    let mut interpreter = Interpreter::new();
    interpreter.eval_program(program).expect("eval")
}

fn eval_res(src: &str) -> Result<Value, String> {
    let parser = Parser::new();
    let program = parser.parse(src).map_err(|e| e.to_string())?;
    let mut interpreter = Interpreter::new();
    interpreter.eval_program(program).map_err(|e| e.to_string())
}

const SHOW_POINT: &str = r#"
trait Show { fn show(self) -> String }
type Point = struct { x: Int, y: Int }
type Circle = struct { r: Int }
impl Show for Point { fn show(self) = "(" + to_string(self.x) + "," + to_string(self.y) + ")" }
"#;

#[test]
fn satisfied_bound_runs() {
    let src = format!(
        "{SHOW_POINT}
fn describe<T: Show>(item: T) -> String = item.show()
describe(Point {{ x: 1, y: 2 }})"
    );
    assert_eq!(eval(&src), Value::String("(1,2)".to_string().into()));
}

#[test]
fn violated_bound_errors_at_the_boundary() {
    let src = format!(
        "{SHOW_POINT}
fn describe<T: Show>(item: T) -> String = item.show()
describe(Circle {{ r: 5 }})"
    );
    let err = eval_res(&src).unwrap_err();
    // The message names the argument, its type, and the unmet trait
    assert!(err.contains("Circle"), "got: {err}");
    assert!(err.contains("Show"), "got: {err}");
}

#[test]
fn implements_builtin_reports_membership() {
    let src = format!(
        "{SHOW_POINT}
[implements(Point {{ x: 0, y: 0 }}, \"Show\"), implements(Circle {{ r: 1 }}, \"Show\")]"
    );
    match eval(&src) {
        Value::List(items) => {
            assert_eq!(items[0], Value::Boolean(true));
            assert_eq!(items[1], Value::Boolean(false));
        }
        other => panic!("expected list, got {:?}", other),
    }
}

#[test]
fn multiple_bounds_all_required() {
    let base = r#"
trait Show { fn show(self) -> String }
trait Rank { fn rank(self) -> Int }
type Item = struct { n: Int }
impl Show for Item { fn show(self) = "i" }
impl Rank for Item { fn rank(self) = self.n }
fn label<T: Show + Rank>(x: T) -> String = x.show() + to_string(x.rank())
"#;
    // Both satisfied
    assert_eq!(
        eval(&format!("{base}\nlabel(Item {{ n: 7 }})")),
        Value::String("i7".to_string().into())
    );
}

#[test]
fn partial_multibound_names_the_missing_trait() {
    let src = r#"
trait Show { fn show(self) -> String }
trait Rank { fn rank(self) -> Int }
type Item = struct { n: Int }
impl Show for Item { fn show(self) = "i" }
fn label<T: Show + Rank>(x: T) = x.show()
label(Item { n: 1 })
"#;
    let err = eval_res(src).unwrap_err();
    assert!(
        err.contains("Rank"),
        "should name the unmet trait Rank; got: {err}"
    );
}

#[test]
fn unbounded_generics_are_unaffected() {
    let src = r#"
fn identity<T>(x: T) -> T = x
[identity(42), identity("hi")]
"#;
    match eval(src) {
        Value::List(items) => {
            assert_eq!(items[0], Value::Integer(42));
            assert_eq!(items[1], Value::String("hi".to_string().into()));
        }
        other => panic!("expected list, got {:?}", other),
    }
}

#[test]
fn bound_can_name_a_concrete_type() {
    // A bound naming a type (not a trait) is satisfied by that exact type,
    // so `<T: Point>` accepts a Point and rejects others.
    let src = format!(
        "{SHOW_POINT}
fn only_points<T: Point>(p: T) -> Int = p.x
only_points(Point {{ x: 9, y: 0 }})"
    );
    assert_eq!(eval(&src), Value::Integer(9));
}

#[test]
fn bounds_apply_through_a_trait_default_receiver() {
    // The bound is checked on the argument; the function body may then call
    // trait methods (concrete or default) freely.
    let src = r#"
trait Greet {
    fn name(self) -> String
    fn hello(self) -> String = "hi " + self.name()
}
type Dog = struct {}
impl Greet for Dog { fn name(self) = "rex" }
fn greet<T: Greet>(x: T) -> String = x.hello()
greet(Dog {})
"#;
    assert_eq!(eval(src), Value::String("hi rex".to_string().into()));
}
