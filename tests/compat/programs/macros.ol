// compat: macros — meta fn, @name in expression position, decorators,
// applicative order, the meta helpers.

meta fn unless(cond, body) = `if !(${cond}) => ${body} else => ()`
let x = 3
@unless(x > 5, println("x is small"))

meta fn twice(e) = `(${e}) * 2`
println(@twice(@twice(5)))

meta fn square_of(n) = { let v = unwrap(meta.eval(n)); meta.lit(v * v) }
println(@square_of(7))

meta fn traced(decl) = `${decl}
let traced_ok = true`
@traced
fn hello() = "hello"
println(hello())
println(traced_ok)
