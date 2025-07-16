// Test file for Feature 2: Cross-file imports and usage
// This file imports from test_feature2_simple_share_use.ol and validates usage

// Test 1: Import specific functions
use test_feature2_simple_share_use { add, greet, multiply }

// Test 2: Import constants/variables
use test_feature2_simple_share_use { PI, MAX_SIZE, WELCOME_MESSAGE }

// Test 3: Import custom types
use test_feature2_simple_share_use { Point, User, Color }

// Test 4: Import functions that use shared types
use test_feature2_simple_share_use { create_point, create_user, distance }

// Test 5: Import complex functions
use test_feature2_simple_share_use { factorial, fibonacci, circle_area, validate_user }

// Test 6: Local functions that use imported items
fn test_imported_functions() = {
    println("Testing imported functions...")
    println("add(10, 20) = " + add(10, 20))
    println("greet('World') = " + greet("World"))
    println("multiply(3.5, 2.0) = " + multiply(3.5, 2.0))
}

fn test_imported_constants() = {
    println("Testing imported constants...")
    println("PI = " + PI)
    println("MAX_SIZE = " + MAX_SIZE)
    println("WELCOME_MESSAGE = " + WELCOME_MESSAGE)
}

fn test_imported_types() = {
    println("Testing imported types...")
    
    // Test Point type
    let origin = create_point(0.0, 0.0)
    let target = create_point(5.0, 12.0)
    println("Origin: (" + origin.x + ", " + origin.y + ")")
    println("Target: (" + target.x + ", " + target.y + ")")
    println("Distance from origin to target: " + distance(origin, target))
    
    // Test User type
    let test_user = create_user(42, "Charlie", "charlie@test.com")
    println("Test user: " + test_user.name + " (ID: " + test_user.id + ")")
    println("User valid: " + validate_user(test_user))
    
    // Test invalid user
    let invalid_user = create_user(0, "", "")
    // println("Invalid user valid: " + validate_user(invalid_user))
}

fn test_complex_functions() = {
    println("Testing complex imported functions...")
    println("factorial(6) = " + factorial(6))
    println("fibonacci(8) = " + fibonacci(8))
    println("circle_area(10.0) = " + circle_area(10.0))
}

fn test_combinations() = {
    println("Testing combinations of imported items...")
    
    // Use multiple imported items together
    let radius = 7.0
    let area = circle_area(radius)
    let user = create_user(1, "Math User", "math@example.com")
    
    println("Circle with radius " + radius + " has area " + area)
    println("Created user: " + user.name)
    
    // Test with constants
    let max_factorial = factorial(5)  // Using imported function
    if max_factorial < MAX_SIZE => {
        println("Factorial result is within MAX_SIZE")
    } else => {
        println("Factorial result exceeds MAX_SIZE")
    }
}

fn main() = {
    println("Feature 2: Cross-file Import Tests")
    println("=================================")
    
    test_imported_functions()
    println()
    
    test_imported_constants()
    println()
    
    test_imported_types()
    println()
    
    test_complex_functions()
    println()
    
    test_combinations()
    println()
    
    println("All Feature 2 import tests completed!")
}

// Execute the tests
main() 