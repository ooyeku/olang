// Simple SIMD Performance Test
// Tests basic SIMD operations with correct syntax

println("=== Simple SIMD Performance Test ===")

// Test 1: Large array arithmetic (should trigger SIMD)
println("Test 1: Large array arithmetic operations...")
let numbers = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 100, 200, 300, 400, 500]
let squares = numbers |> map((x) => x * x)
let total_squares = sum(squares)
println("Sum of squares: " + to_string(total_squares))

// Test 2: Vectorized filtering (should use SIMD predicates)  
println("Test 2: Vectorized filtering...")
let large_numbers = [-50, -25, 0, 25, 50, 75, 100, 125, 150, 175, 200]
let positive_only = large_numbers |> filter((x) => x > 0)
let count_positive = len(positive_only)
println("Count of positive numbers: " + to_string(count_positive))

// Test 3: Combined map-filter operations
println("Test 3: Combined pipeline operations...")
let data = [1, 2, 3, 4, 5, 10, 15, 20, 25, 30]
let processed = data
    |> map((x) => x * 2)           // Double each number
    |> filter((x) => x > 10)       // Keep only large numbers
    |> map((x) => x + 5)           // Add 5
let final_sum = sum(processed)
println("Pipeline result: " + to_string(final_sum))

// Test 4: Nested computation
println("Test 4: Nested computation...")
let outer_data = [1, 2, 3, 4, 5]
let nested_results = outer_data |> map((outer) => {
    let inner_data = [1, 2, 3, 4, 5]
    let products = inner_data |> map((inner) => outer * inner)
    sum(products)
})
let nested_total = sum(nested_results)
println("Nested computation result: " + to_string(nested_total))

println("")
println("=== Test Complete ===")
println("If optimizations are working:")
println("- Large numbers should compute quickly (SIMD vectorization)")
println("- Complex pipelines should be efficient (fusion optimization)")
println("- Memory usage should be reasonable (GC optimization)")