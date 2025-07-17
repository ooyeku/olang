// Test file for the stats_module functionality
use stats_module { mean, min_value, max_value, range_value, variance, calculate_stats, Stats }

test "mean calculation with positive numbers" {
    let data = [1.0, 2.0, 3.0, 4.0, 5.0]
    let result = mean(data)
    assert_eq(result, 3.0, "Mean of [1,2,3,4,5] should be 3.0")
}

test "mean calculation with negative numbers" {
    let data = [-2.0, -1.0, 0.0, 1.0, 2.0]
    let result = mean(data)
    assert_eq(result, 0.0, "Mean of [-2,-1,0,1,2] should be 0.0")
}

test "minimum value detection" {
    let data = [5.2, 1.8, 9.1, 3.7, 2.4]
    let result = min_value(data)
    assert_eq(result, 1.8, "Minimum value should be 1.8")
}

test "maximum value detection" {
    let data = [5.2, 1.8, 9.1, 3.7, 2.4]
    let result = max_value(data)
    assert_eq(result, 9.1, "Maximum value should be 9.1")
}

test "range calculation" {
    let data = [10.0, 5.0, 15.0, 8.0]
    let result = range_value(data)
    assert_eq(result, 10.0, "Range should be 15.0 - 5.0 = 10.0")
}

test "variance calculation" {
    let data = [2.0, 4.0, 6.0, 8.0]
    let result = variance(data)
    // Variance of [2,4,6,8]: mean=5, variance=[(2-5)²+(4-5)²+(6-5)²+(8-5)²]/4 = [9+1+1+9]/4 = 5
    assert_eq(result, 5.0, "Variance should be 5.0")
}

test "stats struct creation and validation" {
    let data = [1.0, 2.0, 3.0, 4.0, 5.0]
    let stats = calculate_stats(data)
    
    assert_eq(stats.count, 5, "Count should be 5")
    assert_eq(stats.sum, 15.0, "Sum should be 15.0")
    assert_eq(stats.mean, 3.0, "Mean should be 3.0")
    assert_eq(stats.min, 1.0, "Min should be 1.0")
    assert_eq(stats.max, 5.0, "Max should be 5.0")
    assert_eq(stats.range, 4.0, "Range should be 4.0")
}

test "empty data list handling" {
    let empty_data = []
    let stats = calculate_stats(empty_data)
    
    assert_eq(stats.count, 0, "Count of empty list should be 0")
    assert_eq(stats.sum, 0.0, "Sum of empty list should be 0.0")
}

test "single element data" {
    let single_data = [42.5]
    let stats = calculate_stats(single_data)
    
    assert_eq(stats.count, 1, "Count should be 1")
    assert_eq(stats.sum, 42.5, "Sum should be 42.5")
    assert_eq(stats.mean, 42.5, "Mean should be 42.5")
    assert_eq(stats.min, 42.5, "Min should be 42.5")
    assert_eq(stats.max, 42.5, "Max should be 42.5")
    assert_eq(stats.range, 0.0, "Range should be 0.0")
}

test "boolean assertions work correctly" {
    let data = [1.0, 2.0, 3.0]
    let avg = mean(data)
    
    assert_true(avg > 1.0, "Average should be greater than 1.0")
    assert_false(avg < 1.0, "Average should not be less than 1.0")
    assert(avg == 2.0, "Average should equal 2.0")
}

test "assertion messages are displayed on failure" {
    let data = [10.0, 20.0, 30.0]
    let result = mean(data)
    
    // This should pass
    assert_ne(result, 15.0, "Mean should not be 15.0 (it should be 20.0)")
} 