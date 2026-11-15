// compat: test
// Test blocks under `olang test`, and the testing module. A test fixture
// pins its exit code and that the lines of its .out appear in order;
// the runner's own report around them is not contract.

fn add(a, b) = a + b

test "addition works" {
    assert_eq(add(2, 2), 4)
    assert(add(1, 1) > 1, "sum should exceed operands")
    assert_ne(add(1, 2), 4)
    assert_true(add(0, 1) == 1)
    assert_false(add(1, 1) == 3)
    assert_close(0.1 + 0.2, 0.3, 0.000001)
}

test "a test block is its own scope" {
    let fs = "shadowed here only"
    assert_eq(fs, "shadowed here only")
}

println(typeof(fs))
testing.reset_tests()
testing.assert_eq(1, 1)
testing.assert_ok(Ok(1))
testing.assert_err(Err("e"))
let tally = testing.test_summary()
println(`${tally.passed} ${tally.failed} ${tally.total}`)
println(is_err(attempt(() => assert_eq(1, 2))))
println("tests passed")
