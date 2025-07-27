// Simplified Union Types Example - Working Around Type Declaration Limitations

// Functions working with union-like patterns without type declarations

// Status handling function
share fn format_status(status) -> String = {
    match status {
        "pending" => "Waiting",
        "running" => "In Progress", 
        "completed" => "Done",
        "failed" => "Error",
        _ => "Unknown status"
    }
}

// Shape calculation with discriminated unions
share fn calculate_area(shape) -> Float = {
    match shape {
        { type: "circle", radius } => 3.14159 * radius * radius,
        { type: "rectangle", width, height } => width * height,
        { type: "triangle", base, height } => 0.5 * base * height,
        _ => 0.0
    }
}

// API result handling
fn handle_api_result(result) -> String = {
    match result {
        { success: true, data, timestamp } => `Success at ${timestamp}: ${data}`,
        { success: false, error, code } => `Error ${code}: ${error}`,
        _ => "Unknown result format"
    }
}

// Working with flexible data types using discriminated unions
fn process_value(value) -> String = {
    match value {
        { type: "string", value } => `String: "${value}"`,
        { type: "int", value } => `Integer: ${value}`,
        { type: "float", value } => `Float: ${value}`,
        _ => "Unknown value type"
    }
}

// HTTP response handling
fn handle_http_response(response) -> String = {
    match response {
        { status: 200, body, headers } => `OK: ${body}`,
        { status: 404, message } => `Not found: ${message}`,
        { status: 500, message, debug_info } => `Server error: ${message} (${debug_info})`,
        _ => "Unknown response format"
    }
}

// Database operation results
fn process_db_result(result) -> String = {
    match result {
        { success: true, data, affected_rows } => {
            `Database operation successful: ${affected_rows} rows affected`
        },
        { success: false, error_type: "connection", message } => {
            `Connection error: ${message}`
        },
        { success: false, error_type: "syntax", message } => {
            `SQL syntax error: ${message}`
        },
        { success: false, error_type: "permission", message } => {
            `Permission denied: ${message}`
        },
        _ => "Unknown database result"
    }
}

// User authentication system
fn handle_auth(auth) -> String = {
    match auth {
        { authenticated: true, user, token, expires } => {
            `Welcome ${user.username}! Token expires: ${expires}`
        },
        { authenticated: false, reason: "invalid_credentials" } => {
            "Login failed: Invalid username or password"
        },
        { authenticated: false, reason: "account_locked" } => {
            "Login failed: Account is locked"
        },
        { authenticated: false, reason: "expired_token" } => {
            "Login failed: Token has expired"
        },
        _ => "Unknown authentication result"
    }
}

// Example usage
let task_status = "running"
println(format_status(task_status))  // In Progress

let circle = { type: "circle", radius: 5.0 }
let area = calculate_area(circle)
println(`Circle area: ${area}`)  // Circle area: 78.53975

let api_success = {
    success: true,
    data: "User data retrieved",
    timestamp: "2024-01-15T10:30:00Z"
}

let api_error = {
    success: false,
    error: "Database connection failed",
    code: 500
}

println(handle_api_result(api_success))
println(handle_api_result(api_error))

// Test StringOrNumber with discriminated unions
let string_value = { type: "string", value: "Hello World" }
let int_value = { type: "int", value: 42 }
let float_value = { type: "float", value: 3.14159 }

println(process_value(string_value))   // String: "Hello World"
println(process_value(int_value))      // Integer: 42
println(process_value(float_value))    // Float: 3.14159

// HTTP response examples
let success_response = { 
    status: 200, 
    body: "Welcome to the API!", 
    headers: ["Content-Type: application/json"] 
}

let not_found = { 
    status: 404, 
    message: "Resource not found" 
}

println(handle_http_response(success_response))
println(handle_http_response(not_found))

// Authentication examples  
let successful_auth = {
    authenticated: true,
    user: { 
        id: 123,
        username: "alice",
        role: "admin"
    },
    token: "jwt-token-here",
    expires: "2024-12-31T23:59:59Z"
}

let failed_auth = {
    authenticated: false,
    reason: "invalid_credentials"
}

println(handle_auth(successful_auth))
println(handle_auth(failed_auth)) 