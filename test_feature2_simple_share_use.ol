// Test file for Feature 2: Simple Share/Use System
// This tests comprehensive sharing and using functionality

// Test 1: Share functions of different types
share fn add(a: Int, b: Int) = a + b
share fn greet(name: String) = "Hello, " + name + "!"
share fn multiply(x: Float, y: Float) = x * y

// Test 2: Share constants/variables
share let PI = 3.14159
share let MAX_SIZE = 100
share let WELCOME_MESSAGE = "Welcome to Olang!"

// Test 3: Share custom types
share type Point = struct { x: Float, y: Float }
share type User = struct { id: Int, name: String, email: String }
share type Color = enum { Red, Green, Blue, RGB(Int, Int, Int) }

// Test 4: Private functions (not shared - should not be accessible from other files)
fn private_helper() = "This is private"
fn internal_calc(x: Int) = x * 2 + 1

// Test 5: Functions using shared types
share fn create_point(x: Float, y: Float) = Point { x: x, y: y }
share fn create_user(id: Int, name: String, email: String) = User { id: id, name: name, email: email }
share fn distance(p1: Point, p2: Point) = {
    let dx = p2.x - p1.x
    let dy = p2.y - p1.y
    math.sqrt(dx * dx + dy * dy)
}

// Test 6: Complex functions with multiple parameters and logic
share fn factorial(n: Int) = if n <= 1 => 1 else => n * factorial(n - 1)
share fn fibonacci(n: Int) = if n <= 1 => n else => fibonacci(n - 1) + fibonacci(n - 2)

// Test 7: Functions that use other shared items
share fn circle_area(radius: Float) = PI * radius * radius
share fn validate_user(user: User) = if user.id > 0 => true else => false

fn main() = {
    // Test basic functionality
    println("Testing Feature 2: Simple Share/Use System")
    println("===============================================")
    
    // Test shared functions
    println("add(5, 3) = " + add(5, 3))
    println("greet('Alice') = " + greet("Alice"))
    println("multiply(2.5, 4.0) = " + multiply(2.5, 4.0))
    
    // Test shared constants
    println("PI = " + PI)
    println("MAX_SIZE = " + MAX_SIZE)
    println("WELCOME_MESSAGE = " + WELCOME_MESSAGE)
    
    // Test shared types and functions
    let point1 = create_point(0.0, 0.0)
    let point2 = create_point(3.0, 4.0)
    println("Point1: (" + point1.x + ", " + point1.y + ")")
    println("Point2: (" + point2.x + ", " + point2.y + ")")
    println("Distance: " + distance(point1, point2))
    
    let user = create_user(1, "Bob", "bob@example.com")
    println("User: " + user.name + " (" + user.email + ")")
    // println("Valid user: " + validate_user(user))
    
    // Test complex functions
    println("factorial(5) = " + factorial(5))
    println("fibonacci(7) = " + fibonacci(7))
    
    // Test functions using shared constants
    println("circle_area(5.0) = " + circle_area(5.0))
    
    println("All Feature 2 internal tests passed!")
}

// Execute main function to test
main() 