// Chain B - Intermediate module in the dependency chain
// This module imports from chain_a and adds functionality

use chain_a { chain_a_func, multiply_by_2, CHAIN_A_CONSTANT }

// Re-share some functions from chain_a
share use chain_a { chain_a_func, multiply_by_2 }

// Add new functionality
share fn chain_b_func() = "Function from Chain B that uses: " + chain_a_func()

share fn multiply_by_4(x: Int) = multiply_by_2(multiply_by_2(x))

println("")
println(chain_b_func())
println(multiply_by_2(multiply_by_4(4)))
