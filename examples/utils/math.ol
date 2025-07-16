// Math Utilities Module  
// This module provides mathematical helper functions

share fn calculate_average(numbers: [Float]) = {
    sum(numbers) / len(numbers)
}

share fn square(x: Float) = {
    x * x
}

share fn cube(x: Float) = {
    x * x * x
}

// Private function (not shared)
fn private_calc() = {
    42
} 