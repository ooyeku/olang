// math_utils.ol - Test module for Feature 1

// Private function (not shared)
fn calculate_area_internal(radius: Float) = 3.14159 * radius * radius

// Shared functions
share fn area_circle(radius: Float) = calculate_area_internal(radius)
share fn area_square(side: Float) = side * side
share let PI = 3.14159 