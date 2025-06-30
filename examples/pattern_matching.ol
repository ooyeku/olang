// Advanced Pattern Matching Examples

// Range patterns - classify numbers
fn classify_number(n: Int) -> String = {
    match n {
        1..=9 => "single digit",
        10..=99 => "double digit",
        100..=999 => "triple digit", 
        1000..=9999 => "four digits",
        _ => "very large number"
    }
}

println(classify_number(5))     // single digit
println(classify_number(42))    // double digit  
println(classify_number(789))   // triple digit

// Character range patterns
fn classify_character(c: String) -> String = {
    match c {
        'a'..'z' => "lowercase letter",
        'A'..'Z' => "uppercase letter",
        '0'..'9' => "digit",
        _ => "special character"
    }
}

// Or-patterns - handle multiple similar cases
fn handle_response(result) -> String = {
    match result {
        Ok(data) | Some(data) => `Got data: ${data}`,
        Err(err) | None => "No data available"
    }
}

// Guard clauses - conditional matching
fn categorize_age(age: Int) -> String = {
    match age {
        a if a < 0 => "invalid age",
        a if a <= 12 => "child", 
        a if a <= 17 => "teenager",
        a if a <= 64 => "adult",
        a if a <= 120 => "senior",
        _ => "unrealistic age"
    }
}

// Complex pattern with guards
fn analyze_list(numbers: [Int]) -> String = {
    match numbers {
        [] => "empty list",
        [x] if x > 100 => `single large number: ${x}`,
        [x] => `single number: ${x}`,
        [first, second] if first > second => `descending pair: ${first}, ${second}`,
        [first, second] if first < second => `ascending pair: ${first}, ${second}`,
        [first, second] => `equal pair: ${first}, ${second}`,
        [first, ...rest] if first > 50 => `starts with large number: ${first}`,
        _ => "complex list"
    }
}

// Struct patterns with guards
type User = struct {
    name: String,
    age: Int,
    role: String
}

fn get_permissions(user: User) -> String = {
    match user {
        User { role: "admin", age } if age >= 21 => "full admin access",
        User { role: "admin", age } => "limited admin access",
        User { role: "moderator", age } if age >= 18 => "moderation tools",
        User { role: "user" | "guest", age } if age >= 13 => "basic access",
        _ => "restricted access"
    }
}

// Result pattern matching with guards
fn process_result(result, retry_count: Int) -> String = {
    match result {
        Ok(data) => `Success: ${data}`,
        Err(error) if retry_count < 3 => `Retrying... (attempt ${retry_count + 1})`,
        Err(error) => `Failed after retries: ${error}`
    }
}

// Nested pattern matching
fn analyze_nested_data(data) -> String = {
    match data {
        { status: "ok", data: [first, ...rest] } if first > 0 => "valid data with positive start",
        { status: "ok", data: [] } => "valid but empty data",
        { status: "error", message } => `Error occurred: ${message}`,
        _ => "unknown data format"
    }
}

// Example usage
let alice = User { name: "Alice", age: 25, role: "admin" }
let bob = User { name: "Bob", age: 16, role: "user" }

println(get_permissions(alice))  // full admin access
println(get_permissions(bob))    // basic access
println(analyze_list([100, 50, 25]))  // descending pair: 100, 50 