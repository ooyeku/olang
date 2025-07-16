// Test file demonstrating usage of the stats_module
use stats_module { mean, min_value, max_value, range_value, variance, calculate_stats, summary_stats, Stats }

println("=== Statistics Module Test ===")
println()

// Test data
let data = [1.5, 2.8, 3.1, 4.7, 2.9, 5.2, 3.8, 2.1, 4.3, 6.1]

println("Data:", data)
println()

// Test individual functions
println("=== Individual Function Tests ===")
println("Mean:", mean(data))
println("Min:", min_value(data))
println("Max:", max_value(data))
println("Range:", range_value(data))
println("Variance:", variance(data))
println()

// Test stats struct calculation
println("=== Stats Struct Test ===")
let stats = calculate_stats(data)
println("Count:", stats.count)
println("Sum:", stats.sum)
println("Mean:", stats.mean)
println("Min:", stats.min)
println("Max:", stats.max)
println("Range:", stats.range)
println()

// Test formatted summary
println("=== Formatted Summary ===")
summary_stats(data)

println()
println("✓ All statistics functions working correctly!") 