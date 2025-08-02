// Quick SIMD Performance Test
// Tests the core SIMD operations implemented in v0.19

print("=== Quick SIMD Performance Test ===")

// Test 1: Large array arithmetic (should trigger SIMD)
print("Test 1: Large array arithmetic operations...")
let large_array = range(1, 10000)
let squares = large_array |> map((x) => x * x)
let sum_squares = squares |> reduce(0, (a, b) => a + b)
print("Sum of squares (1-10000): " + sum_squares)

// Test 2: Vectorized filtering (should use SIMD predicates)  
print("\nTest 2: Vectorized filtering...")
let positive_only = range(-5000, 5000) |> filter((x) => x > 0)
let count_positive = positive_only |> reduce(0, (acc, x) => acc + 1)
print("Count of positive numbers: " + count_positive)

// Test 3: Combined map-filter-reduce pipeline (should be optimized)
print("\nTest 3: Combined pipeline operations...")
let pipeline_result = range(1, 5000)
    |> map((x) => x * 2)           // Double each number
    |> filter((x) => x > 1000)     // Keep only large numbers
    |> map((x) => x + 100)         // Add 100
    |> filter((x) => x % 10 == 0)  // Keep multiples of 10
    |> reduce(0, (a, b) => a + b) // Sum them up

print("Pipeline result: " + pipeline_result)

// Test 4: Nested computation (parallel processing opportunity)
print("\nTest 4: Nested computation...")
let nested_result = range(1, 100) |> map((outer) => {
    range(1, 50) |> map((inner) => outer * inner) |> reduce(0, (a, b) => a + b)
}) |> reduce(0, (a, b) => a + b)

print("Sum nested computation: " + nested_result)

// Test 5: Arithmetic operations between arrays
print("\nTest 5: Array arithmetic...")
let array1 = range(1, 1000)
let array2 = range(1, 1000) |> map((x) => x * 2)
let combined = zip(array1, array2) |> map((pair) => {
    let (a, b) = pair
    a + b
}) |> reduce(0, (acc, x) => acc + x)
print("Combined array sum: " + combined)

print("\n=== Test Complete ===")
print("If optimizations are working:")
print("- Large numbers should compute quickly (SIMD vectorization)")
print("- Complex pipelines should be efficient (fusion optimization)")
print("- Memory usage should be reasonable (GC optimization)")

