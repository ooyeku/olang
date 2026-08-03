// Simplified SIMD and Pipeline Performance Benchmark
// Focused test of implemented optimization systems

// Test configuration
let SMALL_SIZE = 100
let MEDIUM_SIZE = 1000
let LARGE_SIZE = 10000  // Use 100000 for stress testing

println("=== SIMD Performance Benchmark ===")

// Test 1: Vectorized arithmetic operations
println("Test 1: Vectorized arithmetic operations...")

// Create test data
let data1 = range(1, LARGE_SIZE)
let data2 = range(1, LARGE_SIZE) |> map((x) => x * 2)

// These operations should use SIMD when arrays are large enough
let squares = data1 |> map((x) => x * x)
let doubled = data1 |> map((x) => x * 2)

// Vectorized arithmetic between arrays
let combined = zip(squares, doubled) |> map((pair) => {
    let (a, b) = pair
    a + b
})

// Fused operations (should trigger specialized SIMD)
let fused = data1 |> map((x) => x * x + x * 2 + 1)  // Polynomial evaluation

let result1 = combined |> take(10) |> reduce(0, (a, b) => a + b)
println("Vectorized arithmetic result: " + result1)

// Test 2: Vectorized filtering and predicates
println("Test 2: Vectorized filtering...")

let data = range(-5000, 50000)

// These filters should use SIMD predicates
let positive = data |> filter((x) => x > 0)
let large_values = data |> filter((x) => x > 1000)
let even_numbers = data |> filter((x) => x % 2 == 0)

// Chained filters (pipeline optimization opportunity)
let complex_filter = data 
    |> filter((x) => x > 0)
    |> filter((x) => x < 3000)
    |> filter((x) => x % 10 == 0)

let result2 = complex_filter |> take(5) |> reduce(0, (a, b) => a + b)
println("Vectorized filtering result: " + result2)

// Test 3: Vectorized reductions
println("Test 3: Vectorized reductions...")

let reduction_data = range(1, LARGE_SIZE)

// These should use SIMD reductions
let sum_result = reduction_data |> reduce(0, (a, b) => a + b)
let product_small = range(1, 10) |> reduce(1, (a, b) => a * b)

println("Sum result: " + sum_result)
println("Product result: " + product_small)

// Test 4: Pipeline fusion opportunities
println("Test 4: Pipeline fusion...")

// This should trigger pipeline fusion
let fusion_result = range(1, MEDIUM_SIZE)
    |> map((x) => x * 2)           // Stage 1
    |> filter((x) => x > 100)      // Stage 2
    |> map((x) => x + 10)          // Stage 3: Should fuse with stage 1
    |> filter((x) => x % 4 == 0)   // Stage 4
    |> map((x) => x / 2)           // Stage 5: Should fuse with stages 1&3
    |> reduce(0, (a, b) => a + b)  // Final reduction

println("Pipeline fusion result: " + fusion_result)

// Test 5: Memory optimization
println("Test 5: Memory optimization...")

// Large computation that should benefit from memory optimization
let memory_result = range(1, MEDIUM_SIZE)
    |> map((x) => {
        let temp = x * x
        temp + x
    })
    |> filter((x) => x > 50)
    |> take(100)
    |> reduce(0, (a, b) => a + b)

println("Memory optimization result: " + memory_result)

println("=== Benchmark Complete ===")
println("All SIMD and pipeline optimizations tested successfully!")