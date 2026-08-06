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
16. [Help System and REPL](#help-system)
17. [Implementation Status](#implementation-status)

For how Olang executes code (interpreter, bytecode tier, performance), see
[OVM Architecture](ovm.md).

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
let simple = "Hello"
let quoted = "Hello \"World\""      // Escaped quotes
let multiline = "Line 1\nLine 2"    // Newline escape

// Raw strings (no escape processing)
let raw = r"Raw string with \n literal backslash"
let path = r"Path: C:\Users\Name\file.txt"

// Template strings with interpolation
let name = "Alice"
let greeting = `Hello ${name}!`     // "Hello Alice!"
let math = `2 + 2 = ${2 + 2}`       // "2 + 2 = 4"
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

### Boolean

```olang
true        // Boolean true
false       // Boolean false
```

### Character Literals

```olang
let letter = 'a'
let digit = '7'
let space = ' '
```

Character literals are single characters in single quotes. They can be
compared and used in range patterns (`'a'..'z'`) inside `match`.

### Collection Literals

```olang
// Lists
let ints = [1, 2, 3]           // List of integers
let strs = ["a", "b"]          // List of strings
let empty = []                 // Empty list

// Tuples
let pair = (1, 2)              // Tuple with two elements
let mixed = (1, "hello", true) // Mixed type tuple

// Maps
let none = #{}                     // Empty map
let user = #{"key": "value"}       // String key-value map
let ages = #{"ann": 30, "bob": 25} // Multiple entries
```

## Identifiers and Variables

Identifiers start with a letter and may contain letters, digits, and
underscores. The reserved keywords are `fn`, `let`, `type`, `if`, `else`,
`match`, `for`, `while`, `loop`, `break`, `continue`, `true`, `false`,
`async`, `await`, `try`, `catch`, `error`, `share`, `use`, `struct`, `enum`,
and `test` — note that `error` is reserved, so it cannot be used as a
variable or binding name. Names that merely *begin* with a keyword are ordinary
identifiers — `match_count`, `for_each`, and `type_name` are all valid, since
only the exact keyword is reserved.

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
let (x, y) = (1, 2)
let (first, second, third) = (10, 20, 30)

// List destructuring — patterns match the exact length...
let [a, b] = [1, 2]

// ...or use ...rest to capture the remainder
let [head, ...tail] = [1, 2, 3, 4, 5]   // head = 1, tail = [2, 3, 4, 5]

// Nested destructuring
let ((p, q), r) = ((1, 2), 3)
let [outer, [inner_a, inner_b]] = [10, [20, 30]]
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
let scores = #{"Alice": 95, "Bob": 87, "Charlie": 92}

// Map access and manipulation
let score = map_get(scores, "Alice")          // Get value -> 95
let updated = map_set(scores, "David", 88)    // Returns new map
let exists = map_has_key(scores, "Bob")       // Check existence -> true
let all_keys = map_keys(scores)               // Get keys
let all_values = map_values(scores)           // Get values
let size = map_len(scores)                    // Get size -> 3
let removed = map_remove(scores, "Bob")       // Returns map without key
let merged = map_merge(scores, #{"Eve": 99})  // Combine maps
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
fn greet(name: String = "World") = "Hello, " + name

// Multiple default parameters
fn connect(host: String = "localhost", port: Int = 8080, timeout: Int = 30) = {
    "Connecting to " + host + ":" + to_string(port) + " (timeout: " + to_string(timeout) + "s)"
}

// Function calls with defaults
let result1 = greet()                     // Uses default: "Hello, World"
let result2 = greet("Alice")              // "Hello, Alice"
let result3 = connect()                   // Uses all defaults
let result4 = connect("example.com")      // Custom host, default port/timeout
let result5 = connect("example.com", 9000) // Custom host and port
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
let square = (x) => x * x

// Lambda with type annotation
let divide: (Float, Float) -> Float = (x, y) => x / y

// Lambda with no parameters
let get_pi = () => 3.14159

// Lambda with block body
let complex_calc = (x) => {
    let doubled = x * 2
    let squared = doubled * doubled
    squared + 1
}
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
let x = 5
let is_valid = true
let score = 85

// Simple if-else
let result = if x > 0 => "positive" else => "non-positive"

// If without else (evaluates to Unit when false)
let msg = if is_valid => "Valid input"

// If with block bodies
let category = if score >= 90 => {
    "excellent"
} else => {
    "good"
}
```

## Pattern Matching

### Match Expressions

```olang
let value = 2

// Simple pattern matching
let result = match value {
    1 => "one",
    2 => "two",
    _ => "other"
}

// Pattern matching with variable binding
let description = match value {
    0 => "zero",
    n => "the number " + to_string(n)
}
```

### Result Pattern Matching

```olang
let operation_result = Ok(42)

// Result pattern matching (`error` is a reserved word — use `err` or `e`)
let processed = match operation_result {
    Ok(value) => "Success: " + to_string(value),
    Err(err) => "Error: " + err
}
```

### Struct Pattern Matching

```olang
type User = struct { name: String, age: Int }
let user = User { name: "ann", age: 30 }

// Struct pattern matching
let info = match user {
    User { name: "admin" } => "Administrator",
    User { name, age } => name + " (" + to_string(age) + ")",
    _ => "Unknown user"
}
```

## Operators

### Arithmetic Operators

```olang
let a = 10
let b = 3

// Basic arithmetic
let sum = a + b
let difference = a - b
let product = a * b
let quotient = a / b
let remainder = a % b
```

### Comparison Operators

```olang
let a = 10
let b = 3

// Comparison
let equal = a == b
let not_equal = a != b
let less_than = a < b
let less_equal = a <= b
let greater_than = a > b
let greater_equal = a >= b
```

### Logical Operators

```olang
let a = true
let b = false

// Boolean logic
let and_result = a && b
let or_result = a || b
let not_result = !a
```

### Bitwise Operators

```olang
let a = 12
let b = 10

// Bitwise operations
let and_result = a & b;    // Bitwise AND
let or_result = a | b;     // Bitwise OR
let xor_result = a ^ b;    // Bitwise XOR
let left_shift = a << 2;   // Left shift by 2
let right_shift = a >> 1;  // Right shift by 1
```

### Pipeline Operator

```olang
let data = range(1, 20)

// Pipeline chaining
let result = data
    |> map((x) => x * 2)
    |> filter((x) => x > 10)
    |> sum()

// Piping into a partial call: the piped value fills the first parameter
fn add(a, b) = a + b
let bumped = 5 |> add(3)     // add(5, 3) = 8

// Piping a bare function
let total = [1, 2, 3] |> sum
```

## Type System

### Type Annotations

```olang
// Basic types
let count: Int = 42
let price: Float = 9.99
let name: String = "Alice"
let active: Bool = true

// Container types
let numbers: [Int] = [1, 2, 3]
let coords: (Float, Float) = (10.5, 20.3)
let user_data: Map<String, String> = #{"name": "Alice", "role": "admin"}

// Function types
let calculator: (Int, Int) -> Int = (a, b) => a + b

// Generic types
let items: List<String> = ["a", "b", "c"]

// Result types
let operation: Result<Int, String> = Ok(42)

// Union types (as annotations)
let flexible: Int | String = 42
let flexible2: Int | String = "hello"

// Literal types
let specific: "admin" | "user" = "admin"
let magic_number: 42 = 42
```

### Custom Types

```olang
// Struct definition
type User = struct {
    name: String,
    age: Int,
    email: String
}

// Enum definition
type Color = enum {
    Red,
    Green,
    Blue,
    RGB(Int, Int, Int)
}

// Generic type definition (type parameters are erased at runtime)
type Maybe<T> = enum {
    Some(T),
    None
}

// Error type declaration (variants are bare names)
error NetworkError {
    Timeout,
    ConnectionFailed,
    InvalidResponse
}
```

### Constructing and matching enums

Unit variants (`Red`) are values; payload variants (`Circle(2.0)`) are
constructed by applying them like a function. Match binds the payloads:

```olang
type Shape = enum { Circle(Float), Rect(Float, Float), Empty }

let c = Circle(2.0)            // a Shape value
let r = Rect(3.0, 4.0)

fn area(s) = match s {
    Circle(radius)   => 3.14159 * radius * radius,
    Rect(w, h)       => w * h,
    Empty            => 0.0
}

println(area(c))               // 12.56636
```

Enum values compare structurally (`Circle(2.0) == Circle(2.0)` is `true`),
and `typeof` returns the enum's name (`"Shape"`). A generic enum like
`Maybe<T>` constructs for any payload — the type parameter is checked
statically (when type checking is on) and erased at runtime.

## Traits

A trait is a named set of methods; an `impl` block provides them for a
specific type. A method call `value.method(args)` dispatches on the
*runtime type* of `value`, which is passed as `self`. This is
single-dispatch polymorphism (protocols / interfaces), resolved at runtime.

```olang
trait Show {
    fn show(self) -> String
    // a default method — used unless an impl overrides it
    fn shout(self) -> String = self.show() + "!"
}

type Point = struct { x: Int, y: Int }
type Circle = struct { r: Int }

impl Show for Point {
    fn show(self) = "(" + to_string(self.x) + ", " + to_string(self.y) + ")"
}
impl Show for Circle {
    fn show(self) = "Circle(" + to_string(self.r) + ")"
}

let p = Point { x: 3, y: 4 }
println(p.show())        // "(3, 4)"      — Point's impl
println(p.shout())       // "(3, 4)!"     — trait default, calls back into show

// Polymorphism: one call site, dispatched per element type
let shapes = [Point { x: 1, y: 1 }, Circle { r: 2 }]
for s in shapes {
    println(s.show())
}
```

Traits work over enums too, and methods can take arguments:

```olang
type Shape = enum { Sq(Int), Tri(Int, Int) }
trait Area { fn area(self) -> Int }
impl Area for Shape {
    fn area(self) = match self {
        Sq(s)    => s * s,
        Tri(b, h) => b * h / 2
    }
}
println(Sq(4).area())        // 16
println(Tri(6, 4).area())    // 12
```

Struct fields take precedence over methods of the same name, so field
access is never shadowed. A call to a method no `impl` provides is a
runtime error.

### Trait bounds

A generic function can require its type parameters to implement traits.
`fn f<T: Show>(x: T)` accepts only arguments whose type implements `Show`;
a value that doesn't fails at the call boundary with a clear message,
rather than deep inside the body. Multiple bounds use `+`.

```olang
trait Show { fn show(self) -> String }
type Point = struct { x: Int, y: Int }
impl Show for Point { fn show(self) = "(" + to_string(self.x) + ", " + to_string(self.y) + ")" }

fn describe<T: Show>(item: T) -> String = "showing " + item.show()
println(describe(Point { x: 1, y: 2 }))   // ok — Point implements Show

// describe(Circle { r: 5 }) would error:
//   describe: argument 1 of type Circle does not implement trait Show

// Multiple bounds: T must implement both
fn label<T: Show + Ord>(x: T) -> String = x.show()
```

`implements(value, "Trait")` answers whether a value's type implements a
trait, for introspection:

```olang
implements(Point { x: 0, y: 0 }, "Show")   // true
```

Bounds are enforced at runtime (olang stays dynamically typed); an
unbounded generic like `fn identity<T>(x: T) = x` accepts anything.

## Async/Await

### Async Operations

```olang
// Promise creation
let promise1 = Promise.resolve(42)
let promise2 = Promise.reject("Error message")

// Promise.delay(value, milliseconds): resolves to the value; `await`
// sleeps out whatever remains of the delay
let delayed = Promise.delay("Hello", 100)
let value = await delayed              // "Hello", after ~100ms

// Awaiting a resolved promise yields its value immediately
let n = await promise1                 // 42
```

### Concurrency

```olang
// Promise.all: await every promise, collect results in order
let results = await Promise.all([Promise.resolve(1), Promise.resolve(2)])
// results = [1, 2]

// Promise.race: the first settled promise wins
let first = await Promise.race([Promise.resolve("fast"), Promise.delay("slow", 500)])
// first = "fast"

// spawn: run a function call, await its handle for the result
fn work() = 42
let handle = spawn work()
let answer = await handle              // 42
```

## Loops

### For Loops

```olang
// For loop over list
for item in [1, 2, 3, 4] {
    println(item)
}

// For loop over range
for i in 0..10 {
    println("Number: " + to_string(i))
}

let names = ["ann", "bob"]

// For loop over a variable
for name in names {
    println("Hello, " + name)
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
let counter = 0
loop {
    counter = counter + 1
    if counter >= 3 => break
}
println(counter)   // 3
```

### Loop Control

Note that `if` is an expression and always uses `=>`, including when its body
is `break` or `continue`. There is no `if cond { ... }` statement form.

```olang
// Break and continue
for i in 0..10 {
    if i % 2 == 0 => continue   // Skip even numbers
    if i > 6 => break           // Stop past 6
    println(i)
}
// prints 1, 3, 5
```

To run several statements in a branch, use a block after `=>`:

```olang
for i in 0..5 {
    if i == 2 => {
        println("found two")
        continue
    }
    println(i)
}
```

## Module System

### Share Declarations

```olang no-run
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

```olang no-run
// Import specific items from module
use module_name { function1, function2, type1 };

// Import from nested module
use parent.child { item1, item2 };

// Import from multiple levels
use utils.math { add, subtract };
use utils.string { join, split };
```


For dependencies on other packages (path, git, or registry), see
[Packages](packages.md).

## Error Handling

### Result Types

```olang
// Result creation
let success: Result<Int, String> = Ok(42)
let failure: Result<Int, String> = Err("Something went wrong")

// Result handling (`error` is a reserved word — use `err` or `e`)
let processed = match success {
    Ok(value) => value * 2,
    Err(err) => {
        println("Error: " + err)
        0
    }
}
```

### Try-Catch

```olang
fn risky_operation() = Err("boom")
fn default_value() = 0

// Try-catch expression
let result = try {
    risky_operation()
} catch (e) {
    println("Caught error: " + e)
    default_value()
}
```

### Try Operator

```olang
fn risky_operation() = Ok(21)

// The postfix ? operator unwraps Ok or propagates Err to the caller
fn doubled() = {
    let value = risky_operation()?
    Ok(value * 2)
}
let result = doubled()      // Ok(42)
```

## Testing

### Test Declarations

```olang
fn add(a, b) = a + b

// Basic test
test "addition test" {
    let result = add(2, 3)
    assert_eq(result, 5)
}

// Test with custom message
test "string concatenation" {
    let result = "Hello" + " " + "World"
    assert_eq(result, "Hello World", "String concatenation failed")
}

// Test with assertions
test "boolean operations" {
    let value = true
    assert_true(value)
    assert_false(!value)
    assert(value == true)
}

// Test with inequality
test "inequality test" {
    let a = 10
    let b = 20
    assert_ne(a, b, "Values should not be equal")
}
```

### Assertion Functions

```olang
let actual = 5
let expected = 5
let condition = true
let expression = 1 < 2

test "assertion forms" {
    // Equality / inequality
    assert_eq(actual, expected)
    assert_eq(actual, expected, "Custom message")
    assert_ne(actual, 99)

    // Boolean assertions
    assert(condition)
    assert(condition, "Custom message")
    assert_true(expression)
    assert_false(!expression)
}
```

## Standard Library

olang ships with modules for common tasks. Access a module function with
dot syntax: `math.sqrt(2.0)`, `str.trim(text)`, `re.find_all(pattern, text)`.

### Return-type convention

Stdlib functions follow one rule:

- **Total** operations — those that cannot fail for a correctly-typed
  argument — return the value directly. `crypto.sha256("x")` returns the hex
  string; `str.to_upper("hi")` returns `"HI"`; `math.sqrt(2.0)` returns the
  root.
- **Fallible** operations — parsing, I/O, decoding, network, or anything that
  can fail on the *value* of its input — return a `Result`. Handle it with
  `match`, `unwrap`, `unwrap_or`, or the `?` operator.

```olang
// total: use the value directly
let h = crypto.sha256("password")
let upper = str.to_upper("hello")

// fallible: handle the Result
let parsed = match str.parse_int("42") {
    Ok(n) => n,
    Err(e) => 0
}
let day = unwrap(dates.add_days("2026-08-05", 90))
```

### Modules

| Module | Purpose |
|---|---|
| `str` | String manipulation (case, trim, split, replace, pad, search, parse) |
| `re` | Regular expressions (match, find, captures, split, replace) |
| `col` | Higher-order list operations (min_by, sort_by, group/count, partition, unique, window, ...) |
| `db` | SQLite database: open, execute, query with bound parameters |
| `math` | Numeric functions and constants |
| `crypto` | Hashing, HMAC, bcrypt passwords, AES, RSA signatures |
| `dates` | Calendar arithmetic, parsing, formatting, components |
| `json` | Parse, query, and transform JSON text |
| `csv` | Read and write CSV |
| `base64` | Base64 encode/decode |
| `random` | Random numbers, choices, strings |
| `fs` | File system operations |
| `http` | HTTP client |
| `os` | Environment, process, and system info |

### str

Character-indexed string operations. Total operations return the value;
`parse_int` / `parse_float` return a `Result`.

```olang
str.to_upper("hi")                  // "HI"
str.to_lower("HI")                  // "hi"
str.trim("  x  ")                   // "x"
str.replace("a-b-c", "-", "+")      // "a+b+c"
str.split("a,b,c", ",")             // ["a", "b", "c"]
str.join(["a", "b"], "-")           // "a-b"
str.substring("hello", 0, 3)        // "hel" (indices clamp; never fails)
str.index_of("hello", "llo")        // 2 (-1 if absent)
str.repeat("ab", 3)                 // "ababab"
str.pad_start("7", 3, "0")          // "007"
str.reverse("abc")                  // "cba"
str.capitalize("hi")                // "Hi"
str.words("  a  b ")                // ["a", "b"]
str.lines("a\nb")                  // ["a", "b"]
str.count("banana", "a")            // 3
str.char_at("héllo", 1)            // "é" (by character; "" if out of range)
str.length("héllo")                // 5 (characters, not bytes)
unwrap(str.parse_int("42"))         // 42
unwrap(str.parse_float("3.5"))      // 3.5
```

### re

Regular expressions. Every operation but `is_valid` returns a `Result` —
a malformed pattern is a recoverable `Err`, not a crash.

```olang
re.is_valid("[a-z]+")                          // true (total)
unwrap(re.is_match("^\\d+$", "123"))            // true
unwrap(re.find("\\d+", "abc123"))               // "123" ("" if no match)
unwrap(re.find_all("\\d+", "a1b22c333"))        // ["1", "22", "333"]
unwrap(re.captures("(\\w+)@(\\w+)", "u@h"))    // ["u@h", "u", "h"]
unwrap(re.split(",\\s*", "a, b,c"))             // ["a", "b", "c"]
unwrap(re.replace_all("\\s+", "a  b", "_"))     // "a_b"
```

### col

Higher-order list operations — the ones you would otherwise write by hand
with `fold`. Key functions and predicates are ordinary olang functions.

```olang no-run
col.min_by(people, (p) => p.age)       // element with the smallest key
col.max_by(people, (p) => p.age)       // element with the largest key
col.sort_by([3, 1, 2], (x) => x)       // [1, 2, 3]
col.count_by(people, (p) => p.team)    // {team: count, ...}
col.frequencies([1, 2, 2, 3])          // {1: 1, 2: 2, 3: 1}
col.partition([1,2,3,4], (x)=>x%2==0)  // ([2, 4], [1, 3])
col.flat_map([1, 2], (x) => [x, x])    // [1, 1, 2, 2]
col.take_while([1,2,9,1], (x)=>x<5)    // [1, 2]
col.drop_while([1,2,9,1], (x)=>x<5)    // [9, 1]
col.all([2, 4], (x) => x % 2 == 0)     // true
col.any([1, 2], (x) => x % 2 == 0)     // true
col.sum_by(items, (x) => x.price)      // total of a projection
col.unique([1, 1, 2, 3, 3])            // [1, 2, 3]
col.window([1, 2, 3, 4], 2)            // [[1,2], [2,3], [3,4]]
col.zip_with([1,2], [3,4], (a,b)=>a+b) // [4, 6]
col.last([7, 8, 9])                    // 9
```

The core operations — `map`, `filter`, `fold`, `reduce`, `group_by`, `find`,
`zip`, `sort` — are top-level builtins, callable without a module prefix.

### db

An embedded SQLite database (bundled — no system dependency). Every
operation returns a `Result`. Bind values with `?` placeholders rather than
splicing them into SQL, so untrusted input is always safe.

```olang no-run
let c = unwrap(db.open(":memory:"))          // or a file path to persist

unwrap(db.execute(c, "CREATE TABLE users (name TEXT, age INTEGER)"))
unwrap(db.execute(c, "INSERT INTO users VALUES (?, ?)", ["Ann", 30]))

// query -> list of rows; each row is a map from column name to value
let rows = unwrap(db.query(c, "SELECT name, age FROM users WHERE age >= ?", [18]))
for row in rows {
    println(map_get(row, "name"))
}

// query_one -> the first row (a map), or unit when there is none
let count = unwrap(db.query_one(c, "SELECT COUNT(*) AS n FROM users"))

unwrap(db.close(c))
```

SQLite types map to olang as: NULL→unit, INTEGER→Int, REAL→Float, TEXT→String.

## Help System

### Enhanced Help Commands

```text
// Basic help
:help                    // Show general help
:help println           // Show function help
:help list               // Show categories and functions

// Advanced search
:help search print       // Search for print-related functions
:help search "http get"  // Search with multiple terms

// Interactive tutorials
:help tutorials          // List available tutorials
:help tutorial basic     // View basic tutorial
:help tutorial lists     // View list operations tutorial

// Contextual help
:help contextual         // Get help based on current REPL state
```

### Shell Commands

The REPL can run shell commands and navigate the filesystem. `cd` changes the
REPL's own working directory, so relative paths in `fs.` calls follow along:

```text
:pwd                     // Print working directory
:cd src                  // Change directory (supports ~)
:ls -la                  // List files
:sh cat data.csv | head  // Any shell command; pipes and globs work
!git status              // ! is shorthand for :sh
```

TAB completes REPL commands, function and variable names, and file paths —
including inside string literals, so `fs.read("src/ma` completes to
`src/main.rs`.

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
   - Complete Stdlib Implementation (11 modules)
   - All stdlib modules fully tested

9. **Testing**
   - Test declarations
   - Assertion functions
   - Test framework integration

10. **Developer Experience**
    - Enhanced Help System
    - Interactive Tutorials
    - Better REPL Error Messages

11. **Execution**
    - Tree-walking interpreter (the semantics reference)
    - Opt-in bytecode tier for hot functions (`--ovm-tier`), covered by
      differential tests against the interpreter — see
      [OVM Architecture](ovm.md)

### In Progress Features

1. **Advanced Type System**
   - Generic type constraints
   - Type inference improvements
   - Complex type validation

2. **Bytecode Tier Coverage**
   - Only self-recursive and leaf functions are promoted today; functions
     calling other user functions still interpret
   - `match`, `for` loops, lambdas, and pipelines are not yet compiled
   - See [Known limitations](ovm.md#known-limitations)

3. **Performance Optimizations**
   - Lazy evaluation edge cases

### Known Limitations

1. **Module System**
   - Module resolution has verbose debug output
   - Import/export needs better error handling

2. **Advanced Type Features**
   - Generic type constraints partially implemented
   - Some complex type scenarios not fully supported

3. **Performance**
   - Recursion is bounded at 1000 frames and reports
     "Maximum call depth exceeded" rather than crashing
   - The bytecode tier covers a subset of the language; everything outside it
     runs on the interpreter

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

```text
// REPL commands for testing help system
:help map_get                    // Function help
:help search json                // Search
:help tutorials                  // List tutorials
:help tutorial basic             // View tutorial
:help contextual                 // Contextal help
```

This comprehensive syntax documentation reflects the current state of Olang, including all completed features and known limitations. The language has evolved significantly with the addition of comprehensive literal support, bitwise operations, test declarations, enhanced type system, and an improved developer experience. 