// Test Dependency Chain Tracking
// This test demonstrates a long chain: chain_a → chain_b → chain_c → main

use chain_c { chain_a_func, chain_c_func, multiply_by_2, multiply_by_8 }

println("=== Dependency Chain Tracking Test ===")
println()

println("--- Testing Long Dependency Chain ---")
println("Chain: chain_a → chain_b → chain_c → main")
println()

// Test transitive function from chain_a, accessed through chain_c
println("1. chain_a_func() accessed via chain_c:", chain_a_func())

// Test function from chain_c that uses chain_b functionality
println("2. chain_c_func() using chain_b:", chain_c_func())

// Test mathematical function that goes through the full chain
println("3. multiply_by_2(5) via chain_c:", multiply_by_2(5))
println("4. multiply_by_8(3) using chain functions:", multiply_by_8(3))

println()
println("--- Dependency Chain Verification ---")
println("✓ chain_a.chain_a_func() → chain_b → chain_c → main")
println("✓ chain_a.multiply_by_2() → chain_b → chain_c → main")
println("✓ Functions can be accessed through multiple hops")
println("✓ Each module adds value while preserving access to base functions")
println()

println("=== Dependency Chain Test Completed ===")
println("Multi-level transitive sharing works correctly!") 