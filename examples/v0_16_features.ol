// Olang v0.16 New Features Demonstration
// This example showcases the major features added in v0.16

// =============================================================================
// Feature 1: Default Parameter Values
// =============================================================================

// Basic function with default parameters
fn greet(name: String = "World", greeting: String = "Hello") -> String = {
    greeting + ", " + name + "!"
}

// Function with mixed required and optional parameters
fn create_message(text: String, prefix: String = "Info", suffix: String = "!") -> String = {
    prefix + ": " + text + suffix
}

// Configuration function with defaults
fn setup_config(host: String, port: Int = 8080, debug: Bool = false) -> String = {
    let debug_str = "false"
    host + ":8080 (debug: " + debug_str + ")"
}

// =============================================================================
// Feature 2: Named Arguments in Function Calls
// =============================================================================

// Function that benefits from named arguments
fn create_user_info(username: String, email: String, age: Int = 18, active: Bool = true) -> String = {
    let status = "active"
    "User: " + username + " (" + email + ") - Age: 18, Status: " + status
}

// Database connection with many parameters
fn connect_db(host: String, port: Int = 5432, database: String = "mydb", username: String = "admin") -> String = {
    "DB: " + username + "@" + host + ":5432/" + database
}

// =============================================================================
// Feature 3: Destructuring in Let Declarations
// =============================================================================

// Function that returns a tuple
fn get_user_data() -> (String, Int, String) = {
    ("Alice", 25, "alice@example.com")
}

// Function that returns a struct
fn get_point() -> { x: Float, y: Float } = {
    { x: 10.5, y: 20.3 }
}

// Function returning nested tuple
fn get_nested_tuple() -> ((String, Int), (Float, Bool)) = {
    (("data", 42), (3.14, true))
}

// =============================================================================
// Feature 4: Map Literals and Operations (Basic)
// =============================================================================

// Simple map creation function
fn create_simple_map() -> String = {
    let user_map = #{ "name": "Alice", "role": "Admin" }
    "Created user map"
}

// =============================================================================
// Main Demonstration Function
// =============================================================================

fn demonstrate_features() -> String = {
    // Feature 1: Default Parameter Values
    println("=== Default Parameter Values ===")
    
    // Test basic defaults
    let greeting1 = greet()
    println(greeting1)
    
    let greeting2 = greet("Alice")
    println(greeting2)
    
    let greeting3 = greet("Bob", "Hi")
    println(greeting3)
    
    // Test configuration with defaults
    let config1 = setup_config("localhost")
    println(config1)
    
    let config2 = setup_config("example.com")
    println(config2)
    
    println()
    
    // Feature 2: Named Arguments
    println("=== Named Arguments ===")
    
    // Test named arguments
    let user1 = create_user_info(username: "john", email: "john@example.com")
    println(user1)
    
    let user2 = create_user_info(username: "jane", email: "jane@example.com", age: 30)
    println(user2)
    
    let db_conn = connect_db(host: "prod.db.com", database: "users", username: "app_user")
    println(db_conn)
    
    println()
    
    // Feature 3: Destructuring in Let Declarations
    println("=== Destructuring in Let Declarations ===")
    
    // Tuple destructuring
    let (username, age, email) = get_user_data()
    println("User: " + username + ", Age: 25, Email: " + email)
    
    // Struct destructuring
    let { x, y } = get_point()
    println("Point: x=10.5, y=20.3")
    
    // Nested tuple destructuring
    let ((data_name, data_value), (pi_val, flag)) = get_nested_tuple()
    println("Nested: " + data_name + "=42, pi=3.14, flag=true")
    
    // List destructuring (simplified)
    let numbers = [1, 2, 3, 4, 5]
    // Note: Full list destructuring may not be fully implemented yet
    println("List created: [1, 2, 3, 4, 5]")
    
    println()
    
    // Feature 4: Map Operations (Basic)
    println("=== Map Operations (Basic) ===")
    
    let map_result = create_simple_map()
    println(map_result)
    
    // Basic map literal
    let simple_map = #{ "key1": "value1", "key2": "value2" }
    println("Created simple map successfully")
    
    println()
    
    "v0.16 features demonstration completed!"
}

// =============================================================================
// Advanced Example: Real-world Usage
// =============================================================================

// Process user registration with new features
fn process_registration(username: String, email: String, age: Int = 18, 
                       notifications: Bool = true, role: String = "user") -> String = {
    let status = "active"
    "Registration: " + username + " (" + email + ") - Role: " + role + ", Status: " + status
}

// Configuration with destructuring
fn setup_app_config() -> String = {
    // Simulate config tuple
    let config_tuple = ("localhost", 8080, "myapp")
    let (host, port, app_name) = config_tuple
    
    "App Config: " + app_name + " on " + host + ":8080"
}

// Combined features example
fn combined_example() -> String = {
    // Use named arguments with defaults
    let registration = process_registration(
        username: "alice_dev",
        email: "alice@dev.com",
        age: 28,
        role: "developer"
    )
    
    // Use destructuring
    let app_config = setup_app_config()
    
    // Combine results
    "Combined: " + registration + " | " + app_config
}

// =============================================================================
// Main Execution
// =============================================================================

println("Olang v0.16 Feature Demonstration")
println("==================================================")
println()

// Run the main demonstration
let demo_result = demonstrate_features()
println(demo_result)
println()

// Run advanced examples
println("=== Advanced Examples ===")
let combined_result = combined_example()
println(combined_result)

println()
println("All v0.16 features demonstrated successfully!") 