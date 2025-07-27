// Scale test - find where memory usage explodes

share fn create_test_data(size: Int) = {
    println("Creating dataset with " + to_string(size) + " records...")
    
    // Use range to create data efficiently
    range(size) |> map((i) => {
        product: "Product_" + to_string(i),
        amount: i * 100, 
        date: "2024-01-15",
        category: if i % 2 == 0 => "Electronics" else => "Furniture"
    })
}

share fn run_memory_test(size: Int) = {
    println("=== Testing with " + to_string(size) + " records ===")
    
    let data = create_test_data(size)
    println("✓ Data created: " + to_string(len(data)) + " records")
    
    // Test basic filter
    let filtered = data |> filter((record) => record.date[5] == "0" && record.date[6] == "1")
    println("✓ Filter works: " + to_string(len(filtered)) + " records")
    
    // Test basic map
    let mapped = data |> map((record) => record.amount * 2)
    println("✓ Map works: " + to_string(len(mapped)) + " values")
    
    // Test pipeline (this is likely where it breaks)
    let result = data 
        |> filter((record) => record.category == "Electronics")
        |> map((record) => record.amount * 2)
        |> sum()
    
    println("✓ Pipeline works: total = " + to_string(result))
    println()
}

share fn run_all_tests() = {
    // Start small and gradually increase
    run_memory_test(2)    // Known to work
    run_memory_test(5)    // Should work
    run_memory_test(10)   // Might work
    run_memory_test(15)   // This is where sales_analyzer fails
    run_memory_test(20)   // This will definitely fail if 15 fails
}

// Run the scale tests
run_all_tests() 