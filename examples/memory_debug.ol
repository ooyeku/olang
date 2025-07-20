// Minimal memory debug test - isolate the exact cause of malloc failures

share fn run_memory_debug() = {
    println("=== Memory Debug Test ===")

    // Test 1: Simple data creation
    println("Test 1: Creating simple data...")
    let simple_data = [
        { value: 10 },
        { value: 20 }
    ]
    println("✓ Simple data created: " + to_string(len(simple_data)) + " items")

    // Test 2: Basic string indexing (our fix)
    println("Test 2: String indexing...")
    let date_str = "2024-01-15"
    let month_char_1 = date_str[5]
    let month_char_2 = date_str[6]
    println("✓ String indexing works: chars = " + month_char_1 + ", " + month_char_2)

    // Test 3: Simple filter operation
    println("Test 3: Simple filter...")
    let filtered = simple_data |> filter((item) => item.value > 15)
    println("✓ Filter works: " + to_string(len(filtered)) + " items")

    // Test 4: Simple map operation  
    println("Test 4: Simple map...")
    let mapped = simple_data |> map((item) => item.value * 2)
    println("✓ Map works: " + to_string(len(mapped)) + " items")

    // Test 5: Pipeline with more operations (this might be where it breaks)
    println("Test 5: More complex pipeline...")
    let result = simple_data 
        |> filter((item) => item.value > 5)
        |> map((item) => item.value * 2)
        |> map((item) => item + 10)

    println("✓ Complex pipeline works: " + to_string(len(result)) + " items")

    // Test 6: Date string operations like sales analyzer
    println("Test 6: Date string filtering...")
    let dates = [
        { date: "2024-01-15", amount: 100.0 },
        { date: "2024-02-01", amount: 200.0 }
    ]

    let jan_data = dates |> filter((record) => record.date[5] == "0" && record.date[6] == "1")
    println("✓ Date filtering works: " + to_string(len(jan_data)) + " items")

    println("✓ All tests passed! Memory seems stable with small data.")
}

// Run the debug test
run_memory_debug() 