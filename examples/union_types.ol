// Union Types & Enhanced Type System Examples

// Simple union types with literal values
type Status = "pending" | "running" | "completed" | "failed"
type Priority = "low" | "medium" | "high" | "urgent"
type HttpStatus = 200 | 404 | 500 | 503

// Union types mixing different base types
type StringOrNumber = String | Int | Float
type OptionalData = String | "none" | "empty"

// Complex union types with anonymous structs
type ApiResult<T> = {
    success: true,
    data: T,
    timestamp: String
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

type Option<T> = {
    some: true,
    value: T
} | {
    some: false
}

// Union types for different data shapes
type Shape = {
    type: "circle",
    radius: Float
} | {
    type: "rectangle", 
    width: Float,
    height: Float
} | {
    type: "triangle",
    base: Float,
    height: Float
}

// Functions working with union types
fn format_status(status: Status) -> String = {
    match status {
        "pending" => "⏳ Waiting",
        "running" => "🏃 In Progress", 
        "completed" => "✅ Done",
        "failed" => "❌ Error"
    }
}

fn calculate_area(shape: Shape) -> Float = {
    match shape {
        { type: "circle", radius } => 3.14159 * radius * radius,
        { type: "rectangle", width, height } => width * height,
        { type: "triangle", base, height } => 0.5 * base * height
    }
}

fn handle_api_result<T>(result: ApiResult<T>) -> String = {
    match result {
        { success: true, data, timestamp } => `Success at ${timestamp}: ${data}`,
        { success: false, error, code } => `Error ${code}: ${error}`
    }
}

// Working with flexible data types
fn process_value(value: StringOrNumber) -> String = {
    match value {
        s: String => `String: "${s}"`,
        i: Int => `Integer: ${i}`,
        f: Float => `Float: ${f}`
    }
}

// HTTP response handling
type HttpResponse = {
    status: 200,
    body: String,
    headers: [String]
} | {
    status: 404,
    message: "Not Found"
} | {
    status: 500,
    message: "Internal Server Error",
    debug_info: String
}

fn handle_http_response(response: HttpResponse) -> String = {
    match response {
        { status: 200, body, headers } => `OK: ${body}`,
        { status: 404, message } => `Not found: ${message}`,
        { status: 500, message, debug_info } => `Server error: ${message} (${debug_info})`
    }
}

// Database operation results
type DatabaseResult<T> = {
    success: true,
    data: T,
    affected_rows: Int
} | {
    success: false,
    error_type: "connection" | "syntax" | "permission",
    message: String
}

fn process_db_result<T>(result: DatabaseResult<T>) -> String = {
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
        }
    }
}

// User authentication system
type UserRole = "guest" | "user" | "admin" | "superuser"

type AuthResult = {
    authenticated: true,
    user: {
        id: Int,
        username: String,
        role: UserRole
    },
    token: String,
    expires: String
} | {
    authenticated: false,
    reason: "invalid_credentials" | "account_locked" | "expired_token"
}

fn handle_auth(auth: AuthResult) -> String = {
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
        }
    }
}

// Example usage
let task_status: Status = "running"
println(format_status(task_status))  // 🏃 In Progress

let circle: Shape = { type: "circle", radius: 5.0 }
let area = calculate_area(circle)
println(`Circle area: ${area}`)  // Circle area: 78.53975

let api_success: ApiResult<String> = {
    success: true,
    data: "User data retrieved",
    timestamp: "2024-01-15T10:30:00Z"
}

let api_error: ApiResult<String> = {
    success: false,
    error: "Database connection failed",
    code: 500
}

println(handle_api_result(api_success))
println(handle_api_result(api_error)) 