// Feature 1: Automatic File-Based Modules - Complete Test

// Test 1: Share function declaration and access
share fn get_greeting() = "Hello from shared function"

// Test 2: Share let declaration and access  
share let SHARED_MESSAGE = "This is a shared constant"

// Test 3: Share type declaration and usage - using proper struct syntax
share type UserProfile = struct { name: String, age: Int }

// Test 4: Private function (not shared)
fn private_helper() = "This is private"

// Test 5: Use share declarations locally
let profile = UserProfile { name: "Alice", age: 30 }
println("User: " + profile.name + ", Age: " + profile.age)
println("Greeting: " + get_greeting())
println("Message: " + SHARED_MESSAGE)
println("Private: " + private_helper()) 