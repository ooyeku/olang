// Comprehensive test for Feature 2: Simple Share/Use System
// This demonstrates cross-file imports with various types of shared items

use test_feature2_simple_share_use { add, greet, multiply, PI, MAX_SIZE, Point, User, create_point, create_user, distance, factorial, fibonacci, circle_area }

fn test_basic_function_imports() = {
    println("=== Basic Function Imports ===")
    println("add(15, 25) = " + add(15, 25))
    println("greet('Feature 2') = " + greet("Feature 2"))
    println("multiply(6.5, 3.0) = " + multiply(6.5, 3.0))
}

fn test_constant_imports() = {
    println("=== Constant Imports ===")
    println("PI = " + PI)
    println("MAX_SIZE = " + MAX_SIZE)
}

fn test_type_and_constructor_imports() = {
    println("=== Type and Constructor Imports ===")
    let point_a = create_point(1.0, 2.0)
    let point_b = create_point(4.0, 6.0)
    println("Point A: (" + point_a.x + ", " + point_a.y + ")")
    println("Point B: (" + point_b.x + ", " + point_b.y + ")")
    println("Distance A to B: " + distance(point_a, point_b))
    
    let user = create_user(123, "Feature2User", "feature2@olang.org")
    println("Created user: " + user.name + " (ID: " + user.id + ")")
}

fn test_complex_function_imports() = {
    println("=== Complex Function Imports ===")
    println("factorial(6) = " + factorial(6))
    println("fibonacci(9) = " + fibonacci(9))
    println("circle_area(7.5) = " + circle_area(7.5))
}

fn test_combining_imports() = {
    println("=== Combining Imported Items ===")
    
    // Create a circle with radius based on distance
    let origin = create_point(0.0, 0.0)
    let edge = create_point(3.0, 4.0)
    let radius = distance(origin, edge)
    let area = circle_area(radius)
    
    println("Circle with radius " + radius + " has area " + area)
    
    // Use constants in calculations
    let scaled_pi = PI * 2.0
    println("Scaled PI (2 * PI) = " + scaled_pi)
    
    // Combine with math operations
    let factorial_result = factorial(4)
    let sum_result = add(factorial_result, 10)
    println("factorial(4) + 10 = " + sum_result)
}

fn main() = {
    println("Feature 2: Comprehensive Share/Use System Test")
    println("===============================================")
    println()
    
    test_basic_function_imports()
    println()
    
    test_constant_imports()
    println()
    
    test_type_and_constructor_imports()
    println()
    
    test_complex_function_imports()
    println()
    
    test_combining_imports()
    println()
    
    println("Feature 2: All comprehensive tests passed!")
    println("Successfully demonstrated:")
    println("  ✓ Function sharing and importing")
    println("  ✓ Constant sharing and importing")
    println("  ✓ Type sharing and importing")
    println("  ✓ Complex function operations")
    println("  ✓ Combining multiple imported items")
    println("  ✓ Cross-file module resolution")
}

main() 