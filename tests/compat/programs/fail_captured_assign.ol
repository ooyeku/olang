// compat: exit 1
// Assigning to a binding captured from an enclosing scope is an error.
println("never printed")
fn outer() = {
    let mut n = 0
    let bump = () => { n = n + 1 }
    bump()
    n
}
println(outer())
