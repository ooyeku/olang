// Olang v0.13 Feature Examples
// Demonstrates all three major v0.13 features

// =============================================================================
// FEATURE 1: Enhanced String Literals & Template Interpolation
// =============================================================================

// Basic template interpolation
let name = "Alice"
let age = 25
let greeting = `Hello ${name}! You are ${age} years old.`
println(greeting)

// Advanced escape sequences
let unicode_demo = "Emoji: \u{1F600} Unicode: \u{03B1}\u{03B2}\u{03B3}"
let hex_demo = "Hex chars: \x41\x42\x43 = ABC"
let null_demo = "Null char: \0"

// Raw strings (no escape processing)
let file_path = r"C:\Users\Alice\Documents\file.txt"
let regex_pattern = r"^\d+\.\d+$"
let json_template = r"{\"name\": \"${name}\", \"age\": ${age}}"

// Complex template interpolation
let user = {
    name: "Bob",
    score: 1250,
    level: 15
}

let status_message = `Player ${user.name} reached level ${user.level} with ${user.score} points!`
let calculation_demo = `Math: 2 + 2 = ${2 + 2}, 10 * 5 = ${10 * 5}`

// =============================================================================
// FEATURE 2: Advanced Pattern Matching with Guards & Ranges
// =============================================================================

// Range patterns
fn classify_number(n: Int) -> String = {
    match n {
        1..=10 => "single digit",
        11..=99 => "double digit", 
        100..=999 => "triple digit",
        _ => "large number"
    }
}

// Or-patterns (multiple patterns in one arm)
fn handle_result(result) = {
    match result {
        Ok(data) | Some(data) => `Success: ${data}`,
        Err(error) | None => `Error: No data available`
    }
}

// Guard clauses (conditional pattern matching)
fn categorize_user(user) = {
    match user {
        User { age } if age >= 65 => "senior",
        User { age } if age >= 18 => "adult", 
        User { age } if age >= 13 => "teenager",
        User { age } if age > 0 => "child",
        _ => "invalid user"
    }
}

// Complex pattern combinations
fn process_data(data) = {
    match data {
        [first, second, ...rest] if first > second => `Descending: ${first} > ${second}`,
        [first, second, ...rest] if first < second => `Ascending: ${first} < ${second}`,
        [single] if single > 100 => `Large single value: ${single}`,
        [] => "Empty list",
        _ => "Other pattern"
    }
}

// Character range patterns
fn classify_char(c: String) = {
    match c {
        'a'..'z' => "lowercase letter",
        'A'..'Z' => "uppercase letter", 
        '0'..'9' => "digit",
        _ => "other character"
    }
}

// =============================================================================
// FEATURE 3: Union Types & Enhanced Type System
// =============================================================================

// Simple union types
type Status = "pending" | "running" | "completed" | "failed"
type Number = Int | Float
type OptionalString = String | "none"

// Complex union types with anonymous structs
type ApiResponse<T> = {
    success: true,
    data: T
} | {
    success: false,
    error: String,
    code: Int
}

// Generic union types
type Result<T, E> = {
    ok: true,
    value: T
} | {
    ok: false,
    error: E
}

// Union types with literal types
type HttpMethod = "GET" | "POST" | "PUT" | "DELETE" | "PATCH"
type LogLevel = "DEBUG" | "INFO" | "WARN" | "ERROR"

// Functions using union types
fn process_status(status: Status) -> String = {
    match status {
        "pending" => "⏳ Waiting to start",
        "running" => "🏃 In progress", 
        "completed" => "✅ Done",
        "failed" => "❌ Error occurred"
    }
}

fn handle_api_response<T>(response: ApiResponse<T>) -> String = {
    match response {
        { success: true, data } => `Success: ${data}`,
        { success: false, error, code } => `Error ${code}: ${error}`
    }
}

fn make_http_request(method: HttpMethod, url: String) -> ApiResponse<String> = {
    match method {
        "GET" => { success: true, data: `GET request to ${url}` },
        "POST" => { success: true, data: `POST request to ${url}` },
        _ => { success: false, error: "Method not implemented", code: 501 }
    }
}

// =============================================================================
// COMBINED FEATURE DEMONSTRATION
// =============================================================================

// User management system showcasing all v0.13 features
type UserRole = "admin" | "moderator" | "user" | "guest"

type User = {
    id: Int,
    name: String,
    email: String,
    age: Int,
    role: UserRole
}

type UserOperation<T> = {
    success: true,
    user: User,
    data: T
} | {
    success: false,
    message: String,
    code: Int
}

fn create_user_profile(name: String, email: String, age: Int, role: UserRole) -> UserOperation<String> = {
    // Template interpolation for validation messages
    let validation_result = match (name, email, age) {
        (n, e, a) if n == "" => { success: false, message: "Name cannot be empty", code: 400 },
        (n, e, a) if a < 0 => { success: false, message: `Invalid age: ${a}`, code: 400 },
        (n, e, a) if a > 150 => { success: false, message: `Unrealistic age: ${a}`, code: 400 },
        _ => { success: true, message: "Valid", code: 200 }
    }
    
    match validation_result {
        { success: false, message, code } => { 
            success: false, 
            message: `Validation failed: ${message}`, 
            code: code 
        },
        _ => {
            let user = User {
                id: 1000,
                name: name,
                email: email,
                age: age,
                role: role
            }
            
            // Complex template with user data
            let profile_summary = `User Profile Created:
Name: ${user.name}
Email: ${user.email} 
Age: ${user.age} years old
Role: ${user.role}
Status: ${process_user_status(user)}`
            
            { success: true, user: user, data: profile_summary }
        }
    }
}

fn process_user_status(user: User) -> String = {
    // Advanced pattern matching with guards and ranges
    match user {
        User { role: "admin", age } if age >= 21 => "Senior Administrator",
        User { role: "admin", age } => "Junior Administrator", 
        User { role: "moderator", age } if age >= 18 => "Active Moderator",
        User { role: "user" | "guest", age } if age >= 65 => "Senior Member",
        User { role: "user" | "guest", age } if age >= 18 => "Regular Member",
        User { age } if age >= 13 => "Young Member",
        _ => "Restricted Account"
    }
}

// Example usage
let admin_result = create_user_profile("Alice Johnson", "alice@example.com", 28, "admin")
let user_result = create_user_profile("Bob Smith", "bob@example.com", 22, "user")

// Raw string for file paths and regex
let config_path = r"C:\Program Files\MyApp\config.json"
let email_regex = r"^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}$"

// Template strings for logging
let log_entry = `[${dates.now()}] User operation completed: ${admin_result.success ? "SUCCESS" : "FAILED"}`

println("=== Olang v0.13 Features Demo Complete ===")
println("All three major features demonstrated:")
println("1. ✅ Enhanced String Literals & Template Interpolation")  
println("2. ✅ Advanced Pattern Matching with Guards & Ranges")
println("3. ✅ Union Types & Enhanced Type System") 