// Chain C - Final intermediate module in the dependency chain
// This module imports from chain_b and creates a longer dependency chain

use chain_b { chain_a_func, chain_b_func, multiply_by_2, multiply_by_4 }

// Re-share functions through the chain: chain_a → chain_b → chain_c
share use chain_b { chain_a_func, multiply_by_2 }

// Add final layer functionality
share fn chain_c_func() = "Function from Chain C that uses: " + chain_b_func()

share fn multiply_by_8(x: Int) = multiply_by_4(multiply_by_2(x)) 

for i in 1..100 {
    println(multiply_by_8(i))
    println(chain_c_func())
    println(chain_a_func())
    println(chain_b_func())
    println(multiply_by_2(i))
    println(multiply_by_4(i))
    for i in 1..100 {
    println(multiply_by_8(i))
    println(chain_c_func())
    println(chain_a_func())
    println(chain_b_func())
    println(multiply_by_2(i))
    println(multiply_by_4(i))
}
}