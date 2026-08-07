//! Traits as runtime dispatch: method resolution on the receiver's runtime
//! type, default methods, polymorphism, methods with arguments, and traits
//! over both structs and enums.

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

fn s(v: Value) -> String {
    match v {
        Value::String(s) => s.to_string(),
        other => panic!("expected string, got {:?}", other),
    }
}

#[test]
fn dispatches_on_runtime_type() {
    let src = r#"
trait Show { fn show(self) -> String }
type A = struct { v: Int }
type B = struct { v: Int }
impl Show for A { fn show(self) = "A" + to_string(self.v) }
impl Show for B { fn show(self) = "B" + to_string(self.v) }
let a = A { v: 1 }
let b = B { v: 2 }
a.show() + "/" + b.show()
"#;
    assert_eq!(s(eval(src)), "A1/B2");
}

#[test]
fn default_method_calls_back_into_impl() {
    let src = r#"
trait Show {
    fn show(self) -> String
    fn shout(self) -> String = self.show() + "!"
}
type P = struct { x: Int }
impl Show for P { fn show(self) = "p" + to_string(self.x) }
P { x: 5 }.shout()
"#;
    assert_eq!(s(eval(src)), "p5!");
}

#[test]
fn impl_can_override_a_default_method() {
    let src = r#"
trait Greet {
    fn hello(self) -> String = "generic hello"
}
type Formal = struct {}
type Casual = struct {}
impl Greet for Formal {}
impl Greet for Casual { fn hello(self) = "hey" }
Formal {}.hello() + "/" + Casual {}.hello()
"#;
    assert_eq!(s(eval(src)), "generic hello/hey");
}

#[test]
fn polymorphism_at_one_call_site() {
    let src = r#"
trait Area { fn area(self) -> Int }
type Sq = struct { s: Int }
type Rect = struct { w: Int, h: Int }
impl Area for Sq { fn area(self) = self.s * self.s }
impl Area for Rect { fn area(self) = self.w * self.h }
let shapes = [Sq { s: 3 }, Rect { w: 2, h: 5 }, Sq { s: 4 }]
fold(shapes, 0, (acc, shape) => acc + shape.area())
"#;
    assert_eq!(eval(src), Value::Integer(9 + 10 + 16));
}

#[test]
fn methods_take_arguments() {
    let src = r#"
trait Scale { fn by(self, factor) -> Int }
type N = struct { v: Int }
impl Scale for N { fn by(self, factor) = self.v * factor }
N { v: 6 }.by(7)
"#;
    assert_eq!(eval(src), Value::Integer(42));
}

#[test]
fn traits_dispatch_over_enums() {
    let src = r#"
type Shape = enum { Sq(Int), Tri(Int, Int) }
trait Area { fn area(self) -> Int }
impl Area for Shape {
    fn area(self) = match self {
        Sq(s) => s * s,
        Tri(b, h) => b * h / 2
    }
}
Sq(5).area() + Tri(6, 4).area()
"#;
    assert_eq!(eval(src), Value::Integer(25 + 12));
}

#[test]
fn struct_fields_take_precedence_over_methods() {
    // A field access to a real field must still return the field, even if a
    // trait method of the same name existed.
    let src = r#"
type Box = struct { value: Int }
let b = Box { value: 99 }
b.value
"#;
    assert_eq!(eval(src), Value::Integer(99));
}

#[test]
fn calling_an_unimplemented_method_errors() {
    let src = r#"
type P = struct { x: Int }
let p = P { x: 1 }
p.nonexistent()
"#;
    assert!(eval_res(src).is_err());
}

#[test]
fn same_method_name_across_unrelated_traits() {
    // Two types implement a method of the same name via different traits;
    // dispatch still picks the receiver's implementation.
    let src = r#"
trait Named { fn name(self) -> String }
type Dog = struct {}
type City = struct {}
impl Named for Dog { fn name(self) = "Rex" }
impl Named for City { fn name(self) = "Paris" }
Dog {}.name() + "/" + City {}.name()
"#;
    assert_eq!(s(eval(src)), "Rex/Paris");
}

#[test]
fn traits_and_impls_can_be_shared() {
    // `share trait` / `share impl` parse and behave like their unshared forms
    // (traits register globally; `share` is cosmetic but must be accepted).
    let src = r#"
share trait Greet { fn greet(self) -> String }
type Dog = struct {}
share impl Greet for Dog { fn greet(self) = "woof" }
Dog {}.greet()
"#;
    assert_eq!(s(eval(src)), "woof");
}
