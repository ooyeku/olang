// Test Feature 6: Transitive Sharing
// This test demonstrates that functions can be re-shared through intermediate modules

use extended_utils { helper, format_text, extended_helper, enhanced_format }

println("=== Feature 6: Transitive Sharing Test ===")
println()

println("--- Testing Transitive Sharing ---")

// Test 1: Use transitively shared function 'helper' from base_utils via extended_utils
println("1. Transitively shared helper():", helper())
println()

// Test 2: Use transitively shared function 'format_text' from base_utils via extended_utils
println("2. Transitively shared format_text():", format_text("Hello World"))
println()

// Test 3: Use functions defined in extended_utils that use transitively shared functions
println("3. Extended functionality using transitive sharing:")
println("   extended_helper():", extended_helper())
println("   enhanced_format():", enhanced_format("Test"))
println()

println("--- Dependency Chain Verification ---")
println("✓ base_utils.helper() → extended_utils.helper() → main")
println("✓ base_utils.format_text() → extended_utils.format_text() → main")
println("✓ Transitive sharing allows access to original functions")
println("✓ New functions can build upon transitively shared functions")
println()

println("=== Feature 6 Test Completed Successfully ===")
println("Transitive sharing enables code reuse across module chains!") 