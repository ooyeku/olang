// Precise memory corruption test - isolate the exact breaking point

share fn run_test(size: Int) = {
    println("Testing " + to_string(size) + " records...")
    
    // Create simple data
    let data = range(size) |> map((i) => {
        amount: i * 100,
        category: if i % 2 == 0 => "A" else => "B"
    })
    
    // Test filter only (no complex operations)
    let filtered = data |> filter((record) => record.category == "A")
    println("Filter result: " + to_string(len(filtered)) + " items")
    
    // Test sum only (no complex operations)
    let amounts = data |> map((record) => record.amount)
    let total = sum(amounts)
    println("Sum result: " + to_string(total))
    
    println("Test completed successfully for " + to_string(size) + " records")
    println() // Empty line for separation
}

share fn find_breaking_point() = {
    // Test each size individually to find exact breaking point
    run_test(8)   // Should work
    run_test(9)   // Should work  
    run_test(10)  // Might work
    run_test(11)  // This is where we need to find the issue
    run_test(12)  // Likely to fail
}

// Run the precise test
find_breaking_point() 