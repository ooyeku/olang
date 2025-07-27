// Debug the sequence issue - multiple operations in sequence

share fn debug_seq() = {
    println("=== Testing sequence of operations ===")
    
    // Test 1: 8 records
    println("Test 1: 8 records")
    let data1 = range(8) |> map((i) => {
        amount: i * 100,
        category: if i % 2 == 0 => "A" else => "B"
    })
    let filtered1 = data1 |> filter((record) => record.category == "A")
    let total1 = sum(filtered1 |> map((record) => record.amount))
    println("✓ Test 1 result: " + to_string(total1))
    
    // Test 2: 9 records  
    println("Test 2: 9 records")
    let data2 = range(9) |> map((i) => {
        amount: i * 100,
        category: if i % 2 == 0 => "A" else => "B"
    })
    let filtered2 = data2 |> filter((record) => record.category == "A")
    let total2 = sum(filtered2 |> map((record) => record.amount))
    println("✓ Test 2 result: " + to_string(total2))
    
    // Test 3: 10 records
    println("Test 3: 10 records")
    let data3 = range(10) |> map((i) => {
        amount: i * 100,
        category: if i % 2 == 0 => "A" else => "B"
    })
    let filtered3 = data3 |> filter((record) => record.category == "A")
    let total3 = sum(filtered3 |> map((record) => record.amount))
    println("✓ Test 3 result: " + to_string(total3))
    
    // Test 4: 11 records
    println("Test 4: 11 records")
    let data4 = range(11) |> map((i) => {
        amount: i * 100,
        category: if i % 2 == 0 => "A" else => "B"
    })
    let filtered4 = data4 |> filter((record) => record.category == "A")
    let total4 = sum(filtered4 |> map((record) => record.amount))
    println("✓ Test 4 result: " + to_string(total4))
    
    println("=== All sequence tests completed ===")
}

// Run the sequence test
debug_seq() 