// Memory leak test - simple operations that should not cause malloc failures
println("=== Memory Leak Test ===")

// Create a very small dataset
let small_data = [
    { name: "A", value: 10 },
    { name: "B", value: 20 },
    { name: "C", value: 30 }
]

println("Created " + to_string(len(small_data)) + " records")

// Test basic filter operation
println("Testing filter...")
let filtered = small_data |> filter((item) => item.value > 15)
println("Filtered result: " + to_string(len(filtered)) + " records")

// Test basic map operation  
println("Testing map...")
let mapped = small_data |> map((item) => item.value * 2)
println("Mapped result: " + to_string(len(mapped)) + " records")

// Test chained operations (this is where memory explodes)
println("Testing chained operations...")
let result = small_data 
    |> filter((item) => item.value > 5)
    |> map((item) => item.value * 2)
    |> sum()

println("Final result: " + to_string(result))
println("Test completed successfully!") 