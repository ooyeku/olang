# Olang Syntax Documentation

## Overview

This document describes the complete syntax of the Olang programming language based on the grammar specification and identifies implementation gaps where the interpreter doesn't properly support defined syntax.

## Table of Contents

1. [Literals](#literals)
2. [Identifiers and Variables](#identifiers-and-variables)
3. [Lists and Tuples](#lists-and-tuples)
4. [Ranges](#ranges)
5. [Functions](#functions)
6. [Control Flow](#control-flow)
7. [Pattern Matching](#pattern-matching)
8. [Operators](#operators)
9. [Type System](#type-system)
10. [Async/Await](#asyncawait)
11. [Loops](#loops)
12. [Module System](#module-system)
13. [Error Handling](#error-handling)
14. [Implementation Gaps](#implementation-gaps)

## Literals

### Supported Literals

```olang
// Numbers
42          // Integer
3.14        // Float
-123        // Negative integer
-2.5        // Negative float

// Strings
"Hello"     // String literal
"Hello \"World\""  // String with escaped quotes
"Line 1\nLine 2"   // String with newlines

// Booleans
true
false

// Lists
[1, 2, 3]      // List of integers
["a", "b"]     // List of strings
[]             // Empty list

// Tuples
(1, 2)         // Tuple with two elements
(1, "hello", true)  // Mixed type tuple

// Unit
()            // Unit type (represented as empty tuple syntax)
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
"Unicode: \u0041" // \uXXXX (4 hex digits)
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
let first = numbers[0];    // ✅ SUPPORTED
let second = words[1];     // ✅ SUPPORTED

// List functions (built-in)
let doubled = numbers |> map((x) => x * 2);     // ✅ SUPPORTED
let evens = numbers |> filter((x) => x % 2 == 0); // ✅ SUPPORTED
let total = numbers |> sum();                   // ✅ SUPPORTED
```

### Tuple Operations

```olang
// Tuple creation
let point = (10, 20);
let person = ("Alice", 30, true);

// Tuple indexing
let x = point[0];     // ✅ SUPPORTED
let y = point[1];     // ✅ SUPPORTED
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
```

### 🚨 IMPLEMENTATION GAP: Range Integration Issues

The grammar supports ranges, and the interpreter creates `Value::Range` objects, but there are integration issues:

1. **Pipeline Issue**: Ranges work with builtin functions but fail with lazy evaluation
2. **Error**: "Cannot map over non-list value" from internal lazy system
3. **Root Cause**: `internal/mod.rs` doesn't handle `Value::Range` in lazy evaluation

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

### 🚨 IMPLEMENTATION GAP: If Expression Issues

The grammar supports if expressions but there may be parsing issues with the `=>` syntax.

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

// Function types
let calculator: (Int, Int) -> Int = (a, b) => a + b;

// Generic types
let items: List<String> = ["a", "b", "c"];

// Result types
let operation: Result<Int, String> = Ok(42);

// Promise types
let async_data: Promise<String, String> = fetch_data("url");
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
```

### Error Types

```olang
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

### Imports

```olang
// Import entire module
import "path/to/module";

// Import specific items
import { function1, function2 } from "module";

// Wildcard import
import * from "utilities";
```

### Exports

```olang
// Export declaration
export my_function = (x) => x * 2;
export my_constant = 42;
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

## Implementation Gaps

### 🚨 Critical Issues

1. **Range Pipeline Integration**
   - **Grammar**: ✅ Supports `1..10` and `1..=10`
   - **AST**: ✅ Has `Value::Range` type
   - **Interpreter**: ✅ Creates ranges correctly
   - **Builtin Functions**: ✅ `map`, `filter` handle ranges
   - **❌ Issue**: Lazy evaluation system doesn't handle ranges
   - **Error**: "Cannot map over non-list value" from `internal/mod.rs:270`

2. **Lambda Syntax in Pipelines**
   - **Grammar**: ✅ Supports `(x) => expr` syntax
   - **❌ Issue**: Parser may not handle lambda arguments correctly in pipelines
   - **Symptom**: `large_range |> map((x) => x * x)` fails

3. **Zero-Argument Function Calls**
   - **Grammar**: ✅ Supports `function_call = { "(" ~ arg_list? ~ ")" }`
   - **❌ Issue**: Parser might not handle `()` calls correctly
   - **Memory**: [[memory:5151249837983706124]] mentions this was fixed

### 🚨 Identified Syntax Gaps

1. **If Expression Syntax**
   - **Grammar**: Uses `=>` syntax: `if condition => then_expr else => else_expr`
   - **❌ Issue**: May not be parsing correctly

2. **Block Expressions**
   - **Grammar**: ✅ Supports `{ statements }`
   - **❌ Issue**: Block return values may not work correctly

3. **Struct Literal Syntax**
   - **Grammar**: ✅ Supports `TypeName { field: value }`
   - **❌ Issue**: May not be parsing field assignments correctly

4. **Generic Type Syntax**
   - **Grammar**: ✅ Supports `List<T>`, `Result<T, E>`
   - **❌ Issue**: Type checker may not handle generics

5. **Pattern Matching Completeness**
   - **Grammar**: ✅ Supports comprehensive patterns
   - **❌ Issue**: Some pattern types may not be implemented

6. **Async Syntax**
   - **Grammar**: ✅ Supports `async fn`, `await expr`
   - **❌ Issue**: Parser integration may be incomplete

7. **Loop Syntax**
   - **Grammar**: ✅ Supports `for`, `while`, `loop`
   - **❌ Issue**: Loop variable scoping may be incorrect

8. **Module System**
   - **Grammar**: ✅ Supports `import`/`export`
   - **❌ Issue**: Module resolution not implemented

9. **Error Handling**
   - **Grammar**: ✅ Supports `try`/`catch`, `?` operator
   - **❌ Issue**: Error propagation may not work

10. **Type Annotations**
    - **Grammar**: ✅ Supports comprehensive type syntax
    - **❌ Issue**: Type checker may not validate all cases

### 🔧 Immediate Fixes Needed

1. **Fix Range Lazy Evaluation**
   ```rust
   // In src/internal/mod.rs, update evaluate_mapped_list
   match source_value {
       Value::List(items) => { /* existing code */ }
       Value::Range { start, end, inclusive } => {
           // Convert range to vector and map
           let end_val = if inclusive { end + 1 } else { end };
           let mut results = Vec::new();
           for i in start..end_val {
               let result = interpreter.call_function(
                   Value::Function((**mapper).to_function()),
                   vec![Value::Integer(i)],
               )?;
               results.push(result);
           }
           Ok(Value::List(Arc::from(results)))
       }
       _ => Err(InterpreterError::TypeError {
           message: "Cannot map over non-list value".to_string(),
       }),
   }
   ```

2. **Fix Lambda Parsing**
   - Check parser handling of lambda expressions in pipeline contexts

3. **Fix If Expression Parsing**
   - Verify `=>` syntax parsing in conditional expressions

4. **Add Missing Builtin Functions**
   - Implement missing functions referenced in grammar

## Testing Strategy

### Syntax Validation Tests

```olang
// Test file: test_syntax_validation.rap

// Range operations
let range_test = 1..1000 |> map((x) => x * 2) |> sum();

// Lambda syntax
let lambda_test = [1, 2, 3] |> map((x) => x + 1) |> filter((x) => x > 2);

// If expressions
let if_test = if true => "yes" else => "no";

// Pattern matching
let pattern_test = match [1, 2, 3] {
    [] => "empty",
    [x] => "single",
    _ => "multiple"
};

// Async operations
async fn test_async() = {
    let result = await Promise.resolve(42);
    result * 2
};
```

This comprehensive syntax documentation reveals that while Olang has an extensive and well-designed grammar, there are several critical implementation gaps that prevent many syntactic features from working correctly. The most pressing issue is the range pipeline integration problem that's causing the errors you're seeing. 