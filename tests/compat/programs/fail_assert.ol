// compat: test, exit 1
// A failing assertion fails its test, and `olang test` exits 1.
println("before")
test "fails" { assert_eq(1, 2) }
println("after")
