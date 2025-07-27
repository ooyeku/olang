# Olang Syntax Documentation

## Overview

This document describes the complete syntax of the Olang programming language based on the current grammar specification and implementation status.

## Table of Contents

1. [Literals](#literals)
2. [Identifiers and Variables](#identifiers-and-variables)
3. [Lists and Tuples](#lists-and-tuples)
4. [Maps](#maps)
5. [Ranges](#ranges)
6. [Functions](#functions)
7. [Control Flow](#control-flow)
8. [Pattern Matching](#pattern-matching)
9. [Operators](#operators)
10. [Type System](#type-system)
11. [Async/Await](#asyncawait)
12. [Loops](#loops)
13. [Module System](#module-system)
14. [Error Handling](#error-handling)
15. [Testing](#testing)
16. [Help System](#help-system)
17. [Implementation Status](#implementation-status)

## Literals

### Numeric Literals

```olang
// Integers
42          // Decimal integer
-123        // Negative integer
1_000_000   // Integer with underscores for readability

// Binary, Octal, and Hexadecimal
0b1010      // Binary: 10
0o755       // Octal: 493
0xFF        // Hexadecimal: 255
-0x1A       // Negative hexadecimal: -26

// Floats
3.14        // Float
-2.5        // Negative float
1.0e10      // Scientific notation
1.5E-3      // Scientific notation with negative exponent
```

### String Literals

```olang
// Regular strings
"Hello"     // String literal
"Hello \"World\""  // String with escaped quotes
"Line 1\nLine 2"   // String with newlines

// Raw strings (no escape processing)
r"Raw string with \n literal backslash"
r"Path: C:\Users\Name\file.txt"

// Template strings (string interpolation)
`Hello ${name}!`
`Sum: ${a + b}`
`Multi-line template
 with ${interpolation}`

// Character literals
'a'         // Single character
'\n'        // Escaped character
```

### String Escape Sequences

```olang
"Quote: \""       // \"
"Backslash: \\"   // \\
"Forward slash: \/" // \/
"Backspace: \b"   // \b
"Form feed: \f"   // \f
"Newline: \n"     // \n
"Carriage return: \r"  // \r
"Tab: \t"         // \t
"Null: \0"        // \0
"Hex: \x41"       // \xXX (2 hex digits)
"Unicode: \u0041" // \uXXXX (4 hex digits)
"Unicode: \u{1F600}" // \u{XXXXXX} (variable length)
```

### Boolean and Unit Literals

```olang
true        // Boolean true
false       // Boolean false
()          // Unit type (empty tuple)
```

### Collection Literals

```olang
// Lists
[1, 2, 3]      // List of integers
["a", "b"]     // List of strings
[]             // Empty list

// Tuples
(1, 2)         // Tuple with two elements
(1, "hello", true)  // Mixed type tuple

// Maps
#{}                    // Empty map
#{"key": "value"}      // String key-value map
#{1: "one", 2: "two"}  // Integer key map
#{                     // Multi-line map
    "name": "Alice",
    "age": 30,
    "active": true
}
```

## Identifiers and Variables

### Variable Declaration

```olang
// Simple variable declaration
let x = 42;
let name = "Alice";

// Variable with type annotation
let age: Int = 25;
let height: Float = 5.8;

// Variable without initial value (defaults to Unit)
let uninitialized;
let with_type: String;
```

### Destructuring Let Declarations

```olang
// Tuple destructuring
let (x, y) = (1, 2);
let (first, second, third) = (10, 20, 30);

// List destructuring
let [head, tail] = [1, 2, 3, 4, 5];
let [first, second, ...rest] = [1, 2, 3, 4, 5];

// Nested destructuring
let ((a, b), c) = ((1, 2), 3);
let [first, [nested_a, nested_b]] = [10, [20, 30]];

// Wildcard patterns
let (x, _) = (42, "ignored");
let [first, _, third] = [1, 2, 3];

// Pattern matching with literals
let (x, 2) = (42, 2);  // Matches when second element is 2

// Struct destructuring
let Point { x, y } = some_point;
let User { name, age, .. } = user_data;  // Ignore remaining fields
```

### Variable Assignment

```olang
// Assignment to existing variable
x = 50;
name = "Bob";
```

## Lists and Tuples

### List Operations

```olang
// List creation
let numbers = [1, 2, 3, 4, 5];
let words = ["hello", "world"];

// List indexing
let first = numbers[0];    // SUPPORTED
let second = words[1];     // SUPPORTED

// List functions (built-in)
let doubled = numbers |> map((x) => x * 2);     // SUPPORTED
let evens = numbers |> filter((x) => x % 2 == 0); // SUPPORTED
let total = numbers |> sum();                   // SUPPORTED
```

### Tuple Operations

```olang
// Tuple creation
let point = (10, 20);
let person = ("Alice", 30, true);

// Tuple indexing
let x = point[0];     // SUPPORTED
let y = point[1];     // SUPPORTED
```

## Maps

### Map Creation

```olang
// Empty map
let empty_map = #{};

// String key maps
let scores = #{
    "Alice": 95,
    "Bob": 87,
    "Charlie": 92
};

// Mixed key types (converted to strings)
let mixed_map = #{
    "string_key": "value",
    42: "number_key",      // Converted to "42"
    true: "boolean_key"    // Converted to "true"
};

// Nested maps
let nested = #{
    "user": #{
        "name": "Alice",
        "settings": #{
            "theme": "dark",
            "notifications": true
        }
    }
};
```

### Map Operations

```olang
// Map access and manipulation
let score = map_get(scores, "Alice");         // Get value -> 95
let updated = map_set(scores, "David", 88);   // Returns new map
let exists = map_has_key(scores, "Bob");      // Check existence -> true
let all_keys = map_keys(scores);              // Get keys -> ["Alice", "Bob", "Charlie"]
let all_values = map_values(scores);          // Get values -> [95, 87, 92]
let size = map_len(scores);                   // Get size -> 3
let removed = map_remove(scores, "Bob");      // Returns map without key
let cleared = map_clear(scores);              // Returns empty map
let merged = map_merge(scores, other_map);    // Merge maps (right overwrites left)
```

### Map Type Annotations

```olang
// Map type annotations
let user_ages: Map<String, Int> = #{"Alice": 25, "Bob": 30};
let config: Map<String, String> = #{"theme": "dark", "lang": "en"};
```

## Ranges

### Range Syntax

```olang
// Exclusive range
let range1 = 1..10;        // [1, 2, 3, 4, 5, 6, 7, 8, 9]

// Inclusive range  
let range2 = 1..=10;       // [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]

// Variable ranges
let start = 5;
let end = 15;
let dynamic_range = start..end;

// Range operations
let doubled = (1..10) |> map((x) => x * 2);  // SUPPORTED
let evens = (1..20) |> filter((x) => x % 2 == 0);  // SUPPORTED
```

## Functions

### Function Declaration

```olang
// Simple function
fn add(x, y) = x + y;

// Function with type annotations
fn multiply(x: Int, y: Int) -> Int = x * y;

// Function with multiple parameters
fn greet(name: String, age: Int) -> String = 
    "Hello " + name + ", you are " + to_string(age);
```

### Default Parameter Values

```olang
// Function with default values
fn greet(name: String = "World") = "Hello, " + name;

// Multiple default parameters
fn connect(host: String = "localhost", port: Int = 8080, timeout: Int = 30) = {
    "Connecting to " + host + ":" + to_string(port) + " (timeout: " + to_string(timeout) + "s)"
};

// Mixed required and default parameters
fn create_user(name: String, email: String, role: String = "user", active: Bool = true) = {
    User { name, email, role, active }
};

// Function calls with defaults
let result1 = greet();                    // Uses default: "Hello, World"
let result2 = greet("Alice");             // "Hello, Alice"
let result3 = connect();                  // Uses all defaults
let result4 = connect("example.com");     // Custom host, default port/timeout
let result5 = connect("example.com", 9000); // Custom host and port
```

### Named Arguments in Function Calls

```olang
// Function with named arguments
fn process_data(filename: String, format: String = "json", compress: Bool = false, timeout: Int = 60) = {
    // Implementation
};

// Named argument calls
let result1 = process_data("data.txt", format: "csv", compress: true);
let result2 = process_data("data.txt", timeout: 120, format: "xml");
let result3 = process_data(filename: "data.txt", compress: true);

// Database connection example
fn connect_db(host: String, port: Int = 5432, username: String, password: String, database: String = "mydb") = {
    // Implementation
};

let conn = connect_db(
    host: "localhost",
    username: "admin",
    password: "secret",
    database: "production"
);

// Mixed positional and named arguments
fn send_email(to: String, subject: String, body: String, priority: String = "normal", html: Bool = false) = {
    // Implementation
};

let email_result = send_email("user@example.com", "Important Update", 
    "This is the message body", 
    priority: "high", 
    html: true
);
```

### Lambda Functions

```olang
// Simple lambda
let square = (x) => x * x;

// Lambda with type annotation
let divide: (Float, Float) -> Float = (x, y) => x / y;

// Lambda with no parameters
let get_pi = () => 3.14159;

// Lambda with block body
let complex_calc = (x) => {
    let doubled = x * 2;
    let squared = doubled * doubled;
    squared + 1
};
```

### Async Functions

```olang
// Async function declaration
async fn fetch_data(url: String) -> Promise<String, String> = {
    // Implementation
    Promise.resolve("data")
};

// Async lambda
let async_compute = async (x) => {
    let result = await some_async_operation(x);
    result * 2
};
```

## Control Flow

### Conditional Expressions

```olang
// Simple if-else
let result = if x > 0 => "positive" else => "non-positive";

// If without else
let msg = if is_valid => "Valid input";

// If with block bodies
let category = if score >= 90 => {
    "Excellent"
} else => {
    if score >= 70 => "Good" else => "Needs improvement"
};
```

## Pattern Matching

### Match Expressions

```olang
// Simple pattern matching
let result = match value {
    1 => "one",
    2 => "two",
    _ => "other"
};

// Pattern matching with variables
let description = match person {
    ("Alice", age) => "Alice is " + to_string(age),
    (name, _) => "Person named " + name
};

// List pattern matching
let head_tail = match numbers {
    [] => "empty",
    [x] => "single: " + to_string(x),
    [first, second] => "pair: " + to_string(first) + ", " + to_string(second),
    _ => "many"
};
```

### Result Pattern Matching

```olang
// Result pattern matching
let process_result = match operation_result {
    Ok(value) => "Success: " + to_string(value),
    Err(error) => "Error: " + error
};
```

### Struct Pattern Matching

```olang
// Struct pattern matching
let info = match user {
    User { name: "admin", .. } => "Administrator",
    User { name, age } => name + " (" + to_string(age) + ")",
    _ => "Unknown user"
};
```

## Operators

### Arithmetic Operators

```olang
// Basic arithmetic
let sum = a + b;
let difference = a - b;
let product = a * b;
let quotient = a / b;
let remainder = a % b;
```

### Comparison Operators

```olang
// Comparison
let equal = a == b;
let not_equal = a != b;
let less_than = a < b;
let less_equal = a <= b;
let greater_than = a > b;
let greater_equal = a >= b;
```

### Logical Operators

```olang
// Boolean logic
let and_result = a && b;
let or_result = a || b;
let not_result = !a;
```

### Bitwise Operators

```olang
// Bitwise operations
let and_result = a & b;    // Bitwise AND
let or_result = a | b;     // Bitwise OR
let xor_result = a ^ b;    // Bitwise XOR
let left_shift = a << 2;   // Left shift by 2
let right_shift = a >> 1;  // Right shift by 1
```

### Pipeline Operator

```olang
// Pipeline chaining
let result = data
    |> map((x) => x * 2)
    |> filter((x) => x > 10)
    |> sum();

// Complex pipeline
let processed = input
    |> parse_json()
    |> get_field("items")
    |> map((item) => item.name)
    |> filter((name) => len(name) > 5)
    |> sort()
    |> join(", ");

// Map operations in pipelines
let user_names = users
    |> map((user) => map_get(user, "name"))
    |> filter((name) => len(name) > 3);
```

## Type System

### Type Annotations

```olang
// Basic types
let count: Int = 42;
let price: Float = 9.99;
let name: String = "Alice";
let active: Bool = true;

// Container types
let numbers: [Int] = [1, 2, 3];
let coords: (Float, Float) = (10.5, 20.3);
let user_data: Map<String, String> = #{"name": "Alice", "role": "admin"};

// Function types
let calculator: (Int, Int) -> Int = (a, b) => a + b;

// Generic types
let items: List<String> = ["a", "b", "c"];

// Result types
let operation: Result<Int, String> = Ok(42);

// Promise types
let async_data: Promise<String, String> = fetch_data("url");

// Union types
let flexible: Int | String = 42;
let flexible2: Int | String = "hello";

// Intersection types
let numeric: Int & Comparable = 42;

// Literal types
let specific: "admin" | "user" = "admin";
let magic_number: 42 = 42;
```

### Custom Types

```olang
// Struct definition
type User = struct {
    name: String,
    age: Int,
    email: String
};

// Enum definition
type Color = enum {
    Red,
    Green,
    Blue,
    RGB(Int, Int, Int)
};

// Generic type definition
type Maybe<T> = enum {
    Some(T),
    None
};

// Union type definition
type Flexible = Int | String | Bool;

// Error type declaration
error NetworkError {
    Timeout,
    ConnectionFailed: String,
    InvalidResponse: {
        status: Int,
        message: String
    }
};
```

## Async/Await

### Async Operations

```olang
// Async function calls
let data = await fetch_data("https://api.example.com");

// Promise creation
let promise1 = Promise.resolve(42);
let promise2 = Promise.reject("Error message");
let delayed = Promise.delay(1000, "Hello");

// Concurrent operations
let results = await Promise.all([
    fetch_user(1),
    fetch_user(2),
    fetch_user(3)
]);

let winner = await Promise.race([
    fetch_from_cache(),
    fetch_from_network()
]);

// Spawn operation
let task = spawn async_computation(data);
```

## Loops

### For Loops

```olang
// For loop over list
for item in [1, 2, 3, 4] {
    println(item);
}

// For loop over range
for i in 0..10 {
    println("Number: " + to_string(i));
}

// For loop with variable
for name in names {
    println("Hello, " + name);
}
```

### While Loops

```olang
// While loop
let counter = 0;
while counter < 10 {
    println(counter);
    counter = counter + 1;
}
```

### Infinite Loops

```olang
// Infinite loop with break
loop {
    let input = read_input();
    if input == "quit" {
        break;
    }
    process(input);
}
```

### Loop Control

```olang
// Break and continue
for i in 0..100 {
    if i % 2 == 0 {
        continue;  // Skip even numbers
    }
    if i > 50 {
        break;     // Stop after 50
    }
    println(i);
}
```

## Module System

### Share Declarations

```olang
// Share function
share fn public_function() = "I'm public";

// Share variable
share let public_constant = 42;

// Share type
share type PublicType = struct {
    field: String
};

// Share use declaration (transitive sharing)
share use module { function1, function2 };
```

### Use Declarations

```olang
// Import specific items from module
use module_name { function1, function2, type1 };

// Import from nested module
use parent.child { item1, item2 };

// Import from multiple levels
use utils.math { add, subtract };
use utils.string { join, split };
```

## Error Handling

### Result Types

```olang
// Result creation
let success: Result<Int, String> = Ok(42);
let failure: Result<Int, String> = Err("Something went wrong");

// Result handling
let processed = match result {
    Ok(value) => value * 2,
    Err(error) => {
        println("Error: " + error);
        0
    }
};
```

### Try-Catch

```olang
// Try-catch expression
let result = try {
    risky_operation()
} catch (error) {
    println("Caught error: " + error);
    default_value()
};
```

### Try Operator

```olang
// Try operator (postfix ?)
let result = risky_operation()?;
```

## Testing

### Test Declarations

```olang
// Basic test
test "addition test" {
    let result = add(2, 3);
    assert_eq(result, 5);
}

// Test with custom message
test "string concatenation" {
    let result = "Hello" + " " + "World";
    assert_eq(result, "Hello World", "String concatenation failed");
}

// Test with assertions
test "boolean operations" {
    let value = true;
    assert_true(value);
    assert_false(!value);
    assert(value == true);
}

// Test with inequality
test "inequality test" {
    let a = 10;
    let b = 20;
    assert_ne(a, b, "Values should not be equal");
}
```

### Assertion Functions

```olang
// Equality assertion
assert_eq(actual, expected);
assert_eq(actual, expected, "Custom message");

// Inequality assertion
assert_ne(actual, expected);
assert_ne(actual, expected, "Custom message");

// Boolean assertion
assert(condition);
assert(condition, "Custom message");

// True/False assertions
assert_true(expression);
assert_true(expression, "Custom message");
assert_false(expression);
assert_false(expression, "Custom message");
```

## Help System

### Enhanced Help Commands

```olang
// Basic help
:help                    // Show general help
:help println           // Show function help
:help list               // Show category help

// Advanced search with fuzzy matching
:help "print"           // Fuzzy search for print-related functions
:help "http"            // Find HTTP-related functions
:help "json"            // Find JSON operations

// Interactive tutorials
:tutorial               // List available tutorials
:tutorial_run basic     // Run basic tutorial
:tutorial_run lists     // Run list operations tutorial
:tutorial_run http      // Run HTTP client tutorial

// Context-sensitive help
:help_context          // Get help based on current context
:help_suggest          // Get suggestions for common operations
```

### Help System Features

1. **Fuzzy Search**: Case-insensitive search with similarity matching
2. **Categorized Help**: Functions organized by category (fs, http, math, etc.)
3. **Interactive Tutorials**: Step-by-step guided learning
4. **Context-Sensitive Suggestions**: Help based on current REPL state
5. **Enhanced Examples**: Runnable code examples with explanations
6. **Search Filters**: Filter by category, function type, or description

## Implementation Status

### Completed Features

1. **Core Language Features**
   - All literal types (integers, floats, strings, booleans, lists, tuples, maps)
   - Binary, octal, and hexadecimal number literals
   - Raw strings and template strings with interpolation
   - Character literals
   - Default Parameter Values in Function Declarations
   - Named Arguments in Function Calls
   - Destructuring in Let Declarations
   - Map Literals and Operations
   - Bitwise operations
   - Test declarations and assertions

2. **Type System**
   - Basic type annotations
   - Union and intersection types
   - Generic types
   - Result and Promise types
   - Literal types
   - Custom type definitions (structs, enums)
   - Error type declarations

3. **Control Flow**
   - If expressions
   - Pattern matching with match expressions
   - For, while, and infinite loops
   - Break and continue statements

4. **Functions**
   - Function declarations with type annotations
   - Lambda functions
   - Async functions
   - Default parameters
   - Named arguments

5. **Operators**
   - Arithmetic operators
   - Comparison operators
   - Logical operators
   - Bitwise operators
   - Pipeline operator

6. **Error Handling**
   - Result types
   - Try-catch expressions
   - Try operator

7. **Module System**
   - Share declarations
   - Use declarations
   - Module imports

8. **Standard Library**
   - Complete Stdlib Implementation (10 modules)
   - All stdlib modules fully tested

9. **Testing**
   - Test declarations
   - Assertion functions
   - Test framework integration

10. **Developer Experience**
    - Enhanced Help System
    - Interactive Tutorials
    - Better REPL Error Messages

### In Progress Features

1. **Advanced Type System**
   - Generic type constraints
   - Type inference improvements
   - Complex type validation

2. **Performance Optimizations**
   - OVM memory management refinements
   - Lazy evaluation edge cases

### Known Limitations

1. **Module System**
   - Module resolution has verbose debug output
   - No module caching implemented
   - Import/export needs better error handling

2. **Advanced Type Features**
   - Generic type constraints partially implemented
   - Some complex type scenarios not fully supported

3. **Performance**
   - Large recursive operations may cause stack overflow
   - Some OVM optimizations still being refined

### Testing Strategy

#### Core Feature Testing

```olang
// Test file: test_comprehensive_features.ol

// Default parameters
fn test_defaults(name: String = "World", count: Int = 1) = 
    "Hello " + name + " x" + to_string(count);

// Named arguments
let result = test_defaults(count: 5, name: "Alice");

// Destructuring
let (x, y) = (10, 20);
let [first, ...rest] = [1, 2, 3, 4, 5];

// Map operations
let scores = #{"Alice": 95, "Bob": 87};
let alice_score = map_get(scores, "Alice");
let updated_scores = map_set(scores, "Charlie", 92);

// Bitwise operations
let bitwise_and = 5 & 3;  // 1
let bitwise_or = 5 | 3;   // 7
let bitwise_xor = 5 ^ 3;  // 6
let left_shift = 1 << 2;  // 4
let right_shift = 8 >> 1; // 4

// Pipeline with ranges
let processed = (1..100) 
    |> filter((x) => x % 2 == 0)
    |> map((x) => x * x)
    |> sum();

// Test assertions
test "comprehensive test" {
    assert_eq(bitwise_and, 1);
    assert_ne(bitwise_or, bitwise_xor);
    assert_true(processed > 0);
    assert_false(processed < 0);
}
```

#### Help System Testing

```olang
// REPL commands for testing help system
:help map_get                    // Function help
:help "json"                     // Fuzzy search
:tutorial_run basic             // Interactive tutorial
:help_context                   // Context-sensitive help
```

This comprehensive syntax documentation reflects the current state of Olang, including all completed features and known limitations. The language has evolved significantly with the addition of comprehensive literal support, bitwise operations, test declarations, enhanced type system, and an improved developer experience. 