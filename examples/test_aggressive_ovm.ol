// Large-scale compute-intensive workload to test OVM performance
// Designed to favor OVM's strengths: large datasets, complex pipelines, heavy math

// Large dataset generation (100,000 items)
let big_data = range(1, 100000)
println("Generated dataset with", len(big_data), "items")

// Complex mathematical pipeline with multiple transformations
let complex_math = big_data 
    |> map((x) => x * x)                    // Square each number
    |> filter((x) => { x % 7 == 0 })        // Filter by divisible by 7
    |> map((x) => math.sqrt(x))             // Square root
    |> map((x) => math.sin(x) * math.cos(x)) // Trigonometric operations
    |> map((x) => math.pow(x, 1.5))         // Power operations
    |> filter((x) => { x > 0.001 })         // Filter small values
    |> take(5000)                           // Take first 5000 results
    |> sum()

println("Complex math result:", complex_math)

// Large-scale data processing with nested operations
let nested_computation = big_data
    |> chunk(1000)                          // Split into chunks of 1000
    |> map((chunk) => chunk 
        |> map((x) => x * x * x)            // Cube each number
        |> filter((x) => { x % 13 == 0 })   // Filter by divisible by 13
        |> sum())                           // Sum each chunk
    |> filter((sum_val) => { sum_val > 1000000 }) // Filter large sums
    |> map((sum_val) => math.sqrt(sum_val)) // Square root of sums
    |> sum()

println("Nested computation result:", nested_computation)

// Memory-intensive operations with large intermediate results
let memory_intensive = range(1, 50000)
    |> map((x) => [x, x * 2, x * 3, x * 4, x * 5]) // Create tuples
    |> flatten()                             // Flatten to single list
    |> map((x) => x * math.PI)            // Multiply by pi
    |> filter((x) => { x > 100.0 })         // Filter large values
    |> map((x) => math.round(x))            // Round to integers
    |> reverse()                            // Reverse the order
    |> take(10000)                          // Take first 10k
    |> sum()

println("Memory intensive result:", memory_intensive)

// Pipeline fusion optimization test
let fusion_test = range(1, 75000)
    |> map((x) => x + 1)
    |> map((x) => x * 2) 
    |> map((x) => x - 3)
    |> filter((x) => { x % 5 == 0 })
    |> map((x) => x / 5)
    |> map((x) => math.abs(x))
    |> filter((x) => { x < 50000 })
    |> map((x) => math.pow(x, 0.5))
    |> reduce(0, (acc, x) => acc + x)

println("Pipeline fusion result:", fusion_test)

// Heavy computational workload
let heavy_compute = range(1, 25000)
    |> map((x) => {
        let temp1 = math.pow(x, 2)
        let temp2 = math.sin(temp1 / 1000.0)
        let temp3 = math.cos(temp2)
        let temp4 = math.sqrt(math.abs(temp3))
        math.round(temp4 * 1000)
    })
    |> filter((x) => { x % 7 == 0 })
    |> map((x) => x * x)
    |> sum()

println("Heavy compute result:", heavy_compute)

// Large statistical operations
let large_stats = range(1, 80000)
    |> map((x) => x * random.random())        // Add randomness
    |> chunk(100)                           // Split into chunks
    |> map((chunk) => {
        let chunk_sum = chunk |> sum()
        let chunk_avg = chunk_sum / len(chunk)
        let chunk_max = chunk |> max()
        chunk_avg + chunk_max
    })
    |> filter((result) => { result > 50 })
    |> sort()
    |> reverse()
    |> take(500)
    |> average()

println("Large stats result:", large_stats)

// Final summary with all intermediate results
let total_operations = 6
let final_summary = [
    complex_math,
    nested_computation, 
    memory_intensive,
    fusion_test,
    heavy_compute,
    large_stats
] |> sum()

println("=== PERFORMANCE SUMMARY ===")
println("Total operations completed:", total_operations)
println("Final aggregated result:", final_summary)
println("Average result per operation:", final_summary / total_operations) 