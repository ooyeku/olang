// Debug test to understand module system

share fn simple_add(a: Int, b: Int) = a + b
share let TEST_CONSTANT = 42

fn main() = {
    println("Module test: simple_add(1, 2) = " + simple_add(1, 2))
    println("Module test: TEST_CONSTANT = " + TEST_CONSTANT)
}

main() 