// compat: exit 1
// Assigning to a binding not declared mut is an error before running.
println("never printed")
let fixed = 1
fixed = 2
