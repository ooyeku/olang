// Debug the exact 11-record memory corruption issue

share fn debug_11() = {
    println("=== Testing exactly 11 records ===")
    
    // Step 1: Create exactly 11 records
    println("Creating 11 records...")
    let data = range(11) |> map((i) => {
        amount: i * 100,
        category: if i % 2 == 0 => "A" else => "B"
    })
    println("✓ Created 11 records")
    
    // Step 2: Test each operation individually
    println("Testing filter operation...")
    let filtered = data |> filter((record) => record.category == "A")
    println("✓ Filter result: " + to_string(len(filtered)) + " items")
    
    println("Testing map operation...")
    let amounts = data |> map((record) => record.amount)
    println("✓ Map result: " + to_string(len(amounts)) + " items")
    
    println("Testing sum operation...")
    let total = sum(amounts)
    println("✓ Sum result: " + to_string(total))
    
    println("=== All operations completed successfully ===")
}

// Run the test
debug_11() 