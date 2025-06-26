# Olang Language - Comprehensive Usage Guide

## Table of Contents
- [Introduction](#introduction)
- [Installation](#installation)
- [Basic Syntax](#basic-syntax)
  - [Comments](#comments)
  - [Variables](#variables)
  - [Data Types](#data-types)
  - [Operators](#operators)
- [Control Structures](#control-structures)
  - [Conditionals](#conditionals)
  - [Loops](#loops)
  - [Pattern Matching](#pattern-matching)
- [Functions](#functions)
  - [Function Declaration](#function-declaration)
  - [Lambda Functions](#lambda-functions)
  - [Recursion](#recursion)
- [Data Structures](#data-structures)
  - [Lists](#lists)
  - [Tuples](#tuples)
  - [Structs](#structs)
  - [Enums](#enums)
- [Generics](#generics)
  - [Generic Functions](#generic-functions)
  - [Generic Types](#generic-types)
- [Error Handling](#error-handling)
  - [Result Type](#result-type)
  - [Try-Catch](#try-catch)
- [Functional Programming](#functional-programming)
  - [Higher-Order Functions](#higher-order-functions)
  - [Pipeline Operator](#pipeline-operator)
  - [Ranges](#ranges)
- [Modules and Imports](#modules-and-imports)
  - [Importing Modules](#importing-modules)
  - [Exporting Definitions](#exporting-definitions)
- [Standard Library](#standard-library)
  - [Math Module](#math-module)
  - [Random Module](#random-module)
  - [Filesystem Module](#filesystem-module)
  - [HTTP Module](#http-module)
- [Advanced Examples](#advanced-examples)
  - [Comprehensive Example](#comprehensive-example)
  - [Real-World Applications](#real-world-applications)

## Introduction

Olang is a modern, expressive programming language designed to combine the best features of functional and imperative programming paradigms. It offers a clean syntax, powerful type system with generics, pattern matching, and a comprehensive standard library.

Key features of Olang include:
- Clean, minimal syntax
- First-class functions and lambdas
- Powerful pattern matching
- Structs and custom types
- Generics for both functions and types
- Pipeline operator for functional composition
- Comprehensive standard library

This guide provides a thorough overview of Olang's features and how to use them effectively.

## Installation

To install Olang, follow these steps:

1. Clone the repository:
   ```bash
   git clone https://github.com/yourusername/olang.git
   cd olang
   ```

2. Build the project:
   ```bash
   cargo build --release
   ```

3. Add the binary to your PATH:
   ```bash
   export PATH=$PATH:$(pwd)/target/release
   ```

## Basic Syntax

### Comments

Olang uses double slashes for single-line comments:

```
// This is a comment
let x = 42; // This is an end-of-line comment
```

### Variables

Variables in Olang are declared using the `let` keyword:

```
let name = "Alice";
let age = 30;
let is_active = true;
```

You can optionally specify the type:

```
let name: String = "Alice";
let age: Int = 30;
let is_active: Bool = true;
```

### Data Types

Olang supports the following basic data types:

- **Int**: Integer numbers
  ```
  let x = 42;
  ```

- **Float**: Floating-point numbers
  ```
  let pi = 3.14159;
  ```

- **String**: Text strings
  ```
  let greeting = "Hello, World!";
  ```

- **Bool**: Boolean values
  ```
  let is_valid = true;
  ```

- **List**: Ordered collections of items
  ```
  let numbers = [1, 2, 3, 4, 5];
  ```

- **Tuple**: Fixed-size collections of items with potentially different types
  ```
  let person = ("Alice", 30);
  ```

- **Unit**: Represents no value, similar to `void` in other languages
  ```
  let nothing = ();
  ```

### Operators

Olang supports the following operators:

- **Arithmetic**: `+`, `-`, `*`, `/`, `%`
  ```
  let sum = 5 + 3;
  let product = 4 * 2;
  ```

- **Comparison**: `==`, `!=`, `<`, `>`, `<=`, `>=`
  ```
  let is_equal = x == y;
  let is_greater = a > b;
  ```

- **Logical**: `&&`, `||`
  ```
  let both_true = condition1 && condition2;
  let either_true = condition1 || condition2;
  ```

- **Range**: `..`, `..=`
  ```
  let range1 = 1..5;  // 1, 2, 3, 4
  let range2 = 1..=5; // 1, 2, 3, 4, 5
  ```

- **Pipeline**: `|>`
  ```
  let result = [1, 2, 3] |> map((n) => n * 2);
  ```

## Control Structures

### Conditionals

Olang uses the `if` expression for conditionals:

```
let result = if x > 10 => "large" else => "small";
```

For multi-line conditionals, you can use blocks:

```
let description = if age < 18 => {
    "minor"
} else => {
    "adult"
};
```

### Loops

Olang supports several types of loops:

- **For loop**:
  ```
  for i in 1..5 {
    println(i);
  }
  ```

- **While loop**:
  ```
  let i = 0;
  while i < 5 {
    println(i);
    i = i + 1;
  }
  ```

- **Loop** (infinite loop):
  ```
  loop {
    println("Infinite loop");
    if condition => break;
  }
  ```

- **Break and Continue**:
  ```
  for i in 1..10 {
    if i % 2 == 0 => continue;
    if i > 7 => break;
    println(i);
  }
  ```

### Pattern Matching

Olang provides powerful pattern matching with the `match` expression:

```
let result = match value {
    0 => "zero",
    1 => "one",
    2 => "two",
    _ => "many"
};
```

Pattern matching works with tuples and other data structures:

```
let description = match person {
    ("Alice", age) => "Alice is " + to_string(age),
    ("Bob", _) => "This is Bob",
    (name, age) if age > 30 => name + " is over 30",
    _ => "Unknown person"
};
```

## Functions

### Function Declaration

Functions in Olang are declared using the `fn` keyword:

```
fn add(a: Int, b: Int) -> Int = a + b;
```

For multi-line functions, use a block:

```
fn calculate(x: Int, y: Int) -> Int {
    let sum = x + y;
    let product = x * y;
    sum + product
}
```

### Lambda Functions

Lambda functions (anonymous functions) use the `=>` syntax:

```
let double = (n) => n * 2;
let add = (a, b) => a + b;
```

Lambda functions can also have blocks:

```
let process = (x) => {
    let doubled = x * 2;
    let squared = doubled * doubled;
    squared
};
```

### Recursion

Olang supports recursive functions:

```
fn factorial(n: Int) -> Int = 
    if n <= 1 => 1 else => n * factorial(n - 1);

fn fibonacci(n: Int) -> Int = 
    if n <= 1 => n else => fibonacci(n - 1) + fibonacci(n - 2);
```

## Data Structures

### Lists

Lists are ordered collections of items of the same type:

```
let numbers = [1, 2, 3, 4, 5];
let empty_list = [];
```

Common list operations:

```
let first = head(numbers);  // 1
let rest = tail(numbers);   // [2, 3, 4, 5]
let length = len(numbers);  // 5
let contains_3 = contains(numbers, 3);  // true
```

### Tuples

Tuples are fixed-size collections of items with potentially different types:

```
let person = ("Alice", 30);
let point_3d = (10.5, 20.3, 30.1);
```

Accessing tuple elements through pattern matching:

```
match person {
    (name, age) => println(name + " is " + to_string(age) + " years old")
}
```

### Structs

Structs are custom data types with named fields:

```
type Person = struct {
    name: String,
    age: Int,
    active: Bool,
};

let alice = Person {
    name: "Alice",
    age: 30,
    active: true,
};

println(alice.name);  // Alice
println(alice.age);   // 30
```

### Enums

Enums define a type that can be one of several variants:

```
type Option<T> = enum {
    Some(T),
    None,
};

type Result<T, E> = enum {
    Ok(T),
    Err(E),
};
```

Using enums with pattern matching:

```
let maybe_value = Some(42);

let result = match maybe_value {
    Some(value) => "Got value: " + to_string(value),
    None => "No value",
};
```

## Generics

### Generic Functions

Functions can be parameterized with type variables:

```
fn identity<T>(x: T) -> T = x;

let int_result = identity(42);
let string_result = identity("hello");
```

### Generic Types

Types can also be parameterized:

```
type Box<T> = struct {
    value: T
};

let int_box = Box { value: 42 };
let string_box = Box { value: "hello" };
```

Multiple type parameters:

```
fn pair<A, B>(a: A, b: B) = (a, b);

let p = pair(42, "answer");
```

## Error Handling

### Result Type

Olang uses the `Result` type for error handling:

```
let file_result = fs.read_file("config.txt");

match file_result {
    Ok(content) => println("File content: " + content),
    Err(error) => println("Error reading file: " + error),
}
```

### Try-Catch

Olang also supports try-catch expressions:

```
let content = try {
    fs.read_file("config.txt")?
} catch (error) {
    println("Error: " + error);
    "default content"
};
```

The `?` operator can be used to propagate errors:

```
fn read_config() -> Result<String, String> {
    let content = fs.read_file("config.txt")?;
    Ok(content)
}
```

## Functional Programming

### Higher-Order Functions

Olang supports higher-order functions like `map`, `filter`, and `reduce`:

```
let numbers = [1, 2, 3, 4, 5];

// Map: transform each element
let doubled = map(numbers, (n) => n * 2);  // [2, 4, 6, 8, 10]

// Filter: keep elements that satisfy a predicate
let evens = filter(numbers, (n) => n % 2 == 0);  // [2, 4]

// Reduce: combine elements into a single value
let sum = reduce(numbers, 0, (acc, n) => acc + n);  // 15
```

### Pipeline Operator

The pipeline operator (`|>`) allows for clean function composition:

```
let result = [1, 2, 3, 4, 5]
  |> filter((n) => n % 2 == 0)
  |> map((n) => n * n)
  |> reduce(0, (acc, n) => acc + n);
```

### Ranges

Olang provides range syntax and functions:

```
// Range syntax
let range1 = 1..5;   // [1, 2, 3, 4]
let range2 = 1..=5;  // [1, 2, 3, 4, 5]

// Range function
let r1 = range(5);        // [0, 1, 2, 3, 4]
let r2 = range(1, 6);     // [1, 2, 3, 4, 5]
let r3 = range(0, 10, 2); // [0, 2, 4, 6, 8]
```

## Modules and Imports

### Importing Modules

Import specific items from a module:

```
import { add, subtract } from "math_utils";
```

Import all items from a module:

```
import * from "math_utils";
```

### Exporting Definitions

Export definitions to make them available to other modules:

```
export add = (a, b) => a + b;
```

## Standard Library

### Math Module

The math module provides mathematical constants and functions:

```
// Constants
println(math.PI);    // 3.141592653589793
println(math.E);     // 2.718281828459045

// Basic functions
println(math.abs(-42));    // 42
println(math.pow(2, 10));  // 1024
println(math.sqrt(144));   // 12

// Trigonometric functions
println(math.sin(math.PI / 2));  // 1.0
println(math.cos(0));            // 1.0
println(math.tan(math.PI / 4));  // 1.0

// Rounding functions
println(math.floor(3.7));  // 3.0
println(math.ceil(3.2));   // 4.0
println(math.round(3.5));  // 4.0

// Other functions
println(math.factorial(5));  // 120
println(math.gcd(12, 8));    // 4
println(math.lcm(4, 6));     // 12
```

### Random Module

The random module provides random number generation and related functions:

```
// Set seed for reproducible results
random.seed(999);

// Random number generation
println(random.random());           // Random float between 0 and 1
println(random.randint(1, 100));    // Random integer between 1 and 100
println(random.uniform(0.0, 1.0));  // Random float in range
println(random.gauss(0.0, 1.0));    // Random float from normal distribution

// Random choices
println(random.randbool());                // Random boolean
println(random.choice([1, 2, 3, 4, 5]));   // Random element from list
println(random.choices([1, 2, 3], 5));     // 5 random elements with replacement
println(random.sample([1, 2, 3, 4, 5], 3)); // 3 random elements without replacement

// Random strings
println(random.randstr(8));           // Random 8-character string
println(random.randstr_alpha(6));     // Random 6-character alphabetic string
println(random.randstr_numeric(4));   // Random 4-character numeric string
println(random.randstr_alnum(10));    // Random 10-character alphanumeric string

// List operations
let numbers = [10, 20, 30, 40, 50];
println(random.shuffle(numbers));     // Shuffled list
```

### Filesystem Module

The filesystem module provides file and directory operations:

```
// File reading and writing
let content = "Hello, World!";
println(fs.write_file("example.txt", content));  // Write to file
println(fs.read_file("example.txt"));            // Read from file
println(fs.append_file("example.txt", "\nMore content")); // Append to file

// File information
println(fs.exists("example.txt"));     // Check if file exists
println(fs.is_file("example.txt"));    // Check if path is a file
println(fs.is_dir("example.txt"));     // Check if path is a directory
println(fs.file_size("example.txt"));  // Get file size in bytes
println(fs.file_info("example.txt"));  // Get detailed file information

// Directory operations
println(fs.create_dir("example_dir"));           // Create directory
println(fs.create_dir_all("nested/dirs"));       // Create nested directories
println(fs.list_dir("example_dir"));             // List directory contents
println(fs.remove_dir("example_dir"));           // Remove empty directory
println(fs.remove_dir_all("nested"));            // Remove directory and contents

// File manipulation
println(fs.copy_file("example.txt", "copy.txt")); // Copy file
println(fs.move_file("copy.txt", "moved.txt"));   // Move/rename file
println(fs.remove_file("moved.txt"));             // Delete file
```

### HTTP Module

The HTTP module provides HTTP client and server functionality:

```
// HTTP client operations
let response = http.get("https://api.example.com/data");
println(response);

let post_response = http.post("https://api.example.com/data", "request body");
println(post_response);

let put_response = http.put("https://api.example.com/data", "updated data");
println(put_response);

let delete_response = http.delete("https://api.example.com/data");
println(delete_response);

// Custom request
let custom_response = http.request("PATCH", "https://api.example.com/data", "partial update");
println(custom_response);

// URL parsing and manipulation
let url_info = http.parse_url("https://example.com/path?query=value");
println(url_info);

// Query string encoding/decoding
let params = {
    name: "John",
    age: "30"
};
let query_string = http.encode_query(params);
println(query_string);  // "name=John&age=30"

let decoded = http.decode_query("name=John&age=30");
println(decoded);  // { name: "John", age: "30" }

// HTTP response creation (for server implementations)
let response = http.response(200, "Success");
println(response);

let headers = {
    "Content-Type": "application/json",
    "X-Custom-Header": "value"
};
let response_with_headers = http.response_with_headers(200, "Success", headers);
println(response_with_headers);
```

## Advanced Examples

### Comprehensive Example

Here's a comprehensive example that demonstrates many of Olang's features:

```
// Define a Person struct
type Person = struct {
    name: String,
    age: Int,
    active: Bool,
};

// Create a list of people
let people = [
    Person { name: "Alice", age: 30, active: true },
    Person { name: "Bob", age: 25, active: false },
    Person { name: "Charlie", age: 35, active: true },
    Person { name: "Diana", age: 28, active: true }
];

// Filter active people, sort by age, and format as strings
let result = people
  |> filter((p) => p.active)
  |> sort((a, b) => a.age - b.age)
  |> map((p) => p.name + " (" + to_string(p.age) + ")");

println("Active people sorted by age:");
result |> map(println);

// Calculate average age using reduce
let total_age = people |> reduce(0, (acc, p) => acc + p.age);
let average_age = total_age / len(people);
println("Average age: " + to_string(average_age));

// Use pattern matching to categorize people
let categories = people |> map((p) => {
    match p {
        Person { age, .. } if age < 25 => "young",
        Person { age, .. } if age >= 25 && age < 35 => "adult",
        _ => "senior"
    }
});

println("Age categories:");
categories |> map(println);
```

### Real-World Applications

Olang can be used for a variety of applications:

1. **Data Processing**:
   ```
   // Read a CSV file, parse it, and calculate statistics
   let content = fs.read_file("data.csv")?;
   let lines = content |> split("\n");
   let header = head(lines);
   let data = tail(lines);
   
   let parsed = data
     |> filter((line) => line != "")
     |> map((line) => line |> split(",") |> map(parse_float));
   
   let averages = parsed
     |> transpose()
     |> map((column) => reduce(column, 0.0, (acc, val) => acc + val) / len(column));
   
   println("Column averages:");
   zip(split(header, ","), averages) |> map(println);
   ```

2. **Web API Client**:
   ```
   // Fetch data from an API and process it
   let response = http.get("https://api.example.com/users")?;
   
   if response.status >= 200 && response.status < 300 {
       let users = parse_json(response.body);
       
       let active_users = users
         |> filter((user) => user.active)
         |> map((user) => user.name);
       
       println("Active users:");
       active_users |> map(println);
   } else {
       println("Error: " + response.body);
   }
   ```

3. **File System Utilities**:
   ```
   // Find all .txt files in a directory and its subdirectories
   fn find_txt_files(dir: String) -> [String] {
       let entries = fs.list_dir(dir)?;
       
       let files = entries |> filter((entry) => {
           let path = dir + "/" + entry;
           fs.is_file(path) && path |> ends_with(".txt")
       });
       
       let subdirs = entries |> filter((entry) => {
           let path = dir + "/" + entry;
           fs.is_dir(path)
       });
       
       let subdir_files = subdirs |> flat_map((subdir) => {
           find_txt_files(dir + "/" + subdir)
       });
       
       files |> map((file) => dir + "/" + file) |> concat(subdir_files)
   }
   
   let txt_files = find_txt_files(".");
   println("Found " + to_string(len(txt_files)) + " .txt files:");
   txt_files |> map(println);
   ```

This concludes the comprehensive guide to the Olang programming language. For more information, refer to the official documentation and examples.