// Statistics Module for Olang
// Provides basic statistical functions and data structures

// Statistical data structures
share type Stats = struct {
    count: Int,
    sum: Float,
    mean: Float,
    min: Float,
    max: Float,
    range: Float
}

// Basic descriptive statistics
share fn mean(data: [Float]) = {
    sum(data) / len(data)
}

share fn min_value(data: [Float]) = {
    min(data)
}

share fn max_value(data: [Float]) = {
    max(data)
}

share fn range_value(data: [Float]) = {
    max_value(data) - min_value(data)
}

// Create basic statistics
share fn calculate_stats(data: [Float]) = {
    let data_mean = mean(data)
    let data_min = min_value(data)
    let data_max = max_value(data)
    let data_range = range_value(data)
    
    { 
        count: len(data),
        sum: sum(data),
        mean: data_mean,
        min: data_min,
        max: data_max,
        range: data_range
    }
}

// Simple variance calculation
share fn variance(data: [Float]) = {
    let data_mean = mean(data)
    let squared_diffs = map(data, (x) => (x - data_mean) * (x - data_mean))
    mean(squared_diffs)
}

// Summary function
share fn summary_stats(data: [Float]) = {
    let stats = calculate_stats(data)
    
    println("Statistical Summary:")
    println("Count: " + to_string(stats.count))
    println("Sum: " + to_string(stats.sum))
    println("Mean: " + to_string(stats.mean))
    println("Min: " + to_string(stats.min))
    println("Max: " + to_string(stats.max))
    println("Range: " + to_string(stats.range))
    
    stats
} 