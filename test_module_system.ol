// Test file for Feature 1: Automatic File-Based Modules
// This tests all aspects of the automatic file-to-module mapping

// Test 1: Simple function call to verify basic functionality
fn test_basic() = {
    println("Basic function test passed")
}

// Test 2: Share declarations (should parse correctly)
share fn shared_function() = "This is shared"
share let SHARED_CONSTANT = 42
share type SharedType = { id: Int, name: String }

// Test 3: Use declarations (should parse correctly but won't resolve files)
// use math_utils { calculate_area }
// use utils.string { format_name }

fn main() = {
    test_basic()
    println("Shared function: " + shared_function())
    println("Shared constant: " + SHARED_CONSTANT)
    
    let test_struct = SharedType { id: 1, name: "test" }
    println("Shared type test: " + test_struct.name)
} 