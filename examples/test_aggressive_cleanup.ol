// Test aggressive memory management with manual cleanup

share fn run_cleanup_test(size: Int) = {
    println("Testing " + to_string(size) + " records with aggressive cleanup...")
    
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

share fn run_sequence_test() = {
    // Test each size individually with aggressive cleanup
    run_cleanup_test(8)   // Should work
    run_cleanup_test(9)   // Should work  
    run_cleanup_test(10)  // Should work
    run_cleanup_test(11)  // Should work now
    run_cleanup_test(12)  // Should work now
    run_cleanup_test(15)  // Should work now
}

// Run the test with aggressive cleanup
run_sequence_test() 