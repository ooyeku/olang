// Debug test for imports

use test_debug_module { simple_add, TEST_CONSTANT }

fn main() = {
    println("Import test: simple_add(5, 10) = " + simple_add(5, 10))
    println("Import test: TEST_CONSTANT = " + TEST_CONSTANT)
}

main() 