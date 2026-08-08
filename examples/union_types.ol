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
        { shapetype: "circle", radius: radius } => 3.14159 * radius * radius,
        { shapetype: "rectangle", width: width, height: height } => width * height,
        { shapetype: "triangle", base: base, height: height } => 0.5 * base * height,
        _ => 0.0
    }
}

// API result handling
fn handle_api_result(result) -> String = {
    match result {
        { success: true, data: data, timestamp: timestamp } => `Success at ${timestamp}: ${data}`,
        { success: false, errormsg: errormsg, code: code } => `Error ${code}: ${errormsg}`,
        _ => "Unknown result format"
    }
}

// Working with flexible data types using discriminated unions
fn process_value(value) -> String = {
    match value {
        { valuetype: "string", value: value } => `String: "${value}"`,
        { valuetype: "int", value: value } => `Integer: ${value}`,
        { valuetype: "float", value: value } => `Float: ${value}`,
        _ => "Unknown value type"
    }
}

// HTTP response handling
fn handle_http_response(response) -> String = {
    match response {
        { status: 200, body: body, headers: headers } => `OK: ${body}`,
        { status: 404, message: message } => `Not found: ${message}`,
        { status: 500, message: message, debuginfo: debuginfo } => `Server error: ${message} (${debuginfo})`,
        _ => "Unknown response format"
    }
}

// Database operation results
fn process_db_result(result) -> String = {
    match result {
        { success: true, data: data, affectedrows: affectedrows } => {
            `Database operation successful: ${affectedrows} rows affected`
        },
        { success: false, errortype: "connection", message: message } => {
            `Connection error: ${message}`
        },
        { success: false, errortype: "syntax", message: message } => {
            `SQL syntax error: ${message}`
        },
        { success: false, errortype: "permission", message: message } => {
            `Permission denied: ${message}`
        },
        _ => "Unknown database result"
    }
}

// User authentication system
fn handle_auth(auth) -> String = {
    match auth {
        { authenticated: true, user: user, token: token, expires: expires } => {
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
let taskstatus = "running"
println(format_status(taskstatus))  // In Progress

let circle = { shapetype: "circle", radius: 5.0 }
let area = calculate_area(circle)
println(`Circle area: ${area}`)  // Circle area: 78.53975

let apisuccess = {
    success: true,
    data: "User data retrieved",
    timestamp: "2024-01-15T10:30:00Z"
}

let apierror = {
    success: false,
    errormsg: "Database connection failed",
    code: 500
}

println(handle_api_result(apisuccess))
println(handle_api_result(apierror))

// Test StringOrNumber with discriminated unions
let stringvalue = { valuetype: "string", value: "Hello World" }
let intvalue = { valuetype: "int", value: 42 }
let floatvalue = { valuetype: "float", value: 3.14159 }

println(process_value(stringvalue))   // String: "Hello World"
println(process_value(intvalue))      // Integer: 42
println(process_value(floatvalue))    // Float: 3.14159

// HTTP response examples
let successresponse = {
    status: 200,
    body: "Welcome to the API!",
    headers: ["Content-Type: application/json"]
}

let notfound = {
    status: 404,
    message: "Resource not found"
}

println(handle_http_response(successresponse))
println(handle_http_response(notfound))

// Authentication examples
let successfulauth = {
    authenticated: true,
    user: {
        id: 123,
        username: "alice",
        role: "admin"
    },
    token: "jwt-token-here",
    expires: "2024-12-31T23:59:59Z"
}

let failedauth = {
    authenticated: false,
    reason: "invalid_credentials"
}

println(handle_auth(successfulauth))
println(handle_auth(failedauth))
