// compat: user-defined types — structs, enums, record shape aliases,
// traits with default methods, annotations, error declarations.

type Point = struct { x: Int, y: Int }
let p = Point { x: 3, y: 4 }
println(p.x * p.x + p.y * p.y)
println(typeof(p))
println(p)
println(map_set(p, "y", 5))
println(is_err(attempt(() => map_set(p, "x", 1.5))))

type Shape = enum { Circle(Float), Rect(Float, Float), Unknown }
fn area(s) = match s { Circle(r) => 3.0 * r * r, Rect(w, h) => w * h, Unknown => 0.0 }
println(`${area(Rect(3.0, 4.0))} ${area(Circle(2.0))} ${area(Unknown)}`)
println(Rect(1.0, 2.0))
println(typeof(Unknown))

type Task = { title: String, done: Bool }
fn title_of(t: Task) -> String = t.title
let t: Task = { title: "ship", done: false }
println(title_of(t))

trait Greet {
    fn name(self) -> String
    fn hello(self) -> String = "hello, " + self.name()
}
type Robot = struct { id: Int }
type Human = struct {}
impl Greet for Robot { fn name(self) = `unit-${self.id}` }
impl Greet for Human {
    fn name(self) = "friend"
    fn hello(self) = "hey there"
}
println(Robot { id: 7 }.hello())
println(Human {}.hello())
println(implements(Robot { id: 1 }, "Greet"))

trait Area { fn area(self) -> Int }
type Sq = struct { s: Int }
type Rc = struct { w: Int, h: Int }
impl Area for Sq { fn area(self) = self.s * self.s }
impl Area for Rc { fn area(self) = self.w * self.h }
println([Sq { s: 3 }, Rc { w: 2, h: 5 }] |> fold(0, (acc, s) => acc + s.area()))

error AppError {
    NotFound,
    Invalid: { msg: String }
}
fn lookup(id) = {
    if id == 0 => return Err(NotFound)
    if id < 0 => return Err(Invalid("id must be positive"))
    Ok("user-" + show(id))
}
fn describe(r) = match r {
    Ok(user) => user,
    Err(NotFound) => "no such user",
    Err(Invalid(msg)) => "bad request: " + msg
}
println([describe(lookup(7)), describe(lookup(0)), describe(lookup(-1))])
println(NotFound == NotFound)

// union aliases and annotated lets
type Id = Int | String
fn show_id(i: Id) = `id ${i}`
println(`${show_id(1)} ${show_id("a")}`)
println(is_err(attempt(() => show_id(1.5))))
let k: Int = 3
println(k)
